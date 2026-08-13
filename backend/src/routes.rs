use std::future::Future;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, State,
    },
    http::{Method, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Duration, Utc};
use futures_util::{SinkExt, StreamExt};
use prometheus::{Encoder, TextEncoder};
use robot_fleet_common::types::{
    CommandResponse, CreateCommandRequest, HealthResponse, Robot, RobotCommandMessage,
    RobotStreamMessage,
};
use serde_json::Value;
use tokio_tungstenite::{connect_async, tungstenite::Message as TungsteniteMessage};
use tower_http::cors::{Any, CorsLayer};
use tracing::warn;
use uuid::Uuid;

use crate::{app, app::AppState, db, error::ApiError};

const DEFAULT_COMMAND_TTL: Duration = Duration::minutes(5);

pub(crate) fn router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::DELETE])
        .allow_origin(Any)
        .allow_headers(Any);

    Router::new()
        .route("/health", get(liveness))
        .route("/health/live", get(liveness))
        .route("/health/ready", get(readiness))
        .route("/robots", get(list_robots))
        .route("/robots/stream", get(robot_stream))
        .route("/robots/:robot_id", get(get_robot).delete(delete_robot))
        .route(
            "/robots/:robot_id/commands",
            post(create_command).get(list_commands),
        )
        .route(
            "/robots/:robot_id/simulated-events",
            post(create_simulated_event),
        )
        .route("/internal/alertmanager/webhook", post(alertmanager_webhook))
        .route("/metrics", get(metrics))
        .route("/mqtt", get(mqtt_websocket))
        .route("/", get(mqtt_websocket))
        .layer(cors)
        .with_state(state)
}

async fn mqtt_websocket(upgrade: WebSocketUpgrade) -> impl IntoResponse {
    upgrade.on_upgrade(proxy_mqtt_websocket)
}

async fn proxy_mqtt_websocket(client: WebSocket) {
    let broker_url = std::env::var("MQTT_WS_URL").unwrap_or_else(|_| "ws://127.0.0.1:9001".into());
    let Ok((broker, _)) = connect_async(broker_url).await else {
        return;
    };
    let (mut broker_sink, mut broker_stream) = broker.split();
    let (mut client_sink, mut client_stream) = client.split();
    let client_to_broker = async {
        while let Some(Ok(message)) = client_stream.next().await {
            let converted = match message {
                Message::Binary(bytes) => TungsteniteMessage::Binary(bytes.to_vec()),
                Message::Text(text) => TungsteniteMessage::Text(text.to_string()),
                Message::Ping(bytes) => TungsteniteMessage::Ping(bytes.to_vec()),
                Message::Pong(bytes) => TungsteniteMessage::Pong(bytes.to_vec()),
                Message::Close(_) => break,
            };
            if broker_sink.send(converted).await.is_err() {
                break;
            }
        }
    };
    let broker_to_client = async {
        while let Some(Ok(message)) = broker_stream.next().await {
            let converted = match message {
                TungsteniteMessage::Binary(bytes) => Message::Binary(bytes.into()),
                TungsteniteMessage::Text(text) => Message::Text(text.into()),
                TungsteniteMessage::Ping(bytes) => Message::Ping(bytes.into()),
                TungsteniteMessage::Pong(bytes) => Message::Pong(bytes.into()),
                TungsteniteMessage::Close(_) => break,
                TungsteniteMessage::Frame(_) => continue,
            };
            if client_sink.send(converted).await.is_err() {
                break;
            }
        }
    };
    tokio::select! {
        _ = client_to_broker => {}
        _ = broker_to_client => {}
    }
}

async fn liveness(State(state): State<AppState>) -> Json<HealthResponse> {
    state
        .metrics
        .http_requests
        .with_label_values(&["/health/live"])
        .inc();
    Json(HealthResponse { status: "ok" })
}

async fn readiness(State(state): State<AppState>) -> (StatusCode, Json<HealthResponse>) {
    state
        .metrics
        .http_requests
        .with_label_values(&["/health/ready"])
        .inc();

    let mqtt_ready = state.metrics.mqtt_connection_status.get() > 0.0;
    let result = readiness_check(mqtt_ready, || async {
        sqlx::query("SELECT 1")
            .execute(&state.pool)
            .await
            .map(|_| ())
    })
    .await;

    match result {
        Ok(()) => (StatusCode::OK, Json(HealthResponse { status: "ok" })),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(HealthResponse {
                status: "unavailable",
            }),
        ),
    }
}

async fn readiness_check<F, Fut>(mqtt_ready: bool, database_check: F) -> Result<(), ReadinessError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<(), sqlx::Error>>,
{
    if !mqtt_ready {
        return Err(ReadinessError::MqttUnavailable);
    }

    database_check()
        .await
        .map_err(|_| ReadinessError::PostgresUnavailable)
}

