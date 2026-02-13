use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use prometheus_client::encoding::text::encode;
use prometheus_client::registry::Registry;

pub struct HealthState {
    pub registry: Arc<Registry>,
    pub ready: Arc<AtomicBool>,
}

pub type SharedHealthState = Arc<HealthState>;

pub async fn healthz() -> &'static str {
    "ok"
}

pub async fn readyz(State(state): State<SharedHealthState>) -> impl IntoResponse {
    if state.ready.load(Ordering::Relaxed) {
        (StatusCode::OK, "ok")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "not ready")
    }
}

pub async fn metrics_handler(State(state): State<SharedHealthState>) -> impl IntoResponse {
    let mut buffer = String::new();
    if let Err(e) = encode(&mut buffer, &state.registry) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to encode metrics: {e}"),
        )
            .into_response();
    }
    (
        [(
            header::CONTENT_TYPE,
            "application/openmetrics-text; version=1.0.0; charset=utf-8",
        )],
        buffer,
    )
        .into_response()
}
