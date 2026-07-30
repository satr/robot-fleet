use std::time::Duration;

use anyhow::Context;
use chrono::{DateTime, Utc};
use robot_fleet_common::{
    status::robot_status_from_last_seen,
    types::{
        CommandResponse, CommandResultMessage, Robot, RobotSensorEventMessage, StateMessage,
        TelemetryMessage,
    },
};
use serde_json::Value;
use sqlx::{postgres::PgPoolOptions, PgPool, Row};
use tokio::time::sleep;
use tracing::warn;
use uuid::Uuid;

use crate::app::AppState;

const ROBOT_VIEW_SELECT: &str = "SELECT
     r.robot_id,
     r.name,
     r.status,
     r.battery_level,
     r.position_x,
     r.position_y,
     r.set_velocity,
     r.velocity,
     r.direction_degrees,
     r.stop,
     r.target_position_x,
     r.target_position_y,
     r.current_mission,
     r.state,
     r.last_seen_at,
     r.software_version,
     r.created_at,
     r.updated_at,
     latest_command.command_type AS current_command,
     latest_command.status AS current_command_status
 FROM robots r
 LEFT JOIN LATERAL (
     SELECT command_type, status
     FROM commands
     WHERE commands.robot_id = r.robot_id
     ORDER BY created_at DESC
     LIMIT 1
 ) latest_command ON TRUE";

pub(crate) async fn connect_postgres(database_url: &str) -> anyhow::Result<PgPool> {
    let mut last_error = None;
    for attempt in 1..=30 {
        match PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await
        {
            Ok(pool) => return Ok(pool),
            Err(err) => {
                warn!(attempt, error = %err, "PostgreSQL is not ready");
                last_error = Some(err);
                sleep(Duration::from_secs(2)).await;
            }
        }
    }
    Err(last_error.context("PostgreSQL connection was not attempted")?)
        .context("failed to connect to PostgreSQL")
}

pub(crate) async fn list_robot_views(pool: &PgPool) -> Result<Vec<Robot>, sqlx::Error> {
    let rows = sqlx::query(&format!("{ROBOT_VIEW_SELECT} ORDER BY r.robot_id"))
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(robot_from_row).collect())
}

pub(crate) async fn get_robot_view(
    pool: &PgPool,
    robot_id: &str,
) -> Result<Option<Robot>, sqlx::Error> {
    let row = sqlx::query(&format!("{ROBOT_VIEW_SELECT} WHERE r.robot_id = $1"))
        .bind(robot_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(robot_from_row))
}

pub(crate) async fn insert_placeholder_robot(
    pool: &PgPool,
    robot_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO robots (robot_id, name, status, software_version, updated_at)
         VALUES ($1, $1, 'unknown', 'unknown', now())
         ON CONFLICT (robot_id) DO NOTHING",
    )
    .bind(robot_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn create_command(
    pool: &PgPool,
    robot_id: &str,
    command_type: &str,
    payload: &Value,
    expires_at: Option<DateTime<Utc>>,
) -> Result<CommandResponse, sqlx::Error> {
    let command_id = Uuid::new_v4();
    let row = sqlx::query(
        "INSERT INTO commands (command_id, robot_id, command_type, payload, status, expires_at)
         VALUES ($1, $2, $3, $4, 'created', $5)
         RETURNING command_id, robot_id, command_type, payload, status, created_at, expires_at, acknowledged_at, completed_at",
    )
    .bind(command_id)
    .bind(robot_id)
    .bind(command_type)
    .bind(payload)
    .bind(expires_at)
    .fetch_one(pool)
    .await?;
    Ok(command_from_row(row))
}

pub(crate) async fn list_commands(
    pool: &PgPool,
    robot_id: &str,
) -> Result<Vec<CommandResponse>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT command_id, robot_id, command_type, payload, status, created_at, expires_at, acknowledged_at, completed_at
         FROM commands WHERE robot_id = $1 ORDER BY created_at DESC LIMIT 100",
    )
    .bind(robot_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(command_from_row).collect())
}

