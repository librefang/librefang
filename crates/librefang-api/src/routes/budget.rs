//! Budget and usage tracking handlers.

use super::AppState;

/// Build routes for the budget and usage domain.
pub fn router() -> axum::Router<std::sync::Arc<AppState>> {
    axum::Router::new()
        .route("/usage", axum::routing::get(usage_stats))
        .route("/usage/summary", axum::routing::get(usage_summary))
        .route("/usage/by-model", axum::routing::get(usage_by_model))
        .route(
            "/usage/by-model/performance",
            axum::routing::get(usage_by_model_performance),
        )
        .route("/usage/daily", axum::routing::get(usage_daily))
        .route(
            "/budget",
            axum::routing::get(budget_status).put(update_budget),
        )
        .route("/budget/agents", axum::routing::get(agent_budget_ranking))
        .route(
            "/budget/agents/{id}",
            axum::routing::get(agent_budget_status).put(update_agent_budget),
        )
}
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use librefang_types::agent::AgentId;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Usage endpoint
// ---------------------------------------------------------------------------

/// GET /api/usage — Get per-agent usage statistics.
#[utoipa::path(
    get,
    path = "/api/usage",
    tag = "budget",
    responses((status = 200, description = "Per-agent usage statistics", body = serde_json::Value))
)]
pub async fn usage_stats(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let usage_store = state.kernel.memory_substrate().usage();
    let agents: Vec<serde_json::Value> = state
        .kernel
        .agent_registry()
        .list()
        .iter()
        .map(|e| {
            // Read from persistent SQLite store (survives restarts)
            let summary = usage_store.query_summary(Some(e.id)).unwrap_or_default();
            serde_json::json!({
                "agent_id": e.id.to_string(),
                "name": e.name,
                "total_tokens": summary.total_input_tokens + summary.total_output_tokens,
                "input_tokens": summary.total_input_tokens,
                "output_tokens": summary.total_output_tokens,
                "total_cost_usd": summary.total_cost_usd,
                "cost": summary.total_cost_usd,
                "call_count": summary.call_count,
                "tool_calls": summary.total_tool_calls,
            })
        })
        .collect();

    Json(serde_json::json!({"agents": agents}))
}

// ---------------------------------------------------------------------------
// Usage summary endpoints
// ---------------------------------------------------------------------------

/// GET /api/usage/summary — Get overall usage summary from UsageStore.
#[utoipa::path(
    get,
    path = "/api/usage/summary",
    tag = "budget",
    responses((status = 200, description = "Overall usage summary", body = serde_json::Value))
)]
pub async fn usage_summary(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.kernel.memory_substrate().usage().query_summary(None) {
        Ok(s) => Json(serde_json::json!({
            "total_input_tokens": s.total_input_tokens,
            "total_output_tokens": s.total_output_tokens,
            "total_cost_usd": s.total_cost_usd,
            "call_count": s.call_count,
            "total_tool_calls": s.total_tool_calls,
        })),
        Err(_) => Json(serde_json::json!({
            "total_input_tokens": 0,
            "total_output_tokens": 0,
            "total_cost_usd": 0.0,
            "call_count": 0,
            "total_tool_calls": 0,
        })),
    }
}

/// GET /api/usage/by-model — Get usage grouped by model.
#[utoipa::path(
    get,
    path = "/api/usage/by-model",
    tag = "budget",
    responses((status = 200, description = "Usage grouped by model", body = serde_json::Value))
)]
pub async fn usage_by_model(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.kernel.memory_substrate().usage().query_by_model() {
        Ok(models) => {
            let list: Vec<serde_json::Value> = models
                .iter()
                .map(|m| {
                    serde_json::json!({
                        "model": m.model,
                        "total_cost_usd": m.total_cost_usd,
                        "total_input_tokens": m.total_input_tokens,
                        "total_output_tokens": m.total_output_tokens,
                        "call_count": m.call_count,
                    })
                })
                .collect();
            Json(serde_json::json!({"models": list}))
        }
        Err(_) => Json(serde_json::json!({"models": []})),
    }
}

