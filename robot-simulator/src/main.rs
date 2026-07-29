mod config;
mod metrics;
mod metrics_server;
mod mqtt;
mod persistence;
mod state;

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::{
    config::Config, metrics::Metrics, metrics_server::run_metrics_server, mqtt::run_robot,
    persistence::load_processed_commands, state::RobotState,
};

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
    let state = Arc::new(Mutex::new(RobotState::new(processed_commands)));

    tokio::spawn(run_metrics_server(metrics.clone(), config.metrics_port));
    tokio::spawn(RobotState::update_state(
        state.clone(),
        config.robot_state_interval,
    ));
    run_robot(config, state, metrics).await
}
