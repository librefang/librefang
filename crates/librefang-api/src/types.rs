//! Request/response types for the LibreFang API.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Generic JSON wrappers — used as `body = JsonObject` / `body = JsonArray`
// in `#[utoipa::path]` attributes for handlers that return free-form JSON
// (heterogeneous shapes that are not worth a dedicated struct, or shapes
// that are still in flux). They emit `{ "type": "object" }` / `{ "type":
// "array" }` instead of the empty `{}` blob that `serde_json::Value`
// produces, so downstream SDK generators see a real schema.
// ---------------------------------------------------------------------------

/// OpenAPI placeholder for an arbitrary JSON object response/request.
///
/// Use `body = JsonObject` (qualified as `crate::types::JsonObject` from
/// route modules) when the underlying handler returns
/// `axum::Json<serde_json::Value>` and the shape is dynamic. The schema
/// renders as `{ "type": "object", "additionalProperties": true }`,
/// which is honest about the contract (any JSON object) without leaving
/// SDK generators with `{}` to ignore.
#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[schema(value_type = Object, description = "Arbitrary JSON object")]
#[allow(dead_code)]
pub struct JsonObject(pub serde_json::Value);

/// OpenAPI placeholder for an arbitrary JSON array response/request.
///
/// Counterpart to [`JsonObject`] for handlers that return a top-level
/// JSON array of heterogeneous items.
#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[schema(value_type = Vec<Object>, description = "Arbitrary JSON array")]
#[allow(dead_code)]
pub struct JsonArray(pub Vec<serde_json::Value>);

// ---------------------------------------------------------------------------
// Unified API error response
// ---------------------------------------------------------------------------

/// Nested error body used by the #3639 envelope migration.
///
/// Serializes as `{"code": "...", "message": "...", "request_id": "..."}`.
/// Lives at the top-level `error` key alongside the flat compatibility
/// fields (`code`, `type`, `request_id`) so old and new clients can both
/// parse a single response.
#[derive(Debug, Serialize)]
pub struct ApiErrorBody {
    /// Stable machine-readable code (mirrors the flat top-level `code`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// Human-readable error message (mirrors the legacy flat `error` string).
    pub message: String,
    /// Per-request correlation id (mirrors the flat top-level `request_id`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

/// A unified error response type for all API endpoints.
///
/// Every error returned by the API uses this shape, ensuring clients can rely
/// on a single parsing strategy.  The `code` and `details` fields are optional
/// and only serialized when present.
///
/// `request_id` (#3639) is populated automatically by the
/// [`crate::middleware::request_logging`] post-response hook on every JSON
/// 4xx/5xx response, so handlers do not need to set it manually.
///
/// Wire shape (#3639 deferred — coexists for one minor):
///
/// ```json
/// {
///   "error": { "code": "not_found", "message": "...", "request_id": "..." },
///   "message": "...",
///   "code": "not_found",
///   "type": "not_found",
///   "request_id": "..."
/// }
/// ```
///
/// The nested `error` object is the long-term shape; the flat `code` /
/// `type` / `request_id` fields are kept for one minor for backward
/// compatibility with the dashboard and external callers that still
/// parse the flat form. New consumers should prefer
/// `response.error.code` over `response.code`.
#[derive(Debug)]
pub struct ApiErrorResponse {
    /// Human-readable error message.
    ///
    /// Serialized as a top-level `message` field and mirrored inside the
    /// nested `error.message`.
    pub error: String,
    /// Stable machine-readable error code (e.g. `"not_found"`).
    ///
    /// **Deprecated**: Removed in next minor; use nested `error.code` instead.
    pub code: Option<String>,
    /// Backward-compatible alias for `code` (legacy clients).
    ///
    /// **Deprecated**: Removed in next minor; use nested `error.code` instead.
    pub r#type: Option<String>,
    /// Optional structured details payload.
    pub details: Option<serde_json::Value>,
    /// Per-request correlation id (matches the `x-request-id` header).
    ///
    /// **Deprecated**: Removed in next minor; use nested `error.request_id`
    /// instead.
    pub request_id: Option<String>,
    /// HTTP status code — not serialized into the JSON body.
    pub status: StatusCode,
}

/// Custom serializer emits BOTH the legacy flat fields AND the new nested
/// `error: ApiErrorBody` envelope (#3639 deferred). The flat fields are
/// kept for one minor so the dashboard and external callers that still
/// parse `response.code` / `response.request_id` keep working while new
/// code migrates to `response.error.code` / `response.error.request_id`.
impl Serialize for ApiErrorResponse {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        // Field count: nested `error` + `message` always present; the rest
        // are serialized only when populated.
        let mut field_count = 2;
        if self.code.is_some() {
            field_count += 1;
        }
        if self.r#type.is_some() {
            field_count += 1;
        }
        if self.details.is_some() {
            field_count += 1;
        }
        if self.request_id.is_some() {
            field_count += 1;
        }

