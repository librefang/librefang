//! Model catalog — registry of known models with metadata, pricing, and auth detection.
//!
//! Provides a comprehensive catalog of 130+ builtin models across 28 providers,
//! with alias resolution, auth status detection, and pricing lookups.

use librefang_types::model_catalog::{
    AliasesCatalogFile, AuthStatus, ModelCatalogEntry, ModelCatalogFile, ModelOverrides, ModelTier,
    ProviderInfo,
};
use std::collections::{HashMap, HashSet};
use tracing::warn;

/// The model catalog — registry of all known models and providers.
pub struct ModelCatalog {
    models: Vec<ModelCatalogEntry>,
    aliases: HashMap<String, String>,
    providers: Vec<ProviderInfo>,
    /// Providers whose fallback/CLI detection is suppressed by the user
    /// (i.e. the user explicitly removed the key via the dashboard).
    suppressed_providers: HashSet<String>,
    /// Per-model inference parameter overrides, keyed by "provider:model_id".
    overrides: HashMap<String, ModelOverrides>,
}

/// Infer (supports_vision, supports_tools, supports_thinking) from a model's
/// name and the `families` array returned by Ollama's `/api/tags`.
///
/// Rules:
/// - `families` contains "clip"  → vision model (LLaVA, BakLLaVA, Moondream, …)
/// - name contains "embed"       → embedding model; tools/thinking N/A
/// - name contains known thinking-model patterns → supports_thinking
fn infer_capabilities(name: &str, families: Option<&[String]>) -> (bool, bool, bool) {
    let lower = name.to_lowercase();

    // Embedding check runs first and short-circuits the rest.
    // A vision-encoder used for embeddings (e.g. a hypothetical "clip-embed")
    // is still not a chat vision model, so the families check is intentionally
    // skipped for embedding models.
    let is_embed = lower.contains("embed") || lower.contains("embedding");
    if is_embed {
        return (false, false, false);
    }

    let supports_vision = families
        .map(|fs| fs.iter().any(|f| f.to_lowercase() == "clip"))
        .unwrap_or(false);

    // Name-based heuristics for thinking/reasoning models.
    // Note: `qwen3` matches all Qwen3 variants, including non-thinking ones —
    // Ollama does not distinguish thinking vs standard mode in `families`.
    let supports_thinking = lower.contains("qwq")
        || lower.contains("deepseek-r1")
        || lower.contains("/r1")
        || lower.contains(":r1")
        || lower.contains("qwen3")
        || lower.contains("marco-o1");

    (supports_vision, true, supports_thinking)
}

impl ModelCatalog {
    /// Create a new catalog by loading providers from `home_dir/providers/`
    /// and aliases from `home_dir/aliases.toml`.
    ///
    /// Providers whose TOML filename also exists in
    /// `home_dir/registry/providers/` are marked as built-in; the rest are
    /// flagged `is_custom = true` so the dashboard can show a real delete
    /// control for them.
    pub fn new(home_dir: &std::path::Path) -> Self {
        let providers_dir = home_dir.join("providers");
        let registry_providers_dir = home_dir.join("registry").join("providers");
        Self::new_from_dir_with_registry(&providers_dir, Some(&registry_providers_dir))
    }

    /// Create a catalog by loading all `*.toml` files from a specific directory.
    ///
    /// Also loads `aliases.toml` from the parent of `providers_dir` if present.
    /// All loaded providers are marked `is_custom = false` (safe default —
    /// callers that want custom detection should use
    /// [`Self::new_from_dir_with_registry`] or [`Self::new`]).
    pub fn new_from_dir(providers_dir: &std::path::Path) -> Self {
        Self::new_from_dir_with_registry(providers_dir, None)
    }

