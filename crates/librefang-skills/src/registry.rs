//! Skill registry — tracks installed skills and their tools.

use crate::openclaw_compat;
use crate::verify::SkillVerifier;
use crate::{InstalledSkill, SkillError, SkillManifest, SkillToolDef};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// Registry of installed skills.
#[derive(Debug, Default)]
pub struct SkillRegistry {
    /// Installed skills keyed by name.
    skills: HashMap<String, InstalledSkill>,
    /// Skills directory.
    skills_dir: PathBuf,
    /// When true, no new skills can be loaded (Stable mode).
    frozen: bool,
    /// Skill names that are globally disabled.
    disabled_skills: Vec<String>,
}

// ── Platform filtering ──────────────────────────────────────────────

/// Tags that hint at OS compatibility and should not be treated as
/// human-facing categories. Keep this list in sync with
/// [`is_platform_tag`].
pub const PLATFORM_TAGS: &[&str] = &[
    "macos",
    "linux",
    "windows",
    "macos-only",
    "linux-only",
    "windows-only",
];

/// True iff `tag` is a reserved platform-compatibility tag. Used to
/// separate category-style tags from OS constraints so UI grouping,
/// prompt grouping, and list filtering all agree on what "category"
/// actually means.
pub fn is_platform_tag(tag: &str) -> bool {
    PLATFORM_TAGS.contains(&tag)
}

/// Derive a human-facing category for a skill.
///
/// Precedence:
///  1. Explicit `[skill].category` field (not yet in manifest — reserved)
///  2. First non-platform tag in `tags`
///  3. Fallback string "general"
///
/// Call sites (API list handler, kernel prompt builder) share this so
/// the dashboard, system prompt, and CLI all group skills identically.
pub fn derive_category(manifest: &crate::SkillManifest) -> &str {
    manifest
        .skill
        .tags
        .iter()
        .map(String::as_str)
        .find(|t| !is_platform_tag(t))
        .unwrap_or("general")
}

/// Check if a skill is compatible with the current platform.
///
/// If the manifest declares no `tags` containing platform hints, the skill
/// loads on all platforms. Recognized platform tags: "macos", "linux", "windows".
fn skill_matches_platform(manifest: &crate::SkillManifest) -> bool {
    let platform_tags: Vec<&str> = manifest
        .skill
        .tags
        .iter()
        .filter(|t| is_platform_tag(t))
        .map(|t| t.as_str())
        .collect();

    if platform_tags.is_empty() {
        return true; // no platform restriction
    }

    let current = if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        return true; // unknown platform, allow all
    };

    platform_tags.iter().any(|tag| tag.starts_with(current))
}

impl SkillRegistry {
    /// Create a new registry rooted at the given skills directory.
    pub fn new(skills_dir: PathBuf) -> Self {
        Self {
            skills: HashMap::new(),
            skills_dir,
            frozen: false,
            disabled_skills: Vec::new(),
        }
    }

    /// Set the list of globally disabled skill names.
    pub fn set_disabled_skills(&mut self, disabled: Vec<String>) {
        self.disabled_skills = disabled;
    }

    /// Check if a skill name is in the disabled list.
    pub fn is_disabled(&self, name: &str) -> bool {
        self.disabled_skills.iter().any(|d| d == name)
    }

    /// Create a cheap owned snapshot of this registry.
    ///
    /// Used to avoid holding `RwLockReadGuard` across `.await` points
    /// (the guard is `!Send`).
    pub fn snapshot(&self) -> SkillRegistry {
        SkillRegistry {
            skills: self.skills.clone(),
            skills_dir: self.skills_dir.clone(),
            frozen: self.frozen,
            disabled_skills: self.disabled_skills.clone(),
        }
    }

    /// Freeze the registry, preventing any new skills from being loaded.
    /// Used in Stable mode after initial boot.
    pub fn freeze(&mut self) {
        self.frozen = true;
        info!("Skill registry frozen — no new skills will be loaded");
    }

    /// Check if the registry is frozen.
    pub fn is_frozen(&self) -> bool {
        self.frozen
    }

