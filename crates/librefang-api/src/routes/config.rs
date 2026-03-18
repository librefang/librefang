//! Health, status, configuration, security, and migration handlers.

use super::AppState;
use crate::types::*;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::sync::Arc;

#[utoipa::path(
    get,
    path = "/api/status",
    tag = "system",
    responses(
        (status = 200, description = "Daemon status", body = serde_json::Value)
    )
)]
pub async fn status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let agents: Vec<serde_json::Value> = state
        .kernel
        .registry
        .list()
        .into_iter()
        .map(|e| {
            serde_json::json!({
                "id": e.id.to_string(),
                "name": e.name,
                "state": format!("{:?}", e.state),
                "mode": e.mode,
                "created_at": e.created_at.to_rfc3339(),
                "model_provider": e.manifest.model.provider,
                "model_name": e.manifest.model.model,
                "profile": e.manifest.profile,
            })
        })
        .collect();

    let uptime = state.started_at.elapsed().as_secs();
    let agent_count = agents.len();

    Json(serde_json::json!({
        "status": "running",
        "version": env!("CARGO_PKG_VERSION"),
        "agent_count": agent_count,
        "default_provider": state.kernel.config.default_model.provider,
        "default_model": state.kernel.config.default_model.model,
        "uptime_seconds": uptime,
        "api_listen": state.kernel.config.api_listen,
        "home_dir": state.kernel.config.home_dir.display().to_string(),
        "log_level": state.kernel.config.log_level,
        "network_enabled": state.kernel.config.network_enabled,
        "agents": agents,
    }))
}

/// POST /api/shutdown — Graceful shutdown.
#[utoipa::path(
    post,
    path = "/api/shutdown",
    tag = "system",
    responses(
        (status = 200, description = "Graceful daemon shutdown", body = serde_json::Value)
    )
)]
pub async fn shutdown(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    tracing::info!("Shutdown requested via API");
    // SECURITY: Record shutdown in audit trail
    state.kernel.audit_log.record(
        "system",
        librefang_runtime::audit::AuditAction::ConfigChange,
        "shutdown requested via API",
        "ok",
    );
    state.kernel.shutdown();
    // Signal the HTTP server to initiate graceful shutdown so the process exits.
    state.shutdown_notify.notify_one();
    Json(serde_json::json!({"status": "shutting_down"}))
}

// ---------------------------------------------------------------------------
// Version endpoint
// ---------------------------------------------------------------------------

/// GET /api/version — Build & version info (includes API versioning).
#[utoipa::path(
    get,
    path = "/api/version",
    tag = "system",
    responses(
        (status = 200, description = "Version information", body = serde_json::Value)
    )
)]
pub async fn version() -> impl IntoResponse {
    Json(serde_json::json!({
        "name": "librefang",
        "version": env!("CARGO_PKG_VERSION"),
        "build_date": option_env!("BUILD_DATE").unwrap_or("dev"),
        "git_sha": option_env!("GIT_SHA").unwrap_or("unknown"),
        "rust_version": option_env!("RUSTC_VERSION").unwrap_or("unknown"),
        "platform": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "api": {
            "current": crate::versioning::CURRENT_VERSION,
            "supported": crate::versioning::SUPPORTED_VERSIONS,
            "deprecated": crate::versioning::DEPRECATED_VERSIONS,
        },
    }))
}

