//! LibreFang daemon server — boots the kernel and serves the HTTP API.

use crate::channel_bridge;
use crate::middleware;
use crate::rate_limiter;
use crate::routes::{self, AppState};
use crate::webchat;
use crate::ws;
use axum::response::IntoResponse;
use axum::Router;
use librefang_kernel::LibreFangKernel;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;
use tracing::info;

/// Daemon info written to `~/.librefang/daemon.json` so the CLI can find us.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct DaemonInfo {
    pub pid: u32,
    pub listen_addr: String,
    pub started_at: String,
    pub version: String,
    pub platform: String,
}

/// Current API version. Bump when introducing a new version.
pub const API_VERSION_LATEST: &str = crate::versioning::CURRENT_VERSION;

/// All available API versions with their status.
pub const API_VERSIONS: &[(&str, &str)] = &[("v1", "stable")];

/// Build the v1 API route tree.
///
/// Returns a `Router` with paths relative to the mount point (e.g. `/health`,
/// `/agents`, etc.). The caller nests this under `/api` and `/api/v1`.
///
/// Adding a future v2 is straightforward: create `api_v2_routes()` and nest it
/// at `/api/v2`, then update `API_VERSION_LATEST` and `API_VERSIONS`.
fn api_v1_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/metrics", axum::routing::get(routes::prometheus_metrics))
        .route("/health", axum::routing::get(routes::health))
        .route("/health/detail", axum::routing::get(routes::health_detail))
        .route("/status", axum::routing::get(routes::status))
        .route("/version", axum::routing::get(routes::version))
        .route(
            "/agents",
            axum::routing::get(routes::list_agents).post(routes::spawn_agent),
        )
        // Bulk agent operations (before /agents/{id} to avoid path conflicts)
        .route(
            "/agents/bulk",
            axum::routing::post(routes::bulk_create_agents).delete(routes::bulk_delete_agents),
        )
        .route(
            "/agents/bulk/start",
            axum::routing::post(routes::bulk_start_agents),
        )
        .route(
            "/agents/bulk/stop",
            axum::routing::post(routes::bulk_stop_agents),
        )
        .route(
            "/agents/{id}",
            axum::routing::get(routes::get_agent)
                .delete(routes::kill_agent)
                .patch(routes::patch_agent),
        )
        .route(
            "/agents/{id}/mode",
            axum::routing::put(routes::set_agent_mode),
        )
        .route(
            "/agents/{id}/suspend",
            axum::routing::put(routes::suspend_agent),
        )
        .route(
            "/agents/{id}/resume",
            axum::routing::put(routes::resume_agent),
        )
        .route("/profiles", axum::routing::get(routes::list_profiles))
        .route("/profiles/{name}", axum::routing::get(routes::get_profile))
        .route(
            "/agents/{id}/message",
            axum::routing::post(routes::send_message),
        )
        .route(
            "/agents/{id}/message/stream",
            axum::routing::post(routes::send_message_stream),
        )
        .route(
            "/agents/{id}/session",
            axum::routing::get(routes::get_agent_session),
        )
        .route(
            "/agents/{id}/sessions",
            axum::routing::get(routes::list_agent_sessions).post(routes::create_agent_session),
        )
        .route(
            "/agents/{id}/sessions/{session_id}/switch",
            axum::routing::post(routes::switch_agent_session),
        )
        .route(
            "/agents/{id}/session/reset",
            axum::routing::post(routes::reset_session),
        )
        .route(
            "/agents/{id}/session/reboot",
            axum::routing::post(routes::reboot_session),
        )
        .route(
            "/agents/{id}/history",
            axum::routing::delete(routes::clear_agent_history),
        )
        .route(
            "/agents/{id}/session/compact",
            axum::routing::post(routes::compact_session),
        )
        .route("/agents/{id}/stop", axum::routing::post(routes::stop_agent))
        .route("/agents/{id}/model", axum::routing::put(routes::set_model))
        .route(
            "/agents/{id}/traces",
            axum::routing::get(routes::get_agent_traces),
        )
        .route(
            "/agents/{id}/tools",
            axum::routing::get(routes::get_agent_tools).put(routes::set_agent_tools),
        )
        .route(
            "/agents/{id}/skills",
            axum::routing::get(routes::get_agent_skills).put(routes::set_agent_skills),
        )
        .route(
            "/agents/{id}/mcp_servers",
            axum::routing::get(routes::get_agent_mcp_servers).put(routes::set_agent_mcp_servers),
        )
        .route(
            "/agents/{id}/identity",
            axum::routing::patch(routes::update_agent_identity),
        )
        .route(
            "/agents/{id}/config",
            axum::routing::patch(routes::patch_agent_config),
        )
        .route(
            "/agents/{id}/clone",
            axum::routing::post(routes::clone_agent),
        )
        .route(
            "/agents/{id}/files",
            axum::routing::get(routes::list_agent_files),
        )
        .route(
            "/agents/{id}/files/{filename}",
            axum::routing::get(routes::get_agent_file)
                .put(routes::set_agent_file)
                .delete(routes::delete_agent_file),
        )
        .route(
            "/agents/{id}/metrics",
            axum::routing::get(routes::agent_metrics),
        )
        .route("/agents/{id}/logs", axum::routing::get(routes::agent_logs))
        .route(
            "/agents/{id}/deliveries",
            axum::routing::get(routes::get_agent_deliveries),
        )
        .route(
            "/agents/{id}/upload",
            axum::routing::post(routes::upload_file),
        )
        .route("/agents/{id}/ws", axum::routing::get(ws::agent_ws))
        .route(
            "/uploads/{file_id}",
            axum::routing::get(routes::serve_upload),
        )
        .route("/channels", axum::routing::get(routes::list_channels))
        .route("/channels/{name}", axum::routing::get(routes::get_channel))
        .route(
            "/channels/{name}/configure",
            axum::routing::post(routes::configure_channel).delete(routes::remove_channel),
        )
        .route(
            "/channels/{name}/test",
            axum::routing::post(routes::test_channel),
        )
        .route(
            "/channels/reload",
            axum::routing::post(routes::reload_channels),
        )
        .route(
            "/channels/whatsapp/qr/start",
            axum::routing::post(routes::whatsapp_qr_start),
        )
        .route(
            "/channels/whatsapp/qr/status",
            axum::routing::get(routes::whatsapp_qr_status),
        )
        .route("/templates", axum::routing::get(routes::list_templates))
        .route(
            "/templates/{name}",
            axum::routing::get(routes::get_template),
        )
        .route(
            "/memory/agents/{id}/kv",
            axum::routing::get(routes::get_agent_kv),
        )
        .route(
            "/memory/agents/{id}/kv/{key}",
            axum::routing::get(routes::get_agent_kv_key)
                .put(routes::set_agent_kv_key)
                .delete(routes::delete_agent_kv_key),
        )
        .route(
            "/agents/{id}/memory/export",
            axum::routing::get(routes::export_agent_memory),
        )
        .route(
            "/agents/{id}/memory/import",
            axum::routing::post(routes::import_agent_memory),
        )
        // Proactive memory (mem0-style) endpoints
        .route(
            "/memory",
            axum::routing::get(routes::memory_list).post(routes::memory_add),
        )
        .route("/memory/search", axum::routing::get(routes::memory_search))
        .route("/memory/stats", axum::routing::get(routes::memory_stats))
        .route(
            "/memory/cleanup",
            axum::routing::post(routes::memory_cleanup),
        )
        .route("/memory/decay", axum::routing::post(routes::memory_decay))
        .route(
            "/memory/bulk-delete",
            axum::routing::post(routes::memory_bulk_delete),
        )
        .route(
            "/memory/items/{memory_id}",
            axum::routing::put(routes::memory_update).delete(routes::memory_delete),
        )
        .route(
            "/memory/items/{memory_id}/history",
            axum::routing::get(routes::memory_history),
        )
        .route(
            "/memory/user/{user_id}",
            axum::routing::get(routes::memory_get_user),
        )
        // Per-agent proactive memory endpoints
        .route(
            "/memory/agents/{id}",
            axum::routing::get(routes::memory_list_agent).delete(routes::memory_reset_agent),
        )
        .route(
            "/memory/agents/{id}/search",
            axum::routing::get(routes::memory_search_agent),
        )
        .route(
            "/memory/agents/{id}/stats",
            axum::routing::get(routes::memory_stats_agent),
        )
        .route(
            "/memory/agents/{id}/level/{level}",
            axum::routing::delete(routes::memory_clear_level),
        )
        .route(
            "/memory/agents/{id}/duplicates",
            axum::routing::get(routes::memory_duplicates),
        )
        .route(
            "/memory/agents/{id}/consolidate",
            axum::routing::post(routes::memory_consolidate),
        )
        .route(
            "/memory/agents/{id}/count",
            axum::routing::get(routes::memory_count_agent),
        )
        .route(
            "/memory/agents/{id}/relations",
            axum::routing::get(routes::memory_query_relations).post(routes::memory_store_relations),
        )
        .route(
            "/memory/agents/{id}/export",
            axum::routing::get(routes::memory_export_agent),
        )
        .route(
            "/memory/agents/{id}/import",
            axum::routing::post(routes::memory_import_agent),
        )
        .route(
            "/triggers",
            axum::routing::get(routes::list_triggers).post(routes::create_trigger),
        )
        .route(
            "/triggers/{id}",
            axum::routing::delete(routes::delete_trigger).put(routes::update_trigger),
        )
        .route(
            "/schedules",
            axum::routing::get(routes::list_schedules).post(routes::create_schedule),
        )
        .route(
            "/schedules/{id}",
            axum::routing::get(routes::get_schedule)
                .delete(routes::delete_schedule)
                .put(routes::update_schedule),
        )
        .route(
            "/schedules/{id}/run",
            axum::routing::post(routes::run_schedule),
        )
        .route(
            "/workflows",
            axum::routing::get(routes::list_workflows).post(routes::create_workflow),
        )
        .route(
            "/workflows/{id}",
            axum::routing::get(routes::get_workflow)
                .put(routes::update_workflow)
                .delete(routes::delete_workflow),
        )
        .route(
            "/workflows/{id}/run",
            axum::routing::post(routes::run_workflow),
        )
        .route(
            "/workflows/{id}/runs",
            axum::routing::get(routes::list_workflow_runs),
        )
        .route("/skills", axum::routing::get(routes::list_skills))
        .route(
            "/skills/install",
            axum::routing::post(routes::install_skill),
        )
        .route(
            "/skills/uninstall",
            axum::routing::post(routes::uninstall_skill),
        )
        .route(
            "/marketplace/search",
            axum::routing::get(routes::marketplace_search),
        )
        .route(
            "/clawhub/search",
            axum::routing::get(routes::clawhub_search),
        )
        .route(
            "/clawhub/browse",
            axum::routing::get(routes::clawhub_browse),
        )
        .route(
            "/clawhub/skill/{slug}",
            axum::routing::get(routes::clawhub_skill_detail),
        )
        .route(
            "/clawhub/skill/{slug}/code",
            axum::routing::get(routes::clawhub_skill_code),
        )
        .route(
            "/clawhub/install",
            axum::routing::post(routes::clawhub_install),
        )
        .route("/hands", axum::routing::get(routes::list_hands))
        .route("/hands/install", axum::routing::post(routes::install_hand))
        .route(
            "/hands/active",
            axum::routing::get(routes::list_active_hands),
        )
        .route("/hands/{hand_id}", axum::routing::get(routes::get_hand))
        .route(
            "/hands/{hand_id}/activate",
            axum::routing::post(routes::activate_hand),
        )
        .route(
            "/hands/{hand_id}/check-deps",
            axum::routing::post(routes::check_hand_deps),
        )
        .route(
            "/hands/{hand_id}/install-deps",
            axum::routing::post(routes::install_hand_deps),
        )
        .route(
            "/hands/{hand_id}/settings",
            axum::routing::get(routes::get_hand_settings).put(routes::update_hand_settings),
        )
        .route(
            "/hands/instances/{id}/pause",
            axum::routing::post(routes::pause_hand),
        )
        .route(
            "/hands/instances/{id}/resume",
            axum::routing::post(routes::resume_hand),
        )
        .route(
            "/hands/instances/{id}",
            axum::routing::delete(routes::deactivate_hand),
        )
        .route(
            "/hands/instances/{id}/stats",
            axum::routing::get(routes::hand_stats),
        )
        .route(
            "/hands/instances/{id}/browser",
            axum::routing::get(routes::hand_instance_browser),
        )
        .route(
            "/mcp/servers",
            axum::routing::get(routes::list_mcp_servers).post(routes::add_mcp_server),
        )
        .route(
            "/mcp/servers/{name}",
            axum::routing::get(routes::get_mcp_server)
                .put(routes::update_mcp_server)
                .delete(routes::delete_mcp_server),
        )
        .route("/audit/recent", axum::routing::get(routes::audit_recent))
        .route("/audit/verify", axum::routing::get(routes::audit_verify))
        .route("/logs/stream", axum::routing::get(routes::logs_stream))
        .route("/peers", axum::routing::get(routes::list_peers))
        .route("/peers/{id}", axum::routing::get(routes::get_peer))
        .route(
            "/network/status",
            axum::routing::get(routes::network_status),
        )
        .route(
            "/comms/topology",
            axum::routing::get(routes::comms_topology),
        )
        .route("/comms/events", axum::routing::get(routes::comms_events))
        .route(
            "/comms/events/stream",
            axum::routing::get(routes::comms_events_stream),
        )
        .route("/comms/send", axum::routing::post(routes::comms_send))
        .route("/comms/task", axum::routing::post(routes::comms_task))
        .route("/tools", axum::routing::get(routes::list_tools))
        .route("/tools/{name}", axum::routing::get(routes::get_tool))
        .route("/config", axum::routing::get(routes::get_config))
        .route("/config/schema", axum::routing::get(routes::config_schema))
        .route("/config/set", axum::routing::post(routes::config_set))
        .route(
            "/approvals",
            axum::routing::get(routes::list_approvals).post(routes::create_approval),
        )
        .route("/approvals/{id}", axum::routing::get(routes::get_approval))
        .route(
            "/approvals/{id}/approve",
            axum::routing::post(routes::approve_request),
        )
        .route(
            "/approvals/{id}/reject",
            axum::routing::post(routes::reject_request),
        )
        .route("/usage", axum::routing::get(routes::usage_stats))
        .route("/usage/summary", axum::routing::get(routes::usage_summary))
        .route(
            "/usage/by-model",
            axum::routing::get(routes::usage_by_model),
        )
        .route("/usage/daily", axum::routing::get(routes::usage_daily))
        .route(
            "/budget",
            axum::routing::get(routes::budget_status).put(routes::update_budget),
        )
        .route(
            "/budget/agents",
            axum::routing::get(routes::agent_budget_ranking),
        )
        .route(
            "/budget/agents/{id}",
            axum::routing::get(routes::agent_budget_status).put(routes::update_agent_budget),
        )
        .route("/sessions", axum::routing::get(routes::list_sessions))
        .route(
            "/sessions/cleanup",
            axum::routing::post(routes::session_cleanup),
        )
        .route(
            "/sessions/{id}",
            axum::routing::get(routes::get_session).delete(routes::delete_session),
        )
        .route(
            "/sessions/{id}/label",
            axum::routing::put(routes::set_session_label),
        )
        .route(
            "/agents/{id}/sessions/by-label/{label}",
            axum::routing::get(routes::find_session_by_label),
        )
        .route(
            "/agents/{id}/update",
            axum::routing::put(routes::update_agent),
        )
        .route("/security", axum::routing::get(routes::security_status))
        .route("/models", axum::routing::get(routes::list_models))
        .route(
            "/models/aliases",
            axum::routing::get(routes::list_aliases).post(routes::create_alias),
        )
        .route(
            "/models/aliases/{alias}",
            axum::routing::delete(routes::delete_alias),
        )
        .route(
            "/models/custom",
            axum::routing::post(routes::add_custom_model),
        )
        .route(
            "/models/custom/{*id}",
            axum::routing::delete(routes::remove_custom_model),
        )
        .route("/models/{*id}", axum::routing::get(routes::get_model))
        .route("/providers", axum::routing::get(routes::list_providers))
        .route(
            "/catalog/update",
            axum::routing::post(routes::catalog_update),
        )
        .route(
            "/catalog/status",
            axum::routing::get(routes::catalog_status),
        )
        .route(
            "/providers/ollama/detect",
            axum::routing::get(routes::detect_ollama),
        )
        .route(
            "/providers/github-copilot/oauth/start",
            axum::routing::post(routes::copilot_oauth_start),
        )
        .route(
            "/providers/github-copilot/oauth/poll/{poll_id}",
            axum::routing::get(routes::copilot_oauth_poll),
        )
        .route(
            "/providers/{name}/key",
            axum::routing::post(routes::set_provider_key).delete(routes::delete_provider_key),
        )
        .route(
            "/providers/{name}/test",
            axum::routing::post(routes::test_provider),
        )
        .route(
            "/providers/{name}/url",
            axum::routing::put(routes::set_provider_url),
        )
        .route(
            "/providers/{name}",
            axum::routing::get(routes::get_provider),
        )
        .route("/skills/create", axum::routing::post(routes::create_skill))
        .route("/extensions", axum::routing::get(routes::list_extensions))
        .route(
            "/extensions/install",
            axum::routing::post(routes::install_extension),
        )
        .route(
            "/extensions/uninstall",
            axum::routing::post(routes::uninstall_extension),
        )
        .route(
            "/extensions/{name}",
            axum::routing::get(routes::get_extension),
        )
        // Context engine plugins
        .route(
            "/plugins/registries",
            axum::routing::get(routes::list_plugin_registries),
        )
        .route("/plugins", axum::routing::get(routes::list_plugins))
        .route(
            "/plugins/install",
            axum::routing::post(routes::install_plugin),
        )
        .route(
            "/plugins/uninstall",
            axum::routing::post(routes::uninstall_plugin),
        )
        .route(
            "/plugins/scaffold",
            axum::routing::post(routes::scaffold_plugin),
        )
        .route("/plugins/{name}", axum::routing::get(routes::get_plugin))
        .route(
            "/plugins/{name}/install-deps",
            axum::routing::post(routes::install_plugin_deps),
        )
        .route(
            "/migrate/detect",
            axum::routing::get(routes::migrate_detect),
        )
        .route("/migrate/scan", axum::routing::post(routes::migrate_scan))
        .route("/migrate", axum::routing::post(routes::run_migrate))
        // Goals endpoints
        .route(
            "/goals",
            axum::routing::get(routes::list_goals).post(routes::create_goal),
        )
        .route(
            "/goals/{id}",
            axum::routing::get(routes::get_goal)
                .put(routes::update_goal_by_id)
                .delete(routes::delete_goal),
        )
        .route(
            "/goals/{id}/children",
            axum::routing::get(routes::get_goal_children),
        )
        .route(
            "/cron/jobs",
            axum::routing::get(routes::list_cron_jobs).post(routes::create_cron_job),
        )
        .route(
            "/cron/jobs/{id}",
            axum::routing::get(routes::get_cron_job)
                .delete(routes::delete_cron_job)
                .put(routes::update_cron_job),
        )
        .route(
            "/cron/jobs/{id}/enable",
            axum::routing::put(routes::toggle_cron_job),
        )
        .route(
            "/cron/jobs/{id}/status",
            axum::routing::get(routes::cron_job_status),
        )
        // Queue status endpoint
        .route("/queue/status", axum::routing::get(routes::queue_status))
        // Backup / Restore endpoints
        .route("/backup", axum::routing::post(routes::create_backup))
        .route("/backups", axum::routing::get(routes::list_backups))
        .route(
            "/backups/{filename}",
            axum::routing::delete(routes::delete_backup),
        )
        .route("/restore", axum::routing::post(routes::restore_backup))
        // Task queue management endpoints (#184)
        .route(
            "/tasks/status",
            axum::routing::get(routes::task_queue_status),
        )
        .route("/tasks/list", axum::routing::get(routes::task_queue_list))
        .route(
            "/tasks/{id}",
            axum::routing::delete(routes::task_queue_delete),
        )
        .route(
            "/tasks/{id}/retry",
            axum::routing::post(routes::task_queue_retry),
        )
        // Event webhook subscription endpoints (#185)
        .route(
            "/webhooks/events",
            axum::routing::get(routes::list_event_webhooks).post(routes::create_event_webhook),
        )
        .route(
            "/webhooks/events/{id}",
            axum::routing::put(routes::update_event_webhook).delete(routes::delete_event_webhook),
        )
        // Outbound webhook management endpoints (#179)
        .route(
            "/webhooks",
            axum::routing::get(routes::list_webhooks).post(routes::create_webhook),
        )
        .route(
            "/webhooks/{id}",
            axum::routing::get(routes::get_webhook)
                .put(routes::update_webhook)
                .delete(routes::delete_webhook),
        )
        .route(
            "/webhooks/{id}/test",
            axum::routing::post(routes::test_webhook),
        )
        // Webhook trigger endpoints (external event injection)
        .route("/hooks/wake", axum::routing::post(routes::webhook_wake))
        .route("/hooks/agent", axum::routing::post(routes::webhook_agent))
        .route("/shutdown", axum::routing::post(routes::shutdown))
        // Chat commands endpoint (dynamic slash menu)
        .route("/commands", axum::routing::get(routes::list_commands))
        .route("/commands/{name}", axum::routing::get(routes::get_command))
        .route("/config/reload", axum::routing::post(routes::config_reload))
        .route(
            "/bindings",
            axum::routing::get(routes::list_bindings).post(routes::add_binding),
        )
        .route(
            "/bindings/{index}",
            axum::routing::delete(routes::remove_binding),
        )
        .route(
            "/a2a/agents",
            axum::routing::get(routes::a2a_list_external_agents),
        )
        .route(
            "/a2a/agents/{id}",
            axum::routing::get(routes::a2a_get_external_agent),
        )
        .route(
            "/a2a/discover",
            axum::routing::post(routes::a2a_discover_external),
        )
        .route("/a2a/send", axum::routing::post(routes::a2a_send_external))
        .route(
            "/a2a/tasks/{id}/status",
            axum::routing::get(routes::a2a_external_task_status),
        )
        .route(
            "/integrations",
            axum::routing::get(routes::list_integrations),
        )
        .route(
            "/integrations/available",
            axum::routing::get(routes::list_available_integrations),
        )
        .route(
            "/integrations/add",
            axum::routing::post(routes::add_integration),
        )
        .route(
            "/integrations/{id}",
            axum::routing::get(routes::get_integration).delete(routes::remove_integration),
        )
        .route(
            "/integrations/{id}/reconnect",
            axum::routing::post(routes::reconnect_integration),
        )
        .route(
            "/integrations/health",
            axum::routing::get(routes::integrations_health),
        )
        .route(
            "/integrations/reload",
            axum::routing::post(routes::reload_integrations),
        )
        .route(
            "/pairing/request",
            axum::routing::post(routes::pairing_request),
        )
        .route(
            "/pairing/complete",
            axum::routing::post(routes::pairing_complete),
        )
        .route(
            "/pairing/devices",
            axum::routing::get(routes::pairing_devices),
        )
        .route(
            "/pairing/devices/{id}",
            axum::routing::delete(routes::pairing_remove_device),
        )
        .route(
            "/pairing/notify",
            axum::routing::post(routes::pairing_notify),
        )
        // Dashboard credential login
        .route(
            "/auth/dashboard-login",
            axum::routing::post(dashboard_login),
        )
        .route(
            "/auth/dashboard-check",
            axum::routing::get(dashboard_auth_check),
        )
        // OAuth/OIDC external authentication endpoints
        .route(
            "/auth/providers",
            axum::routing::get(crate::oauth::auth_providers),
        )
        .route("/auth/login", axum::routing::get(crate::oauth::auth_login))
        .route(
            "/auth/login/{provider}",
            axum::routing::get(crate::oauth::auth_login_provider),
        )
        .route(
            "/auth/callback",
            axum::routing::get(crate::oauth::auth_callback).post(crate::oauth::auth_callback_post),
        )
        .route(
            "/auth/userinfo",
            axum::routing::get(crate::oauth::auth_userinfo),
        )
        .route(
            "/auth/introspect",
            axum::routing::post(crate::oauth::auth_introspect),
        )
}

