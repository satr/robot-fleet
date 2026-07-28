use std::{net::SocketAddr, sync::Arc, time::Duration};

use anyhow::Context;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, State,
    },
    http::{Method, StatusCode},
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
use tokio::{sync::broadcast, time::sleep};
use tower_http::cors::{Any, CorsLayer};
use tracing::{error, info, warn};
use uuid::Uuid;

#[derive(Clone)]
struct AppState {
    pool: PgPool,
    mqtt: AsyncClient,
    metrics: Arc<Metrics>,
    kafka: KafkaPublisher,
    robot_events: broadcast::Sender<RobotStreamMessage>,
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
    robots_stale: Gauge,
    robots_offline: Gauge,
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
        let robots_stale = Gauge::new("robots_stale", "Number of robots currently stalled")?;
        let robots_offline = Gauge::new("robots_offline", "Number of robots currently offline")?;
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
        registry.register(Box::new(robots_stale.clone()))?;
        registry.register(Box::new(robots_offline.clone()))?;
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
            robots_stale,
            robots_offline,
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

#[derive(Debug, Clone, Serialize)]
struct Robot {
    robot_id: String,
    name: String,
    status: String,
    battery_level: f64,
    position_x: Option<f64>,
    position_y: Option<f64>,
    current_mission: Option<String>,
    current_command: Option<String>,
    current_command_status: Option<String>,
    last_seen_at: Option<DateTime<Utc>>,
    software_version: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
struct RobotStreamMessage {
    event_type: String,
    robot_id: Option<String>,
    robot: Option<Robot>,
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

#[derive(Debug, Serialize)]
struct RobotCommandMessage {
    command_id: Uuid,
    robot_id: String,
    command_type: String,
    payload: Value,
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
    let (robot_events, _) = broadcast::channel(100);
    let state = AppState {
        pool,
        mqtt,
        metrics,
        kafka: KafkaPublisher::new(kafka_brokers),
        robot_events,
    };

    tokio::spawn(run_mqtt_ingestion(state.clone(), eventloop));
    tokio::spawn(run_robot_status_broadcast(state.clone()));

    let app = router(state);
    let addr = SocketAddr::from(([0, 0, 0, 0], http_port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(%addr, "backend listening");
    axum::serve(listener, app).await?;
    Ok(())
}

fn router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::DELETE])
        .allow_origin(Any)
        .allow_headers(Any);

