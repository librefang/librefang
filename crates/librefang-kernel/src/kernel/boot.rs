//! Cluster pulled out of mod.rs in #4713 phase 3d.
//!
//! Hosts the kernel boot path: `session_stream_hub` getter, `boot`
//! convenience wrapper, and the giant `boot_with_config` constructor
//! that wires every subsystem (memory, drivers, registries, scheduler,
//! background tasks) into a `LibreFangKernel`.
//!
//! Sibling submodule of `kernel::mod`, so it retains access to
//! `LibreFangKernel`'s private fields and inherent methods without any
//! visibility surgery — descendants of the declaring module see private
//! items, which means `boot_with_config` can construct the struct
//! literal directly.

use super::*;

impl LibreFangKernel {
    /// Per-session stream-event hub (multi-client SSE attach).
    ///
    /// API handlers use this to subscribe attaching clients to a session's
    /// in-flight `StreamEvent` flow. Returns the shared `Arc` so subscribers
    /// outlive any individual turn.
    pub fn session_stream_hub(&self) -> Arc<crate::session_stream_hub::SessionStreamHub> {
        Arc::clone(&self.session_stream_hub)
    }

    /// Boot the kernel with configuration from the given path.
    pub fn boot(config_path: Option<&Path>) -> KernelResult<Self> {
        let config = load_config(config_path);
        Self::boot_with_config(config)
    }

