//! LibreFangKernel — assembles all subsystems and provides the main API.

use crate::auth::AuthManager;
use crate::background::{self, BackgroundExecutor};
use crate::config::load_config;
use crate::error::{KernelError, KernelResult};
use crate::metering::MeteringEngine;
use crate::router;
use crate::supervisor::Supervisor;
use crate::triggers::{TriggerEngine, TriggerId, TriggerPattern};
use crate::workflow::{DryRunStep, StepAgent, Workflow, WorkflowEngine, WorkflowId, WorkflowRunId};

use librefang_memory::MemorySubstrate;
use librefang_runtime::agent_loop::{
    run_agent_loop, run_agent_loop_streaming, strip_provider_prefix, AgentLoopResult,
};
use librefang_runtime::audit::AuditLog;
use librefang_runtime::drivers;
// `kernel_handle::self` is needed by `kernel::tests` (call sites like
// `kernel_handle::ApprovalGate::resolve_user_tool_decision(...)`) —
// keep the self alias alongside the prelude wildcard so tests.rs resolves.
// The `self` alias is `cfg(test)` because the non-test build no longer
// references `kernel_handle::Foo` from inside this file (Phase 3a moved
// the last such use into `kernel::accessors`); the wildcard prelude is
// still needed unconditionally for trait-method resolution on the
// `KernelHandle` impl bodies that remain in this file.
#[cfg(test)]
use librefang_runtime::kernel_handle;
use librefang_runtime::kernel_handle::prelude::*;
use librefang_runtime::llm_driver::{
    CompletionRequest, CompletionResponse, DriverConfig, LlmDriver, LlmError, StreamEvent,
};
use librefang_runtime::python_runtime::{self, PythonConfig};
use librefang_runtime::routing::ModelRouter;
use librefang_runtime::sandbox::{SandboxConfig, WasmSandbox};
use librefang_runtime::tool_runner::builtin_tool_definitions;
use librefang_types::agent::*;
use librefang_types::capability::{glob_matches, Capability};
use librefang_types::config::{AuthProfile, AutoRouteStrategy, KernelConfig};
use librefang_types::error::LibreFangError;
use librefang_types::event::*;
use librefang_types::memory::Memory;
use librefang_types::tool::{AgentLoopSignal, ToolDefinition};

use arc_swap::ArcSwap;
use async_trait::async_trait;
use librefang_channels::types::SenderContext;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
// `Ordering` is no longer used in this file's non-test code (Phase 3a moved
// the last unprefixed-`Ordering` users into `kernel::accessors`); the
// remaining mod.rs sites all spell it `std::sync::atomic::Ordering::*`
// inline. `kernel::tests` still references the bare `Ordering` ident via
// `use super::*`, so keep the import in scope under `cfg(test)` only.
#[cfg(test)]
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock, Weak};
use tracing::{debug, error, info, instrument, warn};

/// Per-trait `kernel_handle::*` impls live in their own files under
/// `kernel/handles/` to keep this file from doubling as a trait-impl
/// dumping ground. The submodules are descendants of `kernel`, so they
/// retain access to `LibreFangKernel`'s private fields and inherent
/// methods without any visibility surgery.
mod handles;

// Cohesive free-fn / non-`LibreFangKernel`-impl chunks pulled out of
// this file in Phase 2 of the kernel/mod.rs split. Re-exported below so
// existing call sites — including `super::foo` references from
// `kernel::tests` — continue to resolve unchanged.
//
// `accessors` (Phase 3a) hosts the first inherent `impl LibreFangKernel`
// block — public-facade getters and lifecycle helpers (vault, GC sweep,
// background sweep tasks). Listed alongside the Phase 2 modules in
// alphabetical order; it is a sibling submodule, so private fields and
// inherent methods on `LibreFangKernel` remain visible without surgery.
mod accessors;
mod agent_execution;
mod agent_runtime;
mod agent_state;
mod assistant_routing;
mod background_lifecycle;
mod bindings_and_handle;
mod boot;
mod config_reload_ops;
mod cron_bridge;
// Cron session compaction helpers (#4683 / #3693). Cherry-picked into
// this file from upstream main so `kernel::tests` resolves the helper
// fns it asserts on. Long-term these belong inside the cron-tick body
// rewrite that the rebase-on-main work will do.
#[allow(dead_code)]
mod cron_compaction;
mod cron_script;
// Phase 3b: cron scheduler tick loop — formerly the longest closure in
// this file (#4683 landing zone). Extracted as `pub(super) async fn`
// so the body can be edited and reviewed in isolation.
mod cron_tick;
mod hands_lifecycle;
mod llm_drivers;
mod mcp_setup;
mod mcp_summary;
mod messaging;
mod pooled_driver;
mod prompt_context;
mod provider_probe;
mod reviewer_sanitize;
mod session_ops;
mod spawn;
mod subsystem_forwards;
pub mod subsystems;
mod tools_and_skills;
mod triggers_and_workflow;

// `cron_deliver_response`, `cron_fan_out_targets`, and `cron_script_wake_gate`
// are now consumed by `kernel::cron_tick` after Phase 3b lifted the cron
// tick loop body out of mod.rs. They are still imported by `cron_tick`
// directly via the `super::` path, so no re-export is needed here.
// Re-export cron_compaction helpers so `kernel::tests`'s `super::*`
// references continue to resolve byte-for-byte.
#[allow(unused_imports)]
use cron_compaction::{
    cron_clamp_keep_recent, cron_compute_keep_count, cron_resolve_compaction_mode,
    try_summarize_trim,
};
use cron_script::atomic_write_toml;
use mcp_summary::{mcp_summary_cache_key, render_mcp_summary};
use provider_probe::probe_all_local_providers_once;
pub use provider_probe::probe_and_update_local_provider;
use reviewer_sanitize::{sanitize_reviewer_block, sanitize_reviewer_line};

/// Synthetic `SenderContext.channel` value the cron dispatcher uses for
/// `[[cron_jobs]]` fires. Matched in [`KernelHandle::resolve_user_tool_decision`]
/// to bypass per-user RBAC the same way the `system_call=true` flag does
/// — daemon-driven calls have no user to attribute to.
///
/// Public so that out-of-crate carve-out sites (`librefang-api::ws.rs` for
/// the dashboard, `librefang-runtime::agent_loop` for the sender-prefix
/// deny-list, …) can reference the same string instead of duplicating the
/// literal. The runtime path can't import this directly (circular dep),
/// but exposing it lets api/cli code stay in lock-step.
pub const SYSTEM_CHANNEL_CRON: &str = "cron";

/// Synthetic `SenderContext.channel` value the autonomous-loop dispatcher
/// uses for agents whose manifest declares `[autonomous]`. Same RBAC
/// carve-out as [`SYSTEM_CHANNEL_CRON`] — both are kernel-internal and
/// have no user to attribute to. Issue #3243.
pub const SYSTEM_CHANNEL_AUTONOMOUS: &str = "autonomous";

/// Synthetic `SenderContext.channel` value the dashboard WebSocket
/// (`librefang-api::ws::handle_ws`) uses when forwarding browser-initiated
/// turns to the kernel. The display name is hard-coded "Web UI" and
/// `user_id` is the resolved client IP — neither is a real human identity,
/// which is why the sender-prefix builder (`librefang-runtime::
/// agent_loop::build_sender_prefix`, #4666) carves this channel out
/// alongside [`SYSTEM_CHANNEL_CRON`] / [`SYSTEM_CHANNEL_AUTONOMOUS`].
pub const SYSTEM_CHANNEL_WEBUI: &str = "webui";

/// Minimum tolerated value for `cron_session_max_messages` (#3459).
/// Mirrors `agent_loop::MIN_HISTORY_MESSAGES`. Smaller values silently
/// destroy enough history to break prompt cache reuse and tool-result
/// referencing.  `0` is treated as "disable" before this clamp is applied.
const MIN_CRON_HISTORY_MESSAGES: usize = 4;

/// Resolve `cron_session_max_messages` from config into an effective cap.
///
/// - `None`    → no cap (pass through)
/// - `Some(0)` → caller set "disable"; treat as no cap
/// - `Some(n)` where `n < MIN_CRON_HISTORY_MESSAGES` → clamp up, emit warning
/// - `Some(n)` otherwise → use as-is
pub(crate) fn resolve_cron_max_messages(raw: Option<usize>) -> Option<usize> {
    match raw {
        None => None,
        Some(0) => None,
        Some(n) if n < MIN_CRON_HISTORY_MESSAGES => {
            tracing::warn!(
                requested = n,
                applied = MIN_CRON_HISTORY_MESSAGES,
                "cron_session_max_messages too small; clamped"
            );
            Some(MIN_CRON_HISTORY_MESSAGES)
        }
        other => other,
    }
}

