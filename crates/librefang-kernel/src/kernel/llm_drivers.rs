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

        // Governance: org-wide provider allowlist (issue #6459). Fail-closed —
        // when a non-empty `[providers] allowed` list does not contain the
        // resolved provider, refuse to build ANY driver for this agent turn.
        // This runs before every construction branch below (CLI-profile
        // rotation, credential pool, single-key, boot-default fallback) so a
        // disallowed provider can never reach a live driver. Read live from
        // `self.config.load()`, so an operator's allowlist edit takes effect on
        // the next turn after a config swap.
        if !cfg.providers.is_provider_allowed(agent_provider) {
            warn!(
                provider = %agent_provider,
                allowed = ?cfg.providers.allowed,
                "LLM provider blocked by org-wide allowlist ([providers] allowed)"
            );
            let reason = cfg.providers.rejection_reason(agent_provider);
            return Err(LibreFangError::CapabilityDenied(reason).into());
        }

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
            // Primary driver uses the agent's own model name (already set in
            // request). Each slot carries its provider name so the
            // store-aware `FallbackDriver` can pre-skip a budget-exhausted
            // slot (#5980): the gate flags the provider in the shared
            // `ProviderExhaustionStore`, and this driver reads that SAME
            // store via `is_slot_exhausted`. Mirrors boot.rs:698-714.
            let mut chain: Vec<(
                std::sync::Arc<dyn librefang_runtime::llm_driver::LlmDriver>,
                String,
                String,
            )> = vec![(
                primary.clone(),
                // Empty model name: the primary slot keeps the request's own
                // model field as-is. A non-empty middle element is a per-slot
                // model OVERRIDE (FallbackDriver rewrites `req.model` with it),
                // so the provider name here would clobber the primary model and
                // force it to 404. Mirrors the boot.rs primary slot.
                String::new(),
                agent_provider.clone(),
            )];
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
                // Governance allowlist (issue #6459): never add a disallowed
                // provider to the fallback chain. Fail-closed skip + WARN,
                // mirroring the init-failure skip below.
                if !cfg.providers.is_provider_allowed(&fb_provider) {
                    warn!(
                        provider = %fb_provider,
                        allowed = ?cfg.providers.allowed,
                        "Fallback LLM provider blocked by org-wide allowlist; skipping slot"
                    );
                    continue;
                }
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
                    Ok(d) => chain.push((
                        d,
                        strip_provider_prefix(&fb.model, &fb_provider),
                        fb_provider.clone(),
                    )),
                    Err(e) => {
                        warn!("Fallback driver '{}' failed to init: {e}", fb_provider);
                    }
                }
            }
            if chain.len() > 1 {
                // Attach the SAME exhaustion store the budget gate flags
                // (`MeteringEngine::exhaustion_store()`), not a fresh one —
                // otherwise the flag would be invisible to this driver
                // (#5980). When the store is unwired, fall back to the
                // provider-less builder (no regression).
                let fb =
                    librefang_runtime::drivers::fallback::FallbackDriver::with_models_and_providers(
                        chain,
                    );
                let fb = match self.metering.engine.exhaustion_store() {
                    Some(store) => fb.with_exhaustion_store(store),
                    None => fb,
                };
                return Ok(Arc::new(fb));
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

    /// Regression: the FallbackDriver primary slot built by `resolve_driver`
    /// must carry an EMPTY model name, not the provider name. The middle tuple
    /// element is a per-slot model OVERRIDE — `FallbackDriver` rewrites
    /// `req.model` with it whenever it is non-empty. Setting it to the provider
    /// string ("anthropic"/"openai"/…) clobbers the agent's primary model, so
    /// the primary request 404s and silently fails over. This mirrors the exact
    /// primary-slot construction in `resolve_driver` above.
    #[tokio::test]
    async fn fallback_primary_slot_preserves_request_model() {
        use librefang_types::message::{ContentBlock, StopReason, TokenUsage};
        use std::sync::Mutex;

        // Records the model each dispatch actually saw.
        struct RecordingDriver(Arc<Mutex<Option<String>>>);
        #[async_trait]
        impl LlmDriver for RecordingDriver {
            async fn complete(
                &self,
                req: CompletionRequest,
            ) -> Result<CompletionResponse, LlmError> {
                *self.0.lock().unwrap() = Some(req.model.clone());
                Ok(CompletionResponse {
                    content: vec![ContentBlock::Text {
                        text: "ok".to_string(),
                        provider_metadata: None,
                    }],
                    stop_reason: StopReason::EndTurn,
                    tool_calls: vec![],
                    usage: TokenUsage::default(),
                    actual_provider: None,
                    actual_model: None,
                })
            }
        }

        let seen = Arc::new(Mutex::new(None));
        let primary = Arc::new(RecordingDriver(Arc::clone(&seen)));
        let agent_provider = "anthropic".to_string();

        // Exact shape of the primary slot in `resolve_driver`.
        let chain: Vec<(Arc<dyn LlmDriver>, String, String)> = vec![(
            primary as Arc<dyn LlmDriver>,
            String::new(),
            agent_provider.clone(),
        )];
        let fb =
            librefang_runtime::drivers::fallback::FallbackDriver::with_models_and_providers(chain);

        let req = CompletionRequest {
            model: "claude-sonnet-4-5".to_string(),
            ..Default::default()
        };
        fb.complete(req).await.expect("primary serves");

        assert_eq!(
            seen.lock().unwrap().as_deref(),
            Some("claude-sonnet-4-5"),
            "primary slot must leave req.model unchanged, not overwrite it with the provider name"
        );
    }

    /// #6459 — `resolve_driver` enforces the org-wide provider allowlist
    /// fail-closed at driver resolution time: an empty allowlist allows any
    /// provider, a non-empty allowlist rejects a disallowed provider with a
    /// governance error before any driver is constructed, and permits a
    /// listed one.
    #[test]
    fn resolve_driver_enforces_provider_allowlist() {
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

        // Swap only the allowlist on the live ArcSwap config, mirroring what a
        // `POST /api/config/reload` does — the field is read live per turn.
        let set_allowlist = |allowed: &[&str]| {
            let mut cfg = (*kernel.config.load_full()).clone();
            cfg.providers.allowed = allowed.iter().map(|s| s.to_string()).collect();
            kernel.config.store(std::sync::Arc::new(cfg));
        };

        // Empty allowlist → no restriction: a default agent resolves.
        set_allowlist(&[]);
        assert!(
            kernel.resolve_driver(&AgentManifest::default()).is_ok(),
            "empty allowlist must allow everything"
        );

        // Non-empty allowlist that excludes the requested provider →
        // fail-closed rejection before any driver is constructed.
        set_allowlist(&["ollama"]);
        let mut disallowed = AgentManifest::default();
        disallowed.model.provider = "anthropic".to_string();
        // `Box<dyn LlmDriver>` is not `Debug`, so `expect_err` (which formats the
        // Ok value) will not compile — match the error out explicitly instead.
        let err = match kernel.resolve_driver(&disallowed) {
            Ok(_) => panic!("disallowed provider must be rejected"),
            Err(e) => e,
        };
        assert!(
            err.to_string().contains("allowlist"),
            "rejection must be a governance error, got: {err}"
        );

        // The same allowlist permits the listed provider. `ollama` is a local
        // provider (no API key required), so the driver builds
        // deterministically regardless of the test environment's env vars.
        let mut allowed = AgentManifest::default();
        allowed.model.provider = "ollama".to_string();
        assert!(
            kernel.resolve_driver(&allowed).is_ok(),
            "listed provider must resolve"
        );
    }
}
