//! Execution approval types for the LibreFang agent OS.
//!
//! When an agent attempts a dangerous operation (e.g. `shell_exec`), the kernel
//! creates an [`ApprovalRequest`] and pauses the agent until a human operator
//! responds with an [`ApprovalResponse`]. The [`ApprovalPolicy`] configures
//! which tools require approval and how long to wait before auto-denying.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum length of tool names (chars).
const MAX_TOOL_NAME_LEN: usize = 64;

/// Maximum length of a request description (chars).
const MAX_DESCRIPTION_LEN: usize = 1024;

/// Maximum length of an action summary (chars).
pub const MAX_ACTION_SUMMARY_LEN: usize = 512;

/// Maximum length of modify-and-retry feedback (chars).
pub const MAX_APPROVAL_FEEDBACK_LEN: usize = 4096;

/// Minimum approval timeout in seconds.
const MIN_TIMEOUT_SECS: u64 = 10;

/// Maximum approval timeout in seconds.
const MAX_TIMEOUT_SECS: u64 = 300;

/// Maximum number of trusted senders.
const MAX_TRUSTED_SENDERS: usize = 100;

/// Maximum number of channel rules.
const MAX_CHANNEL_RULES: usize = 50;

/// Maximum length of a channel name (chars).
const MAX_CHANNEL_NAME_LEN: usize = 64;

/// Maximum number of tools in a single channel rule allow/deny list.
const MAX_CHANNEL_RULE_TOOLS: usize = 50;

// ---------------------------------------------------------------------------
// SecondFactor
// ---------------------------------------------------------------------------

/// Second-factor verification scope.
///
/// Controls where TOTP verification is enforced:
/// - `Totp` = approvals only (backward-compatible default when TOTP is enabled)
/// - `Login` = dashboard login only
/// - `Both` = approvals + dashboard login
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SecondFactor {
    /// No second factor required (default).
    #[default]
    None,
    /// TOTP required for tool approvals only.
    Totp,
    /// TOTP required for dashboard login only.
    Login,
    /// TOTP required for both approvals and dashboard login.
    Both,
}

impl SecondFactor {
    /// Whether TOTP is required for dashboard login.
    pub fn requires_login_totp(self) -> bool {
        matches!(self, SecondFactor::Login | SecondFactor::Both)
    }

    /// Whether TOTP is required for tool approvals.
    pub fn requires_approval_totp(self) -> bool {
        matches!(self, SecondFactor::Totp | SecondFactor::Both)
    }
}

/// Maximum TOTP grace period in seconds (1 hour).
const MAX_TOTP_GRACE_PERIOD_SECS: u64 = 3600;

// ---------------------------------------------------------------------------
// RiskLevel
// ---------------------------------------------------------------------------

/// Risk level of an operation requiring approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

impl RiskLevel {
    /// Returns a warning emoji suitable for display in dashboards and chat.
    pub fn emoji(&self) -> &'static str {
        match self {
            RiskLevel::Low => "\u{2139}\u{fe0f}",      // information source
            RiskLevel::Medium => "\u{26a0}\u{fe0f}",   // warning sign
            RiskLevel::High => "\u{1f6a8}",            // rotating light
            RiskLevel::Critical => "\u{2620}\u{fe0f}", // skull and crossbones
        }
    }
}

// ---------------------------------------------------------------------------
// ApprovalDecision
// ---------------------------------------------------------------------------

/// Decision on an approval request.
///
/// Simple variants serialize as plain strings (`"approved"`, `"denied"`, etc.)
/// for backward compatibility. `ModifyAndRetry` serializes as
/// `{"type": "modify_and_retry", "feedback": "..."}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalDecision {
    Approved,
    Denied,
    TimedOut,
    /// Human requests modification — agent should retry with feedback.
    ModifyAndRetry {
        feedback: String,
    },
    /// Timeout fallback: skip the tool, agent continues without it.
    Skipped,
}

/// ApprovalDecision has a custom Serialize impl that produces either a plain
/// string (`"approved"`, `"denied"`, …) or an object
/// (`{"type": "modify_and_retry", "feedback": "..."}`). The derived
/// `#[derive(JsonSchema)]` can't match that custom shape, so provide it
/// manually as a `oneOf` union.
impl schemars::JsonSchema for ApprovalDecision {
    fn schema_name() -> String {
        "ApprovalDecision".to_string()
    }
    fn json_schema(_gen: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
        // Note: `#[derive(JsonSchema)]` would output draft-07 primitives. We
        // hand-assemble the schema here because the upstream Serialize impl
        // flattens simple variants to plain strings.
        serde_json::from_value(serde_json::json!({
            "oneOf": [
                {"type": "string", "enum": ["approved", "denied", "timed_out", "skipped"]},
                {
                    "type": "object",
                    "required": ["type", "feedback"],
                    "properties": {
                        "type": {"type": "string", "enum": ["modify_and_retry"]},
                        "feedback": {"type": "string"}
                    }
                }
            ]
        }))
        .unwrap()
    }
}

impl Serialize for ApprovalDecision {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::ModifyAndRetry { feedback } => {
                use serde::ser::SerializeMap;
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("type", "modify_and_retry")?;
                map.serialize_entry("feedback", feedback)?;
                map.end()
            }
            other => serializer.serialize_str(other.as_str()),
        }
    }
}

impl<'de> Deserialize<'de> for ApprovalDecision {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de;

        struct Visitor;
        impl<'de> de::Visitor<'de> for Visitor {
            type Value = ApprovalDecision;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str(r#"a string like "approved" or an object like {"type": "modify_and_retry", "feedback": "..."}"#)
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                match v {
                    "approved" => Ok(ApprovalDecision::Approved),
                    "denied" => Ok(ApprovalDecision::Denied),
                    "timed_out" => Ok(ApprovalDecision::TimedOut),
                    "skipped" => Ok(ApprovalDecision::Skipped),
                    other => Err(E::unknown_variant(
                        other,
                        &[
                            "approved",
                            "denied",
                            "timed_out",
                            "skipped",
                            "modify_and_retry",
                        ],
                    )),
                }
            }

            fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut typ: Option<String> = None;
                let mut feedback: Option<String> = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "type" => typ = Some(map.next_value()?),
                        "feedback" => feedback = Some(map.next_value()?),
                        _ => {
                            let _ = map.next_value::<serde::de::IgnoredAny>()?;
                        }
                    }
                }
                match typ.as_deref() {
                    Some("modify_and_retry") => Ok(ApprovalDecision::ModifyAndRetry {
                        feedback: feedback.unwrap_or_default(),
                    }),
                    Some("approved") => Ok(ApprovalDecision::Approved),
                    Some("denied") => Ok(ApprovalDecision::Denied),
                    Some("timed_out") => Ok(ApprovalDecision::TimedOut),
                    Some("skipped") => Ok(ApprovalDecision::Skipped),
                    Some(other) => Err(de::Error::unknown_variant(
                        other,
                        &[
                            "approved",
                            "denied",
                            "timed_out",
                            "skipped",
                            "modify_and_retry",
                        ],
                    )),
                    None => Err(de::Error::missing_field("type")),
                }
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

