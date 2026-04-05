//! Language-agnostic plugin hook runtime.
//!
//! Hook scripts speak a simple JSON-over-stdin/stdout protocol:
//!
//! 1. librefang writes one JSON object + newline to the script's stdin,
//!    then closes stdin.
//! 2. The script emits one or more lines on stdout; the last line that
//!    parses as JSON is taken as the response.
//! 3. Exit code 0 = success, non-zero = error (stderr is surfaced).
//!
//! This module picks *how* to launch the script based on the plugin's
//! declared `runtime`:
//!
//! - `python` — runs `.py` files through the existing `python_runtime`
//!   (keeps every pre-existing plugin working untouched).
//! - `native` — execs a pre-compiled binary directly. Ideal for V / Rust
//!   / Go / Zig / C++ plugins that ship their own binary.
//! - `v`      — `v run script.v` (V language; <https://github.com/vlang/v>)
//! - `node`   — `node script.js`
//! - `deno`   — `deno run --allow-read script.ts`
//! - `go`     — `go run script.go`
//!
//! Unknown runtime strings fall back to `python` with a warning, so a
//! typo in `plugin.toml` never takes a hook completely offline.
//!
//! The protocol itself is language-agnostic — adding another runtime just
//! means adding a variant to [`PluginRuntime`] and a match arm in
//! [`spawn_hook`]. SDK helpers for each language live under
//! `examples/plugin-sdks/`.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tracing::{debug, warn};

/// Which launcher runs a hook script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginRuntime {
    /// `python3 script.py` — the original (and default) runtime.
    Python,
    /// Exec the file directly. Requires the executable bit and a valid
    /// binary (or shebang). Ideal for pre-compiled V / Rust / Go binaries.
    Native,
    /// `v run script.v` — compile-and-run a V source file.
    V,
    /// `node script.js` — CommonJS or ESM, Node's choice.
    Node,
    /// `deno run --allow-read script.ts` — TypeScript via Deno.
    Deno,
    /// `go run script.go` — compile-and-run a single Go file.
    Go,
}

impl PluginRuntime {
    /// Parse a runtime tag from `plugin.toml`. Unknown / empty strings
    /// default to `Python` so a typo is never a hard failure.
    pub fn from_tag(tag: Option<&str>) -> Self {
        match tag.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
            None | Some("") | Some("python") | Some("python3") | Some("py") => Self::Python,
            Some("native") | Some("binary") | Some("exec") => Self::Native,
            Some("v") | Some("vlang") => Self::V,
            Some("node") | Some("nodejs") | Some("js") => Self::Node,
            Some("deno") | Some("ts") | Some("typescript") => Self::Deno,
            Some("go") | Some("golang") => Self::Go,
            Some(other) => {
                warn!(
                    "Unknown plugin runtime '{other}', falling back to 'python'. \
                     Valid values: python, native, v, node, deno, go."
                );
                Self::Python
            }
        }
    }

    /// Human-readable label for error messages.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Python => "python",
            Self::Native => "native",
            Self::V => "v",
            Self::Node => "node",
            Self::Deno => "deno",
            Self::Go => "go",
        }
    }

    /// Whether this runtime requires the script file to carry an executable
    /// bit (only `Native` does — everything else is fed to an interpreter).
    pub fn requires_executable_bit(&self) -> bool {
        matches!(self, Self::Native)
    }
}

/// Error surfaced from a plugin hook run.
#[derive(Debug, thiserror::Error)]
pub enum PluginRuntimeError {
    #[error("Script not found: {0}")]
    ScriptNotFound(String),
    #[error("Path traversal denied: {0}")]
    PathTraversal(String),
    #[error("Runtime launcher '{launcher}' not found on PATH: {reason}")]
    LauncherNotFound { launcher: String, reason: String },
    #[error("Failed to spawn hook: {0}")]
    SpawnFailed(String),
    #[error("IO error: {0}")]
    Io(String),
    #[error("Hook timed out after {0}s")]
    Timeout(u64),
    #[error("Hook exited with code {code:?}. stderr: {stderr}")]
    ScriptError { code: Option<i32>, stderr: String },
    #[error("Hook produced no output")]
    EmptyOutput,
}

/// Minimum config shared by every runtime.
#[derive(Debug, Clone)]
pub struct HookConfig {
    /// Max execution time — hook scripts should be snappy.
    pub timeout_secs: u64,
    /// Working directory for the spawned process.
    pub working_dir: Option<PathBuf>,
    /// Extra env vars to pass through from the parent process.
    pub allowed_env_vars: Vec<String>,
}

impl Default for HookConfig {
    fn default() -> Self {
        Self {
            timeout_secs: 30,
            working_dir: None,
            allowed_env_vars: Vec::new(),
        }
    }
}

