//! Auto-generated OpenAPI specification using utoipa.
//!
//! This module defines the `ApiDoc` struct which collects all annotated
//! handlers and schemas into a single OpenAPI 3.1 document.

use axum::http::StatusCode;
use axum::response::IntoResponse;
use utoipa::OpenApi;

use crate::oauth;
use crate::openai_compat;
use crate::routes;
use crate::types;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "LibreFang API",
        version = env!("CARGO_PKG_VERSION"),
        description = "LibreFang Agent Operating System — REST API for managing AI agents, tools, workflows, and more.",
        license(name = "MIT", url = "https://opensource.org/licenses/MIT"),
    ),
    paths(
        // ── System / Health ──
        routes::health,
        routes::health_detail,
        routes::status,
        routes::version,
        routes::api_versions,
        routes::prometheus_metrics,
        routes::get_config,
        routes::config_schema,
        routes::config_set,
        routes::config_reload,
        routes::quick_init,
        routes::security_status,
        routes::shutdown,
        routes::migrate_detect,
        routes::migrate_scan,
        routes::run_migrate,
        routes::list_profiles,
        routes::get_profile,
        routes::list_agent_templates,
        routes::get_agent_template,
        routes::list_commands,
        routes::get_command,
        routes::queue_status,

        // ── Agents ──
        routes::list_agents,
        routes::get_agent_stats,
        routes::spawn_agent,
        routes::get_agent,
        routes::kill_agent,
        routes::patch_agent,
        routes::update_agent,
        routes::send_message,
        routes::send_message_stream,
        routes::attach_session_stream,
        routes::get_agent_session,
        routes::list_agent_sessions,
        routes::create_agent_session,
        routes::switch_agent_session,
        routes::export_session,
        routes::export_session_trajectory,
        routes::import_session,
        routes::reset_session,
        routes::reboot_session,
        routes::clear_agent_history,
        routes::compact_session,
        routes::stop_agent,
        routes::list_agent_runtime,
        routes::stop_session,
        routes::set_model,
        routes::set_agent_mode,
        routes::get_agent_traces,
        routes::get_agent_tools,
        routes::set_agent_tools,
        routes::get_agent_skills,
        routes::set_agent_skills,
        routes::get_agent_mcp_servers,
        routes::set_agent_mcp_servers,
        routes::update_agent_identity,
        routes::patch_agent_config,
        routes::patch_hand_agent_runtime_config,
        routes::delete_hand_agent_runtime_config,
        routes::clone_agent,
        routes::list_agent_files,
        routes::get_agent_file,
        routes::set_agent_file,
        routes::delete_agent_file,
        routes::upload_file,
        routes::serve_upload,
        routes::get_agent_deliveries,

        // ── Bulk Operations ──
        routes::bulk_create_agents,
        routes::bulk_delete_agents,
        routes::bulk_start_agents,
        routes::bulk_stop_agents,

        // ── Skills & Tools ──
        routes::list_skills,
        routes::install_skill,
        routes::uninstall_skill,
        routes::create_skill,
        routes::list_tools,
        routes::get_tool,
        routes::invoke_tool,
        routes::marketplace_search,
        routes::clawhub_search,
        routes::clawhub_browse,
        routes::clawhub_skill_detail,
        routes::clawhub_skill_code,
        routes::clawhub_install,

        // ── Hands ──
        routes::list_hands,
        routes::install_hand,
        routes::list_active_hands,
        routes::get_hand,
        routes::activate_hand,
        routes::check_hand_deps,
        routes::install_hand_deps,
        routes::get_hand_settings,
        routes::update_hand_settings,
        routes::pause_hand,
        routes::resume_hand,
        routes::deactivate_hand,
        routes::hand_stats,
        routes::hand_instance_browser,
        routes::reload_hands,

        // ── MCP Servers (unified) ──
        routes::list_mcp_servers,
        routes::get_mcp_server,
        routes::add_mcp_server,
        routes::update_mcp_server,
        routes::delete_mcp_server,
        routes::reconnect_mcp_server_handler,
        routes::list_mcp_catalog,
        routes::get_mcp_catalog_entry,
        routes::mcp_health_handler,
        routes::reload_mcp_handler,
        routes::list_mcp_taint_rules,

        // ── Extensions (dashboard-friendly aliases over MCP store) ──
        routes::list_extensions,
        routes::get_extension,
        routes::install_extension,
        routes::uninstall_extension,

        // ── Models & Providers ──
        routes::list_models,
        routes::get_model,
        routes::list_aliases,
        routes::create_alias,
        routes::delete_alias,
        routes::add_custom_model,
        routes::remove_custom_model,
        routes::list_providers,
        routes::get_provider,
        routes::set_provider_key,
        routes::delete_provider_key,
        routes::test_provider,
        routes::set_provider_url,
        routes::set_default_provider,
        routes::copilot_oauth_start,
        routes::copilot_oauth_poll,
        routes::catalog_update,
        routes::catalog_status,

        // ── Channels ──
        routes::list_channels,
        routes::configure_channel,
        routes::remove_channel,
        routes::test_channel,
        routes::reload_channels,
        routes::whatsapp_qr_start,
        routes::whatsapp_qr_status,
        routes::wechat_qr_start,
        routes::wechat_qr_status,

        // ── Workflows / Triggers / Schedules / Cron ──
        routes::list_workflows,
        routes::create_workflow,
        routes::update_workflow,
        routes::delete_workflow,
        routes::run_workflow,
        routes::list_workflow_runs,
        routes::save_workflow_as_template,
        routes::list_triggers,
        routes::create_trigger,
        routes::get_trigger,
        routes::delete_trigger,
        routes::update_trigger,
        routes::list_schedules,
        routes::create_schedule,
        routes::get_schedule,
        routes::update_schedule,
        routes::delete_schedule,
        routes::run_schedule,
        routes::list_cron_jobs,
        routes::create_cron_job,
        routes::delete_cron_job,
        routes::update_cron_job,
        routes::toggle_cron_job,
        routes::cron_job_status,

        // ── Sessions ──
        routes::list_sessions,
        routes::get_session,
        routes::delete_session,
        routes::set_session_label,
        routes::find_session_by_label,
        routes::session_cleanup,

        // ── Budget / Usage ──
        routes::budget_status,
        routes::update_budget,
        routes::agent_budget_status,
        routes::agent_budget_ranking,
        routes::update_agent_budget,
        routes::user_budget_ranking,
        routes::user_budget_detail,
        routes::usage_stats,
        routes::usage_summary,
        routes::usage_by_model,
        routes::usage_daily,

        // ── Auto-Dream (background memory consolidation) ──
        routes::auto_dream_status,
        routes::auto_dream_trigger,
        routes::auto_dream_abort,
        routes::auto_dream_set_enabled,

        // ── Users / RBAC ──
        routes::users::list_users,
        routes::users::get_user,
        routes::users::create_user,
        routes::users::update_user,
        routes::users::delete_user,
        routes::users::import_users,
        routes::users::rotate_user_key,

        // ── Memory (KV) ──
        routes::get_agent_kv,
        routes::get_agent_kv_key,
        routes::set_agent_kv_key,
        routes::delete_agent_kv_key,
        routes::export_agent_memory,
        routes::import_agent_memory,

        // ── Proactive Memory (mem0-style) ──
        routes::memory_search,
        routes::memory_list,
        routes::memory_get_user,
        routes::memory_add,
        routes::memory_update,
        routes::memory_delete,
        routes::memory_stats,
        routes::memory_list_agent,
        routes::memory_reset_agent,
        routes::memory_clear_level,
        routes::memory_search_agent,
        routes::memory_stats_agent,
        routes::memory_duplicates,
        routes::memory_history,
        routes::memory_consolidate,
        routes::memory_cleanup,
        routes::memory_export_agent,
        routes::memory_import_agent,

        // ── Audit / Logs ──
        routes::audit_recent,
        routes::audit_verify,
        routes::audit_query,
        routes::audit_export,
        routes::logs_stream,

        // ── Approvals ──
        routes::list_approvals,
        routes::create_approval,
        routes::get_approval,
        routes::approve_request,
        routes::reject_request,

        // ── Webhooks ──
        routes::webhook_wake,
        routes::webhook_agent,

        // ── Backup / Restore ──
        routes::create_backup,
        routes::list_backups,
        routes::delete_backup,
        routes::restore_backup,

        // ── Bindings ──
        routes::list_bindings,
        routes::add_binding,
        routes::remove_binding,

        // ── Pairing ──
        routes::pairing_request,
        routes::pairing_complete,
        routes::pairing_devices,
        routes::pairing_remove_device,
        routes::pairing_notify,

        // ── Network / Peers / Comms ──
        routes::list_peers,
        routes::get_peer,
        routes::network_status,
        routes::comms_topology,
        routes::comms_events,
        routes::comms_events_stream,
        routes::comms_send,
        routes::comms_task,

        // ── A2A (Agent-to-Agent) ──
        routes::a2a_list_external_agents,
        routes::a2a_get_external_agent,
        routes::a2a_discover_external,
        routes::a2a_send_external,
        routes::a2a_external_task_status,
        routes::a2a_agent_card,
        routes::a2a_list_agents,
        routes::a2a_send_task,
        routes::a2a_get_task,
        routes::a2a_cancel_task,

        // ── MCP HTTP ──
        routes::mcp_http,

        // ── OAuth / OIDC ──
        oauth::auth_providers,
        oauth::auth_login,
        oauth::auth_login_provider,
        oauth::auth_callback,
        oauth::auth_callback_post,
        oauth::auth_userinfo,
        oauth::auth_introspect,

        // ── OpenAI-Compatible API ──
        openai_compat::chat_completions,
        openai_compat::list_models,
    ),
    components(schemas(
        types::SpawnRequest,
        types::SpawnResponse,
        types::AttachmentRef,
        types::MessageRequest,
        types::MessageResponse,
        types::SkillInstallRequest,
        types::SkillUninstallRequest,
        types::AgentUpdateRequest,
        types::SetModeRequest,
        types::MigrateRequest,
        types::MigrateScanRequest,
        types::ClawHubInstallRequest,
        types::BulkCreateRequest,
        types::BulkCreateResult,
        types::BulkAgentIdsRequest,
        types::BulkActionResult,
        types::ExtensionInstallRequest,
        types::ExtensionUninstallRequest,
        routes::auto_dream::SetEnabledRequest,
        routes::agents::AgentStats24hView,
        routes::agents::AgentStatsPrevView,
        routes::users::UserView,
        routes::users::UserUpsert,
        routes::users::BulkImportRequest,
        routes::users::BulkImportResult,
        routes::users::BulkImportRow,
        routes::users::RotateKeyResponse,
    )),
    tags(
        (name = "system", description = "Health checks, status, version, config, and system management"),
        (name = "agents", description = "Agent lifecycle — spawn, query, message, kill, configure"),
        (name = "skills", description = "Skill and tool management, ClawHub marketplace"),
        (name = "hands", description = "Browser automation hands management"),
        (name = "mcp", description = "MCP server management and protocol endpoints"),
        (name = "extensions", description = "Extension management"),
        (name = "models", description = "Model catalog, aliases, and provider management"),
        (name = "channels", description = "Messaging channel configuration"),
        (name = "workflows", description = "Workflow, trigger, schedule, and cron job management"),
        (name = "sessions", description = "Session management and cleanup"),
        (name = "budget", description = "Usage tracking and budget management"),
        (name = "memory", description = "Agent key-value memory store"),
        (name = "proactive-memory", description = "Proactive memory system (mem0-style) — semantic memory that agents build automatically"),
        (name = "approvals", description = "Human-in-the-loop approval requests"),
        (name = "webhooks", description = "External webhook triggers"),
        (name = "network", description = "P2P network, peers, and inter-agent communication"),
        (name = "a2a", description = "Agent-to-Agent protocol endpoints"),
        (name = "pairing", description = "Device pairing and mobile sync"),
        (name = "auth", description = "OAuth/OIDC authentication endpoints"),
        (name = "openai", description = "OpenAI-compatible API endpoints"),
        (name = "users", description = "RBAC user management — CRUD over UserConfig entries plus bulk CSV import"),
    ),
)]
pub struct ApiDoc;

