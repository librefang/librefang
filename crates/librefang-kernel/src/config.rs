//! Configuration loading from `~/.librefang/config.toml` with defaults.
//!
//! Supports config includes: the `include` field specifies additional TOML files
//! to load and deep-merge before the root config (root overrides includes).

use librefang_types::config::{
    default_config_version, run_migrations, KernelConfig, CONFIG_VERSION,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tracing::info;

/// Maximum include nesting depth.
const MAX_INCLUDE_DEPTH: u32 = 10;

/// Load kernel configuration from a TOML file, with defaults.
///
/// If the config contains an `include` field, included files are loaded
/// and deep-merged first, then the root config overrides them.
pub fn load_config(path: Option<&Path>) -> KernelConfig {
    let config_path = path
        .map(|p| p.to_path_buf())
        .unwrap_or_else(default_config_path);

    if config_path.exists() {
        match std::fs::read_to_string(&config_path) {
            Ok(contents) => match toml::from_str::<toml::Value>(&contents) {
                Ok(mut root_value) => {
                    // Process includes before deserializing
                    let config_dir = config_path
                        .parent()
                        .unwrap_or_else(|| Path::new("."))
                        .to_path_buf();
                    let mut visited = HashSet::new();
                    if let Ok(canonical) = std::fs::canonicalize(&config_path) {
                        visited.insert(canonical);
                    } else {
                        visited.insert(config_path.clone());
                    }

                    if let Err(e) =
                        resolve_config_includes(&mut root_value, &config_dir, &mut visited, 0)
                    {
                        tracing::warn!(
                            error = %e,
                            "Config include resolution failed, using root config only"
                        );
                    }

                    // Remove the `include` field before deserializing to avoid confusion
                    if let toml::Value::Table(ref mut tbl) = root_value {
                        tbl.remove("include");
                    }

                    // --- Versioned config migration ---
                    // Keep a clone of the pre-migration value for best-effort fallback.
                    let original_value = root_value.clone();

                    let file_version = root_value
                        .as_table()
                        .and_then(|t| t.get("config_version"))
                        .and_then(|v| v.as_integer())
                        .map(|v| v as u32)
                        .unwrap_or_else(default_config_version);

                    let mut migrated = file_version >= CONFIG_VERSION;
                    if file_version < CONFIG_VERSION {
                        match run_migrations(&mut root_value, file_version) {
                            Ok(_) => {
                                info!(
                                    from = file_version,
                                    to = CONFIG_VERSION,
                                    "Config migrated successfully"
                                );
                                migrated = true;
                            }
                            Err(e) => {
                                tracing::error!(
                                    error = %e,
                                    from = file_version,
                                    to = CONFIG_VERSION,
                                    "Config migration failed, attempting best-effort load of original config"
                                );
                                // Fall back to original value
                                root_value = original_value.clone();
                            }
                        }
                    }

                    // Detect unknown top-level fields before deserialization.
                    let unknown_fields = KernelConfig::detect_unknown_fields(&root_value);
                    // Detect unknown fields in known nested sections (#3460):
                    // typos like `[memory] decay_ratee = 0.1` previously
                    // deserialised into the section's `Default`, silently
                    // dropping the operator's intent.
                    let unknown_nested = KernelConfig::detect_unknown_nested_fields(&root_value);

                    // Check if strict_config is set in the raw TOML (before
                    // deserializing the full struct) so we can decide whether
                    // to reject or warn on unknown fields.
                    let is_strict = root_value
                        .as_table()
                        .and_then(|t| t.get("strict_config"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);

                    let mut all_unknown: Vec<String> = unknown_fields
                        .iter()
                        .cloned()
                        .chain(unknown_nested.iter().cloned())
                        .collect();
                    all_unknown.sort();

                    if !all_unknown.is_empty() {
                        if is_strict {
                            tracing::error!(
                                path = %config_path.display(),
                                fields = %all_unknown.join(", "),
                                "strict_config is enabled and config contains unknown fields, using defaults"
                            );
                            return KernelConfig {
                                strict_config: true,
                                ..KernelConfig::default()
                            };
                        }
                        for field in &all_unknown {
                            tracing::warn!(field, "Unknown config field (ignored)");
                        }
                    }

                    match root_value.try_into::<KernelConfig>() {
                        Ok(config) => {
                            // Write migrated config back to disk so future loads skip migration
                            if migrated && file_version < CONFIG_VERSION {
                                let toml_str = toml::to_string_pretty(&config);
                                match toml_str {
                                    Ok(s) => {
                                        if let Err(e) = std::fs::write(&config_path, &s) {
                                            tracing::warn!(
                                                error = %e,
                                                path = %config_path.display(),
                                                "Failed to write migrated config back to disk"
                                            );
                                        } else {
                                            info!(
                                                path = %config_path.display(),
                                                "Wrote migrated config to disk"
                                            );
                                        }
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            error = %e,
                                            "Failed to serialize migrated config"
                                        );
                                    }
                                }
                            }
                            info!(path = %config_path.display(), "Loaded configuration");
                            return config;
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                path = %config_path.display(),
                                "Failed to deserialize merged config, using defaults"
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        path = %config_path.display(),
                        "Failed to parse config, using defaults"
                    );
                }
            },
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    path = %config_path.display(),
                    "Failed to read config file, using defaults"
                );
            }
        }
    } else {
        info!(
            path = %config_path.display(),
            "Config file not found, using defaults"
        );
    }

    KernelConfig::default()
}

