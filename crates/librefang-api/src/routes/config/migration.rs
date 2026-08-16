use super::*;

enum MigrationTaskError {
    Validation(crate::validation::ValidationError),
    Migration(String),
    Relocation(String),
}

fn relocate_migrated_agent_dirs(
    target_dir: &std::path::Path,
    workspaces_agents_dir: &std::path::Path,
) -> Result<Vec<(std::path::PathBuf, std::path::PathBuf)>, String> {
    let legacy_agents = target_dir.join("agents");
    if legacy_agents == workspaces_agents_dir {
        return Ok(Vec::new());
    }
    if !legacy_agents.is_dir() {
        return Ok(Vec::new());
    }

    let entries = std::fs::read_dir(&legacy_agents)
        .map_err(|error| format!("read {}: {error}", legacy_agents.display()))?;
    let mut moved = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "read directory entry in {}: {error}",
                legacy_agents.display()
            )
        })?;
        let source = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("inspect {}: {error}", source.display()))?;
        if !file_type.is_dir() || !source.join("agent.toml").is_file() {
            continue;
        }

        let destination = workspaces_agents_dir.join(entry.file_name());
        if destination.exists() {
            return Err(format!(
                "cannot relocate {} because {} already exists",
                source.display(),
                destination.display()
            ));
        }
        std::fs::create_dir_all(workspaces_agents_dir)
            .map_err(|error| format!("create {}: {error}", workspaces_agents_dir.display()))?;
        std::fs::rename(&source, &destination).map_err(|error| {
            format!(
                "relocate {} to {}: {error}",
                source.display(),
                destination.display()
            )
        })?;
        moved.push((source, destination));
    }

    match std::fs::remove_dir(&legacy_agents) {
        Ok(()) => {}
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
            ) => {}
        Err(error) => {
            return Err(format!("remove {}: {error}", legacy_agents.display()));
        }
    }
    Ok(moved)
}

