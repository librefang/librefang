//! Skill definitions loaded from disk at runtime.
//!
//! Skills are read from `~/.librefang/skills/` (synced from the registry
//! via `librefang init`). No compile-time embedding.

use crate::openclaw_compat::convert_skillmd_str;
use crate::SkillManifest;

/// Return all skills found on disk as (name, raw SKILL.md content) pairs.
///
/// Scans `home_dir/skills/` for subdirectories containing SKILL.md.
/// The caller passes the authoritative home directory (typically `config.home_dir`).
pub fn bundled_skills(home_dir: &std::path::Path) -> Vec<(&'static str, &'static str)> {
    disk_skills(home_dir)
        .into_iter()
        .map(|(name, content)| {
            let name: &'static str = Box::leak(name.into_boxed_str());
            let content: &'static str = Box::leak(content.into_boxed_str());
            (name, content)
        })
        .collect()
}

fn disk_skills(home_dir: &std::path::Path) -> Vec<(String, String)> {
    let mut results = Vec::new();
    let skills_dir = home_dir.join("skills");

    if let Ok(entries) = std::fs::read_dir(&skills_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            let skill_path = path.join("SKILL.md");
            if !skill_path.exists() {
                continue;
            }
            let content = match std::fs::read_to_string(&skill_path) {
                Ok(s) => s,
                Err(_) => continue,
            };
            results.push((name, content));
        }
    }

    results.sort_by(|a, b| a.0.cmp(&b.0));
    results
}

/// Parse a SKILL.md into a `SkillManifest`.
pub fn parse_bundled(name: &str, content: &str) -> Result<SkillManifest, crate::SkillError> {
    let converted = convert_skillmd_str(name, content)?;
    Ok(converted.manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bundled_valid_skill() {
        let content = r#"---
name: test-skill
description: "A test skill"
---
# Test Skill
Do something useful.
"#;
        let manifest = parse_bundled("test-skill", content).unwrap();
        assert_eq!(manifest.skill.name, "test-skill");
    }
}
