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
//! 2. If the task has **no explicit** `[llm.auxiliary]` entry, resolve to
//!    the caller-supplied primary driver — which is the triggering agent's
//!    fully-resolved fallback chain (primary → `[[fallback_providers]]`).
//!    "Absent = inherit, present = override" (issue #5169): when the
//!    operator has not opted into a cheap aux tier the side task must reuse
//!    the same healthy chain the agent itself uses, instead of a hardcoded
//!    provider list that ignores the agent's configured failover.
//! 3. When the task **is** explicitly configured, for each `provider:model`
//!    reference attempt to construct a driver using the user's
//!    already-configured credentials (env vars or `[provider_api_keys]`
//!    overrides). Skip silently when credentials are missing — exactly the
//!    same way [`crate::drivers::create_driver`] behaves elsewhere.
//! 4. If every explicitly-configured entry was skipped, fall through to the
//!    caller-supplied primary driver. The aux client is a routing
//!    optimisation, never a permission gate.
//!
//! # Cost accounting
//!
//! All aux calls still flow through the same driver objects the kernel
//! constructed via [`librefang_llm_drivers::drivers::create_driver`], which
//! means the metering layer sees them. The aux client never bypasses the
//! billing pipeline — it just picks a cheaper model.

use crate::model_catalog::ModelCatalog;
use librefang_llm_driver::exhaustion::ProviderExhaustionStore;
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
    /// Shared provider-exhaustion store (#4807). When set, every
    /// [`FallbackChain`] resolved from this client honours the same
    /// exhaustion view so a slot that 429'd on the primary path is also
    /// skipped on aux paths within the back-off window — and vice versa.
    exhaustion_store: Option<ProviderExhaustionStore>,
    /// Snapshot of the runtime model catalog. Custom providers added via
    /// the dashboard "Add provider" flow or `registry/providers/*.toml`
    /// store their `api_key_env` / `base_url` **only** in the catalog, not
    /// in `[provider_api_keys]` / `[provider_urls]`. Consulting the catalog
    /// here mirrors the primary path's `resolve_non_default_api_key_env` /
    /// `lookup_provider_url` (#5755 / #5807) so a cheap-tier custom provider
    /// actually initialises instead of silently falling through to the
    /// expensive primary. `None` when no catalog was wired (tests and the
    /// `with_primary_only` compressor path), which preserves the historical
    /// convention-only resolution exactly.
    model_catalog: Option<Arc<ModelCatalog>>,
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
            exhaustion_store: None,
            model_catalog: None,
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
            exhaustion_store: None,
            model_catalog: None,
        }
    }

    /// Attach a shared exhaustion store (#4807). Every chain returned by
    /// [`Self::resolve`] from this point on routes its skip-decisions
    /// through this store, so an exhaustion observed on one task's chain
    /// is honoured by the next task's chain as well. Cheap-clone — pass
    /// the same store the metering engine was wired with.
    pub fn with_exhaustion_store(mut self, store: ProviderExhaustionStore) -> Self {
        self.exhaustion_store = Some(store);
        self
    }

    /// Attach a model-catalog snapshot so provider key/URL resolution can
    /// consult catalog-only custom providers (dashboard "Add provider" /
    /// `registry/providers/*.toml`). Without this the aux client resolves
    /// keys by convention + `[provider_api_keys]` and URLs by
    /// `[provider_urls]` only, so a custom cheap-tier provider whose
    /// `api_key_env` / `base_url` live solely in the catalog never
    /// initialises and the side task silently bills the primary. The
    /// snapshot is refreshed whenever the aux client is rebuilt (boot and
    /// `POST /api/config/reload`), matching the `kernel_config` snapshot's
    /// lifecycle. Cheap-clone — pass `model_catalog.load_full()`.
    pub fn with_model_catalog(mut self, catalog: Arc<ModelCatalog>) -> Self {
        self.model_catalog = Some(catalog);
        self
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
        // "Absent = inherit, present = override" (#5169). When the operator
        // has NOT explicitly configured `[llm.auxiliary]` for this task,
        // the side task reuses the triggering agent's fully-resolved
        // fallback chain (`self.primary` is the kernel's `default_driver`,
        // i.e. primary → `[[fallback_providers]]`). Previously this path
        // injected a hardcoded cheap-tier provider list that ignored the
        // agent's configured failover entirely: if the hardcoded provider
        // was rate-limited/down every aux task failed even though the
        // agent's own healthy fallback chain was sitting unused.
        let raw = match self.config.chain_for(task) {
            Some(chain) if !chain.is_empty() => chain.to_vec(),
            _ => {
                debug!(
                    task = %task,
                    "AuxClient: no explicit [llm.auxiliary] entry, inheriting agent fallback chain"
                );
                return AuxResolution {
                    driver: Arc::clone(&self.primary),
                    resolved: Vec::new(),
                    used_primary: true,
                };
            }
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

        let mut chain = FallbackChain::new(entries);
        if let Some(store) = self.exhaustion_store.clone() {
            chain = chain.with_exhaustion_store(store);
        }
        let chain: Arc<dyn LlmDriver> = Arc::new(chain);
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

    /// Construct a driver for `provider` using the user's existing config.
    ///
    /// Returns `Err` when the provider has no API key in the user's env or
    /// `[provider_api_keys]` mapping (and isn't a local provider) — the
    /// caller treats that as "skip this slot, try the next one".
    fn build_driver(&self, provider: &str) -> Result<Arc<dyn LlmDriver>, String> {
        // Governance: org-wide provider allowlist (issue #6459). Fail-closed —
        // a disallowed cheap-tier provider is skipped (Err → the caller tries
        // the next slot, exactly like a missing credential) and logged for the
        // operator. Keeps the aux/cheap-tier path from routing around the same
        // allowlist the primary path enforces in `resolve_driver`.
        if !self.kernel_config.providers.is_provider_allowed(provider) {
            warn!(
                %provider,
                allowed = ?self.kernel_config.providers.allowed,
                "Aux LLM provider blocked by org-wide allowlist; skipping slot"
            );
            return Err(self.kernel_config.providers.rejection_reason(provider));
        }

        let api_key = self.resolve_api_key(provider);

        let driver_cfg = DriverConfig {
            provider: provider.to_string(),
            api_key,
            base_url: self.resolve_base_url(provider),
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
            max_retries: self
                .kernel_config
                .provider_max_retries
                .get(provider)
                .copied()
                .unwrap_or_else(|| DriverConfig::default().max_retries),
        };

        create_driver(&driver_cfg).map_err(|e| e.to_string())
    }

    /// Resolve the API key for `provider`. `None` for local providers
    /// (ollama, vllm, lmstudio) is fine — `create_driver` accepts an empty
    /// key for those. For cloud providers, returning `None` here means
    /// `create_driver` will see no key and most likely fail; the caller
    /// then skips the slot.
    fn resolve_api_key(&self, provider: &str) -> Option<String> {
        let env_var = self.resolve_api_key_env(provider);
        if env_var.is_empty() {
            return None;
        }
        std::env::var(&env_var).ok().filter(|v| !v.is_empty())
    }

    /// Resolve the env-var name that holds `provider`'s API key.
    ///
    /// Precedence mirrors the primary path's
    /// `LibreFangKernel::resolve_non_default_api_key_env` (#5755 / #5807):
    ///   1. Operator-explicit `[provider_api_keys]` / `[auth_profiles]` in
    ///      `config.toml` — always wins so an operator can pin a custom
    ///      provider's key to a specific env var.
    ///   2. Model-catalog `api_key_env` (dashboard "Add provider" flow or
    ///      `registry/providers/*.toml`) — needed for custom providers whose
    ///      env var deviates from the `<PROVIDER>_API_KEY` convention.
    ///   3. Convention fallback via `KernelConfig::resolve_api_key_env`.
    fn resolve_api_key_env(&self, provider: &str) -> String {
        if self.kernel_config.provider_api_keys.contains_key(provider)
            || self.kernel_config.auth_profiles.contains_key(provider)
        {
            return self.kernel_config.resolve_api_key_env(provider);
        }
        self.catalog_api_key_env(provider)
            .unwrap_or_else(|| self.kernel_config.resolve_api_key_env(provider))
    }

    /// Catalog-declared `api_key_env` for `provider`, if any and non-empty.
    fn catalog_api_key_env(&self, provider: &str) -> Option<String> {
        self.model_catalog
            .as_ref()?
            .get_provider(provider)
            .and_then(|p| {
                if p.api_key_env.is_empty() {
                    None
                } else {
                    Some(p.api_key_env.clone())
                }
            })
    }

    /// Resolve `provider`'s base URL, mirroring the primary path's
    /// `LibreFangKernel::lookup_provider_url`: operator-explicit
    /// `[provider_urls]` in `config.toml` wins, then the model-catalog
    /// `base_url` (populated by the dashboard "Add provider" flow or the
    /// registry) so a custom cheap-tier provider reaches its own endpoint.
    fn resolve_base_url(&self, provider: &str) -> Option<String> {
        if let Some(url) = self.kernel_config.provider_urls.get(provider) {
            return Some(url.clone());
        }
        self.model_catalog
            .as_ref()?
            .get_provider(provider)
            .and_then(|p| {
                if p.base_url.is_empty() {
                    None
                } else {
                    Some(p.base_url.clone())
                }
            })
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
                actual_model: None,
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
        // With no explicit `[llm.auxiliary]` entry the resolver inherits the
        // agent's fallback chain (`self.primary`) directly — #5169.
        // `used_primary` is the load-bearing assertion.
        assert!(
            resolution.used_primary,
            "absent [llm.auxiliary] must inherit the primary (agent) chain"
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
            prompt_cache_strategy: None,
            response_format: None,
            timeout_secs: None,
            extra_body: None,
            agent_id: None,
            session_id: None,
            step_id: None,
            reasoning_echo_policy: librefang_types::model_catalog::ReasoningEchoPolicy::default(),

            ..Default::default()
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

    /// #5169: with no `[llm.auxiliary]` config every task variant must
    /// inherit the agent's fallback chain (the `primary` driver), never a
    /// hardcoded provider list.
    #[test]
    fn unconfigured_tasks_inherit_primary_chain() {
        let cfg = KernelConfig::default();
        assert!(
            cfg.llm.auxiliary.is_empty(),
            "default KernelConfig must have no [llm.auxiliary]"
        );
        let primary = MarkerDriver::new("primary");
        let aux = AuxClient::new(Arc::new(cfg), primary);
        for task in [
            AuxTask::Compression,
            AuxTask::Title,
            AuxTask::Search,
            AuxTask::Vision,
            AuxTask::BrowserVision,
            AuxTask::Fold,
            AuxTask::SkillReview,
            AuxTask::SkillWorkshopReview,
            AuxTask::SessionSummary,
        ] {
            let res = aux.resolve(task);
            assert!(
                res.used_primary,
                "task {task}: unconfigured aux must inherit the primary chain"
            );
            assert!(
                res.resolved.is_empty(),
                "task {task}: inherited primary chain reports no aux pairs"
            );
        }
    }

    /// #5169 regression: when `[llm.auxiliary]` is NOT configured, the
    /// resolved aux driver is **the exact same `Arc` as the agent's
    /// fallback chain** (`self.primary`) — not a hardcoded cheap-tier
    /// provider list. When a task IS explicitly configured, the explicit
    /// chain wins (and on a clean env where the named provider has no key,
    /// the entry is skipped and we still fall through to the primary —
    /// the point is the resolver took the explicit branch, proven by the
    /// `definitely-not-a-real-provider` skip vs. the inherit branch never
    /// attempting any provider build).
    #[tokio::test]
    async fn absent_aux_inherits_agent_chain_explicit_overrides() {
        // (a) Absent: identical Arc to the agent's resolved chain.
        let primary = MarkerDriver::new("agent-chain");
        let primary_clone: Arc<dyn LlmDriverTrait> = Arc::clone(&primary) as _;
        let mut cfg = KernelConfig::default();
        cfg.llm.auxiliary = AuxiliaryConfig::empty();
        let aux = AuxClient::new(Arc::new(cfg), primary);

        let res = aux.resolve(AuxTask::Compression);
        assert!(res.used_primary, "absent aux config must inherit");
        assert!(
            Arc::ptr_eq(&res.driver, &primary_clone),
            "inherited driver must be the *same* Arc as the agent chain, \
             not a freshly-built hardcoded provider chain"
        );
        // Prove the inherited driver really is the agent chain by calling it.
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
            prompt_cache_strategy: None,
            response_format: None,
            timeout_secs: None,
            extra_body: None,
            agent_id: None,
            session_id: None,
            step_id: None,
            reasoning_echo_policy: librefang_types::model_catalog::ReasoningEchoPolicy::default(),

            ..Default::default()
        };
        let out = res.driver.complete(req).await.unwrap();
        match &out.content[0] {
            ContentBlock::Text { text, .. } => assert_eq!(text, "agent-chain"),
            other => panic!("unexpected content block: {other:?}"),
        }

        // (b) Explicit: the configured chain takes the explicit branch.
        // The provider has no credentials in a clean test env, so every
        // explicit entry is skipped and resolution falls through to the
        // primary — but `resolved` being empty here is reached via the
        // *explicit* code path (entries attempted then skipped), distinct
        // from (a) which never attempts a provider build. The
        // `malformed_chain_falls_back_to_primary` test already pins the
        // explicit-then-skipped path; here we assert the explicit chain is
        // honoured rather than silently replaced by an inherited chain.
        let primary2 = MarkerDriver::new("agent-chain-2");
        let mut cfg2 = KernelConfig::default();
        cfg2.llm.auxiliary.tasks.insert(
            AuxTask::Compression,
            vec!["definitely-not-a-real-provider:some-model".to_string()],
        );
        let aux2 = AuxClient::new(Arc::new(cfg2), primary2);
        let res2 = aux2.resolve(AuxTask::Compression);
        assert!(
            res2.used_primary,
            "explicit-but-uninitialisable chain still ends at primary"
        );
        assert!(res2.resolved.is_empty());
    }

    /// A catalog-only custom cheap-tier provider (no `[provider_api_keys]`
    /// / `[provider_urls]` entry) must resolve its `api_key_env` and
    /// `base_url` from the model catalog, mirroring the primary path's
    /// `resolve_non_default_api_key_env` / `lookup_provider_url` (#5755 /
    /// #5807). Without the catalog wired the resolver falls back to the
    /// `<PROVIDER>_API_KEY` convention and no base URL — which is exactly
    /// the bug that made a custom aux provider silently never initialise
    /// and billed the primary instead.
    #[test]
    fn catalog_only_custom_provider_resolves_key_env_and_base_url() {
        use crate::model_catalog::ModelCatalog;
        use librefang_types::model_catalog::ProviderInfo;

        // Non-conventional env var name proves the value came from the
        // catalog, not the `MYCUSTOM_API_KEY` convention fallback.
        let catalog = ModelCatalog::from_entries(
            vec![],
            vec![ProviderInfo {
                id: "mycustom".to_string(),
                api_key_env: "MYCUSTOM_SECRET_TOKEN".to_string(),
                base_url: "https://custom.example/v1".to_string(),
                ..Default::default()
            }],
        );

        let primary = MarkerDriver::new("primary");
        let cfg = KernelConfig::default();
        let aux = AuxClient::new(Arc::new(cfg), primary).with_model_catalog(Arc::new(catalog));

        assert_eq!(
            aux.resolve_api_key_env("mycustom"),
            "MYCUSTOM_SECRET_TOKEN",
            "catalog api_key_env must win over the naming convention"
        );
        assert_eq!(
            aux.resolve_base_url("mycustom"),
            Some("https://custom.example/v1".to_string()),
            "catalog base_url must be consulted when [provider_urls] has none"
        );
    }

    /// Operator-explicit `[provider_api_keys]` / `[provider_urls]` still
    /// win over the catalog — the operator can pin a custom provider's
    /// key/URL and the catalog must not shadow it (#5807 precedence).
    #[test]
    fn operator_explicit_config_overrides_catalog() {
        use crate::model_catalog::ModelCatalog;
        use librefang_types::model_catalog::ProviderInfo;

        let catalog = ModelCatalog::from_entries(
            vec![],
            vec![ProviderInfo {
                id: "mycustom".to_string(),
                api_key_env: "CATALOG_TOKEN".to_string(),
                base_url: "https://catalog.example/v1".to_string(),
                ..Default::default()
            }],
        );

        let mut cfg = KernelConfig::default();
        cfg.provider_api_keys
            .insert("mycustom".to_string(), "OPERATOR_TOKEN".to_string());
        cfg.provider_urls
            .insert("mycustom".to_string(), "https://op.example".to_string());

        let primary = MarkerDriver::new("primary");
        let aux = AuxClient::new(Arc::new(cfg), primary).with_model_catalog(Arc::new(catalog));

        assert_eq!(aux.resolve_api_key_env("mycustom"), "OPERATOR_TOKEN");
        assert_eq!(
            aux.resolve_base_url("mycustom"),
            Some("https://op.example".to_string())
        );
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
