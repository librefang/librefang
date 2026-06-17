//! Agent CRUD, messaging, sessions, files, and upload handlers.

pub(crate) use super::resolve_lang;
use super::AppState;
use crate::middleware::RequestLanguage;
use crate::stream_dedup::StreamDedup;
use crate::types::*;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use dashmap::DashMap;
use librefang_channels::types::SenderContext;
use librefang_kernel::kernel_handle::prelude::*;
use librefang_kernel::kernel_handle::SessionWriter;
use librefang_types::agent::{AgentId, AgentIdentity, AgentManifest, ResetScope};
use librefang_types::i18n::ErrorTranslator;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

mod attachments;
mod cloning;
mod config;
mod files;
mod identity;
mod lifecycle;
mod messaging;
mod observability;
mod sessions;
mod uploads;

pub use attachments::*;
pub use cloning::*;
pub use config::*;
pub use files::*;
pub use identity::*;
pub use lifecycle::*;
pub use messaging::*;
pub use observability::*;
pub use sessions::*;
pub use uploads::*;

/// Build all routes for the Agent domain.
pub fn router() -> axum::Router<std::sync::Arc<AppState>> {
    axum::Router::new()
        .route(
            "/agents",
            axum::routing::get(list_agents).post(spawn_agent),
        )
        // Canonical agent UUID registry (refs #4614). Routed before
        // /agents/{id} so the literal segment doesn't get parsed as a UUID.
        .route(
            "/agents/identities",
            axum::routing::get(list_agent_identities),
        )
        .route(
            "/agents/identities/{name}/reset",
            axum::routing::post(reset_agent_identity),
        )
        // Bulk agent operations (placed before /agents/{id} to avoid path conflicts)
        .route(
            "/agents/bulk",
            axum::routing::post(bulk_create_agents).delete(bulk_delete_agents),
        )
        .route(
            "/agents/bulk/start",
            axum::routing::post(bulk_start_agents),
        )
        .route(
            "/agents/bulk/stop",
            axum::routing::post(bulk_stop_agents),
        )
        .route(
            "/agents/{id}",
            axum::routing::get(get_agent)
                .delete(kill_agent)
                .patch(patch_agent),
        )
        .route(
            "/agents/{id}/stats",
            axum::routing::get(get_agent_stats),
        )
        .route(
            "/agents/{id}/events",
            axum::routing::get(list_agent_events),
        )
        .route(
            "/agents/{id}/mode",
            axum::routing::put(set_agent_mode),
        )
        .route(
            "/agents/{id}/suspend",
            axum::routing::put(suspend_agent),
        )
        .route(
            "/agents/{id}/resume",
            axum::routing::put(resume_agent),
        )
        .route(
            "/agents/{id}/message",
            axum::routing::post(send_message),
        )
        .route(
            "/agents/{id}/inject",
            axum::routing::post(inject_message),
        )
        .route(
            "/agents/{id}/message/stream",
            axum::routing::post(send_message_stream),
        )
        .route(
            "/agents/{id}/sessions/{session_id}/stream",
            axum::routing::get(attach_session_stream),
        )
        .route(
            "/agents/{id}/session",
            axum::routing::get(get_agent_session),
        )
        .route(
            "/agents/{id}/sessions",
            axum::routing::get(list_agent_sessions).post(create_agent_session),
        )
        .route(
            "/agents/{id}/sessions/{session_id}/switch",
            axum::routing::post(switch_agent_session),
        )
        .route(
            "/agents/{id}/sessions/{session_id}/export",
            axum::routing::get(export_session),
        )
        .route(
            "/agents/{id}/sessions/{session_id}/trajectory",
            axum::routing::get(export_session_trajectory),
        )
        .route(
            "/agents/{id}/sessions/import",
            axum::routing::post(import_session),
        )
        .route(
            "/agents/{id}/session/reset",
            axum::routing::post(reset_session),
        )
        .route(
            "/agents/{id}/session/reboot",
            axum::routing::post(reboot_session),
        )
        .route(
            "/agents/{id}/history",
            axum::routing::delete(clear_agent_history),
        )
        .route(
            "/agents/{id}/session/compact",
            axum::routing::post(compact_session),
        )
        .route("/agents/{id}/stop", axum::routing::post(stop_agent))
        .route(
            "/agents/{id}/runtime",
            axum::routing::get(list_agent_runtime),
        )
        .route(
            "/agents/{id}/sessions/{session_id}/stop",
            axum::routing::post(stop_session),
        )
        .route("/agents/{id}/model", axum::routing::put(set_model))
        .route(
            "/agents/{id}/traces",
            axum::routing::get(get_agent_traces),
        )
        .route(
            "/agents/{id}/tools",
            axum::routing::get(get_agent_tools).put(set_agent_tools),
        )
        .route(
            "/agents/{id}/skills",
            axum::routing::get(get_agent_skills).put(set_agent_skills),
        )
        .route(
            "/agents/{id}/mcp_servers",
            axum::routing::get(get_agent_mcp_servers).put(set_agent_mcp_servers),
        )
        .route(
            "/agents/{id}/channels",
            axum::routing::get(get_agent_channels).put(set_agent_channels),
        )
        .route(
            "/agents/{id}/identity",
            axum::routing::patch(update_agent_identity),
        )
        .route(
            "/agents/{id}/config",
            axum::routing::patch(patch_agent_config),
        )
        .route(
            "/agents/{id}/hand-runtime-config",
            axum::routing::patch(patch_hand_agent_runtime_config)
                .delete(delete_hand_agent_runtime_config),
        )
        .route(
            "/agents/{id}/clone",
            axum::routing::post(clone_agent),
        )
        .route(
            "/agents/{id}/reload",
            axum::routing::post(reload_agent_manifest),
        )
        .route(
            "/agents/{id}/files",
            axum::routing::get(list_agent_files),
        )
        .route(
            "/agents/{id}/files/{filename}",
            axum::routing::get(get_agent_file)
                .put(set_agent_file)
                .delete(delete_agent_file),
        )
        .route(
            "/agents/{id}/metrics",
            axum::routing::get(agent_metrics),
        )
        .route("/agents/{id}/logs", axum::routing::get(agent_logs))
        .route(
            "/agents/{id}/deliveries",
            axum::routing::get(get_agent_deliveries),
        )
        .route("/agents/{id}/ws", axum::routing::get(crate::ws::agent_ws))
        .route(
            "/uploads/{file_id}",
            axum::routing::get(serve_upload),
        )
        .route(
            "/agents/{id}/push",
            axum::routing::post(push_message),
        )
}

/// Shape an `ApiErrorResponse`-compatible JSON envelope into the
/// `(status, bytes)` tuple the idempotency middleware caches.
/// Mirrors `ApiErrorResponse::into_response` so callers see the same
/// shape they did before this handler split.
fn json_error(status: StatusCode, code: &str, error: String) -> (StatusCode, Vec<u8>) {
    let body = serde_json::json!({
        "error": error,
        "code": code,
        "type": code,
    });
    (status, serde_json::to_vec(&body).unwrap_or_default())
}

// ---------------------------------------------------------------------------
// Bulk agent operations
// ---------------------------------------------------------------------------
/// Maximum number of agents allowed in a single bulk request.
const BULK_LIMIT: usize = 50;

/// Default page size for `GET /api/agents` when the caller does not
/// supply `limit`. Picked to match `MAX_AGENT_LIST_LIMIT` so the
/// historical "single request returns all agents on a small
/// deployment" behaviour survives, while large deployments fall
/// inside the cap. Callers that need explicit small pages still get
/// them via `?limit=`.
const DEFAULT_AGENT_LIST_LIMIT: usize = 500;

/// Hard cap on `limit`. Existing behaviour was
/// `limit.map(|l| l.min(500))`, so 500 is the historical ceiling.
/// (audit: agent-list-limit-none-unbounded).
const MAX_AGENT_LIST_LIMIT: usize = 500;

