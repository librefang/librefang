//! Owner-side notification when the LLM provider rate-limits the agent
//! kernel-wide and retries have been exhausted.
//!
//! ## Why this module exists
//!
//! The original silent-failure incident (2026-05-20, chat
//! `!CbDmKayZoOd53YJsAOAJ`, msg `355199`): the operator sent an image to
//! Ambrogio on WhatsApp; the daemon spent its budget retrying against an
//! OAuth-Max account that had hit its rolling 5-hour cap; after the final
//! retry the daemon logged
//!
//! ```text
//! WARN ... Claude Code CLI streaming subprocess exited with error
//!        exit_code=1 stderr=
//! ```
//!
//! and dropped the turn. The owner saw nothing in the chat. They had no
//! idea Ambrogio was offline, nor when the quota would reset.
//!
//! The Claude CLI emits a `rate_limit_event` line on every streaming call
//! that includes a precise `resetsAt` unix timestamp. Layer 1 of this
//! change (`crates/librefang-llm-drivers/src/drivers/claude_code.rs`)
//! captures that line and surfaces a structured
//! [`LlmError::RateLimited`] with a machine-readable `message` payload.
//! This module is layer 3: when the agent loop's retry helper has given
//! up, we render an operator-configurable template and dispatch it
//! through the same channel the request arrived on — bypassing the
//! agent loop entirely (re-entering it would just hit the same wall).
//!
//! ## Resolution order
//!
//! 1. `AgentManifest.rate_limit_notify` (per-agent override in
//!    `agent.toml`)
//! 2. `KernelConfig.rate_limit_notify` (deployment-wide default in
//!    `config.toml`)
//! 3. Hardcoded fallback template ([`DEFAULT_TEMPLATE`])
//!
//! The first non-disabled config in that chain wins. `enabled = false`
//! anywhere up the chain short-circuits the dispatch.
//!
//! ## Placeholders
//!
//! The template is rendered by [`render_rate_limit_template`], a simple
//! `{name}` substitution (no Tera/Handlebars). Unknown placeholders are
//! left as the literal `{foo}` so template typos surface visibly in the
//! delivered notification instead of being elided.
//!
//! | Placeholder              | Example                          |
//! |--------------------------|----------------------------------|
//! | `{reset_time}`           | `13:40`                          |
//! | `{reset_time_full}`      | `2026-05-20 13:40:00 Europe/Rome`|
//! | `{reset_in_minutes}`     | `45`                             |
//! | `{reset_tz}`             | `Europe/Rome` (or `UTC` on fallback)|
//! | `{agent_name}`           | `ambrogio`                       |
//!
//! ## Timezone resolution
//!
//! [`resolve_timezone`] consults `KernelConfig.system.timezone`. An empty,
//! `None`, or unparseable value falls back to UTC and logs a `warn!`
//! exactly once per process so the operator sees the misconfiguration in
//! their daemon log without it spamming on every turn.
//!
//! ## Dedup window
//!
//! The dispatcher dedupes by `(agent_id, peer, reset_bucket_5min)` via a
//! small **per-process** LRU ([`OWNER_NOTIFY_DEDUP`], capacity 64).
//! Bucketing to 5-minute slots means the rapid retry storm a single
//! rate-limit incident triggers (multiple turns from the same peer landing
//! on the same exhausted quota window) produces exactly one chat
//! notification per daemon process. The bucket key uses the *reset*
//! timestamp, not the current time, so a notify-on-retry within the same
//! window is suppressed even when the retries straddle a wall-clock
//! 5-minute boundary.
//!
//! The LRU is **not persisted across daemon restarts** (houko #5311
//! finding 3): a config reload, `librefang start`, OOM-kill, or normal
//! redeploy inside an open 5-hour OAuth-Max window wipes the bucket and
//! the next incident from the same peer will notify again. Persisting to
//! `~/.librefang/state/` was considered and rejected — the dedup window
//! is short, the cross-process collision rate is low (daemon restarts
//! are rare relative to the retry storms inside one process), and the
//! marginal value of suppressing a second ping after an explicit restart
//! does not justify a disk-backed cache here.

use chrono::Utc;
use chrono_tz::Tz;
use librefang_kernel_handle::{ChannelSender, KernelHandle};
use librefang_types::agent::AgentManifest;
use librefang_types::config::{KernelConfig, RateLimitNotifyConfig};
use std::collections::VecDeque;
use std::str::FromStr;
use std::sync::{Arc, Mutex, OnceLock};
use tracing::{debug, info, warn};

/// Hardcoded fallback when neither the agent manifest nor the kernel config
/// supplies a template. Italian-language default matches the deployed
/// Ambrogio agent — operators with other locales should set their own
/// template in `config.toml`'s `[rate_limit_notify]`.
pub const DEFAULT_TEMPLATE: &str =
    "⏸️ Limite Claude raggiunto. Reset alle {reset_time}. Ti rispondo dopo.";

