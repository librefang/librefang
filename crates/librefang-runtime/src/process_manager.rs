//! Interactive process manager — persistent process sessions.
//!
//! Allows agents to start long-running processes (REPLs, servers, watchers),
//! write to their stdin, read from stdout/stderr, and kill them.

use async_trait::async_trait;
use dashmap::DashMap;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;
use tracing::{debug, warn};

/// Unique process identifier.
pub type ProcessId = String;

/// Maximum bytes of trailing output handed to a [`ProcessCompletionSink`].
/// The reaper keeps only the newest bytes so a chatty long-running process
/// does not ship its entire buffer through the completion event.
const COMPLETION_OUTPUT_TAIL_BYTES: usize = 4 * 1024;

/// Terminal outcome of a tracked background process, delivered to a
/// [`ProcessCompletionSink`] the moment the process leaves the registry.
#[derive(Debug, Clone)]
pub struct ProcessOutcome {
    /// Exit code when the process exited on its own; `None` when it was
    /// terminated by the manager (`process_kill` / idle-cleanup sweep) or
    /// its status could not be collected.
    pub exit_code: Option<i32>,
    /// `true` when the manager terminated the process rather than it
    /// exiting on its own.
    pub cancelled: bool,
    /// Tail of the process's captured stdout+stderr (newest bytes, capped
    /// at [`COMPLETION_OUTPUT_TAIL_BYTES`]). Empty if the agent already
    /// drained the buffer via `process_poll`.
    pub output_tail: String,
}

/// Notified exactly once when a tracked background process reaches a
/// terminal state. Defined in the runtime and implemented by the kernel's
/// async-task tracker (#4983 / #6471) so the runtime never gains a direct
/// kernel dependency — the same trait-injection pattern used for other
/// kernel→runtime callbacks.
#[async_trait]
pub trait ProcessCompletionSink: Send + Sync {
    /// Deliver the terminal outcome. Invoked at most once per process,
    /// from whichever terminal path (natural exit, kill, cleanup) removes
    /// the registry entry first.
    async fn on_process_finished(&self, outcome: ProcessOutcome);
}

/// Join the buffered stdout then stderr lines and keep only the newest
/// [`COMPLETION_OUTPUT_TAIL_BYTES`] bytes, aligned to a char boundary.
async fn collect_output_tail(
    stdout_buf: &Arc<Mutex<Vec<String>>>,
    stderr_buf: &Arc<Mutex<Vec<String>>>,
) -> String {
    let mut joined = String::new();
    for buf in [stdout_buf, stderr_buf] {
        let lines = buf.lock().await;
        for line in lines.iter() {
            joined.push_str(line);
            joined.push('\n');
        }
    }
    if joined.len() > COMPLETION_OUTPUT_TAIL_BYTES {
        let mut start = joined.len() - COMPLETION_OUTPUT_TAIL_BYTES;
        while start < joined.len() && !joined.is_char_boundary(start) {
            start += 1;
        }
        joined = joined[start..].to_string();
    }
    joined
}

/// A managed persistent process.
struct ManagedProcess {
    /// stdin writer.
    stdin: Option<tokio::process::ChildStdin>,
    /// Accumulated stdout output.
    stdout_buf: Arc<Mutex<Vec<String>>>,
    /// Accumulated stderr output.
    stderr_buf: Arc<Mutex<Vec<String>>>,
    /// The child process handle.
    child: tokio::process::Child,
    /// Agent that owns this process.
    agent_id: String,
    /// Command that was started.
    command: String,
    /// When the process was started.
    started_at: std::time::Instant,
    /// Optional completion sink (#6471). When present, the terminal path
    /// that removes this entry (natural exit, kill, or cleanup) hands it
    /// the process's [`ProcessOutcome`] exactly once.
    completion_sink: Option<Arc<dyn ProcessCompletionSink>>,
}

/// Process info for listing.
#[derive(Debug, Clone)]
pub struct ProcessInfo {
    /// Process ID.
    pub id: ProcessId,
    /// Agent that owns this process.
    pub agent_id: String,
    /// Command that was started.
    pub command: String,
    /// Whether the process is still running.
    pub alive: bool,
    /// Uptime in seconds.
    pub uptime_secs: u64,
}

