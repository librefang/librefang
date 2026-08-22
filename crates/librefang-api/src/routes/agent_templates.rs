//! Tool profile + agent template endpoints — extracted from `system.rs` per #3749.
//!
//! Mounts `/profiles`, `/profiles/{name}`, `/templates`, `/templates/{name}`,
//! and `/templates/{name}/toml`. Public route paths are unchanged; this module
//! is a sibling under `routes::` and is mounted via
//! `.merge(crate::routes::agent_templates::router())` from `system::router()`.

use super::AppState;
use crate::middleware::RequestLanguage;
use crate::types::ApiErrorResponse;
use axum::extract::Path;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use librefang_types::agent::AgentManifest;
use librefang_types::i18n::ErrorTranslator;
use std::sync::Arc;

/// Build routes for the tool-profile + agent-template domain.
pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route("/profiles", axum::routing::get(list_profiles))
        .route("/profiles/{name}", axum::routing::get(get_profile))
        .route(
            "/templates",
            axum::routing::get(list_agent_templates).post(create_agent_type),
        )
        .route(
            "/templates/{name}",
            axum::routing::get(get_agent_template)
                .put(update_agent_type)
                .delete(delete_agent_type),
        )
        .route(
            "/templates/{name}/toml",
            axum::routing::get(get_agent_template_toml),
        )
}

// ---------------------------------------------------------------------------
// Profile + Mode endpoints
// ---------------------------------------------------------------------------

/// GET /api/profiles — List all tool profiles and their tool lists.
#[utoipa::path(
    get,
    path = "/api/profiles",
    tag = "system",
    responses(
        (status = 200, description = "List tool profiles", body = Vec<serde_json::Value>)
    )
)]
pub async fn list_profiles() -> impl IntoResponse {
    use librefang_types::agent::ToolProfile;

    let profiles = [
        ("minimal", ToolProfile::Minimal),
        ("coding", ToolProfile::Coding),
        ("research", ToolProfile::Research),
        ("messaging", ToolProfile::Messaging),
        ("automation", ToolProfile::Automation),
        ("full", ToolProfile::Full),
    ];

    let result: Vec<serde_json::Value> = profiles
        .iter()
        .map(|(name, profile)| {
            serde_json::json!({
                "name": name,
                "tools": profile.tools(),
            })
        })
        .collect();

    Json(result)
}

/// GET /api/profiles/:name — Get a single profile by name.
#[utoipa::path(get, path = "/api/profiles/{name}", tag = "system", params(("name" = String, Path, description = "Profile name")), responses((status = 200, description = "Profile details", body = crate::types::JsonObject)))]
pub async fn get_profile(
    Path(name): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    use librefang_types::agent::ToolProfile;

    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));

    let profiles: &[(&str, ToolProfile)] = &[
        ("minimal", ToolProfile::Minimal),
        ("coding", ToolProfile::Coding),
        ("research", ToolProfile::Research),
        ("messaging", ToolProfile::Messaging),
        ("automation", ToolProfile::Automation),
        ("full", ToolProfile::Full),
    ];

    match profiles.iter().find(|(n, _)| *n == name) {
        Some((n, profile)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "name": n,
                "tools": profile.tools(),
            })),
        ),
        None => {
            ApiErrorResponse::not_found(t.t_args("api-error-profile-not-found", &[("name", &name)]))
                .into_json_tuple()
        }
    }
}

// ---------------------------------------------------------------------------
// Template endpoints
// ---------------------------------------------------------------------------

/// Validate a template name supplied via URL path before joining it onto the
/// templates directory. Only permits `[A-Za-z0-9_-]` to guarantee the result
/// cannot escape the base directory through `..`, absolute paths, or platform
/// separators (`/`, `\`). Rejects empty names and anything longer than 64
/// chars to cap log noise.
fn validate_template_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty() || name.len() > 64 {
        return Err("invalid template name");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err("invalid template name");
    }
    Ok(())
}

#[cfg(test)]
mod template_name_validation_tests {
    use super::validate_template_name;

    #[test]
    fn accepts_simple_names() {
        assert!(validate_template_name("assistant").is_ok());
        assert!(validate_template_name("customer-support").is_ok());
        assert!(validate_template_name("coder_v2").is_ok());
        assert!(validate_template_name("a1").is_ok());
    }

    #[test]
    fn rejects_path_traversal() {
        assert!(validate_template_name("..").is_err());
        assert!(validate_template_name("../../etc").is_err());
        assert!(validate_template_name("foo/../bar").is_err());
        assert!(validate_template_name("..\\..\\tmp").is_err());
    }

    #[test]
    fn rejects_separators_and_absolute_paths() {
        assert!(validate_template_name("foo/bar").is_err());
        assert!(validate_template_name("foo\\bar").is_err());
        assert!(validate_template_name("/etc/passwd").is_err());
        assert!(validate_template_name("C:\\Windows").is_err());
    }

