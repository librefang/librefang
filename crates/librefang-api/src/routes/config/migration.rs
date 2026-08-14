use super::*;

fn migration_validation_error(
    error: crate::validation::ValidationError,
) -> (StatusCode, Json<serde_json::Value>) {
    let status = error.status;
    if status.is_server_error() {
        return ApiErrorResponse::internal_scrub(error.message)
            .with_status(status)
            .into_json_tuple();
    }

    ApiErrorResponse::bad_request(error.message)
        .with_status(status)
        .into_json_tuple()
}

#[utoipa::path(
    get,
    path = "/api/migrate/detect",
    tag = "system",
    responses(
        (status = 200, description = "Detect migratable framework installation", body = crate::types::JsonObject)
    )
)]
pub async fn migrate_detect() -> impl IntoResponse {
    // Check OpenClaw first
    if let Some(path) = librefang_import::openclaw::detect_openclaw_home() {
        let scan = librefang_import::openclaw::scan_openclaw_workspace(&path);
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "detected": true,
                "source": "openclaw",
                "path": path.display().to_string(),
                "scan": scan,
            })),
        );
    }

    // Check OpenFang
    if let Some(home) = dirs::home_dir() {
        let openfang_path = home.join(".openfang");
        if openfang_path.exists() && openfang_path.is_dir() {
            return (
                StatusCode::OK,
                Json(serde_json::json!({
                    "detected": true,
                    "source": "openfang",
                    "path": openfang_path.display().to_string(),
                    "scan": null,
                })),
            );
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "detected": false,
            "source": null,
            "path": null,
            "scan": null,
        })),
    )
}

/// POST /api/migrate/scan — Scan a specific directory for OpenClaw workspace.
#[utoipa::path(
    post,
    path = "/api/migrate/scan",
    tag = "system",
    responses(
        (status = 200, description = "Scan directory for migratable workspace", body = crate::types::JsonObject)
    )
)]
pub async fn migrate_scan(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MigrateScanRequest>,
) -> impl IntoResponse {
    // SECURITY: same containment policy as `run_migrate` below. Without it,
    // the 200-vs-400 `Directory not found` branch is a `.exists()` oracle
    // for any path readable as the daemon UID — see
    // `docs/issues/migrate-arbitrary-paths.md`. The probe path is the
    // sibling of the write primitive `run_migrate` patches; both endpoints
    // share the same audit-cited threat model and must share the same
    // allowlist: the librefang home plus the known framework source dirs
    // that exist under the OS home (see `migrate_source_roots`).
    let home_dir = state.kernel.home_dir().to_path_buf();
    let source_roots = migrate_source_roots(&home_dir, dirs::home_dir().as_deref());
    let allowed_roots: Vec<&std::path::Path> = source_roots.iter().map(|p| p.as_path()).collect();

    let path = match crate::validation::validate_path_containment(
        "path",
        std::path::Path::new(req.path.trim()),
        &allowed_roots,
        true, // scan target must already exist
    ) {
        Ok(p) => p,
        Err(error) => return migration_validation_error(error),
    };

    let scan = librefang_import::openclaw::scan_openclaw_workspace(&path);
    (StatusCode::OK, Json(serde_json::json!(scan)))
}

