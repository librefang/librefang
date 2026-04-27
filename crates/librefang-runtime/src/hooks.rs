//! Plugin lifecycle hooks — intercept points at key moments in agent execution.
//!
//! Provides a callback-based hook system (not dynamic loading) for safe extensibility.
//! Five hook types:
//! - `BeforeToolCall`: Fires before tool execution. Can block the call by returning Err.
//! - `AfterToolCall`: Fires after tool execution. Observe-only.
//! - `TransformToolResult`: Fires after tool execution to rewrite the result string.
//!   The first handler returning `Ok(Some(s))` wins and replaces the result.
//! - `BeforePromptBuild`: Fires before system prompt construction. Handlers can
//!   observe and/or contribute a labeled `DynamicSection` that gets injected
//!   into the prompt — see [`HookHandler::provide_prompt_section`].
//! - `AgentLoopEnd`: Fires after the agent loop completes. Observe-only.

use dashmap::DashMap;
use librefang_types::agent::HookEvent;
use std::sync::Arc;
use tracing::warn;

/// Per-section body cap before any provider's contribution is truncated.
///
/// Matches the prompt-build hook contract documented in #3326.
pub const PER_SECTION_CHAR_CAP: usize = 8 * 1024;

/// Total cap across all dynamic sections combined. Providers that would push
/// the total over this cap are dropped (in registration order, earlier wins),
/// with a `WARN` log per drop. See #3326.
pub const TOTAL_DYNAMIC_CHAR_CAP: usize = 32 * 1024;

/// A labeled section produced by a prompt-context provider, injected into the
/// system prompt at build time.
///
/// Each section renders as `## {heading}\n{body}` in the final prompt.
/// `provider` is used for logs and metrics (slug-style identifier).
#[derive(Debug, Clone)]
pub struct DynamicSection {
    /// Stable identifier for the contributing provider (used for logs / dedup).
    pub provider: String,
    /// Human-readable section heading rendered as `## {heading}`.
    pub heading: String,
    /// Markdown body. Capped at [`PER_SECTION_CHAR_CAP`] before merge.
    pub body: String,
}

/// Context passed to hook handlers.
pub struct HookContext<'a> {
    /// Agent display name.
    pub agent_name: &'a str,
    /// Agent ID string.
    pub agent_id: &'a str,
    /// Which hook event triggered this call.
    pub event: HookEvent,
    /// Event-specific payload (tool name, input, result, etc.).
    pub data: serde_json::Value,
}

/// Hook handler trait. Implementations must be thread-safe.
pub trait HookHandler: Send + Sync {
    /// Called when the hook fires.
    ///
    /// For `BeforeToolCall`: returning `Err(reason)` blocks the tool call.
    /// For all other events: return value is ignored (observe-only).
    fn on_event(&self, ctx: &HookContext) -> Result<(), String>;

    /// Called for `TransformToolResult` hooks to optionally rewrite the tool result.
    ///
    /// Return `Ok(Some(new_result))` to replace the result string.
    /// Return `Ok(None)` to leave the result unchanged and let later handlers run.
    /// Return `Err(reason)` to signal a failure; the error is logged and this handler is skipped.
    ///
    /// Default implementation returns `Ok(None)` (no transformation).
    fn transform(&self, _ctx: &HookContext) -> Result<Option<String>, String> {
        Ok(None)
    }

    /// Called for `BeforePromptBuild` hooks to optionally contribute a labeled
    /// section that gets injected into the system prompt.
    ///
    /// Return `Ok(Some(section))` to inject the section. The kernel applies a
    /// per-section character cap ([`PER_SECTION_CHAR_CAP`]) and a total cap
    /// across all providers ([`TOTAL_DYNAMIC_CHAR_CAP`]) before the prompt is
    /// rendered.
    ///
    /// Return `Ok(None)` to skip this turn (e.g. the agent isn't in the
    /// provider's allowlist).
    ///
    /// Return `Err(reason)` to signal a failure; the error is logged at WARN
    /// and the provider's contribution is dropped for this turn.
    ///
    /// Implementations must complete synchronously and quickly — long-running
    /// work (e.g. a memory sub-agent recall) should be done on a background
    /// task that posts results to a shared cache the hook reads from.
    ///
    /// Default implementation returns `Ok(None)` (no section provided).
    fn provide_prompt_section(&self, _ctx: &HookContext) -> Result<Option<DynamicSection>, String> {
        Ok(None)
    }
}