#[derive(Debug, PartialEq, Eq)]
enum ReadinessError {
    MqttUnavailable,
    PostgresUnavailable,
}

async fn list_robots(State(state): State<AppState>) -> Result<Json<Vec<Robot>>, ApiError> {
    state
        .metrics
        .http_requests
        .with_label_values(&["/robots"])
        .inc();
    Ok(Json(db::list_robot_views(&state.pool).await?))
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
    db::get_robot_view(&state.pool, &robot_id)
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

    let robot = db::get_robot_view(&state.pool, &robot_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if robot.status != "offline" {
        return Err(ApiError::BadRequest(
            "only offline robots can be deleted".into(),
        ));
    }

    db::delete_robot(&state.pool, &robot_id).await?;
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
    validate_command_request(&request)?;

    let command = create_command_for_robot(
        &state,
        &robot_id,
        &request.command_type,
        &request.payload,
        request.expires_at,
        format!("robots/{robot_id}/commands"),
    )
    .await?;
    Ok(Json(command))
}

async fn create_simulated_event(
    State(state): State<AppState>,
    Path(robot_id): Path<String>,
    Json(request): Json<CreateCommandRequest>,
) -> Result<StatusCode, ApiError> {
    state
        .metrics
        .http_requests
        .with_label_values(&["POST /robots/{robot_id}/simulated-events"])
        .inc();
    if request.command_type.trim().is_empty() {
        return Err(ApiError::BadRequest("command_type is required".into()));
    }
    validate_simulated_event_request(&request)?;

    let _ = ensure_robot_accepts_commands(db::get_robot_view(&state.pool, &robot_id).await?)?;
    publish_simulated_event(&state, &robot_id, &request).await?;
    Ok(StatusCode::ACCEPTED)
}

fn normalize_command_type(command_type: &str) -> String {
    command_type
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '-'], "_")
}

fn is_simulated_event_command(command_type: &str) -> bool {
    matches!(
        normalize_command_type(command_type).as_str(),
        "extreme_temperature" | "robot_stack"
    )
}

async fn create_command_for_robot(
    state: &AppState,
    robot_id: &str,
    command_type: &str,
    payload: &Value,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
    topic: String,
) -> Result<CommandResponse, ApiError> {
    let _ = ensure_robot_accepts_commands(db::get_robot_view(&state.pool, robot_id).await?)?;

    let command = db::create_command(
        &state.pool,
        robot_id,
        command_type,
        payload,
        Some(command_expires_at(expires_at)),
    )
    .await?;
    state.metrics.commands_created.inc();

    let command =
        match publish_robot_command(state, topic, &RobotCommandMessage::from(&command)).await {
            Ok(()) => command,
            Err(err) => {
                warn!(
                    command_id = %command.command_id,
                    robot_id = %command.robot_id,
                    error = %err,
                    "failed to publish command"
                );
                db::mark_command_publish_failed(&state.pool, command.command_id).await?
            }
        };

    app::broadcast_robot_update(state, robot_id).await;
    Ok(command)
}

fn simulated_event_message(robot_id: &str, request: &CreateCommandRequest) -> RobotCommandMessage {
    RobotCommandMessage {
        command_id: Uuid::new_v4(),
        robot_id: robot_id.into(),
        command_type: request.command_type.clone(),
        payload: request.payload.clone(),
        expires_at: request.expires_at,
    }
}

async fn publish_simulated_event(
    state: &AppState,
    robot_id: &str,
    request: &CreateCommandRequest,
) -> Result<(), ApiError> {
    let message = simulated_event_message(robot_id, request);
    publish_robot_command(
        state,
        format!("robots/{robot_id}/simulated-events"),
        &message,
    )
    .await?;
    Ok(())
}

fn ensure_robot_accepts_commands(robot: Option<Robot>) -> Result<Robot, ApiError> {
    let Some(robot) = robot else {
        return Err(ApiError::NotFound);
    };

    if robot.status == "offline" {
        return Err(ApiError::BadRequest(
            "robot must be online before commands can be created".into(),
        ));
    }

    Ok(robot)
}

fn command_expires_at(expires_at: Option<DateTime<Utc>>) -> DateTime<Utc> {
    expires_at.unwrap_or_else(|| Utc::now() + DEFAULT_COMMAND_TTL)
}

async fn publish_robot_command(
    state: &AppState,
    topic: String,
    message: &RobotCommandMessage,
) -> anyhow::Result<()> {
    let payload = serde_json::to_vec(message)?;
    state
        .mqtt
        .publish(topic, rumqttc::QoS::AtLeastOnce, false, payload)
        .await?;
    Ok(())
}

