use super::*;

// ---------------------------------------------------------------------------
// Config endpoint
// ---------------------------------------------------------------------------
/// GET /api/config — Get kernel configuration (secrets redacted).
#[utoipa::path(
    get,
    path = "/api/config",
    tag = "system",
    responses(
        (status = 200, description = "Get kernel configuration (secrets redacted)", body = crate::types::JsonObject)
    )
)]
pub async fn get_config(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Return a redacted view of the kernel config
    let config = state.kernel.config_ref();

    // -- channels: show which platforms are configured (instance counts), no tokens --
    // All previously in-process channels (whatsapp, teams,
    // google_chat, webhook, …) migrated to sidecars; their fields no
    // longer exist on `ChannelsConfig` so there's nothing to
    // enumerate here. The macro shape + lookup are preserved as a
    // comment block so a future in-process channel can rebuild this
    // block by uncommenting + appending one `ch!()` line per field.
    //
    //   let c = &config.channels;
    //   let mut map = serde_json::Map::new();
    //   macro_rules! ch {
    //       ($name:ident) => {{
    //           if !c.$name.is_empty() {
    //               map.insert(
    //                   stringify!($name).to_string(),
    //                   serde_json::json!({ "instances": c.$name.len() }),
    //               );
    //           }
    //       }};
    //   }
    //   ch!(<future_in_process_channel>);
    //   serde_json::Value::Object(map)
    let channels = serde_json::Value::Object(serde_json::Map::new());

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
    set!("agent_max_iterations", config.agent_max_iterations);
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
    set!("web", redacted_web(&config.web));

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
                "trigger_lane": config.queue.concurrency.trigger_lane,
                "default_per_agent": config.queue.concurrency.default_per_agent,
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

    // ── Newly surfaced sections (#4678) ──

    // Top-level scalar additions exposed in the "general" section overlay.
    set!(
        "update_channel",
        serde_json::to_value(config.update_channel).unwrap_or(serde_json::json!("stable"))
    );
    set!("max_history_messages", config.max_history_messages);
    set!("max_upload_size_bytes", config.max_upload_size_bytes);
    set!("max_concurrent_bg_llm", config.max_concurrent_bg_llm);
    set!("max_agent_call_depth", config.max_agent_call_depth);
    set!("max_request_body_bytes", config.max_request_body_bytes);
    set!(
        "workflow_stale_timeout_minutes",
        config.workflow_stale_timeout_minutes
    );
    set!("tool_timeout_secs", config.tool_timeout_secs);
    set!(
        "local_probe_interval_secs",
        config.local_probe_interval_secs
    );
    set!("require_auth_for_reads", config.require_auth_for_reads);
    set!("dashboard_user", config.dashboard_user);
    set!(
        "log_dir",
        config
            .log_dir
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
    );
    set!("cors_origin", config.cors_origin);
    set!("trust_forwarded_for", config.trust_forwarded_for);
    set!("cron_session_max_tokens", config.cron_session_max_tokens);
    set!(
        "cron_session_max_messages",
        config.cron_session_max_messages
    );
    set!(
        "cron_session_warn_fraction",
        config.cron_session_warn_fraction
    );
    set!(
        "cron_session_warn_total_tokens",
        config.cron_session_warn_total_tokens
    );
    set!("strict_config", config.strict_config);

    // ── llm (auxiliary fallback chains; provider:model strings — not secrets) ──
    set!("llm", {
        "auxiliary": serde_json::to_value(&config.llm.auxiliary).unwrap_or(serde_json::json!({})),
    });

    // ── skills ──
    set!("skills", {
        "load_user": config.skills.load_user,
        "extra_dirs": config.skills.extra_dirs.iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<_>>(),
        "disabled": config.skills.disabled,
        "env_passthrough_denied_patterns": config.skills.env_passthrough_denied_patterns,
        "env_passthrough_per_skill": config.skills.env_passthrough_per_skill,
        "registry_repo": config.skills.registry_repo,
    });

    // ── triggers ──
    set!("triggers", {
        "cooldown_secs": config.triggers.cooldown_secs,
        "max_per_event": config.triggers.max_per_event,
        "max_depth": config.triggers.max_depth,
        "max_workflow_secs": config.triggers.max_workflow_secs,
    });

    // ── notification (channel routing — recipients are not secrets, but pass through unchanged) ──
    set!(
        "notification",
        serde_json::to_value(&config.notification).unwrap_or(serde_json::json!({}))
    );

    // ── task_board ──
    set!("task_board", {
        "claim_ttl_secs": config.task_board.claim_ttl_secs,
        "sweep_interval_secs": config.task_board.sweep_interval_secs,
        "max_retries": config.task_board.max_retries,
    });

    // ── tool_policy (rules + groups, no secrets) ──
    set!(
        "tool_policy",
        serde_json::to_value(&config.tool_policy).unwrap_or(serde_json::json!({}))
    );

    // ── context_engine (engine name, plugin paths, hook scripts — no secrets) ──
    set!(
        "context_engine",
        serde_json::to_value(&config.context_engine).unwrap_or(serde_json::json!({}))
    );

    // ── audit ──
    set!("audit", {
        "retention_days": config.audit.retention_days,
        "anchor_path": config.audit.anchor_path.as_ref().map(|p| p.to_string_lossy().to_string()),
        "retention": serde_json::to_value(&config.audit.retention).unwrap_or(serde_json::json!({})),
    });

    // ── health_check ──
    set!("health_check", {
        "health_check_interval_secs": config.health_check.health_check_interval_secs,
    });

    // ── heartbeat ──
    set!("heartbeat", {
        "check_interval_secs": config.heartbeat.check_interval_secs,
        "default_timeout_secs": config.heartbeat.default_timeout_secs,
        "keep_recent": config.heartbeat.keep_recent,
    });

    // ── plugins ──
    set!("plugins", {
        "plugin_registries": config.plugins.plugin_registries,
    });

    // ── registry (mirror URL is not a secret, just a public proxy prefix) ──
    set!("registry", {
        "cache_ttl_secs": config.registry.cache_ttl_secs,
        "registry_mirror": config.registry.registry_mirror,
    });

    // ── privacy ──
    set!("privacy", {
        "mode": serde_json::to_value(&config.privacy.mode).unwrap_or(serde_json::json!("off")),
        "redact_patterns": config.privacy.redact_patterns,
    });

    // ── sanitize ──
    set!(
        "sanitize",
        serde_json::to_value(&config.sanitize).unwrap_or(serde_json::json!({}))
    );

    // ── inbox ──
    set!("inbox", {
        "enabled": config.inbox.enabled,
        "directory": config.inbox.directory,
        "poll_interval_secs": config.inbox.poll_interval_secs,
        "default_agent": config.inbox.default_agent,
    });

    // ── telemetry (otlp_endpoint may carry credentials in URL; keep host/port only) ──
    set!("telemetry", {
        "enabled": config.telemetry.enabled,
        "otlp_endpoint": redact_url_credentials(&config.telemetry.otlp_endpoint),
        "service_name": config.telemetry.service_name,
        "sample_rate": config.telemetry.sample_rate,
        "prometheus_enabled": config.telemetry.prometheus_enabled,
        "auto_start_observability_stack": config.telemetry.auto_start_observability_stack,
        "emit_caller_trace_headers": config.telemetry.emit_caller_trace_headers,
    });

    // ── prompt_intelligence ──
    set!("prompt_intelligence", {
        "enabled": config.prompt_intelligence.enabled,
        "hash_prompts": config.prompt_intelligence.hash_prompts,
        "max_versions_per_agent": config.prompt_intelligence.max_versions_per_agent,
    });

    // ── rate_limit ──
    set!("rate_limit", {
        "api_requests_per_minute": config.rate_limit.api_requests_per_minute,
        "retry_after_secs": config.rate_limit.retry_after_secs,
        "max_ws_per_ip": config.rate_limit.max_ws_per_ip,
        "ws_messages_per_minute": config.rate_limit.ws_messages_per_minute,
        "ws_terminal_messages_per_minute": config.rate_limit.ws_terminal_messages_per_minute,
        "ws_idle_timeout_secs": config.rate_limit.ws_idle_timeout_secs,
        "ws_debounce_ms": config.rate_limit.ws_debounce_ms,
        "ws_debounce_chars": config.rate_limit.ws_debounce_chars,
        "auth_rate_limit_per_ip": config.rate_limit.auth_rate_limit_per_ip,
    });

    // ── tool_invoke ──
    set!("tool_invoke", {
        "enabled": config.tool_invoke.enabled,
        "allowlist": config.tool_invoke.allowlist,
    });

    // ── parallel_tools ──
    set!("parallel_tools", {
        "enabled": config.parallel_tools.enabled,
        "max_concurrent": config.parallel_tools.max_concurrent,
        "mcp_default_safety": config.parallel_tools.mcp_default_safety,
        "mcp_readonly_allowlist": config.parallel_tools.mcp_readonly_allowlist,
    });

    // ── tool_results ──
    set!("tool_results", {
        "spill_threshold_bytes": config.tool_results.spill_threshold_bytes,
        "max_artifact_bytes": config.tool_results.max_artifact_bytes,
        "max_bytes_per_turn": config.tool_results.max_bytes_per_turn,
        "history_fold_after_turns": config.tool_results.history_fold_after_turns,
        "fold_min_batch_size": config.tool_results.fold_min_batch_size,
        "artifact_max_age_days": config.tool_results.artifact_max_age_days,
    });

    // ── compaction ──
    set!("compaction", {
        "threshold_messages": config.compaction.threshold_messages,
        "keep_recent": config.compaction.keep_recent,
        "max_summary_tokens": config.compaction.max_summary_tokens,
        "token_threshold_ratio": config.compaction.token_threshold_ratio,
        "max_chunk_chars": config.compaction.max_chunk_chars,
        "max_retries": config.compaction.max_retries,
    });

    // ── azure_openai (endpoint URL may identify a tenant; keep as-is, deployment is non-secret) ──
    set!("azure_openai", {
        "endpoint": config.azure_openai.endpoint,
        "api_version": config.azure_openai.api_version,
        "deployment": config.azure_openai.deployment,
    });

    // ── proxy (URLs may carry user:pass — strip credentials before exposing) ──
    set!("proxy", {
        "http_proxy": config.proxy.http_proxy.as_deref().map(librefang_types::config::redact_proxy_url),
        "https_proxy": config.proxy.https_proxy.as_deref().map(librefang_types::config::redact_proxy_url),
        "no_proxy": config.proxy.no_proxy,
    });

    // ── taint_rules: pass-through (rule names + actions; no secrets) ──
    set!(
        "taint_rules",
        serde_json::to_value(&config.taint_rules).unwrap_or(serde_json::json!([]))
    );

    // ── sidecar_channels (already redacted above — env_keys only, no values) ──
    set!("sidecar_channels", sidecar_channels);

    // ── Provider URL/region/timeout maps (#4678): non-secret, pass-through ──
    set!(
        "provider_request_timeout_secs",
        config.provider_request_timeout_secs
    );
    set!("provider_max_retries", config.provider_max_retries);
    // Note: `provider_urls`, `provider_proxy_urls`, `provider_regions`, and
    // `provider_api_keys` are already inserted above. `tool_timeouts`:
    set!("tool_timeouts", config.tool_timeouts);

    Json(serde_json::Value::Object(out))
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
        (status = 200, description = "Reload configuration from disk", body = crate::types::JsonObject)
    )
)]
pub async fn config_reload(
    State(state): State<Arc<AppState>>,
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
) -> impl IntoResponse {
    // SECURITY: Record config reload in audit trail with caller attribution.
    let user_id = api_user.as_ref().map(|u| u.0.user_id);
    state.kernel.audit().record_with_context(
        "system",
        librefang_kernel::audit::AuditAction::ConfigChange,
        "config reload requested via API",
        "pending",
        user_id,
        Some("api".to_string()),
    );
    match state.kernel.reload_config().await {
        Ok(plan) => {
            // If channel config changed, the kernel already cleared the adapter
            // registry — but we also need to stop the old BridgeManager and
            // restart adapters from the new config.
            if plan.hot_actions.contains(&HotAction::ReloadChannels) {
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

            // Phase 9 (C-006): reload_config re-read config.toml and, via its
            // ReloadMcpServers hot action, transiently set the MCP list back to
            // the bootstrap file values — which would clobber DB-resolved state
            // (UI edits). Re-run the same seed → overlay → reconcile pipeline as
            // boot so the config store stays authoritative: seed picks up any
            // genuine config.toml change (provenance-aware, never clobbering a
            // runtime row), overlay pushes the store-resolved list back into the
            // kernel, and reload_mcp_servers reconciles live connections to it.
            #[cfg(feature = "surreal-backend")]
            {
                crate::config_store_overlay::seed_config_store(state.kernel.as_ref()).await;
                crate::config_store_overlay::overlay_mcp_servers(state.kernel.as_ref()).await;
                crate::config_store_overlay::overlay_default_model(state.kernel.as_ref()).await;
                if let Err(e) = state.kernel.clone().reload_mcp_servers().await {
                    tracing::warn!(error = %e, "config reload: MCP reconcile after overlay failed");
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
                // Scrub the io error (audit: rusqlite-errors-leak).
                tracing::error!(error = %e, "failed to read config for export");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    [(axum::http::header::CONTENT_TYPE, "application/json")],
                    Body::from(
                        serde_json::json!({"status": "error", "error": "Internal server error"})
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
                // Scrub the serialize error (audit: rusqlite-errors-leak).
                tracing::error!(error = %e, "failed to serialize config for export");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    [(axum::http::header::CONTENT_TYPE, "application/json")],
                    Body::from(
                        serde_json::json!({"status": "error", "error": "Internal server error"})
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
        (status = 200, description = "Get config structure schema", body = crate::types::JsonObject)
    )
)]
pub async fn config_schema(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Build the draft-07 JSON Schema directly from `KernelConfig` via
    // `schemars`, then apply a small overlay for UI-only metadata that the
    // struct cannot carry: curated select options with multi-locale labels,
    // numeric `min`/`max`/`step` ranges, section grouping, dynamic provider
    // and model options pulled from the live catalog.
    //
    // Return shape extends draft-07 with two custom extensions:
    //   - `x-sections` — ordered list of UI section groupings. Each entry
    //     has `{ key, title?, root_level?, struct_field?, hot_reloadable?,
    //     fields: [...], virtual: bool }`. `virtual = true` collects
    //     top-level KernelConfig fields into a synthetic "general" section.
    //   - `x-ui-options` — per-field UI hints mapped by JSON-pointer path.
    //     Carries `{ select?, number_select?, min?, max?, step?, placeholder? }`.
    //
    // Replaces a 245-line hand-authored schema (issue #3048 follow-up).
    let catalog = state.kernel.model_catalog_ref().load();
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

    // Generate the base draft-07 schema.
    let mut root =
        serde_json::to_value(schemars::schema_for!(librefang_types::config::KernelConfig))
            .unwrap_or_else(|_| serde_json::json!({}));

    // Attach the UI overlay: sections + option/range hints.
    if let Some(obj) = root.as_object_mut() {
        obj.insert("x-sections".into(), ui_sections_overlay());
        obj.insert(
            "x-ui-options".into(),
            ui_options_overlay(provider_options, model_options),
        );
    }

    Json(root)
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
    request_body(content = crate::types::JsonObject, description = "`{ \"path\": \"section.key\", \"value\": ... }`"),
    responses(
        (status = 200, description = "Set a single config value and persist", body = crate::types::JsonObject)
    )
)]
pub async fn config_set(
    State(state): State<Arc<AppState>>,
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
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

    // SECURITY #3458: Validate the config key path before touching any files.
    // Each dot-separated component must only contain alphanumeric characters
    // and underscores.  This prevents:
    //   - Path traversal (e.g. "../secrets")
    //   - Injection into structured TOML tables via special characters
    //   - Empty segment attacks (e.g. "section..key")
    //
    // The path string itself is never used as a filesystem path — it is only
    // used as a key chain into the in-memory TOML document — but we validate
    // early to fail fast and to document the expected namespace.
    fn validate_config_key_path(path: &str) -> Result<(), String> {
        if path.is_empty() {
            return Err("config path must not be empty".to_string());
        }
        // Reject absolute paths and filesystem separators outright.
        if path.starts_with('/') || path.starts_with('\\') || path.contains("..") {
            return Err(format!(
                "config path '{path}' is not a valid key path (no filesystem separators allowed)"
            ));
        }
        for part in path.split('.') {
            if part.is_empty() {
                return Err(format!("config path '{path}' contains an empty segment"));
            }
            if !part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                return Err(format!(
                    "config path segment '{part}' contains disallowed characters \
                     (only ASCII alphanumeric, '_', and '-' are permitted)"
                ));
            }
        }
        Ok(())
    }

    if let Err(e) = validate_config_key_path(&path) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"status": "error", "error": e})),
        );
    }

    // SECURITY (#3458): Restrict /api/config/set to a curated allowlist of
    // user-tunable config paths. Without this gate any caller authorized to
    // change config (Owner role, post-auth) can clobber structured tables
    // (e.g. overwrite `[channels]` with a string), corrupt nested credentials
    // (`default_model.api_key`), or flip security-critical flags
    // (`auth.bypass = true` style). The allowlist deliberately excludes:
    //   - auth/credentials/api_key/users     (account takeover)
    //   - default_model / providers / *.api_key  (silent provider hijack)
    //   - approval / second_factor / totp_*  (2FA bypass)
    //   - migration_state / schema_version   (DB corruption)
    //   - network / shared_secret / cors_*   (federation hijack)
    // Operators who genuinely need those paths must edit `config.toml` on
    // disk — that path keeps an audit trail (file mtime, git, etc.) and
    // requires shell access, raising the bar above a leaked API key.
    if !is_writable_config_path(&path) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "status": "error",
                "error": format!(
                    "config path '{path}' is not user-tunable via /api/config/set; \
                     edit ~/.librefang/config.toml directly to change it"
                )
            })),
        );
    }

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

    // Read existing config — use toml_edit to preserve comments and formatting.
    // A read failure on an existing file (permission denied, hardware fault,
    // …) MUST abort — falling back to "" would silently drop every other
    // section in `config.toml` (agents, providers, taint rules, …) on the
    // next write. Same protection as `users::persist_users` (#3368).
    let raw_content = if config_path.exists() {
        match std::fs::read_to_string(&config_path) {
            Ok(s) => s,
            Err(e) => {
                // Scrub the io error (audit: rusqlite-errors-leak) —
                // path / permission detail stays in the log.
                tracing::error!(error = %e, "could not read existing config.toml");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "status": "error",
                        "error": "Internal server error"
                    })),
                );
            }
        }
    } else {
        String::new()
    };
    // Parse failure means the on-disk file is already corrupt — refuse to
    // write rather than overwriting with an empty document, which would
    // clobber every other section the operator is hand-editing (#3368).
    let mut doc: toml_edit::DocumentMut = match raw_content.parse() {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "status": "error",
                    "error": format!(
                        "config.toml has a syntax error and cannot be safely edited \
                         from the dashboard. Fix the file manually first: {e}"
                    )
                })),
            );
        }
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
    if let Err(errors) = validate_config_for_reload(&parsed_config) {
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
    if let Err(e) = crate::atomic_write(&config_path, new_toml_str.as_bytes()) {
        // Scrub the io error (audit: rusqlite-errors-leak).
        tracing::error!(error = %e, "failed to write config.toml");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"status": "error", "error": "Internal server error"})),
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

    let user_id = api_user.as_ref().map(|u| u.0.user_id);
    state.kernel.audit().record_with_context(
        "system",
        librefang_kernel::audit::AuditAction::ConfigChange,
        format!("config set: {path}"),
        "completed",
        user_id,
        Some("api".to_string()),
    );

    let mut body = serde_json::json!({"status": reload_status, "path": path});
    if let Some(err) = reload_error {
        body["reload_error"] = serde_json::Value::String(err);
    }
    (StatusCode::OK, Json(body))
}