/// Strict counterpart of [`load_config`] that returns `Err` on every failure
/// mode instead of silently falling back to [`KernelConfig::default`].
///
/// Used by [`crate::kernel::Kernel::reload_config`] (issue #4664) so a bad
/// on-disk config — TOML syntax error, broken `include = [...]`, migration
/// failure, deserialize-shape mismatch — never wipes the operator's live
/// in-memory state. The hot-reload watcher and `POST /api/config/reload`
/// handler both already map `Err(...)` to a warning + 400 respectively,
/// so the live config stays intact and the operator gets an actionable
/// error.
///
/// Differences from `load_config`:
///
/// - No write-back of the migrated TOML to disk. The reload path doesn't
///   own the file, and a partial migration that fails downstream would
///   leave the disk file in a half-migrated state. The next initial-boot
///   `load_config` call still does the write-back.
/// - Unknown fields under `strict_config = true` produce `Err` instead of
///   "return defaults with `strict_config: true`" — same intent, but
///   surfaced as an error so the reload path can refuse to apply it.
/// - Unknown fields under `strict_config = false` (or unset) still warn
///   and proceed, matching `load_config`'s tolerant behaviour.
pub fn try_load_config(path: &Path) -> Result<KernelConfig, String> {
    if !path.exists() {
        return Err(format!("Config file not found: {}", path.display()));
    }
    let contents =
        std::fs::read_to_string(path).map_err(|e| format!("Failed to read config file: {e}"))?;
    let mut root_value: toml::Value =
        toml::from_str(&contents).map_err(|e| format!("Config file has invalid TOML: {e}"))?;

    // Resolve `include = [...]` chains. Failures here include: missing
    // file, unparseable include, traversal / absolute-path attempts,
    // circular references, depth overflow.
    let config_dir = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let mut visited = HashSet::new();
    if let Ok(canonical) = std::fs::canonicalize(path) {
        visited.insert(canonical);
    } else {
        visited.insert(path.to_path_buf());
    }
    resolve_config_includes(&mut root_value, &config_dir, &mut visited, 0)
        .map_err(|e| format!("Config include resolution failed: {e}"))?;
    if let toml::Value::Table(ref mut tbl) = root_value {
        tbl.remove("include");
    }

    // Migrate older config versions in place. We do NOT write the result
    // back to disk from the reload path (see doc comment above).
    let file_version = root_value
        .as_table()
        .and_then(|t| t.get("config_version"))
        .and_then(|v| v.as_integer())
        .map(|v| v as u32)
        .unwrap_or_else(default_config_version);
    if file_version < CONFIG_VERSION {
        run_migrations(&mut root_value, file_version).map_err(|e| {
            format!("Config migration failed (from v{file_version} to v{CONFIG_VERSION}): {e}")
        })?;
    }

    // Strict mode: refuse to load when unknown / typo'd fields are present.
    let unknown_fields = KernelConfig::detect_unknown_fields(&root_value);
    let unknown_nested = KernelConfig::detect_unknown_nested_fields(&root_value);
    let is_strict = root_value
        .as_table()
        .and_then(|t| t.get("strict_config"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let mut all_unknown: Vec<String> = unknown_fields.into_iter().chain(unknown_nested).collect();
    all_unknown.sort();
    if !all_unknown.is_empty() {
        if is_strict {
            return Err(format!(
                "strict_config is enabled and config contains unknown fields: {}",
                all_unknown.join(", ")
            ));
        }
        for field in &all_unknown {
            tracing::warn!(field, "Unknown config field (ignored on reload)");
        }
    }

    root_value
        .try_into::<KernelConfig>()
        .map_err(|e| format!("Failed to deserialize config: {e}"))
}

/// Resolve config includes by deep-merging included files into the root value.
///
/// Included files are loaded first and the root config overrides them.
/// Security: rejects absolute paths, `..` components, and circular references.
fn resolve_config_includes(
    root_value: &mut toml::Value,
    config_dir: &Path,
    visited: &mut HashSet<PathBuf>,
    depth: u32,
) -> Result<(), String> {
    if depth > MAX_INCLUDE_DEPTH {
        return Err(format!(
            "Config include depth exceeded maximum of {MAX_INCLUDE_DEPTH}"
        ));
    }

    // Extract include list from the current value
    let includes = match root_value {
        toml::Value::Table(tbl) => {
            if let Some(toml::Value::Array(arr)) = tbl.get("include") {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect::<Vec<_>>()
            } else {
                return Ok(());
            }
        }
        _ => return Ok(()),
    };

    if includes.is_empty() {
        return Ok(());
    }

    // Merge each include (earlier includes are overridden by later ones,
    // and the root config overrides everything).
    let mut merged_base = toml::Value::Table(toml::map::Map::new());

    for include_path_str in &includes {
        // SECURITY: reject absolute paths
        let include_path = Path::new(include_path_str);
        if include_path.is_absolute() {
            return Err(format!(
                "Config include rejects absolute path: {include_path_str}"
            ));
        }
        // SECURITY: reject `..` components
        for component in include_path.components() {
            if let std::path::Component::ParentDir = component {
                return Err(format!(
                    "Config include rejects path traversal: {include_path_str}"
                ));
            }
        }

        let resolved = config_dir.join(include_path);
        // SECURITY: verify resolved path stays within config dir
        let canonical = std::fs::canonicalize(&resolved).map_err(|e| {
            format!(
                "Config include '{}' cannot be resolved: {e}",
                include_path_str
            )
        })?;
        let canonical_dir = std::fs::canonicalize(config_dir)
            .map_err(|e| format!("Config dir cannot be canonicalized: {e}"))?;
        if !canonical.starts_with(&canonical_dir) {
            return Err(format!(
                "Config include '{}' escapes config directory",
                include_path_str
            ));
        }

        // SECURITY: circular detection
        if !visited.insert(canonical.clone()) {
            return Err(format!(
                "Circular config include detected: {include_path_str}"
            ));
        }

        info!(include = %include_path_str, "Loading config include");

        let contents = std::fs::read_to_string(&canonical)
            .map_err(|e| format!("Failed to read config include '{}': {e}", include_path_str))?;
        let mut include_value: toml::Value = toml::from_str(&contents)
            .map_err(|e| format!("Failed to parse config include '{}': {e}", include_path_str))?;

        // Recursively resolve includes in the included file
        let include_dir = canonical.parent().unwrap_or(config_dir).to_path_buf();
        resolve_config_includes(&mut include_value, &include_dir, visited, depth + 1)?;

        // Remove include field from the included file
        if let toml::Value::Table(ref mut tbl) = include_value {
            tbl.remove("include");
        }

        // Deep merge: include overrides the base built so far
        deep_merge_toml(&mut merged_base, &include_value);
    }

    // Now deep merge: root overrides the merged includes
    // Save root's current values (minus include), then merge root on top
    let root_without_include = {
        let mut v = root_value.clone();
        if let toml::Value::Table(ref mut tbl) = v {
            tbl.remove("include");
        }
        v
    };
    deep_merge_toml(&mut merged_base, &root_without_include);
    *root_value = merged_base;

    Ok(())
}

/// Deep-merge two TOML values. `overlay` values override `base` values.
/// For tables, recursively merge. For everything else, overlay wins.
pub fn deep_merge_toml(base: &mut toml::Value, overlay: &toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(base_tbl), toml::Value::Table(overlay_tbl)) => {
            for (key, overlay_val) in overlay_tbl {
                if let Some(base_val) = base_tbl.get_mut(key) {
                    deep_merge_toml(base_val, overlay_val);
                } else {
                    base_tbl.insert(key.clone(), overlay_val.clone());
                }
            }
        }
        (base, overlay) => {
            *base = overlay.clone();
        }
    }
}

