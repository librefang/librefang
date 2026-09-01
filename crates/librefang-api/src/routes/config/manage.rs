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
    let config = state.kernel.config_ref();
    let budget = state.kernel.budget_config();
    Json(redacted_config_json(&config, &budget))
}

/// Build the redacted body returned by `GET /api/config`.
///
/// Split out of the handler so the read/write parity guard can render a payload without booting a kernel.
///
/// Every leaf that `is_writable_config_path` accepts MUST be represented here (redacted where the value is a secret, but present as a key).
/// The dashboard populates its form fields from this payload, so a writable field missing from the read side renders blank and reads back as "not configured" even immediately after a successful save (#6596).
/// `config_read_write_parity_tests::every_writable_config_leaf_is_readable` fails the build when the two sides drift apart again.
fn redacted_config_json(
    config: &librefang_types::config::KernelConfig,
    budget: &librefang_types::config::BudgetConfig,
) -> serde_json::Value {
    // -- channels: file-transfer limits + which platforms are configured (instance counts), no tokens --
    // All previously in-process channels (whatsapp, teams, google_chat, webhook, …) migrated to sidecars; their per-vendor fields no longer exist on `ChannelsConfig`, so only its file-transfer scalars are enumerated here.
    // The macro shape + lookup are preserved as a comment block so a future in-process channel can rebuild the per-vendor part: uncomment the block, append one `ch!()` line per field, then bind `channels` below as `let mut` and fold `map` into it with `channels.as_object_mut().expect("object").extend(map)`.
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
    //
    // The file-transfer scalars are non-secret and the dashboard declares a `channels` section for them, so omitting them rendered the section blank (#6596).
    let channels = serde_json::json!({
        "file_download_max_bytes": config.channels.file_download_max_bytes,
        "file_download_dir": config.channels.file_download_dir,
        "file_upload_max_bytes": config.channels.file_upload_max_bytes,
    });

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
    // Every field of `OidcProvider` is enumerated here.
    // None of them holds a secret value: `client_secret_env` is the *name* of the environment variable the secret is read from (the secret itself is never stored in config), and `client_id` is the public half of the OAuth client registration — both were already emitted before #6605.
    // `auth_url` / `token_url` / `userinfo_url` / `jwks_uri` / `audience` / `require_email_verified` were missing, which left an operator unable to tell from the API whether a non-OIDC provider's explicit endpoints were picked up or whether this provider overrides the global email-verification gate (#6605).
    // These stay read-only: `external_auth.` is deliberately absent from `WRITABLE_SECTION_PREFIXES`, because flipping an endpoint or the verification gate post-auth is the #3703 impersonation vector.
    let external_auth_providers: Vec<serde_json::Value> = config
        .external_auth
        .providers
        .iter()
        .map(|p| {
            serde_json::json!({
                "id": p.id,
                "display_name": p.display_name,
                "issuer_url": p.issuer_url,
                "auth_url": p.auth_url,
                "token_url": p.token_url,
                "userinfo_url": p.userinfo_url,
                "jwks_uri": p.jwks_uri,
                "client_id": p.client_id,
                "client_secret_env": p.client_secret_env,
                "redirect_url": p.redirect_url,
                "scopes": p.scopes,
                "allowed_domains": p.allowed_domains,
                "audience": p.audience,
                // `None` renders as `null` — "inherit the global setting", which is a different state from an explicit `false` and must stay distinguishable.
                "require_email_verified": p.require_email_verified,
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
    // Serde encoding, not `Debug`: `KernelMode` is `rename_all = "snake_case"`, so the write path and the schema's select options both speak `"stable" | "default" | "dev"`.
    // Emitting `Debug`'s `"Default"` here left the dashboard dropdown with a value none of its options matched (#6596).
    set!(
        "mode",
        serde_json::to_value(config.mode).unwrap_or(serde_json::json!("default"))
    );
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
    // `extra_params` is deliberately absent: it is `#[serde(flatten)]`, so it has no key of its own on the wire — its entries live directly on the `[default_model]` table.
    // Inventing a nested `extra_params` object here would put a shape in the read payload that neither config.toml nor the generated schema has.
    set!("default_model", {
        "provider": config.default_model.provider,
        "model": config.default_model.model,
        "api_key_env": config.default_model.api_key_env,
        "base_url": config.default_model.base_url,
        "message_timeout_secs": config.default_model.message_timeout_secs,
        "cli_profile_dirs": config.default_model.cli_profile_dirs,
    });

    // ── Memory ──
    set!("memory", {
        "sqlite_path": config.memory.sqlite_path.as_ref().map(|p| p.to_string_lossy().to_string()),
        "embedding_model": config.memory.embedding_model,
        "consolidation_threshold": config.memory.consolidation_threshold,
        "decay_rate": config.memory.decay_rate,
        "embedding_provider": config.memory.embedding_provider,
        "embedding_api_key_env": config.memory.embedding_api_key_env,
        "embedding_dimensions": config.memory.embedding_dimensions,
        "consolidation_interval_hours": config.memory.consolidation_interval_hours,
        "fts_only": config.memory.fts_only,
        "decay": serde_json::to_value(&config.memory.decay).unwrap_or_default(),
        "chunking": serde_json::to_value(&config.memory.chunking).unwrap_or_default(),
        "vector_backend": config.memory.vector_backend,
        "vector_store_url": config.memory.vector_store_url,
        "soft_delete_retention_days": config.memory.soft_delete_retention_days,
        "pool_size": config.memory.pool_size,
        "max_episodic_chars": config.memory.max_episodic_chars,
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
        "format_context_max_chars": config.proactive_memory.format_context_max_chars,
        "update_threshold_same_category": config.proactive_memory.update_threshold_same_category,
        "update_threshold_cross_category": config.proactive_memory.update_threshold_cross_category,
        "extractor_sidecar": serde_json::to_value(&config.proactive_memory.extractor_sidecar).unwrap_or_default(),
        "session_scoped_recall": config.proactive_memory.session_scoped_recall,
        "min_similarity": config.proactive_memory.min_similarity,
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
        "max_messages_per_peer_per_minute": config.network.max_messages_per_peer_per_minute,
        "max_llm_tokens_per_peer_per_hour": config.network.max_llm_tokens_per_peer_per_hour,
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
                "name": a2a.name,
                "description": a2a.description,
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

    // `cdp_auth_token_env` is an env-var *name*, not a token — same treatment as `default_model.api_key_env`, which the dashboard has always shown.
    set!("browser", {
        "enabled": config.browser.enabled,
        "headless": config.browser.headless,
        "viewport_width": config.browser.viewport_width,
        "viewport_height": config.browser.viewport_height,
        "timeout_secs": config.browser.timeout_secs,
        "idle_timeout_secs": config.browser.idle_timeout_secs,
        "max_sessions": config.browser.max_sessions,
        "chromium_path": config.browser.chromium_path,
        "cdp_endpoint": config.browser.cdp_endpoint,
        "cdp_auth_token_env": config.browser.cdp_auth_token_env,
        "max_content_chars": config.browser.max_content_chars,
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
        "use_os_keyring": config.vault.use_os_keyring,
    });

    let stt_available = config.media.audio_provider.is_some();
    // `custom_stt` is serialized wholesale: `CustomSttConfig` carries only a base URL, an env-var name, a bool and a model id — no secret values.
    set!("media", {
        "image_description": config.media.image_description,
        "audio_transcription": config.media.audio_transcription,
        "video_description": config.media.video_description,
        "max_concurrency": config.media.max_concurrency,
        "transcription_timeout_secs": config.media.transcription_timeout_secs,
        "ffmpeg_timeout_secs": config.media.ffmpeg_timeout_secs,
        "image_provider": config.media.image_provider,
        "image_model": config.media.image_model,
        "audio_provider": config.media.audio_provider,
        "audio_model": config.media.audio_model,
        "audio_language": config.media.audio_language,
        "audio_prompt": config.media.audio_prompt,
        "video_provider": config.media.video_provider,
        "video_model": config.media.video_model,
        "custom_stt": serde_json::to_value(&config.media.custom_stt).unwrap_or_default(),
        "custom_image": serde_json::to_value(&config.media.custom_image).unwrap_or_default(),
        "custom_video": serde_json::to_value(&config.media.custom_video).unwrap_or_default(),
        "stt_available": stt_available,
    });

    set!("links", {
        "enabled": config.links.enabled,
        "max_links": config.links.max_links,
        "max_content_bytes": config.links.max_content_bytes,
        "timeout_secs": config.links.timeout_secs,
    });

    // `ReloadMode` is `rename_all = "snake_case"` — emit the serde form the write path accepts, not `Debug`'s capitalised variant name.
    set!("reload", {
        "mode": serde_json::to_value(config.reload.mode).unwrap_or(serde_json::json!("hybrid")),
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

    // The `approval` section declares no explicit `fields` list in `ui_sections_overlay`, so the dashboard renders a control for every `ApprovalPolicy` field the derived schema knows about — and seven of the fourteen were absent here, rendering blank and reading back as their JSON zero value.
    // `cache_approvals_per_session` was the visible one: it defaults to `true`, so an operator who left it alone still saw the box unchecked and had no way to tell whether per-session caching was on (#6636 observation (e)).
    // None of the seven is writable, which is why `every_writable_config_leaf_is_readable` never saw them; `approval_policy_fields_are_all_readable` is the guard for this direction.
    // Nothing added here is secret-bearing: `trusted_senders`, `channel_rules`, and `routing` hold operator-chosen user ids, channel names, tool globs, and notification recipients — the same class as the `external_auth` fields exposed in #6605, and credentials never live in `ApprovalPolicy`.
    // Of those seven, `trusted_senders` is the one with teeth: it is an approval-*bypass* list, so a sender on it skips the prompt for every tool `classify_risk` ranks below `High` (see `ApprovalManager::requires_approval_with_context_for`), which is why the Approvals page renders it as its own card rather than leaving it to the generic config form (#6611).
    // Read-only on purpose — it stays out of `WRITABLE_EXACT_PATHS` so an Owner-role caller holding a leaked API key cannot add themselves to it over HTTP, which is also why the `writable ⊆ readable` guard never noticed while it was missing from this response.
    set!("approval", {
        "require_approval": config.approval.require_approval,
        "timeout_secs": config.approval.timeout_secs,
        "auto_approve_autonomous": config.approval.auto_approve_autonomous,
        "auto_approve": config.approval.auto_approve,
        "trusted_senders": config.approval.trusted_senders,
        "channel_rules": serde_json::to_value(&config.approval.channel_rules).unwrap_or(serde_json::json!([])),
        // Serde encoding, not `Debug` — see `enum_valued_fields_use_the_serde_encoding_not_debug`.
        "timeout_fallback": serde_json::to_value(&config.approval.timeout_fallback).unwrap_or(serde_json::json!("deny")),
        "routing": serde_json::to_value(&config.approval.routing).unwrap_or(serde_json::json!([])),
        "second_factor": serde_json::to_value(config.approval.second_factor).unwrap_or(serde_json::json!("none")),
        "totp_issuer": config.approval.totp_issuer,
        "totp_grace_period_secs": config.approval.totp_grace_period_secs,
        "totp_tools": config.approval.totp_tools,
        "audit_retention_days": config.approval.audit_retention_days,
        "cache_approvals_per_session": config.approval.cache_approvals_per_session,
    });

    // `ExecSecurityMode` is `rename_all = "lowercase"` — the schema's select offers `deny | allowlist | full`, so `Debug`'s `"Allowlist"` never matched an option.
    set!("exec_policy", {
        "mode": serde_json::to_value(config.exec_policy.mode).unwrap_or(serde_json::json!("allowlist")),
        "safe_bins": config.exec_policy.safe_bins,
        "safe_bins_skip_approval": config.exec_policy.safe_bins_skip_approval,
        "full_mode_skips_approval": config.exec_policy.full_mode_skips_approval,
        "allowed_commands": config.exec_policy.allowed_commands,
        "allowed_env_vars": config.exec_policy.allowed_env_vars,
        "timeout_secs": config.exec_policy.timeout_secs,
        "max_output_bytes": config.exec_policy.max_output_bytes,
        "no_output_timeout_secs": config.exec_policy.no_output_timeout_secs,
    });

    // ── Terminal access controls ──
    // The whole section was missing from the read side even though `ui_sections_overlay` declares it and `terminal.` is a writable section prefix, so the dashboard rendered every field blank and a save silently read back as unset (#6596).
    // None of these are secrets: the two remote-access booleans were already mutable through `POST /api/config/set`, and showing an operator the value they can already change is strictly weaker than the existing write surface.
    set!("terminal", {
        "enabled": config.terminal.enabled,
        "allowed_origins": config.terminal.allowed_origins,
        "allow_remote": config.terminal.allow_remote,
        "require_proxy_headers": config.terminal.require_proxy_headers,
        "allow_unauthenticated_remote": config.terminal.allow_unauthenticated_remote,
        "tmux_enabled": config.terminal.tmux_enabled,
        "max_windows": config.terminal.max_windows,
        "tmux_binary_path": config.terminal.tmux_binary_path,
    });

    set!("bindings", bindings);

    // `BroadcastStrategy` is `rename_all = "lowercase"`.
    set!("broadcast", {
        "strategy": serde_json::to_value(config.broadcast.strategy).unwrap_or(serde_json::json!("parallel")),
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
                "output_format": config.tts.elevenlabs.output_format,
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
        // `CustomTtsConfig` mirrors `CustomSttConfig`: base URL, env-var name, bool, model / voice / format ids — no secret values.
        tts.insert(
            "custom".into(),
            serde_json::to_value(&config.tts.custom).unwrap_or_default(),
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
        // Both enums are `rename_all = "snake_case"`; `Debug` would emit `"NonMain"` where the write path and the schema expect `"non_main"`.
        docker.insert(
            "mode".into(),
            serde_json::to_value(config.docker.mode).unwrap_or(serde_json::json!("off")),
        );
        docker.insert(
            "scope".into(),
            serde_json::to_value(config.docker.scope).unwrap_or(serde_json::json!("session")),
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
        "public_base_url": config.pairing.public_base_url,
        "push_provider": config.pairing.push_provider,
        "ntfy_url": config
            .pairing
            .ntfy_url
            .as_deref()
            .map(redact_url_credentials),
        "ntfy_topic": config.pairing.ntfy_topic,
    });

    set!("auth_profiles", auth_profiles);

    out.insert(
        "thinking".into(),
        match &config.thinking {
            Some(t) => serde_json::json!({
                "budget_tokens": t.budget_tokens,
                "stream_thinking": t.stream_thinking,
                // #7946. Serialized through `ReasoningMode`'s serde form, not
                // `Debug`, so the dashboard receives one of the values its
                // dropdown offers — see
                // `enum_valued_fields_use_the_serde_encoding_not_debug`.
                "reasoning_mode": t.reasoning_mode,
            }),
            None => serde_json::json!(null),
        },
    );

    // Budget is read from the kernel's live `BudgetConfig` rather than `config.budget`: `/api/budget` mutations update the former in place.
    set!("budget", {
        "max_hourly_usd": budget.max_hourly_usd,
        "max_daily_usd": budget.max_daily_usd,
        "max_monthly_usd": budget.max_monthly_usd,
        "alert_threshold": budget.alert_threshold,
        "default_max_llm_tokens_per_hour": budget.default_max_llm_tokens_per_hour,
        "default_burst_ratio": budget.default_burst_ratio,
        "providers": serde_json::to_value(&budget.providers).unwrap_or_default(),
    });

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
        "reset_prompt": config.session.reset_prompt,
        "context_injection": serde_json::to_value(&config.session.context_injection).unwrap_or_default(),
        "on_session_start_script": config.session.on_session_start_script,
        "reset": serde_json::to_value(&config.session.reset).unwrap_or_default(),
    });

    set!("queue", {
        "max_depth_per_agent": config.queue.max_depth_per_agent,
        "max_depth_global": config.queue.max_depth_global,
        "task_ttl_secs": config.queue.task_ttl_secs,
        "task_queue_retention_days": config.queue.task_queue_retention_days,
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
                "trigger_fire_timeout_secs": config.queue.concurrency.trigger_fire_timeout_secs,
            }),
        );
    }

    set!("usage", {
        "retention_days": config.usage.retention_days,
    });

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
        // Read-only by design, and absent from this payload entirely until #6605.
        // `external_auth.require_email_verified` is the #3703 mitigation — it rejects logins whose ID token does not carry `email_verified = true`, which is what stops an attacker registering an unverified address in an `allowed_domains` domain and inheriting that domain's authorization.
        // It stays out of `WRITABLE_EXACT_PATHS` / `WRITABLE_SECTION_PREFIXES` precisely so an Owner-role caller with a leaked API key cannot turn it off, but the write-side exclusion is not a reason to hide the value: an operator otherwise has no way short of reading `config.toml` on the host to confirm the protection is on.
        // The read/write parity guard cannot catch this class — it enforces `writable ⊆ readable`, and a field that is intentionally non-writable sits outside that invariant by construction.
        ea.insert(
            "require_email_verified".into(),
            serde_json::json!(config.external_auth.require_email_verified),
        );
        // Read-only for the same reason as `require_email_verified`, and readable for the same reason too (#7744).
        // `role_map` is what turns a signed ID token into an API credential, so a caller who could write it could grant themselves Owner by naming a claim they already hold; it stays out of the writable sets.
        // Reading it back is how an operator confirms which IdP groups currently carry privilege — the values are group names the operator chose, never secrets.
        ea.insert(
            "role_map".into(),
            serde_json::json!(config.external_auth.role_map),
        );
        // #7746: read-only and readable for exactly the same pair of reasons.
        // `group_map` decides which local `[[groups]]` an IdP claim confers, and a group confers ownership and the role strings channel binding matches on, so a caller who could write it could join themselves to any team; `claim_paths` decides *where* the claim values both maps are matched against come from, and pointing it at an attacker-controlled claim would be the same escalation one level up.
        // Both are group and claim names an operator chose, never secrets, and reading them back is how an operator confirms which IdP groups currently confer membership and which part of the token is being trusted.
        ea.insert(
            "group_map".into(),
            serde_json::json!(config.external_auth.group_map),
        );
        ea.insert(
            "claim_paths".into(),
            serde_json::json!(config.external_auth.claim_paths),
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
        "assignee_wake": config.task_board.assignee_wake,
        "pending_grace_secs": config.task_board.pending_grace_secs,
        "wake_backoff_max_secs": config.task_board.wake_backoff_max_secs,
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
        "registry_host": config.registry.registry_host,
        "auto_sync": config.registry.auto_sync,
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
        "aggregate_developer_loops": config.compaction.aggregate_developer_loops,
        "max_loop_steps_before_aggregate": config.compaction.max_loop_steps_before_aggregate,
        "strip_reasoning_after_turns": config.compaction.strip_reasoning_after_turns,
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

    serde_json::Value::Object(out)
}

// ── Model Catalog Endpoints ─────────────────────────────────────────

// ---------------------------------------------------------------------------
// Config Reload endpoint
// ---------------------------------------------------------------------------
fn config_reload_status(
    restart_required: bool,
    has_changes: bool,
    channel_reload_failed: bool,
) -> &'static str {
    if restart_required || channel_reload_failed {
        "partial"
    } else if has_changes {
        "applied"
    } else {
        "no_changes"
    }
}

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
    let user_id = api_user.as_ref().map(|u| u.0.user_id);
    match state.kernel.reload_config().await {
        Ok(plan) => {
            // `api_key` / `api_key_hash` are classified as read-live in
            // `build_reload_plan`, which is true for the WS and terminal
            // upgrade paths (they call `valid_api_tokens(&auth_snapshot())`
            // per connection) but was never true for the HTTP middleware:
            // `api_key_lock` was written only at boot and on a dashboard
            // credential change, so a reloaded master key kept authenticating
            // with the old value until the daemon restarted. Push the fresh
            // snapshot into both live handles here (#6613).
            let snap = state.kernel.auth_snapshot();
            crate::server::refresh_master_credential(&snap, &state.api_key_lock, &state.master_key)
                .await;

            // If channel config changed, the kernel already cleared the adapter
            // registry — but we also need to stop the old BridgeManager and
            // restart adapters from the new config.
            let mut warnings = Vec::new();
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
                        warnings.push(
                            "Channel adapters could not be restarted; see server logs".to_string(),
                        );
                    }
                }
            }

            let status = config_reload_status(
                plan.restart_required,
                plan.has_changes(),
                !warnings.is_empty(),
            );
            state.kernel.audit().record_with_context(
                "system",
                librefang_kernel::audit::AuditAction::ConfigChange,
                "config reload requested via API",
                status,
                user_id,
                Some("api".to_string()),
            );

            let mut body = serde_json::json!({
                "status": status,
                "restart_required": plan.restart_required,
                "restart_reasons": plan.restart_reasons,
                "hot_actions_applied": plan.hot_actions.iter().map(|a| format!("{a:?}")).collect::<Vec<_>>(),
                "noop_changes": plan.noop_changes,
            });
            if !warnings.is_empty() {
                body["warnings"] = serde_json::json!(warnings);
            }

            (StatusCode::OK, Json(body))
        }
        Err(e) => {
            state.kernel.audit().record_with_context(
                "system",
                librefang_kernel::audit::AuditAction::ConfigChange,
                "config reload requested via API",
                "failed",
                user_id,
                Some("api".to_string()),
            );
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"status": "error", "error": e})),
            )
        }
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

    let config_path = state.kernel.config_path().to_path_buf();

    let toml_content = match tokio::fs::read_to_string(&config_path).await {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Fall back to serializing in-memory config only when there is no
            // persisted file to export.
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
        }
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
/// GET /api/config/status — where the effective configuration came from, and whether it can be written.
///
/// The dashboard branches on `writable` to decide whether to render write controls, rather than discovering managed mode by attempting a save and reading the `423` back (#6695).
/// Authenticated like every other `/api/*` route; it exposes a path and a checksum over the file's bytes, never a value from inside it.
#[utoipa::path(
    get,
    path = "/api/config/status",
    tag = "system",
    responses(
        (status = 200, description = "Configuration provenance and writability", body = crate::types::JsonObject)
    )
)]
pub async fn config_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // The kernel's resolved path, not a second resolution: `source` is the file an operator will go and edit, and a status endpoint that names a different one is worse than no status endpoint (#6695).
    axum::Json(librefang_kernel::config::config_provenance(Some(
        state.kernel.config_path(),
    )))
}

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
    crate::openrouter_catalog::refresh_if_missing_in_background(&state.kernel);
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

    // Attach the UI overlay: sections + option/range hints + read-only paths.
    let non_writable = non_writable_schema_paths(&root);
    if let Some(obj) = root.as_object_mut() {
        obj.insert("x-sections".into(), ui_sections_overlay());
        obj.insert(
            "x-ui-options".into(),
            ui_options_overlay(provider_options, model_options),
        );
        obj.insert("x-non-writable".into(), serde_json::json!(non_writable));
    }

    Json(root)
}

