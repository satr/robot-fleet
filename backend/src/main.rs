mod app;
mod db;
mod error;
mod kafka;
mod metrics;
mod mqtt;
mod routes;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    app::run().await
}
