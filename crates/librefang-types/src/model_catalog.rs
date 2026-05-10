//! Model catalog types — shared data structures for the model registry.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

/// A model's capability tier.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
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
#[non_exhaustive]
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
    /// Local provider was probed and found offline (port not listening).
    /// Unlike `Missing`, `detect_auth()` will not reset this — the probe
    /// owns the transition back to `NotRequired` when the service comes up.
    LocalOffline,
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
            AuthStatus::LocalOffline => write!(f, "local_offline"),
        }
    }
}

/// Model modality — what kind of output the model produces.
///
/// Mirrors the `modality` field in the librefang-registry schema. Text models
/// follow the usual chat/completion flow (context_window + max_output_tokens
/// are required). Image, audio, video, and music models are priced per-call
/// or per-asset but have no conventional context window, so their
/// `context_window` / `max_output_tokens` fields may be zero/absent in the
/// catalog TOML.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Modality {
    /// Chat / completion / reasoning model. Default when the field is absent.
    #[default]
    Text,
    /// Image-generation model (e.g. OpenAI gpt-image-2).
    Image,
    /// Speech / audio model (TTS, STT).
    Audio,
    /// Video-generation model (e.g. ByteDance Seedance, MiniMax Hailuo).
    Video,
    /// Music / lyrics generation model.
    Music,
}

impl fmt::Display for Modality {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Modality::Text => write!(f, "text"),
            Modality::Image => write!(f, "image"),
            Modality::Audio => write!(f, "audio"),
            Modality::Video => write!(f, "video"),
            Modality::Music => write!(f, "music"),
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
    /// Model modality. Defaults to `Text` when absent in the catalog TOML.
    #[serde(default)]
    pub modality: Modality,
    /// Context window size in tokens. `0` or absent means "unknown / not
    /// applicable" — image and audio models in the registry omit this field.
    /// Consumers MUST treat `0` as unknown and supply their own default;
    /// never propagate `0` into compaction thresholds or budget math.
    #[serde(default)]
    pub context_window: u64,
    /// Maximum output tokens. `0` or absent means "unknown / not applicable".
    /// Same handling rule as `context_window`: do not feed `0` into
    /// downstream calculations.
    #[serde(default)]
    pub max_output_tokens: u64,
    /// Cost per million input tokens (USD) — text tokens for image/audio models.
    pub input_cost_per_m: f64,
    /// Cost per million output tokens (USD) — text tokens for image/audio models.
    pub output_cost_per_m: f64,
    /// Cost per million image input tokens (USD). Only set for image/multimodal
    /// models where image pixels are priced separately from text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_input_cost_per_m: Option<f64>,
    /// Cost per million image output tokens (USD). Only set for image-generation
    /// models.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_output_cost_per_m: Option<f64>,
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
    /// How the OpenAI-compatible driver must handle the `reasoning_content`
    /// field on historical assistant turns. Sourced from the registry per
    /// model so the driver doesn't have to encode this in substring matches.
    /// See [`ReasoningEchoPolicy`] for the four cases.
    #[serde(default)]
    pub reasoning_echo_policy: ReasoningEchoPolicy,
    /// Aliases for this model (e.g. ["sonnet", "claude-sonnet"]).
    #[serde(default)]
    pub aliases: Vec<String>,
}

