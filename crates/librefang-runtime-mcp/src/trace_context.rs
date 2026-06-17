//! Outbound W3C traceparent propagation for MCP tool calls; uses `opentelemetry::Context::current()` (not `tracing::Span`) to survive the `reload::Layer` downcast limitation (#6128).

use opentelemetry::global;
use opentelemetry::Context;
use opentelemetry_http::HeaderInjector;

/// MCP `_meta` key for the W3C trace context; value is `{"traceparent": "…"}` so OTel-instrumented servers can extract it the same way they would an HTTP header.
pub(crate) const TRACE_CONTEXT_META_KEY: &str = "io.librefang/trace";

/// Returns the active W3C trace context as an HTTP header map; empty when no recording span is active.
pub(crate) fn current_w3c_trace_headers() -> http::HeaderMap {
    let mut map = http::HeaderMap::new();
    let cx = Context::current();
    global::get_text_map_propagator(|propagator| {
        propagator.inject_context(&cx, &mut HeaderInjector(&mut map));
    });
    map
}

/// Same as `current_w3c_trace_headers` but as `(name, value)` pairs for embedding in MCP `_meta`; empty when no recording span is active.
pub(crate) fn current_w3c_trace_meta() -> Vec<(String, String)> {
    current_w3c_trace_headers()
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|v| (name.as_str().to_string(), v.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::trace::{TraceContextExt, TracerProvider as _};
    use opentelemetry::Context as OtelContext;
    use opentelemetry_sdk::propagation::TraceContextPropagator;
    use opentelemetry_sdk::trace::{Sampler, SdkTracerProvider};
    use tracing::Subscriber;
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::{reload, Registry};

    type BoxedReloadLayer = Box<dyn tracing_subscriber::Layer<Registry> + Send + Sync + 'static>;

    /// Wires OTel behind a `reload::Layer` (production topology) so the `OpenTelemetrySpanExt::context()` downcast failure is exercised rather than masked.
    fn otel_subscriber_behind_reload() -> (impl Subscriber + Send + Sync, SdkTracerProvider) {
        let provider = SdkTracerProvider::builder()
            .with_sampler(Sampler::AlwaysOn)
            .build();
        let tracer = provider.tracer("mcp-trace-context-test");

        let (reload_layer, handle) = reload::Layer::<Option<BoxedReloadLayer>, Registry>::new(None);
        let subscriber = tracing_subscriber::registry().with(reload_layer);
        let otel_layer: BoxedReloadLayer =
            Box::new(tracing_opentelemetry::layer().with_tracer(tracer));
        handle
            .modify(|slot| *slot = Some(otel_layer))
            .expect("reload handle must accept the OTel layer");
        (subscriber, provider)
    }

    #[test]
    fn recording_span_yields_non_all_zero_traceparent() {
        opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());

        let (subscriber, _provider) = otel_subscriber_behind_reload();
        let (meta, header_map, expected_trace_id) =
            tracing::subscriber::with_default(subscriber, || {
                let span = tracing::info_span!("mcp.tool.call");
                let _enter = span.enter();

                let expected_trace_id = OtelContext::current()
                    .span()
                    .span_context()
                    .trace_id()
                    .to_string();

                (
                    current_w3c_trace_meta(),
                    current_w3c_trace_headers(),
                    expected_trace_id,
                )
            });

        let traceparent = meta
            .iter()
            .find(|(k, _)| k == "traceparent")
            .map(|(_, v)| v.clone())
            .expect("traceparent must be present under a recording span");

        // W3C format: "00-<32 hex trace id>-<16 hex span id>-<2 hex flags>".
        let parts: Vec<&str> = traceparent.split('-').collect();
        assert_eq!(
            parts.len(),
            4,
            "traceparent must have 4 parts: {traceparent}"
        );
        assert_eq!(parts[0], "00", "version must be 00");
        assert_eq!(parts[1].len(), 32, "trace id must be 32 hex chars");
        assert_ne!(parts[1], "0".repeat(32), "trace id must not be all-zero");
        assert_eq!(
            parts[1], expected_trace_id,
            "trace id must match active span"
        );
        assert_eq!(parts[3], "01", "sampled flag must be set");

        // The header view carries the same context.
        assert_eq!(
            header_map
                .get("traceparent")
                .and_then(|v| v.to_str().ok())
                .expect("header view must also carry traceparent"),
            traceparent,
        );
    }

    #[test]
    fn no_active_span_yields_empty() {
        assert!(
            current_w3c_trace_meta().is_empty(),
            "no active span → no _meta trace pairs",
        );
        assert!(
            !current_w3c_trace_headers().contains_key("traceparent"),
            "no active span → no traceparent header",
        );
    }

    #[test]
    fn meta_and_header_views_are_consistent() {
        opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());

        let (subscriber, _provider) = otel_subscriber_behind_reload();
        let (meta, header_map) = tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("mcp.tool.call");
            let _enter = span.enter();
            (current_w3c_trace_meta(), current_w3c_trace_headers())
        });

        assert!(
            meta.iter().any(|(k, _)| k == "traceparent"),
            "traceparent present in the _meta view",
        );

        let mut meta_sorted = meta.clone();
        meta_sorted.sort();
        let mut header_sorted: Vec<(String, String)> = header_map
            .iter()
            .map(|(n, v)| (n.as_str().to_string(), v.to_str().unwrap().to_string()))
            .collect();
        header_sorted.sort();
        assert_eq!(
            meta_sorted, header_sorted,
            "_meta and header views must carry identical pairs",
        );
    }
}