/// Every schema path `POST /api/config/set` would reject, so the dashboard can render those fields read-only instead of offering an edit that 403s (#6636 observation (d)).
///
/// The server sends the resolved verdict rather than the allowlists themselves.
/// `is_writable_config_path` is not a lookup — it layers an exact-path list, section prefixes, a depth-2-only rule for some of those prefixes, and a suffix scrub for secret-bearing and privilege-bearing key names.
/// Re-implementing that in TypeScript would make the SPA a third place to keep in sync with the two Rust lists, and it would drift silently: the UI would grey out the wrong fields while the write path kept its own opinion.
///
/// The oracle is the schema, not a serialized config, for the same reason `every_writable_allowlist_entry_has_a_backing_config_field` chose it: `config/types.rs` carries dozens of `#[serde(skip_serializing_if = …)]` attributes, so a value walk cannot see a field whose predicate holds for its default.
///
/// Depth mirrors what the allowlist accepts — root leaves and one nested level.
/// A path absent from this set is treated as writable by the dashboard, which is exactly today's behaviour, so an enumeration gap degrades to the status quo rather than to a field the operator cannot edit.
fn non_writable_schema_paths(root: &serde_json::Value) -> Vec<String> {
    /// Name of the definition a property `$ref`s, directly or through `allOf` / `anyOf`.
    fn referenced_definition(prop: &serde_json::Value) -> Option<&str> {
        if let Some(r) = prop.get("$ref").and_then(|v| v.as_str()) {
            return r.rsplit('/').next();
        }
        for combinator in ["allOf", "anyOf", "oneOf"] {
            if let Some(entries) = prop.get(combinator).and_then(|v| v.as_array()) {
                for entry in entries {
                    if let Some(r) = entry.get("$ref").and_then(|v| v.as_str()) {
                        return r.rsplit('/').next();
                    }
                }
            }
        }
        None
    }

    let definitions = root.get("definitions").and_then(|v| v.as_object());
    let Some(properties) = root.get("properties").and_then(|v| v.as_object()) else {
        return Vec::new();
    };

    let mut paths = Vec::new();
    for (section, prop) in properties {
        if !super::is_writable_config_path(section) {
            paths.push(section.clone());
        }
        let Some(nested) = referenced_definition(prop)
            .and_then(|name| definitions?.get(name))
            .and_then(|d| d.get("properties"))
            .and_then(|v| v.as_object())
        else {
            continue;
        };
        for field in nested.keys() {
            let path = format!("{section}.{field}");
            if !super::is_writable_config_path(&path) {
                paths.push(path);
            }
        }
    }
    paths.sort();
    paths
}

