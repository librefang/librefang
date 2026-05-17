//! Interactive process manager — persistent process sessions.
//!
//! Allows agents to start long-running processes (REPLs, servers, watchers),
//! write to their stdin, read from stdout/stderr, and kill them.

use dashmap::DashMap;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;
use tracing::{debug, warn};

/// Unique process identifier.
pub type ProcessId = String;

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

    /// Start a persistent process. Returns the process ID.
    pub async fn start(
        &self,
        agent_id: &str,
        command: &str,
        args: &[String],
    ) -> Result<ProcessId, String> {
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
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Failed to start process '{}': {}", command, e))?;

        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let stdout_buf = Arc::new(Mutex::new(Vec::<String>::new()));
        let stderr_buf = Arc::new(Mutex::new(Vec::<String>::new()));

        // Spawn background readers for stdout/stderr. We keep the join
        // handles so the per-process reaper can await pipe drain before
        // evicting the registry entry (#5144).
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
        tokio::spawn(async move {
            if let Some(h) = stdout_reader {
                let _ = h.await;
            }
            if let Some(h) = stderr_reader {
                let _ = h.await;
            }
            // Both pipes drained → the child has exited. Remove the
            // registry entry first (releasing the DashMap shard lock
            // immediately — never hold a `get_mut` guard across the
            // `child.wait()` await, or a concurrent `kill()` would
            // block on the same shard), then reap the owned child to
            // collect its exit status and avoid a zombie. If `kill()`
            // already removed the entry this is a harmless no-op.
            if let Some((_, mut proc)) = processes.remove(&reap_id) {
                let _ = proc.child.wait().await;
                debug!(process_id = %reap_id, "Reaped naturally-exited process");
            }
        });

        Ok(id)
    }

    /// Write data to a process's stdin.
    pub async fn write(&self, process_id: &str, data: &str) -> Result<(), String> {
        let mut entry = self
            .processes
            .get_mut(process_id)
            .ok_or_else(|| format!("Process '{}' not found", process_id))?;

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
    pub async fn read(&self, process_id: &str) -> Result<(Vec<String>, Vec<String>), String> {
        let entry = self
            .processes
            .get(process_id)
            .ok_or_else(|| format!("Process '{}' not found", process_id))?;

        let mut stdout = entry.stdout_buf.lock().await;
        let mut stderr = entry.stderr_buf.lock().await;

        let out_lines: Vec<String> = stdout.drain(..).collect();
        let err_lines: Vec<String> = stderr.drain(..).collect();

        Ok((out_lines, err_lines))
    }

    /// Kill a process.
    pub async fn kill(&self, process_id: &str) -> Result<(), String> {
        let (_, mut proc) = self
            .processes
            .remove(process_id)
            .ok_or_else(|| format!("Process '{}' not found", process_id))?;

        if let Some(pid) = proc.child.id() {
            debug!(process_id, pid, "Killing persistent process");
            let _ = crate::subprocess_sandbox::kill_process_tree(pid, 3000).await;
        }
        let _ = proc.child.kill().await;
        Ok(())
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
            let _ = self.kill(&id).await;
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
        let id = pm.start("agent1", cmd, &args).await.unwrap();
        assert!(id.starts_with("proc_"));

        let list = pm.list("agent1");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].agent_id, "agent1");

        // Cleanup
        let _ = pm.kill(&id).await;
    }

    #[tokio::test]
    async fn test_per_agent_limit() {
        let pm = ProcessManager::new(1);

        let (cmd, args) = long_running_proc();
        let id1 = pm.start("agent1", cmd, &args).await.unwrap();
        let result = pm.start("agent1", cmd, &args).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("max: 1"));

        let _ = pm.kill(&id1).await;
    }

    #[tokio::test]
    async fn test_kill_nonexistent() {
        let pm = ProcessManager::new(5);
        let result = pm.kill("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_read_nonexistent() {
        let pm = ProcessManager::new(5);
        let result = pm.read("nonexistent").await;
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
        let id = pm.start("agent1", cmd, &args).await.unwrap();
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
        assert!(pm.kill(&id).await.is_err());
    }

    /// The reaper must not fight an explicit `kill()`: killing a
    /// long-running process still removes exactly one entry and leaves
    /// the manager consistent (no double-remove panic, count == 0).
    #[tokio::test]
    async fn explicit_kill_still_reaps_exactly_once() {
        let pm = ProcessManager::new(5);
        let (cmd, args) = long_running_proc();
        let id = pm.start("agent1", cmd, &args).await.unwrap();
        assert_eq!(pm.count(), 1);
        pm.kill(&id).await.unwrap();

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
}
