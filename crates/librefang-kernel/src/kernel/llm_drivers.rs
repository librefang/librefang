//! Cluster pulled out of mod.rs in #4713 phase 3e/6.
//!
//! Hosts the kernel's LLM driver-resolution surface: provider URL
//! lookup (`lookup_provider_url`) and the driver chain construction
//! that wraps the primary driver in fallbacks when configured. These
//! methods bridge the in-memory model catalog + provider-key store +
//! fallback chain configuration into the `Arc<dyn LlmDriver>` used by
//! every agent turn.
//!
//! Sibling submodule of `kernel::mod`, so it retains access to
//! `LibreFangKernel`'s private fields and inherent methods without any
//! visibility surgery.

use super::*;

impl LibreFangKernel {
    /// Resolve the LLM driver for an agent.
    ///
    /// Always creates a fresh driver using current environment variables so that
    /// API keys saved via the dashboard (`set_provider_key`) take effect immediately
    /// without requiring a daemon restart. Uses the hot-reloaded default model
    /// override when available.
    /// If fallback models are configured, wraps the primary in a `FallbackDriver`.
    /// Look up a provider's base URL, checking runtime catalog first, then boot-time config.
    ///
    /// Custom providers added at runtime via the dashboard (`set_provider_url`) are
    /// stored in the model catalog but NOT in `self.config.provider_urls` (which is
    /// the boot-time snapshot). This helper checks both sources so that custom
    /// providers work immediately without a daemon restart.
    fn lookup_provider_url(&self, provider: &str) -> Option<String> {
        let cfg = self.config.load();
        // 1. Boot-time config (from config.toml [provider_urls])
        if let Some(url) = cfg.provider_urls.get(provider) {
            return Some(url.clone());
        }
        // 2. Model catalog (updated at runtime by set_provider_url / apply_url_overrides)
        let catalog = self.llm.model_catalog.load();
        {
            if let Some(p) = catalog.get_provider(provider) {
                if !p.base_url.is_empty() {
                    return Some(p.base_url.clone());
                }
            }
        }
        // 3. Dedicated CLI path config fields (more discoverable than provider_urls).
        if provider == "qwen-code" {
            if let Some(ref path) = cfg.qwen_code_path {
                if !path.is_empty() {
                    return Some(path.clone());
                }
            }
        }
        None
    }

