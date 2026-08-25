//! Agent-template lookup for workflow step agent types (refs #7712).
//!
//! A workflow step may reference an agent *type* — `agent = { type = "researcher" }` —
//! instead of a pre-registered instance.
//! Resolving that reference needs the template manifest of the same name, and the
//! caller needs to know *why* a lookup failed: a typo in the workflow, an
//! unreadable file, a corrupt manifest, and a manifest that describes a
//! different agent than the one asked for all demand different operator
//! actions, and collapsing them into "no such template" makes the corrupt
//! manifest indistinguishable from the typo.
//! [`TemplateLoadError`] keeps them apart.

use std::path::{Path, PathBuf};

use librefang_types::agent::AgentManifest;

/// File name every agent template directory is keyed by.
const MANIFEST_FILE: &str = "agent.toml";

/// Why loading the template manifest for an agent type failed.
///
/// Each variant names the path it was looking at, so the message an operator
/// reads points at the file they have to fix.
#[derive(Debug)]
pub enum TemplateLoadError {
    /// The type name cannot address a template directory at all.
    ///
    /// Rejected before touching the filesystem: a name carrying a path
    /// separator or `..` would otherwise let a workflow definition read an
    /// `agent.toml` from anywhere on the host.
    InvalidName { requested: String, reason: String },
    /// No `agent.toml` exists under any of the searched template directories.
    ///
    /// This is the typo case, and the only one where "try a different name"
    /// is the right advice — hence `searched`, so the operator can see which
    /// directories were actually consulted.
    Missing {
        requested: String,
        searched: Vec<PathBuf>,
    },
    /// A manifest file exists but could not be read (permissions, a dangling
    /// symlink, a directory where the file should be).
    Unreadable { path: PathBuf, detail: String },
    /// A manifest file exists and was read but is not a valid `AgentManifest`.
    Malformed { path: PathBuf, detail: String },
    /// The manifest declares a `name` other than the type that was requested.
    ///
    /// Spawning it anyway would register the agent under the *declared* name,
    /// so the next find-or-spawn for this type would miss the registry again
    /// and try to spawn a second copy — the type would never converge on one
    /// instance. Refuse instead, and name both halves of the mismatch.
    NameMismatch {
        path: PathBuf,
        requested: String,
        declared: String,
    },
}

impl std::fmt::Display for TemplateLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidName { requested, reason } => write!(
                f,
                "agent type '{requested}' is not a usable template name: {reason}"
            ),
            Self::Missing {
                requested,
                searched,
            } => {
                let dirs = searched
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(
                    f,
                    "no template named '{requested}' (searched {dirs} for {MANIFEST_FILE})"
                )
            }
            Self::Unreadable { path, detail } => write!(
                f,
                "template manifest {} exists but could not be read: {detail}",
                path.display()
            ),
            Self::Malformed { path, detail } => write!(
                f,
                "template manifest {} is malformed: {detail}",
                path.display()
            ),
            Self::NameMismatch {
                path,
                requested,
                declared,
            } => write!(
                f,
                "template manifest {} declares name '{declared}' but was loaded as agent type \
                 '{requested}'; rename the directory or the manifest so they agree",
                path.display()
            ),
        }
    }
}

impl std::error::Error for TemplateLoadError {}

impl TemplateLoadError {
    /// Short, stable discriminator for structured logs and assertions.
    ///
    /// Lets a caller log `kind = "malformed"` without formatting (and
    /// leaking) the whole path into a metric label.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::InvalidName { .. } => "invalid_name",
            Self::Missing { .. } => "missing",
            Self::Unreadable { .. } => "unreadable",
            Self::Malformed { .. } => "malformed",
            Self::NameMismatch { .. } => "name_mismatch",
        }
    }

    /// Whether the type simply does not exist, as opposed to existing and
    /// being broken.
    ///
    /// Only [`Self::Missing`] means "nothing here"; every other variant means
    /// a real file needs attention, and a caller that silently skips those is
    /// hiding a corrupt manifest behind a spelling suggestion.
    pub fn is_missing(&self) -> bool {
        matches!(self, Self::Missing { .. })
    }
}

/// The directories searched for an agent template, in precedence order.
///
/// `workspaces/agents/` is where an operator's own and dashboard-installed
/// templates live, and it shadows `registry/agents/` — the read-only registry
/// checkout — so a local edit of an upstream template wins.
/// This is the same precedence `librefang-kernel-router` uses for hands.
pub fn agent_template_dirs(home_dir: &Path) -> Vec<PathBuf> {
    vec![
        home_dir.join("workspaces").join("agents"),
        home_dir.join("registry").join("agents"),
    ]
}