// ---------------------------------------------------------------------------
// Config Set endpoint
// ---------------------------------------------------------------------------
/// Make `item` addressable as a table, creating a standard table only when it
/// is not already table-shaped.
///
/// The distinction matters because `toml_edit` models `[media]` and
/// `media = { … }` as different `Item` variants — `Item::Table` and
/// `Item::Value(Value::InlineTable)` — while `contains_table` / `as_table_mut`
/// recognise only the former. The previous `if !doc.contains_table(name)` guard
/// therefore judged a hand-written inline section "missing" and replaced it
/// with an empty table, dropping every key it held, `api_key_env` included.
///
/// A caller editing one leaf of such a section would have silently deleted the
/// rest of it — and #8085 recommends exactly those per-leaf writes as the safe
/// route for tables carrying credential fields, which is what makes this
/// reachable rather than theoretical.
fn ensure_table_like(item: &mut toml_edit::Item) {
    if !item.is_table_like() {
        *item = toml_edit::Item::Table(toml_edit::Table::new());
    }
}

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

    // SECURITY (#8085): the path check above governs only the name being
    // assigned. A write one level below a writable section assigns the
    // submitted JSON wholesale, so an innocuous-looking path can carry a
    // scrubbed field as a member of the table it replaces — the
    // `{"path": "media.custom_stt", "value": {"api_key_env": "..."}}` shape.
    // Scan the payload for the same key names the path check refuses, so a
    // credential-shaped field is unreachable by either route.
    if let Some(offending) = super::scrubbed_key_in_payload(&value) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "status": "error",
                "error": format!(
                    "value posted to '{path}' contains '{offending}', which is not \
                     user-tunable via /api/config/set; post the other fields \
                     individually (e.g. '{path}.<field>') and edit \
                     '{offending}' in ~/.librefang/config.toml directly"
                )
            })),
        );
    }

    // No basename / traversal check on `config_path`: it is the kernel's boot-resolved path, not anything the request supplied.
    // Under `LIBREFANG_CONFIG_PATH` the operator's chosen filename is the point, so rejecting a name that is not literally `config.toml` would refuse to write the very file this daemon loaded (#6695).
    let config_path = state.kernel.config_path().to_path_buf();

    // Serialize concurrent writes to prevent read-modify-write races
    if let Some(locked) = crate::routes::guard_config_write(state.kernel.config_path()) {
        return locked;
    }
    let _config_guard = state.config_write_lock.lock().await;

    // Read existing config — use toml_edit to preserve comments and formatting.
    // A read failure on an existing file (permission denied, hardware fault,
    // …) MUST abort — falling back to "" would silently drop every other
    // section in `config.toml` (agents, providers, taint rules, …) on the
    // next write. Same protection as `users::persist_users` (#3368).
    let raw_content = match tokio::fs::read_to_string(&config_path).await {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            // Scrub the io error (audit: rusqlite-errors-leak) —
            // path / permission detail stays in the log.
            tracing::error!(%error, "could not read existing config.toml");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "status": "error",
                    "error": "Internal server error"
                })),
            );
        }
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
                // `as_table_like_mut` rather than `as_table_mut`: a section an
                // operator hand-wrote as an inline table (`media = { … }`) is
                // `Item::Value(InlineTable)`, not `Item::Table`, and the
                // narrower accessor returns `None` — so the removal silently
                // did nothing while the handler still answered success.
                if let Some(t) = doc[parts[0]].as_table_like_mut() {
                    t.remove(parts[1]);
                }
            } else {
                ensure_table_like(&mut doc[parts[0]]);
                doc[parts[0]][parts[1]] = toml_edit::Item::Value(json_to_toml_edit_value(&value));
            }
        }
        3 => {
            if is_remove {
                if let Some(t) = doc[parts[0]].as_table_like_mut() {
                    if let Some(t2) = t.get_mut(parts[1]).and_then(|i| i.as_table_like_mut()) {
                        t2.remove(parts[2]);
                    }
                }
            } else {
                ensure_table_like(&mut doc[parts[0]]);
                if let Some(section) = doc[parts[0]].as_table_like_mut() {
                    if !section.get(parts[1]).is_some_and(|i| i.is_table_like()) {
                        section.insert(parts[1], toml_edit::Item::Table(toml_edit::Table::new()));
                    }
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
    if let Some(home_dir) = config_path.parent() {
        let backups_dir = home_dir.join("backups");
        if tokio::fs::create_dir_all(&backups_dir).await.is_ok() {
            match tokio::fs::copy(&config_path, backups_dir.join("config.toml.prev")).await {
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => tracing::warn!(%error, "failed to back up config.toml"),
            }
        }
    }

    // Write back — preserves comments, whitespace, and key ordering
    let write_path = config_path.clone();
    let write_bytes = new_toml_str.into_bytes();
    let write_result =
        tokio::task::spawn_blocking(move || crate::atomic_write(&write_path, &write_bytes)).await;
    match write_result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            // Scrub the io error (audit: rusqlite-errors-leak).
            tracing::error!(%error, "failed to write config.toml");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"status": "error", "error": "Internal server error"})),
            );
        }
        Err(error) => {
            tracing::error!(%error, "config write task failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"status": "error", "error": "Internal server error"})),
            );
        }
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
        reload_status,
        user_id,
        Some("api".to_string()),
    );

    let mut body = serde_json::json!({"status": reload_status, "path": path});
    if let Some(err) = reload_error {
        body["reload_error"] = serde_json::Value::String(err);
    }
    (StatusCode::OK, Json(body))
}

