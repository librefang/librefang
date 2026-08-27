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
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
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
    if !super::super::can_access_agent(&state, agent_id, api_user.as_ref()) {
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
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> axum::response::Response {
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

    let entry = match state.kernel.agent_registry().get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            )
                .into_response();
        }
    };
    if !super::super::can_access_agent(&state, agent_id, api_user.as_ref()) {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
        )
            .into_response();
    }

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

    let agent_id_str = agent_id.to_string();
    let error_count = match state.kernel.audit().count_agent_errors(&agent_id_str) {
        Ok(count) => count,
        Err(error) => return ApiErrorResponse::internal_scrub(error).into_response(),
    };

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
        .into_response()
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
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Query(params): Query<HashMap<String, String>>,
) -> axum::response::Response {
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

    // Verify the agent exists.
    if !super::super::can_access_agent(&state, agent_id, api_user.as_ref()) {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
        )
            .into_response();
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

    let outcome = (!level_filter.is_empty()).then_some(level_filter.as_str());
    let audit_entries =
        match state
            .kernel
            .audit()
            .recent_for_agent(&agent_id_str, outcome, offset, max_entries)
        {
            Ok(entries) => entries,
            Err(error) => return ApiErrorResponse::internal_scrub(error).into_response(),
        };
    let entries: Vec<serde_json::Value> = audit_entries
        .iter()
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
        .into_response()
}

/// Wire-shape for one ephemeral worker run under a parent agent (refs #7752).
///
/// Mirrors [`librefang_memory::EphemeralRunRow`], defined here as a `utoipa::ToSchema` view so the OpenAPI doc and generated SDKs pick up the shape without forcing utoipa into the memory crate.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct EphemeralRunView {
    pub id: String,
    /// Caller-supplied mission label, before sanitization into a directory name.
    pub label: String,
    /// Uid-style name the worker ran under (`<label>-<8 hex>`).
    pub worker_name: String,
    /// Agent type whose template supplied the worker's persona, when one was named.
    pub agent_type: Option<String>,
    /// The task the parent delegated, clipped to the store's text cap.
    pub task: String,
    /// The worker's answer, clipped to the store's text cap. Empty for a failed run.
    pub response: String,
    /// `completed` or `failed`.
    pub status: String,
    /// Why the run failed, when it did.
    pub error: Option<String>,
    pub provider: String,
    pub model: String,
    pub iterations: i64,
    pub tool_calls: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    /// Cost billed to the parent for this run.
    pub cost_usd: f64,
    pub latency_ms: i64,
    pub started_at: String,
    pub finished_at: String,
}

impl From<librefang_memory::EphemeralRunRow> for EphemeralRunView {
    fn from(r: librefang_memory::EphemeralRunRow) -> Self {
        Self {
            id: r.id,
            label: r.label,
            worker_name: r.worker_name,
            agent_type: r.agent_type,
            task: r.task,
            response: r.response,
            status: r.status,
            error: r.error,
            provider: r.provider,
            model: r.model,
            iterations: r.iterations,
            tool_calls: r.tool_calls,
            input_tokens: r.input_tokens,
            output_tokens: r.output_tokens,
            cost_usd: r.cost_usd,
            latency_ms: r.latency_ms,
            started_at: r.started_at,
            finished_at: r.finished_at,
        }
    }
}

/// Aggregate across the runs retained for one parent.
#[derive(Debug, Clone, Default, serde::Serialize, utoipa::ToSchema)]
pub struct EphemeralRunRollupView {
    pub runs: u64,
    pub failed: u64,
    pub cost_usd: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct EphemeralRunsResponse {
    pub runs: Vec<EphemeralRunView>,
    pub rollup: EphemeralRunRollupView,
}

/// GET /api/agents/{id}/ephemeral-runs — what this agent delegated to ephemeral workers, and what each one cost (refs #7752).
///
/// An ephemeral worker (`agent_spawn` with `ephemeral: true`) runs one turn under its parent's identity and then vanishes — no registry entry, no persisted session, and a mission workspace deleted on the way out.
/// Its spend reached the parent's ledger through `usage_events.billed_agent_id`, but the *work* behind the spend had no record, so an operator watching an agent misbehave through workers had nothing to inspect.
/// This endpoint is that record.
///
/// The rollup covers the same retained rows as `runs`, not all time: the store keeps a bounded number of runs per parent so a path designed to be called cheaply and often cannot grow the table without limit.
#[utoipa::path(
    get,
    path = "/api/agents/{id}/ephemeral-runs",
    tag = "agents",
    params(
        ("id" = String, Path, description = "Parent agent ID"),
        ("limit" = Option<u32>, Query, description = "Max rows (default 30, max 200)"),
    ),
    responses(
        (status = 200, description = "Ephemeral worker runs spawned by this agent", body = EphemeralRunsResponse),
        (status = 400, description = "Invalid agent id"),
        (status = 404, description = "Agent not found")
    )
)]
pub async fn list_agent_ephemeral_runs(
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
    // Same owner-scoping as /stats and /events, and for a stronger reason: a run record carries the delegated task and the worker's answer verbatim, which is conversation content, not just counters.
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
        .min(200) as usize;

    let store = librefang_memory::EphemeralRunStore::new(state.kernel.memory_substrate().pool());
    let parent = agent_uuid.0.to_string();
    let runs = match store.list_for_parent(&parent, limit) {
        Ok(r) => r,
        // `e` carries raw rusqlite error text (column names, constraint identifiers, "database is locked") from the memory layer (audit: rusqlite-errors-leak).
        // Scrub the body; the full chain still lands in `tracing::error!` for ops.
        Err(e) => return ApiErrorResponse::internal_scrub(e).into_response(),
    };
    let rollup = match store.rollup_for_parent(&parent) {
        Ok(r) => EphemeralRunRollupView {
            runs: r.runs,
            failed: r.failed,
            cost_usd: r.cost_usd,
            input_tokens: r.input_tokens,
            output_tokens: r.output_tokens,
        },
        Err(e) => return ApiErrorResponse::internal_scrub(e).into_response(),
    };

    Json(EphemeralRunsResponse {
        runs: runs.into_iter().map(EphemeralRunView::from).collect(),
        rollup,
    })
    .into_response()
}
