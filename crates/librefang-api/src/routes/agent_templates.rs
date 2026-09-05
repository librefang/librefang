//! Tool profile + agent template endpoints — extracted from `system.rs` per #3749.
//!
//! Mounts `/profiles`, `/profiles/{name}`, `/templates`, `/templates/{name}`, and `/templates/{name}/toml`.
//! This module is a sibling under `routes::` and is mounted via `.merge(crate::routes::agent_templates::router())` from `system::router()`.
//!
//! `/templates` and `/templates/{name}` also carry the agent-type write verbs (#7740, #7731): `POST` creates an operator-authored agent type, `PUT` patches one, `DELETE` removes one.
//! Read the doc comment on [`update_agent_type`] before touching the write path — the patch semantics there are the whole point of the endpoint, not an implementation detail.

use super::AppState;
use crate::middleware::RequestLanguage;
use crate::routes::agents::lifecycle::MAX_MANIFEST_SIZE;
use crate::types::ApiErrorResponse;
use axum::extract::{Path, Query, State};
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
            axum::routing::get(get_agent_template_toml).put(put_agent_template_toml),
        )
        .route(
            "/templates/{name}/promote",
            axum::routing::post(promote_agent_type),
        )
        .route(
            "/templates/{name}/history",
            axum::routing::get(list_template_history),
        )
        .route(
            "/templates/{name}/history/{version_id}/restore",
            axum::routing::post(restore_template_version),
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

// The agent-type name rule, shared with the `agent_type_create` tool through `librefang_types::agent_type_store`.
// It lives there rather than here because the tool joins a model-supplied name onto the same directory these routes do, and a traversal guard that only one of the two callers runs is not a guard.
use librefang_types::agent_type_store::validate_agent_type_name as validate_template_name;

/// Render the three template error strings every read handler might need, and drop the translator.
///
/// `ErrorTranslator` is `!Send`, so holding one across an `.await` makes the enclosing handler fail axum's `Handler` bound with an error that names neither the translator nor the await.
/// Rendering eagerly into owned `String`s in a scope that ends before the first await sidesteps that entirely.
fn template_error_messages(lang: &str, name: &str) -> (String, String, String) {
    let t = ErrorTranslator::new(lang);
    (
        t.t_args("api-error-template-not-found", &[("name", name)]),
        t.t("api-error-template-invalid-manifest"),
        t.t("api-error-template-read-failed"),
    )
}
/// Top-level keys the submitted TOML carries that `AgentManifest` does not recognize.
///
/// The write path re-serializes from the parsed struct, so any key that does not
/// round-trip through `AgentManifest` is dropped on persist. Diffing against the
/// re-serialized struct rather than a hand-kept key list means the check stays
/// correct as the struct's serde attributes evolve; a key that deserializes but
/// does not re-serialize is dropped too, so flagging it is the honest answer either way.
fn unrecognized_manifest_keys(doc: &toml::Value, manifest: &AgentManifest) -> Vec<String> {
    let Ok(round_tripped) = toml::Value::try_from(manifest) else {
        return Vec::new();
    };
    let Some(doc_table) = doc.as_table() else {
        return Vec::new();
    };
    let recognized: std::collections::HashSet<&String> = round_tripped
        .as_table()
        .map(|t| t.keys().collect())
        .unwrap_or_default();
    doc_table
        .keys()
        .filter(|k| !recognized.contains(*k))
        .cloned()
        .collect()
}

/// Render the two promote messages that need no dynamic argument, and drop the translator.
///
/// The render-failure message carries the TOML error, which is only known after the
/// sanitization runs, so it is rendered inline at its one call site instead.
fn promote_error_messages(lang: &str) -> (String, String) {
    let t = ErrorTranslator::new(lang);
    (
        t.t("api-error-template-promote-no-token"),
        t.t("api-error-template-promote-review-required"),
    )
}

// ---------------------------------------------------------------------------
// Agent-type storage
// ---------------------------------------------------------------------------

/// Where an agent type comes from, and therefore whether this API can write it.
///
/// The catalog is deliberately dual-source: an operator-authored agent type is a standalone document under `agent-types/`, while every live agent's own `agent.toml` is also spawnable-from and has always been listed here.
/// Only the first is a file this API owns.
/// The second belongs to a running agent and is edited through `/api/agents/{id}`, so a write verb aimed at it is refused rather than silently creating a shadowing copy — and the row carries `editable: false` so a client can render it as managed elsewhere instead of offering a control that cannot work (#7731).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TemplateSource {
    /// `~/.librefang/agent-types/{name}.toml` — created and edited through this API.
    AgentType,
    /// `~/.librefang/workspaces/agents/{name}/agent.toml` — a live agent's manifest.
    WorkspaceAgent,
}

impl TemplateSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::AgentType => "agent-type",
            Self::WorkspaceAgent => "agent",
        }
    }

    fn is_editable(self) -> bool {
        matches!(self, Self::AgentType)
    }
}

use librefang_types::agent_type_store::{
    agent_type_path, agent_types_dir, workspace_agent_manifest_path, workspace_agents_dir,
};

