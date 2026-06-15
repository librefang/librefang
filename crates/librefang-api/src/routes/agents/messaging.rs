use super::*;

/// POST /api/agents/:id/message — Send a message to an agent.
#[utoipa::path(
    post,
    path = "/api/agents/{id}/message",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body = crate::types::MessageRequest,
    responses(
        (status = 200, description = "Message response", body = crate::types::MessageResponse),
        (status = 404, description = "Agent not found")
    )
)]
pub async fn send_message(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<MessageRequest>,
) -> impl IntoResponse {
    // Pre-translate error messages before the `.await` point below.
    // `ErrorTranslator` wraps a `FluentBundle` which is `!Send`, so it must
    // not be held across an await boundary (axum requires `Send` futures).
    let l = super::resolve_lang(lang.as_ref());
    let (err_invalid_id, err_too_large, err_not_found, err_auth_missing) = {
        let t = ErrorTranslator::new(l);
        (
            t.t("api-error-agent-invalid-id"),
            t.t("api-error-message-too-large"),
            t.t("api-error-agent-not-found"),
            t.t("api-error-auth-missing"),
        )
    };

    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return ApiErrorResponse::bad_request(err_invalid_id)
                .with_code("invalid_agent_id")
                .into_response();
        }
    };

    // SECURITY: Reject oversized messages to prevent OOM / LLM token abuse.
    // Audit: message-byte-vs-char-cap — the byte-only check used to
    // unfairly clip CJK users (3 bytes/glyph). The helper enforces
    // both MAX_MESSAGE_BYTES (memory cap) and MAX_MESSAGE_CHARS
    // (LLM-cost cap) so the limits are fair across scripts.
    if crate::validation::check_message_size(&req.message).is_err() {
        // #3511: tag every response for which `agent_id` is known so
        // request_logging middleware can emit it as a structured field.
        return crate::extensions::with_agent_id(
            agent_id,
            ApiErrorResponse::bad_request(err_too_large)
                .with_code("message_too_large")
                .with_status(StatusCode::PAYLOAD_TOO_LARGE),
        );
    }

    // Check agent exists before processing
    if state.kernel.agent_registry().get(agent_id).is_none() {
        return crate::extensions::with_agent_id(
            agent_id,
            ApiErrorResponse::not_found(err_not_found).with_code("agent_not_found"),
        );
    }

    // Reject messages when the agent's provider has no API key configured
    {
        let registry = state.kernel.agent_registry();
        if let Some(entry) = registry.get(agent_id) {
            let dm = {
                let dm_override = state
                    .kernel
                    .default_model_override_ref()
                    .read()
                    .unwrap_or_else(|e| e.into_inner());
                effective_default_model(
                    &state.kernel.config_ref().default_model,
                    dm_override.as_ref(),
                )
            };
            let provider = if entry.manifest.model.provider.is_empty()
                || entry.manifest.model.provider == "default"
            {
                &dm.provider
            } else {
                &entry.manifest.model.provider
            };
            {
                let catalog = state.kernel.model_catalog_ref().load();
                if let Some(p) = catalog.get_provider(provider) {
                    if !p.auth_status.is_available() {
                        return crate::extensions::with_agent_id(
                            agent_id,
                            ApiErrorResponse {
                                error: format!("{} (provider: {})", err_auth_missing, provider),
                                code: Some("provider_auth_missing".to_string()),
                                r#type: Some("provider_auth_missing".to_string()),
                                details: None,
                                request_id: None,
                                status: StatusCode::PRECONDITION_FAILED,
                            },
                        );
                    }
                }
            }
        }
    }

    // Parse optional explicit session_id override from the request body.
    // Hoisted above the attachment-injection block so it can be threaded
    // into `inject_attachments_into_session` — attachments must land in
    // the *same* session the text dispatch will land in.
    let session_id_override = match req.session_id.as_deref() {
        None => None,
        Some(s) => match s.parse::<uuid::Uuid>() {
            Ok(id) => Some(librefang_types::agent::SessionId(id)),
            Err(_) => {
                return ApiErrorResponse::bad_request("invalid session_id: must be a UUID")
                    .with_code("invalid_session_id")
                    .into_response();
            }
        },
    };

    // Build the sender context now (hoisted from the non-ephemeral
    // branch) so the attachment pre-inject can derive the same session
    // id `send_message_with_incognito` will. Without this, the DM
    // attachment lands on the agent's most-recent registry session
    // (typically a warm group session for chat agents) and leaks across
    // chats — the 2026-05-20 incident this PR closes.
    let sender_context = request_sender_context(&req);

    // Resolve file attachments into image content blocks
    if !req.attachments.is_empty() {
        let image_blocks = resolve_attachments(&state, &req.attachments);
        if !image_blocks.is_empty() {
            // Snapshot the agent's persistent (registry) session id as
            // the last-resort fallback in
            // `resolve_attachment_session_id`. Matches the
            // `SessionMode::Persistent => entry.session_id` branch of
            // the kernel resolver in `kernel/messaging.rs`.
            let fallback_session_id = state
                .kernel
                .agent_registry()
                .get(agent_id)
                .map(|e| e.session_id)
                .unwrap_or_else(librefang_types::agent::SessionId::new);
            inject_attachments_into_session(
                state.kernel.as_ref(),
                agent_id,
                sender_context.as_ref(),
                session_id_override,
                fallback_session_id,
                image_blocks,
            );
        }
    }

    // Detect ephemeral mode: explicit flag OR `/btw ` prefix in the message text
    let (effective_message, is_ephemeral) = if req.ephemeral {
        (req.message.clone(), true)
    } else if let Some(stripped) = req.message.strip_prefix("/btw ") {
        (stripped.to_string(), true)
    } else {
        (req.message.clone(), false)
    };

    let thinking_override = req.thinking;
    let show_thinking = req.show_thinking.unwrap_or(true);

    let result = if is_ephemeral {
        // Ephemeral "side question" — use a temp session, no persistence
        let kernel = state.kernel.clone();
        let msg = effective_message.clone();
        match run_cancel_on_disconnect(async move {
            kernel.send_message_ephemeral(agent_id, &msg, None).await
        })
        .await
        {
            Ok(r) => r,
            Err(join_err) if join_err.is_cancelled() => {
                tracing::info!("send_message cancelled: client disconnected");
                return StatusCode::from_u16(499)
                    .unwrap_or(StatusCode::BAD_REQUEST)
                    .into_response();
            }
            Err(e) => Err(crate::error::KernelError::LibreFang(
                librefang_types::error::LibreFangError::Internal(format!("task panicked: {e}")),
            )),
        }
    } else {
        // `sender_context` was hoisted above the attachment-injection
        // block earlier in this handler; reuse the same value so the
        // attachment session and the text-part session are guaranteed
        // identical.
        let kernel = state.kernel.clone();
        let kernel_handle: Arc<dyn KernelHandle> = kernel.clone();
        let msg = effective_message.clone();
        let sc = sender_context.clone();
        let incognito = req.incognito;
        match run_cancel_on_disconnect(async move {
            kernel
                .send_message_with_incognito(
                    agent_id,
                    &msg,
                    Some(kernel_handle),
                    sc,
                    thinking_override,
                    session_id_override,
                    incognito,
                )
                .await
        })
        .await
        {
            Ok(r) => r,
            Err(join_err) if join_err.is_cancelled() => {
                tracing::info!("send_message cancelled: client disconnected");
                return StatusCode::from_u16(499)
                    .unwrap_or(StatusCode::BAD_REQUEST)
                    .into_response();
            }
            Err(e) => Err(crate::error::KernelError::LibreFang(
                librefang_types::error::LibreFangError::Internal(format!("task panicked: {e}")),
            )),
        }
    };

    match result {
        Ok(result) => {
            // #3511: read the post-turn registry entry to get the resolved
            // session_id. The kernel may have created a new session during the
            // turn (e.g. session_mode = "new"), so we re-read rather than
            // reusing the pre-call guard check above. Falls back to None if the
            // agent was deleted mid-turn (exceedingly rare).
            let resolved_session_id = state
                .kernel
                .agent_registry()
                .get(agent_id)
                .map(|e| e.session_id);

            // When the agent intentionally chose not to reply (NO_REPLY / [[silent]]),
            // return an empty response with the silent flag so callers can distinguish
            // intentional silence from a bug.
            if result.silent {
                let body = (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "response": "",
                        "silent": true,
                        "input_tokens": result.total_usage.input_tokens,
                        "output_tokens": result.total_usage.output_tokens,
                        "iterations": result.iterations,
                        "cost_usd": result.cost_usd,
                    })),
                );
                return match resolved_session_id {
                    Some(sid) => crate::extensions::with_session_id(
                        sid,
                        crate::extensions::with_agent_id(agent_id, body),
                    ),
                    None => crate::extensions::with_agent_id(agent_id, body),
                };
            }

            // Extract reasoning trace (optional) and strip <think>...</think>
            // blocks from the final model output.
            let thinking_trace = if show_thinking {
                crate::ws::extract_think_content(&result.response)
            } else {
                None
            };
            let cleaned = crate::ws::strip_think_tags(&result.response);

            // Guard: ensure we never return an empty response to the client
            let response = if cleaned.trim().is_empty() {
                format!(
                    "[The agent completed processing but returned no text response. ({} in / {} out | {} iter)]",
                    result.total_usage.input_tokens,
                    result.total_usage.output_tokens,
                    result.iterations,
                )
            } else {
                cleaned
            };
            // Issue #5199: surface the resolved session id in the response
            // body when the caller did NOT pin an explicit session in the
            // request. This mirrors the WS handler's `explicit_session.is_none()`
            // branch in `ws.rs` — without it the dashboard's HTTP fallback
            // (first send before WS connects, or WS drop mid-turn) cannot
            // auto-pin `?sessionId=` in the URL, and a bare `?agentId=`
            // chat stays bookmarkable into a different canonical session
            // after a daemon restart.
            //
            // Skipped when the caller already pinned a session, both to
            // mirror WS semantics and to avoid implying a server-side
            // auto-resolution that did not happen.
            let body_session_id = if session_id_override.is_none() {
                resolved_session_id.map(|sid| sid.to_string())
            } else {
                None
            };
            let body = (
                StatusCode::OK,
                Json(serde_json::json!(MessageResponse {
                    response,
                    input_tokens: result.total_usage.input_tokens,
                    output_tokens: result.total_usage.output_tokens,
                    iterations: result.iterations,
                    cost_usd: result.cost_usd,
                    decision_traces: result.decision_traces,
                    memories_saved: result.memories_saved,
                    memories_used: result.memories_used,
                    memory_conflicts: result.memory_conflicts,
                    thinking: thinking_trace,
                    owner_notice: result.owner_notice,
                    session_id: body_session_id,
                })),
            );
            match resolved_session_id {
                Some(sid) => crate::extensions::with_session_id(
                    sid,
                    crate::extensions::with_agent_id(agent_id, body),
                ),
                None => crate::extensions::with_agent_id(agent_id, body),
            }
        }
        Err(e) => {
            tracing::warn!("send_message failed for agent {id}: {e}");
            // #3541: replace the legacy `format!("{e}").contains(...)`
            // grep with a typed match on the kernel error surface. The two
            // categories with dedicated variants (`AgentNotFound`,
            // `QuotaExceeded`) become structural matches; the
            // session-mismatch path still flows through
            // `LibreFangError::Internal(_)` at the kernel side (see
            // `crates/librefang-kernel/src/kernel/mod.rs:6446 / :8099 /
            // :9454 / :9486`) so it remains a substring check scoped to
            // that variant — eliminating that last grep needs a kernel
            // emit-site refactor to a typed `SessionAgentMismatch`
            // variant, tracked as #3541 follow-up.
            use crate::error::KernelError;
            use librefang_types::error::LibreFangError;
            let (status, code) = match &e {
                KernelError::LibreFang(LibreFangError::AgentNotFound(_)) => {
                    (StatusCode::NOT_FOUND, "agent_not_found")
                }
                KernelError::LibreFang(LibreFangError::QuotaExceeded(_)) => {
                    (StatusCode::TOO_MANY_REQUESTS, "budget_exceeded")
                }
                KernelError::LibreFang(LibreFangError::Internal(msg))
                    if msg.contains("belongs to a different agent") =>
                {
                    (StatusCode::BAD_REQUEST, "session_agent_mismatch")
                }
                _ => (StatusCode::INTERNAL_SERVER_ERROR, "message_delivery_failed"),
            };
            let t = ErrorTranslator::new(l);
            // 4xx / 429 echo the kernel reason (caller-useful: not
            // found, budget exceeded, session mismatch). The 500
            // catch-all scrubs the reason (audit: rusqlite-errors-leak)
            // so a delivery failure rooted in the memory substrate does
            // not leak SQL detail. Full error already logged above.
            let error = if status == StatusCode::INTERNAL_SERVER_ERROR {
                t.t("api-error-internal")
            } else {
                t.t_args(
                    "api-error-message-delivery-failed",
                    &[("reason", &e.to_string())],
                )
            };
            ApiErrorResponse {
                error,
                code: Some(code.to_string()),
                r#type: Some(code.to_string()),
                details: None,
                request_id: None,
                status,
            }
            .into_response()
        }
    }
}

