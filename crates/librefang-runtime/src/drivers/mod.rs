//! LLM driver implementations.
//!
//! Contains drivers for Anthropic Claude, Google Gemini, OpenAI-compatible APIs, and more.
//! Supports: Anthropic, Gemini, OpenAI, Groq, OpenRouter, DeepSeek, Together,
//! Mistral, Fireworks, Ollama, vLLM, Chutes.ai, and any OpenAI-compatible endpoint.

pub mod aider;
pub mod anthropic;
pub mod chatgpt;
pub mod claude_code;
pub mod codex_cli;
pub mod copilot;
pub mod fallback;
pub mod gemini;
pub mod gemini_cli;
pub mod openai;
pub mod qwen_code;
pub mod token_rotation;
pub mod vertex_ai;

use crate::llm_driver::{DriverConfig, LlmDriver, LlmError};
use librefang_types::model_catalog::{
    AI21_BASE_URL, ANTHROPIC_BASE_URL, CEREBRAS_BASE_URL, CHATGPT_BASE_URL, CHUTES_BASE_URL,
    COHERE_BASE_URL, DEEPSEEK_BASE_URL, FIREWORKS_BASE_URL, GEMINI_BASE_URL,
    GITHUB_COPILOT_BASE_URL, GROQ_BASE_URL, HUGGINGFACE_BASE_URL, KIMI_CODING_BASE_URL,
    LEMONADE_BASE_URL, LMSTUDIO_BASE_URL, MINIMAX_CN_BASE_URL, MINIMAX_INTL_BASE_URL,
    MISTRAL_BASE_URL, MOONSHOT_BASE_URL, NVIDIA_NIM_BASE_URL, OLLAMA_BASE_URL, OPENAI_BASE_URL,
    OPENROUTER_BASE_URL, PERPLEXITY_BASE_URL, QIANFAN_BASE_URL, QWEN_BASE_URL, REPLICATE_BASE_URL,
    SAMBANOVA_BASE_URL, TOGETHER_BASE_URL, VENICE_BASE_URL, VERTEX_AI_BASE_URL, VLLM_BASE_URL,
    VOLCENGINE_BASE_URL, VOLCENGINE_CODING_BASE_URL, XAI_BASE_URL, ZAI_BASE_URL,
    ZAI_CODING_BASE_URL, ZHIPU_BASE_URL, ZHIPU_CODING_BASE_URL,
};
use std::sync::Arc;

// ── Registry Types ───────────────────────────────────────────────

/// API format determines which driver implementation to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiFormat {
    /// OpenAI-compatible chat completions API (used by 90%+ of providers).
    OpenAI,
    /// Anthropic Messages API.
    Anthropic,
    /// Google Gemini generateContent API.
    Gemini,
    /// Claude Code CLI subprocess.
    ClaudeCode,
    /// Qwen Code CLI subprocess.
    QwenCode,
    /// Gemini CLI subprocess.
    GeminiCli,
    /// Codex CLI subprocess.
    CodexCli,
    /// Aider CLI subprocess.
    Aider,
    /// ChatGPT with session token authentication.
    ChatGpt,
    /// GitHub Copilot with automatic token exchange.
    Copilot,
    /// Google Cloud Vertex AI (Gemini format with OAuth2 auth).
    VertexAI,
}

/// A provider entry in the static registry.
#[derive(Debug)]
struct ProviderEntry {
    /// Canonical provider name.
    name: &'static str,
    /// Alternative names that resolve to this provider.
    aliases: &'static [&'static str],
    /// Default base URL for the API.
    base_url: &'static str,
    /// Environment variable name for the API key.
    api_key_env: &'static str,
    /// Whether an API key is required (false for local providers like Ollama).
    key_required: bool,
    /// Which API format/driver to use.
    api_format: ApiFormat,
    /// Optional secondary env var for API key (e.g., GOOGLE_API_KEY for Gemini).
    alt_api_key_env: Option<&'static str>,
    /// Whether this provider is hidden from `known_providers()` output.
    hidden: bool,
}

// ── Static Provider Registry ─────────────────────────────────────

