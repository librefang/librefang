//! Cron/scheduled job types for the LibreFang scheduler.
//!
//! Defines the core types for recurring and one-shot scheduled jobs that can
//! trigger agent turns, system events, or webhook deliveries.

use crate::agent::{AgentId, SessionMode};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
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
    /// `env` contains a key from the dangerous-keys denylist (e.g.
    /// `LD_PRELOAD`, `PATH`) that would defeat the path allowlist.
    #[error("pre_script.env contains dangerous key `{key}` that would defeat the path allowlist")]
    DangerousEnvKey {
        /// The offending env key (preserved in original case for the user).
        key: String,
    },
}

/// Environment variable keys that, if attacker-controlled, defeat the
/// path allowlist on `argv[0]`. Setting any of these in `PreScript.env`
/// is rejected by `validate_pre_script`.
///
/// References:
/// - `LD_PRELOAD` / `LD_LIBRARY_PATH` / `LD_AUDIT` — glibc dynamic linker
///   inject arbitrary code into the spawned process.
/// - `DYLD_INSERT_LIBRARIES` / `DYLD_LIBRARY_PATH` / `DYLD_FALLBACK_LIBRARY_PATH`
///   — Darwin equivalent.
/// - `PATH` — `argv[0]` is allowlisted but PATH-rewriting hijacks any
///   `subprocess.Popen("subcmd")` style call inside the script.
/// - `IFS` — POSIX shell field-splitter rewrite.
const DANGEROUS_ENV_KEYS: &[&str] = &[
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "LD_AUDIT",
    "DYLD_INSERT_LIBRARIES",
    "DYLD_LIBRARY_PATH",
    "DYLD_FALLBACK_LIBRARY_PATH",
    "PATH",
    "IFS",
];

/// Returns true if `key` matches one of [`DANGEROUS_ENV_KEYS`] under an
/// ASCII case-insensitive comparison.
///
/// Linux/macOS env keys are case-sensitive, but the danger keys are
/// canonical uppercase; matching case-insensitively is defence-in-depth
/// (a subprocess might lowercase / mixed-case the key) and matches
/// Windows env semantics where keys are case-insensitive.
fn is_dangerous_env_key(key: &str) -> bool {
    DANGEROUS_ENV_KEYS
        .iter()
        .any(|k| k.eq_ignore_ascii_case(key))
}