/// Read one agent type by name from whichever source holds it.
///
/// `agent-types/` wins a name collision because it is the source the write verbs act on: if `Edit` loaded a live agent's manifest and `Save` wrote the agent-type file, the operator would be editing one document and saving another.
/// A collision can still arise after the fact — an agent spawned under a name an agent type already uses — so it is logged rather than passed over in silence.
async fn read_agent_type(name: &str) -> std::io::Result<Option<(TemplateSource, String)>> {
    let own = match tokio::fs::read_to_string(agent_type_path(name)).await {
        Ok(content) => Some(content),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    let workspace = match tokio::fs::read_to_string(workspace_agent_manifest_path(name)).await {
        Ok(content) => Some(content),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };

    match (own, workspace) {
        (Some(content), Some(_)) => {
            tracing::warn!(
                "agent type '{name}' and a live agent workspace share a name; serving the agent type, which is the copy this API can write"
            );
            Ok(Some((TemplateSource::AgentType, content)))
        }
        (Some(content), None) => Ok(Some((TemplateSource::AgentType, content))),
        (None, Some(content)) => Ok(Some((TemplateSource::WorkspaceAgent, content))),
        (None, None) => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Listing
// ---------------------------------------------------------------------------

/// GET /api/templates — List available agent templates.
#[utoipa::path(get, path = "/api/templates", tag = "system", operation_id = "list_agent_templates", responses((status = 200, description = "List templates", body = Vec<serde_json::Value>)))]
pub async fn list_agent_templates() -> impl IntoResponse {
    let mut rows: Vec<(String, TemplateSource, AgentManifest)> = Vec::new();

    match load_agent_type_files(&agent_types_dir()).await {
        Ok(found) => rows.extend(
            found
                .into_iter()
                .map(|(name, manifest)| (name, TemplateSource::AgentType, manifest)),
        ),
        Err(e) => return ApiErrorResponse::internal_scrub(e).into_json_tuple(),
    }
    match load_agent_templates(&workspace_agents_dir()).await {
        Ok(found) => rows.extend(
            found
                .into_iter()
                .map(|(name, manifest)| (name, TemplateSource::WorkspaceAgent, manifest)),
        ),
        Err(e) => return ApiErrorResponse::internal_scrub(e).into_json_tuple(),
    }

    // Same precedence as `read_agent_type`: the writable copy wins, so a row's `editable` flag
    // agrees with what a PUT to that name would actually do.
    // `sort_by` is stable, and the agent-type rows were pushed first, so `dedup_by` keeps them.
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    rows.dedup_by(|a, b| a.0 == b.0);

    let templates: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|(name, source, manifest)| {
            serde_json::json!({
                "name": name,
                "description": manifest.description,
                // `provider` / `model` let a client show and gate on what the template actually declares rather than assuming a default (#7760).
                "provider": manifest.model.provider,
                "model": manifest.model.model,
                "source": source.as_str(),
                "editable": source.is_editable(),
            })
        })
        .collect();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "templates": templates,
            "total": templates.len(),
        })),
    )
}

/// Load every operator-authored agent type — one flat `{name}.toml` per type.
async fn load_agent_type_files(
    dir: &std::path::Path,
) -> Result<Vec<(String, AgentManifest)>, String> {
    let mut found = Vec::new();
    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(found),
        Err(e) => return Err(format!("failed to list agent types: {e}")),
    };

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| format!("failed to read an agent type entry: {e}"))?
    {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            // Skips the staging files `atomic_write` leaves behind if the process dies mid-rename, as well as anything else an operator dropped in the directory.
            continue;
        }
        let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        // Do not advertise an agent type whose name `/templates/{name}` will reject — a listed row a client cannot fetch or spawn from is a dead end.
        if validate_template_name(name).is_err() {
            tracing::warn!("skipping agent type file with unusable name: {name:?}");
            continue;
        }
        let content = match tokio::fs::read_to_string(&path).await {
            Ok(content) => content,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                return Err(format!("failed to read agent type {}: {e}", path.display()));
            }
        };
        match toml::from_str::<AgentManifest>(&content) {
            // One operator typo must not blank the whole catalog for every client.
            // Skip the entry and name the file so the mistake is diagnosable instead of arriving as a bare 500.
            Err(e) => tracing::warn!(
                "skipping agent type {}: invalid manifest: {e}",
                path.display()
            ),
            Ok(manifest) => found.push((name.to_string(), manifest)),
        }
    }

    Ok(found)
}

/// Load every live agent's manifest — the second source of the catalog, `{name}/agent.toml` per agent.
async fn load_agent_templates(
    agents_dir: &std::path::Path,
) -> Result<Vec<(String, AgentManifest)>, String> {
    let mut templates = Vec::new();
    let mut entries = match tokio::fs::read_dir(agents_dir).await {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(templates),
        Err(e) => return Err(format!("failed to list agent templates: {e}")),
    };

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| format!("failed to read an agent template entry: {e}"))?
    {
        let file_type = entry
            .file_type()
            .await
            .map_err(|e| format!("failed to inspect an agent template entry: {e}"))?;
        if !file_type.is_dir() {
            continue;
        }

        let path = entry.path();
        let manifest_path = path.join("agent.toml");
        let content = match tokio::fs::read_to_string(&manifest_path).await {
            Ok(content) => content,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                return Err(format!(
                    "failed to read agent template manifest {}: {e}",
                    manifest_path.display()
                ));
            }
        };
        let manifest = match toml::from_str::<AgentManifest>(&content) {
            Ok(manifest) => manifest,
            Err(e) => {
                // One operator typo must not blank the whole catalog for every client.
                // Skip the entry and name the file so the mistake is diagnosable instead of arriving as a bare 500.
                tracing::warn!(
                    "skipping agent template {}: invalid manifest: {e}",
                    manifest_path.display()
                );
                continue;
            }
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        // Do not advertise a template whose name `/templates/{name}` and `/templates/{name}/toml` will reject — a listed row a client cannot fetch or spawn from is a dead end.
        if validate_template_name(&name).is_err() {
            tracing::warn!("skipping agent template directory with unusable name: {name:?}");
            continue;
        }
        templates.push((name, manifest));
    }

    Ok(templates)
}

