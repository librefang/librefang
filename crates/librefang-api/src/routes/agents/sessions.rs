use super::*;

const TOOL_RESULT_MAX_BYTES: usize = 100 * 1024;

fn cap_tool_result(result: &str) -> String {
    if result.len() <= TOOL_RESULT_MAX_BYTES {
        return result.to_string();
    }
    let mut end = TOOL_RESULT_MAX_BYTES;
    while !result.is_char_boundary(end) {
        end -= 1;
    }
    result[..end].to_string()
}

async fn remove_history_image_temp(path: &std::path::Path) {
    if let Err(error) = tokio::fs::remove_file(path).await {
        if error.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(%error, "failed to clean up session history image");
        }
    }
}

async fn materialize_history_image(
    upload_dir: &std::path::Path,
    session_scope: &[u8],
    media_type: &str,
    data: &str,
) -> Option<serde_json::Value> {
    use base64::Engine as _;
    use sha2::{Digest, Sha256};

    let bytes = match base64::engine::general_purpose::STANDARD.decode(data) {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!(%error, "failed to decode session history image");
            return None;
        }
    };
    let mut hasher = Sha256::new();
    hasher.update(session_scope);
    hasher.update([0]);
    hasher.update(media_type.as_bytes());
    hasher.update([0]);
    hasher.update(&bytes);
    let digest = hasher.finalize();
    let mut file_id_bytes = [0_u8; 16];
    file_id_bytes.copy_from_slice(&digest[..16]);
    // RFC 9562 UUIDv8 keeps the content-derived 128-bit identifier compatible with the upload route while reserving the standard version/variant bits.
    file_id_bytes[6] = (file_id_bytes[6] & 0x0f) | 0x80;
    file_id_bytes[8] = (file_id_bytes[8] & 0x3f) | 0x80;
    let file_id = uuid::Uuid::from_bytes(file_id_bytes).to_string();
    let on_disk = librefang_types::media::on_disk_name(&file_id, media_type, "");

    if let Err(error) = tokio::fs::create_dir_all(upload_dir).await {
        tracing::warn!(%error, "failed to create session image directory");
        return None;
    }
    let path = upload_dir.join(on_disk);
    let exists = match tokio::fs::try_exists(&path).await {
        Ok(exists) => exists,
        Err(error) => {
            tracing::warn!(%error, "failed to inspect session history image");
            return None;
        }
    };
    if !exists {
        let temporary = upload_dir.join(format!(".{file_id}.{}.tmp", uuid::Uuid::new_v4()));
        if let Err(error) = tokio::fs::write(&temporary, &bytes).await {
            tracing::warn!(%error, "failed to write temporary session history image");
            remove_history_image_temp(&temporary).await;
            return None;
        }
        if let Err(error) = tokio::fs::rename(&temporary, &path).await {
            if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
                tracing::warn!(%error, "failed to publish session history image");
                remove_history_image_temp(&temporary).await;
                return None;
            }
            remove_history_image_temp(&temporary).await;
        }
    }

    let filename = format!("image.{}", media_type.rsplit('/').next().unwrap_or("png"));
    UPLOAD_REGISTRY
        .entry(file_id.clone())
        .or_insert_with(|| UploadMeta {
            filename: filename.clone(),
            content_type: media_type.to_string(),
            uploaded_by: None,
        });
    Some(serde_json::json!({
        "file_id": file_id,
        "filename": filename,
    }))
}

/// Query params for `GET /api/agents/{id}/session`.
///
/// Using a typed struct (rather than `HashMap<String,String>`) gives us
/// automatic UUID validation: a malformed `session_id` is rejected by serde
/// before the handler runs, returning a clean 400.
#[derive(serde::Deserialize)]
pub struct GetAgentSessionQuery {
    pub session_id: Option<uuid::Uuid>,
}

