//! Webhook subscription management AND external-trigger endpoints
//! (`/api/webhooks*`, `/api/hooks/wake`, `/api/hooks/agent`).
//!
//! Three distinct flavors live here:
//!
//! * **Event webhooks** (`/api/webhooks/events`) — in-memory subscriptions to
//!   internal system events (`agent.spawned`, `message.received`, …). These
//!   are intentionally non-persistent (#185); a future iteration should move
//!   them onto the same persistent store as outbound webhooks.
//! * **Outbound webhooks** (`/api/webhooks`) — file-persisted subscriptions
//!   backed by `crate::webhook_store`. Includes the `POST /test` fire-time
//!   handler that re-validates the destination URL against SSRF rules
//!   (#3701) before sending a signed test payload.
//! * **External triggers** (`/api/hooks/wake`, `/api/hooks/agent`) — public
//!   webhook endpoints that inject events / messages into the kernel from
//!   outside (CI/CD, Slack, etc.). Bearer-token authenticated via the
//!   `[webhook_triggers]` config block. Moved from `system.rs` per #3749 11/N.

use super::AppState;
use crate::middleware::RequestLanguage;
use crate::types::ApiErrorResponse;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use librefang_kernel::kernel_handle::prelude::*;
use librefang_types::agent::AgentId;
use librefang_types::i18n::ErrorTranslator;
use std::collections::HashMap;
use std::sync::Arc;

/// Build webhook subscription routes.
pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        // Event webhook subscriptions
        .route(
            "/webhooks/events",
            axum::routing::get(list_event_webhooks).post(create_event_webhook),
        )
        .route(
            "/webhooks/events/{id}",
            axum::routing::put(update_event_webhook).delete(delete_event_webhook),
        )
        // Outbound webhook management
        .route(
            "/webhooks",
            axum::routing::get(list_webhooks).post(create_webhook),
        )
        .route(
            "/webhooks/{id}",
            axum::routing::get(get_webhook)
                .put(update_webhook)
                .delete(delete_webhook),
        )
        .route("/webhooks/{id}/test", axum::routing::post(test_webhook))
        // External-trigger webhook endpoints (#3749 11/N: moved from system.rs).
        .route("/hooks/wake", axum::routing::post(webhook_wake))
        .route("/hooks/agent", axum::routing::post(webhook_agent))
}

// ---------------------------------------------------------------------------
// External-trigger webhook endpoints (`/api/hooks/wake`, `/api/hooks/agent`)
// ---------------------------------------------------------------------------

/// POST /hooks/wake — Inject a system event via webhook trigger.
///
/// Publishes a custom event through the kernel's event system, which can
/// trigger proactive agents that subscribe to the event type.
///
/// Auth (#3509): missing or invalid bearer token returns `401 Unauthorized`
/// with a `WWW-Authenticate: Bearer realm="librefang-webhook"` header per
/// RFC 9110 §11.6.1. The previous behaviour (400 Bad Request) confused
/// clients that tried to retry with a fixed body instead of fixing the
/// token.
#[utoipa::path(
    post,
    path = "/api/hooks/wake",
    tag = "webhooks",
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Wake hook triggered", body = crate::types::JsonObject),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 404, description = "Webhook triggers not enabled")
    )
)]
pub async fn webhook_wake(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(body): Json<librefang_types::webhook::WakePayload>,
) -> axum::response::Response {
    let (err_webhook_not_enabled, err_invalid_token) = {
        let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
        (
            t.t("api-error-webhook-triggers-not-enabled"),
            t.t("api-error-webhook-invalid-token"),
        )
    };
    // Check if webhook triggers are enabled — use config_snapshot()
    // because wh_config is held across .await below.
    let cfg = state.kernel.config_snapshot();
    let wh_config = match &cfg.webhook_triggers {
        Some(c) if c.enabled => c,
        _ => {
            return ApiErrorResponse::not_found(err_webhook_not_enabled).into_response();
        }
    };

    // Validate bearer token (constant-time comparison). Invalid token is
    // an authentication failure, not a malformed request — return 401 with
    // the standard `WWW-Authenticate` challenge per RFC 9110 §11.6.1
    // (#3509).
    if !validate_webhook_token(&headers, &wh_config.token_env) {
        return webhook_unauthorized_response(err_invalid_token);
    }

    // Validate payload
    if let Err(e) = body.validate() {
        return ApiErrorResponse::bad_request(e).into_response();
    }

    // Publish through the kernel's publish_event (KernelHandle trait), which
    // goes through the full event processing pipeline including trigger evaluation.
    let event_payload = serde_json::json!({
        "source": "webhook",
        "mode": body.mode,
        "text": body.text,
    });
    if let Err(e) =
        EventBus::publish_event(state.kernel.as_ref(), "webhook.wake", event_payload).await
    {
        tracing::warn!("Webhook wake event publish failed: {e}");
        let err_msg = {
            let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
            t.t_args(
                "api-error-webhook-publish-failed",
                &[("error", &e.to_string())],
            )
        };
        return ApiErrorResponse::internal(err_msg).into_response();
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "accepted", "mode": body.mode})),
    )
        .into_response()
}