/// Reject a type name that cannot safely address a template directory.
///
/// Mirrors the rule `librefang-api`'s `/api/templates/{name}` route enforces:
/// a simple, non-empty name with no path separators, no `..`, and no NUL.
fn validate_type_name(requested: &str) -> Result<(), TemplateLoadError> {
    let reason = if requested.is_empty() {
        Some("must not be empty")
    } else if requested.contains('/') || requested.contains('\\') {
        Some("must not contain a path separator")
    } else if requested == "." || requested == ".." {
        Some("must not be a relative path component")
    } else if requested.contains('\0') {
        Some("must not contain a NUL byte")
    } else {
        None
    };
    match reason {
        Some(reason) => Err(TemplateLoadError::InvalidName {
            requested: requested.to_string(),
            reason: reason.to_string(),
        }),
        None => Ok(()),
    }
}

/// Load the template manifest for an agent type.
///
/// Returns the manifest and the path it was read from, so the spawn that
/// follows can record a `source_toml_path` and a later hot-reload knows which
/// file backs the agent.
///
/// Search stops at the first directory that *has* the manifest file: a
/// `workspaces/agents/<type>/agent.toml` that fails to parse is reported as
/// [`TemplateLoadError::Malformed`] rather than falling through to the
/// registry copy, because silently serving a different manifest than the one
/// on the operator's disk is how an edit appears to have no effect.
pub fn load_agent_template(
    home_dir: &Path,
    requested: &str,
) -> Result<(AgentManifest, PathBuf), TemplateLoadError> {
    validate_type_name(requested)?;

    let dirs = agent_template_dirs(home_dir);
    for dir in &dirs {
        let path = dir.join(requested).join(MANIFEST_FILE);
        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            // Only an absent file continues the search. An existing file we
            // cannot read is a real failure and must not be reported as a
            // missing template.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                return Err(TemplateLoadError::Unreadable {
                    path,
                    detail: e.to_string(),
                })
            }
        };
        return parse_template(&path, requested, &content).map(|m| (m, path));
    }

    Err(TemplateLoadError::Missing {
        requested: requested.to_string(),
        searched: dirs,
    })
}

