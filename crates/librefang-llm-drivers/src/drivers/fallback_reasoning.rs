//! Re-target a request's reasoning metadata at the model a fallback slot will
//! actually call (#8616 review).
//!
//! A [`CompletionRequest`] carries reasoning state resolved for the *primary*
//! model: the explicit effort token plus the catalog's
//! [`ReasoningEffortPolicy`] and [`ReasoningEchoPolicy`]. A failover rewrites
//! `req.model`, so without this module the fallback provider receives a token
//! that only the primary model declared — `xhigh` on a model that has never
//! heard of it — and the primary's echo policy, which describes a different
//! wire format.
//!
//! Every chain the kernel builds carries a resolver: the per-agent
//! `FallbackDriver` (`resolve_driver`), the boot-level
//! `[[fallback_providers]]` driver, and the auxiliary `FallbackChain` built by
//! `AuxClient` when a model catalog is wired. A wrapper constructed without one
//! (tests, `AuxClient::with_primary_only`) applies the undeclared policy, which
//! is conservative — no cross-model leak — and never a silent pass-through.

use crate::llm_driver::CompletionRequest;
use librefang_types::config::ReasoningEffortPolicy;
use librefang_types::model_catalog::ReasoningEchoPolicy;
use std::sync::Arc;

/// Catalog lookup the fallback wrappers consult when they rewrite a model.
///
/// Arguments are `(provider, model)` — `provider` may be empty for legacy
/// chains that never registered provider names. Returns the model's effective
/// reasoning policy and its echo policy. The kernel wires this to its model
/// catalog; wrappers built without one behave as if the catalog declared
/// nothing.
pub type ReasoningResolver =
    Arc<dyn Fn(&str, &str) -> (ReasoningEffortPolicy, ReasoningEchoPolicy) + Send + Sync>;

/// Re-resolve `req`'s reasoning metadata for the model it now targets.
///
/// * `resolver` present → the fallback's declared policy coerces the token
///   and its echo policy replaces the primary's.
/// * `resolver` absent → the undeclared policy is applied: the four pre-#8591
///   rungs survive, and a rung only another model declared (`xhigh`,
///   `medium`, `minimal`) is dropped so the fallback keeps its provider
///   default instead of receiving a token it may reject.
pub(crate) fn realign_reasoning(
    req: &mut CompletionRequest,
    provider: &str,
    resolver: Option<&ReasoningResolver>,
) {
    let (policy, echo) = match resolver {
        Some(resolve) => resolve(provider, req.model.as_str()),
        None => (
            ReasoningEffortPolicy::default(),
            ReasoningEchoPolicy::default(),
        ),
    };

    if let Some(cfg) = req.thinking.as_mut() {
        if let Some(mode) = cfg.reasoning_effort.as_ref() {
            match policy.resolve_for_model_switch(mode) {
                Some(effective) if &effective != mode => {
                    tracing::warn!(
                        from = %mode,
                        to = %effective,
                        model = %req.model,
                        "reasoning effort is outside the fallback model's declared set; coerced"
                    );
                    cfg.reasoning_effort = Some(effective);
                }
                Some(_) => {}
                None => {
                    tracing::warn!(
                        token = %mode,
                        model = %req.model,
                        "dropping a reasoning rung the fallback model does not declare; its provider default applies"
                    );
                    cfg.reasoning_effort = None;
                }
            }
        }
    }

    req.reasoning_policy = policy;
    req.reasoning_echo_policy = echo;
}