/// Reject `..` components. Every runtime validates this before spawn.
fn validate_path_traversal(path: &str) -> Result<(), PluginRuntimeError> {
    for component in Path::new(path).components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err(PluginRuntimeError::PathTraversal(path.to_string()));
        }
    }
    Ok(())
}

/// Build the command line for a given runtime + script path.
///
/// Returns `(launcher, args)`. `launcher` is the program we `exec`;
/// `args` are its arguments (the first arg is typically the script path).
fn build_command(
    runtime: PluginRuntime,
    script_path: &str,
) -> Result<(String, Vec<String>), PluginRuntimeError> {
    match runtime {
        PluginRuntime::Python => {
            // Python delegates to the existing python_runtime which has
            // its own validation, interpreter discovery, env scrubbing.
            // This branch is never hit in practice (we dispatch to
            // python_runtime::run_python_json before reaching here) but
            // keeping it for symmetry makes the fallback path obvious.
            Ok(("python3".to_string(), vec![script_path.to_string()]))
        }
        PluginRuntime::Native => {
            // Exec the file directly — no interpreter. We rely on the
            // executable bit + shebang, so absolute paths are fine.
            Ok((script_path.to_string(), Vec::new()))
        }
        PluginRuntime::V => Ok((
            "v".to_string(),
            vec![
                "-no-retry-compilation".to_string(),
                "run".to_string(),
                script_path.to_string(),
            ],
        )),
        PluginRuntime::Node => Ok(("node".to_string(), vec![script_path.to_string()])),
        PluginRuntime::Deno => Ok((
            "deno".to_string(),
            vec![
                "run".to_string(),
                "--allow-read".to_string(),
                "--allow-env".to_string(),
                script_path.to_string(),
            ],
        )),
        PluginRuntime::Go => Ok((
            "go".to_string(),
            vec!["run".to_string(), script_path.to_string()],
        )),
    }
}

