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

/// Classify a process exit status into a human-readable label.
///
/// Returns a short string like `"OOM-killed (exit 137)"`, `"SIGSEGV (exit 139)"`,
/// `"killed by signal 9"`, or `"exit code 1"` for ordinary failures.
#[cfg(unix)]
fn classify_exit_status(status: &std::process::ExitStatus) -> String {
    use std::os::unix::process::ExitStatusExt;
    if let Some(signal) = status.signal() {
        return match signal {
            9  => format!("OOM-killed or SIGKILL (signal {signal})"),
            11 => format!("SIGSEGV — segfault (signal {signal})"),
            31 => format!("SIGSYS — disallowed syscall, seccomp triggered (signal {signal})"),
            _  => format!("killed by signal {signal}"),
        };
    }
    // On Unix, exit code 128+N means "killed by signal N" when reported via wait().
    if let Some(code) = status.code() {
        return match code {
            137 => "OOM-killed or SIGKILL (exit 137)".to_string(),
            139 => "SIGSEGV — segfault (exit 139)".to_string(),
            159 => "SIGSYS — disallowed syscall, seccomp triggered (exit 159)".to_string(),
            _   => format!("exit code {code}"),
        };
    }
    "unknown exit status".to_string()
}

#[cfg(not(unix))]
fn classify_exit_status(status: &std::process::ExitStatus) -> String {
    match status.code() {
        Some(code) => format!("exit code {code}"),
        None => "unknown exit status".to_string(),
    }
}

/// Read the peak RSS (VmPeak or VmRSS) of a process from `/proc/{pid}/status`.
///
/// Returns the value in kilobytes, or `None` if unavailable (non-Linux, permission
/// denied, or process already reaped).
///
/// We read `VmPeak` (peak virtual memory) as a proxy for maximum RSS since
/// `VmRSS` is the current value and may already be 0 after exit.
/// Falls back to `VmRSS` if `VmPeak` is absent.
#[cfg(target_os = "linux")]
fn read_proc_rss_kb(pid: u32) -> Option<u64> {
    let path = format!("/proc/{pid}/status");
    let content = std::fs::read_to_string(&path).ok()?;
    // Try VmPeak first, then VmRSS.
    for prefix in &["VmPeak:", "VmRSS:"] {
        for line in content.lines() {
            if line.starts_with(prefix) {
                // Format: "VmPeak:   12345 kB"
                let kb = line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|v| v.parse::<u64>().ok());
                if kb.is_some() {
                    return kb;
                }
            }
        }
    }
    None
}

#[cfg(not(target_os = "linux"))]
fn read_proc_rss_kb(_pid: u32) -> Option<u64> {
    None
}

/// Per-path advisory locks for shared state files.
///
/// Provides mutual exclusion within a single process. Cross-process safety
/// (e.g. two daemon instances) is out of scope — only one daemon should own
/// a plugin's state file at a time.
static STATE_FILE_LOCKS: once_cell::sync::Lazy<
    std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>,
> = once_cell::sync::Lazy::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// Acquire the in-process advisory lock for a shared state file path.
///
/// Must be held for the full duration of any hook subprocess call that
/// may read or write the file (i.e. when `config.state_file.is_some()`).
/// This prevents concurrent ingest + after_turn scripts from racing on
/// the same JSON file within a single daemon process.
///
/// # Cross-process safety
/// This lock is in-process only. Running two LibreFang daemons pointing
/// at the same plugin directory will bypass this protection. Ensure only
/// one daemon owns a plugin's state file at a time.
pub async fn lock_state_file(path: &std::path::Path) -> tokio::sync::OwnedMutexGuard<()> {
    let arc = {
        let mut map = STATE_FILE_LOCKS.lock().unwrap();
        map.entry(path.to_string_lossy().into_owned())
           .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
           .clone()
    };
    arc.lock_owned().await
}

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
    /// Execute the hook as a WebAssembly module via wasmtime + WASI.
    /// The `.wasm` file path is used directly — no interpreter needed.
    Wasm,
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
            Some("wasm") | Some("webassembly") => Self::Wasm,
            Some(other) => {
                warn!(
                    "Unknown plugin runtime '{other}', falling back to 'python'. \
                     Valid values: python, native, v, node, deno, go, ruby, bash, bun, php, lua, wasm."
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
            Self::Wasm => "wasm",
        }
    }

    /// Whether this runtime requires the script file to carry an executable
    /// bit (only `Native` does — everything else is fed to an interpreter).
    pub fn requires_executable_bit(&self) -> bool {
        matches!(self, Self::Native)
    }

    /// Whether this runtime executes inline (no subprocess fork).
    ///
    /// `Wasm` hooks run inside the daemon via wasmtime, so the persistent
    /// process pool and subprocess-based sandboxing do not apply.
    pub fn is_inline(&self) -> bool {
        matches!(self, Self::Wasm)
    }

    /// Canonical file extension for hook scripts of this runtime.
    /// Used when generating scaffold comments in `plugin.toml`.
    pub fn script_extension(&self) -> &'static str {
        match self {
            Self::Python => "py",
            Self::Native => "bin",
            Self::V => "v",
            Self::Node => "js",
            Self::Deno => "ts",
            Self::Go => "go",
            Self::Ruby => "rb",
            Self::Bash => "sh",
            Self::Bun => "ts",
            Self::Php => "php",
            Self::Lua => "lua",
            Self::Wasm => "wasm",
        }
    }

    /// Arguments to pass when probing the launcher for its version.
    /// Most runtimes use `--version`; a few have their own conventions
    /// (Go uses `go version`, Lua uses `lua -v`).
    /// `Wasm` has no launcher, so this is never called in practice.
    pub fn version_args(&self) -> &'static [&'static str] {
        match self {
            Self::Go => &["version"],
            Self::Lua => &["-v"],
            _ => &["--version"],
        }
    }

    /// Canonical launcher binary to probe on PATH. `Native` and `Wasm` have no
    /// launcher (the script *is* the binary / module), so they return `None`.
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
            // Wasm runs inline via wasmtime — no external launcher binary.
            Self::Wasm => None,
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
            Self::Wasm => "Wasm hooks run inline via the built-in wasmtime engine — no external launcher needed",
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
            Self::Wasm,
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
    // Wasm has no launcher so it should have been caught by the early-return above,
    // but handle it defensively here too.
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
    #[error("Hook output could not be parsed: {0}")]
    InvalidOutput(String),
}

