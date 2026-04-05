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
//! [`build_command`]. Each runtime ships with a working ingest +
//! after_turn scaffold template (see `plugin_manager::hook_templates`)
//! that demonstrates the stdin/stdout contract.

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
    /// `ruby script.rb`
    Ruby,
    /// `bash script.sh` — portable shell scripts without needing an exec bit.
    Bash,
    /// `bun run script.ts` — modern JS/TS runtime.
    Bun,
    /// `php script.php` — CLI PHP.
    Php,
    /// `lua script.lua`
    Lua,
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
            Some("ruby") | Some("rb") => Self::Ruby,
            Some("bash") | Some("sh") | Some("shell") => Self::Bash,
            Some("bun") => Self::Bun,
            Some("php") => Self::Php,
            Some("lua") => Self::Lua,
            Some(other) => {
                warn!(
                    "Unknown plugin runtime '{other}', falling back to 'python'. \
                     Valid values: python, native, v, node, deno, go, ruby, bash, bun, php, lua."
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
            Self::Ruby => "ruby",
            Self::Bash => "bash",
            Self::Bun => "bun",
            Self::Php => "php",
            Self::Lua => "lua",
        }
    }

    /// Whether this runtime requires the script file to carry an executable
    /// bit (only `Native` does — everything else is fed to an interpreter).
    pub fn requires_executable_bit(&self) -> bool {
        matches!(self, Self::Native)
    }

    /// Arguments to pass when probing the launcher for its version.
    /// Most runtimes use `--version`; a few have their own conventions
    /// (Go uses `go version`, Lua uses `lua -v`).
    pub fn version_args(&self) -> &'static [&'static str] {
        match self {
            Self::Go => &["version"],
            Self::Lua => &["-v"],
            _ => &["--version"],
        }
    }

    /// Canonical launcher binary to probe on PATH. `Native` has no launcher
    /// (the script *is* the binary), so it returns `None`.
    pub fn launcher_binary(&self) -> Option<&'static str> {
        match self {
            // Python has a fallback chain (python3 → python → py). The doctor
            // probes all three so a host with only `python` still reports OK.
            Self::Python => Some("python3"),
            Self::Native => None,
            Self::V => Some("v"),
            Self::Node => Some("node"),
            Self::Deno => Some("deno"),
            Self::Go => Some("go"),
            Self::Ruby => Some("ruby"),
            Self::Bash => Some("bash"),
            Self::Bun => Some("bun"),
            Self::Php => Some("php"),
            Self::Lua => Some("lua"),
        }
    }

    /// Install hint shown when a runtime's launcher is missing.
    pub fn install_hint(&self) -> &'static str {
        match self {
            Self::Python => "Install Python 3 from https://www.python.org/downloads/ or your OS package manager",
            Self::Native => "Native runtimes have no launcher — make sure the script is executable",
            Self::V => "Install V from https://vlang.io/#install (`v` must be on PATH)",
            Self::Node => "Install Node.js from https://nodejs.org/ (or via nvm/fnm/volta)",
            Self::Deno => "Install Deno from https://deno.com/ (`curl -fsSL https://deno.land/install.sh | sh`)",
            Self::Go => "Install Go from https://go.dev/dl/ (`go` must be on PATH)",
            Self::Ruby => "Install Ruby from https://www.ruby-lang.org/en/downloads/ (or via rbenv/rvm/asdf)",
            Self::Bash => "Install bash via your OS package manager (pre-installed on most Unix-like systems)",
            Self::Bun => "Install Bun from https://bun.sh/ (`curl -fsSL https://bun.sh/install | bash`)",
            Self::Php => "Install PHP from https://www.php.net/downloads.php or your OS package manager",
            Self::Lua => "Install Lua from https://www.lua.org/download.html or your OS package manager",
        }
    }

    /// All runtime variants, in a stable order (useful for diagnostics).
    pub fn all() -> &'static [Self] {
        &[
            Self::Python,
            Self::Native,
            Self::V,
            Self::Node,
            Self::Deno,
            Self::Go,
            Self::Ruby,
            Self::Bash,
            Self::Bun,
            Self::Php,
            Self::Lua,
        ]
    }
}

/// Availability + version info for a single runtime on this host.
///
/// Returned by [`check_runtime_status`] and aggregated into the doctor
/// endpoint. `Native` is always reported as available — nothing to probe.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RuntimeStatus {
    /// Canonical runtime tag (`python`, `native`, `v`, ...).
    pub runtime: String,
    /// Launcher binary actually resolved on PATH, if any.
    pub launcher: Option<String>,
    /// `true` if the launcher was found and responded to `--version`.
    pub available: bool,
    /// First non-empty line of the launcher's `--version` output, trimmed.
    pub version: Option<String>,
    /// Human-facing install hint. Populated for every runtime; consumers
    /// should only surface it when `available` is false.
    pub install_hint: String,
}

