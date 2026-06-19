//! tmux CLI controller for LibreFang terminal session management.
//!
//! Manages named tmux sessions and windows on behalf of the API layer.
//! Every tmux invocation is isolated via `-L <socket_name> -f /dev/null` so it
//! never touches the user's global tmux server or config.
//!
//! # Security invariants
//! - All arguments are passed as separate `Command::arg(…)` calls — no shell
//!   expansion, no string concatenation, no `sh -c`.
//! - Window IDs and names are validated by regex before being forwarded to the
//!   subprocess.
//! - The socket is namespaced (`-L librefang`) so LibreFang never interferes
//!   with the user's own tmux sessions.

use std::path::{Path, PathBuf};
use std::time::Duration;

use thiserror::Error;
use tokio::process::Command;

// ── error type ─────────────────────────────────────────────────────────────────

/// Structured error returned by [`TmuxController`] methods and [`parse_window_list`].
///
/// Replaces the historical `anyhow::Error` shape so callers can branch on the *kind* of failure (spawn vs. timeout vs. non-zero exit vs. malformed output) rather than substring-matching the rendered string.
/// The `Display` text is preserved byte-for-byte from the previous `anyhow!(…)` call sites because it reaches daemon logs (`warn!(error = %e, …)`) — see `routes/terminal.rs`.
///
/// Refs: #3576.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum TmuxError {
    /// `Command::spawn()` failed before the process started.
    /// `cmd` names which tmux invocation could not be launched (e.g. `"tmux"`, `"tmux has-session"`, `"tmux new-session"`); the `source` is the underlying spawn `io::Error`.
    #[error("failed to spawn {cmd}: {source}")]
    Spawn {
        cmd: &'static str,
        #[source]
        source: std::io::Error,
    },

    /// The subprocess did not finish within the hard timeout.
    /// `what` names the operation that stalled (e.g. `"tmux command"`, `"tmux has-session"`, `"tmux new-session"`).
    #[error("{what} timed out")]
    Timeout { what: &'static str },

    /// Waiting on the subprocess (collecting its output) failed with an I/O error after it was spawned.
    #[error("tmux I/O error: {source}")]
    Io {
        #[source]
        source: std::io::Error,
    },

    /// tmux ran but exited non-zero.
    /// `status` is the rendered exit status and `stderr` is the trimmed standard-error output.
    #[error("tmux exited with {status}: {stderr}")]
    NonZeroExit { status: String, stderr: String },

    /// A window id or name failed validation before any subprocess was spawned.
    /// The message preserves the regex-pattern detail the previous `anyhow!` call sites surfaced.
    #[error("{0}")]
    InvalidArgument(String),

    /// `tmux new-window -P` succeeded but produced no parseable window line.
    #[error("tmux new-window returned no output")]
    NoOutput,

    /// A line of `list-windows -F` output was missing an expected field.
    /// `field` is the absent column (`"window_id"`, `"window_index"`, `"window_name"`, `"window_active"`); `line` is the offending raw line.
    #[error("missing {field} in tmux output: {line:?}")]
    MissingField { field: &'static str, line: String },

    /// The `window_index` column could not be parsed as a `u32`.
    /// `value` is the raw token that failed to parse.
    #[error("invalid window index {value:?}")]
    InvalidIndex { value: String },
}

// ── constants ────────────────────────────────────────────────────────────────

/// Name passed to tmux `-L` (socket namespace).
const SOCKET_NAME: &str = "librefang";

/// Hard timeout for any tmux subprocess call.
const TMUX_TIMEOUT: Duration = Duration::from_secs(5);

/// Availability probe timeout (shorter — just `-V`).
const TMUX_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Default tmux session managed by the API terminal.
pub const DEFAULT_TMUX_SESSION_NAME: &str = "main";

// ── public types ─────────────────────────────────────────────────────────────

/// Information about a single tmux window.
#[derive(Debug, Clone, serde::Serialize)]
pub struct WindowInfo {
    /// tmux window ID, e.g. `"@1"`.
    pub id: String,
    /// Sequential window index within the session.
    pub index: u32,
    /// Human-readable window name.
    pub name: String,
    /// Whether this is the currently active window.
    pub active: bool,
}

/// Controller for a single named tmux session.
///
/// All tmux invocations use `-L librefang -f /dev/null` so the socket and
/// config are fully isolated from the user's environment.
pub struct TmuxController {
    socket_name: &'static str,
    session_name: String,
    tmux_path: PathBuf,
}

impl TmuxController {
    // ── construction ─────────────────────────────────────────────────────────

    /// Create a new controller.
    ///
    /// `tmux_path` should be the absolute path to the tmux binary (resolved
    /// once at startup via `which` or a config override). `session_name` is
    /// the tmux session that will be managed (e.g. `"main"` or `"user-42"`).
    pub fn new(tmux_path: PathBuf, session_name: String) -> Self {
        Self {
            socket_name: SOCKET_NAME,
            session_name,
            tmux_path,
        }
    }

    // ── helpers ──────────────────────────────────────────────────────────────

    /// Build a `Command` pre-loaded with the isolation flags every tmux call
    /// requires: `-L <socket>` and `-f /dev/null`.
    fn base_cmd(&self) -> Command {
        let mut cmd = Command::new(&self.tmux_path);
        cmd.kill_on_drop(true);
        cmd.arg("-L").arg(self.socket_name);
        cmd.arg("-f").arg("/dev/null");
        cmd
    }

    /// Run a command, collect its stdout, and return an error if it exits
    /// non-zero.
    async fn run(&self, mut cmd: Command) -> Result<String, TmuxError> {
        // Silence stderr so tmux error messages don't bleed into daemon logs.
        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let child = cmd.spawn().map_err(|e| TmuxError::Spawn {
            cmd: "tmux",
            source: e,
        })?;

        let result = tokio::time::timeout(TMUX_TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| TmuxError::Timeout {
                what: "tmux command",
            })?
            .map_err(|e| TmuxError::Io { source: e })?;

        if !result.status.success() {
            let stderr = String::from_utf8_lossy(&result.stderr);
            return Err(TmuxError::NonZeroExit {
                status: result.status.to_string(),
                stderr: stderr.trim().to_string(),
            });
        }

        Ok(String::from_utf8_lossy(&result.stdout).into_owned())
    }

    // ── public API ───────────────────────────────────────────────────────────

    /// Return `true` if the binary at `tmux_path` exists and responds to
    /// `tmux -V` within 2 seconds.
    pub async fn is_available(tmux_path: &Path) -> bool {
        let mut cmd = Command::new(tmux_path);
        cmd.kill_on_drop(true);
        cmd.arg("-V")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());

        let child = match cmd.spawn() {
            Ok(c) => c,
            Err(_) => return false,
        };

        match tokio::time::timeout(TMUX_PROBE_TIMEOUT, child.wait_with_output()).await {
            Ok(Ok(out)) => out.status.success(),
            _ => false,
        }
    }

    /// Ensure the named session exists, creating it detached if it does not.
    ///
    /// Idempotent — safe to call concurrently. The `has-session` → `new-session`
    /// sequence is inherently racy when multiple requests arrive simultaneously;
    /// a "duplicate session" error from `new-session` means another concurrent
    /// caller already created the session, which is a success condition.
    pub async fn ensure_session(&self) -> Result<(), TmuxError> {
        // Check whether the session already exists.
        let mut check = self.base_cmd();
        check.arg("has-session").arg("-t").arg(&self.session_name);

        let already_exists = {
            check
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());

            let child = check.spawn().map_err(|e| TmuxError::Spawn {
                cmd: "tmux has-session",
                source: e,
            })?;

            let out = tokio::time::timeout(TMUX_TIMEOUT, child.wait_with_output())
                .await
                .map_err(|_| TmuxError::Timeout {
                    what: "tmux has-session",
                })?
                .map_err(|e| TmuxError::Io { source: e })?;

            out.status.success()
        };

        if already_exists {
            return Ok(());
        }

        // Create a new detached session. If this races with another caller and
        // tmux reports "duplicate session", the session exists — treat as success.
        let mut create = self.base_cmd();
        create
            .arg("new-session")
            .arg("-d")
            .arg("-s")
            .arg(&self.session_name)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped());

        let child = create.spawn().map_err(|e| TmuxError::Spawn {
            cmd: "tmux new-session",
            source: e,
        })?;

        let out = tokio::time::timeout(TMUX_TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| TmuxError::Timeout {
                what: "tmux new-session",
            })?
            .map_err(|e| TmuxError::Io { source: e })?;

        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            // A concurrent ensure_session already created the session — not an error.
            if stderr.contains("duplicate session") {
                return Ok(());
            }
            return Err(TmuxError::NonZeroExit {
                status: out.status.to_string(),
                stderr: stderr.trim().to_string(),
            });
        }

        Ok(())
    }

    /// Return metadata for all windows in the session.
    pub async fn list_windows(&self) -> Result<Vec<WindowInfo>, TmuxError> {
        let mut cmd = self.base_cmd();
        cmd.arg("list-windows")
            .arg("-t")
            .arg(&self.session_name)
            .arg("-F")
            .arg("#{window_id}|#{window_index}|#{window_name}|#{window_active}");

        let output = self.run(cmd).await?;
        parse_window_list(&output)
    }

    /// Open a new window in the session.
    ///
    /// If `name` is provided it is validated before the subprocess is spawned;
    /// an invalid name is rejected immediately without touching tmux.
    pub async fn new_window(&self, name: Option<&str>) -> Result<WindowInfo, TmuxError> {
        if let Some(n) = name {
            if !validate_window_name(n) {
                return Err(TmuxError::InvalidArgument(format!(
                    "invalid window name {:?}: must match ^[A-Za-z0-9 ._-]{{1,64}}$",
                    n
                )));
            }
        }

        // Create the window and capture its ID via the print-format flag.
        let mut cmd = self.base_cmd();
        cmd.arg("new-window")
            .arg("-t")
            .arg(&self.session_name)
            .arg("-P") // print info about the new window
            .arg("-F")
            .arg("#{window_id}|#{window_index}|#{window_name}|#{window_active}");

        if let Some(n) = name {
            cmd.arg("-n").arg(n);
        }

        let output = self.run(cmd).await?;
        let mut windows = parse_window_list(output.trim())?;
        windows.pop().ok_or(TmuxError::NoOutput)
    }

    /// Switch the active window to the one identified by `id` (e.g. `"@1"`).
    ///
    /// The ID is validated before the subprocess is spawned.
    pub async fn select_window(&self, id: &str) -> Result<(), TmuxError> {
        if !validate_window_id(id) {
            return Err(TmuxError::InvalidArgument(format!(
                "invalid window id {:?}: must match ^@[0-9]{{1,9}}$",
                id
            )));
        }

        let mut cmd = self.base_cmd();
        cmd.arg("select-window")
            .arg("-t")
            .arg(format!("{}:{}", self.session_name, id));

        self.run(cmd).await?;
        Ok(())
    }

    /// Rename a window identified by `id` to `name`.
    ///
    /// Both the ID and the new name are validated before the subprocess is
    /// spawned.
    pub async fn rename_window(&self, id: &str, name: &str) -> Result<(), TmuxError> {
        if !validate_window_id(id) {
            return Err(TmuxError::InvalidArgument(format!(
                "invalid window id {:?}: must match ^@[0-9]{{1,9}}$",
                id
            )));
        }
        if !validate_window_name(name) {
            return Err(TmuxError::InvalidArgument(format!(
                "invalid window name {:?}: must match ^[A-Za-z0-9 ._-]{{1,64}}$",
                name
            )));
        }

        let mut cmd = self.base_cmd();
        cmd.arg("rename-window")
            .arg("-t")
            .arg(format!("{}:{}", self.session_name, id))
            .arg(name);
        self.run(cmd).await?;
        Ok(())
    }

    /// Kill a single window by ID (e.g. `"@1"`).
    ///
    /// The ID is validated before the subprocess is spawned.
    pub async fn kill_window(&self, id: &str) -> Result<(), TmuxError> {
        if !validate_window_id(id) {
            return Err(TmuxError::InvalidArgument(format!(
                "invalid window id {:?}: must match ^@[0-9]{{1,9}}$",
                id
            )));
        }

        let mut cmd = self.base_cmd();
        cmd.arg("kill-window")
            .arg("-t")
            .arg(format!("{}:{}", self.session_name, id));
        self.run(cmd).await?;
        Ok(())
    }

    /// Destroy the entire session. Intended for daemon shutdown / cleanup.
    pub async fn kill_session(&self) -> Result<(), TmuxError> {
        let mut cmd = self.base_cmd();
        cmd.arg("kill-session").arg("-t").arg(&self.session_name);
        self.run(cmd).await?;
        Ok(())
    }
}

