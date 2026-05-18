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
/// (telemetry disabled), `inject_context` is a no-op and the context's span
/// is invalid, so no header is written — the request is unaffected.
///
/// ## Why `opentelemetry::Context::current()` and **not**
/// `tracing::Span::current().context()`
///
/// The obvious-looking `tracing::Span::current().context()` (from
/// [`tracing_opentelemetry::OpenTelemetrySpanExt`]) **silently fails in this
/// process** and was the root cause of the first attempt (PR #5190
/// commit `da5f34c5`) failing end-to-end verification while its unit tests
/// passed.
///
/// `OpenTelemetrySpanExt::context()` works by calling
/// `subscriber.downcast_ref::<WithContext>()` to reach the OTel layer's
/// context-extraction hook (see `tracing-opentelemetry`'s
/// `src/span_ext.rs::context` → `cx.unwrap_or_default()` on downcast miss).
/// LibreFang installs the `OpenTelemetryLayer` **behind a
/// `tracing_subscriber::reload::Layer`** (the deferred-init reload slot in
/// `librefang_api::telemetry::install_otel_reload_layer`, required because
/// the OTLP batch exporter needs a Tokio runtime that does not exist yet at
/// subscriber-construction time). `reload::Layer::downcast_raw`
/// (tracing-subscriber `src/reload.rs`) is *hard-coded to forward only
/// `NoneLayerMarker`* and returns `None` for every other `TypeId` —
/// including `WithContext` — because a raw pointer into a hot-swappable
/// inner layer could dangle. Consequently `downcast_ref::<WithContext>()`
/// can never see the OTel layer, `context()` always returns
/// `Context::default()` (invalid `SpanContext`), and the injected
/// `traceparent` is absent / all-zero — so the downstream proxy starts a
/// fresh root trace. (The same defect silently disabled the `trace_id=`
/// log suffix; zero such suffixes ever appeared in the container log.)
///
/// `opentelemetry::Context::current()` does **not** go through any
/// subscriber downcast. With context activation enabled (the default for
/// `tracing_opentelemetry::layer()`), the OTel layer's `on_enter` calls
/// `cx.clone().attach()`, pushing the span's OTel `Context` onto
/// `opentelemetry`'s task/thread-local context stack
/// (`tracing-opentelemetry` `src/layer.rs::on_enter`). That hook runs
/// regardless of the reload indirection, so reading
/// `Context::current()` synchronously inside an `#[instrument]`-ed async
/// fn body (every `build_trace_header_map` call site is a synchronous
/// statement between the span entry and the first `.await`) yields the
/// live agent-loop span context — exactly the trace the proxy must join.
///
/// [`traceparent`]: https://www.w3.org/TR/trace-context/#traceparent-header
fn inject_w3c_trace_context(map: &mut reqwest::header::HeaderMap) {
    use opentelemetry::global;
    use opentelemetry::Context;
    use opentelemetry_http::HeaderInjector;

    let cx = Context::current();
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
    use opentelemetry::Context as OtelContext;
    use opentelemetry_sdk::propagation::TraceContextPropagator;
    use opentelemetry_sdk::trace::{Sampler, SdkTracerProvider};
    use tracing::Subscriber;
    use tracing_opentelemetry::OpenTelemetrySpanExt;
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::{reload, Registry};

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

    /// Build a `tracing` subscriber that faithfully reproduces **the
    /// production wiring**: the `OpenTelemetryLayer` is installed *behind a
    /// `tracing_subscriber::reload::Layer`*, exactly as
    /// `librefang_api::telemetry::install_otel_reload_layer` /
    /// `init_otel_tracing` do at runtime (deferred-init reload slot, real
    /// layer swapped in via `reload::Handle::modify`).
    ///
    /// This is the load-bearing difference from the original tests, which
    /// installed the OTel layer *directly* on `registry()`. With a direct
    /// install, `OpenTelemetrySpanExt::context()`'s
    /// `downcast_ref::<WithContext>()` succeeds and the old approach
    /// appeared to work; behind the reload layer it can never succeed
    /// (`reload::Layer::downcast_raw` forwards only `NoneLayerMarker`), so
    /// the old approach silently degraded to an invalid context in prod
    /// while every unit test stayed green. These tests must exercise the
    /// reload path or they re-introduce that exact blind spot.
    type BoxedReloadLayer = Box<dyn tracing_subscriber::Layer<Registry> + Send + Sync + 'static>;

    fn otel_subscriber_behind_reload() -> (impl Subscriber + Send + Sync, SdkTracerProvider) {
        let provider = SdkTracerProvider::builder()
            .with_sampler(Sampler::AlwaysOn)
            .build();
        let tracer = provider.tracer("trace-headers-test");

        // Register an empty reload slot first (mirrors
        // `install_otel_reload_layer`), then swap the real OTel layer in
        // via the reload handle (mirrors `init_otel_tracing`).
        let (reload_layer, handle) = reload::Layer::<Option<BoxedReloadLayer>, Registry>::new(None);
        let subscriber = tracing_subscriber::registry().with(reload_layer);
        let otel_layer: BoxedReloadLayer =
            Box::new(tracing_opentelemetry::layer().with_tracer(tracer));
        handle
            .modify(|slot| *slot = Some(otel_layer))
            .expect("reload handle must accept the OTel layer");
        (subscriber, provider)
    }

    /// With the OTel layer behind the reload slot (production wiring) and the
    /// global W3C propagator registered, `build_trace_header_map` must emit a
    /// `traceparent` header whose embedded trace id matches the active span's
    /// trace id, sourced via `opentelemetry::Context::current()`.
    ///
    /// This is the test that would have caught the PR #5190 attempt-1
    /// failure: with the old `tracing::Span::current().context()` source it
    /// fails (no `traceparent` / all-zero trace id) precisely because the
    /// reload layer blocks the `WithContext` downcast; with the corrected
    /// `Context::current()` source it passes because the OTel layer's
    /// `on_enter` activated the context independent of any downcast.
    #[test]
    fn build_trace_header_map_injects_traceparent_behind_reload_layer() {
        opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());

        let (subscriber, _provider) = otel_subscriber_behind_reload();
        let (header_value, expected_trace_id) =
            tracing::subscriber::with_default(subscriber, || {
                let span = tracing::info_span!("llm.complete");
                let _enter = span.enter();

                // Ground truth taken from the OTel-activated current context
                // (NOT via OpenTelemetrySpanExt, which is the broken path).
                let expected_trace_id = OtelContext::current()
                    .span()
                    .span_context()
                    .trace_id()
                    .to_string();

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
        assert_eq!(parts[1].len(), 32, "trace id must be 32 hex chars");
        assert_ne!(
            parts[1],
            "0".repeat(32),
            "trace id must not be all-zero (invalid context — the exact \
             PR #5190 attempt-1 failure mode)"
        );
        assert_eq!(
            parts[1], expected_trace_id,
            "traceparent trace id must match the OTel-active span's trace id"
        );
        // Sampled flag must be set so the proxy's FastAPI instrumentation
        // records the joined span instead of dropping it.
        assert_eq!(parts[3], "01", "sampled flag must be set");
    }

    /// Regression guard documenting *why the original tests passed while
    /// production failed*. Behind the production reload-layer wiring,
    /// `tracing::Span::current().context()` (the
    /// `OpenTelemetrySpanExt::context()` path used by PR #5190 attempt-1)
    /// yields an **invalid** `SpanContext`, because
    /// `reload::Layer::downcast_raw` refuses to forward `WithContext`. If a
    /// future change moves the OTel layer out from behind the reload slot
    /// this assertion will start failing — a signal to revisit whether the
    /// `Context::current()` indirection is still required.
    #[test]
    fn span_ext_context_is_invalid_behind_reload_layer_documents_root_cause() {
        let (subscriber, _provider) = otel_subscriber_behind_reload();
        let span_ext_valid = tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("llm.complete");
            let _enter = span.enter();
            // The OLD approach: this is what attempt-1 shipped.
            span.context().span().span_context().is_valid()
        });
        assert!(
            !span_ext_valid,
            "OpenTelemetrySpanExt::context() is expected to be INVALID behind \
             the reload layer — this is the documented root cause of PR #5190 \
             attempt-1's silent prod failure. If this now succeeds the OTel \
             layer is no longer behind a reload slot; re-evaluate the fix.",
        );
    }

    /// The W3C injection is unconditional: even when caller-id headers are
    /// suppressed (`emit_caller_trace_headers = false`), the `traceparent`
    /// header is still emitted so trace continuity is never lost — and it
    /// must hold under the production reload-layer wiring.
    #[test]
    fn traceparent_injected_even_when_caller_headers_disabled() {
        opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());

        let (subscriber, _provider) = otel_subscriber_behind_reload();
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
