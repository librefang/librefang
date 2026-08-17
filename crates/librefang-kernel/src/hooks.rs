//! External Event Hook System — file-system-based lifecycle hooks.
//!
//! Scans `~/.librefang/hooks/` (or `$LIBREFANG_HOME/hooks/`) for hook directories.
//! Each directory must contain a `HOOK.yaml` metadata file plus a `command` field
//! specifying the executable to run when the event fires.
//!
//! ## HOOK.yaml format
//! ```yaml
//! name: my-hook
//! description: "Notify Slack on agent completion"
//! events:
//!   - agent:end
//!   - session:start
//! command: /home/user/my hooks/run.sh
//! args:
//!   - --message
//!   - "agent finished"
//! ```
//!
//! ## Wildcard matching
//! `agent:*` matches `agent:start`, `agent:end`, `agent:step`.
//! `session:*` matches all session events. `*` alone matches everything.
//!
//! ## Event data
//! The event payload is serialised as JSON and passed via the `HOOK_EVENT_DATA`
//! environment variable. The event name is also set as `HOOK_EVENT`.
//!
//! ## Failure semantics
//! Hook failures are logged at `WARN` level and **never** propagate to the
//! caller. Hook execution is fire-and-forget via `tokio::spawn`.

use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;
use tracing::{debug, error, warn};

/// Limits the number of hook tasks that may execute concurrently.
///
/// `agent:step` fires on every tool-loop iteration and can produce a large
/// number of concurrent spawns when several agents run simultaneously.  Eight
/// concurrent executions is enough for normal workloads while preventing
/// unbounded task accumulation.
static HOOK_CONCURRENCY: std::sync::LazyLock<tokio::sync::Semaphore> =
    std::sync::LazyLock::new(|| tokio::sync::Semaphore::new(8));

/// Acquire a concurrency permit for a hook, or refuse to run on `SemaphoreClosed`.
///
/// Returning `None` is a hard refusal that causes the caller to skip executing
/// the hook entirely, so the documented system-wide cap on external hook
/// `fork()` rate stays a hard guarantee — even in the (currently impossible)
/// case where the static `LazyLock` semaphore is closed.
async fn acquire_hook_permit<'a>(
    sem: &'a tokio::sync::Semaphore,
    hook_name: &str,
    ev: &str,
) -> Option<tokio::sync::SemaphorePermit<'a>> {
    match sem.acquire().await {
        Ok(permit) => Some(permit),
        Err(_closed) => {
            error!(
                hook_name = %hook_name,
                event = %ev,
                "HOOK_CONCURRENCY semaphore closed; refusing to run hook"
            );
            None
        }
    }
}

/// Lifecycle events that external hooks can subscribe to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ExternalHookEvent {
    /// Agent begins processing a message.
    AgentStart,
    /// Agent finishes processing (success or error).
    AgentEnd,
    /// Each turn in the tool-calling loop.
    AgentStep,
    /// A new session is created (first message in a fresh session).
    SessionStart,
    /// A session ends (reset / new-session command).
    SessionEnd,
    /// Session has been fully reset and a new one created.
    SessionReset,
    /// The LibreFang daemon finishes startup.
    GatewayStartup,
}

impl ExternalHookEvent {
    /// Canonical string representation used in HOOK.yaml event patterns.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AgentStart => "agent:start",
            Self::AgentEnd => "agent:end",
            Self::AgentStep => "agent:step",
            Self::SessionStart => "session:start",
            Self::SessionEnd => "session:end",
            Self::SessionReset => "session:reset",
            Self::GatewayStartup => "gateway:startup",
        }
    }

    /// Returns true if `pattern` matches this event.
    ///
    /// Supported patterns:
    /// - Exact: `"agent:start"` matches only `AgentStart`.
    /// - Wildcard suffix: `"agent:*"` matches all `agent:…` events.
    /// - Global wildcard: `"*"` matches every event.
    pub fn matches_pattern(&self, pattern: &str) -> bool {
        if pattern == "*" {
            return true;
        }
        let event_str = self.as_str();
        if pattern == event_str {
            return true;
        }
        // Check "prefix:*" wildcard
        if let Some(prefix) = pattern.strip_suffix(":*") {
            if let Some(event_prefix) = event_str.split(':').next() {
                return event_prefix == prefix;
            }
        }
        false
    }
}