    /// Load all installed skills from the skills directory.
    pub fn load_all(&mut self) -> Result<usize, SkillError> {
        if !self.skills_dir.exists() {
            return Ok(0);
        }

        let mut count = 0;
        let entries = std::fs::read_dir(&self.skills_dir)?;

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let manifest_path = path.join("skill.toml");
            if !manifest_path.exists() {
                // Auto-detect SKILL.md and convert to skill.toml + prompt_context.md
                if openclaw_compat::detect_skillmd(&path) {
                    match openclaw_compat::convert_skillmd(&path) {
                        Ok(converted) => {
                            // SECURITY: Scan prompt content for injection attacks
                            // before accepting the skill. 341 malicious skills were
                            // found on ClawHub — block critical threats at load time.
                            let warnings =
                                SkillVerifier::scan_prompt_content(&converted.prompt_context);
                            let has_critical = warnings.iter().any(|w| {
                                matches!(w.severity, crate::verify::WarningSeverity::Critical)
                            });
                            if has_critical {
                                warn!(
                                    skill = %converted.manifest.skill.name,
                                    "BLOCKED: SKILL.md contains critical prompt injection patterns"
                                );
                                for w in &warnings {
                                    warn!("  [{:?}] {}", w.severity, w.message);
                                }
                                continue;
                            }
                            if !warnings.is_empty() {
                                for w in &warnings {
                                    warn!(
                                        skill = %converted.manifest.skill.name,
                                        "[{:?}] {}",
                                        w.severity,
                                        w.message
                                    );
                                }
                            }

                            info!(
                                skill = %converted.manifest.skill.name,
                                "Auto-converting SKILL.md to LibreFang format"
                            );
                            if let Err(e) = openclaw_compat::write_librefang_manifest(
                                &path,
                                &converted.manifest,
                            ) {
                                warn!("Failed to write skill.toml for {}: {e}", path.display());
                                continue;
                            }
                            if let Err(e) = openclaw_compat::write_prompt_context(
                                &path,
                                &converted.prompt_context,
                            ) {
                                warn!(
                                    "Failed to write prompt_context.md for {}: {e}",
                                    path.display()
                                );
                            }
                            // Fall through to load the newly written skill.toml
                        }
                        Err(e) => {
                            warn!("Failed to convert SKILL.md at {}: {e}", path.display());
                            continue;
                        }
                    }
                } else {
                    continue;
                }
            }

            match self.load_skill(&path) {
                Ok(_) => count += 1,
                Err(e) => {
                    warn!("Failed to load skill at {}: {e}", path.display());
                }
            }
        }

