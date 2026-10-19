//! Skill evolution tools — create / update / patch / delete / rollback /
//! write_file / remove_file, plus `skill_read_file` for reading companion
//! files from an installed skill directory.
//!
//! Migrated from `Result<String, String>` to `Result<String, ToolError>`
//! (#3576). A missing registry -> `Unavailable("Skill registry")`; the freeze
//! gate and the read-file allowlist / absolute-path / containment guards ->
//! `PermissionDenied` (messages preserved); missing params ->
//! `MissingParameter`; a skill that can't be loaded -> `NotFound`; the
//! `librefang_skills::evolution` operations (typed `SkillError`) ->
//! `ToolError::upstream`; JSON serialization via `?`.

use super::error::{ToolError, ToolResult};
use crate::kernel_handle::prelude::*;
use librefang_skills::registry::SkillRegistry;
use librefang_types::skill_history::SkillWrite;
use std::path::Path;
use std::sync::Arc;

/// Build the author tag for an agent-triggered evolution. Use the
/// agent's id so the dashboard history can attribute the change.
fn agent_author_tag(caller: Option<&str>) -> String {
    caller
        .map(|id| format!("agent:{id}"))
        .unwrap_or_else(|| "agent".to_string())
}

/// Read the v1 snapshot (`skill.toml` + `prompt_context.md`) from an installed
/// skill directory.
///
/// `None` when the manifest is missing or unreadable: a snapshot without its
/// manifest cannot be restored, so recording half a snapshot would only
/// produce a corrupt history row. A missing `prompt_context.md` records as an
/// empty string — an inline-prompt or prompt-less skill is a legal shape.
fn read_skill_snapshot(skill_dir: &Path) -> Option<(String, String)> {
    let manifest_toml = std::fs::read_to_string(skill_dir.join("skill.toml")).ok()?;
    let prompt_context =
        std::fs::read_to_string(skill_dir.join("prompt_context.md")).unwrap_or_default();
    Some((manifest_toml, prompt_context))
}

/// Whether the v1 history scope covers this skill directory.
///
/// v1 keys history by skill name alone, so only skills under the registry's
/// global `skills_dir` are recorded: a workspace or hand skill shadowing a
/// global name would otherwise conflate two different skills under one key.
/// Workspace/hand skills are explicitly out of scope in v1 — see the
/// `skill_version_store` module docs.
fn is_global_skill_dir(registry: &SkillRegistry, skill_dir: &Path) -> bool {
    let global = std::fs::canonicalize(registry.skills_dir())
        .unwrap_or_else(|_| registry.skills_dir().to_path_buf());
    let dir = std::fs::canonicalize(skill_dir).unwrap_or_else(|_| skill_dir.to_path_buf());
    dir.starts_with(&global)
}

/// Best-effort record of one skill snapshot after a successful mutation.
///
/// Called *after* the content is already on disk, so a failure costs one
/// history row, never the write — the same semantics as
/// `record_template_version` on the API side. `snapshot` carries a pre-read
/// snapshot for the delete path, where the files are gone by the time the
/// operation has succeeded; every other caller passes `None` and the files are
/// read from `skill_dir` here.
fn record_skill_snapshot(
    kernel: Option<&Arc<dyn KernelHandle>>,
    registry: &SkillRegistry,
    skill_dir: &Path,
    skill_name: &str,
    author: &str,
    source: &str,
    snapshot: Option<(String, String)>,
) {
    let Some(kernel) = kernel else {
        return;
    };
    if !is_global_skill_dir(registry, skill_dir) {
        tracing::debug!(
            skill = skill_name,
            dir = %skill_dir.display(),
            "skill history: workspace/hand skill — v1 records global skills only"
        );
        return;
    }
    let Some((manifest_toml, prompt_context)) = snapshot.or_else(|| read_skill_snapshot(skill_dir))
    else {
        tracing::warn!(
            skill = skill_name,
            dir = %skill_dir.display(),
            "skill history: skill.toml unreadable after a successful mutation; snapshot skipped"
        );
        return;
    };
    if let Err(e) =
        kernel.record_skill_version(skill_name, &manifest_toml, &prompt_context, author, source)
    {
        tracing::warn!(
            skill = skill_name,
            error = %e,
            "Failed to record skill version snapshot"
        );
    }
}

