//! Checkpoint Manager — Transparent filesystem snapshots via shadow git repos.
//!
//! Creates automatic snapshots of working directories before file-mutating
//! operations (`file_write`, `apply_patch`), providing rollback to any previous
//! checkpoint.
//!
//! # Architecture
//!
//! ```text
//! ~/.librefang/checkpoints/{sha256(abs_dir)[:16]}/   — shadow git repo
//!     HEAD, refs/, objects/                            — standard git internals
//!     LIBREFANG_WORKDIR                               — original dir path
//!     info/exclude                                    — default excludes
//! ```
//!
//! The shadow repo uses `GIT_DIR` + `GIT_WORK_TREE` so no git state leaks
//! into the user's project directory.  All git operations use isolated
//! config (no user `~/.gitconfig`, no GPG signing).

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tracing::{debug, warn};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum number of files to snapshot.  Directories exceeding this are
/// skipped to prevent slowdowns on large repositories.
pub const MAX_FILES: usize = 50_000;

/// Sub-directory under `~/.librefang/` where shadow repos are stored.
pub const CHECKPOINT_BASE: &str = "checkpoints";

/// Git subprocess timeout in seconds.
const GIT_TIMEOUT_SECS: u64 = 30;

/// Default exclude patterns written into each shadow repo's `info/exclude`.
const DEFAULT_EXCLUDES: &[&str] = &[
    "node_modules/",
    "dist/",
    "build/",
    ".env",
    ".env.*",
    ".env.local",
    ".env.*.local",
    "__pycache__/",
    "*.pyc",
    "*.pyo",
    ".DS_Store",
    "*.log",
    ".cache/",
    ".next/",
    ".nuxt/",
    "coverage/",
    ".pytest_cache/",
    ".venv/",
    "venv/",
    "target/",
    ".git/",
];

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum CheckpointError {
    #[error("Shadow repo initialisation failed: {0}")]
    InitFailed(String),

    #[error("Working directory not found or not a directory: {0}")]
    BadWorkDir(PathBuf),

    #[error("Directory is too large to snapshot (>{MAX_FILES} files)")]
    TooManyFiles,

    #[error("git executable not found")]
    GitNotFound,

    #[error("git command failed: {0}")]
    GitFailed(String),

    #[error("Invalid commit hash: {0}")]
    InvalidHash(String),

    #[error("No snapshots found for this directory")]
    NoSnapshots,

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

// ---------------------------------------------------------------------------
// Public data types
// ---------------------------------------------------------------------------

/// Metadata for a single snapshot commit.
#[derive(Debug, Clone)]
pub struct SnapshotEntry {
    /// Full 40-character SHA-1 commit hash.
    pub hash: String,
    /// Short 7-character hash for display.
    pub short_hash: String,
    /// Commit subject (the reason string passed to `snapshot()`).
    pub message: String,
    /// Approximate creation time (parsed from the git author timestamp).
    pub timestamp: SystemTime,
}

// ---------------------------------------------------------------------------
// CheckpointManager
// ---------------------------------------------------------------------------

/// Manages automatic filesystem checkpoints backed by shadow git repositories.
///
/// Designed to be held as an `Arc<CheckpointManager>` and shared across the
/// agent runtime.  All public methods are `&self` (interior mutability is not
/// required).
///
/// # Usage
///
/// ```ignore
/// let mgr = CheckpointManager::new(home_dir.join("checkpoints"));
/// // Before mutating a file:
/// if let Err(e) = mgr.snapshot(workspace_root, "pre file_write") {
///     warn!("checkpoint failed (non-fatal): {e}");
/// }
/// ```
#[derive(Debug)]
pub struct CheckpointManager {
    /// `~/.librefang/checkpoints/` — base directory for all shadow repos.
    base_dir: PathBuf,
}

impl CheckpointManager {
    /// Create a `CheckpointManager` rooted at `base_dir`.
    ///
    /// `base_dir` is typically `~/.librefang/checkpoints/`.  The directory
    /// is created on first use.
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    // -----------------------------------------------------------------------
    // Public API
    // -----------------------------------------------------------------------

    /// Create a snapshot of the given working directory.
    ///
    /// Returns the full commit hash on success.  If there are no changes since
    /// the last snapshot the call succeeds and returns the most recent commit
    /// hash (or a sentinel `"no-op"` string when the repo is brand-new and
    /// empty).
    ///
    /// Fails with [`CheckpointError::TooManyFiles`] when the directory
    /// contains more than [`MAX_FILES`] files, to avoid freezing the daemon
    /// on huge work trees.
    pub fn snapshot(&self, work_dir: &Path, reason: &str) -> Result<String, CheckpointError> {
        let work_dir = normalize_path(work_dir)?;
        let shadow = self.shadow_repo_dir(&work_dir);

        self.init_shadow_repo_if_needed(&shadow, &work_dir)?;

        // Size guard
        if count_files_up_to(&work_dir, MAX_FILES + 1) > MAX_FILES {
            return Err(CheckpointError::TooManyFiles);
        }

        // Stage everything
        let (ok, _, stderr) = run_git(
            &["add", "-A"],
            &shadow,
            &work_dir,
            GIT_TIMEOUT_SECS * 2,
            &[],
        );
        if !ok {
            return Err(CheckpointError::GitFailed(format!("git add -A: {stderr}")));
        }

        // Check if there is anything new to commit
        let (no_changes, _, _) = run_git(
            &["diff", "--cached", "--quiet"],
            &shadow,
            &work_dir,
            GIT_TIMEOUT_SECS,
            &[1], // exit 1 = there ARE staged changes
        );
        if no_changes {
            // Nothing to commit — return the current HEAD hash if available.
            let (ok_head, head_hash, _) = run_git(
                &["rev-parse", "HEAD"],
                &shadow,
                &work_dir,
                GIT_TIMEOUT_SECS,
                &[],
            );
            if ok_head && !head_hash.is_empty() {
                return Ok(head_hash);
            }
            return Ok("no-op".to_string());
        }

        // Commit
        let (ok, _, stderr) = run_git(
            &[
                "commit",
                "-m",
                reason,
                "--allow-empty-message",
                "--no-gpg-sign",
            ],
            &shadow,
            &work_dir,
            GIT_TIMEOUT_SECS * 2,
            &[],
        );
        if !ok {
            return Err(CheckpointError::GitFailed(format!("git commit: {stderr}")));
        }

        // Return the new commit hash
        let (ok_hash, hash, _) = run_git(
            &["rev-parse", "HEAD"],
            &shadow,
            &work_dir,
            GIT_TIMEOUT_SECS,
            &[],
        );
        if ok_hash && !hash.is_empty() {
            debug!(hash, reason, "checkpoint taken for {}", work_dir.display());
            Ok(hash)
        } else {
            Ok("unknown".to_string())
        }
    }

    /// Restore working directory to a previous snapshot.
    ///
    /// Uses `git checkout <commit_hash> -- .` which restores tracked files
    /// without moving `HEAD` — safe and reversible (a pre-rollback snapshot
    /// is taken automatically).
    pub fn restore(&self, work_dir: &Path, commit_hash: &str) -> Result<(), CheckpointError> {
        Self::validate_commit_hash(commit_hash)?;
        let work_dir = normalize_path(work_dir)?;
        let shadow = self.shadow_repo_dir(&work_dir);

        if !shadow.join("HEAD").exists() {
            return Err(CheckpointError::NoSnapshots);
        }

        // Verify the commit exists
        let (ok, _, _) = run_git(
            &["cat-file", "-t", commit_hash],
            &shadow,
            &work_dir,
            GIT_TIMEOUT_SECS,
            &[],
        );
        if !ok {
            return Err(CheckpointError::InvalidHash(format!(
                "commit '{commit_hash}' not found in shadow repo"
            )));
        }

        // Take a pre-rollback snapshot so the user can undo the undo.
        let pre_reason = format!("pre-rollback snapshot (restoring to {})", &commit_hash[..8]);
        if let Err(e) = self.snapshot(&work_dir, &pre_reason) {
            // Non-fatal: warn but continue with the restore.
            warn!("pre-rollback snapshot failed (continuing): {e}");
        }

        let (ok, _, stderr) = run_git(
            &["checkout", commit_hash, "--", "."],
            &shadow,
            &work_dir,
            GIT_TIMEOUT_SECS * 2,
            &[],
        );
        if !ok {
            return Err(CheckpointError::GitFailed(format!(
                "git checkout {commit_hash}: {stderr}"
            )));
        }

        Ok(())
    }

    /// List snapshots for a working directory, newest first.
    ///
    /// Returns an empty `Vec` (not an error) when no shadow repo exists yet.
    pub fn list_snapshots(&self, work_dir: &Path) -> Result<Vec<SnapshotEntry>, CheckpointError> {
        let work_dir = normalize_path(work_dir)?;
        let shadow = self.shadow_repo_dir(&work_dir);

        if !shadow.join("HEAD").exists() {
            return Ok(vec![]);
        }

        // Format: <full-hash>|<short-hash>|<unix-timestamp>|<subject>
        let (ok, stdout, _) = run_git(
            &["log", "--format=%H|%h|%at|%s", "-n", "50"],
            &shadow,
            &work_dir,
            GIT_TIMEOUT_SECS,
            &[],
        );
        if !ok || stdout.is_empty() {
            return Ok(vec![]);
        }

        let entries = stdout
            .lines()
            .filter_map(|line| {
                let mut parts = line.splitn(4, '|');
                let hash = parts.next()?.to_string();
                let short_hash = parts.next()?.to_string();
                let unix_str = parts.next()?;
                let message = parts.next().unwrap_or("").to_string();

                let secs: u64 = unix_str.parse().ok()?;
                let timestamp = UNIX_EPOCH + std::time::Duration::from_secs(secs);

                Some(SnapshotEntry {
                    hash,
                    short_hash,
                    message,
                    timestamp,
                })
            })
            .collect();

        Ok(entries)
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Return the shadow repo path for a working directory.
    ///
    /// The name is the first 16 hex characters of `SHA-256(abs_path_utf8)`,
    /// giving a deterministic, collision-resistant, injection-safe identifier.
    fn shadow_repo_dir(&self, work_dir: &Path) -> PathBuf {
        let path_str = work_dir.to_string_lossy();
        let mut hasher = Sha256::new();
        hasher.update(path_str.as_bytes());
        let digest = hasher.finalize();
        // Format as lowercase hex and take the first 16 chars (8 bytes).
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        self.base_dir.join(&hex[..16])
    }

    /// Initialise the shadow repo at `shadow` if it does not already exist.
    fn init_shadow_repo_if_needed(
        &self,
        shadow: &Path,
        work_dir: &Path,
    ) -> Result<(), CheckpointError> {
        if shadow.join("HEAD").exists() {
            return Ok(());
        }

        std::fs::create_dir_all(shadow)?;

        // `git init` inside the shadow dir (bare-style via GIT_DIR env var)
        let (ok, _, err) = run_git(&["init"], shadow, work_dir, GIT_TIMEOUT_SECS, &[]);
        if !ok {
            return Err(CheckpointError::InitFailed(err));
        }

        // Per-repo config: identity + disable signing
        for args in &[
            vec!["config", "user.email", "librefang@local"],
            vec!["config", "user.name", "LibreFang Checkpoint"],
            vec!["config", "commit.gpgsign", "false"],
            vec!["config", "tag.gpgSign", "false"],
        ] {
            run_git(args, shadow, work_dir, GIT_TIMEOUT_SECS, &[]);
        }

        // Write default excludes
        let info_dir = shadow.join("info");
        std::fs::create_dir_all(&info_dir)?;
        std::fs::write(info_dir.join("exclude"), DEFAULT_EXCLUDES.join("\n") + "\n")?;

        // Record the canonical working directory path for introspection
        std::fs::write(
            shadow.join("LIBREFANG_WORKDIR"),
            work_dir.to_string_lossy().as_bytes(),
        )?;

        debug!(
            shadow = %shadow.display(),
            workdir = %work_dir.display(),
            "initialised checkpoint repo"
        );
        Ok(())
    }

    /// Validate a commit hash to prevent git argument injection.
    ///
    /// Accepts 4–64 lowercase or uppercase hex characters.  Values starting
    /// with `-` would be misinterpreted as git flags.
    fn validate_commit_hash(hash: &str) -> Result<(), CheckpointError> {
        if hash.is_empty() {
            return Err(CheckpointError::InvalidHash("empty hash".to_string()));
        }
        if hash.starts_with('-') {
            return Err(CheckpointError::InvalidHash(format!(
                "hash must not start with '-': {hash:?}"
            )));
        }
        let len = hash.len();
        if !(4..=64).contains(&len) {
            return Err(CheckpointError::InvalidHash(format!(
                "hash length {len} not in 4–64 range"
            )));
        }
        if !hash.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(CheckpointError::InvalidHash(format!(
                "hash contains non-hex characters: {hash:?}"
            )));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Git subprocess helpers
// ---------------------------------------------------------------------------

/// Build the environment map for a git command targeting the shadow repo.
///
/// - Sets `GIT_DIR` to the shadow repo path.
/// - Sets `GIT_WORK_TREE` to the real working directory.
/// - Isolates from the user's global/system git config to prevent
///   `commit.gpgsign`, credential helpers, etc. from interfering.
fn git_env(shadow: &Path, work_dir: &Path) -> Vec<(String, String)> {
    // Start from the current process environment and override the
    // git-relevant variables.
    let mut env: Vec<(String, String)> = std::env::vars()
        .filter(|(k, _)| {
            !matches!(
                k.as_str(),
                "GIT_DIR"
                    | "GIT_WORK_TREE"
                    | "GIT_INDEX_FILE"
                    | "GIT_NAMESPACE"
                    | "GIT_ALTERNATE_OBJECT_DIRECTORIES"
                    | "GIT_CONFIG_GLOBAL"
                    | "GIT_CONFIG_SYSTEM"
                    | "GIT_CONFIG_NOSYSTEM"
            )
        })
        .collect();

    env.push(("GIT_DIR".to_string(), shadow.to_string_lossy().into_owned()));
    env.push((
        "GIT_WORK_TREE".to_string(),
        work_dir.to_string_lossy().into_owned(),
    ));
    // Isolate from the user's global/system config (git ≥ 2.32 honours these).
    env.push(("GIT_CONFIG_GLOBAL".to_string(), "/dev/null".to_string()));
    env.push(("GIT_CONFIG_SYSTEM".to_string(), "/dev/null".to_string()));
    env.push(("GIT_CONFIG_NOSYSTEM".to_string(), "1".to_string()));

    env
}

/// Run a git command against the shadow repo.
///
/// Returns `(ok, stdout, stderr)`.  `allowed_non_zero` lists exit codes that
/// are expected (e.g. `1` from `git diff --quiet` when there are changes) and
/// should not be logged as errors.
fn run_git(
    args: &[&str],
    shadow: &Path,
    work_dir: &Path,
    timeout_secs: u64,
    allowed_non_zero: &[i32],
) -> (bool, String, String) {
    if !work_dir.exists() || !work_dir.is_dir() {
        warn!(
            path = %work_dir.display(),
            "git command skipped: working directory not found"
        );
        return (
            false,
            String::new(),
            "working directory not found".to_string(),
        );
    }

    let env = git_env(shadow, work_dir);

    // Build owned copies of the path strings so we can move them into a
    // thread without lifetime issues.
    let shadow_str = shadow.to_string_lossy().into_owned();
    let work_dir_str = work_dir.to_string_lossy().into_owned();
    let args_owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let env_owned: Vec<(String, String)> = env;
    let allowed_owned: Vec<i32> = allowed_non_zero.to_vec();

    // Spawn a thread so we can join with a timeout — std::process::Command
    // has no native timeout support.
    use std::sync::mpsc;
    let (tx, rx) = mpsc::channel::<Result<std::process::Output, std::io::Error>>();

    std::thread::spawn(move || {
        let mut cmd = Command::new("git");
        cmd.args(&args_owned)
            .current_dir(&work_dir_str)
            .env_clear()
            .envs(env_owned.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        let _ = tx.send(cmd.output());
    });

    match rx.recv_timeout(std::time::Duration::from_secs(timeout_secs)) {
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            warn!("git executable not found (shadow={})", shadow_str);
            (false, String::new(), "git not found".to_string())
        }
        Ok(Err(e)) => {
            let msg = e.to_string();
            warn!(error = %msg, "git command spawn failed");
            (false, String::new(), msg)
        }
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let code = output.status.code().unwrap_or(-1);
            let ok = output.status.success();

            if !ok && !allowed_owned.contains(&code) {
                warn!(
                    args = ?args,
                    exit_code = code,
                    stderr = %stderr,
                    "git command failed"
                );
            }
            (ok, stdout, stderr)
        }
        Err(_) => {
            warn!(args = ?args, timeout_secs, "git command timed out");
            (
                false,
                String::new(),
                format!("timed out after {timeout_secs}s"),
            )
        }
    }
}

// ---------------------------------------------------------------------------
// File-count helper
// ---------------------------------------------------------------------------

/// Count files under `dir`, stopping early once `limit` is exceeded.
///
/// Ignores permission errors and skips `.git/` directories.
fn count_files_up_to(dir: &Path, limit: usize) -> usize {
    let mut count = 0usize;
    count_recursive(dir, limit, &mut count);
    count
}

fn count_recursive(dir: &Path, limit: usize, count: &mut usize) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        if *count >= limit {
            return;
        }
        let path = entry.path();
        if path.is_dir() {
            // Skip well-known large / irrelevant directories.
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(
                name,
                ".git" | "node_modules" | "target" | "venv" | ".venv" | "__pycache__"
            ) {
                continue;
            }
            count_recursive(&path, limit, count);
        } else {
            *count += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Path normalisation
// ---------------------------------------------------------------------------

fn normalize_path(path: &Path) -> Result<PathBuf, CheckpointError> {
    let canonical = path
        .canonicalize()
        .map_err(|_| CheckpointError::BadWorkDir(path.to_path_buf()))?;
    if !canonical.is_dir() {
        return Err(CheckpointError::BadWorkDir(canonical));
    }
    Ok(canonical)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_commit_hashes_are_accepted() {
        assert!(CheckpointManager::validate_commit_hash("abcdef01").is_ok());
        assert!(CheckpointManager::validate_commit_hash("ABCDEF01").is_ok());
        assert!(CheckpointManager::validate_commit_hash(
            "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2"
        )
        .is_ok());
        assert!(CheckpointManager::validate_commit_hash("deadbeef1234567890abcdef").is_ok());
    }

    #[test]
    fn invalid_commit_hashes_are_rejected() {
        // Too short
        assert!(CheckpointManager::validate_commit_hash("abc").is_err());
        // Starts with dash (injection)
        assert!(CheckpointManager::validate_commit_hash("--patch").is_err());
        assert!(CheckpointManager::validate_commit_hash("-p").is_err());
        // Non-hex characters
        assert!(CheckpointManager::validate_commit_hash("xyz12345").is_err());
        // Empty
        assert!(CheckpointManager::validate_commit_hash("").is_err());
        // Too long (65 chars)
        assert!(CheckpointManager::validate_commit_hash(
            "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2a1b2c3d4e5f6a1b2c3d4e5f6f"
        )
        .is_err());
    }

    #[test]
    fn shadow_repo_dir_is_deterministic() {
        let mgr = CheckpointManager::new(PathBuf::from("/tmp/cp_test_base"));
        let a = mgr.shadow_repo_dir(Path::new("/home/user/project"));
        let b = mgr.shadow_repo_dir(Path::new("/home/user/project"));
        assert_eq!(a, b);
    }

    #[test]
    fn shadow_repo_dir_differs_for_different_paths() {
        let mgr = CheckpointManager::new(PathBuf::from("/tmp/cp_test_base"));
        let a = mgr.shadow_repo_dir(Path::new("/home/user/project_a"));
        let b = mgr.shadow_repo_dir(Path::new("/home/user/project_b"));
        assert_ne!(a, b);
    }

    #[test]
    fn shadow_repo_dir_name_is_16_hex_chars() {
        let mgr = CheckpointManager::new(PathBuf::from("/tmp/cp_test_base"));
        let dir = mgr.shadow_repo_dir(Path::new("/some/path"));
        let name = dir.file_name().unwrap().to_string_lossy();
        assert_eq!(name.len(), 16);
        assert!(name.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn count_files_up_to_returns_zero_for_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(count_files_up_to(dir.path(), MAX_FILES + 1), 0);
    }

    #[test]
    fn count_files_up_to_counts_correctly() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..5 {
            std::fs::write(dir.path().join(format!("f{i}.txt")), "x").unwrap();
        }
        assert_eq!(count_files_up_to(dir.path(), MAX_FILES + 1), 5);
    }
}