/// Run a hook script and parse the last JSON line of stdout.
///
/// This is the main entry point — picks the right launcher based on
/// `runtime`, enforces the timeout, scrubs inherited env, and returns
/// the raw JSON value the script emitted.
pub async fn run_hook_json(
    script_path: &str,
    runtime: PluginRuntime,
    input: &serde_json::Value,
    config: &HookConfig,
) -> Result<serde_json::Value, PluginRuntimeError> {
    validate_path_traversal(script_path)?;
    if !Path::new(script_path).exists() {
        return Err(PluginRuntimeError::ScriptNotFound(script_path.to_string()));
    }

    // Python gets the battle-tested path for backwards compat.
    if runtime == PluginRuntime::Python {
        let py_config = crate::python_runtime::PythonConfig {
            timeout_secs: config.timeout_secs,
            working_dir: config
                .working_dir
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            allowed_env_vars: config.allowed_env_vars.clone(),
            ..Default::default()
        };
        let result = crate::python_runtime::run_python_json(script_path, input, &py_config)
            .await
            .map_err(|e| PluginRuntimeError::ScriptError {
                code: None,
                stderr: e.to_string(),
            })?;
        return Ok(serde_json::from_str(&result.response)
            .unwrap_or_else(|_| serde_json::json!({ "text": result.response })));
    }

    // Non-Python runtimes share this spawn path.
    let input_line =
        serde_json::to_string(input).map_err(|e| PluginRuntimeError::Io(e.to_string()))?;
    let (launcher, args) = build_command(runtime, script_path)?;
    let agent_id = input.get("agent_id").and_then(|v| v.as_str()).unwrap_or("");
    let message = input.get("message").and_then(|v| v.as_str()).unwrap_or("");

    debug!(
        "Running {} hook: launcher={} args={:?}",
        runtime.label(),
        launcher,
        args
    );

    let mut cmd = Command::new(&launcher);
    cmd.args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Some(ref wd) = config.working_dir {
        cmd.current_dir(wd);
    }

    // SECURITY: Wipe inherited environment, then re-add only a safe baseline.
    // Matches the hardening in python_runtime so V / Node / Go plugins don't
    // accidentally get host credentials.
    cmd.env_clear();
    cmd.env("LIBREFANG_AGENT_ID", agent_id);
    cmd.env("LIBREFANG_MESSAGE", message);
    cmd.env("LIBREFANG_RUNTIME", runtime.label());
    if let Ok(path) = std::env::var("PATH") {
        cmd.env("PATH", path);
    }
    if let Ok(home) = std::env::var("HOME") {
        cmd.env("HOME", home);
    }
    #[cfg(windows)]
    {
        for var in &[
            "USERPROFILE",
            "SYSTEMROOT",
            "APPDATA",
            "LOCALAPPDATA",
            "COMSPEC",
        ] {
            if let Ok(val) = std::env::var(var) {
                cmd.env(var, val);
            }
        }
    }
    // Runtime-specific extras (Go modules, V cache, Deno dir, Node paths).
    for var in &[
        "GOPATH",
        "GOMODCACHE",
        "GOCACHE",
        "VMODULES",
        "DENO_DIR",
        "NODE_PATH",
    ] {
        if let Ok(val) = std::env::var(var) {
            cmd.env(var, val);
        }
    }
    for var in &config.allowed_env_vars {
        if let Ok(val) = std::env::var(var) {
            cmd.env(var, val);
        }
    }

    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            PluginRuntimeError::LauncherNotFound {
                launcher: launcher.clone(),
                reason: e.to_string(),
            }
        } else {
            PluginRuntimeError::SpawnFailed(e.to_string())
        }
    })?;

    // Write JSON payload + newline, then close stdin.
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(input_line.as_bytes())
            .await
            .map_err(|e| PluginRuntimeError::Io(e.to_string()))?;
        stdin
            .write_all(b"\n")
            .await
            .map_err(|e| PluginRuntimeError::Io(e.to_string()))?;
        drop(stdin);
    }

    let timeout = Duration::from_secs(config.timeout_secs);
    let result = tokio::time::timeout(timeout, async {
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| PluginRuntimeError::Io("stdout not captured".to_string()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| PluginRuntimeError::Io("stderr not captured".to_string()))?;

        let mut stdout_reader = BufReader::new(stdout);
        let mut stderr_reader = BufReader::new(stderr);

        let mut stdout_lines: Vec<String> = Vec::new();
        let mut stderr_text = String::new();

        let mut line = String::new();
        loop {
            line.clear();
            match stdout_reader.read_line(&mut line).await {
                Ok(0) => break,
                Ok(_) => stdout_lines.push(line.trim_end().to_string()),
                Err(e) => {
                    warn!("hook stdout read error: {e}");
                    break;
                }
            }
        }
        let mut err_line = String::new();
        loop {
            err_line.clear();
            match stderr_reader.read_line(&mut err_line).await {
                Ok(0) => break,
                Ok(_) => stderr_text.push_str(&err_line),
                Err(_) => break,
            }
        }

        let status = child
            .wait()
            .await
            .map_err(|e| PluginRuntimeError::Io(e.to_string()))?;

        Ok::<(Vec<String>, String, Option<i32>), PluginRuntimeError>((
            stdout_lines,
            stderr_text,
            status.code(),
        ))
    })
    .await;

    match result {
        Ok(Ok((stdout_lines, stderr_text, exit_code))) => {
            if exit_code != Some(0) {
                return Err(PluginRuntimeError::ScriptError {
                    code: exit_code,
                    stderr: stderr_text.trim().to_string(),
                });
            }
            if !stderr_text.trim().is_empty() {
                debug!("hook stderr: {}", stderr_text.trim());
            }
            parse_output(&stdout_lines)
        }
        Ok(Err(e)) => Err(e),
        Err(_) => {
            let _ = child.kill().await;
            Err(PluginRuntimeError::Timeout(config.timeout_secs))
        }
    }
}

