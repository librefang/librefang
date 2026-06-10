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
//! `notify_owner = false` and no guidance block is injected.
//!
//! ## Financial safety: the gate is a no-op until a cheap-tier aux slot is wired
//!
//! When no explicit `[llm.auxiliary] owner_notify_triage` chain is
//! configured, `AuxClient::resolve` returns the primary driver with
//! `used_primary = true`. Because the gate fires on **every inbound
//! stranger message** (an attacker-controllable path on a public
//! receptionist agent), billing the primary provider here would be a
//! financial-DoS amplifier. Following the `SkillWorkshopReview`
//! precedent (#3328), `evaluate_stranger_request` returns
//! `TriageVerdict::skip` when `resolution.used_primary` is true and
//! emits a one-shot operator WARN pointing at the `[llm.auxiliary]`
//! slug. **Operators must wire a cheap-tier slot before the gate
//! becomes active:**
//!
//! ```toml
//! [llm.auxiliary]
//! owner_notify_triage = ["anthropic:claude-haiku-4.5"]
//! ```
//!
//! ## Escalation-miss-on-outage trade-off
//!
//! Failure paths collapse to `notify_owner = false` / no guidance
//! block. This means an aux outage silently suppresses escalations
//! (emergencies, scam/impersonation). This is a conscious "no
//! behaviour change" design choice: the primary LLM still has access
//! to `notify_owner` in its toolset and can call it based on its own
//! judgement -- the gate is an optimisation hint, not a hard gate.
//!
//! ## Per-sender verdict caching
//!
//! A per-`(agent_id, sender_user_id)` cache with a configurable TTL
//! (default 120s) prevents repeat strangers from re-triaging on every
//! message. Spam bursts from the same sender reuse the cached verdict
//! instead of synthesizing one aux call per message.
//!
//! [`notify_owner`]: https://librefang.ai/docs/tools#notify_owner

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use librefang_llm_driver::CompletionRequest;
use librefang_types::config::AuxTask;
use librefang_types::message::Message;
use tracing::{debug, warn};

use crate::aux_client::AuxClient;

/// Default TTL for cached triage verdicts (seconds).
const DEFAULT_CACHE_TTL_SECS: u64 = 120;

/// Maximum number of entries in the verdict cache before the oldest
/// entries are evicted.
const MAX_CACHE_ENTRIES: usize = 256;

/// System prompt for the triage gate. English-only -- the model is asked
/// to emit guidance text in the inbound message's language.
const TRIAGE_SYSTEM_PROMPT: &str = "You are a triage assistant for a personal-assistant agent. \
The agent's owner has set boundaries on when the agent may reply autonomously vs. when the \
owner must be looped in. You receive a single inbound message from a NON-OWNER sender and \
you decide whether the agent should notify the owner before answering.\n\n\
OUTPUT FORMAT -- respond with ONE LINE of valid JSON, no prose, no markdown fences:\n\
{\"notify_owner\": <true|false>, \"category\": \"<short_snake_case_category>\", \
\"guidance\": \"<one short sentence in the same language as the inbound message that the \
primary agent can echo back to the sender when notify_owner=true>\"}\n\n\
DECISION RULES -- set notify_owner=true ONLY if AT LEAST ONE of these applies:\n\
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
- When notify_owner=false -> guidance can be a short note like \"respond autonomously, no \
escalation needed\".\n\
- When notify_owner=true -> guidance is a SHORT sentence in the SENDER'S LANGUAGE that the \
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
    ///     (e.g. *"Ho avvisato il Signore, Le fara sapere a breve"*);
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
    /// Gate fell back to a no-notify default -- `notify_owner` is always
    /// `false` on this path. `reason` is a short static string for logs.
    DefaultedNoNotify { reason: &'static str },
    /// Verdict was served from the per-(agent, sender) cache.
    CachedVerdict,
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

    /// `true` when the verdict should produce a guidance block (either
    /// a fresh aux verdict or a cached one).
    pub fn has_guidance(&self) -> bool {
        matches!(
            self.origin,
            TriageOrigin::AuxModel | TriageOrigin::CachedVerdict
        )
    }
}

