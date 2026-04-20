//! OpenTelemetry tracing and Prometheus metrics integration.
//!
//! This module is compiled only when the `telemetry` feature is enabled.
//! It provides:
//! - OpenTelemetry OTLP span export (layered on top of existing `tracing`)
//! - A Prometheus metrics recorder for `/api/metrics`

use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::sync::OnceLock;
use tracing_subscriber::{reload, Registry};

static PROMETHEUS_HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

/// Type-erased layer that can be swapped into the OTel reload slot.
pub type OtelBoxedLayer = Box<dyn tracing_subscriber::Layer<Registry> + Send + Sync + 'static>;

/// Handle used by `init_otel_tracing` to swap the real OTel layer into the
/// pre-registered reload slot. Set once, at CLI tracing init time.
static OTEL_RELOAD_HANDLE: OnceLock<reload::Handle<Option<OtelBoxedLayer>, Registry>> =
    OnceLock::new();

/// Install a no-op reload slot for the OTel layer in the tracing subscriber.
///
/// Must be called at most once, and the returned layer must be added to the
/// global `tracing_subscriber::registry()` **before** `.init()`. Later,
/// `init_otel_tracing` swaps a real OTel layer into this slot via the stored
/// reload handle.
///
/// Why this dance: `init_otel_tracing` needs the Tokio runtime to build the
/// batch span exporter, but by the time we reach `run_daemon` the global
/// tracing dispatcher is already installed, so a late `registry().try_init()`
/// silently fails. Registering a reload slot up front lets us defer the real
/// OTel layer creation until the runtime exists without losing the ability
/// to install it.
pub fn install_otel_reload_layer() -> reload::Layer<Option<OtelBoxedLayer>, Registry> {
    let (layer, handle) = reload::Layer::new(None);
    // Silently ignore a second call — tests may exercise init paths twice.
    let _ = OTEL_RELOAD_HANDLE.set(handle);
    layer
}

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

    // Build the tracing-opentelemetry layer and swap it into the reload slot
    // registered at CLI tracing init time. A fresh `registry().try_init()`
    // would silently fail because the global dispatcher is already set.
    let otel_layer: OtelBoxedLayer = Box::new(tracing_opentelemetry::layer().with_tracer(tracer));

    match OTEL_RELOAD_HANDLE.get() {
        Some(handle) => {
            handle
                .modify(|slot| *slot = Some(otel_layer))
                .map_err(|e| format!("failed to install OTel layer via reload handle: {e}"))?;
            tracing::info!(
                endpoint = endpoint,
                service_name = service_name,
                sample_rate = sample_rate,
                "OpenTelemetry OTLP tracing initialized"
            );
        }
        None => {
            tracing::warn!(
                "OTel reload slot not registered; OTLP tracing will be inactive. \
                 The CLI must call `install_otel_reload_layer()` during tracing init."
            );
        }
    }

    Ok(())
}

// NOTE: HTTP metrics recording is handled by `request_logging` in middleware.rs
// which calls `librefang_telemetry::metrics::record_http_request()`.
// A separate middleware layer is not needed here.