/// GET /api/health — Minimal liveness probe (public, no auth required).
/// Returns only status and version to prevent information leakage.
/// Use GET /api/health/detail for full diagnostics (requires auth).
#[utoipa::path(
    get,
    path = "/api/health",
    tag = "system",
    responses(
        (status = 200, description = "Health check", body = serde_json::Value)
    )
)]
pub async fn health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Check database connectivity
    let shared_id = librefang_types::agent::AgentId(uuid::Uuid::from_bytes([
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
    ]));
    let db_ok = state
        .kernel
        .memory
        .structured_get(shared_id, "__health_check__")
        .is_ok();

    let status = if db_ok { "ok" } else { "degraded" };

    Json(serde_json::json!({
        "status": status,
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// GET /api/health/detail — Full health diagnostics (requires auth).
#[utoipa::path(
    get,
    path = "/api/health/detail",
    tag = "system",
    responses(
        (status = 200, description = "Detailed health diagnostics", body = serde_json::Value)
    )
)]
pub async fn health_detail(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let health = state.kernel.supervisor.health();

    let shared_id = librefang_types::agent::AgentId(uuid::Uuid::from_bytes([
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
    ]));
    let db_ok = state
        .kernel
        .memory
        .structured_get(shared_id, "__health_check__")
        .is_ok();

    let config_warnings = state.kernel.config.validate();
    let status = if db_ok { "ok" } else { "degraded" };

    Json(serde_json::json!({
        "status": status,
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_seconds": state.started_at.elapsed().as_secs(),
        "panic_count": health.panic_count,
        "restart_count": health.restart_count,
        "agent_count": state.kernel.registry.count(),
        "database": if db_ok { "connected" } else { "error" },
        "config_warnings": config_warnings,
    }))
}

// ---------------------------------------------------------------------------
// Prometheus metrics endpoint
// ---------------------------------------------------------------------------

/// GET /api/metrics — Prometheus text-format metrics.
///
/// Returns counters and gauges for monitoring LibreFang in production:
/// - `librefang_agents_active` — number of active agents
/// - `librefang_uptime_seconds` — seconds since daemon started
/// - `librefang_tokens_total` — total tokens consumed (per agent)
/// - `librefang_tool_calls_total` — total tool calls (per agent)
/// - `librefang_panics_total` — supervisor panic count
/// - `librefang_restarts_total` — supervisor restart count
#[utoipa::path(
    get,
    path = "/api/metrics",
    tag = "system",
    responses(
        (status = 200, description = "Prometheus text-format metrics", body = serde_json::Value)
    )
)]
pub async fn prometheus_metrics(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut out = String::with_capacity(2048);

    // Uptime
    let uptime = state.started_at.elapsed().as_secs();
    out.push_str("# HELP librefang_uptime_seconds Time since daemon started.\n");
    out.push_str("# TYPE librefang_uptime_seconds gauge\n");
    out.push_str(&format!("librefang_uptime_seconds {uptime}\n\n"));

    // Active agents
    let agents = state.kernel.registry.list();
    let active = agents
        .iter()
        .filter(|a| matches!(a.state, librefang_types::agent::AgentState::Running))
        .count();
    out.push_str("# HELP librefang_agents_active Number of active agents.\n");
    out.push_str("# TYPE librefang_agents_active gauge\n");
    out.push_str(&format!("librefang_agents_active {active}\n"));
    out.push_str("# HELP librefang_agents_total Total number of registered agents.\n");
    out.push_str("# TYPE librefang_agents_total gauge\n");
    out.push_str(&format!("librefang_agents_total {}\n\n", agents.len()));

    // Per-agent token and tool usage
    out.push_str("# HELP librefang_tokens_total Total tokens consumed (rolling hourly window).\n");
    out.push_str("# TYPE librefang_tokens_total gauge\n");
    out.push_str("# HELP librefang_tool_calls_total Total tool calls (rolling hourly window).\n");
    out.push_str("# TYPE librefang_tool_calls_total gauge\n");
    for agent in &agents {
        let name = &agent.name;
        let provider = &agent.manifest.model.provider;
        let model = &agent.manifest.model.model;
        if let Some((tokens, tools)) = state.kernel.scheduler.get_usage(agent.id) {
            out.push_str(&format!(
                "librefang_tokens_total{{agent=\"{name}\",provider=\"{provider}\",model=\"{model}\"}} {tokens}\n"
            ));
            out.push_str(&format!(
                "librefang_tool_calls_total{{agent=\"{name}\"}} {tools}\n"
            ));
        }
    }
    out.push('\n');

    // Supervisor health
    let health = state.kernel.supervisor.health();
    out.push_str("# HELP librefang_panics_total Total supervisor panics since start.\n");
    out.push_str("# TYPE librefang_panics_total counter\n");
    out.push_str(&format!("librefang_panics_total {}\n", health.panic_count));
    out.push_str("# HELP librefang_restarts_total Total supervisor restarts since start.\n");
    out.push_str("# TYPE librefang_restarts_total counter\n");
    out.push_str(&format!(
        "librefang_restarts_total {}\n\n",
        health.restart_count
    ));

    // Version info
    out.push_str("# HELP librefang_info LibreFang version and build info.\n");
    out.push_str("# TYPE librefang_info gauge\n");
    out.push_str(&format!(
        "librefang_info{{version=\"{}\"}} 1\n",
        env!("CARGO_PKG_VERSION")
    ));

    (
        StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        out,
    )
}