/// Resolve `cron_session_max_tokens` from config into an effective cap.
///
/// - `None`    → no cap
/// - `Some(0)` → disable (treat as no cap)
/// - `Some(n)` otherwise → use as-is
pub(crate) fn resolve_cron_max_tokens(raw: Option<u64>) -> Option<u64> {
    match raw {
        Some(0) => None,
        other => other,
    }
}

/// Resolve the cron session-size warn threshold (#3693).
///
/// Pure function so it can be unit-tested without a kernel.  Returns
/// the absolute token count at which the kernel should emit a
/// `tracing::warn!` after pruning — or `None` to skip warning.
///
/// Inputs:
/// - `max_tokens`     — already-resolved `cron_session_max_tokens`
///   (post `resolve_cron_max_tokens`).
/// - `warn_fallback`  — `cron_session_warn_total_tokens`, used when
///   `max_tokens` is `None`.
/// - `fraction`       — `cron_session_warn_fraction`. Must be in
///   `(0.0, 1.0]`; out-of-range or non-finite values disable the
///   warn.
pub(crate) fn resolve_cron_warn_threshold(
    max_tokens: Option<u64>,
    warn_fallback: Option<u64>,
    fraction: Option<f64>,
) -> Option<u64> {
    let frac = fraction?;
    if !frac.is_finite() || frac <= 0.0 || frac > 1.0 {
        return None;
    }
    let budget = max_tokens.or(warn_fallback)?;
    if budget == 0 {
        return None;
    }
    // ceil so a near-budget estimate still trips the warn before the
    // hard cap; saturate to budget so callers can compare with `>=`.
    let raw = (budget as f64) * frac;
    let threshold = raw.ceil() as u64;
    Some(threshold.min(budget))
}

// ---------------------------------------------------------------------------
// Per-task trigger recursion depth (bug #3780)
// ---------------------------------------------------------------------------

// Per-task trigger-chain recursion depth counter.
// Declared at module level so it has a true `'static` key, as required by
// `tokio::task_local!`.  Each independent event-processing task establishes
// its own scope via `PUBLISH_EVENT_DEPTH.scope(Cell::new(0), future)`,
// keeping depth counts isolated between concurrent chains.
tokio::task_local! {
    static PUBLISH_EVENT_DEPTH: std::cell::Cell<u32>;
}

/// Extract a `(user_text, assistant_text)` seed pair for session-label
/// generation.  Returns `None` when the session lacks at least one
/// non-empty user message AND one non-empty assistant message — there
/// is nothing to title until both sides have spoken once.
fn extract_label_seed(messages: &[librefang_types::message::Message]) -> Option<(String, String)> {
    use librefang_types::message::{ContentBlock, MessageContent, Role};

    fn text_of(m: &librefang_types::message::Message) -> String {
        match &m.content {
            MessageContent::Text(t) => t.trim().to_string(),
            MessageContent::Blocks(blocks) => {
                let mut buf = String::new();
                for b in blocks {
                    if let ContentBlock::Text { text, .. } = b {
                        if !buf.is_empty() {
                            buf.push(' ');
                        }
                        buf.push_str(text.trim());
                    }
                }
                buf
            }
        }
    }

    let user = messages
        .iter()
        .find(|m| m.role == Role::User)
        .map(text_of)
        .filter(|s| !s.is_empty())?;
    let assistant = messages
        .iter()
        .find(|m| m.role == Role::Assistant)
        .map(text_of)
        .filter(|s| !s.is_empty())?;
    Some((user, assistant))
}

/// Clean up a raw model-generated title: strip surrounding quotes,
/// keep only the first line, and cap at 60 chars (UTF-8 safe).  Models
/// occasionally prefix with `Title:` or wrap in quotes despite the
/// prompt — the cleanup keeps the column rendering tidy without
/// rejecting otherwise-valid titles.
fn sanitize_session_title(raw: &str) -> String {
    let first_line = raw.lines().next().unwrap_or("").trim();
    // Strip a leading "Title:" / "title:" prefix some models add.
    let without_prefix = first_line
        .strip_prefix("Title:")
        .or_else(|| first_line.strip_prefix("title:"))
        .unwrap_or(first_line)
        .trim();
    // Strip surrounding ASCII quotes / single quotes / backticks.
    let trimmed = without_prefix
        .trim_matches('"')
        .trim_matches('\'')
        .trim_matches('`')
        .trim();
    // Cap at 60 chars (UTF-8 safe) — same ceiling derive_session_label
    // uses, so list views don't shift width when one path beats the
    // other.
    librefang_types::truncate_str(trimmed, 60)
        .trim()
        .to_string()
}

/// Build the MCP bridge config that lets CLI-based drivers (Claude Code)
/// reach back into the daemon's own `/mcp` endpoint. Uses loopback when the
/// API listens on a wildcard address.
fn build_mcp_bridge_cfg(cfg: &KernelConfig) -> librefang_llm_driver::McpBridgeConfig {
    let listen = cfg.api_listen.trim();
    let base = if listen.is_empty() {
        "http://127.0.0.1:4545".to_string()
    } else if listen.starts_with("0.0.0.0")
        || listen.starts_with("[::]")
        || listen.starts_with("::")
    {
        let port = listen.rsplit(':').next().unwrap_or("4545");
        format!("http://127.0.0.1:{port}")
    } else {
        format!("http://{listen}")
    };
    let api_key = if cfg.api_key.is_empty() {
        None
    } else {
        Some(cfg.api_key.clone())
    };
    librefang_llm_driver::McpBridgeConfig {
        base_url: base,
        api_key,
    }
}

// ---------------------------------------------------------------------------
// Prompt metadata cache — avoids redundant filesystem I/O and skill registry
// iteration on every message.
// ---------------------------------------------------------------------------

/// TTL for cached prompt metadata entries (30 seconds).
const PROMPT_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// Best-effort load of the raw `config.toml` as a `toml::Value` for
/// skill config-var injection.  Used **only** at boot and on
/// `reload_config` — never on the per-message hot path (#3722).
///
/// A missing or unparseable file falls back to an empty table, matching
/// the behaviour the inline read previously had on `read_to_string` /
/// `from_str` errors.
fn load_raw_config_toml(config_path: &Path) -> toml::Value {
    let empty = || toml::Value::Table(toml::map::Map::new());
    if !config_path.exists() {
        return empty();
    }
    let contents = match std::fs::read_to_string(config_path) {
        Ok(s) => s,
        Err(e) => {
            // Not on the hot path — surface the failure so a misconfigured
            // file doesn't silently disable `[skills.config.*]` injection
            // for the whole process lifetime.
            tracing::warn!(
                path = %config_path.display(),
                error = %e,
                "failed to read raw config.toml for skill config injection; \
                 falling back to empty table"
            );
            return empty();
        }
    };
    match toml::from_str(&contents) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                path = %config_path.display(),
                error = %e,
                "failed to parse raw config.toml for skill config injection; \
                 falling back to empty table"
            );
            empty()
        }
    }
}

/// Cached workspace context and identity files for an agent's workspace.
#[derive(Clone, Debug)]
pub(crate) struct CachedWorkspaceMetadata {
    workspace_context: Option<String>,
    soul_md: Option<String>,
    user_md: Option<String>,
    memory_md: Option<String>,
    agents_md: Option<String>,
    bootstrap_md: Option<String>,
    identity_md: Option<String>,
    heartbeat_md: Option<String>,
    tools_md: Option<String>,
    created_at: std::time::Instant,
}

impl CachedWorkspaceMetadata {
    fn is_expired(&self) -> bool {
        self.created_at.elapsed() > PROMPT_CACHE_TTL
    }
}

/// Cached skill summary and prompt context for a given skill allowlist.
#[derive(Clone, Debug)]
pub(crate) struct CachedSkillMetadata {
    skill_summary: String,
    skill_prompt_context: String,
    /// Total number of enabled skills represented in this summary.
    /// Used by the prompt builder for progressive disclosure (inline vs summary mode).
    skill_count: usize,
    /// Pre-formatted skill config variable section for the system prompt.
    /// Empty when no skills declare config variables or none have resolvable values.
    skill_config_section: String,
    created_at: std::time::Instant,
}

impl CachedSkillMetadata {
    fn is_expired(&self) -> bool {
        self.created_at.elapsed() > PROMPT_CACHE_TTL
    }
}

/// Cached tool list for an agent, keyed by agent ID.
/// Stores the computed tool definitions along with generation counters that were
/// current at the time the cache was populated, enabling staleness detection.
#[derive(Clone, Debug)]
struct CachedToolList {
    tools: Arc<Vec<ToolDefinition>>,
    skill_generation: u64,
    mcp_generation: u64,
    created_at: std::time::Instant,
}

impl CachedToolList {
    fn is_expired(&self) -> bool {
        self.created_at.elapsed() > PROMPT_CACHE_TTL
    }

