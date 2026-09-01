//! Manifest -> capability conversion and small config helpers.
//!
//! Pure functions extracted from `kernel.rs`. None of these touch
//! `LibreFangKernel` itself — they operate on `AgentManifest`,
//! capability sets, and provider/model name strings.

use librefang_types::agent::*;
use librefang_types::capability::Capability;
use librefang_types::model_catalog::{ContextWindowSource, LimitSource, ResolvedContextWindow};

/// Convert a manifest's capability declarations into Capability enums.
///
/// If a `profile` is set and the manifest has no explicit tools, the profile's
/// implied capabilities are used as a base — preserving any non-tool overrides
/// from the manifest.
pub(super) fn manifest_to_capabilities(manifest: &AgentManifest) -> Vec<Capability> {
    let mut caps = Vec::new();

    // Profile expansion: use profile's implied capabilities when no explicit tools
    let effective_caps = if let Some(ref profile) = manifest.profile {
        if manifest.capabilities.tools.is_empty() {
            let mut merged = profile.implied_capabilities();
            if !manifest.capabilities.network.is_empty() {
                merged.network = manifest.capabilities.network.clone();
            }
            if !manifest.capabilities.shell.is_empty() {
                merged.shell = manifest.capabilities.shell.clone();
            }
            if !manifest.capabilities.agent_message.is_empty() {
                merged.agent_message = manifest.capabilities.agent_message.clone();
            }
            if manifest.capabilities.agent_spawn {
                merged.agent_spawn = true;
            }
            // A declared list wins over the profile's implied one, including when it is empty: `memory_read = []` is the operator saying "grant nothing", which #7605 made expressible and load-bearing for the automatic memorize / retrieve paths.
            if manifest.capabilities.memory_read.is_some() {
                merged.memory_read = manifest.capabilities.memory_read.clone();
            }
            if manifest.capabilities.memory_write.is_some() {
                merged.memory_write = manifest.capabilities.memory_write.clone();
            }
            if manifest.capabilities.ofp_discover {
                merged.ofp_discover = true;
            }
            if !manifest.capabilities.ofp_connect.is_empty() {
                merged.ofp_connect = manifest.capabilities.ofp_connect.clone();
            }
            merged
        } else {
            manifest.capabilities.clone()
        }
    } else {
        manifest.capabilities.clone()
    };

    for host in &effective_caps.network {
        caps.push(Capability::NetConnect(host.clone()));
    }
    for tool in &effective_caps.tools {
        caps.push(Capability::ToolInvoke(tool.clone()));
    }
    for scope in effective_caps.memory_read.iter().flatten() {
        caps.push(Capability::MemoryRead(scope.clone()));
    }
    for scope in effective_caps.memory_write.iter().flatten() {
        caps.push(Capability::MemoryWrite(scope.clone()));
    }
    if effective_caps.agent_spawn {
        caps.push(Capability::AgentSpawn);
    }
    for pattern in &effective_caps.agent_message {
        caps.push(Capability::AgentMessage(pattern.clone()));
    }
    for cmd in &effective_caps.shell {
        caps.push(Capability::ShellExec(cmd.clone()));
    }
    if effective_caps.ofp_discover {
        caps.push(Capability::OfpDiscover);
    }
    for peer in &effective_caps.ofp_connect {
        caps.push(Capability::OfpConnect(peer.clone()));
    }

    caps
}

/// Whether the global `[thinking]` config may be backfilled onto a manifest for `model`.
///
/// Backfill is suppressed only when the catalog knows the model and marks it `supports_thinking = false` — an unknown/custom model keeps the historical backfill so explicit operator setups are never silently degraded (#6398).
/// This bounds the blast radius of a global `[thinking]` block: with the OpenAI-compat driver now emitting `reasoning_effort` for a configured budget, backfilling onto a known non-reasoning model (e.g. `gpt-4o`) would turn every request into a parameter error where it previously was a silent no-op.
/// Explicit `manifest.thinking` and the per-call override below are deliberately not gated — they are direct user intent.
/// Provider-aware, prefix-reconciling lookup (mirrors `ModelCatalog::find_model_for_manifest`, #6423): a bare OpenRouter manifest model must still resolve the prefixed catalog entry, or this falls back to the permissive "unknown model" default and silently backfills thinking config onto a model that does not support it.
pub(super) fn global_thinking_backfill_allowed(
    catalog: &librefang_runtime::model_catalog::ModelCatalog,
    provider: &str,
    model: &str,
) -> bool {
    catalog
        .find_model_for_manifest(provider, model)
        .map(|m| catalog.effective_capabilities(m).supports_thinking)
        .unwrap_or(true)
}

/// Resolve the context window (in tokens) for one turn, honouring the
/// documented precedence chain (#6568, extended by #7774).
///
/// 1. `agent.toml: [model] context_window` — an explicit *per-agent* override. The
///    warning the agent loop emits for an unknown model literally tells the
///    operator to set this field, so it has to win; before this helper the three
///    execution paths never read it and the field was inert.
/// 2. `model_overrides.json: context_window` — the *per-model* operator override
///    (#7774), reached from the API and the dashboard and keyed by
///    `provider:model_id`. It sits above the catalog because it exists precisely
///    to correct the catalog: a window the registry never carried, or one a
///    `/models` discovery pass assumed. Being keyed rather than attached to an
///    entry, it also applies to a model the catalog does not know at all —
///    the reported case, a gateway-served model whose window is configured in
///    the runtime behind it and surfaced by nothing.
/// 3. `ModelCatalog` lookup — provider-aware and prefix-reconciling (#6423), with
///    `0` filtered out so image / audio entries (which carry no context window)
///    fall through instead of poisoning the budget math.
/// 4. `session_hint` — the value persisted on the session, authoritative only
///    when neither an override nor the catalog resolves. Callers with no session
///    in hand pass `None`.
///
/// Returns `None` when nothing resolves, leaving the fallback (currently
/// `UNKNOWN_MODEL_CONTEXT_WINDOW`, 8192) to the agent loop, which also logs it.
///
/// The answer carries the layer that produced it ([`ResolvedContextWindow`]),
/// because the number alone cannot tell an operator whether their model's
/// window is known or guessed — the distinction #7774 was filed over.
/// A caller that only needs the size reads `.tokens`.
pub(super) fn resolve_context_window(
    catalog: &librefang_runtime::model_catalog::ModelCatalog,
    model: &librefang_types::agent::ModelConfig,
    session_hint: Option<u64>,
) -> Option<ResolvedContextWindow> {
    if let Some(tokens) = model.context_window.filter(|v| *v > 0) {
        return Some(ResolvedContextWindow {
            tokens: tokens as usize,
            source: ContextWindowSource::AgentOverride,
        });
    }
    // Layers 2 and 3 in one call: `effective_limits_for_manifest`
    // already ranks the operator override above the catalog entry and
    // filters both sides' zeros, so the two cannot drift apart here — and it
    // reports which of the two answered, so neither can this.
    let limits = catalog.effective_limits_for_manifest(&model.provider, &model.model);
    if let Some(tokens) = limits.context_window {
        let source = match limits.context_window_source {
            LimitSource::Override => ContextWindowSource::ModelOverride,
            // `Unknown` is unreachable while `context_window` is `Some` —
            // `rank_limit` sets the two in the same expression — but mapping it
            // to `Catalog` keeps this total without an unreachable panic.
            LimitSource::Catalog | LimitSource::Unknown => ContextWindowSource::Catalog,
        };
        return Some(ResolvedContextWindow {
            tokens: tokens as usize,
            source,
        });
    }
    session_hint
        .filter(|v| *v > 0)
        .map(|tokens| ResolvedContextWindow {
            tokens: tokens as usize,
            source: ContextWindowSource::SessionHint,
        })
}

/// Apply a per-call reasoning override to a manifest clone.
///
/// This is the top rung of the #7946 resolution order — per-call > per-agent >
/// global > compiled default. The two rungs below it have already been applied
/// by the caller: the manifest carries the per-agent `[thinking]` table, and
/// the global `[thinking]` section was backfilled into it a few lines earlier
/// when the agent declared none. So this function only has to overwrite, and
/// whatever it leaves behind is the effective configuration for the turn.
///
/// - [`ThinkingOverride::Enable`] (legacy `thinking: true`) — ensure the
///   manifest has a `ThinkingConfig`, inserting the default one if previously
///   empty, so the driver enables reasoning. No mode is pinned: the caller
///   asked for reasoning, not for a particular amount of it. An inherited
///   `reasoning_mode = "none"` *is* cleared, though: the boolean documents
///   itself as "force thinking on even if the manifest has it off", and a
///   non-think mode is exactly that off-state, so leaving it in place would
///   make `thinking: true` a silent no-op.
/// - [`ThinkingOverride::Disable`] (legacy `thinking: false`) — clear
///   `manifest.thinking` so the driver does not request thinking regardless of
///   the manifest/global default.
/// - [`ThinkingOverride::Mode`] — stamp the mode onto the manifest's thinking
///   config, creating it if absent. Note that `Mode(ReasoningMode::None)` is
///   *not* the same as `Disable`: it keeps a thinking config so the driver can
///   send the provider's explicit non-think toggle, which is the whole point
///   of #7946. `Disable` merely omits the opt-in, which leaves a model that
///   reasons by default reasoning.
/// - [`ThinkingOverride::Inherit`] — leave the manifest untouched.
pub(super) fn apply_thinking_override(
    manifest: &mut librefang_types::agent::AgentManifest,
    thinking_override: librefang_types::config::ThinkingOverride,
) {
    use librefang_types::config::{ReasoningMode, ThinkingConfig, ThinkingOverride};
    match thinking_override {
        ThinkingOverride::Enable if manifest.thinking.is_none() => {
            manifest.thinking = Some(ThinkingConfig::default());
        }
        // Enable when thinking is already set — keep the existing budget, but
        // drop an inherited non-think mode so the caller's "on" is not silently
        // overruled by the agent's (or the global) `reasoning_mode = "none"`.
        ThinkingOverride::Enable => {
            if let Some(tc) = manifest.thinking.as_mut() {
                if tc.reasoning_mode == Some(ReasoningMode::None) {
                    tc.reasoning_mode = None;
                }
            }
        }
        ThinkingOverride::Disable => {
            manifest.thinking = None;
        }
        ThinkingOverride::Mode(mode) => {
            manifest
                .thinking
                .get_or_insert_with(ThinkingConfig::default)
                .reasoning_mode = Some(mode);
        }
        ThinkingOverride::Inherit => {}
    }
}

