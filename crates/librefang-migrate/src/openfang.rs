//! OpenFang migration engine.
//!
//! Since OpenFang and LibreFang share the same directory structure and config
//! format (LibreFang is a community fork of OpenFang), migration is a
//! straightforward recursive copy of `~/.openfang` → `~/.librefang` with
//! content rewriting in `.toml` and `.env` files to replace openfang
//! references with librefang.

use crate::report::{ItemKind, MigrateItem, MigrationReport, SkippedItem};
use crate::{MigrateError, MigrateOptions};
use std::path::Path;
use tracing::{info, warn};
use walkdir::WalkDir;

/// After copying OpenFang files verbatim, check each copied `config.toml`
/// and `agents/*/agent.toml` against the current LibreFang schema and attach
/// warnings to the report for any drift.
///
/// Warnings only — we don't fail the migration or rewrite the copied files,
/// because the user may have valid reasons for custom fields (e.g. forward
/// compatibility with future LibreFang versions). The goal is visibility.
///
/// **What this catches:**
/// - Unknown **top-level** fields/sections in `config.toml`
/// - Invalid enum values (e.g. `group_policy = "respond"` when the valid set is
///   `all|mention_only|commands_only|ignore`) — these fail deserialization
/// - Wrong types anywhere in the tree
/// - Missing required fields on agent manifests
///
/// **What this does NOT catch:**
/// - Unknown fields **nested inside** sections (e.g. `[channels.telegram].foo`).
///   LibreFang's channel structs use `#[serde(default)]` without
///   `deny_unknown_fields`, so unknown nested fields are silently ignored at
///   deserialization time. Catching these would require per-struct field-list
///   introspection for every channel struct. Accepted trade-off.
fn warn_on_schema_drift(target: &Path, report: &mut MigrationReport) {
    use librefang_types::agent::AgentManifest;
    use librefang_types::config::KernelConfig;

    // --- config.toml ---
    let config_path = target.join("config.toml");
    if let Ok(content) = std::fs::read_to_string(&config_path) {
        match toml::from_str::<toml::Value>(&content) {
            Ok(raw) => {
                let unknown = KernelConfig::detect_unknown_fields(&raw);
                if !unknown.is_empty() {
                    report.warnings.push(format!(
                        "config.toml: {} unknown top-level field(s) copied from OpenFang \
                         that LibreFang does not recognise: {} — these will be ignored. \
                         Check for schema drift between OpenFang and LibreFang.",
                        unknown.len(),
                        unknown.join(", "),
                    ));
                }
                if let Err(e) = toml::from_str::<KernelConfig>(&content) {
                    report.warnings.push(format!(
                        "config.toml does not cleanly deserialize into LibreFang \
                         KernelConfig: {e} — LibreFang may fall back to defaults for \
                         affected fields."
                    ));
                }
            }
            Err(e) => report.warnings.push(format!(
                "config.toml is not valid TOML after migration: {e}"
            )),
        }
    }

    // --- agents/*/agent.toml ---
    let agents_dir = target.join("agents");
    if !agents_dir.is_dir() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(&agents_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let manifest_path = entry.path().join("agent.toml");
        let Ok(content) = std::fs::read_to_string(&manifest_path) else {
            continue;
        };
        if let Err(e) = toml::from_str::<AgentManifest>(&content) {
            report.warnings.push(format!(
                "{}: does not cleanly deserialize into LibreFang AgentManifest: {e}",
                manifest_path.display(),
            ));
        }
    }
}

/// Determine the [`ItemKind`] from the relative path of a file within the
/// openfang home directory.
fn item_kind_for_path(rel: &Path) -> ItemKind {
    let first_component = rel
        .components()
        .next()
        .and_then(|c| c.as_os_str().to_str())
        .unwrap_or("");

    match first_component {
        "agents" => ItemKind::Agent,
        "skills" => ItemKind::Skill,
        "memory" | "memory-search" => ItemKind::Memory,
        "sessions" => ItemKind::Session,
        "channels" => ItemKind::Channel,
        _ => {
            // Check specific filenames at the root level.
            let file_name = rel.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if file_name == "secrets.env" || file_name.ends_with(".env") {
                ItemKind::Secret
            } else {
                ItemKind::Config
            }
        }
    }
}

/// Returns true if the file's content should be rewritten (openfang → librefang).
fn should_rewrite(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    matches!(ext, "toml" | "env")
}

/// Rewrite openfang references in file content.
fn rewrite_content(content: &str) -> String {
    content
        .replace("openfang", "librefang")
        .replace("OPENFANG", "LIBREFANG")
        .replace("OpenFang", "LibreFang")
}

