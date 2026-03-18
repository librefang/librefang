//! Request/response types for the LibreFang API.

use serde::{Deserialize, Serialize};

/// Request to spawn an agent from a TOML manifest string or a template name.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SpawnRequest {
    /// Agent manifest as TOML string (optional if `template` is provided).
    #[serde(default)]
    pub manifest_toml: String,
    /// Template name from `~/.librefang/agents/{template}/agent.toml`.
    /// When provided and `manifest_toml` is empty, the template is loaded automatically.
    #[serde(default)]
    pub template: Option<String>,
    /// Optional Ed25519 signed manifest envelope (JSON).
    /// When present, the signature is verified before spawning.
    #[serde(default)]
    pub signed_manifest: Option<String>,
}

/// Response after spawning an agent.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SpawnResponse {
    pub agent_id: String,
    pub name: String,
}

/// A file attachment reference (from a prior upload).
#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct AttachmentRef {
    pub file_id: String,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub content_type: String,
}

/// Request to send a message to an agent.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct MessageRequest {
    pub message: String,
    /// Optional file attachments (uploaded via /upload endpoint).
    #[serde(default)]
    pub attachments: Vec<AttachmentRef>,
}

/// Response from sending a message.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct MessageResponse {
    pub response: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub iterations: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// Decision traces from tool calls made during the agent loop.
    /// Empty if no tools were called.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[schema(value_type = Vec<serde_json::Value>)]
    pub decision_traces: Vec<librefang_types::tool::DecisionTrace>,
    /// Summaries of memories that were saved during this turn.
    /// Empty when no new memories were extracted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memories_saved: Vec<String>,
    /// Summaries of memories that were recalled and used as context.
    /// Empty when no relevant memories were found.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memories_used: Vec<String>,
}

/// Request to install a skill from the marketplace.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SkillInstallRequest {
    pub name: String,
}

/// Request to uninstall a skill.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SkillUninstallRequest {
    pub name: String,
}

/// Request to update an agent's manifest.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct AgentUpdateRequest {
    pub manifest_toml: String,
}

/// Request to change an agent's operational mode.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SetModeRequest {
    #[schema(value_type = String)]
    pub mode: librefang_types::agent::AgentMode,
}

/// Request to run a migration.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct MigrateRequest {
    pub source: String,
    pub source_dir: String,
    pub target_dir: String,
    #[serde(default)]
    pub dry_run: bool,
}

/// Request to scan a directory for migration.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct MigrateScanRequest {
    pub path: String,
}

/// Request to install a skill from ClawHub.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ClawHubInstallRequest {
    /// ClawHub skill slug (e.g., "github-helper").
    pub slug: String,
}

// ---------------------------------------------------------------------------
// Bulk operations
// ---------------------------------------------------------------------------

/// Request to create multiple agents at once.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct BulkCreateRequest {
    pub agents: Vec<SpawnRequest>,
}

/// Outcome of a single bulk-create item.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct BulkCreateResult {
    pub index: usize,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Request containing a list of agent IDs for bulk operations (delete/start/stop).
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct BulkAgentIdsRequest {
    pub agent_ids: Vec<String>,
}

/// Outcome of a single bulk action (delete/start/stop).
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct BulkActionResult {
    pub agent_id: String,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Request to install an extension (integration).
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ExtensionInstallRequest {
    /// Extension/integration ID (e.g., "github", "slack").
    pub name: String,
}

/// Request to uninstall an extension (integration).
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ExtensionUninstallRequest {
    /// Extension/integration ID to remove.
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_install_request_deserialize() {
        let json = r#"{"name": "github"}"#;
        let req: ExtensionInstallRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name, "github");
    }

    #[test]
    fn extension_uninstall_request_deserialize() {
        let json = r#"{"name": "slack"}"#;
        let req: ExtensionUninstallRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name, "slack");
    }

    #[test]
    fn extension_install_request_missing_name_fails() {
        let json = r#"{}"#;
        let result = serde_json::from_str::<ExtensionInstallRequest>(json);
        assert!(result.is_err());
    }

    #[test]
    fn extension_uninstall_request_missing_name_fails() {
        let json = r#"{}"#;
        let result = serde_json::from_str::<ExtensionUninstallRequest>(json);
        assert!(result.is_err());
    }

    // Bulk operation type tests

    #[test]
    fn bulk_create_request_deserialize() {
        let json = r#"{"agents": [{"manifest_toml": "name = \"test\""}]}"#;
        let req: BulkCreateRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.agents.len(), 1);
        assert_eq!(req.agents[0].manifest_toml, "name = \"test\"");
    }

    #[test]
    fn bulk_create_request_empty_agents() {
        let json = r#"{"agents": []}"#;
        let req: BulkCreateRequest = serde_json::from_str(json).unwrap();
        assert!(req.agents.is_empty());
    }

    #[test]
    fn bulk_create_request_missing_agents_fails() {
        let json = r#"{}"#;
        let result = serde_json::from_str::<BulkCreateRequest>(json);
        assert!(result.is_err());
    }

    #[test]
    fn bulk_agent_ids_request_deserialize() {
        let json = r#"{"agent_ids": ["id1", "id2"]}"#;
        let req: BulkAgentIdsRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.agent_ids.len(), 2);
    }

    #[test]
    fn bulk_agent_ids_request_missing_ids_fails() {
        let json = r#"{}"#;
        let result = serde_json::from_str::<BulkAgentIdsRequest>(json);
        assert!(result.is_err());
    }

    #[test]
    fn bulk_create_result_serialize_success() {
        let result = BulkCreateResult {
            index: 0,
            success: true,
            agent_id: Some("abc-123".into()),
            name: Some("test-agent".into()),
            error: None,
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["success"], true);
        assert_eq!(json["agent_id"], "abc-123");
        // error field should be omitted (skip_serializing_if)
        assert!(json.get("error").is_none());
    }

    #[test]
    fn bulk_create_result_serialize_failure() {
        let result = BulkCreateResult {
            index: 1,
            success: false,
            agent_id: None,
            name: None,
            error: Some("Invalid manifest".into()),
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["success"], false);
        assert_eq!(json["error"], "Invalid manifest");
        // agent_id and name should be omitted
        assert!(json.get("agent_id").is_none());
        assert!(json.get("name").is_none());
    }

    #[test]
    fn bulk_action_result_serialize() {
        let result = BulkActionResult {
            agent_id: "xyz".into(),
            success: true,
            message: Some("Deleted".into()),
            error: None,
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["agent_id"], "xyz");
        assert_eq!(json["message"], "Deleted");
        assert!(json.get("error").is_none());
    }
}
