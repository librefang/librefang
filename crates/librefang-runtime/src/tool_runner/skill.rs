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

/// Values a caller must capture *before* a mutation, when the post-mutation
/// directory can no longer yield them: the snapshot the writer captured under
/// its lock and the pre-delete scope decision.
///
/// `content` is always the `*_with_snapshot` writer's return value, never a
/// post-lock re-read: another writer can land between the unlock and the
/// record, and the row must hold the bytes this write produced.
/// `is_global: None` makes the recorder canonicalize the directory now (every
/// caller whose directory still exists).
#[derive(Default)]
struct PreMutationSnapshot {
    content: Option<(String, String)>,
    is_global: Option<bool>,
}

/// Best-effort record of one skill snapshot after a successful mutation.
///
/// Called *after* the content is already on disk, so a failure costs one
/// history row, never the write — the same semantics as
/// `record_template_version` on the API side.
///
/// The delete path also passes the pre-mutation scope decision because
/// `is_global_skill_dir` canonicalizes `skill_dir`, which fails once the
/// delete has removed it, and the fallback (the uncleared path) does not match
/// a symlinked skills root's canonical form — the row would be silently
/// dropped.
fn record_skill_snapshot(
    kernel: Option<&Arc<dyn KernelHandle>>,
    registry: &SkillRegistry,
    skill_dir: &Path,
    skill_name: &str,
    author: &str,
    source: &str,
    pre: PreMutationSnapshot,
) {
    let Some(kernel) = kernel else {
        return;
    };
    if !pre
        .is_global
        .unwrap_or_else(|| is_global_skill_dir(registry, skill_dir))
    {
        tracing::debug!(
            skill = skill_name,
            dir = %skill_dir.display(),
            "skill history: workspace/hand skill — v1 records global skills only"
        );
        return;
    }
    let Some((manifest_toml, prompt_context)) = pre.content else {
        tracing::warn!(
            skill = skill_name,
            dir = %skill_dir.display(),
            "skill history: writer could not capture skill.toml; snapshot skipped"
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
    let (result, snapshot) = librefang_skills::evolution::create_skill_with_snapshot(
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
        PreMutationSnapshot {
            content: snapshot,
            is_global: None,
        },
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
    let (result, snapshot) = librefang_skills::evolution::update_skill_with_snapshot(
        skill,
        prompt_context,
        changelog,
        Some(&author),
    )
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
        PreMutationSnapshot {
            content: snapshot,
            is_global: None,
        },
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
    let (result, snapshot) = librefang_skills::evolution::patch_skill_with_snapshot(
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
        PreMutationSnapshot {
            content: snapshot,
            is_global: None,
        },
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
    // The scope check must happen before the delete: `is_global_skill_dir`
    // canonicalizes `skill_dir`, and a canonicalize of a symlinked root does
    // not match its post-delete fallback form, silently dropping the row. The
    // content snapshot comes from the writer, captured under its lock.
    let is_global = is_global_skill_dir(registry, &skill_dir);
    let (result, snapshot) = librefang_skills::evolution::delete_skill_with_snapshot(&parent, name)
        .map_err(ToolError::upstream)?;
    let author = agent_author_tag(caller_agent_id);
    record_skill_snapshot(
        kernel,
        registry,
        &skill_dir,
        &result.skill_name,
        &author,
        SkillWrite::Delete.as_str(),
        PreMutationSnapshot {
            content: snapshot,
            is_global: Some(is_global),
        },
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
    let (result, snapshot) =
        librefang_skills::evolution::rollback_skill_with_snapshot(skill, Some(&author))
            .map_err(ToolError::upstream)?;
    record_skill_snapshot(
        kernel,
        registry,
        &skill.path,
        &result.skill_name,
        &author,
        SkillWrite::Rollback.as_str(),
        PreMutationSnapshot {
            content: snapshot,
            is_global: None,
        },
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

    let (result, snapshot) =
        librefang_skills::evolution::write_supporting_file_with_snapshot(skill, path, content)
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
        PreMutationSnapshot {
            content: snapshot,
            is_global: None,
        },
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

    let (result, snapshot) =
        librefang_skills::evolution::remove_supporting_file_with_snapshot(skill, path)
            .map_err(ToolError::upstream)?;
    let author = agent_author_tag(caller_agent_id);
    record_skill_snapshot(
        kernel,
        registry,
        &skill.path,
        &result.skill_name,
        &author,
        SkillWrite::FileRemove.as_str(),
        PreMutationSnapshot {
            content: snapshot,
            is_global: None,
        },
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

#[cfg(test)]
mod raw_writer_drift_guard {
    //! Every tool write must record the snapshot its `*_with_snapshot` writer
    //! captured under the per-skill lock. A raw writer call followed by a
    //! re-read records whatever a later writer left on disk, not the bytes
    //! this call produced.

    /// The module's own source: what this scan sees is exactly what rustc
    /// compiled.
    const SOURCE: &str = include_str!("skill.rs");

    /// Raw writers whose call sites must be zero here.
    const RAW_WRITERS: &[&str] = &[
        "evolution::create_skill",
        "evolution::update_skill",
        "evolution::patch_skill",
        "evolution::rollback_skill",
        "evolution::delete_skill",
        "evolution::write_supporting_file",
        "evolution::remove_supporting_file",
    ];

    /// 0-based line range of `signature`'s body, closing brace included.
    fn body_lines(signature: &str) -> std::ops::Range<usize> {
        let lines: Vec<&str> = SOURCE.lines().collect();
        let start = lines
            .iter()
            .position(|line| line.contains(signature) && !line.contains('"'))
            .unwrap_or_else(|| panic!("`{signature}` not found"));
        let end = lines[start + 1..]
            .iter()
            .position(|line| line.trim_end() == "}")
            .map(|offset| start + 1 + offset + 1)
            .unwrap_or_else(|| panic!("closing brace of `{signature}` not found"));
        start..end
    }

    #[test]
    fn raw_evolution_writers_have_no_call_sites() {
        let lines: Vec<&str> = SOURCE.lines().collect();
        for writer in RAW_WRITERS {
            let needle = format!("{writer}(");
            let call_lines: Vec<usize> = lines
                .iter()
                .enumerate()
                .filter(|(_, line)| line.contains(&needle))
                .map(|(index, _)| index)
                .collect();
            assert!(
                call_lines.is_empty(),
                "`{needle}` is called at skill.rs:{}; use `{writer}_with_snapshot` so the \
                 snapshot the writer produced reaches the history row.",
                call_lines.first().map(|index| index + 1).unwrap_or(0)
            );
        }
    }

    /// The recorder must consume the captured snapshot only. A filesystem read
    /// inside it would race every writer that lands between the unlock and the
    /// record.
    #[test]
    fn record_helper_never_rereads_disk() {
        let lines: Vec<&str> = SOURCE.lines().collect();
        let body = body_lines("fn record_skill_snapshot(");
        let start = body.start;
        for (index, line) in lines[body].iter().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            assert!(
                !code.contains("fs::read") && !code.contains("read_to_string"),
                "record_skill_snapshot reads the filesystem at skill.rs:{}; record the \
                 snapshot captured under the writer's lock instead.",
                start + index + 1
            );
        }
    }
}
