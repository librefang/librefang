//! OpenTelemetry tracing and Prometheus metrics integration.
//!
//! This module is compiled only when the `telemetry` feature is enabled.
//! It provides:
//! - OpenTelemetry OTLP span export (layered on top of existing `tracing`)
//! - A Prometheus metrics recorder with an HTTP handler for `/api/metrics`
//! - An axum middleware that records HTTP request counters and duration histograms

use axum::body::Body;
use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::sync::OnceLock;
use std::time::Instant;

static PROMETHEUS_HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

/// Initialize the Prometheus metrics recorder.
///
/// Must be called once before any metrics are recorded.
/// Returns the handle for rendering metrics output.
pub fn init_prometheus() -> PrometheusHandle {
    let builder = PrometheusBuilder::new();
    builder
        .install_recorder()
        .expect("failed to install prometheus recorder")
}

/// Get the global Prometheus handle (if initialized).
pub fn prometheus_handle() -> Option<&'static PrometheusHandle> {
    PROMETHEUS_HANDLE.get()
}

/// Initialize OpenTelemetry tracing (OTLP export).
///
/// Currently a stub - OTLP export requires additional configuration.
/// Returns Ok(()) when OTLP is properly configured.
pub fn init_otel_tracing(_endpoint: &str) -> Result<(), Box<dyn std::error::Error>> {
    tracing::info!("OTLP tracing initialization skipped (feature not fully configured)");
    Ok(())
}

/// Axum middleware that records HTTP request metrics via the `metrics` crate.
///
/// Recorded metrics:
/// - `librefang_http_requests_total` — counter with labels `method`, `path`, `status`
/// - `librefang_http_request_duration_seconds` — histogram with labels `method`, `path`
pub async fn http_metrics_middleware(request: Request<Body>, next: Next) -> Response {
    let method = request.method().to_string();
    let path = request.uri().path().to_string();

    let start = Instant::now();
    let response = next.run(request).await;
    let duration = start.elapsed().as_secs_f64();

    let status = response.status().as_u16().to_string();

    metrics::counter!(
        "librefang_http_requests_total",
        "method" => method.clone(),
        "path" => path.clone(),
        "status" => status,
    )
    .increment(1);

    metrics::histogram!(
        "librefang_http_request_duration_seconds",
        "method" => method,
        "path" => path,
    )
    .record(duration);

    response
}
