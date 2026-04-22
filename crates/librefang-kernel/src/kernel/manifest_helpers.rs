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
            | "cerebras" | "sambanova" | "replicate" | "huggingface" | "ai21" | "codex"
            | "claude-code" | "copilot" | "github-copilot" | "qwen" | "zhipu" | "zai"
            | "moonshot" | "openrouter" | "volcengine" | "doubao" | "dashscope" => {
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
    } else if lower.starts_with("jamba") {
        Some("ai21".to_string())
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
/// `name` field (or role key) matches `agent_name`.
pub(super) fn extract_manifest_from_hand_toml(
    toml_str: &str,
    agent_name: &str,
) -> Option<librefang_types::agent::AgentManifest> {
    let def: librefang_hands::HandDefinition = toml::from_str(toml_str).ok()?;
    for (role, hand_agent) in &def.agents {
        if hand_agent.manifest.name == agent_name || role == agent_name {
            return Some(hand_agent.manifest.clone());
        }
    }
    // Also try matching by the "{hand_id}-{role}" convention used for spawned agents.
    for (role, hand_agent) in &def.agents {
        let qualified = format!("{}-{}", def.id, role);
        if qualified == agent_name {
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

// ─────────────────────────────────────────────────────────────────────────────
// UAR-AGENT-MD / agent.json loader
// ─────────────────────────────────────────────────────────────────────────────

/// Attempt to load an agent from a UAR-AGENT-MD (`.uar.md`) or pre-compiled
/// descriptor (`.agent.json`) file.
#[expect(
    dead_code,
    reason = "called from agent loader and /api/agents/uar handler"
)]
///
/// Returns `Ok(Some(manifest))` when the path has a recognised UAR extension
/// and parsing succeeds, `Ok(None)` when the extension does not match (so the
/// caller can fall through to the standard TOML loader), and `Err(message)`
/// on I/O or parse failures.
pub(super) fn try_load_uar_manifest(
    path: &std::path::Path,
) -> Result<Option<AgentManifest>, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

    // Accept *.uar.md and *.agent.json
    let is_uar = (ext == "json" && file_name.ends_with(".agent.json"))
        || (ext == "md" && file_name.ends_with(".uar.md"));

    if !is_uar {
        return Ok(None);
    }

    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read UAR artifact at {}: {e}", path.display()))?;

    let artifact = if ext == "md" {
        librefang_uar_spec::parser::parse(&content)
            .map_err(|e| format!("UAR-AGENT-MD parse error in {}: {e}", path.display()))?
    } else {
        librefang_uar_spec::parser::parse_json(&content)
            .map_err(|e| format!("agent.json parse error in {}: {e}", path.display()))?
    };

    let manifest = librefang_uar_spec::translator::artifact_to_manifest(&artifact)
        .map_err(|e| format!("UAR→AgentManifest translation error: {e}"))?;

    tracing::info!(
        path = %path.display(),
        agent_name = %manifest.name,
        agent_version = %manifest.version,
        "loaded agent from UAR artifact"
    );

    Ok(Some(manifest))
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
