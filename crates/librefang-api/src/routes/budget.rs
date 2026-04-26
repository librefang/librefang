//! Budget and usage tracking handlers.

use super::AppState;
use crate::types::ApiErrorResponse;

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
        // RBAC M5: per-user budget endpoints (admin-only, gated in-handler).
        .route("/budget/users", axum::routing::get(user_budget_ranking))
        .route(
            "/budget/users/{user_id}",
            axum::routing::get(user_budget_detail),
        )
}
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use librefang_kernel::auth::UserRole;
use librefang_types::agent::{AgentId, UserId};
use std::collections::HashMap;
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
                "is_hand": e.is_hand,
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
        .budget_status(&state.kernel.budget_config());
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
    // Apply updates — accept both config field names (max_hourly_usd) and
    // GET response field names (hourly_limit) so read-modify-write works.
    state.kernel.update_budget_config(|budget| {
        if let Some(v) = body["max_hourly_usd"]
            .as_f64()
            .or_else(|| body["hourly_limit"].as_f64())
        {
            budget.max_hourly_usd = v;
        }
        if let Some(v) = body["max_daily_usd"]
            .as_f64()
            .or_else(|| body["daily_limit"].as_f64())
        {
            budget.max_daily_usd = v;
        }
        if let Some(v) = body["max_monthly_usd"]
            .as_f64()
            .or_else(|| body["monthly_limit"].as_f64())
        {
            budget.max_monthly_usd = v;
        }
        if let Some(v) = body["alert_threshold"].as_f64() {
            budget.alert_threshold = v.clamp(0.0, 1.0);
        }
        if let Some(v) = body["default_max_llm_tokens_per_hour"].as_u64() {
            budget.default_max_llm_tokens_per_hour = v;
        }
    });

    let status = state
        .kernel
        .metering_ref()
        .budget_status(&state.kernel.budget_config());
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
            return ApiErrorResponse::bad_request("Invalid agent ID").into_response();
        }
    };

    let entry = match state.kernel.agent_registry().get(agent_id) {
        Some(e) => e,
        None => {
            return ApiErrorResponse::not_found("Agent not found").into_response();
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
                "limit": quota.effective_token_limit(),
                "pct": if quota.effective_token_limit() > 0 { tokens_used as f64 / quota.effective_token_limit() as f64 } else { 0.0 },
            },
        })),
    )
        .into_response()
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
                    "max_llm_tokens_per_hour": entry.manifest.resources.effective_token_limit(),
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
            return ApiErrorResponse::bad_request("Invalid agent ID").into_response();
        }
    };

    let hourly = body["max_cost_per_hour_usd"].as_f64();
    let daily = body["max_cost_per_day_usd"].as_f64();
    let monthly = body["max_cost_per_month_usd"].as_f64();
    let tokens = body["max_llm_tokens_per_hour"].as_u64();

    if hourly.is_none() && daily.is_none() && monthly.is_none() && tokens.is_none() {
        return ApiErrorResponse::bad_request(
            "Provide at least one of: max_cost_per_hour_usd, max_cost_per_day_usd, max_cost_per_month_usd, max_llm_tokens_per_hour",
        )
        .into_response();
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
                .into_response()
        }
        Err(e) => ApiErrorResponse::not_found(format!("{e}")).into_response(),
    }
}

// ---------------------------------------------------------------------------
// RBAC M5 — per-user budget endpoints
// ---------------------------------------------------------------------------