/// Build a `401 Unauthorized` response with the standard
/// `WWW-Authenticate: Bearer realm="librefang-webhook"` challenge header
/// (RFC 9110 §11.6.1). Used by webhook trigger endpoints whose bearer-token
/// check failed (#3509).
fn webhook_unauthorized_response(message: String) -> axum::response::Response {
    let body = ApiErrorResponse {
        error: message,
        code: Some("webhook_invalid_token".to_string()),
        r#type: Some("webhook_invalid_token".to_string()),
        details: None,
        request_id: None,
        status: StatusCode::UNAUTHORIZED,
    };
    let mut resp = body.into_response();
    resp.headers_mut().insert(
        axum::http::header::WWW_AUTHENTICATE,
        axum::http::HeaderValue::from_static("Bearer realm=\"librefang-webhook\""),
    );
    resp
}

/// POST /hooks/agent — Run an isolated agent turn via webhook.
///
/// Sends a message directly to the specified agent and returns the response.
/// This enables external systems (CI/CD, Slack, etc.) to trigger agent work.
///
/// Auth (#3509): missing or invalid bearer token returns `401 Unauthorized`
/// with a `WWW-Authenticate: Bearer realm="librefang-webhook"` header per
/// RFC 9110 §11.6.1, mirroring the `/hooks/wake` fix.
#[utoipa::path(
    post,
    path = "/api/hooks/agent",
    tag = "webhooks",
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Agent hook triggered", body = crate::types::JsonObject),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 404, description = "Webhook triggers not enabled or agent not found")
    )
)]
pub async fn webhook_agent(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(body): Json<librefang_types::webhook::AgentHookPayload>,
) -> axum::response::Response {
    let (err_webhook_not_enabled, err_invalid_token, err_no_agents) = {
        let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
        (
            t.t("api-error-webhook-triggers-not-enabled"),
            t.t("api-error-webhook-invalid-token"),
            t.t("api-error-webhook-no-agents"),
        )
    };
    // Check if webhook triggers are enabled — use config_snapshot()
    // because wh_config is held across .await below.
    let cfg2 = state.kernel.config_snapshot();
    let wh_config = match &cfg2.webhook_triggers {
        Some(c) if c.enabled => c,
        _ => {
            return ApiErrorResponse::not_found(err_webhook_not_enabled).into_response();
        }
    };

    // Validate bearer token (#3509: 401 + WWW-Authenticate, not 400).
    if !validate_webhook_token(&headers, &wh_config.token_env) {
        return webhook_unauthorized_response(err_invalid_token);
    }

    // Validate payload
    if let Err(e) = body.validate() {
        return ApiErrorResponse::bad_request(e).into_response();
    }

    // Resolve the agent by name or ID (if not specified, use the first running agent)
    let agent_id: AgentId = match &body.agent {
        Some(agent_ref) => match agent_ref.parse() {
            Ok(id) => id,
            Err(_) => {
                // Try name lookup
                match state.kernel.agent_registry().find_by_name(agent_ref) {
                    Some(entry) => entry.id,
                    None => {
                        let err_msg = {
                            let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
                            t.t_args("api-error-webhook-agent-not-found", &[("id", agent_ref)])
                        };
                        return ApiErrorResponse::not_found(err_msg).into_response();
                    }
                }
            }
        },
        None => {
            // No agent specified — use the first available agent. Read-only
            // peek at the id, so use cheap Arc clones (#3569).
            match state.kernel.agent_registry().list_arcs().first() {
                Some(entry) => entry.id,
                None => {
                    return ApiErrorResponse::not_found(err_no_agents).into_response();
                }
            }
        }
    };

    // Actually send the message to the agent and get the response
    match state.kernel.send_message(agent_id, &body.message).await {
        Ok(result) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "completed",
                "agent_id": agent_id.to_string(),
                "response": result.response,
                "usage": {
                    "input_tokens": result.total_usage.input_tokens,
                    "output_tokens": result.total_usage.output_tokens,
                },
            })),
        )
            .into_response(),
        Err(e) => {
            let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
            let msg = t.t_args(
                "api-error-webhook-agent-exec-failed",
                &[("error", &e.to_string())],
            );
            ApiErrorResponse::internal(msg).into_response()
        }
    }
}