fn rewrite_relocated_agent_destinations(
    report: &mut librefang_import::report::MigrationReport,
    moved: &[(std::path::PathBuf, std::path::PathBuf)],
) {
    for item in &mut report.imported {
        for (source, destination) in moved {
            if std::path::Path::new(&item.destination) == source.join("agent.toml") {
                item.destination = destination.join("agent.toml").display().to_string();
                break;
            }
        }
    }
}

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
    let detection = tokio::task::spawn_blocking(|| {
        // Check OpenClaw first.
        if let Some(path) = librefang_import::openclaw::detect_openclaw_home() {
            let scan = librefang_import::openclaw::scan_openclaw_workspace(&path);
            return serde_json::json!({
                "detected": true,
                "source": "openclaw",
                "path": path.display().to_string(),
                "scan": scan,
            });
        }

        // Check OpenFang.
        if let Some(home) = dirs::home_dir() {
            let openfang_path = home.join(".openfang");
            if openfang_path.is_dir() {
                return serde_json::json!({
                    "detected": true,
                    "source": "openfang",
                    "path": openfang_path.display().to_string(),
                    "scan": null,
                });
            }
        }

        serde_json::json!({
            "detected": false,
            "source": null,
            "path": null,
            "scan": null,
        })
    })
    .await;

    match detection {
        Ok(body) => (StatusCode::OK, Json(body)),
        Err(error) => {
            ApiErrorResponse::internal_scrub(format!("migration detection task failed: {error}"))
                .into_json_tuple()
        }
    }
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
    let requested_path = req.path;
    let scan = tokio::task::spawn_blocking(move || {
        let allowed_roots: Vec<&std::path::Path> =
            source_roots.iter().map(|path| path.as_path()).collect();
        let path = crate::validation::validate_path_containment(
            "path",
            std::path::Path::new(requested_path.trim()),
            &allowed_roots,
            true,
        )?;
        Ok::<_, crate::validation::ValidationError>(
            librefang_import::openclaw::scan_openclaw_workspace(&path),
        )
    })
    .await;

    match scan {
        Ok(Ok(scan)) => (StatusCode::OK, Json(serde_json::json!(scan))),
        Ok(Err(error)) => migration_validation_error(error),
        Err(error) => {
            ApiErrorResponse::internal_scrub(format!("migration scan task failed: {error}"))
                .into_json_tuple()
        }
    }
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
    let requested_source = req.source_dir;
    let requested_target = req.target_dir;
    let dry_run = req.dry_run;
    let workspaces_agents_dir = state
        .kernel
        .config_snapshot()
        .effective_agent_workspaces_dir();

    let migration = tokio::task::spawn_blocking(move || {
        let source_allowed: Vec<&std::path::Path> =
            source_roots.iter().map(|path| path.as_path()).collect();
        let target_allowed: Vec<&std::path::Path> = vec![home_dir.as_path()];
        let source_dir = crate::validation::validate_path_containment(
            "source_dir",
            std::path::Path::new(requested_source.trim()),
            &source_allowed,
            true,
        )
        .map_err(MigrationTaskError::Validation)?;
        let target_dir = if requested_target.trim().is_empty() {
            home_dir
        } else {
            crate::validation::validate_path_containment(
                "target_dir",
                std::path::Path::new(requested_target.trim()),
                &target_allowed,
                false,
            )
            .map_err(MigrationTaskError::Validation)?
        };
        let options = librefang_import::MigrateOptions {
            source,
            source_dir,
            target_dir: target_dir.clone(),
            dry_run,
        };
        let mut report = librefang_import::run_migration(&options)
            .map_err(|error| MigrationTaskError::Migration(error.to_string()))?;
        if !dry_run {
            let moved = relocate_migrated_agent_dirs(&target_dir, &workspaces_agents_dir)
                .map_err(MigrationTaskError::Relocation)?;
            rewrite_relocated_agent_destinations(&mut report, &moved);
        }
        Ok::<_, MigrationTaskError>(report)
    })
    .await;

    match migration {
        Ok(Ok(report)) => {
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
                    "dry_run": dry_run,
                    "imported": imported,
                    "imported_count": imported.len(),
                    "skipped": skipped,
                    "skipped_count": skipped.len(),
                    "warnings": report.warnings,
                    "report_markdown": report.to_markdown(),
                })),
            )
        }
        Ok(Err(MigrationTaskError::Validation(error))) => migration_validation_error(error),
        Ok(Err(MigrationTaskError::Migration(error)))
        | Ok(Err(MigrationTaskError::Relocation(error))) => {
            ApiErrorResponse::internal_scrub(error).into_json_tuple()
        }
        Err(error) => ApiErrorResponse::internal_scrub(format!("migration task failed: {error}"))
            .into_json_tuple(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relocation_uses_actual_migration_target_and_rewrites_report() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("custom-target");
        let source_agent = target.join("agents").join("main");
        let canonical_agents = dir.path().join("workspaces").join("agents");
        std::fs::create_dir_all(&source_agent).unwrap();
        std::fs::write(source_agent.join("agent.toml"), "name = \"main\"\n").unwrap();
        let mut report = librefang_import::report::MigrationReport::default();
        report.imported.push(librefang_import::report::MigrateItem {
            kind: librefang_import::report::ItemKind::Agent,
            name: "main".to_string(),
            destination: source_agent.join("agent.toml").display().to_string(),
        });

        let moved = relocate_migrated_agent_dirs(&target, &canonical_agents).unwrap();
        rewrite_relocated_agent_destinations(&mut report, &moved);

        let canonical_manifest = canonical_agents.join("main").join("agent.toml");
        assert!(canonical_manifest.is_file());
        assert!(!source_agent.exists());
        assert_eq!(
            report.imported[0].destination,
            canonical_manifest.display().to_string()
        );
    }

    #[test]
    fn relocation_is_a_noop_when_target_already_uses_canonical_layout() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("workspaces");
        let canonical_agents = target.join("agents");
        let manifest = canonical_agents.join("main").join("agent.toml");
        std::fs::create_dir_all(manifest.parent().unwrap()).unwrap();
        std::fs::write(&manifest, "name = \"main\"\n").unwrap();

        let moved = relocate_migrated_agent_dirs(&target, &canonical_agents).unwrap();

        assert!(moved.is_empty());
        assert!(manifest.is_file());
    }

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
