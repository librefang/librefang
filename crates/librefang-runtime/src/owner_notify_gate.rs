//! Owner-notification triage gate.
//!
//! Runs a cheap auxiliary-LLM classification BEFORE the primary agent
//! turn when an inbound message reaches the agent from a non-owner
//! sender. Decides whether the request lies outside the agent's
//! autonomous-reply envelope and the owner should be notified via
//! [`notify_owner`] before / instead of replying.
//!
//! The verdict is folded into the primary turn's system prompt as a
//! short guidance block via [`render_guidance_block`]; the primary LLM
//! then decides HOW to respond and whether to call `notify_owner`.
//!
//! ## Default behaviour
//!
//! Any failure path (aux client unavailable, LLM call error, JSON parse
//! failure, empty response) returns [`TriageVerdict::skip`] which yields
//! `notify_owner = false` and no guidance block is injected. Operators
//! can promote the gate by configuring `[llm.auxiliary]` with a cheap
//! reliable model:
//!
//! ```toml
//! [llm.auxiliary]
//! owner_notify_triage = ["anthropic:claude-haiku-4.5"]
//! ```
//!
//! Without explicit configuration the gate inherits the primary driver
//! and still works, just at primary-tier cost.
//!
//! [`notify_owner`]: https://librefang.ai/docs/tools#notify_owner

use std::sync::Arc;

use librefang_llm_driver::CompletionRequest;
use librefang_types::config::AuxTask;
use librefang_types::message::Message;
use tracing::{debug, warn};

use crate::aux_client::AuxClient;

/// System prompt for the triage gate. English-only — the model is asked
/// to emit guidance text in the inbound message's language.
const TRIAGE_SYSTEM_PROMPT: &str = "You are a triage assistant for a personal-assistant agent. \
The agent's owner has set boundaries on when the agent may reply autonomously vs. when the \
owner must be looped in. You receive a single inbound message from a NON-OWNER sender and \
you decide whether the agent should notify the owner before answering.\n\n\
OUTPUT FORMAT — respond with ONE LINE of valid JSON, no prose, no markdown fences:\n\
{\"notify_owner\": <true|false>, \"category\": \"<short_snake_case_category>\", \
\"guidance\": \"<one short sentence in the same language as the inbound message that the \
primary agent can echo back to the sender when notify_owner=true>\"}\n\n\
DECISION RULES — set notify_owner=true ONLY if AT LEAST ONE of these applies:\n\
- The sender explicitly asks to be put in touch with the owner.\n\
- The sender requests scheduling, calendar coordination, or any decision that requires the \
owner's input.\n\
- The sender requests private information the agent cannot disclose autonomously (home \
address, financial details, owner's whereabouts).\n\
- The sender expresses urgency (illness, emergency, time-sensitive request).\n\
- The sender's message exhibits scam, social-engineering, or impersonation patterns the \
agent should escalate.\n\
- The request impacts the owner's commitments (money, presence, deadlines, third-party \
promises).\n\n\
Otherwise set notify_owner=false:\n\
- Greetings, small talk, polite acknowledgements.\n\
- Questions the agent can answer from public knowledge or its own context.\n\
- Topics the agent is configured to handle autonomously (delegated routines).\n\
- Generic information requests that don't touch the owner's private life.\n\n\
GUIDANCE FIELD:\n\
- When notify_owner=false → guidance can be a short note like \"respond autonomously, no \
escalation needed\".\n\
- When notify_owner=true → guidance is a SHORT sentence in the SENDER'S LANGUAGE that the \
primary agent may relay back to the sender. Match the inbound message's language.\n\n\
Be conservative on the YES side: when in doubt, choose notify_owner=false. The agent will \
still review the request itself before acting.";