    fn is_stale(&self, skill_gen: u64, mcp_gen: u64) -> bool {
        self.skill_generation != skill_gen || self.mcp_generation != mcp_gen
    }
}

/// Thread-safe cache for prompt-building metadata. Avoids redundant filesystem
/// scans and skill registry iteration on every incoming message.
///
/// Keyed by workspace path (for workspace metadata) and a sorted skill
/// allowlist string (for skill metadata). Entries expire after [`PROMPT_CACHE_TTL`].
///
/// Invalidated explicitly on skill reload, config reload, or workspace change.
struct PromptMetadataCache {
    workspace: dashmap::DashMap<PathBuf, CachedWorkspaceMetadata>,
    skills: dashmap::DashMap<String, CachedSkillMetadata>,
    /// Per-agent cached tool list. Invalidated by TTL, generation counters
    /// (skill reload / MCP tool changes), or explicit removal.
    tools: dashmap::DashMap<AgentId, CachedToolList>,
}

impl PromptMetadataCache {
    fn new() -> Self {
        Self {
            workspace: dashmap::DashMap::new(),
            skills: dashmap::DashMap::new(),
            tools: dashmap::DashMap::new(),
        }
    }

    /// Invalidate all cached entries (used on skill reload, config reload).
    fn invalidate_all(&self) {
        self.workspace.clear();
        self.skills.clear();
        self.tools.clear();
    }

    /// Build a cache key for the skill allowlist.
    fn skill_cache_key(allowlist: &[String]) -> String {
        if allowlist.is_empty() {
            return String::from("*");
        }
        let mut sorted = allowlist.to_vec();
        sorted.sort();
        sorted.join(",")
    }
}

/// The main LibreFang kernel — coordinates all subsystems.
/// Stub LLM driver used when no providers are configured.
/// Returns a helpful error so the dashboard still boots and users can configure providers.
struct StubDriver;

#[async_trait]
impl LlmDriver for StubDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Err(LlmError::MissingApiKey(
            "No LLM provider configured. Set an API key (e.g. GROQ_API_KEY) and restart, \
             configure a provider via the dashboard, \
             or use Ollama for local models (no API key needed)."
                .to_string(),
        ))
    }

    fn is_configured(&self) -> bool {
        false
    }
}

#[derive(Clone, PartialEq, Eq)]
struct RotationKeySpec {
    name: String,
    api_key: String,
    use_primary_driver: bool,
}

/// Custom Debug impl that redacts the API key to prevent accidental log leakage.
impl std::fmt::Debug for RotationKeySpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RotationKeySpec")
            .field("name", &self.name)
            .field("api_key", &"<redacted>")
            .field("use_primary_driver", &self.use_primary_driver)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AssistantRouteTarget {
    Specialist(String),
    Hand(String),
}

impl AssistantRouteTarget {
    fn route_type(&self) -> &'static str {
        match self {
            Self::Specialist(_) => "specialist",
            Self::Hand(_) => "hand",
        }
    }

    fn name(&self) -> &str {
        match self {
            Self::Specialist(name) | Self::Hand(name) => name,
        }
    }
}

fn collect_rotation_key_specs(
    profiles: Option<&[AuthProfile]>,
    primary_api_key: Option<&str>,
) -> Vec<RotationKeySpec> {
    let mut seen_keys = HashSet::new();
    let mut specs = Vec::new();
    let mut sorted_profiles = profiles.map_or_else(Vec::new, |items| items.to_vec());
    sorted_profiles.sort_by_key(|profile| profile.priority);

    for profile in sorted_profiles {
        let Ok(api_key) = std::env::var(&profile.api_key_env) else {
            warn!(
                profile = %profile.name,
                env_var = %profile.api_key_env,
                "Auth profile env var not set — skipping"
            );
            continue;
        };
        if api_key.is_empty() || !seen_keys.insert(api_key.clone()) {
            continue;
        }
        specs.push(RotationKeySpec {
            name: profile.name,
            use_primary_driver: primary_api_key == Some(api_key.as_str()),
            api_key,
        });
    }

    if let Some(primary_api_key) = primary_api_key.filter(|key| !key.is_empty()) {
        if seen_keys.insert(primary_api_key.to_string()) {
            specs.insert(
                0,
                RotationKeySpec {
                    name: "primary".to_string(),
                    api_key: primary_api_key.to_string(),
                    use_primary_driver: true,
                },
            );
        }
    }

    specs
}

/// Resolve the effective session id used by the dispatch site in
/// `send_message_full_with_upstream`. Mirrors the resolution that
/// `execute_llm_agent` performs internally so the kernel and any failure /
/// supervisor logs agree on which session id was actually used — including
/// when `session_mode = "new"` would otherwise mint a fresh id deeper in
/// the stack. Returns `None` for module types that do not carry a session
/// (wasm, python).
fn resolve_dispatch_session_id(
    module: &str,
    agent_id: AgentId,
    entry_session_id: SessionId,
    manifest_session_mode: librefang_types::agent::SessionMode,
    sender_context: Option<&SenderContext>,
    session_mode_override: Option<librefang_types::agent::SessionMode>,
    session_id_override: Option<SessionId>,
) -> Option<SessionId> {
    if module.starts_with("wasm:") || module.starts_with("python:") {
        return None;
    }
    if let Some(sid) = session_id_override {
        return Some(sid);
    }
    Some(match sender_context {
        Some(ctx) if !ctx.channel.is_empty() && !ctx.use_canonical_session => {
            SessionId::for_sender_scope(agent_id, &ctx.channel, ctx.chat_id.as_deref())
        }
        _ => {
            let mode = session_mode_override.unwrap_or(manifest_session_mode);
            match mode {
                librefang_types::agent::SessionMode::Persistent => entry_session_id,
                librefang_types::agent::SessionMode::New => SessionId::new(),
            }
        }
    })
}

/// One in-flight `(agent, session)` loop. Stored in
/// `LibreFangKernel.running_tasks` to support per-session cancellation
/// (`stop_session_run`) and runtime introspection
/// (`list_running_sessions` / `GET /api/agents/{id}/runtime`).
///
/// `started_at` is captured at spawn time, before the agent loop yields
/// — callers reading the snapshot get a stable wall-clock timestamp for
/// "when was this turn launched", independent of how long the loop has
/// been blocked on the LLM or a tool. UTC, RFC3339-serialised on the wire.
pub(crate) struct RunningTask {
    pub(crate) abort: tokio::task::AbortHandle,
    pub(crate) started_at: chrono::DateTime<chrono::Utc>,
    /// Unique id for this turn — used by cleanup to ensure a task only
    /// removes its OWN entry from `running_tasks`, never a successor's
    /// (#3445 stale-entry guard). Compared with `Uuid` equality.
    pub(crate) task_id: uuid::Uuid,
}

