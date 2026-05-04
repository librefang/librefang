//! Chat command catalog endpoints (#3749 11/N).
//!
//! Exposes the slash-command dictionary used by the dashboard's chat UI:
//! a fixed builtin list plus skill-derived dynamic entries.

use super::AppState;
use crate::middleware::RequestLanguage;
use crate::types::ApiErrorResponse;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use librefang_types::i18n::ErrorTranslator;
use std::sync::Arc;

pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route("/commands", axum::routing::get(list_commands))
        .route("/commands/{name}", axum::routing::get(get_command))
}

/// Built-in slash commands shared by [`list_commands`] and [`get_command`].
const BUILTIN_COMMANDS: &[(&str, &str)] = &[
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

/// GET /api/commands — List available chat commands (for dynamic slash menu).
#[utoipa::path(get, path = "/api/commands", tag = "system", responses((status = 200, description = "List chat commands", body = Vec<serde_json::Value>)))]
pub async fn list_commands(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut commands: Vec<serde_json::Value> = BUILTIN_COMMANDS
        .iter()
        .map(|(cmd, desc)| serde_json::json!({"cmd": cmd, "desc": desc}))
        .collect();

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

    for (cmd, desc) in BUILTIN_COMMANDS {
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
