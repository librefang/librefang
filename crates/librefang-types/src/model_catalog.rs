//! Model catalog types — shared data structures for the model registry.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

fn default_true() -> bool {
    true
}

/// A model's capability tier.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
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

impl<'de> Deserialize<'de> for ModelTier {
    /// Deserialize leniently: an unrecognized tier string maps to
    /// [`ModelTier::Custom`] rather than failing the whole file.
    ///
    /// Catalog files are loaded one-per-provider, and a single hard parse
    /// error makes the entire provider vanish (see `from_sources` /
    /// `load_catalog_file`). A dashboard or hand-edited file carrying an
    /// out-of-vocabulary tier (e.g. `tier = "reasoning"`, #5822) must not
    /// take the provider down with it — treat the unknown label as a custom
    /// tier and keep the provider usable.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Ok(match raw.trim().to_ascii_lowercase().as_str() {
            "frontier" => ModelTier::Frontier,
            "smart" => ModelTier::Smart,
            "balanced" => ModelTier::Balanced,
            "fast" => ModelTier::Fast,
            "local" => ModelTier::Local,
            // "custom" and anything unrecognized collapse to Custom.
            _ => ModelTier::Custom,
        })
    }
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
    /// [`Self::limits_known`] distinguishes the two readings of `0`.
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
    /// Whether text-token pricing is known.
    ///
    /// Older registry entries predate this field and carry explicit prices, so a missing value defaults to true.
    #[serde(default = "default_true")]
    pub pricing_known: bool,
    /// Whether `context_window` / `max_output_tokens` above came from a source.
    ///
    /// `true` — a curated registry entry, an operator override, or the endpoint itself supplied the numbers, so a consumer may treat them as a real ceiling.
    /// `false` — no source supplied them: whatever value is present is a LibreFang-chosen default, and it MUST NOT be packed against, clamped against, or presented to an operator as a discovered fact.
    ///
    /// The flag records provenance, which the number alone cannot.
    /// An image or audio entry legitimately has no token context and carries `context_window: 0` with `limits_known: true` — the limits are known to be inapplicable, which is not the same as unknown.
    /// A model discovered behind an OpenAI-compatible gateway that reports no capacity carries `limits_known: false`; that path additionally zeroes both fields so the existing `> 0` guards in the compaction and budget math fall through to their conservative default instead of packing a prompt against a guess (#7780).
    ///
    /// Older registry entries predate this field and carry real numbers, so a missing value defaults to true — the same convention `pricing_known` uses.
    #[serde(default = "default_true")]
    pub limits_known: bool,
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
///   previous assistant turn. The variant *also* implies "force a non-null
///   `content` field on assistant turns whose `text_parts` would otherwise
///   be empty" — DeepSeek R1's other multi-turn quirk has always
///   co-occurred with the strip rule, so the two share one knob. A future
///   provider that needs only one of the two behaviours will require a
///   new variant (`#[non_exhaustive]` is set for that reason).
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
            pricing_known: true,
            limits_known: true,
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
    /// Operator override for the model's context window, in tokens (refs #7774).
    ///
    /// Corrects a *capacity fact* about the model rather than an inference parameter: a gateway that proxies a self-hosted runtime routinely reports `max_input_tokens: null`, and a model discovered from a `/models` listing gets whatever window the discovery path assumed.
    /// `None` defers to the catalog entry (probed or registry-declared); `Some(n)` with `n > 0` wins over it.
    /// A `Some(0)` is treated as absent everywhere the field is read, matching how `ModelCatalogEntry::context_window` already treats `0` as "unknown".
    ///
    /// Deliberately distinct from [`Self::max_tokens`], which is the per-request *output* cap sent on the wire. Setting the output cap to the model's window asks the model to reserve its whole context for the reply, which is the confusion this field exists to end.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    /// Operator override for the model's declared maximum output tokens (refs #7774).
    ///
    /// The sibling capacity limit to [`Self::context_window`], and unknown from
    /// the same sources for the same reason — a gateway reporting
    /// `max_input_tokens: null` reports `max_output_tokens: null` alongside it.
    /// `None` defers to the catalog entry; `Some(n)` with `n > 0` wins over it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
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
            && self.context_window.is_none()
            && self.max_output_tokens.is_none()
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