/// Scan stdout lines in reverse, returning the last one that parses as JSON.
/// Falls back to wrapping the whole output in `{"text": "..."}` when nothing
/// looks like JSON — matches the behaviour of the Python hook dispatcher so
/// ad-hoc `println!("hello")` scripts still work.
fn parse_output(lines: &[String]) -> Result<serde_json::Value, PluginRuntimeError> {
    for line in lines.iter().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
            return Ok(v);
        }
    }
    let joined = lines.join("\n");
    if joined.trim().is_empty() {
        return Err(PluginRuntimeError::EmptyOutput);
    }
    Ok(serde_json::json!({ "text": joined }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_tag_defaults_to_python() {
        assert_eq!(PluginRuntime::from_tag(None), PluginRuntime::Python);
        assert_eq!(PluginRuntime::from_tag(Some("")), PluginRuntime::Python);
        assert_eq!(
            PluginRuntime::from_tag(Some("python")),
            PluginRuntime::Python
        );
        assert_eq!(PluginRuntime::from_tag(Some("py")), PluginRuntime::Python);
    }

    #[test]
    fn from_tag_normalizes_case_and_aliases() {
        assert_eq!(PluginRuntime::from_tag(Some("V")), PluginRuntime::V);
        assert_eq!(PluginRuntime::from_tag(Some("VLang")), PluginRuntime::V);
        assert_eq!(PluginRuntime::from_tag(Some("Node")), PluginRuntime::Node);
        assert_eq!(PluginRuntime::from_tag(Some("JS")), PluginRuntime::Node);
        assert_eq!(PluginRuntime::from_tag(Some("golang")), PluginRuntime::Go);
        assert_eq!(
            PluginRuntime::from_tag(Some("binary")),
            PluginRuntime::Native
        );
    }

    #[test]
    fn from_tag_unknown_falls_back_to_python() {
        assert_eq!(PluginRuntime::from_tag(Some("ruby")), PluginRuntime::Python);
    }

    #[test]
    fn parse_output_picks_last_json_line() {
        let lines = vec![
            "warming up...".to_string(),
            "{\"type\":\"ingest_result\",\"memories\":[]}".to_string(),
        ];
        let v = parse_output(&lines).unwrap();
        assert_eq!(v["type"], "ingest_result");
    }

    #[test]
    fn parse_output_falls_back_to_text_wrapper() {
        let lines = vec!["just plain text".to_string()];
        let v = parse_output(&lines).unwrap();
        assert_eq!(v["text"], "just plain text");
    }

    #[test]
    fn parse_output_empty_is_error() {
        assert!(matches!(
            parse_output(&[]),
            Err(PluginRuntimeError::EmptyOutput)
        ));
    }

    #[test]
    fn validate_path_traversal_rejects_parent_dir() {
        assert!(validate_path_traversal("../etc/passwd").is_err());
        assert!(validate_path_traversal("hooks/../evil.sh").is_err());
        assert!(validate_path_traversal("hooks/ingest.py").is_ok());
    }

    #[test]
    fn build_command_shapes() {
        let (l, a) = build_command(PluginRuntime::V, "hooks/ingest.v").unwrap();
        assert_eq!(l, "v");
        assert!(a.contains(&"run".to_string()));
        assert!(a.contains(&"hooks/ingest.v".to_string()));

        let (l, a) = build_command(PluginRuntime::Native, "hooks/ingest").unwrap();
        assert_eq!(l, "hooks/ingest");
        assert!(a.is_empty());

        let (l, a) = build_command(PluginRuntime::Go, "hooks/ingest.go").unwrap();
        assert_eq!(l, "go");
        assert_eq!(a, vec!["run".to_string(), "hooks/ingest.go".to_string()]);

        let (l, a) = build_command(PluginRuntime::Deno, "hooks/ingest.ts").unwrap();
        assert_eq!(l, "deno");
        assert!(a.contains(&"--allow-read".to_string()));
    }

    /// End-to-end: scaffold a sh-based native hook, run it, check JSON round-trip.
    /// Uses `sh` so it works without V/Go/Node installed. Skipped on Windows
    /// (no /bin/sh by default).
    #[cfg(unix)]
    #[tokio::test]
    async fn native_runtime_round_trip() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let hook = tmp.path().join("echo_hook");
        std::fs::write(
            &hook,
            "#!/bin/sh\nread _input\nprintf '{\"type\":\"ingest_result\",\"memories\":[]}\\n'\n",
        )
        .unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();

        let input = serde_json::json!({
            "type": "ingest",
            "agent_id": "agent-42",
            "message": "hello",
        });
        let out = run_hook_json(
            hook.to_str().unwrap(),
            PluginRuntime::Native,
            &input,
            &HookConfig::default(),
        )
        .await
        .expect("native hook ran");
        assert_eq!(out["type"], "ingest_result");
        assert!(out["memories"].is_array());
    }

    /// Timeout path: a hook that sleeps forever should be killed.
    #[cfg(unix)]
    #[tokio::test]
    async fn native_runtime_timeout_is_enforced() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let hook = tmp.path().join("slow_hook");
        std::fs::write(&hook, "#!/bin/sh\nsleep 30\n").unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();

        let config = HookConfig {
            timeout_secs: 1,
            ..Default::default()
        };
        let err = run_hook_json(
            hook.to_str().unwrap(),
            PluginRuntime::Native,
            &serde_json::json!({"type": "ingest"}),
            &config,
        )
        .await
        .expect_err("should time out");
        assert!(matches!(err, PluginRuntimeError::Timeout(1)));
    }

    /// Missing script surfaces ScriptNotFound (the launcher-not-found path is
    /// exercised on real systems where `v` / `go` / `deno` aren't installed).
    #[tokio::test]
    async fn missing_script_is_script_not_found() {
        let err = run_hook_json(
            "hooks/does-not-exist.v",
            PluginRuntime::V,
            &serde_json::json!({}),
            &HookConfig::default(),
        )
        .await
        .expect_err("should fail");
        assert!(matches!(err, PluginRuntimeError::ScriptNotFound(_)));
    }
}