/// Build the privacy pass over an agent type that an operator is considering
/// contributing to a shared registry (#7771).
///
/// A manifest written on one machine carries that machine's details — an absolute workspace path, the environment variable holding the operator's provider credentials, a private base URL, a command allowlist, free text pasted into a system prompt.
/// The registry validator requires only `name`, `description` and `module`, so nothing downstream catches any of it, and once published it is in git history.
///
/// This is read-only and advisory.
/// It reports what a promotion would strip (`findings` with `removed_by_sanitizer: true`), what the operator has to edit by hand because it sits inside a field worth keeping (`removed_by_sanitizer: false`, summarised by `requires_review`), and the scrubbed manifest itself so it can be attached to a registry pull request today.
/// Nothing here rewrites the operator's file.
///
/// `manifest_toml` is `null` when the publishable copy cannot be rendered as TOML.
/// That is not a reason to withhold the findings, which are the part that protects the operator.
fn promotion_preview(name: &str, manifest: &AgentManifest) -> serde_json::Value {
    use librefang_types::manifest_privacy::{sanitize_for_publication, scan_for_publication};

    let findings = scan_for_publication(manifest);
    let requires_review = findings.iter().any(|finding| !finding.removed_by_sanitizer);

    let publishable = sanitize_for_publication(manifest);
    let manifest_toml = match toml::to_string_pretty(&publishable) {
        Ok(rendered) => Some(rendered),
        Err(e) => {
            tracing::warn!("Could not render the publishable manifest for template '{name}': {e}");
            None
        }
    };

    serde_json::json!({
        "requires_review": requires_review,
        "findings": findings,
        "manifest_toml": manifest_toml,
    })
}

/// The single detail document, shared by GET, POST and PUT so the three can never drift apart.
///
/// `spec` is the flat editor projection — exactly the seven keys a `PUT` accepts back.
/// It sits alongside, not instead of, the pre-existing nested `manifest` summary and the verbatim `manifest_toml`, both of which existing clients already read.
fn agent_type_detail(
    name: &str,
    source: TemplateSource,
    manifest: &AgentManifest,
    manifest_toml: &str,
) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "source": source.as_str(),
        "editable": source.is_editable(),
        "spec": librefang_types::agent_type::agent_type_spec_of(manifest),
        "promotion_preview": promotion_preview(name, manifest),
        "manifest": {
            "name": manifest.name,
            "description": manifest.description,
            "module": manifest.module,
            "tags": manifest.tags,
            "model": {
                "provider": manifest.model.provider,
                "model": manifest.model.model,
            },
            "capabilities": {
                "tools": manifest.capabilities.tools,
                "network": manifest.capabilities.network,
            },
        },
        "manifest_toml": manifest_toml,
    })
}

/// GET /api/templates/:name — Get template details.
#[utoipa::path(get, path = "/api/templates/{name}", tag = "system", operation_id = "get_agent_template", params(("name" = String, Path, description = "Template name")), responses((status = 200, description = "Template details, plus the flat editor projection and a read-only privacy pass over the manifest for registry promotion", body = crate::types::JsonObject)))]
pub async fn get_agent_template(
    Path(name): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let lang = super::resolve_lang(lang.as_ref());
    // `ErrorTranslator` is `!Send`, so every message this handler might need is rendered and the
    // translator dropped before the first `.await` — see the note in the root `CLAUDE.md`.
    let (not_found, invalid_manifest, read_failed) = template_error_messages(lang, &name);

    if validate_template_name(&name).is_err() {
        return ApiErrorResponse::not_found(not_found).into_json_tuple();
    }

    match read_agent_type(&name).await {
        Ok(Some((source, content))) => match toml::from_str::<AgentManifest>(&content) {
            Ok(manifest) => (
                StatusCode::OK,
                Json(agent_type_detail(&name, source, &manifest, &content)),
            ),
            Err(e) => {
                tracing::warn!("Invalid template manifest for '{name}': {e}");
                ApiErrorResponse::internal(invalid_manifest).into_json_tuple()
            }
        },
        Ok(None) => ApiErrorResponse::not_found(not_found).into_json_tuple(),
        Err(e) => {
            tracing::warn!("Failed to read template '{name}': {e}");
            ApiErrorResponse::internal(read_failed).into_json_tuple()
        }
    }
}

/// GET /api/templates/:name/toml — Get the raw TOML content of a template.
#[utoipa::path(get, path = "/api/templates/{name}/toml", tag = "system", operation_id = "get_agent_template_toml", params(("name" = String, Path, description = "Template name")), responses((status = 200, description = "Template TOML content as plain text", body = String)))]
pub async fn get_agent_template_toml(
    Path(name): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let lang = super::resolve_lang(lang.as_ref());
    let (not_found, _invalid_manifest, read_failed) = template_error_messages(lang, &name);

    if validate_template_name(&name).is_err() {
        return (
            StatusCode::NOT_FOUND,
            [(axum::http::header::CONTENT_TYPE, "text/plain")],
            not_found,
        )
            .into_response();
    }

    match read_agent_type(&name).await {
        Ok(Some((_source, content))) => (
            StatusCode::OK,
            [(
                axum::http::header::CONTENT_TYPE,
                "text/plain; charset=utf-8",
            )],
            content,
        )
            .into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            [(axum::http::header::CONTENT_TYPE, "text/plain")],
            not_found,
        )
            .into_response(),
        Err(e) => {
            tracing::warn!("Failed to read template '{name}': {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(axum::http::header::CONTENT_TYPE, "text/plain")],
                read_failed,
            )
                .into_response()
        }
    }
}

