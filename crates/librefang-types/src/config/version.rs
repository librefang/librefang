//! Configuration version tracking for automatic config migrations.
//!
//! Each time the config schema changes in a backward-incompatible way, bump
//! `CONFIG_VERSION` and add a corresponding `migrate_vN_to_vN+1` function.

/// Current configuration file version. Increment when adding a new migration.
pub const CONFIG_VERSION: u32 = 2;

/// Default version for legacy config files that lack a `config_version` field.
pub const fn default_config_version() -> u32 {
    1
}

/// Run all necessary in-place migrations on the raw TOML value.
///
/// Starting from `from_version`, applies each migration step sequentially
/// until the value matches `CONFIG_VERSION`. Returns the final version
/// reached (equal to `CONFIG_VERSION` on success).
pub fn run_migrations(raw: &mut toml::Value, from_version: u32) -> Result<u32, String> {
    let mut version = from_version;
    while version < CONFIG_VERSION {
        match version {
            1 => migrate_v1_to_v2(raw)?,
            other => {
                return Err(format!(
                    "No migration defined for config version {other} -> {}",
                    other + 1
                ));
            }
        }
        version += 1;
        // Stamp the new version into the raw value after each step
        if let toml::Value::Table(ref mut tbl) = raw {
            tbl.insert(
                "config_version".to_string(),
                toml::Value::Integer(i64::from(version)),
            );
        }
    }
    Ok(version)
}

/// v1 → v2: Move misplaced fields from `[api]` section to root level.
///
/// Early config schema allowed `api_key`, `api_listen`, and `log_level`
/// under an `[api]` table. This migration hoists them to the root where
/// the current `KernelConfig` expects them.
fn migrate_v1_to_v2(raw: &mut toml::Value) -> Result<(), String> {
    if let toml::Value::Table(ref mut tbl) = raw {
        if let Some(toml::Value::Table(api_section)) = tbl.get("api").cloned() {
            for key in &["api_key", "api_listen", "log_level"] {
                if !tbl.contains_key(*key) {
                    if let Some(val) = api_section.get(*key) {
                        tbl.insert(key.to_string(), val.clone());
                    }
                }
            }
            tbl.remove("api");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_migrate_v1_to_v2_moves_api_fields() {
        let mut raw: toml::Value = toml::from_str(
            r#"
            [api]
            api_key = "secret"
            api_listen = "0.0.0.0:9999"
            log_level = "debug"
        "#,
        )
        .unwrap();
        migrate_v1_to_v2(&mut raw).unwrap();
        let tbl = raw.as_table().unwrap();
        assert_eq!(tbl.get("api_key").unwrap().as_str(), Some("secret"));
        assert_eq!(
            tbl.get("api_listen").unwrap().as_str(),
            Some("0.0.0.0:9999")
        );
        assert_eq!(tbl.get("log_level").unwrap().as_str(), Some("debug"));
        assert!(!tbl.contains_key("api"));
    }

    #[test]
    fn test_migrate_v1_to_v2_no_api_section() {
        let mut raw: toml::Value = toml::from_str(
            r#"
            log_level = "info"
        "#,
        )
        .unwrap();
        migrate_v1_to_v2(&mut raw).unwrap();
        assert_eq!(
            raw.as_table().unwrap().get("log_level").unwrap().as_str(),
            Some("info")
        );
    }

    #[test]
    fn test_migrate_v1_to_v2_does_not_overwrite_root() {
        let mut raw: toml::Value = toml::from_str(
            r#"
            api_listen = "127.0.0.1:4545"
            [api]
            api_listen = "0.0.0.0:9999"
        "#,
        )
        .unwrap();
        migrate_v1_to_v2(&mut raw).unwrap();
        // Root value should be preserved, not overwritten
        assert_eq!(
            raw.as_table().unwrap().get("api_listen").unwrap().as_str(),
            Some("127.0.0.1:4545")
        );
    }

    #[test]
    fn test_run_migrations_from_v1() {
        let mut raw: toml::Value = toml::from_str(
            r#"
            config_version = 1
            [api]
            api_key = "test"
        "#,
        )
        .unwrap();
        let final_ver = run_migrations(&mut raw, 1).unwrap();
        assert_eq!(final_ver, CONFIG_VERSION);
        let tbl = raw.as_table().unwrap();
        assert_eq!(tbl.get("api_key").unwrap().as_str(), Some("test"));
        assert_eq!(
            tbl.get("config_version").unwrap().as_integer(),
            Some(i64::from(CONFIG_VERSION))
        );
    }

    #[test]
    fn test_run_migrations_already_current() {
        let mut raw: toml::Value = toml::from_str(
            r#"
            config_version = 2
            log_level = "info"
        "#,
        )
        .unwrap();
        let final_ver = run_migrations(&mut raw, CONFIG_VERSION).unwrap();
        assert_eq!(final_ver, CONFIG_VERSION);
    }

    #[test]
    fn test_run_migrations_unknown_version() {
        let mut raw: toml::Value = toml::from_str("log_level = \"info\"").unwrap();
        let result = run_migrations(&mut raw, 999);
        assert!(result.is_err());
    }
}
