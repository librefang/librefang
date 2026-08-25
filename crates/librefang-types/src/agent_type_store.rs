//! The on-disk agent-type store — one flat `{name}.toml` per operator-authored agent type under `$LIBREFANG_HOME/agent-types/`.
//!
//! This module exists because an agent type now has two authors, not one.
//! `POST /api/templates` is written by a human through the dashboard editor; the `agent_type_create` tool (#7722) is written by an agent mid-conversation.
//! Both land in the same directory and are read back by the same `GET`, so any rule that lives on only one of the two paths is a rule the other silently does not have — which is how the name-shadowing check, the `deny_unknown_fields` refusal and the race-free claim all become optional depending on who happened to make the call.
//!
//! So the write itself lives here, once, and both surfaces call [`create_agent_type`].
//! What stays on each surface is only the part that genuinely differs: HTTP status codes and translated operator-facing strings on the API side, `ToolError` variants and model-readable prose on the tool side.
//!
//! The construction of the manifest is [`AgentTypeSpec::into_new_manifest`] in the sibling [`crate::agent_type`] module, and nothing here reaches around it.
//! That is the guarantee #7740 bought: a create cannot invent its own field defaults, because the only constructor is an exhaustive struct literal that fails to compile when `AgentManifest` grows a field nobody has decided about yet.

use crate::agent::AgentManifest;
use crate::agent_type::AgentTypeSpec;
use std::path::PathBuf;

/// Resolve the LibreFang home directory: `LIBREFANG_HOME` if set, otherwise `~/.librefang`.
///
/// Read live on every call rather than cached, because the integration suites set the variable per test binary and a cached value would leak one suite's fixtures into another's assertions.
fn librefang_home() -> PathBuf {
    if let Ok(home) = std::env::var("LIBREFANG_HOME") {
        return PathBuf::from(home);
    }
    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".librefang")
}

/// Operator-authored agent types, one flat `{name}.toml` per type.
///
/// Flat rather than a directory per type on purpose: the whole document is a single manifest, so a create or an edit is exactly one atomic rename with nothing to leave half-built if the process dies between two writes.
pub fn agent_types_dir() -> PathBuf {
    agent_types_dir_in(&librefang_home())
}

/// The agent-type store under an explicitly supplied home directory.
///
/// The kernel resolves an agent type against a `home_dir` it was handed rather than against the process environment, so the layout has to be expressible without reading `LIBREFANG_HOME` again.
/// Both spellings route through this one function so a reader and a writer can never disagree about where the store is (#6699).
pub fn agent_types_dir_in(home_dir: &std::path::Path) -> PathBuf {
    home_dir.join("agent-types")
}

/// The file backing one agent type. Call [`validate_agent_type_name`] before joining an untrusted name onto this.
pub fn agent_type_path(name: &str) -> PathBuf {
    agent_type_path_in(&librefang_home(), name)
}

/// The file backing one agent type under an explicitly supplied home directory.
pub fn agent_type_path_in(home_dir: &std::path::Path, name: &str) -> PathBuf {
    agent_types_dir_in(home_dir).join(format!("{name}.toml"))
}

/// Live agent workspaces — the second, read-only source of the agent-type catalog.
pub fn workspace_agents_dir() -> PathBuf {
    librefang_home().join("workspaces").join("agents")
}

/// The `agent.toml` of a live agent, which the catalog lists but this store never writes.
pub fn workspace_agent_manifest_path(name: &str) -> PathBuf {
    workspace_agents_dir().join(name).join("agent.toml")
}

/// Validate an agent-type name before it is joined onto the store directory.
///
/// Only permits `[A-Za-z0-9_-]` to guarantee the result cannot escape the base directory through `..`, absolute paths, or platform separators (`/`, `\`).
/// Rejects empty names and anything longer than 64 chars to cap log noise.
///
/// The same rule governs the URL path segment of `/api/templates/{name}` and the `name` argument of the `agent_type_create` tool, so a name one surface accepts is a name the other can address.
pub fn validate_agent_type_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty() || name.len() > 64 {
        return Err("invalid agent type name");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err("invalid agent type name");
    }
    Ok(())
}