/// Apply global budget defaults to an agent's resource quota.
///
/// When the global budget config specifies limits and the agent still has
/// the built-in defaults, override them so agents respect the user's config.
pub(super) fn apply_budget_defaults(
    budget: &librefang_types::config::BudgetConfig,
    resources: &mut ResourceQuota,
) {
    // Only override hourly if agent has unlimited (0.0) and global is set
    if budget.max_hourly_usd > 0.0 && resources.max_cost_per_hour_usd == 0.0 {
        resources.max_cost_per_hour_usd = budget.max_hourly_usd;
    }
    // Only override daily/monthly if agent has unlimited (0.0) and global is set
    if budget.max_daily_usd > 0.0 && resources.max_cost_per_day_usd == 0.0 {
        resources.max_cost_per_day_usd = budget.max_daily_usd;
    }
    if budget.max_monthly_usd > 0.0 && resources.max_cost_per_month_usd == 0.0 {
        resources.max_cost_per_month_usd = budget.max_monthly_usd;
    }
    // Override per-agent hourly token limit when:
    //   1. The global default is set (> 0), AND
    //   2. The agent has NOT explicitly configured its own limit (None).
    //
    // When an agent explicitly sets `max_llm_tokens_per_hour = 0` in its
    // agent.toml (Some(0)), that means "unlimited" and must not be
    // overridden by the global default.
    if budget.default_max_llm_tokens_per_hour > 0 && resources.max_llm_tokens_per_hour.is_none() {
        resources.max_llm_tokens_per_hour = Some(budget.default_max_llm_tokens_per_hour);
    }
}

/// Pick a sensible default embedding model for a given provider when the user
/// configured an explicit `embedding_provider` but left `embedding_model` at the
/// default value (which is a local model name that cloud APIs wouldn't recognise).
pub(super) fn default_embedding_model_for_provider(provider: &str) -> &'static str {
    match provider {
        "openai" | "openrouter" => "text-embedding-3-small",
        "mistral" => "mistral-embed",
        "cohere" => "embed-english-v3.0",
        // Local providers use nomic-embed-text as a good default
        "ollama" | "vllm" | "lmstudio" => "nomic-embed-text",
        // Other OpenAI-compatible APIs typically support the OpenAI model names
        _ => "text-embedding-3-small",
    }
}

/// Infer provider from a model name when catalog lookup fails.
///
/// Uses well-known model name prefixes to map to the correct provider.
/// This is a defense-in-depth fallback — models should ideally be in the catalog.
pub(super) fn infer_provider_from_model(model: &str) -> Option<String> {
    let lower = model.to_lowercase();
    // Check for explicit provider prefix with / or : delimiter
    // (e.g., "minimax/MiniMax-M2.5" or "qwen:qwen-plus")
    let (prefix, has_delim) = if let Some(idx) = lower.find('/') {
        (&lower[..idx], true)
    } else if let Some(idx) = lower.find(':') {
        (&lower[..idx], true)
    } else {
        (lower.as_str(), false)
    };
    if has_delim {
        match prefix {
            "minimax" | "gemini" | "anthropic" | "openai" | "groq" | "deepseek" | "mistral"
            | "cohere" | "xai" | "ollama" | "together" | "fireworks" | "perplexity"
            | "cerebras" | "sambanova" | "replicate" | "huggingface" | "codex" | "claude-code"
            | "copilot" | "github-copilot" | "qwen" | "zhipu" | "zai" | "moonshot"
            | "openrouter" | "volcengine" | "doubao" | "dashscope" | "byteplus"
            | "byteplus_coding" => {
                return Some(prefix.to_string());
            }
            // "z.ai" is a domain alias for the zai provider
            "z.ai" => {
                return Some("zai".to_string());
            }
            // "kimi" / "kimi2" are brand aliases for moonshot
            "kimi" | "kimi2" => {
                return Some("moonshot".to_string());
            }
            _ => {}
        }
    }
    // Infer from well-known model name patterns
    if lower.starts_with("minimax") {
        Some("minimax".to_string())
    } else if lower.starts_with("gemini") {
        Some("gemini".to_string())
    } else if lower.starts_with("claude") {
        Some("anthropic".to_string())
    } else if lower.starts_with("gpt")
        || lower.starts_with("o1")
        || lower.starts_with("o3")
        || lower.starts_with("o4")
    {
        Some("openai".to_string())
    } else if lower.starts_with("llama")
        || lower.starts_with("mixtral")
        || lower.starts_with("qwen")
    {
        // These could be on multiple providers; don't infer
        None
    } else if lower.starts_with("grok") {
        Some("xai".to_string())
    } else if lower.starts_with("deepseek") {
        Some("deepseek".to_string())
    } else if lower.starts_with("mistral")
        || lower.starts_with("codestral")
        || lower.starts_with("pixtral")
    {
        Some("mistral".to_string())
    } else if lower.starts_with("command") || lower.starts_with("embed-") {
        Some("cohere".to_string())
    } else if lower.starts_with("sonar") {
        Some("perplexity".to_string())
    } else if lower.starts_with("glm") {
        Some("zhipu".to_string())
    } else if lower.starts_with("ernie") {
        Some("qianfan".to_string())
    } else if lower.starts_with("abab") {
        Some("minimax".to_string())
    } else if lower.starts_with("moonshot") || lower.starts_with("kimi") {
        Some("moonshot".to_string())
    } else {
        None
    }
}

pub(super) fn resolve_fallback_target(
    fallback_provider: &str,
    fallback_model: &str,
    default_provider: &str,
    default_model: &str,
) -> (String, String) {
    let inherits_default_model = fallback_model.is_empty() || fallback_model == "default";
    let resolved_model = if inherits_default_model {
        default_model
    } else {
        fallback_model
    };
    let inherits_default_provider = fallback_provider.is_empty() || fallback_provider == "default";
    let resolved_provider = if !inherits_default_provider {
        fallback_provider.to_string()
    } else if inherits_default_model {
        default_provider.to_string()
    } else {
        infer_provider_from_model(resolved_model).unwrap_or_else(|| default_provider.to_string())
    };
    let resolved_model =
        librefang_runtime::agent_loop::strip_provider_prefix(resolved_model, &resolved_provider);
    (resolved_provider, resolved_model)
}

/// A well-known agent ID used for the legacy shared memory namespace.
/// This is a fixed UUID. Pre-#5070, all agents read/wrote to this single
/// namespace. Post-#5070, LLM-facing tools use per-agent scoping; this ID
/// remains for internal kernel subsystems and backward compatibility.
/// Parse an agent.toml string and return true if `enabled` is explicitly set
/// Try to extract an `AgentManifest` from a `hand.toml` file (HandDefinition format).
///
/// When `source_toml_path` points to a hand.toml rather than an agent.toml, the file
/// contains a `HandDefinition` with multiple agent manifests keyed by role name.
/// This function parses the file as a `HandDefinition` and returns the manifest whose
/// name (in any of the four forms the kernel may have stamped) matches `agent_name`.
///
/// The four forms tried, in order, are:
/// 1. `manifest.name` as written in the TOML (e.g. `"jarvis-operator"`).
/// 2. The `[agents.<role>]` key (e.g. `"operator"`).
/// 3. `"{hand_id}:{manifest.name}"` — the canonical form stamped by hand activation
///    in `kernel/mod.rs` when persisting the agent record. This is the form returned
///    by `GET /api/agents` and stored in `agents.name` in the SQLite DB, so the
///    boot-time TOML drift detection MUST recognise it or hand-derived agents
///    silently fall through to "Cannot parse TOML on disk as agent manifest, using
///    DB version" and the on-disk hand.toml never propagates.
/// 4. `"{hand_id}-{role}"` — legacy qualifier kept for backwards compatibility.
pub(super) fn extract_manifest_from_hand_toml(
    toml_str: &str,
    agent_name: &str,
) -> Option<librefang_types::agent::AgentManifest> {
    let def: librefang_hands::HandDefinition = toml::from_str(toml_str).ok()?;
    for (role, hand_agent) in &def.agents {
        // Forms 1 + 2: bare manifest name or role key.
        if hand_agent.manifest.name == agent_name || role == agent_name {
            return Some(hand_agent.manifest.clone());
        }
        // Form 3: canonical "{hand_id}:{manifest.name}" stamped at activation.
        if format!("{}:{}", def.id, hand_agent.manifest.name) == agent_name {
            return Some(hand_agent.manifest.clone());
        }
        // Form 4: legacy "{hand_id}-{role}" qualifier.
        if format!("{}-{}", def.id, role) == agent_name {
            return Some(hand_agent.manifest.clone());
        }
    }
    None
}

/// to `false`. Uses proper TOML parsing to handle all valid whitespace variants
/// and avoid false positives from commented-out lines.
pub(super) fn toml_enabled_false(content: &str) -> bool {
    #[derive(serde::Deserialize)]
    struct Probe {
        enabled: Option<bool>,
    }
    toml::from_str::<Probe>(content)
        .ok()
        .and_then(|p| p.enabled)
        == Some(false)
}