// ---------------------------------------------------------------------------
// Config endpoint
// ---------------------------------------------------------------------------

/// GET /api/config — Get kernel configuration (secrets redacted).
#[utoipa::path(
    get,
    path = "/api/config",
    tag = "system",
    responses(
        (status = 200, description = "Get kernel configuration (secrets redacted)", body = serde_json::Value)
    )
)]
pub async fn get_config(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Return a redacted view of the kernel config
    let config = &state.kernel.config;
    Json(serde_json::json!({
        "home_dir": config.home_dir.to_string_lossy(),
        "data_dir": config.data_dir.to_string_lossy(),
        "api_key": if config.api_key.is_empty() { "not set" } else { "***" },
        "default_model": {
            "provider": config.default_model.provider,
            "model": config.default_model.model,
            "api_key_env": config.default_model.api_key_env,
        },
        "memory": {
            "decay_rate": config.memory.decay_rate,
        },
        "proactive_memory": {
            "auto_memorize": config.proactive_memory.auto_memorize,
            "auto_retrieve": config.proactive_memory.auto_retrieve,
            "max_retrieve": config.proactive_memory.max_retrieve,
            "extraction_threshold": config.proactive_memory.extraction_threshold,
            "extraction_model": config.proactive_memory.extraction_model,
            "extract_categories": config.proactive_memory.extract_categories,
            "session_ttl_hours": config.proactive_memory.session_ttl_hours,
            "confidence_decay_rate": config.proactive_memory.confidence_decay_rate,
            "duplicate_threshold": config.proactive_memory.duplicate_threshold,
        },
    }))
}

// ---------------------------------------------------------------------------
// Migration endpoint
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Security dashboard endpoint
// ---------------------------------------------------------------------------

/// GET /api/security — Security feature status for the dashboard.
#[utoipa::path(
    get,
    path = "/api/security",
    tag = "system",
    responses(
        (status = 200, description = "Security feature status", body = serde_json::Value)
    )
)]
pub async fn security_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let auth_mode = if state.kernel.config.api_key.is_empty() {
        "localhost_only"
    } else {
        "bearer_token"
    };

    let audit_count = state.kernel.audit_log.len();

    Json(serde_json::json!({
        "core_protections": {
            "path_traversal": true,
            "ssrf_protection": true,
            "capability_system": true,
            "privilege_escalation_prevention": true,
            "subprocess_isolation": true,
            "security_headers": true,
            "wire_hmac_auth": true,
            "request_id_tracking": true
        },
        "configurable": {
            "rate_limiter": {
                "enabled": true,
                "tokens_per_minute": 500,
                "algorithm": "GCRA"
            },
            "websocket_limits": {
                "max_per_ip": 5,
                "idle_timeout_secs": 1800,
                "max_message_size": 65536,
                "max_messages_per_minute": 10
            },
            "wasm_sandbox": {
                "fuel_metering": true,
                "epoch_interruption": true,
                "default_timeout_secs": 30,
                "default_fuel_limit": 1_000_000u64
            },
            "auth": {
                "mode": auth_mode,
                "api_key_set": !state.kernel.config.api_key.is_empty()
            }
        },
        "monitoring": {
            "audit_trail": {
                "enabled": true,
                "algorithm": "SHA-256 Merkle Chain",
                "entry_count": audit_count
            },
            "taint_tracking": {
                "enabled": true,
                "tracked_labels": [
                    "ExternalNetwork",
                    "UserInput",
                    "PII",
                    "Secret",
                    "UntrustedAgent"
                ]
            },
            "manifest_signing": {
                "algorithm": "Ed25519",
                "available": true
            }
        },
        "secret_zeroization": true,
        "total_features": 15
    }))
}

