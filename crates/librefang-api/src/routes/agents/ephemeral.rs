//! `POST /api/agents/spawn-ephemeral` — the HTTP entry point to the ephemeral worker engine (#6699).
//!
//! The engine itself is `LibreFangKernel::spawn_ephemeral_worker` (#7875), reachable until now only from inside a running agent's turn through `agent_spawn`'s `ephemeral: true` branch.
//! That left the primitive unusable by anything that is not already an agent: the dashboard, a script, a CI job, the CLI.
//! This route is the same call with an HTTP shape around it and nothing else — no second policy layer, no second tool filter, no second quota check.
//! Every guarantee the engine carries (the advertised tool set equals the executable one, spend and quota land on the parent, the recursion depth is the shared `max_agent_call_depth`) holds here because this handler adds nothing that could weaken them.
//!
//! ## Why a parent is required on the wire too
//!
//! `EphemeralSpawnRequest::parent_id` is mandatory, and the HTTP body cannot dodge that.
//! An ephemeral worker has no registry entry, so it has no budget, no `[resources]` quota and no tool allowlist of its own; the parent supplies all three.
//! A route that defaulted the parent to "some agent" would hand any authenticated caller an unattributed spend path, which is the exact hole the type was designed to close.
//! `parent` accepts either the agent's UUID or its name, the same way every other agent-addressed route in this module does.

use super::AppState;
use crate::middleware::RequestLanguage;
use crate::types::ApiErrorResponse;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use librefang_types::agent::AgentId;
use librefang_types::ephemeral::{EphemeralModelOverride, EphemeralSpawnRequest};
use librefang_types::i18n::ErrorTranslator;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Body of `POST /api/agents/spawn-ephemeral`.
///
/// `provider` and `model` are flat rather than a nested object because that is the shape every other authoring surface already speaks — the agent-type editor, `AgentTypeSpec`, `PATCH /api/agents/{id}/model`.
/// They are assembled into an [`EphemeralModelOverride`] here, which deliberately carries only those two fields: a `base_url` / `api_key_env` reachable from this endpoint would let a caller name any environment variable and any destination host.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SpawnEphemeralRequest {
    /// The agent the worker runs on behalf of — UUID or name.
    ///
    /// Not optional: it names the ledger the spend is billed to, the `[resources]` quota that is enforced, and the ceiling on the worker's tool set.
    pub parent: String,

    /// The task the worker is being asked to perform.
    pub message: String,

    /// Short human-meaningful label for the mission, used to name the mission workspace and the worker.
    ///
    /// Defaults to `agent_type` when one is given, and to `worker` otherwise, so the common "run this type on this task" call needs only two fields.
    #[serde(default)]
    pub label: Option<String>,

    /// Agent type whose template manifest supplies the worker's system prompt, model and declared tools.
    #[serde(default)]
    pub agent_type: Option<String>,

    /// System prompt for the worker; overrides the agent type's when both are given.
    #[serde(default)]
    pub system_prompt: Option<String>,

    /// Tool names the worker may use. Omit for "everything the effective manifest grants", which is itself capped by the parent's own set.
    #[serde(default)]
    pub tools: Option<Vec<String>>,

    /// Provider id override (`anthropic`, `openai`, …).
    #[serde(default)]
    pub provider: Option<String>,

    /// Model id override.
    #[serde(default)]
    pub model: Option<String>,

    /// Iteration ceiling, clamped down to the operator's configured `agent_max_iterations`.
    #[serde(default)]
    pub max_iterations: Option<u32>,
}

/// What one ephemeral worker turn produced.
///
/// Mirrors `librefang_types::ephemeral::EphemeralSpawnResult` field for field.
/// It is restated here rather than annotated there because `librefang-types` does not depend on `utoipa`, and adding a web-documentation dependency to the crate every other crate depends on to buy one schema is the wrong trade.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SpawnEphemeralResponse {
    /// Display name the worker ran under (`<label>-<8 hex>`), also its mission workspace directory name.
    pub name: String,
    /// The worker's final assistant text.
    pub response: String,
    /// Loop iterations consumed.
    pub iterations: u32,
    /// Cost billed to the parent, when one could be estimated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// Tool names the worker was given — which, by construction, is also the set it could execute.
    pub tools: Vec<String>,
}

/// Default mission label when the caller supplies none.
const DEFAULT_LABEL: &str = "worker";