// ---------------------------------------------------------------------------
// Per-(agent, sender) verdict cache
// ---------------------------------------------------------------------------

/// A cached triage verdict entry.
struct CacheEntry {
    agent_id: String,
    sender_user_id: String,
    verdict: TriageVerdict,
    inserted_at: Instant,
}

/// Process-wide verdict cache. Bounded LRU with TTL expiry.
static VERDICT_CACHE: OnceLock<Mutex<VecDeque<CacheEntry>>> = OnceLock::new();

fn cache() -> &'static Mutex<VecDeque<CacheEntry>> {
    VERDICT_CACHE.get_or_init(|| Mutex::new(VecDeque::with_capacity(MAX_CACHE_ENTRIES)))
}

/// Look up a cached verdict for `(agent_id, sender_user_id)`. Returns
/// `None` when no entry exists or the entry has expired.
pub fn lookup_cached_verdict(
    agent_id: &str,
    sender_user_id: &str,
    ttl: Duration,
) -> Option<TriageVerdict> {
    let mut guard = cache().lock().ok()?;
    // Evict expired entries from the front (oldest first).
    let now = Instant::now();
    while let Some(front) = guard.front() {
        if now.duration_since(front.inserted_at) > ttl {
            guard.pop_front();
        } else {
            break;
        }
    }
    // Linear scan is fine for a small bounded cache.
    guard
        .iter()
        .rev()
        .find(|e| e.agent_id == agent_id && e.sender_user_id == sender_user_id)
        .map(|e| {
            let mut v = e.verdict.clone();
            v.origin = TriageOrigin::CachedVerdict;
            v
        })
}

/// Insert a verdict into the cache. Evicts the oldest entry when full.
pub fn insert_cached_verdict(agent_id: &str, sender_user_id: &str, verdict: &TriageVerdict) {
    let Ok(mut guard) = cache().lock() else {
        return;
    };
    // Cap size.
    while guard.len() >= MAX_CACHE_ENTRIES {
        guard.pop_front();
    }
    guard.push_back(CacheEntry {
        agent_id: agent_id.to_string(),
        sender_user_id: sender_user_id.to_string(),
        verdict: verdict.clone(),
        inserted_at: Instant::now(),
    });
}

/// Clear the verdict cache (used in tests).
#[cfg(test)]
fn clear_cache() {
    if let Some(guard) = VERDICT_CACHE.get() {
        if let Ok(mut g) = guard.lock() {
            g.clear();
        }
    }
}

// ---------------------------------------------------------------------------
// Core evaluation
// ---------------------------------------------------------------------------

/// One-shot flag to avoid spamming the operator WARN when no aux chain
/// is configured. The warning fires once per process lifetime.
static PRIMARY_WARN_FIRED: OnceLock<()> = OnceLock::new();