    #[test]
    fn rejects_empty_and_oversized() {
        assert!(validate_template_name("").is_err());
        assert!(validate_template_name(&"a".repeat(65)).is_err());
    }

    #[test]
    fn rejects_null_and_special_chars() {
        assert!(validate_template_name("foo\0bar").is_err());
        assert!(validate_template_name("foo bar").is_err());
        assert!(validate_template_name("foo.bar").is_err());
        assert!(validate_template_name("foo%2fbar").is_err());
    }
}

/// GET /api/templates — List available agent templates from both
/// `~/.librefang/workspaces/agents/` (source = "agent") and
/// `~/.librefang/templates/` (source = "template").
#[utoipa::path(get, path = "/api/templates", tag = "system", operation_id = "list_agent_templates", responses((status = 200, description = "List templates", body = Vec<serde_json::Value>)))]
pub async fn list_agent_templates() -> impl IntoResponse {
    let mut templates = Vec::new();

    // Workspace agents (existing behaviour)
    let agents_dir = super::system::librefang_home()
        .join("workspaces")
        .join("agents");
    if let Ok(entries) = std::fs::read_dir(&agents_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let manifest_path = path.join("agent.toml");
            if !manifest_path.exists() {
                continue;
            }
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let description = std::fs::read_to_string(&manifest_path)
                .ok()
                .and_then(|content| toml::from_str::<AgentManifest>(&content).ok())
                .map(|m| m.description)
                .unwrap_or_default();
            templates.push(serde_json::json!({
                "name": name,
                "description": description,
                "source": "agent",
            }));
        }
    }

    // Template files (new — user-created via POST /api/templates)
    let templates_dir = agent_types_dir();
    if let Ok(entries) = std::fs::read_dir(&templates_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let name = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let description = std::fs::read_to_string(&path)
                .ok()
                .and_then(|content| toml::from_str::<AgentManifest>(&content).ok())
                .map(|m| m.description)
                .unwrap_or_default();
            templates.push(serde_json::json!({
                "name": name,
                "description": description,
                "source": "template",
            }));
        }
    }

    // Deterministic ordering
    templates.sort_by(|a, b| {
        a["name"]
            .as_str()
            .unwrap_or_default()
            .cmp(b["name"].as_str().unwrap_or_default())
    });

    Json(serde_json::json!({
        "templates": templates,
        "items": templates,
        "total": templates.len(),
    }))
}

/// GET /api/templates/:name — Get template details.
#[utoipa::path(get, path = "/api/templates/{name}", tag = "system", operation_id = "get_agent_template", params(("name" = String, Path, description = "Template name")), responses((status = 200, description = "Template details", body = crate::types::JsonObject)))]
pub async fn get_agent_template(
    Path(name): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    if validate_template_name(&name).is_err() {
        return ApiErrorResponse::not_found(t.t("api-error-template-not-found")).into_json_tuple();
    }

    // Check templates dir first (user-created), then workspaces agents
    let template_path = agent_types_dir().join(format!("{name}.toml"));
    let agents_dir = super::system::librefang_home()
        .join("workspaces")
        .join("agents");
    let agent_path = agents_dir.join(&name).join("agent.toml");

    let (manifest_path, source) = if template_path.exists() {
        (template_path, "template")
    } else if agent_path.exists() {
        (agent_path, "agent")
    } else {
        return ApiErrorResponse::not_found(t.t("api-error-template-not-found")).into_json_tuple();
    };

    match std::fs::read_to_string(&manifest_path) {
        Ok(content) => match toml::from_str::<AgentManifest>(&content) {
            Ok(manifest) => (
                StatusCode::OK,
                Json({
                    let mut v = manifest_to_agent_type(&name, &manifest);
                    if let Some(o) = v.as_object_mut() {
                        o.insert(
                            "source".to_string(),
                            serde_json::Value::String(source.to_string()),
                        );
                        // The flat fields above are the list-row shape the
                        // dashboard's AgentTypes page reads, and `name` there is
                        // the *template id* (the `.toml` filename). They collapse
                        // that id together with the agent name the manifest
                        // itself declares, and drop `module` / `version` /
                        // `author` entirely — so a detail response built only
                        // from them cannot answer "what does this template
                        // actually declare?".
                        //
                        // Expose the parsed manifest under its own key
                        // alongside them: `name` stays the template id,
                        // `manifest.name` is the declared agent name, and the
                        // two are allowed to differ. Additive on purpose — the
                        // flat fields keep working unchanged.
                        match serde_json::to_value(&manifest) {
                            Ok(m) => {
                                o.insert("manifest".to_string(), m);
                            }
                            Err(e) => {
                                // Serializing a manifest that already parsed
                                // from TOML should not fail; surface it in the
                                // log rather than silently shipping a response
                                // with the key missing.
                                tracing::warn!(
                                    "Failed to serialize manifest for template '{name}': {e}"
                                );
                            }
                        }
                        o.insert(
                            "manifest_toml".to_string(),
                            serde_json::Value::String(content),
                        );
                    }
                    v
                }),
            ),
            Err(e) => {
                tracing::warn!("Invalid template manifest for '{name}': {e}");
                ApiErrorResponse::internal(t.t("api-error-template-invalid-manifest"))
                    .into_json_tuple()
            }
        },
        Err(e) => {
            tracing::warn!("Failed to read template '{name}': {e}");
            ApiErrorResponse::internal(t.t("api-error-template-read-failed")).into_json_tuple()
        }
    }
}

