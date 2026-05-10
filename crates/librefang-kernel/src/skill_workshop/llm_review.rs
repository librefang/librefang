//! LLM second-pass review for skill workshop candidates (#3328).
//!
//! When [`librefang_types::agent::ReviewMode`] is `ThresholdLlm` or
//! `Both`, a candidate that passed the heuristic gate is forwarded
//! here for confirmation. The LLM call:
//!
//! 1. Decides whether the candidate is genuinely worth keeping.
//! 2. Optionally refines the suggested name and one-line description.
//!
//! The model receives a strict JSON-output contract so the response is
//! parseable without an LLM-style preamble. Failures degrade gracefully
//! to "accept the heuristic verdict unmodified" — the LLM is an
//! optional refinement, not a permission gate.

use crate::skill_workshop::heuristic::HeuristicHit;
use librefang_llm_driver::{CompletionRequest, LlmDriver};
use librefang_types::message::{ContentBlock, Message, MessageContent, Role};
use serde::Deserialize;
use std::sync::Arc;
use tracing::{debug, warn};

/// Decision returned by [`review_candidate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewDecision {
    /// Keep the candidate. Optional fields, when `Some`, replace the
    /// heuristic-derived name / description before the candidate is
    /// written to disk.
    Accept {
        refined_name: Option<String>,
        refined_description: Option<String>,
        reason: String,
    },
    /// Drop the candidate.
    Reject { reason: String },
    /// LLM call failed (network, parse, timeout). Caller should fall
    /// back to the heuristic verdict — the LLM is a refinement, not
    /// a gate, so an outage cannot break capture.
    Indeterminate { reason: String },
}

/// What the model is asked to return — the system prompt instructs it
/// to emit exactly one JSON object matching this shape.
#[derive(Debug, Deserialize)]
struct ReviewPayload {
    accept: bool,
    #[serde(default)]
    refined_name: Option<String>,
    #[serde(default)]
    refined_description: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

/// Maximum response length we ask the model for. The reply is a tiny
/// JSON object; 256 tokens is plenty even with a verbose `reason`.
const MAX_RESPONSE_TOKENS: u32 = 256;

const SYSTEM_PROMPT: &str = r#"You are reviewing a *candidate skill* drafted from a successful agent-user interaction. The candidate carries a name, description, and Markdown body proposed by a heuristic scanner.

Decide whether the candidate captures a *reusable* workflow that another turn of the same agent should follow. Reject when:
- The candidate is too situational or one-off to generalise.
- The body restates a single tool call already documented elsewhere.
- The triggering message was small-talk, frustration, or off-topic.

When accepting you may optionally refine `name` (snake_case, ≤64 chars) or `description` (one line, ≤200 chars). Include `refined_name` and `refined_description` only when refining; otherwise omit them entirely (do not emit empty strings or null).

Respond with a single JSON object and nothing else. Required keys: `accept` (bool), `reason` (one short sentence). Optional keys: `refined_name` (string), `refined_description` (string). Example:
{"accept": true, "reason": "useful rule, kept the heuristic name"}
"#;

/// Attribution metadata forwarded into the driver's [`CompletionRequest`]
/// so per-agent budgets and trace correlation work for workshop LLM
/// reviews. `agent_id` is required (every review is on behalf of a real
/// agent); `session_id` and `candidate_id` are optional because the
/// surrounding capture pipeline does not always have both — the candidate
/// id is generated up-front in `capture_one`, the session id falls back to
/// `None` only in the test path.
#[derive(Debug, Clone, Copy)]
pub struct ReviewAttribution<'a> {
    pub agent_id: &'a str,
    pub session_id: Option<&'a str>,
    pub candidate_id: Option<&'a str>,
}