/// PUT /api/templates/:name/toml — Replace the template manifest with raw TOML.
///
/// Accepts the full manifest as `text/plain` TOML. Parses, validates, and persists it.
/// This is the full-manifest counterpart of the flat-shape `PUT /templates/{name}`.
///
/// Three contracts worth stating before touching this handler:
///
/// - **Identity is pinned to the URL.** The document's `name` key is overwritten with the path segment, so an operator who edits `name = "…"` in the raw-TOML tab gets a 200 whose response carries the URL's name — the same deliberate pin the flat `PUT /templates/{name}` makes.
///   It is asserted by `toml_put_pins_the_name_to_the_url_rather_than_the_body`.
/// - **Keys the manifest does not recognize are reported, not dropped in silence.**
///   `AgentManifest` is `#[serde(default)]` without `deny_unknown_fields` (forward compatibility with manifests written by a newer daemon), so a typo like `sesion_mode` would otherwise parse cleanly, persist, and vanish from the file.
///   The handler round-trips the submitted document through `AgentManifest` and reports any top-level key that did not survive as `unknown_keys` in the 200 response, with a `WARN` log naming the template.
/// - **Every save is snapshotted into the version history**, with the change source `"toml"`, the same way create and the flat `PUT` snapshot theirs.
///   A write path that skips the snapshot makes `GET /api/templates/{name}/history` report the previous save as current while the file on disk is something else, and a history that is silently incomplete is worse than none because nothing distinguishes the two.
#[utoipa::path(put, path = "/api/templates/{name}/toml", tag = "system", operation_id = "put_agent_template_toml", params(("name" = String, Path, description = "Template name")), request_body(content = String, content_type = "text/plain"), responses((status = 200, description = "Template updated", body = crate::types::JsonObject), (status = 400, description = "Invalid TOML"), (status = 404, description = "No such agent type"), (status = 409, description = "Name belongs to a live agent")))]
pub async fn put_agent_template_toml(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    body: String,
) -> impl IntoResponse {
    let lang = super::resolve_lang(lang.as_ref());
    // `ErrorTranslator` is `!Send`, so every message this handler might need is rendered and the
    // translator dropped before the first `.await` — see the note in the root `CLAUDE.md`.
    let (not_found, invalid_manifest, ..) = template_error_messages(lang, &name);
    let (invalid_name, managed_elsewhere, manifest_too_large) = {
        let t = ErrorTranslator::new(lang);
        (
            t.t("api-error-template-invalid-name"),
            t.t_args("api-error-agent-type-not-editable", &[("name", &name)]),
            t.t("api-error-manifest-too-large"),
        )
    };

    if validate_template_name(&name).is_err() {
        return ApiErrorResponse::bad_request(invalid_name)
            .with_code("template_invalid_name")
            .into_json_tuple();
    }

    if !agent_type_path(&name).exists() {
        return if workspace_agent_manifest_path(&name).exists() {
            ApiErrorResponse::conflict(managed_elsewhere)
                .with_code("template_not_editable")
                .into_json_tuple()
        } else {
            ApiErrorResponse::not_found(not_found)
                .with_code("template_not_found")
                .into_json_tuple()
        };
    }

    // Size guard — the same cap the agent spawn path enforces (routes/agents/lifecycle.rs).
    // The global RequestBodyLimitLayer uses the operator-configurable max_request_body_bytes,
    // which may be raised for file uploads and is explicitly not the manifest cap.
    if body.len() > MAX_MANIFEST_SIZE {
        return ApiErrorResponse::bad_request(manifest_too_large)
            .with_code("template_manifest_too_large")
            .into_json_tuple();
    }

    // Parse twice: once into the generic document (for the unknown-key report) and once into
    // AgentManifest (for validation). `AgentManifest` is #[serde(default)] with no
    // deny_unknown_fields, so a typo like `sesion_mode` parses cleanly and would otherwise be
    // dropped from the file by the persist re-serialization without a word.
    let doc: toml::Value = match toml::from_str(&body) {
        Ok(v) => v,
        Err(e) => {
            return ApiErrorResponse::bad_request(invalid_manifest)
                .with_code("template_invalid_toml")
                .with_details(serde_json::json!({ "toml_error": e.to_string() }))
                .into_json_tuple()
        }
    };

    let mut manifest: AgentManifest = match toml::from_str(&body) {
        Ok(m) => m,
        Err(e) => {
            return ApiErrorResponse::bad_request(invalid_manifest)
                .with_details(serde_json::json!({ "toml_error": e.to_string() }))
                .with_code("template_invalid_toml")
                .into_json_tuple()
        }
    };

    manifest.name = name.clone();

    // Report what would be silently dropped. The response stays 200 — the recognized
    // document did save — but the caller sees exactly which keys went missing.
    let unknown_keys = unrecognized_manifest_keys(&doc, &manifest);
    if !unknown_keys.is_empty() {
        tracing::warn!(
            "agent type '{name}': submitted TOML carries keys AgentManifest does not recognize, which this save will drop: {unknown_keys:?}"
        );
    }

    match persist_agent_type(&name, &manifest) {
        Ok(rendered) => {
            record_template_version(&state, &name, &rendered, "toml");
            let mut detail =
                agent_type_detail(&name, TemplateSource::AgentType, &manifest, &rendered);
            if !unknown_keys.is_empty() {
                detail["unknown_keys"] = serde_json::json!(unknown_keys);
            }
            (StatusCode::OK, Json(detail))
        }
        Err(e) => {
            tracing::error!("{e}");
            ApiErrorResponse::internal_scrub(e).into_json_tuple()
        }
    }
}