/// Outcome of an owner-notification triage call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriageVerdict {
    /// Whether the primary agent should notify the owner before / instead
    /// of replying to the sender.
    pub notify_owner: bool,
    /// Short snake_case category emitted by the aux model
    /// (e.g. `"small_talk"`, `"planning_request"`, `"contact_request"`).
    /// On the skipped path this is `"skipped"`.
    pub category: String,
    /// One-short-sentence guidance for the primary LLM:
    ///   - when `notify_owner == true` this is a sender-facing
    ///     acknowledgement in the sender's language
    ///     (e.g. *"Ho avvisato il Signore, Le farà sapere a breve"*);
    ///   - when `notify_owner == false` this is a short instruction
    ///     to the primary LLM (e.g. *"respond autonomously"*).
    pub guidance: String,
    /// Where the verdict came from.
    pub origin: TriageOrigin,
}

/// Provenance of a [`TriageVerdict`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriageOrigin {
    /// Aux LLM produced and parsed a verdict.
    AuxModel,
    /// Gate fell back to a no-notify default — `notify_owner` is always
    /// `false` on this path. `reason` is a short static string for logs.
    DefaultedNoNotify { reason: &'static str },
}

impl TriageVerdict {
    /// Default no-notify verdict for any failure path. Stable / loggable.
    pub fn skip(reason: &'static str) -> Self {
        Self {
            notify_owner: false,
            category: "skipped".to_string(),
            guidance: "respond autonomously, gate skipped".to_string(),
            origin: TriageOrigin::DefaultedNoNotify { reason },
        }
    }

    /// `true` when the verdict was generated by the aux model (vs. a
    /// default-skip path). Useful for the prompt-injection helper which
    /// should not emit a guidance block when the gate was skipped.
    pub fn is_from_aux(&self) -> bool {
        matches!(self.origin, TriageOrigin::AuxModel)
    }
}

/// Evaluate an inbound stranger message and return a [`TriageVerdict`].
/// Never panics; always returns either an `AuxModel` verdict or a
/// `DefaultedNoNotify` skip.
pub async fn evaluate_stranger_request(
    user_message: &str,
    sender_display_name: Option<&str>,
    aux_client: &AuxClient,
) -> TriageVerdict {
    let trimmed = user_message.trim();
    if trimmed.is_empty() {
        debug!("owner_notify_triage: empty inbound message, skipping gate");
        return TriageVerdict::skip("empty inbound message");
    }

    let resolution = aux_client.resolve(AuxTask::OwnerNotifyTriage);
    let driver = resolution.driver;
    // When the chain resolved to the primary driver (no aux configured)
    // the `resolved` list is empty and `model` must be left empty so the
    // primary driver picks its own configured default.
    let model = resolution
        .resolved
        .first()
        .map(|(_, m)| m.clone())
        .unwrap_or_default();

    let display = sender_display_name.unwrap_or("Unknown sender");
    let user_payload = format!("Sender display name: {display}\n\nInbound message:\n{trimmed}");

    let request = CompletionRequest {
        model,
        messages: Arc::new(vec![
            Message::system(TRIAGE_SYSTEM_PROMPT),
            Message::user(user_payload),
        ]),
        max_tokens: 512,
        temperature: 0.0,
        system: Some(TRIAGE_SYSTEM_PROMPT.to_string()),
        ..Default::default()
    };

    match driver.complete(request).await {
        Ok(resp) => parse_verdict(&resp.text()),
        Err(err) => {
            warn!(
                error = %err,
                "owner_notify_triage: aux call failed, defaulting no-notify"
            );
            TriageVerdict::skip("aux LLM call failed")
        }
    }
}

