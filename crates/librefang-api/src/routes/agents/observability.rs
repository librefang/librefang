use super::*;

/// 24-hour KPI rollup view returned by `GET /api/agents/{id}/stats`.
/// Mirrors [`librefang_memory::session::AgentStats24h`] — defined here as a
/// view so we can derive `utoipa::ToSchema` without forcing utoipa into the
/// memory crate. Generated SDKs and the OpenAPI spec pick up this shape.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct AgentStats24hView {
    pub sessions_24h: u64,
    pub cost_24h: f64,
    pub p95_latency_ms: u64,
    pub active_now: u64,
    pub samples: u64,
    pub prev: AgentStatsPrevView,
}

/// Prior 24-48h window scoped fields backing the KPI tile trend deltas.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct AgentStatsPrevView {
    pub sessions_24h: u64,
    pub cost_24h: f64,
    pub p95_latency_ms: u64,
}

impl From<librefang_memory::session::AgentStats24h> for AgentStats24hView {
    fn from(s: librefang_memory::session::AgentStats24h) -> Self {
        Self {
            sessions_24h: s.sessions_24h,
            cost_24h: s.cost_24h,
            p95_latency_ms: s.p95_latency_ms,
            active_now: s.active_now,
            samples: s.samples,
            prev: AgentStatsPrevView {
                sessions_24h: s.prev.sessions_24h,
                cost_24h: s.prev.cost_24h,
                p95_latency_ms: s.prev.p95_latency_ms,
            },
        }
    }
}

/// GET /api/agents/{id}/stats — 24-hour KPI rollup for one agent.
///
/// Returns sessions/cost/P95-latency/active-now in a single round trip so
/// the dashboard's per-agent KPI tiles don't have to scan the global
/// `/api/sessions` page (which is paginated and was clipping data for
/// agents that hadn't appeared in the latest N sessions).
#[utoipa::path(
    get,
    path = "/api/agents/{id}/stats",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "24-hour stats rollup", body = AgentStats24hView),
        (status = 404, description = "Agent not found")
    )
)]
pub async fn get_agent_stats(
    State(state): State<Arc<AppState>>,
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_uuid = match uuid::Uuid::parse_str(&id) {
        Ok(u) => librefang_types::agent::AgentId(u),
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "invalid agent id" })),
            )
                .into_response();
        }
    };
    let entry = match state.kernel.agent_registry().get(agent_uuid) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "agent not found" })),
            )
                .into_response();
        }
    };

    // Owner-scoping: non-admin callers can only read stats for agents
    // they authored. Mirrors the filter applied in `list_agents` so the
    // detail-panel rollup can't leak per-agent cost / latency to other
    // users on the same instance.
    if let Some(ref user) = api_user {
        use crate::middleware::UserRole;
        if user.0.role < UserRole::Admin
            && !entry.manifest.author.eq_ignore_ascii_case(&user.0.name)
        {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "agent not found" })),
            )
                .into_response();
        }
    }

    let substrate = state.kernel.memory_substrate();
    match substrate.agent_stats_24h(&id) {
        Ok(stats) => Json(AgentStats24hView::from(stats)).into_response(),
        // `e` carries raw rusqlite error messages (column names,
        // constraint identifiers, "database is locked") from the
        // memory layer (audit: rusqlite-errors-leak). Scrub the
        // body before sending to the client; the full chain still
        // lands in `tracing::error!` for ops.
        Err(e) => ApiErrorResponse::internal_scrub(e).into_response(),
    }
}

/// Wire-shape for one row in [`list_agent_events`]. Mirrors
/// [`librefang_memory::usage::AgentEventRow`] but defined here as a
/// utoipa::ToSchema view so we can register it with the OpenAPI doc
/// without forcing utoipa into the memory crate.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct AgentEventRowView {
    pub timestamp: String,
    pub model: String,
    pub provider: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub tool_calls: u64,
    pub latency_ms: u64,
}

