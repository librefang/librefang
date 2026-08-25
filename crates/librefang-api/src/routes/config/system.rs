use super::*;
use librefang_types::config::DefaultModelConfig;
use std::sync::RwLock;

fn status_default_model_snapshot(
    model_override: &RwLock<Option<DefaultModelConfig>>,
    configured: &DefaultModelConfig,
) -> (String, String) {
    let guard = model_override.read().unwrap_or_else(|poisoned| {
        tracing::warn!(
            "System status default-model override lock poisoned; recovering response state"
        );
        model_override.clear_poison();
        poisoned.into_inner()
    });
    let effective = guard.as_ref().unwrap_or(configured);
    (effective.provider.clone(), effective.model.clone())
}

#[derive(serde::Serialize)]
struct QuickInitConfig<'a> {
    log_level: &'a str,
    api_listen: &'a str,
    default_model: QuickInitDefaultModel<'a>,
}

#[derive(serde::Serialize)]
struct QuickInitDefaultModel<'a> {
    provider: &'a str,
    model: &'a str,
    api_key_env: &'a str,
}

fn quick_init_config_content(
    provider: &str,
    model: &str,
    api_key_env: &str,
) -> Result<String, toml::ser::Error> {
    let config = QuickInitConfig {
        log_level: "info",
        api_listen: "127.0.0.1:4545",
        default_model: QuickInitDefaultModel {
            provider,
            model,
            api_key_env,
        },
    };
    let serialized = toml::to_string_pretty(&config)?;
    Ok(format!(
        "# LibreFang configuration (auto-generated)\n\
         # Run `librefang init --upgrade` for full annotated config.\n\n\
         {serialized}"
    ))
}

#[utoipa::path(
    get,
    path = "/api/status",
    tag = "system",
    responses(
        (status = 200, description = "Daemon status", body = crate::types::JsonObject)
    )
)]
pub async fn status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let (memory_used_mb, hostname) = tokio::join!(current_process_rss_mb(), system_hostname());
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

    let cfg = state.kernel.config_snapshot();
    let (default_provider, default_model) = status_default_model_snapshot(
        state.kernel.default_model_override_ref(),
        &cfg.default_model,
    );
    Json(serde_json::json!({
        "status": "running",
        "version": env!("CARGO_PKG_VERSION"),
        "agent_count": agent_count,
        "active_agent_count": active_agent_count,
        "session_count": session_count,
        "memory_used_mb": memory_used_mb,
        "default_provider": default_provider,
        "default_model": default_model,
        "uptime_seconds": uptime,
        "api_listen": cfg.api_listen,
        "home_dir": state.kernel.home_dir().display().to_string(),
        "log_level": cfg.log_level,
        "hostname": hostname,
        "network_enabled": cfg.network_enabled,
        "terminal_enabled": cfg.terminal.enabled,
        "config_exists": state.kernel.config_path().exists(),
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
    let config_path = state.kernel.config_path().to_path_buf();
    if tokio::fs::try_exists(&config_path).await.unwrap_or(false) {
        return Json(serde_json::json!({
            "status": "already_initialized",
            "message": "config.toml already exists"
        }))
        .into_response();
    }

    // Detect best available provider
    let (provider, api_key_env) = if let Some((p, _model, env_var)) =
        librefang_kernel::drivers::detect_available_provider()
    {
        (p.to_string(), env_var.to_string())
    } else {
        ("groq".to_string(), "GROQ_API_KEY".to_string())
    };

    // Resolve the default model from the kernel's live catalog rather than a throwaway `ModelCatalog::default()`.
    // This ensures a first-run auto-detect of `openrouter` (via `OPENROUTER_API_KEY`) picks a model consistent with the live catalog instead of only ever the checked-in build snapshot (#6384).
    // The live catalog is refreshed synchronously here so the first-run resolution can immediately use it.
    let _ = crate::openrouter_catalog::refresh_if_missing(&state.kernel).await;
    let model = state
        .kernel
        .model_catalog_ref()
        .load()
        .automatic_default_model_for_provider(&provider)
        .unwrap_or_else(|| "auto".to_string());

    // Use the TOML serializer so catalog-provided identifiers cannot escape their string values.
    let config_content = match quick_init_config_content(&provider, &model, &api_key_env) {
        Ok(content) => content,
        Err(error) => {
            tracing::error!(%error, "failed to serialize quick init config");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "status": "error",
                    "message": "Internal server error"
                })),
            )
                .into_response();
        }
    };

    if let Some(locked) = crate::routes::guard_config_write(state.kernel.config_path()) {
        return locked.into_response();
    }
    let _config_guard = state.config_write_lock.lock().await;
    let home = state.kernel.home_dir().to_path_buf();
    let write_result = tokio::task::spawn_blocking(move || {
        write_quick_init_config(&home, &config_path, config_content.as_bytes())
    })
    .await;
    match write_result {
        Ok(Ok(false)) => {
            return Json(serde_json::json!({
                "status": "already_initialized",
                "message": "config.toml already exists"
            }))
            .into_response();
        }
        Ok(Ok(true)) => {}
        Ok(Err(e)) => {
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
        Err(e) => {
            tracing::error!(error = %e, "quick init config write task failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "status": "error",
                    "message": "Internal server error"
                })),
            )
                .into_response();
        }
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