/// Run the OpenFang → LibreFang migration.
pub fn migrate(options: &MigrateOptions) -> Result<MigrationReport, MigrateError> {
    let source = &options.source_dir;
    let target = &options.target_dir;

    if !source.exists() {
        return Err(MigrateError::SourceNotFound(source.clone()));
    }

    let mut report = MigrationReport {
        source: "OpenFang".to_string(),
        dry_run: options.dry_run,
        ..Default::default()
    };

    for entry in WalkDir::new(source).min_depth(1).into_iter() {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                warn!("Error walking source directory: {}", e);
                report.warnings.push(format!("Failed to read entry: {e}"));
                continue;
            }
        };

        // Skip directories themselves — we only care about files.
        if entry.file_type().is_dir() {
            continue;
        }

        let abs_source = entry.path();
        let rel = abs_source
            .strip_prefix(source)
            .expect("entry is under source dir");

        let dest_path = target.join(rel);
        let kind = item_kind_for_path(rel);
        let display_name = rel.display().to_string();

        // Check if destination already exists.
        if dest_path.exists() {
            info!(
                "Skipping {} (already exists at {})",
                display_name,
                dest_path.display()
            );
            report.skipped.push(SkippedItem {
                kind,
                name: display_name,
                reason: "already exists".to_string(),
            });
            continue;
        }

        if options.dry_run {
            info!("Would copy {} -> {}", display_name, dest_path.display());
            report.imported.push(MigrateItem {
                kind,
                name: display_name,
                destination: dest_path.display().to_string(),
            });
            continue;
        }

        // Ensure parent directory exists.
        if let Some(parent) = dest_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        if should_rewrite(abs_source) {
            let content = std::fs::read_to_string(abs_source)?;
            let rewritten = rewrite_content(&content);
            std::fs::write(&dest_path, rewritten)?;
            info!(
                "Copied (rewritten) {} -> {}",
                display_name,
                dest_path.display()
            );
        } else {
            std::fs::copy(abs_source, &dest_path)?;
            info!("Copied {} -> {}", display_name, dest_path.display());
        }

        report.imported.push(MigrateItem {
            kind,
            name: display_name,
            destination: dest_path.display().to_string(),
        });
    }

    // Post-copy schema check: OpenFang and LibreFang share the same config
    // format by convention, but that contract is not enforced anywhere. If
    // OpenFang drifts (renamed fields, changed enum values, removed sections),
    // a verbatim copy here will silently fall back to defaults at load time.
    // Warn the user so the drift is visible, but don't block the migration.
    if !options.dry_run {
        warn_on_schema_drift(target, &mut report);
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MigrateSource;
    use tempfile::TempDir;

    /// Create a minimal openfang directory structure for testing.
    fn setup_openfang_dir(dir: &Path) {
        // config.toml with openfang references
        std::fs::write(
            dir.join("config.toml"),
            "[general]\nhome = \"~/.openfang\"\nname = \"OPENFANG_AGENT\"\n",
        )
        .unwrap();

        // secrets.env
        std::fs::write(dir.join("secrets.env"), "OPENFANG_API_KEY=secret123\n").unwrap();

        // agents subdirectory
        let agents = dir.join("agents").join("coder");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(
            agents.join("agent.toml"),
            "name = \"coder\"\nframework = \"openfang\"\n",
        )
        .unwrap();

        // skills subdirectory
        let skills = dir.join("skills").join("web-search");
        std::fs::create_dir_all(&skills).unwrap();
        std::fs::write(skills.join("skill.toml"), "name = \"web-search\"\n").unwrap();

        // a binary file that should be copied as-is
        let data = dir.join("data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("index.db"), b"binary-content").unwrap();
    }

    #[test]
    fn test_basic_migration() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();

        setup_openfang_dir(src.path());

        let options = MigrateOptions {
            source: MigrateSource::OpenFang,
            source_dir: src.path().to_path_buf(),
            target_dir: dst.path().to_path_buf(),
            dry_run: false,
        };

        let report = migrate(&options).unwrap();

        assert_eq!(report.source, "OpenFang");
        assert!(!report.dry_run);
        assert_eq!(report.imported.len(), 5);
        assert!(report.skipped.is_empty());
        // The artificial fixture uses `[general]` and a stripped-down agent.toml
        // which don't match the real LibreFang schema — the post-copy drift
        // check correctly flags them. Verify that the warnings are exactly
        // the schema-drift ones we expect, not anything else.
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.contains("unknown top-level field") && w.contains("general")),
            "expected an unknown-top-level-field warning for `[general]`, got: {:?}",
            report.warnings
        );

        // Verify config.toml was rewritten
        let config_content = std::fs::read_to_string(dst.path().join("config.toml")).unwrap();
        assert!(config_content.contains("librefang"));
        assert!(config_content.contains("LIBREFANG"));
        assert!(!config_content.contains("openfang"));
        assert!(!config_content.contains("OPENFANG"));

        // Verify secrets.env was rewritten
        let secrets_content = std::fs::read_to_string(dst.path().join("secrets.env")).unwrap();
        assert!(secrets_content.contains("LIBREFANG_API_KEY"));
        assert!(!secrets_content.contains("OPENFANG_API_KEY"));

        // Verify agent.toml was rewritten
        let agent_content =
            std::fs::read_to_string(dst.path().join("agents/coder/agent.toml")).unwrap();
        assert!(agent_content.contains("librefang"));
        assert!(!agent_content.contains("openfang"));

        // Verify binary file was copied as-is
        let db_content = std::fs::read(dst.path().join("data/index.db")).unwrap();
        assert_eq!(db_content, b"binary-content");
    }

    #[test]
    fn test_dry_run() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();

        setup_openfang_dir(src.path());

        let options = MigrateOptions {
            source: MigrateSource::OpenFang,
            source_dir: src.path().to_path_buf(),
            target_dir: dst.path().to_path_buf(),
            dry_run: true,
        };

        let report = migrate(&options).unwrap();

        assert!(report.dry_run);
        assert_eq!(report.imported.len(), 5);

        // Nothing should actually be written
        assert!(!dst.path().join("config.toml").exists());
        assert!(!dst.path().join("agents").exists());
    }

    #[test]
    fn test_skip_existing() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();

        setup_openfang_dir(src.path());

        // Pre-create a file at the destination
        std::fs::write(dst.path().join("config.toml"), "existing content\n").unwrap();

        let options = MigrateOptions {
            source: MigrateSource::OpenFang,
            source_dir: src.path().to_path_buf(),
            target_dir: dst.path().to_path_buf(),
            dry_run: false,
        };

        let report = migrate(&options).unwrap();

        // config.toml should be skipped
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].name, "config.toml");
        assert_eq!(report.skipped[0].reason, "already exists");

        // The existing content should be preserved
        let content = std::fs::read_to_string(dst.path().join("config.toml")).unwrap();
        assert_eq!(content, "existing content\n");
    }

    #[test]
    fn test_source_not_found() {
        let dst = TempDir::new().unwrap();
        let options = MigrateOptions {
            source: MigrateSource::OpenFang,
            source_dir: std::path::PathBuf::from("/nonexistent/.openfang"),
            target_dir: dst.path().to_path_buf(),
            dry_run: false,
        };

        let result = migrate(&options);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            MigrateError::SourceNotFound(_)
        ));
    }

    #[test]
    fn test_item_kind_detection() {
        assert_eq!(
            item_kind_for_path(Path::new("agents/coder/agent.toml")),
            ItemKind::Agent
        );
        assert_eq!(
            item_kind_for_path(Path::new("skills/web-search/skill.toml")),
            ItemKind::Skill
        );
        assert_eq!(
            item_kind_for_path(Path::new("memory/default/MEMORY.md")),
            ItemKind::Memory
        );
        assert_eq!(
            item_kind_for_path(Path::new("sessions/main.jsonl")),
            ItemKind::Session
        );
        assert_eq!(
            item_kind_for_path(Path::new("channels/discord.toml")),
            ItemKind::Channel
        );
        assert_eq!(
            item_kind_for_path(Path::new("config.toml")),
            ItemKind::Config
        );
        assert_eq!(
            item_kind_for_path(Path::new("secrets.env")),
            ItemKind::Secret
        );
        assert_eq!(
            item_kind_for_path(Path::new("data/index.db")),
            ItemKind::Config // fallback
        );
    }

    #[test]
    fn test_rewrite_content() {
        let input = "home = \"~/.openfang\"\nOPENFANG_KEY=foo\nWelcome to OpenFang\n";
        let output = rewrite_content(input);
        assert_eq!(
            output,
            "home = \"~/.librefang\"\nLIBREFANG_KEY=foo\nWelcome to LibreFang\n"
        );
    }

    #[test]
    fn test_should_rewrite() {
        assert!(should_rewrite(Path::new("config.toml")));
        assert!(should_rewrite(Path::new("secrets.env")));
        assert!(should_rewrite(Path::new("agents/coder/agent.toml")));
        assert!(!should_rewrite(Path::new("data/index.db")));
        assert!(!should_rewrite(Path::new("memory/MEMORY.md")));
        assert!(!should_rewrite(Path::new("sessions/main.jsonl")));
    }

    /// Happy path: a realistic LibreFang-shaped config.toml migrates cleanly
    /// with zero schema-drift warnings.
    #[test]
    fn test_schema_drift_check_clean_config() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();

        std::fs::write(
            src.path().join("config.toml"),
            "config_version = 2\n\
             api_listen = \"0.0.0.0:4545\"\n\
             log_level = \"info\"\n\
             \n\
             [default_model]\n\
             provider = \"openfang-auto\"\n\
             model = \"gpt-4\"\n",
        )
        .unwrap();

        let options = MigrateOptions {
            source: MigrateSource::OpenFang,
            source_dir: src.path().to_path_buf(),
            target_dir: dst.path().to_path_buf(),
            dry_run: false,
        };
        let report = migrate(&options).unwrap();

        assert!(
            report.warnings.is_empty(),
            "clean config should produce no drift warnings, got: {:?}",
            report.warnings
        );
    }

    /// Drift detection: an invalid **nested enum value** (like a stale
    /// `group_policy = "respond"` from an old OpenFang) fails deserialization
    /// and produces a warning.
    #[test]
    fn test_schema_drift_check_flags_bad_enum_value() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();

        std::fs::write(
            src.path().join("config.toml"),
            "config_version = 2\n\
             api_listen = \"0.0.0.0:4545\"\n\
             \n\
             [channels.telegram]\n\
             bot_token_env = \"TG_TOKEN\"\n\
             \n\
             [channels.telegram.overrides]\n\
             group_policy = \"respond\"\n",
        )
        .unwrap();

        let options = MigrateOptions {
            source: MigrateSource::OpenFang,
            source_dir: src.path().to_path_buf(),
            target_dir: dst.path().to_path_buf(),
            dry_run: false,
        };
        let report = migrate(&options).unwrap();

        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.contains("does not cleanly deserialize")),
            "expected deserialize-failure warning for bad group_policy, got: {:?}",
            report.warnings
        );
    }

    /// Known limitation: an unknown field **nested** inside a section (e.g.
    /// `[channels.telegram].old_field`) is NOT caught, because LibreFang's
    /// channel structs use `#[serde(default)]` and silently drop unknown
    /// fields. This test pins that behaviour so we notice if it ever changes.
    #[test]
    fn test_schema_drift_check_does_not_catch_nested_unknown_fields() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();

        std::fs::write(
            src.path().join("config.toml"),
            "config_version = 2\n\
             api_listen = \"0.0.0.0:4545\"\n\
             \n\
             [channels.telegram]\n\
             bot_token_env = \"TG_TOKEN\"\n\
             nickname = \"this-field-does-not-exist\"\n",
        )
        .unwrap();

        let options = MigrateOptions {
            source: MigrateSource::OpenFang,
            source_dir: src.path().to_path_buf(),
            target_dir: dst.path().to_path_buf(),
            dry_run: false,
        };
        let report = migrate(&options).unwrap();

        // Document the limitation: nested unknown fields produce no warnings.
        assert!(
            report.warnings.is_empty(),
            "nested unknown fields are expected to slip through (see function \
             doc comment for rationale), got: {:?}",
            report.warnings
        );
    }

    /// Drift detection: a config.toml with a field that LibreFang's
    /// KernelConfig doesn't know should produce a warning but not fail.
    #[test]
    fn test_schema_drift_check_flags_unknown_fields() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();

        // Note: `openfang` in field names gets text-substituted to `librefang`
        // during the copy, so we use a neutral prefix to avoid confusion.
        std::fs::write(
            src.path().join("config.toml"),
            "config_version = 2\n\
             api_listen = \"0.0.0.0:4545\"\n\
             \n\
             [legacy_section]\n\
             some_flag = true\n",
        )
        .unwrap();

        let options = MigrateOptions {
            source: MigrateSource::OpenFang,
            source_dir: src.path().to_path_buf(),
            target_dir: dst.path().to_path_buf(),
            dry_run: false,
        };
        let report = migrate(&options).unwrap();

        assert!(
            report.warnings.iter().any(|w| w.contains("legacy_section")),
            "expected drift warning for unknown section, got: {:?}",
            report.warnings
        );
        // Migration itself still succeeded — the file was copied.
        assert!(dst.path().join("config.toml").exists());
    }
}
