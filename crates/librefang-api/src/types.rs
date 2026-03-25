//! Request/response types for the LibreFang API.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Standardized API error response envelope
// ---------------------------------------------------------------------------

/// Standardized error response returned by all API endpoints.
///
/// The JSON envelope always contains `error` (human-readable message).
/// Optional fields `code` and `type` carry the same machine-readable tag
/// (kept in sync for backward compatibility — old clients may parse either).
/// `details` carries additional structured context when available.
///
/// # Examples
///
/// Minimal:
/// ```json
/// {"error": "Agent not found"}
/// ```
///
/// With code/type:
/// ```json
/// {"error": "Missing API key", "code": "missing_key", "type": "missing_key"}
/// ```
///
/// With details:
/// ```json
/// {"error": "Validation failed", "code": "validation_error", "type": "validation_error",
///  "details": {"field": "name", "max_length": 256}}
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ApiErrorResponse {
    /// Human-readable error message.
    pub error: String,

    /// Machine-readable error code (e.g. `"not_found"`, `"validation_error"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,

    /// Backward-compatible alias for `code`. Always kept in sync with `code`.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub error_type: Option<String>,

    /// Optional structured details (e.g. field-level validation info).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,

    /// HTTP status code (not serialized — used only for the response status line).
    #[serde(skip)]
    #[schema(ignore)]
    pub status: StatusCode,
}

impl ApiErrorResponse {
    /// Create an error response with only a message and HTTP status.
    pub fn new(status: StatusCode, error: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            code: None,
            error_type: None,
            details: None,
            status,
        }
    }

    /// Create an error response with a machine-readable code.
    ///
    /// The `code` value is duplicated into `type` for backward compatibility.
    pub fn with_code(
        status: StatusCode,
        error: impl Into<String>,
        code: impl Into<String>,
    ) -> Self {
        let code_str = code.into();
        Self {
            error: error.into(),
            code: Some(code_str.clone()),
            error_type: Some(code_str),
            details: None,
            status,
        }
    }

    /// Attach structured details to the error response.
    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }

    // -- Convenience constructors for common HTTP status codes --

    /// 400 Bad Request.
    pub fn bad_request(error: impl Into<String>) -> Self {
        Self::with_code(StatusCode::BAD_REQUEST, error, "bad_request")
    }

    /// 401 Unauthorized.
    pub fn unauthorized(error: impl Into<String>) -> Self {
        Self::with_code(StatusCode::UNAUTHORIZED, error, "unauthorized")
    }

    /// 404 Not Found.
    pub fn not_found(error: impl Into<String>) -> Self {
        Self::with_code(StatusCode::NOT_FOUND, error, "not_found")
    }

    /// 409 Conflict.
    pub fn conflict(error: impl Into<String>) -> Self {
        Self::with_code(StatusCode::CONFLICT, error, "conflict")
    }

    /// 422 Unprocessable Entity.
    pub fn unprocessable(error: impl Into<String>) -> Self {
        Self::with_code(
            StatusCode::UNPROCESSABLE_ENTITY,
            error,
            "unprocessable_entity",
        )
    }

    /// 413 Payload Too Large.
    pub fn payload_too_large(error: impl Into<String>) -> Self {
        Self::with_code(StatusCode::PAYLOAD_TOO_LARGE, error, "payload_too_large")
    }

    /// 429 Too Many Requests.
    pub fn too_many_requests(error: impl Into<String>) -> Self {
        Self::with_code(StatusCode::TOO_MANY_REQUESTS, error, "rate_limited")
    }

    /// 500 Internal Server Error.
    pub fn internal(error: impl Into<String>) -> Self {
        Self::with_code(StatusCode::INTERNAL_SERVER_ERROR, error, "internal_error")
    }

    /// 502 Bad Gateway.
    pub fn bad_gateway(error: impl Into<String>) -> Self {
        Self::with_code(StatusCode::BAD_GATEWAY, error, "bad_gateway")
    }

    /// 503 Service Unavailable.
    pub fn service_unavailable(error: impl Into<String>) -> Self {
        Self::with_code(
            StatusCode::SERVICE_UNAVAILABLE,
            error,
            "service_unavailable",
        )
    }

    /// Convert to a `(StatusCode, Json<Value>)` tuple for use in handlers
    /// that mix error and success responses with the same return type.
    pub fn to_json_tuple(self) -> (StatusCode, Json<serde_json::Value>) {
        let status = self.status;
        (
            status,
            Json(serde_json::to_value(&self).unwrap_or_default()),
        )
    }
}