/// POST /api/migrate — Run migration from another agent framework.
#[utoipa::path(
    post,
    path = "/api/migrate",
    tag = "system",
    responses(
        (status = 200, description = "Run migration from another agent framework", body = crate::types::JsonObject)
    )
)]
pub async fn run_migrate(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MigrateRequest>,
) -> impl IntoResponse {
    let source = match req.source.as_str() {
        "openclaw" => librefang_import::MigrateSource::OpenClaw,
        "langchain" => librefang_import::MigrateSource::LangChain,
        "autogpt" => librefang_import::MigrateSource::AutoGpt,
        "openfang" => librefang_import::MigrateSource::OpenFang,
        other => {
            return ApiErrorResponse::bad_request(format!(
                "Unknown source: {other}. Use 'openclaw', 'openfang', 'langchain', or 'autogpt'"
            ))
            .into_json_tuple();
        }
    };

    // SECURITY: source_dir and target_dir must canonicalize to a descendant
    // of an allowed root. Without this check, Admin can probe arbitrary
    // filesystem paths via the 200-vs-400 oracle and write under
    // attacker-chosen target directories — see
    // `docs/issues/migrate-arbitrary-paths.md`. Admin is dev/ops, not the
    // trust ceiling; a leaked Admin token MUST NOT become a daemon-UID
    // write primitive.
    //
    // The source allow-list is the librefang home plus the known framework
    // source dirs under the OS home (the documented `~/.openclaw` etc. are
    // siblings of `~/.librefang`, not descendants — #5577 confined both to
    // the librefang home and regressed migrate-from-OpenClaw). The target
    // allow-list stays the librefang home only: reads may come from a source
    // dir, but writes never leave the librefang home.
    let home_dir = state.kernel.home_dir().to_path_buf();
    let source_roots = migrate_source_roots(&home_dir, dirs::home_dir().as_deref());
    let source_allowed: Vec<&std::path::Path> = source_roots.iter().map(|p| p.as_path()).collect();
    let target_allowed: Vec<&std::path::Path> = vec![home_dir.as_path()];

    let source_dir = match crate::validation::validate_path_containment(
        "source_dir",
        std::path::Path::new(req.source_dir.trim()),
        &source_allowed,
        true, // source must already exist
    ) {
        Ok(p) => p,
        Err(error) => return migration_validation_error(error),
    };

    let target_dir = if req.target_dir.trim().is_empty() {
        home_dir.clone()
    } else {
        match crate::validation::validate_path_containment(
            "target_dir",
            std::path::Path::new(req.target_dir.trim()),
            &target_allowed,
            false, // target may not exist yet — migration creates it
        ) {
            Ok(p) => p,
            Err(error) => return migration_validation_error(error),
        }
    };

    let options = librefang_import::MigrateOptions {
        source,
        source_dir,
        target_dir,
        dry_run: req.dry_run,
    };

    match librefang_import::run_migration(&options) {
        Ok(report) => {
            // Migrate writes agent manifests under `<target>/agents/<name>/`
            // (legacy schema). Relocate them into the canonical
            // `workspaces/agents/<name>/` layout immediately so the running
            // daemon can use them without a restart.
            if !req.dry_run {
                state.kernel.relocate_legacy_agent_dirs();
            }

            let imported: Vec<serde_json::Value> = report
                .imported
                .iter()
                .map(|i| {
                    serde_json::json!({
                        "kind": format!("{}", i.kind),
                        "name": i.name,
                        "destination": i.destination,
                    })
                })
                .collect();

            let skipped: Vec<serde_json::Value> = report
                .skipped
                .iter()
                .map(|s| {
                    serde_json::json!({
                        "kind": format!("{}", s.kind),
                        "name": s.name,
                        "reason": s.reason,
                    })
                })
                .collect();

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "completed",
                    "dry_run": req.dry_run,
                    "imported": imported,
                    "imported_count": imported.len(),
                    "skipped": skipped,
                    "skipped_count": skipped.len(),
                    "warnings": report.warnings,
                    "report_markdown": report.to_markdown(),
                })),
            )
        }
        Err(e) => ApiErrorResponse::internal_scrub(e).into_json_tuple(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_validation_preserves_client_errors() {
        let (status, body) = migration_validation_error(
            crate::validation::ValidationError::bad_request("source is outside allowed roots"),
        );

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["message"], "source is outside allowed roots");
    }

    #[test]
    fn migration_validation_preserves_non_400_client_status() {
        let (status, body) = migration_validation_error(
            crate::validation::ValidationError::payload_too_large("migration request too large"),
        );

        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(body["error"]["message"], "migration request too large");
    }

    #[test]
    fn migration_validation_scrubs_internal_paths() {
        let (status, body) = migration_validation_error(crate::validation::ValidationError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: "allowed root '/srv/private' could not be canonicalized".to_string(),
        });

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"]["message"], "Internal server error");
        assert!(!body.to_string().contains("/srv/private"));
    }

    #[test]
    fn migration_validation_scrubs_and_preserves_other_server_statuses() {
        let (status, body) = migration_validation_error(crate::validation::ValidationError {
            status: StatusCode::BAD_GATEWAY,
            message: "upstream path '/srv/private' failed".to_string(),
        });

        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(body["error"]["message"], "Internal server error");
        assert!(!body.to_string().contains("/srv/private"));
    }
}