/// Constant-time bearer-token check for webhook trigger endpoints.
///
/// The expected token is read from the env var named in the
/// `[webhook_triggers]` config block (so secrets never live in
/// `config.toml`); we require >= 32 bytes to avoid trivial brute-forcing.
/// Comparison uses `subtle::ConstantTimeEq` after a length pre-check so a
/// per-byte timing leak cannot reveal the expected token's contents.
fn validate_webhook_token(headers: &axum::http::HeaderMap, token_env: &str) -> bool {
    let expected = match std::env::var(token_env) {
        Ok(t) if t.len() >= 32 => t,
        _ => return false,
    };

    let provided = match headers.get("authorization") {
        Some(v) => match v.to_str() {
            Ok(s) if s.starts_with("Bearer ") => &s[7..],
            _ => return false,
        },
        None => return false,
    };

    use subtle::ConstantTimeEq;
    if provided.len() != expected.len() {
        return false;
    }
    provided.as_bytes().ct_eq(expected.as_bytes()).into()
}

// ---------------------------------------------------------------------------
// Event Webhooks — subscribe to system events via HTTP callbacks (#185)
// ---------------------------------------------------------------------------

/// Supported event types for webhook subscriptions.
static VALID_EVENT_TYPES: &[&str] = &[
    "agent.spawned",
    "agent.terminated",
    "agent.error",
    "message.received",
    "workflow.completed",
    "workflow.failed",
];

/// In-memory store for event webhook subscriptions.
///
/// NOTE: subscriptions are lost on daemon restart. A future iteration should
/// persist these to the config/data directory.
static EVENT_WEBHOOKS: std::sync::LazyLock<
    tokio::sync::RwLock<HashMap<String, serde_json::Value>>,
> = std::sync::LazyLock::new(|| tokio::sync::RwLock::new(HashMap::new()));

/// Validate an events JSON array against VALID_EVENT_TYPES.
fn validate_event_types(
    arr: &[serde_json::Value],
    lang: Option<&axum::Extension<RequestLanguage>>,
) -> Result<Vec<String>, (StatusCode, Json<serde_json::Value>)> {
    let t = ErrorTranslator::new(super::resolve_lang(lang));
    let mut event_list = Vec::new();
    for ev in arr {
        match ev.as_str() {
            Some(s) if VALID_EVENT_TYPES.contains(&s) => {
                event_list.push(s.to_string());
            }
            Some(s) => {
                let valid_str = format!("{VALID_EVENT_TYPES:?}");
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": t.t_args("api-error-webhook-unknown-event", &[("event", s), ("valid", &valid_str)])
                    })),
                ));
            }
            None => {
                return Err(ApiErrorResponse::bad_request(
                    t.t("api-error-webhook-event-not-string"),
                )
                .into_json_tuple());
            }
        }
    }
    if event_list.is_empty() {
        return Err(
            ApiErrorResponse::bad_request(t.t("api-error-webhook-events-empty")).into_json_tuple(),
        );
    }
    Ok(event_list)
}

/// Redact the secret field from a webhook JSON value before returning it.
fn redact_webhook_secret(webhook: &serde_json::Value) -> serde_json::Value {
    let mut w = webhook.clone();
    if let Some(obj) = w.as_object_mut() {
        if obj.contains_key("secret") {
            obj.insert("secret".to_string(), serde_json::json!("***"));
        }
    }
    w
}