/// POST /api/agents/:id/message/stream — SSE streaming response.
#[utoipa::path(
    post,
    path = "/api/agents/{id}/message/stream",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body = crate::types::MessageRequest,
    responses(
        (status = 200, description = "Streaming message response (SSE)")
    )
)]
pub async fn send_message_stream(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<MessageRequest>,
) -> axum::response::Response {
    use axum::response::sse::{Event, Sse};
    use futures::stream;
    use librefang_kernel::llm_driver::StreamEvent;

    let (err_too_large, err_invalid_id, err_not_found, err_streaming_failed) = {
        let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
        (
            t.t("api-error-message-too-large"),
            t.t("api-error-agent-invalid-id"),
            t.t("api-error-agent-not-found"),
            t.t("api-error-message-streaming-failed"),
        )
    };

    // SECURITY: Reject oversized messages to prevent OOM / LLM token abuse.
    // Audit: message-byte-vs-char-cap — see the sibling check_message_size
    // call in `post_message`.
    if crate::validation::check_message_size(&req.message).is_err() {
        return ApiErrorResponse::bad_request(err_too_large)
            .with_code("message_too_large")
            .with_status(StatusCode::PAYLOAD_TOO_LARGE)
            .into_response();
    }

    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return ApiErrorResponse::bad_request(err_invalid_id)
                .with_code("invalid_agent_id")
                .into_response();
        }
    };

    if state.kernel.agent_registry().get(agent_id).is_none() {
        return ApiErrorResponse::not_found(err_not_found)
            .with_code("agent_not_found")
            .into_response();
    }

    // Parse optional explicit session_id override from the request body.
    // Hoisted above the attachment-injection block so it can be threaded
    // into `inject_attachments_into_session`.
    let session_id_override = match req.session_id.as_deref() {
        None => None,
        Some(s) => match s.parse::<uuid::Uuid>() {
            Ok(id) => Some(librefang_types::agent::SessionId(id)),
            Err(_) => {
                return ApiErrorResponse::bad_request("invalid session_id: must be a UUID")
                    .with_code("invalid_session_id")
                    .into_response();
            }
        },
    };

    let (sender_context, incognito, session_override) =
        build_streaming_kernel_args(&req, session_id_override);

    if !req.attachments.is_empty() {
        let image_blocks = resolve_attachments(&state, &req.attachments);
        if !image_blocks.is_empty() {
            let fallback_session_id = state
                .kernel
                .agent_registry()
                .get(agent_id)
                .map(|e| e.session_id)
                .unwrap_or_else(librefang_types::agent::SessionId::new);
            inject_attachments_into_session(
                state.kernel.as_ref(),
                agent_id,
                sender_context.as_ref(),
                session_override,
                fallback_session_id,
                image_blocks,
            );
        }
    }
    let kernel_handle: Arc<dyn KernelHandle> = state.kernel.clone();
    let (rx, handle) = match state
        .kernel
        .clone()
        .send_message_streaming_with_incognito(
            agent_id,
            &req.message,
            Some(kernel_handle),
            sender_context,
            session_override,
            incognito,
        )
        .await
    {
        Ok(pair) => pair,
        Err(e) => {
            tracing::warn!("Streaming message failed for agent {id}: {e}");
            return ApiErrorResponse::internal(err_streaming_failed)
                .with_code("streaming_failed")
                .into_response();
        }
    };

    // Tie the agent loop's lifetime to the SSE stream — when the client
    // disconnects, axum drops the SSE response future, which drops the
    // unfold state and this guard, aborting the spawned LLM task and
    // releasing per-agent locks immediately (#3464).
    //
    // CRITICAL: the kernel task does substantial post-stream work AFTER
    // the agent loop emits `ContentComplete` — token-reservation settle,
    // canonical session append, JSONL mirror, metering record, audit
    // log, lifecycle bus publish, experiment recording. We MUST disarm
    // the guard the moment we observe `ContentComplete`, otherwise the
    // natural end of the SSE stream (sender drained → caller_rx returns
    // None → unfold ends → guard drops) races against the post-stream
    // cleanup and silently aborts settle/audit/canonical writes,
    // leaking token reservations and dropping the user's last turn from
    // history.
    let abort_guard = AbortOnDrop::new(handle.abort_handle());

    // Defense against the agent loop emitting the same text span twice in a
    // single streaming turn (observed when multi-iteration loops re-assert a
    // final sentence after a tool step). The dedup window is per-request, so
    // legitimate repetitions across turns stay unaffected.
    let sse_stream = stream::unfold(
        (rx, StreamDedup::new(), abort_guard),
        |(mut rx, mut dedup, mut abort_guard)| async move {
            loop {
                let event = rx.recv().await?;
                let sse_event: Result<Event, std::convert::Infallible> = Ok(match event {
                    StreamEvent::TextDelta { text } => {
                        if dedup.is_duplicate(&text) {
                            tracing::debug!(
                                len = text.len(),
                                preview = %text.chars().take(40).collect::<String>(),
                                "stream dedup: dropping duplicate TextDelta",
                            );
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
                    StreamEvent::ContentComplete { usage, .. } => {
                        // The LLM stream is done — every byte the client
                        // cares about has been emitted. Release the abort
                        // permission BEFORE we yield the `done` event so
                        // the kernel task is free to finish settle /
                        // canonical / audit work even if the SSE stream
                        // ends a few milliseconds later (#3464).
                        abort_guard.disarm();
                        Event::default()
                            .event("done")
                            .json_data(serde_json::json!({
                                "done": true,
                                "usage": {
                                    "input_tokens": usage.input_tokens,
                                    "output_tokens": usage.output_tokens,
                                }
                            }))
                            .unwrap_or_else(|_| Event::default().data("error"))
                    }
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
                return Some((sse_event, (rx, dedup, abort_guard)));
            }
        },
    );

    Sse::new(sse_stream)
        .keep_alive(
            axum::response::sse::KeepAlive::new()
                .interval(std::time::Duration::from_secs(15))
                .text("keep-alive"),
        )
        .into_response()
}

// ---------------------------------------------------------------------------
// Delivery tracking endpoints
// ---------------------------------------------------------------------------
/// GET /api/agents/:id/deliveries — List recent delivery receipts for an agent.
#[utoipa::path(
    get,
    path = "/api/agents/{id}/deliveries",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "List recent delivery receipts for an agent", body = crate::types::JsonObject)
    )
)]
pub async fn get_agent_deliveries(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            // Try name lookup
            match state.kernel.agent_registry().find_by_name(&id) {
                Some(entry) => entry.id,
                None => {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
                    );
                }
            }
        }
    };

    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(50)
        .min(500);

    let receipts = state.kernel.delivery().get_receipts(agent_id, limit);
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "agent_id": agent_id.to_string(),
            "count": receipts.len(),
            "receipts": receipts,
        })),
    )
}