impl ApprovalDecision {
    /// Whether the decision grants permission to proceed.
    pub fn is_approved(&self) -> bool {
        matches!(self, Self::Approved)
    }

    /// Whether the decision is terminal (no further action possible).
    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::ModifyAndRetry { .. })
    }

    /// String label for display and storage.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Approved => "approved",
            Self::Denied => "denied",
            Self::TimedOut => "timed_out",
            Self::ModifyAndRetry { .. } => "modify_and_retry",
            Self::Skipped => "skipped",
        }
    }
}

// ---------------------------------------------------------------------------
// TimeoutFallback
// ---------------------------------------------------------------------------

/// Behavior when an approval request times out.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TimeoutFallback {
    /// Deny the request (current default behavior).
    #[default]
    Deny,
    /// Skip the tool — agent continues without executing it.
    Skip,
    /// Extend timeout and re-notify. `extra_timeout_secs` is added each escalation.
    Escalate {
        #[serde(default = "default_escalation_timeout")]
        extra_timeout_secs: u64,
    },
}

fn default_escalation_timeout() -> u64 {
    120
}

// ---------------------------------------------------------------------------
// ApprovalRequest
// ---------------------------------------------------------------------------

/// An approval request for a dangerous agent operation.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ApprovalRequest {
    pub id: Uuid,
    pub agent_id: String,
    pub tool_name: String,
    pub description: String,
    /// The specific action being requested (sanitized for display).
    pub action_summary: String,
    pub risk_level: RiskLevel,
    pub requested_at: DateTime<Utc>,
    /// Auto-deny timeout in seconds.
    pub timeout_secs: u64,
    /// Sender user ID (from the channel that originated the request).
    #[serde(default)]
    pub sender_id: Option<String>,
    /// Channel name (e.g. "telegram", "discord") that originated the request.
    #[serde(default)]
    pub channel: Option<String>,
    /// Notification targets for this specific request (overrides policy defaults).
    #[serde(default)]
    pub route_to: Vec<NotificationTarget>,
    /// Number of times this request has been escalated (max 3).
    #[serde(default)]
    pub escalation_count: u8,
    /// Session ID that submitted this request (for per-session queue operations).
    ///
    /// When set, `resolve_all_for_session` can resolve all pending requests
    /// belonging to this session atomically (mirrors Hermes-Agent's
    /// `resolve_gateway_approval(session_key, choice, resolve_all=True)`).
    #[serde(default)]
    pub session_id: Option<String>,
    /// LLM-assigned `tool_use_id` from the original `ToolUseStart` block
    /// (deferred-path only). Carried so external surfaces can correlate
    /// the approval request back to the streaming `ToolCall` already
    /// rendered — e.g. the ACP adapter uses this as the
    /// `RequestPermissionRequest` `ToolCallId` so the editor's permission
    /// modal attaches to the existing tool-call card instead of a fresh
    /// orphan id (#3313). `None` for the synchronous `request_approval`
    /// path and for pre-existing rows that pre-date this field — callers
    /// that need a stable id should fall back to `request.id`.
    #[serde(default)]
    pub tool_use_id: Option<String>,
}

