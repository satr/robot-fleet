use std::time::Duration;

use chrono::Utc;
use robot_fleet_common::{
    mqtt::parse_mqtt_url,
    types::{
        CommandResultMessage, RobotSensorEventMessage, RobotStreamMessage, StateMessage,
        TelemetryMessage,
    },
};
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS};
use tokio::time::sleep;
use tracing::{error, warn};

use crate::{app, app::AppState, db};

pub(crate) async fn connect_mqtt(
    url: &str,
    client_id: &str,
) -> anyhow::Result<(AsyncClient, rumqttc::EventLoop)> {
    let (host, port) = parse_mqtt_url(url)?;
    let mut options = MqttOptions::new(client_id, host, port);
    options.set_keep_alive(Duration::from_secs(10));
    Ok(AsyncClient::new(options, 10))
}

pub(crate) async fn run_mqtt_ingestion(state: AppState, mut eventloop: rumqttc::EventLoop) {
    subscribe(&state, "robots/+/telemetry", QoS::AtMostOnce, "telemetry").await;
    subscribe(&state, "robots/+/state", QoS::AtLeastOnce, "state").await;
    subscribe(
        &state,
        "robots/+/command-results",
        QoS::AtLeastOnce,
        "command results",
    )
    .await;
    subscribe(&state, "robots/+/events", QoS::AtLeastOnce, "sensor events").await;
    subscribe(
        &state,
        "robots/+/events/high-priority",
        QoS::AtLeastOnce,
        "high priority sensor events",
    )
    .await;

    loop {
        match eventloop.poll().await {
            Ok(Event::Incoming(Incoming::Publish(publish))) => {
                state.metrics.mqtt_connection_status.set(1.0);
                state.metrics.messages_received.inc();
                if let Err(err) =
                    handle_mqtt_message(&state, &publish.topic, &publish.payload).await
                {
                    warn!(topic = publish.topic, error = %err, "failed to handle MQTT message");
                }
            }
            Ok(_) => state.metrics.mqtt_connection_status.set(1.0),
            Err(err) => {
                state.metrics.mqtt_connection_status.set(0.0);
                warn!(error = %err, "MQTT event loop error; retrying");
                sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

pub(crate) async fn run_robot_status_broadcast(state: AppState) {
    loop {
        sleep(Duration::from_secs(5)).await;
        if let Err(err) = db::refresh_robot_status_metrics(&state).await {
            warn!(error = %err, "failed to refresh robot status metrics");
        }
        if let Err(err) = db::refresh_robot_motion_metrics(&state).await {
            warn!(error = %err, "failed to refresh robot motion metrics");
        }
        match db::list_robot_views(&state.pool).await {
            Ok(robots) => {
                for robot in robots {
                    let _ = state.robot_events.send(RobotStreamMessage {
                        event_type: "robot_updated".into(),
                        robot_id: Some(robot.robot_id.clone()),
                        robot: Some(robot),
                    });
                }
            }
            Err(err) => warn!(error = %err, "failed to broadcast robot status updates"),
        }
    }
}

pub(crate) async fn run_command_expiry(state: AppState) {
    loop {
        sleep(Duration::from_secs(2)).await;
        if let Err(err) = expire_commands(&state).await {
            warn!(error = %err, "failed to expire unacknowledged commands");
        }
    }
}

async fn subscribe(state: &AppState, topic: &str, qos: QoS, description: &str) {
    if let Err(err) = state.mqtt.subscribe(topic, qos).await {
        error!(error = %err, "failed to subscribe to {description}");
    }
}

async fn handle_mqtt_message(state: &AppState, topic: &str, payload: &[u8]) -> anyhow::Result<()> {
    if topic.ends_with("/telemetry") {
        let message: TelemetryMessage = serde_json::from_slice(payload)?;
        db::upsert_robot_from_telemetry(state, &message).await?;
        db::refresh_robot_status_metrics(state).await?;
        db::insert_telemetry(state, &message).await?;
        app::broadcast_robot_update(state, &message.robot_id).await;
        state.metrics.telemetry_received.inc();
        state
            .metrics
            .telemetry_lag_seconds
            .set((Utc::now() - message.recorded_at).num_milliseconds().max(0) as f64 / 1000.0);
    } else if topic.ends_with("/state") {
        let message: StateMessage = serde_json::from_slice(payload)?;
        db::upsert_robot_state(state, &message).await?;
        db::refresh_robot_status_metrics(state).await?;
        app::broadcast_robot_update(state, &message.robot_id).await;
    } else if topic.ends_with("/command-results") {
        let message: CommandResultMessage = serde_json::from_slice(payload)?;
        db::apply_command_result(state, &message).await?;
        app::broadcast_robot_update(state, &message.robot_id).await;
    } else if topic.ends_with("/events") || topic.ends_with("/events/high-priority") {
        let message: RobotSensorEventMessage = serde_json::from_slice(payload)?;
        db::insert_robot_sensor_event(state, &message).await?;
        app::broadcast_robot_update(state, &message.robot_id).await;
    }
    Ok(())
}

async fn expire_commands(state: &AppState) -> anyhow::Result<()> {
    let expired_commands = db::expire_unacknowledged_commands(state).await?;
    if !expired_commands.is_empty() {
        state
            .metrics
            .commands_expired_without_ack
            .inc_by(expired_commands.len() as u64);
        for command in expired_commands {
            app::broadcast_robot_update(state, &command.robot_id).await;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::Utc;
    use robot_fleet_common::types::CommandResultMessage;
    use rumqttc::{AsyncClient, MqttOptions};
    use serde_json::json;
    use sqlx::{postgres::PgPoolOptions, PgPool, Row};
    use tokio::sync::OnceCell;
    use uuid::Uuid;

    use super::handle_mqtt_message;
    use crate::{app::AppState, db, metrics::Metrics};

    static TEST_POOL: OnceCell<PgPool> = OnceCell::const_new();

    #[derive(Clone)]
    struct TestConfig {
        database_url: String,
        mqtt_host: String,
        mqtt_port: u16,
    }

    impl TestConfig {
        fn from_env() -> Option<Self> {
            Some(Self {
                database_url: std::env::var("DATABASE_URL").ok()?,
                mqtt_host: "localhost".into(),
                mqtt_port: 1883,
            })
        }
    }

    async fn test_state(config: &TestConfig) -> AppState {
        let database_url = config.database_url.clone();
        let pool = TEST_POOL
            .get_or_init(|| async move {
                let pool = PgPoolOptions::new()
                    .max_connections(5)
                    .connect(&database_url)
                    .await
                    .expect("connect to PostgreSQL");
                sqlx::migrate!("./migrations")
                    .run(&pool)
                    .await
                    .expect("run migrations");
                pool
            })
            .await
            .clone();
        let metrics = Arc::new(Metrics::new().expect("metrics"));
        let (mqtt, _) = AsyncClient::new(
            MqttOptions::new(
                format!("backend-test-{}", Uuid::new_v4()),
                &config.mqtt_host,
                config.mqtt_port,
            ),
            1,
        );

        AppState {
            pool,
            mqtt,
            metrics,
            robot_events: tokio::sync::broadcast::channel(16).0,
        }
    }

    async fn command_row(
        pool: &PgPool,
        command_id: Uuid,
    ) -> (
        String,
        Option<chrono::DateTime<Utc>>,
        Option<chrono::DateTime<Utc>>,
    ) {
        let row = sqlx::query(
            "SELECT status, acknowledged_at, completed_at
             FROM commands
             WHERE command_id = $1",
        )
        .bind(command_id)
        .fetch_one(pool)
        .await
        .expect("command row");
        (
            row.get("status"),
            row.get("acknowledged_at"),
            row.get("completed_at"),
        )
    }

    fn command_result(
        command_id: Uuid,
        robot_id: &str,
        status: &str,
        event_type: &str,
        payload: serde_json::Value,
    ) -> CommandResultMessage {
        CommandResultMessage {
            event_id: Uuid::new_v4(),
            command_id,
            robot_id: robot_id.into(),
            status: status.into(),
            event_type: event_type.into(),
            payload,
            occurred_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn mqtt_command_results_transition_and_ignore_duplicate_delivery() {
        let Some(config) = TestConfig::from_env() else {
            return;
        };
        let state = test_state(&config).await;

        let robot_id = format!("robot-{}", Uuid::new_v4());
        db::insert_placeholder_robot(&state.pool, &robot_id)
            .await
            .expect("insert placeholder robot");

        let command = db::create_command(
            &state.pool,
            &robot_id,
            "set_velocity",
            &json!({ "set_velocity": 1.5 }),
            None,
        )
        .await
        .expect("create command");

        let topic = format!("robots/{robot_id}/command-results");
        let acknowledged = command_result(
            command.command_id,
            &robot_id,
            "acknowledged",
            "command_acknowledged",
            json!({
                "command_type": "set_velocity",
                "payload": { "set_velocity": 1.5 }
            }),
        );
        handle_mqtt_message(
            &state,
            &topic,
            &serde_json::to_vec(&acknowledged).expect("serialize command result"),
        )
        .await
        .expect("handle acknowledged command result");

        let (status, acknowledged_at, completed_at) =
            command_row(&state.pool, command.command_id).await;
        assert_eq!(status, "acknowledged");
        assert!(acknowledged_at.is_some());
        assert!(completed_at.is_none());

        let running = command_result(
            command.command_id,
            &robot_id,
            "running",
            "command_running",
            json!({
                "command_type": "set_velocity",
                "payload": { "set_velocity": 1.5 }
            }),
        );
        handle_mqtt_message(
            &state,
            &topic,
            &serde_json::to_vec(&running).expect("serialize command result"),
        )
        .await
        .expect("handle running command result");

        let (status, acknowledged_at, completed_at) =
            command_row(&state.pool, command.command_id).await;
        assert_eq!(status, "running");
        assert!(acknowledged_at.is_some());
        assert!(completed_at.is_none());

        let completed = command_result(
            command.command_id,
            &robot_id,
            "completed",
            "command_completed",
            json!({
                "command_type": "set_velocity",
                "payload": { "set_velocity": 1.5 }
            }),
        );
        handle_mqtt_message(
            &state,
            &topic,
            &serde_json::to_vec(&completed).expect("serialize command result"),
        )
        .await
        .expect("handle completed command result");

        let (status, acknowledged_at, completed_at) =
            command_row(&state.pool, command.command_id).await;
        assert_eq!(status, "completed");
        assert!(acknowledged_at.is_some());
        assert!(completed_at.is_some());

        let duplicate_completed = CommandResultMessage {
            event_id: completed.event_id,
            command_id: completed.command_id,
            robot_id: completed.robot_id,
            status: completed.status,
            event_type: completed.event_type,
            payload: completed.payload,
            occurred_at: completed.occurred_at,
        };
        handle_mqtt_message(
            &state,
            &topic,
            &serde_json::to_vec(&duplicate_completed).expect("serialize duplicate command result"),
        )
        .await
        .expect("handle duplicate command result");

        let (status, acknowledged_at, completed_at) =
            command_row(&state.pool, command.command_id).await;
        assert_eq!(status, "completed");
        assert!(acknowledged_at.is_some());
        assert!(completed_at.is_some());

        let event_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM command_events WHERE command_id = $1")
                .bind(command.command_id)
                .fetch_one(&state.pool)
                .await
                .expect("count command events");
        assert_eq!(event_count, 3);
    }

    #[tokio::test]
    async fn mqtt_command_resume_only_revives_stopped_commands() {
        let Some(config) = TestConfig::from_env() else {
            return;
        };
        let state = test_state(&config).await;

        let robot_id = format!("robot-{}", Uuid::new_v4());
        db::insert_placeholder_robot(&state.pool, &robot_id)
            .await
            .expect("insert placeholder robot");

        let command = db::create_command(
            &state.pool,
            &robot_id,
            "move",
            &json!({ "target_position_x": 10.0, "target_position_y": 5.0 }),
            None,
        )
        .await
        .expect("create command");

        let topic = format!("robots/{robot_id}/command-results");
        for (status, event_type) in [
            ("acknowledged", "command_acknowledged"),
            ("running", "command_running"),
            ("stopped", "command_stopped"),
        ] {
            let message = command_result(
                command.command_id,
                &robot_id,
                status,
                event_type,
                json!({
                    "command_type": "move",
                    "payload": { "target_position_x": 10.0, "target_position_y": 5.0 }
                }),
            );
            handle_mqtt_message(
                &state,
                &topic,
                &serde_json::to_vec(&message).expect("serialize command result"),
            )
            .await
            .expect("handle command result");
        }

        let stale_running = command_result(
            command.command_id,
            &robot_id,
            "running",
            "command_running",
            json!({
                "command_type": "move",
                "payload": { "target_position_x": 10.0, "target_position_y": 5.0 }
            }),
        );
        handle_mqtt_message(
            &state,
            &topic,
            &serde_json::to_vec(&stale_running).expect("serialize command result"),
        )
        .await
        .expect("handle stale running command result");

        let (status, acknowledged_at, completed_at) =
            command_row(&state.pool, command.command_id).await;
        assert_eq!(status, "stopped");
        assert!(acknowledged_at.is_some());
        assert!(completed_at.is_none());

        let resumed = command_result(
            command.command_id,
            &robot_id,
            "running",
            "command_resumed",
            json!({
                "command_type": "move",
                "payload": { "target_position_x": 10.0, "target_position_y": 5.0 }
            }),
        );
        handle_mqtt_message(
            &state,
            &topic,
            &serde_json::to_vec(&resumed).expect("serialize command result"),
        )
        .await
        .expect("handle resumed command result");

        let (status, acknowledged_at, completed_at) =
            command_row(&state.pool, command.command_id).await;
        assert_eq!(status, "running");
        assert!(acknowledged_at.is_some());
        assert!(completed_at.is_none());
    }
}
