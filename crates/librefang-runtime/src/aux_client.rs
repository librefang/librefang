//! Auxiliary LLM client — cheap-tier fallback chains for side tasks.
//!
//! This module addresses issue #3314: side tasks in LibreFang (context
//! compression, session-title generation, search summarisation, vision
//! captioning, browser-vision page understanding) historically run on the
//! same model the agent is configured with. That means a user running
//! Opus pays Opus rates to summarise their conversation history into a
//! 4 k-token blurb, and a user running a tiny local model has no fallback
//! when compression demands more capability than `qwen:0.5b` can provide.
//!
//! [`AuxClient`] resolves a per-task [`FallbackChain`] composed of cheap-
//! tier providers declared in `[llm.auxiliary]`. The same `FallbackChain`
//! engine that powers the primary path (rate-limit retries, credit-
//! exhaustion failover, auth-error skip) is reused here — there is **no
//! new fallback engine**, only a new chain composition rule.
//!
//! # Resolution algorithm
//!
//! 1. Look up `[llm.auxiliary]` for `task` in [`AuxiliaryConfig`].
//! 2. If empty, fall back to a built-in published default chain.
//! 3. For each `provider:model` reference, attempt to construct a driver
//!    using the user's already-configured credentials (env vars or
//!    `[provider_api_keys]` overrides). Skip silently when credentials are
//!    missing — exactly the same way [`crate::drivers::create_driver`]
//!    behaves elsewhere.
//! 4. If every entry was skipped, fall through to the caller-supplied
//!    primary driver. The aux client is a routing optimisation, never a
//!    permission gate.
//!
//! # Cost accounting
//!
//! All aux calls still flow through the same driver objects the kernel
//! constructed via [`librefang_llm_drivers::drivers::create_driver`], which
//! means the metering layer sees them. The aux client never bypasses the
//! billing pipeline — it just picks a cheaper model.

use librefang_llm_driver::{DriverConfig, LlmDriver};
use librefang_llm_drivers::drivers::{
    create_driver,
    fallback_chain::{ChainEntry, FallbackChain},
};
use librefang_types::config::{AuxTask, AuxiliaryConfig, KernelConfig};
use std::sync::Arc;
use tracing::{debug, warn};

/// Auxiliary LLM client: resolves a [`FallbackChain`] per [`AuxTask`].
///
/// Construct once at kernel boot and share via `Arc<AuxClient>`. The struct
/// is `Send + Sync`; resolution is cheap (driver instances are cached on
/// the kernel-supplied [`librefang_llm_drivers::drivers::DriverCache`]
/// when one is wired through, or built ad-hoc otherwise).
#[derive(Clone)]
#[allow(missing_debug_implementations)]
pub struct AuxClient {
    /// User-supplied per-task chain configuration.
    config: AuxiliaryConfig,
    /// Snapshot of the kernel config — needed to resolve provider env-var
    /// names, base URL overrides, proxy settings, and provider-specific
    /// auth (Vertex AI, Azure OpenAI). Cloned at construction time; if the
    /// kernel hot-reloads its config it must rebuild the [`AuxClient`].
    kernel_config: Arc<KernelConfig>,
    /// Fallback driver used when no aux entry could be initialised. This
    /// is normally the primary driver chain so callers see no change in
    /// behaviour relative to the pre-aux baseline.
    primary: Arc<dyn LlmDriver>,
}

impl AuxClient {
    /// Build a new auxiliary client from a kernel config snapshot.
    ///
    /// `primary` is the driver returned to callers when no auxiliary entry
    /// can be initialised for the requested task. Pass the kernel's
    /// already-constructed primary fallback driver so behaviour matches
    /// the pre-aux baseline.
    pub fn new(config: Arc<KernelConfig>, primary: Arc<dyn LlmDriver>) -> Self {
        Self {
            config: config.llm.auxiliary.clone(),
            kernel_config: config,
            primary,
        }
    }

    /// Build an aux client without a kernel config — used by tests and the
    /// fallback path inside the context compressor when the surrounding
    /// component was constructed before kernel boot completed.
    ///
    /// Every task resolves directly to `primary`.
    pub fn with_primary_only(primary: Arc<dyn LlmDriver>) -> Self {
        Self {
            config: AuxiliaryConfig::empty(),
            kernel_config: Arc::new(KernelConfig::default()),
            primary,
        }
    }

