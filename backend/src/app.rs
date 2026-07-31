use std::{net::SocketAddr, sync::Arc};

use anyhow::Context;
use robot_fleet_common::config::env_or;
use robot_fleet_common::types::RobotStreamMessage;
use rumqttc::AsyncClient;
use sqlx::PgPool;
use tokio::sync::broadcast;
use tracing::{info, warn};

use crate::{db, metrics::Metrics, mqtt, routes};

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) pool: PgPool,
    pub(crate) mqtt: AsyncClient,
    pub(crate) metrics: Arc<Metrics>,
    pub(crate) robot_events: broadcast::Sender<RobotStreamMessage>,
}

pub(crate) async fn run() -> anyhow::Result<()> {
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
    let http_port: u16 = env_or("HTTP_PORT", "8089")
        .parse()
        .context("HTTP_PORT must be a port")?;

    let pool = db::connect_postgres(&database_url).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;

    let metrics = Arc::new(Metrics::new()?);
    let (mqtt_client, eventloop) = mqtt::connect_mqtt(&mqtt_url, "robot-fleet-backend").await?;
    let (robot_events, _) = broadcast::channel(100);
    let state = AppState {
        pool,
        mqtt: mqtt_client,
        metrics,
        robot_events,
    };

    tokio::spawn(mqtt::run_mqtt_ingestion(state.clone(), eventloop));
    tokio::spawn(mqtt::run_robot_status_broadcast(state.clone()));
    tokio::spawn(mqtt::run_command_expiry(state.clone()));

    let app = routes::router(state);
    let addr = SocketAddr::from(([0, 0, 0, 0], http_port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(%addr, "backend listening");
    axum::serve(listener, app).await?;
    Ok(())
}

pub(crate) async fn broadcast_robot_update(state: &AppState, robot_id: &str) {
    match db::get_robot_view(&state.pool, robot_id).await {
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
