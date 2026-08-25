//! Transient mission workspaces (#7723).
//!
//! A mission workspace is a scratch directory handed to a short-lived run — an ephemeral worker spawned from an agent type — so tools that need to write intermediate files have somewhere to put them that is guaranteed to disappear when the run ends.
//! It lives under `<librefang_home>/transient/<label>-<uid>`, is created before the run starts, and is removed when the [`MissionWorkspace`] guard is dropped, on the success path and the failure path alike.
//!
//! The directory name is derived from caller-supplied text — an agent-type name, or a manifest `name` field parsed out of a template TOML.
//! That text is not trustworthy: a template carrying `name = "../../evil"` would otherwise turn `PathBuf::join` into an escape from the transient root, and because the same path is later handed to `std::fs::remove_dir_all`, the escape is an attacker-influenced recursive delete rather than a merely misplaced `mkdir`.
//! Every path this module produces is therefore built from a sanitized single component, created with a non-clobbering `create_dir`, and verified to canonicalize back inside the transient root before anything is written to it or deleted from it.
//!
//! Crash robustness: the guard runs in-process, so a hard kill (SIGKILL, power loss, panic in a thread that aborts the process) leaves the directory behind.
//! Nothing else references it, so it is inert — it costs disk until the next daemon boot, when [`sweep_orphan_missions`] empties the transient root.
//! Boot is the correct moment for that sweep because it is the one point at which no mission of this daemon can be running.

use super::workspace_setup::{ensure_workspace, safe_path_component};
use crate::error::{KernelError, KernelResult};
use librefang_types::error::LibreFangError;
use std::path::{Path, PathBuf};

/// Directory under the LibreFang home that holds every mission workspace.
pub const TRANSIENT_DIR_NAME: &str = "transient";

/// Number of hex characters appended to a mission directory name.
///
/// Sixteen bits short of a full UUID is ample: the suffix only has to keep concurrent missions of the same agent type apart, and the non-clobbering `create_dir` below turns a collision into a retry rather than into two runs sharing one directory.
const UID_LEN: usize = 8;

/// Longest sanitized label accepted as the directory-name prefix.
///
/// Matches the 64-character cap the ephemeral spawn path applies to a requested agent type, so a label that arrived through that guard is never truncated by this one.
const MAX_LABEL_LEN: usize = 64;

/// How many fresh uids to try before giving up on a name collision.
const CREATE_ATTEMPTS: usize = 4;

/// A scratch directory owned by one mission, removed when this value is dropped.
///
/// Construct it with [`MissionWorkspace::create`] before the run starts and keep it alive for exactly as long as the run.
/// Dropping it — whether the run returned a result, returned an error, or unwound through a panic — removes the directory and everything in it.
#[derive(Debug)]
pub struct MissionWorkspace {
    /// Canonical absolute path of the mission directory.
    path: PathBuf,
    /// Directory name, also usable as the mission agent's uid-style display name.
    name: String,
}