    pub(crate) fn resolve_driver(
        &self,
        manifest: &AgentManifest,
    ) -> KernelResult<Arc<dyn LlmDriver>> {
        let cfg = self.config.load();

        // Use the effective default model: hot-reloaded override takes priority
        // over the boot-time config. This ensures that when a user saves a new
        // API key via the dashboard and the default provider is switched,
        // resolve_driver sees the updated provider/model/api_key_env.
        let override_guard = self
            .llm
            .default_model_override
            .read()
            .unwrap_or_else(|e: std::sync::PoisonError<_>| e.into_inner());
        let effective_default = override_guard.as_ref().unwrap_or(&cfg.default_model);
        let default_provider = &effective_default.provider;

        // Resolve "default" or empty provider to the effective default provider.
        // Without this, agents configured with provider = "default" would pass
        // the literal string "default" to create_driver(), which fails with
        // "Unknown provider 'default'" (issue #2196).
        let resolved_provider_str =
            if manifest.model.provider.is_empty() || manifest.model.provider == "default" {
                default_provider.clone()
            } else {
                manifest.model.provider.clone()
            };
        let agent_provider = &resolved_provider_str;

        let has_custom_key = manifest.model.api_key_env.is_some();
        let has_custom_url = manifest.model.base_url.is_some();

        // CLI profile rotation: when the agent uses the default provider
        // and CLI profiles are configured, use the boot-time
        // TokenRotationDriver directly. The driver_cache would create a
        // single vanilla driver without config_dir, bypassing rotation.
        if !has_custom_key
            && !has_custom_url
            && (agent_provider.is_empty() || agent_provider == default_provider)
            && matches!(
                effective_default.provider.as_str(),
                "claude_code" | "claude-code"
            )
            && !effective_default.cli_profile_dirs.is_empty()
        {
            return Ok(self.llm.default_driver.clone());
        }

        // Resolve base_url (shared between pooled and single-key paths).
        let base_url = if has_custom_url {
            manifest.model.base_url.clone()
        } else if agent_provider == default_provider {
            effective_default
                .base_url
                .clone()
                .or_else(|| self.lookup_provider_url(agent_provider))
        } else {
            self.lookup_provider_url(agent_provider)
        };

        // Build the base DriverConfig skeleton (without api_key — will be
        // filled in by either the pool or single-key path below).
        let make_driver_config = |api_key: Option<String>| DriverConfig {
            provider: agent_provider.clone(),
            api_key,
            base_url: base_url.clone(),
            vertex_ai: cfg.vertex_ai.clone(),
            azure_openai: cfg.azure_openai.clone(),
            skip_permissions: true,
            message_timeout_secs: cfg.default_model.message_timeout_secs,
            mcp_bridge: Some(build_mcp_bridge_cfg(&cfg)),
            proxy_url: cfg.provider_proxy_urls.get(agent_provider).cloned(),
            request_timeout_secs: cfg
                .provider_request_timeout_secs
                .get(agent_provider)
                .copied(),
            emit_caller_trace_headers: cfg.telemetry.emit_caller_trace_headers,
        };

        // Check for a credential pool for this provider.
        // If the pool exists and has at least one available key, create a
        // PooledDriver that acquires a key from the pool on every call.
        // If the pool is empty / all keys exhausted, fall through to the
        // normal single-key path.
        // When the agent explicitly sets a custom API key env var, skip the
        // pool and use the agent-specified key directly.
        let pool_opt = if has_custom_key {
            None
        } else {
            self.llm
                .credential_pools
                .get(agent_provider)
                .filter(|entry| entry.available_count() > 0)
                .map(|entry| entry.value().clone())
        };

        let primary: Arc<dyn LlmDriver> = if let Some(pool) = pool_opt {
            // Credential pool is non-empty — wrap in PooledDriver.
            let base_config = make_driver_config(None);
            Arc::new(pooled_driver::PooledDriver::new(
                pool,
                Arc::clone(&self.llm.driver_cache),
                base_config,
            ))
        } else {
            // No credential pool (or all keys exhausted) — resolve a single
            // API key the traditional way.
            let api_key = if has_custom_key {
                manifest
                    .model
                    .api_key_env
                    .as_ref()
                    .and_then(|env| std::env::var(env).ok())
            } else if agent_provider == default_provider {
                if !effective_default.api_key_env.is_empty() {
                    std::env::var(&effective_default.api_key_env).ok()
                } else {
                    let env_var = cfg.resolve_api_key_env(agent_provider);
                    std::env::var(&env_var).ok()
                }
            } else {
                let env_var = cfg.resolve_api_key_env(agent_provider);
                std::env::var(&env_var).ok()
            };

            let driver_config = make_driver_config(api_key);

            match self.llm.driver_cache.get_or_create(&driver_config) {
                Ok(d) => d,
                Err(e) => {
                    if agent_provider == default_provider && !has_custom_key && !has_custom_url {
                        debug!(
                            provider = %agent_provider,
                            error = %e,
                            "Fresh driver creation failed, falling back to boot-time default"
                        );
                        Arc::clone(&self.llm.default_driver)
                    } else {
                        return Err(KernelError::BootFailed(format!(
                            "Agent LLM driver init failed: {e}"
                        )));
                    }
                }
            }
        };

        // Build effective fallback list: agent-level fallbacks + global fallback_providers.
        // Resolve "default" provider in fallback entries to the actual default provider.
        let mut effective_fallbacks = manifest.fallback_models.clone();
        // Append global fallback_providers so every agent benefits from the configured chain
        for gfb in &cfg.fallback_providers {
            let already_present = effective_fallbacks
                .iter()
                .any(|fb| fb.provider == gfb.provider && fb.model == gfb.model);
            if !already_present {
                effective_fallbacks.push(librefang_types::agent::FallbackModel {
                    provider: gfb.provider.clone(),
                    model: gfb.model.clone(),
                    api_key_env: if gfb.api_key_env.is_empty() {
                        None
                    } else {
                        Some(gfb.api_key_env.clone())
                    },
                    base_url: gfb.base_url.clone(),
                    extra_params: std::collections::HashMap::new(),
                });
            }
        }

        // If fallback models are configured, wrap in FallbackDriver
        if !effective_fallbacks.is_empty() {
            // Primary driver uses the agent's own model name (already set in request)
            let mut chain: Vec<(
                std::sync::Arc<dyn librefang_runtime::llm_driver::LlmDriver>,
                String,
            )> = vec![(primary.clone(), String::new())];
            for fb in &effective_fallbacks {
                // Resolve "default" to the actual default provider, but if the
                // model name implies a specific provider (e.g. "gemini-2.0-flash"
                // → "gemini"), use that instead of blindly falling back to the
                // default provider which may be a completely different service.
                let fb_provider = if fb.provider.is_empty() || fb.provider == "default" {
                    infer_provider_from_model(&fb.model).unwrap_or_else(|| default_provider.clone())
                } else {
                    fb.provider.clone()
                };
                let fb_api_key = if let Some(env) = &fb.api_key_env {
                    std::env::var(env).ok()
                } else {
                    // Resolve using provider_api_keys / convention for custom providers
                    let env_var = cfg.resolve_api_key_env(&fb_provider);
                    std::env::var(&env_var).ok()
                };
                let config = DriverConfig {
                    provider: fb_provider.clone(),
                    api_key: fb_api_key,
                    base_url: fb
                        .base_url
                        .clone()
                        .or_else(|| self.lookup_provider_url(&fb_provider)),
                    vertex_ai: cfg.vertex_ai.clone(),
                    azure_openai: cfg.azure_openai.clone(),
                    mcp_bridge: Some(build_mcp_bridge_cfg(&cfg)),
                    skip_permissions: true,
                    message_timeout_secs: cfg.default_model.message_timeout_secs,
                    proxy_url: cfg.provider_proxy_urls.get(&fb_provider).cloned(),
                    request_timeout_secs: cfg
                        .provider_request_timeout_secs
                        .get(&fb_provider)
                        .copied(),
                    emit_caller_trace_headers: cfg.telemetry.emit_caller_trace_headers,
                };
                match self.llm.driver_cache.get_or_create(&config) {
                    Ok(d) => chain.push((d, strip_provider_prefix(&fb.model, &fb_provider))),
                    Err(e) => {
                        warn!("Fallback driver '{}' failed to init: {e}", fb_provider);
                    }
                }
            }
            if chain.len() > 1 {
                return Ok(Arc::new(
                    librefang_runtime::drivers::fallback::FallbackDriver::with_models(chain),
                ));
            }
        }

        Ok(primary)
    }
}