    /// Resolve the chain for `task`.
    ///
    /// Returns an `Arc<dyn LlmDriver>` that callers invoke exactly like the
    /// primary driver. The returned object is either a [`FallbackChain`]
    /// composed of cheap providers, or — when no aux entry could be
    /// initialised — a clone of the primary driver `Arc`.
    ///
    /// Also returns a slice of `(provider, model)` pairs describing the
    /// resolved chain for logging / debugging. When the slice is empty the
    /// caller is talking to the primary driver, not an aux chain.
    pub fn resolve(&self, task: AuxTask) -> AuxResolution {
        let raw = match self.config.chain_for(task) {
            Some(chain) if !chain.is_empty() => chain.to_vec(),
            _ => self.default_chain(task),
        };

        if raw.is_empty() {
            debug!(task = %task, "AuxClient: no chain configured, using primary driver");
            return AuxResolution {
                driver: Arc::clone(&self.primary),
                resolved: Vec::new(),
                used_primary: true,
            };
        }

        let mut entries: Vec<ChainEntry> = Vec::with_capacity(raw.len());
        let mut resolved_pairs: Vec<(String, String)> = Vec::with_capacity(raw.len());

        for spec in &raw {
            let Some((provider, model)) = parse_spec(spec) else {
                warn!(spec, "AuxClient: malformed entry, skipping");
                continue;
            };

            match self.build_driver(&provider) {
                Ok(driver) => {
                    let model_resolved = resolve_model_alias(&provider, &model);
                    debug!(task = %task, %provider, model = %model_resolved, "AuxClient: chain entry resolved");
                    entries.push(ChainEntry {
                        driver,
                        model_override: model_resolved.clone(),
                        provider_name: provider.clone(),
                    });
                    resolved_pairs.push((provider, model_resolved));
                }
                Err(reason) => {
                    debug!(task = %task, %provider, %reason, "AuxClient: chain entry skipped");
                }
            }
        }

        if entries.is_empty() {
            debug!(task = %task, "AuxClient: every aux entry skipped, falling back to primary");
            return AuxResolution {
                driver: Arc::clone(&self.primary),
                resolved: Vec::new(),
                used_primary: true,
            };
        }

        let chain: Arc<dyn LlmDriver> = Arc::new(FallbackChain::new(entries));
        AuxResolution {
            driver: chain,
            resolved: resolved_pairs,
            used_primary: false,
        }
    }

    /// Convenience: return just the driver. Most call sites only need this.
    pub fn driver_for(&self, task: AuxTask) -> Arc<dyn LlmDriver> {
        self.resolve(task).driver
    }

    /// Default chain for `task` when the user has not configured `[llm.auxiliary]`.
    ///
    /// These defaults are intentionally conservative — the aux client only
    /// engages when the named provider has its API key set in the user's
    /// environment, otherwise the entry is silently skipped and we fall
    /// through to the primary driver. That preserves behaviour for users
    /// who haven't opted in (no `OPENROUTER_API_KEY` set → no aux call).
    ///
    /// Aliases (`sonnet`, `haiku`, `gpt-4o`) are used rather than concrete
    /// model IDs so catalog drift in the model registry doesn't break this
    /// list. Provider drivers expand the alias before sending the request.
    fn default_chain(&self, task: AuxTask) -> Vec<String> {
        match task {
            // Compression and fold are high-volume side tasks. Cheap haiku-class
            // models are good enough; OpenRouter is preferred because most
            // adopters of "auxiliary cheap tier" already have a key there.
            AuxTask::Compression | AuxTask::Title | AuxTask::Search | AuxTask::Fold => vec![
                "openrouter:anthropic/claude-3-5-haiku".to_string(),
                "anthropic:haiku".to_string(),
                "openai:gpt-4o-mini".to_string(),
            ],
            // Vision-capable models are scarce; only providers with
            // first-class multimodal support are listed.
            AuxTask::Vision | AuxTask::BrowserVision => vec![
                "anthropic:sonnet".to_string(),
                "openai:gpt-4o-mini".to_string(),
                "openrouter:anthropic/claude-3-5-sonnet".to_string(),
            ],
        }
    }