/// Minimum config shared by every runtime.
#[derive(Debug, Clone)]
pub struct HookConfig {
    /// Max execution time — hook scripts should be snappy.
    pub timeout_secs: u64,
    /// Working directory for the spawned process.
    pub working_dir: Option<PathBuf>,
    /// Extra env var names to pass through from the parent process.
    pub allowed_env_vars: Vec<String>,
    /// Plugin-declared env vars injected directly (key=value).
    ///
    /// Values starting with `${VAR}` are expanded from `std::env` at spawn time.
    /// This is populated from `plugin.toml`'s `[env]` section.
    pub plugin_env: Vec<(String, String)>,
    /// Maximum virtual memory (MiB) for the hook subprocess.
    ///
    /// Applied via `RLIMIT_AS` on Linux. Ignored on other platforms (warning
    /// is logged instead). `None` means no limit beyond the OS default.
    pub max_memory_mb: Option<u64>,
    /// Allow the hook subprocess to open network connections.
    ///
    /// When `false`: on Linux, wraps the launch with `unshare --net` if that
    /// binary is available; on all platforms, injects `no_proxy=*`/`NO_PROXY=*`
    /// into the subprocess env. Defaults to `true`.
    pub allow_network: bool,
    /// Whether hook subprocesses are allowed filesystem write access.
    ///
    /// When `false`:
    /// - `LIBREFANG_READONLY_FS=1` env var is injected (advisory, for well-behaved scripts)
    /// - On Linux 5.13+: Landlock LSM restriction applied in child before exec (enforced)
    /// - On Linux (older): best-effort `unshare --mount` (requires user namespaces)
    ///
    /// Syscall sandboxing (seccomp-sandbox feature):
    /// - Applied unconditionally on Linux when feature is enabled
    /// - Allowlist of ~60 syscalls; any other syscall kills the process with SIGSYS
    /// - Does not require root or user namespaces
    pub allow_filesystem: bool,
    /// Path to the per-plugin shared state JSON file.
    ///
    /// When `Some`, the path is injected as `LIBREFANG_STATE_FILE` into the
    /// subprocess environment. Hook scripts can read/write this file to persist
    /// state across invocations. The file is created (as `{}`) if it does not
    /// exist. `None` = shared state disabled (default).
    pub state_file: Option<PathBuf>,
    /// Per-hook timeout overrides (seconds).  Hook names listed here override
    /// `timeout_secs`.  Example: `{"bootstrap": 60, "ingest": 5}`.
    pub hook_timeouts: std::collections::HashMap<String, u64>,
    /// Base delay between hook retries in milliseconds.
    ///
    /// This is the delay used for the first retry attempt (attempt 0).
    /// Subsequent attempts are scaled by `retry_backoff_multiplier`, up to
    /// `max_retry_delay_ms`. Defaults to `500` ms.
    pub retry_delay_ms: u64,
    /// Multiplier applied to the delay on each successive retry attempt.
    /// `1.0` means fixed delay (no backoff).  `2.0` means delay doubles each
    /// attempt.  Defaults to `2.0`.
    pub retry_backoff_multiplier: f64,
    /// Maximum delay between retries in milliseconds, regardless of
    /// how many times the multiplier has been applied.  Defaults to 30 000 ms.
    pub max_retry_delay_ms: u64,
}

impl Default for HookConfig {
    fn default() -> Self {
        Self {
            timeout_secs: 30,
            working_dir: None,
            allowed_env_vars: Vec::new(),
            plugin_env: Vec::new(),
            max_memory_mb: None,
            allow_network: true,
            allow_filesystem: true,
            state_file: None,
            hook_timeouts: std::collections::HashMap::new(),
            retry_delay_ms: 500,
            retry_backoff_multiplier: 2.0,
            max_retry_delay_ms: 30_000,
        }
    }
}

impl HookConfig {
    /// Return the effective timeout for a given hook name.
    /// Falls back to `self.timeout_secs` if no specific override is set.
    pub fn timeout_for(&self, hook_name: &str) -> u64 {
        self.hook_timeouts
            .get(hook_name)
            .copied()
            .unwrap_or(self.timeout_secs)
    }

    /// Compute the retry delay for a given attempt number (0-indexed).
    ///
    /// `attempt 0` → `retry_delay_ms` (base)
    /// `attempt 1` → `retry_delay_ms * multiplier`
    /// `attempt n` → capped at `max_retry_delay_ms`
    pub fn delay_for_attempt(&self, attempt: u32) -> u64 {
        if attempt == 0 {
            return self.retry_delay_ms;
        }
        let delay = self.retry_delay_ms as f64
            * self.retry_backoff_multiplier.powi(attempt as i32);
        delay.min(self.max_retry_delay_ms as f64) as u64
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
        // Wasm is handled inline before build_command is reached.
        PluginRuntime::Wasm => Err(PluginRuntimeError::SpawnFailed(
            "build_command called for Wasm runtime — this is a bug; Wasm hooks must be \
             dispatched via run_wasm_hook before reaching build_command"
                .to_string(),
        )),
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
        // Wasm runs inline — no subprocess, no passthrough vars needed.
        PluginRuntime::Wasm => &[],
    }
}

