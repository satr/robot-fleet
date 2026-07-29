use std::{net::SocketAddr, sync::Arc};

use axum::{http::StatusCode, routing::get, Router};
use prometheus::{Encoder, TextEncoder};

use crate::metrics::Metrics;

pub(crate) async fn run_metrics_server(metrics: Arc<Metrics>, port: u16) -> anyhow::Result<()> {
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