/// Manager for persistent agent processes.
pub struct ProcessManager {
    // `Arc` so a detached reaper task (spawned per process in `start`)
    // can hold a clone and evict the entry the moment the child exits
    // on its own — `kill()` only reaps explicitly-killed processes, and
    // `cleanup()` evicts by uptime, so without this a long-lived daemon
    // running many short-lived process tools accumulates zombie
    // `ManagedProcess` records forever (#5144).
    processes: Arc<DashMap<ProcessId, ManagedProcess>>,
    max_per_agent: usize,
    next_id: std::sync::atomic::AtomicU64,
}

impl ProcessManager {
    /// Create a new process manager.
    pub fn new(max_per_agent: usize) -> Self {
        Self {
            processes: Arc::new(DashMap::new()),
            max_per_agent,
            next_id: std::sync::atomic::AtomicU64::new(1),
        }
    }

    /// Start a persistent process. Returns the process ID together with any
    /// env var names the sandbox refused to inject, so the caller can surface
    /// the drop to the agent (#6458).
    ///
    /// `operator_env` is the operator's own `exec_policy.allowed_env_vars`
    /// (trusted — only the daemon's reserved secrets are refused);
    /// `untrusted_env` is an attacker-controllable passthrough list (a hand's
    /// `hand_allowed_env` — full secret heuristic applies). The spawned
    /// child's environment is scrubbed down to the safe baseline plus these
    /// names (see [`crate::subprocess_sandbox::sandbox_command`]).
    ///
    /// Untracked variant — no completion sink. Equivalent to
    /// [`start_tracked`](Self::start_tracked) with an `on_spawn` closure
    /// that always returns `None`.
    pub async fn start(
        &self,
        agent_id: &str,
        command: &str,
        args: &[String],
        operator_env: &[String],
        untrusted_env: &[String],
    ) -> Result<(ProcessId, Vec<String>), String> {
        self.start_tracked(agent_id, command, args, operator_env, untrusted_env, |_| {
            None
        })
        .await
    }

