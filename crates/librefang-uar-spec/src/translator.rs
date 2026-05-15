//! Bidirectional translation between [`AgentArtifact`] and librefang's
//! [`AgentManifest`].
//!
//! # Direction: Artifact → Manifest  (`artifact_to_manifest`)
//!
//! Converts a UAR-compiled agent descriptor into the TOML-compatible struct
//! that librefang's kernel uses to spawn and configure agents. The mapping
//! follows these rules:
//!
//! | UAR field | `AgentManifest` field |
//! |---|---|
//! | `identity.name` | `name` |
//! | `metadata.version` | `version` |
//! | `metadata.description` | `description` |
//! | `metadata.author` | `author` |
//! | `metadata.tags` | `tags` |
//! | `capabilities.model` (provider/model) | `model.provider` + `model.model` |
//! | `identity.system_prompt` or `identity.persona` | `model.system_prompt` |
//! | `skills[*].id` | `skills` |
//! | `mcp_servers[*].id` | `mcp_servers` |
//! | `tools.allow` | `tool_allowlist` |
//! | `tools.deny` | `tool_blocklist` |
//! | `capabilities.code_execution` | `capabilities.shell = ["*"]` |
//! | `capabilities.web_browsing` | `capabilities.network = ["*"]` |
//!
//! # Direction: Manifest → Artifact  (`manifest_to_artifact`)
//!
//! Reconstructs a best-effort `AgentArtifact` from an `AgentManifest`. Useful
//! for exposing librefang agents over the A2A protocol.

use crate::error::Result;
use crate::types::{
    A2ASection, AgentArtifact, AgentCard, AgentCardCapabilities, BudgetsSection,
    CapabilitiesSection, ConversationMemory, ExecutionSection, IdentitySection, McpServersSection,
    MemorySection, MetadataSection, SkillRef, SkillsSection, ToolsSection,
};
use librefang_types::agent::{
    AgentManifest, ManifestCapabilities, ModelConfig, ResourceQuota, ScheduleMode,
};
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────────────────────
// AgentArtifact → AgentManifest
// ─────────────────────────────────────────────────────────────────────────────