/// GET /api/agents/:id/session — Get agent session (conversation history).
#[utoipa::path(
    get,
    path = "/api/agents/{id}/session",
    tag = "agents",
    params(
        ("id" = String, Path, description = "Agent ID"),
        ("session_id" = Option<String>, Query, description = "Optional session id to load instead of the canonical active session"),
    ),
    responses(
        (status = 200, description = "Get agent conversation session history", body = crate::types::JsonObject)
    )
)]
pub async fn get_agent_session(
    State(state): State<Arc<AppState>>,
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
    Path(id): Path<String>,
    query: Result<Query<GetAgentSessionQuery>, axum::extract::rejection::QueryRejection>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let (err_session_invalid, err_agent_invalid, err_agent_not_found, err_session_load_failed) = {
        let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
        (
            t.t("api-error-session-invalid-id"),
            t.t("api-error-agent-invalid-id"),
            t.t("api-error-agent-not-found"),
            t.t("api-error-session-load-failed"),
        )
    };
    let Query(params) = match query {
        Ok(q) => q,
        Err(_) => {
            return ApiErrorResponse::bad_request(err_session_invalid)
                .with_code("invalid_session_id")
                .into_response();
        }
    };
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return ApiErrorResponse::bad_request(err_agent_invalid)
                .with_code("invalid_agent_id")
                .into_response();
        }
    };

    let entry = match state.kernel.agent_registry().get(agent_id) {
        Some(e) => e,
        None => {
            return ApiErrorResponse::not_found(err_agent_not_found)
                .with_code("agent_not_found")
                .into_response();
        }
    };
    if !super::super::can_access_agent(&state, agent_id, api_user.as_ref()) {
        // `err_agent_not_found` rather than a fresh `ErrorTranslator`: the
        // translator is `!Send` and this handler awaits below, so #6921 moved
        // it into a block that pre-resolves every message and drops it before
        // the first await. The move in the registry-miss arm above sits on a
        // diverging branch, so the binding is still live on this path.
        return ApiErrorResponse::not_found(err_agent_not_found)
            .with_code("agent_not_found")
            .into_response();
    }

    // Callers (e.g. the dashboard tab with `?sessionId=` pinned) can override
    // the canonical-active session for this request. The returned messages
    // must belong to that exact session; otherwise tabs pinned to different
    // sessions all render whichever session the kernel thinks is active.
    let target_session_id = match params.session_id {
        Some(uuid) => librefang_types::agent::SessionId(uuid),
        None => entry.session_id,
    };

    match state
        .kernel
        .memory_substrate()
        .get_session(target_session_id)
    {
        Ok(Some(session)) => {
            // Reject cross-agent reads when the caller passed an explicit
            // session_id — prevents leaking one agent's history via another's
            // id.
            if session.agent_id != agent_id {
                return ApiErrorResponse::not_found("session not found for this agent")
                    .with_code("session_agent_mismatch")
                    .into_response();
            }
            // Two-pass approach: ToolUse blocks live in Assistant messages while
            // ToolResult blocks arrive in subsequent User messages.  Pass 1
            // collects all tool_use entries keyed by id; pass 2 attaches results.

            // Pass 1: build messages and a lookup from tool_use_id → (msg_idx, tool_idx)
            let mut built_messages: Vec<serde_json::Value> = Vec::new();
            let mut tool_use_index: std::collections::HashMap<String, (usize, usize)> =
                std::collections::HashMap::new();

            for m in &session.messages {
                let mut tools: Vec<serde_json::Value> = Vec::new();
                let mut msg_images: Vec<serde_json::Value> = Vec::new();
                // Extended-thinking traces are flattened the same way text /
                // tool_use / images already are. The dashboard renders these
                // in a collapsible drawer; without surfacing them here, the
                // reload path silently loses reasoning that was visible during
                // streaming. Multiple thinking blocks in a single turn are
                // joined with a blank line so the drawer reads naturally —
                // matches the live `thinking_delta` accumulation on the WS
                // path. `redacted_thinking` is not modeled separately yet and
                // would fall through the catch-all, same as today.
                let mut thinkings: Vec<String> = Vec::new();
                let content = match &m.content {
                    librefang_types::message::MessageContent::Text(t) => t.clone(),
                    librefang_types::message::MessageContent::Blocks(blocks) => {
                        let mut texts = Vec::new();
                        for b in blocks {
                            match b {
                                librefang_types::message::ContentBlock::Text { text, .. } => {
                                    texts.push(text.clone());
                                }
                                librefang_types::message::ContentBlock::Thinking {
                                    thinking,
                                    ..
                                } => {
                                    thinkings.push(thinking.clone());
                                }
                                librefang_types::message::ContentBlock::Image {
                                    media_type,
                                    data,
                                } => {
                                    texts.push("[Image]".to_string());
                                    let upload_dir = state
                                        .kernel
                                        .config_ref()
                                        .channels
                                        .effective_file_download_dir();
                                    if let Some(image) = materialize_history_image(
                                        &upload_dir,
                                        target_session_id.0.as_bytes(),
                                        media_type,
                                        data,
                                    )
                                    .await
                                    {
                                        msg_images.push(image);
                                    }
                                }
                                librefang_types::message::ContentBlock::ToolUse {
                                    id,
                                    name,
                                    input,
                                    ..
                                } => {
                                    let tool_idx = tools.len();
                                    tools.push(serde_json::json!({
                                        "name": name,
                                        "input": input,
                                        "running": false,
                                        "expanded": false,
                                    }));
                                    // Will be filled after this loop when we know msg_idx
                                    tool_use_index.insert(id.clone(), (usize::MAX, tool_idx));
                                }
                                // ToolResult blocks are handled in pass 2
                                librefang_types::message::ContentBlock::ToolResult { .. } => {}
                                _ => {}
                            }
                        }
                        texts.join("\n")
                    }
                };
                // Skip messages that are purely tool results (User role with only ToolResult blocks).
                // A turn whose `MessageContent::Blocks` contains ONLY `Thinking` (e.g. an
                // aborted/cancelled response, or a server filter that stripped the visible
                // text) must NOT be dropped here — the dashboard's `hasThinking` branch
                // explicitly renders thinking-only turns. Gating on `thinkings.is_empty()`
                // keeps the original tool-result-only skip semantics intact.
                if content.is_empty() && tools.is_empty() && thinkings.is_empty() {
                    continue;
                }
                let msg_idx = built_messages.len();
                // Fix up the msg_idx for tool_use entries registered with sentinel
                for (mi, _) in tool_use_index.values_mut() {
                    if *mi == usize::MAX {
                        *mi = msg_idx;
                    }
                }
                let mut msg = serde_json::json!({
                    "role": format!("{:?}", m.role),
                    "content": content,
                });
                if !tools.is_empty() {
                    msg["tools"] = serde_json::Value::Array(tools);
                }
                if !msg_images.is_empty() {
                    msg["images"] = serde_json::Value::Array(msg_images);
                }
                if !thinkings.is_empty() {
                    // Joined the same way the dashboard's history mapper joins
                    // thinking deltas during live streaming — a blank line
                    // between blocks keeps the collapsible drawer readable.
                    msg["thinking"] = serde_json::Value::String(thinkings.join("\n\n"));
                }
                // Expose the real message timestamp so the dashboard can
                // render historical times correctly on resume instead of
                // falling back to render-time `Date.now()` (#2934). Serialized
                // as RFC 3339; messages persisted before the field existed
                // come through as `null`.
                if let Some(ts) = m.timestamp {
                    msg["timestamp"] = serde_json::Value::String(ts.to_rfc3339());
                }
                built_messages.push(msg);
            }

            // Pass 2: walk messages again and attach ToolResult to the correct tool
            for m in &session.messages {
                if let librefang_types::message::MessageContent::Blocks(blocks) = &m.content {
                    for b in blocks {
                        if let librefang_types::message::ContentBlock::ToolResult {
                            tool_use_id,
                            content: result,
                            is_error,
                            ..
                        } = b
                        {
                            if let Some(&(msg_idx, tool_idx)) = tool_use_index.get(tool_use_id) {
                                if let Some(msg) = built_messages.get_mut(msg_idx) {
                                    if let Some(tools_arr) =
                                        msg.get_mut("tools").and_then(|v| v.as_array_mut())
                                    {
                                        if let Some(tool_obj) = tools_arr.get_mut(tool_idx) {
                                            // Cap at 100 KiB of UTF-8 without splitting a code point.
                                            let capped = cap_tool_result(result);
                                            tool_obj["result"] = serde_json::Value::String(capped);
                                            tool_obj["is_error"] =
                                                serde_json::Value::Bool(*is_error);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            let messages = built_messages;

            // Expose the LLM-generated compaction summary only on the session
            // whose own history was actually compacted (#6225). The summary
            // lives in the agent-scoped `canonical_sessions` row and outlives
            // any individual session, so gating on "is this the active
            // session?" leaked a prior conversation's summary onto a freshly
            // created session that merely became active. Gate on recorded
            // ownership instead: a session — pinned or active — that never
            // produced this summary gets null and the banner stays hidden.
            let compacted_summary: Option<String> = state
                .kernel
                .memory_substrate()
                .compacted_summary_for_session(agent_id, target_session_id)
                .ok()
                .flatten();

            // #3511: tag session_id (and agent_id) so the access-log
            // middleware can emit them as structured fields.
            crate::extensions::with_session_id(
                session.id,
                crate::extensions::with_agent_id(
                    agent_id,
                    (
                        StatusCode::OK,
                        Json(serde_json::json!({
                            "session_id": session.id.0.to_string(),
                            "agent_id": session.agent_id.0.to_string(),
                            "message_count": session.messages.len(),
                            "context_window_tokens": session.context_window_tokens,
                            "label": session.label,
                            "messages": messages,
                            "compacted_summary": compacted_summary,
                        })),
                    ),
                ),
            )
        }
        Ok(None) => {
            // The session row is not materialized in the memory substrate
            // (e.g. agent just spawned, no messages yet). If the caller pinned
            // an explicit session_id that does NOT match this agent's
            // canonical-active id, refuse — otherwise the response would
            // silently fall back to the agent's own canonical-empty session
            // under the requested id, hiding the cross-agent guard. The
            // canonical id is owned by this agent by construction (registry
            // entry), so matching it is safe to treat as the no-query path.
            if let Some(requested) = params.session_id {
                if requested != entry.session_id.0 {
                    return ApiErrorResponse::not_found("session not found for this agent")
                        .with_code("session_agent_mismatch")
                        .into_response();
                }
            }
            // Expose the LLM-generated compaction summary even when the
            // session row itself is not yet materialised (e.g. agent just
            // spawned but store_llm_summary was called directly, as in
            // tests), but only when this active session is the one that
            // actually owns the summary (#6225) — never a freshly created
            // session that inherited the agent-scoped row.
            let compacted_summary: Option<String> = state
                .kernel
                .memory_substrate()
                .compacted_summary_for_session(agent_id, entry.session_id)
                .ok()
                .flatten();

            // #3511: tag both identifiers even for the empty-session case.
            crate::extensions::with_session_id(
                entry.session_id,
                crate::extensions::with_agent_id(
                    agent_id,
                    (
                        StatusCode::OK,
                        Json(serde_json::json!({
                            "session_id": entry.session_id.0.to_string(),
                            "agent_id": agent_id.to_string(),
                            "message_count": 0,
                            "context_window_tokens": 0,
                            "label": null,
                            "messages": [],
                            "compacted_summary": compacted_summary,
                        })),
                    ),
                ),
            )
        }
        Err(e) => {
            tracing::warn!("Session load failed for agent {id}: {e}");
            ApiErrorResponse::internal(err_session_load_failed)
                .with_code("session_load_failed")
                .into_response()
        }
    }
}

/// Lightweight context-window usage indicator for an agent session.
///
/// Distinct from `GET /api/agents/{id}/session`: that endpoint returns the full
/// message history and exposes only the X numerator
/// (`context_window_tokens`). This endpoint resolves the Y denominator (the
/// model's context window, via the same precedence chain the agent loop uses)
/// and the percentage, so the dashboard can render a cheap polled "how full is
/// the window" bar without pulling the heavy history payload.
#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct SessionContextResponse {
    /// Estimated tokens currently in the context window (chars/4 heuristic).
    pub used_tokens: usize,
    /// Resolved model context window. Falls back to
    /// `UNKNOWN_MODEL_CONTEXT_WINDOW` (8192) for an unknown model, so this is
    /// always positive.
    pub max_context_tokens: usize,
    /// Which layer of the precedence chain produced `max_context_tokens`
    /// (refs #7774): `agent_override`, `model_override`, `catalog`,
    /// `session_hint` or `fallback`.
    ///
    /// Without this the number is unreadable: a window an operator set, one the
    /// registry declared and one the runtime invented are all the same integer.
    pub max_context_tokens_source: String,
    /// True when `max_context_tokens` is a guess rather than a fact about the
    /// model — i.e. the source is `fallback` (refs #7774).
    ///
    /// The condition behind the report that opened the issue: a gateway-served
    /// model reports no window, the runtime assumes 8192, and a conversation
    /// well inside the model's real window is refused for an overflow that
    /// exists only in that assumption.
    /// Clients render the warning off this flag rather than string-matching the
    /// source.
    pub max_context_tokens_assumed: bool,
    /// Usage percentage, clamped to 100 with one decimal of precision.
    pub pct: f64,
    /// The agent's model id.
    pub model: String,
    /// Pressure level: `low` / `medium` / `high` / `critical`.
    pub pressure: String,
}

/// GET /api/agents/{id}/session/context — context-window usage indicator.
#[utoipa::path(
    get,
    path = "/api/agents/{id}/session/context",
    tag = "agents",
    params(
        ("id" = String, Path, description = "Agent ID"),
        ("session_id" = Option<String>, Query, description = "Optional session id to report on instead of the canonical active session"),
    ),
    responses(
        (status = 200, description = "Context window usage for the requested (or active) session", body = SessionContextResponse),
        (status = 400, description = "Invalid agent or session ID"),
        (status = 404, description = "Agent or session not found")
    )
)]
pub async fn get_agent_session_context(
    State(state): State<Arc<AppState>>,
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
    Path(id): Path<String>,
    query: Result<Query<GetAgentSessionQuery>, axum::extract::rejection::QueryRejection>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let Query(params) = match query {
        Ok(q) => q,
        Err(_) => {
            return ApiErrorResponse::bad_request(t.t("api-error-session-invalid-id"))
                .with_code("invalid_session_id")
                .into_response();
        }
    };
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return ApiErrorResponse::bad_request(t.t("api-error-agent-invalid-id"))
                .with_code("invalid_agent_id")
                .into_response();
        }
    };

    let entry = match state.kernel.agent_registry().get(agent_id) {
        Some(e) => e,
        None => {
            return ApiErrorResponse::not_found(t.t("api-error-agent-not-found"))
                .with_code("agent_not_found")
                .into_response();
        }
    };
    if !super::super::can_access_agent(&state, agent_id, api_user.as_ref()) {
        return ApiErrorResponse::not_found(t.t("api-error-agent-not-found"))
            .with_code("agent_not_found")
            .into_response();
    }
    let model = entry.manifest.model.model.clone();

    // A dashboard tab can pin a non-active session via `?session_id=`. Validate
    // ownership exactly as `get_agent_session` does so one agent's usage cannot
    // be read through another agent's id. An unmaterialized session row (no
    // messages yet) is only accepted when it is this agent's own canonical id.
    let session_override = params.session_id.map(librefang_types::agent::SessionId);
    if let Some(target) = session_override {
        match state.kernel.memory_substrate().get_session(target) {
            Ok(Some(s)) if s.agent_id != agent_id => {
                return ApiErrorResponse::not_found(t.t("api-error-session-not-found"))
                    .with_code("session_agent_mismatch")
                    .into_response();
            }
            Ok(Some(_)) => {}
            Ok(None) => {
                if target.0 != entry.session_id.0 {
                    return ApiErrorResponse::not_found(t.t("api-error-session-not-found"))
                        .with_code("session_agent_mismatch")
                        .into_response();
                }
            }
            Err(e) => {
                tracing::warn!("Session load failed for agent {id}: {e}");
                return ApiErrorResponse::internal(t.t("api-error-session-load-failed"))
                    .with_code("session_load_failed")
                    .into_response();
            }
        }
    }
    // ErrorTranslator is !Send; context_report below is sync so drop happens
    // before there is any await, but keep the drop explicit per the repo gotcha.
    // Translate the context-report failure message before the drop.
    let context_report_failed_msg = t.t("api-error-context-report-failed");
    drop(t);

    let report = match state
        .kernel
        .context_report_for_session(agent_id, session_override)
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("Context report failed for agent {id}: {e}");
            return ApiErrorResponse::internal(context_report_failed_msg)
                .with_code("context_report_failed")
                .into_response();
        }
    };

    crate::extensions::with_agent_id(
        agent_id,
        (
            StatusCode::OK,
            Json(SessionContextResponse {
                used_tokens: report.estimated_tokens,
                max_context_tokens: report.context_window,
                max_context_tokens_source: report.context_window_source.as_str().to_string(),
                max_context_tokens_assumed: report.context_window_source.is_assumed(),
                pct: report.usage_percent,
                model,
                pressure: format!("{:?}", report.pressure).to_lowercase(),
            }),
        ),
    )
}