/// Reject the request unless the caller is an authenticated `Admin`+.
///
/// Anonymous callers (loopback / `LIBREFANG_ALLOW_NO_AUTH=1`) are denied:
/// per-user spend exposes who-spent-what attribution, which is sensitive
/// enough that we don't blanket-trust an unauthenticated origin even on
/// loopback. To use these endpoints in a no-auth deployment, configure at
/// least one user with an admin api_key.
fn require_admin_for_user_budget(
    state: &AppState,
    api_user: Option<&crate::middleware::AuthenticatedApiUser>,
) -> Option<Response> {
    match api_user {
        Some(u) if u.role >= UserRole::Admin => None,
        Some(u) => {
            state.kernel.audit().record_with_context(
                "system",
                librefang_runtime::audit::AuditAction::PermissionDenied,
                format!("user budget endpoint denied for role {}", u.role),
                "denied",
                Some(u.user_id),
                Some("api".to_string()),
            );
            Some(
                ApiErrorResponse::forbidden("Admin role required for user budget access")
                    .into_response(),
            )
        }
        None => {
            state.kernel.audit().record_with_context(
                "system",
                librefang_runtime::audit::AuditAction::PermissionDenied,
                "user budget endpoint denied for anonymous caller",
                "denied",
                None,
                Some("api".to_string()),
            );
            Some(
                ApiErrorResponse::forbidden(
                    "Authenticated Admin role required for user budget access (configure an admin api_key)",
                )
                .into_response(),
            )
        }
    }
}

/// GET /api/budget/users — admin-only per-user spend ranking.
///
/// Query params:
///   - `limit` (default 25, hard cap 1000) — max rows returned.
///
/// Response: `{ "users": [...], "total": N }`. Each row carries the
/// rolled-up hourly / daily / monthly cost plus the per-user budget
/// limits (when configured) so the dashboard can render % bars without
/// a follow-up call.
#[utoipa::path(
    get,
    path = "/api/budget/users",
    tag = "budget",
    params(("limit" = Option<u32>, Query, description = "Top N users (default 25, cap 1000)")),
    responses((status = 200, description = "Per-user cost ranking", body = serde_json::Value))
)]
pub async fn user_budget_ranking(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
) -> Response {
    let api_user_ref = api_user.as_ref().map(|e| &e.0);
    if let Some(deny) = require_admin_for_user_budget(&state, api_user_ref) {
        return deny;
    }

    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(25)
        .clamp(1, 1000);

    let usage_store = state.kernel.memory_substrate().usage();
    let ranking = match usage_store.query_user_ranking(Some(limit)) {
        Ok(r) => r,
        Err(e) => {
            return ApiErrorResponse::internal(format!("Failed to query user spend: {e}"))
                .into_response();
        }
    };

    // Resolve display names + per-user budgets (when configured) so the
    // dashboard does not need a second round-trip.
    let cfg = state.kernel.config_snapshot();
    let user_budgets: HashMap<String, librefang_types::config::UserBudgetConfig> = cfg
        .users
        .iter()
        .filter_map(|u| {
            u.budget
                .as_ref()
                .map(|b| (UserId::from_name(&u.name).to_string(), b.clone()))
        })
        .collect();
    let user_names: HashMap<String, String> = cfg
        .users
        .iter()
        .map(|u| (UserId::from_name(&u.name).to_string(), u.name.clone()))
        .collect();

    let users: Vec<serde_json::Value> = ranking
        .iter()
        .map(|row| {
            let budget = user_budgets.get(&row.user_id);
            serde_json::json!({
                "user_id": row.user_id,
                "name": user_names.get(&row.user_id),
                "hourly_cost_usd": row.hourly_cost_usd,
                "daily_cost_usd": row.daily_cost_usd,
                "monthly_cost_usd": row.monthly_cost_usd,
                "call_count": row.call_count,
                "limits": budget.map(|b| serde_json::json!({
                    "max_hourly_usd": b.max_hourly_usd,
                    "max_daily_usd": b.max_daily_usd,
                    "max_monthly_usd": b.max_monthly_usd,
                    "alert_threshold": b.alert_threshold,
                })),
            })
        })
        .collect();

    Json(serde_json::json!({
        "users": users,
        "total": users.len(),
        "limit": limit,
    }))
    .into_response()
}