impl From<librefang_memory::usage::AgentEventRow> for AgentEventRowView {
    fn from(r: librefang_memory::usage::AgentEventRow) -> Self {
        Self {
            timestamp: r.timestamp,
            model: r.model,
            provider: r.provider,
            input_tokens: r.input_tokens,
            output_tokens: r.output_tokens,
            cost_usd: r.cost_usd,
            tool_calls: r.tool_calls,
            latency_ms: r.latency_ms,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct AgentEventsResponse {
    pub events: Vec<AgentEventRowView>,
}

/// GET /api/agents/{id}/events — Recent turn-level events for one agent.
///
/// Backs the dashboard's agent-detail Logs tab. Returns rows sourced
/// from `usage_events` (newest first) so the panel shows real
/// operational data — model dispatch, latency, tokens, cost — instead
/// of the audit ledger, which is mostly admin lifecycle entries.
#[utoipa::path(
    get,
    path = "/api/agents/{id}/events",
    tag = "agents",
    params(
        ("id" = String, Path, description = "Agent ID"),
        ("limit" = Option<u32>, Query, description = "Max rows (default 30, max 200)"),
    ),
    responses(
        (status = 200, description = "Recent agent events", body = AgentEventsResponse),
        (status = 404, description = "Agent not found")
    )
)]
pub async fn list_agent_events(
    State(state): State<Arc<AppState>>,
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
    Path(id): Path<String>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let agent_uuid = match uuid::Uuid::parse_str(&id) {
        Ok(u) => librefang_types::agent::AgentId(u),
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "invalid agent id" })),
            )
                .into_response();
        }
    };
    let entry = match state.kernel.agent_registry().get(agent_uuid) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "agent not found" })),
            )
                .into_response();
        }
    };
    // Mirror the owner-scoping on /stats and /sessions — turn-level
    // event data carries token counts and cost, so it shouldn't leak.
    if let Some(ref user) = api_user {
        use crate::middleware::UserRole;
        if user.0.role < UserRole::Admin
            && !entry.manifest.author.eq_ignore_ascii_case(&user.0.name)
        {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "agent not found" })),
            )
                .into_response();
        }
    }

    let limit = params
        .get("limit")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(30)
        .min(200);

    let substrate = state.kernel.memory_substrate();
    match substrate
        .usage()
        .list_agent_events_recent(agent_uuid, limit)
    {
        Ok(events) => {
            let view = AgentEventsResponse {
                events: events.into_iter().map(AgentEventRowView::from).collect(),
            };
            Json(view).into_response()
        }
        // `e` carries raw rusqlite error messages (column names,
        // constraint identifiers, "database is locked") from the
        // memory layer (audit: rusqlite-errors-leak). Scrub the
        // body before sending to the client; the full chain still
        // lands in `tracing::error!` for ops.
        Err(e) => ApiErrorResponse::internal_scrub(e).into_response(),
    }
}

/// GET /api/agents/{id}/traces — Get decision traces from the agent's most recent message.
///
/// Returns structured traces showing why each tool was selected during the last
/// agent loop execution. Useful for debugging, auditing, and optimization.
#[utoipa::path(
    get,
    path = "/api/agents/{id}/traces",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "Get decision traces from the agent's most recent message", body = crate::types::JsonObject)
    )
)]
pub async fn get_agent_traces(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
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

    // Check agent exists
    if state.kernel.agent_registry().get(agent_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
        );
    }

    let traces = state
        .kernel
        .traces()
        .get(&agent_id)
        .map(|entry| entry.value().clone())
        .unwrap_or_default();

    (
        StatusCode::OK,
        Json(serde_json::json!({ "traces": traces })),
    )
}

// ---------------------------------------------------------------------------
// Agent monitoring and profiling endpoints (#181)
// ---------------------------------------------------------------------------