/// GET /api/agents/{id}/sessions/{session_id}/stream — attach to a session's
/// in-flight stream events (SSE or WebSocket).
///
/// Any client can subscribe to the events emitted by an active turn on this
/// session: the originating client (CLI, Tauri desktop, web) plus any number
/// of additional clients. Late attachers begin receiving events from the
/// moment they subscribe — partial-turn snapshots are not replayed.
///
/// Returns 404 if the session does not exist or belongs to a different agent.
#[utoipa::path(
    get,
    path = "/api/agents/{id}/sessions/{session_id}/stream",
    tag = "agents",
    params(
        ("id" = String, Path, description = "Agent ID"),
        ("session_id" = String, Path, description = "Session ID to attach to"),
    ),
    responses(
        (status = 200, description = "Server-sent events stream of session events"),
        (status = 101, description = "WebSocket session event stream"),
        (status = 400, description = "Invalid agent or session ID"),
        (status = 404, description = "Agent or session not found")
    )
)]
pub async fn attach_session_stream(
    ws: Result<
        axum::extract::ws::WebSocketUpgrade,
        axum::extract::ws::rejection::WebSocketUpgradeRejection,
    >,
    State(state): State<Arc<AppState>>,
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
    connect_info: Option<axum::Extension<axum::extract::ConnectInfo<std::net::SocketAddr>>>,
    Path((id, session_id_str)): Path<(String, String)>,
    headers: axum::http::HeaderMap,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> axum::response::Response {
    use axum::response::sse::{Event, Sse};
    use futures::stream;
    use tokio::sync::broadcast::error::RecvError;

    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));

    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            )
                .into_response();
        }
    };

    let session_id = match session_id_str.parse::<uuid::Uuid>() {
        Ok(uuid) => librefang_types::agent::SessionId(uuid),
        Err(_) => {
            return ApiErrorResponse::bad_request(t.t("api-error-session-invalid-id"))
                .with_code("invalid_session_id")
                .into_response();
        }
    };

    let agent_entry = match state.kernel.agent_registry().get(agent_id) {
        Some(e) => e,
        None => {
            return ApiErrorResponse::not_found(t.t("api-error-agent-not-found"))
                .with_code("agent_not_found")
                .into_response();
        }
    };
    if !super::super::can_access_agent(&state, agent_id, api_user.as_ref()) {
        return ApiErrorResponse::not_found(t.t("api-error-agent-not-found"))
            .with_code("agent_not_found")
            .into_response();
    }

    // Validate the session belongs to this agent. Two acceptable shapes:
    //   1. The session has been persisted (one or more turns ran) and its
    //      `agent_id` matches the path agent.
    //   2. The session has not been persisted yet (fresh agent, no turn yet)
    //      but the id matches the agent's canonical `session_id` from the
    //      registry. Sessions are written lazily on first turn, so requiring
    //      a memory row would forbid attach-before-first-turn.
    // Anything else is rejected — a caller cannot attach to an arbitrary
    // session UUID without first proving the agent–session binding.
    let session_lookup = state.kernel.memory_substrate().get_session(session_id);
    let session_valid = match &session_lookup {
        Ok(Some(s)) => s.agent_id == agent_id,
        Ok(None) => agent_entry.session_id == session_id,
        Err(_) => false,
    };
    if !session_valid {
        if let Err(e) = session_lookup {
            // Scrub the raw session-load error (audit:
            // rusqlite-errors-leak) — the failure originates in the
            // memory substrate, so the chain carries SQL detail. The
            // full error reaches `error!`; the client sees the generic
            // body plus the stable `session_load_failed` code.
            return ApiErrorResponse::internal_scrub(&e)
                .with_code("session_load_failed")
                .into_response();
        }
        return ApiErrorResponse::not_found("session not found for this agent")
            .with_code("session_agent_mismatch")
            .into_response();
    }

    let receiver = state.kernel.session_stream_hub().subscribe(session_id);
    let lifecycle = state.kernel.session_lifecycle_bus().subscribe();

    if let Ok(ws) = ws {
        let cfg = state.kernel.config_ref();
        let listen_port = cfg
            .api_listen
            .parse::<std::net::SocketAddr>()
            .ok()
            .map(|address| address.port());
        let allow_remote = std::env::var("LIBREFANG_ALLOW_NO_AUTH")
            .map(|value| matches!(value.trim(), "1" | "true" | "TRUE" | "yes" | "on"))
            .unwrap_or(false);
        if crate::ws::validate_ws_origin(&headers, listen_port, &cfg.cors_origin, allow_remote)
            .is_err()
        {
            return StatusCode::FORBIDDEN.into_response();
        }

        let Some(axum::Extension(axum::extract::ConnectInfo(peer))) = connect_info else {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        };
        let client_ip = crate::client_ip::resolve_real_client_ip(
            peer.ip(),
            &headers,
            &state.trusted_proxies,
            state.trust_forwarded_for,
        );
        let Some(connection_guard) = crate::ws::try_acquire_ws_slot(
            client_ip,
            state.kernel.config_ref().rate_limit.max_ws_per_ip,
        ) else {
            return StatusCode::TOO_MANY_REQUESTS.into_response();
        };

        let upgrade = match crate::ws::ws_bearer_protocol(&headers) {
            Some(protocol) => ws.protocols([protocol]),
            None => ws,
        };
        return crate::extensions::with_session_id(
            session_id,
            crate::extensions::with_agent_id(
                agent_id,
                upgrade.on_upgrade(move |socket| {
                    session_stream_websocket(
                        socket,
                        receiver,
                        lifecycle,
                        agent_id,
                        session_id,
                        connection_guard,
                    )
                }),
            ),
        );
    }

    // Bridge broadcast::Receiver into an SSE stream. Skip Lagged events with
    // a debug log (intentionally lossy semantics — see SessionStreamHub
    // docs) and end the stream when the channel closes.
    let sse_stream = stream::unfold(
        (receiver, lifecycle, SessionStreamState::new(), false),
        move |(mut rx, mut lifecycle, mut stream_state, finished)| async move {
            if finished {
                return None;
            }
            loop {
                tokio::select! {
                    received = rx.recv() => {
                        let event = match received {
                            Ok(event) => event,
                            Err(RecvError::Lagged(n)) => {
                                tracing::debug!(skipped = n, "session attach stream lagged, skipping");
                                continue;
                            }
                            Err(RecvError::Closed) => return None,
                        };
                        let Some((event_type, payload, terminal)) =
                            session_stream_payload(event, &mut stream_state)
                        else {
                            continue;
                        };
                        let sse_event: Result<Event, std::convert::Infallible> =
                            Ok(Event::default()
                                .event(event_type)
                                .json_data(payload)
                                .unwrap_or_else(|_| Event::default().data("error")));
                        return Some((
                            sse_event,
                            (rx, lifecycle, stream_state, terminal),
                        ));
                    }
                    received = lifecycle.recv() => {
                        let event = match received {
                            Ok(event) => event,
                            Err(RecvError::Lagged(n)) => {
                                tracing::debug!(skipped = n, "session lifecycle stream lagged, skipping");
                                continue;
                            }
                            Err(RecvError::Closed) => return None,
                        };
                        let Some((event_type, payload)) = session_lifecycle_payload(
                            event,
                            agent_id,
                            session_id,
                            &stream_state,
                        ) else {
                            continue;
                        };
                        let sse_event: Result<Event, std::convert::Infallible> =
                            Ok(Event::default()
                                .event(event_type)
                                .json_data(payload)
                                .unwrap_or_else(|_| Event::default().data("error")));
                        return Some((
                            sse_event,
                            (rx, lifecycle, stream_state, true),
                        ));
                    }
                }
            }
        },
    );

    // #3511: tag both agent_id and session_id so the access-log middleware
    // can emit them as structured fields on this SSE endpoint's log line.
    crate::extensions::with_session_id(
        session_id,
        crate::extensions::with_agent_id(
            agent_id,
            Sse::new(sse_stream).keep_alive(
                axum::response::sse::KeepAlive::new()
                    .interval(std::time::Duration::from_secs(15))
                    .text("keep-alive"),
            ),
        ),
    )
}