/// Which layer supplied one of the values in [`EffectiveLimits`] (refs #7774).
///
/// A limit and its provenance are computed together, in one pass over the same
/// override map and catalog entry, so a caller can never read a value from one
/// layer and attribute it to another.
/// The operator-facing consequence is the whole point of #7774's item 5: an
/// 8192 that came from a registry-declared catalog entry and an 8192 nobody
/// ever measured are the same number and a completely different fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LimitSource {
    /// Neither the operator override nor the catalog entry carried a usable
    /// value, so the paired `Option` is `None` and the caller's own fallback
    /// decides the number.
    #[default]
    Unknown,
    /// The catalog entry's registry-declared or probe-discovered value.
    Catalog,
    /// The operator's `model_overrides.json` correction, which outranks the
    /// catalog because it exists to correct it.
    Override,
}

/// Effective capacity limits for a model after applying operator overrides on
/// top of the catalog entry's declared values (refs #7774). Returned by
/// `ModelCatalog::effective_limits` / `effective_limits_for_manifest`.
///
/// Both value fields are `Option` because "unknown" is a real answer here and must
/// not be flattened to `0`: a `0` propagated into compaction thresholds or
/// budget math is the bug `ModelCatalogEntry::context_window` documents.
/// `None` means neither the operator nor the catalog knows, and the caller
/// applies its own fallback (and, for the context window, logs that it did).
///
/// Each value is paired with the [`LimitSource`] that produced it, so a caller
/// reporting the number to an operator can also say where it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct EffectiveLimits {
    /// Context window in tokens, or `None` when unknown.
    pub context_window: Option<u64>,
    /// Which layer supplied [`Self::context_window`].
    /// [`LimitSource::Unknown`] exactly when the value is `None`.
    #[serde(default)]
    pub context_window_source: LimitSource,
    /// Maximum output tokens, or `None` when unknown.
    pub max_output_tokens: Option<u64>,
    /// Which layer supplied [`Self::max_output_tokens`].
    /// [`LimitSource::Unknown`] exactly when the value is `None`.
    #[serde(default)]
    pub max_output_tokens_source: LimitSource,
}

/// Which layer of the context-window precedence chain answered (refs #7774).
///
/// The chain itself lives in the kernel's `resolve_context_window`; this enum
/// is the name it hands back so every surface can report the *provenance* of a
/// window rather than only its size.
/// [`Self::Fallback`] is the one an operator most needs to see: it means no
/// layer knew the model's window and the number on screen is a guess the
/// runtime made, which is exactly the condition that turns a real 16K
/// conversation into an imaginary overflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextWindowSource {
    /// `agent.toml: [model] context_window` — an explicit per-agent override.
    AgentOverride,
    /// `model_overrides.json: context_window` — the per-model operator override.
    ModelOverride,
    /// The model catalog entry, registry-declared or probe-discovered.
    Catalog,
    /// The window persisted on the session by an earlier turn, used only when
    /// nothing above resolves.
    SessionHint,
    /// Nothing resolved and the caller applied its own conservative default.
    /// The value on screen is assumed, not known.
    Fallback,
}

impl ContextWindowSource {
    /// Whether the window this source produced is a guess rather than a fact
    /// about the model.
    ///
    /// Only [`Self::Fallback`] qualifies. A session hint is a value some
    /// earlier turn resolved and persisted, so it is second-hand rather than
    /// invented — and when that earlier turn had nothing either, the hint it
    /// wrote is filtered out as a zero rather than promoted to a fact.
    pub fn is_assumed(self) -> bool {
        matches!(self, ContextWindowSource::Fallback)
    }

    /// The stable wire name, matching the `serde` representation.
    ///
    /// Used where the value has to reach a JSON body or a log field without
    /// routing through `serde_json` for one enum.
    pub fn as_str(self) -> &'static str {
        match self {
            ContextWindowSource::AgentOverride => "agent_override",
            ContextWindowSource::ModelOverride => "model_override",
            ContextWindowSource::Catalog => "catalog",
            ContextWindowSource::SessionHint => "session_hint",
            ContextWindowSource::Fallback => "fallback",
        }
    }
}