        info!("Loaded {count} skills from {}", self.skills_dir.display());
        Ok(count)
    }

    /// Load a single skill from a directory.
    ///
    /// Progressively loads skill resources:
    /// 1. Parse `skill.toml` manifest
    /// 2. Load `prompt_context.md` if the manifest lacks inline prompt context
    /// 3. Canonicalize the skill directory path for reliable entry-point resolution
    pub fn load_skill(&mut self, skill_dir: &Path) -> Result<String, SkillError> {
        if self.frozen {
            return Err(SkillError::NotFound(
                "Skill registry is frozen (Stable mode)".to_string(),
            ));
        }
        let manifest_path = skill_dir.join("skill.toml");
        let toml_str = std::fs::read_to_string(&manifest_path)?;
        let mut manifest: SkillManifest = toml::from_str(&toml_str)?;

        // Skip disabled skills
        if self.is_disabled(&manifest.skill.name) {
            info!(skill = %manifest.skill.name, "Skipping disabled skill");
            return Ok(manifest.skill.name);
        }

        // Skip skills incompatible with the current platform
        if !skill_matches_platform(&manifest) {
            info!(
                skill = %manifest.skill.name,
                "Skipping skill — incompatible with current platform"
            );
            return Ok(manifest.skill.name);
        }

        // Progressive loading: if prompt_context is not inlined in skill.toml,
        // try to load it from the companion prompt_context.md file.
        let needs_prompt_context = manifest
            .prompt_context
            .as_ref()
            .is_none_or(|ctx| ctx.is_empty());
        if needs_prompt_context {
            let prompt_path = skill_dir.join("prompt_context.md");
            if prompt_path.exists() {
                match std::fs::read_to_string(&prompt_path) {
                    Ok(content) if !content.is_empty() => {
                        manifest.prompt_context = Some(content);
                    }
                    Ok(_) => {}
                    Err(e) => {
                        warn!(
                            "Failed to read prompt_context.md for {}: {e}",
                            skill_dir.display()
                        );
                    }
                }
            }
        }

        let name = manifest.skill.name.clone();

        // Canonicalize the skill directory path so entry-point resolution
        // works regardless of the process working directory.
        let resolved_path =
            std::fs::canonicalize(skill_dir).unwrap_or_else(|_| skill_dir.to_path_buf());

        self.skills.insert(
            name.clone(),
            InstalledSkill {
                manifest,
                path: resolved_path,
                enabled: true,
            },
        );

        info!("Loaded skill: {name}");
        Ok(name)
    }

    /// Get an installed skill by name.
    pub fn get(&self, name: &str) -> Option<&InstalledSkill> {
        self.skills.get(name)
    }

    /// List all installed skills.
    pub fn list(&self) -> Vec<&InstalledSkill> {
        self.skills.values().collect()
    }

    /// Remove a skill by name.
    pub fn remove(&mut self, name: &str) -> Result<(), SkillError> {
        let skill = self
            .skills
            .remove(name)
            .ok_or_else(|| SkillError::NotFound(name.to_string()))?;

        // Remove the skill directory
        if skill.path.exists() {
            std::fs::remove_dir_all(&skill.path)?;
        }

        info!("Removed skill: {name}");
        Ok(())
    }

    /// Get all tool definitions from all enabled skills.
    pub fn all_tool_definitions(&self) -> Vec<SkillToolDef> {
        self.skills
            .values()
            .filter(|s| s.enabled)
            .flat_map(|s| s.manifest.tools.provided.iter().cloned())
            .collect()
    }

    /// Get tool definitions only from the named skills.
    pub fn tool_definitions_for_skills(&self, names: &[String]) -> Vec<SkillToolDef> {
        self.skills
            .values()
            .filter(|s| s.enabled && names.contains(&s.manifest.skill.name))
            .flat_map(|s| s.manifest.tools.provided.iter().cloned())
            .collect()
    }

    /// Return all installed skill names.
    pub fn skill_names(&self) -> Vec<String> {
        self.skills.keys().cloned().collect()
    }

    /// Find which skill provides a given tool name.
    pub fn find_tool_provider(&self, tool_name: &str) -> Option<&InstalledSkill> {
        self.skills.values().find(|s| {
            s.enabled
                && s.manifest
                    .tools
                    .provided
                    .iter()
                    .any(|t| t.name == tool_name)
        })
    }

    /// Count installed skills.
    pub fn count(&self) -> usize {
        self.skills.len()
    }

    /// Return the skills directory path.
    pub fn skills_dir(&self) -> &Path {
        &self.skills_dir
    }

    /// Reload a single skill from disk (hot-reload after evolution).
    ///
    /// Unlike `load_skill`, this works even when frozen — it only refreshes
    /// an existing entry, never adds a new one.
    pub fn reload_skill(&mut self, name: &str) -> Result<(), SkillError> {
        let skill = self
            .skills
            .get(name)
            .ok_or_else(|| SkillError::NotFound(name.to_string()))?;
        let skill_dir = skill.path.clone();
        // Preserve the prior enabled flag — evolution mutations must not
        // silently re-enable a skill the operator explicitly disabled.
        let prior_enabled = skill.enabled;

        // Re-read from disk
        let manifest_path = skill_dir.join("skill.toml");
        let toml_str = std::fs::read_to_string(&manifest_path)?;
        let mut manifest: SkillManifest = toml::from_str(&toml_str)?;

        // Progressive loading of prompt_context.md
        let needs_prompt_context = manifest
            .prompt_context
            .as_ref()
            .is_none_or(|ctx| ctx.is_empty());
        if needs_prompt_context {
            let prompt_path = skill_dir.join("prompt_context.md");
            if prompt_path.exists() {
                if let Ok(content) = std::fs::read_to_string(&prompt_path) {
                    if !content.is_empty() {
                        manifest.prompt_context = Some(content);
                    }
                }
            }
        }

        self.skills.insert(
            name.to_string(),
            InstalledSkill {
                manifest,
                path: skill_dir,
                enabled: prior_enabled,
            },
        );

        info!("Hot-reloaded skill: {name}");
        Ok(())
    }

    /// Update a skill's prompt_context in-memory and on disk via the evolution module.
    ///
    /// This is the primary path for agent-driven skill mutation.
    pub fn evolve_update(
        &mut self,
        name: &str,
        new_prompt_context: &str,
        changelog: &str,
        author: crate::evolution::EvolutionAuthor<'_>,
    ) -> Result<crate::evolution::EvolutionResult, SkillError> {
        let skill = self
            .skills
            .get(name)
            .ok_or_else(|| SkillError::NotFound(name.to_string()))?
            .clone();

        let result = crate::evolution::update_skill(&skill, new_prompt_context, changelog, author)?;
        self.reload_skill(name)?;
        Ok(result)
    }

    /// Patch a skill's prompt_context using fuzzy find-and-replace.
    pub fn evolve_patch(
        &mut self,
        name: &str,
        old_str: &str,
        new_str: &str,
        changelog: &str,
        replace_all: bool,
        author: crate::evolution::EvolutionAuthor<'_>,
    ) -> Result<crate::evolution::EvolutionResult, SkillError> {
        let skill = self
            .skills
            .get(name)
            .ok_or_else(|| SkillError::NotFound(name.to_string()))?
            .clone();

        let result = crate::evolution::patch_skill(
            &skill,
            old_str,
            new_str,
            changelog,
            replace_all,
            author,
        )?;
        self.reload_skill(name)?;
        Ok(result)
    }

    /// Rollback a skill to its previous version.
    pub fn evolve_rollback(
        &mut self,
        name: &str,
        author: crate::evolution::EvolutionAuthor<'_>,
    ) -> Result<crate::evolution::EvolutionResult, SkillError> {
        let skill = self
            .skills
            .get(name)
            .ok_or_else(|| SkillError::NotFound(name.to_string()))?
            .clone();

        let result = crate::evolution::rollback_skill(&skill, author)?;
        self.reload_skill(name)?;
        Ok(result)
    }

    /// Load workspace-scoped skills that override global/bundled skills.
    ///
    /// Load skills from external directories (read-only).
    ///
    /// External skills don't override local skills with the same name.
    /// Directories that don't exist are silently skipped.
    pub fn load_external_dirs(&mut self, dirs: &[PathBuf]) -> Result<usize, SkillError> {
        let mut count = 0;
        for dir in dirs {
            if !dir.exists() || !dir.is_dir() {
                continue;
            }
            let entries = std::fs::read_dir(dir)?;
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let manifest_path = path.join("skill.toml");
                if !manifest_path.exists() {
                    // Try auto-convert SKILL.md
                    if openclaw_compat::detect_skillmd(&path) {
                        if let Ok(converted) = openclaw_compat::convert_skillmd(&path) {
                            let _ = openclaw_compat::write_librefang_manifest(
                                &path,
                                &converted.manifest,
                            );
                            let _ = openclaw_compat::write_prompt_context(
                                &path,
                                &converted.prompt_context,
                            );
                        }
                    }
                    if !path.join("skill.toml").exists() {
                        continue;
                    }
                }

                // Read manifest to check name collision
                if let Ok(toml_str) = std::fs::read_to_string(path.join("skill.toml")) {
                    if let Ok(manifest) = toml::from_str::<SkillManifest>(&toml_str) {
                        // Local skills take precedence — skip if name already loaded
                        if self.skills.contains_key(&manifest.skill.name) {
                            continue;
                        }
                    }
                }

                match self.load_skill(&path) {
                    Ok(_) => count += 1,
                    Err(e) => {
                        warn!("Failed to load external skill at {}: {e}", path.display());
                    }
                }
            }
        }
        if count > 0 {
            info!(
                "Loaded {count} external skill(s) from {} dir(s)",
                dirs.len()
            );
        }
        Ok(count)
    }

    /// Scans subdirectories of `workspace_skills_dir` using the same loading
    /// logic as `load_all()`: auto-converts SKILL.md, runs prompt injection
    /// scan, blocks critical threats. Skills loaded here override global ones
    /// with the same name (insert semantics).
    pub fn load_workspace_skills(
        &mut self,
        workspace_skills_dir: &Path,
    ) -> Result<usize, SkillError> {
        if !workspace_skills_dir.exists() {
            return Ok(0);
        }
        if self.frozen {
            return Err(SkillError::NotFound(
                "Skill registry is frozen (Stable mode)".to_string(),
            ));
        }

        let mut count = 0;
        let entries = std::fs::read_dir(workspace_skills_dir)?;

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let manifest_path = path.join("skill.toml");
            if !manifest_path.exists() {
                // Auto-detect SKILL.md and convert
                if openclaw_compat::detect_skillmd(&path) {
                    match openclaw_compat::convert_skillmd(&path) {
                        Ok(converted) => {
                            let warnings =
                                SkillVerifier::scan_prompt_content(&converted.prompt_context);
                            let has_critical = warnings.iter().any(|w| {
                                matches!(w.severity, crate::verify::WarningSeverity::Critical)
                            });
                            if has_critical {
                                warn!(
                                    skill = %converted.manifest.skill.name,
                                    "BLOCKED workspace skill: critical prompt injection patterns"
                                );
                                continue;
                            }

                            if let Err(e) = openclaw_compat::write_librefang_manifest(
                                &path,
                                &converted.manifest,
                            ) {
                                warn!("Failed to write skill.toml for {}: {e}", path.display());
                                continue;
                            }
                            if let Err(e) = openclaw_compat::write_prompt_context(
                                &path,
                                &converted.prompt_context,
                            ) {
                                warn!(
                                    "Failed to write prompt_context.md for {}: {e}",
                                    path.display()
                                );
                            }
                        }
                        Err(e) => {
                            warn!(
                                "Failed to convert workspace SKILL.md at {}: {e}",
                                path.display()
                            );
                            continue;
                        }
                    }
                } else {
                    continue;
                }
            }

            match self.load_skill(&path) {
                Ok(name) => {
                    info!("Loaded workspace skill: {name}");
                    count += 1;
                }
                Err(e) => {
                    warn!("Failed to load workspace skill at {}: {e}", path.display());
                }
            }
        }

        if count > 0 {
            info!(
                "Loaded {count} workspace skill(s) from {}",
                workspace_skills_dir.display()
            );
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_manifest(tags: &[&str]) -> crate::SkillManifest {
        crate::SkillManifest {
            skill: crate::SkillMeta {
                name: "t".into(),
                version: "0.1.0".into(),
                description: String::new(),
                author: String::new(),
                license: String::new(),
                tags: tags.iter().map(|s| s.to_string()).collect(),
            },
            runtime: Default::default(),
            tools: Default::default(),
            requirements: Default::default(),
            prompt_context: None,
            source: None,
            config: Default::default(),
        }
    }

    #[test]
    fn derive_category_skips_platform_tags() {
        // First tag is a platform tag — must fall through to the next.
        let m = make_manifest(&["macos", "devops"]);
        assert_eq!(derive_category(&m), "devops");
    }

    #[test]
    fn derive_category_only_platform_tags_falls_back_to_general() {
        let m = make_manifest(&["linux"]);
        assert_eq!(derive_category(&m), "general");
    }

    #[test]
    fn derive_category_no_tags_returns_general() {
        let m = make_manifest(&[]);
        assert_eq!(derive_category(&m), "general");
    }

    #[test]
    fn derive_category_first_non_platform_wins() {
        let m = make_manifest(&["data", "linux", "pipeline"]);
        assert_eq!(derive_category(&m), "data");
    }

    fn create_test_skill(dir: &Path, name: &str) {
        let skill_dir = dir.join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("skill.toml"),
            format!(
                r#"
[skill]
name = "{name}"
version = "0.1.0"
description = "Test skill"

[runtime]
type = "python"
entry = "main.py"

[[tools.provided]]
name = "{name}_tool"
description = "A test tool"
input_schema = {{ type = "object" }}
"#
            ),
        )
        .unwrap();
    }

    #[test]
    fn test_load_all() {
        let dir = TempDir::new().unwrap();
        create_test_skill(dir.path(), "skill-a");
        create_test_skill(dir.path(), "skill-b");

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        let count = registry.load_all().unwrap();
        assert_eq!(count, 2);
        assert_eq!(registry.count(), 2);
    }

    #[test]
    fn test_get_skill() {
        let dir = TempDir::new().unwrap();
        create_test_skill(dir.path(), "my-skill");

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        registry.load_all().unwrap();

        let skill = registry.get("my-skill");
        assert!(skill.is_some());
        assert_eq!(skill.unwrap().manifest.skill.name, "my-skill");
    }

    #[test]
    fn test_tool_definitions() {
        let dir = TempDir::new().unwrap();
        create_test_skill(dir.path(), "alpha");
        create_test_skill(dir.path(), "beta");

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        registry.load_all().unwrap();

        let tools = registry.all_tool_definitions();
        assert_eq!(tools.len(), 2);
    }

    #[test]
    fn test_find_tool_provider() {
        let dir = TempDir::new().unwrap();
        create_test_skill(dir.path(), "finder");

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        registry.load_all().unwrap();

        assert!(registry.find_tool_provider("finder_tool").is_some());
        assert!(registry.find_tool_provider("nonexistent").is_none());
    }

    #[test]
    fn test_remove_skill() {
        let dir = TempDir::new().unwrap();
        create_test_skill(dir.path(), "removable");

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        registry.load_all().unwrap();
        assert_eq!(registry.count(), 1);

        registry.remove("removable").unwrap();
        assert_eq!(registry.count(), 0);
    }

    #[test]
    fn test_empty_dir() {
        let dir = TempDir::new().unwrap();
        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        assert_eq!(registry.load_all().unwrap(), 0);
    }

    #[test]
    fn test_frozen_blocks_load() {
        let dir = TempDir::new().unwrap();
        create_test_skill(dir.path(), "blocked");

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        registry.freeze();
        assert!(registry.is_frozen());

        // Trying to load a skill should fail
        let result = registry.load_skill(&dir.path().join("blocked"));
        assert!(result.is_err());
    }

    #[test]
    fn test_frozen_after_initial_load() {
        let dir = TempDir::new().unwrap();
        create_test_skill(dir.path(), "initial");
        create_test_skill(dir.path(), "later");

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        // Initial load works
        registry.load_all().unwrap();
        assert_eq!(registry.count(), 2);

        // Freeze
        registry.freeze();

        // Dynamic load blocked
        create_test_skill(dir.path(), "new-skill");
        let result = registry.load_skill(&dir.path().join("new-skill"));
        assert!(result.is_err());
        // Still has the original skills
        assert_eq!(registry.count(), 2);
    }

    #[test]
    fn test_registry_auto_convert_skillmd() {
        let dir = TempDir::new().unwrap();

        // Create a SKILL.md-only skill (no skill.toml)
        let skill_dir = dir.path().join("writing-coach");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: writing-coach\ndescription: Helps improve writing\n---\n# Writing Coach\n\nHelp users write better.",
        ).unwrap();

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        let count = registry.load_all().unwrap();
        assert_eq!(count, 1, "Should auto-convert and load the SKILL.md skill");

        let skill = registry.get("writing-coach");
        assert!(skill.is_some());
        let manifest = &skill.unwrap().manifest;
        assert_eq!(
            manifest.runtime.runtime_type,
            crate::SkillRuntime::PromptOnly
        );
        assert!(manifest.prompt_context.is_some());

        // Verify that skill.toml was written
        assert!(skill_dir.join("skill.toml").exists());
    }

    #[test]
    fn test_progressive_prompt_context_loading() {
        let dir = TempDir::new().unwrap();
        let skill_dir = dir.path().join("context-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();

        // Create a skill.toml WITHOUT inline prompt_context
        std::fs::write(
            skill_dir.join("skill.toml"),
            r#"
[skill]
name = "context-skill"
version = "0.1.0"
description = "A skill with external prompt context"
"#,
        )
        .unwrap();

        // Create a companion prompt_context.md file
        std::fs::write(
            skill_dir.join("prompt_context.md"),
            "# Context Skill\n\nYou are a helpful context-aware assistant.",
        )
        .unwrap();

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        let count = registry.load_all().unwrap();
        assert_eq!(count, 1);

        let skill = registry.get("context-skill").unwrap();
        // Progressive loading should have picked up prompt_context.md
        assert!(
            skill.manifest.prompt_context.is_some(),
            "prompt_context should be loaded from prompt_context.md"
        );
        assert!(skill
            .manifest
            .prompt_context
            .as_ref()
            .unwrap()
            .contains("context-aware assistant"));
    }

    #[test]
    fn test_skill_path_is_absolute() {
        let dir = TempDir::new().unwrap();
        create_test_skill(dir.path(), "abs-path-skill");

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        registry.load_all().unwrap();

        let skill = registry.get("abs-path-skill").unwrap();
        assert!(
            skill.path.is_absolute(),
            "Skill path should be absolute for reliable entry-point resolution"
        );
    }
}
