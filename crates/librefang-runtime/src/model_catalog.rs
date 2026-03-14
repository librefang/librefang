//! Model catalog — registry of known models with metadata, pricing, and auth detection.
//!
//! Provides a comprehensive catalog of 130+ builtin models across 28 providers,
//! with alias resolution, auth status detection, and pricing lookups.

use librefang_types::model_catalog::{
    AliasCatalogFile, AuthStatus, ModelCatalogEntry, ModelCatalogFile, ModelTier,
    ProviderCatalogFile, ProviderInfo,
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

        let mut catalog = Self {
            models,
            aliases,
            providers,
        };

        // Load user-defined models from ~/.librefang/model_catalog.toml
        catalog.load_default_user_catalog();

        catalog
    }

    /// Load additional models from a user's local catalog file.
    ///
    /// User models take priority over builtin ones (same ID = override).
    /// Returns the number of new models added (overrides don't count).
    pub fn load_user_catalog(&mut self, path: &std::path::Path) -> Result<usize, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read catalog file {}: {}", path.display(), e))?;
        let file: ModelCatalogFile = toml::from_str(&content)
            .map_err(|e| format!("Failed to parse catalog file {}: {}", path.display(), e))?;

        let mut added = 0;
        for model in file.models {
            // Register any aliases from the model entry
            for alias in &model.aliases {
                let lower = alias.to_lowercase();
                self.aliases
                    .entry(lower)
                    .or_insert_with(|| model.id.clone());
            }

            // Override existing or add new
            if let Some(existing) = self.models.iter_mut().find(|m| m.id == model.id) {
                *existing = model;
            } else {
                self.models.push(model);
                added += 1;
            }
        }

        // Update provider model counts
        for provider in &mut self.providers {
            provider.model_count = self
                .models
                .iter()
                .filter(|m| m.provider == provider.id)
                .count();
        }

        Ok(added)
    }

    /// Load user catalog from the default location (`~/.librefang/model_catalog.toml`).
    pub fn load_default_user_catalog(&mut self) {
        if let Some(home) = dirs::home_dir() {
            let user_catalog = home.join(".librefang").join("model_catalog.toml");
            if user_catalog.exists() {
                match self.load_user_catalog(&user_catalog) {
                    Ok(n) => tracing::info!(
                        "Loaded {} user-defined models from {}",
                        n,
                        user_catalog.display()
                    ),
                    Err(e) => tracing::warn!("Failed to load user model catalog: {}", e),
                }
            }
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
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

// ---------------------------------------------------------------------------
// Builtin data — loaded from embedded TOML catalog files at compile time
// ---------------------------------------------------------------------------

/// Builtin provider catalog embedded at compile time.
const BUILTIN_PROVIDERS_TOML: &str = include_str!("../../../catalog/providers.toml");

/// Builtin alias catalog embedded at compile time.
const BUILTIN_ALIASES_TOML: &str = include_str!("../../../catalog/aliases.toml");

/// Builtin model catalog TOML sources embedded at compile time.
const BUILTIN_MODELS_ANTHROPIC: &str = include_str!("../../../catalog/models/anthropic.toml");
const BUILTIN_MODELS_OPENAI: &str = include_str!("../../../catalog/models/openai.toml");
const BUILTIN_MODELS_GEMINI: &str = include_str!("../../../catalog/models/gemini.toml");
const BUILTIN_MODELS_DEEPSEEK: &str = include_str!("../../../catalog/models/deepseek.toml");
const BUILTIN_MODELS_GROQ: &str = include_str!("../../../catalog/models/groq.toml");
const BUILTIN_MODELS_OPENROUTER: &str = include_str!("../../../catalog/models/openrouter.toml");
const BUILTIN_MODELS_MISTRAL: &str = include_str!("../../../catalog/models/mistral.toml");
const BUILTIN_MODELS_TOGETHER: &str = include_str!("../../../catalog/models/together.toml");
const BUILTIN_MODELS_FIREWORKS: &str = include_str!("../../../catalog/models/fireworks.toml");
const BUILTIN_MODELS_LOCAL: &str = include_str!("../../../catalog/models/local.toml");
const BUILTIN_MODELS_PERPLEXITY: &str = include_str!("../../../catalog/models/perplexity.toml");
const BUILTIN_MODELS_COHERE: &str = include_str!("../../../catalog/models/cohere.toml");
const BUILTIN_MODELS_AI21: &str = include_str!("../../../catalog/models/ai21.toml");
const BUILTIN_MODELS_CEREBRAS: &str = include_str!("../../../catalog/models/cerebras.toml");
const BUILTIN_MODELS_SAMBANOVA: &str = include_str!("../../../catalog/models/sambanova.toml");
const BUILTIN_MODELS_XAI: &str = include_str!("../../../catalog/models/xai.toml");
const BUILTIN_MODELS_HUGGINGFACE: &str = include_str!("../../../catalog/models/huggingface.toml");
const BUILTIN_MODELS_REPLICATE: &str = include_str!("../../../catalog/models/replicate.toml");
const BUILTIN_MODELS_GITHUB_COPILOT: &str =
    include_str!("../../../catalog/models/github_copilot.toml");
const BUILTIN_MODELS_CHINESE: &str = include_str!("../../../catalog/models/chinese.toml");
const BUILTIN_MODELS_BEDROCK: &str = include_str!("../../../catalog/models/bedrock.toml");
const BUILTIN_MODELS_CHATGPT: &str = include_str!("../../../catalog/models/chatgpt.toml");
const BUILTIN_MODELS_CLAUDE_CODE: &str = include_str!("../../../catalog/models/claude_code.toml");
const BUILTIN_MODELS_CHUTES: &str = include_str!("../../../catalog/models/chutes.toml");
const BUILTIN_MODELS_VENICE: &str = include_str!("../../../catalog/models/venice.toml");

fn builtin_providers() -> Vec<ProviderInfo> {
    let file: ProviderCatalogFile =
        toml::from_str(BUILTIN_PROVIDERS_TOML).expect("builtin providers TOML is invalid");
    // Ensure runtime fields are initialized
    file.providers
        .into_iter()
        .map(|mut p| {
            p.auth_status = if p.key_required {
                AuthStatus::Missing
            } else {
                AuthStatus::NotRequired
            };
            p.model_count = 0;
            p
        })
        .collect()
}

fn builtin_aliases() -> HashMap<String, String> {
    let file: AliasCatalogFile =
        toml::from_str(BUILTIN_ALIASES_TOML).expect("builtin aliases TOML is invalid");
    file.aliases
        .into_iter()
        .map(|(k, v)| (k.to_lowercase(), v))
        .collect()
}

fn builtin_models() -> Vec<ModelCatalogEntry> {
    let sources: &[&str] = &[
        BUILTIN_MODELS_ANTHROPIC,
        BUILTIN_MODELS_OPENAI,
        BUILTIN_MODELS_GEMINI,
        BUILTIN_MODELS_DEEPSEEK,
        BUILTIN_MODELS_GROQ,
        BUILTIN_MODELS_OPENROUTER,
        BUILTIN_MODELS_MISTRAL,
        BUILTIN_MODELS_TOGETHER,
        BUILTIN_MODELS_FIREWORKS,
        BUILTIN_MODELS_LOCAL,
        BUILTIN_MODELS_PERPLEXITY,
        BUILTIN_MODELS_COHERE,
        BUILTIN_MODELS_AI21,
        BUILTIN_MODELS_CEREBRAS,
        BUILTIN_MODELS_SAMBANOVA,
        BUILTIN_MODELS_XAI,
        BUILTIN_MODELS_HUGGINGFACE,
        BUILTIN_MODELS_REPLICATE,
        BUILTIN_MODELS_GITHUB_COPILOT,
        BUILTIN_MODELS_CHINESE,
        BUILTIN_MODELS_BEDROCK,
        BUILTIN_MODELS_CHATGPT,
        BUILTIN_MODELS_CLAUDE_CODE,
        BUILTIN_MODELS_CHUTES,
        BUILTIN_MODELS_VENICE,
    ];
    let mut models = Vec::new();
    for source in sources {
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
        assert_eq!(catalog.list_providers().len(), 39);
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
}