#[cfg(test)]
mod config_reload_outcome_tests {
    use super::config_reload_status;

    #[test]
    fn channel_restart_failure_forces_partial_reload_status() {
        assert_eq!(config_reload_status(false, true, true), "partial");
        assert_eq!(config_reload_status(false, false, true), "partial");
    }

    #[test]
    fn reload_status_preserves_existing_success_states() {
        assert_eq!(config_reload_status(true, true, false), "partial");
        assert_eq!(config_reload_status(false, true, false), "applied");
        assert_eq!(config_reload_status(false, false, false), "no_changes");
    }
}

// ---------------------------------------------------------------------------
// Read/write parity guard (#6596)
// ---------------------------------------------------------------------------

/// Guards the invariant that `GET /api/config` exposes every config leaf `POST /api/config/set` accepts.
///
/// The response body is hand-enumerated per section rather than derived from `Serialize`, because the dashboard depends on keys serde would never emit (`web.search_available`, `media.stt_available`) and on redaction markers that replace secret values in place.
/// That shape is worth keeping, but it drifts silently: a field added to a config struct and wired into the write allowlist stays invisible on the read side, so the dashboard renders the setting blank and a successful save reads back as "not configured".
/// This module turns that drift into a build failure.
///
/// The exclusion rules (secret suffixes, per-section depth limits, sections that are deliberately edit-on-disk) are NOT restated here — the guard calls the real `is_writable_config_path` so there is exactly one copy of them.
#[cfg(test)]
mod config_read_write_parity_tests {
    use librefang_types::config::{
        A2aConfig, BudgetConfig, KernelConfig, ThinkingConfig, WebhookTriggerConfig,
    };

