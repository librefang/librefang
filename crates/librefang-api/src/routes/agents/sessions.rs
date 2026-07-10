use super::*;

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
    Path(id): Path<String>,
    query: Result<Query<GetAgentSessionQuery>, axum::extract::rejection::QueryRejection>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let Query(params) = match query {
        Ok(q) => q,
        Err(_) => {
            return ApiErrorResponse::bad_request("invalid session_id")
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
            use base64::Engine as _;
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
                                    // Persist image to upload dir so it can be
                                    // served back when loading session history.
                                    let file_id = uuid::Uuid::new_v4().to_string();
                                    let upload_dir = state
                                        .kernel
                                        .config_ref()
                                        .channels
                                        .effective_file_download_dir();
                                    if let Err(e) = std::fs::create_dir_all(&upload_dir) {
                                        tracing::warn!("Failed to create upload directory: {e}");
                                    }
                                    if let Ok(bytes) =
                                        base64::engine::general_purpose::STANDARD.decode(data)
                                    {
                                        if let Err(e) =
                                            std::fs::write(upload_dir.join(&file_id), &bytes)
                                        {
                                            tracing::warn!("Failed to write upload file: {e}");
                                        }
                                        UPLOAD_REGISTRY.insert(
                                            file_id.clone(),
                                            UploadMeta {
                                                filename: format!(
                                                    "image.{}",
                                                    media_type.rsplit('/').next().unwrap_or("png")
                                                ),
                                                content_type: media_type.clone(),
                                                // Generated content has no
                                                // operator owner — leave None.
                                                uploaded_by: None,
                                            },
                                        );
                                        msg_images.push(serde_json::json!({
                                            "file_id": file_id,
                                            "filename": format!("image.{}", media_type.rsplit('/').next().unwrap_or("png")),
                                        }));
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
                                            // Cap at 100 KB to keep session responses manageable
                                            let capped: String =
                                                result.chars().take(102_400).collect();
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
                            "messages": [],
                            "compacted_summary": compacted_summary,
                        })),
                    ),
                ),
            )
        }
        Err(e) => {
            tracing::warn!("Session load failed for agent {id}: {e}");
            ApiErrorResponse::internal(t.t("api-error-session-load-failed"))
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
                pct: report.usage_percent,
                model,
                pressure: format!("{:?}", report.pressure).to_lowercase(),
            }),
        ),
    )
}

/// GET /api/agents/{id}/sessions/{session_id}/stream — attach to a session's
/// in-flight stream events (SSE).
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
        (status = 400, description = "Invalid agent or session ID"),
        (status = 404, description = "Agent or session not found")
    )
)]
pub async fn attach_session_stream(
    State(state): State<Arc<AppState>>,
    Path((id, session_id_str)): Path<(String, String)>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> axum::response::Response {
    use axum::response::sse::{Event, Sse};
    use futures::stream;
    use librefang_kernel::llm_driver::StreamEvent;
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

    // Bridge broadcast::Receiver into an SSE stream. Skip Lagged events with
    // a debug log (intentionally lossy semantics — see SessionStreamHub
    // docs) and end the stream when the channel closes.
    let sse_stream = stream::unfold(
        (receiver, StreamDedup::new()),
        |(mut rx, mut dedup)| async move {
            loop {
                let event = match rx.recv().await {
                    Ok(ev) => ev,
                    Err(RecvError::Lagged(n)) => {
                        tracing::debug!(skipped = n, "session attach stream lagged, skipping");
                        continue;
                    }
                    Err(RecvError::Closed) => return None,
                };
                let sse_event: Result<Event, std::convert::Infallible> = Ok(match event {
                    StreamEvent::TextDelta { text } => {
                        if dedup.is_duplicate(&text) {
                            continue;
                        }
                        dedup.record_sent(&text);
                        Event::default()
                            .event("chunk")
                            .json_data(serde_json::json!({"content": text, "done": false}))
                            .unwrap_or_else(|_| Event::default().data("error"))
                    }
                    StreamEvent::ToolUseStart { name, .. } => Event::default()
                        .event("tool_use")
                        .json_data(serde_json::json!({"tool": name}))
                        .unwrap_or_else(|_| Event::default().data("error")),
                    StreamEvent::ToolUseEnd { name, input, .. } => Event::default()
                        .event("tool_result")
                        .json_data(serde_json::json!({"tool": name, "input": input}))
                        .unwrap_or_else(|_| Event::default().data("error")),
                    StreamEvent::ContentComplete { usage, .. } => Event::default()
                        .event("done")
                        .json_data(serde_json::json!({
                            "done": true,
                            "usage": {
                                "input_tokens": usage.input_tokens,
                                "output_tokens": usage.output_tokens,
                            }
                        }))
                        .unwrap_or_else(|_| Event::default().data("error")),
                    StreamEvent::PhaseChange { phase, detail } => Event::default()
                        .event("phase")
                        .json_data(serde_json::json!({
                            "phase": phase,
                            "detail": detail,
                        }))
                        .unwrap_or_else(|_| Event::default().data("error")),
                    StreamEvent::OwnerNotice { text } => Event::default()
                        .event("owner_notice")
                        .json_data(serde_json::json!({ "text": text }))
                        .unwrap_or_else(|_| Event::default().data("error")),
                    _ => Event::default().comment("skip"),
                });
                return Some((sse_event, (rx, dedup)));
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
    Path((id, session_id_str)): Path<(String, String)>,
    Query(params): Query<HashMap<String, String>>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> axum::response::Response {
    use axum::http::header;
    use axum::response::IntoResponse;

    let (
        err_invalid_id,
        err_session_invalid,
        err_not_found,
        err_session_not_found,
        err_generic_key,
    ) = {
        let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
        (
            t.t("api-error-agent-invalid-id"),
            t.t("api-error-session-invalid-id"),
            t.t("api-error-agent-not-found"),
            "Session not found".to_string(),
            "api-error-generic".to_string(),
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
            let msg = t.t_args(&err_generic_key, &[("error", &e.to_string())]);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": msg})),
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
        (bundle.to_json().to_string(), "application/json", "json")
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