pub(crate) async fn delete_robot(pool: &PgPool, robot_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM robots WHERE robot_id = $1")
        .bind(robot_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub(crate) async fn insert_telemetry(
    state: &AppState,
    message: &TelemetryMessage,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO telemetry (robot_id, recorded_at, battery_level, temperature, position_x, position_y, velocity_cm_s, direction_degrees, payload)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         ON CONFLICT (robot_id, recorded_at) DO NOTHING",
    )
    .bind(&message.robot_id)
    .bind(message.recorded_at)
    .bind(message.battery_level)
    .bind(message.temperature)
    .bind(message.position_x)
    .bind(message.position_y)
    .bind(message.velocity_cm_s)
    .bind(message.direction_degrees)
    .bind(&message.payload)
    .execute(&state.pool)
    .await?;
    sync_motion_metrics(
        &state.metrics,
        &message.robot_id,
        message.position_x,
        message.position_y,
        message.velocity_cm_s,
        message.direction_degrees,
    );
    Ok(())
}

pub(crate) async fn upsert_robot_from_telemetry(
    state: &AppState,
    message: &TelemetryMessage,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO robots (robot_id, name, status, battery_level, position_x, position_y, velocity, direction_degrees, last_seen_at, software_version, updated_at)
         VALUES ($1, $1, 'online', $2, $3, $4, $5, $6, now(), 'unknown', now())
         ON CONFLICT (robot_id) DO UPDATE
         SET status = 'online',
            battery_level = EXCLUDED.battery_level,
            position_x = EXCLUDED.position_x,
            position_y = EXCLUDED.position_y,
            velocity = EXCLUDED.velocity,
            direction_degrees = EXCLUDED.direction_degrees,
            last_seen_at = now(),
            updated_at = now()",
    )
    .bind(&message.robot_id)
    .bind(message.battery_level)
    .bind(message.position_x)
    .bind(message.position_y)
    .bind(message.velocity_cm_s)
    .bind(message.direction_degrees)
    .execute(&state.pool)
    .await?;
    sync_motion_metrics(
        &state.metrics,
        &message.robot_id,
        message.position_x,
        message.position_y,
        message.velocity_cm_s,
        message.direction_degrees,
    );
    Ok(())
}

pub(crate) async fn upsert_robot_state(
    state: &AppState,
    message: &StateMessage,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO robots (robot_id, name, status, battery_level, position_x, position_y, set_velocity, velocity, direction_degrees, stop, target_position_x, target_position_y, current_mission, state, last_seen_at, software_version, updated_at)
         VALUES ($1, $2, 'online', $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, now(), $14, now())
         ON CONFLICT (robot_id) DO UPDATE
         SET name = EXCLUDED.name,
            status = 'online',
            battery_level = EXCLUDED.battery_level,
            position_x = EXCLUDED.position_x,
            position_y = EXCLUDED.position_y,
            set_velocity = EXCLUDED.set_velocity,
            velocity = EXCLUDED.velocity,
            direction_degrees = EXCLUDED.direction_degrees,
            stop = EXCLUDED.stop,
            target_position_x = EXCLUDED.target_position_x,
            target_position_y = EXCLUDED.target_position_y,
            current_mission = EXCLUDED.current_mission,
            state = EXCLUDED.state,
            last_seen_at = now(),
            software_version = EXCLUDED.software_version,
            updated_at = now()",
    )
    .bind(&message.robot_id)
    .bind(&message.name)
    .bind(message.battery_level)
    .bind(message.position_x)
    .bind(message.position_y)
    .bind(message.set_velocity)
    .bind(message.velocity)
    .bind(message.direction_degrees)
    .bind(message.stop)
    .bind(message.target_position_x)
    .bind(message.target_position_y)
    .bind(&message.current_mission)
    .bind(&message.state)
    .bind(&message.software_version)
    .execute(&state.pool)
    .await?;
    insert_robot_state_history(state, message).await?;
    sync_motion_metrics(
        &state.metrics,
        &message.robot_id,
        message.position_x,
        message.position_y,
        message.velocity,
        message.direction_degrees,
    );
    Ok(())
}

