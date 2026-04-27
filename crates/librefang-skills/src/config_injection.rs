//! Skill config variable declaration collection and resolution.
//!
//! Skills can declare global configuration values they depend on via
//! `[[config_vars]]` in their `skill.toml`.  This module:
//!
//! 1. Collects all declarations from a set of enabled [`InstalledSkill`]s,
//!    deduplicating by key (first declaration wins).
//! 2. Resolves each declared key against a parsed `toml::Value` tree,
//!    following the storage convention `skills.config.<key>` with dotted-path
//!    traversal.
//! 3. Formats the resolved values as a compact system-prompt section that
//!    the prompt builder appends to the Skills block.
//!
//! # Storage convention
//!
//! Declared keys use a *logical* dotted path (e.g. `wiki.base_url`).  In
//! `~/.librefang/config.toml` the value is stored under
//! `skills.config.<logical-key>`, so:
//!
//! ```toml
//! [skills.config.wiki]
//! base_url = "https://wiki.corp.example.com"
//! ```
//!
//! resolves the key `wiki.base_url` to `"https://wiki.corp.example.com"`.
//!
//! # Prompt injection format
//!
//! ```text
//! ## Skill Config Variables
//! wiki.base_url = https://wiki.corp.example.com
//! db.host = localhost
//! ```

use crate::{InstalledSkill, SkillConfigVar};

/// Storage prefix prepended to logical keys when looking up values in the
/// config TOML tree.  A logical key `wiki.base_url` becomes the path
/// `skills` → `config` → `wiki` → `base_url`.
const SKILL_CONFIG_PREFIX: &str = "skills.config";

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Collect all config variable declarations from a slice of enabled skills.
///
/// Disabled skills are skipped by the caller (only enabled skills are passed
/// in). Duplicate keys — where two skills declare the same key — are
/// deduplicated: the first declaration encountered wins, preserving the
/// ordering of `skills`.
pub fn collect_config_vars(skills: &[InstalledSkill]) -> Vec<SkillConfigVar> {
    let mut seen = std::collections::HashSet::new();
    let mut vars: Vec<SkillConfigVar> = Vec::new();

    for skill in skills {
        if !skill.enabled {
            continue;
        }
        for var in &skill.manifest.config_vars {
            let key = var.key.trim();
            if key.is_empty() || var.description.trim().is_empty() {
                // Incomplete declaration — skip silently (mirrors Python impl).
                continue;
            }
            if seen.contains(key) {
                continue;
            }
            seen.insert(key.to_string());
            vars.push(SkillConfigVar {
                key: key.to_string(),
                description: var.description.clone(),
                default: var.default.clone(),
            });
        }
    }

    vars
}

/// Resolve a slice of config variable declarations against the raw config TOML.
///
/// For each declared variable the function walks the path
/// `skills.config.<logical-key>` inside `config_toml`.  If the path is
/// absent or its leaf value is an empty string, the declared `default` is
/// used instead.  Variables with neither a config value nor a default are
/// omitted from the output (they would be empty strings in the prompt, which
/// adds noise without information).
///
/// Returns `(logical_key, resolved_value)` pairs in declaration order.
pub fn resolve_config_vars(
    vars: &[SkillConfigVar],
    config_toml: &toml::Value,
) -> Vec<(String, String)> {
    let mut resolved = Vec::with_capacity(vars.len());

    for var in vars {
        // Build the full storage path: SKILL_CONFIG_PREFIX + "." + logical key
        let storage_path = format!("{}.{}", SKILL_CONFIG_PREFIX, var.key);
        let value = resolve_dotpath(config_toml, &storage_path);

        // Normalise to Option<String>; treat empty strings as absent.
        let value_str: Option<String> = value.and_then(|v| {
            let s = toml_value_to_string(v);
            if s.trim().is_empty() {
                None
            } else {
                Some(s)
            }
        });

        // Fall back to declared default, then skip if still nothing.
        let final_value = match value_str.or_else(|| var.default.clone()) {
            Some(v) if !v.trim().is_empty() => v,
            _ => continue,
        };

        resolved.push((var.key.clone(), final_value));
    }

    resolved
}

/// Format resolved config variable pairs as a system-prompt section string.
///
/// Returns an empty string when `resolved` is empty so callers can cheaply
/// skip injection with an `is_empty()` guard.
///
/// Format:
/// ```text
/// ## Skill Config Variables
/// wiki.base_url = https://wiki.example.com
/// db.host = localhost
/// ```
pub fn format_config_section(resolved: &[(String, String)]) -> String {
    if resolved.is_empty() {
        return String::new();
    }
    let mut out = String::from("## Skill Config Variables\n");
    for (key, value) in resolved {
        out.push_str(key);
        out.push_str(" = ");
        out.push_str(value);
        out.push('\n');
    }
    // Trim the trailing newline so the caller controls spacing.
    out.trim_end_matches('\n').to_string()
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Walk a nested `toml::Value` tree following a dotted key path.
/// Returns `None` if any segment is missing or if the current node is not
/// a table (i.e. we cannot descend further).
fn resolve_dotpath<'a>(root: &'a toml::Value, dotted_key: &str) -> Option<&'a toml::Value> {
    let mut current = root;
    for part in dotted_key.split('.') {
        match current {
            toml::Value::Table(tbl) => match tbl.get(part) {
                Some(next) => current = next,
                None => return None,
            },
            _ => return None,
        }
    }
    Some(current)
}