    /// Like [`start`](Self::start), but wires a completion sink (#6471).
    ///
    /// After the child is spawned and its OS pid is known, `on_spawn(pid)`
    /// is invoked **synchronously** (before the per-process reaper task is
    /// spawned, so there is no race window in which the process could
    /// complete before the caller has registered its tracking). Whatever
    /// [`ProcessCompletionSink`] it returns is stored and handed the
    /// process's [`ProcessOutcome`] exactly once when the process reaches a
    /// terminal state — natural exit (reaper), explicit `process_kill`, or
    /// the idle-cleanup sweep. Returning `None` disables tracking for this
    /// process. When the child has already exited (no pid) `on_spawn` is not
    /// called.
    pub async fn start_tracked<F>(
        &self,
        agent_id: &str,
        command: &str,
        args: &[String],
        operator_env: &[String],
        untrusted_env: &[String],
        on_spawn: F,
    ) -> Result<(ProcessId, Vec<String>), String>
    where
        F: FnOnce(u32) -> Option<Arc<dyn ProcessCompletionSink>>,
    {
        // Check per-agent limit
        let agent_count = self
            .processes
            .iter()
            .filter(|entry| entry.value().agent_id == agent_id)
            .count();

        if agent_count >= self.max_per_agent {
            return Err(format!(
                "Agent '{}' already has {} processes (max: {})",
                agent_id, agent_count, self.max_per_agent
            ));
        }

        let mut cmd = tokio::process::Command::new(command);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Put the child in its own process group so `kill_process_tree`
        // can safely use `kill(-pgid, ...)` to reach the whole subtree.
        // Without this the child inherits the parent's pgid and the
        // tree-kill path would target whichever unrelated process group
        // happens to have the child's PID as its PGID — see
        // `is_process_group_leader` in subprocess_sandbox.rs for why
        // that matters on long-lived runners like GitHub Actions.
        #[cfg(unix)]
        cmd.process_group(0);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
                                         // SECURITY: scrub the daemon environment before spawn, mirroring `tool_shell_exec` (shell.rs:130) and `LocalBackend::run_command`.
                                         // Without this the child inherits the daemon's full process env — including `LIBREFANG_VAULT_KEY` and provider API keys — which an agent can read straight back via `process_poll` (e.g. `process_start {"command":"env"}` under the default Allowlist, where `env` is a safe_bin).
                                         // `sandbox_command` `env_clear()`s and re-adds only the safe allowlist + the agent's env allowlists (operator + untrusted, #6458).
        let refused_env =
            crate::subprocess_sandbox::sandbox_command(&mut cmd, operator_env, untrusted_env);
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Failed to start process '{}': {}", command, e))?;

        // Resolve the completion sink synchronously, before the reaper task
        // is spawned, so a fast-exiting child cannot fire completion before
        // the caller has registered its tracking (#6471). A child that has
        // already exited exposes no pid → no tracking.
        let completion_sink = child.id().and_then(on_spawn);

        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let stdout_buf = Arc::new(Mutex::new(Vec::<String>::new()));
        let stderr_buf = Arc::new(Mutex::new(Vec::<String>::new()));

        // Spawn background readers for stdout/stderr. We keep the join
        // handles so the per-process reaper can await pipe drain before
        // evicting the registry entry (#5144), and surfaces panics so a
        // crashed reader no longer silently truncates captured output
        // (#5137).
        let stdout_reader = stdout.map(|out| {
            let buf = stdout_buf.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(out);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let mut b = buf.lock().await;
                    // Cap buffer at 1000 lines
                    if b.len() >= 1000 {
                        b.drain(..100); // remove oldest 100
                    }
                    b.push(line);
                }
            })
        });

        let stderr_reader = stderr.map(|err| {
            let buf = stderr_buf.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(err);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let mut b = buf.lock().await;
                    if b.len() >= 1000 {
                        b.drain(..100);
                    }
                    b.push(line);
                }
            })
        });

        let id = format!(
            "proc_{}",
            self.next_id
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        );

        let cmd_display = if args.is_empty() {
            command.to_string()
        } else {
            format!("{} {}", command, args.join(" "))
        };

        debug!(process_id = %id, command = %cmd_display, agent = %agent_id, "Started persistent process");

        self.processes.insert(
            id.clone(),
            ManagedProcess {
                stdin,
                stdout_buf,
                stderr_buf,
                child,
                agent_id: agent_id.to_string(),
                command: cmd_display,
                started_at: std::time::Instant::now(),
                completion_sink,
            },
        );

        // Per-process reaper: a child that exits on its own closes both
        // pipes (the readers above hit EOF and their tasks finish). Once
        // both readers are done, confirm the child has actually exited
        // via the registry entry's `child.wait()` and evict it, so
        // naturally-exited processes don't linger as zombie records
        // until `cleanup()`'s uptime sweep or an explicit `kill()`
        // (#5144). If a `kill()` removed the entry first this is a
        // harmless no-op.
        let processes = self.processes.clone();
        let reap_id = id.clone();
        let reap_agent = agent_id.to_string();
        tokio::spawn(async move {
            if let Some(h) = stdout_reader {
                if let Err(e) = h.await {
                    if e.is_panic() {
                        tracing::error!(
                            agent = %reap_agent,
                            process_id = %reap_id,
                            error = %e,
                            "stdout reader task panicked; process output truncated"
                        );
                    }
                }
            }
            if let Some(h) = stderr_reader {
                if let Err(e) = h.await {
                    if e.is_panic() {
                        tracing::error!(
                            agent = %reap_agent,
                            process_id = %reap_id,
                            error = %e,
                            "stderr reader task panicked; process output truncated"
                        );
                    }
                }
            }
            // Both pipes drained → the child has exited. Remove the
            // registry entry first (releasing the DashMap shard lock
            // immediately — never hold a `get_mut` guard across the
            // `child.wait()` await, or a concurrent `kill()` would
            // block on the same shard), then reap the owned child to
            // collect its exit status and avoid a zombie. If `kill()`
            // already removed the entry this is a harmless no-op.
            if let Some((_, mut proc)) = processes.remove(&reap_id) {
                let exit_code = proc.child.wait().await.ok().and_then(|s| s.code());
                debug!(process_id = %reap_id, "Reaped naturally-exited process");
                // Natural-exit terminal path. Fire the completion sink (if
                // any) exactly once — the DashMap remove above is atomic, so
                // an explicit kill that removed the entry first reaches this
                // `remove` as `None` and never double-fires (#6471).
                if let Some(sink) = proc.completion_sink.take() {
                    let output_tail = collect_output_tail(&proc.stdout_buf, &proc.stderr_buf).await;
                    sink.on_process_finished(ProcessOutcome {
                        exit_code,
                        cancelled: false,
                        output_tail,
                    })
                    .await;
                }
            }
        });

        Ok((id, refused_env))
    }

    /// Write data to a process's stdin.
    ///
    /// Ownership-checked: a process owned by a different agent is reported as
    /// "not found" so a caller cannot probe or write to another agent's
    /// process via a guessable `proc_N` id.
    pub async fn write(&self, process_id: &str, agent_id: &str, data: &str) -> Result<(), String> {
        let mut entry = match self.processes.get_mut(process_id) {
            Some(e) if e.agent_id == agent_id => e,
            _ => return Err(format!("Process '{}' not found", process_id)),
        };

        let proc = entry.value_mut();
        if let Some(stdin) = &mut proc.stdin {
            stdin
                .write_all(data.as_bytes())
                .await
                .map_err(|e| format!("Write failed: {}", e))?;
            stdin
                .flush()
                .await
                .map_err(|e| format!("Flush failed: {}", e))?;
            Ok(())
        } else {
            Err("Process stdin is closed".to_string())
        }
    }

    /// Read accumulated stdout/stderr (non-blocking drain).
    ///
    /// Ownership-checked: a process owned by a different agent is reported as
    /// "not found" so a caller cannot drain another agent's process output
    /// (which may contain secrets) via a guessable `proc_N` id.
    pub async fn read(
        &self,
        process_id: &str,
        agent_id: &str,
    ) -> Result<(Vec<String>, Vec<String>), String> {
        let entry = match self.processes.get(process_id) {
            Some(e) if e.agent_id == agent_id => e,
            _ => return Err(format!("Process '{}' not found", process_id)),
        };

        let mut stdout = entry.stdout_buf.lock().await;
        let mut stderr = entry.stderr_buf.lock().await;

        let out_lines: Vec<String> = stdout.drain(..).collect();
        let err_lines: Vec<String> = stderr.drain(..).collect();

        Ok((out_lines, err_lines))
    }

    /// Kill a process the agent owns.
    ///
    /// Ownership-checked: `remove_if` deletes the entry only when its
    /// `agent_id` matches, atomically. A process owned by a different agent
    /// (or a nonexistent id) is reported as "not found" so a caller cannot
    /// terminate another agent's process via a guessable `proc_N` id.
    pub async fn kill(&self, process_id: &str, agent_id: &str) -> Result<(), String> {
        let (_, mut proc) = self
            .processes
            .remove_if(process_id, |_, v| v.agent_id == agent_id)
            .ok_or_else(|| format!("Process '{}' not found", process_id))?;
        Self::reap(process_id, &mut proc).await;
        Ok(())
    }

    /// Kill a process without an ownership check — for daemon-internal
    /// sweeps (`cleanup`) that operate across all agents.
    async fn force_kill(&self, process_id: &str) -> Result<(), String> {
        let (_, mut proc) = self
            .processes
            .remove(process_id)
            .ok_or_else(|| format!("Process '{}' not found", process_id))?;
        Self::reap(process_id, &mut proc).await;
        Ok(())
    }

    /// Terminate an already-removed child (tree-kill then reap).
    async fn reap(process_id: &str, proc: &mut ManagedProcess) {
        if let Some(pid) = proc.child.id() {
            debug!(process_id, pid, "Killing persistent process");
            let _ = crate::subprocess_sandbox::kill_process_tree(pid, 3000).await;
        }
        let _ = proc.child.kill().await;
        // Kill / cleanup terminal path. The caller already removed the entry
        // from the registry (`remove_if` / `remove`), so this owns the sink
        // and the natural-exit reaper will see `None` — exactly-once holds
        // (#6471). Surfaced as `cancelled` (no meaningful exit code after a
        // kill) so the agent can distinguish a self-terminated process from
        // one that ran to completion.
        if let Some(sink) = proc.completion_sink.take() {
            // The cancelled outcome carries no output tail — the production sink
            // discards it for a killed process, so don't pay to collect it here.
            sink.on_process_finished(ProcessOutcome {
                exit_code: None,
                cancelled: true,
                output_tail: String::new(),
            })
            .await;
        }
    }

    /// List all processes for an agent.
    pub fn list(&self, agent_id: &str) -> Vec<ProcessInfo> {
        self.processes
            .iter()
            .filter(|entry| entry.value().agent_id == agent_id)
            .map(|entry| {
                let alive = entry.value().child.id().is_some();
                ProcessInfo {
                    id: entry.key().clone(),
                    agent_id: entry.value().agent_id.clone(),
                    command: entry.value().command.clone(),
                    alive,
                    uptime_secs: entry.value().started_at.elapsed().as_secs(),
                }
            })
            .collect()
    }

    /// Cleanup: kill processes older than timeout.
    pub async fn cleanup(&self, max_age_secs: u64) {
        let to_remove: Vec<ProcessId> = self
            .processes
            .iter()
            .filter(|entry| entry.value().started_at.elapsed().as_secs() > max_age_secs)
            .map(|entry| entry.key().clone())
            .collect();

        for id in to_remove {
            warn!(process_id = %id, "Cleaning up stale process");
            let _ = self.force_kill(&id).await;
        }
    }

    /// Total process count.
    pub fn count(&self) -> usize {
        self.processes.len()
    }
}

