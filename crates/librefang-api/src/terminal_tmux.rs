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

use tokio::process::Command;

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
    async fn run(&self, mut cmd: Command) -> anyhow::Result<String> {
        // Silence stderr so tmux error messages don't bleed into daemon logs.
        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let child = cmd
            .spawn()
            .map_err(|e| anyhow::anyhow!("failed to spawn tmux: {e}"))?;

        let result = tokio::time::timeout(TMUX_TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| anyhow::anyhow!("tmux command timed out"))?
            .map_err(|e| anyhow::anyhow!("tmux I/O error: {e}"))?;

        if !result.status.success() {
            let stderr = String::from_utf8_lossy(&result.stderr);
            return Err(anyhow::anyhow!(
                "tmux exited with {}: {}",
                result.status,
                stderr.trim()
            ));
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
    /// Idempotent — safe to call every time the API starts.
    pub async fn ensure_session(&self) -> anyhow::Result<()> {
        // Check whether the session already exists.
        let mut check = self.base_cmd();
        check.arg("has-session").arg("-t").arg(&self.session_name);

        let already_exists = {
            check
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());

            let child = check
                .spawn()
                .map_err(|e| anyhow::anyhow!("failed to spawn tmux has-session: {e}"))?;

            let out = tokio::time::timeout(TMUX_TIMEOUT, child.wait_with_output())
                .await
                .map_err(|_| anyhow::anyhow!("tmux has-session timed out"))?
                .map_err(|e| anyhow::anyhow!("tmux I/O error: {e}"))?;

            out.status.success()
        };

        if already_exists {
            return Ok(());
        }

        // Create a new detached session.
        let mut create = self.base_cmd();
        create
            .arg("new-session")
            .arg("-d")
            .arg("-s")
            .arg(&self.session_name);

        self.run(create).await?;
        Ok(())
    }

    /// Return metadata for all windows in the session.
    pub async fn list_windows(&self) -> anyhow::Result<Vec<WindowInfo>> {
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
    pub async fn new_window(&self, name: Option<&str>) -> anyhow::Result<WindowInfo> {
        if let Some(n) = name {
            if !validate_window_name(n) {
                return Err(anyhow::anyhow!(
                    "invalid window name {:?}: must match ^[A-Za-z0-9 ._-]{{1,64}}$",
                    n
                ));
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
        windows
            .pop()
            .ok_or_else(|| anyhow::anyhow!("tmux new-window returned no output"))
    }

    /// Switch the active window to the one identified by `id` (e.g. `"@1"`).
    ///
    /// The ID is validated before the subprocess is spawned.
    pub async fn select_window(&self, id: &str) -> anyhow::Result<()> {
        if !validate_window_id(id) {
            return Err(anyhow::anyhow!(
                "invalid window id {:?}: must match ^@[0-9]{{1,9}}$",
                id
            ));
        }

        let mut cmd = self.base_cmd();
        cmd.arg("select-window")
            .arg("-t")
            .arg(format!("{}:{}", self.session_name, id));

        self.run(cmd).await?;
        Ok(())
    }

    /// Kill a single window by ID (e.g. `"@1"`).
    ///
    /// The ID is validated before the subprocess is spawned.
    pub async fn kill_window(&self, id: &str) -> anyhow::Result<()> {
        if !validate_window_id(id) {
            return Err(anyhow::anyhow!(
                "invalid window id {:?}: must match ^@[0-9]{{1,9}}$",
                id
            ));
        }

        let mut cmd = self.base_cmd();
        cmd.arg("kill-window")
            .arg("-t")
            .arg(format!("{}:{}", self.session_name, id));
        self.run(cmd).await?;
        Ok(())
    }

    /// Destroy the entire session. Intended for daemon shutdown / cleanup.
    pub async fn kill_session(&self) -> anyhow::Result<()> {
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
///
/// Allowed: ASCII alphanumerics, space, `.`, `_`, `-`; length 1–64.
/// Rejected: empty, > 64 chars, non-ASCII (including emoji), control chars,
/// shell-special chars (`;`, `&`, `|`, `` ` ``, `$`, etc.).
pub fn validate_window_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 64 {
        return false;
    }
    name.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b' ' || b == b'.' || b == b'_' || b == b'-')
}

// ── internal parser ───────────────────────────────────────────────────────────

/// Parse the output of `list-windows -F '#{window_id}|#{window_index}|#{window_name}|#{window_active}'`.
///
/// Each non-empty line is split into exactly four fields using `splitn(4, '|')`.
/// Window names from tmux are returned as-is (upstream validators ensure names
/// were whitelisted when created through this controller).
fn parse_window_list(output: &str) -> anyhow::Result<Vec<WindowInfo>> {
    let mut windows = Vec::new();

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let mut parts = line.splitn(4, '|');
        let id = parts
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing window_id in tmux output: {:?}", line))?;
        let index_str = parts
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing window_index in tmux output: {:?}", line))?;
        let name = parts
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing window_name in tmux output: {:?}", line))?;
        let active_str = parts
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing window_active in tmux output: {:?}", line))?;

        let index: u32 = index_str
            .trim()
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid window index {:?}", index_str))?;

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
        assert!(parse_window_list(raw).is_err());
    }

    #[test]
    fn parse_bad_index_returns_error() {
        let raw = "@1|abc|editor|1";
        assert!(parse_window_list(raw).is_err());
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
                                            // 64-character string — maximum
        let max_name = "a".repeat(64);
        assert!(validate_window_name(&max_name));
    }

    #[test]
    fn invalid_window_name_empty() {
        assert!(!validate_window_name(""));
    }

    #[test]
    fn invalid_window_name_shell_injection() {
        assert!(!validate_window_name("a;rm -rf /"));
        assert!(!validate_window_name("$(evil)"));
        assert!(!validate_window_name("a&b"));
        assert!(!validate_window_name("a|b"));
        assert!(!validate_window_name("`cmd`"));
    }

    #[test]
    fn invalid_window_name_too_long() {
        let long = "a".repeat(65);
        assert!(!validate_window_name(&long));
    }

    #[test]
    fn invalid_window_name_unicode_emoji() {
        assert!(!validate_window_name("hello🦊"));
        assert!(!validate_window_name("café"));
    }

    #[test]
    fn invalid_window_name_newline() {
        assert!(!validate_window_name("foo\nbar"));
        assert!(!validate_window_name("foo\r\nbar"));
    }

    #[test]
    fn invalid_window_name_tab() {
        assert!(!validate_window_name("foo\tbar"));
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