/// GET /api/usage/by-model/performance — Get model performance metrics including latency statistics.
#[utoipa::path(
    get,
    path = "/api/usage/by-model/performance",
    tag = "budget",
    responses((status = 200, description = "Model performance metrics", body = serde_json::Value))
)]
pub async fn usage_by_model_performance(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state
        .kernel
        .memory_substrate()
        .usage()
        .query_model_performance()
    {
        Ok(models) => {
            let list: Vec<serde_json::Value> = models
                .iter()
                .map(|m| {
                    serde_json::json!({
                        "model": m.model,
                        "total_cost_usd": m.total_cost_usd,
                        "total_input_tokens": m.total_input_tokens,
                        "total_output_tokens": m.total_output_tokens,
                        "call_count": m.call_count,
                        "avg_latency_ms": m.avg_latency_ms,
                        "min_latency_ms": m.min_latency_ms,
                        "max_latency_ms": m.max_latency_ms,
                        "cost_per_call": m.cost_per_call,
                        "avg_latency_per_call": m.avg_latency_per_call,
                    })
                })
                .collect();
            Json(serde_json::json!({"models": list}))
        }
        Err(_) => Json(serde_json::json!({"models": []})),
    }
}

/// GET /api/usage/daily — Get daily usage breakdown for the last 7 days.
#[utoipa::path(
    get,
    path = "/api/usage/daily",
    tag = "budget",
    responses((status = 200, description = "Daily usage breakdown", body = serde_json::Value))
)]
pub async fn usage_daily(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let days = state
        .kernel
        .memory_substrate()
        .usage()
        .query_daily_breakdown(7);
    let today_cost = state.kernel.memory_substrate().usage().query_today_cost();
    let first_event = state
        .kernel
        .memory_substrate()
        .usage()
        .query_first_event_date();

    let days_list = match days {
        Ok(d) => d
            .iter()
            .map(|day| {
                serde_json::json!({
                    "date": day.date,
                    "cost_usd": day.cost_usd,
                    "tokens": day.tokens,
                    "calls": day.calls,
                })
            })
            .collect::<Vec<_>>(),
        Err(_) => vec![],
    };

    Json(serde_json::json!({
        "days": days_list,
        "today_cost_usd": today_cost.unwrap_or(0.0),
        "first_event_date": first_event.unwrap_or(None),
    }))
}

// ---------------------------------------------------------------------------
// Budget endpoints
// ---------------------------------------------------------------------------

/// GET /api/budget — Current budget status (limits, spend, % used).
#[utoipa::path(
    get,
    path = "/api/budget",
    tag = "budget",
    responses(
        (status = 200, description = "Global budget status", body = serde_json::Value)
    )
)]
pub async fn budget_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let status = state
        .kernel
        .metering_ref()
        .budget_status(&state.kernel.config_ref().budget);
    Json(serde_json::to_value(&status).unwrap_or_default())
}

/// PUT /api/budget — Update global budget limits (in-memory only, not persisted to config.toml).
#[utoipa::path(
    put,
    path = "/api/budget",
    tag = "budget",
    responses((status = 200, description = "Updated global budget status", body = serde_json::Value))
)]
pub async fn update_budget(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    // SAFETY: Budget config is updated in-place. Since KernelConfig is behind
    // an Arc and we only have &self, we use ptr mutation (same pattern as OFP).
    let config_ptr = state.kernel.config_ref() as *const librefang_types::config::KernelConfig
        as *mut librefang_types::config::KernelConfig;

    // Apply updates — accept both config field names (max_hourly_usd) and
    // GET response field names (hourly_limit) so read-modify-write works.
    unsafe {
        if let Some(v) = body["max_hourly_usd"]
            .as_f64()
            .or_else(|| body["hourly_limit"].as_f64())
        {
            (*config_ptr).budget.max_hourly_usd = v;
        }
        if let Some(v) = body["max_daily_usd"]
            .as_f64()
            .or_else(|| body["daily_limit"].as_f64())
        {
            (*config_ptr).budget.max_daily_usd = v;
        }
        if let Some(v) = body["max_monthly_usd"]
            .as_f64()
            .or_else(|| body["monthly_limit"].as_f64())
        {
            (*config_ptr).budget.max_monthly_usd = v;
        }
        if let Some(v) = body["alert_threshold"].as_f64() {
            (*config_ptr).budget.alert_threshold = v.clamp(0.0, 1.0);
        }
        if let Some(v) = body["default_max_llm_tokens_per_hour"].as_u64() {
            (*config_ptr).budget.default_max_llm_tokens_per_hour = v;
        }
    }

    let status = state
        .kernel
        .metering_ref()
        .budget_status(&state.kernel.config_ref().budget);
    Json(serde_json::to_value(&status).unwrap_or_default())
}

