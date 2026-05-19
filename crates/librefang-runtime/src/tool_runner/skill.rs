//! Skill evolution tools — create / update / patch / delete / rollback /
//! write_file / remove_file, plus `skill_read_file` for reading companion
//! files from an installed skill directory.

use librefang_skills::registry::SkillRegistry;

/// Build the author tag for an agent-triggered evolution. Use the
/// agent's id so the dashboard history can attribute the change.
fn agent_author_tag(caller: Option<&str>) -> String {
    caller
        .map(|id| format!("agent:{id}"))
        .unwrap_or_else(|| "agent".to_string())
}

/// Reject evolution ops when the registry is frozen (Stable mode).
///
/// The registry's frozen flag is meant to express "no skill changes in
/// this kernel", but the evolution module writes to disk directly and
/// then triggers `reload_skills`, which no-ops under freeze. Without
/// this gate, an agent running under Stable mode would silently
/// persist skill mutations that'd be picked up at the next unfreeze
/// or restart — defeating the whole point of the mode.
fn ensure_not_frozen(registry: &SkillRegistry) -> Result<(), String> {
    if registry.is_frozen() {
        Err("Skill registry is frozen (Stable mode) — skill evolution is disabled".to_string())
    } else {
        Ok(())
    }
}

pub(super) async fn tool_skill_evolve_create(
    input: &serde_json::Value,
    skill_registry: Option<&SkillRegistry>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let registry = skill_registry.ok_or("Skill registry not available")?;
    ensure_not_frozen(registry)?;
    let name = input["name"].as_str().ok_or("Missing 'name' parameter")?;
    let description = input["description"]
        .as_str()
        .ok_or("Missing 'description' parameter")?;
    let prompt_context = input["prompt_context"]
        .as_str()
        .ok_or("Missing 'prompt_context' parameter")?;
    let tags: Vec<String> = input["tags"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let author = agent_author_tag(caller_agent_id);
    let skills_dir = registry.skills_dir();
    match librefang_skills::evolution::create_skill(
        skills_dir,
        name,
        description,
        prompt_context,
        tags,
        Some(&author),
    ) {
        Ok(result) => serde_json::to_string(&result).map_err(|e| e.to_string()),
        Err(e) => Err(format!("Failed to create skill: {e}")),
    }
}

pub(super) async fn tool_skill_evolve_update(
    input: &serde_json::Value,
    skill_registry: Option<&SkillRegistry>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let registry = skill_registry.ok_or("Skill registry not available")?;
    ensure_not_frozen(registry)?;
    let name = input["name"].as_str().ok_or("Missing 'name' parameter")?;
    let prompt_context = input["prompt_context"]
        .as_str()
        .ok_or("Missing 'prompt_context' parameter")?;
    let changelog = input["changelog"]
        .as_str()
        .ok_or("Missing 'changelog' parameter")?;

    // Registry hot-reload happens AFTER the turn finishes, so within
    // the same turn `create` followed by `update` would find the
    // registry cache still stale. Fall back to loading straight from
    // disk when the cache misses — if the skill truly doesn't exist
    // the helper returns NotFound too.
    let skill_owned;
    let skill = match registry.get(name) {
        Some(s) => s,
        None => {
            skill_owned = librefang_skills::evolution::load_installed_skill_from_disk(
                registry.skills_dir(),
                name,
            )
            .map_err(|e| format!("Skill '{name}' not found: {e}"))?;
            &skill_owned
        }
    };

    let author = agent_author_tag(caller_agent_id);
    match librefang_skills::evolution::update_skill(skill, prompt_context, changelog, Some(&author))
    {
        Ok(result) => serde_json::to_string(&result).map_err(|e| e.to_string()),
        Err(e) => Err(format!("Failed to update skill: {e}")),
    }
}

pub(super) async fn tool_skill_evolve_patch(
    input: &serde_json::Value,
    skill_registry: Option<&SkillRegistry>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let registry = skill_registry.ok_or("Skill registry not available")?;
    ensure_not_frozen(registry)?;
    let name = input["name"].as_str().ok_or("Missing 'name' parameter")?;
    let old_string = input["old_string"]
        .as_str()
        .ok_or("Missing 'old_string' parameter")?;
    let new_string = input["new_string"]
        .as_str()
        .ok_or("Missing 'new_string' parameter")?;
    let changelog = input["changelog"]
        .as_str()
        .ok_or("Missing 'changelog' parameter")?;
    let replace_all = input["replace_all"].as_bool().unwrap_or(false);

    // Same-turn create→patch fallback (see tool_skill_evolve_update).
    let skill_owned;
    let skill = match registry.get(name) {
        Some(s) => s,
        None => {
            skill_owned = librefang_skills::evolution::load_installed_skill_from_disk(
                registry.skills_dir(),
                name,
            )
            .map_err(|e| format!("Skill '{name}' not found: {e}"))?;
            &skill_owned
        }
    };

    let author = agent_author_tag(caller_agent_id);
    match librefang_skills::evolution::patch_skill(
        skill,
        old_string,
        new_string,
        changelog,
        replace_all,
        Some(&author),
    ) {
        Ok(result) => serde_json::to_string(&result).map_err(|e| e.to_string()),
        Err(e) => Err(format!("Failed to patch skill: {e}")),
    }
}

pub(super) async fn tool_skill_evolve_delete(
    input: &serde_json::Value,
    skill_registry: Option<&SkillRegistry>,
) -> Result<String, String> {
    let registry = skill_registry.ok_or("Skill registry not available")?;
    ensure_not_frozen(registry)?;
    let name = input["name"].as_str().ok_or("Missing 'name' parameter")?;

    // Resolve the actual installed skill's parent directory instead of
    // blindly targeting `registry.skills_dir() + name`. Workspace skills
    // shadow global skills with the same name in an agent run; without
    // this, `skill_evolve_delete` removed the global skill (or reported
    // NotFound) while leaving the workspace copy the agent was actually
    // using in place — destructive against the wrong resource.
    let parent = match registry.get(name) {
        Some(s) => s
            .path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| registry.skills_dir().to_path_buf()),
        // Fall back to the global dir when the registry hasn't caught up
        // yet (e.g. a skill created in this same turn hasn't been
        // hot-reloaded into the live view) — delete_skill will return
        // NotFound if nothing exists there either.
        None => registry.skills_dir().to_path_buf(),
    };
    match librefang_skills::evolution::delete_skill(&parent, name) {
        Ok(result) => serde_json::to_string(&result).map_err(|e| e.to_string()),
        Err(e) => Err(format!("Failed to delete skill: {e}")),
    }
}