/// GET /api/openapi.json — Serve the auto-generated OpenAPI specification.
///
/// The spec includes paths for both `/api/*` (unversioned) and `/api/v1/*`
/// (explicit version) since v1 routes are mounted at both prefixes.
pub async fn openapi_spec() -> impl IntoResponse {
    let doc = ApiDoc::openapi();
    let json_str = match doc.to_json() {
        Ok(j) => j,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to generate OpenAPI spec: {e}"),
            )
                .into_response();
        }
    };

    // Parse the generated spec so we can inject /api/v1/* path copies.
    let mut spec: serde_json::Value = match serde_json::from_str(&json_str) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to parse generated spec: {e}"),
            )
                .into_response();
        }
    };

    // Duplicate every /api/* path as /api/v1/* so clients can discover both
    // the unversioned and explicitly-versioned routes from the single spec.
    if let Some(paths) = spec.get("paths").and_then(|p| p.as_object()).cloned() {
        let mut v1_entries: Vec<(String, serde_json::Value)> = Vec::new();
        for (path, ops) in &paths {
            if let Some(suffix) = path.strip_prefix("/api/") {
                let v1_path = format!("/api/v1/{suffix}");
                if !paths.contains_key(&v1_path) {
                    v1_entries.push((v1_path, ops.clone()));
                }
            }
        }
        if !v1_entries.is_empty() {
            if let Some(paths_obj) = spec.get_mut("paths").and_then(|p| p.as_object_mut()) {
                for (k, v) in v1_entries {
                    paths_obj.insert(k, v);
                }
            }
        }
    }

    match serde_json::to_string(&spec) {
        Ok(output) => (
            StatusCode::OK,
            [(
                axum::http::header::CONTENT_TYPE,
                "application/json; charset=utf-8",
            )],
            output,
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to serialize spec: {e}"),
        )
            .into_response(),
    }
}