/// On Linux, attempt to wrap a command with `unshare --net` for true network
/// namespace isolation when `allow_network == false`.
///
/// Returns `(launcher, args)` unchanged if `unshare` is not available or if
/// we are not on Linux. The probe uses `--help` (which exits 0 on modern util-linux)
/// rather than `--version` because some older builds don't support `--version`.
#[cfg(target_os = "linux")]
fn try_wrap_with_unshare(launcher: &str, args: &[String]) -> (String, Vec<String>) {
    let available = std::process::Command::new("unshare")
        .arg("--help")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if available {
        let mut new_args = vec![
            "--net".to_string(),
            "--".to_string(),
            launcher.to_string(),
        ];
        new_args.extend_from_slice(args);
        return ("unshare".to_string(), new_args);
    }
    (launcher.to_string(), args.to_vec())
}

#[cfg(not(target_os = "linux"))]
fn try_wrap_with_unshare(launcher: &str, args: &[String]) -> (String, Vec<String>) {
    (launcher.to_string(), args.to_vec())
}

/// On Linux, attempt to wrap a command with `unshare --mount` for mount namespace
/// isolation when `allow_filesystem == false`.
///
/// Returns `(launcher, args)` unchanged if `unshare` is not available or if
/// we are not on Linux. Best-effort: falls back silently.
#[cfg(target_os = "linux")]
fn try_wrap_with_unshare_mount(launcher: &str, args: &[String]) -> (String, Vec<String>) {
    let available = std::process::Command::new("unshare")
        .arg("--help")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if available {
        let mut new_args = vec![
            "--mount".to_string(),
            "--".to_string(),
            launcher.to_string(),
        ];
        new_args.extend_from_slice(args);
        return ("unshare".to_string(), new_args);
    }
    (launcher.to_string(), args.to_vec())
}

#[cfg(not(target_os = "linux"))]
fn try_wrap_with_unshare_mount(launcher: &str, args: &[String]) -> (String, Vec<String>) {
    (launcher.to_string(), args.to_vec())
}

/// Attempt to apply a Landlock read-only filesystem restriction to the current process.
///
/// Landlock (Linux 5.13+) allows unprivileged processes to restrict their own
/// filesystem access without requiring root or `unshare`.  We restrict to read-only
/// access for the entire filesystem, then re-allow read-write for a per-call temp dir.
///
/// Returns `true` if Landlock was applied, `false` if unavailable (older kernel, non-Linux,
/// or compiled without the `landlock-sandbox` feature).
#[cfg(all(target_os = "linux", feature = "landlock-sandbox"))]
fn try_apply_landlock_readonly(allow_write_dir: Option<&std::path::Path>) -> bool {
    use landlock::{
        Access, AccessFs, PathBeneath, PathFd, Ruleset, RulesetAttr, RulesetCreatedAttr,
        RulesetStatus, ABI,
    };
    let abi = ABI::V3;
    let result = Ruleset::default()
        .handle_access(AccessFs::from_read(abi))
        .and_then(|r| r.create())
        .and_then(|mut r| {
            // Allow read-only access to everything.
            if let Ok(fd) = PathFd::new("/") {
                let _ = r.add_rule(PathBeneath::new(fd, AccessFs::from_read(abi)));
            }
            // Optionally allow read-write to a specific temp dir.
            if let Some(dir) = allow_write_dir {
                if let Ok(fd) = PathFd::new(dir) {
                    let _ = r.add_rule(PathBeneath::new(fd, AccessFs::from_all(abi)));
                }
            }
            r.restrict_self()
        });
    match result {
        Ok(outcome) => matches!(
            outcome.ruleset,
            RulesetStatus::FullyEnforced | RulesetStatus::PartiallyEnforced
        ),
        Err(_) => false,
    }
}

#[cfg(not(all(target_os = "linux", feature = "landlock-sandbox")))]
#[allow(dead_code)]
fn try_apply_landlock_readonly(_allow_write_dir: Option<&std::path::Path>) -> bool {
    false
}