/// Reject evolution ops when the registry is frozen (Stable mode).
///
/// The registry's frozen flag is meant to express "no skill changes in
/// this kernel", but the evolution module writes to disk directly and
/// then triggers `reload_skills`, which no-ops under freeze. Without
/// this gate, an agent running under Stable mode would silently
/// persist skill mutations that'd be picked up at the next unfreeze
/// or restart — defeating the whole point of the mode.
fn ensure_not_frozen(registry: &SkillRegistry) -> Result<(), ToolError> {
    if registry.is_frozen() {
        Err(ToolError::PermissionDenied(
            "Skill registry is frozen (Stable mode) — skill evolution is disabled".to_string(),
        ))
    } else {
        Ok(())
    }
}

/// Typed `NotFound` for a skill that the registry / on-disk loader can't find.
fn skill_not_found(name: &str) -> ToolError {
    ToolError::NotFound {
        kind: "Skill",
        id: name.to_string(),
    }
}

pub(super) async fn tool_skill_evolve_create(
    input: &serde_json::Value,
    skill_registry: Option<&SkillRegistry>,
    caller_agent_id: Option<&str>,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> ToolResult {
    let registry = skill_registry.ok_or(ToolError::Unavailable("Skill registry"))?;
    ensure_not_frozen(registry)?;
    let name = input["name"]
        .as_str()
        .ok_or(ToolError::MissingParameter("name"))?;
    let description = input["description"]
        .as_str()
        .ok_or(ToolError::MissingParameter("description"))?;
    let prompt_context = input["prompt_context"]
        .as_str()
        .ok_or(ToolError::MissingParameter("prompt_context"))?;
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
    let result = librefang_skills::evolution::create_skill(
        skills_dir,
        name,
        description,
        prompt_context,
        tags,
        Some(&author),
    )
    .map_err(ToolError::upstream)?;
    record_skill_snapshot(
        kernel,
        registry,
        &skills_dir.join(&result.skill_name),
        &result.skill_name,
        &author,
        SkillWrite::Create.as_str(),
        None,
    );
    Ok(serde_json::to_string(&result)?)
}

pub(super) async fn tool_skill_evolve_update(
    input: &serde_json::Value,
    skill_registry: Option<&SkillRegistry>,
    caller_agent_id: Option<&str>,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> ToolResult {
    let registry = skill_registry.ok_or(ToolError::Unavailable("Skill registry"))?;
    ensure_not_frozen(registry)?;
    let name = input["name"]
        .as_str()
        .ok_or(ToolError::MissingParameter("name"))?;
    let prompt_context = input["prompt_context"]
        .as_str()
        .ok_or(ToolError::MissingParameter("prompt_context"))?;
    let changelog = input["changelog"]
        .as_str()
        .ok_or(ToolError::MissingParameter("changelog"))?;

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
            .map_err(|_| skill_not_found(name))?;
            &skill_owned
        }
    };

    let author = agent_author_tag(caller_agent_id);
    let result =
        librefang_skills::evolution::update_skill(skill, prompt_context, changelog, Some(&author))
            .map_err(ToolError::upstream)?;
    // The evolve surface's full rewrite records as `evolve`; the reviewer and
    // workshop-approve paths record the same operation as `update` — see the
    // shared `SkillWrite` vocabulary in `librefang_types::skill_history`.
    record_skill_snapshot(
        kernel,
        registry,
        &skill.path,
        &result.skill_name,
        &author,
        SkillWrite::Evolve.as_str(),
        None,
    );
    Ok(serde_json::to_string(&result)?)
}

pub(super) async fn tool_skill_evolve_patch(
    input: &serde_json::Value,
    skill_registry: Option<&SkillRegistry>,
    caller_agent_id: Option<&str>,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> ToolResult {
    let registry = skill_registry.ok_or(ToolError::Unavailable("Skill registry"))?;
    ensure_not_frozen(registry)?;
    let name = input["name"]
        .as_str()
        .ok_or(ToolError::MissingParameter("name"))?;
    let old_string = input["old_string"]
        .as_str()
        .ok_or(ToolError::MissingParameter("old_string"))?;
    let new_string = input["new_string"]
        .as_str()
        .ok_or(ToolError::MissingParameter("new_string"))?;
    let changelog = input["changelog"]
        .as_str()
        .ok_or(ToolError::MissingParameter("changelog"))?;
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
            .map_err(|_| skill_not_found(name))?;
            &skill_owned
        }
    };

    let author = agent_author_tag(caller_agent_id);
    let result = librefang_skills::evolution::patch_skill(
        skill,
        old_string,
        new_string,
        changelog,
        replace_all,
        Some(&author),
    )
    .map_err(ToolError::upstream)?;
    record_skill_snapshot(
        kernel,
        registry,
        &skill.path,
        &result.skill_name,
        &author,
        SkillWrite::Patch.as_str(),
        None,
    );
    Ok(serde_json::to_string(&result)?)
}