// ---------------------------------------------------------------------------
// Write endpoints (#7740)
// ---------------------------------------------------------------------------

// Serializing a manifest over an existing agent type, and creating a new one, both live in `librefang_types::agent_type_store`.
// The `agent_type_create` tool (#7722) writes into the same directory, so the atomic rename, the `File::create_new` claim and the live-agent shadow check are shared rather than reimplemented per surface — which is the divergence that made the pre-#7740 design lose data on one path while the other was correct.
use librefang_types::agent_type_store::{
    create_agent_type as store_create, persist_agent_type, CreateAgentTypeError,
};

/// POST /api/templates — Create an operator-authored agent type.
///
/// The body is the flat editor shape; `name` is required here because there is no URL segment to take it from.
#[utoipa::path(post, path = "/api/templates", tag = "system", operation_id = "create_agent_type", request_body = crate::types::JsonObject, responses((status = 201, description = "Agent type created", body = crate::types::JsonObject), (status = 400, description = "Missing or invalid name"), (status = 409, description = "Name already taken by an agent type or a live agent")))]
pub async fn create_agent_type(
    State(state): State<Arc<AppState>>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(spec): Json<librefang_types::agent_type::AgentTypeSpec>,
) -> impl IntoResponse {
    let lang = super::resolve_lang(lang.as_ref());
    // `name` is required here because there is no URL segment to take it from; an absent one falls through to the store's name rule and comes back as `InvalidName`.
    let name = spec.name.clone().unwrap_or_default();
    let (invalid_name, exists, shadow) = {
        let t = ErrorTranslator::new(lang);
        (
            t.t("api-error-template-invalid-name"),
            t.t_args("api-error-agent-type-exists", &[("name", &name)]),
            t.t_args("api-error-agent-type-name-taken", &[("name", &name)]),
        )
    };

    match store_create(&name, spec) {
        Ok(created) => {
            record_template_version(&state, &created.name, &created.manifest_toml, "create");
            (
                StatusCode::CREATED,
                Json(agent_type_detail(
                    &created.name,
                    TemplateSource::AgentType,
                    &created.manifest,
                    &created.manifest_toml,
                )),
            )
        }
        Err(CreateAgentTypeError::InvalidName) => ApiErrorResponse::bad_request(invalid_name)
            .with_code("template_invalid_name")
            .into_json_tuple(),
        // A type that shadows a live agent's name would win every subsequent `GET /templates/{name}` and make the agent unreachable through this catalog.
        Err(CreateAgentTypeError::ShadowsLiveAgent) => ApiErrorResponse::conflict(shadow)
            .with_code("template_name_taken")
            .into_json_tuple(),
        Err(CreateAgentTypeError::NameTaken) => ApiErrorResponse::conflict(exists)
            .with_code("template_exists")
            .into_json_tuple(),
        Err(CreateAgentTypeError::Io(e)) => {
            tracing::error!("{e}");
            ApiErrorResponse::internal_scrub(e).into_json_tuple()
        }
    }
}

/// PUT /api/templates/:name — Apply the flat editor shape as a **patch** over the stored manifest.
///
/// This is a read-modify-write, and that is load-bearing rather than incidental (#7740).
/// The flat shape carries seven of `AgentManifest`'s fifty-eight fields.
/// Rebuilding the document from the request body and writing it over the file — which is what the endpoint did on the branch this replaces — resets the other fifty-one to their defaults and answers 200: `[[triggers]]`, `[compaction]`, `max_history_messages`, `mcp_servers`, `tool_allowlist`, `session_mode`, `[workspaces]`, `channels`, `[exec_policy]` and `fallback_models` all disappear the first time anyone opens the editor and saves.
///
/// So the stored manifest is parsed first and the body applied over it.
/// A key the client did not send leaves its field untouched; a key it did send is written through verbatim, including an empty string, because "the operator cleared this" and "the client did not mention it" are different instructions and only a typed `Option` can tell them apart.
#[utoipa::path(put, path = "/api/templates/{name}", tag = "system", operation_id = "update_agent_type", params(("name" = String, Path, description = "Agent type name")), request_body = crate::types::JsonObject, responses((status = 200, description = "Agent type updated", body = crate::types::JsonObject), (status = 404, description = "No such agent type"), (status = 409, description = "The name belongs to a live agent, which is edited through /api/agents")))]
pub async fn update_agent_type(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(mut spec): Json<librefang_types::agent_type::AgentTypeSpec>,
) -> impl IntoResponse {
    let lang = super::resolve_lang(lang.as_ref());
    let (not_found, invalid_manifest, read_failed) = template_error_messages(lang, &name);
    let (invalid_name, managed_elsewhere) = {
        let t = ErrorTranslator::new(lang);
        (
            t.t("api-error-template-invalid-name"),
            t.t_args("api-error-agent-type-not-editable", &[("name", &name)]),
        )
    };

    if validate_template_name(&name).is_err() {
        return ApiErrorResponse::bad_request(invalid_name)
            .with_code("template_invalid_name")
            .into_json_tuple();
    }
    // The URL segment is the identity; a `name` in the body would rename the row out from under the
    // path that addressed it and leave the file name and the manifest's own `name` disagreeing.
    spec.name = None;

    let stored = match tokio::fs::read_to_string(agent_type_path(&name)).await {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Distinguish "no such agent type" from "that name is a live agent". The second is the
            // dual-source gap #7731 asks about, and answering a bare 404 for it tells the operator
            // nothing about why the row they can see refuses to save.
            return if workspace_agent_manifest_path(&name).exists() {
                ApiErrorResponse::conflict(managed_elsewhere)
                    .with_code("template_not_editable")
                    .into_json_tuple()
            } else {
                ApiErrorResponse::not_found(not_found)
                    .with_code("template_not_found")
                    .into_json_tuple()
            };
        }
        Err(e) => {
            tracing::warn!("Failed to read agent type '{name}': {e}");
            return ApiErrorResponse::internal(read_failed).into_json_tuple();
        }
    };

    let mut manifest: AgentManifest = match toml::from_str(&stored) {
        Ok(manifest) => manifest,
        Err(e) => {
            // Refuse rather than fall back to a blank manifest: overwriting a file we could not
            // parse is exactly the data loss this handler exists to prevent.
            tracing::warn!("Invalid agent type manifest for '{name}': {e}");
            return ApiErrorResponse::internal(invalid_manifest).into_json_tuple();
        }
    };

    spec.apply_to(&mut manifest);
    manifest.name = name.clone();

    match persist_agent_type(&name, &manifest) {
        Ok(rendered) => {
            record_template_version(&state, &name, &rendered, "dashboard");
            (
                StatusCode::OK,
                Json(agent_type_detail(
                    &name,
                    TemplateSource::AgentType,
                    &manifest,
                    &rendered,
                )),
            )
        }
        Err(e) => {
            tracing::error!("{e}");
            ApiErrorResponse::internal_scrub(e).into_json_tuple()
        }
    }
}