    /// Boot the kernel with an explicit configuration.
    ///
    /// Callers must have loaded `.env` / `secrets.env` / vault into the
    /// process env before calling this — use
    /// [`librefang_extensions::dotenv::load_dotenv`] from a synchronous
    /// `main()`. Mutating env from here would be UB: this function is
    /// reached from inside a tokio runtime, and `std::env::set_var` is
    /// unsound once other threads exist (Rust 1.80+).
    pub fn boot_with_config(mut config: KernelConfig) -> KernelResult<Self> {
        use librefang_types::config::KernelMode;

        // Env var overrides — useful for Docker where config.toml is baked in.
        if let Ok(listen) = std::env::var("LIBREFANG_LISTEN") {
            config.api_listen = listen;
        }

        // Clamp configuration bounds to prevent zero-value or unbounded misconfigs
        config.clamp_bounds();

        // Resolve `vault.use_os_keyring` into the process-global vault state
        // before any vault operation runs. Must happen before the TOTP
        // check below (which unlocks the vault) and before any agent boot
        // path that touches MCP OAuth tokens. Idempotent: first call wins.
        librefang_extensions::vault::CredentialVault::init_with_config(config.vault.use_os_keyring);

        // Vault startup-sentinel verification (#3651).
        //
        // If a vault file already exists, refuse to boot when it cannot be
        // unlocked with the resolved master key OR when the sentinel
        // plaintext does not match. Pre-fix, the daemon would silently
        // boot with the wrong key and every subsequent vault read would
        // fail with a generic "Decryption failed" log line — operators
        // never learned the root cause. The sentinel turns that into a
        // single, actionable error at boot time.
        //
        // If the vault does not yet exist we say nothing — first-run / CLI
        // bootstrap creates it later via `init()`, which writes the
        // sentinel automatically.
        let vault_path = config.home_dir.join("vault.enc");
        if vault_path.exists() {
            let mut vault = librefang_extensions::vault::CredentialVault::new(vault_path.clone());
            match vault.unlock() {
                Ok(()) => {
                    if let Err(e) = vault.verify_or_install_sentinel() {
                        match e {
                            librefang_extensions::ExtensionError::VaultKeyMismatch { hint } => {
                                return Err(KernelError::BootFailed(format!(
                                    "Vault key mismatch — refusing to boot. {hint} \
                                     Recovery: restore the original LIBREFANG_VAULT_KEY env var, \
                                     restore the vault file from backup, or run \
                                     `librefang vault rotate-key` if you intended to rotate."
                                )));
                            }
                            other => {
                                // Sentinel backfill failed for some other
                                // reason (disk full, permissions). Surface
                                // it but don't pretend it's a key mismatch.
                                return Err(KernelError::BootFailed(format!(
                                    "Vault sentinel write failed: {other}"
                                )));
                            }
                        }
                    }
                }
                Err(librefang_extensions::ExtensionError::VaultLocked) => {
                    // No master key available at all — don't refuse boot
                    // (some deployments run without a vault and rely on env
                    // vars), but warn loudly so the operator notices the
                    // mismatch between "vault file exists" and "no key".
                    warn!(
                        "Vault file exists at {:?} but no master key is \
                         resolvable (LIBREFANG_VAULT_KEY unset and OS keyring \
                         empty). Encrypted credentials will be unreadable until \
                         the key is restored.",
                        vault_path
                    );
                }
                Err(e) => {
                    // Non-locked unlock failure is almost always wrong-key
                    // (AES-GCM decrypt fails). Refuse to boot — same
                    // rationale as the sentinel-mismatch branch above.
                    return Err(KernelError::BootFailed(format!(
                        "Vault unlock failed at boot ({e}). This usually means \
                         LIBREFANG_VAULT_KEY does not match the key the vault \
                         was encrypted with. Recovery: restore the original \
                         env var, restore the vault file from backup, or run \
                         `librefang vault rotate-key` if you intended to rotate."
                    )));
                }
            }
        }

        match config.mode {
            KernelMode::Stable => {
                info!("Booting LibreFang kernel in STABLE mode — conservative defaults enforced");
            }
            KernelMode::Dev => {
                warn!("Booting LibreFang kernel in DEV mode — experimental features enabled");
            }
            KernelMode::Default => {
                info!("Booting LibreFang kernel...");
            }
        }

        // Validate configuration and log warnings
        let warnings = config.validate();
        for w in &warnings {
            warn!("Config: {}", w);
        }

        // Tool-exec subtable check: missing `[tool_exec.ssh]` /
        // `[tool_exec.daytona]` is fatal-on-boot rather than a warning,
        // so an operator typo doesn't silently let the daemon come up
        // with an unusable backend that fails on first tool call.
        // Lost during the kernel/mod split; restored here.
        if let Err(e) = config.tool_exec.validate() {
            return Err(KernelError::BootFailed(format!(
                "Invalid [tool_exec] config: {e}"
            )));
        }

        // Check TOTP configuration consistency
        if config.approval.second_factor == librefang_types::approval::SecondFactor::Totp {
            let vault_path = config.home_dir.join("vault.enc");
            let mut vault = librefang_extensions::vault::CredentialVault::new(vault_path);
            let totp_ready = vault.unlock().is_ok()
                && vault
                    .get("totp_confirmed")
                    .map(|v| v.as_str() == "true")
                    .unwrap_or(false);
            if !totp_ready {
                warn!(
                    "Config: second_factor = \"totp\" but TOTP is not enrolled/confirmed in vault. \
                     Approvals will require TOTP but no secret is configured. \
                     Run POST /api/approvals/totp/setup to enroll."
                );
            }
        }

        // Initialise global HTTP proxy settings so all outbound reqwest
        // clients pick up proxy configuration from config.toml / env vars.
        librefang_runtime::http_client::init_proxy(config.proxy.clone());

        // Ensure data directory exists
        std::fs::create_dir_all(&config.data_dir)
            .map_err(|e| KernelError::BootFailed(format!("Failed to create data dir: {e}")))?;

        // Migrate old directory layout (hands/, workspaces/<agent>/) to unified layout
        ensure_workspaces_layout(&config.home_dir)?;
        migrate_legacy_agent_dirs(&config.home_dir, &config.effective_agent_workspaces_dir());
        migrate_root_backups(&config.home_dir);
        migrate_root_state_files(&config.home_dir);
        cleanup_legacy_root_logs(&config.home_dir);

        // Initialize memory substrate
        let db_path = config
            .memory
            .sqlite_path
            .clone()
            .unwrap_or_else(|| config.data_dir.join("librefang.db"));
        // Honour `[memory] pool_size` (#4685). The kernel/mod split
        // briefly regressed this to `open_with_chunking`, which forces
        // the r2d2 default pool size of 8 regardless of operator
        // configuration; the prompt-store init below already used the
        // configured value, so the two SQLite pools were sized
        // inconsistently.
        let mut substrate = MemorySubstrate::open_with_pool_size(
            &db_path,
            config.memory.decay_rate as f32,
            config.memory.chunking.clone(),
            config.memory.pool_size,
        )
        .map_err(|e| KernelError::BootFailed(format!("Memory init failed: {e}")))?;

        // Optionally attach an external vector store backend.
        if let Some(ref backend) = config.memory.vector_backend {
            match backend.as_str() {
                "http" => {
                    let url = config.memory.vector_store_url.as_deref().ok_or_else(|| {
                        KernelError::BootFailed(
                            "vector_backend = \"http\" requires vector_store_url".into(),
                        )
                    })?;
                    let store = std::sync::Arc::new(librefang_memory::HttpVectorStore::new(url));
                    substrate.set_vector_store(store);
                    tracing::info!("Vector store backend: http ({})", url);
                }
                "sqlite" | "" => { /* default — no external backend */ }
                other => {
                    return Err(KernelError::BootFailed(format!(
                        "Unknown vector_backend: {other:?}"
                    )));
                }
            }
        }

        let memory = Arc::new(substrate);

        // Check if Ollama is reachable on localhost:11434 (TCP probe, 500ms timeout).
        fn is_ollama_reachable() -> bool {
            std::net::TcpStream::connect_timeout(
                &std::net::SocketAddr::from(([127, 0, 0, 1], 11434)),
                std::time::Duration::from_millis(500),
            )
            .is_ok()
        }

        // Resolve "auto" provider: scan environment for the first available API key.
        if config.default_model.provider == "auto" || config.default_model.provider.is_empty() {
            if let Some((provider, model_hint, env_var)) = drivers::detect_available_provider() {
                // model_hint may be empty if detected from the registry fallback;
                // resolve a sensible default from the model catalog.
                let model = if model_hint.is_empty() {
                    librefang_runtime::model_catalog::ModelCatalog::default()
                        .default_model_for_provider(provider)
                        .unwrap_or_else(|| "default".to_string())
                } else {
                    model_hint.to_string()
                };
                let auth_source = if env_var.is_empty() {
                    "CLI login"
                } else {
                    env_var
                };
                info!(
                    provider = %provider,
                    model = %model,
                    auth_source = %auth_source,
                    "Auto-detected default provider"
                );
                config.default_model.provider = provider.to_string();
                config.default_model.model = model;
                config.default_model.api_key_env = env_var.to_string();
            } else if is_ollama_reachable() {
                // Ollama is running locally — use the catalog's default model, not a hardcoded one.
                let model = librefang_runtime::model_catalog::ModelCatalog::default()
                    .default_model_for_provider("ollama")
                    .unwrap_or_else(|| {
                        warn!("Model catalog has no default for ollama — falling back to gemma4");
                        "gemma4".to_string()
                    });
                info!(
                    model = %model,
                    "No API keys detected — Ollama is running locally, using as default"
                );
                config.default_model.provider = "ollama".to_string();
                config.default_model.model = model;
                config.default_model.api_key_env = String::new();
                if !config.provider_urls.contains_key("ollama") {
                    // Use 127.0.0.1: on macOS `localhost` resolves to ::1 first
                    // and Ollama only binds IPv4, so the IPv6 attempt fails
                    // without reliable fallback. See PROVIDER_REGISTRY in
                    // librefang-llm-drivers for the same reasoning.
                    config.provider_urls.insert(
                        "ollama".to_string(),
                        "http://127.0.0.1:11434/v1".to_string(),
                    );
                }
            } else {
                warn!(
                    "No API keys detected and Ollama is not running. \
                     Set an API key or start Ollama to enable LLM features."
                );
            }
        }

        // Create LLM driver.
        // For the API key, try: 1) explicit api_key_env from config, 2) provider_api_keys
        // mapping, 3) auth profiles, 4) convention {PROVIDER}_API_KEY. This ensures
        // custom providers (e.g. nvidia, azure) work without hardcoded env var names.
        let default_api_key = if !config.default_model.api_key_env.is_empty() {
            std::env::var(&config.default_model.api_key_env).ok()
        } else {
            // api_key_env not set — resolve using provider_api_keys / convention
            let env_var = config.resolve_api_key_env(&config.default_model.provider);
            std::env::var(&env_var).ok()
        };
        let default_base_url = config.default_model.base_url.clone().or_else(|| {
            config
                .provider_urls
                .get(&config.default_model.provider)
                .cloned()
        });
        let mcp_bridge_cfg = build_mcp_bridge_cfg(&config);
        let default_proxy_url = config
            .provider_proxy_urls
            .get(&config.default_model.provider)
            .cloned();
        let default_request_timeout_secs = config
            .provider_request_timeout_secs
            .get(&config.default_model.provider)
            .copied();
        let driver_config = DriverConfig {
            provider: config.default_model.provider.clone(),
            api_key: default_api_key.clone(),
            base_url: default_base_url.clone(),
            vertex_ai: config.vertex_ai.clone(),
            azure_openai: config.azure_openai.clone(),
            skip_permissions: true,
            message_timeout_secs: config.default_model.message_timeout_secs,
            mcp_bridge: Some(mcp_bridge_cfg.clone()),
            proxy_url: default_proxy_url.clone(),
            request_timeout_secs: default_request_timeout_secs,
            emit_caller_trace_headers: config.telemetry.emit_caller_trace_headers,
        };
        // Primary driver failure is non-fatal: the dashboard should remain accessible
        // even if the LLM provider is misconfigured. Users can fix config via dashboard.
        let primary_result = drivers::create_driver(&driver_config);
        let mut driver_chain: Vec<Arc<dyn LlmDriver>> = Vec::new();

        let rotation_specs = collect_rotation_key_specs(
            config
                .auth_profiles
                .get(&config.default_model.provider)
                .map(Vec::as_slice),
            default_api_key.as_deref(),
        );

        if rotation_specs.len() > 1 || (primary_result.is_err() && !rotation_specs.is_empty()) {
            let mut rotation_drivers: Vec<(Arc<dyn LlmDriver>, String)> = Vec::new();

            for spec in rotation_specs {
                if spec.use_primary_driver {
                    if let Ok(driver) = &primary_result {
                        rotation_drivers.push((driver.clone(), spec.name));
                        continue;
                    }
                }

                let profile_name = spec.name;
                let profile_config = DriverConfig {
                    provider: config.default_model.provider.clone(),
                    api_key: Some(spec.api_key),
                    base_url: default_base_url.clone(),
                    vertex_ai: config.vertex_ai.clone(),
                    azure_openai: config.azure_openai.clone(),
                    skip_permissions: true,
                    message_timeout_secs: config.default_model.message_timeout_secs,
                    mcp_bridge: Some(mcp_bridge_cfg.clone()),
                    proxy_url: default_proxy_url.clone(),
                    request_timeout_secs: default_request_timeout_secs,
                    emit_caller_trace_headers: config.telemetry.emit_caller_trace_headers,
                };
                match drivers::create_driver(&profile_config) {
                    Ok(profile_driver) => {
                        rotation_drivers.push((profile_driver, profile_name));
                    }
                    Err(e) => {
                        warn!(
                            profile = %profile_name,
                            error = %e,
                            "Auth profile driver creation failed — skipped"
                        );
                    }
                }
            }

            if rotation_drivers.len() > 1 {
                info!(
                    provider = %config.default_model.provider,
                    pool_size = rotation_drivers.len(),
                    "Token rotation enabled for default provider"
                );
                let rotation = drivers::token_rotation::TokenRotationDriver::new(
                    rotation_drivers,
                    config.default_model.provider.clone(),
                );
                driver_chain.push(Arc::new(rotation));
            } else if let Some((driver, _)) = rotation_drivers.pop() {
                driver_chain.push(driver);
            }
        }

        // CLI profile rotation (Claude Code): create one driver per profile
        // directory, wrapped in TokenRotationDriver for automatic failover.
        if driver_chain.is_empty()
            && !config.default_model.cli_profile_dirs.is_empty()
            && matches!(
                config.default_model.provider.as_str(),
                "claude_code" | "claude-code"
            )
        {
            let profiles = &config.default_model.cli_profile_dirs;
            let mut profile_drivers: Vec<(Arc<dyn LlmDriver>, String)> = Vec::new();
            for (i, profile_path) in profiles.iter().enumerate() {
                let dir = if let Some(rest) = profile_path.strip_prefix("~/") {
                    dirs::home_dir()
                        .map(|h| h.join(rest))
                        .unwrap_or_else(|| std::path::PathBuf::from(profile_path))
                } else {
                    std::path::PathBuf::from(profile_path)
                };
                let d = drivers::claude_code::ClaudeCodeDriver::with_timeout(
                    config.default_model.base_url.clone(),
                    true, // skip_permissions — daemon mode
                    config.default_model.message_timeout_secs,
                )
                .with_config_dir(dir)
                .with_mcp_bridge(mcp_bridge_cfg.clone());
                let name = format!("profile-{}", i + 1);
                profile_drivers.push((Arc::new(d), name));
            }
            if profile_drivers.len() > 1 {
                info!(
                    pool_size = profile_drivers.len(),
                    "Claude Code CLI profile rotation enabled"
                );
                let rotation = drivers::token_rotation::TokenRotationDriver::new(
                    profile_drivers,
                    config.default_model.provider.clone(),
                );
                driver_chain.push(Arc::new(rotation));
            } else if let Some((d, _)) = profile_drivers.pop() {
                driver_chain.push(d);
            }
        }

        if driver_chain.is_empty() {
            match &primary_result {
                Ok(d) => driver_chain.push(d.clone()),
                Err(e) => {
                    warn!(
                        provider = %config.default_model.provider,
                        error = %e,
                        "Primary LLM driver init failed — trying auto-detect"
                    );
                    // Auto-detect: scan env for any configured provider key
                    if let Some((provider, model_hint, env_var)) =
                        drivers::detect_available_provider()
                    {
                        let model = if model_hint.is_empty() {
                            librefang_runtime::model_catalog::ModelCatalog::default()
                                .default_model_for_provider(provider)
                                .unwrap_or_else(|| "default".to_string())
                        } else {
                            model_hint.to_string()
                        };
                        let auto_config = DriverConfig {
                            provider: provider.to_string(),
                            api_key: std::env::var(env_var).ok(),
                            base_url: config.provider_urls.get(provider).cloned(),
                            vertex_ai: config.vertex_ai.clone(),
                            azure_openai: config.azure_openai.clone(),
                            skip_permissions: true,
                            message_timeout_secs: config.default_model.message_timeout_secs,
                            mcp_bridge: Some(mcp_bridge_cfg.clone()),
                            proxy_url: config.provider_proxy_urls.get(provider).cloned(),
                            request_timeout_secs: config
                                .provider_request_timeout_secs
                                .get(provider)
                                .copied(),
                            emit_caller_trace_headers: config.telemetry.emit_caller_trace_headers,
                        };
                        match drivers::create_driver(&auto_config) {
                            Ok(d) => {
                                let auth_source = if env_var.is_empty() {
                                    "CLI login"
                                } else {
                                    env_var
                                };
                                info!(
                                    provider = %provider,
                                    model = %model,
                                    auth_source = %auth_source,
                                    "Auto-detected provider — using as default"
                                );
                                driver_chain.push(d);
                                // Update the running config so agents get the right model
                                config.default_model.provider = provider.to_string();
                                config.default_model.model = model;
                                config.default_model.api_key_env = env_var.to_string();
                            }
                            Err(e2) => {
                                warn!(provider = %provider, error = %e2, "Auto-detected provider also failed");
                            }
                        }
                    }
                }
            }
        }

        // Add fallback providers to the chain (with model names for cross-provider fallback)
        let mut model_chain: Vec<(Arc<dyn LlmDriver>, String)> = Vec::new();
        // Primary driver uses empty model name (uses the request's model field as-is)
        for d in &driver_chain {
            model_chain.push((d.clone(), String::new()));
        }
        for fb in &config.fallback_providers {
            let fb_api_key = if !fb.api_key_env.is_empty() {
                std::env::var(&fb.api_key_env).ok()
            } else {
                // Resolve using provider_api_keys / convention for custom providers
                let env_var = config.resolve_api_key_env(&fb.provider);
                std::env::var(&env_var).ok()
            };
            let fb_config = DriverConfig {
                provider: fb.provider.clone(),
                api_key: fb_api_key,
                base_url: fb
                    .base_url
                    .clone()
                    .or_else(|| config.provider_urls.get(&fb.provider).cloned()),
                vertex_ai: config.vertex_ai.clone(),
                azure_openai: config.azure_openai.clone(),
                skip_permissions: true,
                message_timeout_secs: config.default_model.message_timeout_secs,
                mcp_bridge: Some(mcp_bridge_cfg.clone()),
                proxy_url: config.provider_proxy_urls.get(&fb.provider).cloned(),
                request_timeout_secs: config
                    .provider_request_timeout_secs
                    .get(&fb.provider)
                    .copied(),
                emit_caller_trace_headers: config.telemetry.emit_caller_trace_headers,
            };
            match drivers::create_driver(&fb_config) {
                Ok(d) => {
                    info!(
                        provider = %fb.provider,
                        model = %fb.model,
                        "Fallback provider configured"
                    );
                    driver_chain.push(d.clone());
                    model_chain.push((d, strip_provider_prefix(&fb.model, &fb.provider)));
                }
                Err(e) => {
                    warn!(
                        provider = %fb.provider,
                        error = %e,
                        "Fallback provider init failed — skipped"
                    );
                }
            }
        }

        // Use the chain, or create a stub driver if everything failed
        let driver: Arc<dyn LlmDriver> = if driver_chain.len() > 1 {
            Arc::new(librefang_runtime::drivers::fallback::FallbackDriver::with_models(model_chain))
        } else if let Some(single) = driver_chain.into_iter().next() {
            single
        } else {
            // All drivers failed — use a stub that returns a helpful error.
            // The kernel boots, dashboard is accessible, users can fix their config.
            warn!("No LLM drivers available — agents will return errors until a provider is configured");
            Arc::new(StubDriver) as Arc<dyn LlmDriver>
        };

        // Initialize metering engine (shares the same SQLite connection as the memory substrate)
        let metering = Arc::new(MeteringEngine::new(Arc::new(
            librefang_memory::usage::UsageStore::new(memory.pool()),
        )));

        // Initialize prompt versioning and A/B experiment store with its own connection
        // to avoid conflicts with UsageStore concurrent writes
        let prompt_store =
            librefang_memory::PromptStore::new_with_path(&db_path, config.memory.pool_size)
                .map_err(|e| KernelError::BootFailed(format!("Prompt store init failed: {e}")))?;

        let supervisor = Supervisor::new();
        let background = BackgroundExecutor::with_concurrency(
            supervisor.subscribe(),
            config.max_concurrent_bg_llm,
        );

        // Initialize WASM sandbox engine (shared across all WASM agents)
        let wasm_sandbox = WasmSandbox::new()
            .map_err(|e| KernelError::BootFailed(format!("WASM sandbox init failed: {e}")))?;

        // Initialize RBAC authentication manager. Tool groups are passed
        // through so per-user `tool_categories` (RBAC M3) can resolve
        // group names to their tool patterns.
        let auth = AuthManager::with_tool_groups(&config.users, &config.tool_policy.groups);
        if auth.is_enabled() {
            info!("RBAC enabled with {} users", auth.user_count());
        }
        // Validate channel-role-mapping role strings at boot so operator
        // typos (e.g. `admin_role = "admn"`) surface as a WARN line at
        // startup rather than as silent default-deny on every message.
        // The runtime path is already strict (RBAC M4); this is purely
        // a visibility fix.
        let typo_count = crate::auth::validate_channel_role_mapping(&config.channel_role_mapping);
        if typo_count > 0 {
            warn!(
                "channel_role_mapping: {typo_count} entr(ies) reference an unrecognized \
                 LibreFang role and will default-deny — see WARN lines above"
            );
        }

        // Initialize git repo for config version control (first boot)
        init_git_if_missing(&config.home_dir);

        // Auto-sync registry content on first boot or after upgrade when
        // Sync registry: downloads if cache is stale, pre-installs providers/agents/integrations.
        // Skips download if cache is fresh; skips copy if files already exist.
        librefang_runtime::registry_sync::sync_registry(
            &config.home_dir,
            config.registry.cache_ttl_secs,
            &config.registry.registry_mirror,
        );

        // One-shot: reclaim the duplicate registry checkout that older
        // librefang versions maintained under `~/.librefang/cache/registry/`.
        // Catalog sync now reads directly from `~/.librefang/registry/` (the
        // directory registry_sync already maintains), so the duplicate is
        // pure waste.
        librefang_runtime::catalog_sync::remove_legacy_cache_dirs(&config.home_dir);

        // Initialize model catalog, detect provider auth, and apply URL overrides
        let mut model_catalog =
            librefang_runtime::model_catalog::ModelCatalog::new(&config.home_dir);
        model_catalog.load_suppressed(
            &config
                .home_dir
                .join("data")
                .join("suppressed_providers.json"),
        );
        model_catalog.load_overrides(&config.home_dir.join("data").join("model_overrides.json"));
        model_catalog.detect_auth();
        // Apply region selections first (lower priority than explicit provider_urls)
        if !config.provider_regions.is_empty() {
            let region_urls = model_catalog.resolve_region_urls(&config.provider_regions);
            if !region_urls.is_empty() {
                model_catalog.apply_url_overrides(&region_urls);
                info!("applied {} provider region override(s)", region_urls.len());
            }
            // Also apply region-specific api_key_env overrides (e.g. minimax china
            // uses MINIMAX_CN_API_KEY instead of MINIMAX_API_KEY). Only inserts if
            // the user hasn't already set an explicit provider_api_keys entry.
            let region_api_keys = model_catalog.resolve_region_api_keys(&config.provider_regions);
            for (provider, env_var) in region_api_keys {
                config.provider_api_keys.entry(provider).or_insert(env_var);
            }
        }
        // Load cached catalog from remote sync (overrides builtins)
        model_catalog.load_cached_catalog_for(&config.home_dir);
        // Apply provider URL overrides from config.toml AFTER loading cached catalog
        // so that user-provided URLs always take precedence over catalog defaults.
        if !config.provider_urls.is_empty() {
            model_catalog.apply_url_overrides(&config.provider_urls);
            info!(
                "applied {} provider URL override(s)",
                config.provider_urls.len()
            );
        }
        if !config.provider_proxy_urls.is_empty() {
            model_catalog.apply_proxy_url_overrides(&config.provider_proxy_urls);
            info!(
                "applied {} provider proxy URL override(s)",
                config.provider_proxy_urls.len()
            );
        }
        // Load user's custom models from ~/.librefang/data/custom_models.json (highest priority)
        let custom_models_path = config.home_dir.join("data").join("custom_models.json");
        model_catalog.load_custom_models(&custom_models_path);
        let available_count = model_catalog.available_models().len();
        let total_count = model_catalog.list_models().len();
        let local_count = model_catalog
            .list_providers()
            .iter()
            .filter(|p| !p.key_required)
            .count();
        info!(
            "Model catalog: {total_count} models, {available_count} available from configured providers ({local_count} local)"
        );

        // Initialize skill registry. Before `load_all()` we set the
        // operator-supplied disabled list so the loader can skip those
        // names at manifest-read time (avoids scanning, prompt-injection
        // checks, and hot-reload traffic for skills the operator never
        // wants active). After the primary dir we fold in any
        // `extra_dirs` — read-only overlays whose skills do NOT override
        // locally-installed skills of the same name (see
        // `load_external_dirs`). The exact same order is repeated in
        // `reload_skills` so hot-reload doesn't silently forget either
        // field.
        let skills_dir = config.home_dir.join("skills");
        let mut skill_registry = librefang_skills::registry::SkillRegistry::new(skills_dir);
        skill_registry.set_disabled_skills(config.skills.disabled.clone());

        match skill_registry.load_all() {
            Ok(count) => {
                if count > 0 {
                    info!("Loaded {count} user skill(s) from skill registry");
                }
            }
            Err(e) => {
                warn!("Failed to load skill registry: {e}");
            }
        }
        if !config.skills.extra_dirs.is_empty() {
            match skill_registry.load_external_dirs(&config.skills.extra_dirs) {
                Ok(count) if count > 0 => {
                    info!(
                        "Loaded {count} external skill(s) from {} extra dir(s)",
                        config.skills.extra_dirs.len()
                    );
                }
                Ok(_) => {}
                Err(e) => {
                    warn!("Failed to load external skill dirs: {e}");
                }
            }
        }
        // In Stable mode, freeze the skill registry
        if config.mode == KernelMode::Stable {
            skill_registry.freeze();
        }

        // Initialize hand registry (curated autonomous packages)
        let hand_registry = librefang_hands::registry::HandRegistry::new();
        router::set_hand_route_home_dir(&config.home_dir);
        let (hand_count, _) = hand_registry.reload_from_disk(&config.home_dir);
        if hand_count > 0 {
            info!("Loaded {hand_count} hand(s)");
        }

        // Run the one-time migration from the legacy two-store layout
        // (`integrations.toml` + `integrations/`) into the unified
        // `config.toml` + `mcp/catalog/` layout. This is a no-op after the
        // first successful run.
        //
        // We reload `config.toml` ONLY when the migrator reports it actually
        // wrote something (`Ok(Some(_))`). Reloading unconditionally would
        // silently replace the caller's in-memory config with whatever is on
        // disk, which is wrong when the caller started the kernel with a
        // non-default config path or a programmatically-built config.
        let migrated = match librefang_runtime::mcp_migrate::migrate_if_needed(&config.home_dir) {
            Ok(Some(summary)) => {
                info!("MCP migration: {summary}");
                true
            }
            Ok(None) => false,
            Err(e) => {
                warn!("MCP migration skipped due to error: {e}");
                false
            }
        };

        // Load the MCP catalog from `~/.librefang/mcp/catalog/`.
        let mut mcp_catalog = librefang_extensions::catalog::McpCatalog::new(&config.home_dir);
        let catalog_count = mcp_catalog.load(&config.home_dir);
        info!("MCP catalog: {catalog_count} template(s) available");

        let config = if migrated {
            let cfg_path = config.home_dir.join("config.toml");
            if cfg_path.is_file() {
                let reloaded = load_config(Some(&cfg_path));
                // Defensive: only accept the reloaded view if it didn't drop
                // any `[[mcp_servers]]` entries the caller already had.
                if reloaded.mcp_servers.len() >= config.mcp_servers.len() {
                    reloaded
                } else {
                    config
                }
            } else {
                config
            }
        } else {
            config
        };
        let all_mcp_servers = config.mcp_servers.clone();

        // Initialize MCP health monitor.
        // [health_check] section overrides [extensions] when explicitly set (non-default).
        let hc_interval = if config.health_check.health_check_interval_secs != 60 {
            config.health_check.health_check_interval_secs
        } else {
            config.extensions.health_check_interval_secs
        };
        let health_config = librefang_extensions::health::HealthMonitorConfig {
            auto_reconnect: config.extensions.auto_reconnect,
            max_reconnect_attempts: config.extensions.reconnect_max_attempts,
            max_backoff_secs: config.extensions.reconnect_max_backoff_secs,
            check_interval_secs: hc_interval,
        };
        let mcp_health = librefang_extensions::health::HealthMonitor::new(health_config);
        // Register every configured MCP server for health monitoring.
        for srv in &all_mcp_servers {
            mcp_health.register(&srv.name);
        }

        // Initialize web tools (multi-provider search + SSRF-protected fetch + caching)
        let cache_ttl = std::time::Duration::from_secs(config.web.cache_ttl_minutes * 60);
        let web_cache = Arc::new(librefang_runtime::web_cache::WebCache::new(cache_ttl));
        let brave_auth_profiles: Vec<(String, u32)> = config
            .auth_profiles
            .get("brave")
            .map(|profiles| {
                profiles
                    .iter()
                    .map(|p| (p.api_key_env.clone(), p.priority))
                    .collect()
            })
            .unwrap_or_default();
        let web_ctx = librefang_runtime::web_search::WebToolsContext {
            search: librefang_runtime::web_search::WebSearchEngine::new(
                config.web.clone(),
                web_cache.clone(),
                brave_auth_profiles,
            ),
            fetch: librefang_runtime::web_fetch::WebFetchEngine::new(
                config.web.fetch.clone(),
                web_cache,
            ),
        };

        // Auto-detect embedding driver for vector similarity search
        let embedding_driver: Option<
            Arc<dyn librefang_runtime::embedding::EmbeddingDriver + Send + Sync>,
        > = if config.memory.fts_only == Some(true) {
            info!("FTS-only memory mode active — skipping embedding driver, using SQLite FTS5 text search");
            None
        } else {
            use librefang_runtime::embedding::create_embedding_driver;
            let configured_model = &config.memory.embedding_model;
            if let Some(ref provider) = config.memory.embedding_provider {
                // Explicit config takes priority — use the configured embedding model.
                // If the user left embedding_model at the default ("all-MiniLM-L6-v2"),
                // pick a sensible default for the chosen provider so we don't send a
                // local model name to a cloud API.
                let model = if configured_model == "all-MiniLM-L6-v2"
                    || configured_model == "text-embedding-3-small"
                {
                    default_embedding_model_for_provider(provider)
                } else {
                    configured_model.as_str()
                };
                let api_key_env = config.memory.embedding_api_key_env.as_deref().unwrap_or("");
                // Prefer the catalog's provider base_url (which already has
                // `config.provider_urls` overrides applied at this point, see
                // `apply_url_overrides` above). Falls back to `provider_urls`
                // directly if the catalog has no entry for this provider —
                // and ultimately to the hardcoded default baked into
                // `create_embedding_driver` if neither source knows.
                let custom_url = model_catalog
                    .get_provider(provider)
                    .map(|p| p.base_url.as_str())
                    .filter(|s| !s.is_empty())
                    .or_else(|| {
                        config
                            .provider_urls
                            .get(provider.as_str())
                            .map(|s| s.as_str())
                    });
                match create_embedding_driver(
                    provider,
                    model,
                    api_key_env,
                    custom_url,
                    config.memory.embedding_dimensions,
                ) {
                    Ok(d) => {
                        info!(provider = %provider, model = %model, "Embedding driver configured from memory config");
                        Some(Arc::from(d))
                    }
                    Err(e) => {
                        warn!(provider = %provider, error = %e, "Embedding driver init failed — falling back to text search");
                        None
                    }
                }
            } else {
                // No explicit provider configured — probe environment to find one.
                use librefang_runtime::embedding::detect_embedding_provider;
                if let Some(detected) = detect_embedding_provider() {
                    let model = if configured_model == "all-MiniLM-L6-v2"
                        || configured_model == "text-embedding-3-small"
                    {
                        default_embedding_model_for_provider(detected)
                    } else {
                        configured_model.as_str()
                    };
                    // Prefer catalog-derived base_url (with user overrides
                    // already applied) over raw `config.provider_urls`, so a
                    // provider entry from the registry with a non-default
                    // base URL (e.g. Cohere's `api.cohere.com/v2`) is actually
                    // honored rather than silently falling back to the
                    // hardcoded default inside `create_embedding_driver`.
                    let provider_url = model_catalog
                        .get_provider(detected)
                        .map(|p| p.base_url.as_str())
                        .filter(|s| !s.is_empty())
                        .or_else(|| config.provider_urls.get(detected).map(|s| s.as_str()));
                    // Determine the API key env var for the detected provider.
                    // `detect_embedding_provider` never returns `"groq"` (Groq
                    // has no embeddings endpoint), so it doesn't appear here.
                    let key_env = match detected {
                        "openai" => "OPENAI_API_KEY",
                        "openrouter" => "OPENROUTER_API_KEY",
                        "mistral" => "MISTRAL_API_KEY",
                        "together" => "TOGETHER_API_KEY",
                        "fireworks" => "FIREWORKS_API_KEY",
                        "cohere" => "COHERE_API_KEY",
                        _ => "",
                    };
                    match create_embedding_driver(
                        detected,
                        model,
                        key_env,
                        provider_url,
                        config.memory.embedding_dimensions,
                    ) {
                        Ok(d) => {
                            info!(provider = %detected, model = %model, "Embedding driver auto-detected");
                            Some(Arc::from(d))
                        }
                        Err(e) => {
                            warn!(provider = %detected, error = %e, "Auto-detected embedding driver init failed — falling back to text search");
                            None
                        }
                    }
                } else {
                    warn!(
                        "No embedding provider available. Set one of: OPENAI_API_KEY, \
                         OPENROUTER_API_KEY, MISTRAL_API_KEY, TOGETHER_API_KEY, \
                         FIREWORKS_API_KEY, COHERE_API_KEY, or configure Ollama. \
                         (GROQ_API_KEY is not accepted — Groq has no embeddings endpoint.)"
                    );
                    None
                }
            }
        };

        let browser_ctx = librefang_runtime::browser::BrowserManager::new(config.browser.clone());

        // Initialize media understanding engine
        let media_engine =
            librefang_runtime::media_understanding::MediaEngine::new(config.media.clone());
        let tts_engine = librefang_runtime::tts::TtsEngine::new(config.tts.clone());
        let media_drivers =
            librefang_runtime::media::MediaDriverCache::new_with_urls(config.provider_urls.clone());
        // Load media provider order from registry
        media_drivers.load_providers_from_registry(model_catalog.list_providers());
        let mut pairing = crate::pairing::PairingManager::new(config.pairing.clone());

        // Load paired devices from database and set up persistence callback
        if config.pairing.enabled {
            match memory.load_paired_devices() {
                Ok(rows) => {
                    let devices: Vec<crate::pairing::PairedDevice> = rows
                        .into_iter()
                        .filter_map(|row| {
                            Some(crate::pairing::PairedDevice {
                                device_id: row["device_id"].as_str()?.to_string(),
                                display_name: row["display_name"].as_str()?.to_string(),
                                platform: row["platform"].as_str()?.to_string(),
                                paired_at: chrono::DateTime::parse_from_rfc3339(
                                    row["paired_at"].as_str()?,
                                )
                                .ok()?
                                .with_timezone(&chrono::Utc),
                                last_seen: chrono::DateTime::parse_from_rfc3339(
                                    row["last_seen"].as_str()?,
                                )
                                .ok()?
                                .with_timezone(&chrono::Utc),
                                push_token: row["push_token"].as_str().map(String::from),
                                api_key_hash: row["api_key_hash"]
                                    .as_str()
                                    .unwrap_or("")
                                    .to_string(),
                            })
                        })
                        .collect();
                    pairing.load_devices(devices);
                }
                Err(e) => {
                    warn!("Failed to load paired devices from database: {e}");
                }
            }

            let persist_memory = Arc::clone(&memory);
            pairing.set_persist(Box::new(move |device, op| match op {
                crate::pairing::PersistOp::Save => {
                    if let Err(e) = persist_memory.save_paired_device(
                        &device.device_id,
                        &device.display_name,
                        &device.platform,
                        &device.paired_at.to_rfc3339(),
                        &device.last_seen.to_rfc3339(),
                        device.push_token.as_deref(),
                        &device.api_key_hash,
                    ) {
                        tracing::warn!("Failed to persist paired device: {e}");
                    }
                }
                crate::pairing::PersistOp::Remove => {
                    if let Err(e) = persist_memory.remove_paired_device(&device.device_id) {
                        tracing::warn!("Failed to remove paired device from DB: {e}");
                    }
                }
            }));
        }

        // Initialize cron scheduler
        let cron_scheduler =
            crate::cron::CronScheduler::new(&config.home_dir, config.max_cron_jobs);
        match cron_scheduler.load() {
            Ok(count) => {
                if count > 0 {
                    info!("Loaded {count} cron job(s) from disk");
                    // Bug #3828: warn about any fires that were missed while the
                    // daemon was down.  We use "5 minutes ago" as a conservative
                    // lower bound because we don't persist a shutdown timestamp;
                    // operators can correlate with daemon restart time in logs.
                    // This only logs warnings — it does not catch-up-fire.
                    let warn_since = chrono::Utc::now() - chrono::Duration::minutes(5);
                    cron_scheduler.log_missed_fires_since(warn_since);
                }
            }
            Err(e) => {
                warn!("Failed to load cron jobs: {e}");
            }
        }
        // Warn about any jobs that missed fires while the daemon was offline,
        // and reschedule them to fire immediately on the next tick (#3828).
        cron_scheduler.warn_missed_fires();

        // Initialize trigger engine and reload persisted triggers
        let trigger_engine = TriggerEngine::with_config(&config.triggers, &config.home_dir);
        match trigger_engine.load() {
            Ok(count) => {
                if count > 0 {
                    info!("Loaded {count} trigger job(s) from disk");
                }
            }
            Err(e) => {
                warn!("Failed to load trigger jobs: {e}");
            }
        }

        // Initialize execution approval manager
        let approval_manager =
            crate::approval::ApprovalManager::new_with_db(config.approval.clone(), memory.pool());

        // Validate notification config — warn (not error) on unrecognized values
        {
            let known_events = [
                "approval_requested",
                "task_completed",
                "task_failed",
                "tool_failure",
            ];
            for (i, rule) in config.notification.agent_rules.iter().enumerate() {
                for event in &rule.events {
                    if !known_events.contains(&event.as_str()) {
                        warn!(
                            rule_index = i,
                            agent_pattern = %rule.agent_pattern,
                            event = %event,
                            known = ?known_events,
                            "Notification agent_rule references unknown event type"
                        );
                    }
                }
            }
        }

        // Initialize binding/broadcast/auto-reply from config
        let initial_bindings = config.bindings.clone();
        let initial_broadcast = config.broadcast.clone();
        let auto_reply_engine = crate::auto_reply::AutoReplyEngine::new(config.auto_reply.clone());
        let initial_budget = config.budget.clone();

        // Initialize command queue with configured concurrency limits
        let command_queue = librefang_runtime::command_lane::CommandQueue::with_capacities(
            config.queue.concurrency.main_lane as u32,
            config.queue.concurrency.cron_lane as u32,
            config.queue.concurrency.subagent_lane as u32,
            config.queue.concurrency.trigger_lane as u32,
        );

        // Build the pluggable context engine from config
        let context_engine_config = librefang_runtime::context_engine::ContextEngineConfig {
            context_window_tokens: 200_000, // default, overridden per-agent at call time
            stable_prefix_mode: config.stable_prefix_mode,
            max_recall_results: 5,
            compaction: Some(config.compaction.clone()),
            output_schema_strict: false,
            max_hook_calls_per_minute: 0,
        };
        let context_engine: Option<Box<dyn librefang_runtime::context_engine::ContextEngine>> = {
            let emb_arc: Option<
                Arc<dyn librefang_runtime::embedding::EmbeddingDriver + Send + Sync>,
            > = embedding_driver.as_ref().map(Arc::clone);
            let vault_path = config.home_dir.join("vault.enc");
            let engine = librefang_runtime::context_engine::build_context_engine(
                &config.context_engine,
                context_engine_config.clone(),
                memory.clone(),
                emb_arc,
                &|secret_name| {
                    let mut vault =
                        librefang_extensions::vault::CredentialVault::new(vault_path.clone());
                    if vault.unlock().is_err() {
                        return None;
                    }
                    vault.get(secret_name).map(|v| v.as_str().to_string())
                },
            );
            Some(engine)
        };

        let workflow_home_dir = config.home_dir.clone();
        let oauth_home_dir = config.home_dir.clone();
        let checkpoint_base_dir = config.home_dir.clone();
        let a2a_db_path = config.data_dir.join("a2a_tasks.db");
        // Resolve the audit anchor path from `[audit].anchor_path`. When
        // unset, the default is `data_dir/audit.anchor` — good enough to
        // catch most casual tampering since it sits next to the SQLite
        // file. When the operator points it somewhere the daemon can
        // write to but unprivileged code cannot (chmod-0400 file, systemd
        // `ReadOnlyPaths=` mount, NFS share, pipe to `logger`), the same
        // rewrite check becomes a real supply-chain boundary. Relative
        // paths resolve against `data_dir` so operators can write
        // `anchor_path = "audit/tip.anchor"` without hard-coding an
        // absolute path in config.toml.
        let audit_anchor_path = match config.audit.anchor_path.as_ref() {
            Some(path) if path.is_absolute() => path.clone(),
            Some(path) => config.data_dir.join(path),
            None => config.data_dir.join("audit.anchor"),
        };
        let hooks_dir = config.home_dir.join("hooks");
        // Optional memory-wiki vault (#3329). Off by default; only
        // constructed when the operator has flipped `[memory_wiki]
        // enabled = true`. A construction failure (e.g. unwritable
        // vault path) logs a warning and disables the vault for this
        // boot — it must not abort the kernel because the rest of the
        // daemon is independent of the wiki feature. Lost during the
        // kernel/mod split; restored here alongside the
        // `wiki_vault` field on `LibreFangKernel` and the trait
        // method bodies in `handles/wiki_access.rs`.
        let wiki_vault: Option<Arc<librefang_memory_wiki::WikiVault>> =
            if config.memory_wiki.enabled {
                match librefang_memory_wiki::WikiVault::new(&config.memory_wiki, &config.home_dir) {
                    Ok(v) => Some(Arc::new(v)),
                    Err(err) => {
                        tracing::warn!(
                            error = %err,
                            "[memory_wiki] enabled but vault construction failed; \
                             wiki tools will return KernelOpError::unavailable"
                        );
                        None
                    }
                }
            } else {
                None
            };
        // Snapshot the initial taint rule registry into a shared
        // `Arc<ArcSwap<...>>`. This swap is the single source of truth read
        // by every connected MCP server's scanner — `Self::reload_config`
        // calls `.store(...)` on it so config edits propagate without
        // restarting servers.
        let initial_taint_rules =
            std::sync::Arc::new(arc_swap::ArcSwap::from_pointee(config.taint_rules.clone()));
        // Build the aux client BEFORE moving `config` into the struct so we
        // can clone the snapshot without re-loading from the swap. The
        // primary driver is shared by `Arc::clone` so failover behaviour
        // matches the kernel's main `default_driver`.
        let initial_aux_client = librefang_runtime::aux_client::AuxClient::new(
            std::sync::Arc::new(config.clone()),
            Arc::clone(&driver),
        );
        // Pre-parse `config.toml` once at boot so the per-message hot path
        // never has to re-read it (#3722). Errors here are non-fatal — the
        // skill config injection layer treats a missing/invalid file as an
        // empty table, which is the same semantics as the previous on-miss
        // path.
        let initial_raw_config_toml = load_raw_config_toml(&config.home_dir.join("config.toml"));

        // Canonical agent UUID registry (refs #4614). Loaded from
        // `<home_dir>/agent_identities.toml`; missing or malformed files
        // start the registry empty (the load helper logs the cause).
        let agent_identities = Arc::new(
            crate::agent_identity_registry::AgentIdentityRegistry::load(&config.home_dir),
        );
        if !agent_identities.is_empty() {
            info!(
                count = agent_identities.len(),
                "Loaded canonical agent UUID registry"
            );
        }

        let kernel = Self {
            home_dir_boot: config.home_dir.clone(),
            data_dir_boot: config.data_dir.clone(),
            config: ArcSwap::new(std::sync::Arc::new(config)),
            raw_config_toml: ArcSwap::new(std::sync::Arc::new(initial_raw_config_toml)),
            registry: AgentRegistry::new(),
            agent_identities,
            capabilities: CapabilityManager::new(),
            event_bus: EventBus::new(),
            session_lifecycle_bus: Arc::new(crate::session_lifecycle::SessionLifecycleBus::new(
                256,
            )),
            session_stream_hub: Arc::new(crate::session_stream_hub::SessionStreamHub::new()),
            scheduler: AgentScheduler::new(),
            memory: memory.clone(),
            wiki_vault: wiki_vault.clone(),
            proactive_memory: OnceLock::new(),
            proactive_memory_extractor: OnceLock::new(),
            prompt_store: OnceLock::new(),
            supervisor,
            workflows: crate::kernel::subsystems::WorkflowSubsystem::new(
                WorkflowEngine::new_with_persistence(&workflow_home_dir),
                trigger_engine,
                background,
                cron_scheduler,
                command_queue,
            ),
            // ArcSwap lets config_reload rebuild on `[llm.auxiliary]` edits
            // without invalidating any long-lived `Arc<Kernel>` handle.
            llm: crate::kernel::subsystems::LlmSubsystem::new(
                driver,
                initial_aux_client,
                embedding_driver,
                model_catalog,
            ),
            wasm_sandbox,
            security: crate::kernel::subsystems::SecuritySubsystem::new(auth, pairing),
            skills: crate::kernel::subsystems::SkillsSubsystem::new(
                skill_registry,
                hand_registry,
                Self::MAX_INFLIGHT_SKILL_REVIEWS,
            ),
            running_tasks: dashmap::DashMap::new(),
            session_interrupts: dashmap::DashMap::new(),
            mcp: crate::kernel::subsystems::McpSubsystem::new(
                Arc::new(crate::mcp_oauth_provider::KernelOAuthProvider::new(
                    oauth_home_dir,
                )),
                mcp_catalog,
                mcp_health,
                all_mcp_servers,
            ),
            a2a_task_store: librefang_runtime::a2a::A2aTaskStore::with_persistence(
                1000,
                &a2a_db_path,
            ),
            a2a_external_agents: std::sync::Mutex::new(Vec::new()),
            media: crate::kernel::subsystems::MediaSubsystem::new(
                web_ctx,
                browser_ctx,
                media_engine,
                tts_engine,
                media_drivers,
            ),
            delivery_tracker: DeliveryTracker::new(),
            approval_manager,
            bindings: std::sync::Mutex::new(initial_bindings),
            broadcast: initial_broadcast,
            auto_reply_engine,
            hooks: librefang_runtime::hooks::HookRegistry::new(),
            external_hooks: crate::hooks::ExternalHookSystem::load(hooks_dir),
            processes: crate::kernel::subsystems::ProcessSubsystem::new(
                Arc::new(librefang_runtime::process_manager::ProcessManager::new(5)),
                Arc::new(librefang_runtime::process_registry::ProcessRegistry::new()),
            ),
            peer_registry: OnceLock::new(),
            peer_node: OnceLock::new(),
            booted_at: std::time::Instant::now(),
            whatsapp_gateway_pid: Arc::new(std::sync::Mutex::new(None)),
            channel_adapters: dashmap::DashMap::new(),
            tool_policy_override: std::sync::RwLock::new(None),
            agent_msg_locks: dashmap::DashMap::new(),
            session_msg_locks: dashmap::DashMap::new(),
            agent_concurrency: dashmap::DashMap::new(),
            hand_runtime_override_locks: dashmap::DashMap::new(),
            injection_senders: dashmap::DashMap::new(),
            injection_receivers: dashmap::DashMap::new(),
            assistant_routes: dashmap::DashMap::new(),
            route_divergence: dashmap::DashMap::new(),
            decision_traces: dashmap::DashMap::new(),
            context_engine,
            context_engine_config,
            self_handle: OnceLock::new(),
            provider_unconfigured_logged: std::sync::atomic::AtomicBool::new(false),
            config_reload_lock: tokio::sync::RwLock::new(()),
            prompt_metadata_cache: PromptMetadataCache::new(),
            agent_watchers: dashmap::DashMap::new(),
            metering: crate::kernel::subsystems::MeteringSubsystem::new(
                Arc::new(AuditLog::with_db_anchored(memory.pool(), audit_anchor_path)),
                metering,
                initial_budget,
            ),
            approval_sweep_started: AtomicBool::new(false),
            task_board_sweep_started: AtomicBool::new(false),
            session_stream_hub_gc_started: AtomicBool::new(false),
            shutdown_tx: tokio::sync::watch::channel(false).0,
            checkpoint_manager: {
                let cp_dir = checkpoint_base_dir
                    .join(librefang_runtime::checkpoint_manager::CHECKPOINT_BASE);
                Some(Arc::new(
                    librefang_runtime::checkpoint_manager::CheckpointManager::new(cp_dir),
                ))
            },
            taint_rules_swap: initial_taint_rules,
            log_reloader: OnceLock::new(),
        };

        // Initialize proactive memory system (mem0-style) from config.
        // Uses extraction_model if set, otherwise falls back to agent's default model.
        // This allows using a cheap model (e.g., llama/haiku) for extraction while
        // keeping an expensive model (e.g., opus/gpt-4o) for agent responses.
        let cfg = kernel.config.load();
        if cfg.proactive_memory.enabled {
            let pm_config = cfg.proactive_memory.clone();
            let extraction_model = pm_config
                .extraction_model
                .clone()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| cfg.default_model.model.clone());
            // Strip provider prefix (e.g. "minimax/minimax-M2.5-highspeed" → "minimax-M2.5-highspeed")
            // so the model name is valid for the upstream API.
            let extraction_model = librefang_runtime::agent_loop::strip_provider_prefix(
                &extraction_model,
                &cfg.default_model.provider,
            );
            let llm = Some((
                Arc::clone(&kernel.llm.default_driver) as _,
                extraction_model,
            ));
            // Use the _with_extractor variant so we get the concrete
            // `LlmMemoryExtractor` back alongside the store. The extractor
            // needs a `Weak<dyn KernelHandle>` installed before its fork-
            // based extraction path can light up, and that weak ref can
            // only be formed after `Arc::new(kernel)` — so we hold the
            // concrete handle here and call `install_kernel_handle` from
            // `set_self_handle` below.
            let embedding = kernel.llm.embedding_driver.as_ref().map(Arc::clone);
            // Thread the global `prompt_caching` toggle through so the
            // extractor's fallback `driver.complete()` path respects the
            // same switch operators use for the main loop. The fork path
            // inherits caching from the agent's manifest metadata which
            // the kernel derives from this same flag.
            let prompt_caching = cfg.prompt_caching;
            let result =
                librefang_runtime::proactive_memory::init_proactive_memory_full_with_extractor(
                    Arc::clone(&kernel.memory),
                    pm_config,
                    llm,
                    embedding,
                    prompt_caching,
                );
            if let Some((store, extractor)) = result {
                let _ = kernel.proactive_memory.set(store);
                if let Some(ex) = extractor {
                    let _ = kernel.proactive_memory_extractor.set(ex);
                }
            }
        }