        let mut map = serializer.serialize_map(Some(field_count))?;

        // New nested envelope (#3639 — preferred shape).
        let nested = ApiErrorBody {
            code: self.code.clone(),
            message: self.error.clone(),
            request_id: self.request_id.clone(),
        };
        map.serialize_entry("error", &nested)?;

        // Flat compatibility fields (deprecated, removed next minor).
        map.serialize_entry("message", &self.error)?;
        if let Some(code) = &self.code {
            map.serialize_entry("code", code)?;
        }
        if let Some(t) = &self.r#type {
            map.serialize_entry("type", t)?;
        }
        if let Some(details) = &self.details {
            map.serialize_entry("details", details)?;
        }
        if let Some(request_id) = &self.request_id {
            map.serialize_entry("request_id", request_id)?;
        }
        map.end()
    }
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
        request_id: None,
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
            request_id: None,
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
            request_id: None,
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
            request_id: None,
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
            request_id: None,
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
            request_id: None,
            status: StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Attach an error code (e.g. `"not_supported"`, `"rate_limited"`).
    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        let code = code.into();
        self.r#type = Some(code.clone());
        self.code = Some(code);
        self
    }

    /// Attach a typed [`librefang_types::error_code::ErrorCode`] (#3639).
    ///
    /// Preferred over [`Self::with_code`] for new code paths because the
    /// stable wire token is enforced by the type system. Sets both `code`
    /// and the legacy `type` alias.
    pub fn with_error_code(self, code: librefang_types::error_code::ErrorCode) -> Self {
        self.with_code(code.as_str())
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

    /// Stamp a `request_id` correlation token onto the body (#3639).
    ///
    /// Consumed by the request-logging middleware after `next.run()` so
    /// every JSON error response carries the same id that appears in the
    /// `x-request-id` response header and the structured access-log line.
    pub fn with_request_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = Some(request_id.into());
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct AttachmentRef {
    pub file_id: String,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub content_type: String,
}

/// Request to send a message to an agent.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
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
    /// When true, this turn runs in **incognito mode**: session messages and
    /// proactive-memory writes are suppressed while memory reads remain
    /// fully operational. Useful for sensitive queries that should leave no
    /// trace in the conversation history.
    ///
    /// Defaults to `false` (normal persistent turn).
    #[serde(default)]
    pub incognito: bool,
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
#[serde(deny_unknown_fields)]
pub struct InjectMessageRequest {
    /// The message to inject between tool calls.
    pub message: String,
    /// Optional session id; when omitted the message broadcasts to all live sessions for the agent.
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
#[serde(deny_unknown_fields)]
pub struct SkillInstallRequest {
    pub name: String,
    /// Install into a specific hand's workspace instead of globally.
    #[serde(default)]
    pub hand: Option<String>,
}

/// Request to uninstall a skill.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillUninstallRequest {
    pub name: String,
}

/// Request to change an agent's operational mode.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SetModeRequest {
    #[schema(value_type = String)]
    pub mode: librefang_types::agent::AgentMode,
}

/// Request to run a migration.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MigrateRequest {
    pub source: String,
    pub source_dir: String,
    pub target_dir: String,
    #[serde(default)]
    pub dry_run: bool,
}

/// Request to scan a directory for migration.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MigrateScanRequest {
    pub path: String,
}

/// Request to install a skill from ClawHub.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct ExtensionInstallRequest {
    /// Extension/integration ID (e.g., "github", "slack").
    pub name: String,
}

/// Request to uninstall an extension (integration).
#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
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

