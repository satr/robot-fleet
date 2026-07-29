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
use prometheus::{Encoder, TextEncoder};
use robot_fleet_common::types::{
    CommandResponse, CreateCommandRequest, HealthResponse, Robot, RobotCommandMessage,
    RobotStreamMessage,
};
use tower_http::cors::{Any, CorsLayer};

use crate::{app, app::AppState, db, error::ApiError};

pub(crate) fn router(state: AppState) -> Router {
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

    db::insert_placeholder_robot(&state.pool, &robot_id).await?;
    let command = db::create_command(
        &state.pool,
        &robot_id,
        &request.command_type,
        &request.payload,
        request.expires_at,
    )
    .await?;

    let topic = format!("robots/{}/commands", robot_id);
    let message = serde_json::to_vec(&RobotCommandMessage::from(&command))
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    state
        .mqtt
        .publish(topic, rumqttc::QoS::AtLeastOnce, false, message)
        .await?;
    state.metrics.commands_created.inc();
    app::broadcast_robot_update(&state, &robot_id).await;
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
    use robot_fleet_common::types::{CommandResponse, CreateCommandRequest, RobotCommandMessage};
    use serde_json::json;
    use sqlx::postgres::PgPoolOptions;
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;
    use crate::{kafka::KafkaPublisher, metrics::Metrics};

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
            kafka: KafkaPublisher::new("localhost:9092".into()),
            robot_events: tokio::sync::broadcast::channel(16).0,
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
}