pub struct LibreFangKernel {
    /// Boot-time home directory (immutable — cannot hot-reload).
    home_dir_boot: PathBuf,
    /// Boot-time data directory (immutable — cannot hot-reload).
    data_dir_boot: PathBuf,
    /// Kernel configuration (atomically swappable for hot-reload).
    pub(crate) config: ArcSwap<KernelConfig>,
    /// Cached raw `config.toml` value used for skill config-var injection.
    ///
    /// Refreshed once at boot and once per successful `reload_config` call —
    /// **never** on the per-message hot path (#3722).  `KernelConfig` itself
    /// is strongly-typed and does not preserve the open-ended
    /// `[skills.config.<key>]` namespace that `resolve_config_vars`
    /// walks, so we keep a separate `toml::Value` snapshot.
    pub(crate) raw_config_toml: ArcSwap<toml::Value>,
    /// Agent registries + scheduler + supervisor + lock maps + traces.
    /// See [`subsystems::AgentSubsystem`].
    pub(crate) agents: subsystems::AgentSubsystem,
    /// Event buses + mid-turn injection channels + sticky routing
    /// state + session-stream-hub GC guard. See
    /// [`subsystems::EventSubsystem`].
    pub(crate) events: subsystems::EventSubsystem,
    /// Memory substrate + wiki vault + proactive memory + prompt store.
    /// See [`subsystems::MemorySubsystem`].
    pub(crate) memory: subsystems::MemorySubsystem,
    /// Workflow engine + triggers + background + cron + command queue.
    /// See [`subsystems::WorkflowSubsystem`].
    pub(crate) workflows: subsystems::WorkflowSubsystem,
    /// LLM drivers + model catalog + embedding fallback. See
    /// [`subsystems::LlmSubsystem`].
    pub(crate) llm: subsystems::LlmSubsystem,
    /// WASM sandbox engine (shared across all WASM agent executions).
    wasm_sandbox: WasmSandbox,
    /// RBAC + device pairing + credential vault. See
    /// [`subsystems::SecuritySubsystem`].
    pub(crate) security: subsystems::SecuritySubsystem,
    /// Plugin skill registry + hand registry + skill review bookkeeping.
    /// See [`subsystems::SkillsSubsystem`].
    pub(crate) skills: subsystems::SkillsSubsystem,
    /// MCP connection pool + OAuth + tool cache + catalog + health
    /// monitor + summary cache. See [`subsystems::McpSubsystem`].
    pub(crate) mcp: subsystems::McpSubsystem,
    /// Web search + browser + media understanding + TTS + media drivers.
    /// See [`subsystems::MediaSubsystem`].
    pub(crate) media: subsystems::MediaSubsystem,
    /// A2A registry + OFP peers + channel adapters + bindings + broadcast
    /// + delivery tracker. See [`subsystems::MeshSubsystem`].
    pub(crate) mesh: subsystems::MeshSubsystem,
    /// Approval enforcement + lifecycle hooks + sweeper guards. See
    /// [`subsystems::GovernanceSubsystem`].
    pub(crate) governance: subsystems::GovernanceSubsystem,
    /// Per-LibreFang-session ACP `fs/*` clients, populated by the ACP
    /// adapter at `session/new` time so runtime tools can route file
    /// reads/writes back through the editor instead of the local
    /// filesystem (#3313). Lookup is per-tool-call so we keep this as a
    /// top-level field rather than wrapping it in a subsystem — there's
    /// no other coupled state, and the runtime accesses the map via
    /// `kernel.acp_fs_clients` at the deepest tool path.
    pub(crate) acp_fs_clients: dashmap::DashMap<
        librefang_types::agent::SessionId,
        std::sync::Arc<dyn librefang_runtime::kernel_handle::AcpFsClient>,
    >,
    /// Per-LibreFang-session ACP `terminal/*` clients (#3313).
    /// Same shape as `acp_fs_clients`; lets `shell_exec` route
    /// commands through the editor's terminal panel.
    pub(crate) acp_terminal_clients: dashmap::DashMap<
        librefang_types::agent::SessionId,
        std::sync::Arc<dyn librefang_runtime::kernel_handle::AcpTerminalClient>,
    >,
    /// Auto-reply engine.
    pub(crate) auto_reply_engine: crate::auto_reply::AutoReplyEngine,
    /// Persistent + background process registries. See
    /// [`subsystems::ProcessSubsystem`].
    pub(crate) processes: subsystems::ProcessSubsystem,
    /// Boot timestamp for uptime calculation.
    pub(crate) booted_at: std::time::Instant,
    /// WhatsApp Web gateway child process PID (for shutdown cleanup).
    pub(crate) whatsapp_gateway_pid: Arc<std::sync::Mutex<Option<u32>>>,
    /// Hot-reloadable tool policy override (set via config hot-reload, read in available_tools).
    pub(crate) tool_policy_override:
        std::sync::RwLock<Option<librefang_types::tool_policy::ToolPolicy>>,
    /// Pluggable context engine for memory recall, assembly, and compaction.
    pub(crate) context_engine: Option<Box<dyn librefang_runtime::context_engine::ContextEngine>>,
    /// Runtime config passed to context-engine lifecycle hooks.
    context_engine_config: librefang_runtime::context_engine::ContextEngineConfig,
    /// Weak self-reference for trigger dispatch (set after Arc wrapping).
    self_handle: OnceLock<Weak<LibreFangKernel>>,
    /// Whether we've already logged the "no provider" audit entry (prevents spam).
    pub(crate) provider_unconfigured_logged: std::sync::atomic::AtomicBool,
    /// Config reload barrier — write-locked during `apply_hot_actions_inner` to prevent
    /// concurrent readers from seeing a half-updated configuration (e.g. new provider
    /// URLs but old default model). Read-locked in message hot paths so multiple
    /// requests proceed in parallel but block briefly during a reload.
    /// Uses `tokio::sync::RwLock` so guards are `Send` and can be held across `.await`.
    pub(crate) config_reload_lock: tokio::sync::RwLock<()>,
    /// Cache for workspace context, identity files, and skill metadata to avoid
    /// redundant filesystem I/O and registry scans on every message.
    prompt_metadata_cache: PromptMetadataCache,
    /// Audit trail + cost metering + hot-reloadable budget. See
    /// [`subsystems::MeteringSubsystem`].
    pub(crate) metering: subsystems::MeteringSubsystem,
    /// Shutdown signal sender for background tasks (e.g., approval expiry sweep).
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    /// Checkpoint manager — takes automatic shadow-git snapshots before every
    /// `file_write` / `apply_patch` tool call.  `None` when the base
    /// directory could not be resolved at boot.
    pub(crate) checkpoint_manager:
        Option<Arc<librefang_runtime::checkpoint_manager::CheckpointManager>>,
    /// Live, atomically-swappable handle to `KernelConfig.taint_rules`.
    ///
    /// The kernel mirrors `config.load().taint_rules` into this swap on boot
    /// and on every config reload (see [`Self::reload_config`]). Each
    /// connected MCP server holds an [`Arc::clone`] of this same swap as its
    /// `taint_rule_sets` field, so reading via `.load()` at scan time always
    /// returns the latest registry — without restarting the server. The
    /// scanner takes a single `.load()` per call so a mid-call reload can't
    /// change the rule set under an in-flight tool invocation.
    pub(crate) taint_rules_swap: librefang_runtime::mcp::TaintRuleSetsHandle,
    /// Pluggable hook that swaps the live tracing `EnvFilter` when
    /// `config.log_level` changes via hot-reload. Injected by the binary
    /// (`librefang-cli` for the daemon) post-construction; absent for
    /// in-process callers that don't own a tracing subscriber, in which
    /// case `log_level` changes still update `KernelConfig` in-memory but
    /// don't take effect on the active filter (the hot-reload action is a
    /// no-op with a warning).
    pub(crate) log_reloader: OnceLock<crate::log_reload::LogLevelReloaderArc>,
}

/// Bounded in-memory delivery receipt tracker.
/// Stores up to `MAX_RECEIPTS` most recent delivery receipts per agent.
pub struct DeliveryTracker {
    receipts: dashmap::DashMap<AgentId, Vec<librefang_channels::types::DeliveryReceipt>>,
}

impl Default for DeliveryTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl DeliveryTracker {
    const MAX_RECEIPTS: usize = 10_000;
    const MAX_PER_AGENT: usize = 500;

    /// Create a new empty delivery tracker.
    pub fn new() -> Self {
        Self {
            receipts: dashmap::DashMap::new(),
        }
    }

    /// Record a delivery receipt for an agent.
    pub fn record(&self, agent_id: AgentId, receipt: librefang_channels::types::DeliveryReceipt) {
        let mut entry = self.receipts.entry(agent_id).or_default();
        entry.push(receipt);
        // Per-agent cap
        if entry.len() > Self::MAX_PER_AGENT {
            let drain = entry.len() - Self::MAX_PER_AGENT;
            entry.drain(..drain);
        }
        // Global cap: evict oldest agents' receipts if total exceeds limit
        drop(entry);
        let total: usize = self.receipts.iter().map(|e| e.value().len()).sum();
        if total > Self::MAX_RECEIPTS {
            // Simple eviction: remove oldest entries from first agent found
            if let Some(mut oldest) = self.receipts.iter_mut().next() {
                let to_remove = total - Self::MAX_RECEIPTS;
                let drain = to_remove.min(oldest.value().len());
                oldest.value_mut().drain(..drain);
            }
        }
    }

    /// Get recent delivery receipts for an agent (newest first).
    pub fn get_receipts(
        &self,
        agent_id: AgentId,
        limit: usize,
    ) -> Vec<librefang_channels::types::DeliveryReceipt> {
        self.receipts
            .get(&agent_id)
            .map(|entries| entries.iter().rev().take(limit).cloned().collect())
            .unwrap_or_default()
    }

    /// Create a receipt for a successful send.
    pub fn sent_receipt(
        channel: &str,
        recipient: &str,
    ) -> librefang_channels::types::DeliveryReceipt {
        librefang_channels::types::DeliveryReceipt {
            message_id: uuid::Uuid::new_v4().to_string(),
            channel: channel.to_string(),
            recipient: Self::sanitize_recipient(recipient),
            status: librefang_channels::types::DeliveryStatus::Sent,
            timestamp: chrono::Utc::now(),
            error: None,
        }
    }