impl IntoResponse for ApiErrorResponse {
    fn into_response(self) -> Response {
        (self.status, Json(self)).into_response()
    }
}

impl std::fmt::Display for ApiErrorResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.error)
    }
}

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
    /// Optional sender ID (platform-specific user ID).
    #[serde(default)]
    pub sender_id: Option<String>,
    /// Optional sender display name.
    #[serde(default)]
    pub sender_name: Option<String>,
    /// Optional channel type (e.g. "whatsapp", "telegram").
    #[serde(default)]
    pub channel_type: Option<String>,
    /// If true, this is an ephemeral "side question" (`/btw`).
    /// The message is answered using the agent's system prompt but WITHOUT
    /// loading or saving session history — the real conversation is untouched.
    #[serde(default)]
    pub ephemeral: bool,
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
}

/// Request to inject a message into a running agent's tool-execution loop (#956).
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct InjectMessageRequest {
    /// The message to inject between tool calls.
    pub message: String,
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

    // ── ApiErrorResponse tests ────────────────────────────────────

    #[test]
    fn api_error_minimal_serialization() {
        let err = ApiErrorResponse::new(StatusCode::BAD_REQUEST, "something went wrong");
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["error"], "something went wrong");
        // code and type should be absent when not set
        assert!(json.get("code").is_none());
        assert!(json.get("type").is_none());
        assert!(json.get("details").is_none());
    }

    #[test]
    fn api_error_with_code_has_both_code_and_type() {
        let err =
            ApiErrorResponse::with_code(StatusCode::NOT_FOUND, "Agent not found", "not_found");
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["error"], "Agent not found");
        assert_eq!(json["code"], "not_found");
        assert_eq!(json["type"], "not_found");
        // status field should not appear in JSON
        assert!(json.get("status").is_none());
    }

    #[test]
    fn api_error_with_details() {
        let err = ApiErrorResponse::bad_request("Validation failed")
            .with_details(serde_json::json!({"field": "name", "max_length": 256}));
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["error"], "Validation failed");
        assert_eq!(json["code"], "bad_request");
        assert_eq!(json["type"], "bad_request");
        assert_eq!(json["details"]["field"], "name");
        assert_eq!(json["details"]["max_length"], 256);
    }

    #[test]
    fn api_error_convenience_constructors() {
        assert_eq!(
            ApiErrorResponse::bad_request("x").status,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            ApiErrorResponse::unauthorized("x").status,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            ApiErrorResponse::not_found("x").status,
            StatusCode::NOT_FOUND
        );
        assert_eq!(ApiErrorResponse::conflict("x").status, StatusCode::CONFLICT);
        assert_eq!(
            ApiErrorResponse::unprocessable("x").status,
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            ApiErrorResponse::payload_too_large("x").status,
            StatusCode::PAYLOAD_TOO_LARGE
        );
        assert_eq!(
            ApiErrorResponse::too_many_requests("x").status,
            StatusCode::TOO_MANY_REQUESTS
        );
        assert_eq!(
            ApiErrorResponse::internal("x").status,
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            ApiErrorResponse::bad_gateway("x").status,
            StatusCode::BAD_GATEWAY
        );
        assert_eq!(
            ApiErrorResponse::service_unavailable("x").status,
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[test]
    fn api_error_code_type_backward_compat() {
        // Old clients parsing "type" still get the same value as "code"
        let err = ApiErrorResponse::not_found("gone");
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["code"], json["type"]);
    }

    #[test]
    fn api_error_deserialization() {
        // Clients should be able to deserialize the envelope
        let json_str = r#"{"error":"bad","code":"bad_request","type":"bad_request"}"#;
        let err: ApiErrorResponse = serde_json::from_str(json_str).unwrap();
        assert_eq!(err.error, "bad");
        assert_eq!(err.code.as_deref(), Some("bad_request"));
        assert_eq!(err.error_type.as_deref(), Some("bad_request"));
    }

    #[test]
    fn api_error_deserialization_minimal() {
        // Minimal envelope with only "error" field
        let json_str = r#"{"error":"oops"}"#;
        let err: ApiErrorResponse = serde_json::from_str(json_str).unwrap();
        assert_eq!(err.error, "oops");
        assert!(err.code.is_none());
        assert!(err.error_type.is_none());
        assert!(err.details.is_none());
    }

    #[test]
    fn api_error_display() {
        let err = ApiErrorResponse::bad_request("test message");
        assert_eq!(format!("{err}"), "test message");
    }
}