/// How the OpenAI-compatible driver must handle the `reasoning_content`
/// field on historical assistant turns for a given model.
///
/// The OpenAI-compat ecosystem has at least three incompatible conventions
/// here. Encoding the choice as catalog metadata lets the driver resolve
/// the correct behaviour by lookup instead of substring-matching the model
/// name. The variants:
///
/// * [`Self::None`] — the field is omitted on history (default; most
///   providers reject the unknown field).
/// * [`Self::Strip`] — historical `reasoning_content` MUST be stripped from
///   request payloads. DeepSeek-R1 / `deepseek-reasoner` is the canonical
///   case: the API rejects requests carrying `reasoning_content` from a
///   previous assistant turn.
/// * [`Self::Echo`] — the original thinking text MUST be echoed back on
///   assistant turns containing `tool_calls`, otherwise the API returns
///   400. DeepSeek V4 Flash (thinking-mode-on) requires this — see
///   librefang/librefang#4842.
/// * [`Self::EmptyString`] — the field must be present (empty string) on
///   `tool_calls` turns, with thinking disabled wire-side. Moonshot / Kimi
///   K2 family.
///
/// Drivers that don't speak the OpenAI-compatible chat-completions wire
/// format (Anthropic, Gemini, etc.) ignore this entirely.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ReasoningEchoPolicy {
    /// No `reasoning_content` field on historical assistant turns (default).
    #[default]
    None,
    /// Strip historical `reasoning_content` (DeepSeek R1 / reasoner).
    Strip,
    /// Echo the original thinking text on `tool_calls` turns
    /// (DeepSeek V4 Flash with thinking mode on).
    Echo,
    /// Send empty-string `reasoning_content` on `tool_calls` turns plus
    /// disable thinking wire-side (Moonshot / Kimi K2 family).
    EmptyString,
}

impl ModelCatalogEntry {
    /// Returns true if this entry is an image-generation model.
    pub fn is_image_generation(&self) -> bool {
        self.modality == Modality::Image
    }

    /// Modality-aware schema check applied after TOML deserialization.
    ///
    /// `context_window` and `max_output_tokens` use `#[serde(default)]` so
    /// image and audio entries (which don't have a token context) can omit
    /// the fields. Without this check, a malformed `Modality::Text` entry
    /// missing those fields would silently load with `0` and propagate that
    /// `0` into compaction thresholds and budget math downstream. Catalog
    /// loaders MUST call this and reject entries that fail.
    pub fn validate(&self) -> Result<(), String> {
        if self.modality == Modality::Text {
            if self.context_window == 0 {
                return Err(format!(
                    "text model {}/{} is missing context_window",
                    self.provider, self.id
                ));
            }
            if self.max_output_tokens == 0 {
                return Err(format!(
                    "text model {}/{} is missing max_output_tokens",
                    self.provider, self.id
                ));
            }
        }
        Ok(())
    }
}

impl Default for ModelCatalogEntry {
    fn default() -> Self {
        Self {
            id: String::new(),
            display_name: String::new(),
            provider: String::new(),
            tier: ModelTier::default(),
            modality: Modality::default(),
            context_window: 0,
            max_output_tokens: 0,
            input_cost_per_m: 0.0,
            output_cost_per_m: 0.0,
            image_input_cost_per_m: None,
            image_output_cost_per_m: None,
            supports_tools: false,
            supports_vision: false,
            supports_streaming: false,
            supports_thinking: false,
            reasoning_echo_policy: ReasoningEchoPolicy::default(),
            aliases: Vec::new(),
        }
    }
}

/// Model type classification.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum ModelType {
    /// Conversational / text generation model.
    #[default]
    Chat,
    /// Speech / audio model (TTS, STT).
    Speech,
    /// Embedding / vector model.
    Embedding,
}

impl fmt::Display for ModelType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModelType::Chat => write!(f, "chat"),
            ModelType::Speech => write!(f, "speech"),
            ModelType::Embedding => write!(f, "embedding"),
        }
    }
}

