use std::{collections::HashSet, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use anyhow::Context;
use axum::{http::StatusCode, routing::get, Router};
use chrono::{DateTime, Utc};
use prometheus::{Encoder, Gauge, IntCounter, Registry, TextEncoder};
use rand::Rng;
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::{sync::Mutex, time::sleep};
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Clone)]
struct Config {
    robot_id: String,
    robot_name: String,
    mqtt_url: String,
    telemetry_interval: Duration,
    metrics_port: u16,
    processed_commands_path: PathBuf,
}

#[derive(Clone)]
struct Metrics {
    registry: Registry,
    mqtt_connection_status: Gauge,
    telemetry_sent: IntCounter,
    commands_processed: IntCounter,
}

impl Metrics {
    fn new() -> anyhow::Result<Self> {
        let registry = Registry::new();
        let mqtt_connection_status =
            Gauge::new("mqtt_connection_status", "Simulator MQTT connection status")?;
        let telemetry_sent =
            IntCounter::new("robot_telemetry_sent_total", "Telemetry messages published")?;
        let commands_processed = IntCounter::new(
            "robot_commands_processed_total",
            "Unique commands processed",
        )?;
        registry.register(Box::new(mqtt_connection_status.clone()))?;
        registry.register(Box::new(telemetry_sent.clone()))?;
        registry.register(Box::new(commands_processed.clone()))?;
        Ok(Self {
            registry,
            mqtt_connection_status,
            telemetry_sent,
            commands_processed,
        })
    }
}

#[derive(Debug)]
struct RobotState {
    battery_level: f64,
    position_x: f64,
    position_y: f64,
    online: bool,
    current_mission: Option<String>,
    software_version: String,
    processed_commands: HashSet<Uuid>,
}

#[derive(Debug, Deserialize)]
struct CommandMessage {
    command_id: Uuid,
    robot_id: String,
    command_type: String,
    #[serde(default)]
    payload: Value,
}

#[derive(Debug, Serialize)]
struct StateMessage {
    robot_id: String,
    name: String,
    status: &'static str,
    battery_level: f64,
    current_mission: Option<String>,
    software_version: String,
    recorded_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct TelemetryMessage {
    robot_id: String,
    recorded_at: DateTime<Utc>,
    battery_level: f64,
    temperature: f64,
    position_x: f64,
    position_y: f64,
    payload: Value,
}

#[derive(Debug, Serialize)]
struct CommandResultMessage<'a> {
    command_id: Uuid,
    robot_id: &'a str,
    status: &'a str,
    event_type: &'a str,
    payload: Value,
    occurred_at: DateTime<Utc>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();

    let config = Config::from_env()?;
    let metrics = Arc::new(Metrics::new()?);
    let processed_commands = load_processed_commands(&config.processed_commands_path).await?;
    let state = Arc::new(Mutex::new(RobotState {
        battery_level: 100.0,
        position_x: 0.0,
        position_y: 0.0,
        online: true,
        current_mission: None,
        software_version: "0.1.0".into(),
        processed_commands,
    }));

    tokio::spawn(run_metrics_server(metrics.clone(), config.metrics_port));
    run_robot(config, state, metrics).await
}

impl Config {
    fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            robot_id: env_or("ROBOT_ID", "robot-local"),
            robot_name: env_or("ROBOT_NAME", "Local Robot"),
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
                "/data/processed_commands.txt",
            )
            .into(),
        })
    }
}