/// Parsed metadata from a `HOOK.yaml` file.
#[derive(Debug, Clone, Deserialize)]
pub struct HookManifest {
    /// Human-readable hook name (used in log messages).
    pub name: String,
    /// Event patterns this hook subscribes to. Supports wildcards.
    pub events: Vec<String>,
    /// Executable to invoke when the hook fires.
    ///
    /// For backward compatibility, this is split on whitespace when `args` is
    /// omitted. Set `args` (including an empty list) to treat this value as an
    /// exact executable path, including any spaces.
    pub command: String,
    /// Optional argument vector passed verbatim to the executable.
    ///
    /// Prefer this over embedding arguments in `command`; individual entries
    /// may contain whitespace without quoting or shell parsing.
    #[serde(default)]
    pub args: Option<Vec<String>>,
    /// Optional description (informational only).
    #[serde(default)]
    pub description: String,
}

/// Discovered hook: parsed manifest plus the directory it lives in.
#[derive(Debug, Clone)]
struct LoadedHook {
    manifest: HookManifest,
    /// Directory containing `HOOK.yaml`. Used as the working directory when
    /// spawning the hook process so that relative commands (e.g. `./run.sh`)
    /// resolve correctly.
    dir: PathBuf,
}

/// Registry of externally-configured lifecycle hooks.
///
/// Loaded once at startup via [`ExternalHookSystem::load`] and then used
/// read-only. All firing is fire-and-forget — hook failures never propagate.
#[derive(Debug, Default)]
pub struct ExternalHookSystem {
    hooks: Vec<LoadedHook>,
}

impl ExternalHookSystem {
    /// Scan `hooks_dir` for subdirectories containing `HOOK.yaml` and load them.
    ///
    /// Directories without `HOOK.yaml`, or with unparseable manifests, are
    /// skipped with a `WARN` log. The returned registry is always valid (may
    /// be empty if nothing is found).
    pub fn load(hooks_dir: PathBuf) -> Self {
        let mut hooks = Vec::new();

        if !hooks_dir.exists() {
            debug!(
                path = %hooks_dir.display(),
                "External hooks directory does not exist, skipping"
            );
            return Self { hooks };
        }

        let entries = match std::fs::read_dir(&hooks_dir) {
            Ok(e) => e,
            Err(err) => {
                warn!(
                    path = %hooks_dir.display(),
                    error = %err,
                    "Failed to read hooks directory"
                );
                return Self { hooks };
            }
        };

        let mut dirs: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort(); // deterministic load order

        for dir in dirs {
            let manifest_path = dir.join("HOOK.yaml");
            if !manifest_path.exists() {
                continue;
            }

            let yaml_text = match std::fs::read_to_string(&manifest_path) {
                Ok(t) => t,
                Err(err) => {
                    warn!(
                        path = %manifest_path.display(),
                        error = %err,
                        "Failed to read HOOK.yaml"
                    );
                    continue;
                }
            };

            let manifest: HookManifest = match serde_yaml::from_str(&yaml_text) {
                Ok(m) => m,
                Err(err) => {
                    warn!(
                        path = %manifest_path.display(),
                        error = %err,
                        "Failed to parse HOOK.yaml — skipping"
                    );
                    continue;
                }
            };

            if manifest.events.is_empty() {
                warn!(
                    hook = %manifest.name,
                    path = %manifest_path.display(),
                    "HOOK.yaml declares no events — skipping"
                );
                continue;
            }

            if manifest.command.is_empty() {
                warn!(
                    hook = %manifest.name,
                    path = %manifest_path.display(),
                    "HOOK.yaml has empty command — skipping"
                );
                continue;
            }

            tracing::info!(
                hook = %manifest.name,
                events = ?manifest.events,
                command = %manifest.command,
                "Loaded external hook"
            );

            hooks.push(LoadedHook { manifest, dir });
        }

        Self { hooks }
    }