/// GET /api/webhooks/events — List all event webhook subscriptions.
pub async fn list_event_webhooks() -> impl IntoResponse {
    let store = EVENT_WEBHOOKS.read().await;
    let list: Vec<serde_json::Value> = store.values().map(redact_webhook_secret).collect();
    Json(list)
}

/// POST /api/webhooks/events — Create a new event webhook subscription.
pub async fn create_event_webhook(
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    // Pre-translate error messages before .await to avoid holding !Send ErrorTranslator across await
    let (err_missing_url, err_invalid_url, err_missing_events) = {
        let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
        (
            t.t("api-error-webhook-missing-url"),
            t.t("api-error-webhook-invalid-url"),
            t.t("api-error-webhook-missing-events"),
        )
    };

    let url = match req["url"].as_str() {
        Some(u) if !u.is_empty() => u.to_string(),
        _ => {
            return ApiErrorResponse::bad_request(err_missing_url).into_json_tuple();
        }
    };

    if url::Url::parse(&url).is_err() {
        return ApiErrorResponse::bad_request(err_invalid_url).into_json_tuple();
    }

    let events = match req.get("events").and_then(|v| v.as_array()) {
        Some(arr) => match validate_event_types(arr, lang.as_ref()) {
            Ok(ev) => ev,
            Err(e) => return e,
        },
        None => {
            return ApiErrorResponse::bad_request(err_missing_events).into_json_tuple();
        }
    };

    let secret = req["secret"].as_str().map(|s| s.to_string());
    let enabled = req["enabled"].as_bool().unwrap_or(true);
    let id = uuid::Uuid::new_v4().to_string();

    let webhook = serde_json::json!({
        "id": id,
        "url": url,
        "events": events,
        "secret": secret,
        "enabled": enabled,
        "created_at": chrono::Utc::now().to_rfc3339(),
    });

    EVENT_WEBHOOKS
        .write()
        .await
        .insert(id.clone(), webhook.clone());

    (StatusCode::CREATED, Json(redact_webhook_secret(&webhook)))
}

/// PUT /api/webhooks/events/{id} — Update an event webhook subscription.
pub async fn update_event_webhook(
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let (err_webhook_not_found, err_invalid_url) = {
        let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
        (
            t.t("api-error-webhook-not-found"),
            t.t("api-error-webhook-invalid-url"),
        )
    };
    let mut store = EVENT_WEBHOOKS.write().await;
    let existing = match store.get(&id) {
        Some(w) => w.clone(),
        None => {
            return ApiErrorResponse::not_found(err_webhook_not_found).into_json_tuple();
        }
    };

    let mut updated = existing;

    if let Some(url_val) = req.get("url").and_then(|v| v.as_str()) {
        if url::Url::parse(url_val).is_err() {
            return ApiErrorResponse::bad_request(err_invalid_url).into_json_tuple();
        }
        updated["url"] = serde_json::json!(url_val);
    }

    if let Some(arr) = req.get("events").and_then(|v| v.as_array()) {
        match validate_event_types(arr, lang.as_ref()) {
            Ok(ev) => updated["events"] = serde_json::json!(ev),
            Err(e) => return e,
        }
    }

    if let Some(enabled) = req.get("enabled").and_then(|v| v.as_bool()) {
        updated["enabled"] = serde_json::json!(enabled);
    }

    if let Some(secret) = req.get("secret") {
        updated["secret"] = secret.clone();
    }

    store.insert(id, updated.clone());

    (StatusCode::OK, Json(redact_webhook_secret(&updated)))
}

/// DELETE /api/webhooks/events/{id} — Remove an event webhook subscription.
pub async fn delete_event_webhook(
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let err_webhook_not_found = {
        let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
        t.t("api-error-webhook-not-found")
    };
    let mut store = EVENT_WEBHOOKS.write().await;
    if store.remove(&id).is_some() {
        (StatusCode::NO_CONTENT, Json(serde_json::json!(null)))
    } else {
        ApiErrorResponse::not_found(err_webhook_not_found).into_json_tuple()
    }
}