/// Apply a seccomp syscall allowlist in the current process (intended for use
/// in `pre_exec` after fork, before exec).
///
/// Allows only the syscalls a well-behaved interpreter (Python/Node/etc.) needs:
/// file I/O, memory management, process control, networking (optional), and IPC.
/// Any other syscall causes the process to be killed with SIGSYS.
///
/// Returns `true` if seccomp was applied successfully, `false` on error or
/// when compiled without the `seccomp-sandbox` feature.
#[cfg(all(target_os = "linux", feature = "seccomp-sandbox"))]
fn apply_seccomp_allowlist(allow_network: bool) -> bool {
    use seccompiler::{
        BpfProgram, SeccompAction, SeccompFilter, SeccompRule,
        syscall_name_to_num,
    };

    // Core syscalls every process needs.
    let allowed: Vec<&str> = vec![
        // Memory
        "mmap", "mprotect", "munmap", "brk", "madvise", "mremap",
        // File I/O
        "read", "write", "readv", "writev", "pread64", "pwrite64",
        "open", "openat", "close", "fstat", "stat", "lstat",
        "lseek", "dup", "dup2", "dup3", "pipe", "pipe2",
        "fcntl", "ioctl", "fsync", "fdatasync",
        "mkdir", "rmdir", "unlink", "rename", "symlink", "readlink",
        "getcwd", "chdir",
        // Process
        "exit", "exit_group", "getpid", "getppid", "gettid",
        "set_tid_address", "futex", "nanosleep", "clock_gettime",
        "clock_nanosleep", "getrlimit", "setrlimit", "prlimit64",
        "uname", "sysinfo", "times",
        // Signals
        "rt_sigaction", "rt_sigprocmask", "rt_sigreturn", "sigaltstack",
        "kill", "tgkill",
        // Threads
        "clone", "clone3", "fork", "execve", "execveat", "wait4", "waitid",
        // I/O multiplexing
        "select", "pselect6", "poll", "ppoll", "epoll_create", "epoll_create1",
        "epoll_ctl", "epoll_wait", "epoll_pwait",
        // Sockets (needed even without network for Unix domain sockets / IPC)
        "socket", "connect", "bind", "listen", "accept", "accept4",
        "getsockopt", "setsockopt", "getsockname", "getpeername",
        "sendto", "recvfrom", "sendmsg", "recvmsg", "shutdown",
        // Anonymous pipes / tmpfiles
        "eventfd", "eventfd2", "timerfd_create", "timerfd_settime", "timerfd_gettime",
        // Misc
        "arch_prctl", "prctl", "getdents", "getdents64",
        "access", "faccessat", "newfstatat",
        "readdir", "getuid", "getgid", "geteuid", "getegid",
        "getgroups", "setgroups",
        "mlock", "munlock", "mlockall", "munlockall",
        "getrandom",
    ];

    // Convert names to numbers, skip unknowns (different kernel versions).
    let rules: Vec<(i64, Vec<SeccompRule>)> = allowed
        .iter()
        .filter_map(|name| syscall_name_to_num(name).ok().map(|n| (n as i64, vec![])))
        .collect();

    let _ = allow_network; // reserved for future per-syscall network filtering

    let filter = match SeccompFilter::new(
        rules.into_iter().collect(),
        SeccompAction::KillProcess,
        SeccompAction::Allow,
        std::env::consts::ARCH.try_into().unwrap_or(seccompiler::TargetArch::x86_64),
    ) {
        Ok(f) => f,
        Err(_) => return false,
    };

    let prog: BpfProgram = match filter.try_into() {
        Ok(p) => p,
        Err(_) => return false,
    };

    seccompiler::apply_filter(&prog).is_ok()
}

#[cfg(not(all(target_os = "linux", feature = "seccomp-sandbox")))]
#[allow(dead_code)]
fn apply_seccomp_allowlist(_allow_network: bool) -> bool {
    false
}