async fn run_robot(
    config: Config,
    state: Arc<Mutex<RobotState>>,
    metrics: Arc<Metrics>,
) -> anyhow::Result<()> {
    loop {
        let (client, mut eventloop) = connect_mqtt(&config).await?;
        client
            .subscribe(
                format!("robots/{}/commands", config.robot_id),
                QoS::AtLeastOnce,
            )
            .await?;
        publish_state(&client, &config, &state).await?;

        let telemetry_client = client.clone();
        let telemetry_config = config.clone();
        let telemetry_state = state.clone();
        let telemetry_metrics = metrics.clone();
        tokio::spawn(async move {
            loop {
                if let Err(err) = publish_telemetry(
                    &telemetry_client,
                    &telemetry_config,
                    &telemetry_state,
                    &telemetry_metrics,
                )
                .await
                {
                    warn!(robot_id = telemetry_config.robot_id, error = %err, "telemetry publish failed");
                }
                sleep(telemetry_config.telemetry_interval).await;
            }
        });

        loop {
            match eventloop.poll().await {
                Ok(Event::Incoming(Incoming::Publish(publish))) => {
                    metrics.mqtt_connection_status.set(1.0);
                    if let Err(err) =
                        handle_command(&client, &config, &state, &metrics, &publish.payload).await
                    {
                        warn!(robot_id = config.robot_id, error = %err, "command handling failed");
                    }
                }
                Ok(_) => metrics.mqtt_connection_status.set(1.0),
                Err(err) => {
                    metrics.mqtt_connection_status.set(0.0);
                    warn!(robot_id = config.robot_id, error = %err, "MQTT disconnected; reconnecting");
                    sleep(Duration::from_secs(2)).await;
                    break;
                }
            }
        }
    }
}

async fn publish_telemetry(
    client: &AsyncClient,
    config: &Config,
    state: &Arc<Mutex<RobotState>>,
    metrics: &Metrics,
) -> anyhow::Result<()> {
    let mut guard = state.lock().await;
    guard.battery_level = (guard.battery_level - 0.2).max(0.0);
    guard.position_x += rand::thread_rng().gen_range(-0.5..=0.5);
    guard.position_y += rand::thread_rng().gen_range(-0.5..=0.5);
    guard.online = rand::thread_rng().gen_bool(0.98);

    let now = Utc::now();
    let telemetry = TelemetryMessage {
        robot_id: config.robot_id.clone(),
        recorded_at: now,
        battery_level: guard.battery_level,
        temperature: rand::thread_rng().gen_range(20.0..=45.0),
        position_x: guard.position_x,
        position_y: guard.position_y,
        payload: json!({ "online": guard.online }),
    };
    let status = if guard.online { "online" } else { "offline" };
    let state_message = StateMessage {
        robot_id: config.robot_id.clone(),
        name: config.robot_name.clone(),
        status,
        battery_level: guard.battery_level,
        current_mission: guard.current_mission.clone(),
        software_version: guard.software_version.clone(),
        recorded_at: now,
    };
    drop(guard);

    client
        .publish(
            format!("robots/{}/telemetry", config.robot_id),
            QoS::AtMostOnce,
            false,
            serde_json::to_vec(&telemetry)?,
        )
        .await?;
    client
        .publish(
            format!("robots/{}/state", config.robot_id),
            QoS::AtLeastOnce,
            false,
            serde_json::to_vec(&state_message)?,
        )
        .await?;
    metrics.telemetry_sent.inc();
    Ok(())
}

async fn publish_state(
    client: &AsyncClient,
    config: &Config,
    state: &Arc<Mutex<RobotState>>,
) -> anyhow::Result<()> {
    let guard = state.lock().await;
    let message = StateMessage {
        robot_id: config.robot_id.clone(),
        name: config.robot_name.clone(),
        status: if guard.online { "online" } else { "offline" },
        battery_level: guard.battery_level,
        current_mission: guard.current_mission.clone(),
        software_version: guard.software_version.clone(),
        recorded_at: Utc::now(),
    };
    client
        .publish(
            format!("robots/{}/state", config.robot_id),
            QoS::AtLeastOnce,
            false,
            serde_json::to_vec(&message)?,
        )
        .await?;
    Ok(())
}