// ---------------------------------------------------------------------------
// Outbound webhook management endpoints (file-persisted subscriptions)
// ---------------------------------------------------------------------------

/// GET /api/webhooks — List all webhook subscriptions (secrets redacted).
pub async fn list_webhooks(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let webhooks: Vec<_> = state
        .webhook_store
        .list()
        .iter()
        .map(crate::webhook_store::redact_webhook_secret)
        .collect();
    let total = webhooks.len();
    (
        StatusCode::OK,
        Json(serde_json::json!({"webhooks": webhooks, "total": total})),
    )
}

/// GET /api/webhooks/{id} — Get a single webhook subscription (secret redacted).
pub async fn get_webhook(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let wh_id = match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => crate::webhook_store::WebhookId(uuid),
        Err(_) => {
            return ApiErrorResponse::bad_request(t.t("api-error-webhook-invalid-id"))
                .into_json_tuple();
        }
    };
    match state.webhook_store.get(wh_id) {
        Some(wh) => {
            let redacted = crate::webhook_store::redact_webhook_secret(&wh);
            match serde_json::to_value(&redacted) {
                Ok(v) => (StatusCode::OK, Json(v)),
                Err(_) => ApiErrorResponse::internal(t.t("api-error-webhook-serialize-error"))
                    .into_json_tuple(),
            }
        }
        None => ApiErrorResponse::not_found(t.t("api-error-webhook-not-found")).into_json_tuple(),
    }
}

/// POST /api/webhooks — Create a new webhook subscription.
///
/// Honours `Idempotency-Key` (#3637): when set, a duplicate request
/// with the same key + same body replays the cached response instead
/// of creating a second subscription. A different body under the same
/// key is rejected with 409 Conflict.
pub async fn create_webhook(
    State(state): State<Arc<AppState>>,
    lang: Option<axum::Extension<RequestLanguage>>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> axum::response::Response {
    let key = crate::idempotency::extract_key(&headers);
    let body_bytes: Vec<u8> = body.to_vec();
    let store = Arc::clone(&state.idempotency_store);
    let inner_body = body_bytes.clone();
    let l = super::resolve_lang(lang.as_ref());

    crate::idempotency::run_idempotent(
        store.as_ref(),
        key.as_deref(),
        &body_bytes,
        move || async move { create_webhook_inner(state, l, &inner_body).await },
    )
    .await
}

/// Inner handler — produces a `(StatusCode, Vec<u8>)` snapshot suitable
/// for caching by the Idempotency-Key middleware.
async fn create_webhook_inner(
    state: Arc<AppState>,
    l: &'static str,
    body_bytes: &[u8],
) -> (StatusCode, Vec<u8>) {
    let t = ErrorTranslator::new(l);
    let req: crate::webhook_store::CreateWebhookRequest = match serde_json::from_slice(body_bytes) {
        Ok(r) => r,
        Err(e) => {
            let payload = serde_json::json!({"error": format!("Invalid JSON body: {e}"), "code": "invalid_json", "type": "invalid_json"});
            return (
                StatusCode::BAD_REQUEST,
                serde_json::to_vec(&payload).unwrap_or_default(),
            );
        }
    };
    match state.webhook_store.create(req) {
        Ok(webhook) => {
            let redacted = crate::webhook_store::redact_webhook_secret(&webhook);
            match serde_json::to_vec(&redacted) {
                Ok(v) => (StatusCode::CREATED, v),
                Err(_) => {
                    let payload = serde_json::json!({"error": t.t("api-error-webhook-serialize-error"), "code": "serialize_error", "type": "serialize_error"});
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        serde_json::to_vec(&payload).unwrap_or_default(),
                    )
                }
            }
        }
        Err(e) => {
            let payload = serde_json::json!({"error": e, "code": "invalid_request", "type": "invalid_request"});
            (
                StatusCode::BAD_REQUEST,
                serde_json::to_vec(&payload).unwrap_or_default(),
            )
        }
    }
}

