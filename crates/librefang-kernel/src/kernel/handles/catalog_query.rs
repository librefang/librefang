//! [`kernel_handle::CatalogQuery`] (#4842) — read-side projection of the
//! model catalog used by drivers at request-build time.
//!
//! Currently surfaces `reasoning_echo_policy_for(model)` so the
//! OpenAI-compat driver can dispatch the right wire shape for
//! `reasoning_content` per model by catalog lookup, replacing a substring
//! match that lived in the driver. Looks up the model by id or alias; a
//! catalog miss returns `ReasoningEchoPolicy::None`, which signals the
//! driver to fall back to substring detection.

use librefang_runtime::kernel_handle;
use librefang_types::model_catalog::{ReasoningEchoPolicy, VisionSupport};

use super::super::LibreFangKernel;
use crate::kernel_api::KernelApi;

impl LibreFangKernel {
    /// Inherent mirror of [`kernel_handle::CatalogQuery::reasoning_echo_policy_for`]
    /// so `LibreFangKernel`'s own internal `CompletionRequest`-construction
    /// sites can dispatch the policy without bringing the `CatalogQuery`
    /// trait into scope.
    pub(crate) fn lookup_reasoning_echo_policy(&self, model: &str) -> ReasoningEchoPolicy {
        self.model_catalog_ref()
            .load()
            .find_model(model)
            .map(|entry| entry.reasoning_echo_policy)
            .unwrap_or_default()
    }

    /// Inherent mirror of [`kernel_handle::CatalogQuery::vision_support_for`] (#6010, refs #7957).
    ///
    /// Resolves what the catalog actually *knows* about the model's image-input support, honouring operator capability overrides (#4745).
    /// Three answers, not two: a catalog miss and an entry whose flag was inferred from the model's name both return [`VisionSupport::Unknown`], because they carry the same amount of information and the gate must therefore behave identically for both.
    /// Only a declared `supports_vision = false` returns `Unsupported`, and only that answer removes images from a request.
    pub(crate) fn lookup_vision_support(&self, model: &str) -> VisionSupport {
        self.model_catalog_ref().load().vision_support_for(model)
    }
}

impl kernel_handle::CatalogQuery for LibreFangKernel {
    fn reasoning_echo_policy_for(&self, model: &str) -> ReasoningEchoPolicy {
        self.lookup_reasoning_echo_policy(model)
    }

    fn vision_support_for(&self, model: &str) -> VisionSupport {
        self.lookup_vision_support(model)
    }

    /// Resolve the per-agent `extraction_model` for proactive memory
    /// (#5475). The chain is: agent manifest `[proactive_memory]
    /// extraction_model` → kernel-global `[proactive_memory]
    /// extraction_model` → `None`. Empty strings on either side are
    /// treated as unset.
    ///
    /// `agent_id` is the UUID string the proactive-memory store
    /// already stamps onto its `user_id` and forwards through the
    /// `_with_agent_id` extractor entry point. A malformed UUID
    /// returns `None` and the extractor falls back to the boot-time
    /// model — same behaviour as the pre-#5475 single-model path.
    fn proactive_memory_extraction_model_for(&self, agent_id: &str) -> Option<String> {
        use librefang_types::agent::AgentId;
        use std::str::FromStr;

        let aid = AgentId::from_str(agent_id).ok()?;
        let entry = self.agents.registry.get_arc(aid)?;
        let cfg = self.config.load();
        entry
            .manifest
            .proactive_memory
            .resolve_extraction_model(&cfg.proactive_memory)
    }

