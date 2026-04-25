//! Cron/scheduled job types for the LibreFang scheduler.
//!
//! Defines the core types for recurring and one-shot scheduled jobs that can
//! trigger agent turns, system events, or webhook deliveries.

use crate::agent::{AgentId, SessionMode};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Maximum number of scheduled jobs per agent.
pub const MAX_JOBS_PER_AGENT: usize = 50;

/// Maximum name length in characters.
const MAX_NAME_LEN: usize = 128;

/// Minimum interval for recurring jobs (seconds).
const MIN_EVERY_SECS: u64 = 60;

/// Maximum interval for recurring jobs (seconds) = 24 hours.
const MAX_EVERY_SECS: u64 = 86_400;

/// Maximum future horizon for one-shot `At` jobs (seconds) = 1 year.
const MAX_AT_HORIZON_SECS: i64 = 365 * 24 * 3600;

/// Maximum length of SystemEvent text.
const MAX_EVENT_TEXT_LEN: usize = 4096;

/// Maximum length of AgentTurn message.
const MAX_TURN_MESSAGE_LEN: usize = 16_384;

/// Maximum length of Workflow ID string.
const MAX_WORKFLOW_ID_LEN: usize = 256;

/// Minimum timeout for AgentTurn (seconds).
const MIN_TIMEOUT_SECS: u64 = 10;

/// Maximum timeout for AgentTurn (seconds).
const MAX_TIMEOUT_SECS: u64 = 600;

/// Maximum webhook URL length.
const MAX_WEBHOOK_URL_LEN: usize = 2048;

// ---------------------------------------------------------------------------
// CronJobId
// ---------------------------------------------------------------------------

/// Unique identifier for a scheduled job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CronJobId(pub Uuid);

impl CronJobId {
    /// Generate a new random CronJobId.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for CronJobId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for CronJobId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for CronJobId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

// ---------------------------------------------------------------------------
// CronSchedule
// ---------------------------------------------------------------------------

/// When a scheduled job fires.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CronSchedule {
    /// Fire once at a specific time.
    At {
        /// The exact UTC time to fire.
        at: DateTime<Utc>,
    },
    /// Fire on a fixed interval.
    Every {
        /// Interval in seconds (60..=86400).
        every_secs: u64,
    },
    /// Fire on a cron expression (5-field standard cron).
    Cron {
        /// Cron expression, e.g. `"0 9 * * 1-5"`.
        expr: String,
        /// Optional IANA timezone (e.g. `"America/New_York"`). Defaults to UTC.
        tz: Option<String>,
    },
}

// ---------------------------------------------------------------------------
// CronAction
// ---------------------------------------------------------------------------

/// What a scheduled job does when it fires.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CronAction {
    /// Publish a system event.
    SystemEvent {
        /// Event text/payload (max 4096 chars).
        text: String,
    },
    /// Trigger an agent conversation turn.
    AgentTurn {
        /// Message to send to the agent.
        message: String,
        /// Optional model override for this turn.
        model_override: Option<String>,
        /// Timeout in seconds (10..=600).
        timeout_secs: Option<u64>,
        /// Optional pre-check script path. The script runs before the agent turn;
        /// if its last non-empty stdout line is `{"wakeAgent": false}` the agent
        /// call is skipped entirely (no LLM spend). Any other output, non-JSON, or
        /// a missing `wakeAgent` key are treated as "wake normally".
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pre_check_script: Option<String>,
        /// Pre-processing script: the agent loop runs this argv before the
        /// scheduled prompt fires, captures stdout, and injects it into the
        /// LLM context. Use it to split deterministic data fetching (HTTP
        /// scrape, diff, computation) from the LLM reasoning step — saves
        /// tokens and reduces hallucination risk.
        ///
        /// Distinct from the existing `pre_check_script` field: that one
        /// gates whether the agent runs at all (via `{"wakeAgent": false}`),
        /// this one feeds *additional context* to a normally-firing run.
        /// Both can coexist.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pre_script: Option<PreScript>,
        /// Marker that, when present at the end of the agent's response,
        /// suppresses the delivery (no Telegram message, no email, no
        /// dashboard ping). Default is `"[SILENT]"`. Match is "last
        /// non-empty trimmed line == marker" — strict, won't trigger if
        /// the marker only appears mid-response.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        silent_marker: Option<String>,
    },
    /// Trigger a workflow execution by ID or name.
    Workflow {
        /// Workflow UUID or human-readable name.
        workflow_id: String,
        /// Optional input text passed to the first workflow step.
        #[serde(default)]
        input: Option<String>,
        /// Timeout in seconds (1..=3600). Defaults to 300 (5 min) if unset.
        #[serde(default)]
        timeout_secs: Option<u64>,
    },
}

// ---------------------------------------------------------------------------
// PreScript
// ---------------------------------------------------------------------------

/// Pre-processing script invocation spec.
///
/// `argv` is split form (no shell parsing). `argv[0]` MUST resolve
/// to a file under `<home_dir>/scripts/` after canonicalization —
/// the validator rejects absolute paths outside that allowlist or
/// relative paths that escape it via `..`.
///
/// `cwd` defaults to the agent's workspace; relative paths in `cwd`
/// are interpreted against the workspace root (resolved when the
/// scheduler dispatcher launches the script in M2).
///
/// `env` entries are added on top of the daemon's environment.
/// Use it to pass per-job secrets / endpoints; sensitive values
/// should still go through `LIBREFANG_VAULT_KEY` instead of plain
/// strings here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PreScript {
    /// Argv-split command. `argv[0]` is the executable, the rest are passed
    /// verbatim as arguments (no shell expansion).
    pub argv: Vec<String>,
    /// Working directory for the spawned process. Resolved by the dispatcher
    /// at execution time — the validator does not constrain `cwd` because
    /// agent workspaces are not visible at config-validation time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Extra environment variables layered on top of the daemon env. Empty
    /// by default; use sparingly — prefer the vault for secrets.
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub env: std::collections::HashMap<String, String>,
}

/// Errors returned by [`validate_pre_script`].
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PreScriptValidationError {
    /// `argv` was empty — there is no command to run.
    #[error("pre_script.argv must not be empty")]
    EmptyArgv,
    /// Resolved `argv[0]` canonicalized to a path outside the
    /// `<home_dir>/scripts/` allowlist.
    #[error(
        "pre_script binary `{path}` is outside the scripts allowlist (must be under {home_dir})"
    )]
    OutsideAllowlist {
        /// Resolved path that failed the allowlist check.
        path: String,
        /// The expected allowlist root (`<home_dir>/scripts`).
        home_dir: String,
    },
    /// Resolved `argv[0]` does not exist on disk (canonicalize failed).
    #[error("pre_script binary `{path}` does not exist")]
    NotFound {
        /// The path that could not be canonicalized.
        path: String,
    },
}