/// Parse one template manifest and pin its identity to the requested type.
///
/// The raw TOML is inspected for a `name` key before deserializing, because
/// `AgentManifest` carries `#[serde(default)]` and would otherwise turn an
/// omitted name into the literal `"unnamed"` — indistinguishable from a
/// template that really declares that name.
/// An omitted name is *pinned* to the requested type (the directory is the
/// identity); a declared name that disagrees is rejected.
fn parse_template(
    path: &Path,
    requested: &str,
    content: &str,
) -> Result<AgentManifest, TemplateLoadError> {
    let value: toml::Value = toml::from_str(content).map_err(|e| TemplateLoadError::Malformed {
        path: path.to_path_buf(),
        detail: e.to_string(),
    })?;
    let declared = value.get("name").map(|n| match n.as_str() {
        Some(s) => s.to_string(),
        // A non-string `name` is a manifest defect, not a mismatch; render it
        // so the error quotes what is actually in the file.
        None => n.to_string(),
    });
    let mut manifest: AgentManifest =
        value
            .try_into()
            .map_err(|e: toml::de::Error| TemplateLoadError::Malformed {
                path: path.to_path_buf(),
                detail: e.to_string(),
            })?;
    match declared {
        Some(declared) if declared != requested => {
            return Err(TemplateLoadError::NameMismatch {
                path: path.to_path_buf(),
                requested: requested.to_string(),
                declared,
            })
        }
        Some(_) => {}
        None => manifest.name = requested.to_string(),
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_template(home: &Path, dir: &str, name: &str, body: &str) -> PathBuf {
        let path = home.join(dir).join(name);
        std::fs::create_dir_all(&path).unwrap();
        let manifest = path.join(MANIFEST_FILE);
        std::fs::write(&manifest, body).unwrap();
        manifest
    }

    #[test]
    fn loads_a_template_from_workspaces_agents() {
        let tmp = tempfile::tempdir().unwrap();
        write_template(
            tmp.path(),
            "workspaces/agents",
            "researcher",
            "name = \"researcher\"\ndescription = \"digs\"\n",
        );
        let (manifest, path) = load_agent_template(tmp.path(), "researcher").unwrap();
        assert_eq!(manifest.name, "researcher");
        assert_eq!(manifest.description, "digs");
        assert!(path.ends_with("workspaces/agents/researcher/agent.toml"));
    }

    #[test]
    fn workspaces_agents_shadows_the_registry_checkout() {
        let tmp = tempfile::tempdir().unwrap();
        write_template(
            tmp.path(),
            "registry/agents",
            "researcher",
            "name = \"researcher\"\ndescription = \"upstream\"\n",
        );
        write_template(
            tmp.path(),
            "workspaces/agents",
            "researcher",
            "name = \"researcher\"\ndescription = \"local edit\"\n",
        );
        let (manifest, _) = load_agent_template(tmp.path(), "researcher").unwrap();
        assert_eq!(manifest.description, "local edit");
    }

    #[test]
    fn falls_through_to_the_registry_checkout_when_absent_locally() {
        let tmp = tempfile::tempdir().unwrap();
        write_template(
            tmp.path(),
            "registry/agents",
            "researcher",
            "name = \"researcher\"\n",
        );
        let (_, path) = load_agent_template(tmp.path(), "researcher").unwrap();
        assert!(path.ends_with("registry/agents/researcher/agent.toml"));
    }

    #[test]
    fn a_missing_template_names_the_directories_searched() {
        let tmp = tempfile::tempdir().unwrap();
        let err = load_agent_template(tmp.path(), "nope").unwrap_err();
        assert!(err.is_missing(), "{err}");
        assert_eq!(err.kind(), "missing");
        let rendered = err.to_string();
        // The message interpolates real paths, so the separator is the platform's: `workspaces/agents` on Unix and `workspaces\agents` on Windows.
        // Build the expected fragment the same way rather than hard-coding a forward slash, which passed everywhere except the Windows shard (#7889).
        let workspaces_agents: String =
            ["workspaces", "agents"].join(std::path::MAIN_SEPARATOR_STR);
        let registry_agents: String = ["registry", "agents"].join(std::path::MAIN_SEPARATOR_STR);
        assert!(rendered.contains(&workspaces_agents), "{rendered}");
        assert!(rendered.contains(&registry_agents), "{rendered}");
    }

    /// The review finding this module exists for: a corrupt manifest used to
    /// arrive as "no such template", so an operator chased a spelling mistake
    /// that was not there.
    #[test]
    fn a_corrupt_manifest_is_malformed_not_missing() {
        let tmp = tempfile::tempdir().unwrap();
        write_template(
            tmp.path(),
            "workspaces/agents",
            "researcher",
            "name = \"researcher\"\nthis is not toml [[[\n",
        );
        let err = load_agent_template(tmp.path(), "researcher").unwrap_err();
        assert!(!err.is_missing(), "{err}");
        assert_eq!(err.kind(), "malformed");
        assert!(err.to_string().contains("agent.toml"), "{err}");
    }

    #[test]
    fn a_manifest_with_a_bad_field_type_is_malformed() {
        let tmp = tempfile::tempdir().unwrap();
        write_template(
            tmp.path(),
            "workspaces/agents",
            "researcher",
            "name = \"researcher\"\nversion = 7\n",
        );
        let err = load_agent_template(tmp.path(), "researcher").unwrap_err();
        assert_eq!(err.kind(), "malformed");
    }

    /// A malformed local template must not be papered over by an intact
    /// registry copy — the operator edited the local file and needs to hear
    /// that their edit is broken.
    #[test]
    fn a_malformed_local_template_does_not_fall_through_to_the_registry() {
        let tmp = tempfile::tempdir().unwrap();
        write_template(
            tmp.path(),
            "registry/agents",
            "researcher",
            "name = \"researcher\"\n",
        );
        write_template(
            tmp.path(),
            "workspaces/agents",
            "researcher",
            "name = \"researcher\"\nbroken [[[\n",
        );
        let err = load_agent_template(tmp.path(), "researcher").unwrap_err();
        assert_eq!(err.kind(), "malformed");
        assert!(
            err.to_string().contains("workspaces"),
            "should name the local file, got: {err}"
        );
    }

    #[test]
    fn a_manifest_naming_a_different_agent_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        write_template(
            tmp.path(),
            "workspaces/agents",
            "researcher",
            "name = \"summarizer\"\n",
        );
        let err = load_agent_template(tmp.path(), "researcher").unwrap_err();
        assert_eq!(err.kind(), "name_mismatch");
        let rendered = err.to_string();
        assert!(rendered.contains("summarizer"), "{rendered}");
        assert!(rendered.contains("researcher"), "{rendered}");
    }

    #[test]
    fn a_manifest_without_a_name_is_pinned_to_the_requested_type() {
        let tmp = tempfile::tempdir().unwrap();
        write_template(
            tmp.path(),
            "workspaces/agents",
            "researcher",
            "description = \"no name key\"\n",
        );
        let (manifest, _) = load_agent_template(tmp.path(), "researcher").unwrap();
        assert_eq!(manifest.name, "researcher");
    }

    #[test]
    fn a_path_traversing_type_name_never_touches_the_filesystem() {
        let tmp = tempfile::tempdir().unwrap();
        for bad in ["../escape", "a/b", "", "..", "a\\b"] {
            let err = load_agent_template(tmp.path(), bad).unwrap_err();
            assert_eq!(err.kind(), "invalid_name", "{bad:?} -> {err}");
        }
    }
}