    /// Same as [`Self::new_from_dir`] but with a registry-providers directory
    /// used to classify each loaded provider as built-in vs user-added.
    pub fn new_from_dir_with_registry(
        providers_dir: &std::path::Path,
        registry_providers_dir: Option<&std::path::Path>,
    ) -> Self {
        // Built-in filename set for custom classification.
        //
        // Tri-state semantics:
        //   - `None` registry dir passed, or `read_dir` on it failed
        //     (missing / corrupt / unreadable cache) → classification
        //     unavailable, fall back to is_custom=false for every provider.
        //     This keeps the delete button hidden, which is the safe
        //     default — a user can always remove an API key via the edit
        //     dialog's "Remove Key" control.
        //   - `Some(set)` successful read, even if `set` is empty → trust
        //     the classification. An empty registry dir genuinely means
        //     every provider is user-added.
        let builtin_filenames: Option<std::collections::HashSet<std::ffi::OsString>> =
            registry_providers_dir.and_then(|dir| {
                std::fs::read_dir(dir).ok().map(|entries| {
                    entries
                        .flatten()
                        .filter_map(|e| {
                            let path = e.path();
                            if path.extension().is_some_and(|ext| ext == "toml") {
                                path.file_name().map(|n| n.to_os_string())
                            } else {
                                None
                            }
                        })
                        .collect()
                })
            });

        let mut sources: Vec<(String, bool)> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(providers_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "toml") {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        let is_custom = match (&builtin_filenames, path.file_name()) {
                            (Some(set), Some(name)) => !set.contains(name),
                            _ => false,
                        };
                        sources.push((content, is_custom));
                    }
                }
            }
        }
        let aliases_source = providers_dir
            .parent()
            .and_then(|p| std::fs::read_to_string(p.join("aliases.toml")).ok());
        Self::from_sources(&sources, aliases_source.as_deref())
    }

    /// Build a catalog from pre-loaded TOML source strings.
    ///
    /// Each source is tagged with an `is_custom` flag that is copied onto
    /// the corresponding [`ProviderInfo`].
    fn from_sources(sources: &[(String, bool)], aliases_source: Option<&str>) -> Self {
        let mut models: Vec<ModelCatalogEntry> = Vec::new();
        let mut providers: Vec<ProviderInfo> = Vec::new();
        for (source, is_custom) in sources {
            if let Ok(file) = toml::from_str::<ModelCatalogFile>(source) {
                let provider_id = file.provider.as_ref().map(|p| p.id.clone());
                if let Some(p) = file.provider {
                    let mut info: ProviderInfo = p.into();
                    info.is_custom = *is_custom;
                    providers.push(info);
                }
                for mut model in file.models {
                    // Back-fill provider from the [provider] section when
                    // the model entry omits it (common in registry TOML files).
                    if model.provider.is_empty() {
                        if let Some(ref pid) = provider_id {
                            model.provider = pid.clone();
                        }
                    }
                    models.push(model);
                }
            }
        }

        let mut aliases: HashMap<String, String> = aliases_source
            .and_then(|s| toml::from_str::<AliasesCatalogFile>(s).ok())
            .map(|f| {
                f.aliases
                    .into_iter()
                    .map(|(k, v)| (k.to_lowercase(), v))
                    .collect()
            })
            .unwrap_or_default();

        // Auto-register aliases defined on model entries
        for model in &models {
            for alias in &model.aliases {
                let lower = alias.to_lowercase();
                aliases.entry(lower).or_insert_with(|| model.id.clone());
            }
        }

        // Set model counts on providers
        for provider in &mut providers {
            provider.model_count = models.iter().filter(|m| m.provider == provider.id).count();
        }

        Self {
            models,
            aliases,
            providers,
            suppressed_providers: HashSet::new(),
            overrides: HashMap::new(),
        }
    }

    /// Detect which providers have API keys configured.
    ///
    /// Checks `std::env::var()` for each provider's API key env var.
    /// Only checks presence — never reads or stores the actual secret.
    pub fn detect_auth(&mut self) {
        for provider in &mut self.providers {
            // CLI-based providers: no API key needed, but we probe for CLI
            // installation so the dashboard shows "Configured" vs "Not Installed".
            if crate::drivers::is_cli_provider(&provider.id) {
                provider.auth_status = if crate::drivers::cli_provider_available(&provider.id) {
                    AuthStatus::Configured
                } else {
                    AuthStatus::CliNotInstalled
                };
                continue;
            }

            if !provider.key_required {
                // Local providers (ollama, vllm, etc.) have their status set by
                // the async probe at startup. Only set NotRequired as a fallback
                // when the probe hasn't run yet (status still Missing).
                // LocalOffline means the probe ran and found the service down —
                // do NOT reset it here, or offline providers would re-appear in
                // the model switcher after any unrelated detect_auth() call.
                if crate::provider_health::is_local_provider(&provider.id) {
                    if provider.auth_status == AuthStatus::Missing {
                        provider.auth_status = AuthStatus::NotRequired;
                    }
                    // LocalOffline: leave unchanged — owned by the probe
                } else if !provider.base_url.is_empty() {
                    // Has a base_url, no key needed (e.g. custom local proxy).
                    provider.auth_status = AuthStatus::NotRequired;
                }
                // Otherwise (no key required, no base_url, not local/CLI):
                // leave as Missing — these providers are only usable through
                // hosting platforms like OpenRouter and cannot be called directly.
                continue;
            }

            // Primary: check the provider's declared env var (non-empty after trim).
            //
            // GITHUB_TOKEN is a generic PAT shared by multiple services (Copilot,
            // GitHub Models, git operations, CI/CD). Its mere presence does NOT
            // prove the user has access to a specific provider, so we do not
            // auto-detect it as "Configured".  Users who actually want these
            // providers will authenticate via the dashboard OAuth flow, which
            // validates access before marking the provider as configured.
            let has_key = if provider.api_key_env == "GITHUB_TOKEN" {
                false
            } else {
                std::env::var(&provider.api_key_env).is_ok_and(|v| !v.trim().is_empty())
            };

            // If the user explicitly removed this provider's key, skip
            // fallback/CLI detection — only honour the primary env var.
            let suppressed = self.suppressed_providers.contains(&provider.id);

            // Secondary: provider-specific fallback keys (still API-key-based auth)
            let has_key_fallback = if suppressed {
                false
            } else {
                match provider.id.as_str() {
                    "gemini" => std::env::var("GOOGLE_API_KEY").is_ok_and(|v| !v.trim().is_empty()),
                    "openai" | "codex" => {
                        std::env::var("OPENAI_API_KEY").is_ok_and(|v| !v.trim().is_empty())
                            || read_codex_credential().is_some()
                    }
                    _ => false,
                }
            };

            // Tertiary: CLI tools that can serve as fallback for API providers
            let has_cli_fallback = if suppressed {
                false
            } else {
                let aider_ok = || crate::drivers::cli_provider_available("aider");
                match provider.id.as_str() {
                    "anthropic" => {
                        crate::drivers::cli_provider_available("claude-code") || aider_ok()
                    }
                    "gemini" => crate::drivers::cli_provider_available("gemini-cli") || aider_ok(),
                    "openai" | "codex" => {
                        crate::drivers::cli_provider_available("codex-cli") || aider_ok()
                    }
                    "qwen" => crate::drivers::cli_provider_available("qwen-code") || aider_ok(),
                    _ => false,
                }
            };

            provider.auth_status = if has_key {
                AuthStatus::Configured
            } else if has_key_fallback {
                AuthStatus::AutoDetected
            } else if has_cli_fallback {
                AuthStatus::ConfiguredCli
            } else {
                AuthStatus::Missing
            };
            tracing::debug!(
                provider = %provider.id,
                has_key,
                has_key_fallback,
                has_cli_fallback,
                auth_status = %provider.auth_status,
                "detect_auth result"
            );
        }
    }

    /// Collect providers that need background key validation.
    ///
    /// Returns `(provider_id, base_url, api_key_env)` for every provider
    /// whose current auth status is `Configured` (key present, not yet validated).
    pub fn providers_needing_validation(&self) -> Vec<(String, String, String)> {
        self.providers
            .iter()
            .filter(|p| {
                p.auth_status == AuthStatus::Configured || p.auth_status == AuthStatus::AutoDetected
            })
            .map(|p| (p.id.clone(), p.base_url.clone(), p.api_key_env.clone()))
            .collect()
    }

    /// Update the `auth_status` of a single provider after background validation.
    pub fn set_provider_auth_status(&mut self, provider_id: &str, status: AuthStatus) {
        if let Some(p) = self.providers.iter_mut().find(|p| p.id == provider_id) {
            p.auth_status = status;
        }
    }

    /// Store the list of models confirmed available via live probe.
    pub fn set_provider_available_models(&mut self, provider_id: &str, models: Vec<String>) {
        if let Some(p) = self.providers.iter_mut().find(|p| p.id == provider_id) {
            p.available_models = models;
        }
    }

    /// Check whether a model is confirmed available on its provider.
    /// Returns `None` if the provider hasn't been probed yet (no data),
    /// `Some(true)` if the model is in the probed list, `Some(false)` if not.
    pub fn is_model_available(&self, provider_id: &str, model: &str) -> Option<bool> {
        let p = self.providers.iter().find(|p| p.id == provider_id)?;
        if p.available_models.is_empty() {
            return None; // not probed yet
        }
        Some(p.available_models.iter().any(|m| m == model))
    }

    /// List all models in the catalog.
    pub fn list_models(&self) -> &[ModelCatalogEntry] {
        &self.models
    }

    /// Find a model by its canonical ID, display name, or alias.
    pub fn find_model(&self, id_or_alias: &str) -> Option<&ModelCatalogEntry> {
        let lower = id_or_alias.to_lowercase();
        // Direct ID match — prefer Custom tier entries over builtins so that
        // user-defined custom models (from custom_models.json) take precedence
        // when the same model ID exists under a different provider (#983).
        {
            let mut found: Option<&ModelCatalogEntry> = None;
            for m in &self.models {
                if m.id.to_lowercase() == lower {
                    if m.tier == ModelTier::Custom {
                        // Custom model always wins — return immediately
                        return Some(m);
                    }
                    if found.is_none() {
                        found = Some(m);
                    }
                }
            }
            if let Some(entry) = found {
                return Some(entry);
            }
        }
        // Display-name match for dashboard/UI payloads that send labels.
        if let Some(entry) = self
            .models
            .iter()
            .find(|m| m.display_name.to_lowercase() == lower)
        {
            return Some(entry);
        }
        // Alias resolution
        if let Some(canonical) = self.aliases.get(&lower) {
            return self.models.iter().find(|m| m.id == *canonical);
        }
        None
    }

    /// Resolve an alias to a canonical model ID, or None if not an alias.
    pub fn resolve_alias(&self, alias: &str) -> Option<&str> {
        self.aliases.get(&alias.to_lowercase()).map(|s| s.as_str())
    }

    /// List all providers.
    pub fn list_providers(&self) -> &[ProviderInfo] {
        &self.providers
    }

    /// Get a provider by ID.
    pub fn get_provider(&self, provider_id: &str) -> Option<&ProviderInfo> {
        self.providers.iter().find(|p| p.id == provider_id)
    }

    /// List models from a specific provider.
    pub fn models_by_provider(&self, provider: &str) -> Vec<&ModelCatalogEntry> {
        self.models
            .iter()
            .filter(|m| m.provider == provider)
            .collect()
    }

    /// Return the default model ID for a provider (first model in catalog order).
    pub fn default_model_for_provider(&self, provider: &str) -> Option<String> {
        // Check aliases first — e.g. "minimax" alias resolves to "MiniMax-M2.7"
        if let Some(model_id) = self.aliases.get(provider) {
            return Some(model_id.clone());
        }
        // Fall back to the first model registered for this provider
        self.models
            .iter()
            .find(|m| m.provider == provider)
            .map(|m| m.id.clone())
    }

    /// List models that are available (from configured providers only).
    pub fn available_models(&self) -> Vec<&ModelCatalogEntry> {
        let configured: Vec<&str> = self
            .providers
            .iter()
            .filter(|p| p.auth_status.is_available())
            .map(|p| p.id.as_str())
            .collect();
        self.models
            .iter()
            .filter(|m| configured.contains(&m.provider.as_str()))
            .collect()
    }

    /// Get pricing for a model: (input_cost_per_million, output_cost_per_million).
    pub fn pricing(&self, model_id: &str) -> Option<(f64, f64)> {
        self.find_model(model_id)
            .map(|m| (m.input_cost_per_m, m.output_cost_per_m))
    }

    /// List all alias mappings.
    pub fn list_aliases(&self) -> &HashMap<String, String> {
        &self.aliases
    }

    /// Add a custom alias mapping from `alias` to `model_id`.
    ///
    /// The alias is stored in lowercase. Returns `false` if the alias already
    /// exists (use `remove_alias` first to overwrite).
    pub fn add_alias(&mut self, alias: &str, model_id: &str) -> bool {
        let lower = alias.to_lowercase();
        if self.aliases.contains_key(&lower) {
            return false;
        }
        self.aliases.insert(lower, model_id.to_string());
        true
    }

    /// Remove a custom alias by name.
    ///
    /// Returns `true` if the alias was found and removed.
    pub fn remove_alias(&mut self, alias: &str) -> bool {
        self.aliases.remove(&alias.to_lowercase()).is_some()
    }

    /// Mark a provider as suppressed — fallback/CLI detection will be skipped
    /// for this provider until `unsuppress_provider` is called.
    pub fn suppress_provider(&mut self, id: &str) {
        self.suppressed_providers.insert(id.to_string());
    }

    /// Remove a provider from the suppressed set, re-enabling fallback/CLI detection.
    pub fn unsuppress_provider(&mut self, id: &str) {
        self.suppressed_providers.remove(id);
    }

    /// Load the suppressed-providers list from a JSON file.
    pub fn load_suppressed(&mut self, path: &std::path::Path) {
        if let Ok(data) = std::fs::read_to_string(path) {
            if let Ok(list) = serde_json::from_str::<Vec<String>>(&data) {
                self.suppressed_providers = list.into_iter().collect();
            }
        }
    }

    /// Persist the suppressed-providers list to a JSON file.
    /// Removes the file when the set is empty.
    pub fn save_suppressed(&self, path: &std::path::Path) {
        if self.suppressed_providers.is_empty() {
            let _ = std::fs::remove_file(path);
            return;
        }
        let mut list: Vec<&String> = self.suppressed_providers.iter().collect();
        list.sort();
        if let Ok(json) = serde_json::to_string_pretty(&list) {
            let _ = std::fs::write(path, json);
        }
    }

    // ── Per-model overrides ──────────────────────────────────────────

    /// Get inference parameter overrides for a model.
    /// Key format: "provider:model_id".
    pub fn get_overrides(&self, key: &str) -> Option<&ModelOverrides> {
        self.overrides.get(key)
    }

    /// Set inference parameter overrides for a model.
    /// Removes the entry if `overrides.is_empty()`.
    pub fn set_overrides(&mut self, key: String, overrides: ModelOverrides) {
        if overrides.is_empty() {
            self.overrides.remove(&key);
        } else {
            self.overrides.insert(key, overrides);
        }
    }

    /// Remove inference parameter overrides for a model.
    pub fn remove_overrides(&mut self, key: &str) -> bool {
        self.overrides.remove(key).is_some()
    }

    /// Load model overrides from a JSON file.
    pub fn load_overrides(&mut self, path: &std::path::Path) {
        if let Ok(data) = std::fs::read_to_string(path) {
            if let Ok(map) = serde_json::from_str::<HashMap<String, ModelOverrides>>(&data) {
                self.overrides = map;
            }
        }
    }

    /// Persist model overrides to a JSON file.
    /// Removes the file when no overrides are set.
    pub fn save_overrides(&self, path: &std::path::Path) -> Result<(), String> {
        if self.overrides.is_empty() {
            let _ = std::fs::remove_file(path);
            return Ok(());
        }
        let json = serde_json::to_string_pretty(&self.overrides)
            .map_err(|e| format!("Failed to serialize model overrides: {e}"))?;
        std::fs::write(path, json)
            .map_err(|e| format!("Failed to write model overrides file: {e}"))?;
        Ok(())
    }

    /// Set a custom base URL for a provider, overriding the default.
    ///
    /// Returns `true` if the provider was found and updated.
    pub fn set_provider_url(&mut self, provider: &str, url: &str) -> bool {
        if let Some(p) = self.providers.iter_mut().find(|p| p.id == provider) {
            p.base_url = url.to_string();
            true
        } else {
            // Custom provider — add a new entry so it appears in /api/providers
            let env_var = format!("{}_API_KEY", provider.to_uppercase().replace('-', "_"));
            self.providers.push(ProviderInfo {
                id: provider.to_string(),
                display_name: provider.to_string(),
                api_key_env: env_var,
                base_url: url.to_string(),
                key_required: true,
                auth_status: AuthStatus::Missing,
                model_count: 0,
                signup_url: None,
                regions: std::collections::HashMap::new(),
                media_capabilities: Vec::new(),
                available_models: Vec::new(),
                // Added at runtime via set_provider_url → always custom.
                is_custom: true,
                proxy_url: None,
            });
            // Re-detect auth for the newly added provider
            self.detect_auth();
            true
        }
    }

    /// Apply a batch of provider URL overrides from config.
    ///
    /// Each entry maps a provider ID to a custom base URL.
    /// Unknown providers are automatically added as custom OpenAI-compatible entries.
    /// Providers with explicit URL overrides are marked as configured since
    /// the user intentionally set them up (e.g. local proxies, custom endpoints).
    pub fn apply_url_overrides(&mut self, overrides: &HashMap<String, String>) {
        for (provider, url) in overrides {
            if self.set_provider_url(provider, url) {
                // Mark as configured so models from this provider show as available
                if let Some(p) = self.providers.iter_mut().find(|p| p.id == *provider) {
                    if p.auth_status == AuthStatus::Missing {
                        p.auth_status = AuthStatus::Configured;
                    }
                }
            }
        }
    }

    /// Set a per-provider proxy URL override.
    pub fn set_provider_proxy_url(&mut self, provider: &str, proxy_url: &str) {
        if let Some(p) = self.providers.iter_mut().find(|p| p.id == provider) {
            p.proxy_url = if proxy_url.is_empty() {
                None
            } else {
                Some(proxy_url.to_string())
            };
        }
    }

    /// Apply a batch of per-provider proxy URL overrides from config.
    pub fn apply_proxy_url_overrides(&mut self, overrides: &HashMap<String, String>) {
        for (provider, proxy_url) in overrides {
            self.set_provider_proxy_url(provider, proxy_url);
        }
    }

    /// Resolve provider region selections into URL overrides.
    ///
    /// For each entry in `region_selections` (provider ID → region name), looks up
    /// the region URL from the provider's `regions` map. Returns a map of provider
    /// IDs to resolved URLs that can be applied via [`apply_url_overrides`].
    ///
    /// Entries where the provider or region is not found are skipped with a warning.
    pub fn resolve_region_urls(
        &self,
        region_selections: &HashMap<String, String>,
    ) -> HashMap<String, String> {
        let mut resolved = HashMap::new();
        for (provider_id, region_name) in region_selections {
            if let Some(provider) = self.get_provider(provider_id) {
                if let Some(region_cfg) = provider.regions.get(region_name) {
                    resolved.insert(provider_id.clone(), region_cfg.base_url.clone());
                } else {
                    warn!(
                        "provider_regions: unknown region '{}' for provider '{}' \
                         (available: {:?})",
                        region_name,
                        provider_id,
                        provider.regions.keys().collect::<Vec<_>>()
                    );
                }
            } else {
                warn!(
                    "provider_regions: unknown provider '{}' — not found in catalog",
                    provider_id
                );
            }
        }
        resolved
    }

    /// Resolve provider region selections into API key env var overrides.
    ///
    /// For each entry in `region_selections` (provider ID → region name), looks up
    /// the region's `api_key_env` from the provider's `regions` map. Only returns
    /// entries where the region defines a custom `api_key_env`.
    ///
    /// The returned map can be merged into `config.provider_api_keys` so that
    /// [`KernelConfig::resolve_api_key_env`] picks up region-specific env vars.
    pub fn resolve_region_api_keys(
        &self,
        region_selections: &HashMap<String, String>,
    ) -> HashMap<String, String> {
        let mut resolved = HashMap::new();
        for (provider_id, region_name) in region_selections {
            if let Some(provider) = self.get_provider(provider_id) {
                if let Some(region_cfg) = provider.regions.get(region_name) {
                    if let Some(api_key_env) = &region_cfg.api_key_env {
                        resolved.insert(provider_id.clone(), api_key_env.clone());
                    }
                } else {
                    warn!(
                        "provider_regions: unknown region '{}' for provider '{}' \
                         (available: {:?})",
                        region_name,
                        provider_id,
                        provider.regions.keys().collect::<Vec<_>>()
                    );
                }
            } else {
                warn!(
                    "provider_regions: unknown provider '{}' — not found in catalog",
                    provider_id
                );
            }
        }
        resolved
    }

    /// List models filtered by tier.
    pub fn models_by_tier(&self, tier: ModelTier) -> Vec<&ModelCatalogEntry> {
        self.models.iter().filter(|m| m.tier == tier).collect()
    }

    /// Merge dynamically discovered models from a local provider.
    ///
    /// Accepts enriched metadata from Ollama's `/api/tags` response to infer
    /// capabilities (vision via the "clip" family, embeddings, thinking models).
    /// Falls back to conservative defaults when metadata is absent.
    /// Also updates the provider's `model_count`.
    pub fn merge_discovered_models(
        &mut self,
        provider: &str,
        model_info: &[crate::provider_health::DiscoveredModelInfo],
    ) {
        let existing_ids: std::collections::HashSet<String> = self
            .models
            .iter()
            .filter(|m| m.provider == provider)
            .map(|m| m.id.to_lowercase())
            .collect();

        let mut added = 0usize;
        for info in model_info {
            if existing_ids.contains(&info.name.to_lowercase()) {
                continue;
            }
            let (supports_vision, supports_tools, supports_thinking) =
                infer_capabilities(&info.name, info.families.as_deref());
            let display = format!("{} ({})", info.name, provider);
            self.models.push(ModelCatalogEntry {
                id: info.name.clone(),
                display_name: display,
                provider: provider.to_string(),
                tier: ModelTier::Local,
                context_window: 131_072,
                max_output_tokens: 16_384,
                input_cost_per_m: 0.0,
                output_cost_per_m: 0.0,
                supports_tools,
                supports_vision,
                supports_streaming: true,
                supports_thinking,
                aliases: Vec::new(),
            });
            added += 1;
        }

        // Update model count on the provider
        if added > 0 {
            if let Some(p) = self.providers.iter_mut().find(|p| p.id == provider) {
                p.model_count = self
                    .models
                    .iter()
                    .filter(|m| m.provider == provider)
                    .count();
            }
        }
    }

    /// Add a custom model at runtime.
    ///
    /// Returns `true` if the model was added, `false` if a model with the same
    /// ID **and** provider already exists (case-insensitive).
    pub fn add_custom_model(&mut self, entry: ModelCatalogEntry) -> bool {
        let lower_id = entry.id.to_lowercase();
        let lower_provider = entry.provider.to_lowercase();
        if self
            .models
            .iter()
            .any(|m| m.id.to_lowercase() == lower_id && m.provider.to_lowercase() == lower_provider)
        {
            return false;
        }
        for alias in &entry.aliases {
            let lower = alias.to_lowercase();
            self.aliases
                .entry(lower)
                .or_insert_with(|| entry.id.clone());
        }
        let provider = entry.provider.clone();
        self.models.push(entry);

        // Update provider model count
        if let Some(p) = self.providers.iter_mut().find(|p| p.id == provider) {
            p.model_count = self
                .models
                .iter()
                .filter(|m| m.provider == provider)
                .count();
        }
        true
    }

    /// Remove a custom model by ID.
    ///
    /// Only removes models with `Custom` tier to prevent accidental deletion
    /// of builtin models. Returns `true` if removed.
    pub fn remove_custom_model(&mut self, model_id: &str) -> bool {
        let lower = model_id.to_lowercase();
        let before = self.models.len();
        self.models
            .retain(|m| !(m.id.to_lowercase() == lower && m.tier == ModelTier::Custom));
        self.models.len() < before
    }

    /// Load custom models from a JSON file.
    ///
    /// Merges them into the catalog. Skips models that already exist.
    pub fn load_custom_models(&mut self, path: &std::path::Path) {
        if !path.exists() {
            return;
        }
        let Ok(data) = std::fs::read_to_string(path) else {
            return;
        };
        let Ok(entries) = serde_json::from_str::<Vec<ModelCatalogEntry>>(&data) else {
            return;
        };
        for entry in entries {
            self.add_custom_model(entry);
        }
    }

    /// Save all custom-tier models to a JSON file.
    pub fn save_custom_models(&self, path: &std::path::Path) -> Result<(), String> {
        let custom: Vec<&ModelCatalogEntry> = self
            .models
            .iter()
            .filter(|m| m.tier == ModelTier::Custom)
            .collect();
        let json = serde_json::to_string_pretty(&custom)
            .map_err(|e| format!("Failed to serialize custom models: {e}"))?;
        std::fs::write(path, json)
            .map_err(|e| format!("Failed to write custom models file: {e}"))?;
        Ok(())
    }

    /// Load a single TOML catalog file and merge its contents into the catalog.
    ///
    /// The file may contain an optional `[provider]` section and a `[[models]]`
    /// array. This is the unified format shared between the main repository
    /// (`catalog/providers/*.toml`) and the community model-catalog repository
    /// (`providers/*.toml`).
    ///
    /// Models that already exist (by ID + provider) are skipped.
    /// If a `[provider]` section is present and the provider is not yet
    /// registered, it is added.
    pub fn load_catalog_file(&mut self, path: &std::path::Path) -> Result<usize, String> {
        let data = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read catalog file {}: {e}", path.display()))?;
        let file: ModelCatalogFile = toml::from_str(&data)
            .map_err(|e| format!("Failed to parse catalog file {}: {e}", path.display()))?;
        Ok(self.merge_catalog_file(file))
    }

    /// Merge a parsed [`ModelCatalogFile`] into the catalog.
    ///
    /// Returns the number of new models added.
    pub fn merge_catalog_file(&mut self, file: ModelCatalogFile) -> usize {
        // Merge provider info if present
        let file_provider_id = file.provider.as_ref().map(|p| p.id.clone());
        if let Some(prov_toml) = file.provider {
            let provider_id = prov_toml.id.clone();
            if self.providers.iter().any(|p| p.id == provider_id) {
                // Update existing provider's base_url and display_name if they differ
                if let Some(existing) = self.providers.iter_mut().find(|p| p.id == provider_id) {
                    existing.base_url = prov_toml.base_url;
                    existing.display_name = prov_toml.display_name;
                    // Keep the previous env var when catalog payload omits/empties it.
                    if !prov_toml.api_key_env.trim().is_empty() {
                        existing.api_key_env = prov_toml.api_key_env;
                    }
                    existing.key_required = prov_toml.key_required;
                }
            } else {
                self.providers.push(prov_toml.into());
            }
        }

        // Merge models
        let mut added = 0usize;
        for mut model in file.models {
            // Back-fill provider from the [provider] section when the model
            // entry omits it (common in community catalog files).
            if model.provider.is_empty() {
                if let Some(ref pid) = file_provider_id {
                    model.provider = pid.clone();
                } else {
                    // No provider info at all — skip this model
                    continue;
                }
            }
            let lower_id = model.id.to_lowercase();
            let lower_provider = model.provider.to_lowercase();
            if self.models.iter().any(|m| {
                m.id.to_lowercase() == lower_id && m.provider.to_lowercase() == lower_provider
            }) {
                continue;
            }
            // Register aliases from the model
            for alias in &model.aliases {
                let lower = alias.to_lowercase();
                self.aliases
                    .entry(lower)
                    .or_insert_with(|| model.id.clone());
            }
            let provider_id = model.provider.clone();
            self.models.push(model);
            added += 1;

            // Update provider model count
            if let Some(p) = self.providers.iter_mut().find(|p| p.id == provider_id) {
                p.model_count = self
                    .models
                    .iter()
                    .filter(|m| m.provider == provider_id)
                    .count();
            }
        }
        added
    }

    /// Load all `*.toml` catalog files from a directory.
    ///
    /// This handles both the builtin `catalog/providers/` directory and the
    /// cached community catalog at `~/.librefang/cache/catalog/providers/`.
    /// Also loads an `aliases.toml` file if present in the directory or its
    /// parent.
    ///
    /// Returns the total number of new models added.
    pub fn load_cached_catalog(&mut self, dir: &std::path::Path) -> Result<usize, String> {
        if !dir.is_dir() {
            return Ok(0);
        }

        let mut total_added = 0usize;

        // Load all *.toml files in the directory
        let entries = std::fs::read_dir(dir)
            .map_err(|e| format!("Failed to read directory {}: {e}", dir.display()))?;

        for entry in entries {
            let entry = entry.map_err(|e| format!("Failed to read dir entry: {e}"))?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                match self.load_catalog_file(&path) {
                    Ok(n) => total_added += n,
                    Err(e) => {
                        tracing::warn!("Skipping catalog file {}: {e}", path.display());
                    }
                }
            }
        }

        // Try loading aliases.toml from the directory or its parent
        for aliases_path in &[
            dir.join("aliases.toml"),
            dir.parent()
                .map(|p| p.join("aliases.toml"))
                .unwrap_or_default(),
        ] {
            if aliases_path.is_file() {
                if let Ok(data) = std::fs::read_to_string(aliases_path) {
                    if let Ok(aliases_file) = toml::from_str::<AliasesCatalogFile>(&data) {
                        for (alias, canonical) in aliases_file.aliases {
                            self.aliases
                                .entry(alias.to_lowercase())
                                .or_insert(canonical);
                        }
                    }
                }
                break;
            }
        }

        Ok(total_added)
    }

    /// Load cached catalog from the shared registry checkout at
    /// `home_dir/registry/providers/`.
    ///
    /// Prior to the registry-unify refactor this read from
    /// `home_dir/cache/catalog/providers/`, which was a copy of the same
    /// data. Reading `registry/providers/` directly eliminates the copy
    /// step and guarantees the catalog is never staler than what
    /// `registry_sync` last pulled.
    pub fn load_cached_catalog_for(&mut self, home_dir: &std::path::Path) {
        let providers_dir = home_dir.join("registry").join("providers");
        if providers_dir.exists() {
            match self.load_cached_catalog(&providers_dir) {
                Ok(n) => {
                    if n > 0 {
                        tracing::info!("Loaded {n} cached community models");
                    }
                }
                Err(e) => tracing::warn!("Failed to load cached catalog: {e}"),
            }
        }
    }

    /// Load user-defined models from `home_dir/model_catalog.toml`.
    ///
    /// User models override builtins and cached models by ID.
    pub fn load_user_catalog_for(&mut self, home_dir: &std::path::Path) {
        let user_catalog = home_dir.join("model_catalog.toml");
        if user_catalog.exists() {
            match self.load_catalog_file(&user_catalog) {
                Ok(n) => {
                    if n > 0 {
                        tracing::info!(
                            "Loaded {n} user-defined models from {}",
                            user_catalog.display()
                        );
                    }
                }
                Err(e) => tracing::warn!("Failed to load user model catalog: {e}"),
            }
        }
    }
}