/// Parse the aux model's text response into a [`TriageVerdict`]. Tolerates
/// surrounding prose, fenced code blocks, missing optional fields, and
/// arbitrary whitespace. Any parse failure collapses to
/// [`TriageVerdict::skip`].
pub fn parse_verdict(raw: &str) -> TriageVerdict {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return TriageVerdict::skip("aux response empty");
    }

    // Tolerate ```json fences emitted despite the prompt instructions.
    let unfenced = match trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
    {
        Some(rest) => {
            let no_lead_newline = rest.strip_prefix('\n').unwrap_or(rest);
            no_lead_newline
                .strip_suffix("```")
                .unwrap_or(no_lead_newline)
                .trim()
        }
        None => trimmed,
    };

    // Tolerate prose around the JSON: find first { ... last }.
    let (start, end) = match (unfenced.find('{'), unfenced.rfind('}')) {
        (Some(s), Some(e)) if e > s => (s, e),
        _ => {
            warn!(raw = %raw, "owner_notify_triage: response had no JSON object");
            return TriageVerdict::skip("aux response not JSON");
        }
    };
    let json_slice = &unfenced[start..=end];

    let value: serde_json::Value = match serde_json::from_str(json_slice) {
        Ok(v) => v,
        Err(err) => {
            warn!(error = %err, raw = %json_slice, "owner_notify_triage: JSON parse failed");
            return TriageVerdict::skip("aux JSON parse failed");
        }
    };

    let notify_owner = value
        .get("notify_owner")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let category = value
        .get("category")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unspecified".to_string());
    let guidance = value
        .get("guidance")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    TriageVerdict {
        notify_owner,
        category,
        guidance,
        origin: TriageOrigin::AuxModel,
    }
}