/// Write the quick-init `config.toml`, refusing to overwrite one that already exists.
///
/// `config_path` is the kernel's resolved path rather than `home/config.toml`, so an init through a relocated (`LIBREFANG_CONFIG_PATH`) config writes the file the daemon will actually reload (#6695).
/// `home` still governs the directory layout — `data/` belongs to the home directory whether or not the config file lives inside it.
fn write_quick_init_config(
    home: &std::path::Path,
    config_path: &std::path::Path,
    contents: &[u8],
) -> std::io::Result<bool> {
    if config_path.exists() {
        return Ok(false);
    }
    std::fs::create_dir_all(home)?;
    std::fs::create_dir_all(home.join("data"))?;
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::atomic_write(config_path, contents)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::{
        quick_init_config_content, status_default_model_snapshot, write_quick_init_config,
    };
    use librefang_types::config::DefaultModelConfig;
    use std::sync::RwLock;

    #[test]
    fn status_model_snapshot_recovers_consistent_override_after_poison() {
        let configured = DefaultModelConfig {
            provider: "configured-provider".to_string(),
            model: "configured-model".to_string(),
            ..DefaultModelConfig::default()
        };
        let model_override = RwLock::new(Some(DefaultModelConfig {
            provider: "override-provider".to_string(),
            model: "override-model".to_string(),
            ..DefaultModelConfig::default()
        }));
        let _ = std::panic::catch_unwind(|| {
            let _guard = model_override.write().unwrap();
            panic!("poison status default-model override");
        });
        assert!(model_override.is_poisoned());

        assert_eq!(
            status_default_model_snapshot(&model_override, &configured),
            (
                "override-provider".to_string(),
                "override-model".to_string()
            )
        );

        assert!(!model_override.is_poisoned());
        assert!(model_override.read().is_ok());
        assert!(model_override.write().is_ok());
    }

    #[test]
    fn status_model_snapshot_uses_configured_pair_without_override() {
        let configured = DefaultModelConfig {
            provider: "configured-provider".to_string(),
            model: "configured-model".to_string(),
            ..DefaultModelConfig::default()
        };
        let model_override = RwLock::new(None);

        assert_eq!(
            status_default_model_snapshot(&model_override, &configured),
            (
                "configured-provider".to_string(),
                "configured-model".to_string()
            )
        );
    }

    #[test]
    fn quick_init_write_is_create_once_and_preserves_existing_config() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path().join("home");

        assert!(
            write_quick_init_config(&home, &home.join("config.toml"), b"first = true\n").unwrap()
        );
        assert!(home.join("data").is_dir());
        assert!(
            !write_quick_init_config(&home, &home.join("config.toml"), b"second = true\n").unwrap()
        );
        assert_eq!(
            std::fs::read_to_string(home.join("config.toml")).unwrap(),
            "first = true\n"
        );
    }

    #[test]
    fn quick_init_config_serializes_untrusted_model_fields_as_toml_strings() {
        let provider = "provider\"\n[network]\nenabled = true\n#";
        let model = "vendor\\model\"\n[default_model]";
        let api_key_env = "KEY\\NAME\nVALUE";

        let contents = quick_init_config_content(provider, model, api_key_env).unwrap();
        let parsed: toml::Value = toml::from_str(&contents).unwrap();

        assert_eq!(parsed["default_model"]["provider"].as_str(), Some(provider));
        assert_eq!(parsed["default_model"]["model"].as_str(), Some(model));
        assert_eq!(
            parsed["default_model"]["api_key_env"].as_str(),
            Some(api_key_env)
        );
        assert!(parsed.get("network").is_none());
        assert_eq!(parsed.as_table().unwrap().len(), 3);

        let config: librefang_types::config::KernelConfig = toml::from_str(&contents).unwrap();
        assert_eq!(config.default_model.provider, provider);
        assert_eq!(config.default_model.model, model);
        assert_eq!(config.default_model.api_key_env, api_key_env);
    }
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

/// Probe the SQLite memory substrate with the cheapest possible read.
///
/// Shared by `/api/health`, `/api/health/detail`, and `/api/ready` so all
/// three agree on what "the database is reachable" means. `structured_get`
/// on a well-known sentinel key hits the connection pool and the schema
/// without depending on any row existing — a missing key is `Ok(None)`,
/// only a genuine connection / schema failure is `Err`.
fn database_probe_ok(state: &Arc<AppState>) -> bool {
    let shared_id = librefang_types::agent::AgentId(uuid::Uuid::from_bytes([
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
    ]));
    state
        .kernel
        .memory_substrate()
        .structured_get(shared_id, "__health_check__")
        .is_ok()
}