pub(super) async fn tool_skill_evolve_rollback(
    input: &serde_json::Value,
    skill_registry: Option<&SkillRegistry>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let registry = skill_registry.ok_or("Skill registry not available")?;
    ensure_not_frozen(registry)?;
    let name = input["name"].as_str().ok_or("Missing 'name' parameter")?;

    // Same-turn create→rollback fallback (see tool_skill_evolve_update).
    let skill_owned;
    let skill = match registry.get(name) {
        Some(s) => s,
        None => {
            skill_owned = librefang_skills::evolution::load_installed_skill_from_disk(
                registry.skills_dir(),
                name,
            )
            .map_err(|e| format!("Skill '{name}' not found: {e}"))?;
            &skill_owned
        }
    };

    let author = agent_author_tag(caller_agent_id);
    match librefang_skills::evolution::rollback_skill(skill, Some(&author)) {
        Ok(result) => serde_json::to_string(&result).map_err(|e| e.to_string()),
        Err(e) => Err(format!("Failed to rollback skill: {e}")),
    }
}

pub(super) async fn tool_skill_evolve_write_file(
    input: &serde_json::Value,
    skill_registry: Option<&SkillRegistry>,
) -> Result<String, String> {
    let registry = skill_registry.ok_or("Skill registry not available")?;
    ensure_not_frozen(registry)?;
    let name = input["name"].as_str().ok_or("Missing 'name' parameter")?;
    let path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let content = input["content"]
        .as_str()
        .ok_or("Missing 'content' parameter")?;

    // Same-turn create→write_file fallback.
    let skill_owned;
    let skill = match registry.get(name) {
        Some(s) => s,
        None => {
            skill_owned = librefang_skills::evolution::load_installed_skill_from_disk(
                registry.skills_dir(),
                name,
            )
            .map_err(|e| format!("Skill '{name}' not found: {e}"))?;
            &skill_owned
        }
    };

    match librefang_skills::evolution::write_supporting_file(skill, path, content) {
        Ok(result) => serde_json::to_string(&result).map_err(|e| e.to_string()),
        Err(e) => Err(format!("Failed to write file: {e}")),
    }
}