/// Evaluate an inbound stranger message and return a [`TriageVerdict`].
/// Never panics; always returns either an `AuxModel` verdict or a
/// `DefaultedNoNotify` skip.
///
/// `agent_id` and `sender_user_id` are used for the per-(agent, sender)
/// verdict cache. When a cached verdict exists within
/// [`DEFAULT_CACHE_TTL_SECS`], it is returned without calling the aux
/// model.
pub async fn evaluate_stranger_request(
    user_message: &str,
    sender_display_name: Option<&str>,
    aux_client: &AuxClient,
    agent_id: &str,
    sender_user_id: &str,
) -> TriageVerdict {
    let trimmed = user_message.trim();
    if trimmed.is_empty() {
        debug!("owner_notify_triage: empty inbound message, skipping gate");
        return TriageVerdict::skip("empty inbound message");
    }

    // Check the per-(agent, sender) verdict cache first.
    let ttl = Duration::from_secs(DEFAULT_CACHE_TTL_SECS);
    if let Some(cached) = lookup_cached_verdict(agent_id, sender_user_id, ttl) {
        debug!(
            %agent_id, %sender_user_id,
            "owner_notify_triage: returning cached verdict"
        );
        return cached;
    }

    let resolution = aux_client.resolve(AuxTask::OwnerNotifyTriage);

    // Financial-DoS guard: when no cheap-tier aux chain is configured,
    // `AuxClient::resolve` returns the primary driver with
    // `used_primary = true`. Billing the primary provider on every
    // inbound stranger message is a DoS amplifier. Mirror the
    // SkillWorkshopReview precedent (#3328) and skip.
    if resolution.used_primary {
        PRIMARY_WARN_FIRED.get_or_init(|| {
            warn!(
                "owner_notify_triage: no [llm.auxiliary] owner_notify_triage chain configured; \
                 gate is a no-op to avoid billing the primary provider on every stranger message. \
                 Wire a cheap-tier slot, e.g.: \
                 [llm.auxiliary] owner_notify_triage = [\"anthropic:claude-haiku-4.5\"]"
            );
        });
        return TriageVerdict::skip("no aux chain configured for owner_notify_triage");
    }

    let driver = resolution.driver;
    let model = resolution
        .resolved
        .first()
        .map(|(_, m)| m.clone())
        .unwrap_or_default();

    // Build the user payload. Omit the display name line entirely when
    // the sender is unknown, so the aux model is not biased by a
    // synthetic placeholder.
    let user_payload = match sender_display_name {
        Some(name) => format!("Sender display name: {name}\n\nInbound message:\n{trimmed}"),
        None => format!("Inbound message:\n{trimmed}"),
    };

    // Use only the dedicated `system` field for the system prompt --
    // not both `system` AND `Message::system(...)` in messages. Every
    // other aux caller in the repo (compactor, context_compressor,
    // proactive_memory, history_fold) uses `system: Some(...)` only.
    // Sending the prompt in both slots doubles billed input tokens and
    // may cause provider errors on APIs that reject system messages in
    // the messages array when a top-level `system` field is present.
    let request = CompletionRequest {
        model,
        messages: Arc::new(vec![Message::user(user_payload)]),
        max_tokens: 512,
        temperature: 0.0,
        system: Some(TRIAGE_SYSTEM_PROMPT.to_string()),
        ..Default::default()
    };

    let verdict = match driver.complete(request).await {
        Ok(resp) => parse_verdict(&resp.text()),
        Err(err) => {
            warn!(
                error = %err,
                "owner_notify_triage: aux call failed, defaulting no-notify"
            );
            TriageVerdict::skip("aux LLM call failed")
        }
    };

    // Cache the verdict for this (agent, sender) pair so repeat
    // messages within the TTL window don't re-triage.
    if verdict.is_from_aux() {
        insert_cached_verdict(agent_id, sender_user_id, &verdict);
    }

    verdict
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
/// the verdict came from the default-skip path -- callers should NOT
/// inject anything in that case so the primary prompt stays clean and
/// the agent's existing behaviour is preserved bit-for-bit.
pub fn render_guidance_block(verdict: &TriageVerdict) -> Option<String> {
    if !verdict.has_guidance() {
        return None;
    }
    let action_line = if verdict.notify_owner {
        "OWNER NOTIFICATION ADVISED -- call `notify_owner(reason, summary)` to brief the owner, \
         then reply to the sender using the suggested acknowledgement below."
    } else {
        "OWNER NOTIFICATION NOT NEEDED -- reply to the sender autonomously without calling \
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
        assert!(!v.has_guidance());
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
        clear_cache();
        let aux = aux_with("{\"notify_owner\": true, \"category\": \"c\", \"guidance\": \"g\"}");
        let v = evaluate_stranger_request("", Some("Anyone"), &aux, "agent-1", "sender-1").await;
        assert!(!v.notify_owner);
        assert_eq!(
            v.origin,
            TriageOrigin::DefaultedNoNotify {
                reason: "empty inbound message"
            }
        );
    }

    #[tokio::test]
    async fn evaluate_skips_when_aux_resolved_to_primary() {
        // AuxClient::with_primary_only always returns used_primary = true,
        // so the financial-DoS guard should kick in.
        clear_cache();
        let aux = aux_with(
            r#"{"notify_owner": true, "category": "planning_request",
                "guidance": "Ho avvisato il Signore, Le fara sapere."}"#,
        );
        let v = evaluate_stranger_request(
            "Ciao, dovrei coordinare un meeting con Federico domani.",
            Some("Marco"),
            &aux,
            "agent-primary-guard",
            "marco@lid",
        )
        .await;
        // Must skip -- not call the primary LLM.
        assert!(!v.notify_owner);
        assert_eq!(
            v.origin,
            TriageOrigin::DefaultedNoNotify {
                reason: "no aux chain configured for owner_notify_triage"
            }
        );
        assert!(!v.has_guidance());
    }

    #[tokio::test]
    async fn evaluate_returns_skip_when_aux_response_is_garbage() {
        clear_cache();
        let aux = aux_with("not a json response at all, just prose.");
        let v =
            evaluate_stranger_request("ciao", None, &aux, "agent-garbage", "sender-garbage").await;
        assert!(!v.notify_owner);
        assert!(!v.is_from_aux());
    }

    // ---------- verdict cache ----------

    #[test]
    fn cache_insert_and_lookup_within_ttl() {
        clear_cache();
        let v = TriageVerdict {
            notify_owner: true,
            category: "planning".to_string(),
            guidance: "ok".to_string(),
            origin: TriageOrigin::AuxModel,
        };
        insert_cached_verdict("agent-a", "sender-x", &v);
        let ttl = Duration::from_secs(300);
        let cached = lookup_cached_verdict("agent-a", "sender-x", ttl);
        assert!(cached.is_some());
        let cached = cached.unwrap();
        assert!(cached.notify_owner);
        assert_eq!(cached.origin, TriageOrigin::CachedVerdict);
    }

    #[test]
    fn cache_miss_for_different_agent() {
        clear_cache();
        let v = TriageVerdict {
            notify_owner: true,
            category: "c".to_string(),
            guidance: "g".to_string(),
            origin: TriageOrigin::AuxModel,
        };
        insert_cached_verdict("agent-a", "sender-x", &v);
        let ttl = Duration::from_secs(300);
        assert!(lookup_cached_verdict("agent-b", "sender-x", ttl).is_none());
    }

    #[test]
    fn cache_miss_for_different_sender() {
        clear_cache();
        let v = TriageVerdict {
            notify_owner: true,
            category: "c".to_string(),
            guidance: "g".to_string(),
            origin: TriageOrigin::AuxModel,
        };
        insert_cached_verdict("agent-a", "sender-x", &v);
        let ttl = Duration::from_secs(300);
        assert!(lookup_cached_verdict("agent-a", "sender-y", ttl).is_none());
    }

    #[test]
    fn cache_evicts_beyond_max_entries() {
        clear_cache();
        let v = TriageVerdict {
            notify_owner: false,
            category: "c".to_string(),
            guidance: "g".to_string(),
            origin: TriageOrigin::AuxModel,
        };
        // Fill beyond MAX_CACHE_ENTRIES.
        for i in 0..MAX_CACHE_ENTRIES + 10 {
            insert_cached_verdict("agent", &format!("sender-{i}"), &v);
        }
        let guard = cache().lock().unwrap();
        assert!(guard.len() <= MAX_CACHE_ENTRIES);
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

    #[test]
    fn render_guidance_block_works_for_cached_verdict() {
        let v = TriageVerdict {
            notify_owner: true,
            category: "urgency".to_string(),
            guidance: "Cached guidance".to_string(),
            origin: TriageOrigin::CachedVerdict,
        };
        let block = render_guidance_block(&v).unwrap();
        assert!(block.contains("OWNER NOTIFICATION ADVISED"));
        assert!(block.contains("urgency"));
        assert!(block.contains("Cached guidance"));
    }
}