/// POST /api/agents/spawn-ephemeral — run one ephemeral worker turn and return its result.
#[utoipa::path(
    post,
    path = "/api/agents/spawn-ephemeral",
    tag = "agents",
    operation_id = "spawn_ephemeral_agent",
    request_body = SpawnEphemeralRequest,
    responses(
        (status = 200, description = "The worker ran and returned its result", body = SpawnEphemeralResponse),
        (status = 400, description = "Malformed body, unknown agent type, or a tool the parent cannot call", body = crate::types::JsonObject),
        (status = 403, description = "The parent is suspended, or the spawn depth ceiling was reached", body = crate::types::JsonObject),
        (status = 404, description = "No such parent agent", body = crate::types::JsonObject),
        (status = 429, description = "The parent's resource quota or the global budget is exhausted", body = crate::types::JsonObject)
    )
)]
pub async fn spawn_ephemeral_agent(
    State(state): State<Arc<AppState>>,
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<SpawnEphemeralRequest>,
) -> axum::response::Response {
    // `ErrorTranslator` is `!Send`, so every `.await` below has to happen after this value is dropped — hence the eager `to_string()` and the tight scope.
    let not_found = {
        let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
        t.t("api-error-agent-not-found").to_string()
    };

    let parent_id: AgentId = match req.parent.parse() {
        Ok(id) => id,
        Err(_) => match state.kernel.agent_registry().find_by_name(&req.parent) {
            Some(entry) => entry.id,
            None => return not_found_response(not_found),
        },
    };
    // A non-admin caller may only spawn on behalf of an agent they own, and an id they may not see must not be distinguishable from one that does not exist — the same 404-not-403 rule the rest of this module applies.
    if !crate::routes::can_access_agent(&state, parent_id, api_user.as_ref()) {
        return not_found_response(not_found);
    }

    let label = req
        .label
        .as_deref()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .or(req.agent_type.as_deref())
        .unwrap_or(DEFAULT_LABEL)
        .to_string();

    let model = match (req.provider.as_deref(), req.model.as_deref()) {
        (None, None) => None,
        (provider, model) => Some(EphemeralModelOverride {
            provider: provider.map(str::to_string),
            model: model.map(str::to_string),
        }),
    };

    let request = EphemeralSpawnRequest {
        parent_id,
        label,
        message: req.message,
        agent_type: req.agent_type,
        system_prompt: req.system_prompt,
        tools: req.tools,
        model,
        max_iterations: req.max_iterations,
    };

    // A worker turn is an LLM call and can outlive the client.
    // Aborting it when the connection drops is what keeps a closed browser tab from billing the parent for a result nobody will read — the same guard `send_message` uses.
    let kernel = state.kernel.clone();
    let outcome =
        match super::run_cancel_on_disconnect(async move { kernel.spawn_ephemeral(request).await })
            .await
        {
            Ok(outcome) => outcome,
            Err(join_err) if join_err.is_cancelled() => {
                tracing::info!("spawn_ephemeral cancelled: client disconnected");
                return StatusCode::from_u16(499)
                    .unwrap_or(StatusCode::BAD_REQUEST)
                    .into_response();
            }
            Err(e) => {
                return ApiErrorResponse::internal_scrub(format!(
                    "ephemeral worker task panicked: {e}"
                ))
                .into_response();
            }
        };

    match outcome {
        Ok(result) => (
            StatusCode::OK,
            Json(SpawnEphemeralResponse {
                name: result.name,
                response: result.response,
                iterations: result.iterations,
                cost_usd: result.cost_usd,
                tools: result.tools,
            }),
        )
            .into_response(),
        Err(e) => ApiErrorResponse::from(e).into_response(),
    }
}

/// A 404 carrying the machine-readable `code` every other route emits, rather than a bare `{"error": …}` object.
///
/// Also what keeps the dead-route audit honest: it distinguishes a handler 404 from axum's `text/plain` fallback for an unregistered path by the content type, and this one is JSON.
fn not_found_response(message: String) -> axum::response::Response {
    let code = librefang_types::error_code::ErrorCode::AgentNotFound
        .as_str()
        .to_string();
    let mut err = ApiErrorResponse::not_found(message);
    err.r#type = Some(code.clone());
    err.code = Some(code);
    err.into_response()
}