#[utoipa::path(
    get,
    path = "/api/migrate/detect",
    tag = "system",
    responses(
        (status = 200, description = "Detect migratable framework installation", body = serde_json::Value)
    )
)]
pub async fn migrate_detect() -> impl IntoResponse {
    match librefang_migrate::openclaw::detect_openclaw_home() {
        Some(path) => {
            let scan = librefang_migrate::openclaw::scan_openclaw_workspace(&path);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "detected": true,
                    "path": path.display().to_string(),
                    "scan": scan,
                })),
            )
        }
        None => (
            StatusCode::OK,
            Json(serde_json::json!({
                "detected": false,
                "path": null,
                "scan": null,
            })),
        ),
    }
}

/// POST /api/migrate/scan — Scan a specific directory for OpenClaw workspace.
#[utoipa::path(
    post,
    path = "/api/migrate/scan",
    tag = "system",
    responses(
        (status = 200, description = "Scan directory for migratable workspace", body = serde_json::Value)
    )
)]
pub async fn migrate_scan(Json(req): Json<MigrateScanRequest>) -> impl IntoResponse {
    let path = std::path::PathBuf::from(&req.path);
    if !path.exists() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Directory not found"})),
        );
    }
    let scan = librefang_migrate::openclaw::scan_openclaw_workspace(&path);
    (StatusCode::OK, Json(serde_json::json!(scan)))
}

/// POST /api/migrate — Run migration from another agent framework.
#[utoipa::path(
    post,
    path = "/api/migrate",
    tag = "system",
    responses(
        (status = 200, description = "Run migration from another agent framework", body = serde_json::Value)
    )
)]
pub async fn run_migrate(Json(req): Json<MigrateRequest>) -> impl IntoResponse {
    let source = match req.source.as_str() {
        "openclaw" => librefang_migrate::MigrateSource::OpenClaw,
        "langchain" => librefang_migrate::MigrateSource::LangChain,
        "autogpt" => librefang_migrate::MigrateSource::AutoGpt,
        other => {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"error": format!("Unknown source: {other}. Use 'openclaw', 'langchain', or 'autogpt'")}),
                ),
            );
        }
    };

    let options = librefang_migrate::MigrateOptions {
        source,
        source_dir: std::path::PathBuf::from(&req.source_dir),
        target_dir: std::path::PathBuf::from(&req.target_dir),
        dry_run: req.dry_run,
    };

    match librefang_migrate::run_migration(&options) {
        Ok(report) => {
            let imported: Vec<serde_json::Value> = report
                .imported
                .iter()
                .map(|i| {
                    serde_json::json!({
                        "kind": format!("{}", i.kind),
                        "name": i.name,
                        "destination": i.destination,
                    })
                })
                .collect();

            let skipped: Vec<serde_json::Value> = report
                .skipped
                .iter()
                .map(|s| {
                    serde_json::json!({
                        "kind": format!("{}", s.kind),
                        "name": s.name,
                        "reason": s.reason,
                    })
                })
                .collect();

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "completed",
                    "dry_run": req.dry_run,
                    "imported": imported,
                    "imported_count": imported.len(),
                    "skipped": skipped,
                    "skipped_count": skipped.len(),
                    "warnings": report.warnings,
                    "report_markdown": report.to_markdown(),
                })),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Migration failed: {e}")})),
        ),
    }
}

// ── Model Catalog Endpoints ─────────────────────────────────────────

// ---------------------------------------------------------------------------
// Config Reload endpoint
// ---------------------------------------------------------------------------