/// GET /api/budget/agents/{id} — Per-agent budget/quota status.
#[utoipa::path(
    get,
    path = "/api/budget/agents/{id}",
    tag = "budget",
    params(("id" = String, Path, description = "Agent ID")),
    responses((status = 200, description = "Per-agent budget and quota status", body = serde_json::Value))
)]
pub async fn agent_budget_status(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            )
        }
    };

    let entry = match state.kernel.agent_registry().get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            )
        }
    };

    let quota = &entry.manifest.resources;
    let usage_store =
        librefang_memory::usage::UsageStore::new(state.kernel.memory_substrate().usage_conn());
    let hourly = usage_store.query_hourly(agent_id).unwrap_or(0.0);
    let daily = usage_store.query_daily(agent_id).unwrap_or(0.0);
    let monthly = usage_store.query_monthly(agent_id).unwrap_or(0.0);

    // Token usage from scheduler
    let token_usage = state.kernel.scheduler_ref().get_usage(agent_id);
    let tokens_used = token_usage.map(|s| s.total_tokens).unwrap_or(0);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "agent_id": agent_id.to_string(),
            "agent_name": entry.name,
            "hourly": {
                "spend": hourly,
                "limit": quota.max_cost_per_hour_usd,
                "pct": if quota.max_cost_per_hour_usd > 0.0 { hourly / quota.max_cost_per_hour_usd } else { 0.0 },
            },
            "daily": {
                "spend": daily,
                "limit": quota.max_cost_per_day_usd,
                "pct": if quota.max_cost_per_day_usd > 0.0 { daily / quota.max_cost_per_day_usd } else { 0.0 },
            },
            "monthly": {
                "spend": monthly,
                "limit": quota.max_cost_per_month_usd,
                "pct": if quota.max_cost_per_month_usd > 0.0 { monthly / quota.max_cost_per_month_usd } else { 0.0 },
            },
            "tokens": {
                "used": tokens_used,
                "limit": quota.max_llm_tokens_per_hour,
                "pct": if quota.max_llm_tokens_per_hour > 0 { tokens_used as f64 / quota.max_llm_tokens_per_hour as f64 } else { 0.0 },
            },
        })),
    )
}

/// GET /api/budget/agents — Per-agent cost ranking (top spenders).
#[utoipa::path(
    get,
    path = "/api/budget/agents",
    tag = "budget",
    responses(
        (status = 200, description = "Per-agent cost ranking", body = Vec<serde_json::Value>)
    )
)]
pub async fn agent_budget_ranking(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let usage_store =
        librefang_memory::usage::UsageStore::new(state.kernel.memory_substrate().usage_conn());
    let agents: Vec<serde_json::Value> = state
        .kernel
        .agent_registry()
        .list()
        .iter()
        .filter_map(|entry| {
            let daily = usage_store.query_daily(entry.id).unwrap_or(0.0);
            if daily > 0.0 {
                Some(serde_json::json!({
                    "agent_id": entry.id.to_string(),
                    "name": entry.name,
                    "daily_cost_usd": daily,
                    "hourly_limit": entry.manifest.resources.max_cost_per_hour_usd,
                    "daily_limit": entry.manifest.resources.max_cost_per_day_usd,
                    "monthly_limit": entry.manifest.resources.max_cost_per_month_usd,
                    "max_llm_tokens_per_hour": entry.manifest.resources.max_llm_tokens_per_hour,
                }))
            } else {
                None
            }
        })
        .collect();

    Json(serde_json::json!({"agents": agents, "total": agents.len()}))
}

/// PUT /api/budget/agents/{id} — Update per-agent budget limits at runtime.
#[utoipa::path(
    put,
    path = "/api/budget/agents/{id}",
    tag = "budget",
    params(("id" = String, Path, description = "Agent ID")),
    responses((status = 200, description = "Updated agent budget", body = serde_json::Value))
)]
pub async fn update_agent_budget(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            )
        }
    };

    let hourly = body["max_cost_per_hour_usd"].as_f64();
    let daily = body["max_cost_per_day_usd"].as_f64();
    let monthly = body["max_cost_per_month_usd"].as_f64();
    let tokens = body["max_llm_tokens_per_hour"].as_u64();

    if hourly.is_none() && daily.is_none() && monthly.is_none() && tokens.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": "Provide at least one of: max_cost_per_hour_usd, max_cost_per_day_usd, max_cost_per_month_usd, max_llm_tokens_per_hour"}),
            ),
        );
    }

    match state
        .kernel
        .agent_registry()
        .update_resources(agent_id, hourly, daily, monthly, tokens)
    {
        Ok(()) => {
            // Persist updated entry
            if let Some(entry) = state.kernel.agent_registry().get(agent_id) {
                if let Err(e) = state.kernel.memory_substrate().save_agent(&entry) {
                    tracing::warn!("Failed to persist agent state: {e}");
                }
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "ok", "message": "Agent budget updated"})),
            )
        }
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}