async fn session_stream_websocket(
    mut socket: axum::extract::ws::WebSocket,
    mut receiver: tokio::sync::broadcast::Receiver<librefang_kernel::llm_driver::StreamEvent>,
    mut lifecycle: tokio::sync::broadcast::Receiver<
        librefang_kernel::session_lifecycle::SessionLifecycleEvent,
    >,
    agent_id: AgentId,
    session_id: librefang_types::agent::SessionId,
    _connection_guard: crate::ws::WsConnectionGuard,
) {
    use axum::extract::ws::Message;
    use futures::SinkExt as _;
    use tokio::sync::broadcast::error::RecvError;

    let mut stream_state = SessionStreamState::new();
    loop {
        tokio::select! {
            received = receiver.recv() => {
                let event = match received {
                    Ok(event) => event,
                    Err(RecvError::Lagged(skipped)) => {
                        tracing::debug!(skipped, "session attach WebSocket lagged, skipping");
                        continue;
                    }
                    Err(RecvError::Closed) => break,
                };
                let Some((event_type, payload, terminal)) =
                    session_stream_payload(event, &mut stream_state)
                else {
                    continue;
                };
                if send_session_stream_message(&mut socket, event_type, payload)
                    .await
                    .is_err()
                {
                    break;
                }
                if terminal {
                    let _ = socket.close().await;
                    break;
                }
            }
            received = lifecycle.recv() => {
                let event = match received {
                    Ok(event) => event,
                    Err(RecvError::Lagged(skipped)) => {
                        tracing::debug!(skipped, "session lifecycle WebSocket lagged, skipping");
                        continue;
                    }
                    Err(RecvError::Closed) => break,
                };
                let Some((event_type, payload)) = session_lifecycle_payload(
                    event,
                    agent_id,
                    session_id,
                    &stream_state,
                ) else {
                    continue;
                };
                let _ = send_session_stream_message(&mut socket, event_type, payload).await;
                let _ = socket.close().await;
                break;
            }
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Ping(data))) => {
                        if socket.send(Message::Pong(data)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    Some(Ok(_)) => {}
                }
            }
        }
    }
}