/// Probe one runtime by shelling out to `{launcher} --version`.
///
/// Blocking — call from `spawn_blocking` if invoking from an async handler.
/// Cheap enough (<100ms per launcher on a warm cache) that the doctor
/// endpoint probes every runtime on every call without caching.
pub fn check_runtime_status(runtime: PluginRuntime) -> RuntimeStatus {
    let tag = runtime.label().to_string();
    let hint = runtime.install_hint().to_string();

    // Native has no launcher — report as available unconditionally.
    let Some(primary) = runtime.launcher_binary() else {
        return RuntimeStatus {
            runtime: tag,
            launcher: None,
            available: true,
            version: None,
            install_hint: hint,
        };
    };

    // Python gets a fallback chain (python3 → python → py) to match
    // `find_python_interpreter`'s discovery path.
    let candidates: &[&str] = match runtime {
        PluginRuntime::Python => &["python3", "python", "py"],
        _ => std::slice::from_ref(&primary),
    };

    let version_args = runtime.version_args();
    for candidate in candidates {
        if let Some(version) = probe_launcher_version(candidate, version_args) {
            return RuntimeStatus {
                runtime: tag,
                launcher: Some((*candidate).to_string()),
                available: true,
                version,
                install_hint: hint,
            };
        }
    }

    RuntimeStatus {
        runtime: tag,
        launcher: None,
        available: false,
        version: None,
        install_hint: hint,
    }
}

/// Run `{launcher} {version_args...}` with a 5-second wall-clock cap and
/// return the first non-empty line of its output.
///
/// A bounded timeout protects the doctor endpoint from a hanging launcher
/// (broken PATH shim, interactive prompt, stuck network in a wrapper script)
/// locking the spawn_blocking thread indefinitely. stdin is redirected to
/// null so launchers like `lua -v` don't drop into an interactive REPL
/// when they inherit a TTY. Returns `None` if the launcher is missing,
/// exits non-zero, produces no output, or exceeds the deadline.
///
/// Outer `Option` = success/failure. Inner `Option<String>` = the version
/// string (if any output was captured).
fn probe_launcher_version(launcher: &str, version_args: &[&str]) -> Option<Option<String>> {
    const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
    const POLL_INTERVAL: Duration = Duration::from_millis(25);

    let mut child = std::process::Command::new(launcher)
        .args(version_args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .ok()?;

    let deadline = std::time::Instant::now() + PROBE_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(POLL_INTERVAL);
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };

    if !status.success() {
        return None;
    }

    // Read any buffered output. wait_with_output would re-wait (we already
    // waited), so read the pipes directly.
    use std::io::Read;
    let mut stdout = Vec::new();
    if let Some(mut s) = child.stdout.take() {
        let _ = s.read_to_end(&mut stdout);
    }
    let mut stderr = Vec::new();
    if let Some(mut s) = child.stderr.take() {
        let _ = s.read_to_end(&mut stderr);
    }
    // `--version` may write to stdout OR stderr (old Python 2 wrote to stderr).
    let raw = if !stdout.is_empty() { stdout } else { stderr };
    let version = String::from_utf8_lossy(&raw)
        .lines()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim().to_string());
    Some(version)
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
            // Probe for python3 / python / py at spawn time — matches the
            // interpreter discovery the python_runtime module has always done.
            Ok((
                crate::python_runtime::find_python_interpreter(),
                vec![script_path.to_string()],
            ))
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
        PluginRuntime::Ruby => Ok(("ruby".to_string(), vec![script_path.to_string()])),
        PluginRuntime::Bash => Ok(("bash".to_string(), vec![script_path.to_string()])),
        PluginRuntime::Bun => Ok((
            "bun".to_string(),
            vec!["run".to_string(), script_path.to_string()],
        )),
        PluginRuntime::Php => Ok(("php".to_string(), vec![script_path.to_string()])),
        PluginRuntime::Lua => Ok(("lua".to_string(), vec![script_path.to_string()])),
    }
}