fn validate_command_request(request: &CreateCommandRequest) -> Result<(), ApiError> {
    if is_simulated_event_command(&request.command_type) {
        return Err(ApiError::BadRequest(
            "simulated event commands must be sent to /robots/{robot_id}/simulated-events".into(),
        ));
    }

    let command_type = normalize_command_type(&request.command_type);
    match command_type.as_str() {
        "move" => {
            let has_xy = request
                .payload
                .get("target_position_x")
                .and_then(|value| value.as_f64())
                .is_some()
                && request
                    .payload
                    .get("target_position_y")
                    .and_then(|value| value.as_f64())
                    .is_some();
            let has_position_object = request
                .payload
                .get("target_position")
                .and_then(|value| Some((value.get("x")?.as_f64()?, value.get("y")?.as_f64()?)))
                .is_some();
            let has_position_array = request
                .payload
                .get("target_position")
                .and_then(|value| value.as_array())
                .is_some_and(|position| {
                    position.len() == 2
                        && position[0].as_f64().is_some()
                        && position[1].as_f64().is_some()
                });

            if !(has_xy || has_position_object || has_position_array) {
                return Err(ApiError::BadRequest(
                    "move command requires numeric target_position_x/target_position_y or target_position".into(),
                ));
            }
        }
        "set_velocity" => {
            let set_velocity = request
                .payload
                .get("set_velocity")
                .and_then(|value| value.as_f64())
                .unwrap_or(1.0);
            if !(0.01..=10.0).contains(&set_velocity) {
                return Err(ApiError::BadRequest(
                    "set_velocity must be between 0.01 and 10.0".into(),
                ));
            }
        }
        "stop" => {
            let stop = request.payload.as_bool().or_else(|| {
                request
                    .payload
                    .get("stop")
                    .and_then(|value| value.as_bool())
            });
            if stop.is_none() {
                return Err(ApiError::BadRequest(
                    "stop command requires boolean stop".into(),
                ));
            }
        }
        "extreme_temperature" | "robot_stack" => {
            return Err(ApiError::BadRequest(
                "simulated event commands must be sent to /robots/{robot_id}/simulated-events"
                    .into(),
            ));
        }
        _ => {}
    }

    Ok(())
}

fn validate_simulated_event_request(request: &CreateCommandRequest) -> Result<(), ApiError> {
    if is_simulated_event_command(&request.command_type) {
        return Ok(());
    }

    let command_type = normalize_command_type(&request.command_type);
    match command_type.as_str() {
        "move" | "set_velocity" | "stop" => Err(ApiError::BadRequest(
            "move, set_velocity and stop commands must be sent to /robots/{robot_id}/commands"
                .into(),
        )),
        _ => Err(ApiError::BadRequest("unsupported command_type".into())),
    }
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
    Ok(Json(db::list_commands(&state.pool, &robot_id).await?))
}

async fn metrics(State(state): State<AppState>) -> Result<String, ApiError> {
    state
        .metrics
        .http_requests
        .with_label_values(&["/metrics"])
        .inc();
    db::refresh_robot_status_metrics(&state).await?;
    db::refresh_robot_motion_metrics(&state).await?;
    let encoder = TextEncoder::new();
    let mut buffer = Vec::new();
    encoder
        .encode(&state.metrics.registry.gather(), &mut buffer)
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    String::from_utf8(buffer).map_err(|err| ApiError::BadRequest(err.to_string()))
}

