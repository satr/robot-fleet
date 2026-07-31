use std::{collections::HashMap, path::Path};

use chrono::{DateTime, Utc};
use robot_fleet_common::types::RobotCommandMessage;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProcessedCommandStatus {
    Received,
    Acknowledged,
    Running,
    Resumed,
    Cancelled,
    Completed,
    Failed,
    Expired,
    Stopped,
}

impl ProcessedCommandStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Received => "received",
            Self::Acknowledged => "acknowledged",
            Self::Running => "running",
            Self::Resumed => "resumed",
            Self::Cancelled => "cancelled",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Expired => "expired",
            Self::Stopped => "stopped",
        }
    }

    pub(crate) fn duplicate_response(self) -> Self {
        match self {
            Self::Received | Self::Acknowledged => Self::Acknowledged,
            Self::Running => Self::Running,
            Self::Resumed => Self::Resumed,
            Self::Cancelled => Self::Cancelled,
            Self::Completed => Self::Completed,
            Self::Failed => Self::Failed,
            Self::Expired => Self::Expired,
            Self::Stopped => Self::Stopped,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ProcessedCommandRecord {
    pub command_id: Uuid,
    pub command_type: String,
    pub payload: Value,
    pub status: ProcessedCommandStatus,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acknowledged_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stopped_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resumed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancelled_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_at: Option<DateTime<Utc>>,
}

impl ProcessedCommandRecord {
    pub(crate) fn new(
        command: &RobotCommandMessage,
        status: ProcessedCommandStatus,
        updated_at: DateTime<Utc>,
    ) -> Self {
        let record = Self {
            command_id: command.command_id,
            command_type: command.command_type.clone(),
            payload: command.payload.clone(),
            status,
            updated_at,
            expires_at: command.expires_at,
            acknowledged_at: None,
            stopped_at: None,
            resumed_at: None,
            cancelled_at: None,
            completed_at: None,
            failed_at: None,
        };
        record.with_status(status, updated_at)
    }

    pub(crate) fn with_status(
        &self,
        status: ProcessedCommandStatus,
        updated_at: DateTime<Utc>,
    ) -> Self {
        let mut record = self.clone();
        record.status = status;
        record.updated_at = updated_at;
        match status {
            ProcessedCommandStatus::Acknowledged => {
                record.acknowledged_at = Some(updated_at);
            }
            ProcessedCommandStatus::Stopped => {
                record.stopped_at = Some(updated_at);
            }
            ProcessedCommandStatus::Resumed => {
                record.resumed_at = Some(updated_at);
            }
            ProcessedCommandStatus::Cancelled => {
                record.cancelled_at = Some(updated_at);
                record.completed_at = Some(updated_at);
            }
            ProcessedCommandStatus::Completed => {
                record.completed_at = Some(updated_at);
            }
            ProcessedCommandStatus::Failed => {
                record.failed_at = Some(updated_at);
                record.completed_at = Some(updated_at);
            }
            ProcessedCommandStatus::Expired => {
                record.completed_at = Some(updated_at);
            }
            ProcessedCommandStatus::Received | ProcessedCommandStatus::Running => {}
        }
        record
    }

    pub(crate) fn with_command_and_status(
        &self,
        command: &RobotCommandMessage,
        status: ProcessedCommandStatus,
        updated_at: DateTime<Utc>,
    ) -> Self {
        let mut record = self.with_status(status, updated_at);
        record.command_type = command.command_type.clone();
        record.payload = command.payload.clone();
        record.expires_at = command.expires_at;
        record
    }

    pub(crate) fn matches_command(&self, command: &RobotCommandMessage) -> bool {
        normalize_command_type(&self.command_type) == normalize_command_type(&command.command_type)
            && self.payload == command.payload
            && self.expires_at == command.expires_at
    }

    fn legacy(command_id: Uuid) -> Self {
        Self {
            command_id,
            command_type: "unknown".into(),
            payload: Value::Null,
            status: ProcessedCommandStatus::Completed,
            updated_at: Utc::now(),
            expires_at: None,
            acknowledged_at: None,
            stopped_at: None,
            resumed_at: None,
            cancelled_at: None,
            completed_at: Some(Utc::now()),
            failed_at: None,
        }
    }
}

fn normalize_command_type(command_type: &str) -> String {
    command_type
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '-'], "_")
}

pub(crate) async fn load_processed_commands(
    path: &Path,
) -> anyhow::Result<HashMap<Uuid, ProcessedCommandRecord>> {
    match tokio::fs::read_to_string(path).await {
        Ok(contents) => {
            if contents.trim().is_empty() {
                return Ok(HashMap::new());
            }

            if let Ok(processed_commands) =
                serde_json::from_str::<HashMap<Uuid, ProcessedCommandRecord>>(&contents)
            {
                return Ok(processed_commands);
            }

            let mut processed_commands = HashMap::new();
            for (index, line) in contents.lines().enumerate() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }

                let record = serde_json::from_str::<ProcessedCommandRecord>(line)
                    .or_else(|_| Uuid::parse_str(line).map(ProcessedCommandRecord::legacy))
                    .map_err(|err| {
                        anyhow::anyhow!(
                            "failed to parse processed command record on line {}: {}",
                            index + 1,
                            err
                        )
                    })?;
                processed_commands.insert(record.command_id, record);
            }
            Ok(processed_commands)
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
        Err(err) => Err(err.into()),
    }
}

