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

    /// Look up the `api_key_env` name for a provider from the model catalog.
    ///
    /// Custom providers (added via the dashboard or `registry/providers/`) store
    /// their `api_key_env` in the catalog but NOT in `KernelConfig.provider_api_keys`.
    /// This helper surfaces that catalog value so the chat path can read the same
    /// env-var name that `POST /api/providers/{name}/test` uses — fixing the
    /// mismatch that caused 401s for custom providers with non-conventional
    /// `api_key_env` names (e.g. `UNSLOTH_API_KEY` vs the convention
    /// `UNSLOTH_STUDIO_API_KEY`). Refs: #5755.
    fn lookup_catalog_api_key_env(&self, provider: &str) -> Option<String> {
        let catalog = self.llm.model_catalog.load();
        catalog.get_provider(provider).and_then(|p| {
            if p.api_key_env.is_empty() {
                None
            } else {
                Some(p.api_key_env.clone())
            }
        })
    }

    /// Resolve the env-var name for a non-default provider's API key.
    ///
    /// Precedence:
    ///   1. Operator-explicit `[provider_api_keys]` or `[auth_profiles]` in
    ///      `config.toml` — always wins, so an operator can pin a custom
    ///      provider's key to a specific env var.
    ///   2. Model catalog `api_key_env` (populated by the dashboard
    ///      "Add provider" flow or `registry/providers/*.toml`) — needed
    ///      for custom providers whose env var deviates from the
    ///      `<PROVIDER>_API_KEY` naming convention.
    ///   3. Convention fallback via `cfg.resolve_api_key_env`.
    ///
    /// Refs: #5755 (catalog lookup introduced), #5807 review (precedence:
    /// operator-explicit must not be shadowed by catalog).
    pub(crate) fn resolve_non_default_api_key_env(
        &self,
        cfg: &KernelConfig,
        provider: &str,
    ) -> String {
        if cfg.provider_api_keys.contains_key(provider) || cfg.auth_profiles.contains_key(provider)
        {
            cfg.resolve_api_key_env(provider)
        } else {
            self.lookup_catalog_api_key_env(provider)
                .unwrap_or_else(|| cfg.resolve_api_key_env(provider))
        }
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
            max_retries: cfg
                .provider_max_retries
                .get(agent_provider)
                .copied()
                .unwrap_or_else(|| DriverConfig::default().max_retries),
        };

        // Check for a credential pool for this provider.
        // When the pool exists and the agent didn't set a custom API key,
        // create a PooledDriver that acquires keys from the pool on every
        // call. If the pool is empty / all keys exhausted at call time, the
        // PooledDriver returns a 503 which triggers fallback to the next
        // provider (handled by FallbackDriver below).
        // When the agent explicitly sets a custom API key env var, skip the
        // pool and use the agent-specified key directly.
        let pool_opt = if has_custom_key {
            None
        } else {
            self.llm
                .credential_pools
                .get(agent_provider)
                .map(|entry| entry.value().clone())
        };

        let primary: Arc<dyn LlmDriver> = if let Some(pool) = pool_opt {
            let base_config = make_driver_config(None);
            Arc::new(pooled_driver::PooledDriver::new(
                pool,
                Arc::clone(&self.llm.driver_cache),
                base_config,
            ))
        } else {
            // No credential pool — resolve a single API key the traditional
            // way.
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
                // See `resolve_non_default_api_key_env` for the precedence
                // contract (operator-explicit > catalog > convention).
                // Refs: #5755, #5807.
                let env_var = self.resolve_non_default_api_key_env(&cfg, agent_provider);
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
                        return Err(LibreFangError::BootFailed(format!(
                            "Agent LLM driver init failed: {e}"
                        ))
                        .into());
                    }
                }
            }
        };

        // Build effective fallback list.
        // Three-state logic: None → inherit global, Some([]) → opt-out,
        // Some([…]) → use agent chain exclusively (#5112).
        let effective_fallbacks =
            resolve_effective_fallbacks(&manifest.fallback_models, &cfg.fallback_providers);

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
                    // Same precedence as the primary path (operator-explicit >
                    // catalog `api_key_env` > convention). A custom catalog
                    // provider used only as a fallback model would otherwise
                    // 401 on the convention-only env var — the same #5755 bug
                    // as the primary branch above. Refs: #5755, #5807.
                    let env_var = self.resolve_non_default_api_key_env(&cfg, &fb_provider);
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
                    max_retries: cfg
                        .provider_max_retries
                        .get(&fb_provider)
                        .copied()
                        .unwrap_or_else(|| DriverConfig::default().max_retries),
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