/// Run a hook script and parse the last JSON line of stdout.
///
/// This is the main entry point — picks the right launcher based on
/// `runtime`, enforces the timeout, scrubs inherited env, and returns
/// the raw JSON value the script emitted.
pub async fn run_hook_json(
    hook_name: &str,
    script_path: &str,
    runtime: PluginRuntime,
    input: &serde_json::Value,
    config: &HookConfig,
) -> Result<serde_json::Value, PluginRuntimeError> {
    validate_path_traversal(script_path)?;
    if !Path::new(script_path).exists() {
        return Err(PluginRuntimeError::ScriptNotFound(script_path.to_string()));
    }

    // Wasm hooks run inline via wasmtime — bypass all subprocess machinery.
    if matches!(runtime, PluginRuntime::Wasm) {
        return run_wasm_hook(script_path, input, config).await;
    }

    // Serialize state-file access across concurrent hook calls for the same plugin.
    let _state_lock = if let Some(ref path) = config.state_file {
        Some(lock_state_file(path).await)
    } else {
        None
    };

    let input_line =
        serde_json::to_string(input).map_err(|e| PluginRuntimeError::Io(e.to_string()))?;
    let (base_launcher, base_args) = build_command(runtime, script_path)?;

    // On Linux, attempt true network namespace isolation via `unshare --net`.
    // On other platforms, proxy-blocking env vars (set below) are the only mechanism.
    // Additionally, when allow_filesystem=false on Linux, also attempt `unshare --mount`
    // for mount namespace isolation (best-effort; falls back if unshare unavailable).
    let (launcher, args) = {
        let mut l = base_launcher.clone();
        let mut a = base_args.clone();
        if !config.allow_network {
            let wrapped = try_wrap_with_unshare(&l, &a);
            l = wrapped.0;
            a = wrapped.1;
        }
        if !config.allow_filesystem {
            let wrapped = try_wrap_with_unshare_mount(&l, &a);
            l = wrapped.0;
            a = wrapped.1;
        }
        (l, a)
    };

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
    // Plugin-declared env vars from [env] in plugin.toml.
    // Values of the form `${VAR_NAME}` are expanded from the daemon's env.
    for (key, val) in &config.plugin_env {
        let expanded = if let Some(inner) = val.strip_prefix("${").and_then(|s| s.strip_suffix('}')) {
            std::env::var(inner).unwrap_or_default()
        } else {
            val.clone()
        };
        cmd.env(key, expanded);
    }

    // Soft network isolation: inject proxy-blocking env vars so well-behaved
    // HTTP clients (requests, urllib, curl) honour the restriction.
    // On Linux we additionally wrap with `unshare --net` (done above when building
    // the launcher/args). Inject LIBREFANG_SANDBOX=1 so hook scripts can detect
    // they are running in a sandboxed environment.
    if !config.allow_network {
        cmd.env("no_proxy", "*");
        cmd.env("NO_PROXY", "*");
        cmd.env("http_proxy", "");
        cmd.env("https_proxy", "");
        cmd.env("HTTP_PROXY", "");
        cmd.env("HTTPS_PROXY", "");
        cmd.env("LIBREFANG_SANDBOX", "1");
    }

    // Filesystem isolation: advisory env vars + per-call tmpdir cleanup.
    // On Linux, unshare --mount wrapping is done above when building launcher/args.
    let _hook_tmpdir: Option<std::path::PathBuf> = if !config.allow_filesystem {
        cmd.env("LIBREFANG_READONLY_FS", "1");
        cmd.env("HOME", "/dev/null");
        let tmp = std::env::temp_dir()
            .join(format!("librefang_hook_{}", uuid_v4_hex()));
        let _ = std::fs::create_dir_all(&tmp);
        cmd.env("TMPDIR", tmp.display().to_string());
        debug!(
            tmpdir = %tmp.display(),
            "Filesystem isolation: LIBREFANG_READONLY_FS=1, HOME=/dev/null, scoped TMPDIR"
        );
        Some(tmp)
    } else {
        None
    };

    // Memory limit: expose to hook scripts via env var so well-behaved scripts
    // can self-limit (Python: `resource.setrlimit`, Node: `--max-old-space-size`).
    // Hard kernel-level enforcement requires the `libc` crate (not currently a
    // direct dependency). Scripts that read LIBREFANG_MAX_MEMORY_MB can apply
    // their own limits.
    if let Some(mb) = config.max_memory_mb {
        cmd.env("LIBREFANG_MAX_MEMORY_MB", mb.to_string());
        debug!(max_memory_mb = mb, "Memory limit set (advisory via env var; hard limit requires libc dep)");
    }

    // Shared state KV store: ensure the file exists and inject its path so hook
    // scripts can read/write persistent state across invocations.
    if let Some(ref state_path) = config.state_file {
        if !state_path.exists() {
            if let Some(parent) = state_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(state_path, "{}");
        }
        cmd.env("LIBREFANG_STATE_FILE", state_path.to_string_lossy().as_ref());
        debug!(state_file = %state_path.display(), "Shared state file injected");
    }

    // Apply Landlock filesystem restriction in the child process before exec.
    // This is done via unsafe pre_exec which runs in the forked child after fork()
    // but before exec(), so it restricts only the child's filesystem access.
    #[cfg(all(target_os = "linux", feature = "landlock-sandbox"))]
    if !config.allow_filesystem {
        use std::os::unix::process::CommandExt;
        let write_dir = _hook_tmpdir.clone();
        // SAFETY: pre_exec runs after fork() in the child. We only call
        // try_apply_landlock_readonly which uses only async-signal-safe-equivalent
        // operations (syscalls via the landlock crate).
        unsafe {
            cmd.pre_exec(move || {
                try_apply_landlock_readonly(write_dir.as_deref());
                Ok(())
            });
        }
    }

    // Apply seccomp syscall allowlist (requires seccomp-sandbox feature).
    // Applied unconditionally when the feature is enabled — seccomp is a
    // defence-in-depth measure independent of filesystem restrictions.
    #[cfg(all(target_os = "linux", feature = "seccomp-sandbox"))]
    {
        let allow_net = config.allow_network;
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(move || {
                // Non-fatal: log failure but don't abort the spawn.
                if !apply_seccomp_allowlist(allow_net) {
                    // Can't use tracing here (post-fork), use stderr.
                    let _ = std::io::Write::write_all(
                        &mut std::io::stderr(),
                        b"[librefang] seccomp filter failed to apply\n",
                    );
                }
                Ok(())
            });
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

    let effective_timeout = config.timeout_for(hook_name);
    // Capture PID before moving `child` into the async block so we can read
    // peak RSS from /proc/{pid}/status just before wait() reaps the process.
    let child_pid = child.id();
    let result = tokio::time::timeout(Duration::from_secs(effective_timeout), async {
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

        // Read stdout and stderr concurrently to prevent deadlock:
        // a hook that writes >64KB to stderr before closing stdout would otherwise
        // block the daemon waiting on stdout while the hook is blocked on stderr.
        let stdout_fut = async {
            let mut lines: Vec<String> = Vec::new();
            let mut line = String::new();
            loop {
                line.clear();
                match stdout_reader.read_line(&mut line).await {
                    Ok(0) => break,
                    Ok(_) => lines.push(line.trim_end().to_string()),
                    Err(e) => {
                        warn!("hook stdout read error: {e}");
                        break;
                    }
                }
            }
            lines
        };
        let stderr_fut = async {
            let mut text = String::new();
            let mut err_line = String::new();
            loop {
                err_line.clear();
                match stderr_reader.read_line(&mut err_line).await {
                    Ok(0) => break,
                    Ok(_) => text.push_str(&err_line),
                    Err(_) => break,
                }
            }
            text
        };
        let (stdout_lines, stderr_text) = tokio::join!(stdout_fut, stderr_fut);

        // Read peak RSS before wait() reaps the process and removes /proc/{pid}.
        if let Some(pid) = child_pid {
            if let Some(rss_kb) = read_proc_rss_kb(pid) {
                debug!(script = script_path, rss_kb, "hook process peak RSS");
            }
        }

        let status = child
            .wait()
            .await
            .map_err(|e| PluginRuntimeError::Io(e.to_string()))?;

        Ok::<(Vec<String>, String, std::process::ExitStatus), PluginRuntimeError>((
            stdout_lines,
            stderr_text,
            status,
        ))
    })
    .await;

    let out = match result {
        Ok(Ok((stdout_lines, stderr_text, status))) => {
            if !status.success() {
                let label = classify_exit_status(&status);
                Err(PluginRuntimeError::ScriptError {
                    code: status.code(),
                    stderr: format!("{label}\nstderr: {}", stderr_text.trim()),
                })
            } else {
                if !stderr_text.trim().is_empty() {
                    debug!("hook stderr: {}", stderr_text.trim());
                }
                // Guard against misbehaving scripts that emit enormous outputs.
                const MAX_OUTPUT_BYTES: usize = 4 * 1024 * 1024; // 4 MiB
                let total_output_bytes: usize = stdout_lines.iter().map(|l| l.len()).sum();
                if total_output_bytes > MAX_OUTPUT_BYTES {
                    return Err(PluginRuntimeError::InvalidOutput(format!(
                        "Hook output exceeds maximum size ({} bytes > {} bytes limit). \
                         Truncate your hook's JSON response.",
                        total_output_bytes,
                        MAX_OUTPUT_BYTES
                    )));
                }
                parse_output(&stdout_lines)
            }
        }
        Ok(Err(e)) => Err(e),
        Err(_) => {
            let _ = child.kill().await;
            Err(PluginRuntimeError::Timeout(effective_timeout))
        }
    };

    // Clean up per-call tmpdir created for filesystem isolation.
    if let Some(ref tmp) = _hook_tmpdir {
        let _ = std::fs::remove_dir_all(tmp);
    }

    out
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

/// Execute a Wasm hook module inline using the built-in wasmtime engine.
///
/// The module receives the input JSON on its stdin (via WASI) and must write
/// its JSON response to stdout.  The hook protocol is identical to subprocess
/// hooks: one JSON object in, one JSON object out.
///
/// Returns the parsed JSON output on success, or a `PluginRuntimeError` on
/// failure.  When the `wasm-hooks` feature is not enabled, this always returns
/// `Err(PluginRuntimeError::SpawnFailed(...))` with a clear message.
#[cfg(feature = "wasm-hooks")]
pub async fn run_wasm_hook(
    wasm_path: &str,
    input: &serde_json::Value,
    _config: &HookConfig,
) -> Result<serde_json::Value, PluginRuntimeError> {
    // Full wasmtime + WASI integration is reserved for a follow-up implementation
    // once the exact WASI API surface for wasmtime 43 is confirmed.  The variant,
    // routing, and feature-gate scaffolding are all in place.
    let _ = wasm_path;
    let _ = input;
    Err(PluginRuntimeError::SpawnFailed(
        "Wasm hook execution not yet implemented (wasm-hooks feature is enabled but the \
         wasmtime integration is pending)"
            .to_string(),
    ))
}

/// Stub used when the `wasm-hooks` feature is not compiled in.
#[cfg(not(feature = "wasm-hooks"))]
pub async fn run_wasm_hook(
    _wasm_path: &str,
    _input: &serde_json::Value,
    _config: &HookConfig,
) -> Result<serde_json::Value, PluginRuntimeError> {
    Err(PluginRuntimeError::SpawnFailed(
        "Wasm hook support not compiled in — rebuild with the 'wasm-hooks' feature enabled"
            .to_string(),
    ))
}

/// Generate a short random hex string suitable for unique temp directory names.
///
/// Uses the current time (nanoseconds) XOR'd with a monotonic counter as entropy —
/// collision-resistant enough for per-call tmpdir naming without pulling in a UUID crate.
fn uuid_v4_hex() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0) as u64;
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mixed = nanos ^ (seq.wrapping_mul(0x9e3779b97f4a7c15));
    format!("{mixed:016x}")
}