impl Default for ProcessManager {
    fn default() -> Self {
        Self::new(5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Long-running, IO-quiet placeholder process for tests that need
    /// "something alive in the registry until we kill it". The earlier
    /// history of this helper is a cautionary tale: it used `cat`,
    /// which blocked on stdin and exposed a latent bug where
    /// `kill_process_tree` sent `kill -TERM -<pid>` to a non-leader
    /// (because `ProcessManager::start` didn't put the child in its
    /// own pgid). On Ubuntu CI the resulting signal would occasionally
    /// land on the actions-runner session leader, killing the whole
    /// job mid-test. Moving to `sleep` narrowed the window but didn't
    /// fix the root cause — that took
    /// (a) spawning children with `process_group(0)` and
    /// (b) gating the negative-PID kill on `is_process_group_leader`
    /// in `subprocess_sandbox::kill_tree_unix`.
    fn long_running_proc() -> (&'static str, Vec<String>) {
        if cfg!(windows) {
            (
                "cmd",
                vec![
                    "/C".to_string(),
                    "timeout".to_string(),
                    "/t".to_string(),
                    "30".to_string(),
                ],
            )
        } else {
            ("sleep", vec!["30".to_string()])
        }
    }

    #[tokio::test]
    async fn test_start_and_list() {
        let pm = ProcessManager::new(5);

        let (cmd, args) = long_running_proc();
        let (id, _) = pm.start("agent1", cmd, &args, &[], &[]).await.unwrap();
        assert!(id.starts_with("proc_"));

        let list = pm.list("agent1");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].agent_id, "agent1");

        // Cleanup
        let _ = pm.kill(&id, "agent1").await;
    }