/// Marker that introduces the rendered settings tail in the system prompt.
///
/// The activation path uses `\n\n---\n\n` as the section separator and
/// `## User Configuration` as the block heading (see
/// `librefang_hands::resolve_settings`). We treat the combination as the
/// canonical anchor for the settings tail so we can detect and replace an
/// existing one rather than blindly appending a duplicate.
const USER_CONFIG_TAIL_MARKER: &str = "\n\n---\n\n## User Configuration";

/// Marker that introduces the rendered `## Reference Knowledge` tail —
/// skill content (per-role override or hand-shared) appended at activation.
const SKILL_REFERENCE_TAIL_MARKER: &str = "\n\n---\n\n## Reference Knowledge";

/// Marker that introduces the rendered `## Your Team` tail — peer roster
/// for multi-agent hands. Uses the same `\n\n---\n\n` fence as the other
/// two tails so the heading is unambiguous: a SKILL.md or base prompt that
/// happens to contain a literal `## Your Team` line cannot accidentally
/// match the marker and cause `find()` to truncate user-authored content.
const TEAM_TAIL_MARKER: &str = "\n\n---\n\n## Your Team";

/// Byte offset of the earliest runtime-rendered prompt tail in `prompt`, or `None` when the prompt carries no tail at all.
///
/// The three tails are always appended in a fixed order (settings -> reference -> team), so truncating at the earliest marker drops every tail in one shot and leaves the author-written base prompt.
/// Each marker is probed individually so a prompt carrying only a subset still truncates at the right place.
fn earliest_rendered_tail_idx(prompt: &str) -> Option<usize> {
    [
        USER_CONFIG_TAIL_MARKER,
        SKILL_REFERENCE_TAIL_MARKER,
        TEAM_TAIL_MARKER,
    ]
    .into_iter()
    .filter_map(|marker| prompt.find(marker))
    .min()
}

/// The `## User Configuration` tail for a hand's settings, fenced and ready to concatenate, plus the settings-derived env-var names.
///
/// `None` when the hand declares no settings or every one of them renders empty.
/// Pure: it never looks at an existing prompt, which is what lets [`rerender_hand_prompt_tails`] assemble all three tails without a single content search.
fn render_settings_block(
    settings: &[librefang_hands::HandSetting],
    instance_config: &std::collections::HashMap<String, serde_json::Value>,
) -> (Option<String>, Vec<String>) {
    let resolved = librefang_hands::resolve_settings(settings, instance_config);
    if resolved.prompt_block.is_empty() {
        return (None, resolved.env_vars);
    }
    (
        Some(format!("\n\n---\n\n{}", resolved.prompt_block)),
        resolved.env_vars,
    )
}

/// The `## Reference Knowledge` tail for a role, fenced and ready to concatenate, or `None` when the hand ships no skill content for it.
///
/// Per-role `agent_skill_content` wins over the hand-shared `skill_content`.
fn render_skill_reference_block(
    role: &str,
    def: &librefang_hands::HandDefinition,
) -> Option<String> {
    let role_lower = role.to_lowercase();
    def.agent_skill_content
        .get(&role_lower)
        .or(def.skill_content.as_ref())
        .filter(|content| !content.is_empty())
        .map(|content| format!("\n\n---\n\n## Reference Knowledge\n\n{content}"))
}

/// The `## Your Team` tail for a role, fenced and ready to concatenate, or `None` for a single-agent hand or a role with no peers.
fn render_team_block(role: &str, def: &librefang_hands::HandDefinition) -> Option<String> {
    if !def.is_multi_agent() {
        return None;
    }

    let mut peer_lines = Vec::new();
    for (peer_role, peer_agent) in &def.agents {
        if peer_role == role {
            continue;
        }
        let hint = peer_agent
            .invoke_hint
            .as_deref()
            .unwrap_or(&peer_agent.manifest.description);
        peer_lines.push(format!(
            "- **{peer_role}**: {hint} (use agent_send to message)"
        ));
    }

    if peer_lines.is_empty() {
        return None;
    }
    Some(format!(
        "\n\n---\n\n## Your Team\n\n{}",
        peer_lines.join("\n")
    ))
}

/// Return a clone of `manifest` with all known runtime-rendered prompt
/// tails stripped from `model.system_prompt`, suitable for the boot-time
/// drift comparison.
///
/// The drift loop compares the disk TOML manifest against the DB blob and
/// rewrites the DB when they differ. Without this projection the disk
/// manifest (which never carries any rendered tail) will always look
/// "different" from the DB blob (which carries the tails materialized at
/// activation), triggering a clobber-and-rerender cycle on every restart.
/// Comparing on the projection means drift only fires when the *raw* TOML
/// truly diverges from the *raw* DB form.
///
/// All known tails are appended in a fixed order
/// (settings -> reference -> team), so truncating from the earliest
/// marker drops every tail in one shot. Each marker is checked
/// individually so prompts carrying only a subset of the tails still get
/// the correct truncation point.
pub(super) fn manifest_for_diff(manifest: &AgentManifest) -> AgentManifest {
    let mut copy = manifest.clone();
    let prompt = &mut copy.model.system_prompt;
    if let Some(idx) = earliest_rendered_tail_idx(prompt) {
        prompt.truncate(idx);
    }
    copy
}

/// Env-var passthrough allowlist for a hand instance: the `provider_env` / `env_var` names its resolved settings select, plus the `[[requires]]` entries that name an env var or API key.
///
/// SECURITY: every candidate is attacker-controllable HAND.toml text and is later materialized into a child process's env from the daemon's LIVE environment, so the shared secret blocklist filters the whole list.
/// A marketplace hand cannot exfiltrate `LIBREFANG_VAULT_KEY` / `ANTHROPIC_API_KEY` / … by naming them in a setting or requirement.
/// `sandbox_command` re-checks defensively at spawn time.
///
/// Shared by hand activation and the settings-save re-render so the two paths cannot drift on which names survive the filter.
pub(super) fn resolve_hand_allowed_env(
    def: &librefang_hands::HandDefinition,
    instance_config: &std::collections::HashMap<String, serde_json::Value>,
) -> Vec<String> {
    let mut allowed: Vec<String> =
        librefang_hands::resolve_settings(&def.settings, instance_config)
            .env_vars
            .into_iter()
            .filter(|v| !librefang_runtime::subprocess_sandbox::is_blocked_env_var(v))
            .collect();
    for req in &def.requires {
        match req.requirement_type {
            librefang_hands::RequirementType::ApiKey | librefang_hands::RequirementType::EnvVar
                if !req.check_value.is_empty()
                    && !allowed.contains(&req.check_value)
                    && !librefang_runtime::subprocess_sandbox::is_blocked_env_var(
                        &req.check_value,
                    ) =>
            {
                allowed.push(req.check_value.clone());
            }
            _ => {}
        }
    }
    allowed
}

/// Render a hand agent's complete system prompt: the author-written base plus the settings, reference-knowledge, and team tails, in that order.
///
/// This is the canonical renderer — every production path that materializes a hand prompt goes through it (activation, the boot TOML-drift loop, and the settings save), so the three cannot disagree about content or ordering.
///
/// It works for any input shape.
/// A live registry manifest already ends in `[base][settings][reference][team]`, so the tails are stripped back to the base first via [`earliest_rendered_tail_idx`]; a manifest freshly parsed from disk has no tails and the strip is a no-op.
///
/// The tails are then **assembled from pure renderers rather than the `apply_*` helpers**, and that is the load-bearing difference.
/// Each `apply_*` helper locates its own tail by searching the whole prompt for its marker, which is only sound while nothing downstream of that marker is author-controlled.
/// It is not: a SKILL.md playbook may legitimately contain a `---` rule followed by a `## Your Team` heading, and once the reference tail has been appended, the team helper's search finds *that* copy and truncates the playbook there — silently dropping everything after it from what the LLM sees.
/// Assembling in one pass never searches appended content, so author text cannot be mistaken for a marker.
///
/// Returns the filtered env-var passthrough allowlist for this instance (see [`resolve_hand_allowed_env`]); callers write it to `metadata["hand_allowed_env"]`, removing the key when the list is empty.
pub(super) fn rerender_hand_prompt_tails(
    manifest: &mut AgentManifest,
    role: &str,
    def: &librefang_hands::HandDefinition,
    instance_config: &std::collections::HashMap<String, serde_json::Value>,
) -> Vec<String> {
    if let Some(idx) = earliest_rendered_tail_idx(&manifest.model.system_prompt) {
        manifest.model.system_prompt.truncate(idx);
    }

    let (settings_block, _) = render_settings_block(&def.settings, instance_config);
    for block in [
        settings_block,
        render_skill_reference_block(role, def),
        render_team_block(role, def),
    ]
    .into_iter()
    .flatten()
    {
        manifest.model.system_prompt.push_str(&block);
    }

    resolve_hand_allowed_env(def, instance_config)
}

/// Write (or clear) a hand agent's env passthrough allowlist on its manifest metadata.
///
/// An empty list **removes** the key rather than storing `[]`: a settings change that drops the last `provider_env` has to narrow the passthrough, and an insert-only write would leave the previous, wider list in place.
/// Mirrors `AgentRegistry::update_hand_rendered_prompt`, which applies the same rule to the live registry entry.
pub(super) fn set_hand_allowed_env(manifest: &mut AgentManifest, allowed_env: &[String]) {
    if allowed_env.is_empty() {
        manifest.metadata.remove("hand_allowed_env");
    } else {
        manifest.metadata.insert(
            "hand_allowed_env".to_string(),
            serde_json::to_value(allowed_env).unwrap_or_default(),
        );
    }
}