// ── validation ───────────────────────────────────────────────────────────────

/// Return `true` if `id` is a valid tmux window ID in LibreFang's format.
///
/// Valid: `@` followed by 1–9 decimal digits, total length ≤ 12.
/// Examples: `@0`, `@1`, `@123456789` (9 digits).
/// Rejected: `@`, `@a`, `@1;ls`, `@1234567890` (10 digits), `../`.
pub fn validate_window_id(id: &str) -> bool {
    // Fast length guard: "@" + up to 9 digits = 10 chars max.
    if id.len() < 2 || id.len() > 10 {
        return false;
    }
    let bytes = id.as_bytes();
    if bytes[0] != b'@' {
        return false;
    }
    // All remaining characters must be ASCII digits (at least one).
    bytes[1..].iter().all(|b| b.is_ascii_digit())
}

/// Return `true` if `name` is a safe tmux window name.
/// Validate a tmux window name.
///
/// Allowed: any Unicode character except control characters and `|`.
/// Length is measured in Unicode scalar values (chars), 1–64.
///
/// The `|` character is forbidden because it is used as the field separator
/// in our `list-windows -F` format string; allowing it would corrupt parsing.
/// Control characters (including newlines and null bytes) are rejected because
/// they cannot appear in tmux window names meaningfully.
///
/// Since window names are passed via `Command::arg` (not a shell), there is no
/// shell-injection risk — we only need to guard against chars that break our
/// own tmux output parsing.
pub fn validate_window_name(name: &str) -> bool {
    let len = name.chars().count();
    if len == 0 || len > 64 {
        return false;
    }
    !name.chars().any(|c| c.is_control() || c == '|')
}