/// POST /api/config/reload — Reload configuration from disk and apply hot-reloadable changes.
///
/// Reads the config file, diffs against current config, validates the new config,
/// and applies hot-reloadable actions (approval policy, cron limits, etc.).
/// Returns the reload plan showing what changed and what was applied.
#[utoipa::path(
    post,
    path = "/api/config/reload",
    tag = "system",
    responses(
        (status = 200, description = "Reload configuration from disk", body = serde_json::Value)
    )
)]
pub async fn config_reload(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // SECURITY: Record config reload in audit trail
    state.kernel.audit_log.record(
        "system",
        librefang_runtime::audit::AuditAction::ConfigChange,
        "config reload requested via API",
        "pending",
    );
    match state.kernel.reload_config() {
        Ok(plan) => {
            let status = if plan.restart_required {
                "partial"
            } else if plan.has_changes() {
                "applied"
            } else {
                "no_changes"
            };

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": status,
                    "restart_required": plan.restart_required,
                    "restart_reasons": plan.restart_reasons,
                    "hot_actions_applied": plan.hot_actions.iter().map(|a| format!("{a:?}")).collect::<Vec<_>>(),
                    "noop_changes": plan.noop_changes,
                })),
            )
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"status": "error", "error": e})),
        ),
    }
}

// ---------------------------------------------------------------------------
// Config Schema endpoint
// ---------------------------------------------------------------------------

/// GET /api/config/schema — Return a simplified JSON description of the config structure.
#[utoipa::path(
    get,
    path = "/api/config/schema",
    tag = "system",
    responses(
        (status = 200, description = "Get config structure schema", body = serde_json::Value)
    )
)]
pub async fn config_schema(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Build provider/model options from model catalog for dropdowns
    let catalog = state
        .kernel
        .model_catalog
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let provider_options: Vec<String> = catalog
        .list_providers()
        .iter()
        .map(|p| p.id.clone())
        .collect();
    let model_options: Vec<serde_json::Value> = catalog
        .list_models()
        .iter()
        .map(|m| serde_json::json!({"id": m.id, "name": m.display_name, "provider": m.provider}))
        .collect();
    drop(catalog);

    Json(serde_json::json!({
        "sections": {
            "general": {
                "root_level": true,
                "fields": {
                    "api_listen": "string",
                    "api_key": "string",
                    "log_level": "string"
                }
            },
            "default_model": {
                "hot_reloadable": true,
                "fields": {
                    "provider": { "type": "select", "options": provider_options },
                    "model": { "type": "select", "options": model_options },
                    "api_key_env": "string",
                    "base_url": "string"
                }
            },
            "memory": {
                "fields": {
                    "decay_rate": "number",
                    "vector_dims": "number"
                }
            },
            "web": {
                "fields": {
                    "provider": "string",
                    "timeout_secs": "number",
                    "max_results": "number"
                }
            },
            "browser": {
                "fields": {
                    "headless": "boolean",
                    "timeout_secs": "number",
                    "executable_path": "string"
                }
            },
            "network": {
                "fields": {
                    "enabled": "boolean",
                    "listen_addr": "string",
                    "shared_secret": "string"
                }
            },
            "extensions": {
                "fields": {
                    "auto_connect": "boolean",
                    "health_check_interval_secs": "number"
                }
            },
            "vault": {
                "fields": {
                    "path": "string"
                }
            },
            "a2a": {
                "fields": {
                    "enabled": "boolean",
                    "name": "string",
                    "description": "string",
                    "url": "string"
                }
            },
            "channels": {
                "fields": {
                    "telegram": "object",
                    "discord": "object",
                    "slack": "object",
                    "whatsapp": "object"
                }
            },
            "proactive_memory": {
                "fields": {
                    "auto_memorize": "boolean",
                    "auto_retrieve": "boolean",
                    "max_retrieve": "number",
                    "extraction_threshold": "number",
                    "extraction_model": "string",
                    "extract_categories": "array",
                    "session_ttl_hours": "number",
                    "confidence_decay_rate": "number",
                    "duplicate_threshold": "number"
                }
            }
        }
    }))
}