/// GET /api/templates/:name/toml — Get the raw TOML content of a template.
#[utoipa::path(get, path = "/api/templates/{name}/toml", tag = "system", operation_id = "get_agent_template_toml", params(("name" = String, Path, description = "Template name")), responses((status = 200, description = "Template TOML content as plain text", body = String)))]
pub async fn get_agent_template_toml(
    Path(name): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    if validate_template_name(&name).is_err() {
        return (
            StatusCode::NOT_FOUND,
            [(axum::http::header::CONTENT_TYPE, "text/plain")],
            t.t("api-error-template-not-found"),
        )
            .into_response();
    }
    let template_path = agent_types_dir().join(format!("{name}.toml"));
    let agents_dir = super::system::librefang_home()
        .join("workspaces")
        .join("agents");
    let agent_path = agents_dir.join(&name).join("agent.toml");

    let manifest_path = if template_path.exists() {
        template_path
    } else if agent_path.exists() {
        agent_path
    } else {
        return (
            StatusCode::NOT_FOUND,
            [(axum::http::header::CONTENT_TYPE, "text/plain")],
            t.t("api-error-template-not-found"),
        )
            .into_response();
    };

    match std::fs::read_to_string(&manifest_path) {
        Ok(content) => (
            StatusCode::OK,
            [(
                axum::http::header::CONTENT_TYPE,
                "text/plain; charset=utf-8",
            )],
            content,
        )
            .into_response(),
        Err(e) => {
            tracing::warn!("Failed to read template '{name}': {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(axum::http::header::CONTENT_TYPE, "text/plain")],
                t.t("api-error-template-read-failed"),
            )
                .into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// Agent template CRUD endpoints
// ---------------------------------------------------------------------------
//
// Templates are named manifests consumed by the ephemeral-worker spawn path
// (`EphemeralSpawnRequest.agent_type`). They live as `<name>.toml` under
// `~/.librefang/templates/`. Write operations are wired into `/api/templates`
// in the unified `router()` above; the read endpoints serve both sources.

/// Directory holding agent-template manifests (`~/.librefang/templates/`).
fn agent_types_dir() -> std::path::PathBuf {
    super::system::librefang_home().join("templates")
}

/// Flatten a manifest into the JSON shape the dashboard expects.
fn manifest_to_agent_type(name: &str, m: &AgentManifest) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "description": m.description,
        "system_prompt": m.model.system_prompt,
        "provider": m.model.provider,
        "model": m.model.model,
        "tools": m.capabilities.tools,
        "skills": m.skills,
    })
}

