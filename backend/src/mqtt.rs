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
        if let Err(err) = deliver_commands(&state).await {
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

async fn deliver_commands(state: &AppState) -> anyhow::Result<()> {
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