/// Convert a `toml::Value` leaf to a display string.
///
/// Tables and arrays are rendered as compact TOML (fallback) — in practice
/// skill config vars are expected to be scalars (strings, integers, booleans).
fn toml_value_to_string(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => s.clone(),
        toml::Value::Integer(i) => i.to_string(),
        toml::Value::Float(f) => f.to_string(),
        toml::Value::Boolean(b) => b.to_string(),
        toml::Value::Datetime(dt) => dt.to_string(),
        // Arrays and tables are unlikely for scalar config vars; render as
        // TOML for transparency rather than silently dropping them.
        other => toml::to_string(other).unwrap_or_default(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        InstalledSkill, SkillConfigVar, SkillManifest, SkillMeta, SkillRequirements,
        SkillRuntimeConfig, SkillTools,
    };
    use std::path::PathBuf;

    fn make_skill(name: &str, config_vars: Vec<SkillConfigVar>) -> InstalledSkill {
        InstalledSkill {
            manifest: SkillManifest {
                skill: SkillMeta {
                    name: name.to_string(),
                    version: "0.1.0".to_string(),
                    description: String::new(),
                    author: String::new(),
                    license: String::new(),
                    tags: vec![],
                },
                runtime: SkillRuntimeConfig::default(),
                tools: SkillTools::default(),
                requirements: SkillRequirements::default(),
                prompt_context: None,
                source: None,
                config: std::collections::HashMap::new(),
                config_vars,
                env_passthrough: Vec::new(),
            },
            path: PathBuf::from("/tmp/fake"),
            enabled: true,
        }
    }

    // --- collect_config_vars ---

    #[test]
    fn test_collect_empty() {
        assert!(collect_config_vars(&[]).is_empty());
    }

    #[test]
    fn test_collect_single_skill() {
        let skill = make_skill(
            "wiki",
            vec![SkillConfigVar {
                key: "wiki.base_url".to_string(),
                description: "Base URL of the wiki".to_string(),
                default: Some("https://wiki.example.com".to_string()),
            }],
        );
        let vars = collect_config_vars(&[skill]);
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].key, "wiki.base_url");
    }

    #[test]
    fn test_collect_deduplicates_keys() {
        let skill_a = make_skill(
            "skill-a",
            vec![SkillConfigVar {
                key: "shared.endpoint".to_string(),
                description: "Shared API endpoint (from A)".to_string(),
                default: Some("https://a.example.com".to_string()),
            }],
        );
        let skill_b = make_skill(
            "skill-b",
            vec![SkillConfigVar {
                key: "shared.endpoint".to_string(),
                description: "Shared API endpoint (from B)".to_string(),
                default: Some("https://b.example.com".to_string()),
            }],
        );
        let vars = collect_config_vars(&[skill_a, skill_b]);
        // Only the first declaration survives.
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].description, "Shared API endpoint (from A)");
    }

    #[test]
    fn test_collect_skips_disabled_skills() {
        let mut skill = make_skill(
            "disabled",
            vec![SkillConfigVar {
                key: "foo.bar".to_string(),
                description: "some key".to_string(),
                default: None,
            }],
        );
        skill.enabled = false;
        assert!(collect_config_vars(&[skill]).is_empty());
    }

    #[test]
    fn test_collect_skips_incomplete_entries() {
        let skill = make_skill(
            "bad-skill",
            vec![
                // Missing key
                SkillConfigVar {
                    key: String::new(),
                    description: "no key here".to_string(),
                    default: None,
                },
                // Missing description
                SkillConfigVar {
                    key: "valid.key".to_string(),
                    description: String::new(),
                    default: None,
                },
                // Valid entry — should be kept
                SkillConfigVar {
                    key: "good.key".to_string(),
                    description: "A good key".to_string(),
                    default: None,
                },
            ],
        );
        let vars = collect_config_vars(&[skill]);
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].key, "good.key");
    }

    // --- resolve_config_vars ---

    fn make_config(toml_str: &str) -> toml::Value {
        toml::from_str(toml_str).expect("test TOML is valid")
    }

    #[test]
    fn test_resolve_nested_key() {
        let config = make_config(
            r#"
[skills.config.wiki]
base_url = "https://wiki.corp.example.com"
"#,
        );
        let vars = vec![SkillConfigVar {
            key: "wiki.base_url".to_string(),
            description: "Wiki URL".to_string(),
            default: None,
        }];
        let resolved = resolve_config_vars(&vars, &config);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].0, "wiki.base_url");
        assert_eq!(resolved[0].1, "https://wiki.corp.example.com");
    }

    #[test]
    fn test_resolve_uses_default_when_absent() {
        let config = make_config("[skills]\n");
        let vars = vec![SkillConfigVar {
            key: "wiki.base_url".to_string(),
            description: "Wiki URL".to_string(),
            default: Some("https://default.example.com".to_string()),
        }];
        let resolved = resolve_config_vars(&vars, &config);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].1, "https://default.example.com");
    }

    #[test]
    fn test_resolve_omits_when_no_value_no_default() {
        let config = make_config("[skills]\n");
        let vars = vec![SkillConfigVar {
            key: "missing.key".to_string(),
            description: "Key with no default".to_string(),
            default: None,
        }];
        let resolved = resolve_config_vars(&vars, &config);
        assert!(resolved.is_empty());
    }

    #[test]
    fn test_resolve_config_overrides_default() {
        let config = make_config(
            r#"
[skills.config.db]
host = "prod-db.corp.example.com"
"#,
        );
        let vars = vec![SkillConfigVar {
            key: "db.host".to_string(),
            description: "Database host".to_string(),
            default: Some("localhost".to_string()),
        }];
        let resolved = resolve_config_vars(&vars, &config);
        assert_eq!(resolved[0].1, "prod-db.corp.example.com");
    }

    #[test]
    fn test_resolve_integer_value() {
        let config = make_config(
            r#"
[skills.config.api]
timeout = 30
"#,
        );
        let vars = vec![SkillConfigVar {
            key: "api.timeout".to_string(),
            description: "API timeout in seconds".to_string(),
            default: None,
        }];
        let resolved = resolve_config_vars(&vars, &config);
        assert_eq!(resolved[0].1, "30");
    }

    #[test]
    fn test_resolve_empty_string_falls_back_to_default() {
        let config = make_config(
            r#"
[skills.config.wiki]
base_url = ""
"#,
        );
        let vars = vec![SkillConfigVar {
            key: "wiki.base_url".to_string(),
            description: "Wiki URL".to_string(),
            default: Some("https://fallback.example.com".to_string()),
        }];
        let resolved = resolve_config_vars(&vars, &config);
        assert_eq!(resolved[0].1, "https://fallback.example.com");
    }

    // --- format_config_section ---

    #[test]
    fn test_format_empty() {
        assert!(format_config_section(&[]).is_empty());
    }

    #[test]
    fn test_format_single_entry() {
        let resolved = vec![(
            "wiki.base_url".to_string(),
            "https://wiki.example.com".to_string(),
        )];
        let section = format_config_section(&resolved);
        assert!(section.contains("## Skill Config Variables"));
        assert!(section.contains("wiki.base_url = https://wiki.example.com"));
    }

    #[test]
    fn test_format_multiple_entries() {
        let resolved = vec![
            (
                "wiki.base_url".to_string(),
                "https://wiki.example.com".to_string(),
            ),
            ("db.host".to_string(), "localhost".to_string()),
        ];
        let section = format_config_section(&resolved);
        assert!(section.contains("wiki.base_url = https://wiki.example.com"));
        assert!(section.contains("db.host = localhost"));
        // Should not end with a newline (caller controls spacing)
        assert!(!section.ends_with('\n'));
    }

    // --- SkillManifest round-trip (config_vars field) ---

    #[test]
    fn test_manifest_parses_config_vars() {
        let toml_str = r#"
[skill]
name = "wiki-helper"
version = "0.1.0"
description = "Wiki integration skill"

[[config_vars]]
key = "wiki.base_url"
description = "Base URL of the internal wiki"
default = "https://wiki.example.com"

[[config_vars]]
key = "wiki.api_key"
description = "API key for wiki access"
"#;
        let manifest: crate::SkillManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.config_vars.len(), 2);
        assert_eq!(manifest.config_vars[0].key, "wiki.base_url");
        assert_eq!(
            manifest.config_vars[0].default,
            Some("https://wiki.example.com".to_string())
        );
        assert_eq!(manifest.config_vars[1].key, "wiki.api_key");
        assert!(manifest.config_vars[1].default.is_none());
    }

    #[test]
    fn test_manifest_without_config_vars_is_backward_compatible() {
        let toml_str = r#"
[skill]
name = "plain-skill"
version = "0.1.0"
description = "No config vars declared"
"#;
        let manifest: crate::SkillManifest = toml::from_str(toml_str).unwrap();
        assert!(manifest.config_vars.is_empty());
    }
}
