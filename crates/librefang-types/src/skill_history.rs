//! Closed vocabulary of `skill_versions.change_source` values (#8021).
//!
//! Every surface that records a skill version snapshot — the HTTP handlers in
//! `librefang-api`, the kernel's workshop/reviewer promotion paths and the
//! agent-facing `skill_evolve_*` tools in `librefang-runtime` — names the kind
//! of change it made from this one enumeration rather than a free-form string.
//!
//! Adding a write path means adding a variant, which is the point: the set of
//! ways a skill can change is reviewable in one match arm, and the value that
//! reaches `GET /api/skills/{name}/history` is pinned by
//! [`SkillWrite::as_str`] rather than derived from the variant name.

/// The kind of change a versioned skill write is making.
///
/// The `update` / `evolve` pair is the one distinction that is not
/// operation-by-operation: both are full rewrites through
/// `evolution::update_skill`, but `evolve` marks a rewrite made through the
/// evolve surface (the dashboard's evolve editor and the agent's
/// `skill_evolve_update` tool), while `update` marks one applied without it —
/// an approved workshop-update candidate or a background-reviewer refinement.
/// An operator filtering the history can therefore tell an interactive edit
/// from an automated one.
///
/// The schema default `unknown` (see `migration::migrate_v65`) covers rows
/// written by a writer that does not classify its persist; it deliberately has
/// no variant here because no surface should *choose* it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillWrite {
    /// A skill created where none existed — `POST /api/skills/create`, the
    /// `skill_evolve_create` tool and an approved workshop create-candidate.
    Create,
    /// A full rewrite applied without the evolve editor: an approved
    /// workshop-update candidate or a background-reviewer refinement.
    Update,
    /// A full rewrite through the evolve surface — `POST
    /// /api/skills/{name}/evolve/update` and the `skill_evolve_update` tool.
    Evolve,
    /// Fuzzy find-and-replace (`POST /api/skills/{name}/evolve/patch`, the
    /// `skill_evolve_patch` tool and a reviewer patch).
    Patch,
    /// Back to the skill's own previous `.evolution.json` snapshot
    /// (`POST /api/skills/{name}/evolve/rollback`).
    Rollback,
    /// A skill copied in from the local registry or a marketplace
    /// (`POST /api/skills/install`).
    Install,
    /// A user-initiated removal that accepts any source
    /// (`POST /api/skills/uninstall`).
    Uninstall,
    /// Agent-facing removal, local skills only
    /// (`POST /api/skills/{name}/evolve/delete`).
    Delete,
    /// Supporting-file add (`POST /api/skills/{name}/evolve/file`).
    FileWrite,
    /// Supporting-file removal (`DELETE /api/skills/{name}/evolve/file`).
    FileRemove,
    /// `POST /api/skills/{name}/history/{version_id}/restore` — the restore
    /// itself is a snapshot.
    Restore,
}

impl SkillWrite {
    /// The `change_source` recorded in history. Part of the API surface —
    /// `GET /api/skills/{name}/history` echoes it and clients key on it — so
    /// the strings are pinned rather than derived from the variant names.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Evolve => "evolve",
            Self::Patch => "patch",
            Self::Rollback => "rollback",
            Self::Install => "install",
            Self::Uninstall => "uninstall",
            Self::Delete => "delete",
            Self::FileWrite => "file-write",
            Self::FileRemove => "file-remove",
            Self::Restore => "restore",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SkillWrite;

    /// `change_source` is part of the API surface — `GET
    /// /api/skills/{name}/history` echoes it and clients key on it — so every
    /// variant's recorded string is pinned here. A variant added for a new
    /// write path has to pick its string deliberately rather than inherit
    /// whatever the variant happens to be named.
    #[test]
    fn change_source_vocabulary_is_stable() {
        assert_eq!(SkillWrite::Create.as_str(), "create");
        assert_eq!(SkillWrite::Update.as_str(), "update");
        assert_eq!(SkillWrite::Evolve.as_str(), "evolve");
        assert_eq!(SkillWrite::Patch.as_str(), "patch");
        assert_eq!(SkillWrite::Rollback.as_str(), "rollback");
        assert_eq!(SkillWrite::Install.as_str(), "install");
        assert_eq!(SkillWrite::Uninstall.as_str(), "uninstall");
        assert_eq!(SkillWrite::Delete.as_str(), "delete");
        assert_eq!(SkillWrite::FileWrite.as_str(), "file-write");
        assert_eq!(SkillWrite::FileRemove.as_str(), "file-remove");
        assert_eq!(SkillWrite::Restore.as_str(), "restore");
    }

    /// The strings are a closed set: no two variants may collapse to one
    /// value, or a history filtered by `change_source` would merge two
    /// different operations.
    #[test]
    fn change_source_strings_are_unique() {
        let all = [
            SkillWrite::Create,
            SkillWrite::Update,
            SkillWrite::Evolve,
            SkillWrite::Patch,
            SkillWrite::Rollback,
            SkillWrite::Install,
            SkillWrite::Uninstall,
            SkillWrite::Delete,
            SkillWrite::FileWrite,
            SkillWrite::FileRemove,
            SkillWrite::Restore,
        ];
        let mut strings: Vec<&str> = all.iter().map(|w| w.as_str()).collect();
        strings.sort_unstable();
        let count = strings.len();
        strings.dedup();
        assert_eq!(
            strings.len(),
            count,
            "duplicate change_source strings: {strings:?}"
        );
    }
}