/// Enrich an `AgentEntry` into a JSON value with catalog data.
pub(crate) fn enrich_agent_json(
    e: &librefang_types::agent::AgentEntry,
    dm: &librefang_types::config::DefaultModelConfig,
    catalog: Option<&librefang_kernel::model_catalog::ModelCatalog>,
    bulk_stats: Option<&std::collections::HashMap<String, (u64, f64)>>,
) -> serde_json::Value {
    let provider = if e.manifest.model.provider.is_empty() || e.manifest.model.provider == "default"
    {
        dm.provider.as_str()
    } else {
        e.manifest.model.provider.as_str()
    };
    let model = if e.manifest.model.model.is_empty() || e.manifest.model.model == "default" {
        dm.model.as_str()
    } else {
        e.manifest.model.model.as_str()
    };

    let (tier, auth_status, supports_thinking) = catalog
        .map(|cat| {
            let model_entry = cat.find_model(model);
            let tier = model_entry
                .map(|m| format!("{:?}", m.tier).to_lowercase())
                .unwrap_or_else(|| "unknown".to_string());
            // Refs #4745: surface effective `supports_thinking` (catalog ∘ user
            // override) so the agents page reflects the user's per-model
            // capability overrides.
            let thinking = model_entry
                .map(|m| cat.effective_capabilities(m).supports_thinking)
                .unwrap_or(false);
            let auth = cat
                .get_provider(provider)
                .map(|p| p.auth_status.to_string())
                .unwrap_or_else(|| "unknown".to_string());
            (tier, auth, thinking)
        })
        .unwrap_or(("unknown".to_string(), "unknown".to_string(), false));

    let ready =
        matches!(e.state, librefang_types::agent::AgentState::Running) && auth_status != "missing";

    let schedule = format_schedule_mode(&e.manifest.schedule);

    let (sessions_24h, cost_24h) = bulk_stats
        .and_then(|m| m.get(&e.id.to_string()).copied())
        .unwrap_or((0, 0.0));

    serde_json::json!({
        "id": e.id.to_string(),
        "name": e.name,
        "is_hand": e.is_hand,
        "state": format!("{:?}", e.state),
        "mode": e.mode,
        "created_at": e.created_at.to_rfc3339(),
        "last_active": e.last_active.to_rfc3339(),
        "model_provider": provider,
        "model_name": model,
        "model_tier": tier,
        "auth_status": auth_status,
        "supports_thinking": supports_thinking,
        "ready": ready,
        "profile": e.manifest.profile,
        "schedule": schedule,
        "sessions_24h": sessions_24h,
        "cost_24h": cost_24h,
        "identity": {
            "emoji": e.identity.emoji,
            "avatar_url": e.identity.avatar_url,
            "color": e.identity.color,
        },
        "web_search_augmentation": e.manifest.web_search_augmentation,
        "parent_agent_id": e.parent.as_ref().map(|p| p.to_string()),
        "children": e.children.iter().map(|c| c.to_string()).collect::<Vec<_>>(),
        "session_id": e.session_id.0.to_string(),
        "tags": e.tags,
        "onboarding_completed": e.onboarding_completed,
        "onboarding_completed_at": e.onboarding_completed_at.as_ref().map(|t| t.to_rfc3339()),
        "force_session_wipe": e.force_session_wipe,
        "resume_pending": e.resume_pending,
        "reset_reason": e.reset_reason,
        "has_processed_message": e.has_processed_message,
    })
}

pub(crate) fn effective_default_model(
    base: &librefang_types::config::DefaultModelConfig,
    override_dm: Option<&librefang_types::config::DefaultModelConfig>,
) -> librefang_types::config::DefaultModelConfig {
    override_dm.cloned().unwrap_or_else(|| base.clone())
}

/// Resolve the session id the attachment blocks should be written to,
/// mirroring the resolver used by `send_message_*` in
/// `kernel::messaging`. Pure function (no I/O, no kernel reads) so it can
/// be unit-tested directly and so call sites can assert which session id
/// the attachment landed in.
///
/// `fallback_session_id` is the agent's persistent registry session id
/// (`entry.session_id`), used only when neither override nor channel
/// context is available. Mirrors the
/// `SessionMode::Persistent => entry.session_id` branch of
/// `kernel/messaging.rs`. The `SessionMode::New => SessionId::new()`
/// branch is intentionally NOT mirrored here: attachment injection
/// always precedes the kernel call that actually generates the fresh id,
/// so there is no shared id to write into. In practice the
/// only code paths that hit the resolver fallback today
/// (skill/hand send-message + WebUI WS with `use_canonical_session`)
/// both use `SessionMode::Persistent` agents; if a `New`-mode agent
/// ever lands here, the attachment goes to `entry.session_id`, which is
/// the *same* session a subsequent `Persistent`-fallback turn would
/// write to, and the worst case is a no-channel-context REST caller
/// seeing the image one turn later than expected — never a cross-chat
/// leak. The text dispatch itself is unchanged.
pub(crate) fn resolve_attachment_session_id(
    agent_id: AgentId,
    sender_context: Option<&librefang_channels::types::SenderContext>,
    session_id_override: Option<librefang_types::agent::SessionId>,
    fallback_session_id: librefang_types::agent::SessionId,
) -> librefang_types::agent::SessionId {
    if let Some(sid) = session_id_override {
        return sid;
    }
    if let Some(ctx) = sender_context {
        if !ctx.channel.is_empty() && !ctx.use_canonical_session {
            return librefang_types::agent::SessionId::for_sender_scope(
                agent_id,
                &ctx.channel,
                ctx.chat_id.as_deref(),
            );
        }
    }
    fallback_session_id
}

/// RAII guard that aborts a spawned task when dropped. Used so client
/// disconnect cancels the kernel call and releases per-agent locks +
/// LLM bandwidth instead of letting the round-trip finish unobserved
/// (#3464).
///
/// `disarm()` releases the abort handle without aborting — call it when
/// the spawned task has already produced its observable output and the
/// remaining work (metering settle, canonical session append, audit log
/// write) MUST run to completion. The streaming path uses this once
/// `ContentComplete` has reached the client, so the natural end of the
/// SSE stream (which drops the unfold state and hence the guard) does
/// not race-cancel post-stream cleanup.
struct AbortOnDrop(Option<tokio::task::AbortHandle>);

impl AbortOnDrop {
    fn new(handle: tokio::task::AbortHandle) -> Self {
        Self(Some(handle))
    }

    /// Release the abort permission without aborting.
    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            if !handle.is_finished() {
                handle.abort();
            }
        }
    }
}

/// Run `fut` in a spawned task; abort it if the awaiting future is dropped.
async fn run_cancel_on_disconnect<F, T>(fut: F) -> Result<T, tokio::task::JoinError>
where
    F: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let handle = tokio::spawn(fut);
    let _guard = AbortOnDrop::new(handle.abort_handle());
    handle.await
}

fn request_sender_context(req: &MessageRequest) -> Option<SenderContext> {
    let sender_id = req.sender_id.as_ref()?;
    // Audit: cron-channel-name-not-reserved. An HTTP caller supplying
    // `channel_type = "cron"` (or case variant) used to derive the
    // SAME SessionId as the kernel's internal cron-fire path and
    // interleave history. Sanitize at the construction site so the
    // value reaching `send_message_full` cannot collide with a
    // reserved system channel name.
    let raw_channel = req
        .channel_type
        .clone()
        .unwrap_or_else(|| "api".to_string());
    Some(SenderContext {
        channel: librefang_channels::types::sanitize_channel_name(&raw_channel),
        user_id: sender_id.clone(),
        display_name: req.sender_name.clone().unwrap_or_else(|| sender_id.clone()),
        is_group: req.is_group,
        was_mentioned: req.was_mentioned,
        thread_id: None,
        account_id: None,
        // Phase 2 §C — forward the optional group participant roster from the
        // gateway POST body so the addressee guard can fire downstream. Empty
        // when the caller (Telegram, direct API) doesn't populate it; the
        // guard then becomes a no-op and cannot produce false positives.
        group_participants: req.group_participants.clone().unwrap_or_default(),
        ..Default::default()
    })
}

/// Build the (sender_context, incognito, session_id_override) triple that the
/// streaming handler hands to `send_message_streaming_with_incognito`.
///
/// Factored out of `send_message_stream` so the regression test
/// `test_streaming_handler_threads_sender_context_to_kernel_args` exercises
/// the exact code path the handler uses. A future mutation that silently
/// drops one of the three fields on its way to the kernel call breaks the
/// test, not just a sibling unit that happens to call `request_sender_context`
/// the same way.
fn build_streaming_kernel_args(
    req: &MessageRequest,
    session_id_override: Option<librefang_types::agent::SessionId>,
) -> (
    Option<SenderContext>,
    bool,
    Option<librefang_types::agent::SessionId>,
) {
    (
        request_sender_context(req),
        req.incognito,
        session_id_override,
    )
}

fn patch_agent_mcp_servers(body: &serde_json::Value) -> Result<Option<Vec<String>>, &'static str> {
    let raw = body.get("mcp_servers").or_else(|| {
        body.get("capabilities")
            .and_then(|caps| caps.get("mcp_servers"))
    });

    let Some(raw) = raw else {
        return Ok(None);
    };

    let items = raw
        .as_array()
        .ok_or("mcp_servers must be an array of strings")?;

    // `BULK_LIMIT` (50) bounds the per-agent MCP server list at the same
    // cap as the agents bulk endpoints. Sweep finding from the
    // `Vec::with_capacity(arr.len())` DoS audit
    // (`docs/issues/bulk-with-capacity-no-validate.md`): without this,
    // an `{"mcp_servers": ["", "", ...]}` payload within the 8 MiB body
    // cap would pre-allocate millions of entries.
    if items.len() > BULK_LIMIT {
        return Err("mcp_servers exceeds maximum allowed entries");
    }
    let mut servers = Vec::with_capacity(items.len());
    for item in items {
        let name = item
            .as_str()
            .ok_or("mcp_servers must be an array of strings")?;
        servers.push(name.to_string());
    }

    Ok(Some(servers))
}