// ---------------------------------------------------------------------------
// Config Set endpoint
// ---------------------------------------------------------------------------

/// POST /api/config/set — Set a single config value and persist to config.toml.
///
/// Accepts JSON `{ "path": "section.key", "value": "..." }`.
/// Writes the value to the TOML config file and triggers a reload.
#[utoipa::path(
    post,
    path = "/api/config/set",
    tag = "system",
    responses(
        (status = 200, description = "Set a single config value and persist", body = serde_json::Value)
    )
)]
pub async fn config_set(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let path = match body.get("path").and_then(|v| v.as_str()) {
        Some(p) => p.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"status": "error", "error": "missing 'path' field"})),
            );
        }
    };
    let value = match body.get("value") {
        Some(v) => v.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"status": "error", "error": "missing 'value' field"})),
            );
        }
    };

    let config_path = state.kernel.config.home_dir.join("config.toml");

    // Read existing config as a TOML table, or start fresh
    let mut table: toml::value::Table = if config_path.exists() {
        match std::fs::read_to_string(&config_path) {
            Ok(content) => toml::from_str(&content).unwrap_or_default(),
            Err(_) => toml::value::Table::new(),
        }
    } else {
        toml::value::Table::new()
    };

    // Convert JSON value to TOML value
    let toml_val = json_to_toml_value(&value);

    // Parse "section.key" path and set value
    let parts: Vec<&str> = path.split('.').collect();
    match parts.len() {
        1 => {
            table.insert(parts[0].to_string(), toml_val);
        }
        2 => {
            let section = table
                .entry(parts[0].to_string())
                .or_insert_with(|| toml::Value::Table(toml::value::Table::new()));
            if let toml::Value::Table(ref mut t) = section {
                t.insert(parts[1].to_string(), toml_val);
            }
        }
        3 => {
            let section = table
                .entry(parts[0].to_string())
                .or_insert_with(|| toml::Value::Table(toml::value::Table::new()));
            if let toml::Value::Table(ref mut t) = section {
                let sub = t
                    .entry(parts[1].to_string())
                    .or_insert_with(|| toml::Value::Table(toml::value::Table::new()));
                if let toml::Value::Table(ref mut t2) = sub {
                    t2.insert(parts[2].to_string(), toml_val);
                }
            }
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"status": "error", "error": "path too deep (max 3 levels)"}),
                ),
            );
        }
    }

    // Write back
    let toml_string = match toml::to_string_pretty(&table) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(
                    serde_json::json!({"status": "error", "error": format!("serialize failed: {e}")}),
                ),
            );
        }
    };
    if let Err(e) = std::fs::write(&config_path, &toml_string) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"status": "error", "error": format!("write failed: {e}")})),
        );
    }

    // Trigger reload
    let reload_status = match state.kernel.reload_config() {
        Ok(plan) => {
            if plan.restart_required {
                "applied_partial"
            } else {
                "applied"
            }
        }
        Err(_) => "saved_reload_failed",
    };

    state.kernel.audit_log.record(
        "system",
        librefang_runtime::audit::AuditAction::ConfigChange,
        format!("config set: {path}"),
        "completed",
    );

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": reload_status, "path": path})),
    )
}

/// Convert a serde_json::Value to a toml::Value.
pub(crate) fn json_to_toml_value(value: &serde_json::Value) -> toml::Value {
    match value {
        serde_json::Value::String(s) => toml::Value::String(s.clone()),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_u64() {
                toml::Value::Integer(i as i64)
            } else if let Some(i) = n.as_i64() {
                toml::Value::Integer(i)
            } else if let Some(f) = n.as_f64() {
                toml::Value::Float(f)
            } else {
                toml::Value::String(n.to_string())
            }
        }
        serde_json::Value::Bool(b) => toml::Value::Boolean(*b),
        _ => toml::Value::String(value.to_string()),
    }
}