/// Validate a [`PreScript`] against the home-directory allowlist.
///
/// `home_dir` is the daemon home (typically `~/.librefang`). The script's
/// `argv[0]` must canonicalize to a path under `<home_dir>/scripts/`.
/// Relative paths are joined onto `<home_dir>/scripts/` first, then
/// canonicalized.
///
/// Rejects: empty argv, missing argv[0], paths that escape the scripts
/// dir via `..`, symlinks pointing outside the allowlist (canonicalize
/// follows symlinks). Component-level prefix comparison is used — a sibling
/// directory like `<home_dir>/scripts-other` will not satisfy the allowlist
/// even though its string representation shares the prefix.
pub fn validate_pre_script(
    script: &PreScript,
    home_dir: &std::path::Path,
) -> Result<(), PreScriptValidationError> {
    let argv0 = script
        .argv
        .first()
        .ok_or(PreScriptValidationError::EmptyArgv)?;
    if argv0.is_empty() {
        return Err(PreScriptValidationError::EmptyArgv);
    }

    let scripts_root = home_dir.join("scripts");
    let candidate = {
        let p = std::path::Path::new(argv0);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            scripts_root.join(p)
        }
    };

    // Canonicalize both sides so symlink targets and `..` traversal collapse
    // to their real on-disk paths before the prefix comparison runs.
    let resolved = candidate
        .canonicalize()
        .map_err(|_| PreScriptValidationError::NotFound {
            path: candidate.display().to_string(),
        })?;
    // The scripts root may not exist yet on a fresh install; treat that as
    // "nothing is on the allowlist", which makes any path fail allowlist.
    let allow_root =
        scripts_root
            .canonicalize()
            .map_err(|_| PreScriptValidationError::OutsideAllowlist {
                path: resolved.display().to_string(),
                home_dir: scripts_root.display().to_string(),
            })?;

    // Component-level prefix check rejects e.g. `/home/X-other` vs `/home/X`.
    if !resolved.starts_with(&allow_root) {
        return Err(PreScriptValidationError::OutsideAllowlist {
            path: resolved.display().to_string(),
            home_dir: allow_root.display().to_string(),
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// CronDelivery
// ---------------------------------------------------------------------------

/// Where the job's output is delivered.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CronDelivery {
    /// No delivery — fire and forget.
    None,
    /// Deliver to a specific channel and recipient.
    Channel {
        /// Channel identifier (e.g. `"telegram"`, `"slack"`).
        channel: String,
        /// Recipient in the channel.
        to: String,
    },
    /// Deliver to the last channel the agent interacted on.
    LastChannel,
    /// Deliver via HTTP webhook.
    Webhook {
        /// Webhook URL (must start with `http://` or `https://`).
        url: String,
    },
}

// ---------------------------------------------------------------------------
// CronDeliveryTarget (multi-destination fan-out)
// ---------------------------------------------------------------------------

/// A single destination for multi-destination cron output fan-out.
///
/// A cron job may declare zero or more `CronDeliveryTarget`s on its
/// `delivery_targets` field. When the job fires and produces output, the
/// delivery engine sends the same output to every target concurrently.
/// Failures in one target do not abort delivery to the others.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CronDeliveryTarget {
    /// Deliver via an existing channel adapter (Telegram/Slack/Discord/etc.).
    Channel {
        /// Which adapter to use (e.g. `"telegram"`, `"slack"`).
        channel_type: String,
        /// Platform-specific recipient (chat ID, user ID, etc.).
        recipient: String,
        /// Optional thread/topic id (e.g. Slack `thread_ts`, Telegram
        /// forum-topic id). Reserved up front so adding it later does not
        /// break persisted JSON. `None` preserves the historical behaviour.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
        /// Optional adapter-key suffix used to disambiguate multiple
        /// configured accounts of the same channel (e.g. two Slack
        /// workspaces). Resolved into the `<channel>:<account_id>` adapter
        /// lookup key by `LibreFangKernel::send_channel_message`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account_id: Option<String>,
    },
    /// Deliver via HTTP POST to a webhook URL with a JSON payload.
    Webhook {
        /// Destination URL (`http://` or `https://`).
        url: String,
        /// Optional `Authorization` header value sent verbatim.
        #[serde(default)]
        auth_header: Option<String>,
    },
    /// Append or overwrite a local file on disk.
    LocalFile {
        /// Absolute or relative path to the output file.
        path: String,
        /// If `true`, append to the file; if `false`, overwrite.
        #[serde(default)]
        append: bool,
    },
    /// Deliver via the existing email channel adapter.
    Email {
        /// Recipient email address.
        to: String,
        /// Optional subject template (e.g. `"Cron: {job}"`). Literal `{job}`
        /// placeholders are replaced with the job name at send time.
        #[serde(default)]
        subject_template: Option<String>,
    },
}

// ---------------------------------------------------------------------------
// CronJob
// ---------------------------------------------------------------------------

/// A scheduled job belonging to a specific agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    /// Unique job identifier.
    pub id: CronJobId,
    /// Owning agent.
    pub agent_id: AgentId,
    /// Human-readable name (max 128 chars, alphanumeric + spaces/hyphens/underscores).
    pub name: String,
    /// Whether the job is active.
    pub enabled: bool,
    /// When to fire.
    pub schedule: CronSchedule,
    /// What to do when fired.
    pub action: CronAction,
    /// Where to deliver the result (single legacy destination).
    pub delivery: CronDelivery,
    /// Additional fan-out destinations. May be empty; each target is
    /// delivered concurrently after the job produces its output. Failures in
    /// one target do not abort delivery to the others.
    #[serde(default)]
    pub delivery_targets: Vec<CronDeliveryTarget>,
    /// Optional peer/user ID to use as the `SenderContext.user_id` when the
    /// job fires. When set, memory lookups keyed by peer (e.g.
    /// `peer:{user_id}:KEY`) will resolve correctly. Defaults to `None`
    /// (empty user_id — backward-compatible behaviour).
    #[serde(default)]
    pub peer_id: Option<String>,
    /// Per-job session mode override.
    ///
    /// * `None` / `Some(Persistent)` — all fires for this job share one
    ///   dedicated session (`channel="cron"`), matching the historical
    ///   default.
    /// * `Some(New)` — every fire creates a fresh, isolated session so
    ///   history from previous fires cannot influence the current run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_mode: Option<SessionMode>,
    /// When the job was created.
    pub created_at: DateTime<Utc>,
    /// When the job last fired (if ever).
    pub last_run: Option<DateTime<Utc>>,
    /// When the job is next expected to fire.
    pub next_run: Option<DateTime<Utc>>,
}