    /// Look a profile up in the resolved catalog — the builtin asset with
    /// `~/.librefang/model_profiles.toml` merged over it.
    ///
    /// Deliberately **not** gated on `[model_router] enabled`. That switch
    /// governs the *automatic* per-turn router, which picks a model nobody
    /// asked for; naming a profile on an `agent_spawn` call is an explicit
    /// choice by the parent agent, and silently ignoring an explicit
    /// parameter is the exact failure this lookup exists to remove. It also
    /// keeps the subagent case usable — spawning a cheap verifier does
    /// not require switching every agent onto automatic routing.
    ///
    /// The profile's `model` is resolved through the live model catalog the
    /// same way `route_to_profile` resolves it (#7789 review): every builtin
    /// profile names a catalog alias (`"haiku"`, `"sonnet"`, …) and an
    /// unresolved alias reaches the provider as a model id nobody accepts,
    /// so the spawned agent would fail auth on its first turn.
    fn resolve_model_profile(
        &self,
        name: &str,
    ) -> Option<librefang_types::model_profile::ModelProfile> {
        let cfg = self.config.load();
        let mut profile = crate::model_router::ProfileCatalog::load_cached(
            cfg.home_dir.as_path(),
            &cfg.model_router,
        )
        .get(name)?
        .clone();
        // Resolve catalog aliases ("haiku" -> "claude-haiku-4-5-…") so the
        // builtin profiles do not pin dated model snapshots.
        //
        // Provider-scoped, not the global alias map (#7789 review). `aliases`
        // is one flat table keyed by lowercase alias with first-writer-wins, so
        // resolving the model half of a `(provider, model)` pair through it
        // answers from whichever provider registered the name first. Only
        // `anthropic.toml` claims bare `haiku` / `sonnet` / `opus` today, so
        // nothing collides yet — but it would the moment an operator adds a
        // second provider that also claims one. `find_model_for_manifest` tries
        // the provider-scoped lookups first; its documented last resort is the
        // same provider-blind `find_model`, which this call does not want, so
        // the hit only counts when it landed on this provider's own entry.
        let model_catalog = self.model_catalog_ref().load();
        match model_catalog
            .find_model_for_manifest(&profile.provider, &profile.model)
            .filter(|entry| entry.provider.eq_ignore_ascii_case(&profile.provider))
        {
            Some(entry) => profile.model = entry.id.clone(),
            // A miss used to be silent, which is the original failure in its
            // narrower form: the unresolved alias is written into a durable
            // manifest and the agent fails on every turn, while the spawn
            // reports success. It stays a `WARN` rather than a refusal because,
            // unlike a missing credential, an unresolved id is not provably
            // wrong — a home dir whose `providers/` sync has not run knows no
            // ids at all, and a custom endpoint may well accept one the catalog
            // never listed. Refusing would turn a spawn that probably works
            // into one that certainly does not, on exactly the deployments
            // least able to absorb it.
            None => {
                tracing::warn!(
                    profile = %profile.name,
                    provider = %profile.provider,
                    model = %profile.model,
                    "Model profile names a model id the catalog cannot resolve for its provider — \
                     spawning with the id as written; if the provider rejects it, sync the model \
                     registry or correct the profile"
                );
            }
        }
        Some(profile)
    }

