//! Health, status, configuration, security, and migration handlers.

use super::AppState;

/// Build routes for the config/health/security/migration domain.
pub fn router() -> axum::Router<std::sync::Arc<AppState>> {
    axum::Router::new()
        .route("/metrics", axum::routing::get(prometheus_metrics))
        .route("/health", axum::routing::get(health))
        .route("/health/detail", axum::routing::get(health_detail))
        .route("/status", axum::routing::get(status))
        .route(
            "/dashboard/snapshot",
            axum::routing::get({
                |State(state): State<Arc<AppState>>| async move {
                    axum::Json(dashboard_snapshot_inner(&state).await)
                }
            }),
        )
        .route("/version", axum::routing::get(version))
        .route("/config", axum::routing::get(get_config))
        .route("/config/export", axum::routing::get(export_config))
        .route("/config/schema", axum::routing::get(config_schema))
        .route("/config/set", axum::routing::post(config_set))
        .route("/config/reload", axum::routing::post(config_reload))
        .route("/security", axum::routing::get(security_status))
        .route("/migrate/detect", axum::routing::get(migrate_detect))
        .route("/migrate/scan", axum::routing::post(migrate_scan))
        .route("/migrate", axum::routing::post(run_migrate))
        .route("/shutdown", axum::routing::post(shutdown))
        .route("/init", axum::routing::post(quick_init))
}
use crate::types::*;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::sync::Arc;