async fn insert_robot_state_history(
    state: &AppState,
    message: &StateMessage,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO robot_state_history (robot_id, recorded_at, state, status, battery_level, position_x, position_y, velocity, current_mission, payload)
         SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9, $10
         WHERE NOT EXISTS (
             SELECT 1
             FROM (
                 SELECT state
                 FROM robot_state_history
                 WHERE robot_id = $1
                 ORDER BY recorded_at DESC
                 LIMIT 1
             ) latest
             WHERE latest.state = $3
         )
         ON CONFLICT (robot_id, recorded_at) DO NOTHING",
    )
    .bind(&message.robot_id)
    .bind(message.recorded_at)
    .bind(&message.state)
    .bind(&message.status)
    .bind(message.battery_level)
    .bind(message.position_x)
    .bind(message.position_y)
    .bind(message.velocity)
    .bind(&message.current_mission)
    .bind(serde_json::json!({
        "set_velocity": message.set_velocity,
        "direction_degrees": message.direction_degrees,
        "stop": message.stop,
        "target_position_x": message.target_position_x,
        "target_position_y": message.target_position_y,
        "software_version": message.software_version,
    }))
    .execute(&state.pool)
    .await?;
    Ok(())
}

pub(crate) async fn insert_robot_sensor_event(
    state: &AppState,
    message: &RobotSensorEventMessage,
) -> Result<(), sqlx::Error> {
    insert_placeholder_robot(&state.pool, &message.robot_id).await?;
    sqlx::query(
        "INSERT INTO robot_sensor_events (event_id, robot_id, event_type, priority, command_id, payload, occurred_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (event_id, occurred_at) DO NOTHING",
    )
    .bind(message.event_id)
    .bind(&message.robot_id)
    .bind(&message.event_type)
    .bind(&message.priority)
    .bind(message.command_id)
    .bind(&message.payload)
    .bind(message.occurred_at)
    .execute(&state.pool)
    .await?;
    state
        .metrics
        .sensor_events_received
        .with_label_values(&[&message.event_type, &message.priority, &message.robot_id])
        .inc();
    Ok(())
}

pub(crate) async fn apply_command_result(
    state: &AppState,
    message: &CommandResultMessage,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO command_events (event_id, command_id, robot_id, event_type, payload, occurred_at)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (event_id) DO NOTHING",
    )
    .bind(Uuid::new_v4())
    .bind(message.command_id)
    .bind(&message.robot_id)
    .bind(&message.event_type)
    .bind(&message.payload)
    .bind(message.occurred_at)
    .execute(&state.pool)
    .await?;

    match message.status.as_str() {
        "acknowledged" => {
            sqlx::query("UPDATE commands SET status = 'acknowledged', acknowledged_at = COALESCE(acknowledged_at, $2) WHERE command_id = $1")
                .bind(message.command_id)
                .bind(message.occurred_at)
                .execute(&state.pool)
                .await?;
        }
        "completed" => {
            sqlx::query("UPDATE commands SET status = 'completed', completed_at = COALESCE(completed_at, $2) WHERE command_id = $1")
                .bind(message.command_id)
                .bind(message.occurred_at)
                .execute(&state.pool)
                .await?;
            state.metrics.commands_completed.inc();
        }
        "failed" | "expired" => {
            sqlx::query("UPDATE commands SET status = $2, completed_at = COALESCE(completed_at, $3) WHERE command_id = $1")
                .bind(message.command_id)
                .bind(&message.status)
                .bind(message.occurred_at)
                .execute(&state.pool)
                .await?;
            state.metrics.command_failures.inc();
        }
        "stopped" => {
            sqlx::query("UPDATE commands SET status = 'stopped' WHERE command_id = $1")
                .bind(message.command_id)
                .execute(&state.pool)
                .await?;
        }
        "running" => {
            sqlx::query("UPDATE commands SET status = 'running' WHERE command_id = $1")
                .bind(message.command_id)
                .execute(&state.pool)
                .await?;
        }
        other => warn!(
            robot_id = message.robot_id,
            command_id = %message.command_id,
            status = other,
            "unknown command status"
        ),
    }

    if let Some(projection) = command_state_projection(message) {
        apply_command_state_projection(state, &message.robot_id, projection).await?;
    }
    Ok(())
}