impl MissionWorkspace {
    /// Create a mission workspace for `label` under `<home_dir>/transient`.
    ///
    /// `label` is caller-supplied and is treated as hostile: it is reduced to `[A-Za-z0-9_-]`, truncated, and replaced by the uid alone when nothing survives sanitization, so the result is always exactly one path component.
    /// The returned directory carries the standard agent workspace layout, so a mission agent pointed at it finds the same `.identity/`, `output/`, `memory/` … subdirectories a permanent agent would.
    pub fn create(home_dir: &Path, label: &str) -> KernelResult<Self> {
        let root = ensure_transient_root(home_dir)?;
        let mut last_err = None;
        for _ in 0..CREATE_ATTEMPTS {
            let name = mission_dir_name(label);
            match Self::create_in(&root, &name) {
                Ok(ws) => return Ok(ws),
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap_or_else(|| {
            internal(format!(
                "Failed to create a mission workspace under {}",
                root.display()
            ))
        }))
    }

    /// Create the directory `name` directly inside the already-canonical `root`.
    ///
    /// Split out from [`MissionWorkspace::create`] so the containment checks can be tested against a caller-chosen name — including names that no sanitizer would ever produce.
    fn create_in(root: &Path, name: &str) -> KernelResult<Self> {
        // The name must be a single ordinary component. `create` only ever produces such names; this rejects anything else outright rather than relying on the canonicalization below to notice after the fact.
        if !is_single_normal_component(name) {
            return Err(internal(format!(
                "Refusing to create a mission workspace for the unsafe name {name:?}"
            )));
        }
        let dir = root.join(name);

        // `create_dir`, not `create_dir_all`: it fails with `AlreadyExists` on anything already sitting at that path, including a symlink planted there to redirect the later `remove_dir_all`. `create_dir_all` would happily follow such a symlink and treat the redirect as success.
        std::fs::create_dir(&dir).map_err(|e| {
            internal(format!(
                "Failed to create mission workspace {}: {e}",
                dir.display()
            ))
        })?;

        // Belt and braces behind the component check: resolve what was actually created and confirm it is still under the transient root before any further write — and, more importantly, before the `Drop` impl is ever armed to delete it.
        let canonical = match std::fs::canonicalize(&dir) {
            Ok(c) => c,
            Err(e) => {
                let _ = std::fs::remove_dir(&dir);
                return Err(internal(format!(
                    "Failed to resolve mission workspace {}: {e}",
                    dir.display()
                )));
            }
        };
        if !canonical.starts_with(root) {
            let _ = std::fs::remove_dir(&dir);
            return Err(internal(format!(
                "Mission workspace {} resolves outside the transient root {}",
                canonical.display(),
                root.display()
            )));
        }

        // Only now is the directory ours to own — and to delete.
        let ws = Self {
            path: canonical,
            name: name.to_string(),
        };
        ensure_workspace(&ws.path)?;
        tracing::debug!(path = %ws.path.display(), "created transient mission workspace");
        Ok(ws)
    }

    /// Absolute path of the mission directory.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Directory name — the sanitized label plus its uid suffix.
    ///
    /// Doubles as the mission agent's display name, which is what keeps two concurrent missions of the same agent type from colliding on a name-unique registry.
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl Drop for MissionWorkspace {
    fn drop(&mut self) {
        // `self.path` was canonicalized and containment-checked at construction, so this delete cannot have been redirected by the label that named it.
        if let Err(e) = std::fs::remove_dir_all(&self.path) {
            // A failed cleanup is not worth failing a completed run over: the directory is unreferenced from here on and the next boot's sweep collects it.
            tracing::warn!(
                path = %self.path.display(),
                error = %e,
                "failed to remove transient mission workspace — it will be swept on the next boot",
            );
        }
    }
}

/// Remove every leftover mission workspace under `<home_dir>/transient`.
///
/// Returns the number of entries removed.
/// Call this once during boot: a mission workspace is only ever live for the duration of one in-process run, so anything present when the daemon starts is the residue of a run that did not get to drop its guard.
pub fn sweep_orphan_missions(home_dir: &Path) -> usize {
    let root = home_dir.join(TRANSIENT_DIR_NAME);
    let Ok(entries) = std::fs::read_dir(&root) else {
        // No transient root yet (the common case on a fresh install), or it is unreadable — either way there is nothing this function can usefully do.
        return 0;
    };
    let mut removed = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        // Never recurse through a symlink: unlink the link itself and leave whatever it pointed at alone.
        let is_symlink = std::fs::symlink_metadata(&path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false);
        let result = if is_symlink || path.is_file() {
            std::fs::remove_file(&path)
        } else {
            std::fs::remove_dir_all(&path)
        };
        match result {
            Ok(()) => removed += 1,
            Err(e) => tracing::warn!(
                path = %path.display(),
                error = %e,
                "failed to sweep orphaned mission workspace",
            ),
        }
    }
    if removed > 0 {
        tracing::info!(
            count = removed,
            root = %root.display(),
            "swept orphaned mission workspaces left by an interrupted run",
        );
    }
    removed
}

/// Create `<home_dir>/transient` and return its canonical path.
fn ensure_transient_root(home_dir: &Path) -> KernelResult<PathBuf> {
    let root = home_dir.join(TRANSIENT_DIR_NAME);
    std::fs::create_dir_all(&root).map_err(|e| {
        internal(format!(
            "Failed to create transient root {}: {e}",
            root.display()
        ))
    })?;
    std::fs::canonicalize(&root).map_err(|e| {
        internal(format!(
            "Failed to resolve transient root {}: {e}",
            root.display()
        ))
    })
}

/// Build a `<sanitized label>-<uid>` directory name from untrusted text.
///
/// The uid is also the fallback prefix, so a label that sanitizes away to nothing yields `<uid>-<uid>` rather than a bare `-<uid>` or, worse, a shared directory for every such label (the collision class of #6442).
fn mission_dir_name(label: &str) -> String {
    let uid: String = uuid::Uuid::new_v4()
        .simple()
        .to_string()
        .chars()
        .take(UID_LEN)
        .collect();
    let mut prefix = safe_path_component(label, &uid);
    // `safe_path_component` emits ASCII only, so a byte truncation is also a char truncation.
    prefix.truncate(MAX_LABEL_LEN);
    format!("{prefix}-{uid}")
}

/// Whether `name` is exactly one ordinary path component — no separators, no `..`, no root, no Windows drive prefix.
fn is_single_normal_component(name: &str) -> bool {
    let path = Path::new(name);
    let mut components = path.components();
    let first = components.next();
    components.next().is_none() && matches!(first, Some(std::path::Component::Normal(_)))
}

fn internal(message: String) -> KernelError {
    KernelError::LibreFang(LibreFangError::Internal(message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Canonical transient root for a fresh temp home, created if absent.
    fn root_of(home: &Path) -> PathBuf {
        ensure_transient_root(home).expect("transient root")
    }

    #[test]
    fn create_places_the_mission_under_the_transient_root() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ws = MissionWorkspace::create(tmp.path(), "researcher").expect("create");

        assert!(ws.path().is_dir(), "the mission directory must exist");
        assert_eq!(
            ws.path().parent(),
            Some(root_of(tmp.path()).as_path()),
            "the mission directory must sit directly under the transient root",
        );
        assert!(
            ws.name().starts_with("researcher-"),
            "the directory name must keep the label as its prefix: {}",
            ws.name(),
        );
        assert!(
            ws.path().join(".identity").is_dir(),
            "a mission workspace must carry the standard agent layout",
        );
    }

    #[test]
    fn the_uid_suffix_is_eight_hex_characters() {
        let name = mission_dir_name("researcher");
        let uid = name.rsplit('-').next().expect("suffix");
        assert_eq!(uid.len(), UID_LEN, "unexpected uid length in {name}");
        assert!(
            uid.chars().all(|c| c.is_ascii_hexdigit()),
            "the uid suffix must be hex: {name}",
        );
    }

    #[test]
    fn a_label_that_sanitizes_away_still_yields_a_unique_name() {
        // `#6442` in reverse: two labels that both reduce to nothing must not collapse onto one directory.
        let tmp = tempfile::tempdir().expect("tempdir");
        let a = MissionWorkspace::create(tmp.path(), "///").expect("create a");
        let b = MissionWorkspace::create(tmp.path(), "...").expect("create b");

        assert_ne!(
            a.path(),
            b.path(),
            "empty labels must not share a directory"
        );
        assert!(
            !a.name().starts_with('-'),
            "an empty label must fall back to the uid, not to an empty prefix: {}",
            a.name(),
        );
    }

    /// #7723 — the security property this module exists for.
    ///
    /// A template `name` field is caller-controlled text that reaches the filesystem, and the directory it names is later fed to `remove_dir_all`, so a traversal here is an out-of-tree recursive delete rather than a stray `mkdir`.
    #[test]
    fn hostile_labels_cannot_escape_the_transient_root() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let home = tmp.path().join("home");
        std::fs::create_dir_all(&home).expect("home");
        // A sibling of the home directory that must survive every attempt below untouched.
        let bystander = tmp.path().join("bystander");
        std::fs::create_dir_all(&bystander).expect("bystander");
        std::fs::write(bystander.join("keep.txt"), b"do not delete").expect("bystander file");

        let root = root_of(&home);
        for label in [
            "../../bystander",
            "..",
            "../bystander",
            "/etc/shadow",
            "foo/../../bystander",
            "C:\\Windows\\System32",
            "\\\\?\\C:\\Windows",
            ".",
        ] {
            let ws = MissionWorkspace::create(&home, label)
                .unwrap_or_else(|e| panic!("create for {label:?}: {e}"));
            assert_eq!(
                ws.path().parent(),
                Some(root.as_path()),
                "label {label:?} escaped the transient root: {}",
                ws.path().display(),
            );
            drop(ws);
        }

        assert!(
            bystander.join("keep.txt").is_file(),
            "no hostile label may reach a path outside the transient root",
        );
    }

    #[test]
    fn a_name_with_separators_is_refused_outright() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = root_of(tmp.path());
        for name in ["../evil", "a/b", "..", "/etc", ".", ""] {
            assert!(
                MissionWorkspace::create_in(&root, name).is_err(),
                "name {name:?} must be refused before it reaches the filesystem",
            );
        }
    }