/// Pure helper: resolve the effective fallback list for an agent turn.
///
/// Three-state logic (#5112):
/// - `manifest.fallback_models == None`      → inherit from `global_fallbacks`
/// - `manifest.fallback_models == Some([])`  → opt-out; returns empty vec
/// - `manifest.fallback_models == Some([…])` → use agent's explicit chain only
pub(crate) fn resolve_effective_fallbacks(
    agent_fallbacks: &Option<Vec<librefang_types::agent::FallbackModel>>,
    global_fallbacks: &[librefang_types::config::FallbackProviderConfig],
) -> Vec<librefang_types::agent::FallbackModel> {
    match agent_fallbacks {
        Some(list) => list.clone(),
        None => global_fallbacks
            .iter()
            .map(|gfb| librefang_types::agent::FallbackModel {
                provider: gfb.provider.clone(),
                model: gfb.model.clone(),
                api_key_env: if gfb.api_key_env.is_empty() {
                    None
                } else {
                    Some(gfb.api_key_env.clone())
                },
                base_url: gfb.base_url.clone(),
                extra_params: std::collections::BTreeMap::new(),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::{
        agent::FallbackModel,
        config::{FallbackProviderConfig, KernelConfig, MemoryConfig},
    };

    fn make_global(provider: &str, model: &str) -> FallbackProviderConfig {
        FallbackProviderConfig {
            provider: provider.to_string(),
            model: model.to_string(),
            api_key_env: String::new(),
            base_url: None,
        }
    }

    fn make_agent_fb(provider: &str, model: &str) -> FallbackModel {
        FallbackModel {
            provider: provider.to_string(),
            model: model.to_string(),
            api_key_env: None,
            base_url: None,
            extra_params: std::collections::BTreeMap::new(),
        }
    }

    // Branch 1: None + non-empty global → inherit global chain.
    #[test]
    fn fallback_resolution_none_inherits_global() {
        let global = vec![
            make_global("groq", "llama-3.3-70b"),
            make_global("ollama", "llama3.2:latest"),
        ];
        let result = resolve_effective_fallbacks(&None, &global);
        assert_eq!(result.len(), 2, "None must inherit both global entries");
        assert_eq!(result[0].provider, "groq");
        assert_eq!(result[0].model, "llama-3.3-70b");
        assert_eq!(result[1].provider, "ollama");
        assert_eq!(result[1].model, "llama3.2:latest");
    }

    // Branch 2: Some([]) + non-empty global → opt-out; empty vec (no FallbackDriver).
    #[test]
    fn fallback_resolution_some_empty_opts_out() {
        let global = vec![make_global("groq", "llama-3.3-70b")];
        let result = resolve_effective_fallbacks(&Some(vec![]), &global);
        assert!(
            result.is_empty(),
            "Some([]) must produce empty effective fallbacks regardless of global chain"
        );
    }

    // Branch 3: Some([X]) + non-empty global → agent chain only; global not appended.
    #[test]
    fn fallback_resolution_some_explicit_uses_agent_chain_only() {
        let global = vec![make_global("groq", "llama-3.3-70b")];
        let agent_fb = vec![make_agent_fb("openai", "gpt-4o-mini")];
        let result = resolve_effective_fallbacks(&Some(agent_fb), &global);
        assert_eq!(
            result.len(),
            1,
            "global must not be appended to agent chain"
        );
        assert_eq!(
            result[0].provider, "openai",
            "provider must match agent chain"
        );
        assert_eq!(
            result[0].model, "gpt-4o-mini",
            "model must match agent chain"
        );
    }

    /// Regression test for #5755: a custom provider whose `api_key_env` doesn't
    /// follow the naming convention (`UNSLOTH_API_KEY` instead of
    /// `UNSLOTH_STUDIO_API_KEY`) must be resolvable via the model catalog on the
    /// chat path, matching what `POST /api/providers/{name}/test` does.
    ///
    /// The test writes a provider TOML file into the home-dir's `providers/`
    /// directory before booting the kernel so the catalog is populated the same
    /// way the dashboard "Upload provider" or `registry/providers/` flow does it.
    /// Then it calls `lookup_catalog_api_key_env` (the new helper in `llm_drivers.rs`)
    /// and asserts it returns the TOML-specified env-var name, not the
    /// convention-derived one.
    #[test]
    fn lookup_catalog_api_key_env_returns_catalog_value_for_custom_provider() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let home = tmp.path().to_path_buf();
        let data_dir = home.join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::create_dir_all(home.join("skills")).unwrap();
        std::fs::create_dir_all(home.join("workspaces").join("agents")).unwrap();
        std::fs::create_dir_all(home.join("workspaces").join("hands")).unwrap();

        // Pre-touch the sync marker so `registry_sync::sync_registry` treats
        // the registry cache as fresh and skips the download + fan-out step.
        // Without this, `sync_flat_files` removes any TOML in `providers/` that
        // does not exist in the (empty) registry cache, nuking our fixture before
        // the catalog is loaded. See the same pattern in kernel/tests.rs.
        let registry_dir = home.join("registry");
        std::fs::create_dir_all(&registry_dir).unwrap();
        std::fs::write(registry_dir.join(".sync_marker"), "").unwrap();

        // Write a custom provider TOML with a non-conventional api_key_env name.
        // This mirrors `registry/providers/unsloth-studio.toml` from the issue.
        let providers_dir = home.join("providers");
        std::fs::create_dir_all(&providers_dir).unwrap();
        std::fs::write(
            providers_dir.join("unsloth-studio.toml"),
            r#"
[provider]
id = "unsloth-studio"
display_name = "Unsloth Studio"
api_key_env = "UNSLOTH_API_KEY"
base_url = "http://127.0.0.1:8888/v1"
key_required = true
"#,
        )
        .unwrap();

        let config = KernelConfig {
            home_dir: home.clone(),
            data_dir: data_dir.clone(),
            network_enabled: false,
            memory: MemoryConfig {
                sqlite_path: Some(data_dir.join("test.db")),
                ..Default::default()
            },
            ..KernelConfig::default()
        };

        let kernel = LibreFangKernel::boot_with_config(config).expect("kernel boot");

        // The catalog lookup must return the TOML-specified env-var name, not
        // the convention form (`UNSLOTH_STUDIO_API_KEY`).
        let resolved = kernel.lookup_catalog_api_key_env("unsloth-studio");
        assert_eq!(
            resolved.as_deref(),
            Some("UNSLOTH_API_KEY"),
            "chat path must read api_key_env from catalog, not derive it by convention"
        );

        // A provider not in the catalog must return None so the caller falls
        // back to `cfg.resolve_api_key_env` (convention / provider_api_keys).
        let absent = kernel.lookup_catalog_api_key_env("unknown-provider-xyz");
        assert!(
            absent.is_none(),
            "unknown providers must return None so convention fallback is used"
        );
    }

    /// Precedence regression test: an operator-explicit `[provider_api_keys]`
    /// mapping must beat the model catalog's `api_key_env`. Refs: #5807 review.
    #[test]
    fn resolve_non_default_api_key_env_operator_explicit_beats_catalog() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let home = tmp.path().to_path_buf();
        let data_dir = home.join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::create_dir_all(home.join("skills")).unwrap();
        std::fs::create_dir_all(home.join("workspaces").join("agents")).unwrap();
        std::fs::create_dir_all(home.join("workspaces").join("hands")).unwrap();

        let registry_dir = home.join("registry");
        std::fs::create_dir_all(&registry_dir).unwrap();
        std::fs::write(registry_dir.join(".sync_marker"), "").unwrap();

        // Catalog declares UNSLOTH_API_KEY for the custom provider.
        let providers_dir = home.join("providers");
        std::fs::create_dir_all(&providers_dir).unwrap();
        std::fs::write(
            providers_dir.join("unsloth-studio.toml"),
            r#"
[provider]
id = "unsloth-studio"
display_name = "Unsloth Studio"
api_key_env = "UNSLOTH_API_KEY"
base_url = "http://127.0.0.1:8888/v1"
key_required = true
"#,
        )
        .unwrap();

        // Operator pins the same provider to a DIFFERENT env var.
        let mut provider_api_keys = std::collections::BTreeMap::new();
        provider_api_keys.insert(
            "unsloth-studio".to_string(),
            "PINNED_OPERATOR_KEY".to_string(),
        );

        let config = KernelConfig {
            home_dir: home.clone(),
            data_dir: data_dir.clone(),
            network_enabled: false,
            memory: MemoryConfig {
                sqlite_path: Some(data_dir.join("test.db")),
                ..Default::default()
            },
            provider_api_keys,
            ..KernelConfig::default()
        };

        let kernel = LibreFangKernel::boot_with_config(config.clone()).expect("kernel boot");

        let resolved = kernel.resolve_non_default_api_key_env(&config, "unsloth-studio");
        assert_eq!(
            resolved, "PINNED_OPERATOR_KEY",
            "operator-explicit [provider_api_keys] must beat catalog api_key_env"
        );

        // Catalog still wins when there is no operator-explicit mapping.
        let resolved_no_explicit =
            kernel.resolve_non_default_api_key_env(&config, "other-custom-provider");
        // No catalog entry for "other-custom-provider" either, so we expect the
        // convention fallback ("OTHER_CUSTOM_PROVIDER_API_KEY").
        assert_eq!(
            resolved_no_explicit, "OTHER_CUSTOM_PROVIDER_API_KEY",
            "no explicit + no catalog must fall back to convention"
        );
    }
}
