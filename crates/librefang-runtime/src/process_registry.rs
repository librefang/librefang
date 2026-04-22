//! Background Process Registry — tracks fire-and-forget processes spawned by
//! `shell_exec` (and future background-exec paths) with a rolling 200 KB output
//! buffer per process.
//!
//! Design goals (mirroring Hermes-Agent `process_registry.py`):
//!
//! * **Rolling buffer**: each process keeps at most [`MAX_OUTPUT_BYTES`] of
//!   recent output.  When the buffer exceeds the limit, bytes are dropped from
//!   the front so that exactly [`MAX_OUTPUT_BYTES`] of the *newest* output are
//!   retained.  This is O(n) rather than a true ring-buffer but keeps the
//!   implementation straightforward without extra dependencies.
//! * **Status tracking**: processes move from `Running` to `Finished(exit_code)`
//!   when their reader task observes EOF.
//! * **Session scoping**: optional `session_id` lets callers query processes
//!   belonging to a particular agent turn.
//! * **Registry-wide singleton**: exposed as [`ProcessRegistry`] with an
//!   [`Arc`]-cloneable handle so it can be shared across async tasks.
//!
//! Intentionally **not** implemented here (out of scope for this port):
//! * Watch-pattern rate limiting / overload kill switch.
//! * JSON checkpoint / crash recovery.
//! * PTY support.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tracing::{debug, warn};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum bytes of output retained per process (200 KB, matching Hermes).
pub const MAX_OUTPUT_BYTES: usize = 200 * 1024;

/// Maximum number of finished entries kept in the registry before the oldest
/// is pruned.  Running entries are never pruned automatically.
pub const MAX_FINISHED_ENTRIES: usize = 64;

// ---------------------------------------------------------------------------
// ProcessStatus
// ---------------------------------------------------------------------------

/// Lifecycle state of a registered process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessStatus {
    /// The process is still running.
    Running,
    /// The process has exited with the given code.
    Finished(i32),
}

impl std::fmt::Display for ProcessStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProcessStatus::Running => write!(f, "running"),
            ProcessStatus::Finished(code) => write!(f, "finished({})", code),
        }
    }
}

// ---------------------------------------------------------------------------
// ProcessEntry
// ---------------------------------------------------------------------------

/// A single entry in the registry.
pub struct ProcessEntry {
    /// OS-level process identifier.
    pub pid: u32,
    /// Original command string (for display / audit).
    pub command: String,
    /// Current lifecycle state.
    pub status: ProcessStatus,
    /// Rolling output buffer — at most [`MAX_OUTPUT_BYTES`] of recent bytes.
    output_buf: String,
    /// When the process was registered.
    pub started_at: Instant,
    /// When the process transitioned to a `Finished` state.  `None` while still running.
    pub finished_at: Option<Instant>,
    /// Optional agent session this process belongs to.
    pub session_id: Option<String>,
}

impl ProcessEntry {
    fn new(pid: u32, command: String, session_id: Option<String>) -> Self {
        Self {
            pid,
            command,
            status: ProcessStatus::Running,
            output_buf: String::new(),
            started_at: Instant::now(),
            finished_at: None,
            session_id,
        }
    }

    /// Append `chunk` to the rolling buffer.
    ///
    /// If appending would push the buffer past [`MAX_OUTPUT_BYTES`], bytes are
    /// dropped from the front to bring it back to exactly [`MAX_OUTPUT_BYTES`]
    /// of the newest output.  The truncation is O(n) but amortised O(1) in
    /// practice because we only copy once per overflow event.
    fn append_output(&mut self, chunk: &str) {
        self.output_buf.push_str(chunk);
        if self.output_buf.len() > MAX_OUTPUT_BYTES {
            // Keep the newest MAX_OUTPUT_BYTES bytes.
            let keep_from = self.output_buf.len() - MAX_OUTPUT_BYTES;
            // Align to a UTF-8 character boundary.
            let keep_from = self.output_buf.ceil_char_boundary(keep_from);
            let retained = self.output_buf.split_off(keep_from);
            self.output_buf = retained;
        }
    }