/// Is a working embedding driver a *requirement* for this deployment, or an
/// optional enhancement?
///
/// It is a requirement only when the operator pinned a specific embedding
/// provider while leaving vector search enabled: that combination states an
/// intent the daemon failed to satisfy, so serving traffic would silently
/// degrade memory recall. In every other shape the absence of a driver is
/// the documented, supported fallback and must not fail readiness:
///
/// * `fts_only = true` — vector search is switched off; `boot.rs` never
///   constructs a driver at all.
/// * `embedding_provider` unset or `"auto"` — boot probes provider API-key
///   env vars and falls back to local Ollama, then to FTS. Nothing was
///   promised, so nothing is broken.
///
/// Callers must evaluate this against the config the process **booted** with
/// and cache the answer in `AppState::readiness_requires_embedding`, not read
/// it live per request. See that field's documentation for why.
pub(crate) fn embedding_is_required(config: &librefang_types::config::KernelConfig) -> bool {
    if config.memory.fts_only.unwrap_or(false) {
        return false;
    }
    config
        .memory
        .embedding_provider
        .as_deref()
        .map(str::trim)
        .is_some_and(|provider| !provider.is_empty() && !provider.eq_ignore_ascii_case("auto"))
}

/// GET /api/ready — Readiness probe (public, no auth required).
///
/// Distinct from `/api/health` on purpose. `/api/health` answers "is this
/// process alive?" and always returns 200 while the HTTP server can respond,
/// so a Kubernetes `livenessProbe` pointed at it never restarts a pod over a
/// recoverable storage or provider incident. This endpoint answers "can this
/// process accept work?" and returns 503 when a dependency required to do so
/// is unavailable, which is what removes the pod from Service endpoints
/// without killing it.
///
/// The body is deliberately minimal — check names and a coarse status only.
/// It carries no version, hostname, provider id, model name, path, or error
/// text, because an unauthenticated caller reaches it; detailed diagnostics
/// remain behind `GET /api/health/detail`.
#[utoipa::path(
    get,
    path = "/api/ready",
    tag = "system",
    responses(
        (status = 200, description = "All required dependencies are ready", body = crate::types::JsonObject),
        (status = 503, description = "A required dependency is unavailable", body = crate::types::JsonObject)
    )
)]
pub async fn ready(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let db_ok = database_probe_ok(&state);

    // Boot-time snapshot, not `config_ref()`: the requirement and the driver
    // must be read from the same point in time, or a `POST /api/config/reload`
    // that adds `memory.embedding_provider` pins readiness at 503 against a
    // driver that a `restart_required` change will never rebuild. See
    // `AppState::readiness_requires_embedding`.
    let embedding_required = state.readiness_requires_embedding;
    let embedding_present = state.kernel.embedding().is_some();
    let embedding_status = match (embedding_required, embedding_present) {
        (true, true) => "ok",
        (true, false) => "error",
        // Not required: report whether one happens to be available so the
        // payload stays useful for humans, without gating readiness on it.
        (false, true) => "ok",
        (false, false) => "skipped",
    };

    // Written as "database ok, and the embedding requirement is satisfied"
    // rather than negating the failure case: `clippy::nonminimal_bool` rejects
    // the `!(a && !b)` form, and the positive reading matches the two `checks`
    // entries reported below.
    let is_ready = db_ok && (!embedding_required || embedding_present);
    let code = if is_ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        code,
        Json(serde_json::json!({
            "status": if is_ready { "ready" } else { "not_ready" },
            "checks": [
                { "name": "database", "status": if db_ok { "ok" } else { "error" }, "required": true },
                { "name": "embedding", "status": embedding_status, "required": embedding_required },
            ],
        })),
    )
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
    let db_ok = database_probe_ok(&state);

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

    let db_ok = database_probe_ok(&state);

    let hcfg = state.kernel.config_ref();
    let config_warnings = hcfg.validate();
    let status = if db_ok { "ok" } else { "degraded" };

    // Budget snapshot — already aggregated by MeteringEngine (single-row SQL
    // queries, all indexed). `daily_spend_percent` is `None` when no daily
    // cap is configured so monitors don't false-fire on undefined ratios.
    let budget_status = match state
        .kernel
        .metering_ref()
        .budget_status(&state.kernel.budget_config())
    {
        Ok(status) => status,
        Err(error) => return ApiErrorResponse::internal_scrub(error).into_response(),
    };
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
    .into_response()
}

// ---------------------------------------------------------------------------
// Prometheus metrics endpoint
// ---------------------------------------------------------------------------
fn escape_prometheus_label_value(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' | '\r' => escaped.push_str("\\n"),
            _ => escaped.push(character),
        }
    }
    escaped
}

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
        if let Some(snap) = state.kernel.scheduler_ref().get_usage(agent.id) {
            let name = escape_prometheus_label_value(&agent.name);
            let provider = escape_prometheus_label_value(&agent.manifest.model.provider);
            let model = escape_prometheus_label_value(&agent.manifest.model.model);
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