/// Run the LLM review pass.
///
/// `driver` is typically the resolved [`librefang_runtime::aux_client::AuxClient`]
/// driver for `AuxTask::SkillReview`. Tests inject a mock implementation.
/// `model` is the model name passed to the driver — kept separate from
/// the driver so the same kernel-level driver pool can be queried for
/// any model.
///
/// `attribution` plumbs `(agent_id, session_id, candidate_id)` through to
/// the driver's `CompletionRequest` so the metering / budget layer can
/// charge the call against the right agent. Without this the call shows
/// up as anonymous spend and per-agent budget caps for the workshop
/// pipeline never bind.
pub async fn review_candidate(
    driver: Arc<dyn LlmDriver>,
    model: &str,
    hit: &HeuristicHit,
    attribution: ReviewAttribution<'_>,
    reasoning_echo_policy: librefang_types::model_catalog::ReasoningEchoPolicy,
) -> ReviewDecision {
    let user_prompt = build_user_prompt(hit);
    let request = CompletionRequest {
        model: model.to_string(),
        messages: Arc::new(vec![Message {
            role: Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::Text {
                text: user_prompt,
                provider_metadata: None,
            }]),
            pinned: false,
            timestamp: None,
        }]),
        tools: Arc::new(vec![]),
        max_tokens: MAX_RESPONSE_TOKENS,
        temperature: 0.0,
        system: Some(SYSTEM_PROMPT.to_string()),
        thinking: None,
        // Each candidate is unique; no shared prefix to cache.
        prompt_caching: false,
        cache_ttl: None,
        response_format: None,
        timeout_secs: Some(30),
        extra_body: None,
        agent_id: Some(attribution.agent_id.to_string()),
        session_id: attribution.session_id.map(str::to_string),
        step_id: attribution.candidate_id.map(str::to_string),
        reasoning_echo_policy,
    };

    let response = match driver.complete(request).await {
        Ok(r) => r,
        Err(e) => {
            warn!(error = %e, "skill_workshop: LLM review call failed; falling back to heuristic");
            return ReviewDecision::Indeterminate {
                reason: format!("driver error: {e}"),
            };
        }
    };

    parse_review_response(&response.text())
}

fn build_user_prompt(hit: &HeuristicHit) -> String {
    let trigger = match &hit.source {
        crate::skill_workshop::candidate::CaptureSource::ExplicitInstruction { trigger } => {
            format!("explicit_instruction: \"{trigger}\"")
        }
        crate::skill_workshop::candidate::CaptureSource::UserCorrection { trigger } => {
            format!("user_correction: \"{trigger}\"")
        }
        crate::skill_workshop::candidate::CaptureSource::RepeatedToolPattern {
            tools,
            repeat_count,
        } => format!("repeated_tool_pattern: tools=[{tools}], count={repeat_count}"),
    };
    let assistant_excerpt = hit
        .assistant_response_excerpt
        .as_deref()
        .unwrap_or("(none — capture was not tied to a specific assistant turn)");
    format!(
        "Source signal: {trigger}\n\n\
         Heuristic-suggested name: {name}\n\
         Heuristic-suggested description: {description}\n\n\
         Body draft:\n---\n{body}\n---\n\n\
         User message excerpt:\n---\n{user_msg}\n---\n\n\
         Previous assistant excerpt:\n---\n{assistant_excerpt}\n---\n",
        name = hit.name,
        description = hit.description,
        body = hit.prompt_context,
        user_msg = hit.user_message_excerpt,
    )
}

/// Parse the model's JSON response.
///
/// Models occasionally wrap JSON in ```json fences or add a sentence of
/// preamble despite the system prompt; this parser strips the most
/// common envelopes before delegating to `serde_json`.
fn parse_review_response(raw: &str) -> ReviewDecision {
    let trimmed = strip_json_envelope(raw.trim());
    let payload: ReviewPayload = match serde_json::from_str(trimmed) {
        Ok(p) => p,
        Err(e) => {
            debug!(error = %e, raw = %raw, "skill_workshop: LLM review response did not parse");
            return ReviewDecision::Indeterminate {
                reason: format!("could not parse JSON: {e}"),
            };
        }
    };

    let reason = payload
        .reason
        .unwrap_or_else(|| "(no reason given)".to_string());
    if payload.accept {
        ReviewDecision::Accept {
            refined_name: payload.refined_name.filter(|s| !s.is_empty()),
            refined_description: payload.refined_description.filter(|s| !s.is_empty()),
            reason,
        }
    } else {
        ReviewDecision::Reject { reason }
    }
}