pub fn shared_memory_agent_id() -> AgentId {
    AgentId(uuid::Uuid::from_bytes([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x01,
    ]))
}

/// Percent-encode the namespace delimiters of a `peer_id` so it can be embedded
/// in the `peer:{pid}:{key}` framing without breaking injectivity (#6100).
///
/// `%` is encoded first (to `%25`) so the encoding is unambiguous, then `:` is
/// encoded (to `%3A`). The result therefore contains no bare `:`, which is what
/// makes `peer:{escape_peer_id(pid)}:{key}` injective in `pid` even when `pid`
/// carries colons — e.g. a Matrix user id `@user:matrix.org` becomes
/// `@user%3Amatrix.org`. Colon-free peer_ids are returned unchanged, so the
/// stored form of legacy (colon-free) peers is byte-identical to before.
pub(super) fn escape_peer_id(pid: &str) -> String {
    // Order matters: encode `%` before `:`, otherwise a literal `%3A` in the
    // input would be indistinguishable from an encoded colon.
    pid.replace('%', "%25").replace(':', "%3A")
}

/// Namespace a memory key by peer ID for per-user isolation.
/// When `peer_id` is `Some(pid)` (non-empty), returns
/// `"peer:{escape_peer_id(pid)}:{key}"`. When `None`, returns the key unchanged
/// (global scope).
///
/// SECURITY (#5119 / #5120 / #6100): the `peer:{pid}:{key}` framing is only
/// injective when `pid` carries no bare namespace separator. Rather than
/// rejecting colon-bearing peer_ids (which locked out platforms like Matrix
/// whose user ids look like `@user:matrix.org`, #6100), we percent-encode the
/// colon via [`escape_peer_id`]. Peer `T1` listing computes the prefix
/// `peer:T1:`, which no longer matches the escaped key `peer:T1%3AU2:…` of peer
/// `T1:U2`, so the historical `strip_prefix("peer:{pid}:")` recovery path in
/// `memory_access`'s `memory_list` can never let one peer see another's keys.
/// An empty `peer_id` is still rejected: `peer::{key}` is ambiguous with a
/// `None`-scope key literally named `:{key}` and would split a namespace.
/// Similarly, an LLM-supplied key starting with `peer:` is rejected so the tool
/// layer cannot plant rows that appear to come from a different peer namespace.
pub(super) fn peer_scoped_key(
    key: &str,
    peer_id: Option<&str>,
) -> Result<String, librefang_runtime::kernel_handle::KernelOpError> {
    use librefang_runtime::kernel_handle::KernelOpError;
    if key.starts_with("peer:") {
        return Err(KernelOpError::InvalidInput(format!(
            "memory key '{key}' must not start with reserved 'peer:' prefix"
        )));
    }
    match peer_id {
        Some(pid) => {
            if pid.is_empty() {
                return Err(KernelOpError::InvalidInput(
                    "peer_id must not be empty (ambiguous with global scope)".to_string(),
                ));
            }
            let escaped_pid = escape_peer_id(pid);
            Ok(format!("peer:{escaped_pid}:{key}"))
        }
        None => Ok(key.to_string()),
    }
}

/// Tag prefixes the kernel owns rather than the operator.
///
/// These three are written by hand-role activation (`kernel/hands_lifecycle.rs`) and are the only tags any code branches on.
/// `hand:` and `hand_role:` route the agent's workspace under `hands/<hand>/<role>` instead of `agents/<name>` (`backfill_workspace_dir` in `kernel/workspace_setup.rs`), and `hand:` alone marks an agent autonomous for idle-wake purposes (`kernel/messaging.rs`), decides whether a tool call needs an approval gate (`kernel/handles/approval_gate.rs`), and scopes structured memory (`librefang-memory/src/structured.rs`).
/// An operator who could add or drop one would be relocating a workspace and re-deciding an approval boundary through a field that reads like free-form metadata, so [`merge_agent_tags`] keeps them out of operator reach in both directions.
const SYSTEM_TAG_PREFIXES: [&str; 3] = ["hand:", "hand_instance:", "hand_role:"];

/// Whether a tag belongs to the kernel rather than the operator.
fn is_system_tag(tag: &str) -> bool {
    SYSTEM_TAG_PREFIXES
        .iter()
        .any(|prefix| tag.starts_with(prefix))
}

/// Merge an incoming tag list over the tags an agent is currently running with.
///
/// System-owned tags are taken from `live` and operator-owned tags from `incoming`, so a caller that submits a whole manifest can freely rewrite the operator half without being able to forge, drop or preserve-by-accident the kernel half.
/// A submitted system tag is dropped rather than rejected: the round-trip a dashboard performs is GET-manifest / edit-one-field / PUT-manifest, and echoing back the `hand:` tags it was just shown must not fail the write.
///
/// Ordering is `live` system tags first, then `incoming` operator tags in submitted order, de-duplicated.
/// That is deterministic for a given pair of inputs, which matters because `manifest.tags` is stringified into the router's agent summary (`librefang-kernel-router`) and a reordering there would invalidate provider prompt caches for no reason (#3298).
pub(super) fn merge_agent_tags(live: &[String], incoming: &[String]) -> Vec<String> {
    let mut merged: Vec<String> = live
        .iter()
        .filter(|tag| is_system_tag(tag))
        .cloned()
        .collect();
    for tag in incoming.iter().filter(|tag| !is_system_tag(tag)) {
        if !merged.contains(tag) {
            merged.push(tag.clone());
        }
    }
    merged
}

#[cfg(test)]
mod tag_merge_tests {
    use super::merge_agent_tags;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn operator_tags_are_replaced_wholesale() {
        assert_eq!(
            merge_agent_tags(&v(&["research", "beta"]), &v(&["prod"])),
            v(&["prod"]),
            "an operator must be able to drop a tag, not just add one"
        );
    }

