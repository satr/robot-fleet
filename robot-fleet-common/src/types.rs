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
    pub velocity_cm_s: Option<f64>,
    pub direction_degrees: Option<f64>,
    pub current_mission: Option<String>,
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
    pub position_x: f64,
    pub position_y: f64,
    pub velocity_cm_s: f64,
    pub direction_degrees: f64,
    pub current_mission: Option<String>,
    pub software_version: String,
    pub recorded_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TelemetryMessage {
    pub robot_id: String,
    pub recorded_at: DateTime<Utc>,
    pub battery_level: f64,
    pub temperature: f64,
    pub position_x: f64,
    pub position_y: f64,
    pub velocity_cm_s: f64,
    pub direction_degrees: f64,
    pub payload: Value,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CommandResultMessage {
    pub command_id: Uuid,
    pub robot_id: String,
    pub status: String,
    pub event_type: String,
    pub payload: Value,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RobotCommandMessage {
    pub command_id: Uuid,
    pub robot_id: String,
    pub command_type: String,
    #[serde(default)]
    pub payload: Value,
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
        }
    }
}