static PROVIDER_REGISTRY: &[ProviderEntry] = &[
    ProviderEntry {
        name: "anthropic",
        aliases: &[],
        base_url: ANTHROPIC_BASE_URL,
        api_key_env: "ANTHROPIC_API_KEY",
        key_required: true,
        api_format: ApiFormat::Anthropic,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "chatgpt",
        aliases: &[],
        base_url: CHATGPT_BASE_URL,
        api_key_env: "CHATGPT_SESSION_TOKEN",
        key_required: true,
        api_format: ApiFormat::ChatGpt,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "gemini",
        aliases: &["google"],
        base_url: GEMINI_BASE_URL,
        api_key_env: "GEMINI_API_KEY",
        key_required: true,
        api_format: ApiFormat::Gemini,
        alt_api_key_env: Some("GOOGLE_API_KEY"),
        hidden: false,
    },
    ProviderEntry {
        name: "openai",
        aliases: &["codex", "openai-codex"],
        base_url: OPENAI_BASE_URL,
        api_key_env: "OPENAI_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "groq",
        aliases: &[],
        base_url: GROQ_BASE_URL,
        api_key_env: "GROQ_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "openrouter",
        aliases: &[],
        base_url: OPENROUTER_BASE_URL,
        api_key_env: "OPENROUTER_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "deepseek",
        aliases: &[],
        base_url: DEEPSEEK_BASE_URL,
        api_key_env: "DEEPSEEK_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "together",
        aliases: &[],
        base_url: TOGETHER_BASE_URL,
        api_key_env: "TOGETHER_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "mistral",
        aliases: &[],
        base_url: MISTRAL_BASE_URL,
        api_key_env: "MISTRAL_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "fireworks",
        aliases: &[],
        base_url: FIREWORKS_BASE_URL,
        api_key_env: "FIREWORKS_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "ollama",
        aliases: &[],
        base_url: OLLAMA_BASE_URL,
        api_key_env: "OLLAMA_API_KEY",
        key_required: false,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "vllm",
        aliases: &[],
        base_url: VLLM_BASE_URL,
        api_key_env: "VLLM_API_KEY",
        key_required: false,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "lmstudio",
        aliases: &[],
        base_url: LMSTUDIO_BASE_URL,
        api_key_env: "LMSTUDIO_API_KEY",
        key_required: false,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "lemonade",
        aliases: &[],
        base_url: LEMONADE_BASE_URL,
        api_key_env: "LEMONADE_API_KEY",
        key_required: false,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: true,
    },
    ProviderEntry {
        name: "perplexity",
        aliases: &[],
        base_url: PERPLEXITY_BASE_URL,
        api_key_env: "PERPLEXITY_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "cohere",
        aliases: &[],
        base_url: COHERE_BASE_URL,
        api_key_env: "COHERE_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "ai21",
        aliases: &[],
        base_url: AI21_BASE_URL,
        api_key_env: "AI21_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "cerebras",
        aliases: &[],
        base_url: CEREBRAS_BASE_URL,
        api_key_env: "CEREBRAS_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "sambanova",
        aliases: &[],
        base_url: SAMBANOVA_BASE_URL,
        api_key_env: "SAMBANOVA_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "huggingface",
        aliases: &[],
        base_url: HUGGINGFACE_BASE_URL,
        api_key_env: "HF_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "xai",
        aliases: &[],
        base_url: XAI_BASE_URL,
        api_key_env: "XAI_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "replicate",
        aliases: &[],
        base_url: REPLICATE_BASE_URL,
        api_key_env: "REPLICATE_API_TOKEN",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "github-copilot",
        aliases: &["copilot"],
        base_url: GITHUB_COPILOT_BASE_URL,
        api_key_env: "GITHUB_TOKEN",
        key_required: true,
        api_format: ApiFormat::Copilot,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "claude-code",
        aliases: &[],
        base_url: "",
        api_key_env: "",
        key_required: false,
        api_format: ApiFormat::ClaudeCode,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "qwen-code",
        aliases: &[],
        base_url: "",
        api_key_env: "",
        key_required: false,
        api_format: ApiFormat::QwenCode,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "gemini-cli",
        aliases: &[],
        base_url: "",
        api_key_env: "",
        key_required: false,
        api_format: ApiFormat::GeminiCli,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "codex-cli",
        aliases: &[],
        base_url: "",
        api_key_env: "",
        key_required: false,
        api_format: ApiFormat::CodexCli,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "aider",
        aliases: &[],
        base_url: "",
        api_key_env: "",
        key_required: false,
        api_format: ApiFormat::Aider,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "moonshot",
        aliases: &["kimi", "kimi2"],
        base_url: MOONSHOT_BASE_URL,
        api_key_env: "MOONSHOT_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "kimi_coding",
        aliases: &[],
        base_url: KIMI_CODING_BASE_URL,
        api_key_env: "KIMI_API_KEY",
        key_required: true,
        api_format: ApiFormat::Anthropic,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "qwen",
        aliases: &["dashscope", "model_studio"],
        base_url: QWEN_BASE_URL,
        api_key_env: "DASHSCOPE_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "minimax",
        aliases: &[],
        base_url: MINIMAX_INTL_BASE_URL,
        api_key_env: "MINIMAX_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "minimax-cn",
        aliases: &[],
        base_url: MINIMAX_CN_BASE_URL,
        api_key_env: "MINIMAX_CN_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "zhipu",
        aliases: &["glm"],
        base_url: ZHIPU_BASE_URL,
        api_key_env: "ZHIPU_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "zhipu_coding",
        aliases: &["codegeex"],
        base_url: ZHIPU_CODING_BASE_URL,
        api_key_env: "ZHIPU_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "zai",
        aliases: &["z.ai"],
        base_url: ZAI_BASE_URL,
        api_key_env: "ZHIPU_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "zai_coding",
        aliases: &[],
        base_url: ZAI_CODING_BASE_URL,
        api_key_env: "ZHIPU_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: true,
    },
    ProviderEntry {
        name: "qianfan",
        aliases: &["baidu"],
        base_url: QIANFAN_BASE_URL,
        api_key_env: "QIANFAN_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "volcengine",
        aliases: &["doubao"],
        base_url: VOLCENGINE_BASE_URL,
        api_key_env: "VOLCENGINE_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "volcengine_coding",
        aliases: &[],
        base_url: VOLCENGINE_CODING_BASE_URL,
        api_key_env: "VOLCENGINE_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: true,
    },
    ProviderEntry {
        name: "chutes",
        aliases: &[],
        base_url: CHUTES_BASE_URL,
        api_key_env: "CHUTES_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "venice",
        aliases: &[],
        base_url: VENICE_BASE_URL,
        api_key_env: "VENICE_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "vertex-ai",
        aliases: &["vertex", "vertex_ai"],
        base_url: VERTEX_AI_BASE_URL,
        api_key_env: "GOOGLE_APPLICATION_CREDENTIALS",
        key_required: true, // Requires Google auth, but create_driver handles OAuth flows separately.
        api_format: ApiFormat::VertexAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "nvidia-nim",
        aliases: &[],
        base_url: NVIDIA_NIM_BASE_URL,
        api_key_env: "NVIDIA_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
];

// ── Registry Lookup ──────────────────────────────────────────────

/// Find a provider by name or alias.
fn find_provider(name: &str) -> Option<&'static ProviderEntry> {
    PROVIDER_REGISTRY
        .iter()
        .find(|p| p.name == name || p.aliases.contains(&name))
}

