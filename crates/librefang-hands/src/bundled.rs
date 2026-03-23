//! Hand definitions loaded from disk at runtime.
//!
//! Hands are read from `~/.librefang/hands/` (synced from the registry
//! via `librefang init`). No compile-time embedding.

use crate::{HandDefinition, HandError};
use serde::Deserialize;
use std::sync::OnceLock;
use tracing::warn;

/// Cached result from the first call to `bundled_hands()`.
static BUNDLED_CACHE: OnceLock<Vec<(&'static str, &'static str, &'static str)>> = OnceLock::new();

/// Returns all hand definitions found on disk as (id, HAND.toml content, SKILL.md content).
///
/// Scans `home_dir/hands/` for subdirectories containing HAND.toml.
/// The caller passes the authoritative home directory (typically `config.home_dir`).
///
/// Results are cached after the first call — subsequent calls return the
/// same `&'static` references without additional disk I/O or memory leaks.
pub fn bundled_hands(
    home_dir: &std::path::Path,
) -> Vec<(&'static str, &'static str, &'static str)> {
    BUNDLED_CACHE
        .get_or_init(|| {
            disk_hands(home_dir)
                .into_iter()
                .map(|(id, toml, skill)| {
                    let id: &'static str = Box::leak(id.into_boxed_str());
                    let toml: &'static str = Box::leak(toml.into_boxed_str());
                    let skill: &'static str = Box::leak(skill.into_boxed_str());
                    (id, toml, skill)
                })
                .collect()
        })
        .clone()
}

pub(crate) fn disk_hands(home_dir: &std::path::Path) -> Vec<(String, String, String)> {
    let mut results = Vec::new();
    let hands_dir = home_dir.join("workspaces").join("hands");

    if let Ok(entries) = std::fs::read_dir(&hands_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let id = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            let toml_path = path.join("HAND.toml");
            let skill_path = path.join("SKILL.md");
            if !toml_path.exists() {
                continue;
            }
            let toml = match std::fs::read_to_string(&toml_path) {
                Ok(s) => s,
                Err(e) => {
                    warn!(path = %toml_path.display(), error = %e, "Failed to read HAND.toml");
                    continue;
                }
            };
            let skill = std::fs::read_to_string(&skill_path).unwrap_or_default();
            results.push((id, toml, skill));
        }
    }

    results.sort_by(|a, b| a.0.cmp(&b.0));
    results
}

/// Wrapper struct for HAND.toml files that use the documented `[hand]` section format.
///
/// The official docs show HAND.toml with a `[hand]` wrapper section:
/// ```toml
/// [hand]
/// id = "my-hand"
/// name = "My Hand"
/// ...
/// ```
///
/// Bundled hands use the flat format (no wrapper). Both are accepted.
#[derive(Debug, Clone, Deserialize)]
struct HandTomlWrapper {
    hand: HandDefinition,
}

/// Parse a HAND.toml into a HandDefinition with its skill content attached.
///
/// Accepts both formats:
/// - Flat format (used by bundled hands): fields at top level
/// - Wrapped format (shown in docs): fields under `[hand]` section
pub fn parse_bundled(
    _id: &str,
    toml_content: &str,
    skill_content: &str,
) -> Result<HandDefinition, HandError> {
    // Try flat format first (backwards compatible with bundled hands),
    // then try the documented [hand] wrapper format.
    let mut def: HandDefinition = toml::from_str::<HandDefinition>(toml_content)
        .or_else(|_flat_err| toml::from_str::<HandTomlWrapper>(toml_content).map(|w| w.hand))
        .map_err(|e| HandError::TomlParse(e.to_string()))?;
    if !skill_content.is_empty() {
        def.skill_content = Some(skill_content.to_string());
    }
    Ok(def)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bundled_valid_toml() {
        let toml = r#"
id = "test"
name = "Test Hand"
description = "A test hand"
category = "productivity"

[agent]
name = "test-agent"
description = "A test agent"
system_prompt = "You are a test agent."
tools = ["file_read"]
"#;
        let def = parse_bundled("test", toml, "# Skill").unwrap();
        assert_eq!(def.id, "test");
        assert!(def.skill_content.is_some());
    }

    #[test]
    fn parse_bundled_wrapped_hand_section() {
        let toml = r#"
[hand]
id = "invoice-processor"
name = "Invoice Processor"
description = "Processes invoices automatically"
category = "productivity"

[hand.agent]
name = "invoice-agent"
description = "An invoice processing agent"
system_prompt = "You process invoices."
tools = ["file_read"]
"#;
        let def = parse_bundled("invoice-processor", toml, "").unwrap();
        assert_eq!(def.id, "invoice-processor");
        assert_eq!(def.name, "Invoice Processor");
        assert!(def.skill_content.is_none());
    }
}
