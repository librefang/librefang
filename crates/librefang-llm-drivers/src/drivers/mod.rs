//! LLM driver implementations.
//!
//! Contains drivers for Anthropic Claude, Google Gemini, OpenAI-compatible APIs, and more.
//! Supports: Anthropic, Gemini, OpenAI, Groq, OpenRouter, DeepSeek, DeepInfra,
//! Together, Mistral, Fireworks, Ollama, vLLM, Chutes.ai, Alibaba Coding Plan, and any
//! OpenAI-compatible endpoint.

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
#[cfg(feature = "uar-driver")]
pub mod uar;
pub mod vertex_ai;

use crate::llm_driver::{DriverConfig, LlmDriver, LlmError};
use dashmap::DashMap;
use std::sync::Arc;

// ── Driver Cache ────────────────────────────────────────────────

/// Thread-safe, lazy-initializing cache for LLM drivers.
///
/// Instead of creating a new HTTP-client-bearing driver on every agent message,
/// `DriverCache` keeps one `Arc<dyn LlmDriver>` per unique
/// `(provider, api_key, base_url)` tuple and returns a clone of the `Arc` on
/// subsequent calls. This eliminates redundant TLS handshakes and connection-pool
/// setup during startup and steady-state operation.
pub struct DriverCache {
    cache: DashMap<String, Arc<dyn LlmDriver>>,
}

impl Default for DriverCache {
    fn default() -> Self {
        Self::new()
    }
}

impl DriverCache {
    /// Create an empty driver cache.
    pub fn new() -> Self {
        Self {
            cache: DashMap::new(),
        }
    }

    /// Return a cached driver for the given config, or create (and cache) one.
    ///
    /// The cache key is derived from `(provider, api_key, base_url)` so that
    /// different credentials or endpoints produce distinct drivers.
    pub fn get_or_create(&self, config: &DriverConfig) -> Result<Arc<dyn LlmDriver>, LlmError> {
        let key = Self::cache_key(config);
        if let Some(driver) = self.cache.get(&key) {
            return Ok(Arc::clone(driver.value()));
        }
        let driver = create_driver(config)?;
        self.cache.insert(key, Arc::clone(&driver));
        Ok(driver)
    }

    /// Invalidate all cached drivers (e.g. after a config hot-reload).
    pub fn clear(&self) {
        self.cache.clear();
    }

