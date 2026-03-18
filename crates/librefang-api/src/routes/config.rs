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
                librefang_types::config::McpTransportEntry::Stdio { command, args } => {
                    serde_json::json!({ "type": "stdio", "command": command, "args": args })
                }
                librefang_types::config::McpTransportEntry::Sse { url } => {
                    serde_json::json!({ "type": "sse", "url": url })
                }
                librefang_types::config::McpTransportEntry::HttpCompat { base_url, .. } => {
                    serde_json::json!({ "type": "http_compat", "base_url": base_url })
                }
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
    set!("usage_footer", format!("{:?}", config.usage_footer));
    set!("stable_prefix_mode", config.stable_prefix_mode);
    set!("prompt_caching", config.prompt_caching);
    set!("max_cron_jobs", config.max_cron_jobs);
    set!("include", config.include);
    set!(
        "workspaces_dir",
        config
            .workspaces_dir
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
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
        "auto_memorize": config.proactive_memory.auto_memorize,
        "auto_retrieve": config.proactive_memory.auto_retrieve,
        "max_retrieve": config.proactive_memory.max_retrieve,
        "extraction_threshold": config.proactive_memory.extraction_threshold,
        "extraction_model": config.proactive_memory.extraction_model,
        "extract_categories": config.proactive_memory.extract_categories,
        "session_ttl_hours": config.proactive_memory.session_ttl_hours,
        "confidence_decay_rate": config.proactive_memory.confidence_decay_rate,
        "duplicate_threshold": config.proactive_memory.duplicate_threshold,
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
    set!("web", {
        "search_provider": format!("{:?}", config.web.search_provider),
        "cache_ttl_minutes": config.web.cache_ttl_minutes,
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

    set!("media", {
        "image_description": config.media.image_description,
        "audio_transcription": config.media.audio_transcription,
        "video_description": config.media.video_description,
        "max_concurrency": config.media.max_concurrency,
        "image_provider": config.media.image_provider,
        "audio_provider": config.media.audio_provider,
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

    set!("budget", {
        "max_hourly_usd": config.budget.max_hourly_usd,
        "max_daily_usd": config.budget.max_daily_usd,
        "max_monthly_usd": config.budget.max_monthly_usd,
        "alert_threshold": config.budget.alert_threshold,
        "default_max_llm_tokens_per_hour": config.budget.default_max_llm_tokens_per_hour,
    });

    set!("provider_urls", config.provider_urls);
    set!("provider_api_keys", provider_api_keys);

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
            "language": "string",
            "usage_footer": { "type": "select", "options": ["off", "tokens", "cost", "full"] },
            "stable_prefix_mode": "boolean",
            "prompt_caching": "boolean",
            "max_cron_jobs": "number",
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
        "consolidation_threshold": "number", "decay_rate": "number",
        "embedding_provider": "string", "embedding_api_key_env": "string",
        "consolidation_interval_hours": "number"
    }});
    sec!("proactive_memory", { "fields": {
        "auto_memorize": "boolean", "auto_retrieve": "boolean",
        "max_retrieve": "number", "extraction_threshold": "number",
        "extraction_model": "string", "extract_categories": "array",
        "session_ttl_hours": "number", "confidence_decay_rate": "number",
        "duplicate_threshold": "number"
    }});
    sec!("web", { "fields": {
        "search_provider": { "type": "select", "options": ["brave", "tavily", "perplexity", "duck_duck_go", "auto"] },
        "cache_ttl_minutes": "number"
    }});
    sec!("browser", { "fields": {
        "headless": "boolean", "viewport_width": "number", "viewport_height": "number",
        "timeout_secs": "number", "idle_timeout_secs": "number",
        "max_sessions": "number", "chromium_path": "string"
    }});
    sec!("network", { "fields": {
        "listen_addresses": "string[]", "bootstrap_peers": "string[]",
        "mdns_enabled": "boolean", "max_peers": "number", "shared_secret": "string"
    }});
    sec!("extensions", { "fields": {
        "auto_reconnect": "boolean", "reconnect_max_attempts": "number",
        "reconnect_max_backoff_secs": "number", "health_check_interval_secs": "number"
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
        "video_description": "boolean", "max_concurrency": "number",
        "image_provider": "string", "audio_provider": "string"
    }});
    sec!("links", { "fields": {
        "enabled": "boolean", "max_links": "number",
        "max_content_bytes": "number", "timeout_secs": "number"
    }});
    sec!("reload", { "hot_reloadable": true, "fields": {
        "mode": { "type": "select", "options": ["off", "restart", "hot", "hybrid"] },
        "debounce_ms": "number"
    }});
    sec!("webhook_triggers", { "fields": {
        "enabled": "boolean", "token_env": "string",
        "max_payload_bytes": "number", "rate_limit_per_minute": "number"
    }});
    sec!("approval", { "hot_reloadable": true, "fields": {
        "require_approval": "string[]", "timeout_secs": "number",
        "auto_approve_autonomous": "boolean", "auto_approve": "boolean"
    }});
    sec!("exec_policy", { "fields": {
        "mode": { "type": "select", "options": ["deny", "allowlist", "full"] },
        "safe_bins": "string[]", "allowed_commands": "string[]",
        "timeout_secs": "number", "max_output_bytes": "number",
        "no_output_timeout_secs": "number"
    }});
    sec!("broadcast", { "fields": {
        "strategy": { "type": "select", "options": ["parallel", "sequential"] },
        "routes": "object"
    }});
    sec!("auto_reply", { "fields": {
        "enabled": "boolean", "max_concurrent": "number",
        "timeout_secs": "number", "suppress_patterns": "string[]"
    }});
    sec!("canvas", { "fields": {
        "enabled": "boolean", "max_html_bytes": "number", "allowed_tags": "string[]"
    }});
    sec!("tts", { "fields": {
        "enabled": "boolean",
        "provider": { "type": "select", "options": ["openai", "elevenlabs"] },
        "max_text_length": "number", "timeout_secs": "number"
    }});
    sec!("docker", { "fields": {
        "enabled": "boolean", "image": "string", "container_prefix": "string",
        "workdir": "string", "network": "string", "memory_limit": "string",
        "cpu_limit": "number", "timeout_secs": "number", "read_only_root": "boolean",
        "pids_limit": "number", "reuse_cool_secs": "number",
        "idle_timeout_secs": "number", "max_age_secs": "number"
    }});
    sec!("pairing", { "fields": {
        "enabled": "boolean", "max_devices": "number", "token_expiry_secs": "number",
        "push_provider": { "type": "select", "options": ["none", "ntfy", "gotify"] },
        "ntfy_url": "string", "ntfy_topic": "string"
    }});
    sec!("thinking", { "fields": { "budget_tokens": "number", "stream_thinking": "boolean" }});
    sec!("budget", { "hot_reloadable": true, "fields": {
        "max_hourly_usd": "number", "max_daily_usd": "number",
        "max_monthly_usd": "number", "alert_threshold": "number",
        "default_max_llm_tokens_per_hour": "number"
    }});
    sec!("vertex_ai", { "fields": {
        "project_id": "string", "region": "string", "credentials_path": "string"
    }});
    sec!("oauth", { "fields": {
        "google_client_id": "string", "github_client_id": "string",
        "microsoft_client_id": "string", "slack_client_id": "string"
    }});
    sec!("session", { "fields": {
        "retention_days": "number", "max_sessions_per_agent": "number",
        "cleanup_interval_hours": "number"
    }});
    sec!("queue", { "fields": {
        "max_depth_per_agent": "number", "max_depth_global": "number",
        "task_ttl_secs": "number"
    }});
    sec!("external_auth", { "fields": {
        "enabled": "boolean", "issuer_url": "string", "client_id": "string",
        "client_secret_env": "string", "redirect_url": "string",
        "scopes": "string[]", "allowed_domains": "string[]",
        "audience": "string", "session_ttl_secs": "number"
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