async fn send_session_stream_message(
    socket: &mut axum::extract::ws::WebSocket,
    event_type: &str,
    payload: serde_json::Value,
) -> Result<(), axum::Error> {
    use axum::extract::ws::Message;

    let mut envelope = match payload {
        serde_json::Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    envelope.insert(
        "type".to_string(),
        serde_json::Value::String(event_type.to_string()),
    );
    socket
        .send(Message::Text(
            serde_json::Value::Object(envelope).to_string().into(),
        ))
        .await
}

struct SessionStreamState {
    dedup: StreamDedup,
    input_tokens: u64,
    output_tokens: u64,
}

impl SessionStreamState {
    fn new() -> Self {
        Self {
            dedup: StreamDedup::new(),
            input_tokens: 0,
            output_tokens: 0,
        }
    }
}

fn session_stream_payload(
    event: librefang_kernel::llm_driver::StreamEvent,
    state: &mut SessionStreamState,
) -> Option<(&'static str, serde_json::Value, bool)> {
    use librefang_kernel::llm_driver::{StreamEvent, PHASE_RESPONSE_COMPLETE};

    match event {
        StreamEvent::TextDelta { text } => {
            if state.dedup.is_duplicate(&text) {
                return None;
            }
            state.dedup.record_sent(&text);
            Some((
                "chunk",
                serde_json::json!({"content": text, "done": false}),
                false,
            ))
        }
        StreamEvent::ToolUseStart { name, .. } => {
            Some(("tool_use", serde_json::json!({"tool": name}), false))
        }
        StreamEvent::ToolUseEnd { name, input, .. } => Some((
            "tool_result",
            serde_json::json!({"tool": name, "input": input}),
            false,
        )),
        StreamEvent::ContentComplete { usage, .. } => {
            state.input_tokens = state.input_tokens.saturating_add(usage.input_tokens);
            state.output_tokens = state.output_tokens.saturating_add(usage.output_tokens);
            None
        }
        StreamEvent::PhaseChange { phase, .. } if phase == PHASE_RESPONSE_COMPLETE => Some((
            "done",
            serde_json::json!({
                "done": true,
                "usage": {
                    "input_tokens": state.input_tokens,
                    "output_tokens": state.output_tokens,
                }
            }),
            true,
        )),
        StreamEvent::PhaseChange { phase, detail } => Some((
            "phase",
            serde_json::json!({"phase": phase, "detail": detail}),
            false,
        )),
        StreamEvent::OwnerNotice { text } => {
            Some(("owner_notice", serde_json::json!({"text": text}), false))
        }
        _ => None,
    }
}

fn session_lifecycle_payload(
    event: librefang_kernel::session_lifecycle::SessionLifecycleEvent,
    expected_agent_id: AgentId,
    expected_session_id: librefang_types::agent::SessionId,
    _state: &SessionStreamState,
) -> Option<(&'static str, serde_json::Value)> {
    use librefang_kernel::session_lifecycle::SessionLifecycleEvent;

    match event {
        SessionLifecycleEvent::TurnFailed {
            agent_id,
            session_id,
            ..
        } if agent_id == expected_agent_id && session_id == expected_session_id => Some((
            "phase",
            serde_json::json!({"phase": "error", "detail": null}),
        )),
        SessionLifecycleEvent::AgentTerminated { agent_id, .. }
            if agent_id == expected_agent_id =>
        {
            Some((
                "phase",
                serde_json::json!({"phase": "error", "detail": null}),
            ))
        }
        _ => None,
    }
}

#[utoipa::path(
    get,
    path = "/api/agents/{id}/sessions",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "List all sessions for an agent", body = crate::types::JsonObject)
    )
)]
pub async fn list_agent_sessions(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            )
        }
    };
    // Owner-scoping: non-admins can only list sessions for agents they
    // authored. Mirrors the filter on `list_agents` so per-agent
    // session metadata (cost, message count) doesn't leak.
    if let Some(ref user) = api_user {
        use crate::middleware::UserRole;
        if user.0.role < UserRole::Admin {
            let entry = state.kernel.agent_registry().get(agent_id);
            let owned = entry
                .as_ref()
                .map(|e| e.manifest.author.eq_ignore_ascii_case(&user.0.name))
                .unwrap_or(false);
            if !owned {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
                );
            }
        }
    }
    match state.kernel.list_agent_sessions(agent_id) {
        Ok(sessions) => (
            StatusCode::OK,
            Json(serde_json::json!({"sessions": sessions})),
        ),
        Err(e) => {
            let status = kernel_err_to_status(&e);
            (
                status,
                Json(serde_json::json!({"error": kernel_err_body(status, &e, &t)})),
            )
        }
    }
}