/// Dedup-LRU capacity. 64 is plenty: each entry represents one
/// `(agent, peer, 5-min reset bucket)` combination, and a daemon serving
/// more than 64 distinct peers simultaneously is well past the small-team
/// scale this feature targets — the LRU evicts the oldest entry rather
/// than dropping the notification, so even at saturation the worst case is
/// a duplicate ping after the original entry rolls off.
const DEDUP_CAPACITY: usize = 64;

/// 5-minute bucket size (in seconds) for the dedup key. Matches the
/// granularity at which a single rate-limit incident's worth of retries
/// would land — a tighter bucket would let the same incident notify
/// multiple times; a looser bucket would suppress a second incident in
/// the same hour even though it's a genuinely new event.
const DEDUP_BUCKET_SECS: i64 = 300;

/// Per-process dedup LRU. Stored as `OnceLock<Mutex<VecDeque<...>>>` so
/// the module is lazy-initialised on first use and the test suite can
/// run independent assertions in serial. The VecDeque is small enough
/// (≤ [`DEDUP_CAPACITY`]) that linear scan is faster than a hashmap;
/// optimising further isn't worth the complexity.
///
/// **Not persisted.** A daemon restart wipes the bucket — see the
/// module-level "Dedup window" docs for the design rationale (houko
/// #5311 finding 3).
static OWNER_NOTIFY_DEDUP: OnceLock<Mutex<VecDeque<DedupKey>>> = OnceLock::new();

/// Single-shot warn-once gate for unparseable timezones.
static TIMEZONE_WARN_ONCE: OnceLock<Mutex<bool>> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq)]
struct DedupKey {
    agent_id: String,
    peer: String,
    reset_bucket: i64,
}

fn dedup_store() -> &'static Mutex<VecDeque<DedupKey>> {
    OWNER_NOTIFY_DEDUP.get_or_init(|| Mutex::new(VecDeque::with_capacity(DEDUP_CAPACITY)))
}

/// Returns `true` if this `(agent_id, peer, reset_at_unix)` was *not*
/// recently notified — in which case the caller proceeds and the key is
/// recorded — or `false` if the same key was already seen in the same
/// 5-minute bucket and the dispatch should be skipped.
pub fn should_dispatch(agent_id: &str, peer: &str, reset_at_unix: i64) -> bool {
    let reset_bucket = reset_at_unix.div_euclid(DEDUP_BUCKET_SECS);
    let key = DedupKey {
        agent_id: agent_id.to_string(),
        peer: peer.to_string(),
        reset_bucket,
    };
    let mut guard = match dedup_store().lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(), // poisoned mutex still lets us read
    };
    if guard.iter().any(|k| k == &key) {
        return false;
    }
    if guard.len() >= DEDUP_CAPACITY {
        guard.pop_front();
    }
    guard.push_back(key);
    true
}

/// Clear the dedup LRU. Used by tests and by operator tooling that wants
/// to force a re-notify (the kernel does NOT call this on hot-reload —
/// once we've told the owner about a reset, we don't want to spam them
/// again when their config edit causes a manifest reload).
///
/// Note that a process restart already clears the LRU implicitly — the
/// store lives in `OnceLock<Mutex<VecDeque>>` with no disk backing
/// (houko #5311 finding 3).
pub fn reset_dedup_for_tests() {
    if let Some(store) = OWNER_NOTIFY_DEDUP.get() {
        if let Ok(mut guard) = store.lock() {
            guard.clear();
        }
    }
}

/// Context passed to [`render_rate_limit_template`]. Construct via
/// [`RenderContext::from_reset`] which handles the timezone resolution.
#[derive(Debug, Clone)]
pub struct RenderContext {
    /// `HH:MM` in the resolved timezone.
    pub reset_time: String,
    /// `YYYY-MM-DD HH:MM:SS TZ` in the resolved timezone.
    pub reset_time_full: String,
    /// Floored minutes from `now` to `reset_at`.
    pub reset_in_minutes: i64,
    /// IANA name of the resolved timezone, e.g. `Europe/Rome` or `UTC` on
    /// fallback. Surface this in the template (`{reset_tz}`) so operators
    /// can spot misconfigured `[system].timezone` values in the chat.
    pub reset_tz: String,
    /// `AgentManifest.name`.
    pub agent_name: String,
}