/// GET /api/budget/users/{user_id} — admin-only single-user budget detail.
///
/// `user_id` accepts either a UUID (the canonical `UserId` form) or the
/// raw configured name (re-derived via `UserId::from_name`) so operators
/// can paste a name from `config.toml` directly into the URL.
#[utoipa::path(
    get,
    path = "/api/budget/users/{user_id}",
    tag = "budget",
    params(("user_id" = String, Path, description = "User UUID or configured name")),
    responses((status = 200, description = "Single user budget detail", body = serde_json::Value))
)]
pub async fn user_budget_detail(
    State(state): State<Arc<AppState>>,
    Path(user_id_param): Path<String>,
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
) -> Response {
    let api_user_ref = api_user.as_ref().map(|e| &e.0);
    if let Some(deny) = require_admin_for_user_budget(&state, api_user_ref) {
        return deny;
    }

    // Resolve to a canonical UserId. Try parse-as-uuid first; if that
    // fails fall back to from_name, which always succeeds.
    let user_id: UserId = user_id_param
        .parse()
        .unwrap_or_else(|_| UserId::from_name(&user_id_param));

    let cfg = state.kernel.config_snapshot();
    let user_cfg = cfg
        .users
        .iter()
        .find(|u| UserId::from_name(&u.name) == user_id);
    let display_name = user_cfg.map(|u| u.name.clone());
    let role = user_cfg.map(|u| u.role.clone());
    let budget = user_cfg.and_then(|u| u.budget.clone());

    let usage_store = state.kernel.memory_substrate().usage();
    let hourly = usage_store.query_user_hourly(user_id).unwrap_or(0.0);
    let daily = usage_store.query_user_daily(user_id).unwrap_or(0.0);
    let monthly = usage_store.query_user_monthly(user_id).unwrap_or(0.0);

    // Compute alert breach against the user's configured budget. When no
    // limit is set the percentage is 0 and `alert_breach` is false — the
    // dashboard can still render the spend numbers without budget bars.
    let pct = |spend: f64, limit: f64| -> f64 {
        if limit > 0.0 {
            spend / limit
        } else {
            0.0
        }
    };
    let alert_threshold = budget.as_ref().map(|b| b.alert_threshold).unwrap_or(0.8);
    let limits = budget.as_ref();
    let hourly_pct = limits.map(|b| pct(hourly, b.max_hourly_usd)).unwrap_or(0.0);
    let daily_pct = limits.map(|b| pct(daily, b.max_daily_usd)).unwrap_or(0.0);
    let monthly_pct = limits
        .map(|b| pct(monthly, b.max_monthly_usd))
        .unwrap_or(0.0);
    let alert_breach = hourly_pct >= alert_threshold
        || daily_pct >= alert_threshold
        || monthly_pct >= alert_threshold;

    Json(serde_json::json!({
        "user_id": user_id.to_string(),
        "name": display_name,
        "role": role,
        "hourly": {
            "spend": hourly,
            "limit": limits.map(|b| b.max_hourly_usd).unwrap_or(0.0),
            "pct": hourly_pct,
        },
        "daily": {
            "spend": daily,
            "limit": limits.map(|b| b.max_daily_usd).unwrap_or(0.0),
            "pct": daily_pct,
        },
        "monthly": {
            "spend": monthly,
            "limit": limits.map(|b| b.max_monthly_usd).unwrap_or(0.0),
            "pct": monthly_pct,
        },
        "alert_threshold": alert_threshold,
        "alert_breach": alert_breach,
        // RBAC M5 deferred: per-user budget *enforcement* is not yet wired
        // into the metering pipeline (only global / per-agent / per-provider
        // caps trip a denial). The dashboard MUST surface `enforced=false`
        // so operators don't assume `alert_breach` is also a hard stop.
        // Flip this to `true` once the M5-followup enforcement arm in
        // `kernel/mod.rs::execute_llm_agent` is implemented.
        "enforced": false,
    }))
    .into_response()
}