/// Convert an [`AgentArtifact`] into a librefang [`AgentManifest`].
///
/// The resulting manifest is ready to be written to `agent.toml` or passed
/// directly to the kernel's agent loader.
///
/// # Errors
/// Returns [`UarSpecError::TranslationFailed`] if the model string in
/// `capabilities.model` cannot be split into `provider/model` format and no
/// fallback is available.
pub fn artifact_to_manifest(artifact: &AgentArtifact) -> Result<AgentManifest> {
    let (provider, model_name) = split_provider_model(
        artifact
            .capabilities
            .model
            .as_deref()
            .unwrap_or("openai/gpt-4o"),
    )?;

    let system_prompt = artifact
        .identity
        .system_prompt
        .clone()
        .unwrap_or_else(|| artifact.identity.persona.clone());

    let model = ModelConfig {
        provider,
        model: model_name,
        max_tokens: artifact
            .budgets
            .max_tokens_per_turn
            .map(|t| t as u32)
            .unwrap_or(4096),
        temperature: 0.7,
        system_prompt,
        api_key_env: None,
        base_url: None,
        extra_params: HashMap::new(),
        context_window: None,
        max_output_tokens: None,
    };

    let mut tool_allowlist: Vec<String> = artifact.tools.allow.clone();
    let tool_blocklist: Vec<String> = artifact.tools.deny.clone();
    // Tools from the tools section that are not just allow/deny lists
    for t in &artifact.tools.tools {
        if !tool_allowlist.contains(&t.name) {
            tool_allowlist.push(t.name.clone());
        }
    }

    let skills = artifact
        .skills
        .skills
        .iter()
        .map(|s| s.id.clone())
        .collect();

    let mcp_servers = artifact
        .mcp_servers
        .servers
        .iter()
        .map(|s| s.id.clone())
        .collect();

    let capabilities = ManifestCapabilities {
        network: if artifact.capabilities.web_browsing {
            vec!["*".to_owned()]
        } else {
            Vec::new()
        },
        tools: Vec::new(),
        memory_read: Vec::new(),
        memory_write: Vec::new(),
        agent_spawn: false,
        agent_message: Vec::new(),
        shell: if artifact.capabilities.code_execution {
            vec!["*".to_owned()]
        } else {
            Vec::new()
        },
        ofp_discover: false,
        ofp_connect: Vec::new(),
    };

    let tags = artifact.metadata.tags.clone();

    Ok(AgentManifest {
        name: artifact.identity.name.clone(),
        version: artifact.metadata.version.clone(),
        description: artifact.metadata.description.clone().unwrap_or_default(),
        author: artifact.metadata.author.clone().unwrap_or_default(),
        module: String::new(), // no WASM module for UAR agents
        schedule: ScheduleMode::default(),
        session_mode: librefang_types::agent::SessionMode::default(),
        model,
        fallback_models: Vec::new(),
        resources: ResourceQuota::default(),
        priority: librefang_types::agent::Priority::default(),
        capabilities,
        profile: None,
        tools: HashMap::new(),
        skills,
        skills_disabled: false,
        mcp_servers,
        metadata: HashMap::new(),
        tags,
        routing: None,
        autonomous: None,
        pinned_model: None,
        workspace: None,
        generate_identity_files: true,
        exec_policy: None,
        tool_allowlist,
        tool_blocklist,
        tools_disabled: false,
        response_format: None,
        enabled: true,
        allowed_plugins: Vec::new(),
        inherit_parent_context: true,
        thinking: None,
        context_injection: Vec::new(),
        is_hand: false,
        web_search_augmentation: librefang_types::agent::WebSearchAugmentationMode::default(),
        auto_dream_enabled: false,
        auto_dream_min_hours: None,
        auto_dream_min_sessions: None,
        show_progress: true,
        auto_evolve: true,
        workspaces: HashMap::new(),
        channel_overrides: None,
        max_history_messages: None,
        max_concurrent_invocations: None,
        cache_context: false,
        tool_exec_backend: None,
        skill_workshop: librefang_types::agent::SkillWorkshopConfig::default(),
        proactive_memory: librefang_types::memory::ProactiveMemoryOverrides::default(),
        mcp_disabled: false,
        compaction: None,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// AgentManifest → AgentArtifact
// ─────────────────────────────────────────────────────────────────────────────

/// Reconstruct an [`AgentArtifact`] from a librefang [`AgentManifest`].
///
/// This is the reverse of [`artifact_to_manifest`] and is used when exposing
/// librefang agents over the A2A protocol.
pub fn manifest_to_artifact(manifest: &AgentManifest) -> AgentArtifact {
    let model_str = format!("{}/{}", manifest.model.provider, manifest.model.model);

    AgentArtifact {
        schema: "uar-agent-descriptor/v1".to_owned(),
        agent_id: manifest.name.clone(),
        version: manifest.version.clone(),
        metadata: MetadataSection {
            version: manifest.version.clone(),
            description: Some(manifest.description.clone()),
            author: Some(manifest.author.clone()),
            license: None,
            tags: manifest.tags.clone(),
            created: None,
            updated: None,
        },
        identity: IdentitySection {
            name: manifest.name.clone(),
            role: manifest.description.clone(),
            persona: manifest.model.system_prompt.clone(),
            system_prompt: Some(manifest.model.system_prompt.clone()),
            greeting: None,
        },
        capabilities: CapabilitiesSection {
            streaming: false,
            file_upload: false,
            image_generation: false,
            code_execution: !manifest.capabilities.shell.is_empty(),
            web_browsing: !manifest.capabilities.network.is_empty(),
            model: Some(model_str),
            extensions: HashMap::new(),
        },
        skills: SkillsSection {
            skills: manifest
                .skills
                .iter()
                .map(|id| SkillRef {
                    id: id.clone(),
                    version: None,
                    required: false,
                    config: HashMap::new(),
                })
                .collect(),
        },
        tools: ToolsSection {
            tools: Vec::new(),
            allow: manifest.tool_allowlist.clone(),
            deny: manifest.tool_blocklist.clone(),
        },
        mcp_servers: McpServersSection {
            servers: manifest
                .mcp_servers
                .iter()
                .map(|id| crate::types::McpServerRef {
                    id: id.clone(),
                    url: String::new(),
                    transport: None,
                    tools: Vec::new(),
                    env: HashMap::new(),
                })
                .collect(),
        },
        memory: MemorySection {
            conversation: ConversationMemory::default(),
            persistent: None,
        },
        a2a: A2ASection::default(),
        budgets: BudgetsSection {
            max_tokens_per_turn: Some(manifest.model.max_tokens as u64),
            max_tokens_per_session: None,
            max_tool_calls_per_turn: None,
            max_cost_per_session_usd: None,
            timeout_seconds: None,
        },
        execution: ExecutionSection::default(),
        extensions: HashMap::new(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AgentManifest → AgentCard (A2A discovery)
// ─────────────────────────────────────────────────────────────────────────────

/// Build an [`AgentCard`] (served at `/.well-known/agent.json`) from a
/// librefang [`AgentManifest`].
///
/// `base_url` is the public-facing root URL of this librefang instance,
/// e.g. `"http://localhost:4545"`.
pub fn manifest_to_agent_card(
    manifest: &AgentManifest,
    agent_id: &str,
    base_url: &str,
) -> AgentCard {
    AgentCard {
        name: manifest.name.clone(),
        description: manifest.description.clone(),
        url: format!("{base_url}/a2a/agents/{agent_id}"),
        version: manifest.version.clone(),
        documentation_url: None,
        capabilities: AgentCardCapabilities {
            streaming: false, // ModelConfig has no streaming flag; extend via extra_params if needed
            push_notifications: false,
            state_transition_history: false,
        },
        skills: Vec::new(),
        default_input_modes: vec!["text".to_owned()],
        default_output_modes: vec!["text".to_owned()],
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Split a `"provider/model"` string into `(provider, model)`.
fn split_provider_model(s: &str) -> Result<(String, String)> {
    match s.split_once('/') {
        Some((p, m)) => Ok((p.to_owned(), m.to_owned())),
        None => {
            // Treat the whole string as a model name with an unknown provider.
            // Fall back to "openai" so librefang can still route the request.
            tracing::warn!(
                model_string = s,
                "UAR capabilities.model is not in `provider/model` format; defaulting provider to 'openai'"
            );
            Ok(("openai".to_owned(), s.to_owned()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser;

    const MINIMAL_DOC: &str = r#"
# Agent: TestBot

## Metadata
version: 1.0.0
description: Test agent
author: tester

## Identity
name: TestBot
role: A helpful test assistant
persona: I help with testing.

## Capabilities
streaming: true
model: anthropic/claude-sonnet-4-20250514
"#;

    #[test]
    fn artifact_to_manifest_roundtrip() {
        let artifact = parser::parse(MINIMAL_DOC).unwrap();
        let manifest = artifact_to_manifest(&artifact).unwrap();
        assert_eq!(manifest.name, "TestBot");
        assert_eq!(manifest.model.provider, "anthropic");
        assert_eq!(manifest.model.model, "claude-sonnet-4-20250514");
        assert_eq!(manifest.model.provider, "anthropic"); // streaming flag not on ModelConfig
    }

    #[test]
    fn manifest_to_artifact_preserves_model() {
        let artifact = parser::parse(MINIMAL_DOC).unwrap();
        let manifest = artifact_to_manifest(&artifact).unwrap();
        let back = manifest_to_artifact(&manifest);
        assert_eq!(
            back.capabilities.model.as_deref(),
            Some("anthropic/claude-sonnet-4-20250514")
        );
    }
}
