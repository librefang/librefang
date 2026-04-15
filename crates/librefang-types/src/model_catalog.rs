//! Model catalog types — shared data structures for the model registry.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

/// A model's capability tier.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelTier {
    /// Cutting-edge, most capable models (e.g. Claude Opus, GPT-4.1).
    Frontier,
    /// Smart, cost-effective models (e.g. Claude Sonnet, Gemini 2.5 Flash).
    Smart,
    /// Balanced speed/cost models (e.g. GPT-4o-mini, Groq Llama).
    #[default]
    Balanced,
    /// Fastest, cheapest models for simple tasks.
    Fast,
    /// Local models (Ollama, vLLM, LM Studio).
    Local,
    /// User-defined custom models added at runtime.
    Custom,
}

impl fmt::Display for ModelTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModelTier::Frontier => write!(f, "frontier"),
            ModelTier::Smart => write!(f, "smart"),
            ModelTier::Balanced => write!(f, "balanced"),
            ModelTier::Fast => write!(f, "fast"),
            ModelTier::Local => write!(f, "local"),
            ModelTier::Custom => write!(f, "custom"),
        }
    }
}

/// Provider authentication status.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthStatus {
    /// API key is present and confirmed valid via a live API probe.
    ValidatedKey,
    /// API key is present (non-empty) but not yet validated.
    Configured,
    /// No API key, but a CLI tool (e.g. claude-code) is available as fallback.
    ConfiguredCli,
    /// Key detected via fallback env var — may not match the actual provider.
    /// Functionally usable but user should verify.
    AutoDetected,
    /// API key is present but was rejected by the provider (HTTP 401/403).
    InvalidKey,
    /// API key is missing.
    #[default]
    Missing,
    /// No API key required (local providers).
    NotRequired,
    /// CLI-based provider but CLI is not installed.
    CliNotInstalled,
}

impl AuthStatus {
    /// Returns true if the provider is usable (key or CLI available).
    ///
    /// `InvalidKey` returns false — the key exists but won't work.
    pub fn is_available(self) -> bool {
        matches!(
            self,
            AuthStatus::ValidatedKey
                | AuthStatus::Configured
                | AuthStatus::AutoDetected
                | AuthStatus::ConfiguredCli
                | AuthStatus::NotRequired
        )
    }
}

impl fmt::Display for AuthStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuthStatus::ValidatedKey => write!(f, "validated_key"),
            AuthStatus::Configured => write!(f, "configured"),
            AuthStatus::ConfiguredCli => write!(f, "configured_cli"),
            AuthStatus::AutoDetected => write!(f, "auto_detected"),
            AuthStatus::InvalidKey => write!(f, "invalid_key"),
            AuthStatus::Missing => write!(f, "missing"),
            AuthStatus::NotRequired => write!(f, "not_required"),
            AuthStatus::CliNotInstalled => write!(f, "cli_not_installed"),
        }
    }
}

/// A single model entry in the catalog.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCatalogEntry {
    /// Canonical model identifier (e.g. "claude-sonnet-4-20250514").
    pub id: String,
    /// Human-readable display name (e.g. "Claude Sonnet 4").
    pub display_name: String,
    /// Provider identifier (e.g. "anthropic").
    ///
    /// When omitted in community catalog files the provider is inferred from
    /// the `[provider].id` section during merge.
    #[serde(default)]
    pub provider: String,
    /// Capability tier.
    pub tier: ModelTier,
    /// Context window size in tokens.
    pub context_window: u64,
    /// Maximum output tokens.
    pub max_output_tokens: u64,
    /// Cost per million input tokens (USD).
    pub input_cost_per_m: f64,
    /// Cost per million output tokens (USD).
    pub output_cost_per_m: f64,
    /// Whether the model supports tool/function calling.
    #[serde(default)]
    pub supports_tools: bool,
    /// Whether the model supports vision/image inputs.
    #[serde(default)]
    pub supports_vision: bool,
    /// Whether the model supports streaming responses.
    #[serde(default)]
    pub supports_streaming: bool,
    /// Whether the model supports extended thinking / reasoning.
    #[serde(default)]
    pub supports_thinking: bool,
    /// Aliases for this model (e.g. ["sonnet", "claude-sonnet"]).
    #[serde(default)]
    pub aliases: Vec<String>,
}