/// Why a create was refused.
///
/// Each variant is a distinct thing the caller did, because each one has a distinct remedy and the two surfaces render them differently: the API maps them onto 400 / 409 / 500, the tool onto a `ToolError` the model can act on next turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateAgentTypeError {
    /// The name is empty, over 64 characters, or contains something other than `[A-Za-z0-9_-]`.
    InvalidName,
    /// A live agent already answers to this name.
    /// An agent type sharing it would win every subsequent catalog read and make the agent unreachable through `/api/templates/{name}`, so the collision is refused rather than resolved.
    ShadowsLiveAgent,
    /// An agent type of this name already exists. The existing file is untouched.
    NameTaken,
    /// Rendering or writing the manifest failed. The payload is a diagnostic for the log, not for an untrusted caller.
    Io(String),
}

impl std::fmt::Display for CreateAgentTypeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidName => write!(
                f,
                "an agent type name must be 1-64 characters of letters, digits, '_' or '-'"
            ),
            Self::ShadowsLiveAgent => write!(
                f,
                "that name belongs to a live agent, which is managed through /api/agents rather than the agent-type catalog"
            ),
            Self::NameTaken => write!(f, "an agent type with that name already exists"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for CreateAgentTypeError {}

/// A newly written agent type, as both the parsed manifest and the exact bytes that reached disk.
///
/// The rendered TOML is returned rather than re-read, so a caller that echoes the stored document back is echoing what was actually written instead of racing a concurrent edit.
#[derive(Debug, Clone)]
pub struct CreatedAgentType {
    pub name: String,
    pub manifest: AgentManifest,
    pub manifest_toml: String,
}

/// Create a new agent type from the flat spec, refusing to overwrite anything.
///
/// `name` is the caller's validated identity for the type — a URL path segment, or a tool argument — and `spec.name` is ignored, because identity is the one field a create must not take on trust from a body it did not address.
///
/// The order of the checks is load-bearing: the name is validated before it is ever joined onto a path, and the live-agent shadow check happens before the file is claimed so a refused create leaves nothing behind.
pub fn create_agent_type(
    name: &str,
    spec: AgentTypeSpec,
) -> Result<CreatedAgentType, CreateAgentTypeError> {
    validate_agent_type_name(name).map_err(|_| CreateAgentTypeError::InvalidName)?;
    if workspace_agent_manifest_path(name).exists() {
        return Err(CreateAgentTypeError::ShadowsLiveAgent);
    }

    let manifest = spec.into_new_manifest(name.to_string());
    let rendered = toml::to_string_pretty(&manifest).map_err(|e| {
        CreateAgentTypeError::Io(format!("failed to render agent type '{name}': {e}"))
    })?;

    let dir = agent_types_dir();
    std::fs::create_dir_all(&dir).map_err(|e| {
        CreateAgentTypeError::Io(format!("failed to create {}: {e}", dir.display()))
    })?;

    let path = agent_type_path(name);
    // `Path::exists()` followed by a write is check-then-act: two concurrent creates of the same name both observe "absent" and the second silently replaces the first, which is exactly the refusal this function promises.
    // Claiming the path with `File::create_new` — an atomic create-if-absent at the OS level — lets exactly one of them through.
    match std::fs::File::create_new(&path) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(CreateAgentTypeError::NameTaken)
        }
        Err(e) => {
            return Err(CreateAgentTypeError::Io(format!(
                "failed to claim agent type '{name}': {e}"
            )))
        }
    }

    // The claim is filled by the same atomic rename every other write here uses, and removed again if that fails, so a failed create leaves no empty file behind for the catalog to trip over.
    if let Err(e) = atomic_write(&path, rendered.as_bytes()) {
        let _ = std::fs::remove_file(&path);
        return Err(CreateAgentTypeError::Io(format!(
            "failed to write agent type '{name}': {e}"
        )));
    }

    Ok(CreatedAgentType {
        name: name.to_string(),
        manifest,
        manifest_toml: rendered,
    })
}