    /// Create a receipt for a failed send.
    pub fn failed_receipt(
        channel: &str,
        recipient: &str,
        error: &str,
    ) -> librefang_channels::types::DeliveryReceipt {
        librefang_channels::types::DeliveryReceipt {
            message_id: uuid::Uuid::new_v4().to_string(),
            channel: channel.to_string(),
            recipient: Self::sanitize_recipient(recipient),
            status: librefang_channels::types::DeliveryStatus::Failed,
            timestamp: chrono::Utc::now(),
            // Sanitize error: no credentials, max 256 chars
            error: Some(
                error
                    .chars()
                    .take(256)
                    .collect::<String>()
                    .replace(|c: char| c.is_control(), ""),
            ),
        }
    }

    /// Sanitize recipient to avoid PII logging.
    fn sanitize_recipient(recipient: &str) -> String {
        let s: String = recipient
            .chars()
            .filter(|c| !c.is_control())
            .take(64)
            .collect();
        s
    }

    /// Remove receipt entries for agents not in the live set.
    pub fn gc_stale_agents(&self, live_agents: &std::collections::HashSet<AgentId>) -> usize {
        let stale: Vec<AgentId> = self
            .receipts
            .iter()
            .filter(|entry| !live_agents.contains(entry.key()))
            .map(|entry| *entry.key())
            .collect();
        let count = stale.len();
        for id in stale {
            self.receipts.remove(&id);
        }
        count
    }
}

mod workspace_setup;
use workspace_setup::*;

/// Spawn a fire-and-forget tokio task that logs panics instead of silently
/// swallowing them (#3740).
///
/// `tokio::spawn` drops panics when the returned `JoinHandle` is not awaited.
/// This wrapper catches any panic from the inner future and logs it at `error`
/// level so it surfaces in traces and structured logs.
///
/// Thin alias over [`crate::supervised_spawn::spawn_supervised`] (#3740) — kept
/// for the existing `spawn_logged(tag, fut)` call sites in this file.
fn spawn_logged(
    tag: &'static str,
    fut: impl std::future::Future<Output = ()> + Send + 'static,
) -> tokio::task::JoinHandle<()> {
    crate::supervised_spawn::spawn_supervised(tag, fut)
}

/// SECURITY (#3533): reject manifest `module` strings that escape the
/// LibreFang home dir. Centralised so every entry point that accepts a
/// manifest goes through the same check — without this, hot-reload,
/// `update_manifest`, and boot-time SQLite restore all bypassed the
/// validation that lived inline in `spawn_agent_inner` and a hostile
/// `agent.toml` (peer push, MCP-installed agent, skill bundle, or just
/// edit on disk + restart) could ship `module = "python:/etc/passwd.py"`
/// and have the host interpreter exec it under the agent's capabilities.
///
/// Returns `Err(KernelError)` ready to be `?`-propagated by callers; logs
/// a `warn!` with the agent name so the rejection is visible to operators
/// even when the caller chooses to skip-and-continue (e.g. the boot loop
/// must not abort the whole process for one bad manifest).
fn validate_manifest_module_path(manifest: &AgentManifest, agent_name: &str) -> KernelResult<()> {
    if let Err(reason) = librefang_runtime::python_runtime::validate_module_string(&manifest.module)
    {
        warn!(agent = %agent_name, %reason, "Rejecting manifest — invalid module path");
        return Err(KernelError::LibreFang(
            librefang_types::error::LibreFangError::Internal(format!(
                "Invalid module path: {reason}"
            )),
        ));
    }
    Ok(())
}

// Accessors / lifecycle helpers live in `kernel::accessors`.

mod manifest_helpers;
use manifest_helpers::*;

// ── Background skill review helpers ────────────────────────────────
//
// These are top-level so they can be unit-tested without constructing
// a kernel, and so `background_skill_review` — a method on
// `LibreFangKernel` — can import them by short name.

/// Classification of errors returned from `background_skill_review`.
///
/// The retry loop in [`LibreFangKernel::serve_agent`] treats `Transient`
/// as retry-eligible and `Permanent` as "break out immediately". See the
/// docstring on `background_skill_review` for the detailed rules.
#[derive(Debug, Clone)]
pub(crate) enum ReviewError {
    /// Network / timeout / rate-limit / LLM-driver fault; retry OK.
    Transient(String),
    /// Parse / validation / security-blocked; retry would be
    /// non-idempotent (fresh LLM call, different output each time).
    Permanent(String),
}

// `mcp_summary_cache_key` and `render_mcp_summary` live in `kernel::mcp_summary`.

// `sanitize_reviewer_line` and `sanitize_reviewer_block` live in `kernel::reviewer_sanitize`.

// `cron_script_wake_gate`, `atomic_write_toml`, and the private `parse_wake_gate` helper live in `kernel::cron_script`.

/// Adapter from the kernel's `send_channel_message` to the
/// `CronChannelSender` trait used by the multi-target fan-out engine.
struct KernelCronBridge {
    kernel: Arc<LibreFangKernel>,
}

// `CronChannelSender` impl, `cron_fan_out_targets`, and `cron_deliver_response` live in `kernel::cron_bridge`. The `KernelCronBridge` struct definition stays here because it holds an `Arc<LibreFangKernel>` shared with the rest of the cron dispatcher.

impl LibreFangKernel {
    /// Mark all active Hands' cron jobs as due-now so the next scheduler tick fires them.
    /// Called after a provider is first configured so Hands resume immediately.
    /// Update registry entries for agents that should track the kernel default model.
    /// Called after a provider switch so agents pick up the new provider without restart.
    ///
    /// Agents eligible for update:
    /// - Any agent with provider="default" or "" (new spawn-time behavior)
    /// - The auto-spawned "assistant" agent (may have stale concrete provider in DB)
    /// - Dashboard-created agents (no source_toml_path, no custom api_key_env) whose
    ///   stored provider matches `old_provider` — these were using the old default
    pub fn sync_default_model_agents(
        &self,
        old_provider: &str,
        dm: &librefang_types::config::DefaultModelConfig,
    ) {
        for entry in self.agents.registry.list() {
            let is_default_provider = entry.manifest.model.provider.is_empty()
                || entry.manifest.model.provider == "default";
            let is_default_model =
                entry.manifest.model.model.is_empty() || entry.manifest.model.model == "default";
            let is_auto_spawned = entry.name == "assistant"
                && entry.manifest.description == "General-purpose assistant";
            // Dashboard-created agents that were using the old default provider:
            // no source TOML, no custom API key, and saved provider == old default
            let is_stale_dashboard_default = entry.source_toml_path.is_none()
                && entry.manifest.model.api_key_env.is_none()
                && entry.manifest.model.base_url.is_none()
                && entry.manifest.model.provider == old_provider;

            if (is_default_provider && is_default_model)
                || is_auto_spawned
                || is_stale_dashboard_default
            {
                let _ = self.agents.registry.update_model_and_provider(
                    entry.id,
                    dm.model.clone(),
                    dm.provider.clone(),
                );
                if !dm.api_key_env.is_empty() {
                    if let Some(mut e) = self.agents.registry.get(entry.id) {
                        if e.manifest.model.api_key_env.is_none() {
                            e.manifest.model.api_key_env = Some(dm.api_key_env.clone());
                        }
                        if dm.base_url.is_some() && e.manifest.model.base_url.is_none() {
                            e.manifest.model.base_url.clone_from(&dm.base_url);
                        }
                        // Merge extra_params from default_model (agent-level keys take precedence)
                        for (key, value) in &dm.extra_params {
                            e.manifest
                                .model
                                .extra_params
                                .entry(key.clone())
                                .or_insert(value.clone());
                        }
                        let _ = self.memory.substrate.save_agent(&e);
                    }
                } else if let Some(e) = self.agents.registry.get(entry.id) {
                    let _ = self.memory.substrate.save_agent(&e);
                }
            }
        }
    }

    pub fn trigger_all_hands(&self) {
        let hand_agents: Vec<AgentId> = self
            .skills
            .hand_registry
            .list_instances()
            .into_iter()
            .filter(|inst| inst.status == librefang_hands::HandStatus::Active)
            .filter_map(|inst| inst.agent_id())
            .collect();

        for agent_id in &hand_agents {
            self.workflows
                .cron_scheduler
                .mark_due_now_by_agent(*agent_id);
        }

        if !hand_agents.is_empty() {
            info!(
                count = hand_agents.len(),
                "Marked active hands as due for immediate execution"
            );
        }
    }

    /// Push a notification message to a single [`NotificationTarget`].
    async fn push_to_target(
        &self,
        target: &librefang_types::approval::NotificationTarget,
        message: &str,
    ) {
        if let Err(e) = self
            .send_channel_message(
                &target.channel_type,
                &target.recipient,
                message,
                target.thread_id.as_deref(),
                None,
            )
            .await
        {
            warn!(
                channel = %target.channel_type,
                recipient = %target.recipient,
                error = %e,
                "Failed to push notification"
            );
        }
    }