impl RenderContext {
    /// Build a context from a unix-seconds `reset_at` timestamp, a `now`
    /// reference (passed explicitly so tests are deterministic), the
    /// resolved timezone, and the agent name.
    pub fn from_reset(reset_at_unix: i64, now_unix: i64, tz: Tz, agent_name: &str) -> Self {
        let reset_dt_utc = chrono::DateTime::<chrono::Utc>::from_timestamp(reset_at_unix, 0)
            .unwrap_or_else(|| chrono::DateTime::<chrono::Utc>::from_timestamp(0, 0).unwrap());
        let reset_local = reset_dt_utc.with_timezone(&tz);
        let reset_time = reset_local.format("%H:%M").to_string();
        let reset_time_full = format!("{} {}", reset_local.format("%Y-%m-%d %H:%M:%S"), tz.name());
        // Floored minutes (matching the spec: `{reset_in_minutes}` is an
        // integer, never negative). saturating_sub keeps a past
        // `reset_at` from going negative when retry took longer than
        // the original window.
        let diff_secs = reset_at_unix.saturating_sub(now_unix);
        let reset_in_minutes = (diff_secs / 60).max(0);
        Self {
            reset_time,
            reset_time_full,
            reset_in_minutes,
            reset_tz: tz.name().to_string(),
            agent_name: agent_name.to_string(),
        }
    }
}

/// Render the rate-limit notification template with the given context.
///
/// Substitution rules:
/// - `{name}` is replaced with the matching field of [`RenderContext`].
/// - Unknown placeholders (`{bogus}`) are kept verbatim so template
///   typos are visible in the final delivered message.
/// - No escaping: the template is operator-supplied and trusted; nested
///   braces in user data are not a concern because the only inputs come
///   from operator config and structured timestamps.
pub fn render_rate_limit_template(template: &str, ctx: &RenderContext) -> String {
    let mut out = String::with_capacity(template.len() + 32);
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            // Find matching '}'
            if let Some(end_rel) = template[i + 1..].find('}') {
                let end = i + 1 + end_rel;
                let name = &template[i + 1..end];
                let replacement = match name {
                    "reset_time" => Some(ctx.reset_time.clone()),
                    "reset_time_full" => Some(ctx.reset_time_full.clone()),
                    "reset_in_minutes" => Some(ctx.reset_in_minutes.to_string()),
                    "reset_tz" => Some(ctx.reset_tz.clone()),
                    "agent_name" => Some(ctx.agent_name.clone()),
                    _ => None,
                };
                if let Some(value) = replacement {
                    out.push_str(&value);
                } else {
                    // Unknown placeholder — preserve literal `{name}` so
                    // template typos are visible to the recipient.
                    out.push_str(&template[i..=end]);
                }
                i = end + 1;
                continue;
            }
        }
        // Walk byte-by-byte: safe because we're inside an ASCII-only `{`
        // match check; non-ASCII bytes never collide with `{` and are
        // copied as-is into the output string.
        let ch_start = i;
        // Advance to the next char boundary so we don't split a UTF-8
        // codepoint while copying.
        let mut ch_end = ch_start + 1;
        while ch_end < bytes.len() && !template.is_char_boundary(ch_end) {
            ch_end += 1;
        }
        out.push_str(&template[ch_start..ch_end]);
        i = ch_end;
    }
    out
}

/// Resolve a configured timezone name, falling back to UTC with a single
/// `warn!` for unparseable values (warn-once per process). Accepts the
/// raw operator string so callers can use either `KernelConfig.system.timezone`
/// or `KernelHandle::system_timezone()` interchangeably.
pub fn resolve_timezone_str(name: Option<&str>) -> Tz {
    let name = name.unwrap_or("");
    if name.is_empty() {
        return Tz::UTC;
    }
    match Tz::from_str(name) {
        Ok(tz) => tz,
        Err(_) => {
            let warn_gate = TIMEZONE_WARN_ONCE.get_or_init(|| Mutex::new(false));
            if let Ok(mut fired) = warn_gate.lock() {
                if !*fired {
                    warn!(
                        configured = %name,
                        "Unparseable [system].timezone, falling back to UTC"
                    );
                    *fired = true;
                }
            }
            Tz::UTC
        }
    }
}

/// Convenience wrapper around [`resolve_timezone_str`] that reads from a
/// full `KernelConfig`. Kept for direct-config call paths (mostly tests).
pub fn resolve_timezone(config: &KernelConfig) -> Tz {
    resolve_timezone_str(config.system.timezone.as_deref())
}

/// Resolve the active [`RateLimitNotifyConfig`] for an agent. Per-agent
/// override wins when `Some(…)` — even `Some(RateLimitNotifyConfig {
/// enabled: false, .. })` explicitly disables the feature for that agent.
/// `None` (field absent in `agent.toml`) falls through to the kernel-global
/// config. This uses the `Option<T>` idiom (same as `compaction`) so TOML
/// `enabled = false` is distinguishable from "field not set".
pub fn resolve_config<'a>(
    manifest: &'a AgentManifest,
    kernel: &'a KernelConfig,
) -> &'a RateLimitNotifyConfig {
    match &manifest.rate_limit_notify {
        Some(per_agent) => per_agent,
        None => &kernel.rate_limit_notify,
    }
}