/// PUT /api/webhooks/{id} — Update a webhook subscription.
pub async fn update_webhook(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<crate::webhook_store::UpdateWebhookRequest>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => {
            let wh_id = crate::webhook_store::WebhookId(uuid);
            match state.webhook_store.update(wh_id, req) {
                Ok(webhook) => {
                    let redacted = crate::webhook_store::redact_webhook_secret(&webhook);
                    match serde_json::to_value(&redacted) {
                        Ok(v) => (StatusCode::OK, Json(v)),
                        Err(_) => {
                            ApiErrorResponse::internal(t.t("api-error-webhook-serialize-error"))
                                .into_json_tuple()
                        }
                    }
                }
                Err(e) => ApiErrorResponse::not_found(e).into_json_tuple(),
            }
        }
        Err(_) => {
            ApiErrorResponse::bad_request(t.t("api-error-webhook-invalid-id")).into_json_tuple()
        }
    }
}

/// DELETE /api/webhooks/{id} — Delete a webhook subscription.
pub async fn delete_webhook(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => {
            let wh_id = crate::webhook_store::WebhookId(uuid);
            if state.webhook_store.delete(wh_id) {
                (StatusCode::NO_CONTENT, Json(serde_json::json!(null)))
            } else {
                ApiErrorResponse::not_found(t.t("api-error-webhook-not-found")).into_json_tuple()
            }
        }
        Err(_) => {
            ApiErrorResponse::bad_request(t.t("api-error-webhook-invalid-id")).into_json_tuple()
        }
    }
}