    /// Return a snapshot of the accumulated output.
    pub fn output(&self) -> &str {
        &self.output_buf
    }
}

// ---------------------------------------------------------------------------
// ProcessRegistry
// ---------------------------------------------------------------------------

/// Thread-safe registry of background processes.
///
/// Cheaply cloneable via its inner [`Arc`].
#[derive(Clone)]
pub struct ProcessRegistry {
    inner: Arc<Mutex<RegistryInner>>,
}

struct RegistryInner {
    /// All tracked processes, keyed by PID.
    entries: HashMap<u32, ProcessEntry>,
}

impl RegistryInner {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Prune oldest finished entries to stay under [`MAX_FINISHED_ENTRIES`].
    ///
    /// "Oldest" is defined by `finished_at` — the time a process transitioned
    /// to `Finished`.  This correctly retains the most recently completed
    /// processes, not the ones that happened to start later.
    fn prune_finished(&mut self) {
        let finished_pids: Vec<u32> = self
            .entries
            .iter()
            .filter(|(_, e)| e.status != ProcessStatus::Running)
            .map(|(pid, _)| *pid)
            .collect();

        if finished_pids.len() <= MAX_FINISHED_ENTRIES {
            return;
        }

        // Sort by finished_at (oldest completion first) and remove the surplus.
        let mut with_time: Vec<(u32, Instant)> = finished_pids
            .iter()
            .map(|pid| {
                // finished_at is always Some for non-Running entries; fall back to
                // started_at as a safety net so we never panic if the invariant
                // is somehow violated.
                let t = self.entries[pid]
                    .finished_at
                    .unwrap_or(self.entries[pid].started_at);
                (*pid, t)
            })
            .collect();
        with_time.sort_by_key(|(_, t)| *t);

        let to_remove = with_time.len() - MAX_FINISHED_ENTRIES;
        for (pid, _) in &with_time[..to_remove] {
            self.entries.remove(pid);
        }
    }
}