/// Hard server-side cap on page size (#3639). A client requesting `?limit=N`
/// with N greater than this is silently clamped — protects the wire from
/// pathological "fetch everything" calls on growing catalogs.
pub const PAGINATION_MAX_LIMIT: usize = 100;

/// Generic pagination query parameters: `?offset=&limit=`.
///
/// Used by list endpoints that previously returned the full collection
/// regardless of query params (#3639). When neither `offset` nor `limit`
/// is supplied, the handler returns the full collection — preserves
/// backward compatibility for callers that depended on the unbounded
/// shape. When *either* is supplied, both default to sane values
/// (`offset=0`, `limit=PAGINATION_MAX_LIMIT`) and `limit` is server-capped.
#[derive(Debug, Default, Deserialize)]
pub struct PaginationQuery {
    /// Number of items to skip from the start of the result set.
    pub offset: Option<usize>,
    /// Maximum number of items to return; server-capped at `PAGINATION_MAX_LIMIT`.
    pub limit: Option<usize>,
}

impl PaginationQuery {
    /// Slice a collection according to the query, returning
    /// `(items, total, effective_offset, effective_limit)`.
    ///
    /// `effective_limit` is `None` when the query left both fields blank
    /// (full collection returned); otherwise it's the clamped value the
    /// server actually applied, so clients can tell from the response
    /// envelope whether the cap kicked in.
    pub fn paginate<T>(&self, items: Vec<T>) -> (Vec<T>, usize, usize, Option<usize>) {
        let total = items.len();
        // Both unset → full collection, preserve old behaviour.
        if self.offset.is_none() && self.limit.is_none() {
            return (items, total, 0, None);
        }
        let offset = self.offset.unwrap_or(0).min(total);
        let limit = self
            .limit
            .unwrap_or(PAGINATION_MAX_LIMIT)
            .min(PAGINATION_MAX_LIMIT);
        let end = (offset + limit).min(total);
        let page: Vec<T> = items.into_iter().skip(offset).take(end - offset).collect();
        (page, total, offset, Some(limit))
    }
}

/// Request to push a proactive outbound message from an agent to a channel.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
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
    fn pagination_unspecified_returns_full_collection() {
        let q = PaginationQuery::default();
        let (items, total, offset, limit) = q.paginate((0..10).collect::<Vec<_>>());
        assert_eq!(items.len(), 10);
        assert_eq!(total, 10);
        assert_eq!(offset, 0);
        assert_eq!(limit, None, "no params → no server cap reported");
    }

    #[test]
    fn pagination_explicit_offset_only_uses_default_limit() {
        let q = PaginationQuery {
            offset: Some(5),
            limit: None,
        };
        let (items, total, offset, limit) = q.paginate((0..1000).collect::<Vec<_>>());
        assert_eq!(total, 1000);
        assert_eq!(offset, 5);
        assert_eq!(limit, Some(PAGINATION_MAX_LIMIT));
        assert_eq!(items.len(), PAGINATION_MAX_LIMIT);
        assert_eq!(items.first(), Some(&5));
    }

    #[test]
    fn pagination_clamps_oversized_limit_to_max() {
        let q = PaginationQuery {
            offset: Some(0),
            limit: Some(9999),
        };
        let (items, _, _, limit) = q.paginate((0..200).collect::<Vec<_>>());
        assert_eq!(items.len(), PAGINATION_MAX_LIMIT);
        assert_eq!(limit, Some(PAGINATION_MAX_LIMIT));
    }

    #[test]
    fn pagination_offset_past_total_yields_empty_page() {
        let q = PaginationQuery {
            offset: Some(50),
            limit: Some(10),
        };
        let (items, total, offset, limit) = q.paginate((0..30).collect::<Vec<_>>());
        assert_eq!(items.len(), 0);
        assert_eq!(total, 30);
        assert_eq!(offset, 30, "clamped to total");
        assert_eq!(limit, Some(10));
    }

    #[test]
    fn pagination_returns_partial_last_page() {
        let q = PaginationQuery {
            offset: Some(95),
            limit: Some(10),
        };
        let (items, total, offset, limit) = q.paginate((0..100).collect::<Vec<_>>());
        assert_eq!(items.len(), 5);
        assert_eq!(total, 100);
        assert_eq!(offset, 95);
        assert_eq!(limit, Some(10));
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