/// Per-model inference parameter overrides.
///
/// Each field is `Option` — `None` means "use the agent's or system default".
/// These overrides are applied as a fallback layer: agent-level `ModelConfig`
/// takes precedence, then model overrides, then system defaults.
///
/// Persisted to `~/.librefang/model_overrides.json` keyed by `provider:model_id`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelOverrides {
    /// Model type classification.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_type: Option<ModelType>,
    /// Sampling temperature (0.0–2.0).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// Top-p / nucleus sampling (0.0–1.0).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    /// Maximum tokens for completion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Frequency penalty (-2.0–2.0).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f32>,
    /// Presence penalty (-2.0–2.0).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f32>,
    /// Reasoning effort level ("low", "medium", "high").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    /// Use `max_completion_tokens` instead of `max_tokens` in API requests.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_max_completion_tokens: Option<bool>,
    /// Model does NOT support a system role message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_system_role: Option<bool>,
    /// Force the max_tokens parameter even when the provider doesn't require it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub force_max_tokens: Option<bool>,
    /// User override for `supports_tools`. `None` defers to the catalog entry's
    /// own value; `Some(true|false)` forces capability on/off regardless of
    /// what the provider's catalog declares (refs #4745). Useful when a
    /// provider's `capabilities` field is wrong, missing, or non-standard.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_tools: Option<bool>,
    /// User override for `supports_vision`. See [`Self::supports_tools`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_vision: Option<bool>,
    /// User override for `supports_streaming`. See [`Self::supports_tools`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_streaming: Option<bool>,
    /// User override for `supports_thinking`. See [`Self::supports_tools`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_thinking: Option<bool>,
}

impl ModelOverrides {
    /// Returns true if all fields are `None` (no overrides set).
    pub fn is_empty(&self) -> bool {
        self.model_type.is_none()
            && self.temperature.is_none()
            && self.top_p.is_none()
            && self.max_tokens.is_none()
            && self.frequency_penalty.is_none()
            && self.presence_penalty.is_none()
            && self.reasoning_effort.is_none()
            && self.use_max_completion_tokens.is_none()
            && self.no_system_role.is_none()
            && self.force_max_tokens.is_none()
            && self.supports_tools.is_none()
            && self.supports_vision.is_none()
            && self.supports_streaming.is_none()
            && self.supports_thinking.is_none()
    }
}