    #[tokio::test]
    async fn test_per_agent_limit() {
        let pm = ProcessManager::new(1);

        let (cmd, args) = long_running_proc();
        let (id1, _) = pm.start("agent1", cmd, &args, &[], &[]).await.unwrap();
        let result = pm.start("agent1", cmd, &args, &[], &[]).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("max: 1"));

        let _ = pm.kill(&id1, "agent1").await;
    }

    #[tokio::test]
    async fn test_kill_nonexistent() {
        let pm = ProcessManager::new(5);
        let result = pm.kill("nonexistent", "agent1").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_read_nonexistent() {
        let pm = ProcessManager::new(5);
        let result = pm.read("nonexistent", "agent1").await;
        assert!(result.is_err());
    }

    #[test]
    fn test_default_process_manager() {
        let pm = ProcessManager::default();
        assert_eq!(pm.max_per_agent, 5);
        assert_eq!(pm.count(), 0);
    }

    /// A short-lived command that exits on its own (no explicit `kill`).
    fn short_lived_proc() -> (&'static str, Vec<String>) {
        if cfg!(windows) {
            ("cmd", vec!["/C".to_string(), "exit".to_string()])
        } else {
            ("true", vec![])
        }
    }

    /// Regression (#5144): a managed child that exits on its own must
    /// have its registry entry reaped automatically by the per-process
    /// reaper. Before the fix only `kill()` (explicit) or `cleanup()`
    /// (uptime-based) removed entries, so naturally-exited short-lived
    /// process tools accumulated as zombie `ManagedProcess` records.
    #[tokio::test]
    async fn naturally_exited_process_is_reaped() {
        let pm = ProcessManager::new(5);
        let (cmd, args) = short_lived_proc();
        let (id, _) = pm.start("agent1", cmd, &args, &[], &[]).await.unwrap();
        assert_eq!(pm.count(), 1, "entry present right after start");

        // The reaper awaits both pipe readers then `child.wait()`; for a
        // process that exits immediately this happens quickly. Poll with
        // a bounded budget rather than a fixed sleep to stay robust on
        // slow CI without flaking.
        let mut reaped = false;
        for _ in 0..100 {
            if pm.count() == 0 {
                reaped = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(
            reaped,
            "naturally-exited process entry was not reaped (count still {})",
            pm.count()
        );

        // A late `kill` of the already-reaped id is a harmless error,
        // not a panic / double-free.
        assert!(pm.kill(&id, "agent1").await.is_err());
    }

    /// The reaper must not fight an explicit `kill()`: killing a
    /// long-running process still removes exactly one entry and leaves
    /// the manager consistent (no double-remove panic, count == 0).
    #[tokio::test]
    async fn explicit_kill_still_reaps_exactly_once() {
        let pm = ProcessManager::new(5);
        let (cmd, args) = long_running_proc();
        let (id, _) = pm.start("agent1", cmd, &args, &[], &[]).await.unwrap();
        assert_eq!(pm.count(), 1);
        pm.kill(&id, "agent1").await.unwrap();

        // After kill the pipes EOF and the reaper runs; either path
        // converges on count == 0 with no panic.
        for _ in 0..100 {
            if pm.count() == 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert_eq!(pm.count(), 0, "manager must be empty after kill + reap");
    }

    /// Regression: `ProcessManager::start` must scrub the daemon environment
    /// before spawning, so a secret in the daemon env never reaches the
    /// child (which the agent can read back via `process_poll`), while an
    /// explicitly-allowed var still passes. `sh -c "env; sleep 30"` keeps the
    /// child alive after `env` prints so the drain does not race the reaper.
    #[cfg(unix)]
    #[tokio::test]
    async fn start_scrubs_daemon_env_from_child() {
        std::env::set_var("LIBREFANG_PM_SECRET_SENTINEL", "leaked-secret-value");
        std::env::set_var("LIBREFANG_PM_ALLOWED_SENTINEL", "allowed-value");
        let pm = ProcessManager::new(5);
        let (id, _) = pm
            .start(
                "agent1",
                "sh",
                &["-c".to_string(), "env; sleep 30".to_string()],
                &["LIBREFANG_PM_ALLOWED_SENTINEL".to_string()],
                &[],
            )
            .await
            .unwrap();

        let mut combined = String::new();
        let mut saw_allowed = false;
        for _ in 0..100 {
            if let Ok((out, _err)) = pm.read(&id, "agent1").await {
                for line in out {
                    combined.push_str(&line);
                    combined.push('\n');
                }
            }
            if combined.contains("allowed-value") {
                saw_allowed = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let _ = pm.kill(&id, "agent1").await;
        std::env::remove_var("LIBREFANG_PM_SECRET_SENTINEL");
        std::env::remove_var("LIBREFANG_PM_ALLOWED_SENTINEL");

        assert!(
            saw_allowed,
            "explicitly-allowed env var should reach the child"
        );
        assert!(
            !combined.contains("leaked-secret-value"),
            "daemon secret must NOT leak into the spawned child's environment: {combined}"
        );
    }

    /// Regression: `read` / `write` / `kill` are ownership-checked. A
    /// different agent must not access another agent's process via a
    /// guessable `proc_N` id — the operations report "not found" and leave
    /// the victim's process untouched.
    #[tokio::test]
    async fn cross_agent_process_access_is_denied() {
        let pm = ProcessManager::new(5);
        let (cmd, args) = long_running_proc();
        let (id, _) = pm.start("owner", cmd, &args, &[], &[]).await.unwrap();

        assert!(pm.read(&id, "attacker").await.is_err());
        assert!(pm.write(&id, "attacker", "data\n").await.is_err());
        assert!(pm.kill(&id, "attacker").await.is_err());
        assert_eq!(
            pm.count(),
            1,
            "an unauthorized kill must not remove the owner's process"
        );

        // The owner still has full access.
        assert!(pm.read(&id, "owner").await.is_ok());
        pm.kill(&id, "owner").await.unwrap();
    }

    /// Test sink that forwards every delivered [`ProcessOutcome`] onto a
    /// channel so a test can assert exactly-once firing and the payload.
    struct RecordingSink {
        tx: tokio::sync::mpsc::UnboundedSender<ProcessOutcome>,
    }

    #[async_trait::async_trait]
    impl ProcessCompletionSink for RecordingSink {
        async fn on_process_finished(&self, outcome: ProcessOutcome) {
            let _ = self.tx.send(outcome);
        }
    }

    /// Regression (#6471): a tracked process that exits on its own fires its
    /// completion sink exactly once, reporting a non-cancelled outcome with
    /// the child's exit code.
    #[tokio::test]
    async fn tracked_process_natural_exit_fires_sink_with_exit_code() {
        let pm = ProcessManager::new(5);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let (cmd, args) = short_lived_proc();
        let (_id, _) = pm
            .start_tracked("agent1", cmd, &args, &[], &[], move |_pid| {
                Some(Arc::new(RecordingSink { tx }) as Arc<dyn ProcessCompletionSink>)
            })
            .await
            .unwrap();

        let outcome = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv())
            .await
            .expect("sink should fire within the budget")
            .expect("outcome delivered");
        assert!(
            !outcome.cancelled,
            "a naturally-exited process is not cancelled"
        );
        assert_eq!(
            outcome.exit_code,
            Some(0),
            "short-lived success process exits 0"
        );

        // Exactly once: the reaper removed the entry, so no further outcome
        // and no lingering registry record.
        assert!(rx.try_recv().is_err(), "sink must fire exactly once");
        assert_eq!(pm.count(), 0, "reaped entry is gone");
    }

    /// Regression (#6471): killing a tracked process fires its sink exactly
    /// once with `cancelled = true` (no meaningful exit code after a kill).
    #[tokio::test]
    async fn tracked_process_kill_fires_cancelled_outcome() {
        let pm = ProcessManager::new(5);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let (cmd, args) = long_running_proc();
        let (id, _) = pm
            .start_tracked("agent1", cmd, &args, &[], &[], move |_pid| {
                Some(Arc::new(RecordingSink { tx }) as Arc<dyn ProcessCompletionSink>)
            })
            .await
            .unwrap();

        pm.kill(&id, "agent1").await.unwrap();

        let outcome = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv())
            .await
            .expect("sink should fire within the budget")
            .expect("outcome delivered");
        assert!(
            outcome.cancelled,
            "an explicitly-killed process is cancelled"
        );
        assert!(
            outcome.output_tail.is_empty(),
            "the cancel path carries no output tail — it is discarded for a killed process"
        );

        // Give the natural-exit reaper a chance to run: it must find the
        // entry already removed and NOT fire a second outcome.
        for _ in 0..40 {
            if pm.count() == 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(
            rx.try_recv().is_err(),
            "sink must fire exactly once even though both kill and reaper run"
        );
    }
}