/// Translate a kernel error into the right HTTP status code for the
/// generic CRUD-style /api/agents/* error paths (audit:
/// agent-not-found-returns-500). The handler then renders the error
/// message body with the existing fluent key plus this status; that
/// keeps the per-route ergonomics (translated body, hot-reload-safe)
/// while pinning the structural mapping in ONE place so a future
/// `LibreFangError` variant is gated by adding an arm here, not by
/// hunting every site.
///
/// Variants covered today:
/// - `AgentNotFound`      → 404 (was 500 across 5 sites)
/// - `AgentAlreadyExists` → 409 (was 500 in `clone_agent`)
/// - everything else      → 500 (preserves the pre-fix default)
///
/// The `send_message` handler at `agents.rs:1936-1951` predates this
/// helper and adds its own arms for `QuotaExceeded → 429` and a
/// session-mismatch substring → 400. Those are message-specific
/// statuses worth keeping inline at that site; this helper covers
/// the lowest-common-denominator CRUD shape.
fn kernel_err_to_status(e: &crate::error::KernelError) -> StatusCode {
    use crate::error::KernelError;
    use librefang_types::error::LibreFangError;
    match e {
        KernelError::LibreFang(LibreFangError::AgentNotFound(_)) => StatusCode::NOT_FOUND,
        KernelError::LibreFang(LibreFangError::AgentAlreadyExists(_)) => StatusCode::CONFLICT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// Build the localized error-body string for a kernel error that has
/// already been mapped to `status`.
///
/// 4xx statuses echo the kernel message back (caller-useful detail
/// such as "agent not found" / "already exists"). A 500 status scrubs
/// the raw error to the generic localized "Internal server error"
/// *after* logging the full chain at `error!` (audit:
/// rusqlite-errors-leak). `librefang-memory` wraps every rusqlite
/// error in `LibreFangError::Internal(e.to_string())`, so echoing the
/// raw text into a 500 body leaks SQL schema, column / constraint
/// names, and lock state to any caller able to trigger an internal
/// error. Operators keep forensics via the `error!` log.
fn kernel_err_body(
    status: StatusCode,
    e: &crate::error::KernelError,
    t: &ErrorTranslator,
) -> String {
    if status == StatusCode::INTERNAL_SERVER_ERROR {
        tracing::error!(error = %e, "kernel error scrubbed before response");
        t.t("api-error-internal")
    } else {
        t.t_args("api-error-generic", &[("error", &e.to_string())])
    }
}

/// Scrub an unconditional 500-path error: log the full chain at
/// `error!` for operators, return the generic localized "Internal
/// server error" so the response body leaks no internal detail
/// (audit: rusqlite-errors-leak). Use at sites whose status is always
/// `INTERNAL_SERVER_ERROR`; for sites whose status varies with the
/// error variant, prefer [`kernel_err_body`].
fn scrub_500(e: &impl std::fmt::Display, t: &ErrorTranslator) -> String {
    tracing::error!(error = %e, "internal error scrubbed before response");
    t.t("api-error-internal")
}

/// Translate a kernel error from `update_hand_agent_runtime_override` or
/// `clear_hand_agent_runtime_override` into a `(StatusCode, message)` pair.
///
/// - [`LibreFangError::AgentNotFound`] → 404
/// - [`LibreFangError::Internal`] whose message starts with `"Hand role not
///   found"` → 409 Conflict (the hand instance exists but no role maps to
///   the requested agent id — kernel has no dedicated variant, so we match
///   on the single well-known prefix emitted by the kernel)
/// - everything else → 500
fn map_hand_runtime_override_err(err: &crate::error::KernelError) -> (StatusCode, String) {
    use crate::error::KernelError;
    use librefang_types::error::LibreFangError;
    match err {
        KernelError::LibreFang(LibreFangError::AgentNotFound(_)) => {
            (StatusCode::NOT_FOUND, err.to_string())
        }
        KernelError::LibreFang(LibreFangError::Internal(msg))
            if msg.starts_with("Hand role not found") =>
        {
            (StatusCode::CONFLICT, err.to_string())
        }
        // Scrub the catch-all 500 (audit: rusqlite-errors-leak): the
        // full chain reaches `error!` for operators, the client sees a
        // generic body. The 404 / 409 arms above intentionally echo
        // the kernel message (caller-useful, no internal detail).
        _ => {
            tracing::error!(error = %err, "hand runtime override error scrubbed before response");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error".to_string(),
            )
        }
    }
}

fn apply_clone_inclusion_flags(
    manifest: &mut librefang_types::agent::AgentManifest,
    req: &CloneAgentRequest,
) {
    if !req.include_skills {
        manifest.skills.clear();
        manifest.skills_disabled = true;
    }
    if !req.include_tools {
        manifest.tools.clear();
        manifest.tool_allowlist.clear();
        manifest.tool_blocklist.clear();
        manifest.tools_disabled = true;
    }
}

fn skill_assignment_mode(manifest: &librefang_types::agent::AgentManifest) -> &'static str {
    if manifest.skills_disabled {
        "none"
    } else if manifest.skills.is_empty() {
        "all"
    } else {
        "allowlist"
    }
}

/// Classify an agent's MCP server allowlist for display (#5855).
///
/// Mirrors the kernel semantics in `available_tools`: an empty list grants
/// **no** MCP servers, `["*"]` grants all connected servers, anything else is
/// a literal allowlist.
fn mcp_servers_mode(mcp_servers: &[String]) -> &'static str {
    if mcp_servers.is_empty() {
        "none"
    } else if mcp_servers.iter().any(|s| s == "*") {
        "all"
    } else {
        "allowlist"
    }
}

/// Render a ScheduleMode as the short string the dashboard's Schedule
/// tab displays (and what `enrich_agent_json` already exposes on the
/// agent list). Both endpoints go through this helper so they can't
/// drift apart.
fn format_schedule_mode(schedule: &librefang_types::agent::ScheduleMode) -> String {
    use librefang_types::agent::ScheduleMode;
    match schedule {
        ScheduleMode::Reactive => "manual".to_string(),
        ScheduleMode::Periodic { cron } => cron.clone(),
        ScheduleMode::Proactive { .. } => "proactive".to_string(),
        ScheduleMode::Continuous {
            check_interval_secs,
        } => format!("continuous · {check_interval_secs}s"),
    }
}

// ---------------------------------------------------------------------------
// Workspace File Editor endpoints
// ---------------------------------------------------------------------------
/// Whitelisted workspace identity files that can be read/written via API.
const KNOWN_IDENTITY_FILES: &[&str] = &[
    "SOUL.md",
    "IDENTITY.md",
    "USER.md",
    "TOOLS.md",
    "MEMORY.md",
    "AGENTS.md",
    "BOOTSTRAP.md",
    "HEARTBEAT.md",
];

/// Non-media MIME types also accepted on `/api/agents/{id}/upload` — text
/// files and PDFs that the agent loop consumes directly. Media types are
/// sourced from `librefang_types::media::{ALLOWED_IMAGE_TYPES,
/// ALLOWED_AUDIO_TYPES}` so the upload endpoint, the channel bridge, and
/// `MediaAttachment::validate()` can never drift.
///
/// Browsers send a wide variety of `Content-Type` values for the same file
/// kind (`.json` → `application/json`; `.yaml` → `application/x-yaml` /
/// `application/yaml`; `.ipynb` → `application/x-ipynb+json` / sometimes
/// `application/json`), so this list is intentionally exhaustive on the
/// safe subset.
const EXTRA_ALLOWED_UPLOAD_TYPES: &[&str] = &[
    "application/pdf",
    // Plain text + tables
    "text/plain",
    "text/markdown",
    "text/csv",
    "text/tab-separated-values",
    // Structured data
    "application/json",
    "application/x-ipynb+json",
    "application/xml",
    "application/yaml",
    "application/x-yaml",
    "application/toml",
    "application/x-toml",
    "application/sql",
    "application/graphql",
    // Code (often delivered with these MIMEs)
    "application/javascript",
    "application/x-javascript",
    "application/typescript",
];

