//! Model catalog types — shared data structures for the model registry.

use serde::{Deserialize, Serialize};
use std::fmt;

// ---------------------------------------------------------------------------
// Canonical provider base URLs — single source of truth.
// Referenced by librefang-runtime drivers, model catalog, and embedding modules.
// ---------------------------------------------------------------------------

pub const ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";
pub const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
pub const GEMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com";
pub const DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com/v1";
pub const GROQ_BASE_URL: &str = "https://api.groq.com/openai/v1";
pub const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";
pub const MISTRAL_BASE_URL: &str = "https://api.mistral.ai/v1";
pub const TOGETHER_BASE_URL: &str = "https://api.together.xyz/v1";
pub const FIREWORKS_BASE_URL: &str = "https://api.fireworks.ai/inference/v1";
pub const OLLAMA_BASE_URL: &str = "http://localhost:11434/v1";
pub const VLLM_BASE_URL: &str = "http://localhost:8000/v1";
pub const LMSTUDIO_BASE_URL: &str = "http://localhost:1234/v1";
pub const LEMONADE_BASE_URL: &str = "http://localhost:8888/api/v1";
pub const PERPLEXITY_BASE_URL: &str = "https://api.perplexity.ai";
pub const COHERE_BASE_URL: &str = "https://api.cohere.com/v2";
pub const AI21_BASE_URL: &str = "https://api.ai21.com/studio/v1";
pub const CEREBRAS_BASE_URL: &str = "https://api.cerebras.ai/v1";
pub const SAMBANOVA_BASE_URL: &str = "https://api.sambanova.ai/v1";
pub const HUGGINGFACE_BASE_URL: &str = "https://api-inference.huggingface.co/v1";
pub const XAI_BASE_URL: &str = "https://api.x.ai/v1";
pub const REPLICATE_BASE_URL: &str = "https://api.replicate.com/v1";
pub const VENICE_BASE_URL: &str = "https://api.venice.ai/api/v1";

// ── GitHub Copilot ──────────────────────────────────────────────
pub const GITHUB_COPILOT_BASE_URL: &str = "https://api.githubcopilot.com";

// ── Chinese providers ─────────────────────────────────────────────
pub const QWEN_BASE_URL: &str = "https://dashscope.aliyuncs.com/compatible-mode/v1";
/// MiniMax China mainland (minimaxi.com)
pub const MINIMAX_CN_BASE_URL: &str = "https://api.minimaxi.com/v1";
/// MiniMax International (minimax.io)
pub const MINIMAX_INTL_BASE_URL: &str = "https://api.minimax.io/v1";
pub const ZHIPU_BASE_URL: &str = "https://open.bigmodel.cn/api/paas/v4";
pub const ZHIPU_CODING_BASE_URL: &str = "https://open.bigmodel.cn/api/coding/paas/v4";
/// Z.AI domain aliases (same API, different domain).
pub const ZAI_BASE_URL: &str = "https://api.z.ai/api/paas/v4";
pub const ZAI_CODING_BASE_URL: &str = "https://api.z.ai/api/coding/paas/v4";
pub const MOONSHOT_BASE_URL: &str = "https://api.moonshot.ai/v1";
pub const KIMI_CODING_BASE_URL: &str = "https://api.kimi.com/coding";
pub const QIANFAN_BASE_URL: &str = "https://qianfan.baidubce.com/v2";
pub const VOLCENGINE_BASE_URL: &str = "https://ark.cn-beijing.volces.com/api/v3";
pub const VOLCENGINE_CODING_BASE_URL: &str = "https://ark.cn-beijing.volces.com/api/coding/v3";

// ── Chutes.ai ────────────────────────────────────────────────────
pub const CHUTES_BASE_URL: &str = "https://llm.chutes.ai/v1";

// ── ChatGPT (Session Auth / Codex Responses API) ─────────────────
pub const CHATGPT_BASE_URL: &str = "https://chatgpt.com/backend-api";

// ── AWS Bedrock ───────────────────────────────────────────────────
pub const BEDROCK_BASE_URL: &str = "https://bedrock-runtime.us-east-1.amazonaws.com";

// ── Google Cloud Vertex AI ───────────────────────────────────────
pub const VERTEX_AI_BASE_URL: &str = "https://us-central1-aiplatform.googleapis.com";

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
    /// API key is present in the environment.
    Configured,
    /// API key is missing.
    #[default]
    Missing,
    /// No API key required (local providers).
    NotRequired,
}

impl fmt::Display for AuthStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuthStatus::Configured => write!(f, "configured"),
            AuthStatus::Missing => write!(f, "missing"),
            AuthStatus::NotRequired => write!(f, "not_required"),
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
    pub supports_tools: bool,
    /// Whether the model supports vision/image inputs.
    pub supports_vision: bool,
    /// Whether the model supports streaming responses.
    pub supports_streaming: bool,
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
            aliases: Vec::new(),
        }
    }
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
    pub aliases: std::collections::HashMap<String, String>,
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
        assert_eq!(AuthStatus::Missing.to_string(), "missing");
        assert_eq!(AuthStatus::NotRequired.to_string(), "not_required");
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
        };
        let info: ProviderInfo = toml_provider.into();
        assert_eq!(info.id, "anthropic");
        assert_eq!(info.auth_status, AuthStatus::Missing);
        assert_eq!(info.model_count, 0);
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
}