/// Get the default config file path.
///
/// Respects `LIBREFANG_HOME` env var (e.g. `LIBREFANG_HOME=/opt/librefang`).
pub fn default_config_path() -> PathBuf {
    librefang_home().join("config.toml")
}

/// Get the LibreFang home directory.
///
/// Priority: `LIBREFANG_HOME` env var > `~/.librefang`.
pub fn librefang_home() -> PathBuf {
    if let Ok(home) = std::env::var("LIBREFANG_HOME") {
        return PathBuf::from(home);
    }
    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".librefang")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_load_config_defaults() {
        let config = load_config(None);
        assert_eq!(config.log_level, "info");
    }

    #[test]
    fn test_load_config_missing_file() {
        let config = load_config(Some(Path::new("/nonexistent/config.toml")));
        assert_eq!(config.log_level, "info");
    }

    #[test]
    fn test_deep_merge_simple() {
        let mut base: toml::Value = toml::from_str(
            r#"
            log_level = "debug"
            api_listen = "0.0.0.0:4545"
        "#,
        )
        .unwrap();
        let overlay: toml::Value = toml::from_str(
            r#"
            log_level = "info"
            network_enabled = true
        "#,
        )
        .unwrap();
        deep_merge_toml(&mut base, &overlay);
        assert_eq!(base["log_level"].as_str(), Some("info"));
        assert_eq!(base["api_listen"].as_str(), Some("0.0.0.0:4545"));
        assert_eq!(base["network_enabled"].as_bool(), Some(true));
    }

    #[test]
    fn test_deep_merge_nested_tables() {
        let mut base: toml::Value = toml::from_str(
            r#"
            [memory]
            decay_rate = 0.1
            consolidation_threshold = 10000
        "#,
        )
        .unwrap();
        let overlay: toml::Value = toml::from_str(
            r#"
            [memory]
            decay_rate = 0.5
        "#,
        )
        .unwrap();
        deep_merge_toml(&mut base, &overlay);
        let mem = base["memory"].as_table().unwrap();
        assert_eq!(mem["decay_rate"].as_float(), Some(0.5));
        assert_eq!(mem["consolidation_threshold"].as_integer(), Some(10000));
    }

    #[test]
    fn test_basic_include() {
        let dir = tempfile::tempdir().unwrap();
        let base_path = dir.path().join("base.toml");
        let root_path = dir.path().join("config.toml");

        // Base config
        let mut f = std::fs::File::create(&base_path).unwrap();
        writeln!(f, "log_level = \"debug\"").unwrap();
        writeln!(f, "api_listen = \"0.0.0.0:9999\"").unwrap();
        drop(f);

        // Root config (includes base, overrides log_level)
        let mut f = std::fs::File::create(&root_path).unwrap();
        writeln!(f, "include = [\"base.toml\"]").unwrap();
        writeln!(f, "log_level = \"warn\"").unwrap();
        drop(f);

        let config = load_config(Some(&root_path));
        assert_eq!(config.log_level, "warn"); // root overrides
        assert_eq!(config.api_listen, "0.0.0.0:9999"); // from base
    }

    #[test]
    fn test_nested_include() {
        let dir = tempfile::tempdir().unwrap();
        let grandchild = dir.path().join("grandchild.toml");
        let child = dir.path().join("child.toml");
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&grandchild).unwrap();
        writeln!(f, "log_level = \"trace\"").unwrap();
        drop(f);

        let mut f = std::fs::File::create(&child).unwrap();
        writeln!(f, "include = [\"grandchild.toml\"]").unwrap();
        writeln!(f, "log_level = \"debug\"").unwrap();
        drop(f);

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "include = [\"child.toml\"]").unwrap();
        writeln!(f, "log_level = \"info\"").unwrap();
        drop(f);

        let config = load_config(Some(&root));
        assert_eq!(config.log_level, "info"); // root wins
    }

    #[test]
    fn test_circular_include_detected() {
        let dir = tempfile::tempdir().unwrap();
        let a_path = dir.path().join("a.toml");
        let b_path = dir.path().join("b.toml");

        let mut f = std::fs::File::create(&a_path).unwrap();
        writeln!(f, "include = [\"b.toml\"]").unwrap();
        writeln!(f, "log_level = \"info\"").unwrap();
        drop(f);

        let mut f = std::fs::File::create(&b_path).unwrap();
        writeln!(f, "include = [\"a.toml\"]").unwrap();
        drop(f);

        // Should not panic — circular detection triggers, falls back gracefully
        let config = load_config(Some(&a_path));
        // Falls back to defaults due to the circular error
        assert!(!config.log_level.is_empty());
    }

    #[test]
    fn test_path_traversal_blocked() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "include = [\"../etc/passwd\"]").unwrap();
        drop(f);

        // Should not panic — path traversal triggers error, falls back
        let config = load_config(Some(&root));
        assert_eq!(config.log_level, "info"); // defaults
    }

    #[test]
    fn test_max_depth_exceeded() {
        let dir = tempfile::tempdir().unwrap();

        // Create a chain of 12 files (exceeds MAX_INCLUDE_DEPTH=10)
        for i in (0..12).rev() {
            let name = format!("level{i}.toml");
            let path = dir.path().join(&name);
            let mut f = std::fs::File::create(&path).unwrap();
            if i < 11 {
                let next = format!("level{}.toml", i + 1);
                writeln!(f, "include = [\"{next}\"]").unwrap();
            }
            writeln!(f, "log_level = \"level{i}\"").unwrap();
            drop(f);
        }

        let root = dir.path().join("level0.toml");
        let config = load_config(Some(&root));
        // Falls back due to depth limit — but should not panic
        assert!(!config.log_level.is_empty());
    }

    #[test]
    fn test_absolute_path_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "include = [\"/etc/shadow\"]").unwrap();
        drop(f);

        let config = load_config(Some(&root));
        assert_eq!(config.log_level, "info"); // defaults
    }

    #[test]
    fn test_no_includes_works() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "log_level = \"trace\"").unwrap();
        drop(f);

        let config = load_config(Some(&root));
        assert_eq!(config.log_level, "trace");
    }

    // --- Tolerant / strict config mode tests ---

    #[test]
    fn test_tolerant_mode_loads_with_unknown_fields() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "log_level = \"debug\"").unwrap();
        writeln!(f, "unknown_field_xyz = 42").unwrap();
        writeln!(f, "another_typo = true").unwrap();
        drop(f);

        // Tolerant mode (default): should still load successfully
        let config = load_config(Some(&root));
        assert_eq!(config.log_level, "debug");
        assert!(!config.strict_config);
    }

    #[test]
    fn test_strict_mode_rejects_unknown_fields() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "strict_config = true").unwrap();
        writeln!(f, "log_level = \"debug\"").unwrap();
        writeln!(f, "bogus_field = \"oops\"").unwrap();
        drop(f);

        // Strict mode: should reject and return defaults (with strict_config=true)
        let config = load_config(Some(&root));
        // Falls back to defaults because strict mode rejected unknown fields
        assert_eq!(config.log_level, "info"); // default, not "debug"
        assert!(config.strict_config);
    }

    #[test]
    fn test_strict_mode_accepts_clean_config() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "strict_config = true").unwrap();
        writeln!(f, "log_level = \"warn\"").unwrap();
        drop(f);

        // Strict mode with no unknown fields: should load normally
        let config = load_config(Some(&root));
        assert_eq!(config.log_level, "warn");
        assert!(config.strict_config);
    }

    #[test]
    fn test_tolerant_mode_with_explicit_false() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "strict_config = false").unwrap();
        writeln!(f, "log_level = \"error\"").unwrap();
        writeln!(f, "not_a_real_field = 123").unwrap();
        drop(f);

        // Explicitly tolerant: should load despite unknown field
        let config = load_config(Some(&root));
        assert_eq!(config.log_level, "error");
        assert!(!config.strict_config);
    }

    #[test]
    fn test_load_config_migrates_v1_api_section() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        // v1 config with [api] section (no config_version field)
        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "[api]").unwrap();
        writeln!(f, "api_key = \"my-secret\"").unwrap();
        writeln!(f, "api_listen = \"0.0.0.0:9999\"").unwrap();
        drop(f);

        let config = load_config(Some(&root));
        assert_eq!(config.api_key, "my-secret");
        assert_eq!(config.api_listen, "0.0.0.0:9999");
        assert_eq!(config.config_version, CONFIG_VERSION);

        // Verify the migrated file was written back
        let contents = std::fs::read_to_string(&root).unwrap();
        assert!(
            contents.contains("config_version"),
            "migrated file should contain config_version"
        );
        assert!(
            !contents.contains("[api]"),
            "migrated file should not contain [api] section"
        );
    }

    #[test]
    fn test_load_config_v2_skips_migration() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "config_version = 2").unwrap();
        writeln!(f, "log_level = \"debug\"").unwrap();
        drop(f);

        let config = load_config(Some(&root));
        assert_eq!(config.log_level, "debug");
        assert_eq!(config.config_version, 2);
    }

    #[test]
    fn test_load_config_default_has_current_version() {
        let config = KernelConfig::default();
        assert_eq!(config.config_version, CONFIG_VERSION);
    }

    #[test]
    fn test_load_config_with_users_block() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "[[users]]").unwrap();
        writeln!(f, "name = \"Alice\"").unwrap();
        writeln!(f, "role = \"owner\"").unwrap();
        drop(f);

        let config = load_config(Some(&root));
        assert_eq!(config.users.len(), 1);
        assert_eq!(config.users[0].name, "Alice");
        assert_eq!(config.users[0].role, "owner");
        assert!(config.users[0].channel_bindings.is_empty());
        assert!(config.users[0].api_key_hash.is_none());
    }

    #[test]
    fn test_load_config_with_users_and_channel_bindings() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "[[users]]").unwrap();
        writeln!(f, "name = \"Alice\"").unwrap();
        writeln!(f, "role = \"owner\"").unwrap();
        writeln!(f, "[users.channel_bindings]").unwrap();
        writeln!(f, "telegram = \"123456\"").unwrap();
        drop(f);

        let config = load_config(Some(&root));
        assert_eq!(config.users.len(), 1);
        assert_eq!(config.users[0].name, "Alice");
        assert_eq!(
            config.users[0].channel_bindings.get("telegram").unwrap(),
            "123456"
        );
    }

    #[test]
    fn test_load_config_users_migration_roundtrip() {
        // When no config_version is present, migration runs and writes back.
        // Verify that the round-trip preserves users data.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "log_level = \"debug\"").unwrap();
        writeln!(f, "[[users]]").unwrap();
        writeln!(f, "name = \"Alice\"").unwrap();
        writeln!(f, "role = \"owner\"").unwrap();
        drop(f);

        // First load: triggers migration (v1 → v2), writes back
        let config1 = load_config(Some(&root));
        assert_eq!(config1.log_level, "debug");
        assert_eq!(config1.users.len(), 1);
        assert_eq!(config1.users[0].name, "Alice");
        assert_eq!(config1.config_version, CONFIG_VERSION);

        // Second load: reads migrated file, no migration needed
        let config2 = load_config(Some(&root));
        assert_eq!(config2.log_level, "debug");
        assert_eq!(config2.users.len(), 1);
        assert_eq!(config2.users[0].name, "Alice");
        assert_eq!(config2.config_version, CONFIG_VERSION);
    }

    #[test]
    fn test_migrated_config_no_users_array_key() {
        // When the migration writes back a config WITHOUT [[users]] entries,
        // it must NOT emit `users = []` (an inline empty array). If it does,
        // a subsequent manual `[[users]]` addition by the user would produce
        // duplicate `users` keys in the TOML file, breaking the next parse.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "log_level = \"debug\"").unwrap();
        // No [[users]] section — migration should not add users = []
        drop(f);

        let _config = load_config(Some(&root));
        let contents = std::fs::read_to_string(&root).unwrap();

        // The migrated file should not have a `users = []` line that would
        // conflict with a user later adding `[[users]]` sections.
        assert!(
            !contents.contains("users = []"),
            "Migrated config must not contain `users = []`; got:\n{contents}"
        );
    }

    #[test]
    fn test_users_array_key_conflicts_with_array_of_tables() {
        // Verify that if a config has BOTH `users = []` and `[[users]]`,
        // the TOML parser rejects it (duplicate key). The kernel should
        // fall back to defaults gracefully rather than panic.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "log_level = \"warn\"").unwrap();
        writeln!(f, "users = []").unwrap(); // would conflict with [[users]]
        writeln!(f, "[[users]]").unwrap();
        writeln!(f, "name = \"Alice\"").unwrap();
        writeln!(f, "role = \"owner\"").unwrap();
        drop(f);

        // Should fall back to defaults (TOML parse error: duplicate key)
        let config = load_config(Some(&root));
        assert_eq!(config.log_level, "info"); // default, not "warn"
        assert!(config.users.is_empty());
    }

    /// Regression for #3460: nested typos like `[memory] decay_ratee` were
    /// previously silently swallowed by `#[serde(default)]` because only
    /// top-level field names were checked. The loader must now warn about
    /// typos in known sections (warn-only by default, reject in strict mode).
    #[test]
    fn test_nested_typo_detected_in_warn_mode() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "log_level = \"debug\"").unwrap();
        writeln!(f, "[memory]").unwrap();
        writeln!(f, "decay_ratee = 0.42").unwrap(); // typo: extra 'e'
        writeln!(f, "[queue.concurrency]").unwrap();
        writeln!(f, "trigger_laneeee = 99").unwrap(); // typo
        drop(f);

        // Tolerant: still loads with defaults for the typo'd fields.
        let config = load_config(Some(&root));
        assert_eq!(config.log_level, "debug");
        // Defaults preserved (the typos didn't take effect).
        assert!(
            (config.memory.decay_rate
                - librefang_types::config::KernelConfig::default()
                    .memory
                    .decay_rate)
                .abs()
                < f64::EPSILON
        );
    }

    #[test]
    fn test_nested_typo_rejected_in_strict_mode() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "strict_config = true").unwrap();
        writeln!(f, "log_level = \"warn\"").unwrap();
        writeln!(f, "[budget]").unwrap();
        writeln!(f, "max_hourly_usdd = 5.0").unwrap(); // typo
        drop(f);

        let config = load_config(Some(&root));
        // Falls back to defaults because strict mode rejected the typo.
        assert_eq!(config.log_level, "info");
        assert!(config.strict_config);
    }

    #[test]
    fn test_load_config_users_missing_name_falls_back_to_defaults() {
        // A [[users]] block without a required `name` field should fail deserialization
        // and fall back to defaults rather than panicking.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "log_level = \"warn\"").unwrap();
        writeln!(f, "[[users]]").unwrap();
        writeln!(f, "role = \"owner\"").unwrap(); // no name — invalid
        drop(f);

        let config = load_config(Some(&root));
        // Should fall back to defaults (users deserialization fails)
        assert_eq!(config.log_level, "info"); // default, not "warn"
        assert!(config.users.is_empty());
    }

    // -----------------------------------------------------------------------
    // try_load_config — strict variant used by reload (#4664)
    // -----------------------------------------------------------------------

    #[test]
    fn test_try_load_config_happy_path_returns_kernel_config() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "log_level = \"warn\"").unwrap();
        drop(f);

        let cfg = try_load_config(&root).expect("strict load must succeed on a clean file");
        assert_eq!(cfg.log_level, "warn");
    }

    #[test]
    fn test_try_load_config_missing_file_is_error_not_default() {
        // Tolerant `load_config` returns defaults for a missing file because
        // initial boot wants to come up. The strict variant must `Err` so the
        // reload path does not silently rewrite the live state.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("nonexistent.toml");

        let err = try_load_config(&root).expect_err("missing file must surface as Err");
        assert!(
            err.contains("not found"),
            "missing-file error must be operator-actionable; got: {err}"
        );
    }

    #[test]
    fn test_try_load_config_invalid_toml_is_error() {
        // The exact failure shape from #4664: duplicate `[web.searxng]` key.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "[web.searxng]").unwrap();
        writeln!(f, "url = \"http://first\"").unwrap();
        writeln!(f, "[web.searxng]").unwrap();
        writeln!(f, "url = \"http://second\"").unwrap();
        drop(f);

        let err = try_load_config(&root).expect_err("duplicate key must surface as Err");
        assert!(
            err.contains("invalid TOML"),
            "TOML syntax error must be tagged so the reload caller can wrap it; got: {err}"
        );
    }

    #[test]
    fn test_try_load_config_broken_include_chain_is_error() {
        // Root is well-formed but points at an unparseable include — tolerant
        // `load_config` warns and silently proceeds with root-only, which on a
        // reload path would still effectively zero the operator's settings
        // that lived in the include. Strict variant must refuse.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");
        let bad_include = dir.path().join("bad.toml");

        let mut f = std::fs::File::create(&bad_include).unwrap();
        // Same duplicate-key shape as the bug report, just inside the include.
        writeln!(f, "[memory]").unwrap();
        writeln!(f, "decay_rate = 0.1").unwrap();
        writeln!(f, "[memory]").unwrap();
        writeln!(f, "decay_rate = 0.2").unwrap();
        drop(f);

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "include = [\"bad.toml\"]").unwrap();
        writeln!(f, "log_level = \"debug\"").unwrap();
        drop(f);

        let err = try_load_config(&root).expect_err("broken include must surface as Err");
        assert!(
            err.contains("include"),
            "include-failure error must be tagged so the operator knows where to look; got: {err}"
        );
    }

    #[test]
    fn test_try_load_config_deserialize_shape_mismatch_is_error() {
        // TOML parses cleanly but a field has the wrong shape — `default_model`
        // is a struct, not a scalar. Tolerant `load_config` warns and returns
        // defaults; strict variant must refuse so the reload caller cannot
        // accidentally apply defaults as the diff target.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "default_model = \"not-a-table\"").unwrap();
        drop(f);

        let err = try_load_config(&root).expect_err("wrong-shape field must surface as Err");
        assert!(
            err.contains("deserialize"),
            "deserialize-failure error must be tagged; got: {err}"
        );
    }

    #[test]
    fn test_try_load_config_strict_mode_rejects_unknown_field() {
        // Mirrors the tolerant `load_config` behaviour test, but the strict
        // variant returns `Err` instead of "defaults with strict_config=true"
        // so the reload path can refuse to swap rather than silently zeroing.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "strict_config = true").unwrap();
        writeln!(f, "log_level = \"warn\"").unwrap();
        writeln!(f, "bogus_field = \"oops\"").unwrap();
        drop(f);

        let err =
            try_load_config(&root).expect_err("strict_config + unknown field must surface as Err");
        assert!(
            err.contains("unknown field"),
            "unknown-field error must be tagged; got: {err}"
        );
    }

    #[test]
    fn test_try_load_config_tolerant_mode_warns_on_unknown_field_but_still_loads() {
        // strict_config defaults to false. Unknown fields warn but don't block,
        // matching `load_config`'s tolerant semantics — otherwise a typo
        // anywhere would brick the reload.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "log_level = \"warn\"").unwrap();
        writeln!(f, "bogus_field = \"oops\"").unwrap();
        drop(f);

        let cfg =
            try_load_config(&root).expect("tolerant unknown fields must not fail strict load");
        assert_eq!(cfg.log_level, "warn");
    }
}