// ── Provider Defaults (registry-backed, used by tests) ───────────

/// Provider metadata: base URL and env var name for the API key.
#[cfg(test)]
struct ProviderDefaults {
    base_url: &'static str,
    api_key_env: &'static str,
    /// If true, the API key is required (error if missing).
    key_required: bool,
}

/// Get defaults for known providers.
#[cfg(test)]
fn provider_defaults(provider: &str) -> Option<ProviderDefaults> {
    find_provider(provider).map(|entry| ProviderDefaults {
        base_url: entry.base_url,
        api_key_env: entry.api_key_env,
        key_required: entry.key_required,
    })
}

// ── Driver Creation ──────────────────────────────────────────────

/// Create a driver from a registry entry and configuration.
fn create_driver_from_entry(
    entry: &ProviderEntry,
    config: &DriverConfig,
) -> Result<Arc<dyn LlmDriver>, LlmError> {
    let base_url = config
        .base_url
        .clone()
        .unwrap_or_else(|| entry.base_url.to_string());

    // Resolve API key: explicit config > primary env var > alt env var
    let mut api_key = config
        .api_key
        .clone()
        .or_else(|| std::env::var(entry.api_key_env).ok())
        .or_else(|| entry.alt_api_key_env.and_then(|v| std::env::var(v).ok()))
        .unwrap_or_default();

    // Special: OpenAI also checks Codex credential
    if api_key.is_empty() && entry.api_format == ApiFormat::OpenAI && entry.name == "openai" {
        if let Some(codex_key) = crate::model_catalog::read_codex_credential() {
            api_key = codex_key;
        }
    }

    if entry.key_required && entry.api_format != ApiFormat::VertexAI && api_key.is_empty() {
        return Err(LlmError::MissingApiKey(format!(
            "Set {} environment variable for provider '{}'",
            entry.api_key_env, config.provider
        )));
    }

    match entry.api_format {
        ApiFormat::OpenAI => Ok(Arc::new(openai::OpenAIDriver::new(api_key, base_url))),
        ApiFormat::Anthropic => Ok(Arc::new(anthropic::AnthropicDriver::new(api_key, base_url))),
        ApiFormat::Gemini => Ok(Arc::new(gemini::GeminiDriver::new(api_key, base_url))),
        ApiFormat::ClaudeCode => Ok(Arc::new(claude_code::ClaudeCodeDriver::new(
            config.base_url.clone(),
            config.skip_permissions,
        ))),
        ApiFormat::QwenCode => Ok(Arc::new(qwen_code::QwenCodeDriver::new(
            config.base_url.clone(),
            config.skip_permissions,
        ))),
        ApiFormat::GeminiCli => Ok(Arc::new(gemini_cli::GeminiCliDriver::new(
            config.base_url.clone(),
            config.skip_permissions,
        ))),
        ApiFormat::CodexCli => Ok(Arc::new(codex_cli::CodexCliDriver::new(
            config.base_url.clone(),
            config.skip_permissions,
        ))),
        ApiFormat::Aider => Ok(Arc::new(aider::AiderDriver::new(
            config.base_url.clone(),
            config.skip_permissions,
        ))),
        ApiFormat::ChatGpt => Ok(Arc::new(chatgpt::ChatGptDriver::new(api_key, base_url))),
        ApiFormat::Copilot => Ok(Arc::new(copilot::CopilotDriver::new(api_key, base_url))),
        ApiFormat::VertexAI => Ok(Arc::new(vertex_ai::VertexAiDriver::new(config)?)),
    }
}