    /// Push an interactive approval notification with Approve/Reject buttons.
    ///
    /// When TOTP is enabled, the message includes instructions for providing
    /// the TOTP code and the Approve button is removed (code must be typed).
    async fn push_approval_interactive(
        &self,
        target: &librefang_types::approval::NotificationTarget,
        message: &str,
        request_id: &str,
    ) {
        let short_id = &request_id[..std::cmp::min(8, request_id.len())];
        let totp_enabled = self.governance.approval_manager.requires_totp();

        let display_message = if totp_enabled {
            format!("{message}\n\nTOTP required. Reply: /approve {short_id} <6-digit-code>")
        } else {
            message.to_string()
        };

        // When TOTP is enabled, only show Reject button (approve needs typed code).
        let buttons = if totp_enabled {
            vec![vec![librefang_channels::types::InteractiveButton {
                label: "Reject".to_string(),
                action: format!("/reject {short_id}"),
                style: Some("danger".to_string()),
                url: None,
            }]]
        } else {
            vec![vec![
                librefang_channels::types::InteractiveButton {
                    label: "Approve".to_string(),
                    action: format!("/approve {short_id}"),
                    style: Some("primary".to_string()),
                    url: None,
                },
                librefang_channels::types::InteractiveButton {
                    label: "Reject".to_string(),
                    action: format!("/reject {short_id}"),
                    style: Some("danger".to_string()),
                    url: None,
                },
            ]]
        };

        let interactive = librefang_channels::types::InteractiveMessage {
            text: display_message.clone(),
            buttons,
        };

        if let Some(adapter) = self.mesh.channel_adapters.get(&target.channel_type) {
            let user = librefang_channels::types::ChannelUser {
                platform_id: target.recipient.clone(),
                display_name: target.recipient.clone(),
                librefang_user: None,
            };
            if let Err(e) = adapter.send_interactive(&user, &interactive).await {
                warn!(
                    channel = %target.channel_type,
                    error = %e,
                    "Failed to send interactive approval notification, falling back to text"
                );
                // Fallback to plain text
                self.push_to_target(target, &display_message).await;
            }
        } else {
            // No adapter found — fall back to send_channel_message
            self.push_to_target(target, &display_message).await;
        }
    }

    /// Push a notification to all configured targets, resolving routing rules.
    /// Resolution: per-agent rules (matching event) > global channels for that event type.
    ///
    /// When `session_id` is `Some`, ` [session=<uuid>]` is appended to the
    /// delivered message so operators can correlate the alert with the
    /// failing session's history (matches the `session_id` field in the
    /// `Agent loop failed — recorded in supervisor` warn log).
    /// Pass `None` for agent-level alerts that aren't session-scoped
    /// (e.g. `health_check_failed`).
    async fn push_notification(
        &self,
        agent_id: &str,
        event_type: &str,
        message: &str,
        session_id: Option<&SessionId>,
    ) {
        use librefang_types::capability::glob_matches;
        let cfg = self.config.load_full();

        // Check per-agent notification rules first
        let agent_targets: Vec<librefang_types::approval::NotificationTarget> = cfg
            .notification
            .agent_rules
            .iter()
            .filter(|rule| {
                glob_matches(&rule.agent_pattern, agent_id)
                    && rule.events.iter().any(|e| e == event_type)
            })
            .flat_map(|rule| rule.channels.clone())
            .collect();

        let targets = if !agent_targets.is_empty() {
            agent_targets
        } else {
            // Fallback to global channels based on event type
            match event_type {
                "approval_requested" => cfg.notification.approval_channels.clone(),
                "task_completed" | "task_failed" | "tool_failure" | "health_check_failed" => {
                    cfg.notification.alert_channels.clone()
                }
                _ => Vec::new(),
            }
        };

        let delivered: std::borrow::Cow<'_, str> = match session_id {
            Some(sid) => std::borrow::Cow::Owned(format!("{message} [session={sid}]")),
            None => std::borrow::Cow::Borrowed(message),
        };

        for target in &targets {
            self.push_to_target(target, &delivered).await;
        }
    }

    /// Resolve an agent identifier string (either a UUID or a human-readable
    /// name) to a live `AgentId`. A valid-UUID-format string that doesn't
    /// resolve to a live agent falls through to name lookup so stale or
    /// hallucinated UUIDs from an LLM don't bypass the name path.
    ///
    /// On miss, the error lists every currently-registered agent so the
    /// caller (typically an LLM) can recover without an extra agent_list
    /// round trip.
    fn resolve_agent_identifier(&self, agent_id: &str) -> Result<AgentId, String> {
        if let Ok(uid) = agent_id.parse::<AgentId>() {
            if self.agents.registry.get(uid).is_some() {
                return Ok(uid);
            }
        }
        if let Some(entry) = self.agents.registry.find_by_name(agent_id) {
            return Ok(entry.id);
        }
        let available: Vec<String> = self
            .agents
            .registry
            .list()
            .iter()
            .map(|a| format!("{} ({})", a.name, a.id))
            .collect();
        Err(if available.is_empty() {
            format!("Agent not found: '{agent_id}'. No agents are currently registered.")
        } else {
            format!(
                "Agent not found: '{agent_id}'. Call agent_list to see valid agents. Currently registered: [{}]",
                available.join(", ")
            )
        })
    }
}

// ---- BEGIN role-trait impls (split from former `impl KernelHandle for LibreFangKernel`, #3746) ----
//
// All 16 `impl kernel_handle::* for LibreFangKernel` blocks now live in
// `kernel::handles::*`. Each sub-module is a descendant of `kernel`, so
// it retains access to `LibreFangKernel`'s private fields and inherent
// methods without any visibility surgery. Specifically:
//
//   - `kernel::handles::agent_control`    — `kernel_handle::AgentControl`
//   - `kernel::handles::memory_access`    — `kernel_handle::MemoryAccess`
//   - `kernel::handles::task_queue`       — `kernel_handle::TaskQueue`
//   - `kernel::handles::event_bus`        — `kernel_handle::EventBus`
//   - `kernel::handles::knowledge_graph`  — `kernel_handle::KnowledgeGraph`
//   - `kernel::handles::cron_control`     — `kernel_handle::CronControl`
//   - `kernel::handles::hands_control`    — `kernel_handle::HandsControl`
//   - `kernel::handles::approval_gate`    — `kernel_handle::ApprovalGate`
//   - `kernel::handles::a2a_registry`     — `kernel_handle::A2ARegistry`
//   - `kernel::handles::channel_sender`   — `kernel_handle::ChannelSender`
//   - `kernel::handles::prompt_store`     — `kernel_handle::PromptStore`
//   - `kernel::handles::workflow_runner`  — `kernel_handle::WorkflowRunner`
//   - `kernel::handles::goal_control`     — `kernel_handle::GoalControl`
//   - `kernel::handles::tool_policy`      — `kernel_handle::ToolPolicy`
//   - `kernel::handles::api_auth`         — `kernel_handle::ApiAuth`
//   - `kernel::handles::session_writer`   — `kernel_handle::SessionWriter`
//
// ---- END role-trait impls (#3746) ----

// ---------------------------------------------------------------------------
// Approval resolution helpers (Step 5)
// ---------------------------------------------------------------------------

impl LibreFangKernel {
    /// Render an agent identifier for human-facing messages: `"name" (short-id)`
    /// when the agent is in the registry, otherwise the raw id verbatim.
    ///
    /// Do not use this for audit detail strings or any field that downstream
    /// queries filter on — those need the canonical UUID so that
    /// `/api/audit/query?agent=<uuid>` keeps working. This helper is for
    /// operator-facing copy (push notifications, channel messages,
    /// human-readable descriptions) only.
    fn approval_agent_display(&self, agent_id: &str) -> String {
        if let Ok(aid) = agent_id.parse::<AgentId>() {
            if let Some(entry) = self.agents.registry.get(aid) {
                let short = agent_id.get(..8).unwrap_or(agent_id);
                // Names are user-configured free text. Escape embedded `"` so
                // adapters that interpret the surrounding context (Telegram
                // MarkdownV2, Discord, etc.) don't see a malformed message
                // that fails to render — operators can't approve what they
                // can't see.
                let safe_name = entry.name.replace('"', "\\\"");
                return format!("\"{}\" ({})", safe_name, short);
            }
        }
        format!("\"{}\"", agent_id)
    }