/// Effective capabilities for a model after applying user overrides on top of
/// the catalog entry's declared capabilities. Returned by
/// `ModelCatalog::effective_capabilities` and consumed by callers that gate
/// runtime behaviour (tool gating, vision input validation, …) on capability
/// truth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectiveCapabilities {
    pub supports_tools: bool,
    pub supports_vision: bool,
    pub supports_streaming: bool,
    pub supports_thinking: bool,
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
    /// Per-provider proxy URL override. When set, API calls to this provider
    /// are routed through this proxy instead of the global proxy config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_url: Option<String>,
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
            proxy_url: None,
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
            proxy_url: None,
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
        assert_eq!(AuthStatus::AutoDetected.to_string(), "auto_detected");
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
    fn test_validate_text_requires_nonzero_limits() {
        // A text entry parsed from TOML that omitted both fields would
        // land here with zeros — validate() must reject it so callers
        // never propagate `0` into compaction / budget math.
        let entry = ModelCatalogEntry {
            id: "gpt-x".into(),
            provider: "openai".into(),
            modality: Modality::Text,
            ..Default::default()
        };
        let err = entry.validate().unwrap_err();
        assert!(err.contains("context_window"), "got: {err}");

        // max_output_tokens missing while context_window is set still fails.
        let partial = ModelCatalogEntry {
            id: "gpt-x".into(),
            provider: "openai".into(),
            modality: Modality::Text,
            context_window: 200_000,
            ..Default::default()
        };
        let err2 = partial.validate().unwrap_err();
        assert!(err2.contains("max_output_tokens"), "got: {err2}");

        // Both populated → ok.
        let ok = ModelCatalogEntry {
            id: "gpt-x".into(),
            provider: "openai".into(),
            modality: Modality::Text,
            context_window: 200_000,
            max_output_tokens: 8_192,
            ..Default::default()
        };
        assert!(ok.validate().is_ok());
    }

    #[test]
    fn test_validate_image_models_skip_token_check() {
        // Image entries legitimately omit context_window / max_output_tokens
        // — validate() must not require them.
        let img = ModelCatalogEntry {
            id: "dall-e-3".into(),
            provider: "openai".into(),
            modality: Modality::Image,
            ..Default::default()
        };
        assert!(img.validate().is_ok());

        let audio = ModelCatalogEntry {
            id: "whisper-1".into(),
            provider: "openai".into(),
            modality: Modality::Audio,
            ..Default::default()
        };
        assert!(audio.validate().is_ok());
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
        // Pure serde round-trip — field values are placeholders so the
        // assertions don't track whichever Sonnet / GPT id is canonical
        // in the registry this week.
        let entry = ModelCatalogEntry {
            id: "canonical-id-one".to_string(),
            display_name: "Display Name One".to_string(),
            provider: "test-provider".to_string(),
            tier: ModelTier::Smart,
            context_window: 200_000,
            max_output_tokens: 64_000,
            input_cost_per_m: 3.0,
            output_cost_per_m: 15.0,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            supports_thinking: true,
            aliases: vec!["short-alias".to_string(), "other-alias".to_string()],
            ..Default::default()
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: ModelCatalogEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, entry.id);
        assert_eq!(parsed.tier, ModelTier::Smart);
        assert_eq!(parsed.aliases.len(), 2);
    }

    #[test]
    fn test_image_generation_model_parses_without_context_window() {
        // gpt-image-2 style entry: no context_window / max_output_tokens, has
        // modality + image cost fields. Before the Modality + #[serde(default)]
        // changes this panicked with "missing field `context_window`" and the
        // whole providers/openai.toml would fail to parse, silently dropping
        // every OpenAI model.
        let toml_str = r#"
id = "gpt-image-2"
display_name = "GPT Image 2"
tier = "frontier"
modality = "image"
input_cost_per_m = 5.00
output_cost_per_m = 10.00
image_input_cost_per_m = 8.00
image_output_cost_per_m = 30.00
supports_tools = false
supports_vision = true
supports_streaming = false
aliases = ["gpt-image-2-2026-04-21"]
"#;
        let entry: ModelCatalogEntry = toml::from_str(toml_str).expect("parse image model");
        assert_eq!(entry.modality, Modality::Image);
        assert!(entry.is_image_generation());
        assert_eq!(entry.context_window, 0);
        assert_eq!(entry.max_output_tokens, 0);
        assert_eq!(entry.image_input_cost_per_m, Some(8.0));
        assert_eq!(entry.image_output_cost_per_m, Some(30.0));
    }

    #[test]
    fn test_text_model_defaults_to_text_modality() {
        let toml_str = r#"
id = "gpt-4.1"
display_name = "GPT-4.1"
tier = "frontier"
context_window = 1047576
max_output_tokens = 32768
input_cost_per_m = 2.0
output_cost_per_m = 8.0
"#;
        let entry: ModelCatalogEntry = toml::from_str(toml_str).expect("parse text model");
        assert_eq!(entry.modality, Modality::Text);
        assert!(!entry.is_image_generation());
        assert!(entry.image_input_cost_per_m.is_none());
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
            proxy_url: None,
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
id = "canonical-id-one"
display_name = "Canonical Model One"
provider = "anthropic"
tier = "smart"
context_window = 200000
max_output_tokens = 64000
input_cost_per_m = 3.0
output_cost_per_m = 15.0
supports_tools = true
supports_vision = true
supports_streaming = true
aliases = ["short-alias", "other-alias"]
"#;
        let file: ModelCatalogFile = toml::from_str(toml_str).unwrap();
        assert!(file.provider.is_some());
        let p = file.provider.unwrap();
        assert_eq!(p.id, "anthropic");
        assert_eq!(p.base_url, "https://api.anthropic.com");
        assert!(p.key_required);
        assert_eq!(file.models.len(), 1);
        assert_eq!(file.models[0].id, "canonical-id-one");
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
        // Pure parser test — alias names and target ids are placeholders so
        // the assertions don't track whichever Sonnet / Haiku id is canonical
        // in the registry this week.
        let toml_str = r#"
[aliases]
my-alias = "canonical-target-one"
other-alias = "canonical-target-two"
"#;
        let file: AliasesCatalogFile = toml::from_str(toml_str).unwrap();
        assert_eq!(file.aliases.len(), 2);
        assert_eq!(file.aliases["my-alias"], "canonical-target-one");
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
id = "canonical-id-one"
display_name = "Canonical Model One"
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
            proxy_url: None,
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

    // ----- ReasoningEchoPolicy serde tests (#4842) -----

    #[test]
    fn test_reasoning_echo_policy_serializes_snake_case() {
        // Verify wire-compatibility with the registry schema (#4842 registry PR)
        // which lists options as `["none", "strip", "echo", "empty_string"]`.
        assert_eq!(
            serde_json::to_string(&ReasoningEchoPolicy::None).unwrap(),
            r#""none""#
        );
        assert_eq!(
            serde_json::to_string(&ReasoningEchoPolicy::Strip).unwrap(),
            r#""strip""#
        );
        assert_eq!(
            serde_json::to_string(&ReasoningEchoPolicy::Echo).unwrap(),
            r#""echo""#
        );
        assert_eq!(
            serde_json::to_string(&ReasoningEchoPolicy::EmptyString).unwrap(),
            r#""empty_string""#
        );
    }

    #[test]
    fn test_reasoning_echo_policy_deserializes_snake_case() {
        assert_eq!(
            serde_json::from_str::<ReasoningEchoPolicy>(r#""none""#).unwrap(),
            ReasoningEchoPolicy::None
        );
        assert_eq!(
            serde_json::from_str::<ReasoningEchoPolicy>(r#""strip""#).unwrap(),
            ReasoningEchoPolicy::Strip
        );
        assert_eq!(
            serde_json::from_str::<ReasoningEchoPolicy>(r#""echo""#).unwrap(),
            ReasoningEchoPolicy::Echo
        );
        assert_eq!(
            serde_json::from_str::<ReasoningEchoPolicy>(r#""empty_string""#).unwrap(),
            ReasoningEchoPolicy::EmptyString
        );
    }

    #[test]
    fn test_reasoning_echo_policy_default_is_none() {
        assert_eq!(
            ReasoningEchoPolicy::default(),
            ReasoningEchoPolicy::None,
            "default policy must be None so unmarked catalog entries don't \
             accidentally enable provider-specific behaviour"
        );
    }

    #[test]
    fn test_model_catalog_entry_parses_reasoning_echo_policy_from_toml() {
        // Mirrors what the registry consumer reads from
        // `providers/deepseek.toml` after the registry PR lands.
        let toml_str = r#"
            id = "deepseek-v4-flash"
            display_name = "DeepSeek V4 Flash"
            tier = "smart"
            context_window = 1000000
            max_output_tokens = 384000
            input_cost_per_m = 0.14
            output_cost_per_m = 0.28
            supports_thinking = true
            reasoning_echo_policy = "echo"
        "#;
        let entry: ModelCatalogEntry = toml::from_str(toml_str).expect("valid toml");
        assert_eq!(entry.reasoning_echo_policy, ReasoningEchoPolicy::Echo);
    }

    #[test]
    fn test_model_catalog_entry_defaults_reasoning_echo_policy_when_absent() {
        // Backwards compat: catalogs from older registry releases do not
        // carry the field. They must keep parsing and default to None.
        let toml_str = r#"
            id = "deepseek-chat"
            display_name = "DeepSeek V3"
            tier = "smart"
            context_window = 64000
            max_output_tokens = 8192
            input_cost_per_m = 0.32
            output_cost_per_m = 0.89
        "#;
        let entry: ModelCatalogEntry = toml::from_str(toml_str).expect("valid toml");
        assert_eq!(entry.reasoning_echo_policy, ReasoningEchoPolicy::None);
    }
}