/// Reject any [`PreScript`] whose `env` contains a key on the
/// dangerous-keys denylist. Path-independent: callable from contexts
/// where the daemon home directory is not available.
pub fn validate_pre_script_env(script: &PreScript) -> Result<(), PreScriptValidationError> {
    for key in script.env.keys() {
        if is_dangerous_env_key(key) {
            return Err(PreScriptValidationError::DangerousEnvKey { key: key.clone() });
        }
    }
    Ok(())
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
    // Reject dangerous env keys before touching the filesystem — fast fail and
    // keeps the security check independent of the home directory existing.
    validate_pre_script_env(script)?;

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
    ///
    /// Forwards to [`CronJob::validate_with_home`] with `home_dir = None`,
    /// meaning the dangerous-env-key denylist is enforced but the
    /// `<home_dir>/scripts/` path allowlist is skipped (callers without
    /// a daemon home directory in hand still get the security floor).
    pub fn validate(&self, existing_count: usize) -> Result<(), String> {
        self.validate_with_home(existing_count, None)
    }

    /// Same as [`CronJob::validate`] but additionally enforces the
    /// `<home_dir>/scripts/` path allowlist on any `pre_script.argv[0]`.
    /// Pass the daemon home (typically `~/.librefang`) as `home_dir` from
    /// production code paths; pass `None` only from tests or contexts
    /// where the path check is intentionally deferred.
    pub fn validate_with_home(
        &self,
        existing_count: usize,
        home_dir: Option<&std::path::Path>,
    ) -> Result<(), String> {
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
        self.validate_action(home_dir)?;

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

    fn validate_action(&self, home_dir: Option<&std::path::Path>) -> Result<(), String> {
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
                pre_script,
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
                // pre_script: env denylist always runs; path allowlist only
                // runs when the caller supplied a daemon home directory.
                if let Some(ps) = pre_script {
                    if let Some(home) = home_dir {
                        validate_pre_script(ps, home).map_err(|e| e.to_string())?;
                    } else {
                        validate_pre_script_env(ps).map_err(|e| e.to_string())?;
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
        validate_cron_delivery(&self.delivery)
    }

    /// Validate the multi-destination `delivery_targets` list. Cron jobs are
    /// reachable through the LLM tool surface and the dashboard, so the
    /// path/host restrictions live here at the input boundary — by the
    /// time a target reaches `cron_delivery::deliver_*` we trust it.
    fn validate_delivery_targets(&self) -> Result<(), String> {
        validate_cron_delivery_targets(&self.delivery_targets)
    }
}

/// Validate a single [`CronDelivery`]. Exposed as a free function so the
/// kernel `update_job` / `set_delivery_targets` paths can re-run the same
/// check on an in-place mutation without cloning the whole job (#4732).
pub fn validate_cron_delivery(delivery: &CronDelivery) -> Result<(), String> {
    match delivery {
        CronDelivery::Channel { channel, to } => {
            if channel.is_empty() {
                return Err("delivery channel must not be empty".into());
            }
            if to.is_empty() {
                return Err("delivery recipient must not be empty".into());
            }
        }
        CronDelivery::Webhook { url } => validate_webhook_url(url)?,
        CronDelivery::None | CronDelivery::LastChannel => {}
    }
    Ok(())
}

/// Validate a slice of [`CronDeliveryTarget`]s for the fan-out list.
/// Returns `Err` with `"delivery_targets[<i>]: …"` prefix so the caller
/// can surface which entry failed.
pub fn validate_cron_delivery_targets(targets: &[CronDeliveryTarget]) -> Result<(), String> {
    for (i, target) in targets.iter().enumerate() {
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
                validate_webhook_url(url).map_err(|e| format!("delivery_targets[{i}]: {e}"))?;
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

/// Validate a webhook URL against SSRF: scheme must be http/https, length
/// within `MAX_WEBHOOK_URL_LEN`, and the host (after WHATWG URL
/// normalisation) must not point at the daemon itself, RFC 1918 / shared
/// CGNAT / link-local space, IPv6 ULA / link-local, RFC 1122 "this network"
/// 0.0.0.0/8, or known cloud-metadata services.
///
/// `url::Url::parse` normalises non-canonical IPv4 forms — hex
/// `0x7f000001`, single-decimal `2130706433`, octal-dotted `0177.0.0.1` —
/// and IPv4-mapped IPv6 (`::ffff:127.0.0.1`) before the literal check, so
/// these bypass surfaces (#4732) collapse to standard `127.0.0.1` /
/// `::ffff:7f00:1` shapes that `is_blocked_ip` recognises. The parser
/// also lowercases the scheme, so `HTTPS://…` is accepted (was rejected
/// by the pre-#4739 prefix check).
///
/// DNS-blind: a hostname that resolves to a private IP only at fire-time
/// (DNS rebind) is NOT caught here; the resolver-time check lives in
/// `librefang-api::webhook_store::validate_webhook_url_resolved` (#3701).
pub fn validate_webhook_url(url: &str) -> Result<(), String> {
    if url.len() > MAX_WEBHOOK_URL_LEN {
        return Err(format!(
            "webhook URL too long ({} chars, max {MAX_WEBHOOK_URL_LEN})",
            url.len()
        ));
    }
    let parsed =
        ::url::Url::parse(url).map_err(|e| format!("webhook URL is not parseable: {e}"))?;
    // Use the parsed scheme rather than a raw `starts_with` check so
    // upper-case forms (`HTTPS://…`) and other case-mixed inputs are
    // canonicalised to lowercase before the comparison.
    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            return Err(format!(
                "webhook URL scheme '{other}' is not allowed, only http/https"
            ));
        }
    }
    match parsed.host() {
        Some(::url::Host::Ipv4(v4)) => {
            let ip = IpAddr::V4(v4);
            if is_blocked_ip(ip) {
                return Err(format!(
                    "webhook host '{v4}' is not allowed (loopback / unspecified / private / link-local / metadata)"
                ));
            }
        }
        Some(::url::Host::Ipv6(v6)) => {
            // Canonicalise IPv4-mapped IPv6 before the rule check so a
            // transparent connect to the embedded IPv4 cannot bypass.
            let ip = canonical_ip(IpAddr::V6(v6));
            if is_blocked_ip(ip) {
                return Err(format!(
                    "webhook host '{v6}' is not allowed (loopback / unspecified / private / link-local / metadata)"
                ));
            }
        }
        Some(::url::Host::Domain(host)) => {
            let lower = host.to_ascii_lowercase();
            if is_blocked_domain(&lower) {
                return Err(format!(
                    "webhook host '{host}' is not allowed (loopback / metadata / .internal)"
                ));
            }
        }
        None => {
            // `url::Url::parse` populates `host` for every "special"
            // scheme (http/https/ftp/ws/wss/file). Reaching this arm
            // means the input is malformed in a way the parser tolerated
            // but we cannot safely route — refuse explicitly.
            //
            // Note: `librefang-api::webhook_store::validate_webhook_url`
            // historically returns `Ok(())` here; the stricter behaviour
            // is mirrored back in `webhook_store` for consistency
            // (#4739 review).
            return Err("webhook URL has no host component".into());
        }
    }
    Ok(())
}

/// Hostnames the daemon refuses to webhook to. Mirrors the dashboard
/// editor so users see a consistent rejection regardless of which surface
/// created the target.
///
/// `url::Url::parse` already converts IDN forms (Unicode hostnames) to
/// ASCII punycode (`xn--…`), so the literal-match below operates on the
/// canonical ASCII form. A Unicode homoglyph that punycode-encodes to a
/// string other than `"localhost"` etc. WILL slip through this layer; the
/// resolver-time check at fire-time (#3701, runtime side) is the second
/// line of defence.
///
/// `*.localhost` is reserved by RFC 6761 §6.3 — some loopback adapters and
/// browsers answer for any subdomain of `.localhost`, so we block the
/// whole tree rather than just the literal name.
fn is_blocked_domain(lower: &str) -> bool {
    matches!(
        lower,
        "localhost"
            | "metadata"
            | "metadata.google.internal"
            | "metadata.aws.amazon.com"
            | "instance-data"
            | "instance-data.ec2.internal"
    ) || lower.ends_with(".localhost")
        || lower.ends_with(".internal")
}

/// True for IPs that must never be webhooked to: loopback (`127.0.0.0/8`,
/// `::1`), unspecified (`0.0.0.0`, `::`), RFC 1122 §3.2.1.3 "this network"
/// (`0.0.0.0/8` — historically rewritten to `127.0.0.0/8` by some stacks),
/// RFC 1918 private (`10/8`, `172.16/12`, `192.168/16`), CGNAT (`100.64/10`),
/// IPv6 ULA (`fc00::/7`), multicast (`ff00::/8`), and link-local
/// (`169.254/16`, `fe80::/10`). Always normalises IPv4-mapped IPv6 first.
fn is_blocked_ip(ip: IpAddr) -> bool {
    let ip = canonical_ip(ip);
    ip.is_loopback()
        || ip.is_unspecified()
        || is_zeronet_v4(ip)
        || is_private_ip(ip)
        || is_link_local(ip)
}

/// RFC 1122 §3.2.1.3 reserves `0.0.0.0/8` as "this network". Modern Linux
/// stacks reject outbound traffic to this range, but historically (and on
/// some embedded TCP/IP stacks) `0.x.y.z` was rewritten to `127.x.y.z`,
/// which would make the entire prefix an SSRF surface to localhost.
/// Blocking explicitly is cheap and removes the platform dependency.
fn is_zeronet_v4(ip: IpAddr) -> bool {
    matches!(ip, IpAddr::V4(v4) if v4.octets()[0] == 0)
}

/// Unwrap IPv4-mapped IPv6 (`::ffff:X.X.X.X`) to its IPv4 form. All other
/// addresses are returned unchanged.
fn canonical_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(v6),
        },
        IpAddr::V4(_) => ip,
    }
}

fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            // RFC 1918 (`10/8`, `172.16/12`, `192.168/16`) plus
            // `100.64.0.0/10` (CGNAT shared address space, RFC 6598).
            // CGNAT range: second octet's top 2 bits must be `01` —
            // i.e. `0x40` after the `0xC0` mask. Hex form is used here
            // so the mask/value relationship is visually obvious.
            v4.is_private() || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 0x40)
        }
        IpAddr::V6(v6) => {
            let segs = v6.segments();
            // ULA `fc00::/7` (covers `fd00::/8`) plus multicast `ff00::/8`.
            (segs[0] & 0xfe00) == 0xfc00 || (segs[0] & 0xff00) == 0xff00
        }
    }
}

/// IPv4 link-local is `169.254.0.0/16` per RFC 3927; IPv6 link-local is
/// `fe80::/10` per RFC 4291. We use `Ipv4Addr::is_link_local()` for the
/// V4 arm rather than checking `octets()[0] == 169` to keep the matcher
/// narrow to the actual link-local range — `169.0.0.0 – 169.253.255.255`
/// and `169.255.0.0 – 169.255.255.255` are routable globally and must
/// not be rejected here.
fn is_link_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_link_local(),
        IpAddr::V6(v6) => (v6.segments()[0] & 0xffc0) == 0xfe80,
    }
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
        // Post-#4739: scheme is checked after `Url::parse` via
        // `parsed.scheme()` rather than a raw `starts_with`, so the error
        // message names the scheme rather than the prefix.
        assert!(
            err.contains("http/https") && err.contains("not allowed"),
            "{err}"
        );
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
    fn webhook_http_external_ok() {
        // Public-looking host on plain http is fine — the scheme is
        // permitted (legacy on-prem systems), only SSRF-prone hosts are
        // refused.
        let mut job = valid_job();
        job.delivery = CronDelivery::Webhook {
            url: "http://example.com:8080/hook".into(),
        };
        assert!(job.validate(0).is_ok(), "{:?}", job.validate(0));
    }

    #[test]
    fn webhook_https_ok() {
        let mut job = valid_job();
        job.delivery = CronDelivery::Webhook {
            url: "https://example.com/hook".into(),
        };
        assert!(job.validate(0).is_ok());
    }

    // -- Webhook SSRF coverage (#4732) -------------------------------------
    //
    // Each URL form below either points at a private/loopback/metadata
    // address directly or relies on a host-form normalisation step
    // (numeric IPv4, octal/hex octets, IPv4-mapped IPv6) that the
    // pre-#4732 string-prefix logic missed.

    fn assert_webhook_rejected(url: &str) {
        let mut job = valid_job();
        job.delivery = CronDelivery::Webhook { url: url.into() };
        let err = job
            .validate(0)
            .expect_err(&format!("expected SSRF rejection for {url}"));
        assert!(
            err.contains("not allowed") || err.contains("not parseable"),
            "unexpected error for {url}: {err}"
        );
    }

    #[test]
    fn webhook_localhost_name_rejected() {
        assert_webhook_rejected("http://localhost:8080/hook");
    }

    #[test]
    fn webhook_metadata_aws_rejected() {
        assert_webhook_rejected("http://metadata.aws.amazon.com/latest/meta-data/");
    }

    #[test]
    fn webhook_metadata_gcp_rejected() {
        assert_webhook_rejected("http://metadata.google.internal/");
    }

    #[test]
    fn webhook_dotinternal_suffix_rejected() {
        assert_webhook_rejected("http://kube-apiserver.cluster.internal/");
    }

    #[test]
    fn webhook_loopback_dotted_rejected() {
        assert_webhook_rejected("http://127.0.0.1/");
        assert_webhook_rejected("http://127.255.255.254/");
    }

    #[test]
    fn webhook_unspecified_v4_rejected() {
        assert_webhook_rejected("http://0.0.0.0/");
    }

    /// RFC 1122 §3.2.1.3 reserves `0.0.0.0/8` ("this network"). Some
    /// legacy stacks rewrote `0.x.y.z` to `127.x.y.z`, so the whole
    /// prefix is treated as loopback-equivalent here (#4739 review).
    #[test]
    fn webhook_zeronet_v4_rejected() {
        assert_webhook_rejected("http://0.1.2.3/");
        assert_webhook_rejected("http://0.255.255.255/");
    }

    /// `Ipv4Addr::is_link_local` matches `169.254.0.0/16` per RFC 3927.
    /// Addresses outside that range (e.g. `169.10.0.1`) are globally
    /// routable and must NOT be rejected — an over-broad
    /// `octets()[0] == 169` check would block public IPs by accident.
    #[test]
    fn webhook_169_outside_link_local_accepted() {
        // Caller is responsible for clearing webhook fields it does not
        // want to set; here we just exercise validate_webhook_url.
        assert!(super::validate_webhook_url("http://169.10.0.1/").is_ok());
        assert!(super::validate_webhook_url("http://169.255.0.1/").is_ok());
    }

    /// `url::Url::parse` lowercases the scheme, so mixed-case forms must
    /// be accepted (the pre-#4739 `starts_with("http://")` check would
    /// have refused these).
    #[test]
    fn webhook_mixed_case_scheme_accepted() {
        assert!(super::validate_webhook_url("HTTPS://example.com/").is_ok());
        assert!(super::validate_webhook_url("Http://example.com/").is_ok());
    }

    /// Non-http(s) schemes must be rejected with the new
    /// `parsed.scheme()`-based check (#4739 review).
    #[test]
    fn webhook_non_http_scheme_rejected() {
        let mut job = valid_job();
        job.delivery = CronDelivery::Webhook {
            url: "ftp://example.com/file".into(),
        };
        let err = job.validate(0).expect_err("ftp scheme must be refused");
        assert!(err.contains("not allowed"), "unexpected error: {err}");
    }

    #[test]
    fn webhook_private_v4_rejected() {
        assert_webhook_rejected("http://10.0.0.1/");
        assert_webhook_rejected("http://10.255.255.255/");
        assert_webhook_rejected("http://172.16.0.1/");
        assert_webhook_rejected("http://172.31.255.255/");
        assert_webhook_rejected("http://192.168.1.1/");
    }

    #[test]
    fn webhook_cgnat_rejected() {
        assert_webhook_rejected("http://100.64.0.1/");
    }

    #[test]
    fn webhook_link_local_v4_rejected() {
        assert_webhook_rejected("http://169.254.169.254/");
    }

    #[test]
    fn webhook_loopback_hex_form_rejected() {
        // 0x7f000001 == 127.0.0.1 — WHATWG URL parser normalises both
        // single-component hex and decimal.
        assert_webhook_rejected("http://0x7f000001/");
    }

    #[test]
    fn webhook_loopback_decimal_form_rejected() {
        // 2130706433 == 127.0.0.1.
        assert_webhook_rejected("http://2130706433/");
    }

    #[test]
    fn webhook_loopback_octal_form_rejected() {
        // 0177.0.0.1 == 127.0.0.1.
        assert_webhook_rejected("http://0177.0.0.1/");
    }

    #[test]
    fn webhook_loopback_v6_rejected() {
        assert_webhook_rejected("http://[::1]/");
    }

    #[test]
    fn webhook_unspecified_v6_rejected() {
        assert_webhook_rejected("http://[::]/");
    }

    #[test]
    fn webhook_v4_mapped_v6_rejected() {
        // ::ffff:127.0.0.1 — IPv4-mapped IPv6 wrapping loopback.
        assert_webhook_rejected("http://[::ffff:127.0.0.1]/");
        // ::ffff:7f00:1 — same address in compact hex form.
        assert_webhook_rejected("http://[::ffff:7f00:1]/");
    }

    #[test]
    fn webhook_link_local_v6_rejected() {
        assert_webhook_rejected("http://[fe80::1]/");
    }

    #[test]
    fn webhook_unique_local_v6_rejected() {
        assert_webhook_rejected("http://[fc00::1]/");
        assert_webhook_rejected("http://[fd12:3456:789a::1]/");
    }

    #[test]
    fn webhook_delivery_targets_share_host_validation() {
        // Same blocklist must apply through CronDeliveryTarget::Webhook
        // (which is what `delivery_targets` fan-out exposes).
        let mut job = valid_job();
        job.delivery_targets = vec![CronDeliveryTarget::Webhook {
            url: "http://0x7f000001/hook".into(),
            auth_header: None,
        }];
        let err = job.validate(0).expect_err("hex-form loopback must reject");
        assert!(
            err.starts_with("delivery_targets[0]:") && err.contains("not allowed"),
            "{err}"
        );
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

    // -- PreScript dangerous-env denylist (review fix for PR #3145) --

    fn pre_script_with_env(home: &std::path::Path, key: &str, value: &str) -> PreScript {
        let scripts = home.join("scripts");
        std::fs::create_dir_all(&scripts).unwrap();
        let script_path = scripts.join("safe.sh");
        if !script_path.exists() {
            std::fs::write(&script_path, "#!/bin/sh\n").unwrap();
        }
        let mut env = std::collections::HashMap::new();
        env.insert(key.into(), value.into());
        PreScript {
            argv: vec!["safe.sh".into()],
            cwd: None,
            env,
        }
    }

    #[test]
    fn pre_script_validation_rejects_ld_preload() {
        let home = fixture_home_with_script("safe.sh");
        let script = pre_script_with_env(home.path(), "LD_PRELOAD", "/tmp/evil.so");
        match validate_pre_script(&script, home.path()) {
            Err(PreScriptValidationError::DangerousEnvKey { key }) => {
                assert_eq!(key, "LD_PRELOAD");
            }
            other => panic!("expected DangerousEnvKey, got {other:?}"),
        }
    }

    #[test]
    fn pre_script_validation_rejects_path_override() {
        let home = fixture_home_with_script("safe.sh");
        let script = pre_script_with_env(home.path(), "PATH", "/tmp/evil:/bin");
        assert!(matches!(
            validate_pre_script(&script, home.path()),
            Err(PreScriptValidationError::DangerousEnvKey { .. })
        ));
    }

    #[test]
    fn pre_script_validation_rejects_dyld_insert() {
        let home = fixture_home_with_script("safe.sh");
        let script = pre_script_with_env(home.path(), "DYLD_INSERT_LIBRARIES", "/tmp/evil.dylib");
        assert!(matches!(
            validate_pre_script(&script, home.path()),
            Err(PreScriptValidationError::DangerousEnvKey { .. })
        ));
    }

    #[test]
    fn pre_script_validation_rejects_dangerous_env_case_insensitive() {
        // ASCII case-insensitive matching: `Ld_Preload` is rejected even
        // though the canonical form is uppercase. Mirrors the Windows env
        // semantics and defends against subprocesses that lowercase keys.
        let home = fixture_home_with_script("safe.sh");
        let script = pre_script_with_env(home.path(), "Ld_Preload", "/tmp/evil.so");
        match validate_pre_script(&script, home.path()) {
            Err(PreScriptValidationError::DangerousEnvKey { key }) => {
                assert_eq!(key, "Ld_Preload", "original casing preserved in error");
            }
            other => panic!("expected DangerousEnvKey for mixed case, got {other:?}"),
        }
    }

    #[test]
    fn pre_script_validation_accepts_safe_env_keys() {
        // Arbitrary non-denylist keys must round-trip cleanly. Specifically
        // includes names that *contain* substrings of denylist entries
        // (`HOME_OVERRIDE` ⊃ no denylist match) to confirm we match whole
        // keys, not substrings.
        let home = fixture_home_with_script("safe.sh");
        let mut env = std::collections::HashMap::new();
        env.insert("MY_API_KEY".into(), "secret".into());
        env.insert("HOME_OVERRIDE".into(), "/tmp/work".into());
        env.insert("CUSTOM_LD_FLAG".into(), "x".into());
        let script = PreScript {
            argv: vec!["safe.sh".into()],
            cwd: None,
            env,
        };
        validate_pre_script(&script, home.path())
            .expect("safe env keys must pass dangerous-key denylist");
    }

    #[test]
    fn cron_job_validate_rejects_pre_script_with_dangerous_env() {
        // End-to-end: a cron job carrying a poisoned pre_script.env is
        // rejected at the `CronJob::validate` boundary so ill-formed
        // payloads never reach the scheduler. Path allowlist is skipped
        // here (home_dir = None) but the env denylist still fires.
        let mut env = std::collections::HashMap::new();
        env.insert("LD_PRELOAD".into(), "/tmp/evil.so".into());
        let mut job = valid_job();
        job.action = CronAction::AgentTurn {
            message: "hello".into(),
            model_override: None,
            timeout_secs: None,
            pre_check_script: None,
            pre_script: Some(PreScript {
                argv: vec!["scripts/safe.sh".into()],
                cwd: None,
                env,
            }),
            silent_marker: None,
        };
        let err = job.validate(0).unwrap_err();
        assert!(
            err.contains("LD_PRELOAD") && err.contains("dangerous"),
            "expected dangerous-key error, got: {err}"
        );
    }

    #[test]
    fn cron_job_validate_with_home_rejects_pre_script_outside_allowlist() {
        // Companion test: when home_dir IS provided, both env denylist and
        // path allowlist run. A path-escape with a benign env still fails
        // on the path check, proving the path branch is wired up.
        let home = fixture_home_with_script("safe.sh");
        let mut job = valid_job();
        #[cfg(unix)]
        let outside = "/bin/sh".to_string();
        #[cfg(not(unix))]
        let outside =
            std::env::var("COMSPEC").unwrap_or_else(|_| "C:\\Windows\\System32\\cmd.exe".into());
        job.action = CronAction::AgentTurn {
            message: "hello".into(),
            model_override: None,
            timeout_secs: None,
            pre_check_script: None,
            pre_script: Some(PreScript {
                argv: vec![outside],
                cwd: None,
                env: Default::default(),
            }),
            silent_marker: None,
        };
        let err = job.validate_with_home(0, Some(home.path())).unwrap_err();
        assert!(
            err.contains("allowlist"),
            "expected allowlist error, got: {err}"
        );
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