pub(super) async fn tool_skill_evolve_remove_file(
    input: &serde_json::Value,
    skill_registry: Option<&SkillRegistry>,
) -> Result<String, String> {
    let registry = skill_registry.ok_or("Skill registry not available")?;
    ensure_not_frozen(registry)?;
    let name = input["name"].as_str().ok_or("Missing 'name' parameter")?;
    let path = input["path"].as_str().ok_or("Missing 'path' parameter")?;

    // Same-turn fallback (see tool_skill_evolve_update).
    let skill_owned;
    let skill = match registry.get(name) {
        Some(s) => s,
        None => {
            skill_owned = librefang_skills::evolution::load_installed_skill_from_disk(
                registry.skills_dir(),
                name,
            )
            .map_err(|e| format!("Skill '{name}' not found: {e}"))?;
            &skill_owned
        }
    };

    match librefang_skills::evolution::remove_supporting_file(skill, path) {
        Ok(result) => serde_json::to_string(&result).map_err(|e| e.to_string()),
        Err(e) => Err(format!("Failed to remove file: {e}")),
    }
}

/// Read a companion file from an installed skill directory.
///
/// Security: resolves the path relative to the skill's installed directory and
/// rejects any path that escapes via `..` or absolute components. Symlinks are
/// resolved by `canonicalize()` before the containment check, so a symlink
/// pointing outside the skill directory is correctly rejected.
pub(super) async fn tool_skill_read_file(
    input: &serde_json::Value,
    skill_registry: Option<&SkillRegistry>,
    allowed_skills: Option<&[String]>,
) -> Result<String, String> {
    let registry = skill_registry.ok_or("Skill registry not available")?;
    let skill_name = input["skill"].as_str().ok_or("Missing 'skill' parameter")?;
    let rel_path = input["path"].as_str().ok_or("Missing 'path' parameter")?;

    // Enforce agent skill allowlist: if the agent specifies allowed skills
    // (non-empty list), only those skills can be read. Empty = all allowed.
    if let Some(allowed) = allowed_skills {
        if !allowed.is_empty() && !allowed.iter().any(|s| s == skill_name) {
            return Err(format!(
                "Access denied: agent is not allowed to access skill '{skill_name}'"
            ));
        }
    }

    // Reject absolute paths early — Path::join replaces the base when given
    // an absolute path, which would bypass the skill directory containment.
    if std::path::Path::new(rel_path).is_absolute() {
        return Err("Access denied: absolute paths are not allowed".to_string());
    }

    // Look up the skill
    let skill = registry
        .get(skill_name)
        .ok_or_else(|| format!("Skill '{}' not found", skill_name))?;

    // Resolve the path relative to the skill directory
    let requested = skill.path.join(rel_path);
    let canonical = requested
        .canonicalize()
        .map_err(|e| format!("File not found: {}", e))?;
    let skill_root = skill
        .path
        .canonicalize()
        .map_err(|e| format!("Skill directory error: {}", e))?;

    // Security: ensure the resolved path is within the skill directory
    if !canonical.starts_with(&skill_root) {
        return Err(format!(
            "Access denied: '{}' is outside the skill directory",
            rel_path
        ));
    }

    // Read the file
    let content = tokio::fs::read_to_string(&canonical)
        .await
        .map_err(|e| format!("Failed to read '{}': {}", rel_path, e))?;

    // Fire-and-forget usage tracking — only count when the agent actually
    // loads the skill's core prompt content, not every supporting file
    // read. Reading references/templates/scripts/assets shouldn't inflate
    // the usage metric. Failures (lock contention, disk error) must not
    // affect tool execution, so we swallow them.
    let is_core_prompt = matches!(rel_path, "prompt_context.md" | "SKILL.md" | "skill.md");
    if is_core_prompt {
        let skill_dir = skill.path.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = librefang_skills::evolution::record_skill_usage(&skill_dir) {
                tracing::debug!(error = %e, dir = %skill_dir.display(), "record_skill_usage failed");
            }
        });
    }

    // Cap output to avoid flooding the context.
    // Use floor_char_boundary to avoid panicking on multi-byte UTF-8.
    const MAX_BYTES: usize = 32_000;
    if content.len() > MAX_BYTES {
        let truncate_at = content.floor_char_boundary(MAX_BYTES);
        Ok(format!(
            "{}\n\n... (truncated at {} bytes, file is {} bytes total)",
            &content[..truncate_at],
            truncate_at,
            content.len()
        ))
    } else {
        Ok(content)
    }
}
