//! Manifest -> capability conversion and small config helpers.
//!
//! Pure functions extracted from `kernel.rs`. None of these touch
//! `LibreFangKernel` itself — they operate on `AgentManifest`,
//! capability sets, and provider/model name strings.

use librefang_types::agent::*;
use librefang_types::capability::Capability;

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
            if !manifest.capabilities.memory_read.is_empty() {
                merged.memory_read = manifest.capabilities.memory_read.clone();
            }
            if !manifest.capabilities.memory_write.is_empty() {
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
    for scope in &effective_caps.memory_read {
        caps.push(Capability::MemoryRead(scope.clone()));
    }
    for scope in &effective_caps.memory_write {
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

/// Apply global budget defaults to an agent's resource quota.
///
/// When the global budget config specifies limits and the agent still has
/// the built-in defaults, override them so agents respect the user's config.
/// Apply a per-call deep-thinking override to a manifest clone.
///
/// - `Some(true)` — ensure the manifest has a `ThinkingConfig` (inserting the
///   default one if previously empty) so the driver enables reasoning.
/// - `Some(false)` — clear `manifest.thinking` so the driver does not request
///   thinking regardless of the manifest/global default.
/// - `None` — leave the manifest untouched.
pub(super) fn apply_thinking_override(
    manifest: &mut librefang_types::agent::AgentManifest,
    thinking_override: Option<bool>,
) {
    match thinking_override {
        Some(true) if manifest.thinking.is_none() => {
            manifest.thinking = Some(librefang_types::config::ThinkingConfig::default());
        }
        Some(false) => {
            manifest.thinking = None;
        }
        // Some(true) when thinking is already set — keep the existing budget
        // — and None when no override is requested are both no-ops.
        _ => {}
    }
}

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
            | "openrouter" | "volcengine" | "doubao" | "dashscope" => {
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

/// A well-known agent ID used for shared memory operations across agents.
/// This is a fixed UUID so all agents read/write to the same namespace.
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

/// Append (or refresh) the rendered `## User Configuration` block on a
/// manifest's `model.system_prompt` from a hand's `[[settings]]` schema +
/// instance config.
///
/// This is the single source of truth for the "settings -> system prompt"
/// materialization. Two call sites use it:
///
/// 1. Hand activation (`activate_hand`) — turns the disk TOML's bare prompt
///    into the runtime prompt with settings spliced in before save_agent.
/// 2. Boot-time TOML drift detection (`new_with_config`) — when the disk
///    manifest replaces the DB blob, the bare TOML doesn't carry the
///    settings tail (it's runtime-materialized, not persisted), so without
///    re-rendering here the agent loses its configured values on every
///    restart until somebody re-runs `hand activate`.
///
/// Idempotency: if the prompt already ends with a `## User Configuration`
/// tail, that tail is stripped before the freshly resolved one is appended.
/// This keeps repeated calls (e.g. drift loop firing back-to-back) from
/// growing the prompt without bound.
///
/// No-ops (no allocation, no mutation) when `settings` is empty or the
/// resolved prompt block is empty.
///
/// Returns the env-var allowlist that callers may want to merge into
/// `manifest.metadata["hand_allowed_env"]`.
pub(super) fn apply_settings_block_to_manifest(
    manifest: &mut AgentManifest,
    settings: &[librefang_hands::HandSetting],
    instance_config: &std::collections::HashMap<String, serde_json::Value>,
) -> Vec<String> {
    let resolved = librefang_hands::resolve_settings(settings, instance_config);

    if resolved.prompt_block.is_empty() {
        return resolved.env_vars;
    }

    // Strip any pre-existing settings tail so we replace rather than append.
    // Uses `strip_tail_block` (bounded by the next known marker) instead of
    // a blanket `truncate(idx)` so we don't accidentally clobber sibling
    // tails (`## Reference Knowledge`, `## Your Team`) that the activation
    // path appends after this one.
    strip_tail_block(&mut manifest.model.system_prompt, USER_CONFIG_TAIL_MARKER);

    // Append before any RK/Team blocks so the canonical order
    // (settings → reference knowledge → your team) is preserved across
    // re-renders. Find the earliest marker that follows the current prompt
    // body and splice the new block in front of it.
    let insert_at = ALL_TAIL_MARKERS
        .iter()
        .filter(|&&m| m != USER_CONFIG_TAIL_MARKER)
        .filter_map(|m| manifest.model.system_prompt.find(m))
        .min()
        .unwrap_or(manifest.model.system_prompt.len());

    let block = format!("\n\n---\n\n{}", resolved.prompt_block);
    manifest.model.system_prompt.insert_str(insert_at, &block);

    resolved.env_vars
}

/// Marker that introduces the rendered `## Reference Knowledge` block in the
/// system prompt.
///
/// Mirrors the activation path which appends
/// `"{prompt}\n\n---\n\n## Reference Knowledge\n\n{skill}"` (see
/// `activate_hand_with_id` in `kernel/mod.rs`). Used to detect and replace an
/// existing block rather than blindly appending a duplicate.
const REFERENCE_KNOWLEDGE_TAIL_MARKER: &str = "\n\n---\n\n## Reference Knowledge";

/// Marker that introduces the rendered `## Your Team` block in the system
/// prompt.
///
/// The activation path uses `"\n\n## Your Team\n\n{lines}"` (note the
/// double-newline preamble but no `---` separator — see
/// `activate_hand_with_id`). Treating the marker as the canonical anchor lets
/// us detect and replace an existing peer roster on idempotent re-applies.
const YOUR_TEAM_TAIL_MARKER: &str = "\n\n## Your Team";

/// All known runtime-materialized prompt-tail markers, in canonical activation
/// order. Used by individual `apply_*_to_manifest` helpers to determine where
/// "their" block ends — every helper must trim from its own marker up to
/// (but not including) the next marker that follows it in this list (or to
/// end-of-string when no later marker is present).
///
/// Keep this list in sync with the activation path in `activate_hand_with_id`
/// — adding a new tail without registering it here will let stale data leak
/// across the boot-time drift loop.
const ALL_TAIL_MARKERS: &[&str] = &[
    USER_CONFIG_TAIL_MARKER,
    REFERENCE_KNOWLEDGE_TAIL_MARKER,
    YOUR_TEAM_TAIL_MARKER,
];

/// Strip the block introduced by `marker` from `prompt`.
///
/// "Block" means: from the first occurrence of `marker` up to (but not
/// including) the next marker in `ALL_TAIL_MARKERS` that appears after it,
/// or to end-of-string when no later marker exists. Returns silently when
/// the marker is absent.
fn strip_tail_block(prompt: &mut String, marker: &str) {
    let Some(start) = prompt.find(marker) else {
        return;
    };
    let after = start + marker.len();
    // Find the closest later marker (any of the known tails) after our block.
    let next = ALL_TAIL_MARKERS
        .iter()
        .filter_map(|m| prompt[after..].find(m).map(|idx| after + idx))
        .min();
    match next {
        Some(end) => {
            prompt.replace_range(start..end, "");
        }
        None => {
            prompt.truncate(start);
        }
    }
}

/// Append (or refresh) the rendered `## Reference Knowledge` block on a
/// manifest's `model.system_prompt`.
///
/// This is the single source of truth for the "skill prompt_context →
/// system prompt" materialization for hand-derived agents. Two call sites
/// use it:
///
/// 1. Hand activation (`activate_hand_with_id`) — splices the agent's
///    effective skill content into the prompt before `save_agent`.
/// 2. Boot-time TOML drift detection (`new_with_config`) — when the disk
///    manifest replaces the DB blob, the bare TOML doesn't carry the
///    rendered tail (it's runtime-materialized), so without re-rendering
///    here the agent silently loses its skill knowledge on every restart.
///
/// `skill_content` is the resolved per-agent skill body (per-role override
/// when set, else the hand-shared content). When `None` or empty, any
/// pre-existing block is stripped and no new block is appended — this keeps
/// the helper safe to call on agents whose skill allowlist has shrunk to
/// nothing without leaving stale rendered content behind.
///
/// Idempotency: an existing `## Reference Knowledge` block is detected via
/// the marker constant and replaced rather than duplicated. Repeated calls
/// produce identical prompts.
pub(super) fn apply_reference_knowledge_to_manifest(
    manifest: &mut AgentManifest,
    skill_content: Option<&str>,
) {
    // Always strip a pre-existing block so shrinking-to-empty cleans up.
    strip_tail_block(
        &mut manifest.model.system_prompt,
        REFERENCE_KNOWLEDGE_TAIL_MARKER,
    );

    let Some(content) = skill_content else {
        return;
    };
    if content.is_empty() {
        return;
    }

    // Insert before any tail that follows RK in canonical order (`## Your
    // Team`) so re-renders stay sorted [settings → RK → team]. Append at
    // end-of-prompt when no later marker is present.
    let insert_at = manifest
        .model
        .system_prompt
        .find(YOUR_TEAM_TAIL_MARKER)
        .unwrap_or(manifest.model.system_prompt.len());

    let block = format!("\n\n---\n\n## Reference Knowledge\n\n{content}");
    manifest.model.system_prompt.insert_str(insert_at, &block);
}

/// Append (or refresh) the rendered `## Your Team` block on a manifest's
/// `model.system_prompt` from a list of pre-rendered peer lines.
///
/// `peer_lines` is the exact content already formatted by the caller (e.g.
/// `"- **role**: hint (use agent_send to message)"`). When empty, any
/// pre-existing block is stripped and nothing is appended — covers the
/// single-agent / no-peers case where the block must not appear.
///
/// Two call sites mirror the settings/reference-knowledge pattern:
///
/// 1. Hand activation — populates the team roster for multi-agent hands.
/// 2. Boot-time drift detection — re-renders after the disk TOML overwrite
///    so peer info survives restarts.
///
/// Idempotency: replaces any existing `## Your Team` block via the marker.
pub(super) fn apply_your_team_to_manifest(manifest: &mut AgentManifest, peer_lines: &[String]) {
    strip_tail_block(&mut manifest.model.system_prompt, YOUR_TEAM_TAIL_MARKER);

    if peer_lines.is_empty() {
        return;
    }

    manifest.model.system_prompt = format!(
        "{}\n\n## Your Team\n\n{}",
        manifest.model.system_prompt,
        peer_lines.join("\n")
    );
}

/// Build the team-roster lines for a given role within a hand definition.
///
/// Returns an empty vec for single-agent hands or when the role is the only
/// agent. The line format mirrors `activate_hand_with_id` exactly so the
/// drift-time render is byte-identical to the activation render.
pub(super) fn build_team_lines_for_role(
    def: &librefang_hands::HandDefinition,
    role: &str,
) -> Vec<String> {
    if !def.is_multi_agent() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    for (peer_role, peer_agent) in &def.agents {
        if peer_role == role {
            continue;
        }
        let hint = peer_agent
            .invoke_hint
            .as_deref()
            .unwrap_or(&peer_agent.manifest.description);
        lines.push(format!(
            "- **{peer_role}**: {hint} (use agent_send to message)"
        ));
    }
    lines
}

/// Resolve the effective skill-content body for a role within a hand
/// definition: per-role override (`SKILL-{role}.md`) takes precedence over
/// the shared `SKILL.md`. Returns `None` when neither exists.
///
/// Centralized here so the drift loop and the activation path can't drift
/// apart on resolution semantics.
pub(super) fn resolve_skill_content_for_role<'a>(
    def: &'a librefang_hands::HandDefinition,
    role: &str,
) -> Option<&'a str> {
    let role_lower = role.to_lowercase();
    def.agent_skill_content
        .get(&role_lower)
        .or(def.skill_content.as_ref())
        .map(|s| s.as_str())
}

pub fn shared_memory_agent_id() -> AgentId {
    AgentId(uuid::Uuid::from_bytes([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x01,
    ]))
}

/// Namespace a memory key by peer ID for per-user isolation.
/// When `peer_id` is `Some`, returns `"peer:{peer_id}:{key}"`.
/// When `None`, returns the key unchanged (global scope).
pub(super) fn peer_scoped_key(key: &str, peer_id: Option<&str>) -> String {
    match peer_id {
        Some(pid) => format!("peer:{pid}:{key}"),
        None => key.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn apply_settings_appends_tail_when_settings_present() {
        let mut m = manifest_with_prompt("BASE");
        let env = apply_settings_block_to_manifest(
            &mut m,
            &make_settings(),
            &std::collections::HashMap::new(),
        );
        assert!(
            m.model.system_prompt.contains("## User Configuration"),
            "settings tail must be appended"
        );
        assert!(
            m.model.system_prompt.starts_with("BASE\n\n---\n\n"),
            "base prompt must be preserved with the canonical separator"
        );
        assert_eq!(env, vec!["GROQ_API_KEY".to_string()]);
    }

    #[test]
    fn apply_settings_is_noop_when_settings_empty() {
        let mut m = manifest_with_prompt("BASE");
        let env = apply_settings_block_to_manifest(&mut m, &[], &std::collections::HashMap::new());
        assert_eq!(m.model.system_prompt, "BASE", "no settings -> no mutation");
        assert!(env.is_empty());
    }

    #[test]
    fn apply_settings_is_idempotent_on_repeated_calls() {
        let mut m = manifest_with_prompt("BASE");
        let cfg = std::collections::HashMap::new();
        apply_settings_block_to_manifest(&mut m, &make_settings(), &cfg);
        let after_first = m.model.system_prompt.clone();
        apply_settings_block_to_manifest(&mut m, &make_settings(), &cfg);
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

    #[test]
    fn apply_settings_returns_none_for_standalone_agent_toml_marker() {
        // Sanity: ensures the marker constant matches what `resolve_settings` emits.
        let mut m = manifest_with_prompt("BASE");
        apply_settings_block_to_manifest(
            &mut m,
            &make_settings(),
            &std::collections::HashMap::new(),
        );
        assert!(m.model.system_prompt.contains(USER_CONFIG_TAIL_MARKER));
    }

    // ── Reference Knowledge / Your Team helpers ─────────────────────────
    //
    // Regression guard for issue #3143: the boot-time TOML drift loop
    // overwrites the DB manifest with the bare disk TOML, which never
    // carries any of the runtime-rendered tails. Without idempotent
    // helpers re-running in the drift block, hand-derived agents lose
    // their skill knowledge and peer roster on every restart.

    fn make_hand_def(
        id: &str,
        roles: &[(&str, &str, Option<&str>)], // (role, description, invoke_hint)
        shared_skill: Option<&str>,
        per_agent_skill: &[(&str, &str)],
    ) -> librefang_hands::HandDefinition {
        use librefang_hands::{HandAgentManifest, HandDefinition};
        let mut agents = std::collections::BTreeMap::new();
        for (role, desc, hint) in roles {
            let m = AgentManifest {
                name: format!("{id}-{role}"),
                description: desc.to_string(),
                ..AgentManifest::default()
            };
            agents.insert(
                role.to_string(),
                HandAgentManifest {
                    coordinator: false,
                    invoke_hint: hint.map(|s| s.to_string()),
                    base: None,
                    manifest: m,
                },
            );
        }
        let def = HandDefinition {
            id: id.to_string(),
            version: "0.0.0".to_string(),
            name: id.to_string(),
            description: String::new(),
            category: librefang_hands::HandCategory::Other,
            icon: String::new(),
            tools: Vec::new(),
            skills: Vec::new(),
            mcp_servers: Vec::new(),
            allowed_plugins: Vec::new(),
            requires: Vec::new(),
            settings: Vec::new(),
            agents,
            dashboard: Default::default(),
            routing: Default::default(),
            skill_content: shared_skill.map(|s| s.to_string()),
            agent_skill_content: per_agent_skill
                .iter()
                .map(|(r, s)| (r.to_lowercase(), s.to_string()))
                .collect(),
            metadata: None,
            i18n: Default::default(),
        };
        // Touch fields the compiler needs touched (no-op for the test).
        let _ = def.is_multi_agent();
        def
    }

    #[test]
    fn apply_reference_knowledge_idempotent() {
        let mut m = manifest_with_prompt("BASE");
        apply_reference_knowledge_to_manifest(&mut m, Some("DOC BODY"));
        let after_first = m.model.system_prompt.clone();
        apply_reference_knowledge_to_manifest(&mut m, Some("DOC BODY"));
        assert_eq!(
            m.model.system_prompt, after_first,
            "second invocation must not duplicate the tail"
        );
        assert_eq!(
            m.model
                .system_prompt
                .matches("## Reference Knowledge")
                .count(),
            1,
            "exactly one reference-knowledge block must be present"
        );
        assert!(m.model.system_prompt.contains("DOC BODY"));
    }

    #[test]
    fn apply_reference_knowledge_replaces_stale_block() {
        let mut m = manifest_with_prompt("BASE");
        apply_reference_knowledge_to_manifest(&mut m, Some("OLD CONTENT"));
        apply_reference_knowledge_to_manifest(&mut m, Some("NEW CONTENT"));
        assert!(
            !m.model.system_prompt.contains("OLD CONTENT"),
            "stale block must be replaced, not appended alongside"
        );
        assert!(m.model.system_prompt.contains("NEW CONTENT"));
        assert_eq!(
            m.model
                .system_prompt
                .matches("## Reference Knowledge")
                .count(),
            1
        );
    }

    #[test]
    fn apply_reference_knowledge_no_skills_strips_block() {
        let mut m = manifest_with_prompt("BASE");
        apply_reference_knowledge_to_manifest(&mut m, Some("CONTENT"));
        assert!(m.model.system_prompt.contains("## Reference Knowledge"));
        // Hand re-rendered with no skill content (allowlist shrank to empty)
        // — block must be removed, not left as a stale leftover.
        apply_reference_knowledge_to_manifest(&mut m, None);
        assert!(
            !m.model.system_prompt.contains("## Reference Knowledge"),
            "no skill content -> block must be stripped"
        );
        assert_eq!(m.model.system_prompt, "BASE");
    }

    #[test]
    fn apply_your_team_idempotent() {
        let mut m = manifest_with_prompt("BASE");
        let lines = vec!["- **planner**: plans (use agent_send to message)".to_string()];
        apply_your_team_to_manifest(&mut m, &lines);
        let after_first = m.model.system_prompt.clone();
        apply_your_team_to_manifest(&mut m, &lines);
        assert_eq!(
            m.model.system_prompt, after_first,
            "second invocation must not duplicate the team block"
        );
        assert_eq!(
            m.model.system_prompt.matches("## Your Team").count(),
            1,
            "exactly one team block must be present"
        );
    }

    #[test]
    fn apply_your_team_no_peers_strips_block() {
        let mut m = manifest_with_prompt("BASE");
        apply_your_team_to_manifest(
            &mut m,
            &["- **planner**: plans (use agent_send to message)".to_string()],
        );
        assert!(m.model.system_prompt.contains("## Your Team"));
        apply_your_team_to_manifest(&mut m, &[]);
        assert!(
            !m.model.system_prompt.contains("## Your Team"),
            "no peers -> block must be stripped"
        );
        assert_eq!(m.model.system_prompt, "BASE");
    }

    #[test]
    fn build_team_lines_for_single_agent_returns_empty() {
        let def = make_hand_def("solo", &[("main", "the only agent", None)], None, &[]);
        assert!(
            build_team_lines_for_role(&def, "main").is_empty(),
            "single-agent hand must not produce a team roster"
        );
    }

    #[test]
    fn build_team_lines_excludes_self_and_uses_invoke_hint() {
        let def = make_hand_def(
            "research",
            &[
                ("lead", "team lead", Some("Use lead for routing")),
                ("worker", "fallback description", None),
            ],
            None,
            &[],
        );
        let lines = build_team_lines_for_role(&def, "lead");
        assert_eq!(lines.len(), 1, "self must be excluded");
        assert!(lines[0].contains("**worker**"));
        // Worker has no invoke_hint -> falls back to manifest.description.
        assert!(lines[0].contains("fallback description"));

        let lines = build_team_lines_for_role(&def, "worker");
        assert_eq!(lines.len(), 1);
        // Lead has invoke_hint -> takes precedence over description.
        assert!(lines[0].contains("Use lead for routing"));
    }

    #[test]
    fn resolve_skill_content_prefers_per_role_override() {
        let def = make_hand_def(
            "research",
            &[("lead", "d", None), ("worker", "d", None)],
            Some("SHARED"),
            &[("worker", "WORKER ONLY")],
        );
        assert_eq!(
            resolve_skill_content_for_role(&def, "lead"),
            Some("SHARED"),
            "no override -> shared content"
        );
        assert_eq!(
            resolve_skill_content_for_role(&def, "worker"),
            Some("WORKER ONLY"),
            "per-role override wins"
        );
        // Case-insensitive match (filenames are lowercased during scan).
        assert_eq!(
            resolve_skill_content_for_role(&def, "Worker"),
            Some("WORKER ONLY")
        );
    }

    #[test]
    fn drift_simulation_preserves_settings_and_renders_rk_and_team() {
        // End-to-end shape of the drift loop: simulate a manifest that already
        // carries all three rendered tails (settings, RK, Team), then "swap"
        // to a bare disk manifest (just the base prompt) and re-apply the
        // three helpers in canonical order. The result must contain all
        // three blocks again, in order, with the new content — not the stale
        // pre-swap content.
        let mut bare = manifest_with_prompt("BASE PROMPT");
        let cfg = std::collections::HashMap::new();
        apply_settings_block_to_manifest(&mut bare, &make_settings(), &cfg);
        apply_reference_knowledge_to_manifest(&mut bare, Some("FRESH SKILL"));
        apply_your_team_to_manifest(
            &mut bare,
            &["- **planner**: plans (use agent_send to message)".to_string()],
        );

        let prompt = &bare.model.system_prompt;
        let pos_settings = prompt
            .find("## User Configuration")
            .expect("settings block");
        let pos_rk = prompt
            .find("## Reference Knowledge")
            .expect("reference knowledge block");
        let pos_team = prompt.find("## Your Team").expect("team block");
        assert!(
            pos_settings < pos_rk && pos_rk < pos_team,
            "blocks must render in canonical order: settings -> reference knowledge -> your team"
        );
        assert!(prompt.contains("FRESH SKILL"));
        assert!(prompt.contains("**planner**"));
        assert!(prompt.starts_with("BASE PROMPT"));
    }

    #[test]
    fn drift_simulation_replaces_stale_rk_and_team_blocks() {
        // Simulate the full drift cycle: an entry manifest with stale tail
        // content gets overwritten by a fresh disk manifest (without tails),
        // then helpers re-render. Stale "OLD" content must be gone.
        let mut m = manifest_with_prompt("BASE");
        apply_reference_knowledge_to_manifest(&mut m, Some("OLD SKILL"));
        apply_your_team_to_manifest(
            &mut m,
            &["- **stale**: gone (use agent_send to message)".to_string()],
        );

        // Drift swap: replace prompt with bare disk version (no tails).
        m.model.system_prompt = "BASE".to_string();

        // Re-apply with fresh content.
        apply_reference_knowledge_to_manifest(&mut m, Some("FRESH SKILL"));
        apply_your_team_to_manifest(
            &mut m,
            &["- **planner**: plans (use agent_send to message)".to_string()],
        );

        let prompt = &m.model.system_prompt;
        assert!(prompt.contains("FRESH SKILL"));
        assert!(prompt.contains("**planner**"));
        assert!(!prompt.contains("OLD SKILL"));
        assert!(!prompt.contains("**stale**"));
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
