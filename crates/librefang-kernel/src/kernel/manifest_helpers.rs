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
    if let Some(idx) = manifest.model.system_prompt.find(USER_CONFIG_TAIL_MARKER) {
        manifest.model.system_prompt.truncate(idx);
    }

    manifest.model.system_prompt = format!(
        "{}\n\n---\n\n{}",
        manifest.model.system_prompt, resolved.prompt_block
    );

    resolved.env_vars
}

/// Append (or refresh) the rendered `## Reference Knowledge` block on a
/// manifest's `model.system_prompt` from a hand's skill content.
///
/// Per-role override (`def.agent_skill_content[role.to_lowercase()]`)
/// takes precedence over the hand-shared `def.skill_content`. When neither
/// is set, the call only strips any pre-existing tail without re-appending
/// — covers the case where the hand's SKILL.md was deleted and the prompt
/// must drop its now-stale reference section.
///
/// Idempotency: pre-existing `## Reference Knowledge` tails are stripped
/// before re-appending, so repeated calls (e.g. drift loop firing
/// back-to-back) do not duplicate the section.
///
/// Both call sites use this helper:
///
/// 1. Hand activation (`activate_hand_with_id`) — see `kernel/mod.rs`.
/// 2. Boot-time TOML drift detection — when the disk manifest replaces
///    the DB blob, the bare TOML doesn't carry the rendered tail and
///    without re-rendering here the agent loses skill discoverability on
///    every restart.
pub(super) fn apply_skill_reference_block_to_manifest(
    manifest: &mut AgentManifest,
    role: &str,
    def: &librefang_hands::HandDefinition,
) {
    // Always strip first — covers the case where skill content is now
    // empty (skill removed from hand) so the stale tail doesn't linger.
    //
    // Ordering note: at activation this helper runs BEFORE
    // `apply_team_block_to_manifest`, so a stale team tail (which sits
    // downstream of the skill marker) is also dropped by this truncate
    // and re-appended afterwards. If callers ever invert the order,
    // `apply_team_block_to_manifest` must be widened to strip both
    // markers — otherwise a re-render leaves the team block stranded
    // before the freshly appended skill block.
    if let Some(idx) = manifest
        .model
        .system_prompt
        .find(SKILL_REFERENCE_TAIL_MARKER)
    {
        manifest.model.system_prompt.truncate(idx);
    }

    let role_lower = role.to_lowercase();
    let effective_skill = def
        .agent_skill_content
        .get(&role_lower)
        .or(def.skill_content.as_ref());

    if let Some(skill_content) = effective_skill {
        if !skill_content.is_empty() {
            manifest.model.system_prompt = format!(
                "{}\n\n---\n\n## Reference Knowledge\n\n{}",
                manifest.model.system_prompt, skill_content
            );
        }
    }
}