// ---------------------------------------------------------------------------
// Mid-turn message injection (#956)
// ---------------------------------------------------------------------------
/// POST /api/agents/:id/inject — Inject a message into a running agent's tool loop.
///
/// If the agent is currently executing tools (mid-turn), the injected message
/// will be processed between tool calls, interrupting the remaining sequence.
/// Returns `{"injected": true}` if accepted, `{"injected": false}` if no
/// active tool loop is running for this agent.
#[utoipa::path(
    post,
    path = "/api/agents/{id}/inject",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body = crate::types::InjectMessageRequest,
    responses(
        (status = 200, description = "Injection result", body = crate::types::InjectMessageResponse),
        (status = 400, description = "Invalid agent ID"),
        (status = 404, description = "Agent not found"),
        (status = 413, description = "Message too large"),
        (status = 503, description = "All injection channels for the agent are full; retry shortly (#3575)")
    )
)]
pub async fn inject_message(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<InjectMessageRequest>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return ApiErrorResponse::bad_request("invalid agent ID").into_response();
        }
    };

    // Reject oversized injection messages
    const MAX_INJECT_SIZE: usize = 16 * 1024; // 16KB
    if req.message.len() > MAX_INJECT_SIZE {
        return ApiErrorResponse::bad_request("injection message too large")
            .with_status(StatusCode::PAYLOAD_TOO_LARGE)
            .into_response();
    }

    // None falls back to a broadcast across every live session for the agent.
    let session_id = match req.session_id.as_deref() {
        Some(s) if !s.is_empty() => match s.parse::<uuid::Uuid>() {
            Ok(u) => Some(librefang_types::agent::SessionId(u)),
            Err(_) => {
                return ApiErrorResponse::bad_request("invalid session_id").into_response();
            }
        },
        _ => None,
    };

    match state
        .kernel
        .inject_message_for_session(agent_id, session_id, &req.message)
        .await
    {
        Ok(injected) => (
            StatusCode::OK,
            Json(serde_json::json!({"injected": injected})),
        )
            .into_response(),
        Err(crate::error::KernelError::Backpressure(msg)) => {
            // Stable machine-readable code so clients can distinguish this
            // from other 503s without substring-matching the message body.
            ApiErrorResponse::internal(msg)
                .with_status(StatusCode::SERVICE_UNAVAILABLE)
                .with_code("backpressure")
                .into_response()
        }
        Err(e) => if e.to_string().contains("not found") {
            ApiErrorResponse::not_found(e.to_string())
        } else {
            // Scrub the catch-all 500 (audit: rusqlite-errors-leak):
            // an inject failure rooted in the memory substrate would
            // otherwise leak SQL detail. Full error logged in scrub.
            ApiErrorResponse::internal_scrub(&e)
        }
        .into_response(),
    }
}

