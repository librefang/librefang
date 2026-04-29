//! Request/response types for the LibreFang API.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Unified API error response
// ---------------------------------------------------------------------------

/// A unified error response type for all API endpoints.
///
/// Every error returned by the API uses this shape, ensuring clients can rely
/// on a single parsing strategy.  The `code` and `details` fields are optional
/// and only serialized when present.
#[derive(Debug, Serialize)]
pub struct ApiErrorResponse {
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// Backward-compatible alias for `code`.
    ///
    /// Old clients may parse `"type"` instead of `"code"`.  When both are
    /// set they carry the same value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
    /// HTTP status code — not serialized into the JSON body.
    #[serde(skip)]
    pub status: StatusCode,
}

impl IntoResponse for ApiErrorResponse {
    fn into_response(self) -> Response {
        // Build the JSON body (status is skipped by serde).
        (self.status, Json(self)).into_response()
    }
}

/// Convenience free function — constructs a standard `ApiErrorResponse` and
/// immediately converts it to a `Response`.
///
/// Prefer this over ad-hoc `(StatusCode, Json(json!({"error": ...})))` tuples
/// so clients always receive machine-readable `code` and `type` fields.
///
/// ```
/// use librefang_api::types::api_error;
/// use axum::http::StatusCode;
///
/// let resp = api_error(StatusCode::NOT_FOUND, "agent_not_found", "Agent 42 not found");
/// ```
pub fn api_error(
    status: StatusCode,
    code: &str,
    message: impl Into<String>,
) -> axum::response::Response {
    ApiErrorResponse {
        error: message.into(),
        code: Some(code.to_string()),
        r#type: Some(code.to_string()),
        details: None,
        status,
    }
    .into_response()
}

impl ApiErrorResponse {
    /// 400 Bad Request.
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self {
            error: msg.into(),
            code: None,
            r#type: None,
            details: None,
            status: StatusCode::BAD_REQUEST,
        }
    }

    /// 404 Not Found.
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self {
            error: msg.into(),
            code: None,
            r#type: None,
            details: None,
            status: StatusCode::NOT_FOUND,
        }
    }

    /// 403 Forbidden.
    pub fn forbidden(msg: impl Into<String>) -> Self {
        Self {
            error: msg.into(),
            code: None,
            r#type: None,
            details: None,
            status: StatusCode::FORBIDDEN,
        }
    }

    /// 409 Conflict.
    pub fn conflict(msg: impl Into<String>) -> Self {
        Self {
            error: msg.into(),
            code: None,
            r#type: None,
            details: None,
            status: StatusCode::CONFLICT,
        }
    }

    /// 500 Internal Server Error.
    pub fn internal(msg: impl Into<String>) -> Self {
        Self {
            error: msg.into(),
            code: None,
            r#type: None,
            details: None,
            status: StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Attach an error code (e.g. `"not_supported"`, `"rate_limited"`).
    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    /// Attach arbitrary detail payload.
    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }

    /// Build with a custom status code.
    pub fn with_status(mut self, status: StatusCode) -> Self {
        self.status = status;
        self
    }

    /// Convert into a `(StatusCode, Json<Value>)` tuple that is type-compatible
    /// with the success paths of existing handler functions.
    ///
    /// Prefer `into_response()` in new code; this helper exists for incremental
    /// migration of handlers whose success path still returns a `(StatusCode, Json<Value>)`.
    pub fn into_json_tuple(self) -> (StatusCode, Json<serde_json::Value>) {
        // `StatusCode` is Copy so no move issue.
        let status = self.status;
        // `status` field is `#[serde(skip)]` so `to_value` won't include it.
        let body = serde_json::to_value(&self).unwrap_or_default();
        (status, Json(body))
    }
}

/// Request to spawn an agent from a TOML manifest string or a template name.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SpawnRequest {
    /// Agent manifest as TOML string (optional if `template` is provided).
    #[serde(default)]
    pub manifest_toml: String,
    /// Template name from `~/.librefang/workspaces/agents/{template}/agent.toml`.
    /// When provided and `manifest_toml` is empty, the template is loaded automatically.
    #[serde(default)]
    pub template: Option<String>,
    /// Optional custom name for the agent. Overrides the name from the template
    /// or manifest, allowing multiple agents from the same template.
    #[serde(default)]
    pub name: Option<String>,
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