impl Default for ModelCatalogEntry {
    fn default() -> Self {
        Self {
            id: String::new(),
            display_name: String::new(),
            provider: String::new(),
            tier: ModelTier::default(),
            context_window: 0,
            max_output_tokens: 0,
            input_cost_per_m: 0.0,
            output_cost_per_m: 0.0,
            supports_tools: false,
            supports_vision: false,
            supports_streaming: false,
            supports_thinking: false,
            aliases: Vec::new(),
        }
    }
}

/// Per-region endpoint configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionConfig {
    /// Region-specific base URL.
    pub base_url: String,
    /// Optional override for the API key environment variable.
    /// When absent the provider-level `api_key_env` is used.
    #[serde(default)]
    pub api_key_env: Option<String>,
}

/// Provider metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderInfo {
    /// Provider identifier (e.g. "anthropic").
    pub id: String,
    /// Human-readable display name (e.g. "Anthropic").
    pub display_name: String,
    /// Environment variable name for the API key.
    pub api_key_env: String,
    /// Default base URL.
    pub base_url: String,
    /// Whether an API key is required (false for local providers).
    pub key_required: bool,
    /// Runtime-detected authentication status.
    pub auth_status: AuthStatus,
    /// Number of models from this provider in the catalog.
    pub model_count: usize,
    /// URL where users can sign up and get an API key.
    pub signup_url: Option<String>,
    /// Regional endpoint overrides (region name → config).
    /// e.g. `[provider.regions.us]` with `base_url = "https://..."`.
    #[serde(default)]
    pub regions: HashMap<String, RegionConfig>,
    /// Media capabilities supported by this provider (e.g. "image_generation", "text_to_speech").
    /// Populated from `providers/*.toml` in the registry.
    #[serde(default)]
    pub media_capabilities: Vec<String>,
    /// Model IDs confirmed available via live API probe.
    /// Empty until background validation completes successfully.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available_models: Vec<String>,
    /// True when the provider was added at runtime by the user (via the
    /// dashboard "Add provider" flow), false when it was shipped by the
    /// librefang-registry. Drives whether the dashboard shows a real
    /// "Delete" control — built-in providers can only be deconfigured
    /// (key removed), not deleted, because the registry sync would
    /// re-create their TOML on the next boot anyway.
    #[serde(default)]
    pub is_custom: bool,
}

impl Default for ProviderInfo {
    fn default() -> Self {
        Self {
            id: String::new(),
            display_name: String::new(),
            api_key_env: String::new(),
            base_url: String::new(),
            key_required: true,
            auth_status: AuthStatus::default(),
            model_count: 0,
            signup_url: None,
            regions: HashMap::new(),
            media_capabilities: Vec::new(),
            available_models: Vec::new(),
            is_custom: false,
        }
    }
}

/// Provider metadata as stored in TOML catalog files.
///
/// Unlike [`ProviderInfo`], this struct omits runtime-only fields (`auth_status`,
/// `model_count`) so it maps 1:1 to the `[provider]` section in community catalog
/// files at `providers/<name>.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCatalogToml {
    /// Provider identifier (e.g. "anthropic").
    pub id: String,
    /// Human-readable display name (e.g. "Anthropic").
    pub display_name: String,
    /// Environment variable name for the API key.
    pub api_key_env: String,
    /// Default base URL.
    pub base_url: String,
    /// Whether an API key is required (false for local providers).
    #[serde(default = "default_key_required")]
    pub key_required: bool,
    /// URL where users can sign up and get an API key.
    #[serde(default)]
    pub signup_url: Option<String>,
    /// Regional endpoint overrides (region name → config).
    /// e.g. `[provider.regions.us]` with `base_url = "https://..."`.
    #[serde(default)]
    pub regions: HashMap<String, RegionConfig>,
    /// Media capabilities supported by this provider (e.g. "image_generation", "text_to_speech").
    #[serde(default)]
    pub media_capabilities: Vec<String>,
}

fn default_key_required() -> bool {
    true
}