    /// Construct a driver for `provider` using the user's existing config.
    ///
    /// Returns `Err` when the provider has no API key in the user's env or
    /// `[provider_api_keys]` mapping (and isn't a local provider) — the
    /// caller treats that as "skip this slot, try the next one".
    fn build_driver(&self, provider: &str) -> Result<Arc<dyn LlmDriver>, String> {
        let api_key = self.resolve_api_key(provider);

        let driver_cfg = DriverConfig {
            provider: provider.to_string(),
            api_key,
            base_url: self.kernel_config.provider_urls.get(provider).cloned(),
            vertex_ai: self.kernel_config.vertex_ai.clone(),
            azure_openai: self.kernel_config.azure_openai.clone(),
            skip_permissions: true,
            message_timeout_secs: self.kernel_config.default_model.message_timeout_secs,
            mcp_bridge: None,
            proxy_url: self
                .kernel_config
                .provider_proxy_urls
                .get(provider)
                .cloned(),
            request_timeout_secs: self
                .kernel_config
                .provider_request_timeout_secs
                .get(provider)
                .copied(),
            emit_caller_trace_headers: self.kernel_config.telemetry.emit_caller_trace_headers,
        };

        create_driver(&driver_cfg).map_err(|e| e.to_string())
    }

    /// Resolve the API key for `provider`. `None` for local providers
    /// (ollama, vllm, lmstudio) is fine — `create_driver` accepts an empty
    /// key for those. For cloud providers, returning `None` here means
    /// `create_driver` will see no key and most likely fail; the caller
    /// then skips the slot.
    fn resolve_api_key(&self, provider: &str) -> Option<String> {
        let env_var = self.kernel_config.resolve_api_key_env(provider);
        if env_var.is_empty() {
            return None;
        }
        std::env::var(&env_var).ok().filter(|v| !v.is_empty())
    }
}

