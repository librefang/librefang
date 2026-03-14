//! Request/response types for the LibreFang API.

use serde::{Deserialize, Serialize};

/// Request to spawn an agent from a TOML manifest string or a template name.
#[derive(Debug, Deserialize)]
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
#[derive(Debug, Serialize)]
pub struct SpawnResponse {
    pub agent_id: String,
    pub name: String,
}

/// A file attachment reference (from a prior upload).
#[derive(Debug, Clone, Deserialize)]
pub struct AttachmentRef {
    pub file_id: String,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub content_type: String,
}

/// Request to send a message to an agent.
#[derive(Debug, Deserialize)]
pub struct MessageRequest {
    pub message: String,
    /// Optional file attachments (uploaded via /upload endpoint).
    #[serde(default)]
    pub attachments: Vec<AttachmentRef>,
}

/// Response from sending a message.
#[derive(Debug, Serialize)]
pub struct MessageResponse {
    pub response: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub iterations: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

/// Request to install a skill from the marketplace.
#[derive(Debug, Deserialize)]
pub struct SkillInstallRequest {
    pub name: String,
}

/// Request to uninstall a skill.
#[derive(Debug, Deserialize)]
pub struct SkillUninstallRequest {
    pub name: String,
}

/// Request to update an agent's manifest.
#[derive(Debug, Deserialize)]
pub struct AgentUpdateRequest {
    pub manifest_toml: String,
}

/// Request to change an agent's operational mode.
#[derive(Debug, Deserialize)]
pub struct SetModeRequest {
    pub mode: librefang_types::agent::AgentMode,
}

/// Request to run a migration.
#[derive(Debug, Deserialize)]
pub struct MigrateRequest {
    pub source: String,
    pub source_dir: String,
    pub target_dir: String,
    #[serde(default)]
    pub dry_run: bool,
}

/// Request to scan a directory for migration.
#[derive(Debug, Deserialize)]
pub struct MigrateScanRequest {
    pub path: String,
}

/// Request to install a skill from ClawHub.
#[derive(Debug, Deserialize)]
pub struct ClawHubInstallRequest {
    /// ClawHub skill slug (e.g., "github-helper").
    pub slug: String,
}

/// Query parameters for `GET /api/agents` with filtering, pagination, and sorting.
///
/// All fields are optional. When omitted, the endpoint returns all agents
/// (backwards-compatible with the original behavior).
#[derive(Debug, Default, Deserialize)]
pub struct AgentListQuery {
    /// Free-text search — matches against agent name and description (case-insensitive).
    pub q: Option<String>,
    /// Filter by agent lifecycle state (e.g., "running", "suspended", "terminated").
    pub status: Option<String>,
    /// Maximum number of agents to return (pagination).
    pub limit: Option<usize>,
    /// Number of agents to skip (pagination).
    pub offset: Option<usize>,
    /// Field to sort by: "name", "created_at", "last_active", "state" (default: "name").
    pub sort: Option<String>,
    /// Sort direction: "asc" or "desc" (default: "asc").
    pub order: Option<String>,
}

/// Paginated list response wrapper.
///
/// Wraps a collection with pagination metadata so clients can implement
/// paging UIs without separate count requests.
#[derive(Debug, Serialize)]
pub struct PaginatedResponse<T: Serialize> {
    /// The items in the current page.
    pub items: Vec<T>,
    /// Total number of items matching the filter (before pagination).
    pub total: usize,
    /// Number of items skipped.
    pub offset: usize,
    /// Maximum number of items requested.
    pub limit: Option<usize>,
}