/// Create an LLM driver based on provider name and configuration.
///
/// Supported providers:
/// - `anthropic` — Anthropic Claude (Messages API)
/// - `openai` — OpenAI GPT models
/// - `groq` — Groq (ultra-fast inference)
/// - `openrouter` — OpenRouter (multi-model gateway)
/// - `deepseek` — DeepSeek
/// - `together` — Together AI
/// - `mistral` — Mistral AI
/// - `fireworks` — Fireworks AI
/// - `ollama` — Ollama (local)
/// - `vllm` — vLLM (local)
/// - `lmstudio` — LM Studio (local)
/// - `perplexity` — Perplexity AI (search-augmented)
/// - `cohere` — Cohere (Command R)
/// - `ai21` — AI21 Labs (Jamba)
/// - `cerebras` — Cerebras (ultra-fast inference)
/// - `sambanova` — SambaNova
/// - `huggingface` — Hugging Face Inference API
/// - `xai` — xAI (Grok)
/// - `replicate` — Replicate
/// - `chutes` — Chutes.ai (serverless open-source model inference)
/// - `vertex-ai` — Google Cloud Vertex AI (OAuth2 auth, enterprise Gemini)
/// - Any custom provider with `base_url` set uses OpenAI-compatible format
pub fn create_driver(config: &DriverConfig) -> Result<Arc<dyn LlmDriver>, LlmError> {
    let provider = config.provider.as_str();

    // Look up in the registry first
    if let Some(entry) = find_provider(provider) {
        return create_driver_from_entry(entry, config);
    }

    // Unknown provider — if base_url is set, treat as custom OpenAI-compatible.
    // For custom providers, try the convention {PROVIDER_UPPER}_API_KEY as env var
    // when no explicit api_key was passed. This lets users just set e.g. NVIDIA_API_KEY
    // in their environment and use provider = "nvidia" without extra config.
    if let Some(ref base_url) = config.base_url {
        let api_key = config.api_key.clone().unwrap_or_else(|| {
            let env_var = format!("{}_API_KEY", provider.to_uppercase().replace('-', "_"));
            std::env::var(&env_var).unwrap_or_default()
        });
        return Ok(Arc::new(openai::OpenAIDriver::new(
            api_key,
            base_url.clone(),
        )));
    }

    // No base_url either — last resort: check if the user set an API key env var
    // using the convention {PROVIDER_UPPER}_API_KEY. If found, use OpenAI-compatible
    // driver with a default base URL derived from common patterns.
    {
        let env_var = format!("{}_API_KEY", provider.to_uppercase().replace('-', "_"));
        if let Ok(api_key) = std::env::var(&env_var) {
            if !api_key.is_empty() {
                return Err(LlmError::Api {
                    status: 0,
                    message: format!(
                        "Provider '{}' has API key ({} is set) but no base_url configured. \
                         Add base_url to your [default_model] config or set it in [provider_urls].",
                        provider, env_var
                    ),
                });
            }
        }
    }

    Err(LlmError::Api {
        status: 0,
        message: format!(
            "Unknown provider '{}'. Supported: anthropic, chatgpt, gemini, openai, groq, openrouter, \
             deepseek, together, mistral, fireworks, ollama, vllm, lmstudio, perplexity, \
             cohere, ai21, cerebras, sambanova, huggingface, xai, replicate, github-copilot, \
             chutes, venice, vertex-ai, nvidia-nim, codex, claude-code, qwen-code, \
             gemini-cli, codex-cli, aider. Or set base_url for a custom OpenAI-compatible endpoint.",
            provider
        ),
    })
}