        // Initialize prompt store
        let _ = kernel.prompt_store.set(prompt_store);

        // Pre-load persisted hand instance configs so the per-agent drift
        // detection below can re-render the `## User Configuration` settings
        // tail after overwriting the DB manifest with the bare disk TOML.
        // Without this, every restart strips configured settings from the
        // system prompt of any hand-spawned agent until somebody manually
        // re-runs `hand activate` (issue: settings drift on restart).
        //
        // Hand instances themselves aren't restored into `hand_registry` yet
        // — that happens later in `start_background_agents` via
        // `activate_hand_with_id`. Reading `hand_state.json` directly is the
        // cheapest way to recover the user-chosen config at this point in
        // boot.
        let persisted_hand_configs: std::collections::HashMap<
            String,
            std::collections::HashMap<String, serde_json::Value>,
        > = {
            let state_path = cfg.home_dir.join("data").join("hand_state.json");
            librefang_hands::registry::HandRegistry::load_state_detailed(&state_path)
                .entries
                .into_iter()
                .map(|e| (e.hand_id, e.config))
                .collect()
        };

        // Restore persisted agents from SQLite
        match kernel.memory.load_all_agents() {
            Ok(agents) => {
                let count = agents.len();
                for entry in agents {
                    if entry.is_hand {
                        continue;
                    }
                    let agent_id = entry.id;
                    let name = entry.name.clone();

                    // Check if TOML on disk is newer/different — if so, update from file
                    let mut entry = entry;
                    let fallback_toml_path = {
                        let safe_name = safe_path_component(&name, "agent");
                        cfg.effective_agent_workspaces_dir()
                            .join(safe_name)
                            .join("agent.toml")
                    };
                    // Prefer stored source path when it still exists; otherwise
                    // fall back to the canonical workspaces/agents/<name>/ location.
                    // This self-heals entries whose source_toml_path was recorded
                    // under the legacy `<home>/agents/<name>/` layout and later
                    // relocated by `migrate_legacy_agent_dirs`.
                    let (toml_path, source_path_changed) = match entry.source_toml_path.clone() {
                        Some(p) if p.exists() => (p, false),
                        Some(_) => {
                            // Stored path no longer exists — repoint at the
                            // canonical location if the fallback resolves.
                            let repoint = fallback_toml_path.exists();
                            (fallback_toml_path, repoint)
                        }
                        None => (fallback_toml_path, false),
                    };
                    if source_path_changed {
                        entry.source_toml_path = Some(toml_path.clone());
                        if let Err(e) = kernel.memory.save_agent(&entry) {
                            warn!(
                                agent = %name,
                                "Failed to persist source_toml_path repoint: {e}"
                            );
                        } else {
                            info!(
                                agent = %name,
                                path = %toml_path.display(),
                                "Repointed stale source_toml_path to workspaces/agents/"
                            );
                        }
                    }
                    if toml_path.exists() {
                        match std::fs::read_to_string(&toml_path) {
                            Ok(toml_str) => {
                                // Try the hand-extraction path FIRST, then fall back
                                // to parsing as a flat AgentManifest.
                                //
                                // Order matters: AgentManifest deserialization is lenient
                                // and will silently accept a hand.toml as a "partial"
                                // AgentManifest, picking up top-level `name`/`description`
                                // and defaulting `model.system_prompt` to the
                                // ModelConfig::default() stub ("You are a helpful AI agent.")
                                // because the real prompt is nested under `[agents.<role>.model]`
                                // and never reached. The hand-extraction path correctly walks
                                // the nested structure; HandDefinition deserialization requires
                                // top-level `id` + `category` so it cleanly returns None for
                                // standalone agent.toml files.
                                let parsed = extract_manifest_from_hand_toml(&toml_str, &name)
                                    .or_else(|| {
                                        toml::from_str::<librefang_types::agent::AgentManifest>(
                                            &toml_str,
                                        )
                                        .ok()
                                    });
                                match parsed {
                                    Some(mut disk_manifest) => {
                                        // Compare manifests on a projection that strips
                                        // every known runtime-rendered prompt tail
                                        // (## User Configuration, ## Reference Knowledge,
                                        // ## Your Team) before serialization. The disk
                                        // TOML never carries any of these (they are
                                        // re-rendered at activation/drift time), so a
                                        // raw diff would always trigger on
                                        // hand-with-rendered-tail agents and clobber the
                                        // DB blob with the bare TOML on every restart.
                                        // Comparing on the projection means drift only
                                        // fires when the *source* TOML genuinely
                                        // diverged from the DB form.
                                        let changed =
                                            serde_json::to_value(manifest_for_diff(&disk_manifest))
                                                .ok()
                                                != serde_json::to_value(manifest_for_diff(
                                                    &entry.manifest,
                                                ))
                                                .ok();
                                        if changed {
                                            info!(
                                                agent = %name,
                                                path = %toml_path.display(),
                                                "Agent TOML on disk differs from DB, updating"
                                            );
                                            // Preserve runtime-only fields that TOML files don't carry
                                            if disk_manifest.workspace.is_none() {
                                                disk_manifest.workspace =
                                                    entry.manifest.workspace.clone();
                                            }
                                            if disk_manifest.tags.is_empty() {
                                                disk_manifest.tags = entry.manifest.tags.clone();
                                            }
                                            // Always preserve the canonical name. For hand-derived
                                            // agents the DB name is "{hand_id}:{manifest.name}"
                                            // (stamped at hand activation — grep for
                                            // `format!("{hand_id}:{}", manifest.name)`) while the
                                            // TOML only carries the bare "{manifest.name}". Letting
                                            // the disk version overwrite the canonical name here
                                            // would break `find_by_name` lookups, channel routing,
                                            // and peer discovery — all of which key on the colon
                                            // form. Mirrors the runtime hot-reload path lower in
                                            // this file.
                                            disk_manifest.name = entry.manifest.name.clone();
                                            entry.manifest = disk_manifest;

                                            // Re-render the `## User Configuration` tail that the
                                            // bare disk TOML never carries. Without this, a hand
                                            // with `[[settings]]` silently loses its configured
                                            // values from the system prompt on every restart, and
                                            // the agent improvises (or fails) until somebody
                                            // re-activates the hand by hand. Mirrors the activation
                                            // path in `activate_hand_with_id`.
                                            // The AgentEntry.tags field is not persisted to SQLite
                                            // (see librefang-memory/src/structured.rs::load_agent
                                            // which always returns tags = vec![]); the actual
                                            // hand membership tag lives on manifest.tags. Read
                                            // there to identify the owning hand. We use the DB
                                            // (entry.manifest before the swap to disk_manifest)
                                            // because the disk TOML manifest typically doesn't
                                            // carry the runtime-stamped `hand:<id>` tag either.
                                            if let Some(hand_id) = entry
                                                .manifest
                                                .tags
                                                .iter()
                                                .find_map(|t| t.strip_prefix("hand:"))
                                                .map(|s| s.to_string())
                                            {
                                                if let Some(def) = kernel
                                                    .skills
                                                    .hand_registry
                                                    .get_definition(&hand_id)
                                                {
                                                    if !def.settings.is_empty() {
                                                        let empty =
                                                            std::collections::HashMap::new();
                                                        let cfg_for_settings =
                                                            persisted_hand_configs
                                                                .get(&hand_id)
                                                                .unwrap_or(&empty);
                                                        let _ = apply_settings_block_to_manifest(
                                                            &mut entry.manifest,
                                                            &def.settings,
                                                            cfg_for_settings,
                                                        );
                                                    }

                                                    // Re-render `## Reference Knowledge` and
                                                    // `## Your Team` tails — like the settings
                                                    // tail above, the bare disk TOML never
                                                    // carries them, so without re-rendering
                                                    // here the agent silently loses skill
                                                    // discoverability and peer awareness on
                                                    // every restart. Helpers are
                                                    // unconditionally idempotent: empty skill
                                                    // content / single-agent hand / no peers
                                                    // all collapse to a strip-only call that
                                                    // also clears any stale tail left over
                                                    // from when the hand previously had
                                                    // those.
                                                    //
                                                    // Recover the agent's role from the
                                                    // `hand_role:<role>` tag stamped at
                                                    // activation. Skip silently when the tag
                                                    // is missing — the agent isn't
                                                    // hand-derived in a way we recognise, and
                                                    // the activation path will re-stamp the
                                                    // tags on the next `hand activate`.
                                                    let role_opt = entry
                                                        .manifest
                                                        .tags
                                                        .iter()
                                                        .find_map(|t| t.strip_prefix("hand_role:"))
                                                        .map(|s| s.to_string());
                                                    if let Some(role) = role_opt {
                                                        apply_skill_reference_block_to_manifest(
                                                            &mut entry.manifest,
                                                            &role,
                                                            &def,
                                                        );
                                                        apply_team_block_to_manifest(
                                                            &mut entry.manifest,
                                                            &role,
                                                            &def,
                                                        );
                                                    } else {
                                                        // Hand membership is known (we're inside
                                                        // the `hand:<id>` branch) but the role tag
                                                        // wasn't stamped — this agent will boot
                                                        // without skill discoverability or peer
                                                        // awareness until somebody re-runs
                                                        // `hand activate`. Log so the silent
                                                        // degradation is at least greppable.
                                                        debug!(
                                                            agent = %name,
                                                            hand = %hand_id,
                                                            "hand_role:<role> tag missing on \
                                                             hand-derived agent; skipping skill/team \
                                                             tail re-render until next hand activate"
                                                        );
                                                    }
                                                }
                                            }

                                            // Persist the update back to DB
                                            if let Err(e) = kernel.memory.save_agent(&entry) {
                                                warn!(
                                                    agent = %name,
                                                    "Failed to persist TOML update: {e}"
                                                );
                                            }

                                            // Re-materialize named workspaces and rewrite TOOLS.md
                                            // so a HAND.toml gaining `[agents.<role>.workspaces]`
                                            // (or any other manifest change that affects what's
                                            // injected into TOOLS.md) takes effect on `restart`
                                            // without forcing a hand deactivate/reactivate cycle —
                                            // which would destroy triggers, cron jobs, and runtime
                                            // sessions. Both helpers are idempotent: the dir is
                                            // create_dir_all'd, TOOLS.md is force-rewritten with
                                            // truncate, and user-editable identity files use
                                            // create_new so manual edits are preserved.
                                            //
                                            // Skip when workspace is None — a manifest without a
                                            // resolved workspace path has never been spawned, so
                                            // the normal spawn flow at register_agent() will run
                                            // these helpers when activation eventually happens.
                                            if let Some(ref ws_dir) = entry.manifest.workspace {
                                                let resolved_workspaces = ensure_named_workspaces(
                                                    &cfg.effective_workspaces_dir(),
                                                    &entry.manifest.workspaces,
                                                    &cfg.allowed_mount_roots,
                                                );
                                                if entry.manifest.generate_identity_files {
                                                    generate_identity_files(
                                                        ws_dir,
                                                        &entry.manifest,
                                                        &resolved_workspaces,
                                                    );
                                                }
                                            }
                                        }
                                    }
                                    None => {
                                        warn!(
                                            agent = %name,
                                            path = %toml_path.display(),
                                            "Cannot parse TOML on disk as agent manifest, using DB version"
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                warn!(
                                    agent = %name,
                                    "Failed to read agent TOML: {e}"
                                );
                            }
                        }
                    }

                    // Re-grant capabilities
                    let caps = manifest_to_capabilities(&entry.manifest);
                    kernel.capabilities.grant(agent_id, caps);

                    // Re-register with scheduler
                    kernel
                        .scheduler
                        .register(agent_id, entry.manifest.resources.clone());

                    // Re-register in the in-memory registry
                    let mut restored_entry = entry;
                    restored_entry.last_active = chrono::Utc::now();

                    // Check enabled flag — also do a direct TOML read as fallback
                    let mut is_enabled = restored_entry.manifest.enabled;
                    if is_enabled {
                        // Double-check: read directly from workspaces/{agents,hands}/
                        // TOML in case DB is stale. Use proper TOML parsing instead
                        // of string matching to handle all valid whitespace variants
                        // and avoid false positives from comments.
                        let candidates = [
                            cfg.effective_agent_workspaces_dir()
                                .join(&name)
                                .join("agent.toml"),
                            cfg.effective_hands_workspaces_dir()
                                .join(&name)
                                .join("agent.toml"),
                        ];
                        for check_path in &candidates {
                            if check_path.exists() {
                                if let Ok(content) = std::fs::read_to_string(check_path) {
                                    if toml_enabled_false(&content) {
                                        is_enabled = false;
                                        restored_entry.manifest.enabled = false;
                                    }
                                }
                                break;
                            }
                        }
                    }
                    // Reconciliation (#3665): if the persisted state is
                    // `Running` but no in-memory process actually exists
                    // (the registry was wiped by `shutdown()` or a crash),
                    // a previous shutdown failed to persist `Suspended`.
                    // Emit a warning so unclean shutdowns are visible in
                    // logs rather than silently re-spawning into a state
                    // that looks identical to a clean boot.
                    if matches!(
                        restored_entry.state,
                        AgentState::Running | AgentState::Crashed
                    ) {
                        warn!(
                            agent = %name,
                            id = %agent_id,
                            prev_state = ?restored_entry.state,
                            "Agent restored from non-clean state — last shutdown likely \
                             crashed before persisting Suspended. Reconciling state on boot."
                        );
                    }
                    if is_enabled {
                        restored_entry.state = AgentState::Running;
                    } else {
                        restored_entry.state = AgentState::Suspended;
                        info!(agent = %name, "Agent disabled in config — starting as Suspended");
                    }

                    // Inherit kernel exec_policy for agents that lack one.
                    // Promote to Full when shell_exec is declared in capabilities.
                    if restored_entry.manifest.exec_policy.is_none() {
                        if restored_entry
                            .manifest
                            .capabilities
                            .tools
                            .iter()
                            .any(|t| t == "shell_exec" || t == "*")
                        {
                            restored_entry.manifest.exec_policy =
                                Some(librefang_types::config::ExecPolicy {
                                    mode: librefang_types::config::ExecSecurityMode::Full,
                                    ..cfg.exec_policy.clone()
                                });
                        } else {
                            restored_entry.manifest.exec_policy = Some(cfg.exec_policy.clone());
                        }
                    }

                    // Apply global budget defaults to restored agents
                    apply_budget_defaults(
                        &kernel.budget_config(),
                        &mut restored_entry.manifest.resources,
                    );

                    // Apply default_model to restored agents.
                    //
                    // Three cases:
                    // 1. Agent has empty/default provider → always apply default_model
                    // 2. Agent's source TOML defines provider="default" → the DB value
                    //    is a stale resolved provider from a previous config; override it
                    // 3. Agent named "assistant" (auto-spawned) → update to match
                    //    default_model so config.toml changes take effect on restart
                    {
                        let dm = &cfg.default_model;
                        let is_default_provider = restored_entry.manifest.model.provider.is_empty()
                            || restored_entry.manifest.model.provider == "default";
                        let is_default_model = restored_entry.manifest.model.model.is_empty()
                            || restored_entry.manifest.model.model == "default";

                        // Also check the source TOML: if the agent definition says
                        // provider="default", the persisted value is stale and must
                        // be overridden with the current default_model.
                        let toml_says_default = toml_path.exists()
                            && std::fs::read_to_string(&toml_path)
                                .ok()
                                .and_then(|s| {
                                    toml::from_str::<librefang_types::agent::AgentManifest>(&s).ok()
                                })
                                .map(|m| {
                                    (m.model.provider.is_empty() || m.model.provider == "default")
                                        && (m.model.model.is_empty() || m.model.model == "default")
                                })
                                .unwrap_or(false);

                        let is_auto_spawned = restored_entry.name == "assistant"
                            && restored_entry.manifest.description == "General-purpose assistant";
                        if is_default_provider && is_default_model
                            || toml_says_default
                            || is_auto_spawned
                        {
                            if !dm.provider.is_empty() {
                                restored_entry.manifest.model.provider = dm.provider.clone();
                            }
                            if !dm.model.is_empty() {
                                restored_entry.manifest.model.model = dm.model.clone();
                            }
                            if !dm.api_key_env.is_empty() {
                                restored_entry.manifest.model.api_key_env =
                                    Some(dm.api_key_env.clone());
                            }
                            if dm.base_url.is_some() {
                                restored_entry
                                    .manifest
                                    .model
                                    .base_url
                                    .clone_from(&dm.base_url);
                            }
                            // Merge extra_params from default_model
                            for (key, value) in &dm.extra_params {
                                restored_entry
                                    .manifest
                                    .model
                                    .extra_params
                                    .entry(key.clone())
                                    .or_insert(value.clone());
                            }
                        }
                    }

                    // SECURITY (#3533): skip any restored agent whose
                    // on-disk `module` path escapes the LibreFang home
                    // dir. Logging the rejection is enough — refusing to
                    // boot the whole daemon for one bad manifest would
                    // turn a CVE into a DoS, and the agent stays out of
                    // the registry so no codepath can invoke it.
                    if let Err(e) = validate_manifest_module_path(&restored_entry.manifest, &name) {
                        tracing::error!(
                            agent = %name,
                            error = %e,
                            "Refusing to restore agent with invalid module path; \
                             check agent.toml for absolute paths or '..' traversal"
                        );
                        continue;
                    }
                    if let Err(e) = kernel.registry.register(restored_entry) {
                        tracing::warn!(agent = %name, "Failed to restore agent: {e}");
                    } else {
                        tracing::debug!(agent = %name, id = %agent_id, "Restored agent");
                    }
                }
                if count > 0 {
                    info!("Restored {count} agent(s) from persistent storage");
                }
            }
            Err(e) => {
                tracing::warn!("Failed to load persisted agents: {e}");
            }
        }

        // One-time webui → canonical session migration.
        //
        // Before the unify fix, the dashboard WS wrote to
        // `SessionId::for_channel(agent, "webui")` while GET /session and the
        // sessions management endpoints read `entry.session_id`. Any agent
        // with recent dashboard chat therefore has two sessions: the stale
        // canonical and the active webui one. Adopt the webui session as the
        // canonical pointer when it has strictly more messages, so existing
        // conversations show up after the fix.
        //
        // Idempotent: once `entry.session_id` matches the webui session id
        // (or canonical overtakes it), this is a no-op on subsequent boots.
        {
            let registry_snapshot: Vec<(AgentId, SessionId)> = kernel
                .registry
                .list()
                .iter()
                .map(|e| (e.id, e.session_id))
                .collect();
            for (agent_id, canonical_session_id) in registry_snapshot {
                let webui_session_id = SessionId::for_channel(agent_id, "webui");
                if webui_session_id == canonical_session_id {
                    continue;
                }
                let webui_msgs = match kernel.memory.get_session(webui_session_id) {
                    Ok(Some(s)) => s.messages.len(),
                    _ => continue,
                };
                if webui_msgs == 0 {
                    continue;
                }
                // Inspect canonical: if the user has deliberately labeled it
                // (via create_agent_session / switch_agent_session from the
                // sessions UI), treat that as an explicit choice and don't
                // override it — they can still find the orphaned webui session
                // in `list_agent_sessions` and switch manually if desired.
                let canonical_session = kernel
                    .memory
                    .get_session(canonical_session_id)
                    .ok()
                    .flatten();
                if canonical_session
                    .as_ref()
                    .and_then(|s| s.label.as_ref())
                    .is_some()
                {
                    info!(
                        agent_id = %agent_id,
                        webui_messages = webui_msgs,
                        "Skipping webui adoption — canonical session is labeled (user-managed)"
                    );
                    continue;
                }
                let canonical_msgs = canonical_session.map(|s| s.messages.len()).unwrap_or(0);
                if webui_msgs <= canonical_msgs {
                    continue;
                }
                if let Err(e) = kernel
                    .registry
                    .update_session_id(agent_id, webui_session_id)
                {
                    warn!(agent_id = %agent_id, "Failed to adopt webui session: {e}");
                    continue;
                }
                if let Some(entry) = kernel.registry.get(agent_id) {
                    if let Err(e) = kernel.memory.save_agent(&entry) {
                        warn!(agent_id = %agent_id, "Failed to persist webui adoption: {e}");
                    }
                }
                info!(
                    agent_id = %agent_id,
                    webui_messages = webui_msgs,
                    canonical_messages = canonical_msgs,
                    "Adopted webui channel session as canonical (one-time migration)"
                );
            }
        }

        // If no agents exist (fresh install), spawn a default assistant.
        if kernel.registry.list().is_empty() {
            info!("No agents found — spawning default assistant");
            let manifest = router::load_template_manifest(&kernel.home_dir_boot, "assistant")
                .or_else(|_| {
                    // Fallback: minimal assistant for zero-network boot (init not yet run)
                    toml::from_str::<librefang_types::agent::AgentManifest>(
                        r#"
name = "assistant"
description = "General-purpose assistant"
module = "builtin:chat"
tags = ["general", "assistant"]
[model]
provider = "default"
model = "default"
max_tokens = 8192
temperature = 0.5
system_prompt = "You are a helpful assistant."
"#,
                    )
                    .map_err(|e| format!("fallback manifest parse error: {e}"))
                })
                .map_err(|e| {
                    KernelError::BootFailed(format!("failed to load assistant template: {e}"))
                })?;
            match kernel.spawn_agent(manifest) {
                Ok(id) => info!(id = %id, "Default assistant spawned"),
                Err(e) => warn!("Failed to spawn default assistant: {e}"),
            }
        }

        // Auto-register workflow definitions from ~/.librefang/workflows/
        {
            let workflows_dir = kernel.home_dir_boot.join("workflows");
            let loaded = tokio::task::block_in_place(|| {
                kernel.workflows.engine.load_from_dir_sync(&workflows_dir)
            });
            if loaded > 0 {
                info!(
                    "Auto-registered {loaded} workflow(s) from {}",
                    workflows_dir.display()
                );
            }
        }

        // Load persisted workflow runs (completed/failed) from disk.
        {
            match tokio::task::block_in_place(|| kernel.workflows.engine.load_runs()) {
                Ok(count) if count > 0 => {
                    info!("Loaded {count} persisted workflow run(s) from disk");
                }
                Err(e) => {
                    warn!("Failed to load persisted workflow runs: {e}");
                }
                _ => {}
            }

            // Recover any runs left in Running/Pending state by a prior crash.
            // `recover_stale_running_runs` is a synchronous DashMap walk — no
            // need for `block_in_place` (the runs map is no longer behind a
            // tokio RwLock as of #3969).
            let stale_timeout_mins = kernel.config.load().workflow_stale_timeout_minutes;
            if stale_timeout_mins > 0 {
                let stale_timeout = std::time::Duration::from_secs(stale_timeout_mins * 60);
                let recovered = kernel
                    .workflows
                    .engine
                    .recover_stale_running_runs(stale_timeout);
                if recovered > 0 {
                    info!(
                        "Recovered {recovered} stale workflow run(s) interrupted by daemon restart"
                    );
                }
            }
        }

        // Load workflow templates
        {
            let user_dir = kernel.home_dir_boot.join("workflows").join("templates");
            let loaded = kernel
                .workflows
                .template_registry
                .load_templates_from_dir(&user_dir);
            if loaded > 0 {
                info!("Loaded {loaded} workflow template(s)");
            }
        }

        // Validate routing configs against model catalog
        for entry in kernel.registry.list() {
            if let Some(ref routing_config) = entry.manifest.routing {
                let router = ModelRouter::new(routing_config.clone());
                for warning in router.validate_models(&kernel.llm.model_catalog.load()) {
                    warn!(agent = %entry.name, "{warning}");
                }
            }
        }

        // Validate kernel-wide default_routing (issue #4466) so the init
        // wizard's Smart Router selection surfaces alias / unknown-model
        // warnings at boot, not silently at first dispatch.
        if let Some(ref routing_config) = kernel.config.load().default_routing {
            let router = ModelRouter::new(routing_config.clone());
            for warning in router.validate_models(&kernel.llm.model_catalog.load()) {
                warn!(target: "librefang_kernel::default_routing", "{warning}");
            }
        }

        info!("LibreFang kernel booted successfully");
        Ok(kernel)
    }
}