/// Registry of hook handlers, keyed by event type.
///
/// Thread-safe via `DashMap`. Handlers fire in registration order.
pub struct HookRegistry {
    handlers: DashMap<HookEvent, Vec<Arc<dyn HookHandler>>>,
}

impl HookRegistry {
    /// Create an empty hook registry.
    pub fn new() -> Self {
        Self {
            handlers: DashMap::new(),
        }
    }

    /// Register a handler for a specific event type.
    pub fn register(&self, event: HookEvent, handler: Arc<dyn HookHandler>) {
        self.handlers.entry(event).or_default().push(handler);
    }

    /// Fire all handlers for an event. Returns Err if any handler blocks.
    ///
    /// For `BeforeToolCall`, the first Err stops execution and returns the reason.
    /// For other events, errors are logged but don't propagate.
    pub fn fire(&self, ctx: &HookContext) -> Result<(), String> {
        if let Some(handlers) = self.handlers.get(&ctx.event) {
            for handler in handlers.iter() {
                if let Err(reason) = handler.on_event(ctx) {
                    if ctx.event == HookEvent::BeforeToolCall {
                        return Err(reason);
                    }
                    // For non-blocking hooks, log and continue
                    tracing::warn!(
                        event = ?ctx.event,
                        agent = ctx.agent_name,
                        error = %reason,
                        "Hook handler returned error (non-blocking)"
                    );
                }
            }
        }
        Ok(())
    }

    /// Fire `TransformToolResult` handlers in registration order.
    ///
    /// Returns the first `Ok(Some(s))` result, replacing the tool output.
    /// Handlers returning `Err` are warned and skipped (fail-open).
    /// Returns `None` if no handler produces a replacement.
    pub fn fire_transform(&self, ctx: &HookContext) -> Option<String> {
        if let Some(handlers) = self.handlers.get(&HookEvent::TransformToolResult) {
            for handler in handlers.iter() {
                match handler.transform(ctx) {
                    Ok(Some(new_result)) => return Some(new_result),
                    Ok(None) => continue,
                    Err(reason) => {
                        tracing::warn!(
                            agent = ctx.agent_name,
                            error = %reason,
                            "TransformToolResult hook handler returned error (skipping)"
                        );
                    }
                }
            }
        }
        None
    }

    /// Fire `BeforePromptBuild` handlers in registration order to collect
    /// labeled sections that should be injected into the system prompt.
    ///
    /// Each handler's body is truncated to [`PER_SECTION_CHAR_CAP`] before it
    /// is added to the result. The cumulative size across all sections is
    /// capped at [`TOTAL_DYNAMIC_CHAR_CAP`]; once that cap is reached,
    /// remaining handlers are skipped and a WARN is logged per drop. Errors
    /// from individual handlers are logged at WARN and skipped (fail-open).
    ///
    /// Each handler's `on_event` is also invoked here so observe-only
    /// handlers receive a callback on every prompt build, including the
    /// kernel-direct paths (`send_message_ephemeral`,
    /// `send_message_streaming_with_sender_and_opts`, `execute_llm_agent`)
    /// that don't go through `agent_loop`'s `fire(BeforePromptBuild)`.
    /// `on_event` errors are logged at WARN and do not block section
    /// collection.
    pub fn collect_prompt_sections(&self, ctx: &HookContext) -> Vec<DynamicSection> {
        let Some(handlers) = self.handlers.get(&HookEvent::BeforePromptBuild) else {
            return Vec::new();
        };

        let mut sections: Vec<DynamicSection> = Vec::with_capacity(handlers.len());
        let mut total_chars: usize = 0;

        for handler in handlers.iter() {
            // Fire on_event first so observe-only handlers don't miss the
            // event when prompt is built directly from the kernel rather
            // than via agent_loop's fire(BeforePromptBuild).
            if let Err(reason) = handler.on_event(ctx) {
                warn!(
                    agent = ctx.agent_name,
                    error = %reason,
                    "BeforePromptBuild on_event returned error (continuing)"
                );
            }

            match handler.provide_prompt_section(ctx) {
                Ok(Some(mut section)) => {
                    if section.body.chars().count() > PER_SECTION_CHAR_CAP {
                        let kept_chars = PER_SECTION_CHAR_CAP.saturating_sub(40);
                        let cutoff = section
                            .body
                            .char_indices()
                            .nth(kept_chars)
                            .map(|(i, _)| i)
                            .unwrap_or(section.body.len());
                        let original_chars = section.body.chars().count();
                        section.body.truncate(cutoff);
                        section.body.push_str(&format!(
                            "\n[truncated: {original_chars} → {kept_chars} chars]"
                        ));
                    }

                    let section_chars = section.body.chars().count();
                    if total_chars.saturating_add(section_chars) > TOTAL_DYNAMIC_CHAR_CAP {
                        warn!(
                            agent = ctx.agent_name,
                            provider = section.provider,
                            section_chars,
                            total_chars,
                            cap = TOTAL_DYNAMIC_CHAR_CAP,
                            "Dropping prompt section: total dynamic-section budget exceeded"
                        );
                        continue;
                    }
                    total_chars = total_chars.saturating_add(section_chars);
                    sections.push(section);
                }
                Ok(None) => continue,
                Err(reason) => {
                    warn!(
                        agent = ctx.agent_name,
                        error = %reason,
                        "BeforePromptBuild provide_prompt_section returned error (skipping)"
                    );
                }
            }
        }

        sections
    }

