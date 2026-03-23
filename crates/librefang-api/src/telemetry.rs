//! OpenTelemetry tracing and Prometheus metrics integration.
//!
//! This module is compiled only when the `telemetry` feature is enabled.
//! It provides:
//! - OpenTelemetry OTLP span export (layered on top of existing `tracing`)
//! - A Prometheus metrics recorder for `/api/metrics`

use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::sync::OnceLock;

static PROMETHEUS_HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

/// Initialize the Prometheus metrics recorder.
///
/// Safe to call multiple times — the recorder is installed only once via
/// `OnceLock` and subsequent calls return a clone of the existing handle.
/// This is important for test environments where multiple tests may build
/// their own app state in parallel within the same process.
pub fn init_prometheus() -> PrometheusHandle {
    PROMETHEUS_HANDLE
        .get_or_init(|| {
            let builder = PrometheusBuilder::new();
            builder
                .install_recorder()
                .expect("failed to install prometheus recorder")
        })
        .clone()
}

/// Get the global Prometheus handle (if initialized).
pub fn prometheus_handle() -> Option<&'static PrometheusHandle> {
    PROMETHEUS_HANDLE.get()
}

/// Initialize OpenTelemetry OTLP tracing export.
///
/// Configures an OTLP gRPC span exporter that sends traces to the given
/// endpoint (e.g. `http://localhost:4317`).  The exporter is installed as a
/// `tracing` layer via `tracing-opentelemetry`, so all existing `tracing`
/// spans and events are automatically forwarded.
///
/// # Errors
///
/// Returns an error if the OTLP exporter or tracer pipeline cannot be
/// initialized (e.g. invalid endpoint, missing Tokio runtime).
pub fn init_otel_tracing(
    endpoint: &str,
    service_name: &str,
    sample_rate: f64,
) -> Result<(), Box<dyn std::error::Error>> {
    use opentelemetry::trace::TracerProvider;
    use opentelemetry_otlp::{SpanExporter, WithExportConfig};
    use opentelemetry_sdk::trace::{Sampler, SdkTracerProvider};
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    // Build the OTLP gRPC span exporter pointing at the configured collector.
    let exporter = SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()?;

    // Choose sampler based on configured rate.
    let sampler = if (sample_rate - 1.0).abs() < f64::EPSILON {
        Sampler::AlwaysOn
    } else if sample_rate <= 0.0 {
        Sampler::AlwaysOff
    } else {
        Sampler::TraceIdRatioBased(sample_rate)
    };

    // Build the tracer provider with a batch span processor.
    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_sampler(sampler)
        .with_resource(
            opentelemetry_sdk::Resource::builder()
                .with_service_name(service_name.to_string())
                .build(),
        )
        .build();

    let tracer = provider.tracer(service_name.to_string());

    // Create the tracing-opentelemetry layer and install it.
    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);

    tracing_subscriber::registry()
        .with(otel_layer)
        .try_init()
        .map_err(|e| {
            // If a global subscriber is already set (common in tests or when
            // tracing_subscriber::fmt was already initialised), log a warning
            // but don't treat it as fatal.
            tracing::warn!("Could not set global OTLP tracing subscriber (already set?): {e}");
        })
        .ok();

    tracing::info!(
        endpoint = endpoint,
        service_name = service_name,
        sample_rate = sample_rate,
        "OpenTelemetry OTLP tracing initialized"
    );

    Ok(())
}

// NOTE: HTTP metrics recording is handled by `request_logging` in middleware.rs
// which calls `librefang_telemetry::metrics::record_http_request()`.
// A separate middleware layer is not needed here.