/// Detect the first available provider by scanning environment variables.
///
/// Returns `(provider, model, api_key_env)` for the first provider that has a
/// configured API key, checked in a user-friendly priority order.
pub fn detect_available_provider() -> Option<(&'static str, &'static str, &'static str)> {
    // Priority: popular cloud providers first, then niche, then local
    const PROBE_ORDER: &[(&str, &str, &str)] = &[
        ("openai", "gpt-4o", "OPENAI_API_KEY"),
        ("anthropic", "claude-sonnet-4-20250514", "ANTHROPIC_API_KEY"),
        ("gemini", "gemini-2.5-flash", "GEMINI_API_KEY"),
        ("groq", "llama-3.3-70b-versatile", "GROQ_API_KEY"),
        ("deepseek", "deepseek-chat", "DEEPSEEK_API_KEY"),
        (
            "openrouter",
            "openrouter/google/gemini-2.5-flash",
            "OPENROUTER_API_KEY",
        ),
        ("mistral", "mistral-large-latest", "MISTRAL_API_KEY"),
        (
            "together",
            "meta-llama/Llama-3-70b-chat-hf",
            "TOGETHER_API_KEY",
        ),
        (
            "fireworks",
            "accounts/fireworks/models/llama-v3p1-70b-instruct",
            "FIREWORKS_API_KEY",
        ),
        ("xai", "grok-2", "XAI_API_KEY"),
        (
            "perplexity",
            "llama-3.1-sonar-large-128k-online",
            "PERPLEXITY_API_KEY",
        ),
        ("cohere", "command-r-plus", "COHERE_API_KEY"),
    ];
    for &(provider, model, env_var) in PROBE_ORDER {
        if std::env::var(env_var)
            .ok()
            .filter(|v| !v.is_empty())
            .is_some()
        {
            return Some((provider, model, env_var));
        }
    }
    // Also check GOOGLE_API_KEY as alias for Gemini
    if std::env::var("GOOGLE_API_KEY")
        .ok()
        .filter(|v| !v.is_empty())
        .is_some()
    {
        return Some(("gemini", "gemini-2.5-flash", "GOOGLE_API_KEY"));
    }
    None
}

/// List all known provider names.
///
/// Returns canonical names from the provider registry, excluding hidden
/// internal providers (e.g. `volcengine_coding`, `zai_coding`, `lemonade`).
pub fn known_providers() -> Vec<&'static str> {
    PROVIDER_REGISTRY
        .iter()
        .filter(|p| !p.hidden)
        .map(|p| p.name)
        .collect()
}

/// Check if a CLI-based provider is available (binary on PATH or credentials exist).
pub fn cli_provider_available(name: &str) -> bool {
    match name {
        "claude-code" => claude_code::claude_code_available(),
        "qwen-code" => qwen_code::qwen_code_available(),
        "gemini-cli" => gemini_cli::gemini_cli_available(),
        "codex-cli" => codex_cli::codex_cli_available(),
        "aider" => aider::aider_available(),
        _ => false,
    }
}