    /// Check if any handlers are registered for a given event.
    pub fn has_handlers(&self, event: HookEvent) -> bool {
        self.handlers
            .get(&event)
            .map(|v| !v.is_empty())
            .unwrap_or(false)
    }
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A test handler that always succeeds.
    struct OkHandler;
    impl HookHandler for OkHandler {
        fn on_event(&self, _ctx: &HookContext) -> Result<(), String> {
            Ok(())
        }
    }

    /// A test handler that always blocks.
    struct BlockHandler {
        reason: String,
    }
    impl HookHandler for BlockHandler {
        fn on_event(&self, _ctx: &HookContext) -> Result<(), String> {
            Err(self.reason.clone())
        }
    }

    /// A test handler that records calls.
    struct RecordHandler {
        calls: std::sync::Mutex<Vec<String>>,
    }
    impl RecordHandler {
        fn new() -> Self {
            Self {
                calls: std::sync::Mutex::new(Vec::new()),
            }
        }
        fn call_count(&self) -> usize {
            self.calls.lock().unwrap().len()
        }
    }
    impl HookHandler for RecordHandler {
        fn on_event(&self, ctx: &HookContext) -> Result<(), String> {
            self.calls.lock().unwrap().push(format!("{:?}", ctx.event));
            Ok(())
        }
    }