/// DELETE /api/templates/:name — Remove an operator-authored agent type.
///
/// Only reaches `agent-types/`. A name that resolves to a live agent is refused with the same
/// managed-elsewhere conflict `PUT` uses — deleting an agent is `DELETE /api/agents/{id}`.
#[utoipa::path(delete, path = "/api/templates/{name}", tag = "system", operation_id = "delete_agent_type", params(("name" = String, Path, description = "Agent type name")), responses((status = 200, description = "Agent type deleted", body = crate::types::JsonObject), (status = 404, description = "No such agent type"), (status = 409, description = "The name belongs to a live agent, which is deleted through /api/agents")))]
pub async fn delete_agent_type(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let lang = super::resolve_lang(lang.as_ref());
    let (invalid_name, not_found, managed_elsewhere) = {
        let t = ErrorTranslator::new(lang);
        (
            t.t("api-error-template-invalid-name"),
            t.t_args("api-error-template-not-found", &[("name", &name)]),
            t.t_args("api-error-agent-type-not-editable", &[("name", &name)]),
        )
    };

    if validate_template_name(&name).is_err() {
        return ApiErrorResponse::bad_request(invalid_name)
            .with_code("template_invalid_name")
            .into_json_tuple();
    }

    match tokio::fs::remove_file(agent_type_path(&name)).await {
        Ok(()) => {
            // Best-effort cascade: delete version history for the removed template.
            let store =
                librefang_memory::TemplateVersionStore::new(state.kernel.memory_substrate().pool());
            if let Err(e) = store.delete_for_template(&name) {
                tracing::warn!(template = %name, error = %e, "Failed to cascade-delete template version history");
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({ "deleted": true, "name": name })),
            )
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if workspace_agent_manifest_path(&name).exists() {
                ApiErrorResponse::conflict(managed_elsewhere)
                    .with_code("template_not_editable")
                    .into_json_tuple()
            } else {
                ApiErrorResponse::not_found(not_found)
                    .with_code("template_not_found")
                    .into_json_tuple()
            }
        }
        Err(e) => {
            tracing::error!("Failed to delete agent type '{name}': {e}");
            ApiErrorResponse::internal_scrub(e).into_json_tuple()
        }
    }
}

// ---------------------------------------------------------------------------
// Promote to registry (#8043)
// ---------------------------------------------------------------------------