/// Strip ```json … ``` fences and leading prose so the inner JSON
/// object is left for the parser. Best-effort; if no fence is found the
/// input is returned unchanged.
fn strip_json_envelope(s: &str) -> &str {
    if let Some(start) = s.find("```json") {
        let after = &s[start + "```json".len()..];
        let after = after.trim_start_matches(['\n', '\r', ' ', '\t']);
        if let Some(end) = after.rfind("```") {
            return after[..end].trim();
        }
    }
    if let Some(start) = s.find('{') {
        if let Some(end) = s.rfind('}') {
            if end > start {
                return s[start..=end].trim();
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill_workshop::candidate::CaptureSource;
    use async_trait::async_trait;
    use librefang_llm_driver::{CompletionResponse, LlmError};
    use librefang_types::message::{StopReason, TokenUsage};

    /// Test fixture for the new attribution argument. The metering
    /// pipeline is not exercised in these tests (we use `ScriptedDriver`,
    /// which ignores the `CompletionRequest`'s agent_id), but the call
    /// sites still need to compile against the production signature.
    fn test_attribution() -> ReviewAttribution<'static> {
        ReviewAttribution {
            agent_id: "11111111-1111-1111-1111-111111111111",
            session_id: Some("test-session"),
            candidate_id: Some("test-candidate"),
        }
    }

    fn fixture_hit() -> HeuristicHit {
        HeuristicHit {
            name: "fmt_before_commit".to_string(),
            description: "Run cargo fmt before commit".to_string(),
            prompt_context: "# Run cargo fmt\n\nrun cargo fmt before commit\n".to_string(),
            source: CaptureSource::ExplicitInstruction {
                trigger: "from now on".to_string(),
            },
            user_message_excerpt: "from now on always run cargo fmt".to_string(),
            assistant_response_excerpt: Some("Got it.".to_string()),
        }
    }

    /// Driver stub that returns whatever JSON / text was registered.
    struct ScriptedDriver {
        reply: String,
        fail: bool,
    }

    #[async_trait]
    impl LlmDriver for ScriptedDriver {
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            if self.fail {
                return Err(LlmError::Http("simulated failure".to_string()));
            }
            Ok(CompletionResponse {
                content: vec![ContentBlock::Text {
                    text: self.reply.clone(),
                    provider_metadata: None,
                }],
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
                usage: TokenUsage::default(),
            })
        }
    }

    fn driver(reply: &str) -> Arc<dyn LlmDriver> {
        Arc::new(ScriptedDriver {
            reply: reply.to_string(),
            fail: false,
        })
    }

    fn failing_driver() -> Arc<dyn LlmDriver> {
        Arc::new(ScriptedDriver {
            reply: String::new(),
            fail: true,
        })
    }

    #[tokio::test]
    async fn accept_plain_json() {
        let raw = r#"{"accept": true, "reason": "useful rule"}"#;
        let dec = review_candidate(
            driver(raw),
            "haiku",
            &fixture_hit(),
            test_attribution(),
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
        .await;
        match dec {
            ReviewDecision::Accept {
                refined_name,
                refined_description,
                ..
            } => {
                assert!(refined_name.is_none());
                assert!(refined_description.is_none());
            }
            other => panic!("expected Accept, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn accept_with_refinements() {
        let raw = r#"{"accept": true, "refined_name": "always_fmt", "refined_description": "Run cargo fmt before staging.", "reason": "ok"}"#;
        let dec = review_candidate(
            driver(raw),
            "haiku",
            &fixture_hit(),
            test_attribution(),
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
        .await;
        match dec {
            ReviewDecision::Accept {
                refined_name,
                refined_description,
                ..
            } => {
                assert_eq!(refined_name.as_deref(), Some("always_fmt"));
                assert_eq!(
                    refined_description.as_deref(),
                    Some("Run cargo fmt before staging.")
                );
            }
            other => panic!("expected Accept, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn reject_returns_reason() {
        let raw = r#"{"accept": false, "reason": "too situational"}"#;
        let dec = review_candidate(
            driver(raw),
            "haiku",
            &fixture_hit(),
            test_attribution(),
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
        .await;
        match dec {
            ReviewDecision::Reject { reason } => {
                assert_eq!(reason, "too situational");
            }
            other => panic!("expected Reject, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn driver_error_is_indeterminate() {
        let dec = review_candidate(
            failing_driver(),
            "haiku",
            &fixture_hit(),
            test_attribution(),
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
        .await;
        assert!(matches!(dec, ReviewDecision::Indeterminate { .. }));
    }

    #[tokio::test]
    async fn handles_json_fences() {
        let raw = "```json\n{\"accept\": true, \"reason\": \"ok\"}\n```";
        let dec = review_candidate(
            driver(raw),
            "haiku",
            &fixture_hit(),
            test_attribution(),
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
        .await;
        assert!(matches!(dec, ReviewDecision::Accept { .. }));
    }

    #[tokio::test]
    async fn handles_preamble_then_object() {
        let raw =
            "Sure, here is the result:\n{\"accept\": false, \"reason\": \"trivial\"}\nLet me know.";
        let dec = review_candidate(
            driver(raw),
            "haiku",
            &fixture_hit(),
            test_attribution(),
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
        .await;
        assert!(matches!(dec, ReviewDecision::Reject { .. }));
    }

    #[tokio::test]
    async fn malformed_response_is_indeterminate() {
        let dec = review_candidate(
            driver("not even json"),
            "haiku",
            &fixture_hit(),
            test_attribution(),
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
        .await;
        assert!(matches!(dec, ReviewDecision::Indeterminate { .. }));
    }

    /// Pin the fail-closed invariant: a model reply that contains
    /// multiple JSON-looking blocks (because the candidate body —
    /// partly user-influenced text — leaked into the model's reply)
    /// must NEVER promote a Reject into an Accept. `strip_json_envelope`
    /// takes the leftmost `{` to the rightmost `}`, which deliberately
    /// produces malformed JSON in this case → serde_json fails →
    /// `Indeterminate`. Heuristic verdict (which already passed by the
    /// time the LLM reviewer is consulted) wins, so the LLM acts as a
    /// refinement layer, never an override that an attacker can flip.
    #[tokio::test]
    async fn multiple_json_blocks_does_not_promote_reject_to_accept() {
        let raw = r#"{"accept": false, "reason": "trivial"} btw {"accept": true, "reason": "ok"}"#;
        let dec = review_candidate(
            driver(raw),
            "haiku",
            &fixture_hit(),
            test_attribution(),
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
        .await;
        assert!(
            !matches!(dec, ReviewDecision::Accept { .. }),
            "must never accept on multi-JSON output (got {dec:?})"
        );
    }

    /// Same invariant via the synchronous parser entry point — covers
    /// the case where the parser is reached through code paths other
    /// than `review_candidate` (currently none, but locks the contract).
    #[test]
    fn parse_review_response_multiple_blocks_falls_to_indeterminate() {
        let raw = r#"{"accept": false, "reason": "x"} {"accept": true}"#;
        match parse_review_response(raw) {
            ReviewDecision::Accept { .. } => panic!("must not accept on multi-JSON: got Accept"),
            ReviewDecision::Reject { .. } | ReviewDecision::Indeterminate { .. } => {}
        }
    }

    #[test]
    fn empty_strings_are_dropped_from_refined_fields() {
        let raw =
            r#"{"accept": true, "refined_name": "", "refined_description": "", "reason": "ok"}"#;
        match parse_review_response(raw) {
            ReviewDecision::Accept {
                refined_name,
                refined_description,
                ..
            } => {
                assert!(refined_name.is_none());
                assert!(refined_description.is_none());
            }
            other => panic!("expected Accept, got {other:?}"),
        }
    }
}