#[derive(Debug)]
enum CommandStateProjection {
    SetVelocity(f64),
    Move {
        target_position_x: Option<f64>,
        target_position_y: Option<f64>,
    },
    Stop(bool),
}

fn command_state_projection(message: &CommandResultMessage) -> Option<CommandStateProjection> {
    if !matches!(
        message.status.as_str(),
        "acknowledged" | "running" | "completed"
    ) {
        return None;
    }

    let command_type = message.payload.get("command_type")?.as_str()?;
    let payload = message.payload.get("payload")?;

    match command_type {
        "set_velocity" => payload
            .get("set_velocity")
            .and_then(Value::as_f64)
            .map(CommandStateProjection::SetVelocity),
        "move" => Some(CommandStateProjection::Move {
            target_position_x: payload.get("target_position_x").and_then(Value::as_f64),
            target_position_y: payload.get("target_position_y").and_then(Value::as_f64),
        }),
        "stop" => payload
            .as_bool()
            .or_else(|| payload.get("stop").and_then(Value::as_bool))
            .map(CommandStateProjection::Stop),
        _ => None,
    }
}

async fn apply_command_state_projection(
    state: &AppState,
    robot_id: &str,
    projection: CommandStateProjection,
) -> Result<(), sqlx::Error> {
    match projection {
        CommandStateProjection::SetVelocity(set_velocity) => {
            sqlx::query(
                "UPDATE robots
                 SET set_velocity = $2,
                     updated_at = now()
                 WHERE robot_id = $1",
            )
            .bind(robot_id)
            .bind(set_velocity)
            .execute(&state.pool)
            .await?;
        }
        CommandStateProjection::Move {
            target_position_x,
            target_position_y,
        } => {
            if let (Some(target_position_x), Some(target_position_y)) =
                (target_position_x, target_position_y)
            {
                sqlx::query(
                    "UPDATE robots
                     SET target_position_x = $2,
                         target_position_y = $3,
                         current_mission = 'move',
                         stop = FALSE,
                         updated_at = now()
                     WHERE robot_id = $1",
                )
                .bind(robot_id)
                .bind(target_position_x)
                .bind(target_position_y)
                .execute(&state.pool)
                .await?;
            }
        }
        CommandStateProjection::Stop(stop) => {
            sqlx::query(
                "UPDATE robots
                 SET stop = $2,
                     velocity = CASE WHEN $2 THEN 0 ELSE velocity END,
                     updated_at = now()
                 WHERE robot_id = $1",
            )
            .bind(robot_id)
            .bind(stop)
            .execute(&state.pool)
            .await?;
        }
    }

    Ok(())
}

pub(crate) async fn refresh_robot_status_metrics(state: &AppState) -> Result<(), sqlx::Error> {
    let row = sqlx::query(
        "SELECT
             COUNT(*) FILTER (WHERE last_seen_at >= now() - interval '5 seconds') AS online_count,
             COUNT(*) FILTER (
                 WHERE last_seen_at < now() - interval '5 seconds'
                   AND last_seen_at >= now() - interval '15 seconds'
             ) AS stale_count,
             COUNT(*) FILTER (
                 WHERE last_seen_at IS NULL
                    OR last_seen_at < now() - interval '15 seconds'
             ) AS offline_count
         FROM robots",
    )
    .fetch_one(&state.pool)
    .await?;

    let online_count: i64 = row.get("online_count");
    let stale_count: i64 = row.get("stale_count");
    let offline_count: i64 = row.get("offline_count");
    state.metrics.robots_online.set(online_count as f64);
    state.metrics.robots_stale.set(stale_count as f64);
    state.metrics.robots_offline.set(offline_count as f64);
    Ok(())
}

pub(crate) async fn refresh_robot_motion_metrics(state: &AppState) -> Result<(), sqlx::Error> {
    let rows = sqlx::query(
        "SELECT robot_id, position_x, position_y, velocity, direction_degrees
         FROM robots",
    )
    .fetch_all(&state.pool)
    .await?;

    for row in rows {
        let robot_id: String = row.get("robot_id");
        let position_x: f64 = row.get("position_x");
        let position_y: f64 = row.get("position_y");
        let velocity: f64 = row.get("velocity");
        let direction_degrees: f64 = row.get("direction_degrees");
        sync_motion_metrics(
            &state.metrics,
            &robot_id,
            position_x,
            position_y,
            velocity,
            direction_degrees,
        );
    }

    Ok(())
}

