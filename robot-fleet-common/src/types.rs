use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct Robot {
    pub robot_id: String,
    pub name: String,
    pub status: String,
    pub battery_level: f64,
    pub position_x: Option<f64>,
    pub position_y: Option<f64>,
    pub set_velocity: Option<f64>,
    pub velocity: Option<f64>,
    pub direction_degrees: Option<f64>,
    pub stop: bool,
    pub target_position_x: Option<f64>,
    pub target_position_y: Option<f64>,
    pub current_mission: Option<String>,
    pub state: String,
    pub current_command: Option<String>,
    pub current_command_status: Option<String>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub software_version: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RobotStreamMessage {
    pub event_type: String,
    pub robot_id: Option<String>,
    pub robot: Option<Robot>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct StateMessage {
    pub robot_id: String,
    pub name: String,
    pub status: String,
    pub battery_level: f64,
    #[serde(default)]
    pub position_x: f64,
    #[serde(default)]
    pub position_y: f64,
    pub set_velocity: f64,
    pub velocity: f64,
    pub direction_degrees: f64,
    pub stop: bool,
    pub target_position_x: Option<f64>,
    pub target_position_y: Option<f64>,
    pub current_mission: Option<String>,
    #[serde(default = "default_robot_state")]
    pub state: String,
    pub software_version: String,
    pub recorded_at: DateTime<Utc>,
}

fn default_robot_state() -> String {
    "idle".into()
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TelemetryMessage {
    pub robot_id: String,
    pub recorded_at: DateTime<Utc>,
    pub battery_level: f64,
    pub temperature: f64,
    pub position_x: f64,
    pub position_y: f64,
    #[serde(alias = "speed_cm_s", default)]
    pub velocity_cm_s: f64,
    pub direction_degrees: f64,
    pub payload: Value,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CommandResultMessage {
    pub event_id: Uuid,
    pub command_id: Uuid,
    pub robot_id: String,
    pub status: String,
    pub event_type: String,
    pub payload: Value,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RobotSensorEventMessage {
    pub event_id: Uuid,
    pub robot_id: String,
    pub event_type: String,
    pub priority: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_id: Option<Uuid>,
    #[serde(default)]
    pub payload: Value,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RobotCommandMessage {
    pub command_id: Uuid,
    pub robot_id: String,
    pub command_type: String,
    #[serde(default)]
    pub payload: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateCommandRequest {
    pub command_type: String,
    #[serde(default)]
    pub payload: Value,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct CommandResponse {
    pub command_id: Uuid,
    pub robot_id: String,
    pub command_type: String,
    pub payload: Value,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub acknowledged_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
}

impl From<&CommandResponse> for RobotCommandMessage {
    fn from(command: &CommandResponse) -> Self {
        Self {
            command_id: command.command_id,
            robot_id: command.robot_id.clone(),
            command_type: command.command_type.clone(),
            payload: command.payload.clone(),
            expires_at: command.expires_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{StateMessage, TelemetryMessage};
    use chrono::Utc;
    use serde_json::json;

    #[test]
    fn state_message_defaults_missing_coordinates() {
        let message: StateMessage = serde_json::from_value(json!({
            "robot_id": "robot-01",
            "name": "Loader One",
            "status": "online",
            "battery_level": 87.5,
            "set_velocity": 1.5,
            "velocity": 1.0,
            "direction_degrees": 90.0,
            "stop": false,
            "current_mission": "move",
            "software_version": "0.1.0",
            "recorded_at": Utc::now()
        }))
        .expect("state message");

        assert_eq!(message.position_x, 0.0);
        assert_eq!(message.position_y, 0.0);
        assert_eq!(message.state, "idle");
    }

    #[test]
    fn telemetry_message_accepts_legacy_velocity_field() {
        let message: TelemetryMessage = serde_json::from_value(json!({
            "robot_id": "robot-01",
            "recorded_at": Utc::now(),
            "battery_level": 87.5,
            "temperature": 22.0,
            "position_x": 10.0,
            "position_y": 5.0,
            "speed_cm_s": 1.25,
            "direction_degrees": 90.0,
            "payload": {}
        }))
        .expect("telemetry message");

        assert_eq!(message.velocity_cm_s, 1.25);
    }
}