/// GET /api/agents/{id}/metrics — Returns aggregated metrics for an agent.
///
/// Includes message count, token usage, tool execution count, error count,
/// average response time (estimated), and cost data.
#[utoipa::path(
    get,
    path = "/api/agents/{id}/metrics",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "Aggregated agent metrics", body = crate::types::JsonObject),
        (status = 400, description = "Invalid agent ID"),
        (status = 404, description = "Agent not found")
    )
)]
pub async fn agent_metrics(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            );
        }
    };

    let entry = match state.kernel.agent_registry().get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            );
        }
    };

    // Session-level token/tool stats from the scheduler (in-memory, windowed).
    let sched_snap = state
        .kernel
        .scheduler_ref()
        .get_usage(agent_id)
        .unwrap_or_default();
    let (sched_tokens, sched_tool_calls) = (sched_snap.total_tokens, sched_snap.tool_calls);

    // Persistent usage summary from the UsageStore (SQLite).
    let usage_summary = state
        .kernel
        .memory_substrate()
        .usage()
        .query_summary(Some(agent_id))
        .ok();

    // Message count from the active session.
    let message_count: u64 = state
        .kernel
        .memory_substrate()
        .get_session(entry.session_id)
        .ok()
        .flatten()
        .map(|s| s.messages.len() as u64)
        .unwrap_or(0);

    // Error count from the audit log (count entries with non-"ok" outcome for this agent).
    // NOTE: This scans the most recent 100k audit entries. Agents with errors beyond
    // this window will have under-reported error counts. A dedicated per-agent error
    // counter or index would eliminate this limitation.
    let agent_id_str = agent_id.to_string();
    let error_count: u64 = state
        .kernel
        .audit()
        .recent(100_000)
        .iter()
        .filter(|e| e.agent_id == agent_id_str && e.outcome != "ok" && e.outcome != "success")
        .count() as u64;

    // Uptime since the agent was created.
    let uptime_secs = (chrono::Utc::now() - entry.created_at).num_seconds().max(0) as u64;

    // Persistent usage values (fall back to scheduler data when no DB records exist).
    let (total_input_tokens, total_output_tokens, total_cost_usd, call_count, total_tool_calls) =
        match usage_summary {
            Some(ref s) => (
                s.total_input_tokens,
                s.total_output_tokens,
                s.total_cost_usd,
                s.call_count,
                s.total_tool_calls,
            ),
            None => (0, 0, 0.0, 0, 0),
        };

    // Average response time is not tracked yet; keep the field stable until
    // per-call timing is persisted in UsageStore.
    let avg_response_time_ms: Option<f64> = None;

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "agent_id": agent_id.to_string(),
            "name": entry.name,
            "state": format!("{:?}", entry.state),
            "uptime_secs": uptime_secs,
            "message_count": message_count,
            "token_usage": {
                "session_tokens": sched_tokens,
                "total_input_tokens": total_input_tokens,
                "total_output_tokens": total_output_tokens,
                "total_tokens": total_input_tokens + total_output_tokens,
            },
            "tool_calls": {
                "session_tool_calls": sched_tool_calls,
                "total_tool_calls": total_tool_calls,
            },
            "cost_usd": total_cost_usd,
            "call_count": call_count,
            "error_count": error_count,
            "avg_response_time_ms": avg_response_time_ms,
        })),
    )
}

/// GET /api/agents/{id}/logs — Returns structured execution logs for an agent.
///
/// Supports optional query parameters:
/// - `n`: max number of log entries (default 100, max 1000)
/// - `level`: filter by outcome (e.g. "error", "ok")
/// - `offset`: number of matching entries to skip for pagination (default 0)
#[utoipa::path(
    get,
    path = "/api/agents/{id}/logs",
    tag = "agents",
    params(
        ("id" = String, Path, description = "Agent ID"),
        ("n" = Option<usize>, Query, description = "Max entries to return (default 100, max 1000)"),
        ("level" = Option<String>, Query, description = "Filter by audit outcome (e.g. \"error\", \"ok\")"),
        ("offset" = Option<usize>, Query, description = "Pagination offset over filtered entries")
    ),
    responses(
        (status = 200, description = "Recent agent execution log entries", body = crate::types::JsonObject),
        (status = 400, description = "Invalid agent ID"),
        (status = 404, description = "Agent not found")
    )
)]
pub async fn agent_logs(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            );
        }
    };

    // Verify the agent exists.
    if state.kernel.agent_registry().get(agent_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
        );
    }

    let max_entries: usize = params
        .get("n")
        .and_then(|v| v.parse().ok())
        .unwrap_or(100)
        .min(1000);

    let offset: usize = params
        .get("offset")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    let level_filter = params
        .get("level")
        .cloned()
        .unwrap_or_default()
        .to_lowercase();

    let agent_id_str = agent_id.to_string();

    // Filter audit log entries belonging to this agent.
    let entries: Vec<serde_json::Value> = state
        .kernel
        .audit()
        .recent(100_000)
        .iter()
        .filter(|e| e.agent_id == agent_id_str)
        .filter(|e| {
            if level_filter.is_empty() {
                return true;
            }
            e.outcome.eq_ignore_ascii_case(&level_filter)
        })
        .skip(offset)
        .take(max_entries)
        .map(|e| {
            serde_json::json!({
                "seq": e.seq,
                "timestamp": e.timestamp,
                "action": format!("{:?}", e.action),
                "detail": e.detail,
                "outcome": e.outcome,
            })
        })
        .collect();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "agent_id": agent_id_str,
            "count": entries.len(),
            "offset": offset,
            "logs": entries,
        })),
    )
}