/// A resolved context window and the layer that produced it (refs #7774).
///
/// Returned by the kernel's `resolve_context_window` so the value and its
/// provenance travel together; splitting them into two calls is how a report
/// ends up labelling a catalog value as a fallback, or the reverse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedContextWindow {
    /// The window, in tokens. Always greater than zero — every layer filters
    /// its own zeros before answering.
    pub tokens: usize,
    /// Which layer answered.
    pub source: ContextWindowSource,
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
    /// True when this entry's credentials come from an external CLI's credential process rather than from a declared env var — currently only EveryAPI, registered by [`ModelCatalog::ensure_managed_everyapi`].
    ///
    /// Deliberately separate from [`Self::is_custom`], which answers a different question ("may the dashboard delete this?") and is unreliable as a proxy for this one: the catalog loader falls back to `is_custom = false` for *every* provider when `registry/providers/` is missing or unreadable, so a provider file — an explicit, env-var-credentialled configuration — is routinely non-custom.
    /// Reading `!is_custom` as "CLI-managed" therefore misclassifies an explicitly configured gateway and hands its endpoint to whatever account the CLI happens to be logged into.
    ///
    /// Any explicit configuration clears the flag: a provider file, an `ensure_explicit_everyapi` registration, or a user-set base URL.
    #[serde(default)]
    pub cli_managed: bool,
    /// Per-provider proxy URL override. When set, API calls to this provider
    /// are routed through this proxy instead of the global proxy config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_url: Option<String>,
    /// Opt in to live model discovery for a provider that is not one of the
    /// built-in local ids (`ollama` / `vllm` / `lmstudio` / `lemonade`).
    ///
    /// When true, the periodic probe loop and the `/api/providers/{name}/test`
    /// handler poll this provider's OpenAI-compatible `/models` endpoint and
    /// merge the result into the catalog, exactly as they already do for the
    /// built-in local ids.
    /// The predicate that reads this field ORs it with the built-in id check
    /// (`librefang_runtime::provider_health::discovers_models`), so a built-in
    /// local provider keeps discovering regardless of the flag's value and an
    /// existing install sees no change.
    #[serde(default)]
    pub discover_models: bool,
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
            cli_managed: false,
            proxy_url: None,
            discover_models: false,
        }
    }
}

/// Derive the conventional API-key environment variable name for a provider id.
///
/// `litellm` → `LITELLM_API_KEY`, `alibaba-coding-plan` → `ALIBABA_CODING_PLAN_API_KEY`.
/// This is the same shape the runtime already synthesizes when a provider is
/// registered from a bare `[provider_urls]` entry, so a catalog file that omits
/// `api_key_env` resolves to the variable the operator was already setting.
pub fn default_api_key_env(provider_id: &str) -> String {
    format!("{}_API_KEY", provider_id.to_uppercase().replace('-', "_"))
}

/// Provider metadata as stored in TOML catalog files.
///
/// Unlike [`ProviderInfo`], this struct omits runtime-only fields (`auth_status`,
/// `model_count`) so it maps 1:1 to the `[provider]` section in community catalog
/// files at `providers/<name>.toml`.
///
/// Every field except `id` is optional, because this struct doubles as a
/// partial overlay (#7776). A file that carries only `id` and one flag — which
/// is exactly what the discovery toggle used to write, and what an operator
/// hand-editing the TOML naturally produces — must still deserialize; the
/// alternative is a hard parse error that makes the loader drop the whole file
/// and silently revert the setting on the next boot. Missing values are filled
/// in by [`From<ProviderCatalogToml> for ProviderInfo`] (`display_name` falls
/// back to `id`, `api_key_env` to [`default_api_key_env`]) or left empty for
/// the catalog's merge step to fill from another source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCatalogToml {
    /// Provider identifier (e.g. "anthropic").
    pub id: String,
    /// Human-readable display name (e.g. "Anthropic").
    /// Falls back to `id` when absent.
    #[serde(default)]
    pub display_name: String,
    /// Environment variable name for the API key.
    /// Falls back to [`default_api_key_env`] when absent.
    #[serde(default)]
    pub api_key_env: String,
    /// Default base URL.
    /// May legitimately be empty: CLI-backed providers have no HTTP endpoint,
    /// and for a gateway configured through `[provider_urls]` in `config.toml`
    /// the URL arrives after the catalog is loaded.
    #[serde(default)]
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
    /// Opt in to live model discovery — see [`ProviderInfo::discover_models`].
    /// Absent in every registry-shipped file, so it defaults to `false` and the
    /// built-in local ids keep discovering through the id branch of the predicate.
    #[serde(default)]
    pub discover_models: bool,
}

fn default_key_required() -> bool {
    true
}