impl CronJob {
    /// Validate this job's fields.
    ///
    /// `existing_count` is the number of jobs the owning agent already has
    /// (excluding this job if it already exists). Returns `Ok(())` or an
    /// error message describing the first validation failure.
    pub fn validate(&self, existing_count: usize) -> Result<(), String> {
        // -- job count cap --
        if existing_count >= MAX_JOBS_PER_AGENT {
            return Err(format!(
                "agent already has {existing_count} jobs (max {MAX_JOBS_PER_AGENT})"
            ));
        }

        // -- name --
        if self.name.is_empty() {
            return Err("name must not be empty".into());
        }
        if self.name.len() > MAX_NAME_LEN {
            return Err(format!(
                "name too long ({} chars, max {MAX_NAME_LEN})",
                self.name.len()
            ));
        }
        if !self
            .name
            .chars()
            .all(|c| c.is_alphanumeric() || c == ' ' || c == '-' || c == '_')
        {
            return Err(
                "name may only contain alphanumeric characters, spaces, hyphens, and underscores"
                    .into(),
            );
        }

        // -- schedule --
        self.validate_schedule()?;

        // -- action --
        self.validate_action()?;

        // -- delivery --
        self.validate_delivery()?;

        // -- delivery_targets (multi-destination fan-out) --
        self.validate_delivery_targets()?;

        Ok(())
    }

    fn validate_schedule(&self) -> Result<(), String> {
        match &self.schedule {
            CronSchedule::Every { every_secs } => {
                if *every_secs < MIN_EVERY_SECS {
                    return Err(format!(
                        "every_secs too small ({every_secs}, min {MIN_EVERY_SECS})"
                    ));
                }
                if *every_secs > MAX_EVERY_SECS {
                    return Err(format!(
                        "every_secs too large ({every_secs}, max {MAX_EVERY_SECS})"
                    ));
                }
            }
            CronSchedule::At { at } => {
                let now = Utc::now();
                if *at <= now {
                    return Err("scheduled time must be in the future".into());
                }
                let delta = (*at - now).num_seconds();
                if delta > MAX_AT_HORIZON_SECS {
                    return Err(format!(
                        "scheduled time too far in the future (max {MAX_AT_HORIZON_SECS}s / ~1 year)"
                    ));
                }
            }
            CronSchedule::Cron { expr, .. } => {
                validate_cron_expr(expr)?;
            }
        }
        Ok(())
    }

    fn validate_action(&self) -> Result<(), String> {
        match &self.action {
            CronAction::SystemEvent { text } => {
                if text.is_empty() {
                    return Err("system event text must not be empty".into());
                }
                if text.len() > MAX_EVENT_TEXT_LEN {
                    return Err(format!(
                        "system event text too long ({} chars, max {MAX_EVENT_TEXT_LEN})",
                        text.len()
                    ));
                }
            }
            CronAction::AgentTurn {
                message,
                timeout_secs,
                ..
            } => {
                if message.is_empty() {
                    return Err("agent turn message must not be empty".into());
                }
                if message.len() > MAX_TURN_MESSAGE_LEN {
                    return Err(format!(
                        "agent turn message too long ({} chars, max {MAX_TURN_MESSAGE_LEN})",
                        message.len()
                    ));
                }
                if let Some(t) = timeout_secs {
                    if *t < MIN_TIMEOUT_SECS {
                        return Err(format!(
                            "timeout_secs too small ({t}, min {MIN_TIMEOUT_SECS})"
                        ));
                    }
                    if *t > MAX_TIMEOUT_SECS {
                        return Err(format!(
                            "timeout_secs too large ({t}, max {MAX_TIMEOUT_SECS})"
                        ));
                    }
                }
            }
            CronAction::Workflow {
                workflow_id,
                timeout_secs,
                ..
            } => {
                if workflow_id.is_empty() {
                    return Err("workflow_id must not be empty".into());
                }
                if workflow_id.len() > MAX_WORKFLOW_ID_LEN {
                    return Err(format!(
                        "workflow_id too long ({} chars, max {MAX_WORKFLOW_ID_LEN})",
                        workflow_id.len()
                    ));
                }
                if let Some(t) = timeout_secs {
                    if *t == 0 {
                        return Err("workflow timeout_secs must be > 0".into());
                    }
                    if *t > 3600 {
                        return Err(format!("workflow timeout_secs too large ({t}, max 3600)"));
                    }
                }
            }
        }
        Ok(())
    }

    fn validate_delivery(&self) -> Result<(), String> {
        match &self.delivery {
            CronDelivery::Channel { channel, to } => {
                if channel.is_empty() {
                    return Err("delivery channel must not be empty".into());
                }
                if to.is_empty() {
                    return Err("delivery recipient must not be empty".into());
                }
            }
            CronDelivery::Webhook { url } => {
                if !url.starts_with("http://") && !url.starts_with("https://") {
                    return Err("webhook URL must start with http:// or https://".into());
                }
                if url.len() > MAX_WEBHOOK_URL_LEN {
                    return Err(format!(
                        "webhook URL too long ({} chars, max {MAX_WEBHOOK_URL_LEN})",
                        url.len()
                    ));
                }
            }
            CronDelivery::None | CronDelivery::LastChannel => {}
        }
        Ok(())
    }