    /// Whether `provider` has credentials the kernel can see (#7789 review).
    ///
    /// Accepts on any of: a local provider, a credential pool, a catalog
    /// `key_required = false` declaration, `[default_model] api_key_env` when
    /// this is the default provider, or the env var the kernel resolves for the
    /// provider (operator pin, catalog `api_key_env`, or the
    /// `<PROVIDER>_API_KEY` convention).
    ///
    /// Two deliberate differences from the credential check in
    /// `route_to_profile`, which this otherwise mirrors:
    ///
    /// - **A miss refuses rather than skips.** The router's fallback is "keep
    ///   the agent's own model for this turn"; here the wrong provider is
    ///   persisted into a manifest and would be wrong on every subsequent turn,
    ///   so the spawn is refused with the env var named.
    /// - **It runs unconditionally.** The router skips the check entirely when
    ///   the profile's provider is the one the agent already uses, since that
    ///   provider is demonstrably working. The spawn path has no such evidence:
    ///   the manifest it writes outlives the parent, so the provider is checked
    ///   on its own merits. That makes this the stricter of the two, which is
    ///   why the keyless and `[default_model]` cases above are checked here and
    ///   not there — those are the false refusals the wider scope exposes.
    fn check_provider_credentials(&self, provider: &str) -> Result<(), String> {
        if librefang_runtime::provider_health::is_local_provider(provider) {
            return Ok(());
        }
        if self.llm.credential_pools.contains_key(provider) {
            return Ok(());
        }
        // Providers the catalog declares keyless authenticate out of band and
        // have no key to find (#7789 review). `resolve_non_default_api_key_env`
        // answers "what would the variable be called", never "is one needed":
        // for `claude-code` the catalog carries `api_key_env = ""` /
        // `key_required = false`, so the catalog lookup declines and the
        // convention fallback invents `CLAUDE_CODE_API_KEY` — a variable that
        // provider never reads. Refusing on it made `profile` unusable on a
        // stock subscription install, where `[default_model] provider =
        // "claude-code"` with `cli_profile_dirs` runs every turn the daemon
        // takes, with an error no operator could satisfy. Same for
        // `gemini-cli`, `codex-cli` and every other `key_required = false`
        // entry. A provider absent from the catalog is left to the key check
        // below: `key_required` defaults to `true`, so an unknown provider is
        // still treated as needing one.
        if self
            .model_catalog_ref()
            .load()
            .get_provider(provider)
            .is_some_and(|p| !p.key_required)
        {
            return Ok(());
        }
        let cfg = self.config.load();
        // `[default_model] api_key_env` is where an operator whose single
        // provider deviates from the convention pins their key, and the driver
        // resolver reads it (`llm_drivers.rs`, the `agent_provider ==
        // default_provider` arm) while `KernelConfig::resolve_api_key_env` does
        // not. Without this the guard refused the provider every agent on the
        // install is already running on (#7789 review).
        let default_key_env = cfg.default_model.api_key_env.trim();
        if provider == cfg.default_model.provider
            && !default_key_env.is_empty()
            && std::env::var(default_key_env).is_ok()
        {
            return Ok(());
        }
        let key_env = self.resolve_non_default_api_key_env(&cfg, provider);
        if std::env::var(&key_env).is_ok() {
            return Ok(());
        }
        Err(format!(
            "provider '{provider}' has no API key configured — set {key_env} \
             or add a credential pool for it"
        ))
    }

    /// Ordered by construction: `ProfileCatalog` name-sorts at load (#3298).
    fn model_profile_names(&self) -> Vec<String> {
        let cfg = self.config.load();
        crate::model_router::ProfileCatalog::load_cached(cfg.home_dir.as_path(), &cfg.model_router)
            .names()
    }

    /// The parent's `[model.router_override]`, read from its manifest in the
    /// registry (#7789 review).
    ///
    /// Reads the same `manifest.model.router_override` the per-turn router
    /// reads, from the same registry.
    ///
    /// Unlike `proactive_memory_extraction_model_for` above, a malformed UUID
    /// or an agent missing from the registry is an `Err`, not a `None`: an
    /// agent that is live enough to be calling `agent_spawn` always resolves
    /// here, so a miss is a fault rather than evidence that the agent is
    /// unconstrained. Reporting it as "no constraints" would fail open on a
    /// spend cap.
    fn model_router_override_for(
        &self,
        agent_id: &str,
    ) -> Result<Option<librefang_types::model_profile::AgentRouterOverride>, String> {
        use std::str::FromStr;
        let aid = librefang_types::agent::AgentId::from_str(agent_id)
            .map_err(|e| format!("agent id '{agent_id}' is not a valid agent UUID: {e}"))?;
        let entry = self
            .agents
            .registry
            .get_arc(aid)
            .ok_or_else(|| format!("agent '{agent_id}' is not in the registry"))?;
        Ok(entry.manifest.model.router_override.clone())
    }
}