/// Dashboard credential login — validates username/password from config.toml
/// and returns a session token (HMAC-derived from credentials).
async fn dashboard_login(
    axum::extract::State(state): axum::extract::State<Arc<routes::AppState>>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::response::Response {
    let cfg = &state.kernel.config;
    let cfg_user = cfg.dashboard_user.trim();
    let cfg_pass = cfg.dashboard_pass.trim();

    // If not configured, login is not needed
    if cfg_user.is_empty() || cfg_pass.is_empty() {
        return axum::response::Json(serde_json::json!({
            "ok": true, "token": "", "message": "No credentials required"
        }))
        .into_response();
    }

    let user = body.get("username").and_then(|v| v.as_str()).unwrap_or("");
    let pass = body.get("password").and_then(|v| v.as_str()).unwrap_or("");

    // Constant-time comparison
    use subtle::ConstantTimeEq;
    let user_ok = user.as_bytes().ct_eq(cfg_user.as_bytes());
    let pass_ok = pass.as_bytes().ct_eq(cfg_pass.as_bytes());

    if user_ok.into() && pass_ok.into() {
        // Generate a deterministic token from credentials so no server-side state needed
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        let mut mac =
            Hmac::<Sha256>::new_from_slice(cfg_pass.as_bytes()).expect("HMAC key");
        mac.update(cfg_user.as_bytes());
        mac.update(b"librefang-dashboard-session");
        let token = mac.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect::<String>();

        axum::response::Json(serde_json::json!({
            "ok": true,
            "token": token,
        }))
        .into_response()
    } else {
        (
            axum::http::StatusCode::UNAUTHORIZED,
            axum::response::Json(serde_json::json!({
                "ok": false,
                "error": "Invalid username or password"
            })),
        )
            .into_response()
    }
}

/// Check what auth mode the dashboard needs.
async fn dashboard_auth_check(
    axum::extract::State(state): axum::extract::State<Arc<routes::AppState>>,
) -> axum::response::Json<serde_json::Value> {
    let cfg = &state.kernel.config;
    let has_credentials =
        !cfg.dashboard_user.trim().is_empty() && !cfg.dashboard_pass.trim().is_empty();
    let has_api_key = !cfg.api_key.trim().is_empty();

    axum::response::Json(serde_json::json!({
        "mode": if has_credentials { "credentials" } else if has_api_key { "api_key" } else { "none" },
    }))
}

/// Build the full API router with all routes, middleware, and state.
///
/// This is extracted from `run_daemon()` so that embedders (e.g. librefang-desktop)
/// can create the router without starting the full daemon lifecycle.
///
/// Returns `(router, shared_state)`. The caller can use `state.bridge_manager`
/// to shut down the bridge on exit.
pub async fn build_router(
    kernel: Arc<LibreFangKernel>,
    listen_addr: SocketAddr,
) -> (Router<()>, Arc<AppState>) {
    // Start channel bridges (Telegram, etc.)
    let bridge = channel_bridge::start_channel_bridge(kernel.clone()).await;

    let channels_config = kernel.config.channels.clone();
    let state = Arc::new(AppState {
        kernel: kernel.clone(),
        started_at: Instant::now(),
        peer_registry: kernel.peer_registry.get().map(|r| Arc::new(r.clone())),
        bridge_manager: tokio::sync::Mutex::new(bridge),
        channels_config: tokio::sync::RwLock::new(channels_config),
        shutdown_notify: Arc::new(tokio::sync::Notify::new()),
        clawhub_cache: dashmap::DashMap::new(),
        provider_probe_cache: librefang_runtime::provider_health::ProbeCache::new(),
        webhook_store: crate::webhook_store::WebhookStore::load(
            kernel.config.home_dir.join("webhooks.json"),
        ),
    });

    // CORS: allow localhost origins by default, plus any configured in cors_origin.
    let cors = {
        let port = listen_addr.port();
        let mut origins: Vec<axum::http::HeaderValue> = vec![
            format!("http://{listen_addr}").parse().unwrap(),
            format!("http://localhost:{port}").parse().unwrap(),
            format!("http://127.0.0.1:{port}").parse().unwrap(),
        ];
        // Also allow common dev ports
        for p in [3000u16, 8080] {
            if p != port {
                if let Ok(v) = format!("http://127.0.0.1:{p}").parse() {
                    origins.push(v);
                }
                if let Ok(v) = format!("http://localhost:{p}").parse() {
                    origins.push(v);
                }
            }
        }
        // Add explicitly configured CORS origins from config.toml
        for origin in &state.kernel.config.cors_origin {
            if let Ok(v) = origin.parse::<axum::http::HeaderValue>() {
                origins.push(v);
            } else {
                tracing::warn!("Invalid CORS origin in config, skipping: {origin}");
            }
        }
        CorsLayer::new()
            .allow_origin(origins)
            .allow_methods(tower_http::cors::Any)
            .allow_headers(tower_http::cors::Any)
    };

    // Trim whitespace so `api_key = ""` or `api_key = "  "` both disable auth.
    let mut api_key = state.kernel.config.api_key.trim().to_string();

    // If dashboard credentials are configured but no api_key, derive one from credentials.
    // This ensures the auth middleware protects write endpoints when credentials are set.
    if api_key.is_empty() {
        let du = state.kernel.config.dashboard_user.trim();
        let dp = state.kernel.config.dashboard_pass.trim();
        if !du.is_empty() && !dp.is_empty() {
            use hmac::{Hmac, Mac};
            use sha2::Sha256;
            let mut mac = Hmac::<Sha256>::new_from_slice(dp.as_bytes()).expect("HMAC key");
            mac.update(du.as_bytes());
            mac.update(b"librefang-dashboard-session");
            api_key = mac.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect::<String>();
        }
    }
    let gcra_limiter = rate_limiter::create_rate_limiter();

    // Build the versioned API routes. All /api/* endpoints are defined once
    // in api_v1_routes() and mounted at both /api and /api/v1 for backward
    // compatibility. Future versions (v2, v3) can be added as separate routers.
    let v1_routes = api_v1_routes();

    let app = Router::new()
        .route("/", axum::routing::get(webchat::webchat_page))
        .route(
            "/react-assets/{*path}",
            axum::routing::get(webchat::react_asset),
        )
        .route("/logo.png", axum::routing::get(webchat::logo_png))
        .route("/favicon.ico", axum::routing::get(webchat::favicon_ico))
        .route("/locales/en.json", axum::routing::get(webchat::locale_en))
        .route("/locales/ja.json", axum::routing::get(webchat::locale_ja))
        .route(
            "/locales/zh-CN.json",
            axum::routing::get(webchat::locale_zh_cn),
        )
        // API version discovery endpoint (not versioned itself)
        .route("/api/versions", axum::routing::get(routes::api_versions))
        // Auto-generated OpenAPI specification
        .route(
            "/api/openapi.json",
            axum::routing::get(crate::openapi::openapi_spec),
        )
        // Mount v1 routes at /api/v1 (explicit version)
        .nest("/api/v1", v1_routes.clone())
        // Mount the same routes at /api (latest version alias for backward compat)
        .nest("/api", v1_routes)
        // Webhook trigger endpoints (not versioned - external callers use fixed URLs)
        .route("/hooks/wake", axum::routing::post(routes::webhook_wake))
        .route("/hooks/agent", axum::routing::post(routes::webhook_agent))
        // A2A (Agent-to-Agent) Protocol endpoints (protocol-level, not versioned)
        .route(
            "/.well-known/agent.json",
            axum::routing::get(routes::a2a_agent_card),
        )
        .route("/a2a/agents", axum::routing::get(routes::a2a_list_agents))
        .route(
            "/a2a/tasks/send",
            axum::routing::post(routes::a2a_send_task),
        )
        .route("/a2a/tasks/{id}", axum::routing::get(routes::a2a_get_task))
        .route(
            "/a2a/tasks/{id}/cancel",
            axum::routing::post(routes::a2a_cancel_task),
        )
        // MCP HTTP endpoint (protocol-level, not versioned)
        .route("/mcp", axum::routing::post(routes::mcp_http))
        // OpenAI-compatible API (follows OpenAI versioning, not ours)
        .route(
            "/v1/chat/completions",
            axum::routing::post(crate::openai_compat::chat_completions),
        )
        .route(
            "/v1/models",
            axum::routing::get(crate::openai_compat::list_models),
        )
        .layer(axum::middleware::from_fn_with_state(
            api_key,
            middleware::auth,
        ))
        .layer(axum::middleware::from_fn(middleware::accept_language))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::oauth::oidc_auth_middleware,
        ))
        .layer(axum::middleware::from_fn_with_state(
            gcra_limiter,
            rate_limiter::gcra_rate_limit,
        ))
        .layer(axum::middleware::from_fn(middleware::api_version_headers))
        .layer(axum::middleware::from_fn(middleware::security_headers))
        .layer(axum::middleware::from_fn(middleware::request_logging))
        .layer(RequestBodyLimitLayer::new(
            crate::validation::MAX_REQUEST_BODY_BYTES,
        ))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state.clone());

    (app, state)
}