/// Serialize a manifest over an agent type that already exists, in one atomic rename.
///
/// This is the edit path's landing, and it deliberately does not construct anything: the caller has already read the stored manifest and applied [`AgentTypeSpec::apply_to`] over it, so every field outside the flat shape is the one that was on disk a moment ago.
pub fn persist_agent_type(name: &str, manifest: &AgentManifest) -> Result<String, String> {
    let rendered = toml::to_string_pretty(manifest)
        .map_err(|e| format!("failed to render agent type '{name}': {e}"))?;
    let dir = agent_types_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("failed to create {}: {e}", dir.display()))?;
    atomic_write(&agent_type_path(name), rendered.as_bytes())
        .map_err(|e| format!("failed to write agent type '{name}': {e}"))?;
    Ok(rendered)
}

/// Write `content` to `path` via a sibling temp file plus rename.
///
/// `std::fs::write` truncates in place, so a failure partway through leaves a truncated `agent.toml` where a valid one used to be — the worst possible failure mode for a file the daemon parses at spawn.
/// The staging name carries the process id and a per-process counter so concurrent writers never share one, and the file is `sync_all`-ed before the rename so the rename cannot publish an empty inode.
/// On Unix the parent directory is synced afterwards so the new directory entry survives a power loss.
fn atomic_write(path: &std::path::Path, content: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);

    let file_name = path
        .file_name()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "missing filename"))?;
    let mut tmp_name = file_name.to_os_string();
    tmp_name.push(format!(".{}.{seq}.tmp", std::process::id()));
    let mut tmp = path.to_path_buf();
    tmp.set_file_name(tmp_name);

    let staged = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(content)?;
        f.sync_all()
    })();
    if let Err(e) = staged {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }

    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }

    // `Path::parent()` answers `Some("")` for a bare relative filename, so map the empty-but-present case to `.` rather than failing an `open("")` after the rename already succeeded.
    #[cfg(unix)]
    {
        let parent = match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => std::path::Path::new("."),
        };
        std::fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_simple_names() {
        assert!(validate_agent_type_name("assistant").is_ok());
        assert!(validate_agent_type_name("customer-support").is_ok());
        assert!(validate_agent_type_name("coder_v2").is_ok());
        assert!(validate_agent_type_name("a1").is_ok());
    }

    #[test]
    fn rejects_path_traversal() {
        assert!(validate_agent_type_name("..").is_err());
        assert!(validate_agent_type_name("../../etc").is_err());
        assert!(validate_agent_type_name("foo/../bar").is_err());
        assert!(validate_agent_type_name("..\\..\\tmp").is_err());
    }

    #[test]
    fn rejects_separators_and_absolute_paths() {
        assert!(validate_agent_type_name("foo/bar").is_err());
        assert!(validate_agent_type_name("foo\\bar").is_err());
        assert!(validate_agent_type_name("/etc/passwd").is_err());
        assert!(validate_agent_type_name("C:\\Windows").is_err());
    }

    #[test]
    fn rejects_empty_and_oversized() {
        assert!(validate_agent_type_name("").is_err());
        assert!(validate_agent_type_name(&"a".repeat(65)).is_err());
    }

    #[test]
    fn rejects_null_and_special_chars() {
        assert!(validate_agent_type_name("foo\0bar").is_err());
        assert!(validate_agent_type_name("foo bar").is_err());
        assert!(validate_agent_type_name("foo.bar").is_err());
        assert!(validate_agent_type_name("foo%2fbar").is_err());
    }
}