    /// Fire all hooks whose event patterns match `event`.
    ///
    /// Each matching hook is spawned as a separate `tokio` task (fire-and-forget).
    /// The JSON `data` payload is serialised and passed via `HOOK_EVENT_DATA`.
    /// Errors are logged at `WARN` level and never propagated to the caller.
    ///
    /// If no Tokio runtime is active (i.e., called from a sync kernel API), the
    /// hook is run inline on the current thread rather than spawning, so the
    /// caller does not panic.
    pub fn fire(&self, event: ExternalHookEvent, data: Value) {
        let event_str = event.as_str().to_owned();
        let data_str = data.to_string();

        for loaded in &self.hooks {
            let matches = loaded
                .manifest
                .events
                .iter()
                .any(|p| event.matches_pattern(p));

            if !matches {
                continue;
            }

            let command = loaded.manifest.command.clone();
            let command_args = loaded.manifest.args.clone();
            let hook_name = loaded.manifest.name.clone();
            let hook_dir = loaded.dir.clone();
            let ev = event_str.clone();
            let payload = data_str.clone();

            // Spawn if a Tokio runtime is active; otherwise spin up a background
            // thread so the caller is never blocked (fire-and-forget in both paths).
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    Self::run_hook(
                        &hook_name,
                        &ev,
                        &command,
                        command_args.as_deref(),
                        &hook_dir,
                        &payload,
                    )
                    .await;
                });
            } else {
                // No runtime active (e.g. called from a sync kernel API).
                // Spawn a background thread so we never block the caller and
                // never panic due to a missing runtime.
                std::thread::spawn(move || {
                    if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    {
                        rt.block_on(Self::run_hook(
                            &hook_name,
                            &ev,
                            &command,
                            command_args.as_deref(),
                            &hook_dir,
                            &payload,
                        ));
                    } else {
                        warn!(
                            hook = %hook_name,
                            event = %ev,
                            "Failed to create runtime for external hook"
                        );
                    }
                });
            }
        }
    }

    async fn run_hook(
        hook_name: &str,
        ev: &str,
        command: &str,
        command_args: Option<&[String]>,
        dir: &std::path::Path,
        payload: &str,
    ) {
        // Acquire a concurrency permit before doing any work.  This caps the
        // total number of simultaneously-running hook processes system-wide and
        // prevents unbounded task accumulation on high-frequency events such as
        // `agent:step`.  The permit is held for the duration of the process
        // execution and released automatically when this function returns.
        //
        // The static `LazyLock` semaphore is never closed in practice, but if
        // it ever is, we refuse to run the hook rather than silently bypassing
        // the system-wide fork() rate cap.  See `acquire_hook_permit` above.
        let Some(_permit) = acquire_hook_permit(&HOOK_CONCURRENCY, hook_name, ev).await else {
            return;
        };

        debug!(
            hook = %hook_name,
            event = %ev,
            "Firing external hook"
        );

        let (binary, args) = resolve_hook_command(command, command_args);

        let result = tokio::time::timeout(
            Duration::from_secs(30),
            tokio::process::Command::new(&binary)
                .args(&args)
                .current_dir(dir)
                .env("HOOK_EVENT", ev)
                .env("HOOK_EVENT_DATA", payload)
                .kill_on_drop(true)
                .output(),
        )
        .await;

        match result {
            Ok(Ok(output)) => {
                if !output.status.success() {
                    warn!(
                        hook = %hook_name,
                        event = %ev,
                        status = %output.status,
                        stderr = %String::from_utf8_lossy(&output.stderr),
                        "External hook exited with non-zero status"
                    );
                } else {
                    debug!(
                        hook = %hook_name,
                        event = %ev,
                        "External hook completed successfully"
                    );
                }
            }
            Ok(Err(err)) => {
                warn!(
                    hook = %hook_name,
                    event = %ev,
                    error = %err,
                    "Failed to spawn external hook process"
                );
            }
            Err(_) => {
                warn!(
                    hook = %hook_name,
                    event = %ev,
                    "External hook timed out after 30s"
                );
            }
        }
    }

    /// Returns the number of loaded hooks.
    pub fn len(&self) -> usize {
        self.hooks.len()
    }

    /// Returns true if no hooks are loaded.
    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }
}