// Push message — proactive outbound messaging via channel adapters
// ---------------------------------------------------------------------------
/// `POST /api/agents/:id/push` — push a proactive outbound message from an
/// agent to a channel recipient (e.g., Telegram chat, Slack channel, email).
///
/// The agent must exist, but the message is sent directly through the channel
/// adapter without going through the agent loop. This is the REST API
/// counterpart of the built-in `channel_send` tool that agents can self-invoke.
#[utoipa::path(
    post,
    path = "/api/agents/{id}/push",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body = crate::types::PushMessageRequest,
    responses(
        (status = 200, description = "Message pushed to channel", body = crate::types::JsonObject),
        (status = 400, description = "Invalid agent ID or missing required fields"),
        (status = 404, description = "Agent not found"),
        (status = 502, description = "Channel adapter rejected the message")
    )
)]
pub async fn push_message(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<crate::types::PushMessageRequest>,
) -> impl IntoResponse {
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
            );
        }
    };

    // Validate agent exists
    if state.kernel.agent_registry().get(agent_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": err_not_found})),
        );
    }

    // Validate request fields
    if req.channel.is_empty() || req.recipient.is_empty() || req.message.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "channel, recipient, and message are required"})),
        );
    }

    // Delegate to the bridge manager if available, otherwise use kernel directly.
    // The ArcSwap guard must not be held across an `.await`, so we load it,
    // clone the Arc, drop the guard, then drive the async call.
    let thread_id = req.thread_id.as_deref();
    let bridge_arc = state.bridge_manager.load_full();
    let result = if let Some(ref bm) = *bridge_arc {
        bm.push_message(&req.channel, &req.recipient, &req.message, thread_id)
            .await
    } else {
        // No bridge manager — fall back to kernel's channel adapter registry
        state
            .kernel
            .send_channel_message(&req.channel, &req.recipient, &req.message, thread_id, None)
            .await
            .map_err(|e| e.to_string())
    };

    match result {
        Ok(detail) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "detail": detail,
                "agent_id": agent_id.to_string(),
            })),
        ),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "success": false,
                "detail": e,
                "agent_id": agent_id.to_string(),
            })),
        ),
    }
}
