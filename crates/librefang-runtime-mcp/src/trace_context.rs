//! Outbound W3C Trace Context propagation for MCP tool calls (#6128).
//!
//! Mirrors `librefang-llm-drivers`' `drivers::trace_headers::inject_w3c_trace_context`:
//! it sources the active trace from [`opentelemetry::Context::current()`] (NOT
//! `tracing::Span::current().context()`, which silently yields an invalid
//! context behind LibreFang's `tracing_subscriber::reload::Layer` — see the
//! root-cause analysis in `trace_headers.rs`) and injects via the
//! globally-registered text-map propagator.
//!
//! Two transports can set per-request headers (HttpCompat builds the reqwest
//! request per call), so they take the [`http::HeaderMap`] view. The
//! streamable-HTTP / Rmcp transport sets its `custom_headers` once at connect
//! time and rmcp 1.7 exposes no per-request header hook; the SSE transport
//! sends through a shared `sse_send_request` helper that takes no per-request
//! header argument. Both therefore carry the context in the request `_meta`
//! under [`TRACE_CONTEXT_META_KEY`] via the `Vec<(String, String)>` view,
//! alongside (never replacing) the existing `io.librefang/caller` entry.
//!
//! **Unconditional and self-disabling.** Like the LLM-driver helper this is not
//! gated on any config flag — W3C Trace Context is a standard interop
//! primitive. When telemetry is disabled the global propagator is the default
//! `NoopTextMapPropagator` (writes nothing); when a real propagator is
//! registered but there is no recording span the context's `SpanContext` is
//! invalid and `TraceContextPropagator::inject_context` writes nothing. In both
//! cases the returned carrier is empty and the tool call proceeds unchanged.

use opentelemetry::global;
use opentelemetry::Context;
use opentelemetry_http::HeaderInjector;

/// `_meta` key under which the outbound W3C trace context is shipped on the
/// Rmcp (streamable-HTTP) and SSE transports. Reverse-DNS namespaced per the
/// MCP `_meta` convention, matching the sibling caller key `io.librefang/caller`.
/// The value is a JSON object whose keys are the lowercase W3C field names
/// (`traceparent`, and `tracestate` when non-empty), so an OTel-instrumented
/// server can read them back the same way a `HeaderExtractor` would.
pub(crate) const TRACE_CONTEXT_META_KEY: &str = "io.librefang/trace";

/// Inject the current W3C trace context into a fresh [`http::HeaderMap`] and
/// return it. Empty when telemetry is disabled or there is no active recording
/// span (see module docs). Used by the HttpCompat per-request path, which can
/// attach real per-request headers.
pub(crate) fn current_w3c_trace_headers() -> http::HeaderMap {
    let mut map = http::HeaderMap::new();
    let cx = Context::current();
    global::get_text_map_propagator(|propagator| {
        propagator.inject_context(&cx, &mut HeaderInjector(&mut map));
    });
    map
}

/// Same context as [`current_w3c_trace_headers`], rendered as a list of
/// `(name, value)` pairs (e.g. `("traceparent", "00-…")`) for embedding in the
/// MCP request `_meta`. Header names are already lowercase ASCII as emitted by
/// the W3C propagator; values are guaranteed valid ASCII (a `traceparent` is
/// hex + dashes, a `tracestate` is restricted to ASCII per the spec). Empty
/// when telemetry is disabled or there is no active recording span — callers
/// must then add nothing to `_meta`.
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

    /// Reproduce the production wiring: the `OpenTelemetryLayer` installed
    /// *behind a `tracing_subscriber::reload::Layer`*, exactly as
    /// `librefang_api::telemetry` does at runtime. This is the load-bearing
    /// difference — behind the reload slot the `OpenTelemetrySpanExt::context()`
    /// downcast can never succeed, so a test that installed the OTel layer
    /// directly would mask the real bug `current_w3c_trace_*` is built to
    /// avoid (see `trace_headers.rs`).
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

    /// Under a recording span (production reload-layer wiring) the helper must
    /// surface a `traceparent` whose trace id matches the active span — both in
    /// the `_meta` view and the header view.
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

    /// With no active recording span the context is invalid, so the propagator
    /// writes nothing — proving the no-op-when-disabled guarantee. Runs without
    /// entering any span; `Context::current()` is thread-local, so this is
    /// robust even when the recording-span test runs concurrently.
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

    /// The `_meta` view and the header view are derived from one injection, so
    /// they must carry exactly the same key/value pairs — a server reading
    /// either surface sees the same context. (We faithfully pass through
    /// whatever the global propagator emits, e.g. a possibly-empty `tracestate`,
    /// matching the `trace_headers.rs` LLM-egress precedent rather than
    /// second-guessing the propagator.)
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