    /// A pre-existing entry at the mission path — most dangerously a symlink planted to redirect the eventual `remove_dir_all` — must be refused rather than adopted.
    #[test]
    #[cfg(unix)]
    fn a_planted_symlink_at_the_mission_path_is_refused() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let home = tmp.path().join("home");
        std::fs::create_dir_all(&home).expect("home");
        let victim = tmp.path().join("victim");
        std::fs::create_dir_all(&victim).expect("victim");
        std::fs::write(victim.join("keep.txt"), b"do not delete").expect("victim file");

        let root = root_of(&home);
        std::os::unix::fs::symlink(&victim, root.join("planted")).expect("symlink");

        let err = MissionWorkspace::create_in(&root, "planted")
            .expect_err("an occupied mission path must not be adopted");
        assert!(
            format!("{err}").contains("planted"),
            "the error should name the refused path: {err}",
        );
        assert!(
            victim.join("keep.txt").is_file(),
            "the symlink target must be untouched",
        );
    }

    #[test]
    fn concurrent_missions_of_one_type_get_distinct_directories() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let home = tmp.path().to_path_buf();
        // Force the root to exist first so the threads race on the mission dirs, not on the root.
        let _ = root_of(&home);

        let handles: Vec<_> = (0..16)
            .map(|_| {
                let home = home.clone();
                std::thread::spawn(move || {
                    MissionWorkspace::create(&home, "researcher").expect("create")
                })
            })
            .collect();
        let missions: Vec<MissionWorkspace> = handles
            .into_iter()
            .map(|h| h.join().expect("thread"))
            .collect();

        let paths: BTreeSet<PathBuf> = missions.iter().map(|m| m.path().to_path_buf()).collect();
        assert_eq!(
            paths.len(),
            missions.len(),
            "every concurrent mission must own its own directory",
        );
        for mission in &missions {
            assert!(
                mission.path().is_dir(),
                "all mission directories coexist while their guards are alive",
            );
        }
    }

    #[test]
    fn dropping_the_guard_removes_the_directory_and_its_contents() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ws = MissionWorkspace::create(tmp.path(), "writer").expect("create");
        let path = ws.path().to_path_buf();
        std::fs::write(path.join("output").join("draft.md"), b"intermediate").expect("write");

        assert!(path.is_dir());
        drop(ws);
        assert!(
            !path.exists(),
            "the mission directory must be gone once the run ends: {}",
            path.display(),
        );
        assert!(
            root_of(tmp.path()).is_dir(),
            "cleanup removes the mission, not the transient root",
        );
    }

    #[test]
    fn the_guard_still_cleans_up_when_the_run_unwinds() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = std::panic::catch_unwind({
            let home = tmp.path().to_path_buf();
            move || {
                let ws = MissionWorkspace::create(&home, "failing").expect("create");
                let path = ws.path().to_path_buf();
                std::fs::write(path.join("data").join("partial.json"), b"{}").expect("write");
                std::panic::panic_any(path);
            }
        })
        .expect_err("the closure panics");
        let path = *path.downcast::<PathBuf>().expect("panic payload");

        assert!(
            !path.exists(),
            "a failing run must not leave its mission directory behind: {}",
            path.display(),
        );
    }

    #[test]
    fn sweep_removes_the_residue_of_an_interrupted_run() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = root_of(tmp.path());
        // Stand in for two missions whose process died before their guards ran.
        for name in ["researcher-a1b2c3d4", "writer-99887766"] {
            let dir = root.join(name);
            std::fs::create_dir_all(dir.join("output")).expect("orphan");
            std::fs::write(dir.join("output").join("draft.md"), b"leftover").expect("orphan file");
        }

        assert_eq!(sweep_orphan_missions(tmp.path()), 2);
        assert_eq!(
            std::fs::read_dir(&root).expect("read root").count(),
            0,
            "the transient root must be empty after a sweep",
        );
        assert!(root.is_dir(), "the sweep keeps the root itself");
    }

    #[test]
    fn sweep_is_a_noop_when_no_mission_ever_ran() {
        let tmp = tempfile::tempdir().expect("tempdir");
        assert_eq!(sweep_orphan_missions(tmp.path()), 0);
    }

    #[test]
    #[cfg(unix)]
    fn sweep_unlinks_a_symlink_without_following_it() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let home = tmp.path().join("home");
        std::fs::create_dir_all(&home).expect("home");
        let victim = tmp.path().join("victim");
        std::fs::create_dir_all(&victim).expect("victim");
        std::fs::write(victim.join("keep.txt"), b"do not delete").expect("victim file");

        let root = root_of(&home);
        std::os::unix::fs::symlink(&victim, root.join("planted")).expect("symlink");

        assert_eq!(sweep_orphan_missions(&home), 1);
        assert!(
            victim.join("keep.txt").is_file(),
            "the sweep must unlink the symlink, never recurse through it",
        );
    }
}
