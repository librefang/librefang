//! Shared helper for building the outbound `x-librefang-*` trace header map.
//!
//! Used by every driver that emits caller-identity headers so the logic and
//! the associated doc-comment rationale live in exactly one place.

use crate::llm_driver::CompletionRequest;
use tracing::warn;

/// Build the merged custom-header map for an outbound LLM request. Combines
/// the driver-level `extra_headers` (configured per-driver, typically used for
/// testing or IDE auth shims) with the per-request
/// `x-librefang-{agent,session,step}-id` trace headers sourced from
/// [`CompletionRequest`].
///
/// Naming convention — `x-` prefix: deliberately retained despite RFC 6648
/// (June 2012) deprecating the `x-` "experimental" convention for new
/// protocols. Three reasons we are knowingly not following the RFC's
/// recommendation here:
///
/// 1. **Industry de-facto practice.** Every LLM-adjacent provider and
///    proxy LibreFang interoperates with continues to use `x-` for
///    application-namespaced headers — OpenAI's own `x-request-id` /
///    `x-ratelimit-*`, Cloudflare's `x-amz-cf-id`, AWS SigV4's `x-amz-*`,
///    GitHub's `x-github-*`, Stripe's `x-stripe-*`. Picking
///    unprefixed `librefang-*` would make us the odd one out and mean
///    operators run a *non-prefixed* allowlist on their proxies for
///    LibreFang only, which is exactly the integration-tax RFC 6648 was
///    trying to avoid.
/// 2. **Internal precedent.** The MCP-bridge config in `claude_code.rs`
///    has shipped with `X-LibreFang-Agent-Id` since well before this PR.
///    A second namespace would force two allowlist entries (one with `x-`,
///    one without) on every operator who wants to forward both, defeating
///    the "single allowlist string" ergonomic the prefix was chosen for.
/// 3. **RFC 6648 is non-normative.** The RFC is BCP 178 ("Best Current
///    Practice") guidance for *new protocol designers*, not a wire-format
///    requirement; it explicitly allows existing deployments to keep
///    `x-` headers and Section 3 calls out the cost of churning
///    namespaces. The cost-benefit on a feature-gated observability hint
///    is not worth a third-party-allowlist breakage.
///
/// Casing convention: trace headers are emitted as **lowercase-with-dashes**
/// (`x-librefang-agent-id`). HTTP header names are case-insensitive on the
/// wire, but log-grep tooling and JSON-dump callers benefit from a single
/// canonical spelling.
///
/// Precedence: trace headers always **replace** any same-named entries from
/// `extra_headers`. We unify everything in a single `HeaderMap` and use
/// `insert` semantics so the trace IDs win.
///
/// Validation: each value is parsed via [`reqwest::header::HeaderValue::from_str`]
/// before insertion. Values containing `\r`, `\n`, NUL, or other non-visible
/// bytes are rejected with a `warn!` log and **silently skipped** — the
/// underlying request still goes through. Failing the entire LLM call
/// because of an unprintable trace ID would be far worse than dropping the
/// observability hint, since the caller-provided ID is purely a debugging
/// aid for sidecar log correlation.
pub(crate) fn build_trace_header_map(
    extra_headers: &[(String, String)],
    request: &CompletionRequest,
    emit_caller_trace_headers: bool,
) -> reqwest::header::HeaderMap {
    use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

    let mut map = HeaderMap::new();

    // First, replay the operator-provided extras. We use `append` here so
    // that *non-trace* duplicates from the extras list are still preserved
    // (some custom auth shims legitimately rely on multi-value headers).
    for (k, v) in extra_headers {
        match (
            HeaderName::try_from(k.as_str()),
            HeaderValue::from_str(v.as_str()),
        ) {
            (Ok(name), Ok(value)) => {
                map.append(name, value);
            }
            (name_res, value_res) => {
                warn!(
                    invalid_header_value = true,
                    name = %k,
                    name_error = ?name_res.err().map(|e| e.to_string()),
                    value_error = ?value_res.err().map(|e| e.to_string()),
                    "extra header has invalid name or value; skipping",
                );
            }
        }
    }

    // Operator opt-out: when `telemetry.emit_caller_trace_headers = false`
    // in `config.toml`, skip the three `x-librefang-*` insertions wire-side
    // regardless of whether `CompletionRequest`'s caller-id fields are
    // populated. Returning early here (rather than gating per-header) means
    // an operator who legitimately put `x-librefang-agent-id` into their
    // own `extra_headers` for a diagnostic experiment still sees their
    // value go out — the trace-header path is what's gated, not the
    // namespace.
    if !emit_caller_trace_headers {
        inject_w3c_trace_context(&mut map);
        return map;
    }

    // Then layer trace headers on top with `insert` (overwrite semantics):
    // any same-named entry from `extra_headers` is removed before our
    // trace-id value is set, so the wire only carries one canonical value.
    insert_trace_header(
        &mut map,
        "x-librefang-agent-id",
        request.agent_id.as_deref(),
    );
    insert_trace_header(
        &mut map,
        "x-librefang-session-id",
        request.session_id.as_deref(),
    );
    insert_trace_header(&mut map, "x-librefang-step-id", request.step_id.as_deref());

    inject_w3c_trace_context(&mut map);

    map
}

