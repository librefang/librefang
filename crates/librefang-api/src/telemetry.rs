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
use librefang_types::config::TelemetryConfig;
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::sync::OnceLock;
use std::time::Instant;

static PROMETHEUS_HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

/// Initialize the Prometheus metrics recorder.
///
/// Must be called once before any metrics are recorded.
/// Returns the handle for rendering metrics output.
pub fn init_prometheus() -> PrometheusHandle {
    let (recorder, handle) = PrometheusBuilder::new()
        .build()
        .expect("failed to build prometheus recorder");
    metrics::set_global_recorder(recorder).expect("failed to set global metrics recorder");
    PROMETHEUS_HANDLE.set(handle.clone()).ok();
    handle
}

/// Get the global Prometheus handle (if initialized).
pub fn prometheus_handle() -> Option<&'static PrometheusHandle> {
    PROMETHEUS_HANDLE.get()
}

/// Create an OpenTelemetry [`SdkTracerProvider`] configured for OTLP gRPC export.
///
/// The caller is responsible for registering the returned provider with
/// `tracing_opentelemetry::OpenTelemetryLayer` and adding it to the
/// subscriber registry.
pub fn init_otel_tracing(
    config: &TelemetryConfig,
) -> Result<opentelemetry_sdk::trace::SdkTracerProvider, Box<dyn std::error::Error>> {
    use opentelemetry_otlp::SpanExporter;
    use opentelemetry_sdk::trace::{Sampler, SdkTracerProvider};

    let sampler = if (config.sample_rate - 1.0).abs() < f64::EPSILON {
        Sampler::AlwaysOn
    } else if config.sample_rate <= 0.0 {
        Sampler::AlwaysOff
    } else {
        Sampler::TraceIdRatioBased(config.sample_rate)
    };

    let exporter = SpanExporter::builder()
        .with_tonic()
        .with_endpoint(&config.otlp_endpoint)
        .build()?;

    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_sampler(sampler)
        .with_resource(
            opentelemetry_sdk::Resource::builder()
                .with_service_name(config.service_name.clone())
                .build(),
        )
        .build();

    Ok(provider)
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