async fn handle_command(
    client: &AsyncClient,
    config: &Config,
    state: &Arc<Mutex<RobotState>>,
    metrics: &Metrics,
    payload: &[u8],
) -> anyhow::Result<()> {
    let command: CommandMessage = serde_json::from_slice(payload)?;
    if command.robot_id != config.robot_id {
        warn!(
            robot_id = config.robot_id,
            command_robot_id = command.robot_id,
            command_id = %command.command_id,
            "ignoring command for a different robot"
        );
        return Ok(());
    }

    {
        let mut guard = state.lock().await;
        if !guard.processed_commands.insert(command.command_id) {
            info!(robot_id = config.robot_id, command_id = %command.command_id, "duplicate command ignored");
            return Ok(());
        }
        persist_processed_command(&config.processed_commands_path, command.command_id).await?;
        guard.current_mission = Some(command.command_type.clone());
    }

    metrics.commands_processed.inc();
    publish_command_result(
        client,
        config,
        &command,
        "acknowledged",
        "command_acknowledged",
    )
    .await?;
    publish_command_result(client, config, &command, "running", "command_running").await?;
    sleep(Duration::from_secs(2)).await;
    {
        let mut guard = state.lock().await;
        guard.current_mission = None;
    }
    publish_command_result(client, config, &command, "completed", "command_completed").await?;
    Ok(())
}

async fn publish_command_result(
    client: &AsyncClient,
    config: &Config,
    command: &CommandMessage,
    status: &'static str,
    event_type: &'static str,
) -> anyhow::Result<()> {
    let message = CommandResultMessage {
        command_id: command.command_id,
        robot_id: &config.robot_id,
        status,
        event_type,
        payload: json!({
            "command_type": command.command_type,
            "payload": command.payload,
        }),
        occurred_at: Utc::now(),
    };
    client
        .publish(
            format!("robots/{}/command-results", config.robot_id),
            QoS::AtLeastOnce,
            false,
            serde_json::to_vec(&message)?,
        )
        .await?;
    Ok(())
}

async fn run_metrics_server(metrics: Arc<Metrics>, port: u16) -> anyhow::Result<()> {
    let app = Router::new().route(
        "/metrics",
        get(move || {
            let metrics = metrics.clone();
            async move {
                let encoder = TextEncoder::new();
                let mut buffer = Vec::new();
                if let Err(err) = encoder.encode(&metrics.registry.gather(), &mut buffer) {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("failed to encode metrics: {err}"),
                    );
                }
                match String::from_utf8(buffer) {
                    Ok(body) => (StatusCode::OK, body),
                    Err(err) => (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("metrics output was not utf8: {err}"),
                    ),
                }
            }
        }),
    );
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn connect_mqtt(config: &Config) -> anyhow::Result<(AsyncClient, rumqttc::EventLoop)> {
    let (host, port) = parse_mqtt_url(&config.mqtt_url)?;
    let mut options = MqttOptions::new(&config.robot_id, host, port);
    options.set_keep_alive(Duration::from_secs(10));
    Ok(AsyncClient::new(options, 10))
}

fn parse_mqtt_url(url: &str) -> anyhow::Result<(String, u16)> {
    let value = url.strip_prefix("mqtt://").unwrap_or(url);
    let (host, port) = value
        .rsplit_once(':')
        .context("MQTT_URL must look like mqtt://host:port")?;
    Ok((host.to_string(), port.parse()?))
}

async fn load_processed_commands(path: &PathBuf) -> anyhow::Result<HashSet<Uuid>> {
    match tokio::fs::read_to_string(path).await {
        Ok(contents) => Ok(contents
            .lines()
            .filter_map(|line| Uuid::parse_str(line).ok())
            .collect()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(HashSet::new()),
        Err(err) => Err(err.into()),
    }
}

async fn persist_processed_command(path: &PathBuf, command_id: Uuid) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    use tokio::io::AsyncWriteExt;
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?;
    file.write_all(format!("{command_id}\n").as_bytes()).await?;
    Ok(())
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn duplicate_command_is_detected() {
        let command_id = Uuid::new_v4();
        let mut state = RobotState {
            battery_level: 100.0,
            position_x: 0.0,
            position_y: 0.0,
            online: true,
            current_mission: None,
            software_version: "test".into(),
            processed_commands: HashSet::new(),
        };

        assert!(state.processed_commands.insert(command_id));
        assert!(!state.processed_commands.insert(command_id));
    }
}
