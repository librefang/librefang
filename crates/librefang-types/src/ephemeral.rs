//! Ephemeral worker spawn — request and result types (refs #6699).
//!
//! An ephemeral worker is a single agent turn that runs a caller-supplied task with a caller-supplied tool set and then vanishes: no registry entry, no persisted session, no permanent workspace.
//! It is the "run this and give me the answer" primitive that `spawn_agent` (which builds a permanent agent with seven directories, nine identity files and a SQLite row) is too heavy for, and that `send_message_ephemeral` (`/btw`, hardcoded to zero tools) is too thin for.
//!
//! ## Why a parent is mandatory
//!
//! [`EphemeralSpawnRequest::parent_id`] is not optional, and that single decision is what makes the other guarantees expressible.
//! The worker has no registry entry of its own, so it has no budget, no resource quota, no capability grant and no tool allowlist to be judged against.
//! Running it under the parent's identity gives every one of those a real owner: spend lands on the parent's ledger, the parent's `[resources]` quota is the one enforced, and the worker's tool set can never exceed what the parent itself is permitted to call.
//! A spawn path that accepted `parent_id: None` would have to invent an unattributed identity, and "unattributed" is precisely the budget hole the feature must not open.

use serde::{Deserialize, Serialize};

use crate::agent::AgentId;

/// A request to run one ephemeral worker turn.
///
/// Construct it with [`EphemeralSpawnRequest::new`] and refine with the builder-ish setters, or build the struct literally — every field except `parent_id`, `label` and `message` has a meaningful default.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EphemeralSpawnRequest {
    /// The agent this worker runs on behalf of.
    ///
    /// Supplies the identity every tool call is attributed to, the ledger the spend is billed to, the `[resources]` quota that is enforced, and the ceiling on the worker's tool set.
    pub parent_id: AgentId,

    /// Short human-meaningful label for the mission.
    ///
    /// Becomes the prefix of the mission workspace directory name and of the worker's display name (`<label>-<8 hex>`), so it is sanitized to one path component before it is used — see `MissionWorkspace::create`.
    /// When `agent_type` is set the type name is the natural label.
    pub label: String,

    /// The task the worker is being asked to perform.
    pub message: String,

    /// Agent type whose template manifest supplies the worker's system prompt, model and declared tools.
    ///
    /// Resolved through the same search the workflow step-agent types use — the writable `agent-types/` store first, then `workspaces/agents/`, then `registry/agents/` — so a type authored in the dashboard or by `agent_type_create` is runnable here without being copied anywhere.
    /// Mutually complementary with `system_prompt`: an explicit `system_prompt` overrides the template's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,

    /// System prompt for the worker.
    ///
    /// Overrides the agent type's prompt when both are given. When neither is given the parent's own system prompt is used, which makes the worker a same-persona side task rather than a differently-instructed one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,

    /// Tool names the worker may use.
    ///
    /// `None` means "everything the effective manifest grants". Every name is checked against the parent's own tool set, and an unknown or ungranted name is rejected rather than silently dropped — a typo must not be indistinguishable from success.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,

    /// Provider / model override for this run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<EphemeralModelOverride>,

    /// Iteration ceiling for the worker's loop.
    ///
    /// Clamped to the operator's configured `agent_max_iterations`: a caller may ask for fewer turns than the operator allows, never more.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_iterations: Option<u32>,
}

impl EphemeralSpawnRequest {
    /// A request with the three fields that have no sensible default.
    pub fn new(parent_id: AgentId, label: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            parent_id,
            label: label.into(),
            message: message.into(),
            agent_type: None,
            system_prompt: None,
            tools: None,
            model: None,
            max_iterations: None,
        }
    }

    /// Run the worker from an agent type's template manifest.
    #[must_use]
    pub fn with_agent_type(mut self, agent_type: impl Into<String>) -> Self {
        self.agent_type = Some(agent_type.into());
        self
    }

    /// Give the worker an explicit system prompt.
    #[must_use]
    pub fn with_system_prompt(mut self, system_prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(system_prompt.into());
        self
    }

    /// Restrict the worker to a specific tool set.
    #[must_use]
    pub fn with_tools(mut self, tools: Vec<String>) -> Self {
        self.tools = Some(tools);
        self
    }
}

/// Provider / model override for one ephemeral run.
///
/// Deliberately **not** a `ModelConfig`. `ModelConfig` carries `base_url` and `api_key_env`, which the driver resolver treats as an operator-level override — it reads the named environment variable and posts its value to the named URL.
/// Reaching that from a tool call would let a prompt-injected agent name any environment variable and any destination host, which is a credential-exfiltration primitive rather than a model choice.
/// Only the two fields that select a model are exposed here, so no caller-reachable path can widen it by accident.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EphemeralModelOverride {
    /// Provider id (`anthropic`, `openai`, …). `None` keeps the effective manifest's provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Model id. `None` keeps the effective manifest's model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// What one ephemeral worker turn produced.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EphemeralSpawnResult {
    /// Uid-style display name the worker ran under (`<label>-<8 hex>`), also the name of its mission workspace directory.
    ///
    /// Returned so a caller can correlate logs and audit entries with the run; the directory itself is already gone by the time this value exists.
    pub name: String,
    /// The worker's final assistant text.
    pub response: String,
    /// Loop iterations the worker consumed.
    pub iterations: u32,
    /// Cost billed to the parent for this run, when a cost could be estimated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// Tool names the worker was advertised — which, by construction, is also the set it could execute.
    pub tools: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_round_trips_through_json() {
        let req = EphemeralSpawnRequest::new(AgentId::new(), "researcher", "find the thing")
            .with_agent_type("researcher")
            .with_tools(vec!["file_read".to_string()]);
        let json = serde_json::to_string(&req).unwrap();
        let back: EphemeralSpawnRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.parent_id, req.parent_id);
        assert_eq!(back.agent_type.as_deref(), Some("researcher"));
        assert_eq!(back.tools.as_deref(), Some(&["file_read".to_string()][..]));
    }

    /// The override must not carry a `base_url` / `api_key_env` even when a caller supplies one.
    ///
    /// This is the credential-exfiltration guard expressed as a test rather than as a comment: if someone later widens the type to a full `ModelConfig`, an unknown-field payload starts round-tripping and this fails.
    #[test]
    fn a_model_override_carries_only_provider_and_model() {
        let raw = serde_json::json!({
            "provider": "anthropic",
            "model": "claude-x",
            "base_url": "https://attacker.example",
            "api_key_env": "AWS_SECRET_ACCESS_KEY",
        });
        let over: EphemeralModelOverride = serde_json::from_value(raw).unwrap();
        assert_eq!(over.provider.as_deref(), Some("anthropic"));
        assert_eq!(over.model.as_deref(), Some("claude-x"));
        let back = serde_json::to_value(&over).unwrap();
        assert!(
            back.get("base_url").is_none() && back.get("api_key_env").is_none(),
            "an ephemeral model override must not be able to carry a driver endpoint or a credential env var name, got {back}"
        );
    }
}