    /// Number of cached drivers (useful for metrics / debugging).
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }

    /// Build a deterministic cache key from the driver config fields that
    /// affect which concrete driver instance is produced.
    fn cache_key(config: &DriverConfig) -> String {
        // We include provider, api_key hash (not the raw key), and base_url.
        // Hashing the api_key avoids storing secrets as map keys while still
        // distinguishing configs that differ only by credential.
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        config.api_key.as_deref().unwrap_or("").hash(&mut hasher);
        let key_hash = hasher.finish();

        format!(
            "{}|{}|{}|{}",
            config.provider,
            key_hash,
            config.base_url.as_deref().unwrap_or(""),
            config.proxy_url.as_deref().unwrap_or("")
        )
    }
}

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
    /// Azure OpenAI (OpenAI format with `api-key` header and deployment-based URL).
    AzureOpenAI,
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
        base_url: "https://api.anthropic.com",
        api_key_env: "ANTHROPIC_API_KEY",
        key_required: true,
        api_format: ApiFormat::Anthropic,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "chatgpt",
        aliases: &[],
        base_url: "https://chatgpt.com/backend-api",
        api_key_env: "CHATGPT_SESSION_TOKEN",
        key_required: true,
        api_format: ApiFormat::ChatGpt,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "gemini",
        aliases: &["google"],
        base_url: "https://generativelanguage.googleapis.com",
        api_key_env: "GEMINI_API_KEY",
        key_required: true,
        api_format: ApiFormat::Gemini,
        alt_api_key_env: Some("GOOGLE_API_KEY"),
        hidden: false,
    },
    ProviderEntry {
        name: "openai",
        aliases: &["codex", "openai-codex"],
        base_url: "https://api.openai.com/v1",
        api_key_env: "OPENAI_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "groq",
        aliases: &[],
        base_url: "https://api.groq.com/openai/v1",
        api_key_env: "GROQ_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "openrouter",
        aliases: &[],
        base_url: "https://openrouter.ai/api/v1",
        api_key_env: "OPENROUTER_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "deepseek",
        aliases: &[],
        base_url: "https://api.deepseek.com/v1",
        api_key_env: "DEEPSEEK_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "deepinfra",
        aliases: &[],
        base_url: "https://api.deepinfra.com/v1/openai",
        api_key_env: "DEEPINFRA_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "together",
        aliases: &[],
        base_url: "https://api.together.xyz/v1",
        api_key_env: "TOGETHER_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "mistral",
        aliases: &[],
        base_url: "https://api.mistral.ai/v1",
        api_key_env: "MISTRAL_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "fireworks",
        aliases: &[],
        base_url: "https://api.fireworks.ai/inference/v1",
        api_key_env: "FIREWORKS_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "ollama",
        aliases: &[],
        base_url: "http://localhost:11434/v1",
        api_key_env: "OLLAMA_API_KEY",
        key_required: false,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "vllm",
        aliases: &[],
        base_url: "http://localhost:8000/v1",
        api_key_env: "VLLM_API_KEY",
        key_required: false,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "lmstudio",
        aliases: &[],
        base_url: "http://localhost:1234/v1",
        api_key_env: "LMSTUDIO_API_KEY",
        key_required: false,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "lemonade",
        aliases: &[],
        base_url: "http://localhost:8888/api/v1",
        api_key_env: "LEMONADE_API_KEY",
        key_required: false,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: true,
    },
    ProviderEntry {
        name: "perplexity",
        aliases: &[],
        base_url: "https://api.perplexity.ai",
        api_key_env: "PERPLEXITY_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "cohere",
        aliases: &[],
        base_url: "https://api.cohere.com/v2",
        api_key_env: "COHERE_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "ai21",
        aliases: &[],
        base_url: "https://api.ai21.com/studio/v1",
        api_key_env: "AI21_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "cerebras",
        aliases: &[],
        base_url: "https://api.cerebras.ai/v1",
        api_key_env: "CEREBRAS_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "sambanova",
        aliases: &[],
        base_url: "https://api.sambanova.ai/v1",
        api_key_env: "SAMBANOVA_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "huggingface",
        aliases: &[],
        base_url: "https://api-inference.huggingface.co/v1",
        api_key_env: "HF_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "xai",
        aliases: &[],
        base_url: "https://api.x.ai/v1",
        api_key_env: "XAI_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "replicate",
        aliases: &[],
        base_url: "https://api.replicate.com/v1",
        api_key_env: "REPLICATE_API_TOKEN",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "github-copilot",
        aliases: &["copilot"],
        base_url: "https://api.githubcopilot.com",
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
        base_url: "https://api.moonshot.ai/v1",
        api_key_env: "MOONSHOT_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "kimi_coding",
        aliases: &[],
        base_url: "https://api.kimi.com/coding",
        api_key_env: "KIMI_API_KEY",
        key_required: true,
        api_format: ApiFormat::Anthropic,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "qwen",
        aliases: &["dashscope", "model_studio"],
        base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
        api_key_env: "DASHSCOPE_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "minimax",
        aliases: &[],
        base_url: "https://api.minimax.io/v1",
        api_key_env: "MINIMAX_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "zhipu",
        aliases: &["glm"],
        base_url: "https://open.bigmodel.cn/api/paas/v4",
        api_key_env: "ZHIPU_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "zhipu_coding",
        aliases: &["codegeex"],
        base_url: "https://open.bigmodel.cn/api/coding/paas/v4",
        api_key_env: "ZHIPU_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "zai",
        aliases: &["z.ai"],
        base_url: "https://api.z.ai/api/paas/v4",
        api_key_env: "ZHIPU_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "zai_coding",
        aliases: &[],
        base_url: "https://api.z.ai/api/coding/paas/v4",
        api_key_env: "ZHIPU_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: true,
    },
    ProviderEntry {
        name: "qianfan",
        aliases: &["baidu"],
        base_url: "https://qianfan.baidubce.com/v2",
        api_key_env: "QIANFAN_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "volcengine",
        aliases: &["doubao"],
        base_url: "https://ark.cn-beijing.volces.com/api/v3",
        api_key_env: "VOLCENGINE_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "volcengine_coding",
        aliases: &[],
        base_url: "https://ark.cn-beijing.volces.com/api/coding/v3",
        api_key_env: "VOLCENGINE_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: true,
    },
    ProviderEntry {
        name: "alibaba-coding-plan",
        aliases: &[],
        base_url: "https://coding-intl.dashscope.aliyuncs.com/v1",
        api_key_env: "ALIBABA_CODING_PLAN_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "chutes",
        aliases: &[],
        base_url: "https://llm.chutes.ai/v1",
        api_key_env: "CHUTES_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "venice",
        aliases: &[],
        base_url: "https://api.venice.ai/api/v1",
        api_key_env: "VENICE_API_KEY",
        key_required: true,
        api_format: ApiFormat::OpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "azure-openai",
        aliases: &["azure"],
        base_url: "", // Constructed dynamically from endpoint + deployment
        api_key_env: "AZURE_OPENAI_API_KEY",
        key_required: true,
        api_format: ApiFormat::AzureOpenAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "vertex-ai",
        aliases: &["vertex", "vertex_ai"],
        base_url: "https://us-central1-aiplatform.googleapis.com",
        api_key_env: "GOOGLE_APPLICATION_CREDENTIALS",
        key_required: true, // Requires Google auth, but create_driver handles OAuth flows separately.
        api_format: ApiFormat::VertexAI,
        alt_api_key_env: None,
        hidden: false,
    },
    ProviderEntry {
        name: "nvidia-nim",
        aliases: &["nvidia", "nim"],
        base_url: "https://integrate.api.nvidia.com/v1",
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
        if let Some(codex_key) = read_codex_credential() {
            api_key = codex_key;
        }
    }

    if entry.key_required && entry.api_format != ApiFormat::VertexAI && api_key.is_empty() {
        return Err(LlmError::MissingApiKey(format!(
            "Set {} environment variable for provider '{}'",
            entry.api_key_env, config.provider
        )));
    }

    let proxy_url = config.proxy_url.as_deref();

    match entry.api_format {
        ApiFormat::OpenAI => Ok(Arc::new(openai::OpenAIDriver::with_proxy(
            api_key, base_url, proxy_url,
        ))),
        ApiFormat::Anthropic => Ok(Arc::new(anthropic::AnthropicDriver::with_proxy(
            api_key, base_url, proxy_url,
        ))),
        ApiFormat::Gemini => Ok(Arc::new(gemini::GeminiDriver::with_proxy(
            api_key, base_url, proxy_url,
        ))),
        ApiFormat::ClaudeCode => {
            let mut d = claude_code::ClaudeCodeDriver::with_timeout(
                config.base_url.clone(),
                config.skip_permissions,
                config.message_timeout_secs,
            );
            if let Some(bridge) = config.mcp_bridge.clone() {
                d = d.with_mcp_bridge(bridge);
            }
            Ok(Arc::new(d))
        }
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
        ApiFormat::ChatGpt => Ok(Arc::new(chatgpt::ChatGptDriver::with_proxy(
            api_key, base_url, proxy_url,
        ))),
        ApiFormat::Copilot => Ok(Arc::new(copilot::CopilotDriver::new(api_key, base_url))),
        ApiFormat::VertexAI => Ok(Arc::new(vertex_ai::VertexAiDriver::new(config)?)),
        ApiFormat::AzureOpenAI => {
            let azure = &config.azure_openai;
            let endpoint = azure
                .endpoint
                .clone()
                .or_else(|| config.base_url.clone())
                .or_else(|| std::env::var("AZURE_OPENAI_ENDPOINT").ok())
                .ok_or_else(|| LlmError::Api {
                    status: 0,
                    message: "Azure OpenAI requires an endpoint. Set [azure_openai] endpoint \
                                  in config.toml, or AZURE_OPENAI_ENDPOINT env var."
                        .to_string(),
                })?;
            let deployment = azure
                .deployment
                .clone()
                .or_else(|| std::env::var("AZURE_OPENAI_DEPLOYMENT").ok())
                .unwrap_or_default(); // empty deployment will use model name at request time
            let api_version = azure
                .api_version
                .clone()
                .or_else(|| std::env::var("AZURE_OPENAI_API_VERSION").ok())
                .unwrap_or_else(|| "2024-02-01".to_string());
            Ok(Arc::new(openai::OpenAIDriver::new_azure_with_proxy(
                api_key,
                endpoint,
                deployment,
                api_version,
                proxy_url,
            )))
        }
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
/// - `deepinfra` — DeepInfra (OpenAI-compatible inference)
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
/// - `azure-openai` — Azure OpenAI Service (deployment-based URL, `api-key` header)
/// - `vertex-ai` — Google Cloud Vertex AI (OAuth2 auth, enterprise Gemini)
/// - `qwen` — Qwen / DashScope (use `provider_regions` for intl/us endpoints)
/// - Any custom provider with `base_url` set uses OpenAI-compatible format
pub fn create_driver(config: &DriverConfig) -> Result<Arc<dyn LlmDriver>, LlmError> {
    let provider = config.provider.as_str();

    // UAR driver — feature-gated. Wraps liter-llm's `provider/model` client so
    // a single agent manifest can target 142+ providers via UAR. The model
    // string must use `provider/model` form (e.g. `openai/gpt-4o`).
    #[cfg(feature = "uar-driver")]
    if provider == "uar" {
        return uar::UarDriver::create(config);
    }

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
        return Ok(Arc::new(openai::OpenAIDriver::with_proxy(
            api_key,
            base_url.clone(),
            config.proxy_url.as_deref(),
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
             deepseek, deepinfra, together, mistral, fireworks, ollama, vllm, lmstudio, perplexity, \
             cohere, ai21, cerebras, sambanova, huggingface, xai, replicate, github-copilot, \
             chutes, venice, azure-openai, vertex-ai, nvidia-nim, codex, claude-code, qwen-code, \
             gemini-cli, codex-cli, aider, qwen, minimax, zhipu. \
             Or set base_url for a custom OpenAI-compatible endpoint.",
            provider
        ),
    })
}

/// Detect the first available provider by scanning environment variables.
///
/// Returns `(provider, model, api_key_env)` for the first provider that has a
/// configured API key, checked in a user-friendly priority order.
/// Note: `model` is always `""` — callers should resolve the default model
/// via `ModelCatalog`.
pub fn detect_available_provider() -> Option<(&'static str, &'static str, &'static str)> {
    // Priority order: popular cloud providers are checked first so that
    // users with multiple keys get the most common one by default.
    const PRIORITY: &[&str] = &[
        "openai",
        "anthropic",
        "gemini",
        "groq",
        "deepseek",
        "openrouter",
        "mistral",
        "together",
        "fireworks",
        "xai",
        "perplexity",
        "cohere",
        "azure-openai",
    ];

    let env_set =
        |var: &str| -> bool { std::env::var(var).ok().filter(|v| !v.is_empty()).is_some() };

    // Phase 1: check priority providers in order
    for &name in PRIORITY {
        if let Some(p) = PROVIDER_REGISTRY.iter().find(|p| p.name == name) {
            if p.key_required && env_set(p.api_key_env) {
                return Some((p.name, "", p.api_key_env));
            }
            if let Some(alt) = p.alt_api_key_env {
                if env_set(alt) {
                    return Some((p.name, "", alt));
                }
            }
        }
    }

    // Phase 2: check remaining registry providers not in priority list
    for p in PROVIDER_REGISTRY {
        if p.hidden || !p.key_required {
            continue;
        }
        if PRIORITY.contains(&p.name) {
            continue;
        }
        if env_set(p.api_key_env) {
            return Some((p.name, "", p.api_key_env));
        }
        if let Some(alt) = p.alt_api_key_env {
            if env_set(alt) {
                return Some((p.name, "", alt));
            }
        }
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

/// Check whether any of the given env vars redirect traffic away from official
/// API hosts. Returns `true` when a proxy/non-official endpoint is detected.
///
/// CLI providers inherit environment variables (e.g. `ANTHROPIC_BASE_URL`)
/// that can silently redirect all requests to a third-party proxy. When that
/// happens the provider should not appear as "configured".
///
/// `env_vars` — env var names to check (first non-empty wins).
/// `official_hosts` — substrings that identify the official API (e.g.
/// `"api.anthropic.com"`). If the env var value contains none of them, the
/// provider is considered proxied.
pub fn is_proxied_via_env(env_vars: &[&str], official_hosts: &[&str]) -> bool {
    for var in env_vars {
        if let Ok(val) = std::env::var(var) {
            let val = val.trim().trim_end_matches('/').to_lowercase();
            if val.is_empty() {
                continue;
            }
            return !official_hosts.iter().any(|host| val.contains(host));
        }
    }
    false
}

/// Check if a provider name refers to a CLI-subprocess-based provider.
pub fn is_cli_provider(name: &str) -> bool {
    matches!(
        name,
        "claude-code" | "qwen-code" | "gemini-cli" | "codex-cli" | "aider"
    )
}

/// Resolve the API key for a provider by checking all known sources:
/// primary env var → alt env var → Codex credential file (for openai).
///
/// Returns `None` if no key is found through any source.
pub fn resolve_provider_api_key(provider: &str) -> Option<String> {
    let entry = find_provider(provider)?;
    let non_empty = |v: String| if v.trim().is_empty() { None } else { Some(v) };

    std::env::var(entry.api_key_env)
        .ok()
        .and_then(non_empty)
        .or_else(|| {
            entry
                .alt_api_key_env
                .and_then(|v| std::env::var(v).ok())
                .and_then(non_empty)
        })
        .or_else(|| {
            if entry.name == "openai" {
                read_codex_credential()
            } else {
                None
            }
        })
}

/// Read an OpenAI API key from the Codex CLI credential file.
///
/// Checks `$CODEX_HOME/auth.json` or `~/.codex/auth.json`.
/// Returns `Some(api_key)` if the file exists and contains a valid, non-expired token.
fn read_codex_credential() -> Option<String> {
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

    if let Some(expires_at) = parsed.get("expires_at").and_then(|v| v.as_i64()) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        if now >= expires_at {
            return None;
        }
    }

    parsed
        .get("api_key")
        .or_else(|| parsed.get("token"))
        .or_else(|| parsed.get("tokens").and_then(|t| t.get("id_token")))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
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
            azure_openai: librefang_types::config::AzureOpenAiConfig::default(),
            skip_permissions: true,
            message_timeout_secs: 300,
            mcp_bridge: None,
            proxy_url: None,
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
            azure_openai: librefang_types::config::AzureOpenAiConfig::default(),
            skip_permissions: true,
            message_timeout_secs: 300,
            mcp_bridge: None,
            proxy_url: None,
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
        assert!(providers.contains(&"zhipu"));
        assert!(providers.contains(&"zhipu_coding"));
        assert!(providers.contains(&"zai"));
        assert!(providers.contains(&"kimi_coding"));
        assert!(providers.contains(&"qianfan"));
        assert!(providers.contains(&"volcengine"));
        assert!(providers.contains(&"alibaba-coding-plan"));
        assert!(providers.contains(&"deepinfra"));
        assert!(providers.contains(&"chutes"));
        assert!(providers.contains(&"claude-code"));
        assert!(providers.contains(&"qwen-code"));
        assert!(providers.contains(&"gemini-cli"));
        assert!(providers.contains(&"codex-cli"));
        assert!(providers.contains(&"aider"));
        assert!(providers.contains(&"azure-openai"));
        assert!(providers.contains(&"vertex-ai"));
        assert!(providers.contains(&"nvidia-nim"));
        assert_eq!(providers.len(), 43);
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
    fn test_provider_defaults_alibaba_coding_plan() {
        let d = provider_defaults("alibaba-coding-plan").unwrap();
        assert_eq!(d.base_url, "https://coding-intl.dashscope.aliyuncs.com/v1");
        assert_eq!(d.api_key_env, "ALIBABA_CODING_PLAN_API_KEY");
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
            azure_openai: librefang_types::config::AzureOpenAiConfig::default(),
            skip_permissions: true,
            message_timeout_secs: 300,
            mcp_bridge: None,
            proxy_url: None,
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
        // Use a synthetic provider name to avoid env-var races with other tests
        // that set NVIDIA_API_KEY (e.g. test_custom_provider_convention_env_var).
        let config = DriverConfig {
            provider: "nonexistent-provider-for-test".to_string(),
            api_key: None,
            base_url: None,
            vertex_ai: librefang_types::config::VertexAiConfig::default(),
            azure_openai: librefang_types::config::AzureOpenAiConfig::default(),
            skip_permissions: true,
            message_timeout_secs: 300,
            mcp_bridge: None,
            proxy_url: None,
        };
        let driver = create_driver(&config);
        assert!(driver.is_err());
    }

    #[test]
    fn test_custom_provider_key_no_url_helpful_error() {
        // Unknown custom provider with key set (via env) but no base_url should give helpful
        // error. Use a synthetic provider name that is not in the registry so the test is
        // not broken when well-known providers are added (e.g. "nvidia" is now a registry alias).
        let provider_name = "my-custom-llm-provider";
        let env_var = "MY_CUSTOM_LLM_PROVIDER_API_KEY";
        let unique_key = "test-custom-key-67890";
        std::env::set_var(env_var, unique_key);
        let config = DriverConfig {
            provider: provider_name.to_string(),
            api_key: None,
            base_url: None,
            vertex_ai: librefang_types::config::VertexAiConfig::default(),
            azure_openai: librefang_types::config::AzureOpenAiConfig::default(),
            skip_permissions: true,
            message_timeout_secs: 300,
            mcp_bridge: None,
            proxy_url: None,
        };
        let result = create_driver(&config);
        assert!(result.is_err());
        let err = result.err().unwrap().to_string();
        assert!(
            err.contains("base_url"),
            "Error should mention base_url: {}",
            err
        );
        std::env::remove_var(env_var);
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
            azure_openai: librefang_types::config::AzureOpenAiConfig::default(),
            skip_permissions: true,
            message_timeout_secs: 300,
            mcp_bridge: None,
            proxy_url: None,
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
            azure_openai: librefang_types::config::AzureOpenAiConfig::default(),
            skip_permissions: true,
            message_timeout_secs: 300,
            mcp_bridge: None,
            proxy_url: None,
        };

        let driver = create_driver(&config);
        assert!(
            driver.is_ok(),
            "Vertex AI driver should initialize from [vertex_ai] config without env vars"
        );
    }

    #[test]
    fn test_azure_openai_provider_lookup() {
        let d = provider_defaults("azure-openai").unwrap();
        assert_eq!(d.api_key_env, "AZURE_OPENAI_API_KEY");
        assert!(d.key_required);
    }

    #[test]
    fn test_azure_openai_alias() {
        let d = provider_defaults("azure").unwrap();
        assert_eq!(d.api_key_env, "AZURE_OPENAI_API_KEY");
        assert!(d.key_required);
    }

    #[test]
    fn test_azure_openai_driver_creation() {
        let config = DriverConfig {
            provider: "azure-openai".to_string(),
            api_key: Some("test-azure-key".to_string()),
            base_url: None,
            vertex_ai: librefang_types::config::VertexAiConfig::default(),
            azure_openai: librefang_types::config::AzureOpenAiConfig {
                endpoint: Some("https://my-resource.openai.azure.com".to_string()),
                deployment: Some("gpt-4o".to_string()),
                api_version: Some("2024-02-01".to_string()),
            },
            skip_permissions: true,
            message_timeout_secs: 300,
            mcp_bridge: None,
            proxy_url: None,
        };
        let driver = create_driver(&config);
        assert!(
            driver.is_ok(),
            "Azure OpenAI driver should create successfully with config"
        );
    }

    #[test]
    fn test_azure_openai_missing_endpoint_errors() {
        let config = DriverConfig {
            provider: "azure-openai".to_string(),
            api_key: Some("test-azure-key".to_string()),
            base_url: None,
            vertex_ai: librefang_types::config::VertexAiConfig::default(),
            azure_openai: librefang_types::config::AzureOpenAiConfig::default(),
            skip_permissions: true,
            message_timeout_secs: 300,
            mcp_bridge: None,
            proxy_url: None,
        };
        // Clear any env var that might interfere
        std::env::remove_var("AZURE_OPENAI_ENDPOINT");
        let driver = create_driver(&config);
        assert!(
            driver.is_err(),
            "Azure OpenAI should error without endpoint"
        );
        let err = driver.err().unwrap().to_string();
        assert!(
            err.contains("endpoint"),
            "Error should mention endpoint: {}",
            err
        );
    }

    #[test]
    fn test_driver_cache_returns_same_arc() {
        let cache = DriverCache::new();
        let config = DriverConfig {
            provider: "ollama".to_string(),
            api_key: None,
            base_url: Some("http://localhost:11434/v1".to_string()),
            vertex_ai: librefang_types::config::VertexAiConfig::default(),
            azure_openai: librefang_types::config::AzureOpenAiConfig::default(),
            skip_permissions: true,
            message_timeout_secs: 300,
            mcp_bridge: None,
            proxy_url: None,
        };
        let d1 = cache.get_or_create(&config).unwrap();
        let d2 = cache.get_or_create(&config).unwrap();
        assert!(Arc::ptr_eq(&d1, &d2), "Cache should return the same Arc");
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn test_driver_cache_different_keys_produce_different_drivers() {
        let cache = DriverCache::new();
        let config_a = DriverConfig {
            provider: "ollama".to_string(),
            api_key: Some("key-a".to_string()),
            base_url: Some("http://localhost:11434/v1".to_string()),
            vertex_ai: librefang_types::config::VertexAiConfig::default(),
            azure_openai: librefang_types::config::AzureOpenAiConfig::default(),
            skip_permissions: true,
            message_timeout_secs: 300,
            mcp_bridge: None,
            proxy_url: None,
        };
        let config_b = DriverConfig {
            provider: "ollama".to_string(),
            api_key: Some("key-b".to_string()),
            base_url: Some("http://localhost:11434/v1".to_string()),
            vertex_ai: librefang_types::config::VertexAiConfig::default(),
            azure_openai: librefang_types::config::AzureOpenAiConfig::default(),
            skip_permissions: true,
            message_timeout_secs: 300,
            mcp_bridge: None,
            proxy_url: None,
        };
        let d_a = cache.get_or_create(&config_a).unwrap();
        let d_b = cache.get_or_create(&config_b).unwrap();
        assert!(
            !Arc::ptr_eq(&d_a, &d_b),
            "Different keys should produce different drivers"
        );
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn test_driver_cache_clear() {
        let cache = DriverCache::new();
        let config = DriverConfig {
            provider: "ollama".to_string(),
            api_key: None,
            base_url: Some("http://localhost:11434/v1".to_string()),
            vertex_ai: librefang_types::config::VertexAiConfig::default(),
            azure_openai: librefang_types::config::AzureOpenAiConfig::default(),
            skip_permissions: true,
            message_timeout_secs: 300,
            mcp_bridge: None,
            proxy_url: None,
        };
        cache.get_or_create(&config).unwrap();
        assert_eq!(cache.len(), 1);
        cache.clear();
        assert!(cache.is_empty());
    }
}