    /// `KernelConfig::default()` with every `Option`-wrapped section populated.
    /// Those sections serialize to `null` when unset, which would hide their leaves from the walk below and let a gap under `a2a`, `webhook_triggers`, or `thinking` pass unnoticed.
    fn config_with_optional_sections_populated() -> KernelConfig {
        KernelConfig {
            a2a: Some(A2aConfig::default()),
            webhook_triggers: Some(WebhookTriggerConfig::default()),
            thinking: Some(ThinkingConfig::default()),
            ..KernelConfig::default()
        }
    }

    /// Every dotted path in `value` that `is_writable_config_path` could match: the top-level key plus one and two nested levels, which is the deepest form the allowlist accepts.
    /// Arrays are leaves — a numeric index is not a config key path.
    ///
    /// Known blind spot: a field carrying `#[serde(skip_serializing_if = …)]` whose default value satisfies that predicate is absent from `value`, so the walk cannot see it.
    /// Every writable path in that category today (`exec_policy.allowed_env_vars`, `default_model.cli_profile_dirs`, `budget.providers`, `tool_invoke.allowlist`, `agent_max_iterations`, `max_history_messages`) is in the read payload, checked by hand; a future one has to be added by hand too.
    fn candidate_paths(value: &serde_json::Value) -> Vec<String> {
        let mut paths = Vec::new();
        let Some(root) = value.as_object() else {
            return paths;
        };
        for (k1, v1) in root {
            paths.push(k1.clone());
            let Some(level1) = v1.as_object() else {
                continue;
            };
            for (k2, v2) in level1 {
                paths.push(format!("{k1}.{k2}"));
                let Some(level2) = v2.as_object() else {
                    continue;
                };
                for k3 in level2.keys() {
                    paths.push(format!("{k1}.{k2}.{k3}"));
                }
            }
        }
        paths
    }