/// OpenAPI schema stand-in for `librefang_channels::types::ParticipantRef`.
///
/// The real type lives in `librefang-channels`, which does not depend on
/// utoipa. This mirror struct exists only so `#[schema(value_type = ...)]`
/// on `MessageRequest.group_participants` can expose the shape to the
/// generated OpenAPI document.
#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
#[allow(dead_code)]
pub struct ParticipantRefSchema {
    /// Platform JID (e.g. `1234567890@s.whatsapp.net`).
    pub jid: String,
    /// Human-readable name (push-name, contact name, or first part of JID).
    pub display_name: String,
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
    /// Optional sender ID (platform-specific user ID).
    #[serde(default)]
    pub sender_id: Option<String>,
    /// Optional sender display name.
    #[serde(default)]
    pub sender_name: Option<String>,
    /// Optional channel type (e.g. "whatsapp", "telegram").
    #[serde(default)]
    pub channel_type: Option<String>,
    /// Whether this message originated from a group chat (vs DM).
    #[serde(default)]
    pub is_group: bool,
    /// Whether the bot was @mentioned in a group message.
    #[serde(default)]
    pub was_mentioned: bool,
    /// Optional group participant roster (Phase 2 §C addressee guard).
    ///
    /// Forwarded by the WhatsApp gateway for group messages so the kernel's
    /// addressee guard (`is_addressed_to_other_participant`) can detect when
    /// a turn is addressed to a named participant other than the agent.
    ///
    /// `#[serde(default)]` ensures backward compatibility for callers (Telegram,
    /// direct API) that don't populate this field.
    #[serde(default)]
    #[schema(value_type = Option<Vec<ParticipantRefSchema>>)]
    pub group_participants: Option<Vec<librefang_channels::types::ParticipantRef>>,
    /// If true, this is an ephemeral "side question" (`/btw`).
    /// The message is answered using the agent's system prompt but WITHOUT
    /// loading or saving session history — the real conversation is untouched.
    #[serde(default)]
    pub ephemeral: bool,
    /// Per-call deep-thinking override.
    ///
    /// - `Some(true)`: force thinking on (even if the manifest has it off)
    /// - `Some(false)`: force thinking off (even if the manifest has it on)
    /// - `None`: use the manifest/global default
    #[serde(default)]
    pub thinking: Option<bool>,
    /// Whether the response should include the model's thinking/reasoning trace.
    ///
    /// `None` defaults to `true` when thinking content is available.
    #[serde(default)]
    pub show_thinking: Option<bool>,
    /// Optional explicit session ID (UUID string) to use for this message.
    ///
    /// When set, overrides the default session resolution (channel-derived or
    /// registry canonical). Enables multi-tab / multi-session UIs where the
    /// caller tracks which session each conversation belongs to.
    ///
    /// Safety: the server rejects a `session_id` that belongs to a different
    /// agent with 400 Bad Request.
    #[serde(default)]
    pub session_id: Option<String>,
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
    /// Detected memory conflicts where new info contradicts existing memories.
    /// Empty when no conflicts were detected.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schema(value_type = Vec<serde_json::Value>)]
    pub memory_conflicts: Vec<librefang_types::memory::MemoryConflict>,
    /// Combined thinking/reasoning trace from the model, when the caller
    /// requested `show_thinking = true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    /// §A — Optional private notice destined for the agent's owner DM,
    /// produced when the model invoked the `notify_owner` tool during the
    /// turn. Channel adapters (e.g. whatsapp-gateway) MUST route this to
    /// the owner's address (e.g. OWNER_JID) and NOT to the source chat.
    /// Adapters that don't support owner-side delivery should ignore it
    /// (BC-01 — Telegram/Discord/Slack continue to function unchanged).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_notice: Option<String>,
}

/// Request to inject a message into a running agent's tool-execution loop (#956).
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct InjectMessageRequest {
    /// The message to inject between tool calls.
    pub message: String,
    /// Optional session id to target a specific running loop. When omitted
    /// (#3734), the message is broadcast to every live session for the
    /// agent — preserves backward compatibility with single-session callers
    /// while allowing multi-session UIs to address one session at a time.
    #[serde(default)]
    pub session_id: Option<String>,
}

/// Response from a mid-turn message injection.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct InjectMessageResponse {
    /// Whether the message was accepted (true = injected, false = no active loop).
    pub injected: bool,
}

/// Request to install a skill from the marketplace.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SkillInstallRequest {
    pub name: String,
    /// Install into a specific hand's workspace instead of globally.
    #[serde(default)]
    pub hand: Option<String>,
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
    /// Install into a specific hand's workspace instead of globally.
    #[serde(default)]
    pub hand: Option<String>,
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