    async fn notify_escalated_approval(
        &self,
        req: &librefang_types::approval::ApprovalRequest,
        request_id: uuid::Uuid,
    ) {
        use librefang_types::capability::glob_matches;

        let policy = self.governance.approval_manager.policy();
        let cfg = self.config.load_full();
        let targets: Vec<librefang_types::approval::NotificationTarget> =
            if !req.route_to.is_empty() {
                req.route_to.clone()
            } else {
                let routed: Vec<_> = policy
                    .routing
                    .iter()
                    .filter(|r| glob_matches(&r.tool_pattern, &req.tool_name))
                    .flat_map(|r| r.route_to.clone())
                    .collect();
                if !routed.is_empty() {
                    routed
                } else {
                    let agent_routed: Vec<_> = cfg
                        .notification
                        .agent_rules
                        .iter()
                        .filter(|rule| {
                            glob_matches(&rule.agent_pattern, &req.agent_id)
                                && rule.events.iter().any(|e| e == "approval_requested")
                        })
                        .flat_map(|rule| rule.channels.clone())
                        .collect();
                    if !agent_routed.is_empty() {
                        agent_routed
                    } else {
                        cfg.notification.approval_channels.clone()
                    }
                }
            };

        let msg = format!(
            "{} ESCALATION #{}: Approval still needed: agent {} wants to run `{}` - {}",
            req.risk_level.emoji(),
            req.escalation_count,
            self.approval_agent_display(&req.agent_id),
            req.tool_name,
            req.description,
        );
        let req_id_str = request_id.to_string();
        for target in &targets {
            self.push_approval_interactive(target, &msg, &req_id_str)
                .await;
        }
    }

    /// Handle the aftermath of an approval decision: execute tool (if approved),
    /// build terminal result (if denied/expired/skipped), update session, notify agent.
    pub(crate) async fn handle_approval_resolution(
        &self,
        _request_id: uuid::Uuid,
        decision: librefang_types::approval::ApprovalDecision,
        deferred: librefang_types::tool::DeferredToolExecution,
    ) {
        use librefang_types::approval::ApprovalDecision;
        use librefang_types::tool::{ToolExecutionStatus, ToolResult};

        let agent_id = match uuid::Uuid::parse_str(&deferred.agent_id) {
            Ok(u) => AgentId(u),
            Err(e) => {
                warn!(
                    "handle_approval_resolution: invalid agent_id '{}': {e}",
                    deferred.agent_id
                );
                return;
            }
        };

        let result = match &decision {
            ApprovalDecision::Approved => match self.execute_deferred_tool(&deferred).await {
                Ok(r) => r,
                Err(e) => ToolResult::error(
                    deferred.tool_use_id.clone(),
                    format!("Failed to execute approved tool: {e}"),
                ),
            },
            ApprovalDecision::Denied => ToolResult::with_status(
                deferred.tool_use_id.clone(),
                format!(
                    "Tool '{}' was denied by human operator.",
                    deferred.tool_name
                ),
                ToolExecutionStatus::Denied,
            ),
            ApprovalDecision::TimedOut => ToolResult::with_status(
                deferred.tool_use_id.clone(),
                format!("Tool '{}' approval request expired.", deferred.tool_name),
                ToolExecutionStatus::Expired,
            ),
            ApprovalDecision::ModifyAndRetry { feedback } => ToolResult::with_status(
                deferred.tool_use_id.clone(),
                format!(
                    "[MODIFY_AND_RETRY] Tool '{}': {}",
                    deferred.tool_name, feedback
                ),
                ToolExecutionStatus::ModifyAndRetry,
            ),
            ApprovalDecision::Skipped => ToolResult::with_status(
                deferred.tool_use_id.clone(),
                format!("Tool '{}' was skipped.", deferred.tool_name),
                ToolExecutionStatus::Skipped,
            ),
        };

        // Let the live agent loop own patching and persistence when it can accept
        // the resolution signal. Fall back to direct session mutation only when the
        // agent is not currently reachable.
        if !self.notify_agent_of_resolution(&agent_id, &deferred, &decision, &result) {
            self.replace_tool_result_in_session(&agent_id, &deferred.tool_use_id, &result)
                .await;
        }
    }