/// POST /api/agents/{id}/sessions — Create a new session for an agent.
#[utoipa::path(
    post,
    path = "/api/agents/{id}/sessions",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body(content = crate::types::JsonObject, description = "Optional label for the new session"),
    responses(
        (status = 200, description = "Create a new session for an agent", body = crate::types::JsonObject)
    )
)]
pub async fn create_agent_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            )
        }
    };
    let label = req.get("label").and_then(|v| v.as_str());
    match state.kernel.create_agent_session(agent_id, label) {
        Ok(session) => (StatusCode::OK, Json(session)),
        Err(e) => {
            let status = kernel_err_to_status(&e);
            (
                status,
                Json(serde_json::json!({"error": kernel_err_body(status, &e, &t)})),
            )
        }
    }
}

/// POST /api/agents/{id}/sessions/{session_id}/switch — Switch to an existing session.
#[utoipa::path(
    post,
    path = "/api/agents/{id}/sessions/{session_id}/switch",
    tag = "agents",
    params(
        ("id" = String, Path, description = "Agent ID"),
        ("session_id" = String, Path, description = "Session ID to switch to"),
    ),
    responses(
        (status = 200, description = "Switch to an existing session", body = crate::types::JsonObject)
    )
)]
pub async fn switch_agent_session(
    State(state): State<Arc<AppState>>,
    Path((id, session_id_str)): Path<(String, String)>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            )
        }
    };
    let session_id = match session_id_str.parse::<uuid::Uuid>() {
        Ok(uuid) => librefang_types::agent::SessionId(uuid),
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-session-invalid-id")})),
            )
        }
    };
    match state.kernel.switch_agent_session(agent_id, session_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": "Session switched"})),
        ),
        Err(e) => {
            let status = kernel_err_to_status(&e);
            (
                status,
                Json(serde_json::json!({"error": kernel_err_body(status, &e, &t)})),
            )
        }
    }
}

