//! Hand definitions loaded from disk at runtime.
//!
//! Hands are read from `~/.librefang/hands/` (synced from the registry
//! via `librefang init`). No compile-time embedding.

use crate::{HandDefinition, HandError};

/// Returns all hand definitions found on disk as (id, HAND.toml content, SKILL.md content).
///
/// Scans `home_dir/hands/` for subdirectories containing HAND.toml.
/// The caller passes the authoritative home directory (typically `config.home_dir`).
pub fn bundled_hands(
    home_dir: &std::path::Path,
) -> Vec<(&'static str, &'static str, &'static str)> {
    // Leak strings into 'static to preserve the existing API contract.
    // This is called once at boot and cached, so the leak is bounded.
    disk_hands(home_dir)
        .into_iter()
        .map(|(id, toml, skill)| {
            let id: &'static str = Box::leak(id.into_boxed_str());
            let toml: &'static str = Box::leak(toml.into_boxed_str());
            let skill: &'static str = Box::leak(skill.into_boxed_str());
            (id, toml, skill)
        })
        .collect()
}

fn disk_hands(home_dir: &std::path::Path) -> Vec<(String, String, String)> {
    let mut results = Vec::new();
    let hands_dir = home_dir.join("hands");

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
                Err(_) => continue,
            };
            let skill = std::fs::read_to_string(&skill_path).unwrap_or_default();
            results.push((id, toml, skill));
        }
    }

    results.sort_by(|a, b| a.0.cmp(&b.0));
    results
}

/// Parse a HAND.toml into a HandDefinition with its skill content attached.
pub fn parse_bundled(
    _id: &str,
    toml_content: &str,
    skill_content: &str,
) -> Result<HandDefinition, HandError> {
    let mut def: HandDefinition =
        toml::from_str(toml_content).map_err(|e| HandError::TomlParse(e.to_string()))?;
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
}
