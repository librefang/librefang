//! Model catalog — registry of known models with metadata, pricing, and auth detection.
//!
//! Provides a comprehensive catalog of 130+ builtin models across 28 providers,
//! with alias resolution, auth status detection, and pricing lookups.

use librefang_types::model_catalog::{
    AliasesCatalogFile, AuthStatus, ModelCatalogEntry, ModelCatalogFile, ModelTier, ProviderInfo,
};
use std::collections::HashMap;

/// The model catalog — registry of all known models and providers.
pub struct ModelCatalog {
    models: Vec<ModelCatalogEntry>,
    aliases: HashMap<String, String>,
    providers: Vec<ProviderInfo>,
}

impl ModelCatalog {
    /// Create a new catalog populated with builtin models and providers.
    pub fn new() -> Self {
        let models = builtin_models();
        let mut aliases = builtin_aliases();
        let mut providers = builtin_providers();

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
        }
    }

    /// Detect which providers have API keys configured.
    ///
    /// Checks `std::env::var()` for each provider's API key env var.
    /// Only checks presence — never reads or stores the actual secret.
    pub fn detect_auth(&mut self) {
        for provider in &mut self.providers {
            // Claude Code is special: no API key needed, but we probe for CLI
            // installation so the dashboard shows "Configured" vs "Not Installed".
            if provider.id == "claude-code" {
                provider.auth_status = if crate::drivers::claude_code::claude_code_available() {
                    AuthStatus::Configured
                } else {
                    AuthStatus::Missing
                };
                continue;
            }

            if !provider.key_required {
                provider.auth_status = AuthStatus::NotRequired;
                continue;
            }

            // Primary: check the provider's declared env var
            let has_key = std::env::var(&provider.api_key_env).is_ok();

            // Secondary: provider-specific fallback auth
            let has_fallback = match provider.id.as_str() {
                "gemini" => std::env::var("GOOGLE_API_KEY").is_ok(),
                "codex" => {
                    std::env::var("OPENAI_API_KEY").is_ok() || read_codex_credential().is_some()
                }
                // claude-code is handled above (before key_required check)
                _ => false,
            };

            provider.auth_status = if has_key || has_fallback {
                AuthStatus::Configured
            } else {
                AuthStatus::Missing
            };
        }
    }

    /// List all models in the catalog.
    pub fn list_models(&self) -> &[ModelCatalogEntry] {
        &self.models
    }

    /// Find a model by its canonical ID or by alias.
    pub fn find_model(&self, id_or_alias: &str) -> Option<&ModelCatalogEntry> {
        let lower = id_or_alias.to_lowercase();
        // Direct ID match first
        if let Some(entry) = self.models.iter().find(|m| m.id.to_lowercase() == lower) {
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
        // Check aliases first — e.g. "minimax" alias resolves to "MiniMax-M2.5"
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
            .filter(|p| p.auth_status != AuthStatus::Missing)
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

    /// List models filtered by tier.
    pub fn models_by_tier(&self, tier: ModelTier) -> Vec<&ModelCatalogEntry> {
        self.models.iter().filter(|m| m.tier == tier).collect()
    }

    /// Merge dynamically discovered models from a local provider.
    ///
    /// Adds models not already in the catalog with `Local` tier and zero cost.
    /// Also updates the provider's `model_count`.
    pub fn merge_discovered_models(&mut self, provider: &str, model_ids: &[String]) {
        let existing_ids: std::collections::HashSet<String> = self
            .models
            .iter()
            .filter(|m| m.provider == provider)
            .map(|m| m.id.to_lowercase())
            .collect();

        let mut added = 0usize;
        for id in model_ids {
            if existing_ids.contains(&id.to_lowercase()) {
                continue;
            }
            // Generate a human-friendly display name
            let display = format!("{} ({})", id, provider);
            self.models.push(ModelCatalogEntry {
                id: id.clone(),
                display_name: display,
                provider: provider.to_string(),
                tier: ModelTier::Local,
                context_window: 32_768,
                max_output_tokens: 4_096,
                input_cost_per_m: 0.0,
                output_cost_per_m: 0.0,
                supports_tools: true,
                supports_vision: false,
                supports_streaming: true,
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
                    existing.api_key_env = prov_toml.api_key_env;
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

    /// Load cached catalog from the default location (`~/.librefang/cache/catalog/providers/`).
    ///
    /// Convenience wrapper around `load_cached_catalog(dir)` for use during kernel init.
    pub fn load_default_cached_catalog(&mut self) {
        if let Some(home) = dirs::home_dir() {
            let providers_dir = home
                .join(".librefang")
                .join("cache")
                .join("catalog")
                .join("providers");
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
    }

    /// Load user-defined models from `~/.librefang/model_catalog.toml`.
    ///
    /// User models override builtins and cached models by ID.
    pub fn load_default_user_catalog(&mut self) {
        if let Some(home) = dirs::home_dir() {
            let user_catalog = home.join(".librefang").join("model_catalog.toml");
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
}

impl Default for ModelCatalog {
    fn default() -> Self {
        Self::new()
    }
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

// ---------------------------------------------------------------------------
// Builtin data — loaded from embedded TOML catalog files at compile time
// ---------------------------------------------------------------------------

const BUILTIN_AI21: &str = include_str!("../../../catalog/providers/ai21.toml");
const BUILTIN_ANTHROPIC: &str = include_str!("../../../catalog/providers/anthropic.toml");
const BUILTIN_BEDROCK: &str = include_str!("../../../catalog/providers/bedrock.toml");
const BUILTIN_CEREBRAS: &str = include_str!("../../../catalog/providers/cerebras.toml");
const BUILTIN_CHATGPT: &str = include_str!("../../../catalog/providers/chatgpt.toml");
const BUILTIN_CHUTES: &str = include_str!("../../../catalog/providers/chutes.toml");
const BUILTIN_CLAUDE_CODE: &str = include_str!("../../../catalog/providers/claude-code.toml");
const BUILTIN_COHERE: &str = include_str!("../../../catalog/providers/cohere.toml");
const BUILTIN_DEEPSEEK: &str = include_str!("../../../catalog/providers/deepseek.toml");
const BUILTIN_FIREWORKS: &str = include_str!("../../../catalog/providers/fireworks.toml");
const BUILTIN_GEMINI: &str = include_str!("../../../catalog/providers/gemini.toml");
const BUILTIN_GITHUB_COPILOT: &str = include_str!("../../../catalog/providers/github-copilot.toml");
const BUILTIN_GROQ: &str = include_str!("../../../catalog/providers/groq.toml");
const BUILTIN_HUGGINGFACE: &str = include_str!("../../../catalog/providers/huggingface.toml");
const BUILTIN_KIMI_CODING: &str = include_str!("../../../catalog/providers/kimi-coding.toml");
const BUILTIN_LEMONADE: &str = include_str!("../../../catalog/providers/lemonade.toml");
const BUILTIN_LMSTUDIO: &str = include_str!("../../../catalog/providers/lmstudio.toml");
const BUILTIN_MINIMAX: &str = include_str!("../../../catalog/providers/minimax.toml");
const BUILTIN_MINIMAX_CN: &str = include_str!("../../../catalog/providers/minimax-cn.toml");
const BUILTIN_MISTRAL: &str = include_str!("../../../catalog/providers/mistral.toml");
const BUILTIN_MOONSHOT: &str = include_str!("../../../catalog/providers/moonshot.toml");
const BUILTIN_OLLAMA: &str = include_str!("../../../catalog/providers/ollama.toml");
const BUILTIN_OPENAI: &str = include_str!("../../../catalog/providers/openai.toml");
const BUILTIN_OPENROUTER: &str = include_str!("../../../catalog/providers/openrouter.toml");
const BUILTIN_PERPLEXITY: &str = include_str!("../../../catalog/providers/perplexity.toml");
const BUILTIN_QIANFAN: &str = include_str!("../../../catalog/providers/qianfan.toml");
const BUILTIN_QWEN_CODE: &str = include_str!("../../../catalog/providers/qwen-code.toml");
const BUILTIN_QWEN: &str = include_str!("../../../catalog/providers/qwen.toml");
const BUILTIN_REPLICATE: &str = include_str!("../../../catalog/providers/replicate.toml");
const BUILTIN_SAMBANOVA: &str = include_str!("../../../catalog/providers/sambanova.toml");
const BUILTIN_TOGETHER: &str = include_str!("../../../catalog/providers/together.toml");
const BUILTIN_VENICE: &str = include_str!("../../../catalog/providers/venice.toml");
const BUILTIN_VERTEX_AI: &str = include_str!("../../../catalog/providers/vertex-ai.toml");
const BUILTIN_VLLM: &str = include_str!("../../../catalog/providers/vllm.toml");
const BUILTIN_VOLCENGINE_CODING: &str =
    include_str!("../../../catalog/providers/volcengine-coding.toml");
const BUILTIN_VOLCENGINE: &str = include_str!("../../../catalog/providers/volcengine.toml");
const BUILTIN_XAI: &str = include_str!("../../../catalog/providers/xai.toml");
const BUILTIN_ZAI_CODING: &str = include_str!("../../../catalog/providers/zai-coding.toml");
const BUILTIN_ZAI: &str = include_str!("../../../catalog/providers/zai.toml");
const BUILTIN_ZHIPU_CODING: &str = include_str!("../../../catalog/providers/zhipu-coding.toml");
const BUILTIN_ZHIPU: &str = include_str!("../../../catalog/providers/zhipu.toml");

const BUILTIN_ALIASES: &str = include_str!("../../../catalog/aliases.toml");

/// All builtin provider TOML sources.
const BUILTIN_PROVIDER_SOURCES: &[&str] = &[
    BUILTIN_AI21,
    BUILTIN_ANTHROPIC,
    BUILTIN_BEDROCK,
    BUILTIN_CEREBRAS,
    BUILTIN_CHATGPT,
    BUILTIN_CHUTES,
    BUILTIN_CLAUDE_CODE,
    BUILTIN_COHERE,
    BUILTIN_DEEPSEEK,
    BUILTIN_FIREWORKS,
    BUILTIN_GEMINI,
    BUILTIN_GITHUB_COPILOT,
    BUILTIN_GROQ,
    BUILTIN_HUGGINGFACE,
    BUILTIN_KIMI_CODING,
    BUILTIN_LEMONADE,
    BUILTIN_LMSTUDIO,
    BUILTIN_MINIMAX,
    BUILTIN_MINIMAX_CN,
    BUILTIN_MISTRAL,
    BUILTIN_MOONSHOT,
    BUILTIN_OLLAMA,
    BUILTIN_OPENAI,
    BUILTIN_OPENROUTER,
    BUILTIN_PERPLEXITY,
    BUILTIN_QIANFAN,
    BUILTIN_QWEN_CODE,
    BUILTIN_QWEN,
    BUILTIN_REPLICATE,
    BUILTIN_SAMBANOVA,
    BUILTIN_TOGETHER,
    BUILTIN_VENICE,
    BUILTIN_VERTEX_AI,
    BUILTIN_VLLM,
    BUILTIN_VOLCENGINE_CODING,
    BUILTIN_VOLCENGINE,
    BUILTIN_XAI,
    BUILTIN_ZAI_CODING,
    BUILTIN_ZAI,
    BUILTIN_ZHIPU_CODING,
    BUILTIN_ZHIPU,
];

fn builtin_providers() -> Vec<ProviderInfo> {
    let mut providers = Vec::new();
    for source in BUILTIN_PROVIDER_SOURCES {
        let file: ModelCatalogFile =
            toml::from_str(source).expect("builtin provider TOML is invalid");
        if let Some(p) = file.provider {
            providers.push(p.into());
        }
    }
    providers
}

fn builtin_aliases() -> HashMap<String, String> {
    let file: AliasesCatalogFile =
        toml::from_str(BUILTIN_ALIASES).expect("builtin aliases TOML is invalid");
    file.aliases
        .into_iter()
        .map(|(k, v)| (k.to_lowercase(), v))
        .collect()
}

fn builtin_models() -> Vec<ModelCatalogEntry> {
    let mut models = Vec::new();
    for source in BUILTIN_PROVIDER_SOURCES {
        let file: ModelCatalogFile =
            toml::from_str(source).expect("builtin model catalog TOML is invalid");
        models.extend(file.models);
    }
    models
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::model_catalog::{LMSTUDIO_BASE_URL, OLLAMA_BASE_URL};

    #[test]
    fn test_catalog_has_models() {
        let catalog = ModelCatalog::new();
        assert!(catalog.list_models().len() >= 30);
    }

    #[test]
    fn test_catalog_has_providers() {
        let catalog = ModelCatalog::new();
        assert_eq!(catalog.list_providers().len(), 41);
    }

    #[test]
    fn test_find_model_by_id() {
        let catalog = ModelCatalog::new();
        let entry = catalog.find_model("claude-sonnet-4-20250514").unwrap();
        assert_eq!(entry.display_name, "Claude Sonnet 4");
        assert_eq!(entry.provider, "anthropic");
        assert_eq!(entry.tier, ModelTier::Smart);
    }

    #[test]
    fn test_find_model_by_alias() {
        let catalog = ModelCatalog::new();
        let entry = catalog.find_model("sonnet").unwrap();
        assert_eq!(entry.id, "claude-sonnet-4-6");
    }

    #[test]
    fn test_find_model_case_insensitive() {
        let catalog = ModelCatalog::new();
        assert!(catalog.find_model("Claude-Sonnet-4-20250514").is_some());
        assert!(catalog.find_model("SONNET").is_some());
    }

    #[test]
    fn test_find_model_not_found() {
        let catalog = ModelCatalog::new();
        assert!(catalog.find_model("nonexistent-model").is_none());
    }

    #[test]
    fn test_resolve_alias() {
        let catalog = ModelCatalog::new();
        assert_eq!(catalog.resolve_alias("sonnet"), Some("claude-sonnet-4-6"));
        assert_eq!(
            catalog.resolve_alias("haiku"),
            Some("claude-haiku-4-5-20251001")
        );
        assert!(catalog.resolve_alias("nonexistent").is_none());
    }

    #[test]
    fn test_models_by_provider() {
        let catalog = ModelCatalog::new();
        let anthropic = catalog.models_by_provider("anthropic");
        assert_eq!(anthropic.len(), 7);
        assert!(anthropic.iter().all(|m| m.provider == "anthropic"));
    }

    #[test]
    fn test_models_by_tier() {
        let catalog = ModelCatalog::new();
        let frontier = catalog.models_by_tier(ModelTier::Frontier);
        assert!(frontier.len() >= 3); // At least opus, gpt-4.1, gemini-2.5-pro
        assert!(frontier.iter().all(|m| m.tier == ModelTier::Frontier));
    }

    #[test]
    fn test_pricing_lookup() {
        let catalog = ModelCatalog::new();
        let (input, output) = catalog.pricing("claude-sonnet-4-20250514").unwrap();
        assert!((input - 3.0).abs() < 0.001);
        assert!((output - 15.0).abs() < 0.001);
    }

    #[test]
    fn test_pricing_via_alias() {
        let catalog = ModelCatalog::new();
        let (input, output) = catalog.pricing("sonnet").unwrap();
        assert!((input - 3.0).abs() < 0.001);
        assert!((output - 15.0).abs() < 0.001);
    }

    #[test]
    fn test_pricing_not_found() {
        let catalog = ModelCatalog::new();
        assert!(catalog.pricing("nonexistent").is_none());
    }

    #[test]
    fn test_detect_auth_local_providers() {
        let mut catalog = ModelCatalog::new();
        catalog.detect_auth();
        // Local providers should be NotRequired
        let ollama = catalog.get_provider("ollama").unwrap();
        assert_eq!(ollama.auth_status, AuthStatus::NotRequired);
        let vllm = catalog.get_provider("vllm").unwrap();
        assert_eq!(vllm.auth_status, AuthStatus::NotRequired);
    }

    #[test]
    fn test_available_models_includes_local() {
        let mut catalog = ModelCatalog::new();
        catalog.detect_auth();
        let available = catalog.available_models();
        // Local providers (ollama, vllm, lmstudio) should always be available
        assert!(available.iter().any(|m| m.provider == "ollama"));
    }

    #[test]
    fn test_provider_model_counts() {
        let catalog = ModelCatalog::new();
        let anthropic = catalog.get_provider("anthropic").unwrap();
        assert_eq!(anthropic.model_count, 7);
        let groq = catalog.get_provider("groq").unwrap();
        assert_eq!(groq.model_count, 10);
    }

    #[test]
    fn test_list_aliases() {
        let catalog = ModelCatalog::new();
        let aliases = catalog.list_aliases();
        assert!(aliases.len() >= 20);
        assert_eq!(aliases.get("sonnet").unwrap(), "claude-sonnet-4-6");
        // New aliases
        assert_eq!(aliases.get("grok").unwrap(), "grok-4-0709");
        assert_eq!(aliases.get("jamba").unwrap(), "jamba-1.5-large");
    }

    #[test]
    fn test_find_grok_by_alias() {
        let catalog = ModelCatalog::new();
        let entry = catalog.find_model("grok").unwrap();
        assert_eq!(entry.id, "grok-4-0709");
        assert_eq!(entry.provider, "xai");
    }

    #[test]
    fn test_add_alias() {
        let mut catalog = ModelCatalog::new();
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
        let mut catalog = ModelCatalog::new();
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
        let catalog = ModelCatalog::new();
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
        let catalog = ModelCatalog::new();
        let xai = catalog.models_by_provider("xai");
        assert_eq!(xai.len(), 9);
        assert!(xai.iter().any(|m| m.id == "grok-4-0709"));
        assert!(xai.iter().any(|m| m.id == "grok-4-fast-reasoning"));
        assert!(xai.iter().any(|m| m.id == "grok-4-fast-non-reasoning"));
        assert!(xai.iter().any(|m| m.id == "grok-4-1-fast-reasoning"));
        assert!(xai.iter().any(|m| m.id == "grok-4-1-fast-non-reasoning"));
        assert!(xai.iter().any(|m| m.id == "grok-3"));
        assert!(xai.iter().any(|m| m.id == "grok-3-mini"));
        assert!(xai.iter().any(|m| m.id == "grok-2"));
        assert!(xai.iter().any(|m| m.id == "grok-2-mini"));
    }

    #[test]
    fn test_perplexity_models() {
        let catalog = ModelCatalog::new();
        let pp = catalog.models_by_provider("perplexity");
        assert_eq!(pp.len(), 4);
    }

    #[test]
    fn test_cohere_models() {
        let catalog = ModelCatalog::new();
        let co = catalog.models_by_provider("cohere");
        assert_eq!(co.len(), 4);
    }

    #[test]
    fn test_default_creates_valid_catalog() {
        let catalog = ModelCatalog::default();
        assert!(!catalog.list_models().is_empty());
        assert!(!catalog.list_providers().is_empty());
    }

    #[test]
    fn test_merge_adds_new_models() {
        let mut catalog = ModelCatalog::new();
        let before = catalog.models_by_provider("ollama").len();
        catalog.merge_discovered_models(
            "ollama",
            &["codestral:latest".to_string(), "qwen2:7b".to_string()],
        );
        let after = catalog.models_by_provider("ollama").len();
        assert_eq!(after, before + 2);
        // Verify the new models are Local tier with zero cost
        let qwen = catalog.find_model("qwen2:7b").unwrap();
        assert_eq!(qwen.tier, ModelTier::Local);
        assert!((qwen.input_cost_per_m).abs() < f64::EPSILON);
    }

    #[test]
    fn test_merge_skips_existing() {
        let mut catalog = ModelCatalog::new();
        // "llama3.2" is already a builtin Ollama model
        let before = catalog.list_models().len();
        catalog.merge_discovered_models("ollama", &["llama3.2".to_string()]);
        let after = catalog.list_models().len();
        assert_eq!(after, before); // no new model added
    }

    #[test]
    fn test_merge_updates_model_count() {
        let mut catalog = ModelCatalog::new();
        let before_count = catalog.get_provider("ollama").unwrap().model_count;
        catalog.merge_discovered_models("ollama", &["new-model:latest".to_string()]);
        let after_count = catalog.get_provider("ollama").unwrap().model_count;
        assert_eq!(after_count, before_count + 1);
    }

    #[test]
    fn test_chinese_providers_in_catalog() {
        let catalog = ModelCatalog::new();
        assert!(catalog.get_provider("qwen").is_some());
        assert!(catalog.get_provider("minimax").is_some());
        assert!(catalog.get_provider("zhipu").is_some());
        assert!(catalog.get_provider("zhipu_coding").is_some());
        assert!(catalog.get_provider("moonshot").is_some());
        assert!(catalog.get_provider("qianfan").is_some());
        assert!(catalog.get_provider("bedrock").is_some());
    }

    #[test]
    fn test_chinese_model_aliases() {
        let catalog = ModelCatalog::new();
        assert!(catalog.find_model("kimi").is_some());
        assert!(catalog.find_model("glm").is_some());
        assert!(catalog.find_model("codegeex").is_some());
        assert!(catalog.find_model("ernie").is_some());
        assert!(catalog.find_model("minimax").is_some());
        // MiniMax M2.5 — by exact ID, alias, and case-insensitive
        let m25 = catalog.find_model("MiniMax-M2.5").unwrap();
        assert_eq!(m25.provider, "minimax");
        assert_eq!(m25.tier, ModelTier::Frontier);
        assert!(catalog.find_model("minimax-m2.5").is_some());
        // Default "minimax" alias now points to M2.5
        let default = catalog.find_model("minimax").unwrap();
        assert_eq!(default.id, "MiniMax-M2.5");
        // MiniMax M2.5 Highspeed — by exact ID and aliases
        let hs = catalog.find_model("MiniMax-M2.5-highspeed").unwrap();
        assert_eq!(hs.provider, "minimax");
        assert_eq!(hs.tier, ModelTier::Smart);
        assert!(hs.supports_vision);
        assert!(hs.supports_tools);
        assert!(catalog.find_model("minimax-m2.5-highspeed").is_some());
        assert!(catalog.find_model("minimax-highspeed").is_some());
        // abab7-chat
        let abab7 = catalog.find_model("abab7-chat").unwrap();
        assert_eq!(abab7.provider, "minimax");
        assert!(abab7.supports_vision);
    }

    #[test]
    fn test_bedrock_models() {
        let catalog = ModelCatalog::new();
        let bedrock = catalog.models_by_provider("bedrock");
        assert_eq!(bedrock.len(), 8);
    }

    #[test]
    fn test_set_provider_url() {
        let mut catalog = ModelCatalog::new();
        let old_url = catalog.get_provider("ollama").unwrap().base_url.clone();
        assert_eq!(old_url, OLLAMA_BASE_URL);

        let updated = catalog.set_provider_url("ollama", "http://192.168.1.100:11434/v1");
        assert!(updated);
        assert_eq!(
            catalog.get_provider("ollama").unwrap().base_url,
            "http://192.168.1.100:11434/v1"
        );
    }

    #[test]
    fn test_set_provider_url_unknown() {
        let mut catalog = ModelCatalog::new();
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
        let mut catalog = ModelCatalog::new();
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
            LMSTUDIO_BASE_URL
        );
    }

    #[test]
    fn test_codex_models_under_openai() {
        // Codex models are now merged under the "openai" provider
        let catalog = ModelCatalog::new();
        let models = catalog.models_by_provider("openai");
        assert!(models.iter().any(|m| m.id == "codex/gpt-4.1"));
        assert!(models.iter().any(|m| m.id == "codex/o4-mini"));
    }

    #[test]
    fn test_codex_aliases() {
        let catalog = ModelCatalog::new();
        let entry = catalog.find_model("codex").unwrap();
        assert_eq!(entry.id, "codex/gpt-4.1");
    }

    #[test]
    fn test_claude_code_provider() {
        let catalog = ModelCatalog::new();
        let cc = catalog.get_provider("claude-code").unwrap();
        assert_eq!(cc.display_name, "Claude Code");
        assert!(!cc.key_required);
    }

    #[test]
    fn test_claude_code_models() {
        let catalog = ModelCatalog::new();
        let models = catalog.models_by_provider("claude-code");
        assert_eq!(models.len(), 3);
        assert!(models.iter().any(|m| m.id == "claude-code/opus"));
        assert!(models.iter().any(|m| m.id == "claude-code/sonnet"));
        assert!(models.iter().any(|m| m.id == "claude-code/haiku"));
    }

    #[test]
    fn test_claude_code_aliases() {
        let catalog = ModelCatalog::new();
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
        let mut catalog = ModelCatalog::new();
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

        let mut catalog = ModelCatalog::new();
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
        let mut catalog = ModelCatalog::new();
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

        let mut catalog = ModelCatalog::new();
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

        let mut catalog = ModelCatalog::new();
        let added = catalog.merge_catalog_file(file);
        assert_eq!(added, 1);

        let model = catalog.find_model("test-remote-model-1").unwrap();
        assert_eq!(model.provider, "test-remote");
    }
}
