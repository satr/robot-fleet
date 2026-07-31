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
    pub completed_at: Option<DateTime<Utc>>,
}

impl ProcessedCommandRecord {
    pub(crate) fn new(
        command: &RobotCommandMessage,
        status: ProcessedCommandStatus,
        updated_at: DateTime<Utc>,
    ) -> Self {
        Self {
            command_id: command.command_id,
            command_type: command.command_type.clone(),
            payload: command.payload.clone(),
            status,
            updated_at,
            expires_at: command.expires_at,
            acknowledged_at: None,
            completed_at: None,
        }
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
            ProcessedCommandStatus::Completed
            | ProcessedCommandStatus::Failed
            | ProcessedCommandStatus::Expired
            | ProcessedCommandStatus::Stopped => {
                record.completed_at = Some(updated_at);
            }
            ProcessedCommandStatus::Received | ProcessedCommandStatus::Running => {}
        }
        record
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
            completed_at: Some(Utc::now()),
        }
    }
}

pub(crate) async fn load_processed_commands(
    path: &Path,
) -> anyhow::Result<HashMap<Uuid, ProcessedCommandRecord>> {
    match tokio::fs::read_to_string(path).await {
        Ok(contents) => {
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

pub(crate) async fn persist_processed_command(
    path: &Path,
    record: &ProcessedCommandRecord,
) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?;
    file.write_all(serde_json::to_string(record)?.as_bytes()).await?;
    file.write_all(b"\n").await?;
    file.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use robot_fleet_common::types::RobotCommandMessage;
    use serde_json::json;
    use uuid::Uuid;

    use super::{
        load_processed_commands, persist_processed_command, ProcessedCommandRecord,
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
        let record = ProcessedCommandRecord::new(
            &command,
            ProcessedCommandStatus::Acknowledged,
            Utc::now(),
        );

        persist_processed_command(&path, &record)
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
}