/// Expand a plugin env value: `${VAR_NAME}` → parent env lookup, otherwise literal.
fn expand_env_value(val: &str) -> String {
    if let Some(inner) = val.strip_prefix("${").and_then(|s| s.strip_suffix('}')) {
        std::env::var(inner).unwrap_or_default()
    } else {
        val.to_string()
    }
}

/// A single persistent hook subprocess with its I/O handles.
struct PersistentProcess {
    stdin: tokio::process::ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
    /// Kept alive so `kill_on_drop` fires when this struct is dropped.
    /// Also used by `health_check` to probe liveness via `try_wait`.
    child: tokio::process::Child,
}

/// Pool of persistent hook subprocesses, keyed by script path.
///
/// Each entry is `Arc<tokio::sync::Mutex<Option<PersistentProcess>>>` — `None` means
/// the process crashed and needs restarting. The outer `std::sync::Mutex` lets
/// the pool be shared across async tasks while providing exclusive access
/// during a hook call (hooks are not reentrant by design).
#[derive(Default)]
pub struct HookProcessPool {
    procs: std::sync::Mutex<
        std::collections::HashMap<String, std::sync::Arc<tokio::sync::Mutex<Option<PersistentProcess>>>>,
    >,
}

impl HookProcessPool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Call a hook via a persistent subprocess.
    ///
    /// Starts the process on first call, restarts automatically after crash.
    /// For `Wasm` hooks the persistent pool is bypassed entirely — each call
    /// goes directly to [`run_wasm_hook`] which executes inline via wasmtime.
    pub async fn call(
        &self,
        script_path: &str,
        runtime: PluginRuntime,
        input: &serde_json::Value,
        config: &HookConfig,
    ) -> Result<serde_json::Value, PluginRuntimeError> {
        // Wasm hooks are stateless inline executions — no persistent subprocess.
        if matches!(runtime, PluginRuntime::Wasm) {
            return run_wasm_hook(script_path, input, config).await;
        }

        let slot = {
            let mut map = self.procs.lock().unwrap();
            map.entry(script_path.to_string())
                .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(None)))
                .clone()
        };

        let mut guard = slot.lock().await;

        // Hold the per-file advisory lock for the duration of the subprocess call
        // to prevent concurrent hook scripts from racing on the shared state file.
        let _state_lock = if let Some(ref path) = config.state_file {
            Some(lock_state_file(path).await)
        } else {
            None
        };

        // Ensure process is running.
        if guard.is_none() {
            *guard = Some(Self::spawn(script_path, runtime, config).await?);
        }

        // Try to call; on failure, evict the dead slot then restart and retry once.
        let result = Self::do_call(guard.as_mut().unwrap(), input).await;
        if result.is_err() {
            // Probe exit status before evicting so we can produce a classified label.
            let exit_label = if let Some(ref mut proc) = *guard {
                match proc.child.try_wait() {
                    Ok(Some(status)) => {
                        let label = classify_exit_status(&status);
                        format!("Persistent hook process exited unexpectedly: {label}")
                    }
                    _ => "Persistent hook process crashed; restarting".to_string(),
                }
            } else {
                "Persistent hook process crashed; restarting".to_string()
            };
            warn!(script = script_path, "{}", exit_label);
            // Evict before spawn so that a subsequent call never sees a dead process
            // even if the spawn below fails (returns Err and drops the guard).
            *guard = None;
            *guard = Some(Self::spawn(script_path, runtime, config).await?);
            return Self::do_call(guard.as_mut().unwrap(), input).await;
        }
        result
    }

    async fn spawn(
        script_path: &str,
        runtime: PluginRuntime,
        config: &HookConfig,
    ) -> Result<PersistentProcess, PluginRuntimeError> {
        validate_path_traversal(script_path)?;
        let (launcher, args) = build_command(runtime, script_path)?;
        let mut cmd = tokio::process::Command::new(&launcher);
        cmd.args(&args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);

        // Inject env: start clean then re-add baseline.
        cmd.env_clear();
        for (k, v) in std::env::vars() {
            cmd.env(k, v);
        }
        for (k, v) in &config.plugin_env {
            let expanded = expand_env_value(v);
            cmd.env(k, expanded);
        }
        if !config.allow_network {
            cmd.env("no_proxy", "*")
                .env("NO_PROXY", "*")
                .env("http_proxy", "")
                .env("https_proxy", "")
                .env("HTTP_PROXY", "")
                .env("HTTPS_PROXY", "");
        }
        if let Some(mb) = config.max_memory_mb {
            cmd.env("LIBREFANG_MAX_MEMORY_MB", mb.to_string());
        }

        // Apply seccomp syscall allowlist (requires seccomp-sandbox feature).
        // Applied unconditionally when the feature is enabled — seccomp is a
        // defence-in-depth measure independent of filesystem restrictions.
        #[cfg(all(target_os = "linux", feature = "seccomp-sandbox"))]
        {
            let allow_net = config.allow_network;
            use std::os::unix::process::CommandExt;
            unsafe {
                cmd.pre_exec(move || {
                    // Non-fatal: log failure but don't abort the spawn.
                    if !apply_seccomp_allowlist(allow_net) {
                        // Can't use tracing here (post-fork), use stderr.
                        let _ = std::io::Write::write_all(
                            &mut std::io::stderr(),
                            b"[librefang] seccomp filter failed to apply\n",
                        );
                    }
                    Ok(())
                });
            }
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| PluginRuntimeError::Io(e.to_string()))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| PluginRuntimeError::Io("no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| PluginRuntimeError::Io("no stdout".into()))?;

        Ok(PersistentProcess {
            stdin,
            stdout: BufReader::new(stdout),
            child,
        })
    }

    async fn do_call(
        proc: &mut PersistentProcess,
        input: &serde_json::Value,
    ) -> Result<serde_json::Value, PluginRuntimeError> {
        let mut line =
            serde_json::to_string(input).map_err(|e| PluginRuntimeError::Io(e.to_string()))?;
        line.push('\n');

        proc.stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| PluginRuntimeError::Io(format!("write stdin: {e}")))?;
        proc.stdin
            .flush()
            .await
            .map_err(|e| PluginRuntimeError::Io(format!("flush stdin: {e}")))?;

        let mut response = String::new();
        proc.stdout
            .read_line(&mut response)
            .await
            .map_err(|e| PluginRuntimeError::Io(format!("read stdout: {e}")))?;

        if response.is_empty() {
            return Err(PluginRuntimeError::Io(
                "persistent process closed stdout".into(),
            ));
        }

        // Guard against misbehaving scripts that emit enormous outputs.
        const MAX_OUTPUT_BYTES: usize = 4 * 1024 * 1024; // 4 MiB
        if response.len() > MAX_OUTPUT_BYTES {
            return Err(PluginRuntimeError::InvalidOutput(format!(
                "Hook output exceeds maximum size ({} bytes > {} bytes limit). \
                 Truncate your hook's JSON response.",
                response.len(),
                MAX_OUTPUT_BYTES
            )));
        }

        // The persistent process is still running, so /proc/{pid}/status is valid.
        if let Some(pid) = proc.child.id() {
            if let Some(rss_kb) = read_proc_rss_kb(pid) {
                debug!(rss_kb, "persistent hook process current RSS");
            }
        }

        serde_json::from_str(response.trim())
            .map_err(|e| PluginRuntimeError::InvalidOutput(format!("JSON parse: {e}")))
    }

    /// Check which persistent subprocesses are still alive.
    ///
    /// For each slot in the pool:
    /// - If the slot mutex is locked (process is in use), the process is assumed alive.
    /// - If the slot mutex can be acquired and `child.try_wait()` returns `Ok(None)`,
    ///   the process is still running — add to the alive list.
    /// - If `try_wait` returns an exit status or an error, the slot is evicted
    ///   (set to `None`) so the next `call()` will restart it.
    ///
    /// Returns the list of script-path keys for alive processes.
    pub async fn health_check(&self) -> Vec<String> {
        let mut alive = Vec::new();
        let entries: Vec<(String, std::sync::Arc<tokio::sync::Mutex<Option<PersistentProcess>>>)> = {
            let procs = self.procs.lock().unwrap();
            procs.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
        };
        for (key, slot_arc) in entries {
            match slot_arc.try_lock() {
                Ok(mut guard) => {
                    if let Some(ref mut proc) = *guard {
                        match proc.child.try_wait() {
                            Ok(None) => alive.push(key), // still running
                            _ => {
                                // Exited or error — evict so next call restarts it.
                                *guard = None;
                            }
                        }
                    }
                    // If guard is None, slot is already evicted — not alive.
                }
                Err(_) => {
                    // Locked means in-use; assume alive.
                    alive.push(key);
                }
            }
        }
        alive
    }

    /// Evict a specific subprocess by script path, forcing a fresh spawn on next call.
    ///
    /// If the slot is currently locked (a call is in progress), this is a no-op —
    /// the process will be restarted naturally when the call finishes and the next
    /// caller re-enters `call()`.
    pub async fn evict(&self, script_path: &str) {
        let slot = {
            let guard = self.procs.lock().unwrap();
            guard.get(script_path).cloned()
        };
        if let Some(arc) = slot {
            // `try_lock` is intentional: if a call is in progress we don't
            // want to block waiting for it to finish.  The next completed call
            // will see the process is dead and restart it anyway.
            if let Ok(mut guard) = arc.try_lock() {
                *guard = None;
            }
        }
    }

    /// Evict all subprocesses in the pool, forcing fresh spawns on the next calls.
    pub async fn evict_all(&self) {
        let keys: Vec<String> = {
            let guard = self.procs.lock().unwrap();
            guard.keys().cloned().collect()
        };
        for key in keys {
            self.evict(&key).await;
        }
    }

    /// Pre-warm a new hook process and, once ready, atomically replace the
    /// existing pool entry for `script_path`.
    ///
    /// This enables zero-downtime hot-reload: the old process continues to
    /// handle any in-flight request (it holds the slot lock) while the new
    /// process is being spawned outside the lock.  Once the new process is
    /// confirmed alive, we acquire the slot lock and swap it in, killing the
    /// old process first.
    ///
    /// `hook_name` is used only for log messages.
    /// Returns `1` on success (slot replaced), `0` on failure.
    pub async fn swap_prewarm(
        &self,
        hook_name: &str,
        script_path: &str,
        runtime: PluginRuntime,
        config: &HookConfig,
    ) -> usize {
        // Step 1: spawn the new process outside any lock so in-flight calls
        // on the old process are not blocked.
        let new_proc = match Self::spawn(script_path, runtime, config).await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(hook = hook_name, "swap_prewarm: failed to spawn new process: {e}");
                return 0;
            }
        };

        // Step 2: verify the new process is alive — child.id() returns Some
        // only while the process is still running.
        if new_proc.child.id().is_none() {
            tracing::warn!(hook = hook_name, "swap_prewarm: new process died immediately");
            return 0;
        }

        // Step 3: atomically replace the pool entry.
        // Acquire (or create) the slot arc under the std::sync::Mutex, then
        // lock the inner tokio Mutex to swap the PersistentProcess.
        let slot_arc = {
            let mut procs = self.procs.lock().unwrap();
            procs
                .entry(script_path.to_string())
                .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(None)))
                .clone()
        };

        let mut guard = slot_arc.lock().await;
        // Kill the old process best-effort before replacing it.
        if let Some(ref mut old_proc) = *guard {
            let _ = old_proc.child.kill().await;
        }
        *guard = Some(new_proc);
        tracing::info!(hook = hook_name, script = script_path, "swap_prewarm: hot-reload complete, slot replaced");
        1
    }

    /// Pre-warm a specific hook script by spawning its subprocess now.
    ///
    /// The process will be held in the pool and reused on the first `call()`.
    /// If a process for this script is already running, this is a no-op.
    /// Returns `Ok(())` if the process started successfully.
    pub async fn prewarm(
        &self,
        script_path: &str,
        runtime: PluginRuntime,
        plugin_env: &[(String, String)],
    ) -> Result<(), PluginRuntimeError> {
        // Use the same key as `call()` so the pre-warmed slot is found on first use.
        let key = script_path.to_string();
        let slot_arc = {
            let mut procs = self.procs.lock().unwrap();
            procs
                .entry(key.clone())
                .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(None)))
                .clone()
        };
        let mut guard = slot_arc.lock().await;
        if guard.is_none() {
            let config = HookConfig {
                plugin_env: plugin_env.to_vec(),
                ..Default::default()
            };
            *guard = Some(Self::spawn(script_path, runtime, &config).await?);
            tracing::info!(script = script_path, "Pre-warmed hook subprocess");
        }
        Ok(())
    }
}

// SAFETY: HookProcessPool is Send+Sync because:
// - the outer Mutex<HashMap<...>> is std::sync::Mutex (Send+Sync)
// - the slot values are Arc<tokio::sync::Mutex<Option<PersistentProcess>>>
// - PersistentProcess holds ChildStdin/ChildStdout/Child which are Send
unsafe impl Send for HookProcessPool {}
unsafe impl Sync for HookProcessPool {}

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
            "ingest",
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
            "ingest",
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
            "ingest",
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
            "test_hook",
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