/// Best-effort host identifier for the machine running the daemon.
///
/// Exposed only via authenticated endpoints (`/api/status`,
/// `/api/dashboard/snapshot`) — deliberately **not** surfaced on the
/// public `/api/version` endpoint, because hostname is a per-machine
/// identifier that a remote scanner could correlate to a specific
/// deployment target. `$HOSTNAME` is honoured first for parity with
/// containers that synthesise it; `hostname(1)` is the POSIX fallback.
/// Returns `None` only when both fail (rare).
fn system_hostname() -> Option<String> {
    if let Ok(h) = std::env::var("HOSTNAME") {
        let trimmed = h.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    #[cfg(unix)]
    {
        std::process::Command::new("hostname")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }
    #[cfg(windows)]
    {
        std::env::var("COMPUTERNAME")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

/// Best-effort RSS memory probe for the running process, in MB.
///
/// Shared between `/api/status` and `/api/dashboard/snapshot` so both
/// endpoints surface the same number. Returns `None` on platforms where
/// neither `ps` nor `tasklist` is available, or when parsing the output
/// fails — callers should render a placeholder in that case rather than
/// treating `0` as a real reading.
fn current_process_rss_mb() -> Option<u64> {
    #[cfg(unix)]
    {
        std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &std::process::id().to_string()])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse::<u64>().ok())
            .map(|kb| kb / 1024)
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("tasklist")
            .args([
                "/FI",
                &format!("PID eq {}", std::process::id()),
                "/FO",
                "CSV",
                "/NH",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| {
                // tasklist CSV: "name","pid","session","session#","mem usage"
                let fields: Vec<&str> = s.trim().split(',').collect();
                fields
                    .last()
                    .map(|v| {
                        v.trim_matches('"')
                            .replace(" K", "")
                            .replace(",", "")
                            .replace(" ", "")
                    })
                    .and_then(|v| v.parse::<u64>().ok())
                    .map(|kb| kb / 1024)
            })
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

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
        .agent_registry()
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
    let active_agent_count = state
        .kernel
        .agent_registry()
        .list()
        .iter()
        .filter(|e| matches!(e.state, librefang_types::agent::AgentState::Running))
        .count();
    let session_count = state
        .kernel
        .memory_substrate()
        .list_sessions()
        .map(|s| s.len())
        .unwrap_or(0);

    let memory_used_mb = current_process_rss_mb();

    let cfg = state.kernel.config_snapshot();
    Json(serde_json::json!({
        "status": "running",
        "version": env!("CARGO_PKG_VERSION"),
        "agent_count": agent_count,
        "active_agent_count": active_agent_count,
        "session_count": session_count,
        "memory_used_mb": memory_used_mb,
        "default_provider": state.kernel.default_model_override_ref().read().ok().and_then(|g| g.as_ref().map(|dm| dm.provider.clone())).unwrap_or_else(|| cfg.default_model.provider.clone()),
        "default_model": state.kernel.default_model_override_ref().read().ok().and_then(|g| g.as_ref().map(|dm| dm.model.clone())).unwrap_or_else(|| cfg.default_model.model.clone()),
        "uptime_seconds": uptime,
        "api_listen": cfg.api_listen,
        "home_dir": state.kernel.home_dir().display().to_string(),
        "log_level": cfg.log_level,
        "hostname": system_hostname(),
        "network_enabled": cfg.network_enabled,
        "terminal_enabled": cfg.terminal.enabled,
        "config_exists": state.kernel.home_dir().join("config.toml").exists(),
        "agents": agents,
    }))
}

/// POST /api/init — Quick initialization (detect provider, write config, reload).
///
/// Skips if config.toml already exists. Returns the detected provider/model.
#[utoipa::path(
    post,
    path = "/api/init",
    tag = "system",
    responses(
        (status = 200, description = "Quick init result", body = serde_json::Value)
    )
)]
pub async fn quick_init(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let home = state.kernel.home_dir();
    let config_path = home.join("config.toml");

    if config_path.exists() {
        return Json(serde_json::json!({
            "status": "already_initialized",
            "message": "config.toml already exists"
        }));
    }

    // Ensure directories exist
    let _ = std::fs::create_dir_all(home);
    let _ = std::fs::create_dir_all(home.join("data"));

    // Detect best available provider
    let (provider, api_key_env) = if let Some((p, _model, env_var)) =
        librefang_runtime::drivers::detect_available_provider()
    {
        (p.to_string(), env_var.to_string())
    } else {
        ("groq".to_string(), "GROQ_API_KEY".to_string())
    };

    // Resolve default model from catalog
    let model = librefang_runtime::model_catalog::ModelCatalog::default()
        .default_model_for_provider(&provider)
        .unwrap_or_else(|| "auto".to_string());

    // Write minimal config.toml
    let config_content = format!(
        r#"# LibreFang configuration (auto-generated)
# Run `librefang init --upgrade` for full annotated config.

log_level = "info"
api_listen = "127.0.0.1:4545"

[default_model]
provider = "{provider}"
model = "{model}"
api_key_env = "{api_key_env}"
"#
    );

    if let Err(e) = std::fs::write(&config_path, &config_content) {
        return Json(serde_json::json!({
            "status": "error",
            "message": format!("Failed to write config: {e}")
        }));
    }

    // Reload config so kernel picks up new settings
    let _ = state.kernel.reload_config().await;

    Json(serde_json::json!({
        "status": "initialized",
        "provider": provider,
        "model": model,
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
    state.kernel.audit().record(
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
    // Deliberately omitted from the unauthenticated version response:
    // - `hostname` — a per-machine identifier that helps a remote probe
    //   correlate a daemon to a specific deployment target. Operators who
    //   need the hostname should read it from the daemon's shell
    //   environment rather than pulling it over an unauthenticated HTTP
    //   endpoint.
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
        .memory_substrate()
        .structured_get(shared_id, "__health_check__")
        .is_ok();

    let status = if db_ok { "ok" } else { "degraded" };

    let fts_only = state.kernel.config_ref().memory.fts_only.unwrap_or(false);
    let embedding_ok = fts_only || state.kernel.embedding().is_some();

    Json(serde_json::json!({
        "status": status,
        "version": env!("CARGO_PKG_VERSION"),
        "checks": [
            { "name": "database", "status": if db_ok { "ok" } else { "error" } },
            { "name": "embedding", "status": if embedding_ok { "ok" } else { "warn" } },
        ],
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
    let health = state.kernel.supervisor_ref().health();

    let shared_id = librefang_types::agent::AgentId(uuid::Uuid::from_bytes([
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
    ]));
    let db_ok = state
        .kernel
        .memory_substrate()
        .structured_get(shared_id, "__health_check__")
        .is_ok();

    let hcfg = state.kernel.config_ref();
    let config_warnings = hcfg.validate();
    let status = if db_ok { "ok" } else { "degraded" };

    Json(serde_json::json!({
        "status": status,
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_seconds": state.started_at.elapsed().as_secs(),
        "panic_count": health.panic_count,
        "restart_count": health.restart_count,
        "agent_count": state.kernel.agent_registry().count(),
        "database": if db_ok { "connected" } else { "error" },
        "memory": {
            "embedding_available": state.kernel.embedding().is_some(),
            "embedding_provider": hcfg.memory.embedding_provider,
            "embedding_model": &hcfg.memory.embedding_model,
            "proactive_memory_enabled": hcfg.proactive_memory.enabled,
            "extraction_model": &hcfg.proactive_memory.extraction_model,
        },
        "config_warnings": config_warnings,
        "event_bus": {
            "dropped_events": state.kernel.event_bus_ref().dropped_count(),
        },
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
/// - `librefang_tokens` — total tokens consumed (per agent, rolling 1h gauge)
/// - `librefang_tokens_input` — input tokens consumed (per agent, rolling 1h gauge)
/// - `librefang_tokens_output` — output tokens consumed (per agent, rolling 1h gauge)
/// - `librefang_tool_calls` — tool calls made (per agent, rolling 1h gauge)
/// - `librefang_llm_calls` — LLM API calls made (per agent, rolling 1h gauge)
/// - `librefang_panics_total` — supervisor panic count
/// - `librefang_restarts_total` — supervisor restart count
/// - `librefang_active_sessions` — number of active login sessions
/// - `librefang_cost_usd_today` — total estimated cost for today (USD)
/// - `librefang_http_requests_total` — HTTP request counts (with telemetry feature)
/// - `librefang_http_request_duration_seconds` — HTTP request latencies (with telemetry feature)
#[utoipa::path(
    get,
    path = "/api/metrics",
    tag = "system",
    responses(
        (status = 200, description = "Prometheus text-format metrics", body = serde_json::Value)
    )
)]
pub async fn prometheus_metrics(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut out = String::with_capacity(4096);

    // Uptime
    let uptime = state.started_at.elapsed().as_secs();
    out.push_str("# HELP librefang_uptime_seconds Time since daemon started.\n");
    out.push_str("# TYPE librefang_uptime_seconds gauge\n");
    out.push_str(&format!("librefang_uptime_seconds {uptime}\n\n"));

    // Active agents
    let agents = state.kernel.agent_registry().list();
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

    // Per-agent token, tool, and LLM call usage (rolling 1h window — gauges, not counters)
    out.push_str("# HELP librefang_tokens Tokens consumed (rolling 1h window).\n");
    out.push_str("# TYPE librefang_tokens gauge\n");
    out.push_str("# HELP librefang_tokens_input Input tokens consumed (rolling 1h window).\n");
    out.push_str("# TYPE librefang_tokens_input gauge\n");
    out.push_str("# HELP librefang_tokens_output Output tokens consumed (rolling 1h window).\n");
    out.push_str("# TYPE librefang_tokens_output gauge\n");
    out.push_str("# HELP librefang_tool_calls Tool calls made (rolling 1h window).\n");
    out.push_str("# TYPE librefang_tool_calls gauge\n");
    out.push_str("# HELP librefang_llm_calls LLM API calls made (rolling 1h window).\n");
    out.push_str("# TYPE librefang_llm_calls gauge\n");
    for agent in &agents {
        let name = &agent.name;
        let provider = &agent.manifest.model.provider;
        let model = &agent.manifest.model.model;
        if let Some(snap) = state.kernel.scheduler_ref().get_usage(agent.id) {
            let labels = format!("agent=\"{name}\",provider=\"{provider}\",model=\"{model}\"");
            out.push_str(&format!(
                "librefang_tokens{{{labels}}} {}\n",
                snap.total_tokens
            ));
            out.push_str(&format!(
                "librefang_tokens_input{{{labels}}} {}\n",
                snap.input_tokens
            ));
            out.push_str(&format!(
                "librefang_tokens_output{{{labels}}} {}\n",
                snap.output_tokens
            ));
            out.push_str(&format!(
                "librefang_tool_calls{{{labels}}} {}\n",
                snap.tool_calls
            ));
            out.push_str(&format!(
                "librefang_llm_calls{{{labels}}} {}\n",
                snap.llm_calls
            ));
        }
    }
    out.push('\n');

    // Supervisor health
    let health = state.kernel.supervisor_ref().health();
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
        "librefang_info{{version=\"{}\"}} 1\n\n",
        env!("CARGO_PKG_VERSION")
    ));

    // Active sessions
    let session_count = state.active_sessions.read().await.len();
    out.push_str("# HELP librefang_active_sessions Number of active login sessions.\n");
    out.push_str("# TYPE librefang_active_sessions gauge\n");
    out.push_str(&format!("librefang_active_sessions {session_count}\n\n"));

    // Today's estimated cost (from metering SQLite)
    let today_cost = state
        .kernel
        .memory_substrate()
        .usage()
        .query_today_cost()
        .unwrap_or(0.0);
    out.push_str("# HELP librefang_cost_usd_today Estimated total cost for today (USD).\n");
    out.push_str("# TYPE librefang_cost_usd_today gauge\n");
    out.push_str(&format!("librefang_cost_usd_today {today_cost:.6}\n"));

    // Append metrics from the Prometheus recorder when the telemetry feature is
    // enabled and the recorder has been initialized. This merges the hand-crafted
    // LibreFang metrics above with standard `metrics` crate counters/histograms
    // (e.g. HTTP request metrics from the telemetry middleware).
    #[cfg(feature = "telemetry")]
    if let Some(handle) = &state.prometheus_handle {
        out.push_str("# --- metrics-exporter-prometheus output ---\n");
        out.push_str(&handle.render());
    }

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
    let config = state.kernel.config_ref();

    // -- channels: show which platforms are configured (instance counts), no tokens --
    let channels = {
        let c = &config.channels;
        let mut map = serde_json::Map::new();
        macro_rules! ch {
            ($name:ident) => {
                if !c.$name.is_empty() {
                    map.insert(
                        stringify!($name).to_string(),
                        serde_json::json!({ "instances": c.$name.len() }),
                    );
                }
            };
        }
        ch!(telegram);
        ch!(discord);
        ch!(slack);
        ch!(whatsapp);
        ch!(signal);
        ch!(matrix);
        ch!(email);
        ch!(teams);
        ch!(mattermost);
        ch!(irc);
        ch!(google_chat);
        ch!(twitch);
        ch!(rocketchat);
        ch!(zulip);
        ch!(xmpp);
        ch!(line);
        ch!(viber);
        ch!(messenger);
        ch!(reddit);
        ch!(mastodon);
        ch!(bluesky);
        ch!(feishu);
        ch!(revolt);
        ch!(nextcloud);
        ch!(guilded);
        ch!(keybase);
        ch!(threema);
        ch!(nostr);
        ch!(webex);
        ch!(pumble);
        ch!(flock);
        ch!(twist);
        ch!(mumble);
        ch!(dingtalk);
        ch!(qq);
        ch!(discourse);
        ch!(gitter);
        ch!(ntfy);
        ch!(gotify);
        ch!(webhook);
        ch!(linkedin);
        ch!(wecom);
        serde_json::Value::Object(map)
    };

    // -- mcp_servers: list names/commands, redact env secrets --
    let mcp_servers: Vec<serde_json::Value> = config
        .mcp_servers
        .iter()
        .map(|s| {
            let transport_summary = match &s.transport {
                Some(librefang_types::config::McpTransportEntry::Stdio { command, args }) => {
                    serde_json::json!({ "type": "stdio", "command": command, "args": args })
                }
                Some(librefang_types::config::McpTransportEntry::Sse { url }) => {
                    serde_json::json!({ "type": "sse", "url": url })
                }
                Some(librefang_types::config::McpTransportEntry::Http { url }) => {
                    serde_json::json!({ "type": "http", "url": url })
                }
                Some(librefang_types::config::McpTransportEntry::HttpCompat {
                    base_url, ..
                }) => {
                    serde_json::json!({ "type": "http_compat", "base_url": base_url })
                }
                None => serde_json::json!({ "type": "none" }),
            };
            serde_json::json!({
                "name": s.name,
                "transport": transport_summary,
                "timeout_secs": s.timeout_secs,
                "env_count": s.env.len(),
            })
        })
        .collect();

    // -- fallback_providers --
    let fallback_providers: Vec<serde_json::Value> = config
        .fallback_providers
        .iter()
        .map(|f| {
            serde_json::json!({
                "provider": f.provider,
                "model": f.model,
                "api_key_env": f.api_key_env,
                "base_url": f.base_url,
            })
        })
        .collect();

    // -- bindings --
    let bindings: Vec<serde_json::Value> = config
        .bindings
        .iter()
        .map(|b| {
            serde_json::json!({
                "agent": b.agent,
                "match_rule": {
                    "channel": b.match_rule.channel,
                    "account_id": b.match_rule.account_id,
                    "peer_id": b.match_rule.peer_id,
                    "guild_id": b.match_rule.guild_id,
                    "roles": b.match_rule.roles,
                },
            })
        })
        .collect();

    // -- auth_profiles: provider names only, not keys --
    let auth_profiles: serde_json::Value = config
        .auth_profiles
        .iter()
        .map(|(provider, profiles)| {
            let names: Vec<&str> = profiles.iter().map(|p| p.name.as_str()).collect();
            (provider.clone(), serde_json::json!(names))
        })
        .collect::<serde_json::Map<String, serde_json::Value>>()
        .into();

    // -- provider_api_keys: env var names only, not actual keys --
    let provider_api_keys: serde_json::Value = config
        .provider_api_keys
        .iter()
        .map(|(provider, env_var)| (provider.clone(), serde_json::json!(env_var)))
        .collect::<serde_json::Map<String, serde_json::Value>>()
        .into();

    // -- sidecar_channels: show names/commands, redact env values --
    let sidecar_channels: Vec<serde_json::Value> = config
        .sidecar_channels
        .iter()
        .map(|sc| {
            serde_json::json!({
                "name": sc.name,
                "command": sc.command,
                "args": sc.args,
                "channel_type": sc.channel_type,
                "env_keys": sc.env.keys().collect::<Vec<_>>(),
            })
        })
        .collect();

    // -- external_auth: redact secrets --
    let external_auth_providers: Vec<serde_json::Value> = config
        .external_auth
        .providers
        .iter()
        .map(|p| {
            serde_json::json!({
                "id": p.id,
                "display_name": p.display_name,
                "issuer_url": p.issuer_url,
                "client_id": p.client_id,
                "client_secret_env": p.client_secret_env,
                "redirect_url": p.redirect_url,
                "scopes": p.scopes,
                "allowed_domains": p.allowed_domains,
            })
        })
        .collect();

    let mut out = serde_json::Map::new();
    macro_rules! set {
        ($k:expr, $($json:tt)+) => { out.insert($k.into(), serde_json::json!($($json)+)); };
    }

    // ── General ──
    set!("home_dir", config.home_dir.to_string_lossy());
    set!("data_dir", config.data_dir.to_string_lossy());
    set!("log_level", config.log_level);
    set!("api_listen", config.api_listen);
    set!(
        "api_key",
        if config.api_key.is_empty() {
            "not set"
        } else {
            "***"
        }
    );
    set!("network_enabled", config.network_enabled);
    set!("mode", format!("{:?}", config.mode));
    set!("language", config.language);
    set!(
        "usage_footer",
        serde_json::to_value(config.usage_footer).unwrap_or_default()
    );
    set!("stable_prefix_mode", config.stable_prefix_mode);
    set!("prompt_caching", config.prompt_caching);
    set!("max_cron_jobs", config.max_cron_jobs);
    set!("include", config.include);
    set!(
        "workspaces_dir",
        config
            .effective_workspaces_dir()
            .to_string_lossy()
            .to_string()
    );
    // ── Default Model ──
    set!("default_model", {
        "provider": config.default_model.provider,
        "model": config.default_model.model,
        "api_key_env": config.default_model.api_key_env,
        "base_url": config.default_model.base_url,
    });

    // ── Memory ──
    set!("memory", {
        "sqlite_path": config.memory.sqlite_path.as_ref().map(|p| p.to_string_lossy().to_string()),
        "embedding_model": config.memory.embedding_model,
        "consolidation_threshold": config.memory.consolidation_threshold,
        "decay_rate": config.memory.decay_rate,
        "embedding_provider": config.memory.embedding_provider,
        "embedding_api_key_env": config.memory.embedding_api_key_env,
        "consolidation_interval_hours": config.memory.consolidation_interval_hours,
    });

    // ── Proactive Memory ──
    set!("proactive_memory", {
        "enabled": config.proactive_memory.enabled,
        "auto_memorize": config.proactive_memory.auto_memorize,
        "auto_retrieve": config.proactive_memory.auto_retrieve,
        "max_retrieve": config.proactive_memory.max_retrieve,
        "extraction_threshold": config.proactive_memory.extraction_threshold,
        "extraction_model": config.proactive_memory.extraction_model,
        "extract_categories": config.proactive_memory.extract_categories,
        "session_ttl_hours": config.proactive_memory.session_ttl_hours,
        "confidence_decay_rate": config.proactive_memory.confidence_decay_rate,
        "duplicate_threshold": config.proactive_memory.duplicate_threshold,
        "max_memories_per_agent": config.proactive_memory.max_memories_per_agent,
    });

    // ── Auto-Dream (background memory consolidation) ──
    set!("auto_dream", {
        "enabled": config.auto_dream.enabled,
        "min_hours": config.auto_dream.min_hours,
        "min_sessions": config.auto_dream.min_sessions,
        "check_interval_secs": config.auto_dream.check_interval_secs,
        "timeout_secs": config.auto_dream.timeout_secs,
        "lock_dir": config.auto_dream.lock_dir,
    });

    // ── Network (redact shared_secret) ──
    set!("network", {
        "listen_addresses": config.network.listen_addresses,
        "bootstrap_peers": config.network.bootstrap_peers,
        "mdns_enabled": config.network.mdns_enabled,
        "max_peers": config.network.max_peers,
        "shared_secret": if config.network.shared_secret.is_empty() { "not set" } else { "***" },
    });

    set!("channels", channels);

    // ── Users (count only, don't expose passwords) ──
    set!("users", {
        "count": config.users.len(),
        "names": config.users.iter().map(|u| u.name.as_str()).collect::<Vec<_>>(),
    });

    set!("mcp_servers", mcp_servers);

    // ── A2A ──
    out.insert(
        "a2a".into(),
        match &config.a2a {
            Some(a2a) => serde_json::json!({
                "enabled": a2a.enabled,
                "listen_path": a2a.listen_path,
                "external_agents": a2a.external_agents.iter().map(|ea| {
                    serde_json::json!({ "name": ea.name, "url": ea.url })
                }).collect::<Vec<_>>(),
            }),
            None => serde_json::json!(null),
        },
    );

    // ── Web ──
    // Check if at least one search provider has a configured API key
    let search_available = [
        &config.web.tavily.api_key_env,
        &config.web.brave.api_key_env,
        &config.web.jina.api_key_env,
        &config.web.perplexity.api_key_env,
    ]
    .iter()
    .any(|env_var| {
        std::env::var(env_var)
            .ok()
            .filter(|v| !v.is_empty())
            .is_some()
    });
    set!("web", {
        "search_provider": format!("{:?}", config.web.search_provider),
        "cache_ttl_minutes": config.web.cache_ttl_minutes,
        "search_available": search_available,
    });
    // Web subsections built separately to avoid recursion limit
    if let Some(web) = out.get_mut("web").and_then(|v| v.as_object_mut()) {
        web.insert(
            "brave".into(),
            serde_json::json!({
                "api_key_env": config.web.brave.api_key_env,
                "max_results": config.web.brave.max_results,
                "country": config.web.brave.country,
                "search_lang": config.web.brave.search_lang,
                "freshness": config.web.brave.freshness,
            }),
        );
        web.insert(
            "tavily".into(),
            serde_json::json!({
                "api_key_env": config.web.tavily.api_key_env,
                "search_depth": config.web.tavily.search_depth,
                "max_results": config.web.tavily.max_results,
                "include_answer": config.web.tavily.include_answer,
            }),
        );
        web.insert(
            "perplexity".into(),
            serde_json::json!({
                "api_key_env": config.web.perplexity.api_key_env,
                "model": config.web.perplexity.model,
            }),
        );
        web.insert(
            "fetch".into(),
            serde_json::json!({
                "max_chars": config.web.fetch.max_chars,
                "max_response_bytes": config.web.fetch.max_response_bytes,
                "timeout_secs": config.web.fetch.timeout_secs,
                "readability": config.web.fetch.readability,
            }),
        );
    }

    set!("fallback_providers", fallback_providers);

    set!("browser", {
        "headless": config.browser.headless,
        "viewport_width": config.browser.viewport_width,
        "viewport_height": config.browser.viewport_height,
        "timeout_secs": config.browser.timeout_secs,
        "idle_timeout_secs": config.browser.idle_timeout_secs,
        "max_sessions": config.browser.max_sessions,
        "chromium_path": config.browser.chromium_path,
    });

    set!("extensions", {
        "auto_reconnect": config.extensions.auto_reconnect,
        "reconnect_max_attempts": config.extensions.reconnect_max_attempts,
        "reconnect_max_backoff_secs": config.extensions.reconnect_max_backoff_secs,
        "health_check_interval_secs": config.extensions.health_check_interval_secs,
    });

    set!("vault", {
        "enabled": config.vault.enabled,
        "path": config.vault.path.as_ref().map(|p| p.to_string_lossy().to_string()),
    });

    let stt_available = config.media.audio_provider.is_some();
    set!("media", {
        "image_description": config.media.image_description,
        "audio_transcription": config.media.audio_transcription,
        "video_description": config.media.video_description,
        "max_concurrency": config.media.max_concurrency,
        "image_provider": config.media.image_provider,
        "audio_provider": config.media.audio_provider,
        "audio_model": config.media.audio_model,
        "stt_available": stt_available,
    });

    set!("links", {
        "enabled": config.links.enabled,
        "max_links": config.links.max_links,
        "max_content_bytes": config.links.max_content_bytes,
        "timeout_secs": config.links.timeout_secs,
    });

    set!("reload", {
        "mode": format!("{:?}", config.reload.mode),
        "debounce_ms": config.reload.debounce_ms,
    });

    out.insert(
        "webhook_triggers".into(),
        match &config.webhook_triggers {
            Some(wh) => serde_json::json!({
                "enabled": wh.enabled,
                "token_env": wh.token_env,
                "max_payload_bytes": wh.max_payload_bytes,
                "rate_limit_per_minute": wh.rate_limit_per_minute,
            }),
            None => serde_json::json!(null),
        },
    );

    set!("approval", {
        "require_approval": config.approval.require_approval,
        "timeout_secs": config.approval.timeout_secs,
        "auto_approve_autonomous": config.approval.auto_approve_autonomous,
        "auto_approve": config.approval.auto_approve,
        "second_factor": serde_json::to_value(config.approval.second_factor).unwrap_or(serde_json::json!("none")),
        "totp_issuer": config.approval.totp_issuer,
    });

    set!("exec_policy", {
        "mode": format!("{:?}", config.exec_policy.mode),
        "safe_bins": config.exec_policy.safe_bins,
        "allowed_commands": config.exec_policy.allowed_commands,
        "timeout_secs": config.exec_policy.timeout_secs,
        "max_output_bytes": config.exec_policy.max_output_bytes,
        "no_output_timeout_secs": config.exec_policy.no_output_timeout_secs,
    });

    set!("bindings", bindings);

    set!("broadcast", {
        "strategy": format!("{:?}", config.broadcast.strategy),
        "routes": config.broadcast.routes,
    });

    set!("auto_reply", {
        "enabled": config.auto_reply.enabled,
        "max_concurrent": config.auto_reply.max_concurrent,
        "timeout_secs": config.auto_reply.timeout_secs,
        "suppress_patterns": config.auto_reply.suppress_patterns,
    });

    set!("canvas", {
        "enabled": config.canvas.enabled,
        "max_html_bytes": config.canvas.max_html_bytes,
        "allowed_tags": config.canvas.allowed_tags,
    });

    // ── TTS ──
    set!("tts", {
        "enabled": config.tts.enabled,
        "provider": config.tts.provider,
        "max_text_length": config.tts.max_text_length,
        "timeout_secs": config.tts.timeout_secs,
    });
    if let Some(tts) = out.get_mut("tts").and_then(|v| v.as_object_mut()) {
        tts.insert(
            "openai".into(),
            serde_json::json!({
                "voice": config.tts.openai.voice,
                "model": config.tts.openai.model,
                "format": config.tts.openai.format,
                "speed": config.tts.openai.speed,
            }),
        );
        tts.insert(
            "elevenlabs".into(),
            serde_json::json!({
                "voice_id": config.tts.elevenlabs.voice_id,
                "model_id": config.tts.elevenlabs.model_id,
                "stability": config.tts.elevenlabs.stability,
                "similarity_boost": config.tts.elevenlabs.similarity_boost,
            }),
        );
        tts.insert(
            "google".into(),
            serde_json::json!({
                "voice": config.tts.google.voice,
                "language_code": config.tts.google.language_code,
                "speaking_rate": config.tts.google.speaking_rate,
                "pitch": config.tts.google.pitch,
                "format": config.tts.google.format,
            }),
        );
    }

    // ── Docker Sandbox ──
    set!("docker", {
        "enabled": config.docker.enabled,
        "image": config.docker.image,
        "container_prefix": config.docker.container_prefix,
        "workdir": config.docker.workdir,
        "network": config.docker.network,
        "memory_limit": config.docker.memory_limit,
        "cpu_limit": config.docker.cpu_limit,
        "timeout_secs": config.docker.timeout_secs,
        "read_only_root": config.docker.read_only_root,
    });
    if let Some(docker) = out.get_mut("docker").and_then(|v| v.as_object_mut()) {
        docker.insert("cap_add".into(), serde_json::json!(config.docker.cap_add));
        docker.insert("tmpfs".into(), serde_json::json!(config.docker.tmpfs));
        docker.insert(
            "pids_limit".into(),
            serde_json::json!(config.docker.pids_limit),
        );
        docker.insert(
            "mode".into(),
            serde_json::json!(format!("{:?}", config.docker.mode)),
        );
        docker.insert(
            "scope".into(),
            serde_json::json!(format!("{:?}", config.docker.scope)),
        );
        docker.insert(
            "reuse_cool_secs".into(),
            serde_json::json!(config.docker.reuse_cool_secs),
        );
        docker.insert(
            "idle_timeout_secs".into(),
            serde_json::json!(config.docker.idle_timeout_secs),
        );
        docker.insert(
            "max_age_secs".into(),
            serde_json::json!(config.docker.max_age_secs),
        );
        docker.insert(
            "blocked_mounts".into(),
            serde_json::json!(config.docker.blocked_mounts),
        );
    }

    set!("pairing", {
        "enabled": config.pairing.enabled,
        "max_devices": config.pairing.max_devices,
        "token_expiry_secs": config.pairing.token_expiry_secs,
        "push_provider": config.pairing.push_provider,
        "ntfy_url": config.pairing.ntfy_url,
        "ntfy_topic": config.pairing.ntfy_topic,
    });

    set!("auth_profiles", auth_profiles);

    out.insert(
        "thinking".into(),
        match &config.thinking {
            Some(t) => serde_json::json!({
                "budget_tokens": t.budget_tokens,
                "stream_thinking": t.stream_thinking,
            }),
            None => serde_json::json!(null),
        },
    );

    {
        let budget = state.kernel.budget_config();
        set!("budget", {
            "max_hourly_usd": budget.max_hourly_usd,
            "max_daily_usd": budget.max_daily_usd,
            "max_monthly_usd": budget.max_monthly_usd,
            "alert_threshold": budget.alert_threshold,
            "default_max_llm_tokens_per_hour": budget.default_max_llm_tokens_per_hour,
        });
    }

    set!("provider_urls", config.provider_urls);
    set!("provider_proxy_urls", config.provider_proxy_urls);
    set!("provider_api_keys", provider_api_keys);
    set!("provider_regions", config.provider_regions);

    set!("vertex_ai", {
        "project_id": config.vertex_ai.project_id,
        "region": config.vertex_ai.region,
        "credentials_path": if config.vertex_ai.credentials_path.is_some() { "***" } else { "not set" },
    });

    set!("oauth", {
        "google_client_id": config.oauth.google_client_id.as_ref().map(|_| "***"),
        "github_client_id": config.oauth.github_client_id.as_ref().map(|_| "***"),
        "microsoft_client_id": config.oauth.microsoft_client_id.as_ref().map(|_| "***"),
        "slack_client_id": config.oauth.slack_client_id.as_ref().map(|_| "***"),
    });

    set!("sidecar_channels", sidecar_channels);

    set!("session", {
        "retention_days": config.session.retention_days,
        "max_sessions_per_agent": config.session.max_sessions_per_agent,
        "cleanup_interval_hours": config.session.cleanup_interval_hours,
    });

    set!("queue", {
        "max_depth_per_agent": config.queue.max_depth_per_agent,
        "max_depth_global": config.queue.max_depth_global,
        "task_ttl_secs": config.queue.task_ttl_secs,
    });
    if let Some(queue) = out.get_mut("queue").and_then(|v| v.as_object_mut()) {
        queue.insert(
            "concurrency".into(),
            serde_json::json!({
                "main_lane": config.queue.concurrency.main_lane,
                "cron_lane": config.queue.concurrency.cron_lane,
                "subagent_lane": config.queue.concurrency.subagent_lane,
            }),
        );
    }

    set!("external_auth", {
        "enabled": config.external_auth.enabled,
        "issuer_url": config.external_auth.issuer_url,
        "client_id": config.external_auth.client_id,
        "client_secret_env": config.external_auth.client_secret_env,
        "redirect_url": config.external_auth.redirect_url,
    });
    if let Some(ea) = out.get_mut("external_auth").and_then(|v| v.as_object_mut()) {
        ea.insert(
            "scopes".into(),
            serde_json::json!(config.external_auth.scopes),
        );
        ea.insert(
            "allowed_domains".into(),
            serde_json::json!(config.external_auth.allowed_domains),
        );
        ea.insert(
            "audience".into(),
            serde_json::json!(config.external_auth.audience),
        );
        ea.insert(
            "session_ttl_secs".into(),
            serde_json::json!(config.external_auth.session_ttl_secs),
        );
        ea.insert(
            "providers".into(),
            serde_json::json!(external_auth_providers),
        );
    }

    Json(serde_json::Value::Object(out))
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
    let scfg = state.kernel.config_ref();
    let api_key_empty = scfg.api_key.is_empty();
    drop(scfg);
    let auth_mode = if api_key_empty {
        "localhost_only"
    } else {
        "bearer_token"
    };

    let audit_count = state.kernel.audit().len();

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
                "api_key_set": !api_key_empty
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
    // Check OpenClaw first
    if let Some(path) = librefang_migrate::openclaw::detect_openclaw_home() {
        let scan = librefang_migrate::openclaw::scan_openclaw_workspace(&path);
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "detected": true,
                "source": "openclaw",
                "path": path.display().to_string(),
                "scan": scan,
            })),
        );
    }

    // Check OpenFang
    if let Some(home) = dirs::home_dir() {
        let openfang_path = home.join(".openfang");
        if openfang_path.exists() && openfang_path.is_dir() {
            return (
                StatusCode::OK,
                Json(serde_json::json!({
                    "detected": true,
                    "source": "openfang",
                    "path": openfang_path.display().to_string(),
                    "scan": null,
                })),
            );
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "detected": false,
            "source": null,
            "path": null,
            "scan": null,
        })),
    )
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
        return ApiErrorResponse::bad_request("Directory not found").into_json_tuple();
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
pub async fn run_migrate(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MigrateRequest>,
) -> impl IntoResponse {
    let source = match req.source.as_str() {
        "openclaw" => librefang_migrate::MigrateSource::OpenClaw,
        "langchain" => librefang_migrate::MigrateSource::LangChain,
        "autogpt" => librefang_migrate::MigrateSource::AutoGpt,
        "openfang" => librefang_migrate::MigrateSource::OpenFang,
        other => {
            return ApiErrorResponse::bad_request(format!(
                "Unknown source: {other}. Use 'openclaw', 'openfang', 'langchain', or 'autogpt'"
            ))
            .into_json_tuple();
        }
    };

    let target_dir = if req.target_dir.trim().is_empty() {
        state.kernel.home_dir().to_path_buf()
    } else {
        std::path::PathBuf::from(req.target_dir.trim())
    };

    let options = librefang_migrate::MigrateOptions {
        source,
        source_dir: std::path::PathBuf::from(req.source_dir.trim()),
        target_dir,
        dry_run: req.dry_run,
    };

    match librefang_migrate::run_migration(&options) {
        Ok(report) => {
            // Migrate writes agent manifests under `<target>/agents/<name>/`
            // (legacy schema). Relocate them into the canonical
            // `workspaces/agents/<name>/` layout immediately so the running
            // daemon can use them without a restart.
            if !req.dry_run {
                state.kernel.relocate_legacy_agent_dirs();
            }

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
        Err(e) => ApiErrorResponse::internal(format!("Migration failed: {e}")).into_json_tuple(),
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
    state.kernel.audit().record(
        "system",
        librefang_runtime::audit::AuditAction::ConfigChange,
        "config reload requested via API",
        "pending",
    );
    match state.kernel.reload_config().await {
        Ok(plan) => {
            // If channel config changed, the kernel already cleared the adapter
            // registry — but we also need to stop the old BridgeManager and
            // restart adapters from the new config.
            if plan
                .hot_actions
                .contains(&librefang_kernel::config_reload::HotAction::ReloadChannels)
            {
                match crate::channel_bridge::reload_channels_from_disk(&state).await {
                    Ok(names) => {
                        tracing::info!(
                            "Hot-reload: restarted channel bridge with {} adapter(s): {:?}",
                            names.len(),
                            names,
                        );
                    }
                    Err(e) => {
                        tracing::error!("Hot-reload: failed to restart channel bridge: {e}");
                    }
                }
            }

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
// Config Export endpoint
// ---------------------------------------------------------------------------

/// GET /api/config/export — Download config.toml as a file attachment.
///
/// Reads the raw config.toml from disk. If the file does not exist, falls back
/// to serializing the in-memory config so a download is always available.
#[utoipa::path(
    get,
    path = "/api/config/export",
    tag = "system",
    responses(
        (status = 200, description = "config.toml file download", content_type = "application/toml")
    )
)]
pub async fn export_config(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    use axum::body::Body;

    let config_path = state.kernel.home_dir().join("config.toml");

    let toml_content = if config_path.exists() {
        match std::fs::read_to_string(&config_path) {
            Ok(content) => content,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    [(axum::http::header::CONTENT_TYPE, "application/json")],
                    Body::from(
                        serde_json::json!({"status": "error", "error": format!("failed to read config: {e}")})
                            .to_string(),
                    ),
                )
                    .into_response();
            }
        }
    } else {
        // Fall back to serializing in-memory config
        match toml::to_string_pretty(&**state.kernel.config_ref()) {
            Ok(s) => s,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    [(axum::http::header::CONTENT_TYPE, "application/json")],
                    Body::from(
                        serde_json::json!({"status": "error", "error": format!("failed to serialize config: {e}")})
                            .to_string(),
                    ),
                )
                    .into_response();
            }
        }
    };

    (
        StatusCode::OK,
        [
            (axum::http::header::CONTENT_TYPE, "application/toml"),
            (
                axum::http::header::CONTENT_DISPOSITION,
                "attachment; filename=\"librefang-config.toml\"",
            ),
        ],
        Body::from(toml_content),
    )
        .into_response()
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
        .model_catalog_ref()
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

    let mut sections = serde_json::Map::new();
    macro_rules! sec {
        ($k:expr, $($json:tt)+) => { sections.insert($k.into(), serde_json::json!($($json)+)); };
    }

    sec!("general", {
        "root_level": true,
        "fields": {
            "api_listen": "string",
            "api_key": "string",
            "log_level": { "type": "select", "options": ["trace", "debug", "info", "warn", "error"] },
            "network_enabled": "boolean",
            "mode": { "type": "select", "options": ["stable", "default", "dev"] },
            "language": { "type": "select", "options": [
                {"value": "en", "label": "English"},
                {"value": "zh", "label": "中文"},
                {"value": "ja", "label": "日本語"},
                {"value": "ko", "label": "한국어"},
                {"value": "es", "label": "Español"},
                {"value": "fr", "label": "Français"},
                {"value": "de", "label": "Deutsch"},
                {"value": "it", "label": "Italiano"},
                {"value": "pt", "label": "Português"},
                {"value": "ru", "label": "Русский"},
                {"value": "ar", "label": "العربية"},
                {"value": "hi", "label": "हिन्दी"},
                {"value": "tr", "label": "Türkçe"},
                {"value": "pl", "label": "Polski"},
                {"value": "nl", "label": "Nederlands"},
                {"value": "vi", "label": "Tiếng Việt"},
                {"value": "th", "label": "ภาษาไทย"},
                {"value": "id", "label": "Bahasa Indonesia"}
            ] },
            "usage_footer": { "type": "select", "options": ["off", "tokens", "cost", "full"] },
            "stable_prefix_mode": "boolean",
            "prompt_caching": "boolean",
            "max_cron_jobs": { "type": "number", "min": 0, "max": 100, "step": 1 },
            "workspaces_dir": "string"
        }
    });
    sec!("default_model", {
        "hot_reloadable": true,
        "fields": {
            "provider": { "type": "select", "options": provider_options },
            "model": { "type": "select", "options": model_options },
            "api_key_env": "string",
            "base_url": "string"
        }
    });
    sec!("memory", { "fields": {
        "sqlite_path": "string", "embedding_model": "string",
        "consolidation_threshold": { "type": "number", "min": 1, "max": 1000000, "step": 1 },
        "decay_rate": { "type": "number", "min": 0, "max": 1, "step": 0.01 },
        "embedding_provider": { "type": "select", "options": ["auto", "openai", "openrouter", "groq", "mistral", "together", "fireworks", "cohere", "ollama", "bedrock", "vllm", "lmstudio"] },
        "embedding_api_key_env": "string",
        "consolidation_interval_hours": { "type": "number_select", "options": ["0", "1", "6", "12", "24", "48", "168"] }
    }});
    sec!("proactive_memory", { "fields": {
        "enabled": "boolean", "auto_memorize": "boolean", "auto_retrieve": "boolean",
        "max_retrieve": { "type": "number", "min": 1, "max": 100, "step": 1 },
        "extraction_threshold": { "type": "number", "min": 0, "max": 1, "step": 0.01 },
        "extraction_model": "string", "extract_categories": "array",
        "session_ttl_hours": { "type": "number", "min": 1, "max": 8760, "step": 1 },
        "confidence_decay_rate": { "type": "number", "min": 0, "max": 1, "step": 0.001 },
        "duplicate_threshold": { "type": "number", "min": 0, "max": 1, "step": 0.01 },
        "max_memories_per_agent": { "type": "number", "min": 0, "max": 100000, "step": 100 }
    }});
    sec!("auto_dream", { "fields": {
        "enabled": "boolean",
        "min_hours": { "type": "number", "min": 0, "max": 168, "step": 0.5 },
        "min_sessions": { "type": "number", "min": 0, "max": 1000, "step": 1 },
        "check_interval_secs": { "type": "number", "min": 60, "max": 86400, "step": 60 },
        "timeout_secs": { "type": "number", "min": 30, "max": 3600, "step": 30 },
        "lock_dir": "string"
    }});
    sec!("web", { "fields": {
        "search_provider": { "type": "select", "options": ["brave", "tavily", "perplexity", "duck_duck_go", "auto"] },
        "cache_ttl_minutes": { "type": "number", "min": 0, "max": 10080, "step": 1 }
    }});
    sec!("browser", { "fields": {
        "headless": "boolean",
        "viewport_width": { "type": "number", "min": 320, "max": 3840, "step": 1 },
        "viewport_height": { "type": "number", "min": 240, "max": 2160, "step": 1 },
        "timeout_secs": { "type": "number", "min": 5, "max": 300, "step": 1 },
        "idle_timeout_secs": { "type": "number", "min": 0, "max": 3600, "step": 1 },
        "max_sessions": { "type": "number", "min": 1, "max": 20, "step": 1 },
        "chromium_path": "string"
    }});
    sec!("network", { "fields": {
        "listen_addresses": "string[]", "bootstrap_peers": "string[]",
        "mdns_enabled": "boolean",
        "max_peers": { "type": "number", "min": 1, "max": 1000, "step": 1 },
        "shared_secret": "string"
    }});
    sec!("extensions", { "fields": {
        "auto_reconnect": "boolean",
        "reconnect_max_attempts": { "type": "number", "min": 0, "max": 100, "step": 1 },
        "reconnect_max_backoff_secs": { "type": "number", "min": 1, "max": 3600, "step": 1 },
        "health_check_interval_secs": { "type": "number", "min": 5, "max": 3600, "step": 1 }
    }});
    sec!("vault", { "fields": { "enabled": "boolean", "path": "string" }});
    sec!("a2a", { "fields": { "enabled": "boolean", "listen_path": "string" }});
    sec!("channels", { "fields": {
        "telegram": "object", "discord": "object", "slack": "object", "whatsapp": "object",
        "signal": "object", "matrix": "object", "email": "object", "teams": "object",
        "mattermost": "object", "irc": "object", "google_chat": "object"
    }});
    sec!("media", { "fields": {
        "image_description": "boolean", "audio_transcription": "boolean",
        "video_description": "boolean",
        "max_concurrency": { "type": "number", "min": 1, "max": 20, "step": 1 },
        "image_provider": { "type": "select", "options": ["", "anthropic", "openai", "gemini"] },
        "audio_provider": { "type": "select", "options": ["", "groq", "openai", "gemini", "elevenlabs", "minimax", "fireworks", "together", "siliconflow"] },
        "audio_model": "string"
    }});
    sec!("links", { "fields": {
        "enabled": "boolean",
        "max_links": { "type": "number", "min": 1, "max": 100, "step": 1 },
        "max_content_bytes": { "type": "number", "min": 1024, "max": 10485760, "step": 1024 },
        "timeout_secs": { "type": "number", "min": 5, "max": 120, "step": 1 }
    }});
    sec!("reload", { "hot_reloadable": true, "fields": {
        "mode": { "type": "select", "options": ["off", "restart", "hot", "hybrid"] },
        "debounce_ms": { "type": "number", "min": 100, "max": 10000, "step": 100 }
    }});
    sec!("webhook_triggers", { "fields": {
        "enabled": "boolean", "token_env": "string",
        "max_payload_bytes": { "type": "number", "min": 1024, "max": 10485760, "step": 1024 },
        "rate_limit_per_minute": { "type": "number", "min": 1, "max": 10000, "step": 1 }
    }});
    sec!("approval", { "hot_reloadable": true, "fields": {
        "require_approval": "string[]",
        "timeout_secs": { "type": "number", "min": 30, "max": 86400, "step": 1 },
        "auto_approve_autonomous": "boolean", "auto_approve": "boolean",
        "second_factor": { "type": "select", "options": ["none", "totp", "login", "both"] },
        "totp_issuer": "string"
    }});
    sec!("exec_policy", { "fields": {
        "mode": { "type": "select", "options": ["deny", "allowlist", "full"] },
        "safe_bins": "string[]", "allowed_commands": "string[]",
        "timeout_secs": { "type": "number", "min": 1, "max": 3600, "step": 1 },
        "max_output_bytes": { "type": "number", "min": 1024, "max": 104857600, "step": 1024 },
        "no_output_timeout_secs": { "type": "number", "min": 1, "max": 3600, "step": 1 }
    }});
    sec!("broadcast", { "fields": {
        "strategy": { "type": "select", "options": ["parallel", "sequential"] },
        "routes": "object"
    }});
    sec!("auto_reply", { "fields": {
        "enabled": "boolean",
        "max_concurrent": { "type": "number", "min": 1, "max": 100, "step": 1 },
        "timeout_secs": { "type": "number", "min": 1, "max": 3600, "step": 1 },
        "suppress_patterns": "string[]"
    }});
    sec!("canvas", { "fields": {
        "enabled": "boolean",
        "max_html_bytes": { "type": "number", "min": 1024, "max": 10485760, "step": 1024 },
        "allowed_tags": "string[]"
    }});
    sec!("tts", { "fields": {
        "enabled": "boolean",
        "provider": { "type": "select", "options": ["openai", "elevenlabs"] },
        "max_text_length": { "type": "number", "min": 100, "max": 100000, "step": 100 },
        "timeout_secs": { "type": "number", "min": 5, "max": 300, "step": 1 }
    }});
    sec!("docker", { "fields": {
        "enabled": "boolean", "image": "string", "container_prefix": "string",
        "workdir": "string", "network": "string", "memory_limit": "string",
        "cpu_limit": { "type": "number", "min": 0.1, "max": 32, "step": 0.1 },
        "timeout_secs": { "type": "number", "min": 1, "max": 3600, "step": 1 },
        "read_only_root": "boolean",
        "pids_limit": { "type": "number", "min": 1, "max": 10000, "step": 1 },
        "reuse_cool_secs": { "type": "number", "min": 0, "max": 3600, "step": 1 },
        "idle_timeout_secs": { "type": "number", "min": 0, "max": 86400, "step": 1 },
        "max_age_secs": { "type": "number", "min": 0, "max": 86400, "step": 1 }
    }});
    sec!("pairing", { "fields": {
        "enabled": "boolean",
        "max_devices": { "type": "number", "min": 1, "max": 100, "step": 1 },
        "token_expiry_secs": { "type": "number", "min": 60, "max": 2592000, "step": 60 },
        "push_provider": { "type": "select", "options": ["none", "ntfy", "gotify"] },
        "ntfy_url": "string", "ntfy_topic": "string"
    }});
    sec!("thinking", { "fields": { "budget_tokens": { "type": "number", "min": 1024, "max": 64000, "step": 512 }, "stream_thinking": "boolean" }});
    sec!("budget", { "hot_reloadable": true, "fields": {
        "max_hourly_usd": { "type": "number", "min": 0, "max": 1000, "step": 0.01 },
        "max_daily_usd": { "type": "number", "min": 0, "max": 10000, "step": 0.01 },
        "max_monthly_usd": { "type": "number", "min": 0, "max": 100000, "step": 0.01 },
        "alert_threshold": { "type": "number", "min": 0, "max": 1, "step": 0.01 },
        "default_max_llm_tokens_per_hour": { "type": "number", "min": 0, "max": 10000000, "step": 1000 }
    }});
    sec!("vertex_ai", { "fields": {
        "project_id": "string", "region": "string", "credentials_path": "string"
    }});
    sec!("oauth", { "fields": {
        "google_client_id": "string", "github_client_id": "string",
        "microsoft_client_id": "string", "slack_client_id": "string"
    }});
    sec!("session", { "fields": {
        "retention_days": { "type": "number", "min": 1, "max": 3650, "step": 1 },
        "max_sessions_per_agent": { "type": "number", "min": 1, "max": 10000, "step": 1 },
        "cleanup_interval_hours": { "type": "number_select", "options": ["0", "1", "6", "12", "24", "48", "168"] }
    }});
    sec!("queue", { "fields": {
        "max_depth_per_agent": { "type": "number", "min": 1, "max": 10000, "step": 1 },
        "max_depth_global": { "type": "number", "min": 1, "max": 100000, "step": 1 },
        "task_ttl_secs": { "type": "number", "min": 60, "max": 86400, "step": 60 }
    }});
    sec!("external_auth", { "fields": {
        "enabled": "boolean", "issuer_url": "string", "client_id": "string",
        "client_secret_env": "string", "redirect_url": "string",
        "scopes": "string[]", "allowed_domains": "string[]",
        "audience": "string",
        "session_ttl_secs": { "type": "number", "min": 60, "max": 2592000, "step": 60 }
    }});
    sec!("terminal", { "fields": {
        "enabled": "boolean",
        "allow_remote": "boolean",
        "require_proxy_headers": "boolean",
        "allow_unauthenticated_remote": "boolean",
        "allowed_origins": "string[]",
        "tmux_enabled": "boolean",
        "max_windows": { "type": "number", "min": 1, "max": 64, "step": 1 },
        "tmux_binary_path": "string"
    }});

    Json(serde_json::json!({ "sections": serde_json::Value::Object(sections) }))
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

    let config_path = state.kernel.home_dir().join("config.toml");
    // Block path-traversal (`..`) but allow Windows drive-letter prefixes
    if config_path.file_name().and_then(|n| n.to_str()) != Some("config.toml")
        || config_path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"status":"error","error":"invalid config file path"})),
        );
    }

    // Serialize concurrent writes to prevent read-modify-write races
    let _config_guard = state.config_write_lock.lock().await;

    // Read existing config — use toml_edit to preserve comments and formatting
    let raw_content = if config_path.exists() {
        std::fs::read_to_string(&config_path).unwrap_or_default()
    } else {
        String::new()
    };
    let mut doc: toml_edit::DocumentMut = match raw_content.parse() {
        Ok(d) => d,
        Err(_) => toml_edit::DocumentMut::new(),
    };

    // null → remove key instead of writing empty string
    let is_remove = value.is_null();

    // Parse "section.key" path and set/remove value
    let parts: Vec<&str> = path.split('.').collect();
    match parts.len() {
        1 => {
            if is_remove {
                doc.remove(parts[0]);
            } else {
                doc[parts[0]] = toml_edit::Item::Value(json_to_toml_edit_value(&value));
            }
        }
        2 => {
            if is_remove {
                if let Some(t) = doc[parts[0]].as_table_mut() {
                    t.remove(parts[1]);
                }
            } else {
                if !doc.contains_table(parts[0]) {
                    doc[parts[0]] = toml_edit::Item::Table(toml_edit::Table::new());
                }
                doc[parts[0]][parts[1]] = toml_edit::Item::Value(json_to_toml_edit_value(&value));
            }
        }
        3 => {
            if is_remove {
                if let Some(t) = doc[parts[0]].as_table_mut() {
                    if let Some(t2) = t.get_mut(parts[1]).and_then(|i| i.as_table_mut()) {
                        t2.remove(parts[2]);
                    }
                }
            } else {
                if !doc.contains_table(parts[0]) {
                    doc[parts[0]] = toml_edit::Item::Table(toml_edit::Table::new());
                }
                if !doc[parts[0]]
                    .as_table()
                    .is_some_and(|t| t.contains_table(parts[1]))
                {
                    doc[parts[0]][parts[1]] = toml_edit::Item::Table(toml_edit::Table::new());
                }
                doc[parts[0]][parts[1]][parts[2]] =
                    toml_edit::Item::Value(json_to_toml_edit_value(&value));
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

    // Validate by parsing the result as KernelConfig before writing.
    // This is the *schema* check (types deserialize cleanly), not the
    // *business* check (e.g. cross-field invariants).
    let new_toml_str = doc.to_string();
    let mut parsed_config =
        match toml::from_str::<librefang_types::config::KernelConfig>(&new_toml_str) {
            Ok(cfg) => cfg,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "status": "error",
                        "error": format!("invalid config after edit: {e}")
                    })),
                );
            }
        };

    // Business-level validation BEFORE writing to disk. Without this
    // check, edits like `network_enabled = true` (without setting
    // `shared_secret`) would persist a definitely-broken config to disk
    // and only fail at the post-write reload step, leaving the user
    // with a `saved_reload_failed` status and a TOML file that will
    // also fail the next daemon startup. Apply clamp_bounds first to
    // mirror the reload-side preprocessing — otherwise a user-set
    // out-of-range value would be flagged here even though reload
    // would silently fix it.
    parsed_config.clamp_bounds();
    if let Err(errors) = librefang_kernel::config_reload::validate_config_for_reload(&parsed_config)
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "status": "error",
                "error": format!("invalid config: {}", errors.join("; "))
            })),
        );
    }

    // Backup under backups/ before write (single rolling copy).
    if config_path.exists() {
        if let Some(home_dir) = config_path.parent() {
            let backups_dir = home_dir.join("backups");
            if std::fs::create_dir_all(&backups_dir).is_ok() {
                let _ = std::fs::copy(&config_path, backups_dir.join("config.toml.prev"));
            }
        }
    }

    // Write back — preserves comments, whitespace, and key ordering
    if let Err(e) = std::fs::write(&config_path, &new_toml_str) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"status": "error", "error": format!("write failed: {e}")})),
        );
    }

    // Trigger reload
    let (reload_status, reload_error): (&'static str, Option<String>) =
        match state.kernel.reload_config().await {
            Ok(plan) => {
                let s = if plan.restart_required {
                    "applied_partial"
                } else {
                    "applied"
                };
                (s, None)
            }
            Err(e) => {
                // Surface the actual reload failure reason so the dashboard
                // can show users what's wrong (e.g. "validation failed:
                // network_enabled is true but shared_secret is empty"
                // instead of an opaque "saved but reload failed"). The TOML
                // file has already been written at this point, so leaving
                // the user without a reason is doubly bad — they can't
                // distinguish "transient kernel hiccup, restart will pick
                // it up" from "permanently invalid config that breaks
                // restart too".
                tracing::warn!(error = %e, %path, "config reload failed after write");
                ("saved_reload_failed", Some(e))
            }
        };

    state.kernel.audit().record(
        "system",
        librefang_runtime::audit::AuditAction::ConfigChange,
        &format!("config set: {path}"),
        "completed",
    );

    let mut body = serde_json::json!({"status": reload_status, "path": path});
    if let Some(err) = reload_error {
        body["reload_error"] = serde_json::Value::String(err);
    }
    (StatusCode::OK, Json(body))
}