// ---------------------------------------------------------------------------
// Agent list query / pagination
// ---------------------------------------------------------------------------

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
    /// Include hand agents in the response (default: false).
    pub include_hands: Option<bool>,
    /// Filter agents by owner (matches manifest.author). When the authenticated
    /// caller is a plain User role, this is auto-populated with their username
    /// so that non-admin users only see agents they authored.
    pub owner: Option<String>,
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

/// Request to push a proactive outbound message from an agent to a channel.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct PushMessageRequest {
    /// Channel adapter name (e.g., "telegram", "slack", "discord").
    pub channel: String,
    /// Recipient identifier (platform-specific: chat_id, username, email, etc.).
    pub recipient: String,
    /// The message text to send.
    pub message: String,
    /// Optional thread/topic ID for threaded replies (platform-specific).
    #[serde(default)]
    pub thread_id: Option<String>,
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

    #[test]
    fn message_request_sender_fields_default_to_none() {
        let json = r#"{"message":"hello"}"#;
        let req: MessageRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.message, "hello");
        assert!(req.sender_id.is_none());
        assert!(req.sender_name.is_none());
        assert!(req.channel_type.is_none());
    }

    #[test]
    fn message_request_sender_fields_deserialize() {
        let json = r#"{
            "message":"hello",
            "sender_id":"user-123",
            "sender_name":"Alice",
            "channel_type":"whatsapp"
        }"#;
        let req: MessageRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.message, "hello");
        assert_eq!(req.sender_id.as_deref(), Some("user-123"));
        assert_eq!(req.sender_name.as_deref(), Some("Alice"));
        assert_eq!(req.channel_type.as_deref(), Some("whatsapp"));
    }

    #[test]
    fn message_request_ephemeral_defaults_to_false() {
        let json = r#"{"message":"hello"}"#;
        let req: MessageRequest = serde_json::from_str(json).unwrap();
        assert!(!req.ephemeral);
    }

    #[test]
    fn message_request_ephemeral_true() {
        let json = r#"{"message":"what is rust?","ephemeral":true}"#;
        let req: MessageRequest = serde_json::from_str(json).unwrap();
        assert!(req.ephemeral);
        assert_eq!(req.message, "what is rust?");
    }

    #[test]
    fn message_request_btw_prefix_detection() {
        // The /btw prefix is handled at the route layer, not deserialization,
        // but verify the message text round-trips correctly.
        let json = r#"{"message":"/btw what is rust?"}"#;
        let req: MessageRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.message, "/btw what is rust?");
        assert!(!req.ephemeral); // ephemeral is detected at route level, not here
                                 // Route-level stripping:
        let stripped = req.message.strip_prefix("/btw ").unwrap();
        assert_eq!(stripped, "what is rust?");
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

    #[test]
    fn agent_list_query_defaults() {
        let q: AgentListQuery = serde_json::from_str("{}").unwrap();
        assert!(q.q.is_none());
        assert!(q.status.is_none());
        assert!(q.limit.is_none());
        assert!(q.offset.is_none());
        assert!(q.sort.is_none());
        assert!(q.order.is_none());
    }

    #[test]
    fn agent_list_query_full() {
        let json =
            r#"{"q":"test","status":"running","limit":10,"offset":5,"sort":"name","order":"desc"}"#;
        let q: AgentListQuery = serde_json::from_str(json).unwrap();
        assert_eq!(q.q.as_deref(), Some("test"));
        assert_eq!(q.status.as_deref(), Some("running"));
        assert_eq!(q.limit, Some(10));
        assert_eq!(q.offset, Some(5));
        assert_eq!(q.sort.as_deref(), Some("name"));
        assert_eq!(q.order.as_deref(), Some("desc"));
    }

    #[test]
    fn paginated_response_serialize() {
        let resp = PaginatedResponse {
            items: vec!["a", "b"],
            total: 10,
            offset: 2,
            limit: Some(5),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["items"], serde_json::json!(["a", "b"]));
        assert_eq!(json["total"], 10);
        assert_eq!(json["offset"], 2);
        assert_eq!(json["limit"], 5);
    }

    #[test]
    fn paginated_response_serialize_no_limit() {
        let resp = PaginatedResponse {
            items: vec![1, 2, 3],
            total: 3,
            offset: 0,
            limit: None,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["items"], serde_json::json!([1, 2, 3]));
        assert_eq!(json["total"], 3);
        assert_eq!(json["offset"], 0);
        assert!(json["limit"].is_null());
    }
}