/// Start the LibreFang daemon: boot kernel + HTTP API server.
///
/// This function blocks until Ctrl+C or a shutdown request.
pub async fn run_daemon(
    kernel: LibreFangKernel,
    listen_addr: &str,
    daemon_info_path: Option<&Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    let addr: SocketAddr = listen_addr.parse()?;

    let kernel = Arc::new(kernel);
    kernel.set_self_handle();
    kernel.start_background_agents().await;

    // Config file hot-reload watcher (polls every 30 seconds)
    {
        let k = kernel.clone();
        let config_path = kernel.config.home_dir.join("config.toml");
        tokio::spawn(async move {
            let mut last_modified = std::fs::metadata(&config_path)
                .and_then(|m| m.modified())
                .ok();
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                let current = std::fs::metadata(&config_path)
                    .and_then(|m| m.modified())
                    .ok();
                if current != last_modified && current.is_some() {
                    last_modified = current;
                    tracing::info!("Config file changed, reloading...");
                    match k.reload_config() {
                        Ok(plan) => {
                            if plan.has_changes() {
                                tracing::info!("Config hot-reload applied: {:?}", plan.hot_actions);
                            } else {
                                tracing::debug!("Config hot-reload: no actionable changes");
                            }
                        }
                        Err(e) => tracing::warn!("Config hot-reload failed: {e}"),
                    }
                }
            }
        });
    }

    let (app, state) = build_router(kernel.clone(), addr).await;

    // Write daemon info file
    if let Some(info_path) = daemon_info_path {
        // Check if another daemon is already running with this PID file
        if info_path.exists() {
            if let Ok(existing) = std::fs::read_to_string(info_path) {
                if let Ok(info) = serde_json::from_str::<DaemonInfo>(&existing) {
                    // PID alive AND the health endpoint responds → truly running
                    if is_process_alive(info.pid) && is_daemon_responding(&info.listen_addr) {
                        return Err(format!(
                            "Another daemon (PID {}) is already running at {}",
                            info.pid, info.listen_addr
                        )
                        .into());
                    }
                }
            }
            // Stale PID file (process dead or different process reused PID), remove it
            info!("Removing stale daemon info file");
            let _ = std::fs::remove_file(info_path);
        }

        let daemon_info = DaemonInfo {
            pid: std::process::id(),
            listen_addr: addr.to_string(),
            started_at: chrono::Utc::now().to_rfc3339(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            platform: std::env::consts::OS.to_string(),
        };
        if let Ok(json) = serde_json::to_string_pretty(&daemon_info) {
            let _ = std::fs::write(info_path, json);
            // SECURITY: Restrict daemon info file permissions (contains PID and port).
            restrict_permissions(info_path);
        }
    }

    info!(
        "LibreFang v{} ({}) built {} [{}]",
        env!("CARGO_PKG_VERSION"),
        env!("GIT_SHA"),
        env!("BUILD_DATE"),
        std::env::consts::ARCH,
    );
    info!("LibreFang API server listening on http://{addr}");
    info!("WebChat UI available at http://{addr}/",);
    info!("WebSocket endpoint: ws://{addr}/api/agents/{{id}}/ws",);

    // Background: sync model catalog from community repo on startup, then every 24 hours
    {
        let kernel = state.kernel.clone();
        tokio::spawn(async move {
            loop {
                match librefang_runtime::catalog_sync::sync_catalog_to(&kernel.config.home_dir)
                    .await
                {
                    Ok(result) => {
                        info!(
                            "Model catalog synced: {} files downloaded",
                            result.files_downloaded
                        );
                        if let Ok(mut catalog) = kernel.model_catalog.write() {
                            catalog.load_cached_catalog_for(&kernel.config.home_dir);
                            catalog.detect_auth();
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Background catalog sync failed (will use cached/builtin): {e}"
                        );
                    }
                }
                tokio::time::sleep(std::time::Duration::from_secs(24 * 60 * 60)).await;
            }
        });
    }

    // Use SO_REUSEADDR to allow binding immediately after reboot (avoids TIME_WAIT).
    let socket = socket2::Socket::new(
        if addr.is_ipv4() {
            socket2::Domain::IPV4
        } else {
            socket2::Domain::IPV6
        },
        socket2::Type::STREAM,
        None,
    )?;
    socket.set_reuse_address(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&addr.into())?;
    socket.listen(1024)?;
    let listener = tokio::net::TcpListener::from_std(std::net::TcpListener::from(socket))?;

    // Run server with graceful shutdown.
    // SECURITY: `into_make_service_with_connect_info` injects the peer
    // SocketAddr so the auth middleware can check for loopback connections.
    let api_shutdown = state.shutdown_notify.clone();
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal(api_shutdown))
    .await?;

    // Clean up daemon info file
    if let Some(info_path) = daemon_info_path {
        let _ = std::fs::remove_file(info_path);
    }

    // Stop channel bridges
    if let Some(ref mut b) = *state.bridge_manager.lock().await {
        b.stop().await;
    }

    // Shutdown kernel
    kernel.shutdown();

    info!("LibreFang daemon stopped");
    Ok(())
}