/// POST /api/templates/:name/promote — open a PR contributing this agent
/// type to the configured public registry.
///
/// Runs the privacy scan first and refuses to publish when a finding sits
/// inside a field the sanitiser keeps (`removed_by_sanitizer == false`),
/// so a credential or private endpoint pasted into free text cannot reach
/// a public git history. Then it sanitizes the manifest (stripping private
/// fields), renders it as TOML, forks the registry repo, pushes the file to
/// `agent-types/<name>/agent.toml`, and opens a pull request.
/// Requires `GITHUB_TOKEN` (env or vault).
#[utoipa::path(
    post,
    path = "/api/templates/{name}/promote",
    tag = "system",
    operation_id = "promote_agent_type",
    params(("name" = String, Path, description = "Agent type name")),
    responses(
        (status = 200, description = "PR opened against the registry", body = crate::types::JsonObject),
        (status = 400, description = "Invalid manifest or name"),
        (status = 401, description = "No GitHub token configured"),
        (status = 404, description = "Agent type not found"),
        (status = 409, description = "Manifest still contains private details that require review"),
        (status = 502, description = "GitHub request failed")
    )
)]
pub async fn promote_agent_type(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let lang = super::resolve_lang(lang.as_ref());
    let (not_found, invalid_manifest, read_failed) = template_error_messages(lang, &name);
    let (no_token, review_required) = promote_error_messages(lang);

    if validate_template_name(&name).is_err() {
        return ApiErrorResponse::not_found(not_found).into_json_tuple();
    }

    // Read the manifest.
    let manifest = match read_agent_type(&name).await {
        Ok(Some((_source, content))) => match toml::from_str::<AgentManifest>(&content) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("Invalid template manifest for '{name}': {e}");
                return ApiErrorResponse::internal(invalid_manifest).into_json_tuple();
            }
        },
        Ok(None) => return ApiErrorResponse::not_found(not_found).into_json_tuple(),
        Err(e) => {
            tracing::warn!("Failed to read template '{name}': {e}");
            return ApiErrorResponse::internal(read_failed).into_json_tuple();
        }
    };

    // Privacy gate: refuse to publish while any finding sits inside a field
    // the sanitiser keeps (`removed_by_sanitizer == false`), because that is
    // material the operator has to edit by hand — a credential or private
    // endpoint pasted into a system prompt or description. Findings the
    // sanitiser already strips are fine: publishing drops them. This is the
    // server-side half of `promotion_preview`, so the advisory hint and the
    // endpoint agree.
    let findings = librefang_types::manifest_privacy::scan_for_publication(&manifest);
    if findings.iter().any(|finding| !finding.removed_by_sanitizer) {
        return ApiErrorResponse::conflict(review_required)
            .with_code("review_required")
            .with_details(serde_json::json!({ "findings": findings }))
            .into_json_tuple();
    }

    // Token check.
    let Some(token) = super::skills::resolve_github_token(&state) else {
        return ApiErrorResponse::unauthorized(no_token).into_json_tuple();
    };

    // Sanitize for publication and render.
    let publishable = librefang_types::manifest_privacy::sanitize_for_publication(&manifest);
    let manifest_toml = match toml::to_string_pretty(&publishable) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("Could not render publishable manifest for '{name}': {e}");
            let render_failed = {
                let t = ErrorTranslator::new(lang);
                t.t_args(
                    "api-error-template-promote-render-failed",
                    &[("error", &e.to_string())],
                )
            };
            return ApiErrorResponse::bad_request(render_failed).into_json_tuple();
        }
    };

    let registry_repo = state
        .kernel
        .config_snapshot()
        .skills
        .registry_repo
        .clone()
        .filter(|r| !r.trim().is_empty())
        .unwrap_or_else(|| librefang_skills::registry_pr::DEFAULT_REGISTRY_REPO.to_string());

    // Build the PR body.
    let mut body = format!("Contributes the `{name}` agent type to the registry.\n\n");
    if !publishable.description.is_empty() {
        body.push_str(&format!("- **Description**: {}\n", publishable.description));
    }
    body.push_str(&format!(
        "- **Provider**: {}\n- **Model**: {}\n",
        publishable.model.provider, publishable.model.model
    ));
    if !publishable.tags.is_empty() {
        body.push_str(&format!("- **Tags**: {}\n", publishable.tags.join(", ")));
    }

    let req = librefang_skills::registry_pr::GenericProposeRequest {
        name: &name,
        registry_repo: &registry_repo,
        token: &token,
        prefix: "agent-types",
        files: vec![("agent.toml".to_string(), manifest_toml.into_bytes())],
        pr_title: format!("agent-type: contribute `{name}`"),
        pr_body: body,
    };

    match librefang_skills::registry_pr::propose_files_to_registry(req).await {
        Ok(pr) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "pr_url": pr.pr_url,
                "repo": pr.repo,
                "branch": pr.branch,
            })),
        ),
        Err(librefang_skills::SkillError::SecurityBlocked(msg)) => {
            ApiErrorResponse::unauthorized(msg).into_json_tuple()
        }
        Err(librefang_skills::SkillError::InvalidManifest(msg)) => {
            ApiErrorResponse::bad_request(msg).into_json_tuple()
        }
        Err(librefang_skills::SkillError::NotFound(msg)) => {
            ApiErrorResponse::not_found(msg).into_json_tuple()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

// ---------------------------------------------------------------------------
// Template version history
// ---------------------------------------------------------------------------

/// Best-effort version snapshot — a recording failure must not block the
/// create/update that triggered it.
fn record_template_version(state: &AppState, name: &str, toml: &str, source: &str) {
    let store = librefang_memory::TemplateVersionStore::new(state.kernel.memory_substrate().pool());
    if let Err(e) = store.record_version(name, toml, source) {
        tracing::warn!(
            template = %name,
            error = %e,
            "Failed to record template version snapshot"
        );
    }
}

/// GET /api/templates/{name}/history — how this template's config changed over time.
#[utoipa::path(
    get,
    path = "/api/templates/{name}/history",
    tag = "system",
    operation_id = "list_template_history",
    params(
        ("name" = String, Path, description = "Template name"),
        ("limit" = Option<u32>, Query, description = "Max entries (default 30, max 200)")
    ),
    responses(
        (status = 200, description = "Template version history", body = crate::types::JsonObject),
        (status = 400, description = "Invalid template name"),
        (status = 404, description = "Template not found")
    )
)]
pub async fn list_template_history(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    if validate_template_name(&name).is_err() {
        return ApiErrorResponse::bad_request("invalid template name")
            .with_code("template_invalid_name")
            .into_json_tuple();
    }

    // Verify the template exists on disk (either source).
    if read_agent_type(&name).await.ok().flatten().is_none() {
        return ApiErrorResponse::not_found(format!("Template '{name}' not found"))
            .with_code("template_not_found")
            .into_json_tuple();
    }

    let limit = params
        .get("limit")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(30)
        .min(200) as usize;

    let store = librefang_memory::TemplateVersionStore::new(state.kernel.memory_substrate().pool());
    match store.list_for_template(&name, limit) {
        Ok(versions) => {
            let items: Vec<serde_json::Value> = versions
                .iter()
                .map(|v| {
                    serde_json::json!({
                        "id": v.id,
                        "template_name": v.template_name,
                        "timestamp": v.timestamp,
                        "manifest_toml": v.manifest_toml,
                        "change_source": v.change_source,
                    })
                })
                .collect();
            (
                StatusCode::OK,
                Json(serde_json::json!({ "versions": items })),
            )
        }
        Err(e) => ApiErrorResponse::internal_scrub(e).into_json_tuple(),
    }
}