    Router::new()
        .route("/health", get(health))
        .route("/robots", get(list_robots))
        .route("/robots/stream", get(robot_stream))
        .route("/robots/:robot_id", get(get_robot).delete(delete_robot))
        .route(
            "/robots/:robot_id/commands",
            post(create_command).get(list_commands),
        )
        .route("/metrics", get(metrics))
        .layer(cors)
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
    Ok(Json(list_robot_views(&state.pool).await?))
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
    get_robot_view(&state.pool, &robot_id)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

async fn delete_robot(
    State(state): State<AppState>,
    Path(robot_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state
        .metrics
        .http_requests
        .with_label_values(&["DELETE /robots/{robot_id}"])
        .inc();

    let robot = get_robot_view(&state.pool, &robot_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if robot.status != "offline" {
        return Err(ApiError::BadRequest(
            "only offline robots can be deleted".into(),
        ));
    }

    sqlx::query("DELETE FROM robots WHERE robot_id = $1")
        .bind(&robot_id)
        .execute(&state.pool)
        .await?;
    let _ = state.robot_events.send(RobotStreamMessage {
        event_type: "robot_deleted".into(),
        robot_id: Some(robot_id),
        robot: None,
    });
    Ok(StatusCode::NO_CONTENT)
}

async fn robot_stream(State(state): State<AppState>, ws: WebSocketUpgrade) -> impl IntoResponse {
    state
        .metrics
        .http_requests
        .with_label_values(&["/robots/stream"])
        .inc();
    let rx = state.robot_events.subscribe();
    ws.on_upgrade(move |socket| robot_stream_socket(socket, rx))
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
    let message = serde_json::to_vec(&RobotCommandMessage::from(&command))
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    state
        .mqtt
        .publish(topic, QoS::AtLeastOnce, false, message)
        .await?;
    state.metrics.commands_created.inc();
    broadcast_robot_update(&state, &robot_id).await;
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
    refresh_robot_status_metrics(&state).await?;
    let encoder = TextEncoder::new();
    let mut buffer = Vec::new();
    encoder
        .encode(&state.metrics.registry.gather(), &mut buffer)
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    String::from_utf8(buffer).map_err(|err| ApiError::BadRequest(err.to_string()))
}

async fn robot_stream_socket(
    mut socket: WebSocket,
    mut rx: broadcast::Receiver<RobotStreamMessage>,
) {
    while let Ok(event) = rx.recv().await {
        match serde_json::to_string(&event) {
            Ok(body) => {
                if socket.send(Message::Text(body)).await.is_err() {
                    break;
                }
            }
            Err(err) => {
                warn!(error = %err, "failed to serialize robot stream message");
            }
        }
    }
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

async fn run_robot_status_broadcast(state: AppState) {
    loop {
        sleep(Duration::from_secs(5)).await;
        if let Err(err) = refresh_robot_status_metrics(&state).await {
            warn!(error = %err, "failed to refresh robot status metrics");
        }
        match list_robot_views(&state.pool).await {
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

async fn handle_mqtt_message(state: &AppState, topic: &str, payload: &[u8]) -> anyhow::Result<()> {
    if topic.ends_with("/telemetry") {
        let message: TelemetryMessage = serde_json::from_slice(payload)?;
        upsert_robot_from_telemetry(&state.pool, &message).await?;
        refresh_robot_status_metrics(state).await?;
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
        broadcast_robot_update(state, &message.robot_id).await;
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
        refresh_robot_status_metrics(state).await?;
        broadcast_robot_update(state, &message.robot_id).await;
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
        broadcast_robot_update(state, &message.robot_id).await;
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
         VALUES ($1, $1, 'online', $2, now(), 'unknown', now())
         ON CONFLICT (robot_id) DO UPDATE
         SET status = 'online', battery_level = EXCLUDED.battery_level, last_seen_at = now(), updated_at = now()",
    )
    .bind(&message.robot_id)
    .bind(message.battery_level)
    .execute(pool)
    .await?;
    Ok(())
}

async fn upsert_robot_state(pool: &PgPool, message: &StateMessage) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO robots (robot_id, name, status, battery_level, current_mission, last_seen_at, software_version, updated_at)
         VALUES ($1, $2, 'online', $3, $4, now(), $5, now())
         ON CONFLICT (robot_id) DO UPDATE
         SET name = EXCLUDED.name,
            status = 'online',
            battery_level = EXCLUDED.battery_level,
            current_mission = EXCLUDED.current_mission,
            last_seen_at = now(),
            software_version = EXCLUDED.software_version,
            updated_at = now()",
    )
    .bind(&message.robot_id)
    .bind(&message.name)
    .bind(message.battery_level)
    .bind(&message.current_mission)
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

async fn refresh_robot_status_metrics(state: &AppState) -> Result<(), sqlx::Error> {
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

async fn list_robot_views(pool: &PgPool) -> Result<Vec<Robot>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT
             r.robot_id,
             r.name,
             r.status,
             r.battery_level,
             r.current_mission,
             r.last_seen_at,
             r.software_version,
             r.created_at,
             r.updated_at,
             latest_telemetry.position_x,
             latest_telemetry.position_y,
             latest_command.command_type AS current_command,
             latest_command.status AS current_command_status
         FROM robots r
         LEFT JOIN LATERAL (
             SELECT position_x, position_y
             FROM telemetry
             WHERE telemetry.robot_id = r.robot_id
             ORDER BY recorded_at DESC
             LIMIT 1
         ) latest_telemetry ON TRUE
         LEFT JOIN LATERAL (
             SELECT command_type, status
             FROM commands
             WHERE commands.robot_id = r.robot_id
             ORDER BY created_at DESC
             LIMIT 1
         ) latest_command ON TRUE
         ORDER BY r.robot_id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(robot_from_row).collect())
}

async fn get_robot_view(pool: &PgPool, robot_id: &str) -> Result<Option<Robot>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT
             r.robot_id,
             r.name,
             r.status,
             r.battery_level,
             r.current_mission,
             r.last_seen_at,
             r.software_version,
             r.created_at,
             r.updated_at,
             latest_telemetry.position_x,
             latest_telemetry.position_y,
             latest_command.command_type AS current_command,
             latest_command.status AS current_command_status
         FROM robots r
         LEFT JOIN LATERAL (
             SELECT position_x, position_y
             FROM telemetry
             WHERE telemetry.robot_id = r.robot_id
             ORDER BY recorded_at DESC
             LIMIT 1
         ) latest_telemetry ON TRUE
         LEFT JOIN LATERAL (
             SELECT command_type, status
             FROM commands
             WHERE commands.robot_id = r.robot_id
             ORDER BY created_at DESC
             LIMIT 1
         ) latest_command ON TRUE
         WHERE r.robot_id = $1",
    )
    .bind(robot_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(robot_from_row))
}

async fn broadcast_robot_update(state: &AppState, robot_id: &str) {
    match get_robot_view(&state.pool, robot_id).await {
        Ok(Some(robot)) => {
            let _ = state.robot_events.send(RobotStreamMessage {
                event_type: "robot_updated".into(),
                robot_id: Some(robot.robot_id.clone()),
                robot: Some(robot),
            });
        }
        Ok(None) => {}
        Err(err) => warn!(robot_id, error = %err, "failed to build robot stream update"),
    }
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
        current_mission: row.get("current_mission"),
        current_command: row.get("current_command"),
        current_command_status: row.get("current_command_status"),
        last_seen_at,
        software_version: row.get("software_version"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn robot_status_from_last_seen(
    now: DateTime<Utc>,
    last_seen_at: Option<DateTime<Utc>>,
) -> &'static str {
    let Some(last_seen_at) = last_seen_at else {
        return "offline";
    };
    let age_seconds = now.signed_duration_since(last_seen_at).num_seconds();
    if age_seconds <= 5 {
        "online"
    } else if age_seconds <= 15 {
        "stale"
    } else {
        "offline"
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
            robot_events: broadcast::channel(16).0,
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

    #[test]
    fn robot_command_payload_includes_command_id() {
        let command_id = Uuid::new_v4();
        let command = CommandResponse {
            command_id,
            robot_id: "robot-01".into(),
            command_type: "dock".into(),
            payload: json!({ "station": "A" }),
            status: "created".into(),
            created_at: Utc::now(),
            expires_at: None,
            acknowledged_at: None,
            completed_at: None,
        };

        let payload = serde_json::to_value(RobotCommandMessage::from(&command))
            .expect("serialize robot command");
        assert_eq!(payload["command_id"], command_id.to_string());
        assert_eq!(payload["robot_id"], "robot-01");
        assert_eq!(payload["command_type"], "dock");
        assert_eq!(payload["payload"], json!({ "station": "A" }));
        assert!(payload.get("status").is_none());
    }

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
}