/// SECURITY: Restrict file permissions to owner-only (0600) on Unix.
/// On non-Unix platforms this is a no-op.
#[cfg(unix)]
fn restrict_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) {}

/// Read daemon info from the standard location.
pub fn read_daemon_info(home_dir: &Path) -> Option<DaemonInfo> {
    let info_path = home_dir.join("daemon.json");
    let contents = std::fs::read_to_string(info_path).ok()?;
    serde_json::from_str(&contents).ok()
}

/// Wait for an OS termination signal OR an API shutdown request.
///
/// On Unix: listens for SIGINT, SIGTERM, and API notify.
/// On Windows: listens for Ctrl+C and API notify.
async fn shutdown_signal(api_shutdown: Arc<tokio::sync::Notify>) {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigint = signal(SignalKind::interrupt()).expect("Failed to listen for SIGINT");
        let mut sigterm = signal(SignalKind::terminate()).expect("Failed to listen for SIGTERM");

        tokio::select! {
            _ = sigint.recv() => {
                info!("Received SIGINT (Ctrl+C), shutting down...");
            }
            _ = sigterm.recv() => {
                info!("Received SIGTERM, shutting down...");
            }
            _ = api_shutdown.notified() => {
                info!("Shutdown requested via API, shutting down...");
            }
        }
    }

    #[cfg(not(unix))]
    {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("Ctrl+C received, shutting down...");
            }
            _ = api_shutdown.notified() => {
                info!("Shutdown requested via API, shutting down...");
            }
        }
    }
}