/// Append (or refresh) the rendered `## Your Team` block on a manifest's
/// `model.system_prompt` from the hand's peer roster. No-op for
/// single-agent hands.
///
/// Idempotency: pre-existing `## Your Team` tails are stripped before
/// re-appending, so repeated calls do not duplicate the section.
///
/// `role` identifies the agent we're rendering for; the agent's own role
/// is excluded from the peer list. Each peer line uses
/// `hand_agent.invoke_hint` when set, falling back to the peer's manifest
/// description.
pub(super) fn apply_team_block_to_manifest(
    manifest: &mut AgentManifest,
    role: &str,
    def: &librefang_hands::HandDefinition,
) {
    // Always strip first — covers the case where the hand was edited from
    // multi-agent down to single-agent so the stale Team tail must drop.
    //
    // Ordering note: this helper is the LAST tail appended at activation
    // (settings -> reference -> team). Truncating at the team marker only
    // drops the team block itself — no later tail can be lost. If a future
    // change inserts a new tail after team, this strip will need to widen
    // (or that tail's helper must run before this one).
    if let Some(idx) = manifest.model.system_prompt.find(TEAM_TAIL_MARKER) {
        manifest.model.system_prompt.truncate(idx);
    }

    if !def.is_multi_agent() {
        return;
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

    if !peer_lines.is_empty() {
        let team_block = format!("\n\n---\n\n## Your Team\n\n{}", peer_lines.join("\n"));
        manifest.model.system_prompt = format!("{}{team_block}", manifest.model.system_prompt);
    }
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
    let mut earliest_idx: Option<usize> = None;
    for marker in [
        USER_CONFIG_TAIL_MARKER,
        SKILL_REFERENCE_TAIL_MARKER,
        TEAM_TAIL_MARKER,
    ] {
        if let Some(idx) = prompt.find(marker) {
            earliest_idx = Some(earliest_idx.map_or(idx, |cur| cur.min(idx)));
        }
    }
    if let Some(idx) = earliest_idx {
        prompt.truncate(idx);
    }
    copy
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

    fn parse_hand(toml: &str, skill: &str) -> librefang_hands::HandDefinition {
        librefang_hands::registry::parse_hand_toml(toml, skill, std::collections::HashMap::new())
            .expect("hand toml must parse")
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
    fn apply_skill_reference_appends_tail_when_skill_present() {
        let def = parse_hand(SINGLE_AGENT_HAND, "RESOURCE A\nRESOURCE B");
        let mut m = manifest_with_prompt("BASE");
        apply_skill_reference_block_to_manifest(&mut m, "main", &def);
        assert!(
            m.model
                .system_prompt
                .contains("\n\n---\n\n## Reference Knowledge\n\nRESOURCE A"),
            "skill content must be appended under the Reference Knowledge heading"
        );
    }

    #[test]
    fn apply_skill_reference_is_noop_when_skill_empty() {
        let def = parse_hand(SINGLE_AGENT_HAND, "");
        let mut m = manifest_with_prompt("BASE");
        apply_skill_reference_block_to_manifest(&mut m, "main", &def);
        assert_eq!(m.model.system_prompt, "BASE");
    }

    #[test]
    fn apply_skill_reference_is_idempotent() {
        let def = parse_hand(SINGLE_AGENT_HAND, "STUFF");
        let mut m = manifest_with_prompt("BASE");
        apply_skill_reference_block_to_manifest(&mut m, "main", &def);
        let after_first = m.model.system_prompt.clone();
        apply_skill_reference_block_to_manifest(&mut m, "main", &def);
        assert_eq!(m.model.system_prompt, after_first);
        assert_eq!(
            m.model
                .system_prompt
                .matches("## Reference Knowledge")
                .count(),
            1,
        );
    }

    #[test]
    fn apply_skill_reference_replaces_stale_tail_when_content_changes() {
        let def_old = parse_hand(SINGLE_AGENT_HAND, "OLD");
        let def_new = parse_hand(SINGLE_AGENT_HAND, "NEW");
        let mut m = manifest_with_prompt("BASE");
        apply_skill_reference_block_to_manifest(&mut m, "main", &def_old);
        apply_skill_reference_block_to_manifest(&mut m, "main", &def_new);
        assert!(m.model.system_prompt.contains("NEW"));
        assert!(!m.model.system_prompt.contains("OLD"));
    }

    #[test]
    fn apply_skill_reference_drops_tail_when_skill_removed() {
        // Hand previously had skill content; on next render the SKILL.md is gone.
        let def_with = parse_hand(SINGLE_AGENT_HAND, "STUFF");
        let def_without = parse_hand(SINGLE_AGENT_HAND, "");
        let mut m = manifest_with_prompt("BASE");
        apply_skill_reference_block_to_manifest(&mut m, "main", &def_with);
        assert!(m.model.system_prompt.contains("STUFF"));
        apply_skill_reference_block_to_manifest(&mut m, "main", &def_without);
        assert_eq!(m.model.system_prompt, "BASE");
    }

    #[test]
    fn apply_team_block_noop_for_single_agent_hand() {
        let def = parse_hand(SINGLE_AGENT_HAND, "");
        let mut m = manifest_with_prompt("BASE");
        apply_team_block_to_manifest(&mut m, "main", &def);
        assert_eq!(m.model.system_prompt, "BASE");
    }

    #[test]
    fn apply_team_block_appends_peers_excluding_self() {
        let def = parse_hand(MULTI_AGENT_HAND, "");
        let mut m = manifest_with_prompt("BASE");
        apply_team_block_to_manifest(&mut m, "lead", &def);
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
    fn apply_team_block_ignores_legacy_unfenced_tail() {
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
        apply_team_block_to_manifest(&mut m, "lead", &def);
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
    fn apply_team_block_uses_invoke_hint_when_present() {
        let def = parse_hand(MULTI_AGENT_HAND, "");
        let mut m = manifest_with_prompt("BASE");
        apply_team_block_to_manifest(&mut m, "worker", &def);
        // `lead` has invoke_hint = "delegates work", so the line must use that
        // instead of the manifest description.
        assert!(m.model.system_prompt.contains("- **lead**: delegates work"));
        assert!(!m.model.system_prompt.contains("lead agent"));
    }

    #[test]
    fn apply_team_block_is_idempotent() {
        let def = parse_hand(MULTI_AGENT_HAND, "");
        let mut m = manifest_with_prompt("BASE");
        apply_team_block_to_manifest(&mut m, "lead", &def);
        let after_first = m.model.system_prompt.clone();
        apply_team_block_to_manifest(&mut m, "lead", &def);
        assert_eq!(m.model.system_prompt, after_first);
        assert_eq!(m.model.system_prompt.matches("## Your Team").count(), 1);
    }

    #[test]
    fn manifest_for_diff_strips_all_known_tails() {
        // Build a prompt that contains all three tails in activation order.
        let base = "BASE";
        let mut m = manifest_with_prompt(base);
        apply_settings_block_to_manifest(
            &mut m,
            &make_settings(),
            &std::collections::HashMap::new(),
        );
        let def = parse_hand(MULTI_AGENT_HAND, "STUFF");
        apply_skill_reference_block_to_manifest(&mut m, "lead", &def);
        apply_team_block_to_manifest(&mut m, "lead", &def);
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
        apply_team_block_to_manifest(&mut m, "lead", &def);
        let projected = manifest_for_diff(&m);
        assert_eq!(projected.model.system_prompt, "BASE");

        // Only Reference Knowledge.
        let mut m = manifest_with_prompt("BASE");
        let def = parse_hand(SINGLE_AGENT_HAND, "STUFF");
        apply_skill_reference_block_to_manifest(&mut m, "main", &def);
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