/// Inject the W3C [`traceparent`] (and `tracestate`, if any) header for the
/// currently-active `tracing` span into `map`, using the globally-registered
/// text-map propagator. This stitches the spans of downstream HTTP services
/// (e.g. the `jarvis-llm-proxy` sidecar, which auto-extracts `traceparent`
/// via its FastAPI OTel instrumentation) into the same trace as the
/// LibreFang LLM-call span.
///
/// **Unconditional** — unlike the `x-librefang-*` caller-id headers, this is
/// *not* gated on `telemetry.emit_caller_trace_headers`. W3C Trace Context is
/// a standard interop primitive (not a LibreFang-namespaced diagnostic hint),
/// so it is always emitted. When no OTel layer / propagator is installed
/// (telemetry disabled), `inject_context` is a no-op and the span context is
/// invalid, so no header is written — the request is unaffected.
///
/// [`traceparent`]: https://www.w3.org/TR/trace-context/#traceparent-header
fn inject_w3c_trace_context(map: &mut reqwest::header::HeaderMap) {
    use opentelemetry::global;
    use opentelemetry_http::HeaderInjector;
    use tracing_opentelemetry::OpenTelemetrySpanExt;

    let cx = tracing::Span::current().context();
    global::get_text_map_propagator(|propagator| {
        propagator.inject_context(&cx, &mut HeaderInjector(map));
    });
}