    /// Walk a dotted path through the response body, returning the value at the end.
    /// A JSON `null` counts as found: an unset `Option` is legitimately rendered as `null`, and the dashboard needs the key to exist so it can bind an empty input to it.
    fn lookup<'a>(payload: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
        let mut cursor = payload;
        for segment in path.split('.') {
            cursor = cursor.as_object()?.get(segment)?;
        }
        Some(cursor)
    }

    #[test]
    fn every_writable_config_leaf_is_readable() {
        let config = config_with_optional_sections_populated();
        let budget = BudgetConfig::default();
        let serialized = serde_json::to_value(&config).expect("KernelConfig derives Serialize");
        let payload = super::redacted_config_json(&config, &budget);

        let writable: Vec<String> = candidate_paths(&serialized)
            .into_iter()
            .filter(|path| super::super::is_writable_config_path(path))
            .collect();

        // Sanity floor: if the walk stops producing paths (a refactor changes the serialized shape, say) the assertion below would pass vacuously and the guard would be worthless.
        // The real count is in the hundreds.
        assert!(
            writable.len() > 100,
            "parity guard enumerated only {} writable paths — the walk is broken, \
             not the config (expected hundreds)",
            writable.len()
        );

        let mut missing: Vec<String> = writable
            .into_iter()
            .filter(|path| lookup(&payload, path).is_none())
            .collect();
        missing.sort();

        assert!(
            missing.is_empty(),
            "POST /api/config/set accepts these paths but GET /api/config omits them, so the \
             dashboard renders each one blank and reads it back as \"not configured\" right \
             after a successful save (#6596). Add every path to `redacted_config_json`: \
             {missing:#?}"
        );
    }

    /// The write path, the JSON schema's `select` options, and `config.toml` all speak serde's rename form for these enums.
    /// `format!("{:?}", …)` emitted the Rust variant name instead, so the dashboard received a value matching none of the options it offered and showed the dropdown empty even though the field was set.
    #[test]
    fn enum_valued_fields_use_the_serde_encoding_not_debug() {
        let config = KernelConfig::default();
        let payload = super::redacted_config_json(&config, &BudgetConfig::default());

        for (path, expected) in [
            ("mode", "default"),
            ("reload.mode", "hybrid"),
            ("exec_policy.mode", "allowlist"),
            ("broadcast.strategy", "parallel"),
            ("docker.mode", "off"),
            ("docker.scope", "session"),
            ("web.search_provider", "auto"),
        ] {
            assert_eq!(
                lookup(&payload, path).and_then(|v| v.as_str()),
                Some(expected),
                "`{path}` must be the serde encoding the write path accepts, not Debug's \
                 variant name"
            );
        }
    }

    #[test]
    fn pairing_ntfy_url_hides_embedded_credentials() {
        let mut config = KernelConfig::default();
        config.pairing.ntfy_url =
            Some("https://notify-user:notify-password@ntfy.example.test/topic".to_string());

        let payload = super::redacted_config_json(&config, &BudgetConfig::default());
        let rendered = lookup(&payload, "pairing.ntfy_url")
            .and_then(|value| value.as_str())
            .expect("configured ntfy URL remains visible in redacted form");

        assert_eq!(rendered, "https://***@ntfy.example.test/topic");
        assert!(!rendered.contains("notify-user"));
        assert!(!rendered.contains("notify-password"));
    }

    #[test]
    fn pairing_ntfy_url_preserves_at_signs_outside_the_authority() {
        for url in [
            "https://ntfy.example.test/topic@tenant",
            "https://ntfy.example.test/topic?contact=ops@example.test",
            "https://ntfy.example.test/topic#owner@tenant",
        ] {
            let mut config = KernelConfig::default();
            config.pairing.ntfy_url = Some(url.to_string());

            let payload = super::redacted_config_json(&config, &BudgetConfig::default());
            let rendered = lookup(&payload, "pairing.ntfy_url")
                .and_then(|value| value.as_str())
                .expect("configured ntfy URL remains visible");

            assert_eq!(rendered, url);
        }
    }

    /// The specific paths the #6596 report listed as writable-but-unreadable, pinned by name so a regression names the issue rather than surfacing as one entry in the bulk diff above.
    #[test]
    fn reported_missing_paths_are_present() {
        let config = config_with_optional_sections_populated();
        let payload = super::redacted_config_json(&config, &BudgetConfig::default());

        for path in [
            "browser.enabled",
            "browser.cdp_endpoint",
            "media.image_model",
            "media.custom_stt",
            "media.transcription_timeout_secs",
            "media.ffmpeg_timeout_secs",
            "tts.custom",
            "channels.file_download_dir",
            "terminal.enabled",
            "approval.totp_grace_period_secs",
            "web.timeout_secs",
            "exec_policy.allowed_env_vars",
        ] {
            assert!(
                lookup(&payload, path).is_some(),
                "`{path}` was reported missing from GET /api/config in #6596"
            );
        }
    }

    /// #6605: fields that are intentionally NOT writable but must still be readable.
    ///
    /// `every_writable_config_leaf_is_readable` enforces `writable ⊆ readable`, so it is blind to this direction by construction — a field deliberately excluded from the write allowlist sits outside that invariant.
    /// `external_auth.require_email_verified` is the #3703 mitigation, and an operator who cannot read it back has no way to confirm the protection is active short of reading `config.toml` on the host.
    /// The `OidcProvider` endpoint fields have the same problem: with them hidden, a non-OIDC provider's explicit endpoint overrides were invisible.
    #[test]
    fn non_writable_but_operator_visible_fields_are_readable() {
        let mut config = config_with_optional_sections_populated();
        // Deliberately the non-default value: `require_email_verified` defaults to `true`, so asserting `true` would also pass against a hardcoded literal in the response builder.
        config.external_auth.require_email_verified = false;
        config.external_auth.providers.push(oidc_provider_fixture());
        let payload = super::redacted_config_json(&config, &BudgetConfig::default());

        assert_eq!(
            lookup(&payload, "external_auth.require_email_verified").and_then(|v| v.as_bool()),
            Some(false),
            "the #3703 email-verification gate must be readable, and read from config, even though \
             it is deliberately non-writable (#6605)"
        );

        let provider = lookup(&payload, "external_auth.providers")
            .and_then(|v| v.as_array())
            .and_then(|providers| providers.first())
            .and_then(|p| p.as_object())
            .expect("the configured provider is rendered as an object");
        for key in [
            "id",
            "display_name",
            "issuer_url",
            "auth_url",
            "token_url",
            "userinfo_url",
            "jwks_uri",
            "client_id",
            "client_secret_env",
            "redirect_url",
            "scopes",
            "allowed_domains",
            "audience",
            "require_email_verified",
        ] {
            // Presence is not enough: a response builder that emits the key but drops the value renders `"jwks_uri": null`, which a `contains_key` check accepts.
            // The fixture populates all 14 fields, so a null here is always a dropped field.
            // Exact values are pinned by the `get_config_exposes_non_writable_external_auth_fields` integration test rather than duplicated here.
            assert!(
                provider.get(key).is_some_and(|v| !v.is_null()),
                "`external_auth.providers[].{key}` is missing or null in GET /api/config — every \
                 `OidcProvider` field is non-secret, is set by the fixture, and must be visible \
                 (#6605); got {provider:#?}"
            );
        }
    }

    /// #6636 observation (e): every `ApprovalPolicy` field must be readable.
    ///
    /// The `approval` section declares no explicit `fields` list in `ui_sections_overlay`, so `ConfigPage` renders a control for whatever the derived schema says the struct has — but the response builder enumerated seven of fourteen fields by hand, and the rest rendered blank and read back as their JSON zero value.
    /// `cache_approvals_per_session` made it visible: it defaults to `true`, so the dashboard showed it off for every operator who had never touched it, including one whose `config.toml` said `true`.
    /// Only three approval paths are writable, so `every_writable_config_leaf_is_readable` covers three of fourteen and is blind to the rest by construction.
    ///
    /// The oracle is the serialized struct rather than a restated list, so a field added to `ApprovalPolicy` later fails here instead of quietly joining the gap.
    /// The fixture sets non-default values so a response builder that emits the key with a hardcoded literal cannot pass.
    #[test]
    fn approval_policy_fields_are_all_readable() {
        use librefang_types::approval::{ChannelToolRule, NotificationTarget};

        let mut config = config_with_optional_sections_populated();
        config.approval.cache_approvals_per_session = false;
        config.approval.audit_retention_days = 7;
        config.approval.trusted_senders = vec!["operator-1".into()];
        config.approval.totp_tools = vec!["shell_exec".into()];
        config.approval.timeout_fallback = librefang_types::approval::TimeoutFallback::Escalate {
            extra_timeout_secs: 45,
        };
        config.approval.channel_rules = vec![ChannelToolRule {
            channel: "telegram".into(),
            allowed_tools: vec!["file_read".into()],
            denied_tools: vec!["shell_exec".into()],
        }];
        config.approval.routing = vec![librefang_types::approval::ApprovalRoutingRule {
            tool_pattern: "shell_*".into(),
            route_to: vec![NotificationTarget {
                channel_type: "telegram".into(),
                recipient: "12345".into(),
                thread_id: None,
            }],
        }];

        let payload = super::redacted_config_json(&config, &BudgetConfig::default());
        let serialized =
            serde_json::to_value(&config.approval).expect("ApprovalPolicy derives Serialize");
        let expected = serialized
            .as_object()
            .expect("ApprovalPolicy serializes to an object");

        // Sanity floor: a refactor that empties the oracle would make the loop below pass vacuously.
        assert!(
            expected.len() >= 14,
            "the ApprovalPolicy oracle enumerated only {} fields — the walk is broken, not the config",
            expected.len()
        );

        let mut missing: Vec<&String> = Vec::new();
        let mut mismatched: Vec<String> = Vec::new();
        for (key, want) in expected {
            match lookup(&payload, &format!("approval.{key}")) {
                None => missing.push(key),
                Some(got) if got != want => {
                    mismatched.push(format!("approval.{key}: want {want}, got {got}"))
                }
                Some(_) => {}
            }
        }

        assert!(
            missing.is_empty(),
            "GET /api/config omits these ApprovalPolicy fields, so the dashboard renders each one \
             blank and reads it back as its zero value (#6636). Add them to `redacted_config_json`: \
             {missing:#?}"
        );
        assert!(
            mismatched.is_empty(),
            "these ApprovalPolicy fields are emitted but do not carry the configured value: \
             {mismatched:#?}"
        );
    }

    /// One fully-populated provider.
    /// `OidcProvider` derives no `Default`, so every field is spelled out; distinct values make it obvious in a failure which field a payload dropped.
    fn oidc_provider_fixture() -> librefang_types::config::OidcProvider {
        librefang_types::config::OidcProvider {
            id: "corp".into(),
            display_name: "Corporate SSO".into(),
            issuer_url: "https://issuer.example.invalid".into(),
            auth_url: "https://issuer.example.invalid/authorize".into(),
            token_url: "https://issuer.example.invalid/token".into(),
            userinfo_url: "https://issuer.example.invalid/userinfo".into(),
            jwks_uri: "https://issuer.example.invalid/jwks".into(),
            client_id: "corp-client".into(),
            client_secret_env: "LIBREFANG_PARITY_GUARD_FIXTURE_SECRET".into(),
            redirect_url: "http://127.0.0.1:4545/api/auth/callback".into(),
            scopes: vec!["openid".into()],
            allowed_domains: vec!["example.invalid".into()],
            audience: "corp-audience".into(),
            require_email_verified: Some(false),
        }
    }
}