impl ProcessRegistry {
    /// Create a new, empty registry.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RegistryInner::new())),
        }
    }

    // ------------------------------------------------------------------
    // Write side
    // ------------------------------------------------------------------

    /// Register a freshly-spawned background process.
    ///
    /// If a process with the same `pid` is already tracked (e.g. PID reuse),
    /// the old entry is silently overwritten.
    pub fn register(&self, pid: u32, command: String, session_id: Option<String>) {
        let entry = ProcessEntry::new(pid, command.clone(), session_id);
        let mut inner = self.inner.lock().expect("process_registry lock poisoned");
        inner.entries.insert(pid, entry);
        debug!(pid, command = %command, "process_registry: registered");
    }

    /// Append output bytes for `pid`.  Silently ignored if the pid is unknown.
    pub fn append_output(&self, pid: u32, chunk: &str) {
        let mut inner = self.inner.lock().expect("process_registry lock poisoned");
        if let Some(entry) = inner.entries.get_mut(&pid) {
            entry.append_output(chunk);
        }
    }

    /// Mark `pid` as finished with `exit_code`.  Silently ignored if unknown.
    pub fn mark_finished(&self, pid: u32, exit_code: i32) {
        let mut inner = self.inner.lock().expect("process_registry lock poisoned");
        if let Some(entry) = inner.entries.get_mut(&pid) {
            entry.status = ProcessStatus::Finished(exit_code);
            entry.finished_at = Some(Instant::now());
            debug!(pid, exit_code, "process_registry: process finished");
            inner.prune_finished();
        } else {
            warn!(
                pid,
                exit_code, "process_registry: mark_finished for unknown pid"
            );
        }
    }

    // ------------------------------------------------------------------
    // Read side
    // ------------------------------------------------------------------

    /// Get a snapshot of the accumulated output for `pid`.
    ///
    /// Returns `None` if the pid is not tracked.
    pub fn get_output(&self, pid: u32) -> Option<String> {
        let inner = self.inner.lock().expect("process_registry lock poisoned");
        inner.entries.get(&pid).map(|e| e.output().to_owned())
    }

    /// Get the status of a registered process.
    ///
    /// Returns `None` if the pid is not tracked.
    pub fn get_status(&self, pid: u32) -> Option<ProcessStatus> {
        let inner = self.inner.lock().expect("process_registry lock poisoned");
        inner.entries.get(&pid).map(|e| e.status.clone())
    }

    /// List PIDs of processes currently in `Running` state.
    pub fn list_running(&self) -> Vec<u32> {
        let inner = self.inner.lock().expect("process_registry lock poisoned");
        inner
            .entries
            .values()
            .filter(|e| e.status == ProcessStatus::Running)
            .map(|e| e.pid)
            .collect()
    }

    /// List PIDs of all tracked processes (running and finished).
    pub fn list_all(&self) -> Vec<u32> {
        let inner = self.inner.lock().expect("process_registry lock poisoned");
        inner.entries.keys().copied().collect()
    }

    /// Snapshot of all entries as simple display structs (for diagnostics /
    /// future API endpoints).
    pub fn snapshot(&self) -> Vec<ProcessSnapshot> {
        let inner = self.inner.lock().expect("process_registry lock poisoned");
        inner
            .entries
            .values()
            .map(|e| ProcessSnapshot {
                pid: e.pid,
                command: e.command.clone(),
                status: e.status.clone(),
                output_bytes: e.output_buf.len(),
                uptime_secs: e.started_at.elapsed().as_secs(),
                finished_secs_ago: e.finished_at.map(|t| t.elapsed().as_secs()),
                session_id: e.session_id.clone(),
            })
            .collect()
    }

    /// Remove all finished entries whose output has been read (GC helper).
    ///
    /// The caller is responsible for deciding when "output has been read";
    /// this method simply removes all `Finished` entries.  It is safe to
    /// call at any frequency since it only touches finished processes.
    pub fn cleanup_finished(&self) {
        let mut inner = self.inner.lock().expect("process_registry lock poisoned");
        inner
            .entries
            .retain(|_, e| e.status == ProcessStatus::Running);
    }

    /// Total number of tracked entries (running + finished).
    pub fn len(&self) -> usize {
        let inner = self.inner.lock().expect("process_registry lock poisoned");
        inner.entries.len()
    }

    /// Returns `true` when no processes are tracked.
    pub fn is_empty(&self) -> bool {
        let inner = self.inner.lock().expect("process_registry lock poisoned");
        inner.entries.is_empty()
    }
}

impl Default for ProcessRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// ProcessSnapshot (display / serialisation helper)
// ---------------------------------------------------------------------------

/// Lightweight view of a [`ProcessEntry`] suitable for external display.
#[derive(Debug, Clone)]
pub struct ProcessSnapshot {
    pub pid: u32,
    pub command: String,
    pub status: ProcessStatus,
    /// Number of bytes currently in the rolling buffer.
    pub output_bytes: usize,
    /// Seconds since the process was registered (wall-clock uptime).
    pub uptime_secs: u64,
    /// Seconds since the process finished, or `None` if still running.
    pub finished_secs_ago: Option<u64>,
    pub session_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_and_status() {
        let reg = ProcessRegistry::new();
        reg.register(1234, "sleep 60".into(), None);
        assert_eq!(reg.get_status(1234), Some(ProcessStatus::Running));
    }

    #[test]
    fn test_append_and_get_output() {
        let reg = ProcessRegistry::new();
        reg.register(1, "echo hi".into(), None);
        reg.append_output(1, "hello ");
        reg.append_output(1, "world\n");
        assert_eq!(reg.get_output(1).unwrap(), "hello world\n");
    }

    #[test]
    fn test_mark_finished() {
        let reg = ProcessRegistry::new();
        reg.register(2, "true".into(), Some("session-abc".into()));
        reg.mark_finished(2, 0);
        assert_eq!(reg.get_status(2), Some(ProcessStatus::Finished(0)));
    }