/// Convert a serde_json::Value to a toml_edit::Value (format-preserving).
fn json_to_toml_edit_value(value: &serde_json::Value) -> toml_edit::Value {
    match value {
        serde_json::Value::String(s) => s.as_str().into(),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.into()
            } else if let Some(f) = n.as_f64() {
                f.into()
            } else {
                n.to_string().into()
            }
        }
        serde_json::Value::Bool(b) => (*b).into(),
        serde_json::Value::Array(arr) => {
            let mut a = toml_edit::Array::new();
            for item in arr {
                a.push(json_to_toml_edit_value(item));
            }
            toml_edit::Value::Array(a)
        }
        serde_json::Value::Object(map) => {
            let mut t = toml_edit::InlineTable::new();
            for (k, v) in map {
                t.insert(k, json_to_toml_edit_value(v));
            }
            toml_edit::Value::InlineTable(t)
        }
        // null is handled by the caller (remove key) — fallback to empty string
        serde_json::Value::Null => "".into(),
    }
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
        serde_json::Value::Array(arr) => {
            toml::Value::Array(arr.iter().map(json_to_toml_value).collect())
        }
        serde_json::Value::Object(map) => {
            // Convert nested JSON objects into TOML tables. Without this, the
            // catch-all below would JSON-stringify the whole object, which is
            // how #2319 wrote `mcp_servers = ['{"name":"..."}']` into config.toml
            // and broke reload.
            let mut table = toml::map::Map::new();
            for (k, v) in map {
                table.insert(k.clone(), json_to_toml_value(v));
            }
            toml::Value::Table(table)
        }
        // Null has no TOML analogue — emit an empty string so the key still
        // round-trips; callers that care should filter before calling.
        serde_json::Value::Null => toml::Value::String(String::new()),
    }
}

