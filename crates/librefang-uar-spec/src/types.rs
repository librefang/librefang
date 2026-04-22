//! UAR-AGENT-MD type definitions.
//!
//! These types form the canonical representation of a compiled UAR agent
//! descriptor — the "AgentArtifact". They are intentionally kept as plain
//! `serde`-serialisable data structures so they can be stored, transmitted
//! as JSON, and converted to/from librefang's native `AgentManifest`.
//!
//! The type layout mirrors `universal-agent-runtime`'s
//! `src/uar/compiler/ir.rs` (IR sections) and
//! `src/uar/api/a2a/types.rs` (AgentCard) — keeping them in sync is the
//! responsibility of the translator.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────────────────────
// Top-level artifact
// ─────────────────────────────────────────────────────────────────────────────

/// A compiled UAR agent descriptor.
///
/// Produced by [`crate::parser::parse`] and accepted by
/// [`crate::translator::artifact_to_manifest`].
///
/// The JSON serialization of this struct is the canonical `.agent.json` format
/// understood by the UAR ecosystem.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentArtifact {
    /// Schema identifier (`"uar-agent-descriptor/v1"`).
    pub schema: String,
    /// Unique agent identifier — typically the `identity.name` value.
    pub agent_id: String,
    /// Semantic version from the `## Metadata` section.
    pub version: String,
    /// Metadata section (§04).
    pub metadata: MetadataSection,
    /// Identity section (§05).
    pub identity: IdentitySection,
    /// Capabilities section (§07).
    pub capabilities: CapabilitiesSection,
    /// Skills section (§08).
    #[serde(default)]
    pub skills: SkillsSection,
    /// Tools section (§09).
    #[serde(default)]
    pub tools: ToolsSection,
    /// MCP Servers section (§10).
    #[serde(default)]
    pub mcp_servers: McpServersSection,
    /// Memory Model section (§12).
    #[serde(default)]
    pub memory: MemorySection,
    /// A2A Contracts section (§13).
    #[serde(default)]
    pub a2a: A2ASection,
    /// Budgets & Constraints section (§15).
    #[serde(default)]
    pub budgets: BudgetsSection,
    /// Execution Model section (§16).
    #[serde(default)]
    pub execution: ExecutionSection,
    /// Arbitrary extra fields preserved from the Markdown source.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub extensions: HashMap<String, serde_json::Value>,
}

// ─────────────────────────────────────────────────────────────────────────────
// §04 — Metadata
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataSection {
    /// Semantic version string.
    pub version: String,
    /// Human-readable description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Author name or identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// SPDX license identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Discovery tags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// ISO 8601 creation timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    /// ISO 8601 last-updated timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// §05 — Identity
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentitySection {
    /// Agent display name.
    pub name: String,
    /// One-line role description.
    pub role: String,
    /// Persona paragraph injected into the system prompt.
    pub persona: String,
    /// Full system prompt (optional; overrides `persona` when set).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// Greeting message displayed to the user on session start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub greeting: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// §07 — Capabilities
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CapabilitiesSection {
    #[serde(default)]
    pub streaming: bool,
    #[serde(default)]
    pub file_upload: bool,
    #[serde(default)]
    pub image_generation: bool,
    #[serde(default)]
    pub code_execution: bool,
    #[serde(default)]
    pub web_browsing: bool,
    /// Provider + model in `provider/model` format (e.g. `"openai/gpt-4o"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Additional capability flags.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub extensions: HashMap<String, serde_json::Value>,
}

// ─────────────────────────────────────────────────────────────────────────────
// §08 — Skills
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillsSection {
    #[serde(default)]
    pub skills: Vec<SkillRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRef {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub config: HashMap<String, serde_json::Value>,
}

// ─────────────────────────────────────────────────────────────────────────────
// §09 — Tools
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolsSection {
    #[serde(default)]
    pub tools: Vec<ToolRef>,
    /// Tool allow-list (by name).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow: Vec<String>,
    /// Tool deny-list (by name).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolRef {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub config: HashMap<String, serde_json::Value>,
}

// ─────────────────────────────────────────────────────────────────────────────
// §10 — MCP Servers
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpServersSection {
    #[serde(default)]
    pub servers: Vec<McpServerRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerRef {
    pub id: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// §12 — Memory Model
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemorySection {
    #[serde(default)]
    pub conversation: ConversationMemory,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persistent: Option<PersistentMemory>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMemory {
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
    #[serde(default)]
    pub summarization: bool,
}

impl Default for ConversationMemory {
    fn default() -> Self {
        Self {
            enabled: true,
            max_turns: None,
            summarization: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PersistentMemory {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// §13 — A2A Contracts
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct A2ASection {
    #[serde(default)]
    pub endpoints: Vec<A2AEndpoint>,
    #[serde(default)]
    pub dependencies: Vec<A2ADependency>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2AEndpoint {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2ADependency {
    pub agent_id: String,
    pub endpoint: String,
    #[serde(default)]
    pub required: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// §15 — Budgets & Constraints
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BudgetsSection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens_per_turn: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens_per_session: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tool_calls_per_turn: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cost_per_session_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
}

// ─────────────────────────────────────────────────────────────────────────────
// §16 — Execution Model
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecutionSection {
    /// Execution mode: `"sequential"`, `"parallel"`, `"reactive"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_iterations: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stop_conditions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_behavior: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// A2A AgentCard (served at `/.well-known/agent.json`)
// ─────────────────────────────────────────────────────────────────────────────

/// Machine-readable agent capability declaration per A2A RC v1.0 §3.
///
/// Served at `GET /.well-known/agent.json` and `GET /api/a2a/{id}/agent.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCard {
    pub name: String,
    pub description: String,
    /// Endpoint URL for this agent's A2A task interface.
    pub url: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documentation_url: Option<String>,
    pub capabilities: AgentCardCapabilities,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<AgentCardSkill>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub default_input_modes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub default_output_modes: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentCardCapabilities {
    pub streaming: bool,
    pub push_notifications: bool,
    pub state_transition_history: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCardSkill {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_modes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output_modes: Vec<String>,
}