impl ApprovalRequest {
    /// Validate this request's fields.
    ///
    /// Returns `Ok(())` or an error message describing the first validation failure.
    pub fn validate(&self) -> Result<(), String> {
        // -- tool_name --
        if self.tool_name.is_empty() {
            return Err("tool_name must not be empty".into());
        }
        if self.tool_name.len() > MAX_TOOL_NAME_LEN {
            return Err(format!(
                "tool_name too long (max {MAX_TOOL_NAME_LEN} chars)"
            ));
        }
        if !self
            .tool_name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_')
        {
            return Err(
                "tool_name may only contain alphanumeric characters and underscores".into(),
            );
        }

        // -- description --
        if self.description.len() > MAX_DESCRIPTION_LEN {
            return Err(format!(
                "description too long (max {MAX_DESCRIPTION_LEN} chars)"
            ));
        }

        // -- action_summary --
        if self.action_summary.len() > MAX_ACTION_SUMMARY_LEN {
            return Err(format!(
                "action_summary too long (max {MAX_ACTION_SUMMARY_LEN} chars)"
            ));
        }

        // -- timeout_secs --
        if self.timeout_secs < MIN_TIMEOUT_SECS {
            return Err(format!(
                "timeout_secs too small ({}, min {MIN_TIMEOUT_SECS})",
                self.timeout_secs
            ));
        }
        if self.timeout_secs > MAX_TIMEOUT_SECS {
            return Err(format!(
                "timeout_secs too large ({}, max {MAX_TIMEOUT_SECS})",
                self.timeout_secs
            ));
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// ApprovalResponse
// ---------------------------------------------------------------------------

/// Response to an approval request.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ApprovalResponse {
    pub request_id: Uuid,
    pub decision: ApprovalDecision,
    pub decided_at: DateTime<Utc>,
    pub decided_by: Option<String>,
}

// ---------------------------------------------------------------------------
// ApprovalEvent — broadcast surface for external subscribers (#3313)
// ---------------------------------------------------------------------------

/// Event emitted by the kernel's `ApprovalManager` when a request enters
/// or leaves the pending queue.
///
/// Subscribed to by external transports that need to mirror approval
/// state into their own UI surface (e.g. the ACP adapter forwards
/// `Created` events into editor-side `session/request_permission`
/// requests). The dashboard and TUI continue to use the synchronous
/// `list_pending` / `list_recent` API; the broadcast is purely additive
/// for low-latency subscribers.
///
/// Variants are non-exhaustive so future kernel versions can add states
/// (e.g. `Escalated`) without breaking subscribers.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ApprovalEvent {
    /// A new approval request has been added to the pending queue.
    Created(ApprovalRequest),
    /// A pending approval has been resolved (approved, denied, modified,
    /// timed out, or skipped).
    Resolved {
        request_id: Uuid,
        decision: ApprovalDecision,
        decided_by: Option<String>,
    },
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Validate a tool name or wildcard pattern with a contextual label.
///
/// Accepts alphanumeric characters, underscores, and a single `*` wildcard
/// for glob matching (e.g. `"file_*"`, `"*_read"`, `"*"`).
fn validate_tool_name(name: &str, label: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    if name.len() > MAX_TOOL_NAME_LEN {
        return Err(format!("{label} too long (max {MAX_TOOL_NAME_LEN} chars)"));
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '*')
    {
        return Err(format!(
            "{label} may only contain alphanumeric characters, underscores, and '*': \"{name}\""
        ));
    }
    if name.chars().filter(|&c| c == '*').count() > 1 {
        return Err(format!(
            "{label} may contain at most one '*' wildcard: \"{name}\""
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// ChannelToolRule
// ---------------------------------------------------------------------------

/// Per-channel tool authorization rule.
///
/// Controls which tools are allowed or denied when requests originate from a
/// specific channel (e.g. "telegram", "discord", "slack").  If both
/// `allowed_tools` and `denied_tools` are non-empty, `denied_tools` takes
/// precedence (deny-wins).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct ChannelToolRule {
    /// Channel name to match (e.g. "telegram", "discord", "slack").
    pub channel: String,
    /// Tools explicitly allowed from this channel.  If non-empty, only these
    /// tools may be executed when the request originates from this channel.
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Tools explicitly denied from this channel.  Takes precedence over
    /// `allowed_tools` (deny-wins).
    #[serde(default)]
    pub denied_tools: Vec<String>,
}

impl ChannelToolRule {
    /// Validate this rule's fields.
    pub fn validate(&self) -> Result<(), String> {
        if self.channel.is_empty() {
            return Err("channel must not be empty".into());
        }
        if self.channel.len() > MAX_CHANNEL_NAME_LEN {
            return Err(format!(
                "channel name too long (max {MAX_CHANNEL_NAME_LEN} chars)"
            ));
        }
        if self.allowed_tools.len() > MAX_CHANNEL_RULE_TOOLS {
            return Err(format!(
                "allowed_tools list too long (max {MAX_CHANNEL_RULE_TOOLS})"
            ));
        }
        if self.denied_tools.len() > MAX_CHANNEL_RULE_TOOLS {
            return Err(format!(
                "denied_tools list too long (max {MAX_CHANNEL_RULE_TOOLS})"
            ));
        }
        for (i, name) in self.allowed_tools.iter().enumerate() {
            validate_tool_name(name, &format!("allowed_tools[{i}]"))?;
        }
        for (i, name) in self.denied_tools.iter().enumerate() {
            validate_tool_name(name, &format!("denied_tools[{i}]"))?;
        }
        Ok(())
    }

    /// Check whether a tool is permitted by this rule.
    ///
    /// Returns `Some(true)` if explicitly allowed, `Some(false)` if explicitly
    /// denied, and `None` if the rule does not apply to this tool.
    ///
    /// Tool names in allow/deny lists support wildcard patterns (e.g. `"file_*"`
    /// matches `"file_read"`, `"file_write"`, etc.).
    pub fn check_tool(&self, tool_name: &str) -> Option<bool> {
        use crate::capability::glob_matches;

        // Deny-wins: if tool matches any denied pattern, always deny.
        if self
            .denied_tools
            .iter()
            .any(|pattern| glob_matches(pattern, tool_name))
        {
            return Some(false);
        }
        // If there is an allow-list, tool must match at least one pattern.
        if !self.allowed_tools.is_empty() {
            return Some(
                self.allowed_tools
                    .iter()
                    .any(|pattern| glob_matches(pattern, tool_name)),
            );
        }
        // Rule has no opinion on this tool.
        None
    }
}

// ---------------------------------------------------------------------------
// Notification types
// ---------------------------------------------------------------------------

/// A target for delivering notifications (approval requests, alerts, etc.).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct NotificationTarget {
    /// Channel type (e.g. "telegram", "slack", "email").
    pub channel_type: String,
    /// Recipient identifier (chat_id, channel name, email address).
    pub recipient: String,
    /// Optional thread/topic ID (adapter-specific).
    #[serde(default)]
    pub thread_id: Option<String>,
}

/// Per-agent notification routing rule.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentNotificationRule {
    /// Glob pattern matching agent names (e.g. "social-*", "*").
    pub agent_pattern: String,
    /// Channels to notify for matching agents.
    pub channels: Vec<NotificationTarget>,
    /// Event types to notify for (e.g. "approval_requested", "task_completed", "task_failed").
    pub events: Vec<String>,
}

/// Notification engine configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default, schemars::JsonSchema)]
pub struct NotificationConfig {
    /// Channels to notify when an approval is requested.
    #[serde(default)]
    pub approval_channels: Vec<NotificationTarget>,
    /// Channels to notify for task completion/failure alerts.
    #[serde(default)]
    pub alert_channels: Vec<NotificationTarget>,
    /// Per-agent notification overrides.
    #[serde(default)]
    pub agent_rules: Vec<AgentNotificationRule>,
}

/// Rule for routing approval requests to specific notification targets.
#[derive(Debug, Clone, Serialize, Deserialize, Default, schemars::JsonSchema)]
pub struct ApprovalRoutingRule {
    /// Tool name glob pattern (e.g. "shell_*", "file_delete").
    pub tool_pattern: String,
    /// Targets to route matching approval requests to.
    pub route_to: Vec<NotificationTarget>,
}

/// Persistent audit log entry for an approval decision.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ApprovalAuditEntry {
    pub id: String,
    pub request_id: String,
    pub agent_id: String,
    pub tool_name: String,
    pub description: String,
    pub action_summary: String,
    pub risk_level: String,
    pub decision: String,
    pub decided_by: Option<String>,
    pub decided_at: String,
    pub requested_at: String,
    pub feedback: Option<String>,
    /// Whether TOTP second-factor was used for this decision.
    #[serde(default)]
    pub second_factor_used: bool,
}

// ---------------------------------------------------------------------------
// ApprovalPolicy
// ---------------------------------------------------------------------------

/// Configurable approval policy.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ApprovalPolicy {
    /// Tools that always require approval. Default: `["shell_exec", "file_write", "file_delete", "apply_patch"]`.
    ///
    /// Accepts either a list of tool names or a boolean shorthand:
    /// - `require_approval = false` → empty list (no tools require approval)
    /// - `require_approval = true`  → `["shell_exec", "file_write", "file_delete", "apply_patch"]` (the default set)
    #[serde(deserialize_with = "deserialize_require_approval")]
    pub require_approval: Vec<String>,
    /// Timeout in seconds. Default: 60, range: 10..=300.
    pub timeout_secs: u64,
    /// Auto-approve in autonomous mode. Default: `false`.
    pub auto_approve_autonomous: bool,
    /// Alias: if `auto_approve = true`, clears the require list at boot.
    #[serde(default, alias = "auto_approve")]
    pub auto_approve: bool,
    /// User IDs that are trusted and auto-approved for all tools.
    ///
    /// When a tool execution request comes from a sender whose `user_id`
    /// appears in this list, the approval gate is bypassed automatically.
    #[serde(default)]
    pub trusted_senders: Vec<String>,
    /// Per-channel tool authorization rules.
    ///
    /// Each rule specifies allowed and/or denied tools for a specific channel.
    /// Rules are evaluated in order; the first matching rule wins.  If no rule
    /// matches the request's channel, the default `require_approval` list applies.
    #[serde(default)]
    pub channel_rules: Vec<ChannelToolRule>,
    /// Behavior when an approval request times out.
    #[serde(default)]
    pub timeout_fallback: TimeoutFallback,
    /// Rules for routing approval requests to specific notification targets.
    #[serde(default)]
    pub routing: Vec<ApprovalRoutingRule>,
    /// Second-factor verification method for approvals.
    #[serde(default)]
    pub second_factor: SecondFactor,
    /// Issuer name shown in authenticator apps (e.g. "LibreFang").
    #[serde(default = "default_totp_issuer")]
    pub totp_issuer: String,
    /// Grace period in seconds after a successful TOTP verification.
    /// Subsequent approvals within this window skip the TOTP check.
    #[serde(default = "default_totp_grace_period")]
    pub totp_grace_period_secs: u64,
    /// Tools that require TOTP verification (glob patterns supported).
    /// Empty list means ALL tools in `require_approval` need TOTP.
    /// Example: `["shell_exec", "file_delete"]` — only these tools need TOTP.
    #[serde(default)]
    pub totp_tools: Vec<String>,
    /// How many days to retain `approval_audit` rows before the periodic
    /// retention sweep hard-deletes them. Independent of `audit_entries`
    /// Merkle retention (see #3383); the approval log is a flat audit
    /// table that would otherwise grow forever (#3468).
    /// Default: 90. Set to 0 to disable pruning.
    #[serde(default = "default_approval_audit_retention_days")]
    pub audit_retention_days: u64,
}

fn default_approval_audit_retention_days() -> u64 {
    90
}

impl Default for ApprovalPolicy {
    fn default() -> Self {
        Self {
            require_approval: vec![
                "shell_exec".to_string(),
                "file_write".to_string(),
                "file_delete".to_string(),
                "apply_patch".to_string(),
                // Skill evolution tools write to `~/.librefang/skills/`
                // (create/update/patch/delete) and to supporting files
                // inside each skill (write_file/remove_file). Every
                // call is a persistent filesystem mutation equivalent
                // in blast radius to `file_write`/`file_delete`, so
                // the default policy MUST gate them the same way.
                // Without the glob, agents could mutate skills without
                // hitting the human approval / TOTP flow that the
                // equivalent low-level tools require.
                "skill_evolve_*".to_string(),
            ],
            timeout_secs: 60,
            auto_approve_autonomous: false,
            auto_approve: false,
            trusted_senders: Vec::new(),
            channel_rules: Vec::new(),
            timeout_fallback: TimeoutFallback::default(),
            routing: Vec::new(),
            second_factor: SecondFactor::default(),
            totp_issuer: default_totp_issuer(),
            totp_grace_period_secs: default_totp_grace_period(),
            totp_tools: Vec::new(),
            audit_retention_days: default_approval_audit_retention_days(),
        }
    }
}

fn default_totp_issuer() -> String {
    "LibreFang".to_string()
}

fn default_totp_grace_period() -> u64 {
    300
}

/// Custom deserializer that accepts:
/// - A list of strings: `["shell_exec", "file_write"]`
/// - A boolean: `false` → `[]`, `true` → the default mutation set
fn deserialize_require_approval<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de;