// ── Session Export / Import (Hibernation) ───────────────────────────────
/// GET /api/agents/{id}/sessions/{session_id}/export — Export a session for hibernation.
#[utoipa::path(
    get,
    path = "/api/agents/{id}/sessions/{session_id}/export",
    tag = "agents",
    params(
        ("id" = String, Path, description = "Agent ID"),
        ("session_id" = String, Path, description = "Session ID to export"),
    ),
    responses(
        (status = 200, description = "Exported session data", body = crate::types::JsonObject)
    )
)]
pub async fn export_session(
    State(state): State<Arc<AppState>>,
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
    Path((id, session_id_str)): Path<(String, String)>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            )
        }
    };
    if !super::super::can_access_agent(&state, agent_id, api_user.as_ref()) {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
        );
    }
    let session_id = match session_id_str.parse::<uuid::Uuid>() {
        Ok(uuid) => librefang_types::agent::SessionId(uuid),
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid session ID"})),
            )
        }
    };
    match state.kernel.export_session(agent_id, session_id) {
        Ok(export) => (
            StatusCode::OK,
            Json(serde_json::to_value(export).unwrap_or_default()),
        ),
        Err(e) => {
            let status = kernel_err_to_status(&e);
            (
                status,
                Json(serde_json::json!({"error": kernel_err_body(status, &e, &t)})),
            )
        }
    }
}

/// GET /api/agents/{id}/sessions/{session_id}/trajectory — Export a redacted
/// trajectory (audit trail) for the given session.
///
/// Returns a privacy-redacted bundle of the session messages plus metadata
/// (agent name, model, system prompt fingerprint, librefang version). Intended
/// for support, audit, and compliance flows.
///
/// Query parameters:
/// - `format=json` (default): single JSON object response.
/// - `format=jsonl`: NDJSON, first line is metadata header, subsequent lines
///   are messages one-per-line. `Content-Type: application/x-ndjson`.
#[utoipa::path(
    get,
    path = "/api/agents/{id}/sessions/{session_id}/trajectory",
    tag = "agents",
    params(
        ("id" = String, Path, description = "Agent ID"),
        ("session_id" = String, Path, description = "Session ID to export"),
        ("format" = Option<String>, Query, description = "Response format: 'json' (default) or 'jsonl'"),
    ),
    responses(
        (status = 200, description = "Redacted trajectory bundle", body = crate::types::JsonObject),
        (status = 400, description = "Invalid agent or session ID"),
        (status = 404, description = "Agent or session not found"),
    )
)]
pub async fn export_session_trajectory(
    State(state): State<Arc<AppState>>,
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
    Path((id, session_id_str)): Path<(String, String)>,
    Query(params): Query<HashMap<String, String>>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> axum::response::Response {
    use axum::http::header;
    use axum::response::IntoResponse;

    let (err_invalid_id, err_session_invalid, err_not_found, err_session_not_found) = {
        let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
        (
            t.t("api-error-agent-invalid-id"),
            t.t("api-error-session-invalid-id"),
            t.t("api-error-agent-not-found"),
            "Session not found".to_string(),
        )
    };

    // Parse agent ID.
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": err_invalid_id})),
            )
                .into_response();
        }
    };
    if !super::super::can_access_agent(&state, agent_id, api_user.as_ref()) {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": err_not_found})),
        )
            .into_response();
    }

    // Parse session ID.
    let session_id = match session_id_str.parse::<uuid::Uuid>() {
        Ok(uuid) => librefang_types::agent::SessionId(uuid),
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": err_session_invalid})),
            )
                .into_response();
        }
    };

    // Build the redacted bundle via the kernel surface so this route does
    // not need to import `librefang_kernel::trajectory` directly (#3744).
    let bundle = match state.kernel.export_session_trajectory(agent_id, session_id) {
        Ok(b) => b,
        Err(crate::error::KernelError::LibreFang(
            librefang_types::error::LibreFangError::AgentNotFound(_),
        )) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": err_not_found})),
            )
                .into_response();
        }
        Err(crate::error::KernelError::LibreFang(
            librefang_types::error::LibreFangError::Memory { message: msg, .. },
        )) if msg.contains("not found") || msg.contains("does not belong") => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": err_session_not_found})),
            )
                .into_response();
        }
        Err(e) => {
            let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": scrub_500(&e, &t)})),
            )
                .into_response();
        }
    };

    let format = params
        .get("format")
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_else(|| "json".to_string());

    let (body, content_type, ext): (String, &'static str, &'static str) = if format == "jsonl" {
        (bundle.to_jsonl(), "application/x-ndjson", "jsonl")
    } else {
        let json = match bundle.to_json() {
            Ok(json) => json,
            Err(error) => {
                let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": scrub_500(&error, &t)})),
                )
                    .into_response();
            }
        };
        (json.to_string(), "application/json", "json")
    };

    let filename = format!("trajectory-{}.{}", session_id.0, ext);
    let disposition = format!("attachment; filename=\"{}\"", filename);

    axum::response::Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CONTENT_DISPOSITION, disposition)
        .body(axum::body::Body::from(body))
        .unwrap_or_else(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "failed to build response"})),
            )
                .into_response()
        })
}

/// POST /api/agents/{id}/sessions/import — Import a previously exported session.
#[utoipa::path(
    post,
    path = "/api/agents/{id}/sessions/import",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body(content = crate::types::JsonObject, description = "Exported session JSON"),
    responses(
        (status = 200, description = "Session imported successfully", body = crate::types::JsonObject)
    )
)]
pub async fn import_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            )
        }
    };
    let export: librefang_memory::session::SessionExport = match serde_json::from_value(body) {
        Ok(e) => e,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Invalid export format: {e}")})),
            )
        }
    };
    match state.kernel.import_session(agent_id, export) {
        Ok(new_session_id) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ok",
                "session_id": new_session_id.0.to_string(),
                "message": "Session imported successfully"
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": scrub_500(&e, &t)})),
        ),
    }
}

// ── Extended Chat Command API Endpoints ─────────────────────────────────
/// POST /api/agents/{id}/session/reset — Reset an agent's session.
#[utoipa::path(
    post,
    path = "/api/agents/{id}/session/reset",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "Reset an agent's current session", body = crate::types::JsonObject)
    )
)]
pub async fn reset_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    // `ErrorTranslator` is `!Send` (per repo CLAUDE.md) — never hold it
    // across an `.await`, or axum's `Handler` trait bound fails with a
    // cryptic message. Same shape as `compact_session` below.
    let l = super::resolve_lang(lang.as_ref());
    let err_invalid_id = {
        let t = ErrorTranslator::new(l);
        t.t("api-error-agent-invalid-id")
    };
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": err_invalid_id})),
            )
        }
    };
    match state
        .kernel
        .reset_session(agent_id, ResetScope::Agent)
        .await
    {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": "Session reset"})),
        ),
        Err(crate::error::KernelError::LibreFang(
            librefang_types::error::LibreFangError::InvalidInput(msg),
        )) => {
            let t = ErrorTranslator::new(l);
            (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &msg)])}),
                ),
            )
        }
        Err(e) => {
            let t = ErrorTranslator::new(l);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": scrub_500(&e, &t)})),
            )
        }
    }
}