/// POST /api/webhooks/{id}/test — Send a test event to a webhook.
///
/// Includes HMAC-SHA256 signature in `X-Webhook-Signature` header when
/// the webhook has a secret configured.
pub async fn test_webhook(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let (err_invalid_id, err_not_found) = {
        let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
        (
            t.t("api-error-webhook-invalid-id"),
            t.t("api-error-webhook-not-found"),
        )
    };
    let wh_id = match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => crate::webhook_store::WebhookId(uuid),
        Err(_) => {
            return ApiErrorResponse::bad_request(err_invalid_id).into_json_tuple();
        }
    };

    let webhook = match state.webhook_store.get(wh_id) {
        Some(w) => w,
        None => {
            return ApiErrorResponse::not_found(err_not_found).into_json_tuple();
        }
    };

    // Re-validate the URL against SSRF rules before sending. Use the
    // DNS-resolving variant so a hostname that flips to a private IP after
    // registration (DNS rebind, metadata IMDS, ec2 internal records) is
    // caught at fire-time, not just at registration (issue #3701).
    let pinned_host = match crate::webhook_store::validate_webhook_url_resolved(&webhook.url).await
    {
        Ok(host) => host,
        Err(e) => {
            let err_msg = {
                let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
                t.t_args("api-error-webhook-url-unsafe", &[("error", &e.to_string())])
            };
            return ApiErrorResponse::bad_request(err_msg).into_json_tuple();
        }
    };

    let test_payload = serde_json::json!({
        "event": "test",
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "webhook_id": webhook.id.to_string(),
        "message": "This is a test event from LibreFang.",
    });

    let payload_bytes = serde_json::to_vec(&test_payload).unwrap_or_default();

    // Pin reqwest's DNS resolver to the address we validated above. Without
    // this, reqwest does its own DNS lookup before connecting; a low-TTL
    // record can flip between our validate call and reqwest's resolve call
    // (DNS rebind), bypassing the SSRF check (#3701). `.resolve(host, addr)`
    // forces the connection to go to `addr` and skips reqwest's resolver
    // for that hostname.
    let mut builder = librefang_kernel::http_client::proxied_client_builder()
        .timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none());
    if let Some((ref host, addr)) = pinned_host {
        builder = builder.resolve(host, addr);
    }
    let client = builder.build().expect("HTTP client build");

    let mut request = client
        .post(&webhook.url)
        .header("Content-Type", "application/json")
        .header("User-Agent", "LibreFang-Webhook/1.0");

    // Add HMAC signature if secret is configured
    if let Some(ref secret) = webhook.secret {
        let signature = crate::webhook_store::compute_hmac_signature(secret, &payload_bytes);
        request = request.header("X-Webhook-Signature", signature);
    }

    match request.body(payload_bytes).send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "sent",
                    "response_status": status,
                    "webhook_id": id,
                })),
            )
        }
        Err(e) => {
            let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
            let msg = t.t_args(
                "api-error-webhook-reach-failed",
                &[("error", &e.to_string())],
            );
            (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({
                    "status": "error",
                    "error": msg,
                    "webhook_id": id,
                })),
            )
        }
    }
}
// ---------------------------------------------------------------------------
// Event Webhook Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod event_webhook_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use axum::Router;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    /// Serialize all webhook tests to avoid races on the shared EVENT_WEBHOOKS store.
    static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    fn webhook_router() -> Router {
        Router::new()
            .route(
                "/api/webhooks/events",
                axum::routing::get(list_event_webhooks).post(create_event_webhook),
            )
            .route(
                "/api/webhooks/events/{id}",
                axum::routing::put(update_event_webhook).delete(delete_event_webhook),
            )
    }

    async fn clear_webhooks() {
        EVENT_WEBHOOKS.write().await.clear();
    }

    #[tokio::test]
    async fn test_list_empty() {
        let _guard = TEST_LOCK.lock().await;
        clear_webhooks().await;
        let app = webhook_router();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/webhooks/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json, serde_json::json!([]));
    }

    #[tokio::test]
    async fn test_create_and_list() {
        let _guard = TEST_LOCK.lock().await;
        clear_webhooks().await;
        let app = webhook_router();

        let payload = serde_json::json!({
            "url": "https://example.com/hook",
            "events": ["agent.spawned", "agent.error"],
            "secret": "my-secret-key",
        });

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/webhooks/events")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let created: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(created["id"].as_str().is_some());
        assert_eq!(created["url"], "https://example.com/hook");
        assert_eq!(created["enabled"], true);
        // Secret must be redacted in responses
        assert_eq!(created["secret"], "***");

        // List should contain the webhook with redacted secret
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/webhooks/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let list: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(list.as_array().unwrap().len(), 1);
        assert_eq!(list[0]["secret"], "***");
    }

    #[tokio::test]
    async fn test_create_invalid_event() {
        let _guard = TEST_LOCK.lock().await;
        clear_webhooks().await;
        let app = webhook_router();

        let payload = serde_json::json!({
            "url": "https://example.com/hook",
            "events": ["nonexistent.event"],
        });

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/webhooks/events")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_create_missing_url() {
        let _guard = TEST_LOCK.lock().await;
        clear_webhooks().await;
        let app = webhook_router();

        let payload = serde_json::json!({
            "events": ["agent.spawned"],
        });

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/webhooks/events")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_create_invalid_url() {
        let _guard = TEST_LOCK.lock().await;
        clear_webhooks().await;
        let app = webhook_router();

        let payload = serde_json::json!({
            "url": "not a valid url",
            "events": ["agent.spawned"],
        });

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/webhooks/events")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_update_webhook() {
        let _guard = TEST_LOCK.lock().await;
        clear_webhooks().await;
        let app = webhook_router();

        let payload = serde_json::json!({
            "url": "https://example.com/hook",
            "events": ["agent.spawned"],
        });
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/webhooks/events")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let created: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let id = created["id"].as_str().unwrap();

        let update_payload = serde_json::json!({
            "enabled": false,
            "events": ["agent.spawned", "workflow.completed"],
        });
        let resp = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/webhooks/events/{id}"))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&update_payload).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let updated: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(updated["enabled"], false);
        assert_eq!(updated["events"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_delete_webhook() {
        let _guard = TEST_LOCK.lock().await;
        clear_webhooks().await;
        let app = webhook_router();

        let payload = serde_json::json!({
            "url": "https://example.com/hook",
            "events": ["agent.spawned"],
        });
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/webhooks/events")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let created: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let id = created["id"].as_str().unwrap();

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/webhooks/events/{id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/webhooks/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let list: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(list.as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_delete_not_found() {
        let _guard = TEST_LOCK.lock().await;
        clear_webhooks().await;
        let app = webhook_router();

        let resp = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/webhooks/events/nonexistent-id")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_update_not_found() {
        let _guard = TEST_LOCK.lock().await;
        clear_webhooks().await;
        let app = webhook_router();

        let payload = serde_json::json!({"enabled": false});
        let resp = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/webhooks/events/nonexistent-id")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