    #[test]
    fn empty_incoming_clears_operator_tags() {
        assert_eq!(
            merge_agent_tags(&v(&["research"]), &[]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn system_tags_survive_an_operator_rewrite() {
        assert_eq!(
            merge_agent_tags(
                &v(&["hand:clipper", "hand_role:editor", "research"]),
                &v(&["prod"])
            ),
            v(&["hand:clipper", "hand_role:editor", "prod"]),
            "hand membership drives workspace routing and approval gating; an operator tag edit must not move it"
        );
    }

    #[test]
    fn submitted_system_tags_are_dropped_not_honoured() {
        assert_eq!(
            merge_agent_tags(
                &v(&["hand:real"]),
                &v(&["hand:forged", "hand_role:admin", "ok"])
            ),
            v(&["hand:real", "ok"]),
            "an operator must not be able to forge hand membership through the tags field"
        );
    }

    #[test]
    fn duplicate_operator_tags_collapse() {
        assert_eq!(
            merge_agent_tags(&[], &v(&["a", "b", "a"])),
            v(&["a", "b"]),
            "tags are stringified into the router prompt, so a duplicate is noise in the cache key"
        );
    }

    #[test]
    fn is_deterministic_and_order_preserving() {
        let live = v(&["hand:h", "old"]);
        assert_eq!(
            merge_agent_tags(&live, &v(&["z", "a"])),
            v(&["hand:h", "z", "a"]),
            "submitted order is preserved verbatim so repeated identical writes are byte-identical"
        );
    }
}

#[cfg(test)]
mod context_window_tests {
    use super::resolve_context_window;
    use librefang_runtime::model_catalog::ModelCatalog;
    use librefang_types::agent::ModelConfig;
    use librefang_types::model_catalog::{
        ContextWindowSource, ModelCatalogEntry, ModelOverrides, ModelTier,
    };

    /// The resolved size on its own.
    ///
    /// The precedence assertions below predate provenance and are about which *number* wins; `source_of` covers which layer is named for it, so each test reads as one claim.
    fn resolve(
        catalog: &ModelCatalog,
        model: &ModelConfig,
        session_hint: Option<u64>,
    ) -> Option<usize> {
        resolve_context_window(catalog, model, session_hint).map(|r| r.tokens)
    }

    /// The layer that produced the resolved window, or `None` when nothing did.
    fn source_of(
        catalog: &ModelCatalog,
        model: &ModelConfig,
        session_hint: Option<u64>,
    ) -> Option<ContextWindowSource> {
        resolve_context_window(catalog, model, session_hint).map(|r| r.source)
    }

    fn catalog() -> ModelCatalog {
        ModelCatalog::from_entries(
            vec![
                ModelCatalogEntry {
                    id: "claude-sonnet-4-6".to_string(),
                    display_name: "Claude Sonnet 4.6".to_string(),
                    provider: "anthropic".to_string(),
                    tier: ModelTier::Smart,
                    context_window: 200_000,
                    ..Default::default()
                },
                // An image model — the catalog stores 0 for "not applicable".
                ModelCatalogEntry {
                    id: "dall-e-3".to_string(),
                    display_name: "DALL-E 3".to_string(),
                    provider: "openai".to_string(),
                    tier: ModelTier::Smart,
                    context_window: 0,
                    ..Default::default()
                },
            ],
            vec![],
        )
    }

    fn model(provider: &str, name: &str, context_window: Option<u64>) -> ModelConfig {
        ModelConfig {
            provider: provider.to_string(),
            model: name.to_string(),
            context_window,
            ..Default::default()
        }
    }

    /// The bug in #6568: an operator sets `context_window` in agent.toml for a
    /// model the catalog does not know — exactly what the runtime's warning tells
    /// them to do — and it was ignored, leaving the 8192 fallback in place.
    #[test]
    fn manifest_override_wins_for_an_unknown_model() {
        let resolved = resolve(
            &catalog(),
            &model("deepseek", "deepseek-v4-flash", Some(131_072)),
            None,
        );
        assert_eq!(resolved, Some(131_072));
    }

    #[test]
    fn manifest_override_wins_over_the_catalog() {
        let resolved = resolve(
            &catalog(),
            &model("anthropic", "claude-sonnet-4-6", Some(64_000)),
            None,
        );
        assert_eq!(
            resolved,
            Some(64_000),
            "an explicit operator value must beat the catalog entry"
        );
    }

    #[test]
    fn falls_back_to_the_catalog_without_an_override() {
        let resolved = resolve(
            &catalog(),
            &model("anthropic", "claude-sonnet-4-6", None),
            None,
        );
        assert_eq!(resolved, Some(200_000));
    }

    #[test]
    fn a_zero_override_is_ignored() {
        let resolved = resolve(
            &catalog(),
            &model("anthropic", "claude-sonnet-4-6", Some(0)),
            None,
        );
        assert_eq!(resolved, Some(200_000), "0 means unset, not a real window");
    }

    #[test]
    fn a_zero_catalog_window_falls_through_to_the_session_hint() {
        // Image / audio entries carry 0; feeding that into budget math would
        // divide by an empty window.
        let resolved = resolve(&catalog(), &model("openai", "dall-e-3", None), Some(48_000));
        assert_eq!(resolved, Some(48_000));
    }

    #[test]
    fn session_hint_is_last() {
        let resolved = resolve(
            &catalog(),
            &model("anthropic", "claude-sonnet-4-6", None),
            Some(48_000),
        );
        assert_eq!(
            resolved,
            Some(200_000),
            "the catalog is authoritative over a stale persisted session value"
        );
    }

    #[test]
    fn a_zero_session_hint_is_ignored() {
        let resolved = resolve(
            &catalog(),
            &model("deepseek", "deepseek-v4-flash", None),
            Some(0),
        );
        assert_eq!(
            resolved, None,
            "nothing resolved — caller applies its fallback"
        );
    }

    #[test]
    fn returns_none_when_nothing_resolves() {
        let resolved = resolve(
            &catalog(),
            &model("deepseek", "deepseek-v4-flash", None),
            None,
        );
        assert_eq!(resolved, None);
    }

    /// Refs #7774. The per-model operator override beats the catalog entry —
    /// including one whose window came from a discovery probe rather than the
    /// registry. This is the precedence contract the dashboard, the API and the
    /// agent loop all rely on: operator override > discovered value > fallback.
    #[test]
    fn a_model_level_override_beats_the_catalog_value() {
        let mut cat = catalog();
        cat.set_overrides(
            "anthropic:claude-sonnet-4-6".to_string(),
            ModelOverrides {
                context_window: Some(48_000),
                ..Default::default()
            },
        );
        let resolved = resolve(&cat, &model("anthropic", "claude-sonnet-4-6", None), None);
        assert_eq!(resolved, Some(48_000));
    }

    /// Refs #7774. The reported case: a gateway-served model the catalog has
    /// never heard of. Before this the chain fell straight through to the agent
    /// loop's 8192 and the agent hit an overflow that did not exist.
    #[test]
    fn a_model_level_override_resolves_a_model_the_catalog_does_not_know() {
        let mut cat = catalog();
        cat.set_overrides(
            "litellm:sensor-model-generic-high".to_string(),
            ModelOverrides {
                context_window: Some(16_384),
                ..Default::default()
            },
        );
        let resolved = resolve(
            &cat,
            &model("litellm", "sensor-model-generic-high", None),
            None,
        );
        assert_eq!(resolved, Some(16_384));
    }

    /// Refs #7774 / #6568. The per-agent `agent.toml` value stays the most
    /// specific layer — a model-level correction is inherited by every agent,
    /// so an agent that states its own window must still win.
    #[test]
    fn the_agent_toml_value_still_wins_over_a_model_level_override() {
        let mut cat = catalog();
        cat.set_overrides(
            "anthropic:claude-sonnet-4-6".to_string(),
            ModelOverrides {
                context_window: Some(48_000),
                ..Default::default()
            },
        );
        let resolved = resolve(
            &cat,
            &model("anthropic", "claude-sonnet-4-6", Some(96_000)),
            None,
        );
        assert_eq!(resolved, Some(96_000));
    }

    /// Refs #7774. An override on some *other* model, and an override that
    /// carries only inference parameters, both leave the chain exactly where it
    /// was. This is the backward-compatibility guard: adding the field must not
    /// move a single existing install's resolved window.
    #[test]
    fn an_absent_limit_override_leaves_the_chain_unchanged() {
        let mut cat = catalog();
        cat.set_overrides(
            "anthropic:claude-sonnet-4-6".to_string(),
            ModelOverrides {
                temperature: Some(0.3),
                max_tokens: Some(4_096),
                ..Default::default()
            },
        );
        cat.set_overrides(
            "openai:some-other-model".to_string(),
            ModelOverrides {
                context_window: Some(1_000),
                ..Default::default()
            },
        );
        assert_eq!(
            resolve(&cat, &model("anthropic", "claude-sonnet-4-6", None), None),
            Some(200_000),
            "the catalog value must still be what resolves"
        );
        assert_eq!(
            resolve(
                &cat,
                &model("deepseek", "deepseek-v4-flash", None),
                Some(48_000)
            ),
            Some(48_000),
            "the session hint must still be the last resort"
        );
        assert_eq!(
            resolve(&cat, &model("deepseek", "deepseek-v4-flash", None), None),
            None,
            "nothing resolved — the caller's fallback still applies"
        );
    }

    /// Refs #7774. An override of `0` — what a cleared dashboard field could
    /// submit — must not pin the window to zero and poison the budget math.
    #[test]
    fn a_zero_model_level_override_falls_through_to_the_catalog() {
        let mut cat = catalog();
        cat.set_overrides(
            "anthropic:claude-sonnet-4-6".to_string(),
            ModelOverrides {
                context_window: Some(0),
                ..Default::default()
            },
        );
        let resolved = resolve(&cat, &model("anthropic", "claude-sonnet-4-6", None), None);
        assert_eq!(resolved, Some(200_000));
    }

    /// Refs #7774. An image model carries no window, and an override must be
    /// able to supply one without the catalog's `0` shadowing it.
    #[test]
    fn an_override_supplies_a_window_the_catalog_stores_as_zero() {
        let mut cat = catalog();
        cat.set_overrides(
            "openai:dall-e-3".to_string(),
            ModelOverrides {
                context_window: Some(4_096),
                ..Default::default()
            },
        );
        let resolved = resolve(&cat, &model("openai", "dall-e-3", None), None);
        assert_eq!(resolved, Some(4_096));
    }

    /// Guards the `session_hint: None` choice at the compaction gate.
    ///
    /// `session.context_window_tokens` holds whatever the previous turn
    /// resolved, which for an agent hit by #6568 is the stale 8192 fallback.
    /// The gate's miss-default is a 200K *global default window*, not the agent
    /// loop's conservative unknown-model fallback, so passing the session value
    /// in would rank a stale 8192 above 200K and make compaction fire much
    /// earlier than before this change. Passing `None` keeps the old default.
    #[test]
    fn a_stale_session_hint_would_beat_the_compaction_gate_default() {
        let unknown = model("deepseek", "deepseek-v4-flash", None);
        // What the gate must NOT do: rank the stale value above its default.
        let with_stale_hint = resolve(&catalog(), &unknown, Some(8192));
        assert_eq!(with_stale_hint, Some(8192));
        // What the gate does: no hint, so its own `unwrap_or(200_000)` applies.
        let without_hint = resolve(&catalog(), &unknown, None);
        assert_eq!(without_hint, None);
    }

    /// Refs #7774 item 5. Every layer names itself, so a surface reporting the
    /// window can say where it came from.
    ///
    /// Without this an operator reads one number for four different facts: a
    /// window they set, a window the registry declared, a window an earlier
    /// turn happened to persist, and a window nobody knows.
    /// The reported incident is the last one — 8192 assumed against a real 16K — and it is indistinguishable from the others by size alone.
    #[test]
    fn each_layer_names_itself_as_the_source() {
        let mut cat = catalog();
        cat.set_overrides(
            "litellm:sensor-model-generic-high".to_string(),
            ModelOverrides {
                context_window: Some(16_384),
                ..Default::default()
            },
        );

        assert_eq!(
            source_of(
                &cat,
                &model("anthropic", "claude-sonnet-4-6", Some(96_000)),
                None
            ),
            Some(ContextWindowSource::AgentOverride),
        );
        assert_eq!(
            source_of(
                &cat,
                &model("litellm", "sensor-model-generic-high", None),
                None
            ),
            Some(ContextWindowSource::ModelOverride),
        );
        assert_eq!(
            source_of(&cat, &model("anthropic", "claude-sonnet-4-6", None), None),
            Some(ContextWindowSource::Catalog),
        );
        assert_eq!(
            source_of(
                &cat,
                &model("deepseek", "deepseek-v4-flash", None),
                Some(48_000)
            ),
            Some(ContextWindowSource::SessionHint),
        );
        assert_eq!(
            source_of(&cat, &model("deepseek", "deepseek-v4-flash", None), None),
            None,
            "nothing resolved — the caller labels its own fallback",
        );
    }

    /// A model-level override of a window the catalog also declares reports the
    /// override, not the catalog.
    ///
    /// The two layers are ranked inside one `effective_limits_for_manifest` call, so this is the assertion that keeps the value and its label from being computed by different rules.
    #[test]
    fn an_override_shadowing_a_catalog_entry_reports_the_override() {
        let mut cat = catalog();
        cat.set_overrides(
            "anthropic:claude-sonnet-4-6".to_string(),
            ModelOverrides {
                context_window: Some(48_000),
                ..Default::default()
            },
        );
        assert_eq!(
            resolve_context_window(&cat, &model("anthropic", "claude-sonnet-4-6", None), None)
                .map(|r| (r.tokens, r.source)),
            Some((48_000, ContextWindowSource::ModelOverride)),
        );
    }

    /// A zero override does not get to claim provenance either: it falls
    /// through, and the catalog is named as the source of the value that wins.
    #[test]
    fn a_zero_override_reports_the_catalog_as_the_source() {
        let mut cat = catalog();
        cat.set_overrides(
            "anthropic:claude-sonnet-4-6".to_string(),
            ModelOverrides {
                context_window: Some(0),
                ..Default::default()
            },
        );
        assert_eq!(
            source_of(&cat, &model("anthropic", "claude-sonnet-4-6", None), None),
            Some(ContextWindowSource::Catalog),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_agent_example_grants_tools_matching_its_scopes() {
        let source = include_str!("../../../../examples/custom-agent/agent.toml");
        let manifest: AgentManifest = toml::from_str(source).unwrap();
        let caps = manifest_to_capabilities(&manifest);

        assert!(caps.contains(&Capability::ToolInvoke("web_fetch".into())));
        assert!(caps.contains(&Capability::NetConnect("*".into())));
        assert!(caps.contains(&Capability::ToolInvoke("memory_store".into())));
        assert!(caps.contains(&Capability::ToolInvoke("memory_recall".into())));
        assert!(caps.contains(&Capability::MemoryRead("self.*".into())));
        assert!(caps.contains(&Capability::MemoryWrite("self.*".into())));
        assert_eq!(manifest.resources.max_cost_per_hour_usd, 1.0);
    }

    const HAND_TOML: &str = r#"
id = "jarvis"
version = "1.0.0"
name = "Jarvis"
description = "test"
category = "other"

[agents.operator]
name = "jarvis-operator"
description = "vault operator"
module = "builtin:chat"

[agents.operator.model]
provider = "openrouter"
model = "qwen/qwen3.6-plus"
system_prompt = "You are JARVIS."
"#;

    #[test]
    fn extract_matches_bare_manifest_name() {
        let m = extract_manifest_from_hand_toml(HAND_TOML, "jarvis-operator");
        assert!(m.is_some(), "must match manifest.name");
    }

    #[test]
    fn extract_matches_role_key() {
        let m = extract_manifest_from_hand_toml(HAND_TOML, "operator");
        assert!(m.is_some(), "must match [agents.<role>] key");
    }

    #[test]
    fn extract_matches_canonical_colon_form() {
        // "{hand_id}:{manifest.name}" — what the kernel stamps at activation
        // and what `agents.name` in the DB actually stores.
        let m = extract_manifest_from_hand_toml(HAND_TOML, "jarvis:jarvis-operator");
        assert!(
            m.is_some(),
            "must match the canonical \"{{hand_id}}:{{manifest.name}}\" form"
        );
    }

    #[test]
    fn extract_matches_legacy_dash_qualifier() {
        // Use a hand whose role-key and manifest.name diverge so the
        // "{hand_id}-{role}" form is distinguishable from form 1.
        let toml = r#"
id = "research"
version = "1.0.0"
name = "Research"
description = "t"
category = "other"

[agents.lead]
name = "completely-different-name"
description = "d"
module = "builtin:chat"

[agents.lead.model]
provider = "openrouter"
model = "x"
system_prompt = "p"
"#;
        // "{hand_id}-{role}" → "research-lead"
        let m = extract_manifest_from_hand_toml(toml, "research-lead");
        assert!(m.is_some(), "must match \"{{hand_id}}-{{role}}\" qualifier");
    }

    #[test]
    fn extract_returns_none_for_unknown_agent() {
        assert!(extract_manifest_from_hand_toml(HAND_TOML, "no-such-agent").is_none());
    }

    #[test]
    fn extract_preserves_nested_model_system_prompt() {
        // Regression: AgentManifest::deserialize is lenient and will accept a
        // hand.toml as a partial AgentManifest — top-level `name`/`description`
        // get picked up, but `model.system_prompt` (nested under
        // `[agents.<role>.model]`) is missed and ModelConfig::default() kicks
        // in with the stub "You are a helpful AI agent." prompt.
        //
        // The boot loop must therefore call extract_manifest_from_hand_toml
        // BEFORE falling back to the flat parse. This test verifies the
        // extractor itself returns the nested prompt verbatim — the
        // call-site ordering is enforced by the boot path.
        let m = extract_manifest_from_hand_toml(HAND_TOML, "jarvis:jarvis-operator")
            .expect("hand-extraction must match canonical name");
        assert_eq!(
            m.model.system_prompt, "You are JARVIS.",
            "extracted manifest must preserve nested [agents.<role>.model].system_prompt"
        );
    }

    fn make_settings() -> Vec<librefang_hands::HandSetting> {
        vec![librefang_hands::HandSetting {
            key: "stt".to_string(),
            label: "STT".to_string(),
            description: String::new(),
            setting_type: librefang_hands::HandSettingType::Select,
            default: "groq".to_string(),
            options: vec![librefang_hands::HandSettingOption {
                value: "groq".to_string(),
                label: "Groq".to_string(),
                provider_env: Some("GROQ_API_KEY".to_string()),
                binary: None,
            }],
            env_var: None,
        }]
    }

    fn manifest_with_prompt(prompt: &str) -> AgentManifest {
        let mut m = AgentManifest::default();
        m.model.system_prompt = prompt.to_string();
        m
    }

    /// A hand with only settings: base prompt preserved, one fenced tail, env list returned.
    #[test]
    fn settings_only_hand_renders_one_fenced_tail() {
        let def = parse_hand_with_settings(SINGLE_AGENT_HAND, "", &make_settings());
        let mut m = manifest_with_prompt("BASE");
        let env =
            rerender_hand_prompt_tails(&mut m, "main", &def, &std::collections::HashMap::new());
        assert!(
            m.model.system_prompt.starts_with("BASE\n\n---\n\n"),
            "base prompt must be preserved with the canonical separator; got: {}",
            m.model.system_prompt
        );
        assert!(m.model.system_prompt.contains(USER_CONFIG_TAIL_MARKER));
        assert!(!m.model.system_prompt.contains("## Reference Knowledge"));
        assert!(!m.model.system_prompt.contains("## Your Team"));
        // The renderer returns the *filtered* allowlist, so the selected option's
        // `provider_env = "GROQ_API_KEY"` is dropped by the secret blocklist rather
        // than handed to the subprocess. A hand cannot widen the passthrough into
        // the operator's credentials by naming one in a setting.
        assert!(
            env.is_empty(),
            "GROQ_API_KEY ends in a blocked word and must not survive; got: {env:?}"
        );
    }

    /// A hand declaring nothing renderable leaves the prompt byte-identical.
    #[test]
    fn hand_with_no_tails_leaves_prompt_untouched() {
        let def = parse_hand(SINGLE_AGENT_HAND, "");
        let mut m = manifest_with_prompt("BASE");
        let env =
            rerender_hand_prompt_tails(&mut m, "main", &def, &std::collections::HashMap::new());
        assert_eq!(m.model.system_prompt, "BASE");
        assert!(env.is_empty());
    }

    #[test]
    fn settings_tail_is_not_duplicated_on_repeated_renders() {
        let def = parse_hand_with_settings(SINGLE_AGENT_HAND, "", &make_settings());
        let cfg = std::collections::HashMap::new();
        let mut m = manifest_with_prompt("BASE");
        rerender_hand_prompt_tails(&mut m, "main", &def, &cfg);
        let after_first = m.model.system_prompt.clone();
        rerender_hand_prompt_tails(&mut m, "main", &def, &cfg);
        assert_eq!(
            m.model.system_prompt, after_first,
            "second invocation must not duplicate the tail"
        );
        assert_eq!(
            m.model
                .system_prompt
                .matches("## User Configuration")
                .count(),
            1,
            "exactly one settings block must be present"
        );
    }

    fn parse_hand(toml: &str, skill: &str) -> librefang_hands::HandDefinition {
        librefang_hands::registry::parse_hand_toml(toml, skill, std::collections::HashMap::new())
            .expect("hand toml must parse")
    }

    /// `parse_hand` plus a settings schema grafted on, so a fixture hand can exercise the settings tail without a second TOML constant per case.
    fn parse_hand_with_settings(
        toml: &str,
        skill: &str,
        settings: &[librefang_hands::HandSetting],
    ) -> librefang_hands::HandDefinition {
        let mut def = parse_hand(toml, skill);
        def.settings = settings.to_vec();
        def
    }

    const SINGLE_AGENT_HAND: &str = r#"
id = "demo"
version = "1.0.0"
name = "Demo"
description = "t"
category = "other"

[agents.main]
name = "demo-main"
description = "the only agent"
module = "builtin:chat"

[agents.main.model]
provider = "openrouter"
model = "x"
system_prompt = "BASE"
"#;

    const MULTI_AGENT_HAND: &str = r#"
id = "team"
version = "1.0.0"
name = "Team"
description = "t"
category = "other"

[agents.lead]
name = "team-lead"
description = "lead agent"
module = "builtin:chat"
invoke_hint = "delegates work"

[agents.lead.model]
provider = "openrouter"
model = "x"
system_prompt = "BASE-LEAD"

[agents.worker]
name = "team-worker"
description = "executes tasks"
module = "builtin:chat"

[agents.worker.model]
provider = "openrouter"
model = "x"
system_prompt = "BASE-WORKER"
"#;

    #[test]
    fn skill_reference_tail_is_appended_when_skill_present() {
        let def = parse_hand(SINGLE_AGENT_HAND, "RESOURCE A\nRESOURCE B");
        let mut m = manifest_with_prompt("BASE");
        rerender_hand_prompt_tails(&mut m, "main", &def, &std::collections::HashMap::new());
        assert!(
            m.model
                .system_prompt
                .contains("\n\n---\n\n## Reference Knowledge\n\nRESOURCE A"),
            "skill content must be appended under the Reference Knowledge heading"
        );
    }

    #[test]
    fn skill_reference_tail_is_not_duplicated_on_repeated_renders() {
        let def = parse_hand(SINGLE_AGENT_HAND, "STUFF");
        let cfg = std::collections::HashMap::new();
        let mut m = manifest_with_prompt("BASE");
        rerender_hand_prompt_tails(&mut m, "main", &def, &cfg);
        let after_first = m.model.system_prompt.clone();
        rerender_hand_prompt_tails(&mut m, "main", &def, &cfg);
        assert_eq!(m.model.system_prompt, after_first);
        assert_eq!(
            m.model
                .system_prompt
                .matches("## Reference Knowledge")
                .count(),
            1,
        );
    }

    /// A per-role `SKILL-<role>.md` must win over the hand-shared `SKILL.md`, and the role that has no override must still get the shared one.
    #[test]
    fn per_role_skill_content_overrides_the_shared_one() {
        let mut def = parse_hand(MULTI_AGENT_HAND, "SHARED PLAYBOOK");
        def.agent_skill_content
            .insert("lead".to_string(), "LEAD PLAYBOOK".to_string());
        let cfg = std::collections::HashMap::new();

        let mut lead = manifest_with_prompt("BASE-LEAD");
        rerender_hand_prompt_tails(&mut lead, "lead", &def, &cfg);
        assert!(lead.model.system_prompt.contains("LEAD PLAYBOOK"));
        assert!(!lead.model.system_prompt.contains("SHARED PLAYBOOK"));

        let mut worker = manifest_with_prompt("BASE-WORKER");
        rerender_hand_prompt_tails(&mut worker, "worker", &def, &cfg);
        assert!(worker.model.system_prompt.contains("SHARED PLAYBOOK"));
        assert!(!worker.model.system_prompt.contains("LEAD PLAYBOOK"));
    }

    /// A base prompt that *talks about* the User Configuration section must not be mistaken for one.
    ///
    /// This is the shape the registry's Trading Hand actually has: its Phase 0 says "Read **User Configuration** section for trading_mode, market_focus, …" and Phase 6 says "Read trading_mode from User Configuration", with no `## User Configuration` heading anywhere in the prompt body.
    /// The marker is the fenced `\n\n---\n\n## User Configuration` form precisely so prose like that cannot make `find()` truncate author-written instructions, which would silently delete the phases that consume the settings.
    #[test]
    fn settings_marker_ignores_prose_references_to_the_section() {
        let base = "You are a trader.\n\n\
                    2. Read **User Configuration** section for trading_mode and watchlist\n\n\
                    ## Phase 6\n\nRead trading_mode from User Configuration:";
        let mut m = manifest_with_prompt(base);
        let def = parse_hand_with_settings(SINGLE_AGENT_HAND, "", &make_settings());
        rerender_hand_prompt_tails(&mut m, "main", &def, &std::collections::HashMap::new());
        assert!(
            m.model.system_prompt.starts_with(base),
            "prose mentions must survive verbatim; got: {}",
            m.model.system_prompt
        );
        assert_eq!(
            manifest_for_diff(&m).model.system_prompt,
            base,
            "stripping back to the base prompt must land at the fence, not at a prose mention"
        );

        // And a re-render over that prompt stays stable rather than eating a phase per save.
        let def = parse_hand(MULTI_AGENT_HAND_WITH_SETTINGS, "");
        let mut live = manifest_with_prompt(base);
        rerender_hand_prompt_tails(
            &mut live,
            "lead",
            &def,
            &config_with("trading_mode", "live"),
        );
        let after_first = live.model.system_prompt.clone();
        rerender_hand_prompt_tails(
            &mut live,
            "lead",
            &def,
            &config_with("trading_mode", "live"),
        );
        assert_eq!(live.model.system_prompt, after_first);
        assert!(live.model.system_prompt.starts_with(base));
    }

    #[test]
    fn skill_reference_tail_replaces_stale_content() {
        let def_old = parse_hand(SINGLE_AGENT_HAND, "OLD");
        let def_new = parse_hand(SINGLE_AGENT_HAND, "NEW");
        let cfg = std::collections::HashMap::new();
        let mut m = manifest_with_prompt("BASE");
        rerender_hand_prompt_tails(&mut m, "main", &def_old, &cfg);
        rerender_hand_prompt_tails(&mut m, "main", &def_new, &cfg);
        assert!(m.model.system_prompt.contains("NEW"));
        assert!(!m.model.system_prompt.contains("OLD"));
    }

    #[test]
    fn skill_reference_tail_is_dropped_when_skill_removed() {
        // Hand previously had skill content; on next render the SKILL.md is gone.
        let def_with = parse_hand(SINGLE_AGENT_HAND, "STUFF");
        let def_without = parse_hand(SINGLE_AGENT_HAND, "");
        let cfg = std::collections::HashMap::new();
        let mut m = manifest_with_prompt("BASE");
        rerender_hand_prompt_tails(&mut m, "main", &def_with, &cfg);
        assert!(m.model.system_prompt.contains("STUFF"));
        rerender_hand_prompt_tails(&mut m, "main", &def_without, &cfg);
        assert_eq!(m.model.system_prompt, "BASE");
    }

    /// The hazard behind #6637's review: a playbook may legitimately contain a `---` rule followed by a `## Your Team` heading.
    /// Rendering must not mistake that author text for the rendered team tail and truncate the playbook there — and a second render must not compound the loss.
    #[test]
    fn skill_content_containing_a_team_heading_survives_intact() {
        let playbook =
            "STEP 1\n\n---\n\n## Your Team\n\nthe roster is documented here\n\nSTEP 2 MUST SURVIVE";
        let def = parse_hand(MULTI_AGENT_HAND, playbook);
        let cfg = std::collections::HashMap::new();
        let mut m = manifest_with_prompt("BASE-LEAD");

        rerender_hand_prompt_tails(&mut m, "lead", &def, &cfg);
        assert!(
            m.model.system_prompt.contains("STEP 2 MUST SURVIVE"),
            "author text after a `## Your Team` heading inside SKILL.md must reach the LLM; got: {}",
            m.model.system_prompt
        );
        assert!(
            m.model.system_prompt.contains("- **worker**:"),
            "the real team tail must still be appended; got: {}",
            m.model.system_prompt
        );

        // Idempotent even though the prompt now contains two `## Your Team` headings.
        let after_first = m.model.system_prompt.clone();
        rerender_hand_prompt_tails(&mut m, "lead", &def, &cfg);
        assert_eq!(
            m.model.system_prompt, after_first,
            "a second render must neither duplicate nor erode the prompt"
        );
    }

    #[test]
    fn team_block_absent_for_single_agent_hand() {
        let def = parse_hand(SINGLE_AGENT_HAND, "");
        let mut m = manifest_with_prompt("BASE");
        rerender_hand_prompt_tails(&mut m, "main", &def, &std::collections::HashMap::new());
        assert_eq!(m.model.system_prompt, "BASE");
    }

    #[test]
    fn team_block_lists_peers_excluding_self() {
        let def = parse_hand(MULTI_AGENT_HAND, "");
        let mut m = manifest_with_prompt("BASE");
        rerender_hand_prompt_tails(&mut m, "lead", &def, &std::collections::HashMap::new());
        let prompt = &m.model.system_prompt;
        assert!(
            prompt.contains("\n\n---\n\n## Your Team\n\n"),
            "team block must use the fenced marker so a literal `## Your Team` \
             elsewhere in the prompt cannot collide with the strip lookup"
        );
        assert!(prompt.contains("- **worker**:"));
        assert!(prompt.contains("executes tasks (use agent_send to message)"));
        assert!(
            !prompt.contains("- **lead**:"),
            "self must not appear in own team list"
        );
    }

    #[test]
    fn team_render_ignores_legacy_unfenced_tail() {
        // Lock-down for the LEGACY_TEAM_TAIL_MARKER cleanup. The pre-fence
        // form (`\n\n## Your Team`) is no longer recognised by the strip
        // logic, so a prompt carrying it gets a fresh fenced block appended
        // alongside (not replacing the legacy text). If a future change
        // reintroduces unfenced detection it should fail this assertion
        // first — that's a deliberate design choice, not a regression.
        //
        // The duplicate is harmless: drift loop never repopulates the
        // unfenced form, and any operator-visible `## Your Team` heading is
        // the fresh fenced one. The only path to this state is a
        // cross-version DB copy from pre-#3164 directly into a
        // post-cleanup binary, which is not a supported upgrade flow.
        let def = parse_hand(MULTI_AGENT_HAND, "");
        let mut m = manifest_with_prompt(
            "BASE\n\n## Your Team\n\n- **worker**: stale (use agent_send to message)",
        );
        rerender_hand_prompt_tails(&mut m, "lead", &def, &std::collections::HashMap::new());
        let prompt = &m.model.system_prompt;
        assert!(
            prompt.contains("stale"),
            "legacy unfenced text must NOT be stripped after cleanup; got: {prompt}"
        );
        assert!(
            prompt.contains("\n\n---\n\n## Your Team\n\n"),
            "fresh fenced block must still be appended; got: {prompt}"
        );
        assert_eq!(
            prompt.matches("## Your Team").count(),
            2,
            "exactly two team headings: the leftover legacy one and the new fenced one; got: {prompt}"
        );
    }

    #[test]
    fn team_block_uses_invoke_hint_when_present() {
        let def = parse_hand(MULTI_AGENT_HAND, "");
        let mut m = manifest_with_prompt("BASE");
        rerender_hand_prompt_tails(&mut m, "worker", &def, &std::collections::HashMap::new());
        // `lead` has invoke_hint = "delegates work", so the line must use that
        // instead of the manifest description.
        assert!(m.model.system_prompt.contains("- **lead**: delegates work"));
        assert!(!m.model.system_prompt.contains("lead agent"));
    }

    #[test]
    fn team_block_is_idempotent() {
        let def = parse_hand(MULTI_AGENT_HAND, "");
        let cfg = std::collections::HashMap::new();
        let mut m = manifest_with_prompt("BASE");
        rerender_hand_prompt_tails(&mut m, "lead", &def, &cfg);
        let after_first = m.model.system_prompt.clone();
        rerender_hand_prompt_tails(&mut m, "lead", &def, &cfg);
        assert_eq!(m.model.system_prompt, after_first);
        assert_eq!(m.model.system_prompt.matches("## Your Team").count(), 1);
    }

    #[test]
    fn manifest_for_diff_strips_all_known_tails() {
        // Build a prompt that contains all three tails in activation order.
        let base = "BASE";
        let mut m = manifest_with_prompt(base);
        let def = parse_hand_with_settings(MULTI_AGENT_HAND, "STUFF", &make_settings());
        rerender_hand_prompt_tails(&mut m, "lead", &def, &std::collections::HashMap::new());
        assert!(m.model.system_prompt.contains("## User Configuration"));
        assert!(m.model.system_prompt.contains("## Reference Knowledge"));
        assert!(m.model.system_prompt.contains("## Your Team"));

        let projected = manifest_for_diff(&m);
        assert_eq!(projected.model.system_prompt, base);
    }

    #[test]
    fn manifest_for_diff_strips_tails_in_any_subset() {
        // Only Team tail present (no settings, no skills).
        let mut m = manifest_with_prompt("BASE");
        let def = parse_hand(MULTI_AGENT_HAND, "");
        rerender_hand_prompt_tails(&mut m, "lead", &def, &std::collections::HashMap::new());
        let projected = manifest_for_diff(&m);
        assert_eq!(projected.model.system_prompt, "BASE");

        // Only Reference Knowledge.
        let mut m = manifest_with_prompt("BASE");
        let def = parse_hand(SINGLE_AGENT_HAND, "STUFF");
        rerender_hand_prompt_tails(&mut m, "main", &def, &std::collections::HashMap::new());
        let projected = manifest_for_diff(&m);
        assert_eq!(projected.model.system_prompt, "BASE");
    }

    #[test]
    fn manifest_for_diff_no_tails_returns_input_verbatim() {
        let m = manifest_with_prompt("BASE prompt with no rendered tails");
        let projected = manifest_for_diff(&m);
        assert_eq!(
            projected.model.system_prompt,
            "BASE prompt with no rendered tails"
        );
    }

    const MULTI_AGENT_HAND_WITH_SETTINGS: &str = r#"
id = "team"
version = "1.0.0"
name = "Team"
description = "t"
category = "other"

[[settings]]
key = "trading_mode"
label = "Trading Mode"
setting_type = "select"
default = "paper"

[[settings.options]]
value = "paper"
label = "Paper Trading"

[[settings.options]]
value = "live"
label = "Live Trading"
provider_env = "BROKER_ACCOUNT_ID"

[[requires]]
key = "feed_endpoint"
label = "Feed endpoint"
requirement_type = "env_var"
check_value = "MARKET_FEED_ENDPOINT"

# Deliberately names a daemon secret — must never survive the blocklist.
[[requires]]
key = "vault"
label = "Vault"
requirement_type = "api_key"
check_value = "LIBREFANG_VAULT_KEY"

[agents.lead]
name = "team-lead"
description = "lead agent"
module = "builtin:chat"
invoke_hint = "delegates work"

[agents.lead.model]
provider = "openrouter"
model = "x"
system_prompt = "BASE-LEAD"

[agents.worker]
name = "team-worker"
description = "executes tasks"
module = "builtin:chat"

[agents.worker.model]
provider = "openrouter"
model = "x"
system_prompt = "BASE-WORKER"
"#;

    fn config_with(key: &str, value: &str) -> std::collections::HashMap<String, serde_json::Value> {
        std::collections::HashMap::from([(
            key.to_string(),
            serde_json::Value::String(value.to_string()),
        )])
    }

    /// The #6636 hazard in isolation: re-rendering a *live* prompt — one that already carries all three tails — must not lose the reference or team blocks.
    #[test]
    fn rerender_preserves_reference_and_team_tails() {
        let def = parse_hand(MULTI_AGENT_HAND_WITH_SETTINGS, "STUFF");
        let mut m = manifest_with_prompt("BASE-LEAD");

        // Materialize the activation-time shape.
        rerender_hand_prompt_tails(&mut m, "lead", &def, &std::collections::HashMap::new());
        assert!(m.model.system_prompt.contains("Paper Trading"));

        // Now re-render with a changed setting, as a settings save does.
        rerender_hand_prompt_tails(&mut m, "lead", &def, &config_with("trading_mode", "live"));

        let prompt = &m.model.system_prompt;
        assert!(
            prompt.starts_with("BASE-LEAD\n\n---\n\n"),
            "author-written base prompt must survive; got: {prompt}"
        );
        assert!(
            prompt.contains("Live Trading"),
            "new value must be rendered; got: {prompt}"
        );
        assert!(
            !prompt.contains("Paper Trading"),
            "stale default must be gone; got: {prompt}"
        );
        assert!(
            prompt.contains("## Reference Knowledge\n\nSTUFF"),
            "skill tail must survive the re-render; got: {prompt}"
        );
        assert!(
            prompt.contains("- **worker**: executes tasks"),
            "team tail must survive the re-render; got: {prompt}"
        );
        for heading in [
            "## User Configuration",
            "## Reference Knowledge",
            "## Your Team",
        ] {
            assert_eq!(
                prompt.matches(heading).count(),
                1,
                "exactly one {heading} block must be present; got: {prompt}"
            );
        }
    }

    #[test]
    fn rerender_is_idempotent_and_order_stable() {
        let def = parse_hand(MULTI_AGENT_HAND_WITH_SETTINGS, "STUFF");
        let cfg = config_with("trading_mode", "live");
        let mut m = manifest_with_prompt("BASE-LEAD");

        rerender_hand_prompt_tails(&mut m, "lead", &def, &cfg);
        let after_first = m.model.system_prompt.clone();
        rerender_hand_prompt_tails(&mut m, "lead", &def, &cfg);
        assert_eq!(m.model.system_prompt, after_first);

        // Canonical order: settings -> reference -> team.
        let settings_at = after_first.find("## User Configuration").unwrap();
        let reference_at = after_first.find("## Reference Knowledge").unwrap();
        let team_at = after_first.find("## Your Team").unwrap();
        assert!(settings_at < reference_at && reference_at < team_at);
    }

    /// A hand whose prompt has no rendered tail yet — the very first save after an activation that had nothing to render — must still come out with the base prompt intact.
    #[test]
    fn rerender_from_bare_prompt_keeps_base() {
        let def = parse_hand(MULTI_AGENT_HAND_WITH_SETTINGS, "");
        let mut m = manifest_with_prompt("BASE-WORKER");
        rerender_hand_prompt_tails(&mut m, "worker", &def, &config_with("trading_mode", "live"));
        assert!(m.model.system_prompt.starts_with("BASE-WORKER\n\n---\n\n"));
        assert!(!m.model.system_prompt.contains("## Reference Knowledge"));
    }

    #[test]
    fn allowed_env_covers_selected_option_and_requirements() {
        let def = parse_hand(MULTI_AGENT_HAND_WITH_SETTINGS, "");

        // Default `paper` option declares no provider_env.
        let on_default = resolve_hand_allowed_env(&def, &std::collections::HashMap::new());
        assert_eq!(on_default, vec!["MARKET_FEED_ENDPOINT".to_string()]);

        // Switching to `live` pulls in that option's provider_env — the
        // whole point of re-resolving the allowlist on a settings save.
        let on_live = resolve_hand_allowed_env(&def, &config_with("trading_mode", "live"));
        assert_eq!(
            on_live,
            vec![
                "BROKER_ACCOUNT_ID".to_string(),
                "MARKET_FEED_ENDPOINT".to_string()
            ]
        );

        // The `[[requires]]` entry naming a daemon secret is filtered out on
        // both paths — a marketplace hand cannot widen the passthrough into
        // the operator's own credentials by declaring a requirement for one.
        for resolved in [&on_default, &on_live] {
            assert!(
                !resolved.contains(&"LIBREFANG_VAULT_KEY".to_string()),
                "blocklisted env var must never reach the allowlist; got: {resolved:?}"
            );
        }
    }

    #[test]
    fn extract_returns_none_for_standalone_agent_toml() {
        // Regression: standalone agent.toml files (no `id`, no `category`,
        // no `[agents.X]` table) must NOT be matched by the hand-extraction
        // path. HandDefinition deserialization should reject them so the
        // boot loop's `or_else(|| AgentManifest::deserialize(...))` fallback
        // kicks in for these files.
        let standalone = r#"
name = "my-agent"
description = "standalone"
module = "builtin:chat"

[model]
provider = "openrouter"
model = "x"
system_prompt = "p"
"#;
        assert!(
            extract_manifest_from_hand_toml(standalone, "my-agent").is_none(),
            "standalone agent.toml must not parse as a HandDefinition"
        );
    }
}