impl Default for ModelCatalog {
    fn default() -> Self {
        let home = resolve_home_dir();
        Self::new(&home)
    }
}

/// Resolve the librefang home directory from `LIBREFANG_HOME` or `~/.librefang`.
///
/// Used only as a fallback for `Default` impl and standalone usage.
/// Kernel code should always pass `config.home_dir` explicitly.
fn resolve_home_dir() -> std::path::PathBuf {
    std::env::var("LIBREFANG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::home_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join(".librefang")
        })
}

/// Read an OpenAI API key from the Codex CLI credential file.
///
/// Checks `$CODEX_HOME/auth.json` or `~/.codex/auth.json`.
/// Returns `Some(api_key)` if the file exists and contains a valid, non-expired token.
/// Only checks presence — the actual key value is used transiently, never stored.
pub fn read_codex_credential() -> Option<String> {
    let codex_home = std::env::var("CODEX_HOME")
        .map(std::path::PathBuf::from)
        .ok()
        .or_else(|| {
            #[cfg(target_os = "windows")]
            {
                std::env::var("USERPROFILE")
                    .ok()
                    .map(|h| std::path::PathBuf::from(h).join(".codex"))
            }
            #[cfg(not(target_os = "windows"))]
            {
                std::env::var("HOME")
                    .ok()
                    .map(|h| std::path::PathBuf::from(h).join(".codex"))
            }
        })?;

    let auth_path = codex_home.join("auth.json");
    let content = std::fs::read_to_string(&auth_path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;

    // Check expiry if present
    if let Some(expires_at) = parsed.get("expires_at").and_then(|v| v.as_i64()) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        if now >= expires_at {
            return None; // Expired
        }
    }

    parsed
        .get("api_key")
        .or_else(|| parsed.get("token"))
        // Codex CLI OAuth stores the token nested at tokens.id_token
        .or_else(|| parsed.get("tokens").and_then(|t| t.get("id_token")))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider_health::DiscoveredModelInfo;

    fn test_catalog() -> ModelCatalog {
        let home = crate::registry_sync::resolve_home_dir_for_tests();
        ModelCatalog::new(&home)
    }

    /// Convert plain name strings to minimal `DiscoveredModelInfo` for tests
    /// that don't need to exercise capability inference.
    fn names_to_info(names: &[&str]) -> Vec<DiscoveredModelInfo> {
        names
            .iter()
            .map(|n| DiscoveredModelInfo {
                name: n.to_string(),
                parameter_size: None,
                quantization_level: None,
                family: None,
                families: None,
                size: None,
            })
            .collect()
    }

    #[test]
    fn test_catalog_has_models() {
        let catalog = test_catalog();
        assert!(catalog.list_models().len() >= 30);
    }

    /// Mirrors the pre-refactor `catalog_sync::test_alias_catalog_parse` —
    /// keeps direct coverage of `AliasesCatalogFile` deserialization, which
    /// is now only consumed here in `model_catalog`.
    #[test]
    fn test_aliases_catalog_parse() {
        let toml_str = r#"
[aliases]
sonnet = "claude-sonnet-4-20250514"
gpt4 = "gpt-4o"
"#;
        let file: librefang_types::model_catalog::AliasesCatalogFile =
            toml::from_str(toml_str).unwrap();
        assert_eq!(file.aliases.len(), 2);
        assert_eq!(file.aliases["sonnet"], "claude-sonnet-4-20250514");
        assert_eq!(file.aliases["gpt4"], "gpt-4o");
    }

    /// P2 regression: when registry classification is unavailable
    /// (registry dir unreadable or missing), every provider must fall back
    /// to is_custom=false so the dashboard does not re-enable the misleading
    /// delete button on built-ins.
    #[test]
    fn test_is_custom_safe_fallback_on_missing_registry() {
        let tmp = tempfile::tempdir().unwrap();
        let providers_dir = tmp.path().join("providers");
        std::fs::create_dir_all(&providers_dir).unwrap();
        std::fs::write(
            providers_dir.join("acme.toml"),
            r#"[provider]
id = "acme"
display_name = "Acme"
api_key_env = "ACME_API_KEY"
base_url = "https://acme.test"
"#,
        )
        .unwrap();

        // Case 1: registry dir argument is None → classification skipped.
        let catalog = ModelCatalog::new_from_dir_with_registry(&providers_dir, None);
        assert!(
            !catalog.list_providers().iter().any(|p| p.is_custom),
            "is_custom must be false when no registry dir is supplied"
        );

        // Case 2: registry dir points to a nonexistent path → read_dir
        // fails, classification must degrade to false (not true).
        let missing_registry = tmp.path().join("nonexistent-registry");
        let catalog =
            ModelCatalog::new_from_dir_with_registry(&providers_dir, Some(&missing_registry));
        assert!(
            !catalog.list_providers().iter().any(|p| p.is_custom),
            "is_custom must be false when registry read_dir fails"
        );

        // Case 3: registry dir exists and does NOT contain acme.toml →
        // acme is correctly flagged custom.
        let registry_dir = tmp.path().join("registry");
        std::fs::create_dir_all(&registry_dir).unwrap();
        let catalog = ModelCatalog::new_from_dir_with_registry(&providers_dir, Some(&registry_dir));
        assert!(
            catalog
                .list_providers()
                .iter()
                .any(|p| p.id == "acme" && p.is_custom),
            "acme must be flagged custom when registry dir exists but does not list it"
        );

        // Case 4: registry dir lists acme.toml → acme is a built-in.
        std::fs::write(
            registry_dir.join("acme.toml"),
            r#"[provider]
id = "acme"
"#,
        )
        .unwrap();
        let catalog = ModelCatalog::new_from_dir_with_registry(&providers_dir, Some(&registry_dir));
        assert!(
            catalog
                .list_providers()
                .iter()
                .any(|p| p.id == "acme" && !p.is_custom),
            "acme must NOT be flagged custom when registry dir lists it"
        );
    }

    #[test]
    fn test_catalog_has_providers() {
        let catalog = test_catalog();
        assert!(catalog.list_providers().len() >= 40);
    }

    #[test]
    fn test_find_model_by_id() {
        let catalog = test_catalog();
        let entry = catalog.find_model("claude-sonnet-4-20250514").unwrap();
        assert_eq!(entry.display_name, "Claude Sonnet 4");
        assert_eq!(entry.provider, "anthropic");
        assert_eq!(entry.tier, ModelTier::Smart);
    }

    #[test]
    fn test_find_model_by_alias() {
        let catalog = test_catalog();
        let entry = catalog.find_model("sonnet").unwrap();
        assert_eq!(entry.id, "claude-sonnet-4-6");
    }

    #[test]
    fn test_find_model_case_insensitive() {
        let catalog = test_catalog();
        assert!(catalog.find_model("Claude-Sonnet-4-20250514").is_some());
        assert!(catalog.find_model("SONNET").is_some());
    }

    #[test]
    fn test_find_model_not_found() {
        let catalog = test_catalog();
        assert!(catalog.find_model("nonexistent-model").is_none());
    }

    #[test]
    fn test_resolve_alias() {
        let catalog = test_catalog();
        assert_eq!(catalog.resolve_alias("sonnet"), Some("claude-sonnet-4-6"));
        assert_eq!(
            catalog.resolve_alias("haiku"),
            Some("claude-haiku-4-5-20251001")
        );
        assert!(catalog.resolve_alias("nonexistent").is_none());
    }

    #[test]
    fn test_models_by_provider() {
        let catalog = test_catalog();
        let anthropic = catalog.models_by_provider("anthropic");
        assert!(!anthropic.is_empty());
        assert!(anthropic.iter().all(|m| m.provider == "anthropic"));
    }

    #[test]
    fn test_models_by_tier() {
        let catalog = test_catalog();
        let frontier = catalog.models_by_tier(ModelTier::Frontier);
        assert!(frontier.len() >= 3); // At least opus, gpt-4.1, gemini-2.5-pro
        assert!(frontier.iter().all(|m| m.tier == ModelTier::Frontier));
    }

    #[test]
    fn test_pricing_lookup() {
        let catalog = test_catalog();
        let (input, output) = catalog.pricing("claude-sonnet-4-20250514").unwrap();
        assert!((input - 3.0).abs() < 0.001);
        assert!((output - 15.0).abs() < 0.001);
    }

    #[test]
    fn test_pricing_via_alias() {
        let catalog = test_catalog();
        let (input, output) = catalog.pricing("sonnet").unwrap();
        assert!((input - 3.0).abs() < 0.001);
        assert!((output - 15.0).abs() < 0.001);
    }

    #[test]
    fn test_pricing_not_found() {
        let catalog = test_catalog();
        assert!(catalog.pricing("nonexistent").is_none());
    }

    #[test]
    fn test_detect_auth_local_providers() {
        let mut catalog = test_catalog();
        catalog.detect_auth();
        // Local providers should be NotRequired
        let ollama = catalog.get_provider("ollama").unwrap();
        assert_eq!(ollama.auth_status, AuthStatus::NotRequired);
        let vllm = catalog.get_provider("vllm").unwrap();
        assert_eq!(vllm.auth_status, AuthStatus::NotRequired);
    }

    #[test]
    fn test_available_models_includes_local() {
        let mut catalog = test_catalog();
        catalog.detect_auth();
        let available = catalog.available_models();
        // Local providers (ollama, vllm, lmstudio) should always be available
        assert!(available.iter().any(|m| m.provider == "ollama"));
    }

    #[test]
    fn test_provider_model_counts() {
        let catalog = test_catalog();
        let anthropic = catalog.get_provider("anthropic").unwrap();
        assert!(anthropic.model_count > 0);
        let groq = catalog.get_provider("groq").unwrap();
        assert!(groq.model_count > 0);
    }

    #[test]
    fn test_list_aliases() {
        let catalog = test_catalog();
        let aliases = catalog.list_aliases();
        assert!(aliases.len() >= 20);
        assert_eq!(aliases.get("sonnet").unwrap(), "claude-sonnet-4-6");
        // New aliases
        assert_eq!(aliases.get("grok").unwrap(), "grok-4-0709");
        assert_eq!(aliases.get("jamba").unwrap(), "jamba-1.5-large");
    }

    #[test]
    fn test_find_grok_by_alias() {
        let catalog = test_catalog();
        let entry = catalog.find_model("grok").unwrap();
        assert_eq!(entry.id, "grok-4-0709");
        assert_eq!(entry.provider, "xai");
    }

    #[test]
    fn test_add_alias() {
        let mut catalog = test_catalog();
        assert!(catalog.add_alias("my-sonnet", "claude-sonnet-4-6"));
        assert_eq!(
            catalog.resolve_alias("my-sonnet").unwrap(),
            "claude-sonnet-4-6"
        );
        // Duplicate should return false
        assert!(!catalog.add_alias("my-sonnet", "gpt-4o"));
        // Alias is case-insensitive
        assert!(!catalog.add_alias("MY-SONNET", "gpt-4o"));
    }

    #[test]
    fn test_remove_alias() {
        let mut catalog = test_catalog();
        catalog.add_alias("temp-alias", "gpt-4o");
        assert!(catalog.remove_alias("temp-alias"));
        assert!(catalog.resolve_alias("temp-alias").is_none());
        // Removing non-existent alias returns false
        assert!(!catalog.remove_alias("no-such-alias"));
        // Case-insensitive removal
        catalog.add_alias("upper-alias", "gpt-4o");
        assert!(catalog.remove_alias("UPPER-ALIAS"));
    }

    #[test]
    fn test_new_providers_in_catalog() {
        let catalog = test_catalog();
        assert!(catalog.get_provider("perplexity").is_some());
        assert!(catalog.get_provider("cohere").is_some());
        assert!(catalog.get_provider("ai21").is_some());
        assert!(catalog.get_provider("cerebras").is_some());
        assert!(catalog.get_provider("sambanova").is_some());
        assert!(catalog.get_provider("huggingface").is_some());
        assert!(catalog.get_provider("xai").is_some());
        assert!(catalog.get_provider("replicate").is_some());
    }

    #[test]
    fn test_xai_models() {
        let catalog = test_catalog();
        let xai = catalog.models_by_provider("xai");
        assert!(!xai.is_empty());
        assert!(xai.iter().any(|m| m.id == "grok-4-0709"));
        assert!(xai.iter().any(|m| m.id == "grok-4-fast-reasoning"));
        assert!(xai.iter().any(|m| m.id == "grok-4-fast-non-reasoning"));
        assert!(xai.iter().any(|m| m.id == "grok-4-1-fast-reasoning"));
        assert!(xai.iter().any(|m| m.id == "grok-4-1-fast-non-reasoning"));
        assert!(xai.iter().any(|m| m.id == "grok-3"));
        assert!(xai.iter().any(|m| m.id == "grok-3-mini"));
    }

    #[test]
    fn test_perplexity_models() {
        let catalog = test_catalog();
        let pp = catalog.models_by_provider("perplexity");
        assert!(!pp.is_empty());
    }

    #[test]
    fn test_cohere_models() {
        let catalog = test_catalog();
        let co = catalog.models_by_provider("cohere");
        assert!(!co.is_empty());
    }

    #[test]
    fn test_default_creates_valid_catalog() {
        let catalog = test_catalog();
        assert!(!catalog.list_models().is_empty());
        assert!(!catalog.list_providers().is_empty());
    }

    #[test]
    fn test_merge_adds_new_models() {
        let mut catalog = test_catalog();
        let before = catalog.models_by_provider("ollama").len();
        catalog
            .merge_discovered_models("ollama", &names_to_info(&["codestral:latest", "qwen2:7b"]));
        let after = catalog.models_by_provider("ollama").len();
        assert_eq!(after, before + 2);
        // Verify the new models are Local tier with zero cost
        let qwen = catalog.find_model("qwen2:7b").unwrap();
        assert_eq!(qwen.tier, ModelTier::Local);
        assert!((qwen.input_cost_per_m).abs() < f64::EPSILON);
    }

    #[test]
    fn test_merge_skips_existing() {
        let mut catalog = test_catalog();
        // Pick an existing builtin Ollama model ID dynamically so this test
        // stays green regardless of which models the registry ships.
        let existing_id = catalog
            .models_by_provider("ollama")
            .into_iter()
            .next()
            .expect("ollama must have at least one builtin model")
            .id
            .clone();
        let before = catalog.list_models().len();
        catalog.merge_discovered_models("ollama", &names_to_info(&[existing_id.as_str()]));
        let after = catalog.list_models().len();
        assert_eq!(after, before); // no new model added
    }

    #[test]
    fn test_merge_updates_model_count() {
        let mut catalog = test_catalog();
        let before_count = catalog.get_provider("ollama").unwrap().model_count;
        catalog.merge_discovered_models("ollama", &names_to_info(&["new-model:latest"]));
        let after_count = catalog.get_provider("ollama").unwrap().model_count;
        assert_eq!(after_count, before_count + 1);
    }

    #[test]
    fn test_merge_infers_capabilities_from_ollama_metadata() {
        let mut catalog = test_catalog();

        let models = vec![
            // Vision model: families includes "clip"
            DiscoveredModelInfo {
                name: "llava:latest".to_string(),
                families: Some(vec!["llama".to_string(), "clip".to_string()]),
                family: Some("llama".to_string()),
                parameter_size: None,
                quantization_level: None,
                size: None,
            },
            // Embedding model: name contains "embed"
            DiscoveredModelInfo {
                name: "nomic-embed-text:latest".to_string(),
                families: None,
                family: None,
                parameter_size: None,
                quantization_level: None,
                size: None,
            },
            // Thinking model: name contains "deepseek-r1"
            DiscoveredModelInfo {
                name: "deepseek-r1:8b".to_string(),
                families: None,
                family: None,
                parameter_size: None,
                quantization_level: None,
                size: None,
            },
            // Plain chat model
            DiscoveredModelInfo {
                name: "llama3.2:latest".to_string(),
                families: Some(vec!["llama".to_string()]),
                family: Some("llama".to_string()),
                parameter_size: None,
                quantization_level: None,
                size: None,
            },
        ];
        catalog.merge_discovered_models("ollama", &models);

        let llava = catalog.find_model("llava:latest").unwrap();
        assert!(
            llava.supports_vision,
            "llava should have vision via clip family"
        );
        assert!(llava.supports_tools);

        let embed = catalog.find_model("nomic-embed-text:latest").unwrap();
        assert!(!embed.supports_vision);
        assert!(
            !embed.supports_tools,
            "embedding model should not have tools"
        );
        assert!(!embed.supports_thinking);

        let r1 = catalog.find_model("deepseek-r1:8b").unwrap();
        assert!(r1.supports_thinking, "deepseek-r1 should have thinking");
        assert!(!r1.supports_vision);

        let llama = catalog.find_model("llama3.2:latest").unwrap();
        assert!(!llama.supports_vision);
        assert!(llama.supports_tools);
        assert!(!llama.supports_thinking);
    }

    #[test]
    fn test_custom_model_keeps_assigned_provider() {
        let mut catalog = test_catalog();
        let added = catalog.add_custom_model(ModelCatalogEntry {
            id: "custom-qwen-model".to_string(),
            display_name: "Custom Qwen Model".to_string(),
            provider: "qwen".to_string(),
            tier: ModelTier::Custom,
            context_window: 128_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 0.0,
            output_cost_per_m: 0.0,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            supports_thinking: false,
            aliases: vec!["custom-qwen".to_string()],
        });

        assert!(added);
        let model = catalog.find_model("custom-qwen-model").unwrap();
        assert_eq!(model.provider, "qwen");

        let aliased = catalog.find_model("custom-qwen").unwrap();
        assert_eq!(aliased.provider, "qwen");
    }

    #[test]
    fn test_custom_models_with_same_id_keep_distinct_providers() {
        let mut catalog = test_catalog();

        assert!(catalog.add_custom_model(ModelCatalogEntry {
            id: "shared-custom-id".to_string(),
            display_name: "Shared Custom ID".to_string(),
            provider: "qwen".to_string(),
            tier: ModelTier::Custom,
            context_window: 64_000,
            max_output_tokens: 4_096,
            input_cost_per_m: 0.0,
            output_cost_per_m: 0.0,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            supports_thinking: false,
            aliases: Vec::new(),
        }));

        assert!(catalog.add_custom_model(ModelCatalogEntry {
            id: "shared-custom-id".to_string(),
            display_name: "Shared Custom ID".to_string(),
            provider: "minimax".to_string(),
            tier: ModelTier::Custom,
            context_window: 64_000,
            max_output_tokens: 4_096,
            input_cost_per_m: 0.0,
            output_cost_per_m: 0.0,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            supports_thinking: false,
            aliases: Vec::new(),
        }));

        let qwen_count = catalog
            .models_by_provider("qwen")
            .iter()
            .filter(|m| m.id == "shared-custom-id")
            .count();
        let minimax_count = catalog
            .models_by_provider("minimax")
            .iter()
            .filter(|m| m.id == "shared-custom-id")
            .count();

        assert_eq!(qwen_count, 1);
        assert_eq!(minimax_count, 1);
    }

    #[test]
    fn test_find_model_prefers_custom_over_builtin() {
        // Regression test for #983: when a custom model shares the same ID as a
        // builtin model but specifies a different provider, find_model must
        // return the custom entry so the correct provider is used for routing.
        let mut catalog = test_catalog();

        // Pick a known builtin xai model and verify it exists
        let builtin = catalog.find_model("grok-3").unwrap();
        assert_eq!(builtin.provider, "xai");

        // Add a custom model with the same ID but a different provider
        assert!(catalog.add_custom_model(ModelCatalogEntry {
            id: "grok-3".to_string(),
            display_name: "Grok 3 via OpenRouter".to_string(),
            provider: "openrouter".to_string(),
            tier: ModelTier::Custom,
            context_window: 131_072,
            max_output_tokens: 8_192,
            input_cost_per_m: 0.0,
            output_cost_per_m: 0.0,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            supports_thinking: false,
            aliases: Vec::new(),
        }));

        // find_model should now return the custom entry, not the builtin
        let found = catalog.find_model("grok-3").unwrap();
        assert_eq!(found.provider, "openrouter");
        assert_eq!(found.tier, ModelTier::Custom);
    }

    #[test]
    fn test_chinese_providers_in_catalog() {
        let catalog = test_catalog();
        assert!(catalog.get_provider("qwen").is_some());
        assert!(catalog.get_provider("minimax").is_some());
        assert!(catalog.get_provider("zhipu").is_some());
        assert!(catalog.get_provider("zhipu_coding").is_some());
        assert!(catalog.get_provider("moonshot").is_some());
        assert!(catalog.get_provider("qianfan").is_some());
        assert!(catalog.get_provider("bedrock").is_some());
        assert!(catalog.get_provider("zai").is_some());
        assert!(catalog.get_provider("zai_coding").is_some());
        assert!(catalog.get_provider("kimi_coding").is_some());
        assert!(catalog.get_provider("alibaba-coding-plan").is_some());
    }

    #[test]
    fn test_zai_models() {
        let catalog = test_catalog();
        // Z.AI chat models
        let glm5 = catalog.find_model("zai/glm-5-20250605").unwrap();
        assert_eq!(glm5.provider, "zai");
        assert_eq!(glm5.tier, ModelTier::Frontier);
        let glm47 = catalog.find_model("zai/glm-4.7").unwrap();
        assert_eq!(glm47.provider, "zai");
        assert_eq!(glm47.tier, ModelTier::Smart);
        // Z.AI coding models
        let coding5 = catalog.find_model("glm-5-coding").unwrap();
        assert_eq!(coding5.provider, "zai_coding");
        assert_eq!(coding5.tier, ModelTier::Frontier);
        let coding47 = catalog.find_model("glm-4.7-coding").unwrap();
        assert_eq!(coding47.provider, "zai_coding");
        // Aliases
        assert!(catalog.find_model("zai-glm-5").is_some());
        assert!(catalog.find_model("glm-5-code").is_some());
        assert!(catalog.find_model("glm-coding").is_some());
    }

    #[test]
    fn test_kimi2_models() {
        let catalog = test_catalog();
        // Kimi K2 and K2.5 models
        let k2 = catalog.find_model("kimi-k2").unwrap();
        assert_eq!(k2.provider, "moonshot");
        assert_eq!(k2.tier, ModelTier::Frontier);
        let k25 = catalog.find_model("kimi-k2.5").unwrap();
        assert_eq!(k25.provider, "moonshot");
        assert_eq!(k25.tier, ModelTier::Frontier);
        // Alias resolution
        assert!(catalog.find_model("kimi-k2.5-0711").is_some());
    }

    #[test]
    fn test_chinese_model_aliases() {
        let catalog = test_catalog();
        assert!(catalog.find_model("kimi").is_some());
        assert!(catalog.find_model("glm").is_some());
        assert!(catalog.find_model("codegeex").is_some());
        assert!(catalog.find_model("ernie").is_some());
        assert!(catalog.find_model("minimax").is_some());
        // MiniMax M2.7 — by exact ID, alias, and case-insensitive
        let m27 = catalog.find_model("MiniMax-M2.7").unwrap();
        assert!(
            m27.provider == "minimax" || m27.provider == "minimax-cn",
            "unexpected provider: {}",
            m27.provider
        );
        assert_eq!(m27.tier, ModelTier::Frontier);
        assert!(catalog.find_model("minimax-m2.7").is_some());
        // Default "minimax" alias resolves to a minimax-family model
        let default = catalog.find_model("minimax").unwrap();
        assert!(
            default.provider == "minimax" || default.provider == "minimax-cn",
            "unexpected provider: {}",
            default.provider
        );
        // MiniMax M2.7 Highspeed — by exact ID and aliases
        let hs = catalog.find_model("MiniMax-M2.7-highspeed").unwrap();
        assert!(
            hs.provider == "minimax" || hs.provider == "minimax-cn",
            "unexpected provider: {}",
            hs.provider
        );
        assert!(catalog.find_model("minimax-m2.7-highspeed").is_some());
        // abab7-chat
        let abab7 = catalog.find_model("abab7-chat").unwrap();
        assert!(
            abab7.provider == "minimax" || abab7.provider == "minimax-cn",
            "unexpected provider: {}",
            abab7.provider
        );
        assert!(abab7.supports_vision);
    }

    #[test]
    fn test_bedrock_models() {
        let catalog = test_catalog();
        let bedrock = catalog.models_by_provider("bedrock");
        assert!(!bedrock.is_empty());
    }

    #[test]
    fn test_set_provider_url() {
        let mut catalog = test_catalog();
        let old_url = catalog.get_provider("ollama").unwrap().base_url.clone();
        assert_eq!(old_url, "http://localhost:11434/v1");

        let updated = catalog.set_provider_url("ollama", "http://192.168.1.100:11434/v1");
        assert!(updated);
        assert_eq!(
            catalog.get_provider("ollama").unwrap().base_url,
            "http://192.168.1.100:11434/v1"
        );
    }

    #[test]
    fn test_set_provider_url_unknown() {
        let mut catalog = test_catalog();
        let initial_count = catalog.list_providers().len();
        let updated = catalog.set_provider_url("my-custom-llm", "http://localhost:9999");
        // Unknown providers are now auto-registered as custom entries
        assert!(updated);
        assert_eq!(catalog.list_providers().len(), initial_count + 1);
        assert_eq!(
            catalog.get_provider("my-custom-llm").unwrap().base_url,
            "http://localhost:9999"
        );
    }

    #[test]
    fn test_apply_url_overrides() {
        let mut catalog = test_catalog();
        let mut overrides = HashMap::new();
        overrides.insert("ollama".to_string(), "http://10.0.0.5:11434/v1".to_string());
        overrides.insert("vllm".to_string(), "http://10.0.0.6:8000/v1".to_string());
        overrides.insert("nonexistent".to_string(), "http://nowhere".to_string());

        catalog.apply_url_overrides(&overrides);

        assert_eq!(
            catalog.get_provider("ollama").unwrap().base_url,
            "http://10.0.0.5:11434/v1"
        );
        assert_eq!(
            catalog.get_provider("vllm").unwrap().base_url,
            "http://10.0.0.6:8000/v1"
        );
        // lmstudio should be unchanged
        assert_eq!(
            catalog.get_provider("lmstudio").unwrap().base_url,
            "http://localhost:1234/v1"
        );
    }

    /// Build a synthetic catalog with regions defined inline for deterministic testing.
    fn region_test_catalog() -> ModelCatalog {
        let provider_a = r#"
[provider]
id = "test-provider"
display_name = "Test Provider"
base_url = "https://api.test.com/v1"
api_key_env = "TEST_API_KEY"

[provider.regions.us]
base_url = "https://us.api.test.com/v1"

[provider.regions.cn]
base_url = "https://cn.api.test.com/v1"
api_key_env = "TEST_CN_API_KEY"

[[models]]
id = "test-model"
display_name = "Test Model"
tier = "smart"
context_window = 32768
max_output_tokens = 4096
input_cost_per_m = 1.0
output_cost_per_m = 3.0
supports_tools = true
supports_vision = false
supports_streaming = true
"#;
        let provider_b = r#"
[provider]
id = "test-provider-nokey"
display_name = "Test Provider No Key"
base_url = "https://api.nokey.com/v1"
api_key_env = "NOKEY_API_KEY"

[provider.regions.eu]
base_url = "https://eu.api.nokey.com/v1"

[[models]]
id = "nokey-model"
display_name = "NoKey Model"
tier = "fast"
context_window = 8192
max_output_tokens = 2048
input_cost_per_m = 0.5
output_cost_per_m = 1.5
supports_tools = false
supports_vision = false
supports_streaming = false
"#;
        let sources = vec![
            (provider_a.to_string(), false),
            (provider_b.to_string(), false),
        ];
        ModelCatalog::from_sources(&sources, None)
    }

    #[test]
    fn test_resolve_region_urls() {
        let catalog = region_test_catalog();

        // Known provider + known region -> URL resolved
        let mut sel = HashMap::new();
        sel.insert("test-provider".to_string(), "us".to_string());
        let urls = catalog.resolve_region_urls(&sel);
        assert_eq!(
            urls.get("test-provider").unwrap(),
            "https://us.api.test.com/v1"
        );

        // Known provider + another known region
        sel.clear();
        sel.insert("test-provider".to_string(), "cn".to_string());
        let urls = catalog.resolve_region_urls(&sel);
        assert_eq!(
            urls.get("test-provider").unwrap(),
            "https://cn.api.test.com/v1"
        );

        // Known provider + unknown region -> empty
        sel.clear();
        sel.insert("test-provider".to_string(), "jp".to_string());
        let urls = catalog.resolve_region_urls(&sel);
        assert!(urls.is_empty());
    }

    #[test]
    fn test_resolve_region_api_keys() {
        let catalog = region_test_catalog();

        // Region with api_key_env -> returned
        let mut sel = HashMap::new();
        sel.insert("test-provider".to_string(), "cn".to_string());
        let keys = catalog.resolve_region_api_keys(&sel);
        assert_eq!(
            keys.get("test-provider").map(|s| s.as_str()),
            Some("TEST_CN_API_KEY")
        );

        // Region without api_key_env -> excluded
        sel.clear();
        sel.insert("test-provider".to_string(), "us".to_string());
        let keys = catalog.resolve_region_api_keys(&sel);
        assert!(!keys.contains_key("test-provider"));

        // Provider whose region has no api_key_env -> excluded
        sel.clear();
        sel.insert("test-provider-nokey".to_string(), "eu".to_string());
        let keys = catalog.resolve_region_api_keys(&sel);
        assert!(!keys.contains_key("test-provider-nokey"));
    }

    #[test]
    fn test_resolve_region_unknown_provider() {
        let catalog = region_test_catalog();
        let mut sel = HashMap::new();
        sel.insert("nonexistent".to_string(), "us".to_string());
        let urls = catalog.resolve_region_urls(&sel);
        assert!(urls.is_empty());
        let keys = catalog.resolve_region_api_keys(&sel);
        assert!(keys.is_empty());
    }

    #[test]
    fn test_codex_models_under_openai() {
        // Codex models are now merged under the "openai" provider
        let catalog = test_catalog();
        let models = catalog.models_by_provider("openai");
        assert!(models.iter().any(|m| m.id == "codex/gpt-4.1"));
        assert!(models.iter().any(|m| m.id == "codex/o4-mini"));
    }

    #[test]
    fn test_codex_aliases() {
        let catalog = test_catalog();
        let entry = catalog.find_model("codex").unwrap();
        assert_eq!(entry.id, "codex/gpt-4.1");
    }

    #[test]
    fn test_claude_code_provider() {
        let catalog = test_catalog();
        let cc = catalog.get_provider("claude-code").unwrap();
        assert_eq!(cc.display_name, "Claude Code");
        assert!(!cc.key_required);
    }

    #[test]
    fn test_claude_code_models() {
        let catalog = test_catalog();
        let models = catalog.models_by_provider("claude-code");
        assert_eq!(models.len(), 3);
        assert!(models.iter().any(|m| m.id == "claude-code/opus"));
        assert!(models.iter().any(|m| m.id == "claude-code/sonnet"));
        assert!(models.iter().any(|m| m.id == "claude-code/haiku"));
    }

    #[test]
    fn test_claude_code_aliases() {
        let catalog = test_catalog();
        let entry = catalog.find_model("claude-code").unwrap();
        assert_eq!(entry.id, "claude-code/sonnet");
    }

    #[test]
    fn test_load_catalog_file_with_provider() {
        let toml_content = r#"
[provider]
id = "test-provider"
display_name = "Test Provider"
api_key_env = "TEST_API_KEY"
base_url = "https://api.test.example.com"
key_required = true

[[models]]
id = "test-model-1"
display_name = "Test Model 1"
provider = "test-provider"
tier = "smart"
context_window = 128000
max_output_tokens = 8192
input_cost_per_m = 1.0
output_cost_per_m = 3.0
supports_tools = true
supports_vision = false
supports_streaming = true
aliases = ["tm1"]
"#;
        let file: ModelCatalogFile = toml::from_str(toml_content).unwrap();
        let mut catalog = test_catalog();
        let initial_models = catalog.list_models().len();
        let initial_providers = catalog.list_providers().len();

        let added = catalog.merge_catalog_file(file);
        assert_eq!(added, 1);
        assert_eq!(catalog.list_models().len(), initial_models + 1);
        assert_eq!(catalog.list_providers().len(), initial_providers + 1);

        // Verify the model was added
        let model = catalog.find_model("test-model-1").unwrap();
        assert_eq!(model.provider, "test-provider");
        assert_eq!(model.tier, ModelTier::Smart);

        // Verify the provider was added
        let provider = catalog.get_provider("test-provider").unwrap();
        assert_eq!(provider.display_name, "Test Provider");
        assert_eq!(provider.base_url, "https://api.test.example.com");
        assert_eq!(provider.model_count, 1);

        // Verify alias was registered
        let aliased = catalog.find_model("tm1").unwrap();
        assert_eq!(aliased.id, "test-model-1");
    }

    #[test]
    fn test_merge_catalog_keeps_existing_api_key_env_when_incoming_empty() {
        let mut catalog = test_catalog();
        let original_env = catalog
            .get_provider("deepseek")
            .expect("deepseek provider should exist in test catalog")
            .api_key_env
            .clone();
        assert!(!original_env.is_empty());

        let toml_content = r#"
[provider]
id = "deepseek"
display_name = "DeepSeek"
api_key_env = ""
base_url = "https://api.deepseek.com/v1"
key_required = true
"#;
        let file: ModelCatalogFile = toml::from_str(toml_content).unwrap();
        let added = catalog.merge_catalog_file(file);
        assert_eq!(added, 0);

        let merged = catalog
            .get_provider("deepseek")
            .expect("deepseek provider should still exist");
        assert_eq!(merged.api_key_env, original_env);
    }

    #[test]
    fn test_load_catalog_file_without_provider() {
        let toml_content = r#"
[[models]]
id = "test-standalone-model"
display_name = "Standalone Model"
provider = "anthropic"
tier = "fast"
context_window = 32000
max_output_tokens = 4096
input_cost_per_m = 0.5
output_cost_per_m = 1.0
supports_tools = true
supports_vision = false
supports_streaming = true
aliases = []
"#;
        let file: ModelCatalogFile = toml::from_str(toml_content).unwrap();
        assert!(file.provider.is_none());

        let mut catalog = test_catalog();
        let added = catalog.merge_catalog_file(file);
        assert_eq!(added, 1);

        let model = catalog.find_model("test-standalone-model").unwrap();
        assert_eq!(model.provider, "anthropic");
    }

    #[test]
    fn test_merge_catalog_skips_duplicate_models() {
        let toml_content = r#"
[[models]]
id = "claude-sonnet-4-20250514"
display_name = "Claude Sonnet 4"
provider = "anthropic"
tier = "smart"
context_window = 200000
max_output_tokens = 64000
input_cost_per_m = 3.0
output_cost_per_m = 15.0
supports_tools = true
supports_vision = true
supports_streaming = true
aliases = []
"#;
        let file: ModelCatalogFile = toml::from_str(toml_content).unwrap();
        let mut catalog = test_catalog();
        let initial_models = catalog.list_models().len();

        let added = catalog.merge_catalog_file(file);
        assert_eq!(added, 0); // Already exists
        assert_eq!(catalog.list_models().len(), initial_models);
    }

    #[test]
    fn test_load_cached_catalog_from_dir() {
        let dir = tempfile::tempdir().unwrap();
        let toml_content = r#"
[provider]
id = "cached-provider"
display_name = "Cached Provider"
api_key_env = "CACHED_API_KEY"
base_url = "https://api.cached.example.com"
key_required = true

[[models]]
id = "cached-model-1"
display_name = "Cached Model 1"
provider = "cached-provider"
tier = "balanced"
context_window = 64000
max_output_tokens = 4096
input_cost_per_m = 0.5
output_cost_per_m = 1.5
supports_tools = true
supports_vision = false
supports_streaming = true
aliases = []
"#;
        std::fs::write(dir.path().join("cached.toml"), toml_content).unwrap();

        let mut catalog = test_catalog();
        let added = catalog.load_cached_catalog(dir.path()).unwrap();
        assert_eq!(added, 1);

        let model = catalog.find_model("cached-model-1").unwrap();
        assert_eq!(model.provider, "cached-provider");

        let provider = catalog.get_provider("cached-provider").unwrap();
        assert_eq!(provider.base_url, "https://api.cached.example.com");
    }

    #[test]
    fn test_builtin_toml_files_parse() {
        // Verify all TOML catalog files in catalog/providers/ are valid
        let catalog_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("catalog")
            .join("providers");
        if catalog_dir.is_dir() {
            let mut total_models = 0;
            let mut total_providers = 0;
            for entry in std::fs::read_dir(&catalog_dir).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                    let data = std::fs::read_to_string(&path).unwrap();
                    let file: ModelCatalogFile = toml::from_str(&data).unwrap_or_else(|e| {
                        panic!("Failed to parse {}: {e}", path.display());
                    });
                    if file.provider.is_some() {
                        total_providers += 1;
                    }
                    total_models += file.models.len();
                }
            }
            // We expect at least 25 providers and 100 models
            assert!(
                total_providers >= 25,
                "Expected at least 25 providers, got {total_providers}"
            );
            assert!(
                total_models >= 100,
                "Expected at least 100 models, got {total_models}"
            );
        }
    }

    #[test]
    fn test_parse_remote_catalog_without_provider_on_models() {
        // Remote model-catalog repo omits `provider` on each [[models]] entry
        // because it's already in the [provider] section.
        let toml_content = r#"
[provider]
id = "test-remote"
display_name = "Test Remote"
api_key_env = "TEST_REMOTE_KEY"
base_url = "https://api.test-remote.example.com"
key_required = true

[[models]]
id = "test-remote-model-1"
display_name = "Test Remote Model 1"
tier = "frontier"
context_window = 200000
max_output_tokens = 128000
input_cost_per_m = 5.0
output_cost_per_m = 25.0
supports_tools = true
supports_vision = true
supports_streaming = true
aliases = ["trm1"]
"#;
        let file: ModelCatalogFile =
            toml::from_str(toml_content).expect("should parse without provider on models");
        assert_eq!(file.models.len(), 1);
        assert!(file.models[0].provider.is_empty());

        let mut catalog = test_catalog();
        let added = catalog.merge_catalog_file(file);
        assert_eq!(added, 1);

        let model = catalog.find_model("test-remote-model-1").unwrap();
        assert_eq!(model.provider, "test-remote");
    }

    #[test]
    fn test_media_capabilities_parsed_from_toml() {
        let toml_content = r#"
[provider]
id = "testprov"
display_name = "Test Provider"
api_key_env = "TEST_KEY"
base_url = "https://api.test.com/v1"
key_required = true
media_capabilities = ["image_generation", "text_to_speech"]

[[models]]
id = "test-model"
display_name = "Test Model"
tier = "smart"
context_window = 128000
max_output_tokens = 4096
input_cost_per_m = 1.0
output_cost_per_m = 2.0
supports_tools = true
supports_vision = false
supports_streaming = true
"#;
        let catalog = ModelCatalog::from_sources(&[(toml_content.to_string(), false)], None);
        let providers = catalog.list_providers();
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].id, "testprov");
        assert_eq!(providers[0].media_capabilities.len(), 2);
        assert!(providers[0]
            .media_capabilities
            .contains(&"image_generation".to_string()));
        assert!(providers[0]
            .media_capabilities
            .contains(&"text_to_speech".to_string()));
    }

    #[test]
    fn test_media_capabilities_defaults_to_empty() {
        let toml_content = r#"
[provider]
id = "noprov"
display_name = "No Media"
api_key_env = "NM_KEY"
base_url = "https://api.nomedia.com"
key_required = true

[[models]]
id = "nm-1"
display_name = "NM 1"
tier = "fast"
context_window = 8000
max_output_tokens = 2000
input_cost_per_m = 0.5
output_cost_per_m = 1.0
supports_tools = false
supports_vision = false
supports_streaming = true
"#;
        let catalog = ModelCatalog::from_sources(&[(toml_content.to_string(), false)], None);
        let providers = catalog.list_providers();
        assert_eq!(providers.len(), 1);
        assert!(providers[0].media_capabilities.is_empty());
    }

    #[test]
    fn test_alibaba_coding_plan_provider() {
        let catalog = test_catalog();
        let provider = catalog
            .get_provider("alibaba-coding-plan")
            .expect("alibaba-coding-plan provider should be registered");
        assert_eq!(provider.display_name, "Alibaba Coding Plan (Intl)");
        assert_eq!(provider.api_key_env, "ALIBABA_CODING_PLAN_API_KEY");
        assert_eq!(
            provider.base_url,
            "https://coding-intl.dashscope.aliyuncs.com/v1"
        );
        assert!(provider.key_required);
    }

    #[test]
    fn test_alibaba_coding_plan_has_models() {
        // Smoke check only — the exact model set is owned by the upstream
        // librefang-registry repo and changes over time. Specific model
        // coverage is asserted by name in the sibling tests below.
        let catalog = test_catalog();
        let models = catalog.models_by_provider("alibaba-coding-plan");
        assert!(
            !models.is_empty(),
            "alibaba-coding-plan should expose at least one model"
        );
    }

    #[test]
    fn test_alibaba_coding_plan_zero_cost() {
        let catalog = test_catalog();
        let qwen35plus = catalog
            .find_model("alibaba-coding-plan/qwen3.5-plus")
            .expect("qwen3.5-plus model should be registered");
        assert_eq!(qwen35plus.input_cost_per_m, 0.0);
        assert_eq!(qwen35plus.output_cost_per_m, 0.0);
        let qwen36plus = catalog
            .find_model("alibaba-coding-plan/qwen3.6-plus")
            .expect("qwen3.6-plus model should be registered");
        assert_eq!(qwen36plus.input_cost_per_m, 0.0);
        assert_eq!(qwen36plus.output_cost_per_m, 0.0);
    }

    #[test]
    fn test_alibaba_coding_plan_vision_models() {
        let catalog = test_catalog();
        let qwen35plus = catalog
            .find_model("alibaba-coding-plan/qwen3.5-plus")
            .expect("qwen3.5-plus model should be registered");
        assert!(qwen35plus.supports_vision);
        assert_eq!(qwen35plus.tier, ModelTier::Smart);
        assert_eq!(qwen35plus.context_window, 1_000_000);

        let qwen36plus = catalog
            .find_model("alibaba-coding-plan/qwen3.6-plus")
            .expect("qwen3.6-plus model should be registered");
        assert!(qwen36plus.supports_vision);
        assert_eq!(qwen36plus.tier, ModelTier::Smart);
        assert_eq!(qwen36plus.context_window, 1_000_000);

        let kimi = catalog
            .find_model("alibaba-coding-plan/kimi-k2.5")
            .expect("kimi-k2.5 model should be registered");
        assert!(kimi.supports_vision);
        assert_eq!(kimi.tier, ModelTier::Smart);
    }

    #[test]
    fn test_alibaba_coding_plan_coder_models() {
        let catalog = test_catalog();
        let coder_plus = catalog
            .find_model("alibaba-coding-plan/qwen3-coder-plus")
            .expect("qwen3-coder-plus model should be registered");
        assert_eq!(coder_plus.tier, ModelTier::Smart);
        assert_eq!(coder_plus.context_window, 1_000_000);

        let coder_next = catalog
            .find_model("alibaba-coding-plan/qwen3-coder-next")
            .expect("qwen3-coder-next model should be registered");
        assert_eq!(coder_next.tier, ModelTier::Frontier);
        assert_eq!(coder_next.context_window, 262_144);
    }

    #[test]
    fn test_alibaba_coding_plan_all_models_support_tools() {
        let catalog = test_catalog();
        let models = catalog.models_by_provider("alibaba-coding-plan");
        for model in models {
            assert!(
                model.supports_tools,
                "Model {} should support tools",
                model.id
            );
            assert!(
                model.supports_streaming,
                "Model {} should support streaming",
                model.id
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests for infer_capabilities
// ---------------------------------------------------------------------------

#[cfg(test)]
mod infer_capabilities_tests {
    use super::infer_capabilities;

    fn families(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn embedding_model_has_no_capabilities() {
        assert_eq!(
            infer_capabilities("nomic-embed-text:latest", None),
            (false, false, false)
        );
        assert_eq!(
            infer_capabilities("bge-embedding:latest", None),
            (false, false, false)
        );
        // embedding check wins even when families contains "clip"
        assert_eq!(
            infer_capabilities("clip-embed:latest", Some(&families(&["clip"]))),
            (false, false, false)
        );
    }

    #[test]
    fn vision_model_detected_via_clip_family() {
        assert_eq!(
            infer_capabilities("llava:latest", Some(&families(&["llama", "clip"]))),
            (true, true, false)
        );
        assert_eq!(
            infer_capabilities("moondream:latest", Some(&families(&["clip"]))),
            (true, true, false)
        );
        // clip family match is case-insensitive
        assert_eq!(
            infer_capabilities("llava:7b", Some(&families(&["CLIP"]))),
            (true, true, false)
        );
    }

    #[test]
    fn plain_chat_model_gets_tools_only() {
        assert_eq!(
            infer_capabilities("llama3.2:latest", Some(&families(&["llama"]))),
            (false, true, false)
        );
        assert_eq!(infer_capabilities("mistral:7b", None), (false, true, false));
    }

    #[test]
    fn thinking_models_detected_by_name() {
        assert_eq!(
            infer_capabilities("deepseek-r1:8b", None),
            (false, true, true)
        );
        assert_eq!(infer_capabilities("qwq:32b", None), (false, true, true));
        assert_eq!(infer_capabilities("qwen3:8b", None), (false, true, true));
        assert_eq!(
            infer_capabilities("marco-o1:latest", None),
            (false, true, true)
        );
        // :r1 tag variant
        assert_eq!(
            infer_capabilities("some-model:r1", None),
            (false, true, true)
        );
        // /r1 path variant
        assert_eq!(
            infer_capabilities("vendor/r1:latest", None),
            (false, true, true)
        );
    }

    #[test]
    fn vision_and_thinking_can_combine() {
        // hypothetical future model that is both vision + thinking
        let fs = families(&["llama", "clip"]);
        let (vision, tools, thinking) = infer_capabilities("deepseek-r1-vision:latest", Some(&fs));
        assert!(vision);
        assert!(tools);
        assert!(thinking);
    }
}

// ---------------------------------------------------------------------------
// Background key validation
// ---------------------------------------------------------------------------

/// Probe a single provider's API key via a lightweight `GET /models` request.
///
/// Returns:
/// - `Some(true)`  — HTTP 2xx or 429 (rate-limited = key is valid)
/// - `Some(false)` — HTTP 401 or 403 (key rejected by provider)
/// - `None`        — network error, 404, 5xx, etc. (don't update status)
///
/// Result of probing a provider's API key.
#[derive(Debug)]
pub struct ProbeResult {
    /// Whether the key is valid (true), invalid (false), or unknown (None).
    pub key_valid: Option<bool>,
    /// Model IDs available on this provider (empty if key invalid or models
    /// could not be listed, e.g. rate-limited or non-OpenAI-compatible).
    pub available_models: Vec<String>,
}

pub async fn probe_api_key(provider_id: &str, base_url: &str, api_key: &str) -> ProbeResult {
    use std::time::Duration;

    let client = match crate::http_client::proxied_client_builder()
        .timeout(Duration::from_secs(8))
        .build()
    {
        Ok(c) => c,
        Err(_) => {
            return ProbeResult {
                key_valid: None,
                available_models: Vec::new(),
            }
        }
    };

    let url = format!("{}/models", base_url.trim_end_matches('/'));

    let req = match provider_id.to_lowercase().as_str() {
        "anthropic" => client
            .get(&url)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01"),
        "gemini" => client.get(&url).header("x-goog-api-key", api_key),
        _ => client
            .get(&url)
            .header("Authorization", format!("Bearer {api_key}")),
    };

    let resp = match req.send().await {
        Ok(r) => r,
        Err(_) => {
            return ProbeResult {
                key_valid: None,
                available_models: Vec::new(),
            }
        }
    };

    let status = resp.status().as_u16();
    tracing::debug!(provider = %provider_id, http_status = status, "provider key probe");

    match status {
        200..=299 => {
            // Key is valid — try to extract model IDs from the response body.
            let models = resp
                .json::<serde_json::Value>()
                .await
                .ok()
                .and_then(|body| {
                    // OpenAI-compatible format: { "data": [{ "id": "gpt-4o" }, ...] }
                    // Gemini format: { "models": [{ "name": "models/gemini-..." }, ...] }
                    if let Some(arr) = body.get("data").and_then(|d| d.as_array()) {
                        Some(
                            arr.iter()
                                .filter_map(|m| {
                                    m.get("id").and_then(|v| v.as_str()).map(String::from)
                                })
                                .collect::<Vec<_>>(),
                        )
                    } else {
                        body.get("models").and_then(|d| d.as_array()).map(|arr| {
                            arr.iter()
                                .filter_map(|m| {
                                    m.get("name")
                                        .or_else(|| m.get("id"))
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.strip_prefix("models/").unwrap_or(s).to_string())
                                })
                                .collect::<Vec<_>>()
                        })
                    }
                })
                .unwrap_or_default();
            ProbeResult {
                key_valid: Some(true),
                available_models: models,
            }
        }
        429 => ProbeResult {
            key_valid: Some(true), // rate-limited but key is valid
            available_models: Vec::new(),
        },
        401 | 403 => ProbeResult {
            key_valid: Some(false),
            available_models: Vec::new(),
        },
        _ => ProbeResult {
            key_valid: None, // transient / unknown — don't penalise
            available_models: Vec::new(),
        },
    }
}