impl From<ProviderCatalogToml> for ProviderInfo {
    fn from(p: ProviderCatalogToml) -> Self {
        // Back-fill the two fields a partial overlay is allowed to omit, so
        // downstream code never has to special-case an empty display name or
        // an empty env var name (#7776).
        let display_name = if p.display_name.is_empty() {
            p.id.clone()
        } else {
            p.display_name
        };
        let api_key_env = if p.api_key_env.is_empty() {
            default_api_key_env(&p.id)
        } else {
            p.api_key_env
        };
        Self {
            id: p.id,
            display_name,
            api_key_env,
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
            // A provider file declares its own `api_key_env`, so it is an explicit configuration and never CLI-managed.
            cli_managed: false,
            proxy_url: None,
            discover_models: p.discover_models,
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

    /// #7776: the discovery toggle used to write `id` + `discover_models` and
    /// nothing else. That shape has to keep deserializing, or the loader drops
    /// the file and the operator's opt-in silently reverts on the next boot.
    #[test]
    fn partial_provider_record_deserializes_and_backfills_identity() {
        let raw = "[provider]\nid = \"litellm\"\ndiscover_models = true\n";
        let file: ModelCatalogFile = toml::from_str(raw).expect("partial overlay must parse");
        let provider = file.provider.expect("the [provider] table is present");
        assert!(provider.discover_models);

        let info: ProviderInfo = provider.into();
        assert_eq!(info.id, "litellm");
        assert_eq!(
            info.display_name, "litellm",
            "an absent display name falls back to the id"
        );
        assert_eq!(
            info.api_key_env, "LITELLM_API_KEY",
            "an absent api_key_env falls back to the conventional derivation"
        );
        assert_eq!(
            info.base_url, "",
            "base_url stays empty for the catalog merge / [provider_urls] to fill"
        );
        assert!(
            info.key_required,
            "key_required keeps its historical default"
        );
        assert!(info.discover_models, "the flag the file exists to carry");
    }

    /// A complete record must not be disturbed by the fallbacks above.
    #[test]
    fn complete_provider_record_keeps_every_declared_value() {
        let raw = concat!(
            "[provider]\n",
            "id = \"acme\"\n",
            "display_name = \"ACME Inc\"\n",
            "api_key_env = \"ACME_TOKEN\"\n",
            "base_url = \"https://api.acme.test/v1\"\n",
            "key_required = false\n",
        );
        let file: ModelCatalogFile = toml::from_str(raw).expect("full record must parse");
        let info: ProviderInfo = file.provider.expect("provider table").into();
        assert_eq!(info.display_name, "ACME Inc");
        assert_eq!(info.api_key_env, "ACME_TOKEN");
        assert_eq!(info.base_url, "https://api.acme.test/v1");
        assert!(!info.key_required);
        assert!(!info.discover_models, "absent flag stays off");
    }

    #[test]
    fn default_api_key_env_uppercases_and_underscores() {
        assert_eq!(default_api_key_env("litellm"), "LITELLM_API_KEY");
        assert_eq!(
            default_api_key_env("alibaba-coding-plan"),
            "ALIBABA_CODING_PLAN_API_KEY"
        );
    }

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
    fn model_tier_deserializes_known_values() {
        for (s, want) in [
            ("frontier", ModelTier::Frontier),
            ("smart", ModelTier::Smart),
            ("balanced", ModelTier::Balanced),
            ("fast", ModelTier::Fast),
            ("local", ModelTier::Local),
            ("custom", ModelTier::Custom),
        ] {
            let parsed: ModelTier = toml::from_str(&format!("tier = {s:?}"))
                .map(|w: TierWrap| w.tier)
                .unwrap_or_else(|e| panic!("{s} must parse: {e}"));
            assert_eq!(parsed, want, "tier {s:?}");
        }
    }

    /// #5822: an out-of-vocabulary tier (the dashboard once offered
    /// `"reasoning"`) must NOT fail the parse — it collapses to `Custom` so
    /// the provider stays loadable instead of silently vanishing.
    #[test]
    fn model_tier_deserializes_unknown_value_as_custom() {
        let w: TierWrap = toml::from_str(r#"tier = "reasoning""#).expect("unknown tier must parse");
        assert_eq!(w.tier, ModelTier::Custom);
        let w2: TierWrap = toml::from_str(r#"tier = "totally-made-up""#).expect("must parse");
        assert_eq!(w2.tier, ModelTier::Custom);
        // Case-insensitive on the known set, too.
        let w3: TierWrap = toml::from_str(r#"tier = "FRONTIER""#).expect("must parse");
        assert_eq!(w3.tier, ModelTier::Frontier);
    }

    #[derive(Deserialize)]
    struct TierWrap {
        tier: ModelTier,
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
        assert!(entry.pricing_known);
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
            cli_managed: false,
            proxy_url: None,
            discover_models: true,
        };
        let json = serde_json::to_string(&info).unwrap();
        let parsed: ProviderInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "anthropic");
        assert_eq!(parsed.auth_status, AuthStatus::Configured);
        assert_eq!(parsed.model_count, 3);
        assert!(parsed.discover_models, "the discovery opt-in round-trips");
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
            discover_models: false,
        };
        let info: ProviderInfo = toml_provider.into();
        assert_eq!(info.id, "anthropic");
        assert_eq!(info.auth_status, AuthStatus::Missing);
        assert_eq!(info.model_count, 0);
        assert!(info.regions.is_empty());
        assert!(!info.discover_models);
    }

    /// #6702: `discover_models` must survive the full TOML → `ProviderCatalogToml`
    /// → `ProviderInfo` → TOML round-trip, and must default to `false` when the
    /// key is absent — which is the shape of every registry-shipped provider file.
    #[test]
    fn provider_discover_models_round_trips_through_toml() {
        let with_flag = r#"
[provider]
id = "vllm-local"
display_name = "vLLM Local"
api_key_env = "VLLM_LOCAL_API_KEY"
base_url = "http://gpu-box:4000/v1"
key_required = true
discover_models = true
"#;
        let parsed: ModelCatalogFile = toml::from_str(with_flag).expect("parses");
        let provider = parsed.provider.expect("has a [provider] section");
        assert!(provider.discover_models, "flag read from TOML");

        // Re-serialize the provider section and parse it again: the flag has to
        // come back, otherwise a dashboard-written file would lose the opt-in.
        let reserialized = toml::to_string(&provider).expect("serializes");
        let reparsed: ProviderCatalogToml = toml::from_str(&reserialized).expect("re-parses");
        assert!(reparsed.discover_models, "flag survives the round-trip");

        let info: ProviderInfo = reparsed.into();
        assert!(info.discover_models, "flag reaches ProviderInfo");

        let without_flag = r#"
[provider]
id = "vllm"
display_name = "vLLM"
api_key_env = "VLLM_API_KEY"
base_url = "http://127.0.0.1:8000/v1"
key_required = false
"#;
        let legacy: ModelCatalogFile = toml::from_str(without_flag).expect("parses");
        assert!(
            !legacy
                .provider
                .expect("has a [provider] section")
                .discover_models,
            "absent key defaults to false"
        );
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
            cli_managed: false,
            proxy_url: None,
            discover_models: false,
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

    /// Refs #7774. A `model_overrides.json` written before the capacity-limit
    /// fields existed must keep parsing, and must not acquire a limit override
    /// it never asked for — the whole backward-compatibility contract of this
    /// change rests on `None` here.
    #[test]
    fn overrides_file_without_capacity_limits_still_parses_as_absent() {
        let json = r#"{"temperature": 0.7, "max_tokens": 4096}"#;
        let o: ModelOverrides = serde_json::from_str(json).expect("legacy overrides parse");
        assert_eq!(o.max_tokens, Some(4096));
        assert_eq!(o.context_window, None);
        assert_eq!(o.max_output_tokens, None);
        assert!(!o.is_empty(), "temperature/max_tokens are still overrides");
    }

    /// Refs #7774. The capacity limits are part of `is_empty`, or an overrides
    /// document carrying nothing but a corrected context window would be
    /// dropped by `ModelCatalog::set_overrides` the moment it was saved.
    #[test]
    fn a_context_window_override_alone_is_not_an_empty_override_set() {
        let o = ModelOverrides {
            context_window: Some(16_384),
            ..Default::default()
        };
        assert!(!o.is_empty());
        let max_out_only = ModelOverrides {
            max_output_tokens: Some(8_192),
            ..Default::default()
        };
        assert!(!max_out_only.is_empty());
        assert!(ModelOverrides::default().is_empty());
    }

    /// Refs #7774. Absent limits stay absent on the wire: the dashboard reads
    /// `overrides.context_window == undefined` as "no override, show the
    /// catalog value", so serializing an explicit `null` would be a lie the UI
    /// cannot distinguish from a real zero.
    #[test]
    fn absent_capacity_limits_are_omitted_from_serialized_overrides() {
        let json = serde_json::to_string(&ModelOverrides {
            temperature: Some(0.5),
            ..Default::default()
        })
        .expect("serialize");
        assert!(!json.contains("context_window"), "{json}");
        assert!(!json.contains("max_output_tokens"), "{json}");
    }
}