pub(super) async fn tool_skill_evolve_delete(
    input: &serde_json::Value,
    skill_registry: Option<&SkillRegistry>,
    caller_agent_id: Option<&str>,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> ToolResult {
    let registry = skill_registry.ok_or(ToolError::Unavailable("Skill registry"))?;
    ensure_not_frozen(registry)?;
    let name = input["name"]
        .as_str()
        .ok_or(ToolError::MissingParameter("name"))?;

    // Resolve the actual installed skill's parent directory instead of
    // blindly targeting `registry.skills_dir() + name`. Workspace skills
    // shadow global skills with the same name in an agent run; without
    // this, `skill_evolve_delete` removed the global skill (or reported
    // NotFound) while leaving the workspace copy the agent was actually
    // using in place — destructive against the wrong resource.
    let (parent, skill_dir) = match registry.get(name) {
        Some(s) => {
            let parent = s
                .path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| registry.skills_dir().to_path_buf());
            (parent, s.path.clone())
        }
        // Fall back to the global dir when the registry hasn't caught up
        // yet (e.g. a skill created in this same turn hasn't been
        // hot-reloaded into the live view) — delete_skill will return
        // NotFound if nothing exists there either.
        None => (
            registry.skills_dir().to_path_buf(),
            registry.skills_dir().join(name),
        ),
    };
    // Read the snapshot *before* the delete: the files this row records are
    // what the delete is about to remove, and they cannot be read afterwards.
    // The row itself is only written once the delete has succeeded.
    let snapshot = read_skill_snapshot(&skill_dir);
    let result =
        librefang_skills::evolution::delete_skill(&parent, name).map_err(ToolError::upstream)?;
    let author = agent_author_tag(caller_agent_id);
    record_skill_snapshot(
        kernel,
        registry,
        &skill_dir,
        &result.skill_name,
        &author,
        SkillWrite::Delete.as_str(),
        snapshot,
    );
    Ok(serde_json::to_string(&result)?)
}