/// Env vars a given runtime needs from the parent process to function.
///
/// These land on top of the baseline (PATH, HOME, LIBREFANG_*) that every
/// runtime gets. They're passthrough only — we never synthesize values,
/// just forward whatever the user had.
fn runtime_passthrough_vars(runtime: PluginRuntime) -> &'static [&'static str] {
    match runtime {
        // Python: venv activation + module search path.
        PluginRuntime::Python => &["PYTHONPATH", "VIRTUAL_ENV", "PYTHONIOENCODING"],
        // V: module lookup dir.
        PluginRuntime::V => &["VMODULES"],
        // Node: CommonJS resolver roots.
        PluginRuntime::Node => &["NODE_PATH"],
        // Deno: dep cache.
        PluginRuntime::Deno => &["DENO_DIR"],
        // Go: toolchain + module cache.
        PluginRuntime::Go => &["GOPATH", "GOMODCACHE", "GOCACHE"],
        // Ruby: load path + gem dirs.
        PluginRuntime::Ruby => &["RUBYLIB", "RUBYOPT", "GEM_HOME", "GEM_PATH"],
        // Bash: nothing beyond baseline — scripts read their own env.
        PluginRuntime::Bash => &[],
        // Bun: install + dep cache location.
        PluginRuntime::Bun => &["BUN_INSTALL"],
        // PHP: INI scan dir (user-level php.ini).
        PluginRuntime::Php => &["PHP_INI_SCAN_DIR"],
        // Lua: module search paths.
        PluginRuntime::Lua => &["LUA_PATH", "LUA_CPATH"],
        // Native binaries get nothing runtime-specific — any needed env
        // has to be listed in `config.allowed_env_vars`.
        PluginRuntime::Native => &[],
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
    // Runtime-specific passthrough (venv vars for Python, module cache
    // paths for Go/V, etc.). Table-driven so adding a new runtime is a
    // one-line append.
    for var in runtime_passthrough_vars(runtime) {
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
    fn version_args_are_runtime_specific() {
        // Go and Lua have their own conventions.
        assert_eq!(PluginRuntime::Go.version_args(), &["version"]);
        assert_eq!(PluginRuntime::Lua.version_args(), &["-v"]);
        // Everyone else uses --version.
        assert_eq!(PluginRuntime::Python.version_args(), &["--version"]);
        assert_eq!(PluginRuntime::Node.version_args(), &["--version"]);
        assert_eq!(PluginRuntime::Ruby.version_args(), &["--version"]);
    }

    #[test]
    fn doctor_reports_python_as_available() {
        // Python is on every CI runner we target. A green doctor probe
        // verifies the full path: Command::spawn -> try_wait -> read pipes.
        let status = check_runtime_status(PluginRuntime::Python);
        assert_eq!(status.runtime, "python");
        assert!(
            status.available,
            "python probe failed: {status:?} (version_args mismatch?)"
        );
        assert!(status.launcher.is_some());
        assert!(status.version.is_some());
    }

    #[test]
    fn doctor_reports_native_without_probing() {
        let status = check_runtime_status(PluginRuntime::Native);
        assert_eq!(status.runtime, "native");
        assert!(status.available, "native should always be available");
        assert!(status.launcher.is_none());
        assert!(status.version.is_none());
    }

    #[test]
    fn doctor_flags_missing_launcher() {
        let status = check_runtime_status(PluginRuntime::V); // v is rarely installed
                                                             // We can't assert unavailable deterministically (V *might* be
                                                             // installed), so just check the response shape stays consistent.
        assert_eq!(status.runtime, "v");
        if !status.available {
            assert!(status.launcher.is_none());
            assert!(status.version.is_none());
            assert!(!status.install_hint.is_empty());
        }
    }

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
        assert_eq!(
            PluginRuntime::from_tag(Some("brainfuck")),
            PluginRuntime::Python
        );
    }

    #[test]
    fn from_tag_new_runtimes() {
        assert_eq!(PluginRuntime::from_tag(Some("ruby")), PluginRuntime::Ruby);
        assert_eq!(PluginRuntime::from_tag(Some("rb")), PluginRuntime::Ruby);
        assert_eq!(PluginRuntime::from_tag(Some("bash")), PluginRuntime::Bash);
        assert_eq!(PluginRuntime::from_tag(Some("sh")), PluginRuntime::Bash);
        assert_eq!(PluginRuntime::from_tag(Some("bun")), PluginRuntime::Bun);
        assert_eq!(PluginRuntime::from_tag(Some("php")), PluginRuntime::Php);
        assert_eq!(PluginRuntime::from_tag(Some("lua")), PluginRuntime::Lua);
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

    /// Python runtime goes through the same unified spawn path as V/Go/Node —
    /// proves there's no special-case shim anymore.
    #[cfg(unix)]
    #[tokio::test]
    async fn python_runtime_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let hook = tmp.path().join("ingest.py");
        std::fs::write(
            &hook,
            "import json, sys\n\
             req = json.loads(sys.stdin.read())\n\
             print(json.dumps({\"type\": \"ingest_result\", \"echo\": req[\"message\"]}))\n",
        )
        .unwrap();

        // Skip test if no python interpreter is on PATH (CI can vary).
        let have_python = ["python3", "python"].iter().any(|bin| {
            std::process::Command::new(bin)
                .arg("--version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok()
        });
        if !have_python {
            eprintln!("skipping python_runtime_round_trip: no python on PATH");
            return;
        }

        let input = serde_json::json!({
            "type": "ingest",
            "agent_id": "agent-1",
            "message": "ping",
        });
        let out = run_hook_json(
            hook.to_str().unwrap(),
            PluginRuntime::Python,
            &input,
            &HookConfig::default(),
        )
        .await
        .expect("python hook ran");
        assert_eq!(out["type"], "ingest_result");
        assert_eq!(out["echo"], "ping");
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