/// Build a minimal `agent.toml` from the dashboard's JSON shape.
/// Uses `toml::to_string_pretty` on a constructed `AgentManifest` so every
/// caller-supplied string (name, description, system_prompt, provider, model)
/// is properly escaped — no `format!` interpolation raw-dropping untrusted
/// input into TOML string literals.
fn agent_type_json_to_toml(v: &serde_json::Value) -> String {
    let name = v["name"].as_str().unwrap_or("unnamed");
    let desc = v["description"].as_str().unwrap_or("");
    let prompt = v["system_prompt"]
        .as_str()
        .unwrap_or("You are a helpful AI agent.");
    let provider = v["provider"].as_str().unwrap_or("default");
    let model_name = v["model"].as_str().unwrap_or("default");
    let tools: Vec<String> = v["tools"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|t| t.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let skills: Vec<String> = v["skills"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let manifest = librefang_types::agent::AgentManifest {
        name: name.to_string(),
        description: desc.to_string(),
        skills,
        model: librefang_types::agent::ModelConfig {
            provider: provider.to_string(),
            model: model_name.to_string(),
            system_prompt: prompt.to_string(),
            ..Default::default()
        },
        capabilities: librefang_types::agent::ManifestCapabilities {
            tools,
            ..Default::default()
        },
        ..Default::default()
    };

    toml::to_string_pretty(&manifest).unwrap_or_else(|_| {
        // Fallback for round-trip safety — the TOML serializer should always
        // succeed.
        "[capabilities]\ntools = []\n\n[model]\nmodel = \"default\"\nprovider = \"default\"\nsystem_prompt = \"\"\n"
            .to_string()
    })
}

/// POST /api/templates — Create a new agent template from JSON.
#[utoipa::path(post, path = "/api/templates", tag = "system", operation_id = "create_template", request_body = crate::types::JsonObject, responses((status = 201, description = "Template created", body = crate::types::JsonObject), (status = 400, description = "Invalid input"), (status = 409, description = "Template already exists")))]
pub async fn create_agent_type(
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));

    let name = match body["name"].as_str() {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => {
            return ApiErrorResponse::bad_request("name is required").into_json_tuple();
        }
    };
    if validate_template_name(&name).is_err() {
        return ApiErrorResponse::bad_request("invalid agent type name").into_json_tuple();
    }

    let toml_content = agent_type_json_to_toml(&body);

    let dir = agent_types_dir();
    let path = dir.join(format!("{name}.toml"));
    if path.exists() {
        return ApiErrorResponse::conflict(format!("Agent type '{name}' already exists"))
            .into_json_tuple();
    }

    // Cross-source collision (#6931 review): a workspace agent with the same
    // name is also resolvable as a template (dual-source listing), so
    // creating a template that shadows a live agent's name is a 409 too.
    let workspace_agent_path = super::system::librefang_home()
        .join("workspaces")
        .join("agents")
        .join(&name);
    if workspace_agent_path.exists() {
        return ApiErrorResponse::conflict(format!(
            "A workspace agent named '{name}' already exists — creating a template with the same name would shadow it"
        ))
        .into_json_tuple();
    }

    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!("Failed to create templates dir: {e}");
        return ApiErrorResponse::internal(t.t("api-error-internal")).into_json_tuple();
    }
    if let Err(e) = std::fs::write(&path, &toml_content) {
        tracing::warn!("Failed to write agent-type '{name}': {e}");
        return ApiErrorResponse::internal(t.t("api-error-internal")).into_json_tuple();
    }

    let manifest: AgentManifest = toml::from_str(&toml_content).unwrap_or_default();
    (
        StatusCode::CREATED,
        Json(manifest_to_agent_type(&name, &manifest)),
    )
}

/// PUT /api/templates/{name} — Update an existing agent type from JSON.
#[utoipa::path(put, path = "/api/templates/{name}", tag = "system", operation_id = "update_template", params(("name" = String, Path, description = "Template name")), request_body = crate::types::JsonObject, responses((status = 200, description = "Template updated", body = crate::types::JsonObject), (status = 400, description = "Invalid input"), (status = 404, description = "Template not found")))]
pub async fn update_agent_type(
    Path(name): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    if validate_template_name(&name).is_err() {
        return ApiErrorResponse::not_found(t.t("api-error-template-not-found")).into_json_tuple();
    }

    // Pin the manifest name to the URL path segment — the body's
    // "name" field is advisory; the path is authoritative (#6931 review).
    let mut body = body;
    body["name"] = serde_json::Value::String(name.clone());

    let toml_content = agent_type_json_to_toml(&body);

    let path = agent_types_dir().join(format!("{name}.toml"));
    if !path.exists() {
        return ApiErrorResponse::not_found(t.t("api-error-template-not-found")).into_json_tuple();
    }

    if let Err(e) = std::fs::write(&path, &toml_content) {
        tracing::warn!("Failed to write agent-type '{name}': {e}");
        return ApiErrorResponse::internal(t.t("api-error-internal")).into_json_tuple();
    }

    let manifest: AgentManifest = toml::from_str(&toml_content).unwrap_or_default();
    (
        StatusCode::OK,
        Json(manifest_to_agent_type(&name, &manifest)),
    )
}

/// DELETE /api/templates/{name} — Delete an agent type file.
#[utoipa::path(delete, path = "/api/templates/{name}", tag = "system", operation_id = "delete_template", params(("name" = String, Path, description = "Template name")), responses((status = 200, description = "Template deleted", body = crate::types::JsonObject), (status = 404, description = "Template not found")))]
pub async fn delete_agent_type(
    Path(name): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    if validate_template_name(&name).is_err() {
        return ApiErrorResponse::not_found(t.t("api-error-template-not-found")).into_json_tuple();
    }

    let path = agent_types_dir().join(format!("{name}.toml"));
    if !path.exists() {
        return ApiErrorResponse::not_found(t.t("api-error-template-not-found")).into_json_tuple();
    }

    if let Err(e) = std::fs::remove_file(&path) {
        tracing::warn!("Failed to delete agent-type '{name}': {e}");
        return ApiErrorResponse::internal(t.t("api-error-internal")).into_json_tuple();
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({ "name": name, "deleted": true })),
    )
}