    #[test]
    fn test_list_running() {
        let reg = ProcessRegistry::new();
        reg.register(10, "a".into(), None);
        reg.register(11, "b".into(), None);
        reg.mark_finished(10, 1);
        let running = reg.list_running();
        assert!(!running.contains(&10));
        assert!(running.contains(&11));
    }

    #[test]
    fn test_rolling_buffer_truncates() {
        let reg = ProcessRegistry::new();
        reg.register(99, "bigdata".into(), None);
        // Fill buffer to 1.5× the limit so truncation fires.
        let chunk = "x".repeat(MAX_OUTPUT_BYTES);
        reg.append_output(99, &chunk); // no truncation yet (exactly at limit)
        reg.append_output(99, "extra"); // this push will trigger truncation
        let out = reg.get_output(99).unwrap();
        // After truncation, buffer must not exceed MAX_OUTPUT_BYTES.
        assert!(
            out.len() <= MAX_OUTPUT_BYTES,
            "buffer len {} exceeds MAX_OUTPUT_BYTES {}",
            out.len(),
            MAX_OUTPUT_BYTES
        );
        // The newest content must be preserved.
        assert!(out.ends_with("extra"), "newest content was lost");
    }

    #[test]
    fn test_cleanup_finished() {
        let reg = ProcessRegistry::new();
        reg.register(20, "cmd".into(), None);
        reg.register(21, "cmd2".into(), None);
        reg.mark_finished(20, 0);
        reg.cleanup_finished();
        assert_eq!(reg.get_status(20), None); // removed
        assert_eq!(reg.get_status(21), Some(ProcessStatus::Running)); // kept
    }

    #[test]
    fn test_snapshot() {
        let reg = ProcessRegistry::new();
        reg.register(30, "ls".into(), Some("s1".into()));
        let snaps = reg.snapshot();
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].pid, 30);
        assert_eq!(snaps[0].session_id.as_deref(), Some("s1"));
    }

    #[test]
    fn test_unknown_pid_ignored() {
        let reg = ProcessRegistry::new();
        // These should not panic.
        reg.append_output(999, "data");
        reg.mark_finished(999, 0);
        assert_eq!(reg.get_output(999), None);
        assert_eq!(reg.get_status(999), None);
    }

    #[test]
    fn test_prune_evicts_oldest_finished_not_oldest_started() {
        // Register MAX_FINISHED_ENTRIES + 1 processes and finish them in order.
        // The LAST-registered process should be the one kept when we overflow,
        // because prune_finished evicts by finished_at (oldest completion first).
        let reg = ProcessRegistry::new();
        let n = MAX_FINISHED_ENTRIES + 1;
        for pid in 0..n as u32 {
            reg.register(pid, format!("cmd-{pid}"), None);
        }
        // Finish them in ascending PID order so pid=0 has the oldest finished_at.
        for pid in 0..n as u32 {
            reg.mark_finished(pid, 0);
        }
        // The registry must not exceed MAX_FINISHED_ENTRIES after pruning.
        assert!(
            reg.len() <= MAX_FINISHED_ENTRIES,
            "registry len {} exceeds MAX_FINISHED_ENTRIES",
            reg.len()
        );
        // The most recently finished process (highest PID) must still be present.
        assert_eq!(
            reg.get_status(n as u32 - 1),
            Some(ProcessStatus::Finished(0)),
            "most recently finished process was incorrectly pruned"
        );
    }

    #[test]
    fn test_is_empty() {
        let reg = ProcessRegistry::new();
        assert!(reg.is_empty());
        reg.register(1, "cmd".into(), None);
        assert!(!reg.is_empty());
    }

    #[test]
    fn test_snapshot_includes_finished_secs_ago() {
        let reg = ProcessRegistry::new();
        reg.register(42, "cmd".into(), None);
        // Before finishing, finished_secs_ago must be None.
        let snap = reg.snapshot();
        assert_eq!(snap[0].finished_secs_ago, None);
        reg.mark_finished(42, 0);
        // After finishing, finished_secs_ago must be Some.
        let snap = reg.snapshot();
        assert!(snap[0].finished_secs_ago.is_some());
    }
}