// ---------------------------------------------------------------------------
// Writable-allowlist backing-field guard (#6605)
// ---------------------------------------------------------------------------

/// Guards the mirror of the parity invariant above: every entry in the `POST /api/config/set` allowlist must name a field that actually exists on `KernelConfig`.
///
/// `config_set` validates the caller's dotted path against `is_writable_config_path` and then edits `config.toml` through `toml_edit` keyed by that path.
/// Nothing between those two steps requires the path to correspond to a real field, and the post-edit `toml::from_str::<KernelConfig>` check cannot reject one either because `KernelConfig` does not set `deny_unknown_fields`.
/// So a write against a path with no backing field is accepted, lands a table on disk, and is discarded by the next load: the caller gets a success status for a change that is never applied and never reads back.
/// `ui.theme`, `ui.locale`, `ui.timezone`, and `ui.language` sat in the allowlist in exactly that state from #4113 until #6605 removed them.
///
/// `every_writable_config_leaf_is_readable` cannot catch this class: it derives its candidate paths *from* a serialized config, so a path naming a field that does not exist is structurally invisible to it.
///
/// The oracle here is the schemars-derived JSON Schema rather than a serialized `KernelConfig` value.
/// A value walk cannot see a field whose `#[serde(skip_serializing_if = …)]` predicate holds for its default value, and `librefang-types/src/config/types.rs` carries 63 of those attributes — several on writable paths (`exec_policy.allowed_env_vars`, `budget.providers`, `tool_invoke.allowlist`, …) — so a value-based oracle would report real fields as dangling.
/// The schema declares every field regardless of that attribute.
#[cfg(test)]
mod writable_allowlist_backing_field_tests {
    /// Maximum `$ref` / combinator hops before the resolver gives up.
    /// Descending through `properties` consumes a path segment, so only a `$ref` chain can recurse without making progress; schemars does not emit one that cycles, and the cap keeps a future schema shape from turning this test into a stack overflow.
    const MAX_SCHEMA_DEPTH: usize = 32;