    /// Validate the multi-destination `delivery_targets` list. Cron jobs are
    /// reachable through the LLM tool surface and the dashboard, so the
    /// path/host restrictions live here at the input boundary — by the
    /// time a target reaches `cron_delivery::deliver_*` we trust it.
    fn validate_delivery_targets(&self) -> Result<(), String> {
        for (i, target) in self.delivery_targets.iter().enumerate() {
            match target {
                CronDeliveryTarget::Channel {
                    channel_type,
                    recipient,
                    ..
                } => {
                    if channel_type.trim().is_empty() {
                        return Err(format!(
                            "delivery_targets[{i}]: channel_type must not be empty"
                        ));
                    }
                    if recipient.trim().is_empty() {
                        return Err(format!(
                            "delivery_targets[{i}]: recipient must not be empty"
                        ));
                    }
                }
                CronDeliveryTarget::Webhook { url, .. } => {
                    if !url.starts_with("http://") && !url.starts_with("https://") {
                        return Err(format!(
                            "delivery_targets[{i}]: webhook URL must start with http:// or https://"
                        ));
                    }
                    if url.len() > MAX_WEBHOOK_URL_LEN {
                        return Err(format!(
                            "delivery_targets[{i}]: webhook URL too long ({} chars, max {MAX_WEBHOOK_URL_LEN})",
                            url.len()
                        ));
                    }
                    // SSRF: refuse hosts that point at the daemon itself or
                    // at cloud metadata services. Best-effort URL parse —
                    // malformed URLs were already rejected by the scheme
                    // prefix check above.
                    if let Some(host) = extract_url_host(url) {
                        if is_blocked_webhook_host(&host) {
                            return Err(format!(
                                "delivery_targets[{i}]: webhook host '{host}' is not allowed (loopback / link-local / metadata service)"
                            ));
                        }
                    }
                }
                CronDeliveryTarget::LocalFile { path, .. } => {
                    if path.trim().is_empty() {
                        return Err(format!(
                            "delivery_targets[{i}]: file path must not be empty"
                        ));
                    }
                    let p = std::path::Path::new(path);
                    if p.is_absolute() {
                        return Err(format!(
                            "delivery_targets[{i}]: LocalFile path must be workspace-relative, not absolute ({path})"
                        ));
                    }
                    // Windows drive-letter form (`C:\...` or `C:/...`) is
                    // not flagged absolute on Unix — guard explicitly.
                    let bytes = path.as_bytes();
                    if bytes.len() >= 3
                        && bytes[0].is_ascii_alphabetic()
                        && bytes[1] == b':'
                        && (bytes[2] == b'\\' || bytes[2] == b'/')
                    {
                        return Err(format!(
                            "delivery_targets[{i}]: LocalFile path must be workspace-relative ({path})"
                        ));
                    }
                    if p.components()
                        .any(|c| matches!(c, std::path::Component::ParentDir))
                    {
                        return Err(format!(
                            "delivery_targets[{i}]: LocalFile path must not contain '..' ({path})"
                        ));
                    }
                }
                CronDeliveryTarget::Email { to, .. } => {
                    if to.trim().is_empty() {
                        return Err(format!(
                            "delivery_targets[{i}]: email recipient must not be empty"
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

/// Extract the lowercased host from a URL string. Returns `None` if the URL
/// cannot be parsed. Used by webhook SSRF validation.
fn extract_url_host(url: &str) -> Option<String> {
    // Strip scheme.
    let after_scheme = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))?;
    // Host runs until the next '/', '?', '#', or end-of-string.
    let host_end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    let host_part = &after_scheme[..host_end];
    // Strip optional userinfo (user:pass@host).
    let host_part = host_part.rsplit('@').next().unwrap_or(host_part);
    // Strip optional port. IPv6 addresses are wrapped in `[...]` so a colon
    // outside brackets is the port separator.
    let host = if let Some(stripped) = host_part.strip_prefix('[') {
        // IPv6 — keep the bracketed form for downstream matching.
        let close = stripped.find(']')?;
        &host_part[..=close + 1 - 1] // up to and including ']'
    } else if let Some(colon) = host_part.find(':') {
        &host_part[..colon]
    } else {
        host_part
    };
    if host.is_empty() {
        None
    } else {
        Some(host.to_ascii_lowercase())
    }
}

/// Hosts the daemon refuses to webhook to. Mirrors the dashboard editor
/// so users see a consistent rejection regardless of where the target
/// was created.
fn is_blocked_webhook_host(host: &str) -> bool {
    matches!(
        host,
        "localhost" | "metadata" | "metadata.google.internal" | "metadata.aws.amazon.com"
    ) || host.starts_with("127.")
        || host.starts_with("169.254.")
        || host == "[::1]"
        || host.starts_with("[fe80:")
        || host.starts_with("fe80:")
}

// ---------------------------------------------------------------------------
// Cron expression basic format validation
// ---------------------------------------------------------------------------

/// Basic cron expression format validation: must have exactly 5 whitespace-separated fields.
/// Actual parsing and scheduling is done in the kernel crate.
fn validate_cron_expr(expr: &str) -> Result<(), String> {
    let trimmed = expr.trim();
    if trimmed.is_empty() {
        return Err("cron expression must not be empty".into());
    }
    let fields: Vec<&str> = trimmed.split_whitespace().collect();
    if fields.len() != 5 {
        return Err(format!(
            "cron expression must have exactly 5 fields (got {}): \"{}\"",
            fields.len(),
            trimmed
        ));
    }
    // Basic character validation per field — allow digits, *, /, -, and ,.
    for (i, field) in fields.iter().enumerate() {
        if field.is_empty() {
            return Err(format!("cron field {i} is empty"));
        }
        if !field
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '*' | '/' | '-' | ',' | '?'))
        {
            return Err(format!(
                "cron field {i} contains invalid characters: \"{field}\""
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    /// Helper: build a minimal valid CronJob.
    fn valid_job() -> CronJob {
        CronJob {
            id: CronJobId::new(),
            agent_id: AgentId::new(),
            name: "daily-report".into(),
            enabled: true,
            schedule: CronSchedule::Every { every_secs: 3600 },
            action: CronAction::SystemEvent {
                text: "ping".into(),
            },
            delivery: CronDelivery::None,
            delivery_targets: Vec::new(),
            peer_id: None,
            session_mode: None,
            created_at: Utc::now(),
            last_run: None,
            next_run: None,
        }
    }

    // -- CronJobId --

    #[test]
    fn cron_job_id_display_roundtrip() {
        let id = CronJobId::new();
        let s = id.to_string();
        let parsed: CronJobId = s.parse().unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn cron_job_id_default() {
        let a = CronJobId::default();
        let b = CronJobId::default();
        assert_ne!(a, b);
    }

    // -- Valid job --

    #[test]
    fn valid_job_passes() {
        assert!(valid_job().validate(0).is_ok());
    }

    // -- Name validation --

    #[test]
    fn empty_name_rejected() {
        let mut job = valid_job();
        job.name = String::new();
        let err = job.validate(0).unwrap_err();
        assert!(err.contains("empty"), "{err}");
    }

    #[test]
    fn long_name_rejected() {
        let mut job = valid_job();
        job.name = "a".repeat(129);
        let err = job.validate(0).unwrap_err();
        assert!(err.contains("too long"), "{err}");
    }

    #[test]
    fn name_128_chars_ok() {
        let mut job = valid_job();
        job.name = "a".repeat(128);
        assert!(job.validate(0).is_ok());
    }

    #[test]
    fn name_special_chars_rejected() {
        let mut job = valid_job();
        job.name = "my job!".into();
        let err = job.validate(0).unwrap_err();
        assert!(err.contains("alphanumeric"), "{err}");
    }

    #[test]
    fn name_with_spaces_hyphens_underscores_ok() {
        let mut job = valid_job();
        job.name = "My Daily-Report_v2".into();
        assert!(job.validate(0).is_ok());
    }

    // -- Job count cap --

    #[test]
    fn max_jobs_rejected() {
        let job = valid_job();
        let err = job.validate(50).unwrap_err();
        assert!(err.contains("50"), "{err}");
    }

    #[test]
    fn under_max_jobs_ok() {
        let job = valid_job();
        assert!(job.validate(49).is_ok());
    }

    // -- Schedule: Every --

    #[test]
    fn every_too_small() {
        let mut job = valid_job();
        job.schedule = CronSchedule::Every { every_secs: 59 };
        let err = job.validate(0).unwrap_err();
        assert!(err.contains("too small"), "{err}");
    }

    #[test]
    fn every_too_large() {
        let mut job = valid_job();
        job.schedule = CronSchedule::Every { every_secs: 86_401 };
        let err = job.validate(0).unwrap_err();
        assert!(err.contains("too large"), "{err}");
    }

    #[test]
    fn every_min_boundary_ok() {
        let mut job = valid_job();
        job.schedule = CronSchedule::Every { every_secs: 60 };
        assert!(job.validate(0).is_ok());
    }

    #[test]
    fn every_max_boundary_ok() {
        let mut job = valid_job();
        job.schedule = CronSchedule::Every { every_secs: 86_400 };
        assert!(job.validate(0).is_ok());
    }

    // -- Schedule: At --

    #[test]
    fn at_in_past_rejected() {
        let mut job = valid_job();
        job.schedule = CronSchedule::At {
            at: Utc::now() - Duration::seconds(10),
        };
        let err = job.validate(0).unwrap_err();
        assert!(err.contains("future"), "{err}");
    }

    #[test]
    fn at_too_far_future_rejected() {
        let mut job = valid_job();
        job.schedule = CronSchedule::At {
            at: Utc::now() + Duration::days(366),
        };
        let err = job.validate(0).unwrap_err();
        assert!(err.contains("too far"), "{err}");
    }

    #[test]
    fn at_near_future_ok() {
        let mut job = valid_job();
        job.schedule = CronSchedule::At {
            at: Utc::now() + Duration::hours(1),
        };
        assert!(job.validate(0).is_ok());
    }

    // -- Schedule: Cron --

    #[test]
    fn cron_valid_expr() {
        let mut job = valid_job();
        job.schedule = CronSchedule::Cron {
            expr: "0 9 * * 1-5".into(),
            tz: Some("America/New_York".into()),
        };
        assert!(job.validate(0).is_ok());
    }

    #[test]
    fn cron_empty_expr() {
        let mut job = valid_job();
        job.schedule = CronSchedule::Cron {
            expr: String::new(),
            tz: None,
        };
        let err = job.validate(0).unwrap_err();
        assert!(err.contains("empty"), "{err}");
    }

    #[test]
    fn cron_wrong_field_count() {
        let mut job = valid_job();
        job.schedule = CronSchedule::Cron {
            expr: "0 9 * *".into(),
            tz: None,
        };
        let err = job.validate(0).unwrap_err();
        assert!(err.contains("5 fields"), "{err}");
    }

    #[test]
    fn cron_invalid_chars() {
        let mut job = valid_job();
        job.schedule = CronSchedule::Cron {
            expr: "0 9 * * MON".into(),
            tz: None,
        };
        let err = job.validate(0).unwrap_err();
        assert!(err.contains("invalid characters"), "{err}");
    }

    // -- Action: SystemEvent --

    #[test]
    fn system_event_empty_text() {
        let mut job = valid_job();
        job.action = CronAction::SystemEvent {
            text: String::new(),
        };
        let err = job.validate(0).unwrap_err();
        assert!(err.contains("empty"), "{err}");
    }

    #[test]
    fn system_event_text_too_long() {
        let mut job = valid_job();
        job.action = CronAction::SystemEvent {
            text: "x".repeat(4097),
        };
        let err = job.validate(0).unwrap_err();
        assert!(err.contains("too long"), "{err}");
    }

    #[test]
    fn system_event_max_text_ok() {
        let mut job = valid_job();
        job.action = CronAction::SystemEvent {
            text: "x".repeat(4096),
        };
        assert!(job.validate(0).is_ok());
    }

    // -- Action: AgentTurn --

    #[test]
    fn agent_turn_empty_message() {
        let mut job = valid_job();
        job.action = CronAction::AgentTurn {
            message: String::new(),
            model_override: None,
            timeout_secs: None,
            pre_check_script: None,
            pre_script: None,
            silent_marker: None,
        };
        let err = job.validate(0).unwrap_err();
        assert!(err.contains("empty"), "{err}");
    }

    #[test]
    fn agent_turn_message_too_long() {
        let mut job = valid_job();
        job.action = CronAction::AgentTurn {
            message: "x".repeat(16_385),
            model_override: None,
            timeout_secs: None,
            pre_check_script: None,
            pre_script: None,
            silent_marker: None,
        };
        let err = job.validate(0).unwrap_err();
        assert!(err.contains("too long"), "{err}");
    }

    #[test]
    fn agent_turn_timeout_too_small() {
        let mut job = valid_job();
        job.action = CronAction::AgentTurn {
            message: "hello".into(),
            model_override: None,
            timeout_secs: Some(9),
            pre_check_script: None,
            pre_script: None,
            silent_marker: None,
        };
        let err = job.validate(0).unwrap_err();
        assert!(err.contains("too small"), "{err}");
    }

    #[test]
    fn agent_turn_timeout_too_large() {
        let mut job = valid_job();
        job.action = CronAction::AgentTurn {
            message: "hello".into(),
            model_override: None,
            timeout_secs: Some(601),
            pre_check_script: None,
            pre_script: None,
            silent_marker: None,
        };
        let err = job.validate(0).unwrap_err();
        assert!(err.contains("too large"), "{err}");
    }

    #[test]
    fn agent_turn_timeout_boundaries_ok() {
        let mut job = valid_job();
        job.action = CronAction::AgentTurn {
            message: "hello".into(),
            model_override: Some("claude-haiku-4-5-20251001".into()),
            timeout_secs: Some(10),
            pre_check_script: None,
            pre_script: None,
            silent_marker: None,
        };
        assert!(job.validate(0).is_ok());

        job.action = CronAction::AgentTurn {
            message: "hello".into(),
            model_override: None,
            timeout_secs: Some(600),
            pre_check_script: None,
            pre_script: None,
            silent_marker: None,
        };
        assert!(job.validate(0).is_ok());
    }

    #[test]
    fn agent_turn_no_timeout_ok() {
        let mut job = valid_job();
        job.action = CronAction::AgentTurn {
            message: "hello".into(),
            model_override: None,
            timeout_secs: None,
            pre_check_script: None,
            pre_script: None,
            silent_marker: None,
        };
        assert!(job.validate(0).is_ok());
    }

    // -- Delivery: Channel --

    #[test]
    fn delivery_channel_empty_channel() {
        let mut job = valid_job();
        job.delivery = CronDelivery::Channel {
            channel: String::new(),
            to: "user123".into(),
        };
        let err = job.validate(0).unwrap_err();
        assert!(err.contains("channel must not be empty"), "{err}");
    }

    #[test]
    fn delivery_channel_empty_to() {
        let mut job = valid_job();
        job.delivery = CronDelivery::Channel {
            channel: "slack".into(),
            to: String::new(),
        };
        let err = job.validate(0).unwrap_err();
        assert!(err.contains("recipient must not be empty"), "{err}");
    }

    #[test]
    fn delivery_channel_ok() {
        let mut job = valid_job();
        job.delivery = CronDelivery::Channel {
            channel: "telegram".into(),
            to: "chat_12345".into(),
        };
        assert!(job.validate(0).is_ok());
    }

    // -- Delivery: Webhook --

    #[test]
    fn webhook_bad_scheme() {
        let mut job = valid_job();
        job.delivery = CronDelivery::Webhook {
            url: "ftp://example.com/hook".into(),
        };
        let err = job.validate(0).unwrap_err();
        assert!(err.contains("http://"), "{err}");
    }

    #[test]
    fn webhook_too_long() {
        let mut job = valid_job();
        job.delivery = CronDelivery::Webhook {
            url: format!("https://example.com/{}", "a".repeat(2048)),
        };
        let err = job.validate(0).unwrap_err();
        assert!(err.contains("too long"), "{err}");
    }

    #[test]
    fn webhook_http_ok() {
        let mut job = valid_job();
        job.delivery = CronDelivery::Webhook {
            url: "http://localhost:8080/hook".into(),
        };
        assert!(job.validate(0).is_ok());
    }

    #[test]
    fn webhook_https_ok() {
        let mut job = valid_job();
        job.delivery = CronDelivery::Webhook {
            url: "https://example.com/hook".into(),
        };
        assert!(job.validate(0).is_ok());
    }

    // -- Delivery: None / LastChannel --

    #[test]
    fn delivery_none_ok() {
        let mut job = valid_job();
        job.delivery = CronDelivery::None;
        assert!(job.validate(0).is_ok());
    }

    #[test]
    fn delivery_last_channel_ok() {
        let mut job = valid_job();
        job.delivery = CronDelivery::LastChannel;
        assert!(job.validate(0).is_ok());
    }

    // -- Serde roundtrip --

    #[test]
    fn serde_roundtrip_every() {
        let job = valid_job();
        let json = serde_json::to_string(&job).unwrap();
        let back: CronJob = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, job.name);
        assert_eq!(back.id, job.id);
    }

    #[test]
    fn serde_roundtrip_cron_schedule() {
        let schedule = CronSchedule::Cron {
            expr: "*/5 * * * *".into(),
            tz: Some("UTC".into()),
        };
        let json = serde_json::to_string(&schedule).unwrap();
        assert!(json.contains("\"kind\":\"cron\""));
        let back: CronSchedule = serde_json::from_str(&json).unwrap();
        if let CronSchedule::Cron { expr, tz } = back {
            assert_eq!(expr, "*/5 * * * *");
            assert_eq!(tz, Some("UTC".into()));
        } else {
            panic!("expected Cron variant");
        }
    }

    #[test]
    fn serde_action_tags() {
        let action = CronAction::AgentTurn {
            message: "hi".into(),
            model_override: None,
            timeout_secs: Some(30),
            pre_check_script: None,
            pre_script: None,
            silent_marker: None,
        };
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains("\"kind\":\"agent_turn\""));
    }

    #[test]
    fn serde_delivery_tags() {
        let d = CronDelivery::LastChannel;
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains("\"kind\":\"last_channel\""));

        let d2 = CronDelivery::Webhook {
            url: "https://x.com".into(),
        };
        let json2 = serde_json::to_string(&d2).unwrap();
        assert!(json2.contains("\"kind\":\"webhook\""));
    }

    // -- Cron expression edge cases --

    #[test]
    fn cron_extra_whitespace_ok() {
        let mut job = valid_job();
        job.schedule = CronSchedule::Cron {
            expr: "  0  9  *  *  *  ".into(),
            tz: None,
        };
        assert!(job.validate(0).is_ok());
    }

    #[test]
    fn cron_six_fields_rejected() {
        let mut job = valid_job();
        job.schedule = CronSchedule::Cron {
            expr: "0 0 9 * * 1".into(),
            tz: None,
        };
        let err = job.validate(0).unwrap_err();
        assert!(err.contains("5 fields"), "{err}");
    }

    #[test]
    fn cron_slash_and_comma_ok() {
        let mut job = valid_job();
        job.schedule = CronSchedule::Cron {
            expr: "*/15 0,12 1-15 * 1,3,5".into(),
            tz: None,
        };
        assert!(job.validate(0).is_ok());
    }

    // -- Action: Workflow --

    #[test]
    fn workflow_action_valid_uuid() {
        let mut job = valid_job();
        job.action = CronAction::Workflow {
            workflow_id: "550e8400-e29b-41d4-a716-446655440000".into(),
            input: None,
            timeout_secs: None,
        };
        assert!(job.validate(0).is_ok());
    }

    #[test]
    fn workflow_action_valid_name() {
        let mut job = valid_job();
        job.action = CronAction::Workflow {
            workflow_id: "daily-report-pipeline".into(),
            input: Some("generate report".into()),
            timeout_secs: None,
        };
        assert!(job.validate(0).is_ok());
    }

    #[test]
    fn workflow_action_empty_id() {
        let mut job = valid_job();
        job.action = CronAction::Workflow {
            workflow_id: String::new(),
            input: None,
            timeout_secs: None,
        };
        let err = job.validate(0).unwrap_err();
        assert!(err.contains("empty"), "{err}");
    }

    #[test]
    fn workflow_action_id_too_long() {
        let mut job = valid_job();
        job.action = CronAction::Workflow {
            workflow_id: "x".repeat(257),
            input: None,
            timeout_secs: None,
        };
        let err = job.validate(0).unwrap_err();
        assert!(err.contains("too long"), "{err}");
    }

    #[test]
    fn workflow_action_timeout_valid() {
        let mut job = valid_job();
        job.action = CronAction::Workflow {
            workflow_id: "my-workflow".into(),
            input: None,
            timeout_secs: Some(60),
        };
        assert!(job.validate(0).is_ok());
    }

    #[test]
    fn workflow_action_timeout_max_boundary() {
        let mut job = valid_job();
        job.action = CronAction::Workflow {
            workflow_id: "my-workflow".into(),
            input: None,
            timeout_secs: Some(3600),
        };
        assert!(job.validate(0).is_ok());
    }

    #[test]
    fn workflow_action_timeout_zero_rejected() {
        let mut job = valid_job();
        job.action = CronAction::Workflow {
            workflow_id: "my-workflow".into(),
            input: None,
            timeout_secs: Some(0),
        };
        let err = job.validate(0).unwrap_err();
        assert!(err.contains("must be > 0"), "{err}");
    }

    #[test]
    fn workflow_action_timeout_too_large() {
        let mut job = valid_job();
        job.action = CronAction::Workflow {
            workflow_id: "my-workflow".into(),
            input: None,
            timeout_secs: Some(3601),
        };
        let err = job.validate(0).unwrap_err();
        assert!(err.contains("too large"), "{err}");
    }

    #[test]
    fn serde_workflow_action_tag() {
        let action = CronAction::Workflow {
            workflow_id: "my-workflow".into(),
            input: Some("hello".into()),
            timeout_secs: None,
        };
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains("\"kind\":\"workflow\""));
        assert!(json.contains("\"workflow_id\":\"my-workflow\""));
        assert!(json.contains("\"input\":\"hello\""));
    }

    #[test]
    fn serde_workflow_action_roundtrip() {
        let action = CronAction::Workflow {
            workflow_id: "550e8400-e29b-41d4-a716-446655440000".into(),
            input: None,
            timeout_secs: Some(120),
        };
        let json = serde_json::to_string(&action).unwrap();
        let back: CronAction = serde_json::from_str(&json).unwrap();
        if let CronAction::Workflow {
            workflow_id,
            input,
            timeout_secs,
        } = back
        {
            assert_eq!(workflow_id, "550e8400-e29b-41d4-a716-446655440000");
            assert!(input.is_none());
            assert_eq!(timeout_secs, Some(120));
        } else {
            panic!("expected Workflow variant");
        }
    }

    // -- CronDeliveryTarget serde --

    #[test]
    fn delivery_target_channel_roundtrip() {
        let t = CronDeliveryTarget::Channel {
            channel_type: "telegram".into(),
            recipient: "12345".into(),
            thread_id: None,
            account_id: None,
        };
        let s = serde_json::to_string(&t).unwrap();
        assert!(s.contains("\"type\":\"channel\""), "tag missing: {s}");
        assert!(s.contains("telegram"));
        // `skip_serializing_if = "Option::is_none"` keeps the wire shape
        // identical to the pre-thread_id payload so old persisted JSON
        // round-trips unchanged.
        assert!(!s.contains("thread_id"), "thread_id leaked when None: {s}");
        assert!(
            !s.contains("account_id"),
            "account_id leaked when None: {s}"
        );
        let back: CronDeliveryTarget = serde_json::from_str(&s).unwrap();
        assert_eq!(t, back);
    }

    #[test]
    fn delivery_target_channel_with_thread_and_account_roundtrip() {
        let t = CronDeliveryTarget::Channel {
            channel_type: "slack".into(),
            recipient: "C123".into(),
            thread_id: Some("1700000000.000100".into()),
            account_id: Some("workspace-b".into()),
        };
        let s = serde_json::to_string(&t).unwrap();
        assert!(s.contains("\"thread_id\":\"1700000000.000100\""));
        assert!(s.contains("\"account_id\":\"workspace-b\""));
        let back: CronDeliveryTarget = serde_json::from_str(&s).unwrap();
        assert_eq!(t, back);
    }

    #[test]
    fn delivery_target_channel_legacy_payload_deserializes() {
        // Pre-existing persisted payloads without thread_id/account_id must
        // still deserialize cleanly thanks to #[serde(default)].
        let json = r#"{"type":"channel","channel_type":"telegram","recipient":"12345"}"#;
        let back: CronDeliveryTarget = serde_json::from_str(json).unwrap();
        assert_eq!(
            back,
            CronDeliveryTarget::Channel {
                channel_type: "telegram".into(),
                recipient: "12345".into(),
                thread_id: None,
                account_id: None,
            }
        );
    }

    #[test]
    fn delivery_target_webhook_roundtrip() {
        let t = CronDeliveryTarget::Webhook {
            url: "https://example.com/hook".into(),
            auth_header: Some("Bearer x".into()),
        };
        let s = serde_json::to_string(&t).unwrap();
        assert!(s.contains("\"type\":\"webhook\""), "tag missing: {s}");
        let back: CronDeliveryTarget = serde_json::from_str(&s).unwrap();
        assert_eq!(t, back);
    }

    #[test]
    fn delivery_target_webhook_default_no_auth() {
        // auth_header should default to None when omitted.
        let json = r#"{"type":"webhook","url":"https://x.test/h"}"#;
        let back: CronDeliveryTarget = serde_json::from_str(json).unwrap();
        assert_eq!(
            back,
            CronDeliveryTarget::Webhook {
                url: "https://x.test/h".into(),
                auth_header: None,
            }
        );
    }

    #[test]
    fn delivery_target_localfile_roundtrip() {
        let t = CronDeliveryTarget::LocalFile {
            path: "/var/log/cron-out.log".into(),
            append: true,
        };
        let s = serde_json::to_string(&t).unwrap();
        assert!(s.contains("\"type\":\"local_file\""), "tag missing: {s}");
        let back: CronDeliveryTarget = serde_json::from_str(&s).unwrap();
        assert_eq!(t, back);
    }

    #[test]
    fn delivery_target_localfile_default_append() {
        // append should default to false when omitted.
        let json = r#"{"type":"local_file","path":"/tmp/out.log"}"#;
        let back: CronDeliveryTarget = serde_json::from_str(json).unwrap();
        assert_eq!(
            back,
            CronDeliveryTarget::LocalFile {
                path: "/tmp/out.log".into(),
                append: false,
            }
        );
    }

    #[test]
    fn delivery_target_email_roundtrip() {
        let t = CronDeliveryTarget::Email {
            to: "alice@example.com".into(),
            subject_template: Some("Report: {job}".into()),
        };
        let s = serde_json::to_string(&t).unwrap();
        assert!(s.contains("\"type\":\"email\""), "tag missing: {s}");
        let back: CronDeliveryTarget = serde_json::from_str(&s).unwrap();
        assert_eq!(t, back);
    }

    #[test]
    fn cron_job_delivery_targets_default_empty() {
        // Old persisted JSON without `delivery_targets` must still deserialize
        // (the field has `#[serde(default)]`).
        let json = serde_json::to_string(&valid_job()).unwrap();
        // Strip the field to simulate an older payload.
        let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
        v.as_object_mut().unwrap().remove("delivery_targets");
        let stripped = serde_json::to_string(&v).unwrap();
        let back: CronJob = serde_json::from_str(&stripped).unwrap();
        assert!(back.delivery_targets.is_empty());
    }

    #[test]
    fn cron_job_with_delivery_targets_roundtrip() {
        let mut job = valid_job();
        job.delivery_targets = vec![
            CronDeliveryTarget::Channel {
                channel_type: "slack".into(),
                recipient: "C123".into(),
                thread_id: None,
                account_id: None,
            },
            CronDeliveryTarget::LocalFile {
                path: "/tmp/out.log".into(),
                append: true,
            },
        ];
        let json = serde_json::to_string(&job).unwrap();
        let back: CronJob = serde_json::from_str(&json).unwrap();
        assert_eq!(back.delivery_targets.len(), 2);
        assert_eq!(back.delivery_targets, job.delivery_targets);
    }

    // -- PreScript validation --

    /// Build a temp daemon home with `scripts/foo.sh` so the allowlist
    /// check has a real on-disk anchor to canonicalize against.
    fn fixture_home_with_script(name: &str) -> tempfile::TempDir {
        let home = tempfile::tempdir().expect("tempdir");
        let scripts = home.path().join("scripts");
        std::fs::create_dir_all(&scripts).unwrap();
        let script_path = scripts.join(name);
        std::fs::write(&script_path, "#!/bin/sh\necho hello\n").unwrap();
        // Make it executable on Unix; on Windows the perm bits are no-ops
        // but the validator only checks path location, not the +x bit.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script_path, perms).unwrap();
        }
        home
    }

    #[test]
    fn pre_script_validation_accepts_path_under_scripts_dir() {
        let home = fixture_home_with_script("foo.sh");
        let script = PreScript {
            argv: vec!["foo.sh".into(), "--flag".into()],
            cwd: None,
            env: Default::default(),
        };
        validate_pre_script(&script, home.path()).expect("relative path under scripts/ accepted");

        // Absolute path under the same root must also pass.
        let abs = home.path().join("scripts").join("foo.sh");
        let script_abs = PreScript {
            argv: vec![abs.to_string_lossy().into_owned()],
            cwd: None,
            env: Default::default(),
        };
        validate_pre_script(&script_abs, home.path()).expect("absolute path under scripts/ ok");
    }

    #[test]
    fn pre_script_validation_rejects_outside_allowlist() {
        let home = fixture_home_with_script("foo.sh");
        // Pick a path that exists on every supported platform so we hit
        // the allowlist branch rather than NotFound.
        #[cfg(unix)]
        let outside = "/bin/sh";
        #[cfg(not(unix))]
        let outside =
            std::env::var("COMSPEC").unwrap_or_else(|_| "C:\\Windows\\System32\\cmd.exe".into());
        let script = PreScript {
            argv: vec![outside.to_string()],
            cwd: None,
            env: Default::default(),
        };
        match validate_pre_script(&script, home.path()) {
            Err(PreScriptValidationError::OutsideAllowlist { .. }) => {}
            other => panic!("expected OutsideAllowlist, got {other:?}"),
        }
    }

    #[test]
    fn pre_script_validation_rejects_dot_dot_escape() {
        let home = fixture_home_with_script("foo.sh");
        // `scripts/../foo.sh` would resolve to `<home>/foo.sh`, which is
        // outside `<home>/scripts/`. Place a real file there to make sure
        // the rejection comes from the allowlist check, not NotFound.
        std::fs::write(home.path().join("escape.sh"), "#!/bin/sh\n").unwrap();
        let script = PreScript {
            argv: vec!["../escape.sh".into()],
            cwd: None,
            env: Default::default(),
        };
        match validate_pre_script(&script, home.path()) {
            Err(PreScriptValidationError::OutsideAllowlist { .. }) => {}
            other => panic!("expected OutsideAllowlist for `..` escape, got {other:?}"),
        }
    }

    #[test]
    fn pre_script_validation_rejects_empty_argv() {
        let home = fixture_home_with_script("foo.sh");
        let script = PreScript {
            argv: vec![],
            cwd: None,
            env: Default::default(),
        };
        assert_eq!(
            validate_pre_script(&script, home.path()),
            Err(PreScriptValidationError::EmptyArgv)
        );

        // Also reject an explicit empty string at argv[0].
        let script2 = PreScript {
            argv: vec![String::new()],
            cwd: None,
            env: Default::default(),
        };
        assert_eq!(
            validate_pre_script(&script2, home.path()),
            Err(PreScriptValidationError::EmptyArgv)
        );
    }

    #[test]
    fn pre_script_validation_rejects_nonexistent_path() {
        let home = fixture_home_with_script("foo.sh");
        let script = PreScript {
            argv: vec!["does-not-exist.sh".into()],
            cwd: None,
            env: Default::default(),
        };
        match validate_pre_script(&script, home.path()) {
            Err(PreScriptValidationError::NotFound { .. }) => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    // -- CronAction::AgentTurn serde compat --

    #[test]
    fn cron_action_agent_turn_serde_round_trip_with_pre_script() {
        // Full round-trip via TOML to cover the user-facing config format,
        // plus JSON for the persisted DashMap path.
        let mut env = std::collections::HashMap::new();
        env.insert("FOO".into(), "bar".into());
        let action = CronAction::AgentTurn {
            message: "hello".into(),
            model_override: None,
            timeout_secs: Some(60),
            pre_check_script: None,
            pre_script: Some(PreScript {
                argv: vec!["scripts/poll.sh".into(), "--once".into()],
                cwd: Some("/tmp/work".into()),
                env,
            }),
            silent_marker: Some("[QUIET]".into()),
        };
        let toml_str = toml::to_string(&action).unwrap();
        let back_toml: CronAction = toml::from_str(&toml_str).unwrap();
        let json_str = serde_json::to_string(&action).unwrap();
        let back_json: CronAction = serde_json::from_str(&json_str).unwrap();
        for back in [back_toml, back_json] {
            match back {
                CronAction::AgentTurn {
                    pre_script: Some(ps),
                    silent_marker: Some(sm),
                    ..
                } => {
                    assert_eq!(ps.argv, vec!["scripts/poll.sh", "--once"]);
                    assert_eq!(ps.cwd.as_deref(), Some("/tmp/work"));
                    assert_eq!(ps.env.get("FOO").map(String::as_str), Some("bar"));
                    assert_eq!(sm, "[QUIET]");
                }
                other => panic!("round-trip lost pre_script/silent_marker: {other:?}"),
            }
        }
    }

    #[test]
    fn cron_action_agent_turn_serde_compat_no_pre_script() {
        // Pre-existing config payloads (no pre_script / silent_marker keys)
        // must still deserialize cleanly thanks to #[serde(default)].
        let json = r#"{
            "kind": "agent_turn",
            "message": "hello",
            "model_override": null,
            "timeout_secs": null
        }"#;
        let back: CronAction = serde_json::from_str(json).unwrap();
        match back {
            CronAction::AgentTurn {
                pre_script,
                silent_marker,
                ..
            } => {
                assert!(pre_script.is_none());
                assert!(silent_marker.is_none());
            }
            other => panic!("expected AgentTurn, got {other:?}"),
        }
    }

    #[test]
    fn silent_marker_default_via_serde_skip() {
        // `silent_marker: None` must NOT appear in serialized output so the
        // wire shape stays identical to legacy payloads.
        let action = CronAction::AgentTurn {
            message: "hi".into(),
            model_override: None,
            timeout_secs: None,
            pre_check_script: None,
            pre_script: None,
            silent_marker: None,
        };
        let json = serde_json::to_string(&action).unwrap();
        assert!(
            !json.contains("silent_marker"),
            "silent_marker leaked when None: {json}"
        );
        assert!(
            !json.contains("pre_script"),
            "pre_script leaked when None: {json}"
        );
    }
}