    fn build_deferred_tool_exec_context<'a>(
        &'a self,
        kernel_handle: &'a Arc<dyn librefang_runtime::kernel_handle::KernelHandle>,
        skill_snapshot: &'a librefang_skills::registry::SkillRegistry,
        deferred: &'a librefang_types::tool::DeferredToolExecution,
    ) -> librefang_runtime::tool_runner::ToolExecContext<'a> {
        let cfg = self.config.load();
        librefang_runtime::tool_runner::ToolExecContext {
            kernel: Some(kernel_handle),
            allowed_tools: deferred.allowed_tools.as_deref(),
            // Deferred resume path has no live agent-loop context, so the
            // lazy-load meta-tools fall back to the builtin catalog.
            available_tools: None,
            caller_agent_id: Some(deferred.agent_id.as_str()),
            skill_registry: Some(skill_snapshot),
            // Deferred tools have already passed the approval gate; skill
            // allowlist is not available here so we skip the check (None).
            allowed_skills: None,
            mcp_connections: Some(&self.mcp.mcp_connections),
            web_ctx: Some(&self.media.web_ctx),
            browser_ctx: Some(&self.media.browser_ctx),
            allowed_env_vars: deferred.allowed_env_vars.as_deref(),
            workspace_root: deferred.workspace_root.as_deref(),
            media_engine: Some(&self.media.media_engine),
            media_drivers: Some(&self.media.media_drivers),
            exec_policy: deferred.exec_policy.as_ref(),
            tts_engine: Some(&self.media.tts_engine),
            docker_config: None,
            process_manager: Some(&self.processes.manager),
            sender_id: deferred.sender_id.as_deref(),
            channel: deferred.channel.as_deref(),
            // Restore the originating SessionId from v36's persisted
            // `deferred_payload` so a post-restart `Allow once` resumes
            // through the *original* editor's `acp_fs_client` /
            // `acp_terminal_client` rather than silently falling back
            // to local fs / shell. `None` for deferred rows written
            // before this field existed (pre-#3313 H1) — those still
            // resume, just without ACP routing.
            session_id: deferred.session_id,
            spill_threshold_bytes: cfg.tool_results.spill_threshold_bytes,
            max_artifact_bytes: cfg.tool_results.max_artifact_bytes,
            checkpoint_manager: self.checkpoint_manager.as_ref(),
            process_registry: Some(&self.processes.registry),
            // Deferred tool executions run after the originating session's turn
            // has already ended (approval flow), so no live session interrupt is
            // available.  We set None here; if a session interrupt is needed for
            // deferred tools in the future, wire it through DeferredToolExecution.
            interrupt: None,
            // Deferred executions have already passed the approval gate, and the
            // originating session's checker is no longer live — skip the
            // session-scoped dangerous-command check here.
            dangerous_command_checker: None,
        }
    }

    /// Execute a deferred tool after it has been approved.
    async fn execute_deferred_tool(
        &self,
        deferred: &librefang_types::tool::DeferredToolExecution,
    ) -> Result<librefang_types::tool::ToolResult, String> {
        use librefang_runtime::tool_runner::execute_tool_raw;

        // Build a kernel handle reference so tools can call back into the kernel.
        let kernel_handle: Arc<dyn librefang_runtime::kernel_handle::KernelHandle> =
            match self.self_handle.get().and_then(|w| w.upgrade()) {
                Some(arc) => arc,
                None => {
                    return Err("Kernel self-handle unavailable".to_string());
                }
            };

        // Snapshot the skill registry (drops the read lock before the async await).
        let skill_snapshot = self
            .skills
            .skill_registry
            .read()
            .map_err(|e| format!("skill_registry lock poisoned: {e}"))?
            .snapshot();

        let ctx = self.build_deferred_tool_exec_context(&kernel_handle, &skill_snapshot, deferred);

        let result = execute_tool_raw(
            &deferred.tool_use_id,
            &deferred.tool_name,
            &deferred.input,
            &ctx,
        )
        .await;

        Ok(result)
    }

    /// Replace or reconcile a resolved approval result in the persisted session.
    ///
    /// This fallback may run concurrently with an in-flight agent-loop save, so it
    /// always reloads the latest persisted session just before writing and only
    /// patches against that snapshot. If another writer already persisted the same
    /// terminal result, this becomes a no-op instead of appending a duplicate.
    async fn replace_tool_result_in_session(
        &self,
        agent_id: &AgentId,
        tool_use_id: &str,
        result: &librefang_types::tool::ToolResult,
    ) {
        // Resolve the agent's session_id from the registry.
        let session_id = match self.agents.registry.get(*agent_id) {
            Some(entry) => entry.session_id,
            None => {
                warn!(
                    agent_id = %agent_id,
                    "replace_tool_result_in_session: agent not found in registry"
                );
                return;
            }
        };

        let mut session = match self.memory.substrate.get_session_async(session_id).await {
            Ok(Some(s)) => s,
            Ok(None) => {
                warn!(
                    agent_id = %agent_id,
                    "replace_tool_result_in_session: session not found"
                );
                return;
            }
            Err(e) => {
                warn!(
                    agent_id = %agent_id,
                    error = %e,
                    "replace_tool_result_in_session: failed to load session"
                );
                return;
            }
        };

        fn reconcile_tool_result(
            session: &mut librefang_memory::session::Session,
            tool_use_id: &str,
            result: &librefang_types::tool::ToolResult,
        ) -> bool {
            use librefang_types::message::{ContentBlock, MessageContent};
            use librefang_types::tool::ToolExecutionStatus;

            let mut replaced = false;
            let mut already_final = false;
            let mut messages_mutated = false;
            'outer: for msg in &mut session.messages {
                let blocks = match &mut msg.content {
                    MessageContent::Blocks(blocks) => blocks,
                    _ => continue,
                };
                for block in blocks.iter_mut() {
                    if let ContentBlock::ToolResult {
                        tool_use_id: ref id,
                        content,
                        is_error,
                        status,
                        approval_request_id,
                        ..
                    } = block
                    {
                        if id == tool_use_id {
                            if *status == ToolExecutionStatus::WaitingApproval {
                                *content = result.content.clone();
                                *is_error = result.is_error;
                                *status = result.status;
                                *approval_request_id = None;
                                replaced = true;
                                messages_mutated = true;
                                break 'outer;
                            }

                            if *status == result.status && *content == result.content {
                                already_final = true;
                                break 'outer;
                            }
                        }
                    }
                }
            }

            if !replaced && !already_final {
                if let Some(last_message) = session.messages.last_mut() {
                    let block = ContentBlock::ToolResult {
                        tool_use_id: result.tool_use_id.clone(),
                        tool_name: result.tool_name.clone().unwrap_or_default(),
                        content: result.content.clone(),
                        is_error: result.is_error,
                        status: result.status,
                        approval_request_id: None,
                    };

                    match &mut last_message.content {
                        MessageContent::Blocks(blocks) => blocks.push(block),
                        MessageContent::Text(text) => {
                            let prior = std::mem::take(text);
                            last_message.content = MessageContent::Blocks(vec![
                                ContentBlock::Text {
                                    text: prior,
                                    provider_metadata: None,
                                },
                                block,
                            ]);
                        }
                    }
                    replaced = true;
                    messages_mutated = true;
                }
            }

            if messages_mutated {
                session.mark_messages_mutated();
            }

            replaced || already_final
        }

        if !reconcile_tool_result(&mut session, tool_use_id, result) {
            debug!(
                agent_id = %agent_id,
                tool_use_id,
                "replace_tool_result_in_session: terminal result already present or no writable message found"
            );
            return;
        }

        let persisted_session = match self.memory.substrate.get_session_async(session_id).await {
            Ok(Some(s)) => s,
            Ok(None) => {
                warn!(
                    agent_id = %agent_id,
                    "replace_tool_result_in_session: session disappeared before reconcile-save"
                );
                return;
            }
            Err(e) => {
                warn!(
                    agent_id = %agent_id,
                    error = %e,
                    "replace_tool_result_in_session: failed to reload latest session"
                );
                return;
            }
        };

        session = persisted_session;
        if reconcile_tool_result(&mut session, tool_use_id, result) {
            if let Err(e) = self.memory.substrate.save_session_async(&session).await {
                warn!(
                    agent_id = %agent_id,
                    error = %e,
                    "replace_tool_result_in_session: failed to save session"
                );
            }
        } else {
            debug!(
                agent_id = %agent_id,
                tool_use_id,
                "replace_tool_result_in_session: terminal result already present or no writable message found"
            );
        }
    }

    /// Notify the running agent loop about an approval resolution via an explicit mid-turn signal.
    fn notify_agent_of_resolution(
        &self,
        agent_id: &AgentId,
        deferred: &librefang_types::tool::DeferredToolExecution,
        decision: &librefang_types::approval::ApprovalDecision,
        result: &librefang_types::tool::ToolResult,
    ) -> bool {
        let senders: Vec<(
            (AgentId, SessionId),
            tokio::sync::mpsc::Sender<AgentLoopSignal>,
        )> = self
            .events
            .injection_senders
            .iter()
            .filter(|e| e.key().0 == *agent_id)
            .map(|e| (*e.key(), e.value().clone()))
            .collect();

        if senders.is_empty() {
            debug!(
                agent_id = %agent_id,
                "Approval resolution: no active agent loop to notify"
            );
            return false;
        }

        let mut delivered = false;
        let mut closed_keys: Vec<(AgentId, SessionId)> = Vec::new();
        for (key, tx) in senders {
            match tx.try_send(AgentLoopSignal::ApprovalResolved {
                tool_use_id: deferred.tool_use_id.clone(),
                tool_name: deferred.tool_name.clone(),
                decision: decision.as_str().to_string(),
                result_content: result.content.clone(),
                result_is_error: result.is_error,
                result_status: result.status,
            }) {
                Ok(()) => {
                    debug!(
                        agent_id = %agent_id,
                        session_id = %key.1,
                        "Approval resolution injected into agent loop"
                    );
                    delivered = true;
                }
                Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                    warn!(
                        agent_id = %agent_id,
                        session_id = %key.1,
                        "Approval resolution injection channel full — falling back to session patch"
                    );
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                    debug!(
                        agent_id = %agent_id,
                        session_id = %key.1,
                        "Approval resolution: agent loop is not running (injection channel closed)"
                    );
                    closed_keys.push(key);
                }
            }
        }
        for key in closed_keys {
            self.events.injection_senders.remove(&key);
        }
        delivered
    }
}

// --- Local-provider probe helpers ---
//
// Shared between the periodic background probe (see `start_background_agents`)
// and the on-demand refresh path in `/api/providers/{id}/test`. Authoritative
// for the `auth_status` of local providers (Ollama / vLLM / LM Studio /
// lemonade) — no other code writes `NotRequired` or `LocalOffline` to them.

// `probe_and_update_local_provider` and `probe_all_local_providers_once` live in `kernel::provider_probe`. The inherent `LibreFangKernel::probe_local_provider` method-style facade stays here.
impl LibreFangKernel {
    /// Method-style facade over [`probe_and_update_local_provider`] so callers
    /// outside this crate (e.g. `librefang-api`) do not need to import the
    /// free function from `librefang_kernel::kernel`. Tracks the
    /// KernelHandle boundary cleanup in #3744.
    pub async fn probe_local_provider(
        self: &Arc<Self>,
        provider_id: &str,
        base_url: &str,
        log_offline_as_warn: bool,
    ) -> librefang_runtime::provider_health::ProbeResult {
        probe_and_update_local_provider(self, provider_id, base_url, log_offline_as_warn).await
    }
}

// --- OFP Wire Protocol integration ---

#[async_trait]
impl librefang_wire::peer::PeerHandle for LibreFangKernel {
    fn local_agents(&self) -> Vec<librefang_wire::message::RemoteAgentInfo> {
        self.agents
            .registry
            .list()
            .iter()
            .map(|entry| librefang_wire::message::RemoteAgentInfo {
                id: entry.id.0.to_string(),
                name: entry.name.clone(),
                description: entry.manifest.description.clone(),
                tags: entry.manifest.tags.clone(),
                tools: entry.manifest.capabilities.tools.clone(),
                state: format!("{:?}", entry.state),
            })
            .collect()
    }

    async fn handle_agent_message(
        &self,
        agent: &str,
        message: &str,
        _sender: Option<&str>,
    ) -> Result<String, String> {
        // Resolve agent by name or ID
        let agent_id = if let Ok(uuid) = uuid::Uuid::parse_str(agent) {
            AgentId(uuid)
        } else {
            // Find by name
            self.agents
                .registry
                .list()
                .iter()
                .find(|e| e.name == agent)
                .map(|e| e.id)
                .ok_or_else(|| format!("Agent not found: {agent}"))?
        };

        match self.send_message(agent_id, message).await {
            Ok(result) => Ok(result.response),
            Err(e) => Err(format!("{e}")),
        }
    }

    fn discover_agents(&self, query: &str) -> Vec<librefang_wire::message::RemoteAgentInfo> {
        let q = query.to_lowercase();
        self.agents
            .registry
            .list()
            .iter()
            .filter(|entry| {
                entry.name.to_lowercase().contains(&q)
                    || entry.manifest.description.to_lowercase().contains(&q)
                    || entry
                        .manifest
                        .tags
                        .iter()
                        .any(|t| t.to_lowercase().contains(&q))
            })
            .map(|entry| librefang_wire::message::RemoteAgentInfo {
                id: entry.id.0.to_string(),
                name: entry.name.clone(),
                description: entry.manifest.description.clone(),
                tags: entry.manifest.tags.clone(),
                tools: entry.manifest.capabilities.tools.clone(),
                state: format!("{:?}", entry.state),
            })
            .collect()
    }

    fn uptime_secs(&self) -> u64 {
        self.booted_at.elapsed().as_secs()
    }
}

#[cfg(test)]
mod tests;