impl std::fmt::Debug for AuxClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuxClient")
            .field(
                "configured_tasks",
                &self.config.tasks.keys().collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// Outcome of [`AuxClient::resolve`].
#[derive(Clone)]
pub struct AuxResolution {
    /// Driver to call for this side task.
    pub driver: Arc<dyn LlmDriver>,
    /// `(provider, model)` pairs in chain order. Empty when `used_primary`
    /// is true.
    pub resolved: Vec<(String, String)>,
    /// True when no aux entry could be initialised and the primary driver
    /// is being used as the chain.
    pub used_primary: bool,
}

/// Parse a `provider:model` spec.  Returns `None` on malformed input.
///
/// Supports models that themselves contain `/` (e.g.
/// `openrouter:anthropic/claude-3-5-haiku`) — only the first `:` is the
/// provider/model separator.
fn parse_spec(spec: &str) -> Option<(String, String)> {
    let (provider, model) = spec.split_once(':')?;
    let provider = provider.trim();
    let model = model.trim();
    if provider.is_empty() || model.is_empty() {
        return None;
    }
    Some((provider.to_string(), model.to_string()))
}

/// Expand short aliases to a canonical model slug per provider so users can
/// write `anthropic:sonnet` without pinning a specific dated revision.
///
/// Unknown aliases are returned unchanged — the underlying driver will
/// either accept the model name as-is or surface a `ModelUnavailable`
/// error that triggers chain failover.
fn resolve_model_alias(provider: &str, model: &str) -> String {
    match (provider, model) {
        ("anthropic", "sonnet") => "claude-3-5-sonnet-latest".to_string(),
        ("anthropic", "haiku") => "claude-3-5-haiku-latest".to_string(),
        ("anthropic", "opus") => "claude-3-opus-latest".to_string(),
        ("openai", "gpt-4o") => "gpt-4o".to_string(),
        ("openai", "gpt-4o-mini") => "gpt-4o-mini".to_string(),
        _ => model.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm_driver::{
        CompletionRequest, CompletionResponse, LlmDriver as LlmDriverTrait, LlmError, StreamEvent,
    };
    use async_trait::async_trait;
    use librefang_types::config::AuxiliaryConfig;
    use librefang_types::message::{ContentBlock, StopReason, TokenUsage};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct MarkerDriver(&'static str, AtomicUsize);

    impl MarkerDriver {
        fn new(label: &'static str) -> Arc<Self> {
            Arc::new(Self(label, AtomicUsize::new(0)))
        }
    }

    #[async_trait]
    impl LlmDriverTrait for MarkerDriver {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            self.1.fetch_add(1, Ordering::SeqCst);
            Ok(CompletionResponse {
                content: vec![ContentBlock::Text {
                    text: self.0.to_string(),
                    provider_metadata: None,
                }],
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
                usage: TokenUsage::default(),
                actual_provider: None,
            })
        }

        async fn stream(
            &self,
            req: CompletionRequest,
            _tx: tokio::sync::mpsc::Sender<StreamEvent>,
        ) -> Result<CompletionResponse, LlmError> {
            self.complete(req).await
        }
    }

    /// Empty config + a primary driver → every task hits the primary.
    #[tokio::test]
    async fn empty_config_falls_through_to_primary() {
        let primary = MarkerDriver::new("primary");
        let primary_calls = Arc::clone(&primary);

        let mut cfg = KernelConfig::default();
        cfg.llm.auxiliary = AuxiliaryConfig::empty();

        let aux = AuxClient::new(Arc::new(cfg), primary);
        let resolution = aux.resolve(AuxTask::Compression);
        // No env keys are set in CI for OpenRouter / Anthropic / OpenAI, so
        // the default chain entries get skipped and we fall through to the
        // primary driver. `used_primary` is the load-bearing assertion.
        assert!(
            resolution.used_primary,
            "no aux entries should be initialised in a clean test env"
        );

        let req = CompletionRequest {
            model: "test".to_string(),
            messages: std::sync::Arc::new(vec![]),
            tools: std::sync::Arc::new(vec![]),
            max_tokens: 32,
            temperature: 0.0,
            system: None,
            thinking: None,
            prompt_caching: false,
            cache_ttl: None,
            response_format: None,
            timeout_secs: None,
            extra_body: None,
            agent_id: None,
            session_id: None,
            step_id: None,
        };
        resolution.driver.complete(req).await.unwrap();
        assert_eq!(primary_calls.1.load(Ordering::SeqCst), 1);
    }

    /// Misconfigured aux entries (unknown provider, no base_url) get
    /// skipped silently and resolution falls back to the primary driver.
    #[test]
    fn malformed_chain_falls_back_to_primary() {
        let primary = MarkerDriver::new("primary");
        let mut cfg = KernelConfig::default();
        cfg.llm.auxiliary.tasks.insert(
            AuxTask::Title,
            vec![
                "definitely-not-a-real-provider:foo".to_string(),
                "another-bogus:bar".to_string(),
            ],
        );

        let aux = AuxClient::new(Arc::new(cfg), primary);
        let resolution = aux.resolve(AuxTask::Title);
        assert!(resolution.used_primary, "all entries should fail to init");
        assert!(resolution.resolved.is_empty());
    }

    /// `provider:model` parser handles model strings containing `/`.
    #[test]
    fn parse_spec_handles_slashed_model() {
        let (p, m) = parse_spec("openrouter:anthropic/claude-3-5-haiku").unwrap();
        assert_eq!(p, "openrouter");
        assert_eq!(m, "anthropic/claude-3-5-haiku");
    }

    #[test]
    fn parse_spec_rejects_empty_sides() {
        assert!(parse_spec(":foo").is_none());
        assert!(parse_spec("foo:").is_none());
        assert!(parse_spec("noproto").is_none());
    }

    #[test]
    fn alias_resolution_expands_known_aliases() {
        assert_eq!(
            resolve_model_alias("anthropic", "sonnet"),
            "claude-3-5-sonnet-latest"
        );
        assert_eq!(
            resolve_model_alias("anthropic", "haiku"),
            "claude-3-5-haiku-latest"
        );
        // Unknown aliases pass through unchanged.
        assert_eq!(
            resolve_model_alias("anthropic", "claude-9001"),
            "claude-9001"
        );
        // Unknown provider passes through unchanged.
        assert_eq!(resolve_model_alias("nvidia", "nemotron"), "nemotron");
    }

    #[test]
    fn config_default_chain_covers_all_tasks() {
        let cfg = KernelConfig::default();
        let primary = MarkerDriver::new("primary");
        let aux = AuxClient::new(Arc::new(cfg), primary);
        // None of the default-chain providers should be present in CI env,
        // but the resolver must still produce a non-panicking result for
        // every task variant.
        for task in [
            AuxTask::Compression,
            AuxTask::Title,
            AuxTask::Search,
            AuxTask::Vision,
            AuxTask::BrowserVision,
        ] {
            let res = aux.resolve(task);
            assert!(
                res.used_primary,
                "task {task} should have fallen through to primary in CI env"
            );
        }
    }

    #[test]
    fn with_primary_only_always_returns_primary() {
        let primary = MarkerDriver::new("primary");
        let aux = AuxClient::with_primary_only(primary);
        for task in [AuxTask::Compression, AuxTask::Vision, AuxTask::Title] {
            let res = aux.resolve(task);
            assert!(res.used_primary);
            assert!(res.resolved.is_empty());
        }
    }
}
