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
use librefang_skills::registry::SkillRegistry;
use librefang_types::i18n::ErrorTranslator;
use std::collections::BTreeSet;
use std::sync::{Arc, RwLock, RwLockReadGuard};

fn read_skill_registry(registry: &RwLock<SkillRegistry>) -> RwLockReadGuard<'_, SkillRegistry> {
    registry.read().unwrap_or_else(|poisoned| {
        tracing::warn!("Command catalog skill-registry lock poisoned; recovering installed skills");
        registry.clear_poison();
        poisoned.into_inner()
    })
}

fn snapshot_skill_commands(registry: &RwLock<SkillRegistry>) -> Vec<(String, String)> {
    let registry = read_skill_registry(registry);
    registry
        .list()
        .into_iter()
        .map(|skill| {
            (
                skill.manifest.skill.name.clone(),
                skill.manifest.skill.description.clone(),
            )
        })
        .collect()
}

fn skill_command_description(name: &str, description: &str) -> String {
    let description: String = description.chars().take(80).collect();
    if description.is_empty() {
        format!("Skill: {name}")
    } else {
        description
    }
}

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

    // Builtins own their names. Snapshot only the fields needed below while
    // holding the registry read guard, then do formatting and JSON allocation
    // after dropping it so skill installation is not blocked by response work.
    let mut seen: BTreeSet<String> = BUILTIN_COMMANDS
        .iter()
        .map(|(command, _)| command.to_ascii_lowercase())
        .collect();
    for (name, description) in snapshot_skill_commands(state.kernel.skill_registry_ref()) {
        let command = format!("/{name}");
        if !seen.insert(command.to_ascii_lowercase()) {
            continue;
        }
        commands.push(serde_json::json!({
            "cmd": command,
            "desc": skill_command_description(&name, &description),
            "source": "skill",
        }));
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
        name
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

    // Skill-registered commands. Builtin lookup above establishes precedence
    // for collisions without returning an ambiguous duplicate.
    let candidate = lookup.strip_prefix('/').unwrap_or(&lookup);
    for (skill_name, description) in snapshot_skill_commands(state.kernel.skill_registry_ref()) {
        if skill_name.eq_ignore_ascii_case(candidate) {
            let skill_command = format!("/{skill_name}");
            return (
                StatusCode::OK,
                Json(serde_json::json!({
                    "cmd": skill_command,
                    "desc": skill_command_description(&skill_name, &description),
                    "source": "skill",
                })),
            );
        }
    }

    ApiErrorResponse::not_found(t.t_args("api-error-command-not-found", &[("name", &lookup)]))
        .into_json_tuple()
}

#[cfg(test)]
mod tests {
    use super::read_skill_registry;
    use librefang_skills::registry::SkillRegistry;
    use std::sync::RwLock;

    #[test]
    fn command_registry_read_recovers_after_held_write_lock_panic() {
        let registry = RwLock::new(SkillRegistry::new(std::path::PathBuf::from(
            "/tmp/command-registry-poison-test",
        )));
        let _ = std::panic::catch_unwind(|| {
            let _guard = registry.write().unwrap();
            panic!("poison command catalog skill registry");
        });
        assert!(registry.is_poisoned());

        assert!(read_skill_registry(&registry).list().is_empty());

        assert!(!registry.is_poisoned());
        assert!(registry.read().is_ok());
        assert!(registry.write().is_ok());
    }
}