    /// Does `segments` resolve to a field declared by the draft-07 schema rooted at `node`?
    ///
    /// Follows `$ref` into `definitions`.
    /// Treats `allOf` / `anyOf` / `oneOf` as "any branch that resolves counts": schemars renders a struct-typed field as `allOf: [{$ref}]`, an `Option<T>` as `anyOf: [{$ref}, {"type": "null"}]`, and a `OneOrMany<T>` as a `T` / `[T]` branch pair, so the branch that matches is the one that answers the question.
    /// `additionalProperties` consumes one segment: a map-typed section (`provider_urls`, `tool_timeouts`, …) has dynamic keys by design, and an allowlist entry addressing one is correct rather than dangling.
    /// Everything else — including a dotted segment against an array or a scalar — is not a declared field.
    fn schema_declares_path(
        definitions: &serde_json::Map<String, serde_json::Value>,
        node: &serde_json::Value,
        segments: &[&str],
        depth: usize,
    ) -> bool {
        if segments.is_empty() {
            return true;
        }
        if depth >= MAX_SCHEMA_DEPTH {
            return false;
        }
        let Some(obj) = node.as_object() else {
            return false;
        };
        if let Some(reference) = obj.get("$ref").and_then(|v| v.as_str()) {
            let name = reference.rsplit('/').next().unwrap_or_default();
            return definitions.get(name).is_some_and(|target| {
                schema_declares_path(definitions, target, segments, depth + 1)
            });
        }
        for combinator in ["allOf", "anyOf", "oneOf"] {
            let branch_resolves =
                obj.get(combinator)
                    .and_then(|v| v.as_array())
                    .is_some_and(|branches| {
                        branches.iter().any(|branch| {
                            schema_declares_path(definitions, branch, segments, depth + 1)
                        })
                    });
            if branch_resolves {
                return true;
            }
        }
        if let Some(child) = obj
            .get("properties")
            .and_then(|v| v.as_object())
            .and_then(|properties| properties.get(segments[0]))
        {
            return schema_declares_path(definitions, child, &segments[1..], depth + 1);
        }
        // `additionalProperties: true` is a bool rather than a subschema — it means "anything goes", which is not a field declaration.
        if let Some(values) = obj.get("additionalProperties").filter(|v| v.is_object()) {
            return schema_declares_path(definitions, values, &segments[1..], depth + 1);
        }
        false
    }

    fn declares(
        definitions: &serde_json::Map<String, serde_json::Value>,
        schema: &serde_json::Value,
        path: &str,
    ) -> bool {
        let segments: Vec<&str> = path.split('.').collect();
        schema_declares_path(definitions, schema, &segments, 0)
    }

    #[test]
    fn every_writable_allowlist_entry_has_a_backing_config_field() {
        let schema =
            serde_json::to_value(schemars::schema_for!(librefang_types::config::KernelConfig))
                .expect("KernelConfig derives JsonSchema");
        let definitions = schema
            .get("definitions")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();

        // A schemars upgrade to draft 2020-12 renames this block to `$defs`, which would leave every `$ref` dangling and report all ~90 entries as missing at once.
        // Fail on the shape change directly so the cause is legible instead of inferred from the fallout.
        assert!(
            !definitions.is_empty(),
            "the KernelConfig schema has no `definitions` block, so the resolver cannot follow a \
             single `$ref` and its verdicts are meaningless (draft 2020-12 names it `$defs`)"
        );

        // Sanity floor, same purpose as the one in `every_writable_config_leaf_is_readable`: an assertion over an accidentally-empty enumeration passes vacuously.
        let entry_count = super::super::WRITABLE_EXACT_PATHS.len()
            + super::super::WRITABLE_SECTION_PREFIXES.len();
        assert!(
            entry_count > 50,
            "the allowlist enumerated only {entry_count} entries — the lists shrank drastically or \
             moved, so this guard is no longer checking the real write surface (expected 80+)"
        );

        // Negative controls.
        // The failure mode of a resolver is over-permissiveness: one stray fallback, or a schema shape it walks past instead of rejecting, turns the guard into a no-op that still passes.
        // `ui.theme` is the exact path #6605 removed, which also ties this guard to the defect it exists for.
        for absent in [
            "ui.theme",
            "ui",
            "nonexistent_section.nonexistent_field",
            "log_level.nested_under_a_scalar",
        ] {
            assert!(
                !declares(&definitions, &schema, absent),
                "the resolver claims `{absent}` is a declared `KernelConfig` field — it is not, so \
                 it cannot discriminate and the guard below is worthless"
            );
        }

        let mut dangling: Vec<String> = Vec::new();
        for &path in super::super::WRITABLE_EXACT_PATHS {
            if !declares(&definitions, &schema, path) {
                dangling.push(path.to_string());
            }
        }
        // A section prefix is checked at its base.
        // The prefix admits arbitrary leaves beneath it, so "this section exists" is the strongest claim available without enumerating what a caller might post — the leaf itself is only reachable at request time.
        for &prefix in super::super::WRITABLE_SECTION_PREFIXES {
            if !declares(&definitions, &schema, prefix.trim_end_matches('.')) {
                dangling.push(prefix.to_string());
            }
        }
        dangling.sort();

        assert!(
            dangling.is_empty(),
            "these `POST /api/config/set` allowlist entries name no field on `KernelConfig`, so a \
             write against one is accepted, lands in config.toml, and is silently discarded by the \
             next load — a success status for a no-op (#6605). Add the backing field or drop the \
             entry: {dangling:#?}"
        );
    }

    /// `WRITABLE_DEPTH_2_ONLY_PREFIXES` is only consulted from inside the `WRITABLE_SECTION_PREFIXES` loop, so an entry missing from the latter restricts nothing at all.
    /// Same defect shape as a dangling path: a rule that reads as a restriction but has no effect.
    #[test]
    fn every_depth_2_only_prefix_is_also_a_section_prefix() {
        for &prefix in super::super::WRITABLE_DEPTH_2_ONLY_PREFIXES {
            assert!(
                super::super::WRITABLE_SECTION_PREFIXES.contains(&prefix),
                "`{prefix}` restricts writes to depth 2 but is not a writable section prefix, so \
                 the restriction is never reached"
            );
        }
    }
}