pub(crate) async fn persist_processed_commands(
    path: &Path,
    records: &HashMap<Uuid, ProcessedCommandRecord>,
) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let tmp_path = path.with_extension("jsonl.tmp");
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&tmp_path)
        .await?;
    file.write_all(serde_json::to_string_pretty(records)?.as_bytes())
        .await?;
    file.flush().await?;
    drop(file);
    tokio::fs::rename(&tmp_path, path).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::Utc;
    use robot_fleet_common::types::RobotCommandMessage;
    use serde_json::json;
    use uuid::Uuid;

    use super::{
        load_processed_commands, persist_processed_commands, ProcessedCommandRecord,
        ProcessedCommandStatus,
    };

    fn sample_command(command_id: Uuid) -> RobotCommandMessage {
        RobotCommandMessage {
            command_id,
            robot_id: "robot-01".into(),
            command_type: "move".into(),
            payload: json!({ "target_position_x": 1.0, "target_position_y": 2.0 }),
            expires_at: None,
        }
    }

    #[tokio::test]
    async fn processed_command_records_are_persisted_and_loaded() {
        let command_id = Uuid::new_v4();
        let path = std::env::temp_dir().join(format!(
            "robot-simulator-processed-commands-{command_id}.jsonl"
        ));
        let command = sample_command(command_id);
        let record =
            ProcessedCommandRecord::new(&command, ProcessedCommandStatus::Acknowledged, Utc::now());

        let mut records = HashMap::new();
        records.insert(command_id, record);

        persist_processed_commands(&path, &records)
            .await
            .expect("persist command record");

        let processed = load_processed_commands(&path)
            .await
            .expect("load processed command records");
        let loaded = processed
            .get(&command_id)
            .expect("persisted command record");
        assert_eq!(loaded.command_type, "move");
        assert_eq!(loaded.status, ProcessedCommandStatus::Acknowledged);
        assert!(loaded.acknowledged_at.is_some());

        tokio::fs::remove_file(path)
            .await
            .expect("remove processed commands test file");
    }

    #[tokio::test]
    async fn legacy_processed_command_ids_are_still_loaded() {
        let command_id = Uuid::new_v4();
        let path = std::env::temp_dir().join(format!(
            "robot-simulator-processed-commands-legacy-{command_id}.jsonl"
        ));

        tokio::fs::write(&path, format!("{command_id}\n"))
            .await
            .expect("write legacy processed command id");

        let processed = load_processed_commands(&path)
            .await
            .expect("load legacy processed command ids");
        let loaded = processed
            .get(&command_id)
            .expect("legacy processed command record");
        assert_eq!(loaded.status, ProcessedCommandStatus::Completed);

        tokio::fs::remove_file(path)
            .await
            .expect("remove processed commands test file");
    }

    #[tokio::test]
    async fn processed_commands_are_serialized_as_keyed_json() {
        let command_id = Uuid::new_v4();
        let path = std::env::temp_dir().join(format!(
            "robot-simulator-processed-commands-json-{command_id}.jsonl"
        ));
        let command = sample_command(command_id);
        let record =
            ProcessedCommandRecord::new(&command, ProcessedCommandStatus::Running, Utc::now());
        let expected_record = record.clone();
        let mut records = HashMap::new();
        records.insert(command_id, record.clone());

        persist_processed_commands(&path, &records)
            .await
            .expect("persist keyed commands");

        let contents = tokio::fs::read_to_string(&path)
            .await
            .expect("read persisted command file");
        let parsed: HashMap<Uuid, ProcessedCommandRecord> =
            serde_json::from_str(&contents).expect("parse keyed json");
        assert_eq!(parsed.get(&command_id), Some(&expected_record));
        assert!(contents.trim_start().starts_with('{'));

        tokio::fs::remove_file(path)
            .await
            .expect("remove processed commands test file");
    }

    #[test]
    fn record_updates_command_details_when_arguments_change() {
        let command_id = Uuid::new_v4();
        let original = sample_command(command_id);
        let mut updated = sample_command(command_id);
        updated.command_type = "stop".into();
        updated.payload = json!({ "stop": true });

        let record =
            ProcessedCommandRecord::new(&original, ProcessedCommandStatus::Received, Utc::now());
        assert!(!record.matches_command(&updated));

        let cancelled =
            record.with_command_and_status(&updated, ProcessedCommandStatus::Cancelled, Utc::now());
        assert_eq!(cancelled.command_type, "stop");
        assert_eq!(cancelled.payload, json!({ "stop": true }));
        assert_eq!(cancelled.status, ProcessedCommandStatus::Cancelled);
        assert!(cancelled.cancelled_at.is_some());
    }

    #[test]
    fn record_tracks_resume_status() {
        let command_id = Uuid::new_v4();
        let command = sample_command(command_id);
        let record =
            ProcessedCommandRecord::new(&command, ProcessedCommandStatus::Stopped, Utc::now());
        let resumed = record.with_status(ProcessedCommandStatus::Resumed, Utc::now());
        assert_eq!(resumed.status, ProcessedCommandStatus::Resumed);
        assert!(resumed.resumed_at.is_some());
    }
}