fn sync_motion_metrics(
    metrics: &crate::metrics::Metrics,
    robot_id: &str,
    position_x: f64,
    position_y: f64,
    velocity: f64,
    direction_degrees: f64,
) {
    metrics
        .robot_position_x_cm
        .with_label_values(&[robot_id])
        .set(position_x);
    metrics
        .robot_position_y_cm
        .with_label_values(&[robot_id])
        .set(position_y);
    metrics
        .robot_velocity_cm_s
        .with_label_values(&[robot_id])
        .set(velocity);
    metrics
        .robot_direction_degrees
        .with_label_values(&[robot_id])
        .set(direction_degrees);
}

fn robot_from_row(row: sqlx::postgres::PgRow) -> Robot {
    let last_seen_at = row.get("last_seen_at");
    Robot {
        robot_id: row.get("robot_id"),
        name: row.get("name"),
        status: robot_status_from_last_seen(Utc::now(), last_seen_at).to_string(),
        battery_level: row.get("battery_level"),
        position_x: row.get("position_x"),
        position_y: row.get("position_y"),
        set_velocity: row.get("set_velocity"),
        velocity: row.get("velocity"),
        direction_degrees: row.get("direction_degrees"),
        stop: row.get("stop"),
        target_position_x: row.get("target_position_x"),
        target_position_y: row.get("target_position_y"),
        current_mission: row.get("current_mission"),
        state: row.get("state"),
        current_command: row.get("current_command"),
        current_command_status: row.get("current_command_status"),
        last_seen_at,
        software_version: row.get("software_version"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn command_from_row(row: sqlx::postgres::PgRow) -> CommandResponse {
    CommandResponse {
        command_id: row.get("command_id"),
        robot_id: row.get("robot_id"),
        command_type: row.get("command_type"),
        payload: row.get("payload"),
        status: row.get("status"),
        created_at: row.get("created_at"),
        expires_at: row.get("expires_at"),
        acknowledged_at: row.get("acknowledged_at"),
        completed_at: row.get("completed_at"),
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use robot_fleet_common::{status::robot_status_from_last_seen, types::CommandResultMessage};
    use serde_json::json;
    use uuid::Uuid;

    use super::{command_state_projection, CommandStateProjection};

    #[test]
    fn robot_status_is_derived_from_last_seen_age() {
        let now = Utc::now();

        assert_eq!(robot_status_from_last_seen(now, None), "offline");
        assert_eq!(robot_status_from_last_seen(now, Some(now)), "online");
        assert_eq!(
            robot_status_from_last_seen(now, Some(now - chrono::Duration::seconds(5))),
            "online"
        );
        assert_eq!(
            robot_status_from_last_seen(now, Some(now - chrono::Duration::seconds(6))),
            "stale"
        );
        assert_eq!(
            robot_status_from_last_seen(now, Some(now - chrono::Duration::seconds(15))),
            "stale"
        );
        assert_eq!(
            robot_status_from_last_seen(now, Some(now - chrono::Duration::seconds(16))),
            "offline"
        );
    }

    #[test]
    fn command_state_projection_is_derived_from_command_result_payload() {
        let message = CommandResultMessage {
            command_id: Uuid::new_v4(),
            robot_id: "robot-01".into(),
            status: "running".into(),
            event_type: "command_running".into(),
            payload: json!({
                "command_type": "move",
                "payload": {
                    "target_position_x": 10.0,
                    "target_position_y": 5.0
                }
            }),
            occurred_at: Utc::now(),
        };

        match command_state_projection(&message) {
            Some(CommandStateProjection::Move {
                target_position_x,
                target_position_y,
            }) => {
                assert_eq!(target_position_x, Some(10.0));
                assert_eq!(target_position_y, Some(5.0));
            }
            other => panic!("unexpected projection: {other:?}"),
        }
    }
}