impl From<ProviderCatalogToml> for ProviderInfo {
    fn from(p: ProviderCatalogToml) -> Self {
        Self {
            id: p.id,
            display_name: p.display_name,
            api_key_env: p.api_key_env,
            base_url: p.base_url,
            key_required: p.key_required,
            auth_status: AuthStatus::default(),
            model_count: 0,
            signup_url: p.signup_url,
            regions: p.regions,
            media_capabilities: p.media_capabilities,
            available_models: Vec::new(),
            // Populated by the runtime catalog loader (classifies based on
            // whether the file is also present in registry/providers/).
            is_custom: false,
        }
    }
}

/// A catalog file that can contain an optional `[provider]` section and a
/// `[[models]]` array. This is the unified format shared between the main
/// repository (`catalog/providers/*.toml`) and the community model-catalog
/// repository (`providers/*.toml`).
///
/// # TOML format
///
/// ```toml
/// [provider]
/// id = "anthropic"
/// display_name = "Anthropic"
/// api_key_env = "ANTHROPIC_API_KEY"
/// base_url = "https://api.anthropic.com"
/// key_required = true
///
/// [[models]]
/// id = "claude-sonnet-4-20250514"
/// display_name = "Claude Sonnet 4"
/// provider = "anthropic"
/// tier = "smart"
/// context_window = 200000
/// max_output_tokens = 64000
/// input_cost_per_m = 3.0
/// output_cost_per_m = 15.0
/// supports_tools = true
/// supports_vision = true
/// supports_streaming = true
/// aliases = ["sonnet", "claude-sonnet"]
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCatalogFile {
    /// Optional provider metadata (present in community catalog files).
    pub provider: Option<ProviderCatalogToml>,
    /// Model entries.
    #[serde(default)]
    pub models: Vec<ModelCatalogEntry>,
}