pub(super) async fn tool_skill_evolve_rollback(
    input: &serde_json::Value,
    skill_registry: Option<&SkillRegistry>,
    caller_agent_id: Option<&str>,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> ToolResult {
    let registry = skill_registry.ok_or(ToolError::Unavailable("Skill registry"))?;
    ensure_not_frozen(registry)?;
    let name = input["name"]
        .as_str()
        .ok_or(ToolError::MissingParameter("name"))?;

    // Same-turn create→rollback fallback (see tool_skill_evolve_update).
    let skill_owned;
    let skill = match registry.get(name) {
        Some(s) => s,
        None => {
            skill_owned = librefang_skills::evolution::load_installed_skill_from_disk(
                registry.skills_dir(),
                name,
            )
            .map_err(|_| skill_not_found(name))?;
            &skill_owned
        }
    };

    let author = agent_author_tag(caller_agent_id);
    let result = librefang_skills::evolution::rollback_skill(skill, Some(&author))
        .map_err(ToolError::upstream)?;
    record_skill_snapshot(
        kernel,
        registry,
        &skill.path,
        &result.skill_name,
        &author,
        SkillWrite::Rollback.as_str(),
        None,
    );
    Ok(serde_json::to_string(&result)?)
}

pub(super) async fn tool_skill_evolve_write_file(
    input: &serde_json::Value,
    skill_registry: Option<&SkillRegistry>,
    caller_agent_id: Option<&str>,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> ToolResult {
    let registry = skill_registry.ok_or(ToolError::Unavailable("Skill registry"))?;
    ensure_not_frozen(registry)?;
    let name = input["name"]
        .as_str()
        .ok_or(ToolError::MissingParameter("name"))?;
    let path = input["path"]
        .as_str()
        .ok_or(ToolError::MissingParameter("path"))?;
    let content = input["content"]
        .as_str()
        .ok_or(ToolError::MissingParameter("content"))?;

    // Same-turn create→write_file fallback.
    let skill_owned;
    let skill = match registry.get(name) {
        Some(s) => s,
        None => {
            skill_owned = librefang_skills::evolution::load_installed_skill_from_disk(
                registry.skills_dir(),
                name,
            )
            .map_err(|_| skill_not_found(name))?;
            &skill_owned
        }
    };

    let result = librefang_skills::evolution::write_supporting_file(skill, path, content)
        .map_err(ToolError::upstream)?;
    // A supporting file is not part of the v1 snapshot, so this row pins the
    // event (and the skill's unchanged toml/prompt) rather than a diff.
    let author = agent_author_tag(caller_agent_id);
    record_skill_snapshot(
        kernel,
        registry,
        &skill.path,
        &result.skill_name,
        &author,
        SkillWrite::FileWrite.as_str(),
        None,
    );
    Ok(serde_json::to_string(&result)?)
}

pub(super) async fn tool_skill_evolve_remove_file(
    input: &serde_json::Value,
    skill_registry: Option<&SkillRegistry>,
    caller_agent_id: Option<&str>,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> ToolResult {
    let registry = skill_registry.ok_or(ToolError::Unavailable("Skill registry"))?;
    ensure_not_frozen(registry)?;
    let name = input["name"]
        .as_str()
        .ok_or(ToolError::MissingParameter("name"))?;
    let path = input["path"]
        .as_str()
        .ok_or(ToolError::MissingParameter("path"))?;

    // Same-turn fallback (see tool_skill_evolve_update).
    let skill_owned;
    let skill = match registry.get(name) {
        Some(s) => s,
        None => {
            skill_owned = librefang_skills::evolution::load_installed_skill_from_disk(
                registry.skills_dir(),
                name,
            )
            .map_err(|_| skill_not_found(name))?;
            &skill_owned
        }
    };

    let result = librefang_skills::evolution::remove_supporting_file(skill, path)
        .map_err(ToolError::upstream)?;
    let author = agent_author_tag(caller_agent_id);
    record_skill_snapshot(
        kernel,
        registry,
        &skill.path,
        &result.skill_name,
        &author,
        SkillWrite::FileRemove.as_str(),
        None,
    );
    Ok(serde_json::to_string(&result)?)
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
) -> ToolResult {
    let registry = skill_registry.ok_or(ToolError::Unavailable("Skill registry"))?;
    let skill_name = input["skill"]
        .as_str()
        .ok_or(ToolError::MissingParameter("skill"))?;
    let rel_path = input["path"]
        .as_str()
        .ok_or(ToolError::MissingParameter("path"))?;

    // Enforce agent skill allowlist: if the agent specifies allowed skills
    // (non-empty list), only those skills can be read. Empty = all allowed.
    if let Some(allowed) = allowed_skills {
        if !allowed.is_empty() && !allowed.iter().any(|s| s == skill_name) {
            return Err(ToolError::PermissionDenied(format!(
                "Access denied: agent is not allowed to access skill '{skill_name}'"
            )));
        }
    }

    // Reject absolute paths early — Path::join replaces the base when given
    // an absolute path, which would bypass the skill directory containment.
    if std::path::Path::new(rel_path).is_absolute() {
        return Err(ToolError::PermissionDenied(
            "Access denied: absolute paths are not allowed".to_string(),
        ));
    }

    // Look up the skill
    let skill = registry
        .get(skill_name)
        .ok_or_else(|| skill_not_found(skill_name))?;

    // Resolve the path relative to the skill directory
    let requested = skill.path.join(rel_path);
    let canonical = requested
        .canonicalize()
        .map_err(|e| ToolError::upstream_msg(format!("File not found: {e}")))?;
    let skill_root = skill
        .path
        .canonicalize()
        .map_err(|e| ToolError::upstream_msg(format!("Skill directory error: {e}")))?;

    // Security: ensure the resolved path is within the skill directory
    if !canonical.starts_with(&skill_root) {
        return Err(ToolError::PermissionDenied(format!(
            "Access denied: '{rel_path}' is outside the skill directory"
        )));
    }

    // Read the file
    let content = tokio::fs::read_to_string(&canonical)
        .await
        .map_err(|e| ToolError::Upstream {
            message: format!("Failed to read '{rel_path}': {e}"),
            source: Some(Box::new(e)),
        })?;

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
