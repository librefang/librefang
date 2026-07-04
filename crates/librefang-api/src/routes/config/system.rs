use super::*;

#[utoipa::path(
    get,
    path = "/api/status",
    tag = "system",
    responses(
        (status = 200, description = "Daemon status", body = crate::types::JsonObject)
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
    // Use the indexed `SELECT COUNT(*)` projection — `list_sessions()`
    // here would return a `Vec<serde_json::Value>` with each session's
    // full rmp-encoded message history decoded just to call `.len()`.
    // The dashboard hammers this route on its 5 s status poll, so on
    // a workspace with 100 sessions × 200 KB history apiece the daemon
    // decoded ~20 MB (≈ 4 MB/s) of message bodies every poll for what
    // is morphologically a `SELECT COUNT(*)`.
    let session_count = state
        .kernel
        .memory_substrate()
        .count_sessions()
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
        (status = 200, description = "Quick init result", body = crate::types::JsonObject)
    )
)]
pub async fn quick_init(State(state): State<Arc<AppState>>) -> axum::response::Response {
    let home = state.kernel.home_dir();
    let config_path = home.join("config.toml");

    if config_path.exists() {
        return Json(serde_json::json!({
            "status": "already_initialized",
            "message": "config.toml already exists"
        }))
        .into_response();
    }

    // Ensure directories exist
    let _ = std::fs::create_dir_all(home);
    let _ = std::fs::create_dir_all(home.join("data"));

    // Detect best available provider
    let (provider, api_key_env) = if let Some((p, _model, env_var)) =
        librefang_kernel::drivers::detect_available_provider()
    {
        (p.to_string(), env_var.to_string())
    } else {
        ("groq".to_string(), "GROQ_API_KEY".to_string())
    };

    // Resolve default model from catalog
    let model = librefang_kernel::model_catalog::ModelCatalog::default()
        .automatic_default_model_for_provider(&provider)
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

    if let Err(e) = crate::atomic_write(&config_path, config_content.as_bytes()) {
        // Scrub the io error (audit: rusqlite-errors-leak) — path /
        // permission detail stays in the log, generic body to client.
        tracing::error!(error = %e, "failed to write config during init");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "status": "error",
                "message": "Internal server error"
            })),
        )
            .into_response();
    }

    // Reload config so kernel picks up new settings. Surface failures (#3374) —
    // before this fix the result was swallowed and the handler reported success
    // even though the running daemon kept the stale config.
    if let Err(e) = state.kernel.reload_config().await {
        // Scrub the reload error (audit: rusqlite-errors-leak) — the
        // detail goes to the log; the client keeps the actionable
        // status ("init succeeded but reload failed") without the raw
        // chain.
        tracing::error!(error = %e, "config reload failed after init");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "status": "reload_failed",
                "message": "init succeeded but reload failed",
                "provider": provider,
                "model": model,
            })),
        )
            .into_response();
    }

    Json(serde_json::json!({
        "status": "initialized",
        "provider": provider,
        "model": model,
    }))
    .into_response()
}

/// POST /api/shutdown — Graceful shutdown.
#[utoipa::path(
    post,
    path = "/api/shutdown",
    tag = "system",
    responses(
        (status = 200, description = "Graceful daemon shutdown", body = crate::types::JsonObject)
    )
)]
pub async fn shutdown(
    State(state): State<Arc<AppState>>,
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
) -> impl IntoResponse {
    tracing::info!("Shutdown requested via API");
    // SECURITY: Record shutdown in audit trail with the caller's user_id
    // (None for loopback/unauthenticated calls — see middleware.rs).
    let user_id = api_user.as_ref().map(|u| u.0.user_id);
    state.kernel.audit().record_with_context(
        "system",
        librefang_kernel::audit::AuditAction::ConfigChange,
        "shutdown requested via API",
        "ok",
        user_id,
        Some("api".to_string()),
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
        (status = 200, description = "Version information", body = crate::types::JsonObject)
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
        (status = 200, description = "Health check", body = crate::types::JsonObject)
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
        (status = 200, description = "Detailed health diagnostics", body = crate::types::JsonObject)
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

    // Budget snapshot — already aggregated by MeteringEngine (single-row SQL
    // queries, all indexed). `daily_spend_percent` is `None` when no daily
    // cap is configured so monitors don't false-fire on undefined ratios.
    let budget_status = state
        .kernel
        .metering_ref()
        .budget_status(&state.kernel.budget_config());
    let daily_spend_percent = if budget_status.daily_limit > 0.0 {
        Some(budget_status.daily_pct * 100.0)
    } else {
        None
    };
    let hourly_spend_percent = if budget_status.hourly_limit > 0.0 {
        Some(budget_status.hourly_pct * 100.0)
    } else {
        None
    };
    let monthly_spend_percent = if budget_status.monthly_limit > 0.0 {
        Some(budget_status.monthly_pct * 100.0)
    } else {
        None
    };

    // LLM call latency snapshot — cached for HEALTH_METRICS_TTL to avoid
    // re-running the GROUP BY on every probe scrape. Only `count` and
    // mean / max latency are surfaced; P50/P95 percentiles would require a
    // histogram which the kernel does not currently maintain (see PR notes).
    let llm = llm_health_snapshot(&state);

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
        "budget": {
            "hourly_spend_usd": budget_status.hourly_spend,
            "hourly_limit_usd": budget_status.hourly_limit,
            "hourly_spend_percent": hourly_spend_percent,
            "daily_spend_usd": budget_status.daily_spend,
            "daily_limit_usd": budget_status.daily_limit,
            "daily_spend_percent": daily_spend_percent,
            "monthly_spend_usd": budget_status.monthly_spend,
            "monthly_limit_usd": budget_status.monthly_limit,
            "monthly_spend_percent": monthly_spend_percent,
            "alert_threshold": budget_status.alert_threshold,
        },
        "llm": {
            "total_calls": llm.total_calls,
            "avg_latency_ms": llm.avg_latency_ms,
            "max_latency_ms": llm.max_latency_ms,
            "model_count": llm.model_count,
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
        (status = 200, description = "Prometheus text-format metrics", body = crate::types::JsonObject)
    )
)]
pub async fn prometheus_metrics(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut out = String::with_capacity(4096);

    // Uptime
    let uptime = state.started_at.elapsed().as_secs();
    out.push_str("# HELP librefang_uptime_seconds Time since daemon started.\n");
    out.push_str("# TYPE librefang_uptime_seconds gauge\n");
    out.push_str(&format!("librefang_uptime_seconds {uptime}\n\n"));

    // Active agents — read-only counter and projection; cheap Arc clones (#3569).
    let agents = state.kernel.agent_registry().list_arcs();
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
    if let Some(handle) = crate::telemetry::prometheus_handle() {
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
