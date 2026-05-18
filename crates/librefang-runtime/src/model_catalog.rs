//! Model catalog — registry of known models with metadata, pricing, and auth detection.
//!
//! Provides a comprehensive catalog of 130+ builtin models across 28 providers,
//! with alias resolution, auth status detection, and pricing lookups.

use librefang_types::model_catalog::{
    AliasesCatalogFile, AuthStatus, EffectiveCapabilities, ModelCatalogEntry, ModelCatalogFile,
    ModelOverrides, ModelTier, ProviderInfo,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use tracing::warn;

/// The model catalog — registry of all known models and providers.
///
/// `Clone` is required by the kernel's `ArcSwap<ModelCatalog>` storage
/// (#3384): writes use the RCU pattern (clone snapshot → mutate → store)
/// so the structurally-large catalog (130+ models × 28 providers + per-model
/// overrides) is duplicated only on the rare `&mut self` paths
/// (`set_provider_auth_status`, `merge_discovered_models`, …) and never on
/// the hot read paths.
#[derive(Clone)]
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

/// Resolve capabilities from the explicit Ollama ≥0.7 `capabilities` array, falling back to name heuristics when empty.
fn resolve_discovered_capabilities(
    name: &str,
    families: Option<&[String]>,
    capabilities: &[String],
) -> (bool, bool, bool) {
    if capabilities.is_empty() {
        return infer_capabilities(name, families);
    }
    let is_embedding = capabilities
        .iter()
        .any(|c| c.eq_ignore_ascii_case("embedding"));
    if is_embedding {
        return (false, false, false);
    }
    let has_vision = capabilities
        .iter()
        .any(|c| c.eq_ignore_ascii_case("vision"));
    let has_thinking = capabilities
        .iter()
        .any(|c| c.eq_ignore_ascii_case("thinking"));
    // `tools`/`completion` is the default for any non-embedding chat model.
    // Ollama ≥0.7 emits an explicit `tools` capability for tool-aware models;
    // older daemons just emit `completion`. We treat any non-embedding model
    // as tool-capable to preserve the prior behaviour (`!is_embedding`) and
    // because most modern chat models expose tool calls via the OpenAI shape.
    let supports_tools = true;
    (has_vision, supports_tools, has_thinking)
}

impl ModelCatalog {
    /// Construct a catalog directly from owned entries without going
    /// through TOML loading. Used by:
    /// - this crate's sibling-module unit tests (`model_metadata`, etc.)
    ///   for deterministic fixture injection;
    /// - `librefang-testing::MockKernelBuilder::with_catalog_seed` (#4796)
    ///   so integration tests can pin a known catalog without depending
    ///   on the network-fed `registry_sync` baseline.
    ///
    /// Only assembles owned data — no invariant beyond the alias-lowering
    /// rule — so exposing it outside `cfg(test)` is safe. Production
    /// paths still go through `ModelCatalog::new()` and the
    /// `registry_sync` pipeline; this constructor exists purely as a
    /// fixture-building seam.
    pub fn from_entries(models: Vec<ModelCatalogEntry>, providers: Vec<ProviderInfo>) -> Self {
        let mut aliases: HashMap<String, String> = HashMap::new();
        for m in &models {
            for alias in &m.aliases {
                aliases
                    .entry(alias.to_lowercase())
                    .or_insert_with(|| m.id.clone());
            }
        }
        Self {
            models,
            aliases,
            providers,
            suppressed_providers: HashSet::new(),
            overrides: HashMap::new(),
        }
    }
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
            let file = match toml::from_str::<ModelCatalogFile>(source) {
                Ok(f) => f,
                Err(e) => {
                    // A syntax error here previously reverted to defaults with
                    // no log — a misconfigured custom provider just vanished.
                    tracing::warn!(%e, "provider catalog TOML ignored: parse failed");
                    continue;
                }
            };
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
                // Reject malformed text entries (zero context_window /
                // max_output_tokens) so we fail at parse instead of
                // silently feeding 0 into compaction / budget math.
                if let Err(e) = model.validate() {
                    tracing::warn!("Skipping invalid catalog entry: {e}");
                    continue;
                }
                models.push(model);
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
            // Suppression — set by `delete_provider_key` when the user
            // explicitly hides a provider from the configured page (#4803).
            // For non-key-required providers (CLI, local HTTP) the rest of
            // this loop would unconditionally re-detect them as available,
            // defeating the user's intent. Honour suppression up front so
            // "remove key" works regardless of provider shape.
            let suppressed = self.suppressed_providers.contains(&provider.id);

            // CLI-based providers: no API key needed, but we probe for CLI
            // installation so the dashboard shows "Configured" vs "Not Installed".
            if crate::drivers::is_cli_provider(&provider.id) {
                provider.auth_status = if suppressed {
                    AuthStatus::Missing
                } else if crate::drivers::cli_provider_available(&provider.id) {
                    AuthStatus::Configured
                } else {
                    AuthStatus::CliNotInstalled
                };
                continue;
            }

            if !provider.key_required {
                if suppressed {
                    // User explicitly hid this provider. Holding it as
                    // Missing keeps it out of the configured grid until
                    // `unsuppress_provider` runs (e.g. via `set_provider_url`).
                    provider.auth_status = AuthStatus::Missing;
                    continue;
                }
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

            // Secondary: recognised alias env var. Only officially documented
            // aliases count (e.g. Google AI Studio docs both `GEMINI_API_KEY`
            // and `GOOGLE_API_KEY` as equivalent). This is NOT a CLI-to-API
            // mapping — both are explicit API keys the user set.
            //
            // LibreFang intentionally does NOT promote a CLI login (Claude
            // Code, Codex, Gemini CLI, Qwen Code) to "configured" for the
            // corresponding API provider. CLI auth and API-key auth are
            // surfaced as separate providers so the user sees exactly what
            // they configured — CLI logins show up under `claude-code` /
            // `codex-cli` / `gemini-cli` / `qwen-code`, API keys under
            // `anthropic` / `openai` / `gemini` / `qwen`.
            let has_key_alias = if suppressed {
                false
            } else {
                provider.id == "gemini"
                    && std::env::var("GOOGLE_API_KEY").is_ok_and(|v| !v.trim().is_empty())
            };

            provider.auth_status = if has_key {
                AuthStatus::Configured
            } else if has_key_alias {
                AuthStatus::AutoDetected
            } else {
                AuthStatus::Missing
            };
            tracing::debug!(
                provider = %provider.id,
                has_key,
                has_key_alias,
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

    /// Find a model by canonical ID restricted to a specific provider.
    ///
    /// Same model ID can exist under multiple providers with different
    /// `context_window` values (e.g. `claude-opus-4-7` is 1M on
    /// `anthropic` but 128K on `copilot`). [`Self::find_model`] is
    /// provider-blind and may return the first match — this method
    /// resolves the ambiguity when the caller knows which provider the
    /// agent is targeting.
    ///
    /// Resolution order:
    /// 1. Exact `(provider, id)` match (case-insensitive on both).
    /// 2. Exact `(provider, alias)` match resolved via the alias map.
    /// 3. `None`. Callers fall back to [`Self::find_model`] for
    ///    cross-provider lookup.
    ///
    /// `provider` matches case-insensitively. An empty `provider`
    /// disables the provider filter and behaves like
    /// [`Self::find_model`].
    pub fn find_model_for_provider(
        &self,
        provider: &str,
        id_or_alias: &str,
    ) -> Option<&ModelCatalogEntry> {
        if provider.is_empty() {
            return self.find_model(id_or_alias);
        }
        let want_provider = provider.to_lowercase();
        let want_id = id_or_alias.to_lowercase();

        // Pass 1: exact (provider, id) match. Custom-tier wins, otherwise
        // first occurrence (mirrors the precedence in `find_model`).
        let mut found: Option<&ModelCatalogEntry> = None;
        for m in &self.models {
            if m.provider.to_lowercase() == want_provider && m.id.to_lowercase() == want_id {
                if m.tier == ModelTier::Custom {
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

        // Pass 2: alias resolution restricted to the provider.
        if let Some(canonical) = self.aliases.get(&want_id) {
            return self
                .models
                .iter()
                .find(|m| m.provider.to_lowercase() == want_provider && m.id == *canonical);
        }

        None
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

    /// Whether `id` is currently in the suppressed-providers set.
    ///
    /// Background loops that bypass `detect_auth` (notably
    /// `probe_all_local_providers_once`, which writes `set_provider_auth_status`
    /// directly) need this check so an unrelated tick does not silently
    /// un-do a user's "remove key" action.
    pub fn is_suppressed(&self, id: &str) -> bool {
        self.suppressed_providers.contains(id)
    }

    /// Return `(id, base_url)` for every local HTTP provider that the
    /// periodic probe loop should poll. Filters out providers the user has
    /// explicitly suppressed — without this, the next probe tick would
    /// overwrite the `Missing` status set by `delete_provider_key` with
    /// `NotRequired`/`LocalOffline` and the provider would re-appear in
    /// the configured grid (#4803).
    pub fn local_provider_probe_targets(&self) -> Vec<(String, String)> {
        self.providers
            .iter()
            .filter(|p| {
                crate::provider_health::is_local_provider(&p.id)
                    && !p.base_url.is_empty()
                    && !self.suppressed_providers.contains(&p.id)
            })
            .map(|p| (p.id.clone(), p.base_url.clone()))
            .collect()
    }

    /// Load the suppressed-providers list from a JSON file.
    pub fn load_suppressed(&mut self, path: &std::path::Path) {
        let data = match std::fs::read_to_string(path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(e) => {
                tracing::warn!(path = %path.display(), %e, "suppressed-providers file ignored: read failed");
                return;
            }
        };
        match serde_json::from_str::<Vec<String>>(&data) {
            Ok(list) => self.suppressed_providers = list.into_iter().collect(),
            Err(e) => {
                // A malformed file previously reverted to defaults silently —
                // previously suppressed providers reappeared with no log.
                tracing::warn!(path = %path.display(), %e, "suppressed-providers file ignored: parse failed");
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

    /// Compute the effective capabilities for a catalog entry, applying any
    /// user override on top of the catalog-declared values (refs #4745).
    ///
    /// Override key matches the persistence shape `provider:model_id`. Each
    /// `Option<bool>` field on `ModelOverrides`: `None` defers to the catalog,
    /// `Some(v)` forces the capability to `v`.
    pub fn effective_capabilities(&self, entry: &ModelCatalogEntry) -> EffectiveCapabilities {
        let key = format!("{}:{}", entry.provider, entry.id);
        let o = self.overrides.get(&key);
        EffectiveCapabilities {
            supports_tools: o
                .and_then(|x| x.supports_tools)
                .unwrap_or(entry.supports_tools),
            supports_vision: o
                .and_then(|x| x.supports_vision)
                .unwrap_or(entry.supports_vision),
            supports_streaming: o
                .and_then(|x| x.supports_streaming)
                .unwrap_or(entry.supports_streaming),
            supports_thinking: o
                .and_then(|x| x.supports_thinking)
                .unwrap_or(entry.supports_thinking),
        }
    }

    /// Look up a model by id-or-alias and return its effective capabilities.
    /// Returns `None` if no such model exists in the catalog.
    pub fn effective_capabilities_for(&self, id_or_alias: &str) -> Option<EffectiveCapabilities> {
        self.find_model(id_or_alias)
            .map(|m| self.effective_capabilities(m))
    }

    /// Load model overrides from a JSON file.
    pub fn load_overrides(&mut self, path: &std::path::Path) {
        let data = match std::fs::read_to_string(path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(e) => {
                tracing::warn!(path = %path.display(), %e, "model-overrides file ignored: read failed");
                return;
            }
        };
        match serde_json::from_str::<HashMap<String, ModelOverrides>>(&data) {
            Ok(map) => self.overrides = map,
            Err(e) => {
                // A syntax error previously reverted to defaults with no log,
                // silently dropping the operator's per-model tuning.
                tracing::warn!(path = %path.display(), %e, "model-overrides file ignored: parse failed");
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
    pub fn apply_url_overrides(&mut self, overrides: &BTreeMap<String, String>) {
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
    pub fn apply_proxy_url_overrides(&mut self, overrides: &BTreeMap<String, String>) {
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
        region_selections: &BTreeMap<String, String>,
    ) -> BTreeMap<String, String> {
        let mut resolved = BTreeMap::new();
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
        region_selections: &BTreeMap<String, String>,
    ) -> BTreeMap<String, String> {
        let mut resolved = BTreeMap::new();
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
        // Index existing entries for this provider by lowercase ID so we can
        // both skip duplicates and selectively upgrade Local-tier entries
        // whose capability flags were inferred from a stale signal (e.g.
        // a previous probe before the user upgraded to Ollama ≥0.7, or a
        // first probe that lacked the explicit `capabilities` array).
        let mut existing_local: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        let mut existing_non_local: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for (idx, m) in self.models.iter().enumerate() {
            if m.provider != provider {
                continue;
            }
            let key = m.id.to_lowercase();
            if m.tier == ModelTier::Local {
                existing_local.insert(key, idx);
            } else {
                existing_non_local.insert(key);
            }
        }

        let mut added = 0usize;
        for info in model_info {
            let key = info.name.to_lowercase();
            // A registry-shipped (non-Local) entry already covers this ID —
            // its curated metadata (pricing, context window, tested caps)
            // takes precedence over what a local probe reports.
            if existing_non_local.contains(&key) {
                continue;
            }
            let (supports_vision, supports_tools, supports_thinking) =
                resolve_discovered_capabilities(
                    &info.name,
                    info.families.as_deref(),
                    &info.capabilities,
                );
            // Upgrade the previously-discovered Local entry in place when the
            // current probe reports stronger capabilities. We never downgrade:
            // a transient probe that drops the `capabilities` array (e.g. an
            // older proxy in front of an upgraded Ollama) must not flip a
            // vision-capable model back to non-vision.
            if let Some(&idx) = existing_local.get(&key) {
                let entry = &mut self.models[idx];
                if supports_vision {
                    entry.supports_vision = true;
                }
                if supports_thinking {
                    entry.supports_thinking = true;
                }
                if supports_tools {
                    entry.supports_tools = true;
                    if !entry.supports_streaming {
                        entry.supports_streaming = true;
                    }
                }
                continue;
            }
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
                supports_streaming: supports_tools,
                supports_thinking,
                aliases: Vec::new(),
                ..Default::default()
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
            // Modality-aware schema gate (see ModelCatalogEntry::validate).
            if let Err(e) = model.validate() {
                tracing::warn!("Skipping invalid catalog entry: {e}");
                continue;
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
                match std::fs::read_to_string(aliases_path) {
                    Ok(data) => match toml::from_str::<AliasesCatalogFile>(&data) {
                        Ok(aliases_file) => {
                            for (alias, canonical) in aliases_file.aliases {
                                self.aliases
                                    .entry(alias.to_lowercase())
                                    .or_insert(canonical);
                            }
                        }
                        Err(e) => {
                            // A syntax error here previously dropped every
                            // alias silently — model lookups by alias then
                            // 404'd with no explanation.
                            tracing::warn!(
                                path = %aliases_path.display(),
                                %e,
                                "aliases.toml ignored: parse failed"
                            );
                        }
                    },
                    Err(e) => {
                        tracing::warn!(
                            path = %aliases_path.display(),
                            %e,
                            "aliases.toml ignored: read failed"
                        );
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

#[cfg(test)]
mod tests;
