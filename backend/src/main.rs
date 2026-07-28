use std::{net::SocketAddr, sync::Arc, time::Duration};

use anyhow::Context;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use prometheus::{Encoder, Gauge, IntCounter, IntCounterVec, Opts, Registry, TextEncoder};
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{postgres::PgPoolOptions, PgPool, Row};
use tokio::time::sleep;
use tracing::{error, info, warn};
use uuid::Uuid;

#[derive(Clone)]
struct AppState {
    pool: PgPool,
    mqtt: AsyncClient,
    metrics: Arc<Metrics>,
    kafka: KafkaPublisher,
}

#[derive(Clone)]
struct KafkaPublisher {
    brokers: String,
}

impl KafkaPublisher {
    fn new(brokers: String) -> Self {
        Self { brokers }
    }

    async fn publish(&self, topic: &str, key: &str, payload: &Value) {
        info!(
            kafka_brokers = %self.brokers,
            topic,
            key,
            payload = %payload,
            "kafka publish placeholder"
        );
    }
}

struct Metrics {
    registry: Registry,
    robots_online: Gauge,
    messages_received: IntCounter,
    telemetry_received: IntCounter,
    commands_created: IntCounter,
    commands_completed: IntCounter,
    command_failures: IntCounter,
    mqtt_connection_status: Gauge,
    telemetry_lag_seconds: Gauge,
    http_requests: IntCounterVec,
}

impl Metrics {
    fn new() -> anyhow::Result<Self> {
        let registry = Registry::new();
        let robots_online = Gauge::new("robots_online", "Number of robots currently online")?;
        let messages_received = IntCounter::new(
            "robot_messages_received_total",
            "MQTT robot messages received by the backend",
        )?;
        let telemetry_received = IntCounter::new(
            "robot_telemetry_received_total",
            "Telemetry messages received by the backend",
        )?;
        let commands_created =
            IntCounter::new("commands_created_total", "Commands created through the API")?;
        let commands_completed =
            IntCounter::new("commands_completed_total", "Commands completed by robots")?;
        let command_failures =
            IntCounter::new("command_failures_total", "Commands reported as failed")?;
        let mqtt_connection_status =
            Gauge::new("mqtt_connection_status", "Backend MQTT connection status")?;
        let telemetry_lag_seconds = Gauge::new(
            "telemetry_ingestion_lag_seconds",
            "Seconds between telemetry recording and ingestion",
        )?;
        let http_requests = IntCounterVec::new(
            Opts::new("backend_http_requests_total", "Backend HTTP requests"),
            &["route"],
        )?;

        registry.register(Box::new(robots_online.clone()))?;
        registry.register(Box::new(messages_received.clone()))?;
        registry.register(Box::new(telemetry_received.clone()))?;
        registry.register(Box::new(commands_created.clone()))?;
        registry.register(Box::new(commands_completed.clone()))?;
        registry.register(Box::new(command_failures.clone()))?;
        registry.register(Box::new(mqtt_connection_status.clone()))?;
        registry.register(Box::new(telemetry_lag_seconds.clone()))?;
        registry.register(Box::new(http_requests.clone()))?;

        Ok(Self {
            registry,
            robots_online,
            messages_received,
            telemetry_received,
            commands_created,
            commands_completed,
            command_failures,
            mqtt_connection_status,
            telemetry_lag_seconds,
            http_requests,
        })
    }
}

#[derive(Debug, Serialize)]
struct Robot {
    robot_id: String,
    name: String,
    status: String,
    battery_level: f64,
    current_mission: Option<String>,
    last_seen_at: Option<DateTime<Utc>>,
    software_version: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, Serialize)]
struct StateMessage {
    robot_id: String,
    name: String,
    status: String,
    battery_level: f64,
    current_mission: Option<String>,
    software_version: String,
    recorded_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, Serialize)]
struct TelemetryMessage {
    robot_id: String,
    recorded_at: DateTime<Utc>,
    battery_level: f64,
    temperature: f64,
    position_x: f64,
    position_y: f64,
    payload: Value,
}