/// MIME allowlist for `/api/agents/{id}/upload`.
///
/// Historically this was a permissive prefix list (`image/`, `text/`,
/// `application/pdf`, `audio/`) which accepted dangerous subtypes like
/// `image/svg+xml` (scriptable → XSS / SSRF), `text/html` (stored XSS
/// via downstream renderers), and `text/xml` (XXE / SSRF). That
/// contradicted the SECURITY.md promise of *"Media type whitelist
/// (png/jpeg/gif/webp)"*.
///
/// The check now combines:
///   1. Exact match against the canonical media constants
///      (`ALLOWED_IMAGE_TYPES`, `ALLOWED_AUDIO_TYPES`).
///   2. Exact match against `EXTRA_ALLOWED_UPLOAD_TYPES` (PDF + curated
///      text/data/code MIMEs).
///   3. **Any other `text/*` subtype** EXCEPT `text/html` and `text/xml`.
///      Browsers tag many code files (`.rs`, `.py`, `.go`, `.sh`, …) as
///      `text/x-rust`, `text/x-python`, `text/x-shellscript` etc. — those
///      are safe to inline because the agent loop reads them as plain
///      UTF-8 and never executes/renders them. HTML/XML stay blocked
///      because downstream consumers (markdown renderer, XML parsers)
///      could be tricked into XSS / XXE.
fn is_allowed_content_type(ct: &str) -> bool {
    use librefang_types::media::{mime_base, ALLOWED_AUDIO_TYPES, ALLOWED_IMAGE_TYPES};
    let base = mime_base(ct);
    if ALLOWED_IMAGE_TYPES.contains(&base.as_str())
        || ALLOWED_AUDIO_TYPES.contains(&base.as_str())
        || EXTRA_ALLOWED_UPLOAD_TYPES.contains(&base.as_str())
    {
        return true;
    }
    if let Some(subtype) = base.strip_prefix("text/") {
        // Anything text-like is fine to ingest as a plain-text attachment,
        // except formats that get rendered/parsed by downstream tooling
        // and could carry an exploit payload.
        return !matches!(subtype, "html" | "xml");
    }
    false
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use librefang_channels::types::ParticipantRef;

    /// Mirror of the pagination expression in `list_agents`. Pulled
    /// out so the unit test below can drive it through opaque inputs
    /// — clippy otherwise const-folds away the literal `None` /
    /// `Some(usize::MAX)` we want to feed the helper.
    fn effective_agent_list_limit(caller: Option<usize>) -> usize {
        caller
            .unwrap_or(DEFAULT_AGENT_LIST_LIMIT)
            .min(MAX_AGENT_LIST_LIMIT)
    }

    #[test]
    fn agent_list_limit_clamps_at_max_when_caller_omits_limit() {
        // Audit: agent-list-limit-none-unbounded. The handler must
        // resolve a missing `limit` to a finite cap (the historical
        // `None → unpaginated` behaviour was a DoS lever on
        // multi-thousand-agent deployments), and an oversized
        // explicit `limit` must be clamped to the same ceiling.
        assert_eq!(
            effective_agent_list_limit(None),
            DEFAULT_AGENT_LIST_LIMIT,
            "missing limit must fall back to DEFAULT_AGENT_LIST_LIMIT, not run uncapped"
        );
        assert_eq!(
            effective_agent_list_limit(Some(usize::MAX)),
            MAX_AGENT_LIST_LIMIT,
            "oversized limit must clamp at MAX_AGENT_LIST_LIMIT"
        );
        // Const sanity in a runtime form so clippy doesn't fold it
        // out: zero cap would silently empty the list.
        assert!(effective_agent_list_limit(Some(10)) >= 10.min(MAX_AGENT_LIST_LIMIT));
    }

    /// The pre-fix prefix-match (`"image/"`) let SVG, BMP, TIFF, HEIC and
    /// friends through. Post-fix the allowlist is exact-match over the
    /// same four formats SECURITY.md advertises.
    #[test]
    fn test_upload_mime_allowlist_rejects_previously_accepted_types() {
        // Previously accepted via prefix match, now explicitly rejected.
        for bad in [
            "image/svg+xml",
            "image/svg+xml; charset=utf-8",
            "image/bmp",
            "image/tiff",
            "image/x-icon",
            "image/heic",
            "image/heif",
            "image/avif",
            "image/vnd.microsoft.icon",
            "text/html", // text/ prefix used to let this through
            "text/xml",
            "audio/vnd.rn-realaudio",
            "application/octet-stream",
        ] {
            assert!(
                !is_allowed_content_type(bad),
                "{bad} must be rejected by the upload allowlist"
            );
        }
    }

    #[test]
    fn test_upload_mime_allowlist_accepts_expected_formats() {
        for good in [
            "image/png",
            "image/jpeg",
            "image/gif",
            "image/webp",
            "image/PNG",                 // case-insensitive
            "image/png; charset=binary", // MIME params stripped
            "audio/mpeg",
            "audio/wav",
            "audio/ogg",
            "audio/flac",
            "text/plain",
            "text/markdown",
            "text/csv",
            "application/pdf",
        ] {
            assert!(
                is_allowed_content_type(good),
                "{good} must be accepted by the upload allowlist"
            );
        }
    }

    #[test]
    fn test_clone_request_defaults() {
        let json = r#"{"new_name": "clone-1"}"#;
        let req: CloneAgentRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.new_name, "clone-1");
        assert!(req.include_skills);
        assert!(req.include_tools);
    }

    #[test]
    fn test_map_hand_runtime_override_err_maps_not_found_and_conflict() {
        use crate::error::KernelError;
        use librefang_types::error::LibreFangError;

        let not_found =
            KernelError::LibreFang(LibreFangError::AgentNotFound("missing-agent".to_string()));
        let (status, _) = map_hand_runtime_override_err(&not_found);
        assert_eq!(status, StatusCode::NOT_FOUND);

        let conflict = KernelError::LibreFang(LibreFangError::Internal(
            "Hand role not found for agent 123".to_string(),
        ));
        let (status, _) = map_hand_runtime_override_err(&conflict);
        assert_eq!(status, StatusCode::CONFLICT);
    }

    #[test]
    fn test_clone_request_explicit_false() {
        let json = r#"{"new_name": "clone-2", "include_skills": false, "include_tools": false}"#;
        let req: CloneAgentRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.new_name, "clone-2");
        assert!(!req.include_skills);
        assert!(!req.include_tools);
    }

    /// Issue #3361: UploadMeta carries the uploader's UserId so `serve_upload`
    /// can reject cross-user UUID guessing. Pre-fix the struct had no owner
    /// field at all and any caller knowing the UUID could fetch the file.
    #[test]
    fn issue_3361_upload_meta_carries_owner() {
        use librefang_types::agent::UserId;
        let owner = UserId::from_name("alice");
        let meta = UploadMeta {
            filename: "doc.pdf".to_string(),
            content_type: "application/pdf".to_string(),
            uploaded_by: Some(owner),
        };
        assert_eq!(meta.uploaded_by, Some(owner));

        // Daemon-generated content has no owner — None means "any
        // authenticated caller may read" (e.g. image_generate output).
        let generated = UploadMeta {
            filename: "image.png".to_string(),
            content_type: "image/png".to_string(),
            uploaded_by: None,
        };
        assert!(generated.uploaded_by.is_none());
    }

    #[test]
    fn test_clone_request_partial_flags() {
        let json = r#"{"new_name": "clone-3", "include_skills": false}"#;
        let req: CloneAgentRequest = serde_json::from_str(json).unwrap();
        assert!(!req.include_skills);
        assert!(req.include_tools);

        let json = r#"{"new_name": "clone-4", "include_tools": false}"#;
        let req: CloneAgentRequest = serde_json::from_str(json).unwrap();
        assert!(req.include_skills);
        assert!(!req.include_tools);
    }

    #[test]
    fn test_clone_manifest_strips_skills_when_excluded() {
        let manifest = librefang_types::agent::AgentManifest {
            skills: vec!["skill-a".to_string(), "skill-b".to_string()],
            tools: {
                let mut m = std::collections::HashMap::new();
                m.insert(
                    "tool-a".to_string(),
                    librefang_types::agent::ToolConfig {
                        params: std::collections::HashMap::new(),
                    },
                );
                m
            },
            ..Default::default()
        };

        let mut cloned = manifest.clone();
        apply_clone_inclusion_flags(
            &mut cloned,
            &CloneAgentRequest {
                new_name: "clone-1".to_string(),
                include_skills: false,
                include_tools: true,
            },
        );
        assert!(cloned.skills.is_empty());
        assert!(cloned.skills_disabled);
        assert_eq!(skill_assignment_mode(&cloned), "none");
        assert!(!cloned.tools.is_empty());
    }

    #[test]
    fn test_mcp_servers_mode_classification() {
        // #5855: empty allowlist is "none" (no servers), not "all".
        assert_eq!(mcp_servers_mode(&[]), "none");
        assert_eq!(mcp_servers_mode(&["*".to_string()]), "all");
        assert_eq!(
            mcp_servers_mode(&["server-a".to_string(), "*".to_string()]),
            "all",
            "a wildcard anywhere in the list means all servers"
        );
        assert_eq!(
            mcp_servers_mode(&["server-a".to_string(), "server-b".to_string()]),
            "allowlist"
        );
    }

    #[test]
    fn test_clone_manifest_disables_tools_when_excluded() {
        let manifest = librefang_types::agent::AgentManifest {
            tools: {
                let mut m = std::collections::HashMap::new();
                m.insert(
                    "tool-a".to_string(),
                    librefang_types::agent::ToolConfig {
                        params: std::collections::HashMap::new(),
                    },
                );
                m
            },
            tool_allowlist: vec!["allowed-tool".to_string()],
            tool_blocklist: vec!["blocked-tool".to_string()],
            ..Default::default()
        };

        let mut cloned = manifest.clone();
        apply_clone_inclusion_flags(
            &mut cloned,
            &CloneAgentRequest {
                new_name: "clone-2".to_string(),
                include_skills: true,
                include_tools: false,
            },
        );
        assert!(cloned.tools.is_empty());
        assert!(cloned.tool_allowlist.is_empty());
        assert!(cloned.tool_blocklist.is_empty());
        assert!(cloned.tools_disabled);
    }

    #[test]
    fn test_request_sender_context_none_without_sender_id() {
        let req = MessageRequest {
            message: "hello".to_string(),
            attachments: Vec::new(),
            sender_id: None,
            sender_name: None,
            channel_type: Some("whatsapp".to_string()),
            is_group: false,
            was_mentioned: false,
            ephemeral: false,
            thinking: None,
            show_thinking: None,
            group_participants: None,
            session_id: None,
            incognito: false,
        };
        assert!(request_sender_context(&req).is_none());
    }

    #[test]
    fn test_request_sender_context_builds_defaults() {
        let req = MessageRequest {
            message: "hello".to_string(),
            attachments: Vec::new(),
            sender_id: Some("u-123".to_string()),
            sender_name: None,
            channel_type: None,
            is_group: false,
            was_mentioned: false,
            ephemeral: false,
            thinking: None,
            show_thinking: None,
            group_participants: None,
            session_id: None,
            incognito: false,
        };
        let sender = request_sender_context(&req).expect("sender context");
        assert_eq!(sender.user_id, "u-123");
        assert_eq!(sender.display_name, "u-123");
        assert_eq!(sender.channel, "api");
        assert!(sender.group_participants.is_empty());
    }

    #[test]
    fn test_request_sender_context_propagates_group_and_mention() {
        let req = MessageRequest {
            message: "hello".to_string(),
            attachments: Vec::new(),
            sender_id: Some("u-456".to_string()),
            sender_name: Some("Alice".to_string()),
            channel_type: Some("whatsapp".to_string()),
            is_group: true,
            was_mentioned: true,
            ephemeral: false,
            thinking: None,
            show_thinking: None,
            group_participants: None,
            session_id: None,
            incognito: false,
        };
        let sender = request_sender_context(&req).expect("sender context");
        assert!(sender.is_group);
        assert!(sender.was_mentioned);
    }

    #[test]
    fn test_request_sender_context_threads_group_participants() {
        let roster = vec![
            ParticipantRef {
                jid: "111@s.whatsapp.net".to_string(),
                display_name: "Alice".to_string(),
            },
            ParticipantRef {
                jid: "222@s.whatsapp.net".to_string(),
                display_name: "Bob".to_string(),
            },
        ];
        let req = MessageRequest {
            message: "Bob, ciao".to_string(),
            attachments: Vec::new(),
            sender_id: Some("111@s.whatsapp.net".to_string()),
            sender_name: Some("Alice".to_string()),
            channel_type: Some("whatsapp".to_string()),
            is_group: true,
            was_mentioned: false,
            ephemeral: false,
            thinking: None,
            show_thinking: None,
            group_participants: Some(roster.clone()),
            session_id: None,
            incognito: false,
        };
        let sender = request_sender_context(&req).expect("sender context");
        assert_eq!(sender.group_participants, roster);
    }

    #[test]
    fn test_message_request_group_participants_default_when_missing() {
        // Backward compat: callers (Telegram, direct API) that omit
        // `group_participants` must still deserialize cleanly.
        let json = serde_json::json!({
            "message": "hi",
            "sender_id": "u-1",
            "channel_type": "telegram",
            "is_group": false,
        });
        let req: MessageRequest =
            serde_json::from_value(json).expect("deserialize without group_participants");
        assert!(req.group_participants.is_none());
        let sender = request_sender_context(&req).expect("sender context");
        assert!(sender.group_participants.is_empty());
    }

    #[test]
    fn test_message_request_group_participants_deserializes_from_json() {
        let json = serde_json::json!({
            "message": "hey Bob",
            "sender_id": "111@s.whatsapp.net",
            "sender_name": "Alice",
            "channel_type": "whatsapp:group-jid@g.us",
            "is_group": true,
            "group_participants": [
                {"jid": "111@s.whatsapp.net", "display_name": "Alice"},
                {"jid": "222@s.whatsapp.net", "display_name": "Bob"}
            ]
        });
        let req: MessageRequest =
            serde_json::from_value(json).expect("deserialize with group_participants");
        let sender = request_sender_context(&req).expect("sender context");
        assert_eq!(sender.group_participants.len(), 2);
        assert_eq!(sender.group_participants[1].display_name, "Bob");
    }

    /// Regression for the 2026-05-19 cross-chat leak: the streaming
    /// `/message/stream` handler historically called the kernel without
    /// a `SenderContext`, so the resolver fell through to the per-agent
    /// `Persistent` session pointer and collapsed every chat (DM, group,
    /// stranger) onto one session. After the fix the handler must build
    /// the same `SenderContext` the non-streaming sibling builds, and
    /// the resolver derives a deterministic `SessionId::for_sender_scope`
    /// per `(agent, channel:chat_id)` pair.
    ///
    /// This test exercises the boundary in two parts: (1) the request
    /// shapes the gateway actually posts produce non-`None` sender
    /// contexts whose `channel` field uniquely identifies the chat; (2)
    /// passing those contexts through `SessionId::for_sender_scope` for
    /// the same agent yields *different* session ids — which is the
    /// invariant the streaming endpoint must preserve and the live
    /// incident violated.
    #[test]
    fn test_streaming_handler_builds_sender_context_for_distinct_chats() {
        use librefang_types::agent::SessionId;

        // Two real channel_type shapes captured from the live incident
        // log on 2026-05-19: the WhatsApp gateway posts the JID baked
        // into the `channel_type` field (chat_id remains None on the
        // SenderContext). The resolver's `for_sender_scope` therefore
        // distinguishes scopes via `channel` alone.
        let group_req = MessageRequest {
            message: "ciao tutti".to_string(),
            attachments: Vec::new(),
            sender_id: Some("393285497365@s.whatsapp.net".to_string()),
            sender_name: Some("Cate".to_string()),
            channel_type: Some("whatsapp:393285497365-1412881543@g.us".to_string()),
            is_group: true,
            was_mentioned: false,
            ephemeral: false,
            thinking: None,
            show_thinking: None,
            group_participants: None,
            session_id: None,
            incognito: false,
        };
        let dm_req = MessageRequest {
            message: "ora riproponimi i vocali per erika".to_string(),
            attachments: Vec::new(),
            sender_id: Some("+393760105565".to_string()),
            sender_name: None,
            channel_type: Some("whatsapp:191856289808491@lid".to_string()),
            is_group: false,
            was_mentioned: false,
            ephemeral: false,
            thinking: None,
            show_thinking: None,
            group_participants: None,
            session_id: None,
            incognito: false,
        };

        let group_ctx =
            request_sender_context(&group_req).expect("group request must produce sender context");
        let dm_ctx =
            request_sender_context(&dm_req).expect("dm request must produce sender context");

        // Sanity: the gateway-side channel values match the live
        // incident exactly (no normalization between transport and
        // kernel).
        assert_eq!(group_ctx.channel, "whatsapp:393285497365-1412881543@g.us");
        assert_eq!(dm_ctx.channel, "whatsapp:191856289808491@lid");

        // The resolver invariant: same agent, two different chats →
        // two different deterministic session ids. Before the fix, the
        // streaming handler passed `None` for sender_context and BOTH
        // requests landed on the agent's single `entry.session_id`.
        let agent = AgentId::new();
        let group_sid =
            SessionId::for_sender_scope(agent, &group_ctx.channel, group_ctx.chat_id.as_deref());
        let dm_sid = SessionId::for_sender_scope(agent, &dm_ctx.channel, dm_ctx.chat_id.as_deref());
        assert_ne!(
            group_sid, dm_sid,
            "group and DM must resolve to distinct session ids — same id means cross-chat history bleed"
        );

        // And the derivation is stable: repeating the call must return
        // the same id (otherwise the per-chat session would churn
        // turn-by-turn).
        assert_eq!(
            group_sid,
            SessionId::for_sender_scope(agent, &group_ctx.channel, group_ctx.chat_id.as_deref())
        );
    }

    /// Regression test promoted per houko review on PR #5288: the original
    /// precondition test only validated `request_sender_context` output and
    /// `SessionId` derivation independently. A mutation that drops
    /// sender_context, incognito, or session_id_override on the way to the
    /// kernel call would silently bypass it. This test exercises
    /// `build_streaming_kernel_args` — the exact triple the streaming
    /// handler hands to `send_message_streaming_with_incognito` — so any
    /// such mutation fails here.
    ///
    /// A full SSE-driven e2e test (TestServer → SSE → kernel mock capturing
    /// arg values) was considered but deemed too heavy: it would require a
    /// stubbed `LibreFangKernelApi` impl plus async stream plumbing for a
    /// linear data-flow assertion. The helper-extraction approach gives
    /// equivalent mutation-detection coverage at unit-test cost.
    #[test]
    fn test_streaming_handler_threads_sender_context_to_kernel_args() {
        use librefang_types::agent::SessionId;

        let req = MessageRequest {
            message: "test".to_string(),
            attachments: Vec::new(),
            sender_id: Some("393285497365@s.whatsapp.net".to_string()),
            sender_name: Some("Cate".to_string()),
            channel_type: Some("whatsapp:393285497365-1412881543@g.us".to_string()),
            is_group: true,
            was_mentioned: true,
            ephemeral: false,
            thinking: None,
            show_thinking: None,
            group_participants: Some(vec![
                ParticipantRef {
                    jid: "111@s.whatsapp.net".to_string(),
                    display_name: "Alice".to_string(),
                },
                ParticipantRef {
                    jid: "222@s.whatsapp.net".to_string(),
                    display_name: "Bob".to_string(),
                },
            ]),
            session_id: None,
            incognito: true,
        };
        let session_override = Some(SessionId::new());

        let (sender_ctx, incognito, sid) = build_streaming_kernel_args(&req, session_override);

        // sender_context: every field that influences resolver behaviour
        // must flow through.
        let sender_ctx = sender_ctx.expect("sender_id present must yield SenderContext");
        assert_eq!(sender_ctx.channel, "whatsapp:393285497365-1412881543@g.us");
        assert_eq!(sender_ctx.user_id, "393285497365@s.whatsapp.net");
        assert_eq!(sender_ctx.display_name, "Cate");
        assert!(sender_ctx.is_group);
        assert!(sender_ctx.was_mentioned);
        assert_eq!(sender_ctx.group_participants.len(), 2);

        // incognito + session override must not be dropped on the path to
        // the kernel call.
        assert!(incognito, "incognito flag must propagate to kernel call");
        assert_eq!(
            sid, session_override,
            "session_id_override must propagate unchanged"
        );

        // Negative branch: no sender_id → no sender_context (resolver
        // falls back to global Persistent — historical behaviour for
        // direct API callers).
        let bare = MessageRequest {
            message: "test".to_string(),
            attachments: Vec::new(),
            sender_id: None,
            sender_name: None,
            channel_type: None,
            is_group: false,
            was_mentioned: false,
            ephemeral: false,
            thinking: None,
            show_thinking: None,
            group_participants: None,
            session_id: None,
            incognito: false,
        };
        let (ctx, _, _) = build_streaming_kernel_args(&bare, None);
        assert!(
            ctx.is_none(),
            "missing sender_id must produce None — kernel then uses its own fallback"
        );
    }

    #[test]
    fn test_effective_default_model_prefers_override() {
        let base = librefang_types::config::DefaultModelConfig {
            provider: "openai".to_string(),
            model: "gpt-4.1".to_string(),
            api_key_env: "OPENAI_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::BTreeMap::new(),
            cli_profile_dirs: Vec::new(),
        };
        let override_dm = librefang_types::config::DefaultModelConfig {
            provider: "deepseek".to_string(),
            model: "deepseek-chat".to_string(),
            api_key_env: "DEEPSEEK_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::BTreeMap::new(),
            cli_profile_dirs: Vec::new(),
        };

        let effective = effective_default_model(&base, Some(&override_dm));

        assert_eq!(effective.provider, "deepseek");
        assert_eq!(effective.model, "deepseek-chat");
        assert_eq!(effective.api_key_env, "DEEPSEEK_API_KEY");
    }

    #[test]
    fn test_effective_default_model_falls_back_to_base() {
        let base = librefang_types::config::DefaultModelConfig {
            provider: "openai".to_string(),
            model: "gpt-4.1".to_string(),
            api_key_env: "OPENAI_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::BTreeMap::new(),
            cli_profile_dirs: Vec::new(),
        };

        let effective = effective_default_model(&base, None);

        assert_eq!(effective.provider, "openai");
        assert_eq!(effective.model, "gpt-4.1");
        assert_eq!(effective.api_key_env, "OPENAI_API_KEY");
    }

    #[test]
    fn test_patch_config_request_temperature_deserialization() {
        let json = r#"{"temperature": 1.5}"#;
        let req: PatchAgentConfigRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.temperature, Some(1.5));
        assert!(req.max_tokens.is_none());
        assert!(req.model.is_none());
    }

    #[test]
    fn test_patch_config_request_temperature_range() {
        // Valid ranges
        for temp in [0.0, 0.5, 1.0, 1.5, 2.0] {
            let json = format!(r#"{{"temperature": {temp}}}"#);
            let req: PatchAgentConfigRequest = serde_json::from_str(&json).unwrap();
            assert_eq!(req.temperature, Some(temp));
        }

        // Out of range values still deserialize (validation happens in handler)
        let json = r#"{"temperature": 3.0}"#;
        let req: PatchAgentConfigRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.temperature, Some(3.0));

        // Negative values still deserialize (validation happens in handler)
        let json = r#"{"temperature": -0.5}"#;
        let req: PatchAgentConfigRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.temperature, Some(-0.5));
    }

    #[test]
    fn test_patch_config_request_without_temperature() {
        let json = r#"{"max_tokens": 4096}"#;
        let req: PatchAgentConfigRequest = serde_json::from_str(json).unwrap();
        assert!(req.temperature.is_none());
        assert_eq!(req.max_tokens, Some(4096));
    }

    /// #3464 — when the awaiting future is dropped (simulates client
    /// disconnect), the spawned task is aborted within ~10ms so the kernel
    /// stops doing work for a vanished caller.
    #[tokio::test]
    async fn run_cancel_on_disconnect_aborts_inner_task_when_caller_drops() {
        let observed_progress = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let observed_completion = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

        let progress = observed_progress.clone();
        let completion = observed_completion.clone();
        let inner = async move {
            for _ in 0..200 {
                progress.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            completion.store(true, std::sync::atomic::Ordering::Relaxed);
        };

        // Spawn the helper, drop the join future after a short delay to
        // simulate the axum response future being dropped.
        let helper = run_cancel_on_disconnect(inner);
        let join = tokio::spawn(helper);

        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        join.abort();
        let _ = join.await; // Reaping the JoinHandle drops the helper future.

        // Give the abort signal time to propagate.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let snapshot = observed_progress.load(std::sync::atomic::Ordering::Relaxed);

        // Wait further; if cancellation works the inner task stopped.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let later = observed_progress.load(std::sync::atomic::Ordering::Relaxed);

        assert_eq!(
            snapshot, later,
            "inner task must stop counting after caller drops (got {snapshot} → {later})"
        );
        assert!(
            !observed_completion.load(std::sync::atomic::Ordering::Relaxed),
            "inner task must not run to completion after cancellation"
        );
    }

    /// #3464 — once `disarm()` has been called, dropping the guard MUST
    /// NOT abort the spawned task. This is the streaming path's
    /// invariant: after `ContentComplete` reaches the client, the
    /// kernel still runs settle-reservation / canonical-append / audit
    /// writes; if the SSE stream ends a few ms later and the guard
    /// drops, those side-effects must complete instead of being
    /// silently cancelled.
    #[tokio::test]
    async fn abort_on_drop_after_disarm_does_not_abort_task() {
        let completed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let completed_inner = completed.clone();

        let handle = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(80)).await;
            completed_inner.store(true, std::sync::atomic::Ordering::Relaxed);
        });

        let mut guard = AbortOnDrop::new(handle.abort_handle());
        // Simulate observing `ContentComplete`: release abort permission.
        guard.disarm();
        // Drop the guard immediately, simulating SSE stream end racing
        // ahead of the kernel post-stream cleanup.
        drop(guard);

        // The task must still be allowed to finish.
        let _ = handle.await;
        assert!(
            completed.load(std::sync::atomic::Ordering::Relaxed),
            "disarmed guard must NOT abort the task on drop — \
             post-stream settle/audit work would be silently cancelled"
        );
    }

    /// Regression for the 2026-05-20 cross-chat image leak:
    /// `resolve_attachment_session_id` MUST mirror `send_message_*`'s
    /// resolver. When a non-empty `sender_context` is supplied (and the
    /// caller hasn't asked for the canonical session), the attachment
    /// session MUST be the per-chat `SessionId::for_sender_scope(...)`,
    /// NOT the agent's registry-default `fallback_session_id`.
    #[test]
    fn resolve_attachment_session_id_uses_per_chat_session_for_channel_turn() {
        use librefang_channels::types::SenderContext;
        use librefang_types::agent::SessionId;
        let agent_id = AgentId::new();
        let registry_default = SessionId::new();
        let sender = SenderContext {
            channel: "whatsapp".to_string(),
            user_id: "user-1".to_string(),
            chat_id: Some("chat-XYZ".to_string()),
            display_name: "Alice".to_string(),
            use_canonical_session: false,
            ..Default::default()
        };
        let expected =
            SessionId::for_sender_scope(agent_id, &sender.channel, sender.chat_id.as_deref());
        let resolved =
            resolve_attachment_session_id(agent_id, Some(&sender), None, registry_default);
        assert_eq!(
            resolved, expected,
            "channel-scoped attachment MUST land in the per-chat session, \
             not the agent's registry-default session"
        );
        assert_ne!(
            resolved, registry_default,
            "registry default is the very bug being fixed — must not be used \
             when sender_context has a non-empty channel"
        );
    }

    /// Explicit `session_id_override` (multi-tab WebUI, REST callers that
    /// already pinned a session) must win over channel-derived resolution
    /// AND over the registry-default fallback. Mirrors priority #1 in the
    /// helper's doc comment.
    #[test]
    fn resolve_attachment_session_id_honours_explicit_override() {
        use librefang_channels::types::SenderContext;
        use librefang_types::agent::SessionId;
        let agent_id = AgentId::new();
        let registry_default = SessionId::new();
        let explicit = SessionId::new();
        let sender = SenderContext {
            channel: "whatsapp".to_string(),
            user_id: "user-1".to_string(),
            chat_id: Some("chat-XYZ".to_string()),
            display_name: "Alice".to_string(),
            use_canonical_session: false,
            ..Default::default()
        };
        let resolved = resolve_attachment_session_id(
            agent_id,
            Some(&sender),
            Some(explicit),
            registry_default,
        );
        assert_eq!(resolved, explicit);
        assert_ne!(resolved, registry_default);
    }

    /// Last-resort fallback: no override, no sender context — the registry
    /// default is the only sane choice. Mirrors the
    /// `SessionMode::Persistent => entry.session_id` branch of
    /// `kernel/messaging.rs`.
    #[test]
    fn resolve_attachment_session_id_falls_back_to_registry_default_when_no_context() {
        use librefang_types::agent::SessionId;
        let agent_id = AgentId::new();
        let registry_default = SessionId::new();
        let resolved = resolve_attachment_session_id(agent_id, None, None, registry_default);
        assert_eq!(resolved, registry_default);
    }

    /// WebUI sets `use_canonical_session: true` on its `SenderContext` so
    /// the canonical (registry-default) session is reused across browser
    /// reloads — the helper MUST honour that opt-in and not derive a
    /// per-chat session from the WebUI channel name.
    #[test]
    fn resolve_attachment_session_id_honours_use_canonical_session() {
        use librefang_channels::types::SenderContext;
        use librefang_types::agent::SessionId;
        let agent_id = AgentId::new();
        let registry_default = SessionId::new();
        let sender = SenderContext {
            channel: "webui".to_string(),
            user_id: "127.0.0.1".to_string(),
            display_name: "Web UI".to_string(),
            use_canonical_session: true,
            ..Default::default()
        };
        let resolved =
            resolve_attachment_session_id(agent_id, Some(&sender), None, registry_default);
        assert_eq!(resolved, registry_default);
    }
}