    fn make_ctx(event: HookEvent) -> HookContext<'static> {
        HookContext {
            agent_name: "test-agent",
            agent_id: "abc-123",
            event,
            data: serde_json::json!({}),
        }
    }

    #[test]
    fn test_empty_registry_is_noop() {
        let registry = HookRegistry::new();
        let ctx = make_ctx(HookEvent::BeforeToolCall);
        assert!(registry.fire(&ctx).is_ok());
    }

    #[test]
    fn test_before_tool_call_can_block() {
        let registry = HookRegistry::new();
        registry.register(
            HookEvent::BeforeToolCall,
            Arc::new(BlockHandler {
                reason: "Not allowed".to_string(),
            }),
        );
        let ctx = make_ctx(HookEvent::BeforeToolCall);
        let result = registry.fire(&ctx);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Not allowed");
    }

    #[test]
    fn test_after_tool_call_receives_result() {
        let recorder = Arc::new(RecordHandler::new());
        let registry = HookRegistry::new();
        registry.register(HookEvent::AfterToolCall, recorder.clone());

        let ctx = HookContext {
            agent_name: "test-agent",
            agent_id: "abc-123",
            event: HookEvent::AfterToolCall,
            data: serde_json::json!({"tool_name": "file_read", "result": "ok"}),
        };
        assert!(registry.fire(&ctx).is_ok());
        assert_eq!(recorder.call_count(), 1);
    }

    #[test]
    fn test_multiple_handlers_all_fire() {
        let r1 = Arc::new(RecordHandler::new());
        let r2 = Arc::new(RecordHandler::new());
        let registry = HookRegistry::new();
        registry.register(HookEvent::AgentLoopEnd, r1.clone());
        registry.register(HookEvent::AgentLoopEnd, r2.clone());

        let ctx = make_ctx(HookEvent::AgentLoopEnd);
        assert!(registry.fire(&ctx).is_ok());
        assert_eq!(r1.call_count(), 1);
        assert_eq!(r2.call_count(), 1);
    }

    #[test]
    fn test_hook_errors_dont_crash_non_blocking() {
        let registry = HookRegistry::new();
        // Register a blocking handler for a non-blocking event
        registry.register(
            HookEvent::AfterToolCall,
            Arc::new(BlockHandler {
                reason: "oops".to_string(),
            }),
        );
        let ctx = make_ctx(HookEvent::AfterToolCall);
        // AfterToolCall is non-blocking, so error should be swallowed
        assert!(registry.fire(&ctx).is_ok());
    }

    #[test]
    fn test_all_four_events_fire() {
        let recorder = Arc::new(RecordHandler::new());
        let registry = HookRegistry::new();
        registry.register(HookEvent::BeforeToolCall, recorder.clone());
        registry.register(HookEvent::AfterToolCall, recorder.clone());
        registry.register(HookEvent::BeforePromptBuild, recorder.clone());
        registry.register(HookEvent::AgentLoopEnd, recorder.clone());

        for event in [
            HookEvent::BeforeToolCall,
            HookEvent::AfterToolCall,
            HookEvent::BeforePromptBuild,
            HookEvent::AgentLoopEnd,
        ] {
            let ctx = make_ctx(event);
            let _ = registry.fire(&ctx);
        }
        assert_eq!(recorder.call_count(), 4);
    }

    #[test]
    fn test_has_handlers() {
        let registry = HookRegistry::new();
        assert!(!registry.has_handlers(HookEvent::BeforeToolCall));
        registry.register(HookEvent::BeforeToolCall, Arc::new(OkHandler));
        assert!(registry.has_handlers(HookEvent::BeforeToolCall));
        assert!(!registry.has_handlers(HookEvent::AfterToolCall));
    }

    /// A test handler that contributes a fixed section.
    struct SectionHandler {
        provider: String,
        heading: String,
        body: String,
    }
    impl HookHandler for SectionHandler {
        fn on_event(&self, _ctx: &HookContext) -> Result<(), String> {
            Ok(())
        }
        fn provide_prompt_section(
            &self,
            _ctx: &HookContext,
        ) -> Result<Option<DynamicSection>, String> {
            Ok(Some(DynamicSection {
                provider: self.provider.clone(),
                heading: self.heading.clone(),
                body: self.body.clone(),
            }))
        }
    }

    /// A handler that errors instead of contributing.
    struct SectionErrHandler;
    impl HookHandler for SectionErrHandler {
        fn on_event(&self, _ctx: &HookContext) -> Result<(), String> {
            Ok(())
        }
        fn provide_prompt_section(
            &self,
            _ctx: &HookContext,
        ) -> Result<Option<DynamicSection>, String> {
            Err("provider failure".to_string())
        }
    }

    #[test]
    fn test_collect_prompt_sections_empty_when_no_handlers() {
        let registry = HookRegistry::new();
        let ctx = make_ctx(HookEvent::BeforePromptBuild);
        assert!(registry.collect_prompt_sections(&ctx).is_empty());
    }

    #[test]
    fn test_collect_prompt_sections_returns_in_registration_order() {
        let registry = HookRegistry::new();
        registry.register(
            HookEvent::BeforePromptBuild,
            Arc::new(SectionHandler {
                provider: "alpha".into(),
                heading: "Alpha".into(),
                body: "first".into(),
            }),
        );
        registry.register(
            HookEvent::BeforePromptBuild,
            Arc::new(SectionHandler {
                provider: "beta".into(),
                heading: "Beta".into(),
                body: "second".into(),
            }),
        );
        let ctx = make_ctx(HookEvent::BeforePromptBuild);
        let sections = registry.collect_prompt_sections(&ctx);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].provider, "alpha");
        assert_eq!(sections[1].provider, "beta");
    }

    #[test]
    fn test_collect_prompt_sections_skips_none_and_err() {
        let registry = HookRegistry::new();
        registry.register(HookEvent::BeforePromptBuild, Arc::new(OkHandler)); // returns None
        registry.register(HookEvent::BeforePromptBuild, Arc::new(SectionErrHandler));
        registry.register(
            HookEvent::BeforePromptBuild,
            Arc::new(SectionHandler {
                provider: "gamma".into(),
                heading: "Gamma".into(),
                body: "kept".into(),
            }),
        );
        let ctx = make_ctx(HookEvent::BeforePromptBuild);
        let sections = registry.collect_prompt_sections(&ctx);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].provider, "gamma");
    }

    #[test]
    fn test_collect_prompt_sections_per_section_cap_truncates() {
        let registry = HookRegistry::new();
        let oversize = "x".repeat(PER_SECTION_CHAR_CAP + 500);
        registry.register(
            HookEvent::BeforePromptBuild,
            Arc::new(SectionHandler {
                provider: "big".into(),
                heading: "Big".into(),
                body: oversize,
            }),
        );
        let ctx = make_ctx(HookEvent::BeforePromptBuild);
        let sections = registry.collect_prompt_sections(&ctx);
        assert_eq!(sections.len(), 1);
        let body_chars = sections[0].body.chars().count();
        assert!(body_chars <= PER_SECTION_CHAR_CAP);
        assert!(sections[0].body.contains("[truncated"));
    }

    #[test]
    fn test_collect_prompt_sections_total_cap_drops_late_arrivals() {
        let registry = HookRegistry::new();
        // 4 providers, each producing PER_SECTION_CHAR_CAP - 1 chars.
        // Total cap is TOTAL_DYNAMIC_CHAR_CAP = 32K = 4 × 8K, so the 4th
        // section barely overflows and should be dropped.
        let body = "y".repeat(PER_SECTION_CHAR_CAP - 1);
        for idx in 0..5 {
            registry.register(
                HookEvent::BeforePromptBuild,
                Arc::new(SectionHandler {
                    provider: format!("p{idx}"),
                    heading: format!("H{idx}"),
                    body: body.clone(),
                }),
            );
        }
        let ctx = make_ctx(HookEvent::BeforePromptBuild);
        let sections = registry.collect_prompt_sections(&ctx);
        // First 4 fit (4 × (PER_SECTION_CHAR_CAP - 1) = TOTAL_DYNAMIC_CHAR_CAP - 4
        // which is under cap), the 5th would push over.
        assert_eq!(sections.len(), 4);
        assert_eq!(sections.last().unwrap().provider, "p3");
    }

    /// Records each `on_event` invocation. Used to verify that
    /// `collect_prompt_sections` also fires the observe-only callback so
    /// kernel-direct prompt builds (which don't go through agent_loop's
    /// `fire(BeforePromptBuild)`) still notify observers.
    struct OnEventCounter {
        count: Arc<std::sync::atomic::AtomicUsize>,
    }
    impl HookHandler for OnEventCounter {
        fn on_event(&self, _ctx: &HookContext) -> Result<(), String> {
            self.count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }

    #[test]
    fn test_collect_prompt_sections_also_fires_on_event() {
        let registry = HookRegistry::new();
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        registry.register(
            HookEvent::BeforePromptBuild,
            Arc::new(OnEventCounter {
                count: count.clone(),
            }),
        );
        // Also register a section-only handler to confirm both surfaces fire.
        registry.register(
            HookEvent::BeforePromptBuild,
            Arc::new(SectionHandler {
                provider: "s".into(),
                heading: "S".into(),
                body: "b".into(),
            }),
        );

        let ctx = make_ctx(HookEvent::BeforePromptBuild);
        let sections = registry.collect_prompt_sections(&ctx);

        assert_eq!(
            count.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "observe-only handler must receive on_event when collect_prompt_sections runs"
        );
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].provider, "s");
    }
}