/// Extract the unix-seconds `resets_at` from a `RateLimited.message`
/// payload built by [`build_rate_limit_message`] in the Claude Code
/// driver (Layer 1). Returns `None` when the marker is absent or
/// unparseable — callers should fall back to deriving the reset from
/// `now + retry_after_ms`.
pub fn parse_rate_limit_message(message: &str) -> Option<i64> {
    let needle = "resets_at_unix=";
    let idx = message.find(needle)?;
    let tail = &message[idx + needle.len()..];
    let end = tail
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(tail.len());
    let digits = &tail[..end];
    digits.parse::<i64>().ok()
}

/// Dispatch the rendered notification to the channel that originated the
/// turn. Performs dedup, render, and the actual channel call. Returns
/// `Ok(true)` if a message was sent, `Ok(false)` if the dispatch was
/// skipped (config disabled, missing peer info, dedup hit, or no
/// channel sender). Errors propagate from the underlying channel API.
#[allow(clippy::too_many_arguments)]
pub async fn maybe_dispatch_owner_notify(
    manifest: &AgentManifest,
    kernel_cfg: &KernelConfig,
    channel: Option<&str>,
    recipient: Option<&str>,
    account_id: Option<&str>,
    reset_at_unix: Option<i64>,
    retry_after_ms: u64,
    channel_sender: Option<&dyn ChannelSender>,
) -> Result<bool, String> {
    let cfg = resolve_config(manifest, kernel_cfg);
    if !cfg.enabled {
        debug!(
            agent = %manifest.name,
            "Rate-limit owner notify is disabled (resolved enabled=false)"
        );
        return Ok(false);
    }
    let Some(channel) = channel else {
        debug!(
            agent = %manifest.name,
            "Skipping rate-limit owner notify: no channel context (cron / API direct caller)"
        );
        return Ok(false);
    };
    let Some(recipient) = recipient else {
        debug!(
            agent = %manifest.name,
            channel = %channel,
            "Skipping rate-limit owner notify: no recipient sender_id in context"
        );
        return Ok(false);
    };
    let Some(sender) = channel_sender else {
        debug!(
            agent = %manifest.name,
            channel = %channel,
            "Skipping rate-limit owner notify: no ChannelSender available on kernel"
        );
        return Ok(false);
    };

    let now_unix = Utc::now().timestamp();
    let reset_unix = reset_at_unix.unwrap_or_else(|| {
        // Best-effort fallback when only retry_after_ms was supplied.
        now_unix + (retry_after_ms / 1000) as i64
    });

    if !should_dispatch(&manifest.name, recipient, reset_unix) {
        debug!(
            agent = %manifest.name,
            recipient = %recipient,
            reset_unix,
            "Rate-limit owner notify deduped (already sent within 5-min window)"
        );
        return Ok(false);
    }

    let tz = resolve_timezone(kernel_cfg);
    let ctx = RenderContext::from_reset(reset_unix, now_unix, tz, &manifest.name);
    let template = cfg.template.as_deref().unwrap_or(DEFAULT_TEMPLATE);
    let rendered = render_rate_limit_template(template, &ctx);

    info!(
        event = "rate_limit_owner_notified",
        agent = %manifest.name,
        channel = %channel,
        recipient = %recipient,
        reset_unix,
        retry_after_ms,
        "Dispatching rate-limit owner notification"
    );

    // We pass `thread_id = None` because the rate-limit event is itself
    // not a reply to any specific message; it surfaces in the active
    // chat at the top level so the owner can read it without having to
    // expand a thread.
    sender
        .send_channel_message(channel, recipient, &rendered, None, account_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(true)
}

/// Kernel-aware variant of [`maybe_dispatch_owner_notify`]: pulls the
/// kernel-global `[rate_limit_notify]` config and `[system].timezone`
/// directly from the `KernelHandle`, and uses the handle itself as the
/// `ChannelSender`. This is the call site used by the agent loop, where
/// only an `Arc<dyn KernelHandle>` is in scope.
///
/// Detects a final rate-limit error by parsing the error message for the
/// `[rate_limit_defer_ms]` marker that `agent_loop::retry::handle_retryable_llm_error`
/// appends on exhaustion. Returns `Ok(false)` and short-circuits silently
/// when the marker is absent so non-rate-limit errors flow through
/// untouched.
pub async fn dispatch_via_kernel(
    manifest: &AgentManifest,
    kernel: &Arc<dyn KernelHandle>,
    channel: Option<&str>,
    recipient: Option<&str>,
    account_id: Option<&str>,
    final_error_message: &str,
) -> bool {
    // Bail out fast when the error isn't actually a rate-limit exhaust.
    let defer_ms =
        match librefang_channels::message_journal::parse_defer_marker(final_error_message) {
            Some(ms) => ms,
            None => return false,
        };

    // Pull config + timezone via the KernelHandle accessors so the runtime
    // doesn't need a direct `KernelConfig` reference.
    let kernel_cfg = kernel.rate_limit_notify_config().unwrap_or_default();
    let tz_name = kernel.system_timezone();

    let active = match &manifest.rate_limit_notify {
        Some(per_agent) => per_agent,
        None => &kernel_cfg,
    };
    if !active.enabled {
        debug!(
            agent = %manifest.name,
            "Rate-limit owner notify is disabled (resolved enabled=false)"
        );
        return false;
    }
    let Some(channel) = channel else {
        debug!(
            agent = %manifest.name,
            "Skipping rate-limit owner notify: no channel context (cron / API direct caller)"
        );
        return false;
    };
    let Some(recipient) = recipient else {
        debug!(
            agent = %manifest.name,
            channel = %channel,
            "Skipping rate-limit owner notify: no recipient sender_id in context"
        );
        return false;
    };

    // Recover the precise reset wall-clock from the driver's embedded
    // `resets_at_unix=<ts>` marker (Layer 1 / Claude CLI), else fall back
    // to `now + defer_ms`.
    let now_unix = Utc::now().timestamp();
    let reset_unix = parse_rate_limit_message(final_error_message)
        .unwrap_or_else(|| now_unix + (defer_ms / 1000) as i64);

    if !should_dispatch(&manifest.name, recipient, reset_unix) {
        debug!(
            agent = %manifest.name,
            recipient = %recipient,
            reset_unix,
            "Rate-limit owner notify deduped (already sent within 5-min window)"
        );
        return false;
    }

    let tz = resolve_timezone_str(tz_name.as_deref());
    let ctx = RenderContext::from_reset(reset_unix, now_unix, tz, &manifest.name);
    let template = active.template.as_deref().unwrap_or(DEFAULT_TEMPLATE);
    let rendered = render_rate_limit_template(template, &ctx);

    info!(
        event = "rate_limit_owner_notified",
        agent = %manifest.name,
        channel = %channel,
        recipient = %recipient,
        reset_unix,
        defer_ms,
        "Dispatching rate-limit owner notification"
    );

    let sender: &dyn ChannelSender = kernel.as_ref();
    match sender
        .send_channel_message(channel, recipient, &rendered, None, account_id)
        .await
    {
        Ok(_) => {
            // houko #5311 finding 7: mirror the dispatched notification
            // back into the channel-owning agent's inbound-routing
            // session, the same way `tool_runner::channel::tool_channel_send`
            // does for ordinary agent outbound (PR #4932). Without this
            // mirror, after the quota resets the agent has no
            // transcript record that the user was already pinged "I'm
            // rate-limited" — risk of duplicate apology or contradictory
            // narration on the very next turn. Best-effort: a mirror
            // failure must not cancel the dispatch — the user already
            // received the message — so we only log on no-op paths.
            mirror_owner_notify_into_session(kernel, &manifest.name, channel, recipient, &rendered);
            true
        }
        Err(e) => {
            warn!(
                agent = %manifest.name,
                channel = %channel,
                recipient = %recipient,
                error = %e,
                "Rate-limit owner notify dispatch failed"
            );
            false
        }
    }
}

/// Mirror the rate-limit notification into the channel-owning agent's
/// inbound-routing session so the agent's prompt context records that
/// the user was already told "agent is rate-limited" — same JSON envelope
/// shape used by `tool_runner::channel::mirror_channel_send_to_session`
/// (#4824 decision 3 + #4932), keeping the data contract stable across
/// both outbound paths.
///
/// Best-effort by design: any structural reason the mirror can't land
/// (no owning agent configured, missing session metadata) degrades to a
/// `debug!` log rather than a `warn!` — the user-visible dispatch has
/// already succeeded.
fn mirror_owner_notify_into_session(
    kernel: &Arc<dyn KernelHandle>,
    agent_name: &str,
    channel: &str,
    recipient: &str,
    body: &str,
) {
    use librefang_types::agent::SessionId;
    use librefang_types::message::{Message, MessageContent, Role};

    let Some(owner) = kernel.resolve_channel_owner(channel, recipient) else {
        debug!(
            agent = %agent_name,
            channel = %channel,
            recipient = %recipient,
            "owner-notify mirror: no channel owner agent — skipping"
        );
        return;
    };

    // Mirror under the inbound routing session — the channel + recipient
    // collapse to the same scope that handles inbound user turns.
    let session_id = SessionId::for_sender_scope(owner, channel, Some(recipient));

    // JSON envelope (#4824): from = agent_name (the agent whose loop
    // tripped the rate-limit), body = rendered template. Both fields
    // JSON-escaped via `serde_json::to_string` to neutralise prompt
    // injection through the body content.
    let mirror_text = format!(
        "{{\"mirror_from\":{},\"body\":{}}}",
        serde_json::Value::String(agent_name.to_string()),
        serde_json::Value::String(body.to_string()),
    );

    let msg = Message {
        role: Role::User,
        content: MessageContent::Text(mirror_text),
        pinned: false,
        timestamp: Some(Utc::now()),
    };

    kernel.append_to_session(session_id, owner, msg);
}

/// Synchronous date helper used by tests where mocking `Utc::now()` would
/// add ceremony. `chrono::TimeZone::timestamp_opt` returns a
/// `LocalResult` which can be ambiguous around DST jumps; we resolve
/// ambiguity by taking the earliest valid moment.
#[cfg(test)]
fn unix_to_tz(ts: i64, tz: Tz) -> chrono::DateTime<Tz> {
    use chrono::TimeZone;
    match tz.timestamp_opt(ts, 0) {
        chrono::LocalResult::Single(dt) => dt,
        chrono::LocalResult::Ambiguous(dt, _) => dt,
        chrono::LocalResult::None => chrono::Utc.timestamp_opt(0, 0).unwrap().with_timezone(&tz),
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use librefang_types::config::{KernelConfig, RateLimitNotifyConfig, SystemConfig};

    fn cfg_with_tz(tz: Option<&str>) -> KernelConfig {
        let mut k = KernelConfig::default();
        k.system = SystemConfig {
            timezone: tz.map(String::from),
        };
        k
    }

    fn ctx_for(reset_unix: i64, now_unix: i64, tz: Tz, name: &str) -> RenderContext {
        RenderContext::from_reset(reset_unix, now_unix, tz, name)
    }

    #[test]
    fn template_renders_with_all_placeholders() {
        // 1_779_277_200 = 2026-05-20 11:40:00 UTC
        let reset = 1_779_277_200_i64;
        let now = reset - 45 * 60; // 45 minutes earlier
        let ctx = ctx_for(reset, now, Tz::UTC, "ambrogio");
        let tpl = "{agent_name} resets at {reset_time} ({reset_in_minutes} min) {reset_tz} | full={reset_time_full}";
        let out = render_rate_limit_template(tpl, &ctx);
        assert!(out.starts_with("ambrogio resets at "), "got: {out}");
        assert!(out.contains("(45 min)"), "got: {out}");
        assert!(out.contains("UTC"), "got: {out}");
        assert!(out.contains("11:40"), "expected 11:40 in UTC; got: {out}");
    }

    #[test]
    fn template_uses_configured_timezone_not_utc() {
        // 1_779_277_200 = 2026-05-20 11:40:00 UTC → 13:40 in Europe/Rome (CEST, UTC+2)
        let reset = 1_779_277_200_i64;
        let now = reset - 60;
        let cfg = cfg_with_tz(Some("Europe/Rome"));
        let tz = resolve_timezone(&cfg);
        assert_eq!(tz.name(), "Europe/Rome");
        let ctx = ctx_for(reset, now, tz, "ambrogio");
        assert_eq!(
            ctx.reset_time, "13:40",
            "expected CEST 13:40, got {:?}",
            ctx
        );
        let rendered = render_rate_limit_template("Reset alle {reset_time} ({reset_tz})", &ctx);
        assert_eq!(rendered, "Reset alle 13:40 (Europe/Rome)");
    }

    #[test]
    fn unknown_timezone_falls_back_to_utc() {
        let cfg = cfg_with_tz(Some("Not/Real"));
        let tz = resolve_timezone(&cfg);
        assert_eq!(tz, Tz::UTC);
    }

    #[test]
    fn empty_or_none_timezone_is_utc() {
        assert_eq!(resolve_timezone(&cfg_with_tz(None)), Tz::UTC);
        assert_eq!(resolve_timezone(&cfg_with_tz(Some(""))), Tz::UTC);
    }

    #[test]
    fn unknown_placeholder_kept_literal() {
        let ctx = ctx_for(1_779_270_000, 1_779_270_000 - 60, Tz::UTC, "x");
        assert_eq!(
            render_rate_limit_template("hi {bogus} world", &ctx),
            "hi {bogus} world"
        );
        // Mixed known + unknown
        assert_eq!(
            render_rate_limit_template("{agent_name}: {wat}", &ctx),
            "x: {wat}"
        );
    }

    #[test]
    fn unicode_template_survives_byte_walk() {
        let ctx = ctx_for(1_779_270_000, 1_779_270_000 - 60, Tz::UTC, "ambrogio");
        // Italian + emoji + braces — the byte walker must not split a UTF-8 codepoint.
        let out = render_rate_limit_template(
            "Signore 🎩 — permesso fino alle {reset_time} ({reset_in_minutes} min)",
            &ctx,
        );
        assert!(out.contains("Signore 🎩"), "got: {out}");
        assert!(
            out.contains("1 min)") || out.contains("0 min)"),
            "got: {out}"
        );
    }

    #[test]
    fn resolve_config_prefers_per_agent_when_enabled() {
        let mut kernel = KernelConfig::default();
        kernel.rate_limit_notify = RateLimitNotifyConfig {
            enabled: true,
            template: Some("kernel-template".to_string()),
        };
        let mut manifest = AgentManifest::default();
        manifest.rate_limit_notify = Some(RateLimitNotifyConfig {
            enabled: true,
            template: Some("agent-template".to_string()),
        });
        let resolved = resolve_config(&manifest, &kernel);
        assert_eq!(resolved.template.as_deref(), Some("agent-template"));
    }

    #[test]
    fn resolve_config_falls_back_to_kernel_when_agent_disabled() {
        let mut kernel = KernelConfig::default();
        kernel.rate_limit_notify = RateLimitNotifyConfig {
            enabled: true,
            template: Some("kernel-template".to_string()),
        };
        let manifest = AgentManifest::default(); // enabled=false, template=None
        let resolved = resolve_config(&manifest, &kernel);
        assert_eq!(resolved.template.as_deref(), Some("kernel-template"));
        assert!(resolved.enabled);
    }

    #[test]
    fn resolve_config_explicit_disable_overrides_kernel_enabled() {
        let mut kernel = KernelConfig::default();
        kernel.rate_limit_notify = RateLimitNotifyConfig {
            enabled: true,
            template: Some("kernel-template".to_string()),
        };
        let mut manifest = AgentManifest::default();
        manifest.rate_limit_notify = Some(RateLimitNotifyConfig {
            enabled: false,
            template: None,
        });
        let resolved = resolve_config(&manifest, &kernel);
        assert!(
            !resolved.enabled,
            "explicit per-agent false must override kernel true"
        );
    }

    #[test]
    fn dedup_returns_true_first_then_false_within_window() {
        // Use a unique (agent, peer) namespace per test instead of
        // clearing the global LRU — cargo runs tests concurrently and
        // a shared reset would race against other tests' state.
        let reset = 1_779_270_000_i64;
        assert!(should_dispatch("dedup_test_1_agent", "peer1", reset));
        assert!(!should_dispatch("dedup_test_1_agent", "peer1", reset));
        // Different peer in same window → still dispatch
        assert!(should_dispatch("dedup_test_1_agent", "peer2", reset));
        // Same peer, different bucket → dispatch again
        assert!(should_dispatch(
            "dedup_test_1_agent",
            "peer1",
            reset + DEDUP_BUCKET_SECS * 2
        ));
    }

    #[test]
    fn dedup_buckets_within_5min_collapse() {
        let reset = 1_779_270_000_i64;
        assert!(should_dispatch("dedup_test_2_agent", "p", reset));
        // Same bucket: +200s (still under 300s bucket)
        assert!(!should_dispatch("dedup_test_2_agent", "p", reset + 200));
        // Next bucket: +500s
        assert!(should_dispatch("dedup_test_2_agent", "p", reset + 500));
    }

    #[test]
    fn parse_rate_limit_message_extracts_resets_at() {
        let msg = "Claude Code rate limit (five_hour). Resets at unix 1779282600 (UTC ISO 2026-05-20T14:50:00+00:00). resets_at_unix=1779282600 rate_limit_type=five_hour | stderr_text";
        assert_eq!(parse_rate_limit_message(msg), Some(1_779_282_600));
    }

    #[test]
    fn parse_rate_limit_message_returns_none_when_missing() {
        assert_eq!(parse_rate_limit_message("nothing here"), None);
        assert_eq!(
            parse_rate_limit_message("resets_at_unix=not_a_number tail"),
            None
        );
    }

    #[test]
    fn unix_to_tz_handles_dst_gap_without_panic() {
        // 2026-03-29 03:30 Europe/Rome is well past the DST jump — no
        // ambiguity, just verifying the helper doesn't drop on a real
        // CEST timestamp.
        let dt = unix_to_tz(1_743_213_000, chrono_tz::Europe::Rome);
        assert_eq!(dt.format("%H").to_string().len(), 2);
    }

    #[tokio::test]
    async fn dispatch_skipped_when_disabled() {
        let mut manifest = AgentManifest::default();
        manifest.name = "dispatch_disabled_agent".to_string();
        let kernel = KernelConfig::default(); // both disabled
        let sent = maybe_dispatch_owner_notify(
            &manifest,
            &kernel,
            Some("whatsapp"),
            Some("peer_disabled"),
            None,
            Some(1_779_277_200),
            300_000,
            Some(&StubSender::default()),
        )
        .await
        .unwrap();
        assert!(!sent);
    }

    #[tokio::test]
    async fn dispatch_skipped_when_no_channel() {
        let mut manifest = AgentManifest::default();
        manifest.name = "dispatch_nochannel_agent".to_string();
        manifest.rate_limit_notify = Some(RateLimitNotifyConfig {
            enabled: true,
            template: None,
        });
        let kernel = KernelConfig::default();
        let stub = StubSender::default();
        let sent = maybe_dispatch_owner_notify(
            &manifest,
            &kernel,
            None,
            Some("peer_nochannel"),
            None,
            Some(1_779_277_200),
            300_000,
            Some(&stub),
        )
        .await
        .unwrap();
        assert!(!sent);
        assert_eq!(stub.calls(), 0);
    }

    #[tokio::test]
    async fn dispatch_calls_sender_and_dedupes() {
        let mut manifest = AgentManifest::default();
        manifest.name = "dispatch_dedup_agent".to_string();
        manifest.rate_limit_notify = Some(RateLimitNotifyConfig {
            enabled: true,
            template: Some("⏸ resets {reset_time} ({reset_in_minutes}m)".into()),
        });
        let mut kernel = KernelConfig::default();
        kernel.system.timezone = Some("Europe/Rome".into());

        let stub = StubSender::default();
        let reset = chrono::Utc::now().timestamp() + 30 * 60;

        let sent1 = maybe_dispatch_owner_notify(
            &manifest,
            &kernel,
            Some("whatsapp"),
            Some("peer_dedup_unique"),
            Some("acct"),
            Some(reset),
            30 * 60 * 1000,
            Some(&stub),
        )
        .await
        .unwrap();
        assert!(sent1);
        assert_eq!(stub.calls(), 1);

        // Second call same peer, same reset → deduped, no extra send
        let sent2 = maybe_dispatch_owner_notify(
            &manifest,
            &kernel,
            Some("whatsapp"),
            Some("peer_dedup_unique"),
            Some("acct"),
            Some(reset),
            30 * 60 * 1000,
            Some(&stub),
        )
        .await
        .unwrap();
        assert!(!sent2);
        assert_eq!(stub.calls(), 1);

        let last = stub.last_message();
        assert!(
            last.contains("resets "),
            "rendered template missing 'resets ': {last}"
        );
        assert!(
            last.contains("m)"),
            "rendered template missing minutes: {last}"
        );
    }

    #[tokio::test]
    async fn dispatch_falls_back_to_retry_after_when_reset_missing() {
        let mut manifest = AgentManifest::default();
        manifest.name = "dispatch_fallback_agent".to_string();
        manifest.rate_limit_notify = Some(RateLimitNotifyConfig {
            enabled: true,
            template: Some("reset in {reset_in_minutes}m".into()),
        });
        let kernel = KernelConfig::default();
        let stub = StubSender::default();
        let sent = maybe_dispatch_owner_notify(
            &manifest,
            &kernel,
            Some("telegram"),
            Some("peer_fallback_unique"),
            None,
            None,
            10 * 60 * 1000, // 10 minutes
            Some(&stub),
        )
        .await
        .unwrap();
        assert!(sent);
        let msg = stub.last_message();
        // Minutes can be 9 or 10 depending on truncation of (now+600)−now.
        assert!(
            msg.contains("10m") || msg.contains("9m"),
            "expected ~10 minutes, got: {msg}"
        );
    }

    // ---- Test-only ChannelSender stub --------------------------------

    #[derive(Default)]
    struct StubSender {
        sent: std::sync::Mutex<Vec<(String, String, String)>>,
    }

    impl StubSender {
        fn calls(&self) -> usize {
            self.sent.lock().unwrap().len()
        }
        fn last_message(&self) -> String {
            self.sent
                .lock()
                .unwrap()
                .last()
                .map(|t| t.2.clone())
                .unwrap_or_default()
        }
    }

    #[async_trait::async_trait]
    impl ChannelSender for StubSender {
        async fn send_channel_message(
            &self,
            channel: &str,
            recipient: &str,
            message: &str,
            _thread_id: Option<&str>,
            _account_id: Option<&str>,
        ) -> Result<String, librefang_kernel_handle::KernelOpError> {
            self.sent.lock().unwrap().push((
                channel.to_string(),
                recipient.to_string(),
                message.to_string(),
            ));
            Ok("ok".to_string())
        }
    }
}