#[cfg(test)]
mod monitoring_tests {
    use super::*;
    use axum::extract::{Path, Query, State};
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use librefang_kernel::audit::AuditAction;
    use librefang_kernel::MemorySubsystemApi;
    use librefang_types::config::KernelConfig;

    fn monitoring_test_app_state() -> (Arc<AppState>, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let home_dir = tmp.path().join("librefang-api-monitoring-test");
        std::fs::create_dir_all(&home_dir).unwrap();

        let config = KernelConfig {
            home_dir: home_dir.clone(),
            data_dir: home_dir.join("data"),
            ..KernelConfig::default()
        };

        let kernel = Arc::new(librefang_kernel::LibreFangKernel::boot_with_config(config).unwrap());
        let idempotency_store: Arc<
            dyn librefang_memory::idempotency::IdempotencyStore + Send + Sync,
        > = Arc::new(librefang_memory::idempotency::SqliteIdempotencyStore::new(
            kernel.substrate_ref().pool(),
        ));
        let passkey_store: Arc<dyn librefang_memory::passkey_store::PasskeyStore + Send + Sync> =
            Arc::new(librefang_memory::passkey_store::SqlitePasskeyStore::new(
                kernel.substrate_ref().pool(),
            ));
        let state = Arc::new(AppState {
            kernel,
            started_at: std::time::Instant::now(),
            bridge_manager: arc_swap::ArcSwap::new(std::sync::Arc::new(None)),
            channels_config: tokio::sync::RwLock::new(Default::default()),
            shutdown_notify: Arc::new(tokio::sync::Notify::new()),
            clawhub_cache: dashmap::DashMap::new(),
            skillhub_cache: dashmap::DashMap::new(),
            provider_probe_cache: librefang_kernel::provider_health::ProbeCache::new(),
            provider_test_cache: dashmap::DashMap::new(),
            webhook_store: crate::webhook_store::WebhookStore::load(
                home_dir.join("data").join("webhooks.json"),
            ),
            active_sessions: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            media_drivers: librefang_kernel::media::MediaDriverCache::new(),
            webhook_router: Arc::new(tokio::sync::RwLock::new(Arc::new(axum::Router::new()))),
            api_key_lock: Arc::new(tokio::sync::RwLock::new(String::new())),
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            config_write_lock: tokio::sync::Mutex::new(()),
            pending_a2a_agents: dashmap::DashMap::new(),
            auth_login_limiter: std::sync::Arc::new(crate::rate_limiter::AuthLoginLimiter::new()),
            gcra_limiter: crate::rate_limiter::create_rate_limiter(0),
            trusted_proxies: Arc::new(crate::client_ip::TrustedProxies::default()),
            trust_forwarded_for: false,
            idempotency_store,
            passkey_store,
            passkey_engine: None,
        });
        (state, tmp)
    }