/// Check if a provider name refers to a CLI-subprocess-based provider.
pub fn is_cli_provider(name: &str) -> bool {
    matches!(
        name,
        "claude-code" | "qwen-code" | "gemini-cli" | "codex-cli" | "aider"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_defaults_groq() {
        let d = provider_defaults("groq").unwrap();
        assert_eq!(d.base_url, "https://api.groq.com/openai/v1");
        assert_eq!(d.api_key_env, "GROQ_API_KEY");
        assert!(d.key_required);
    }

    #[test]
    fn test_provider_defaults_openrouter() {
        let d = provider_defaults("openrouter").unwrap();
        assert_eq!(d.base_url, "https://openrouter.ai/api/v1");
        assert!(d.key_required);
    }

    #[test]
    fn test_provider_defaults_ollama() {
        let d = provider_defaults("ollama").unwrap();
        assert!(!d.key_required);
    }

    #[test]
    fn test_unknown_provider_returns_none() {
        assert!(provider_defaults("nonexistent").is_none());
    }

    #[test]
    fn test_custom_provider_with_base_url() {
        let config = DriverConfig {
            provider: "my-custom-llm".to_string(),
            api_key: Some("test".to_string()),
            base_url: Some("http://localhost:9999/v1".to_string()),
            vertex_ai: librefang_types::config::VertexAiConfig::default(),
            skip_permissions: true,
        };
        let driver = create_driver(&config);
        assert!(driver.is_ok());
    }

    #[test]
    fn test_unknown_provider_no_url_errors() {
        let config = DriverConfig {
            provider: "nonexistent".to_string(),
            api_key: None,
            base_url: None,
            vertex_ai: librefang_types::config::VertexAiConfig::default(),
            skip_permissions: true,
        };
        let driver = create_driver(&config);
        assert!(driver.is_err());
    }

    #[test]
    fn test_provider_defaults_gemini() {
        let d = provider_defaults("gemini").unwrap();
        assert_eq!(d.base_url, "https://generativelanguage.googleapis.com");
        assert_eq!(d.api_key_env, "GEMINI_API_KEY");
        assert!(d.key_required);
    }

    #[test]
    fn test_provider_defaults_google_alias() {
        let d = provider_defaults("google").unwrap();
        assert_eq!(d.base_url, "https://generativelanguage.googleapis.com");
        assert!(d.key_required);
    }

    #[test]
    fn test_known_providers_list() {
        let providers = known_providers();
        assert!(providers.contains(&"groq"));
        assert!(providers.contains(&"openrouter"));
        assert!(providers.contains(&"anthropic"));
        assert!(providers.contains(&"gemini"));
        // New providers
        assert!(providers.contains(&"perplexity"));
        assert!(providers.contains(&"cohere"));
        assert!(providers.contains(&"ai21"));
        assert!(providers.contains(&"cerebras"));
        assert!(providers.contains(&"sambanova"));
        assert!(providers.contains(&"huggingface"));
        assert!(providers.contains(&"xai"));
        assert!(providers.contains(&"replicate"));
        assert!(providers.contains(&"chatgpt"));
        assert!(providers.contains(&"github-copilot"));
        assert!(providers.contains(&"moonshot"));
        assert!(providers.contains(&"qwen"));
        assert!(providers.contains(&"minimax"));
        assert!(providers.contains(&"minimax-cn"));
        assert!(providers.contains(&"zhipu"));
        assert!(providers.contains(&"zhipu_coding"));
        assert!(providers.contains(&"zai"));
        assert!(providers.contains(&"kimi_coding"));
        assert!(providers.contains(&"qianfan"));
        assert!(providers.contains(&"volcengine"));
        assert!(providers.contains(&"chutes"));
        assert!(providers.contains(&"claude-code"));
        assert!(providers.contains(&"qwen-code"));
        assert!(providers.contains(&"gemini-cli"));
        assert!(providers.contains(&"codex-cli"));
        assert!(providers.contains(&"aider"));
        assert!(providers.contains(&"vertex-ai"));
        assert!(providers.contains(&"nvidia-nim"));
        assert_eq!(providers.len(), 41);
    }

    #[test]
    fn test_provider_defaults_perplexity() {
        let d = provider_defaults("perplexity").unwrap();
        assert_eq!(d.base_url, "https://api.perplexity.ai");
        assert_eq!(d.api_key_env, "PERPLEXITY_API_KEY");
        assert!(d.key_required);
    }

    #[test]
    fn test_provider_defaults_xai() {
        let d = provider_defaults("xai").unwrap();
        assert_eq!(d.base_url, "https://api.x.ai/v1");
        assert_eq!(d.api_key_env, "XAI_API_KEY");
        assert!(d.key_required);
    }

    #[test]
    fn test_provider_defaults_cohere() {
        let d = provider_defaults("cohere").unwrap();
        assert_eq!(d.base_url, "https://api.cohere.com/v2");
        assert!(d.key_required);
    }

    #[test]
    fn test_provider_defaults_cerebras() {
        let d = provider_defaults("cerebras").unwrap();
        assert_eq!(d.base_url, "https://api.cerebras.ai/v1");
        assert!(d.key_required);
    }

    #[test]
    fn test_provider_defaults_huggingface() {
        let d = provider_defaults("huggingface").unwrap();
        assert_eq!(d.base_url, "https://api-inference.huggingface.co/v1");
        assert_eq!(d.api_key_env, "HF_API_KEY");
        assert!(d.key_required);
    }

    #[test]
    fn test_custom_provider_convention_env_var() {
        // Set NVIDIA_API_KEY env var, then create a custom "nvidia" provider with base_url.
        // The driver should pick up the key automatically via convention.
        let unique_key = "test-nvidia-key-12345";
        std::env::set_var("NVIDIA_API_KEY", unique_key);
        let config = DriverConfig {
            provider: "nvidia".to_string(),
            api_key: None, // not explicitly passed
            base_url: Some("https://integrate.api.nvidia.com/v1".to_string()),
            vertex_ai: librefang_types::config::VertexAiConfig::default(),
            skip_permissions: true,
        };
        let driver = create_driver(&config);
        assert!(
            driver.is_ok(),
            "Custom provider with env var convention should succeed"
        );
        std::env::remove_var("NVIDIA_API_KEY");
    }

    #[test]
    fn test_custom_provider_no_key_no_url_errors() {
        // Custom provider with neither API key nor base_url should error.
        let config = DriverConfig {
            provider: "nvidia".to_string(),
            api_key: None,
            base_url: None,
            vertex_ai: librefang_types::config::VertexAiConfig::default(),
            skip_permissions: true,
        };
        let driver = create_driver(&config);
        assert!(driver.is_err());
    }

    #[test]
    fn test_custom_provider_key_no_url_helpful_error() {
        // Custom provider with key set (via env) but no base_url should give helpful error.
        let unique_key = "test-nvidia-key-67890";
        std::env::set_var("NVIDIA_API_KEY", unique_key);
        let config = DriverConfig {
            provider: "nvidia".to_string(),
            api_key: None,
            base_url: None,
            vertex_ai: librefang_types::config::VertexAiConfig::default(),
            skip_permissions: true,
        };
        let result = create_driver(&config);
        assert!(result.is_err());
        let err = result.err().unwrap().to_string();
        assert!(
            err.contains("base_url"),
            "Error should mention base_url: {}",
            err
        );
        std::env::remove_var("NVIDIA_API_KEY");
    }

    #[test]
    fn test_provider_defaults_kimi_coding() {
        let d = provider_defaults("kimi_coding").unwrap();
        assert_eq!(d.base_url, "https://api.kimi.com/coding");
        assert_eq!(d.api_key_env, "KIMI_API_KEY");
        assert!(d.key_required);
    }

    #[test]
    fn test_custom_provider_explicit_key_with_url() {
        // When api_key is explicitly passed, it should be used regardless of env var.
        let config = DriverConfig {
            provider: "my-custom-provider".to_string(),
            api_key: Some("explicit-key".to_string()),
            base_url: Some("https://api.example.com/v1".to_string()),
            vertex_ai: librefang_types::config::VertexAiConfig::default(),
            skip_permissions: true,
        };
        let driver = create_driver(&config);
        assert!(driver.is_ok());
    }

    #[test]
    fn test_vertex_ai_uses_kernel_vertex_config() {
        let config = DriverConfig {
            provider: "vertex-ai".to_string(),
            api_key: None,
            base_url: None,
            vertex_ai: librefang_types::config::VertexAiConfig {
                project_id: Some("config-project".to_string()),
                region: Some("europe-west4".to_string()),
                credentials_path: Some(
                    serde_json::json!({
                        "type": "service_account",
                        "project_id": "json-project",
                    })
                    .to_string(),
                ),
            },
            skip_permissions: true,
        };

        let driver = create_driver(&config);
        assert!(
            driver.is_ok(),
            "Vertex AI driver should initialize from [vertex_ai] config without env vars"
        );
    }
}
