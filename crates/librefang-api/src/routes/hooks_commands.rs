//! Webhook trigger endpoints (`/hooks/*`) and chat command catalog
//! endpoints (`/commands*`).
//!
//! Extracted from `routes/system.rs` as part of #3749 (sub-domain split
//! 4-of-N). Public route paths are unchanged — `system::router()` mounts
//! this module via `.merge(...)` so callers and OpenAPI bindings see the
//! same surface they did before the split.

use super::AppState;
use crate::middleware::RequestLanguage;
use crate::types::ApiErrorResponse;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use librefang_runtime::kernel_handle::KernelHandle;
use librefang_types::agent::AgentId;
use librefang_types::i18n::ErrorTranslator;
use std::sync::Arc;

/// Build the routes owned by this sub-domain. Mounted from
/// `routes::system::router()` via `.merge()` so the public paths stay
/// identical.
pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        // Webhook triggers (external event injection)
        .route("/hooks/wake", axum::routing::post(webhook_wake))
        .route("/hooks/agent", axum::routing::post(webhook_agent))
        // Chat command catalog
        .route("/commands", axum::routing::get(list_commands))
        .route("/commands/{name}", axum::routing::get(get_command))
}

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
        KernelHandle::publish_event(state.kernel.as_ref(), "webhook.wake", event_payload).await
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
            // No agent specified — use the first available agent
            match state.kernel.agent_registry().list().first() {
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

/// GET /api/commands — List available chat commands (for dynamic slash menu).
#[utoipa::path(get, path = "/api/commands", tag = "system", responses((status = 200, description = "List chat commands", body = Vec<serde_json::Value>)))]
pub async fn list_commands(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut commands = vec![
        serde_json::json!({"cmd": "/help", "desc": "Show available commands"}),
        serde_json::json!({"cmd": "/new", "desc": "Start a new session (new session id)"}),
        serde_json::json!({"cmd": "/reset", "desc": "Reset current session (clear history, same session id)"}),
        serde_json::json!({"cmd": "/reboot", "desc": "Hard reset session (full context clear, no summary)"}),
        serde_json::json!({"cmd": "/compact", "desc": "Trigger LLM session compaction"}),
        serde_json::json!({"cmd": "/model", "desc": "Show or switch model (/model [name])"}),
        serde_json::json!({"cmd": "/stop", "desc": "Cancel current agent run"}),
        serde_json::json!({"cmd": "/usage", "desc": "Show session token usage & cost"}),
        serde_json::json!({"cmd": "/think", "desc": "Toggle extended thinking (/think [on|off|stream])"}),
        serde_json::json!({"cmd": "/context", "desc": "Show context window usage & pressure"}),
        serde_json::json!({"cmd": "/verbose", "desc": "Cycle tool detail level (/verbose [off|on|full])"}),
        serde_json::json!({"cmd": "/queue", "desc": "Check if agent is processing"}),
        serde_json::json!({"cmd": "/status", "desc": "Show system status"}),
        serde_json::json!({"cmd": "/clear", "desc": "Clear chat display"}),
        serde_json::json!({"cmd": "/exit", "desc": "Disconnect from agent"}),
    ];

    // Add skill-registered tool names as potential commands
    if let Ok(registry) = state.kernel.skill_registry_ref().read() {
        for skill in registry.list() {
            let desc: String = skill.manifest.skill.description.chars().take(80).collect();
            commands.push(serde_json::json!({
                "cmd": format!("/{}", skill.manifest.skill.name),
                "desc": if desc.is_empty() { format!("Skill: {}", skill.manifest.skill.name) } else { desc },
                "source": "skill",
            }));
        }
    }

    Json(serde_json::json!({"commands": commands}))
}

/// GET /api/commands/{name} — Lookup a single command by name.
#[utoipa::path(get, path = "/api/commands/{name}", tag = "system", params(("name" = String, Path, description = "Command name")), responses((status = 200, description = "Command details", body = crate::types::JsonObject)))]
pub async fn get_command(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> (StatusCode, Json<serde_json::Value>) {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    // Normalise: ensure lookup key has a leading slash
    let lookup = if name.starts_with('/') {
        name.clone()
    } else {
        format!("/{name}")
    };

    // Built-in commands
    let builtins = [
        ("/help", "Show available commands"),
        ("/new", "Start a new session (new session id)"),
        (
            "/reset",
            "Reset current session (clear history, same session id)",
        ),
        (
            "/reboot",
            "Hard reset session (full context clear, no summary)",
        ),
        ("/compact", "Trigger LLM session compaction"),
        ("/model", "Show or switch model (/model [name])"),
        ("/stop", "Cancel current agent run"),
        ("/usage", "Show session token usage & cost"),
        (
            "/think",
            "Toggle extended thinking (/think [on|off|stream])",
        ),
        ("/context", "Show context window usage & pressure"),
        (
            "/verbose",
            "Cycle tool detail level (/verbose [off|on|full])",
        ),
        ("/queue", "Check if agent is processing"),
        ("/status", "Show system status"),
        ("/clear", "Clear chat display"),
        ("/exit", "Disconnect from agent"),
    ];

    for (cmd, desc) in &builtins {
        if cmd.eq_ignore_ascii_case(&lookup) {
            return (
                StatusCode::OK,
                Json(serde_json::json!({"cmd": cmd, "desc": desc})),
            );
        }
    }

    // Skill-registered commands
    if let Ok(registry) = state.kernel.skill_registry_ref().read() {
        for skill in registry.list() {
            let skill_cmd = format!("/{}", skill.manifest.skill.name);
            if skill_cmd.eq_ignore_ascii_case(&lookup) {
                let desc: String = skill.manifest.skill.description.chars().take(80).collect();
                return (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "cmd": skill_cmd,
                        "desc": if desc.is_empty() { format!("Skill: {}", skill.manifest.skill.name) } else { desc },
                        "source": "skill",
                    })),
                );
            }
        }
    }

    ApiErrorResponse::not_found(t.t_args("api-error-command-not-found", &[("name", &lookup)]))
        .into_json_tuple()
}

/// Constant-time bearer-token check for webhook endpoints. The expected
/// token is read from the env var named in the webhook config (so secrets
/// never live in `config.toml`); we require >= 32 bytes to avoid trivial
/// brute-forcing.
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
