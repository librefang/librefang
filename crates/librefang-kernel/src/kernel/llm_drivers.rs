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
use librefang_types::error::LibreFangError;

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

        // Always create a fresh driver by reading current env vars.
        // This ensures API keys saved at runtime (via dashboard POST
        // /api/providers/{name}/key which calls std::env::set_var) are
        // picked up immediately — the boot-time default_driver cache is
        // only used as a final fallback when driver creation fails.
        let primary = {
            let api_key = if has_custom_key {
                // Agent explicitly set an API key env var — use it
                manifest
                    .model
                    .api_key_env
                    .as_ref()
                    .and_then(|env| std::env::var(env).ok())
            } else if agent_provider == default_provider {
                // Same provider as effective default — use its env var
                if !effective_default.api_key_env.is_empty() {
                    std::env::var(&effective_default.api_key_env).ok()
                } else {
                    let env_var = cfg.resolve_api_key_env(agent_provider);
                    std::env::var(&env_var).ok()
                }
            } else {
                // Different provider — check auth profiles, provider_api_keys,
                // and convention-based env var. For custom providers (not in the
                // hardcoded list), this is the primary path for API key resolution.
                let env_var = cfg.resolve_api_key_env(agent_provider);
                std::env::var(&env_var).ok()
            };

            // Don't inherit default provider's base_url when switching providers.
            // Uses lookup_provider_url() which checks both boot-time config AND the
            // runtime model catalog, so custom providers added via the dashboard
            // (which only update the catalog, not self.config) are found (#494).
            let base_url = if has_custom_url {
                manifest.model.base_url.clone()
            } else if agent_provider == default_provider {
                effective_default
                    .base_url
                    .clone()
                    .or_else(|| self.lookup_provider_url(agent_provider))
            } else {
                // Check provider_urls + catalog before falling back to hardcoded defaults
                self.lookup_provider_url(agent_provider)
            };

            let driver_config = DriverConfig {
                provider: agent_provider.clone(),
                api_key,
                base_url,
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

            match self.llm.driver_cache.get_or_create(&driver_config) {
                Ok(d) => d,
                Err(e) => {
                    // If fresh driver creation fails (e.g. key not yet set for this
                    // provider), fall back to the boot-time default driver. This
                    // keeps existing agents working while the user is still
                    // configuring providers via the dashboard.
                    if agent_provider == default_provider && !has_custom_key && !has_custom_url {
                        debug!(
                            provider = %agent_provider,
                            error = %e,
                            "Fresh driver creation failed, falling back to boot-time default"
                        );
                        Arc::clone(&self.llm.default_driver)
                    } else {
                        return Err(LibreFangError::BootFailed(format!(
                            "Agent LLM driver init failed: {e}"
                        ))
                        .into());
                    }
                }
            }
        };

        // Build effective fallback list.
        // When the agent sets `fallback_models` explicitly (Some), use exactly
        // that list — the agent is opting out of inheriting global fallbacks.
        // When it is None, build from the global `cfg.fallback_providers` so
        // every agent benefits from the operator-configured chain (#5112).
        let effective_fallbacks: Vec<librefang_types::agent::FallbackModel> =
            match &manifest.fallback_models {
                Some(agent_fallbacks) => agent_fallbacks.clone(),
                None => {
                    let mut list = Vec::new();
                    for gfb in &cfg.fallback_providers {
                        list.push(librefang_types::agent::FallbackModel {
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
                    list
                }
            };

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