/// GET /api/dashboard/snapshot — Single aggregated snapshot for the dashboard.
///
/// Replaces 7 parallel frontend requests (health, status, providers, channels,
/// skills, agents, workflows) with one round-trip, cutting poll overhead by ~7x.
pub async fn dashboard_snapshot(
    State(state): State<Arc<AppState>>,
) -> axum::Json<serde_json::Value> {
    axum::Json(dashboard_snapshot_inner(&state).await)
}

async fn dashboard_snapshot_inner(state: &Arc<AppState>) -> serde_json::Value {
    // Health (same logic as /api/health)
    let shared_id = librefang_types::agent::AgentId(uuid::Uuid::from_bytes([
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
    ]));
    let db_ok = state
        .kernel
        .memory_substrate()
        .structured_get(shared_id, "__health_check__")
        .is_ok();
    let health_status = if db_ok { "ok" } else { "degraded" };
    let fts_only = state.kernel.config_ref().memory.fts_only.unwrap_or(false);
    let embedding_ok = fts_only || state.kernel.embedding().is_some();
    let health = serde_json::json!({
        "status": health_status,
        "version": env!("CARGO_PKG_VERSION"),
        "checks": [
            { "name": "database", "status": if db_ok { "ok" } else { "error" } },
            { "name": "embedding", "status": if embedding_ok { "ok" } else { "warn" } },
        ],
    });

    // Status (same logic as /api/status, without the heavy per-agent list)
    let agent_entries = state.kernel.agent_registry().list();
    let agent_count = agent_entries.iter().filter(|e| !e.is_hand).count();
    let active_agent_count = agent_entries
        .iter()
        .filter(|e| !e.is_hand && matches!(e.state, librefang_types::agent::AgentState::Running))
        .count();
    let session_count = state
        .kernel
        .memory_substrate()
        .list_sessions()
        .map(|s| s.len())
        .unwrap_or(0);
    let cfg = state.kernel.config_snapshot();
    // Runtime stats shared with `/api/status` — the dashboard RuntimePage
    // reads these out of the snapshot for its info panel and KPI tiles.
    // Anything missing here renders as "-" on the page.
    let uptime_seconds = state.started_at.elapsed().as_secs();
    let memory_used_mb = current_process_rss_mb();
    let status = serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "agent_count": agent_count,
        "active_agent_count": active_agent_count,
        "session_count": session_count,
        "uptime_seconds": uptime_seconds,
        "memory_used_mb": memory_used_mb,
        "default_provider": cfg.default_model.provider,
        "default_model": cfg.default_model.model,
        "config_exists": state.kernel.home_dir().join("config.toml").exists(),
        "api_listen": cfg.api_listen,
        "home_dir": state.kernel.home_dir().display().to_string(),
        "log_level": cfg.log_level,
        "hostname": system_hostname(),
        "network_enabled": cfg.network_enabled,
        "terminal_enabled": cfg.terminal.enabled,
    });

    // Agents list — fully enriched (same fields as /api/agents) so AgentsPage
    // can use this snapshot directly instead of polling /api/agents separately.
    let agents: Vec<serde_json::Value> = {
        let catalog = state.kernel.model_catalog_ref().read().ok();
        let dm = {
            let dm_override = state
                .kernel
                .default_model_override_ref()
                .read()
                .unwrap_or_else(|e| e.into_inner());
            super::agents::effective_default_model(&cfg.default_model, dm_override.as_ref())
        };
        let mut agent_entries_visible: Vec<_> = agent_entries.iter().collect();
        // Sort by last_active descending — matches AgentsPage default query order.
        agent_entries_visible.sort_by_key(|b| std::cmp::Reverse(b.last_active));
        agent_entries_visible
            .iter()
            .map(|e| super::agents::enrich_agent_json(e, &dm, &catalog))
            .collect()
    };

    // Skills count — cached behind a 30s TTL to avoid scanning the skills
    // directory on every poll cycle.
    static SKILL_COUNT_CACHE: std::sync::Mutex<Option<(usize, std::time::Instant)>> =
        std::sync::Mutex::new(None);
    let skill_count = {
        let cached = SKILL_COUNT_CACHE
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .and_then(|(n, t)| {
                if t.elapsed() < std::time::Duration::from_secs(30) {
                    Some(*n)
                } else {
                    None
                }
            });
        match cached {
            Some(n) => n,
            None => {
                // Use the kernel's LIVE registry so `skills.disabled` and
                // `skills.extra_dirs` from config are honoured. The old
                // fresh-registry path showed disabled skills in the count
                // and missed extra_dirs entries.
                let n = state
                    .kernel
                    .skill_registry_ref()
                    .read()
                    .map(|r| r.list().len())
                    .unwrap_or(0);
                *SKILL_COUNT_CACHE.lock().unwrap_or_else(|p| p.into_inner()) =
                    Some((n, std::time::Instant::now()));
                n
            }
        }
    };

    // Workflows, providers, channels — run concurrently with a 5s timeout on
    // providers/channels in case a local provider probe stalls.
    const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
    let (workflow_result, providers_result, channels_result) = tokio::join!(
        state.kernel.workflow_engine().list_workflows(),
        tokio::time::timeout(PROBE_TIMEOUT, super::providers::providers_snapshot(state)),
        tokio::time::timeout(PROBE_TIMEOUT, super::channels::channels_snapshot(state)),
    );
    let workflow_count = workflow_result.len();
    let providers = providers_result.unwrap_or_default();
    let channels = channels_result.unwrap_or_default();

    // Check if at least one web search provider has a configured API key
    let web_search_available = [
        &cfg.web.tavily.api_key_env,
        &cfg.web.brave.api_key_env,
        &cfg.web.jina.api_key_env,
        &cfg.web.perplexity.api_key_env,
    ]
    .iter()
    .any(|env_var| {
        std::env::var(env_var)
            .ok()
            .filter(|v| !v.is_empty())
            .is_some()
    });

    serde_json::json!({
        "health": health,
        "status": status,
        "agents": agents,
        "providers": providers,
        "channels": channels,
        "skillCount": skill_count,
        "workflowCount": workflow_count,
        "webSearchAvailable": web_search_available,
    })
}