// ── internal parser ───────────────────────────────────────────────────────────

/// Parse the output of `list-windows -F '#{window_id}|#{window_index}|#{window_name}|#{window_active}'`.
///
/// Each non-empty line is split into exactly four fields using `splitn(4, '|')`.
/// Window names from tmux are returned as-is (upstream validators ensure names
/// were whitelisted when created through this controller).
fn parse_window_list(output: &str) -> Result<Vec<WindowInfo>, TmuxError> {
    let mut windows = Vec::new();

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let mut parts = line.splitn(4, '|');
        let id = parts.next().ok_or_else(|| TmuxError::MissingField {
            field: "window_id",
            line: line.to_string(),
        })?;
        let index_str = parts.next().ok_or_else(|| TmuxError::MissingField {
            field: "window_index",
            line: line.to_string(),
        })?;
        let name = parts.next().ok_or_else(|| TmuxError::MissingField {
            field: "window_name",
            line: line.to_string(),
        })?;
        let active_str = parts.next().ok_or_else(|| TmuxError::MissingField {
            field: "window_active",
            line: line.to_string(),
        })?;

        let index: u32 = index_str
            .trim()
            .parse()
            .map_err(|_| TmuxError::InvalidIndex {
                value: index_str.to_string(),
            })?;

        let active = active_str.trim() == "1";

        windows.push(WindowInfo {
            id: id.to_string(),
            index,
            name: name.to_string(),
            active,
        });
    }

    Ok(windows)
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── parser tests ──────────────────────────────────────────────────────────

    #[test]
    fn parse_single_active_window() {
        let raw = "@1|0|editor|1";
        let windows = parse_window_list(raw).unwrap();
        assert_eq!(windows.len(), 1);
        let w = &windows[0];
        assert_eq!(w.id, "@1");
        assert_eq!(w.index, 0);
        assert_eq!(w.name, "editor");
        assert!(w.active);
    }

    #[test]
    fn parse_multiple_windows() {
        let raw = "@1|0|editor|1\n@2|1|build|0\n@3|2|tests|0";
        let windows = parse_window_list(raw).unwrap();
        assert_eq!(windows.len(), 3);
        assert_eq!(windows[0].id, "@1");
        assert!(windows[0].active);
        assert_eq!(windows[1].id, "@2");
        assert!(!windows[1].active);
        assert_eq!(windows[2].name, "tests");
    }

    #[test]
    fn parse_window_with_pipe_in_name_uses_splitn() {
        // If a name somehow contains a pipe (which the validator would reject,
        // but tmux output could theoretically produce), splitn(4) captures the
        // rest as the name field.
        let raw = "@5|2|weird|name|0";
        let windows = parse_window_list(raw).unwrap();
        assert_eq!(windows.len(), 1);
        // The name field is "weird" and the rest "name|0" becomes `active_str`.
        // This is an edge-case acknowledgement — the validator prevents such names
        // from being created through this controller in the first place.
        assert_eq!(windows[0].id, "@5");
        assert_eq!(windows[0].index, 2);
    }

    #[test]
    fn parse_empty_output_returns_empty_vec() {
        let windows = parse_window_list("").unwrap();
        assert!(windows.is_empty());
    }

    #[test]
    fn parse_blank_lines_skipped() {
        let raw = "\n@1|0|editor|1\n\n";
        let windows = parse_window_list(raw).unwrap();
        assert_eq!(windows.len(), 1);
    }

    #[test]
    fn parse_inactive_window() {
        let raw = "@7|3|my-app|0";
        let windows = parse_window_list(raw).unwrap();
        assert!(!windows[0].active);
    }

    #[test]
    fn parse_malformed_line_returns_error() {
        // Missing the active field.
        let raw = "@1|0|editor";
        let err = parse_window_list(raw).unwrap_err();
        assert!(matches!(
            err,
            TmuxError::MissingField {
                field: "window_active",
                ..
            }
        ));
        // Display text preserved byte-for-byte from the previous anyhow! shape.
        assert_eq!(
            err.to_string(),
            "missing window_active in tmux output: \"@1|0|editor\""
        );
    }

    #[test]
    fn parse_bad_index_returns_error() {
        let raw = "@1|abc|editor|1";
        let err = parse_window_list(raw).unwrap_err();
        assert!(matches!(err, TmuxError::InvalidIndex { .. }));
        assert_eq!(err.to_string(), "invalid window index \"abc\"");
    }

    // ── validate_window_id ────────────────────────────────────────────────────

    #[test]
    fn valid_window_ids() {
        assert!(validate_window_id("@0"));
        assert!(validate_window_id("@1"));
        assert!(validate_window_id("@9"));
        assert!(validate_window_id("@42"));
        assert!(validate_window_id("@123456789")); // 9 digits — maximum
    }

    #[test]
    fn invalid_window_id_empty() {
        assert!(!validate_window_id(""));
    }

    #[test]
    fn invalid_window_id_at_only() {
        assert!(!validate_window_id("@"));
    }

    #[test]
    fn invalid_window_id_alpha() {
        assert!(!validate_window_id("@a"));
        assert!(!validate_window_id("@1a"));
    }

    #[test]
    fn invalid_window_id_injection() {
        assert!(!validate_window_id("@1;ls"));
        assert!(!validate_window_id("@1 2"));
    }

    #[test]
    fn invalid_window_id_path_traversal() {
        assert!(!validate_window_id("../"));
        assert!(!validate_window_id("@../"));
    }

    #[test]
    fn invalid_window_id_at_with_spaces() {
        assert!(!validate_window_id("@ 1"));
        assert!(!validate_window_id("@1 "));
    }

    #[test]
    fn invalid_window_id_ten_digits() {
        // 10 digits: "@1234567890" — exceeds the 9-digit maximum.
        assert!(!validate_window_id("@1234567890"));
    }

    #[test]
    fn invalid_window_id_no_at_prefix() {
        assert!(!validate_window_id("1"));
        assert!(!validate_window_id("123"));
    }

    // ── validate_window_name ──────────────────────────────────────────────────

    #[test]
    fn valid_window_names() {
        assert!(validate_window_name("editor"));
        assert!(validate_window_name("my-app_01"));
        assert!(validate_window_name("build 1"));
        assert!(validate_window_name("a")); // minimum length
                                            // 64-char string — maximum (measured in Unicode scalar values)
        let max_name = "a".repeat(64);
        assert!(validate_window_name(&max_name));
        // Unicode / CJK / emoji are allowed
        assert!(validate_window_name("终端"));
        assert!(validate_window_name("开发环境"));
        assert!(validate_window_name("hello🦊"));
        assert!(validate_window_name("café"));
        assert!(validate_window_name("日本語"));
        // 64 Chinese chars — right at the limit
        let max_cjk = "终".repeat(64);
        assert!(validate_window_name(&max_cjk));
    }

    #[test]
    fn invalid_window_name_empty() {
        assert!(!validate_window_name(""));
    }

    #[test]
    fn invalid_window_name_pipe_separator() {
        // '|' is our list-windows format separator — must be rejected.
        assert!(!validate_window_name("a|b"));
        assert!(!validate_window_name("|"));
    }

    #[test]
    fn invalid_window_name_too_long() {
        // 65 chars in Unicode scalar values
        let long = "a".repeat(65);
        assert!(!validate_window_name(&long));
        // 65 CJK chars (each is one scalar value but 3 UTF-8 bytes)
        let long_cjk = "终".repeat(65);
        assert!(!validate_window_name(&long_cjk));
    }

    #[test]
    fn invalid_window_name_control_chars() {
        assert!(!validate_window_name("foo\nbar"));
        assert!(!validate_window_name("foo\r\nbar"));
        assert!(!validate_window_name("foo\tbar"));
        assert!(!validate_window_name("foo\x00bar"));
    }

    #[test]
    fn parse_window_with_special_chars_in_name() {
        // Names with dots, dashes, underscores
        let raw = "@1|0|my-app_v2.1|1";
        let windows = parse_window_list(raw).unwrap();
        assert_eq!(windows[0].name, "my-app_v2.1");
    }

    // ── TmuxController construction ───────────────────────────────────────────

    #[test]
    fn controller_fields_set_correctly() {
        let ctrl = TmuxController::new(
            PathBuf::from("/usr/bin/tmux"),
            DEFAULT_TMUX_SESSION_NAME.to_string(),
        );
        assert_eq!(ctrl.socket_name, "librefang");
        assert_eq!(ctrl.session_name, DEFAULT_TMUX_SESSION_NAME);
        assert_eq!(ctrl.tmux_path, PathBuf::from("/usr/bin/tmux"));
    }
}