/// POST /api/templates/{name}/history/{version_id}/restore — restore a template to a prior version.
#[utoipa::path(
    post,
    path = "/api/templates/{name}/history/{version_id}/restore",
    tag = "system",
    operation_id = "restore_template_version",
    params(
        ("name" = String, Path, description = "Template name"),
        ("version_id" = i64, Path, description = "Version row id to restore")
    ),
    responses(
        (status = 200, description = "Template restored", body = crate::types::JsonObject),
        (status = 400, description = "Invalid template name or version id"),
        (status = 404, description = "Template or version not found"),
        (status = 409, description = "Template belongs to a live agent")
    )
)]
pub async fn restore_template_version(
    State(state): State<Arc<AppState>>,
    Path((name, version_id)): Path<(String, i64)>,
) -> impl IntoResponse {
    if validate_template_name(&name).is_err() {
        return ApiErrorResponse::bad_request("invalid template name")
            .with_code("template_invalid_name")
            .into_json_tuple();
    }

    // Only operator-authored templates can be restored (not live agents).
    if !agent_type_path(&name).exists() {
        if workspace_agent_manifest_path(&name).exists() {
            return ApiErrorResponse::conflict(
                "that name belongs to a live agent; restore it through /api/agents",
            )
            .with_code("template_not_editable")
            .into_json_tuple();
        }
        return ApiErrorResponse::not_found(format!("Template '{name}' not found"))
            .with_code("template_not_found")
            .into_json_tuple();
    }

    let store = librefang_memory::TemplateVersionStore::new(state.kernel.memory_substrate().pool());

    let version = match store.get_version(version_id) {
        Ok(Some(v)) if v.template_name == name => v,
        Ok(Some(_)) => {
            return ApiErrorResponse::bad_request("version does not belong to this template")
                .with_code("version_mismatch")
                .into_json_tuple();
        }
        Ok(None) => {
            return ApiErrorResponse::not_found("version not found")
                .with_code("version_not_found")
                .into_json_tuple();
        }
        Err(e) => return ApiErrorResponse::internal_scrub(e).into_json_tuple(),
    };

    // Parse the stored TOML to validate it before writing.
    let manifest: AgentManifest = match toml::from_str(&version.manifest_toml) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(template = %name, version_id, "Stored version TOML is unparseable: {e}");
            return ApiErrorResponse::internal("stored version is corrupt and cannot be restored")
                .into_json_tuple();
        }
    };

    match persist_agent_type(&name, &manifest) {
        Ok(rendered) => {
            record_template_version(&state, &name, &rendered, "restore");
            (
                StatusCode::OK,
                Json(agent_type_detail(
                    &name,
                    TemplateSource::AgentType,
                    &manifest,
                    &rendered,
                )),
            )
        }
        Err(e) => {
            tracing::error!("{e}");
            ApiErrorResponse::internal_scrub(e).into_json_tuple()
        }
    }
}

#[cfg(test)]
mod template_loading_tests {
    use super::{load_agent_templates, load_agent_type_files};

    #[tokio::test]
    async fn missing_template_directory_is_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let templates = load_agent_templates(&tmp.path().join("missing"))
            .await
            .unwrap();
        assert!(templates.is_empty());
    }

    #[tokio::test]
    async fn template_directory_read_errors_are_propagated() {
        let tmp = tempfile::tempdir().unwrap();
        let not_a_directory = tmp.path().join("agents");
        tokio::fs::write(&not_a_directory, "file").await.unwrap();

        let error = load_agent_templates(&not_a_directory).await.unwrap_err();
        assert!(error.contains("failed to list agent templates"));
    }

    #[tokio::test]
    async fn missing_agent_types_directory_is_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let found = load_agent_type_files(&tmp.path().join("missing"))
            .await
            .unwrap();
        assert!(found.is_empty());
    }

    #[tokio::test]
    async fn agent_type_loader_skips_non_toml_and_unparseable_files() {
        let tmp = tempfile::tempdir().unwrap();
        tokio::fs::write(tmp.path().join("good.toml"), "name = \"good\"\n")
            .await
            .unwrap();
        // The staging file `atomic_write` leaves behind if a process dies mid-rename.
        tokio::fs::write(
            tmp.path().join("good.toml.1234.0.tmp"),
            "name = \"partial\"",
        )
        .await
        .unwrap();
        tokio::fs::write(tmp.path().join("broken.toml"), "name = [[[")
            .await
            .unwrap();
        // A name the detail route would reject, so the catalog must not advertise it.
        tokio::fs::write(tmp.path().join("has space.toml"), "name = \"x\"\n")
            .await
            .unwrap();

        let found = load_agent_type_files(tmp.path()).await.unwrap();
        let names: Vec<&str> = found.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, vec!["good"]);
    }
}