    struct RequireApprovalVisitor;

    impl<'de> de::Visitor<'de> for RequireApprovalVisitor {
        type Value = Vec<String>;

        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a list of tool names or a boolean")
        }

        fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
            Ok(if v {
                // Must stay in lockstep with `ApprovalPolicy::default`
                // above — the boolean `true` shorthand should expand to
                // the same set a freshly-defaulted policy ships with.
                vec![
                    "shell_exec".to_string(),
                    "file_write".to_string(),
                    "file_delete".to_string(),
                    "apply_patch".to_string(),
                    "skill_evolve_*".to_string(),
                ]
            } else {
                vec![]
            })
        }

        fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut v = Vec::new();
            while let Some(s) = seq.next_element::<String>()? {
                v.push(s);
            }
            Ok(v)
        }
    }

    deserializer.deserialize_any(RequireApprovalVisitor)
}

impl ApprovalPolicy {
    /// Apply the `auto_approve` shorthand: if true, clears the require list.
    pub fn apply_shorthands(&mut self) {
        if self.auto_approve {
            self.require_approval.clear();
        }
    }

    /// Check if a specific tool requires TOTP verification.
    ///
    /// Returns `true` if `second_factor` is `Totp` AND (totp_tools is empty
    /// OR the tool matches a pattern in totp_tools).
    pub fn tool_requires_totp(&self, tool_name: &str) -> bool {
        if !self.second_factor.requires_approval_totp() {
            return false;
        }
        if self.totp_tools.is_empty() {
            return true; // All tools require TOTP
        }
        use crate::capability::glob_matches;
        self.totp_tools
            .iter()
            .any(|pattern| glob_matches(pattern, tool_name))
    }

    /// Check if the given sender is trusted (auto-approve bypass).
    pub fn is_trusted_sender(&self, sender_id: &str) -> bool {
        self.trusted_senders.iter().any(|s| s == sender_id)
    }

    /// Check channel-level tool authorization.
    ///
    /// Returns `Some(false)` if the tool is explicitly denied for this channel,
    /// `Some(true)` if explicitly allowed, or `None` if no channel rule applies.
    pub fn check_channel_tool(&self, channel: &str, tool_name: &str) -> Option<bool> {
        for rule in &self.channel_rules {
            if rule.channel == channel {
                return rule.check_tool(tool_name);
            }
        }
        None
    }