    fn spawn_monitoring_test_agent(state: &Arc<AppState>, name: &str) -> AgentId {
        let manifest = AgentManifest {
            name: name.to_string(),
            ..AgentManifest::default()
        };
        state.kernel.spawn_agent_typed(manifest).unwrap()
    }

    async fn json_response(response: impl IntoResponse) -> (StatusCode, serde_json::Value) {
        let response = response.into_response();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json = serde_json::from_slice(&body).unwrap();
        (status, json)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_agent_metrics_returns_json_shape_for_existing_agent() {
        let (state, _tmp) = monitoring_test_app_state();
        let agent_id = spawn_monitoring_test_agent(&state, "metrics-shape");

        let (status, body) =
            json_response(agent_metrics(State(state), Path(agent_id.to_string()), None).await)
                .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["agent_id"], agent_id.to_string());
        assert!(body["token_usage"].is_object());
        assert!(body["tool_calls"].is_object());
        assert!(body.get("avg_response_time_ms").is_some());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_agent_metrics_returns_not_found_for_unknown_agent() {
        let (state, _tmp) = monitoring_test_app_state();

        let (status, body) = json_response(
            agent_metrics(State(state), Path(AgentId::new().to_string()), None).await,
        )
        .await;

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "Agent not found");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_agent_logs_filters_level_by_exact_match() {
        let (state, _tmp) = monitoring_test_app_state();
        let agent_id = spawn_monitoring_test_agent(&state, "logs-filter");
        let agent_id_str = agent_id.to_string();

        state.kernel.audit().record(
            agent_id_str.clone(),
            AuditAction::AgentMessage,
            "exact match target",
            "custom_error",
        );
        state.kernel.audit().record(
            agent_id_str.clone(),
            AuditAction::AgentMessage,
            "should not match substring filter",
            "not_custom_error",
        );

        let mut params = HashMap::new();
        params.insert("level".to_string(), "custom_error".to_string());

        let (status, body) =
            json_response(agent_logs(State(state), Path(agent_id_str), None, Query(params)).await)
                .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["count"], 1);

        let logs = body["logs"].as_array().unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0]["outcome"], "custom_error");
    }

    #[test]
    fn test_patch_agent_mcp_servers_parses_top_level_and_nested_shapes() {
        let top_level = serde_json::json!({"mcp_servers": ["alpha", "beta"]});
        assert_eq!(
            patch_agent_mcp_servers(&top_level).unwrap(),
            Some(vec!["alpha".to_string(), "beta".to_string()])
        );

        let nested = serde_json::json!({"capabilities": {"mcp_servers": ["gamma"]}});
        assert_eq!(
            patch_agent_mcp_servers(&nested).unwrap(),
            Some(vec!["gamma".to_string()])
        );
    }

    #[test]
    fn test_patch_agent_mcp_servers_rejects_invalid_shape() {
        let invalid = serde_json::json!({"mcp_servers": [{}]});
        assert!(patch_agent_mcp_servers(&invalid).is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_patch_agent_updates_top_level_mcp_servers_and_persists() {
        let (state, _tmp) = monitoring_test_app_state();
        let manifest = AgentManifest {
            name: "patch-top-level-mcp".to_string(),
            mcp_servers: vec!["server-a".to_string()],
            ..AgentManifest::default()
        };
        let agent_id = state.kernel.spawn_agent_typed(manifest).unwrap();

        let (status, body) = json_response(
            patch_agent(
                State(state.clone()),
                Path(agent_id.to_string()),
                None,
                Json(serde_json::json!({"mcp_servers": []})),
            )
            .await,
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "ok");
        assert_eq!(
            state
                .kernel
                .agent_registry()
                .get(agent_id)
                .unwrap()
                .manifest
                .mcp_servers,
            Vec::<String>::new()
        );
        assert_eq!(
            state
                .kernel
                .memory_substrate()
                .load_agent(agent_id)
                .unwrap()
                .unwrap()
                .manifest
                .mcp_servers,
            Vec::<String>::new()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_patch_agent_updates_nested_capabilities_mcp_servers_and_persists() {
        let (state, _tmp) = monitoring_test_app_state();
        let manifest = AgentManifest {
            name: "patch-nested-mcp".to_string(),
            mcp_servers: vec!["server-b".to_string()],
            ..AgentManifest::default()
        };
        let agent_id = state.kernel.spawn_agent_typed(manifest).unwrap();

        let (status, body) = json_response(
            patch_agent(
                State(state.clone()),
                Path(agent_id.to_string()),
                None,
                Json(serde_json::json!({"capabilities": {"mcp_servers": []}})),
            )
            .await,
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "ok");
        assert_eq!(
            state
                .kernel
                .agent_registry()
                .get(agent_id)
                .unwrap()
                .manifest
                .mcp_servers,
            Vec::<String>::new()
        );
        assert_eq!(
            state
                .kernel
                .memory_substrate()
                .load_agent(agent_id)
                .unwrap()
                .unwrap()
                .manifest
                .mcp_servers,
            Vec::<String>::new()
        );
    }

    // ---------------------------------------------------------------------
    // Attachment session-isolation regression tests (2026-05-20 incident).
    //
    // PR #5288 closed the streaming text-path leak by threading
    // `SenderContext` through `send_message_stream`. The matching
    // attachment-pre-inject path (`inject_attachments_into_session`) was
    // missed: it called `SessionWriter::inject_attachment_blocks` with
    // only `(kernel, agent_id, blocks)` and the kernel impl wrote into
    // the agent's persistent registry session. For a warm group chat
    // agent that meant a subsequent DM image landed in the most-recent
    // group session — a hard cross-chat data leak (real production
    // incident: owner DM Amazon order screenshot reached a public group
    // 1 h later). The tests below pin the per-chat session derivation
    // so the bug stays fixed.
    // ---------------------------------------------------------------------

    #[test]
    fn attachment_session_id_uses_explicit_override_when_set() {
        use librefang_channels::types::SenderContext;
        use librefang_types::agent::{AgentId, SessionId};

        let agent_id = AgentId::new();
        let override_sid = SessionId::new();
        let entry_sid = SessionId::new();
        let ctx = SenderContext {
            channel: "whatsapp".to_string(),
            chat_id: Some("121043@s.whatsapp.net".to_string()),
            ..Default::default()
        };

        let resolved =
            resolve_attachment_session_id(agent_id, Some(&ctx), Some(override_sid), entry_sid);

        assert_eq!(
            resolved, override_sid,
            "explicit session_id_override must win over both channel derivation and fallback"
        );
    }

    #[test]
    fn attachment_session_id_derives_per_chat_scope_for_dm_vs_group() {
        use librefang_channels::types::SenderContext;
        use librefang_types::agent::{AgentId, SessionId};

        let agent_id = AgentId::new();
        let entry_sid = SessionId::new();

        // Incident-shape DM: chat_id = 121043 (private), is_group = false.
        let dm_ctx = SenderContext {
            channel: "whatsapp".to_string(),
            chat_id: Some("121043@s.whatsapp.net".to_string()),
            is_group: false,
            ..Default::default()
        };

        // Incident-shape group: chat_id = 120957 ("Non perdiamoci 💻").
        let group_ctx = SenderContext {
            channel: "whatsapp".to_string(),
            chat_id: Some("120957@g.us".to_string()),
            is_group: true,
            ..Default::default()
        };

        let dm_sid = resolve_attachment_session_id(agent_id, Some(&dm_ctx), None, entry_sid);
        let group_sid = resolve_attachment_session_id(agent_id, Some(&group_ctx), None, entry_sid);

        let expected_dm =
            SessionId::for_sender_scope(agent_id, "whatsapp", Some("121043@s.whatsapp.net"));
        let expected_group = SessionId::for_sender_scope(agent_id, "whatsapp", Some("120957@g.us"));

        assert_eq!(
            dm_sid, expected_dm,
            "DM attachment must land on the UUID-v5 chat-scoped session, not entry.session_id"
        );
        assert_eq!(
            group_sid, expected_group,
            "group session resolution mismatch"
        );
        assert_ne!(
            dm_sid, group_sid,
            "DM and group sessions must be distinct — this is the cross-chat leak guard"
        );
        assert_ne!(
            dm_sid, entry_sid,
            "DM attachment must NOT land on the agent's persistent registry session \
             (that is the 2026-05-20 incident's exact failure mode)"
        );
        assert_ne!(
            group_sid, entry_sid,
            "group attachment must NOT land on the agent's persistent registry session"
        );
    }

    #[test]
    fn attachment_session_id_falls_back_to_entry_session_for_canonical_or_no_channel() {
        use librefang_channels::types::SenderContext;
        use librefang_types::agent::{AgentId, SessionId};

        let agent_id = AgentId::new();
        let entry_sid = SessionId::new();

        // WebUI shape: non-empty channel BUT use_canonical_session=true.
        let webui_ctx = SenderContext {
            channel: "webui".to_string(),
            user_id: "127.0.0.1".to_string(),
            use_canonical_session: true,
            ..Default::default()
        };
        let webui_resolved =
            resolve_attachment_session_id(agent_id, Some(&webui_ctx), None, entry_sid);
        assert_eq!(
            webui_resolved, entry_sid,
            "use_canonical_session=true must collapse onto entry.session_id"
        );

        // Direct REST caller with no sender_context — must fall back.
        let none_resolved = resolve_attachment_session_id(agent_id, None, None, entry_sid);
        assert_eq!(
            none_resolved, entry_sid,
            "no sender context + no override → entry.session_id fallback"
        );

        // Empty channel string is treated like no channel context.
        let empty_chan = SenderContext {
            channel: String::new(),
            ..Default::default()
        };
        let empty_resolved =
            resolve_attachment_session_id(agent_id, Some(&empty_chan), None, entry_sid);
        assert_eq!(
            empty_resolved, entry_sid,
            "empty channel string must fall through to the persistent fallback, \
             mirroring kernel/messaging.rs"
        );
    }

    /// Direct regression for the 2026-05-20 incident shape: WhatsApp DM
    /// chat 121043 arrives with an Amazon-order image; without the fix
    /// the derived sid would be the agent's registry session (the warm
    /// group session 120957). After the fix the DM attachment lands on
    /// `SessionId::for_sender_scope(agent, "whatsapp", "121043@…")` — a
    /// UUID v5 distinct from BOTH the group session and the registry
    /// session.
    #[test]
    fn incident_20260520_dm_attachment_does_not_land_on_group_session() {
        use librefang_channels::types::SenderContext;
        use librefang_types::agent::{AgentId, SessionId};

        let agent_id = AgentId::new();
        // The warm group session that "won" in the production incident.
        let warm_group_session =
            SessionId::for_sender_scope(agent_id, "whatsapp", Some("120957@g.us"));
        // The persistent registry session — what
        // `inject_attachment_blocks` used to write to unconditionally.
        let registry_session = warm_group_session;

        let dm_ctx = SenderContext {
            channel: "whatsapp".to_string(),
            chat_id: Some("121043@s.whatsapp.net".to_string()),
            is_group: false,
            ..Default::default()
        };

        let resolved =
            resolve_attachment_session_id(agent_id, Some(&dm_ctx), None, registry_session);

        let expected_dm =
            SessionId::for_sender_scope(agent_id, "whatsapp", Some("121043@s.whatsapp.net"));
        assert_eq!(
            resolved, expected_dm,
            "DM image must land on its own UUID v5 session"
        );
        assert_ne!(
            resolved, warm_group_session,
            "CROSS-CHAT LEAK GUARD: DM attachment must not land on the group session — \
             this assertion failing would mean the 2026-05-20 incident has regressed"
        );
    }
}

#[cfg(test)]
mod kernel_err_to_status_tests {
    //! Regression guards for the audit fix
    //! `agent-not-found-returns-500`. The helper is the single
    //! shared mapping table used by 5 session-route err arms +
    //! `clone_agent`'s spawn-error arm. Pinning the table here
    //! means adding a new `LibreFangError` variant that should
    //! map to a non-500 status requires an arm in
    //! `kernel_err_to_status` *and* a test here; both will be
    //! caught by `cargo test` if missed.
    use super::kernel_err_to_status;
    use crate::error::KernelError;
    use axum::http::StatusCode;
    use librefang_types::error::LibreFangError;

    #[test]
    fn agent_not_found_maps_to_404() {
        let err = KernelError::LibreFang(LibreFangError::AgentNotFound("agt_xyz".to_string()));
        assert_eq!(kernel_err_to_status(&err), StatusCode::NOT_FOUND);
    }

    #[test]
    fn agent_already_exists_maps_to_409() {
        let err =
            KernelError::LibreFang(LibreFangError::AgentAlreadyExists("dup-name".to_string()));
        assert_eq!(kernel_err_to_status(&err), StatusCode::CONFLICT);
    }

    #[test]
    fn other_libre_fang_errors_default_to_500() {
        // Sanity: the catch-all preserves the pre-fix behaviour so
        // a transient kernel error doesn't surprise-surface as a
        // client-error class.
        let err = KernelError::LibreFang(LibreFangError::Internal("disk full".to_string()));
        assert_eq!(
            kernel_err_to_status(&err),
            StatusCode::INTERNAL_SERVER_ERROR,
        );
    }
}

#[cfg(test)]
mod url_attachment_ssrf_tests {
    //! SSRF regression guards for `resolve_url_attachments`. The
    //! function is called from `POST /api/a2a/send` (and reachable to
    //! the `User` role per `middleware.rs` allowlist), so any URL we
    //! fetch on the caller's behalf must pass the same blocklist the
    //! webhook subscription store uses at fire-time. A returned empty
    //! block list — paired with a `warn!` — is the contract: the
    //! attacker gets no IMDS / RFC 1918 / link-local / IPv6-ULA round
    //! trip, and no fetched bytes land in the agent session for the
    //! LLM to transcribe back.
    use super::resolve_url_attachments;
    use librefang_types::comms::Attachment;

    fn img(url: &str) -> Attachment {
        Attachment {
            url: url.to_string(),
            filename: None,
            content_type: Some("image/png".to_string()),
            caption: None,
        }
    }

    #[tokio::test]
    async fn rejects_loopback_literal() {
        // The original exploit pathway — bare 127.0.0.1 reaches any
        // localhost-bound service (admin UI, kernel API on 4545, etc).
        let blocks = resolve_url_attachments(&[img("http://127.0.0.1:1/whatever.png")]).await;
        assert!(blocks.is_empty(), "loopback literal must be refused");
    }

    #[tokio::test]
    async fn rejects_imds_literal() {
        // The headline AWS / GCP / Azure cloud-metadata exfil target.
        let blocks = resolve_url_attachments(&[img(
            "http://169.254.169.254/latest/meta-data/iam/security-credentials/role.png",
        )])
        .await;
        assert!(blocks.is_empty(), "IMDS literal must be refused");
    }

    #[tokio::test]
    async fn rejects_ipv6_ula_literal() {
        // fc00::/7 covers fd00::/8 — common kubernetes / docker
        // internal-network range. is_private_ip's V6 arm must catch it.
        let blocks = resolve_url_attachments(&[img("http://[fd00::1]/whatever.png")]).await;
        assert!(blocks.is_empty(), "IPv6 ULA literal must be refused");
    }

    #[tokio::test]
    async fn rejects_localhost_hostname() {
        // Hostname (not literal) — caught by the blocked-domain check
        // in validate_webhook_url, no DNS query happens.
        let blocks = resolve_url_attachments(&[img("http://localhost/whatever.png")]).await;
        assert!(blocks.is_empty(), "localhost hostname must be refused");
    }

    #[tokio::test]
    async fn rejects_rfc1918_literal() {
        // 10.0.0.0/8 — common corporate-LAN target for SSRF pivots.
        let blocks = resolve_url_attachments(&[img("http://10.0.0.1/whatever.png")]).await;
        assert!(blocks.is_empty(), "RFC 1918 literal must be refused");
    }

    #[tokio::test]
    async fn rejects_unsupported_scheme() {
        // `file://`, `gopher://`, etc. would otherwise be a different
        // exfil class entirely. validate_webhook_url only permits
        // http / https — non-image content_type would also skip, but
        // the SSRF guard is the canonical reject path.
        let mut a = img("file:///etc/passwd");
        a.content_type = Some("image/png".to_string());
        let blocks = resolve_url_attachments(&[a]).await;
        assert!(
            blocks.is_empty(),
            "non-http(s) scheme must be refused by SSRF guard"
        );
    }
}
