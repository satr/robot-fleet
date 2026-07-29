use std::{path::PathBuf, time::Duration};

use anyhow::Context;
use robot_fleet_common::config::env_or;

#[derive(Clone)]
pub(crate) struct Config {
    pub(crate) robot_id: String,
    pub(crate) robot_name: String,
    pub(crate) mqtt_client_id: String,
    pub(crate) mqtt_url: String,
    pub(crate) telemetry_interval: Duration,
    pub(crate) metrics_port: u16,
    pub(crate) processed_commands_path: PathBuf,
}

impl Config {
    pub(crate) fn from_env() -> anyhow::Result<Self> {
        let robot_id = env_or("ROBOT_ID", "robot-local");
        let mqtt_client_id = std::env::var("MQTT_CLIENT_ID")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| format!("{robot_id}-{}", std::process::id()));
        Ok(Self {
            robot_id,
            robot_name: env_or("ROBOT_NAME", "Local Robot"),
            mqtt_client_id,
            mqtt_url: env_or("MQTT_URL", "mqtt://localhost:1883"),
            telemetry_interval: Duration::from_secs(
                env_or("TELEMETRY_INTERVAL_SECONDS", "5")
                    .parse()
                    .context("TELEMETRY_INTERVAL_SECONDS must be an integer")?,
            ),
            metrics_port: env_or("METRICS_PORT", "9100")
                .parse()
                .context("METRICS_PORT must be a port")?,
            processed_commands_path: env_or(
                "PROCESSED_COMMANDS_PATH",
                "/state/processed_commands.txt",
            )
            .into(),
        })
    }
}