/// A catalog-level aliases file mapping short names to canonical model IDs.
///
/// # TOML format
///
/// ```toml
/// [aliases]
/// sonnet = "claude-sonnet-4-20250514"
/// haiku = "claude-haiku-4-5-20251001"
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AliasesCatalogFile {
    /// Alias -> canonical model ID mappings.
    #[serde(default)]
    pub aliases: HashMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_tier_display() {
        assert_eq!(ModelTier::Frontier.to_string(), "frontier");
        assert_eq!(ModelTier::Smart.to_string(), "smart");
        assert_eq!(ModelTier::Balanced.to_string(), "balanced");
        assert_eq!(ModelTier::Fast.to_string(), "fast");
        assert_eq!(ModelTier::Local.to_string(), "local");
        assert_eq!(ModelTier::Custom.to_string(), "custom");
    }

    #[test]
    fn test_auth_status_display() {
        assert_eq!(AuthStatus::Configured.to_string(), "configured");
        assert_eq!(AuthStatus::ConfiguredCli.to_string(), "configured_cli");
        assert_eq!(AuthStatus::Missing.to_string(), "missing");
        assert_eq!(AuthStatus::NotRequired.to_string(), "not_required");
        assert_eq!(AuthStatus::CliNotInstalled.to_string(), "cli_not_installed");
    }

    #[test]
    fn test_model_tier_default() {
        assert_eq!(ModelTier::default(), ModelTier::Balanced);
    }

    #[test]
    fn test_auth_status_default() {
        assert_eq!(AuthStatus::default(), AuthStatus::Missing);
    }

    #[test]
    fn test_model_catalog_entry_default() {
        let entry = ModelCatalogEntry::default();
        assert!(entry.id.is_empty());
        assert_eq!(entry.tier, ModelTier::Balanced);
        assert!(entry.aliases.is_empty());
    }

    #[test]
    fn test_provider_info_default() {
        let info = ProviderInfo::default();
        assert!(info.id.is_empty());
        assert!(info.key_required);
        assert_eq!(info.auth_status, AuthStatus::Missing);
    }

    #[test]
    fn test_model_tier_serde_roundtrip() {
        let tier = ModelTier::Frontier;
        let json = serde_json::to_string(&tier).unwrap();
        assert_eq!(json, "\"frontier\"");
        let parsed: ModelTier = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, tier);
    }

    #[test]
    fn test_auth_status_serde_roundtrip() {
        let status = AuthStatus::Configured;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"configured\"");
        let parsed: AuthStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, status);
    }

    #[test]
    fn test_model_entry_serde_roundtrip() {
        let entry = ModelCatalogEntry {
            id: "claude-sonnet-4-20250514".to_string(),
            display_name: "Claude Sonnet 4".to_string(),
            provider: "anthropic".to_string(),
            tier: ModelTier::Smart,
            context_window: 200_000,
            max_output_tokens: 64_000,
            input_cost_per_m: 3.0,
            output_cost_per_m: 15.0,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            supports_thinking: true,
            aliases: vec!["sonnet".to_string(), "claude-sonnet".to_string()],
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: ModelCatalogEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, entry.id);
        assert_eq!(parsed.tier, ModelTier::Smart);
        assert_eq!(parsed.aliases.len(), 2);
    }

    #[test]
    fn test_provider_info_serde_roundtrip() {
        let info = ProviderInfo {
            id: "anthropic".to_string(),
            display_name: "Anthropic".to_string(),
            api_key_env: "ANTHROPIC_API_KEY".to_string(),
            base_url: "https://api.anthropic.com".to_string(),
            key_required: true,
            auth_status: AuthStatus::Configured,
            model_count: 3,
            signup_url: None,
            regions: HashMap::new(),
            media_capabilities: Vec::new(),
            available_models: Vec::new(),
            is_custom: false,
        };
        let json = serde_json::to_string(&info).unwrap();
        let parsed: ProviderInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "anthropic");
        assert_eq!(parsed.auth_status, AuthStatus::Configured);
        assert_eq!(parsed.model_count, 3);
    }

    #[test]
    fn test_model_catalog_file_with_provider() {
        let toml_str = r#"
[provider]
id = "anthropic"
display_name = "Anthropic"
api_key_env = "ANTHROPIC_API_KEY"
base_url = "https://api.anthropic.com"
key_required = true

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
aliases = ["sonnet", "claude-sonnet"]
"#;
        let file: ModelCatalogFile = toml::from_str(toml_str).unwrap();
        assert!(file.provider.is_some());
        let p = file.provider.unwrap();
        assert_eq!(p.id, "anthropic");
        assert_eq!(p.base_url, "https://api.anthropic.com");
        assert!(p.key_required);
        assert_eq!(file.models.len(), 1);
        assert_eq!(file.models[0].id, "claude-sonnet-4-20250514");
        assert_eq!(file.models[0].tier, ModelTier::Smart);
    }

    #[test]
    fn test_model_catalog_file_without_provider() {
        let toml_str = r#"
[[models]]
id = "gpt-4o"
display_name = "GPT-4o"
provider = "openai"
tier = "smart"
context_window = 128000
max_output_tokens = 16384
input_cost_per_m = 2.5
output_cost_per_m = 10.0
supports_tools = true
supports_vision = true
supports_streaming = true
aliases = []
"#;
        let file: ModelCatalogFile = toml::from_str(toml_str).unwrap();
        assert!(file.provider.is_none());
        assert_eq!(file.models.len(), 1);
    }

    #[test]
    fn test_provider_catalog_toml_to_provider_info() {
        let toml_provider = ProviderCatalogToml {
            id: "anthropic".to_string(),
            display_name: "Anthropic".to_string(),
            api_key_env: "ANTHROPIC_API_KEY".to_string(),
            base_url: "https://api.anthropic.com".to_string(),
            key_required: true,
            signup_url: Some("https://console.anthropic.com/settings/keys".to_string()),
            regions: HashMap::new(),
            media_capabilities: Vec::new(),
        };
        let info: ProviderInfo = toml_provider.into();
        assert_eq!(info.id, "anthropic");
        assert_eq!(info.auth_status, AuthStatus::Missing);
        assert_eq!(info.model_count, 0);
        assert!(info.regions.is_empty());
    }

    #[test]
    fn test_aliases_catalog_file() {
        let toml_str = r#"
[aliases]
sonnet = "claude-sonnet-4-20250514"
haiku = "claude-haiku-4-5-20251001"
"#;
        let file: AliasesCatalogFile = toml::from_str(toml_str).unwrap();
        assert_eq!(file.aliases.len(), 2);
        assert_eq!(file.aliases["sonnet"], "claude-sonnet-4-20250514");
    }

    #[test]
    fn test_provider_regions_toml_parse() {
        let toml_str = r#"
[provider]
id = "qwen"
display_name = "Qwen (DashScope)"
api_key_env = "DASHSCOPE_API_KEY"
base_url = "https://dashscope.aliyuncs.com/compatible-mode/v1"
key_required = true

[provider.regions.intl]
base_url = "https://dashscope-intl.aliyuncs.com/compatible-mode/v1"

[provider.regions.us]
base_url = "https://dashscope-us.aliyuncs.com/compatible-mode/v1"

[[models]]
id = "qwen3-235b-a22b"
display_name = "Qwen3 235B"
provider = "qwen"
tier = "frontier"
context_window = 131072
max_output_tokens = 8192
input_cost_per_m = 2.0
output_cost_per_m = 8.0
supports_tools = true
supports_vision = false
supports_streaming = true
aliases = []
"#;
        let file: ModelCatalogFile = toml::from_str(toml_str).unwrap();
        let provider = file.provider.unwrap();
        assert_eq!(provider.id, "qwen");
        assert_eq!(provider.regions.len(), 2);
        assert_eq!(
            provider.regions["intl"].base_url,
            "https://dashscope-intl.aliyuncs.com/compatible-mode/v1"
        );
        assert_eq!(
            provider.regions["us"].base_url,
            "https://dashscope-us.aliyuncs.com/compatible-mode/v1"
        );
        // intl region has no api_key_env override
        assert!(provider.regions["intl"].api_key_env.is_none());

        // Verify conversion to ProviderInfo preserves regions
        let info: ProviderInfo = provider.into();
        assert_eq!(info.regions.len(), 2);
        assert_eq!(
            info.regions["us"].base_url,
            "https://dashscope-us.aliyuncs.com/compatible-mode/v1"
        );
    }

    #[test]
    fn test_provider_without_regions_defaults_empty() {
        let toml_str = r#"
[provider]
id = "anthropic"
display_name = "Anthropic"
api_key_env = "ANTHROPIC_API_KEY"
base_url = "https://api.anthropic.com"
key_required = true

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
        let file: ModelCatalogFile = toml::from_str(toml_str).unwrap();
        let provider = file.provider.unwrap();
        assert!(
            provider.regions.is_empty(),
            "Provider without [provider.regions] should have empty regions map"
        );
    }

    #[test]
    fn test_region_selection_overrides_base_url() {
        let provider = ProviderInfo {
            id: "qwen".to_string(),
            display_name: "Qwen".to_string(),
            api_key_env: "DASHSCOPE_API_KEY".to_string(),
            base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".to_string(),
            key_required: true,
            auth_status: AuthStatus::default(),
            model_count: 0,
            signup_url: None,
            regions: HashMap::from([
                (
                    "intl".to_string(),
                    RegionConfig {
                        base_url: "https://dashscope-intl.aliyuncs.com/compatible-mode/v1"
                            .to_string(),
                        api_key_env: None,
                    },
                ),
                (
                    "us".to_string(),
                    RegionConfig {
                        base_url: "https://dashscope-us.aliyuncs.com/compatible-mode/v1"
                            .to_string(),
                        api_key_env: None,
                    },
                ),
            ]),
            media_capabilities: Vec::new(),
            available_models: Vec::new(),
            is_custom: false,
        };

        // Simulate region selection: if user picks "us", use that region's base_url
        let selected_region = "us";
        let resolved_url = provider
            .regions
            .get(selected_region)
            .map(|r| r.base_url.as_str())
            .unwrap_or(&provider.base_url);
        assert_eq!(
            resolved_url,
            "https://dashscope-us.aliyuncs.com/compatible-mode/v1"
        );

        // Default when no region selected: use base_url
        let no_region: Option<&str> = None;
        let resolved_default = no_region
            .and_then(|r| provider.regions.get(r))
            .map(|r| r.base_url.as_str())
            .unwrap_or(&provider.base_url);
        assert_eq!(
            resolved_default,
            "https://dashscope.aliyuncs.com/compatible-mode/v1"
        );
    }
}