/// Insert one `x-librefang-*` trace header, validating the value and
/// silently skipping (with a `warn!`) on parse failure. See
/// [`build_trace_header_map`] for the rationale on swallow-on-invalid.
///
/// Empty-string values are also treated as absent (no header emitted).
fn insert_trace_header(
    map: &mut reqwest::header::HeaderMap,
    name: &'static str,
    value: Option<&str>,
) {
    use reqwest::header::{HeaderName, HeaderValue};

    let Some(raw) = value.filter(|s| !s.is_empty()) else {
        return;
    };
    match HeaderValue::from_str(raw) {
        Ok(hv) => {
            // `insert` (vs `append`) drops any prior entry under this name —
            // this is what guarantees trace IDs replace `extra_headers`
            // values for the same key instead of duplicating on the wire.
            map.insert(HeaderName::from_static(name), hv);
        }
        Err(err) => {
            warn!(
                invalid_header_value = true,
                name = %name,
                error = %err,
                "trace header value rejected by HeaderValue::from_str (likely contains \\r, \\n, NUL, or non-visible bytes); skipping header but continuing request",
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_llm_driver::CompletionRequest;
    use librefang_types::message::Message;
    use opentelemetry::trace::{TraceContextExt, TracerProvider as _};
    use opentelemetry_sdk::propagation::TraceContextPropagator;
    use opentelemetry_sdk::trace::{Sampler, SdkTracerProvider};
    use tracing::Subscriber;
    use tracing_opentelemetry::OpenTelemetrySpanExt;
    use tracing_subscriber::prelude::*;

    fn empty_request() -> CompletionRequest {
        CompletionRequest {
            model: "test-model".to_string(),
            messages: std::sync::Arc::new(vec![Message::user("hi")]),
            tools: std::sync::Arc::new(Vec::new()),
            max_tokens: 16,
            temperature: 0.0,
            system: None,
            thinking: None,
            prompt_caching: false,
            cache_ttl: None,
            prompt_cache_strategy: None,
            response_format: None,
            timeout_secs: None,
            extra_body: None,
            agent_id: None,
            session_id: None,
            step_id: None,
            reasoning_echo_policy: librefang_types::model_catalog::ReasoningEchoPolicy::default(),
        }
    }

    /// Build a `tracing` subscriber wired to an always-sampling OTel SDK
    /// tracer so that spans entered under it carry a valid, recording
    /// `SpanContext` retrievable via `OpenTelemetrySpanExt::context()`.
    fn otel_subscriber() -> (impl Subscriber + Send + Sync, SdkTracerProvider) {
        let provider = SdkTracerProvider::builder()
            .with_sampler(Sampler::AlwaysOn)
            .build();
        let tracer = provider.tracer("trace-headers-test");
        let subscriber =
            tracing_subscriber::registry().with(tracing_opentelemetry::layer().with_tracer(tracer));
        (subscriber, provider)
    }

    /// With a recording span in scope and the global W3C propagator
    /// registered, `build_trace_header_map` must emit a `traceparent` header
    /// whose embedded trace id matches the active span's trace id — proving
    /// the LLM HTTP request will join the same trace as the caller span.
    #[test]
    fn build_trace_header_map_injects_traceparent_for_active_span() {
        opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());

        let (subscriber, _provider) = otel_subscriber();
        let (header_value, expected_trace_id) =
            tracing::subscriber::with_default(subscriber, || {
                let span = tracing::info_span!("llm.complete");
                let _enter = span.enter();

                let expected_trace_id = span.context().span().span_context().trace_id().to_string();

                let map = build_trace_header_map(&[], &empty_request(), true);
                let tp = map
                    .get("traceparent")
                    .expect("traceparent header must be present under a recording span")
                    .to_str()
                    .expect("traceparent must be valid ASCII")
                    .to_string();
                (tp, expected_trace_id)
            });

        // W3C format: "00-<32 hex trace id>-<16 hex span id>-<2 hex flags>".
        let parts: Vec<&str> = header_value.split('-').collect();
        assert_eq!(
            parts.len(),
            4,
            "traceparent must have 4 dash-delimited parts: {header_value}"
        );
        assert_eq!(parts[0], "00", "version must be 00");
        assert_eq!(
            parts[1], expected_trace_id,
            "traceparent trace id must match the active span's trace id"
        );
        assert_eq!(parts[1].len(), 32, "trace id must be 32 hex chars");
        assert_ne!(
            parts[1],
            "0".repeat(32),
            "trace id must not be all-zero (invalid context)"
        );
    }

    /// The W3C injection is unconditional: even when caller-id headers are
    /// suppressed (`emit_caller_trace_headers = false`), the `traceparent`
    /// header is still emitted so trace continuity is never lost.
    #[test]
    fn traceparent_injected_even_when_caller_headers_disabled() {
        opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());

        let (subscriber, _provider) = otel_subscriber();
        let present = tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("llm.complete");
            let _enter = span.enter();
            let map = build_trace_header_map(&[], &empty_request(), false);
            map.contains_key("traceparent")
        });
        assert!(
            present,
            "traceparent must be emitted regardless of emit_caller_trace_headers"
        );
    }
}