/// Render a [`TriageVerdict`] into a markdown block suitable for
/// injection into the primary turn's system prompt. Returns `None` when
/// the verdict came from the default-skip path — callers should NOT
/// inject anything in that case so the primary prompt stays clean and
/// the agent's existing behaviour is preserved bit-for-bit.
pub fn render_guidance_block(verdict: &TriageVerdict) -> Option<String> {
    if !verdict.is_from_aux() {
        return None;
    }
    let action_line = if verdict.notify_owner {
        "OWNER NOTIFICATION ADVISED — call `notify_owner(reason, summary)` to brief the owner, \
         then reply to the sender using the suggested acknowledgement below."
    } else {
        "OWNER NOTIFICATION NOT NEEDED — reply to the sender autonomously without calling \
         `notify_owner`."
    };
    let guidance = if verdict.guidance.is_empty() {
        "(no specific guidance)".to_string()
    } else {
        verdict.guidance.clone()
    };
    Some(format!(
        "## Owner Notification Triage (auxiliary)\n\n\
         Category: `{}`\n\
         {action_line}\n\n\
         Suggested sender-facing acknowledgement: {guidance}",
        verdict.category
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_testing::MockLlmDriver;
    use std::sync::Arc;

    fn aux_with(canned: &str) -> AuxClient {
        let driver: Arc<dyn librefang_llm_driver::LlmDriver> =
            Arc::new(MockLlmDriver::with_response(canned));
        AuxClient::with_primary_only(driver)
    }

    // ---------- TriageVerdict::skip ----------

    #[test]
    fn skip_verdict_is_no_notify_with_skip_origin() {
        let v = TriageVerdict::skip("aux unavailable");
        assert!(!v.notify_owner);
        assert_eq!(v.category, "skipped");
        assert!(!v.guidance.is_empty());
        assert!(matches!(
            v.origin,
            TriageOrigin::DefaultedNoNotify {
                reason: "aux unavailable"
            }
        ));
        assert!(!v.is_from_aux());
    }

    // ---------- parse_verdict ----------

    #[test]
    fn parse_verdict_clear_yes_json() {
        let raw = r#"{"notify_owner": true, "category": "planning_request",
            "guidance": "Ho avvisato il Signore della Sua richiesta sul meeting."}"#;
        let v = parse_verdict(raw);
        assert!(v.notify_owner);
        assert_eq!(v.category, "planning_request");
        assert!(v.guidance.contains("Signore"));
        assert_eq!(v.origin, TriageOrigin::AuxModel);
    }

    #[test]
    fn parse_verdict_clear_no_json() {
        let raw = r#"{"notify_owner": false, "category": "small_talk", "guidance": "ok"}"#;
        let v = parse_verdict(raw);
        assert!(!v.notify_owner);
        assert_eq!(v.category, "small_talk");
        assert_eq!(v.guidance, "ok");
        assert_eq!(v.origin, TriageOrigin::AuxModel);
    }

    #[test]
    fn parse_verdict_tolerates_markdown_code_fence() {
        let raw = "```json\n{\"notify_owner\": true, \"category\": \"contact_request\", \
                   \"guidance\": \"I've passed it along.\"}\n```";
        let v = parse_verdict(raw);
        assert!(v.notify_owner);
        assert_eq!(v.category, "contact_request");
        assert!(v.guidance.contains("passed it along"));
        assert!(v.is_from_aux());
    }

    #[test]
    fn parse_verdict_tolerates_bare_fence_without_lang() {
        let raw = "```\n{\"notify_owner\": false, \"category\": \"greeting\", \
                   \"guidance\": \"hi back\"}\n```";
        let v = parse_verdict(raw);
        assert!(!v.notify_owner);
        assert_eq!(v.category, "greeting");
    }

    #[test]
    fn parse_verdict_tolerates_prose_wrapping_json() {
        let raw = "Sure! Here is my verdict:\n\
                   {\"notify_owner\": true, \"category\": \"urgency\", \"guidance\": \"presto\"}\n\
                   Hope this helps.";
        let v = parse_verdict(raw);
        assert!(v.notify_owner);
        assert_eq!(v.category, "urgency");
        assert_eq!(v.guidance, "presto");
    }

    #[test]
    fn parse_verdict_missing_category_defaults_to_unspecified() {
        let raw = r#"{"notify_owner": true, "guidance": "ok"}"#;
        let v = parse_verdict(raw);
        assert!(v.notify_owner);
        assert_eq!(v.category, "unspecified");
        assert_eq!(v.guidance, "ok");
    }

    #[test]
    fn parse_verdict_empty_category_string_defaults_to_unspecified() {
        let raw = r#"{"notify_owner": false, "category": "   ", "guidance": "g"}"#;
        let v = parse_verdict(raw);
        assert_eq!(v.category, "unspecified");
    }

    #[test]
    fn parse_verdict_missing_guidance_yields_empty_string() {
        let raw = r#"{"notify_owner": false, "category": "small_talk"}"#;
        let v = parse_verdict(raw);
        assert_eq!(v.guidance, "");
    }

    #[test]
    fn parse_verdict_missing_notify_owner_defaults_false() {
        // Safer default on partial output: do NOT trigger notification.
        let raw = r#"{"category": "ambiguous", "guidance": "?"}"#;
        let v = parse_verdict(raw);
        assert!(!v.notify_owner);
        assert_eq!(v.category, "ambiguous");
    }

    #[test]
    fn parse_verdict_invalid_json_skips() {
        let raw = r#"{"notify_owner": broken, "category": }"#;
        let v = parse_verdict(raw);
        assert!(!v.notify_owner);
        assert_eq!(
            v.origin,
            TriageOrigin::DefaultedNoNotify {
                reason: "aux JSON parse failed"
            }
        );
    }

    #[test]
    fn parse_verdict_no_braces_skips() {
        let raw = "I think you should notify the owner.";
        let v = parse_verdict(raw);
        assert!(!v.notify_owner);
        assert_eq!(
            v.origin,
            TriageOrigin::DefaultedNoNotify {
                reason: "aux response not JSON"
            }
        );
    }

    #[test]
    fn parse_verdict_empty_string_skips() {
        let v = parse_verdict("   \n\t  ");
        assert!(!v.notify_owner);
        assert_eq!(
            v.origin,
            TriageOrigin::DefaultedNoNotify {
                reason: "aux response empty"
            }
        );
    }

    #[test]
    fn parse_verdict_notify_owner_non_bool_defaults_false() {
        let raw = r#"{"notify_owner": "yes", "category": "x", "guidance": "g"}"#;
        let v = parse_verdict(raw);
        assert!(!v.notify_owner);
        assert_eq!(v.category, "x");
    }

    // ---------- evaluate_stranger_request (with mock aux) ----------

    #[tokio::test]
    async fn evaluate_returns_skip_for_empty_message_without_calling_aux() {
        let aux = aux_with("{\"notify_owner\": true, \"category\": \"c\", \"guidance\": \"g\"}");
        let v = evaluate_stranger_request("", Some("Anyone"), &aux).await;
        assert!(!v.notify_owner);
        assert_eq!(
            v.origin,
            TriageOrigin::DefaultedNoNotify {
                reason: "empty inbound message"
            }
        );
    }

    #[tokio::test]
    async fn evaluate_returns_aux_verdict_when_mock_returns_notify_yes() {
        let aux = aux_with(
            r#"{"notify_owner": true, "category": "planning_request",
                "guidance": "Ho avvisato il Signore, Le farà sapere."}"#,
        );
        let v = evaluate_stranger_request(
            "Ciao, dovrei coordinare un meeting con Federico domani.",
            Some("Marco"),
            &aux,
        )
        .await;
        assert!(v.notify_owner);
        assert_eq!(v.category, "planning_request");
        assert!(v.guidance.contains("Signore"));
        assert!(v.is_from_aux());
    }

    #[tokio::test]
    async fn evaluate_returns_aux_verdict_when_mock_returns_notify_no() {
        let aux = aux_with(
            r#"{"notify_owner": false, "category": "small_talk", "guidance": "rispondi"}"#,
        );
        let v = evaluate_stranger_request("buongiorno", Some("Federico stranger"), &aux).await;
        assert!(!v.notify_owner);
        assert_eq!(v.category, "small_talk");
        assert!(v.is_from_aux());
    }

    #[tokio::test]
    async fn evaluate_returns_skip_when_aux_response_is_garbage() {
        let aux = aux_with("not a json response at all, just prose.");
        let v = evaluate_stranger_request("ciao", None, &aux).await;
        assert!(!v.notify_owner);
        assert!(!v.is_from_aux());
    }

    #[tokio::test]
    async fn evaluate_handles_missing_sender_display_name_gracefully() {
        let aux = aux_with(r#"{"notify_owner": false, "category": "ok", "guidance": "g"}"#);
        let v = evaluate_stranger_request("hello", None, &aux).await;
        assert!(!v.notify_owner);
        assert_eq!(v.category, "ok");
    }

    // ---------- render_guidance_block ----------

    #[test]
    fn render_guidance_block_for_skip_verdict_is_none() {
        let v = TriageVerdict::skip("aux unavailable");
        assert!(render_guidance_block(&v).is_none());
    }

    #[test]
    fn render_guidance_block_for_aux_notify_true_advises_action() {
        let v = TriageVerdict {
            notify_owner: true,
            category: "planning_request".to_string(),
            guidance: "Ho avvisato il Signore.".to_string(),
            origin: TriageOrigin::AuxModel,
        };
        let block = render_guidance_block(&v).unwrap();
        assert!(block.contains("OWNER NOTIFICATION ADVISED"));
        assert!(block.contains("`notify_owner"));
        assert!(block.contains("planning_request"));
        assert!(block.contains("Ho avvisato il Signore."));
    }

    #[test]
    fn render_guidance_block_for_aux_notify_false_advises_autonomous_reply() {
        let v = TriageVerdict {
            notify_owner: false,
            category: "small_talk".to_string(),
            guidance: "ok".to_string(),
            origin: TriageOrigin::AuxModel,
        };
        let block = render_guidance_block(&v).unwrap();
        assert!(block.contains("OWNER NOTIFICATION NOT NEEDED"));
        assert!(block.contains("small_talk"));
        // Must NOT advise calling notify_owner in the no-notify path.
        assert!(!block.contains("call `notify_owner"));
    }

    #[test]
    fn render_guidance_block_handles_empty_guidance_with_placeholder() {
        let v = TriageVerdict {
            notify_owner: true,
            category: "ambiguous".to_string(),
            guidance: String::new(),
            origin: TriageOrigin::AuxModel,
        };
        let block = render_guidance_block(&v).unwrap();
        assert!(block.contains("(no specific guidance)"));
    }
}