/// POST /api/agents/{id}/session/reboot — Hard-reboot an agent's session (full clear, no summary).
#[utoipa::path(
    post,
    path = "/api/agents/{id}/session/reboot",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "Hard-reboot an agent's session without saving summary", body = crate::types::JsonObject)
    )
)]
pub async fn reboot_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    // `ErrorTranslator` is `!Send` — see note in `reset_session` above.
    let l = super::resolve_lang(lang.as_ref());
    let err_invalid_id = {
        let t = ErrorTranslator::new(l);
        t.t("api-error-agent-invalid-id")
    };
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": err_invalid_id})),
            )
        }
    };
    match state
        .kernel
        .reboot_session(agent_id, ResetScope::Agent)
        .await
    {
        Ok(()) => (
            StatusCode::OK,
            Json(
                serde_json::json!({"status": "ok", "message": "Session rebooted. Context cleared."}),
            ),
        ),
        Err(crate::error::KernelError::LibreFang(
            librefang_types::error::LibreFangError::InvalidInput(msg),
        )) => {
            let t = ErrorTranslator::new(l);
            (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &msg)])}),
                ),
            )
        }
        Err(e) => {
            let t = ErrorTranslator::new(l);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": scrub_500(&e, &t)})),
            )
        }
    }
}

/// DELETE /api/agents/{id}/history — Clear ALL conversation history for an agent.
#[utoipa::path(
    delete,
    path = "/api/agents/{id}/history",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "Clear all conversation history for an agent", body = crate::types::JsonObject)
    )
)]
pub async fn clear_agent_history(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    // `ErrorTranslator` is `!Send` — see note in `reset_session` above.
    let l = super::resolve_lang(lang.as_ref());
    let (err_invalid_id, err_not_found) = {
        let t = ErrorTranslator::new(l);
        (
            t.t("api-error-agent-invalid-id"),
            t.t("api-error-agent-not-found"),
        )
    };
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": err_invalid_id})),
            )
        }
    };
    if state.kernel.agent_registry().get(agent_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": err_not_found})),
        );
    }
    match state.kernel.clear_agent_history(agent_id).await {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": "All history cleared"})),
        ),
        Err(e) => {
            let t = ErrorTranslator::new(l);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": scrub_500(&e, &t)})),
            )
        }
    }
}

/// POST /api/agents/{id}/session/compact — Trigger LLM session compaction.
#[utoipa::path(
    post,
    path = "/api/agents/{id}/session/compact",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "Trigger LLM session compaction", body = crate::types::JsonObject)
    )
)]
pub async fn compact_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let l = super::resolve_lang(lang.as_ref());
    let err_invalid_id = {
        let t = ErrorTranslator::new(l);
        t.t("api-error-agent-invalid-id")
    };
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": err_invalid_id})),
            )
        }
    };
    match state.kernel.compact_agent_session(agent_id, true).await {
        Ok(msg) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": msg})),
        ),
        Err(e) => {
            let t = ErrorTranslator::new(l);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": scrub_500(&e, &t)})),
            )
        }
    }
}

/// POST /api/agents/{id}/sessions/{session_id}/stop — Cancel a single
/// in-flight `(agent, session)` loop without affecting the agent's other
/// concurrent sessions.
///
/// Returns `{"status":"ok","stopped":true}` when a loop was running for that
/// pair, `{"status":"ok","stopped":false}` when nothing was running (already
/// finished, never started, or the session belongs to a different agent).
#[utoipa::path(
    post,
    path = "/api/agents/{id}/sessions/{session_id}/stop",
    tag = "agents",
    params(
        ("id" = String, Path, description = "Agent ID"),
        ("session_id" = String, Path, description = "Session ID"),
    ),
    responses(
        (status = 200, description = "Cancel a single (agent, session) loop", body = crate::types::JsonObject)
    )
)]
pub async fn stop_session(
    State(state): State<Arc<AppState>>,
    Path((id, session_id_str)): Path<(String, String)>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            )
        }
    };
    let session_id: librefang_types::agent::SessionId = match session_id_str.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-session-invalid-id")})),
            )
        }
    };
    match state.kernel.stop_session_run(agent_id, session_id) {
        Ok(stopped) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "stopped": stopped})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": scrub_500(&e, &t)})),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    #[tokio::test]
    async fn history_image_materialization_is_stable_and_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let encoded = base64::engine::general_purpose::STANDARD.encode(b"same image bytes");
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(16));
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..16 {
            let path = temp.path().to_path_buf();
            let encoded = encoded.clone();
            let barrier = barrier.clone();
            tasks.spawn(async move {
                barrier.wait().await;
                materialize_history_image(&path, b"session-a", "image/png", &encoded).await
            });
        }
        let mut materialized = Vec::new();
        while let Some(result) = tasks.join_next().await {
            materialized.push(result.unwrap().expect("concurrent materialization"));
        }

        let first = &materialized[0];
        assert!(materialized
            .iter()
            .all(|image| image["file_id"] == first["file_id"]));
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
        let file_id = first["file_id"].as_str().unwrap();
        assert!(uuid::Uuid::parse_str(file_id).is_ok());
        assert!(UPLOAD_REGISTRY.contains_key(file_id));

        let other_session =
            materialize_history_image(temp.path(), b"session-b", "image/png", &encoded)
                .await
                .expect("other-session materialization");
        let other_file_id = other_session["file_id"].as_str().unwrap();
        assert_ne!(file_id, other_file_id);
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 2);

        UPLOAD_REGISTRY.remove(file_id);
        UPLOAD_REGISTRY.remove(other_file_id);
    }

    #[test]
    fn tool_result_cap_is_a_utf8_byte_limit() {
        let input = "界".repeat(40_000);
        let capped = cap_tool_result(&input);
        assert!(capped.len() <= 102_400);
        assert!(capped.is_char_boundary(capped.len()));
        assert!(input.starts_with(&capped));
    }

    #[test]
    fn trajectory_export_internal_errors_are_scrubbed() {
        let t = ErrorTranslator::new("en");
        let detail = "database failure at /srv/private/memory.db";
        let body = scrub_500(&detail, &t);

        assert_eq!(body, "Internal server error");
        assert!(!body.contains("/srv/private"));
        assert!(!body.contains("database"));
    }
}