/// Check if a process with the given PID is still alive.
fn is_process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // Use kill -0 to check if process exists without sending a signal
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    #[cfg(windows)]
    {
        // tasklist /FI "PID eq N" returns "INFO: No tasks..." when no match,
        // or a table row with the PID when found. Check exit code and that
        // "INFO:" is NOT in the output to confirm the process exists.
        std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .map(|o| {
                o.status.success() && {
                    let out = String::from_utf8_lossy(&o.stdout);
                    !out.contains("INFO:") && out.contains(&pid.to_string())
                }
            })
            .unwrap_or(false)
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        false
    }
}

/// Check if an LibreFang daemon is actually responding at the given address.
/// This avoids false positives where a different process reused the same PID
/// after a system reboot.
fn is_daemon_responding(addr: &str) -> bool {
    // Quick TCP connect check — don't make a full HTTP request to avoid delays
    let addr_only = addr
        .strip_prefix("http://")
        .or_else(|| addr.strip_prefix("https://"))
        .unwrap_or(addr);
    if let Ok(sock_addr) = addr_only.parse::<std::net::SocketAddr>() {
        std::net::TcpStream::connect_timeout(&sock_addr, std::time::Duration::from_millis(500))
            .is_ok()
    } else {
        // Fallback: try connecting to hostname
        std::net::TcpStream::connect(addr_only)
            .map(|_| true)
            .unwrap_or(false)
    }
}