    /// Validate this policy's fields.
    ///
    /// Returns `Ok(())` or an error message describing the first validation failure.
    pub fn validate(&self) -> Result<(), String> {
        // -- timeout_secs --
        if self.timeout_secs < MIN_TIMEOUT_SECS {
            return Err(format!(
                "timeout_secs too small ({}, min {MIN_TIMEOUT_SECS})",
                self.timeout_secs
            ));
        }
        if self.timeout_secs > MAX_TIMEOUT_SECS {
            return Err(format!(
                "timeout_secs too large ({}, max {MAX_TIMEOUT_SECS})",
                self.timeout_secs
            ));
        }

        // -- require_approval tool names --
        for (i, name) in self.require_approval.iter().enumerate() {
            validate_tool_name(name, &format!("require_approval[{i}]"))?;
        }

        // -- trusted_senders --
        if self.trusted_senders.len() > MAX_TRUSTED_SENDERS {
            return Err(format!(
                "trusted_senders list too long ({}, max {MAX_TRUSTED_SENDERS})",
                self.trusted_senders.len()
            ));
        }
        for (i, sender) in self.trusted_senders.iter().enumerate() {
            if sender.is_empty() {
                return Err(format!("trusted_senders[{i}] must not be empty"));
            }
        }

        // -- channel_rules --
        if self.channel_rules.len() > MAX_CHANNEL_RULES {
            return Err(format!(
                "channel_rules list too long ({}, max {MAX_CHANNEL_RULES})",
                self.channel_rules.len()
            ));
        }
        for (i, rule) in self.channel_rules.iter().enumerate() {
            rule.validate()
                .map_err(|e| format!("channel_rules[{i}]: {e}"))?;
        }

        // -- totp_tools --
        for (i, name) in self.totp_tools.iter().enumerate() {
            validate_tool_name(name, &format!("totp_tools[{i}]"))?;
        }

        // -- totp_grace_period_secs --
        if self.totp_grace_period_secs > MAX_TOTP_GRACE_PERIOD_SECS {
            return Err(format!(
                "totp_grace_period_secs too large ({}, max {MAX_TOTP_GRACE_PERIOD_SECS})",
                self.totp_grace_period_secs
            ));
        }

        // -- totp_issuer --
        if self.second_factor != SecondFactor::None && self.totp_issuer.is_empty() {
            return Err("totp_issuer must not be empty when second_factor is enabled".into());
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- helpers --

    fn valid_request() -> ApprovalRequest {
        ApprovalRequest {
            id: Uuid::new_v4(),
            agent_id: "agent-001".into(),
            tool_name: "shell_exec".into(),
            description: "Execute rm -rf /tmp/stale_cache".into(),
            action_summary: "rm -rf /tmp/stale_cache".into(),
            risk_level: RiskLevel::High,
            requested_at: Utc::now(),
            timeout_secs: 60,
            sender_id: None,
            channel: None,
            route_to: Vec::new(),
            escalation_count: 0,
            session_id: None,
            tool_use_id: None,
        }
    }

    fn valid_policy() -> ApprovalPolicy {
        ApprovalPolicy::default()
    }

    // -----------------------------------------------------------------------
    // RiskLevel
    // -----------------------------------------------------------------------

    #[test]
    fn risk_level_emoji() {
        assert_eq!(RiskLevel::Low.emoji(), "\u{2139}\u{fe0f}");
        assert_eq!(RiskLevel::Medium.emoji(), "\u{26a0}\u{fe0f}");
        assert_eq!(RiskLevel::High.emoji(), "\u{1f6a8}");
        assert_eq!(RiskLevel::Critical.emoji(), "\u{2620}\u{fe0f}");
    }

    #[test]
    fn risk_level_serde_roundtrip() {
        for level in [
            RiskLevel::Low,
            RiskLevel::Medium,
            RiskLevel::High,
            RiskLevel::Critical,
        ] {
            let json = serde_json::to_string(&level).unwrap();
            let back: RiskLevel = serde_json::from_str(&json).unwrap();
            assert_eq!(level, back);
        }
    }

    #[test]
    fn risk_level_rename_all() {
        let json = serde_json::to_string(&RiskLevel::Critical).unwrap();
        assert_eq!(json, "\"critical\"");
        let json = serde_json::to_string(&RiskLevel::Low).unwrap();
        assert_eq!(json, "\"low\"");
    }

    // -----------------------------------------------------------------------
    // ApprovalDecision
    // -----------------------------------------------------------------------

    #[test]
    fn decision_serde_roundtrip() {
        for decision in [
            ApprovalDecision::Approved,
            ApprovalDecision::Denied,
            ApprovalDecision::TimedOut,
            ApprovalDecision::Skipped,
        ] {
            let json = serde_json::to_string(&decision).unwrap();
            let back: ApprovalDecision = serde_json::from_str(&json).unwrap();
            assert_eq!(decision, back);
        }
        // ModifyAndRetry with data
        let modify = ApprovalDecision::ModifyAndRetry {
            feedback: "Use a safer approach".into(),
        };
        let json = serde_json::to_string(&modify).unwrap();
        let back: ApprovalDecision = serde_json::from_str(&json).unwrap();
        assert_eq!(modify, back);
    }

    #[test]
    fn decision_rename_all() {
        // Simple variants serialize as plain strings (backward-compatible)
        assert_eq!(
            serde_json::to_string(&ApprovalDecision::TimedOut).unwrap(),
            "\"timed_out\""
        );
        assert_eq!(
            serde_json::to_string(&ApprovalDecision::Skipped).unwrap(),
            "\"skipped\""
        );
        assert_eq!(
            serde_json::to_string(&ApprovalDecision::Approved).unwrap(),
            "\"approved\""
        );
    }

    #[test]
    fn decision_modify_and_retry_serde() {
        // ModifyAndRetry serializes as an object
        let modify = ApprovalDecision::ModifyAndRetry {
            feedback: "try safer".into(),
        };
        let json = serde_json::to_string(&modify).unwrap();
        assert!(json.contains("modify_and_retry"));
        assert!(json.contains("try safer"));
        // Deserialize back
        let back: ApprovalDecision = serde_json::from_str(&json).unwrap();
        assert_eq!(modify, back);
    }

    #[test]
    fn decision_backward_compat_string_deser() {
        // Old-format plain strings should still deserialize
        let d: ApprovalDecision = serde_json::from_str("\"approved\"").unwrap();
        assert_eq!(d, ApprovalDecision::Approved);
        let d: ApprovalDecision = serde_json::from_str("\"timed_out\"").unwrap();
        assert_eq!(d, ApprovalDecision::TimedOut);
    }

    // -----------------------------------------------------------------------
    // ApprovalRequest — valid
    // -----------------------------------------------------------------------

    #[test]
    fn valid_request_passes() {
        assert!(valid_request().validate().is_ok());
    }

    // -----------------------------------------------------------------------
    // ApprovalRequest — tool_name
    // -----------------------------------------------------------------------

    #[test]
    fn request_empty_tool_name() {
        let mut req = valid_request();
        req.tool_name = String::new();
        let err = req.validate().unwrap_err();
        assert!(err.contains("empty"), "{err}");
    }

    #[test]
    fn request_tool_name_too_long() {
        let mut req = valid_request();
        req.tool_name = "a".repeat(65);
        let err = req.validate().unwrap_err();
        assert!(err.contains("too long"), "{err}");
    }

    #[test]
    fn request_tool_name_64_chars_ok() {
        let mut req = valid_request();
        req.tool_name = "a".repeat(64);
        assert!(req.validate().is_ok());
    }

    #[test]
    fn request_tool_name_invalid_chars() {
        let mut req = valid_request();
        req.tool_name = "shell-exec".into();
        let err = req.validate().unwrap_err();
        assert!(err.contains("alphanumeric"), "{err}");
    }

    #[test]
    fn request_tool_name_with_underscore_ok() {
        let mut req = valid_request();
        req.tool_name = "file_write".into();
        assert!(req.validate().is_ok());
    }

    // -----------------------------------------------------------------------
    // ApprovalRequest — description
    // -----------------------------------------------------------------------

    #[test]
    fn request_description_too_long() {
        let mut req = valid_request();
        req.description = "x".repeat(1025);
        let err = req.validate().unwrap_err();
        assert!(err.contains("description"), "{err}");
        assert!(err.contains("too long"), "{err}");
    }

    #[test]
    fn request_description_1024_ok() {
        let mut req = valid_request();
        req.description = "x".repeat(1024);
        assert!(req.validate().is_ok());
    }

    #[test]
    fn request_description_empty_ok() {
        let mut req = valid_request();
        req.description = String::new();
        assert!(req.validate().is_ok());
    }

    // -----------------------------------------------------------------------
    // ApprovalRequest — action_summary
    // -----------------------------------------------------------------------

    #[test]
    fn request_action_summary_too_long() {
        let mut req = valid_request();
        req.action_summary = "x".repeat(513);
        let err = req.validate().unwrap_err();
        assert!(err.contains("action_summary"), "{err}");
        assert!(err.contains("too long"), "{err}");
    }

    #[test]
    fn request_action_summary_512_ok() {
        let mut req = valid_request();
        req.action_summary = "x".repeat(512);
        assert!(req.validate().is_ok());
    }

    // -----------------------------------------------------------------------
    // ApprovalRequest — timeout_secs
    // -----------------------------------------------------------------------

    #[test]
    fn request_timeout_too_small() {
        let mut req = valid_request();
        req.timeout_secs = 9;
        let err = req.validate().unwrap_err();
        assert!(err.contains("too small"), "{err}");
    }

    #[test]
    fn request_timeout_too_large() {
        let mut req = valid_request();
        req.timeout_secs = 301;
        let err = req.validate().unwrap_err();
        assert!(err.contains("too large"), "{err}");
    }

    #[test]
    fn request_timeout_min_boundary_ok() {
        let mut req = valid_request();
        req.timeout_secs = 10;
        assert!(req.validate().is_ok());
    }

    #[test]
    fn request_timeout_max_boundary_ok() {
        let mut req = valid_request();
        req.timeout_secs = 300;
        assert!(req.validate().is_ok());
    }

    // -----------------------------------------------------------------------
    // ApprovalResponse — serde
    // -----------------------------------------------------------------------

    #[test]
    fn response_serde_roundtrip() {
        let resp = ApprovalResponse {
            request_id: Uuid::new_v4(),
            decision: ApprovalDecision::Approved,
            decided_at: Utc::now(),
            decided_by: Some("admin@example.com".into()),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: ApprovalResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.request_id, resp.request_id);
        assert_eq!(back.decision, ApprovalDecision::Approved);
        assert_eq!(back.decided_by, Some("admin@example.com".into()));
    }

    #[test]
    fn response_decided_by_none() {
        let resp = ApprovalResponse {
            request_id: Uuid::new_v4(),
            decision: ApprovalDecision::TimedOut,
            decided_at: Utc::now(),
            decided_by: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: ApprovalResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.decided_by, None);
        assert_eq!(back.decision, ApprovalDecision::TimedOut);
    }

    // -----------------------------------------------------------------------
    // ApprovalPolicy — defaults
    // -----------------------------------------------------------------------

    #[test]
    fn policy_default_valid() {
        let policy = ApprovalPolicy::default();
        assert!(policy.validate().is_ok());
        assert_eq!(
            policy.require_approval,
            vec![
                "shell_exec",
                "file_write",
                "file_delete",
                "apply_patch",
                "skill_evolve_*",
            ]
        );
        assert_eq!(policy.timeout_secs, 60);
        assert!(!policy.auto_approve_autonomous);
        assert!(!policy.auto_approve);
        assert_eq!(policy.timeout_fallback, TimeoutFallback::Deny);
    }

    #[test]
    fn policy_serde_default() {
        // An empty JSON object should deserialize to defaults via #[serde(default)].
        let policy: ApprovalPolicy = serde_json::from_str("{}").unwrap();
        assert_eq!(policy.timeout_secs, 60);
        assert_eq!(
            policy.require_approval,
            vec![
                "shell_exec",
                "file_write",
                "file_delete",
                "apply_patch",
                "skill_evolve_*",
            ]
        );
        assert!(!policy.auto_approve_autonomous);
    }

    #[test]
    fn policy_require_approval_bool_false() {
        // require_approval = false → empty list
        let policy: ApprovalPolicy =
            serde_json::from_str(r#"{"require_approval": false}"#).unwrap();
        assert!(policy.require_approval.is_empty());
    }

    #[test]
    fn policy_require_approval_bool_true() {
        // require_approval = true → default set
        let policy: ApprovalPolicy = serde_json::from_str(r#"{"require_approval": true}"#).unwrap();
        assert_eq!(
            policy.require_approval,
            vec![
                "shell_exec",
                "file_write",
                "file_delete",
                "apply_patch",
                "skill_evolve_*",
            ]
        );
    }

    #[test]
    fn policy_auto_approve_clears_list() {
        let mut policy = ApprovalPolicy::default();
        assert!(!policy.require_approval.is_empty());
        policy.auto_approve = true;
        policy.apply_shorthands();
        assert!(policy.require_approval.is_empty());
    }

    // -----------------------------------------------------------------------
    // ApprovalPolicy — timeout_secs
    // -----------------------------------------------------------------------

    #[test]
    fn policy_timeout_too_small() {
        let mut policy = valid_policy();
        policy.timeout_secs = 9;
        let err = policy.validate().unwrap_err();
        assert!(err.contains("too small"), "{err}");
    }

    #[test]
    fn policy_timeout_too_large() {
        let mut policy = valid_policy();
        policy.timeout_secs = 301;
        let err = policy.validate().unwrap_err();
        assert!(err.contains("too large"), "{err}");
    }

    #[test]
    fn policy_timeout_boundaries_ok() {
        let mut policy = valid_policy();
        policy.timeout_secs = 10;
        assert!(policy.validate().is_ok());
        policy.timeout_secs = 300;
        assert!(policy.validate().is_ok());
    }

    // -----------------------------------------------------------------------
    // ApprovalPolicy — require_approval tool names
    // -----------------------------------------------------------------------

    #[test]
    fn policy_empty_tool_name() {
        let mut policy = valid_policy();
        policy.require_approval = vec!["shell_exec".into(), "".into()];
        let err = policy.validate().unwrap_err();
        assert!(err.contains("require_approval[1]"), "{err}");
        assert!(err.contains("empty"), "{err}");
    }

    #[test]
    fn policy_tool_name_too_long() {
        let mut policy = valid_policy();
        policy.require_approval = vec!["a".repeat(65)];
        let err = policy.validate().unwrap_err();
        assert!(err.contains("too long"), "{err}");
    }

    #[test]
    fn policy_tool_name_invalid_chars() {
        let mut policy = valid_policy();
        policy.require_approval = vec!["shell-exec".into()];
        let err = policy.validate().unwrap_err();
        assert!(err.contains("alphanumeric"), "{err}");
    }

    #[test]
    fn policy_tool_name_with_spaces_rejected() {
        let mut policy = valid_policy();
        policy.require_approval = vec!["shell exec".into()];
        let err = policy.validate().unwrap_err();
        assert!(err.contains("alphanumeric"), "{err}");
    }

    #[test]
    fn policy_multiple_valid_tools() {
        let mut policy = valid_policy();
        policy.require_approval = vec![
            "shell_exec".into(),
            "file_write".into(),
            "file_delete".into(),
        ];
        assert!(policy.validate().is_ok());
    }

    #[test]
    fn policy_empty_require_approval_ok() {
        let mut policy = valid_policy();
        policy.require_approval = vec![];
        assert!(policy.validate().is_ok());
    }

    // -----------------------------------------------------------------------
    // Full serde roundtrip — ApprovalRequest
    // -----------------------------------------------------------------------

    #[test]
    fn request_serde_roundtrip() {
        let req = valid_request();
        let json = serde_json::to_string_pretty(&req).unwrap();
        let back: ApprovalRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, req.id);
        assert_eq!(back.agent_id, req.agent_id);
        assert_eq!(back.tool_name, req.tool_name);
        assert_eq!(back.description, req.description);
        assert_eq!(back.action_summary, req.action_summary);
        assert_eq!(back.risk_level, req.risk_level);
        assert_eq!(back.timeout_secs, req.timeout_secs);
    }

    // -----------------------------------------------------------------------
    // Full serde roundtrip — ApprovalPolicy
    // -----------------------------------------------------------------------

    #[test]
    fn policy_serde_roundtrip() {
        let policy = ApprovalPolicy {
            require_approval: vec!["shell_exec".into(), "file_delete".into()],
            timeout_secs: 120,
            auto_approve_autonomous: true,
            auto_approve: false,
            trusted_senders: vec!["admin_123".into()],
            channel_rules: vec![ChannelToolRule {
                channel: "telegram".into(),
                allowed_tools: vec!["file_read".into()],
                denied_tools: vec!["shell_exec".into()],
            }],
            timeout_fallback: TimeoutFallback::default(),
            routing: Vec::new(),
            second_factor: SecondFactor::default(),
            totp_issuer: "LibreFang".into(),
            totp_grace_period_secs: 300,
            totp_tools: Vec::new(),
            audit_retention_days: 90,
        };
        let json = serde_json::to_string(&policy).unwrap();
        let back: ApprovalPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(back.require_approval, policy.require_approval);
        assert_eq!(back.timeout_secs, 120);
        assert!(back.auto_approve_autonomous);
        assert_eq!(back.trusted_senders, vec!["admin_123"]);
        assert_eq!(back.channel_rules.len(), 1);
        assert_eq!(back.channel_rules[0].channel, "telegram");
    }

    // -----------------------------------------------------------------------
    // ChannelToolRule
    // -----------------------------------------------------------------------

    #[test]
    fn channel_rule_deny_wins() {
        let rule = ChannelToolRule {
            channel: "telegram".into(),
            allowed_tools: vec!["shell_exec".into()],
            denied_tools: vec!["shell_exec".into()],
        };
        // deny-wins: even though shell_exec is in allowed, denied takes precedence
        assert_eq!(rule.check_tool("shell_exec"), Some(false));
    }

    #[test]
    fn channel_rule_allow_list_only() {
        let rule = ChannelToolRule {
            channel: "discord".into(),
            allowed_tools: vec!["file_read".into(), "web_fetch".into()],
            denied_tools: vec![],
        };
        assert_eq!(rule.check_tool("file_read"), Some(true));
        assert_eq!(rule.check_tool("shell_exec"), Some(false));
    }

    #[test]
    fn channel_rule_deny_list_only() {
        let rule = ChannelToolRule {
            channel: "slack".into(),
            allowed_tools: vec![],
            denied_tools: vec!["shell_exec".into()],
        };
        assert_eq!(rule.check_tool("shell_exec"), Some(false));
        // No allow list, no deny match → no opinion
        assert_eq!(rule.check_tool("file_read"), None);
    }

    #[test]
    fn channel_rule_empty_lists_no_opinion() {
        let rule = ChannelToolRule {
            channel: "matrix".into(),
            allowed_tools: vec![],
            denied_tools: vec![],
        };
        assert_eq!(rule.check_tool("shell_exec"), None);
    }

    #[test]
    fn channel_rule_validate_empty_channel() {
        let rule = ChannelToolRule {
            channel: "".into(),
            allowed_tools: vec![],
            denied_tools: vec![],
        };
        assert!(rule
            .validate()
            .unwrap_err()
            .contains("channel must not be empty"));
    }

    #[test]
    fn channel_rule_validate_invalid_tool_name() {
        let rule = ChannelToolRule {
            channel: "telegram".into(),
            allowed_tools: vec!["bad-name".into()],
            denied_tools: vec![],
        };
        assert!(rule.validate().unwrap_err().contains("alphanumeric"));
    }

    // -----------------------------------------------------------------------
    // ApprovalPolicy — trusted_senders
    // -----------------------------------------------------------------------

    #[test]
    fn policy_trusted_sender_check() {
        let policy = ApprovalPolicy {
            trusted_senders: vec!["admin_123".into(), "ops_456".into()],
            ..Default::default()
        };
        assert!(policy.is_trusted_sender("admin_123"));
        assert!(policy.is_trusted_sender("ops_456"));
        assert!(!policy.is_trusted_sender("random_user"));
    }

    #[test]
    fn policy_trusted_senders_empty_sender_rejected() {
        let mut policy = valid_policy();
        policy.trusted_senders = vec!["".into()];
        let err = policy.validate().unwrap_err();
        assert!(err.contains("trusted_senders[0]"), "{err}");
        assert!(err.contains("empty"), "{err}");
    }

    // -----------------------------------------------------------------------
    // ApprovalPolicy — channel_rules
    // -----------------------------------------------------------------------

    #[test]
    fn policy_check_channel_tool() {
        let policy = ApprovalPolicy {
            channel_rules: vec![
                ChannelToolRule {
                    channel: "telegram".into(),
                    allowed_tools: vec![],
                    denied_tools: vec!["shell_exec".into()],
                },
                ChannelToolRule {
                    channel: "discord".into(),
                    allowed_tools: vec!["file_read".into()],
                    denied_tools: vec![],
                },
            ],
            ..Default::default()
        };
        assert_eq!(
            policy.check_channel_tool("telegram", "shell_exec"),
            Some(false)
        );
        assert_eq!(policy.check_channel_tool("telegram", "file_read"), None);
        assert_eq!(
            policy.check_channel_tool("discord", "file_read"),
            Some(true)
        );
        assert_eq!(
            policy.check_channel_tool("discord", "shell_exec"),
            Some(false)
        );
        assert_eq!(policy.check_channel_tool("slack", "shell_exec"), None);
    }

    #[test]
    fn policy_channel_rules_validate() {
        let mut policy = valid_policy();
        policy.channel_rules = vec![ChannelToolRule {
            channel: "telegram".into(),
            allowed_tools: vec!["file_read".into()],
            denied_tools: vec![],
        }];
        assert!(policy.validate().is_ok());
    }

    #[test]
    fn policy_channel_rules_invalid_propagates() {
        let mut policy = valid_policy();
        policy.channel_rules = vec![ChannelToolRule {
            channel: "".into(),
            allowed_tools: vec![],
            denied_tools: vec![],
        }];
        let err = policy.validate().unwrap_err();
        assert!(err.contains("channel_rules[0]"), "{err}");
    }

    #[test]
    fn policy_default_has_empty_new_fields() {
        let policy = ApprovalPolicy::default();
        assert!(policy.trusted_senders.is_empty());
        assert!(policy.channel_rules.is_empty());
    }

    #[test]
    fn policy_serde_default_new_fields() {
        let policy: ApprovalPolicy = serde_json::from_str("{}").unwrap();
        assert!(policy.trusted_senders.is_empty());
        assert!(policy.channel_rules.is_empty());
    }

    // -----------------------------------------------------------------------
    // Wildcard support
    // -----------------------------------------------------------------------

    #[test]
    fn validate_tool_name_with_wildcard() {
        // Wildcard patterns should pass validation
        let mut policy = valid_policy();
        policy.require_approval = vec!["file_*".into()];
        assert!(policy.validate().is_ok());

        policy.require_approval = vec!["*".into()];
        assert!(policy.validate().is_ok());

        policy.require_approval = vec!["*_exec".into()];
        assert!(policy.validate().is_ok());
    }

    #[test]
    fn channel_rule_wildcard_deny() {
        let rule = ChannelToolRule {
            channel: "telegram".into(),
            allowed_tools: vec![],
            denied_tools: vec!["file_*".into()],
        };
        // file_read and file_write match the pattern "file_*"
        assert_eq!(rule.check_tool("file_read"), Some(false));
        assert_eq!(rule.check_tool("file_write"), Some(false));
        // shell_exec does not match
        assert_eq!(rule.check_tool("shell_exec"), None);
    }

    #[test]
    fn channel_rule_wildcard_allow() {
        let rule = ChannelToolRule {
            channel: "discord".into(),
            allowed_tools: vec!["file_*".into()],
            denied_tools: vec![],
        };
        assert_eq!(rule.check_tool("file_read"), Some(true));
        assert_eq!(rule.check_tool("file_write"), Some(true));
        // shell_exec doesn't match the wildcard
        assert_eq!(rule.check_tool("shell_exec"), Some(false));
    }

    #[test]
    fn channel_rule_wildcard_star_matches_all() {
        let rule = ChannelToolRule {
            channel: "admin".into(),
            allowed_tools: vec!["*".into()],
            denied_tools: vec![],
        };
        assert_eq!(rule.check_tool("file_read"), Some(true));
        assert_eq!(rule.check_tool("shell_exec"), Some(true));
        assert_eq!(rule.check_tool("anything"), Some(true));
    }

    #[test]
    fn channel_rule_wildcard_deny_wins_over_wildcard_allow() {
        let rule = ChannelToolRule {
            channel: "restricted".into(),
            allowed_tools: vec!["file_*".into()],
            denied_tools: vec!["file_delete".into()],
        };
        // file_delete is explicitly denied — deny wins
        assert_eq!(rule.check_tool("file_delete"), Some(false));
        // file_read matches allow wildcard
        assert_eq!(rule.check_tool("file_read"), Some(true));
    }

    #[test]
    fn channel_rule_validate_wildcard_tool_name() {
        let rule = ChannelToolRule {
            channel: "telegram".into(),
            allowed_tools: vec!["file_*".into()],
            denied_tools: vec!["shell_*".into()],
        };
        assert!(rule.validate().is_ok());
    }

    #[test]
    fn validate_rejects_multiple_wildcards() {
        let mut policy = valid_policy();

        // Patterns with more than one '*' should be rejected
        policy.require_approval = vec!["*_*".into()];
        let err = policy.validate().unwrap_err();
        assert!(err.contains("at most one '*' wildcard"), "got: {err}");

        policy.require_approval = vec!["**".into()];
        let err = policy.validate().unwrap_err();
        assert!(err.contains("at most one '*' wildcard"), "got: {err}");

        // Channel rules should also reject multiple wildcards
        let mut policy = valid_policy();
        policy.channel_rules = vec![ChannelToolRule {
            channel: "telegram".into(),
            allowed_tools: vec!["*_*".into()],
            denied_tools: vec![],
        }];
        let err = policy.validate().unwrap_err();
        assert!(err.contains("at most one '*' wildcard"), "got: {err}");
    }

    // -----------------------------------------------------------------------
    // TimeoutFallback
    // -----------------------------------------------------------------------

    #[test]
    fn timeout_fallback_default_is_deny() {
        assert_eq!(TimeoutFallback::default(), TimeoutFallback::Deny);
    }

    #[test]
    fn timeout_fallback_serde_roundtrip() {
        for v in [TimeoutFallback::Deny, TimeoutFallback::Skip] {
            let json = serde_json::to_string(&v).unwrap();
            let back: TimeoutFallback = serde_json::from_str(&json).unwrap();
            assert_eq!(v, back);
        }
    }

    // -----------------------------------------------------------------------
    // NotificationTarget
    // -----------------------------------------------------------------------

    #[test]
    fn notification_target_serde() {
        let target = NotificationTarget {
            channel_type: "telegram".into(),
            recipient: "123456".into(),
            thread_id: None,
        };
        let json = serde_json::to_string(&target).unwrap();
        let back: NotificationTarget = serde_json::from_str(&json).unwrap();
        assert_eq!(target, back);
    }

    // -----------------------------------------------------------------------
    // ApprovalAuditEntry
    // -----------------------------------------------------------------------

    #[test]
    fn audit_entry_serde() {
        let entry = ApprovalAuditEntry {
            id: "1".into(),
            request_id: "req-1".into(),
            agent_id: "agent-1".into(),
            tool_name: "shell_exec".into(),
            description: "test".into(),
            action_summary: "echo hello".into(),
            risk_level: "high".into(),
            decision: "approved".into(),
            decided_by: Some("admin".into()),
            decided_at: "2024-01-01T00:00:00Z".into(),
            requested_at: "2024-01-01T00:00:00Z".into(),
            feedback: None,
            second_factor_used: false,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let back: ApprovalAuditEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back.decision, "approved");
    }

    // -----------------------------------------------------------------------
    // ApprovalDecision helpers
    // -----------------------------------------------------------------------

    #[test]
    fn decision_is_approved() {
        assert!(ApprovalDecision::Approved.is_approved());
        assert!(!ApprovalDecision::Denied.is_approved());
        assert!(!ApprovalDecision::TimedOut.is_approved());
        assert!(!ApprovalDecision::Skipped.is_approved());
        assert!(!(ApprovalDecision::ModifyAndRetry {
            feedback: "x".into()
        })
        .is_approved());
    }

    #[test]
    fn decision_is_terminal() {
        assert!(ApprovalDecision::Approved.is_terminal());
        assert!(ApprovalDecision::Denied.is_terminal());
        assert!(ApprovalDecision::TimedOut.is_terminal());
        assert!(ApprovalDecision::Skipped.is_terminal());
        assert!(!(ApprovalDecision::ModifyAndRetry {
            feedback: "x".into()
        })
        .is_terminal());
    }

    #[test]
    fn decision_as_str() {
        assert_eq!(ApprovalDecision::Approved.as_str(), "approved");
        assert_eq!(ApprovalDecision::Denied.as_str(), "denied");
        assert_eq!(ApprovalDecision::TimedOut.as_str(), "timed_out");
        assert_eq!(ApprovalDecision::Skipped.as_str(), "skipped");
        assert_eq!(
            ApprovalDecision::ModifyAndRetry {
                feedback: "x".into()
            }
            .as_str(),
            "modify_and_retry"
        );
    }
}