/// Resolve a hook executable and its argument vector.
///
/// An explicit `args` field is an opt-in to lossless process arguments: the
/// executable path and every argument are passed through verbatim. Manifests
/// without `args` retain the historical whitespace splitting behavior.
fn resolve_hook_command(command: &str, args: Option<&[String]>) -> (String, Vec<String>) {
    if let Some(args) = args {
        return (command.to_owned(), args.to_vec());
    }

    let mut parts = command.split_whitespace();
    let binary = parts.next().unwrap_or(command).to_owned();
    let args = parts.map(str::to_owned).collect();
    (binary, args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_hook_args_preserve_spaces() {
        let args = vec!["--message".to_string(), "hello world".to_string()];
        let (binary, resolved_args) =
            resolve_hook_command("/home/user/my hooks/run.sh", Some(&args));

        assert_eq!(binary, "/home/user/my hooks/run.sh");
        assert_eq!(resolved_args, args);
    }

    #[test]
    fn omitted_hook_args_preserve_legacy_whitespace_splitting() {
        let (binary, args) = resolve_hook_command("/usr/bin/python3 script.py --quiet", None);

        assert_eq!(binary, "/usr/bin/python3");
        assert_eq!(args, ["script.py", "--quiet"]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn explicit_hook_args_execute_paths_and_arguments_with_spaces() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("hook with spaces.sh");
        let output = dir.path().join("hook output.txt");
        std::fs::write(
            &script,
            "#!/bin/sh\nprintf '%s' \"$1\" > \"hook output.txt\"\n",
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&script, permissions).unwrap();

        let args = vec!["hello world".to_string()];
        ExternalHookSystem::run_hook(
            "space-hook",
            "agent:end",
            script.to_str().unwrap(),
            Some(&args),
            dir.path(),
            "{}",
        )
        .await;

        assert_eq!(std::fs::read_to_string(output).unwrap(), "hello world");
    }

    #[test]
    fn test_exact_match() {
        assert!(ExternalHookEvent::AgentStart.matches_pattern("agent:start"));
        assert!(!ExternalHookEvent::AgentStart.matches_pattern("agent:end"));
        assert!(!ExternalHookEvent::AgentStart.matches_pattern("agent:step"));
    }

    #[test]
    fn test_wildcard_prefix() {
        assert!(ExternalHookEvent::AgentStart.matches_pattern("agent:*"));
        assert!(ExternalHookEvent::AgentEnd.matches_pattern("agent:*"));
        assert!(ExternalHookEvent::AgentStep.matches_pattern("agent:*"));
        assert!(!ExternalHookEvent::SessionStart.matches_pattern("agent:*"));
        assert!(!ExternalHookEvent::GatewayStartup.matches_pattern("agent:*"));
    }

    #[test]
    fn test_global_wildcard() {
        for event in [
            ExternalHookEvent::AgentStart,
            ExternalHookEvent::AgentEnd,
            ExternalHookEvent::AgentStep,
            ExternalHookEvent::SessionStart,
            ExternalHookEvent::SessionEnd,
            ExternalHookEvent::SessionReset,
            ExternalHookEvent::GatewayStartup,
        ] {
            assert!(
                event.matches_pattern("*"),
                "{} should match *",
                event.as_str()
            );
        }
    }

    #[test]
    fn test_session_wildcard() {
        assert!(ExternalHookEvent::SessionStart.matches_pattern("session:*"));
        assert!(ExternalHookEvent::SessionEnd.matches_pattern("session:*"));
        assert!(ExternalHookEvent::SessionReset.matches_pattern("session:*"));
        assert!(!ExternalHookEvent::AgentStart.matches_pattern("session:*"));
    }

    #[test]
    fn test_no_match_wrong_prefix() {
        assert!(!ExternalHookEvent::GatewayStartup.matches_pattern("agent:*"));
        assert!(!ExternalHookEvent::GatewayStartup.matches_pattern("session:*"));
    }

    #[test]
    fn test_load_missing_dir_returns_empty() {
        let system =
            ExternalHookSystem::load(PathBuf::from("/nonexistent/hooks/dir/that/does/not/exist"));
        assert!(system.is_empty());
    }

    #[test]
    fn test_load_empty_dir_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let system = ExternalHookSystem::load(dir.path().to_path_buf());
        assert!(system.is_empty());
    }

    #[test]
    fn test_load_valid_hook() {
        let dir = tempfile::tempdir().unwrap();
        let hook_dir = dir.path().join("my-hook");
        std::fs::create_dir(&hook_dir).unwrap();

        let yaml = r#"
name: my-hook
description: "Test hook"
events:
  - agent:start
  - agent:end
command: /usr/bin/env
args:
  - --message
  - "hello world"
"#;
        std::fs::write(hook_dir.join("HOOK.yaml"), yaml).unwrap();

        let system = ExternalHookSystem::load(dir.path().to_path_buf());
        assert_eq!(system.len(), 1);
        assert_eq!(
            system.hooks[0].manifest.args.as_deref(),
            Some(["--message".to_string(), "hello world".to_string()].as_slice())
        );
    }

    #[test]
    fn test_load_skips_missing_events() {
        let dir = tempfile::tempdir().unwrap();
        let hook_dir = dir.path().join("bad-hook");
        std::fs::create_dir(&hook_dir).unwrap();

        let yaml = r#"
name: bad-hook
events: []
command: /usr/bin/env
"#;
        std::fs::write(hook_dir.join("HOOK.yaml"), yaml).unwrap();

        let system = ExternalHookSystem::load(dir.path().to_path_buf());
        assert!(system.is_empty());
    }

    #[test]
    fn test_load_skips_invalid_yaml() {
        let dir = tempfile::tempdir().unwrap();
        let hook_dir = dir.path().join("corrupt-hook");
        std::fs::create_dir(&hook_dir).unwrap();
        std::fs::write(hook_dir.join("HOOK.yaml"), b"[[[invalid yaml").unwrap();

        let system = ExternalHookSystem::load(dir.path().to_path_buf());
        assert!(system.is_empty());
    }

    #[test]
    fn test_load_multiple_hooks_sorted() {
        let dir = tempfile::tempdir().unwrap();

        for (name, event) in [("z-hook", "agent:end"), ("a-hook", "agent:start")] {
            let hook_dir = dir.path().join(name);
            std::fs::create_dir(&hook_dir).unwrap();
            let yaml = format!("name: {name}\nevents:\n  - {event}\ncommand: /usr/bin/env\n");
            std::fs::write(hook_dir.join("HOOK.yaml"), yaml).unwrap();
        }

        let system = ExternalHookSystem::load(dir.path().to_path_buf());
        assert_eq!(system.len(), 2);
        // Verify load order is alphabetical (a-hook before z-hook)
        assert_eq!(system.hooks[0].manifest.name, "a-hook");
        assert_eq!(system.hooks[1].manifest.name, "z-hook");
    }

    #[test]
    fn test_fire_exact_match() {
        let dir = tempfile::tempdir().unwrap();
        let hook_dir = dir.path().join("echo-hook");
        std::fs::create_dir(&hook_dir).unwrap();

        let yaml = r#"
name: echo-hook
events:
  - agent:start
command: /bin/cat
"#;
        std::fs::write(hook_dir.join("HOOK.yaml"), yaml).unwrap();

        let system = ExternalHookSystem::load(dir.path().to_path_buf());
        assert_eq!(system.len(), 1);

        // Fire agent:start — should match
        let marker = std::time::Instant::now();
        system.fire(
            ExternalHookEvent::AgentStart,
            serde_json::json!({ "test": "data", "ts": marker.elapsed().as_nanos() }),
        );

        // Fire agent:end — should NOT match (hook only listens to agent:start)
        system.fire(
            ExternalHookEvent::AgentEnd,
            serde_json::json!({ "should": "not_match" }),
        );
    }

    #[test]
    fn test_fire_wildcard_pattern() {
        let dir = tempfile::tempdir().unwrap();
        let hook_dir = dir.path().join("agent-star-hook");
        std::fs::create_dir(&hook_dir).unwrap();

        let yaml = r#"
name: agent-star-hook
events:
  - agent:*
command: /usr/bin/env
"#;
        std::fs::write(hook_dir.join("HOOK.yaml"), yaml).unwrap();

        let system = ExternalHookSystem::load(dir.path().to_path_buf());
        assert_eq!(system.len(), 1);

        // agent:start matches agent:*
        system.fire(
            ExternalHookEvent::AgentStart,
            serde_json::json!({ "event": "start" }),
        );
        // agent:end matches agent:*
        system.fire(
            ExternalHookEvent::AgentEnd,
            serde_json::json!({ "event": "end" }),
        );
        // agent:step matches agent:*
        system.fire(
            ExternalHookEvent::AgentStep,
            serde_json::json!({ "event": "step" }),
        );
        // session:start does NOT match agent:*
        system.fire(
            ExternalHookEvent::SessionStart,
            serde_json::json!({ "event": "session" }),
        );
    }

    #[test]
    fn test_fire_global_wildcard() {
        let dir = tempfile::tempdir().unwrap();
        let hook_dir = dir.path().join("catch-all-hook");
        std::fs::create_dir(&hook_dir).unwrap();

        let yaml = r#"
name: catch-all-hook
events:
  - "*"
command: /usr/bin/env
"#;
        std::fs::write(hook_dir.join("HOOK.yaml"), yaml).unwrap();

        let system = ExternalHookSystem::load(dir.path().to_path_buf());
        assert_eq!(system.len(), 1);

        // Every event should match "*"
        for event in [
            ExternalHookEvent::AgentStart,
            ExternalHookEvent::AgentEnd,
            ExternalHookEvent::AgentStep,
            ExternalHookEvent::SessionStart,
            ExternalHookEvent::SessionEnd,
            ExternalHookEvent::SessionReset,
            ExternalHookEvent::GatewayStartup,
        ] {
            system.fire(
                event.clone(),
                serde_json::json!({ "event": event.as_str() }),
            );
        }
    }

    #[tokio::test]
    async fn acquire_hook_permit_returns_some_on_open_semaphore() {
        let sem = tokio::sync::Semaphore::new(1);
        let permit = acquire_hook_permit(&sem, "demo-hook", "agent:start").await;
        assert!(
            permit.is_some(),
            "open semaphore must yield a permit so the hook runs"
        );
        // Permit dropped at end of scope — the cap is respected.
    }

    #[tokio::test]
    async fn acquire_hook_permit_returns_none_when_semaphore_closed() {
        let sem = tokio::sync::Semaphore::new(8);
        sem.close();
        let permit = acquire_hook_permit(&sem, "demo-hook", "agent:start").await;
        assert!(
            permit.is_none(),
            "closed semaphore must refuse the hook so the system-wide \
             fork() rate cap stays a hard guarantee (issue #5118)"
        );
    }

    #[test]
    fn test_fire_empty_payload() {
        let dir = tempfile::tempdir().unwrap();
        let hook_dir = dir.path().join("empty-payload-hook");
        std::fs::create_dir(&hook_dir).unwrap();

        let yaml = r#"
name: empty-payload-hook
events:
  - gateway:startup
command: /usr/bin/env
"#;
        std::fs::write(hook_dir.join("HOOK.yaml"), yaml).unwrap();

        let system = ExternalHookSystem::load(dir.path().to_path_buf());
        assert_eq!(system.len(), 1);

        // Fire with empty JSON object
        system.fire(ExternalHookEvent::GatewayStartup, serde_json::json!({}));
    }
}