async fn alertmanager_webhook(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> StatusCode {
    state
        .metrics
        .http_requests
        .with_label_values(&["POST /internal/alertmanager/webhook"])
        .inc();

    let alert_count = payload
        .get("alerts")
        .and_then(Value::as_array)
        .map_or(0, |alerts| alerts.len());
    tracing::info!(alert_count, "received alertmanager webhook");
    StatusCode::OK
}

async fn robot_stream_socket(
    mut socket: WebSocket,
    mut rx: tokio::sync::broadcast::Receiver<RobotStreamMessage>,
) {
    while let Ok(event) = rx.recv().await {
        match serde_json::to_string(&event) {
            Ok(body) => {
                if socket.send(Message::Text(body)).await.is_err() {
                    break;
                }
            }
            Err(err) => {
                tracing::warn!(error = %err, "failed to serialize robot stream message");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{body::Body, http::Request, http::StatusCode};
    use robot_fleet_common::types::{
        CommandResponse, CreateCommandRequest, Robot, RobotCommandMessage,
    };
    use serde_json::json;
    use sqlx::postgres::PgPoolOptions;
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;
    use crate::metrics::Metrics;

    fn test_state() -> AppState {
        let metrics = Arc::new(Metrics::new().expect("metrics"));
        let (mqtt, _) =
            rumqttc::AsyncClient::new(rumqttc::MqttOptions::new("test", "localhost", 1883), 1);
        AppState {
            pool: PgPoolOptions::new()
                .connect_lazy("postgres://localhost/test")
                .expect("lazy pool"),
            mqtt,
            metrics,
            robot_events: tokio::sync::broadcast::channel(16).0,
        }
    }

    fn test_robot(status: &str) -> Robot {
        let now = chrono::Utc::now();
        Robot {
            robot_id: "robot-01".into(),
            name: "Robot 01".into(),
            status: status.into(),
            battery_level: 100.0,
            position_x: None,
            position_y: None,
            set_velocity: None,
            velocity: None,
            direction_degrees: None,
            stop: false,
            target_position_x: None,
            target_position_y: None,
            current_mission: None,
            state: "idle".into(),
            current_command: None,
            current_command_status: None,
            last_seen_at: Some(now),
            software_version: "0.1.0".into(),
            created_at: now,
            updated_at: now,
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

    #[tokio::test]
    async fn readiness_endpoint_returns_service_unavailable_when_mqtt_is_down() {
        let response = router(test_state())
            .oneshot(
                Request::builder()
                    .uri("/health/ready")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn readiness_check_short_circuits_before_database_access_when_mqtt_is_down() {
        let database_called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let database_called_clone = database_called.clone();

        let result = readiness_check(false, move || {
            let database_called = database_called_clone.clone();
            async move {
                database_called.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
        })
        .await;

        assert_eq!(result, Err(ReadinessError::MqttUnavailable));
        assert!(!database_called.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn readiness_check_accepts_ready_dependencies() {
        let result = readiness_check(true, || async { Ok(()) }).await;
        assert_eq!(result, Ok(()));
    }

    #[tokio::test]
    async fn alertmanager_webhook_returns_ok() {
        let response = router(test_state())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/internal/alertmanager/webhook")
                    .header("content-type", "application/json")
                    .body(Body::from(json!({ "alerts": [] }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn command_creation_requires_type() {
        let request = CreateCommandRequest {
            command_type: "move".into(),
            payload: json!({ "target_position_x": 10, "target_position_y": 20 }),
            expires_at: None,
        };
        assert_eq!(request.command_type, "move");
        assert!(validate_command_request(&request).is_ok());
    }

    #[test]
    fn set_velocity_command_requires_range() {
        let request = CreateCommandRequest {
            command_type: "set_velocity".into(),
            payload: json!({ "set_velocity": 1.5 }),
            expires_at: None,
        };
        assert!(validate_command_request(&request).is_ok());
    }

    #[test]
    fn stop_command_requires_boolean_flag() {
        let request = CreateCommandRequest {
            command_type: "stop".into(),
            payload: json!({ "stop": true }),
            expires_at: None,
        };
        assert!(validate_command_request(&request).is_ok());
    }

    #[test]
    fn simulated_event_commands_require_the_dedicated_endpoint() {
        let simulated_event_request = CreateCommandRequest {
            command_type: "extreme_temperature".into(),
            payload: json!({}),
            expires_at: None,
        };
        assert!(validate_simulated_event_request(&simulated_event_request).is_ok());
        assert!(validate_command_request(&simulated_event_request).is_err());

        let regular_command_request = CreateCommandRequest {
            command_type: "move".into(),
            payload: json!({ "target_position_x": 10, "target_position_y": 20 }),
            expires_at: None,
        };
        assert!(validate_command_request(&regular_command_request).is_ok());
        assert!(validate_simulated_event_request(&regular_command_request).is_err());
    }

    #[test]
    fn command_target_must_exist_and_be_online() {
        assert!(matches!(
            ensure_robot_accepts_commands(None),
            Err(ApiError::NotFound)
        ));
        assert!(ensure_robot_accepts_commands(Some(test_robot("online"))).is_ok());
        assert!(ensure_robot_accepts_commands(Some(test_robot("stale"))).is_ok());
        assert!(matches!(
            ensure_robot_accepts_commands(Some(test_robot("offline"))),
            Err(ApiError::BadRequest(message))
                if message == "robot must be online before commands can be created"
        ));
    }

    #[test]
    fn command_expiry_defaults_when_missing() {
        let expires_at = command_expires_at(None);
        assert!(expires_at > chrono::Utc::now());
        assert!(expires_at <= chrono::Utc::now() + chrono::Duration::minutes(6));
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
            created_at: chrono::Utc::now(),
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
    fn simulated_event_message_keeps_event_fields_and_generates_id() {
        let request = CreateCommandRequest {
            command_type: "robot_stack".into(),
            payload: json!({ "severity": "high" }),
            expires_at: None,
        };

        let message = simulated_event_message("robot-01", &request);
        assert_eq!(message.robot_id, "robot-01");
        assert_eq!(message.command_type, "robot_stack");
        assert_eq!(message.payload, json!({ "severity": "high" }));
        assert!(message.expires_at.is_none());
        assert!(!message.command_id.is_nil());
    }
}