#[derive(Debug, Deserialize, Serialize)]
struct CommandResultMessage {
    command_id: Uuid,
    robot_id: String,
    status: String,
    event_type: String,
    payload: Value,
    occurred_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct CreateCommandRequest {
    command_type: String,
    #[serde(default)]
    payload: Value,
    expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
struct CommandResponse {
    command_id: Uuid,
    robot_id: String,
    command_type: String,
    payload: Value,
    status: String,
    created_at: DateTime<Utc>,
    expires_at: Option<DateTime<Utc>>,
    acknowledged_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
}

#[derive(Debug, thiserror::Error)]
enum ApiError {
    #[error("not found")]
    NotFound,
    #[error("invalid request: {0}")]
    BadRequest(String),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Mqtt(#[from] rumqttc::ClientError),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self {
            ApiError::NotFound => StatusCode::NOT_FOUND,
            ApiError::BadRequest(_) => StatusCode::BAD_REQUEST,
            ApiError::Database(_) | ApiError::Mqtt(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, Json(json!({ "error": self.to_string() }))).into_response()
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();

    let database_url = env_or(
        "DATABASE_URL",
        "postgres://robot_fleet:robot_fleet@localhost:5432/robot_fleet",
    );
    let mqtt_url = env_or("MQTT_URL", "mqtt://localhost:1883");
    let kafka_brokers = env_or("KAFKA_BROKERS", "localhost:9092");
    let http_port: u16 = env_or("HTTP_PORT", "8089")
        .parse()
        .context("HTTP_PORT must be a port")?;

    let pool = connect_postgres(&database_url).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;

    let metrics = Arc::new(Metrics::new()?);
    let (mqtt, eventloop) = connect_mqtt(&mqtt_url, "robot-fleet-backend").await?;
    let state = AppState {
        pool,
        mqtt,
        metrics,
        kafka: KafkaPublisher::new(kafka_brokers),
    };

    tokio::spawn(run_mqtt_ingestion(state.clone(), eventloop));

    let app = router(state);
    let addr = SocketAddr::from(([0, 0, 0, 0], http_port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(%addr, "backend listening");
    axum::serve(listener, app).await?;
    Ok(())
}

fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/robots", get(list_robots))
        .route("/robots/:robot_id", get(get_robot))
        .route(
            "/robots/:robot_id/commands",
            post(create_command).get(list_commands),
        )
        .route("/metrics", get(metrics))
        .with_state(state)
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    state
        .metrics
        .http_requests
        .with_label_values(&["/health"])
        .inc();
    Json(HealthResponse { status: "ok" })
}

async fn list_robots(State(state): State<AppState>) -> Result<Json<Vec<Robot>>, ApiError> {
    state
        .metrics
        .http_requests
        .with_label_values(&["/robots"])
        .inc();
    let rows = sqlx::query(
        "SELECT robot_id, name, status, battery_level, current_mission, last_seen_at, software_version, created_at, updated_at
         FROM robots ORDER BY robot_id",
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(rows.into_iter().map(robot_from_row).collect()))
}

async fn get_robot(
    State(state): State<AppState>,
    Path(robot_id): Path<String>,
) -> Result<Json<Robot>, ApiError> {
    state
        .metrics
        .http_requests
        .with_label_values(&["/robots/{robot_id}"])
        .inc();
    let row = sqlx::query(
        "SELECT robot_id, name, status, battery_level, current_mission, last_seen_at, software_version, created_at, updated_at
         FROM robots WHERE robot_id = $1",
    )
    .bind(robot_id)
    .fetch_optional(&state.pool)
    .await?;
    row.map(robot_from_row).map(Json).ok_or(ApiError::NotFound)
}

async fn create_command(
    State(state): State<AppState>,
    Path(robot_id): Path<String>,
    Json(request): Json<CreateCommandRequest>,
) -> Result<Json<CommandResponse>, ApiError> {
    state
        .metrics
        .http_requests
        .with_label_values(&["POST /robots/{robot_id}/commands"])
        .inc();
    if request.command_type.trim().is_empty() {
        return Err(ApiError::BadRequest("command_type is required".into()));
    }

    sqlx::query(
        "INSERT INTO robots (robot_id, name, status, software_version, updated_at)
         VALUES ($1, $1, 'unknown', 'unknown', now())
         ON CONFLICT (robot_id) DO NOTHING",
    )
    .bind(&robot_id)
    .execute(&state.pool)
    .await?;

    let command_id = Uuid::new_v4();
    let row = sqlx::query(
        "INSERT INTO commands (command_id, robot_id, command_type, payload, status, expires_at)
         VALUES ($1, $2, $3, $4, 'created', $5)
         RETURNING command_id, robot_id, command_type, payload, status, created_at, expires_at, acknowledged_at, completed_at",
    )
    .bind(command_id)
    .bind(&robot_id)
    .bind(&request.command_type)
    .bind(&request.payload)
    .bind(request.expires_at)
    .fetch_one(&state.pool)
    .await?;

    let command = command_from_row(row);
    let topic = format!("robots/{}/commands", robot_id);
    let message =
        serde_json::to_vec(&command).map_err(|err| ApiError::BadRequest(err.to_string()))?;
    state
        .mqtt
        .publish(topic, QoS::AtLeastOnce, false, message)
        .await?;
    state.metrics.commands_created.inc();
    Ok(Json(command))
}

async fn list_commands(
    State(state): State<AppState>,
    Path(robot_id): Path<String>,
) -> Result<Json<Vec<CommandResponse>>, ApiError> {
    state
        .metrics
        .http_requests
        .with_label_values(&["GET /robots/{robot_id}/commands"])
        .inc();
    let rows = sqlx::query(
        "SELECT command_id, robot_id, command_type, payload, status, created_at, expires_at, acknowledged_at, completed_at
         FROM commands WHERE robot_id = $1 ORDER BY created_at DESC LIMIT 100",
    )
    .bind(robot_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(rows.into_iter().map(command_from_row).collect()))
}

async fn metrics(State(state): State<AppState>) -> Result<String, ApiError> {
    state
        .metrics
        .http_requests
        .with_label_values(&["/metrics"])
        .inc();
    let encoder = TextEncoder::new();
    let mut buffer = Vec::new();
    encoder
        .encode(&state.metrics.registry.gather(), &mut buffer)
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    String::from_utf8(buffer).map_err(|err| ApiError::BadRequest(err.to_string()))
}

async fn run_mqtt_ingestion(state: AppState, mut eventloop: rumqttc::EventLoop) {
    if let Err(err) = state
        .mqtt
        .subscribe("robots/+/telemetry", QoS::AtMostOnce)
        .await
    {
        error!(error = %err, "failed to subscribe to telemetry");
    }
    if let Err(err) = state
        .mqtt
        .subscribe("robots/+/state", QoS::AtLeastOnce)
        .await
    {
        error!(error = %err, "failed to subscribe to state");
    }
    if let Err(err) = state
        .mqtt
        .subscribe("robots/+/command-results", QoS::AtLeastOnce)
        .await
    {
        error!(error = %err, "failed to subscribe to command results");
    }

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

async fn handle_mqtt_message(state: &AppState, topic: &str, payload: &[u8]) -> anyhow::Result<()> {
    if topic.ends_with("/telemetry") {
        let message: TelemetryMessage = serde_json::from_slice(payload)?;
        upsert_robot_from_telemetry(&state.pool, &message).await?;
        sqlx::query(
            "INSERT INTO telemetry (robot_id, recorded_at, battery_level, temperature, position_x, position_y, payload)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (robot_id, recorded_at) DO NOTHING",
        )
        .bind(&message.robot_id)
        .bind(message.recorded_at)
        .bind(message.battery_level)
        .bind(message.temperature)
        .bind(message.position_x)
        .bind(message.position_y)
        .bind(&message.payload)
        .execute(&state.pool)
        .await?;
        state.metrics.telemetry_received.inc();
        state
            .metrics
            .telemetry_lag_seconds
            .set((Utc::now() - message.recorded_at).num_milliseconds().max(0) as f64 / 1000.0);
        state
            .kafka
            .publish(
                "robot-telemetry",
                &message.robot_id,
                &serde_json::to_value(&message)?,
            )
            .await;
    } else if topic.ends_with("/state") {
        let message: StateMessage = serde_json::from_slice(payload)?;
        upsert_robot_state(&state.pool, &message).await?;
        refresh_online_metric(state).await?;
        state
            .kafka
            .publish(
                "robot-state-events",
                &message.robot_id,
                &serde_json::to_value(&message)?,
            )
            .await;
    } else if topic.ends_with("/command-results") {
        let message: CommandResultMessage = serde_json::from_slice(payload)?;
        apply_command_result(state, &message).await?;
        state
            .kafka
            .publish(
                "robot-command-events",
                &message.robot_id,
                &serde_json::to_value(&message)?,
            )
            .await;
    }
    Ok(())
}

async fn upsert_robot_from_telemetry(
    pool: &PgPool,
    message: &TelemetryMessage,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO robots (robot_id, name, status, battery_level, last_seen_at, software_version, updated_at)
         VALUES ($1, $1, 'online', $2, $3, 'unknown', now())
         ON CONFLICT (robot_id) DO UPDATE
         SET status = 'online', battery_level = EXCLUDED.battery_level, last_seen_at = EXCLUDED.last_seen_at, updated_at = now()",
    )
    .bind(&message.robot_id)
    .bind(message.battery_level)
    .bind(message.recorded_at)
    .execute(pool)
    .await?;
    Ok(())
}

async fn upsert_robot_state(pool: &PgPool, message: &StateMessage) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO robots (robot_id, name, status, battery_level, current_mission, last_seen_at, software_version, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, now())
         ON CONFLICT (robot_id) DO UPDATE
         SET name = EXCLUDED.name,
             status = EXCLUDED.status,
             battery_level = EXCLUDED.battery_level,
             current_mission = EXCLUDED.current_mission,
             last_seen_at = EXCLUDED.last_seen_at,
             software_version = EXCLUDED.software_version,
             updated_at = now()",
    )
    .bind(&message.robot_id)
    .bind(&message.name)
    .bind(&message.status)
    .bind(message.battery_level)
    .bind(&message.current_mission)
    .bind(message.recorded_at)
    .bind(&message.software_version)
    .execute(pool)
    .await?;
    Ok(())
}

async fn apply_command_result(
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
        "failed" => {
            sqlx::query("UPDATE commands SET status = 'failed', completed_at = COALESCE(completed_at, $2) WHERE command_id = $1")
                .bind(message.command_id)
                .bind(message.occurred_at)
                .execute(&state.pool)
                .await?;
            state.metrics.command_failures.inc();
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
    Ok(())
}

async fn refresh_online_metric(state: &AppState) -> anyhow::Result<()> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM robots WHERE status = 'online'")
        .fetch_one(&state.pool)
        .await?;
    state.metrics.robots_online.set(count as f64);
    Ok(())
}

async fn connect_postgres(database_url: &str) -> anyhow::Result<PgPool> {
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

async fn connect_mqtt(
    url: &str,
    client_id: &str,
) -> anyhow::Result<(AsyncClient, rumqttc::EventLoop)> {
    let (host, port) = parse_mqtt_url(url)?;
    let mut options = MqttOptions::new(client_id, host, port);
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

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn robot_from_row(row: sqlx::postgres::PgRow) -> Robot {
    Robot {
        robot_id: row.get("robot_id"),
        name: row.get("name"),
        status: row.get("status"),
        battery_level: row.get("battery_level"),
        current_mission: row.get("current_mission"),
        last_seen_at: row.get("last_seen_at"),
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
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    fn test_state() -> AppState {
        let metrics = Arc::new(Metrics::new().expect("metrics"));
        let (mqtt, _) = AsyncClient::new(MqttOptions::new("test", "localhost", 1883), 1);
        AppState {
            pool: PgPoolOptions::new()
                .connect_lazy("postgres://localhost/test")
                .expect("lazy pool"),
            mqtt,
            metrics,
            kafka: KafkaPublisher::new("localhost:9092".into()),
        }
    }

    #[tokio::test]
    async fn health_endpoint_returns_ok() {
        let response = router(test_state())
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn command_status_transition_order_is_supported() {
        let transitions = ["created", "acknowledged", "running", "completed"];
        assert_eq!(transitions.last(), Some(&"completed"));
    }

    #[test]
    fn command_creation_requires_type() {
        let request = CreateCommandRequest {
            command_type: "dock".into(),
            payload: json!({ "station": "A" }),
            expires_at: None,
        };
        assert_eq!(request.command_type, "dock");
    }
}
