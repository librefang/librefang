//! Write-ahead journal for channel messages.
//!
//! Every incoming message is recorded **before** dispatch so that a crash
//! mid-processing never loses a message.  On startup the journal is scanned
//! for incomplete entries and those messages are re-dispatched.
//!
//! Storage: a single append-only JSONL file (`message_journal.jsonl`) inside
//! `$LIBREFANG_HOME/`.  Completed entries are rewritten out during periodic
//! compaction (or on clean shutdown).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

/// Status of a journaled message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JournalStatus {
    /// Saved to journal, dispatch not yet started.
    Pending,
    /// Dispatch is in progress (task spawned).
    Processing,
    /// Successfully processed — response delivered.
    Completed,
    /// Processing failed after retries.
    Failed,
    /// Dispatch hit a transient block (rate-limit / overload). The entry is
    /// kept on disk with a `next_retry_after` deadline; a periodic ticker
    /// re-dispatches it once the deadline passes. Distinct from `Failed`
    /// because waiting for a quota window to reset is not a real failure
    /// and must NOT count against the retry budget.
    Deferred,
}

/// Marker appended to LLM rate-limit / overload exhaustion messages so the
/// channel bridge can route the journal entry to `JournalStatus::Deferred`
/// instead of `Failed`. Format: `<message> [rate_limit_defer_ms]=<ms>`.
///
/// Emitted by `librefang_runtime::agent_loop` after exhausting in-loop
/// retries; parsed by [`parse_defer_marker`].
pub const RATE_LIMIT_DEFER_MARKER: &str = "[rate_limit_defer_ms]";

/// Extract the deferral hint (in milliseconds) from a kernel-emitted error
/// string, if present. Returns `None` when the marker is absent — the entry
/// should be treated as a hard failure.
pub fn parse_defer_marker(err: &str) -> Option<u64> {
    let idx = err.find(RATE_LIMIT_DEFER_MARKER)?;
    let tail = &err[idx + RATE_LIMIT_DEFER_MARKER.len()..];
    let tail = tail.strip_prefix('=')?.trim_start();
    let end = tail
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(tail.len());
    tail[..end].parse::<u64>().ok()
}

/// A single journal entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalEntry {
    /// Platform-specific unique message ID.
    pub message_id: String,
    /// Channel type (e.g. "telegram", "whatsapp").
    pub channel: String,
    /// Sender platform ID.
    pub sender_id: String,
    /// Sender display name.
    pub sender_name: String,
    /// Message text content.
    pub content: String,
    /// Target agent name (if resolved before journaling).
    #[serde(default)]
    pub agent_name: Option<String>,
    /// When the message was received.
    pub received_at: DateTime<Utc>,
    /// Current status.
    pub status: JournalStatus,
    /// Number of processing attempts.
    #[serde(default)]
    pub attempts: u32,
    /// Last error message (if failed).
    #[serde(default)]
    pub last_error: Option<String>,
    /// When the status was last updated.
    pub updated_at: DateTime<Utc>,
    /// Whether this is a group message.
    #[serde(default)]
    pub is_group: bool,
    /// Thread ID (for forum topics).
    #[serde(default)]
    pub thread_id: Option<String>,
    /// Extra metadata (platform-specific).
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
    /// Earliest time at which a Deferred entry should be re-dispatched.
    /// `None` for non-deferred entries.
    #[serde(default)]
    pub next_retry_after: Option<DateTime<Utc>>,
}

/// Thread-safe message journal backed by a JSONL file.
#[derive(Clone)]
pub struct MessageJournal {
    inner: Arc<Mutex<JournalInner>>,
}

struct JournalInner {
    path: PathBuf,
    /// In-memory index of non-completed entries for fast lookup.
    pending: HashMap<String, JournalEntry>,
}

impl MessageJournal {
    /// Open or create a journal at `dir/message_journal.jsonl`.
    pub fn open(dir: &Path) -> std::io::Result<Self> {
        let path = dir.join("message_journal.jsonl");
        let mut pending = HashMap::new();

        // Load existing entries
        if path.exists() {
            let file = std::fs::File::open(&path)?;
            let reader = std::io::BufReader::new(file);
            for line in reader.lines() {
                let line = match line {
                    Ok(l) if !l.trim().is_empty() => l,
                    _ => continue,
                };
                match serde_json::from_str::<JournalEntry>(&line) {
                    Ok(entry) => {
                        match entry.status {
                            JournalStatus::Completed => {
                                pending.remove(&entry.message_id);
                            }
                            // Failed entries: drop once they exhaust the
                            // 3-strike retry budget. Deferred entries skip
                            // this gate — they are waiting on an external
                            // quota reset, not failing.
                            JournalStatus::Failed if entry.attempts >= 3 => {
                                pending.remove(&entry.message_id);
                            }
                            _ => {
                                pending.insert(entry.message_id.clone(), entry);
                            }
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "Skipping malformed journal line");
                    }
                }
            }

            info!(
                pending = pending.len(),
                path = %path.display(),
                "Message journal loaded"
            );
        }

        Ok(Self {
            inner: Arc::new(Mutex::new(JournalInner { path, pending })),
        })
    }

    /// Record a new message as pending.  Call this BEFORE dispatching.
    ///
    /// The disk write happens **while the inner mutex is held** so that a
    /// concurrent `compact()` cannot rebuild the file from a stale `pending`
    /// snapshot between the write and the in-memory insert (that race let
    /// just-journaled entries get rename-truncated off disk before the
    /// in-memory index caught up — see audit of #3967).  The write runs
    /// inside `spawn_blocking` to keep `OpenOptions::open` + `flush` off the
    /// async reactor; the lock is `tokio::sync::Mutex`, so we can hold it
    /// across the `.await` without blocking other tokio tasks (only other
    /// journal mutators queue, which is what we want).
    pub async fn record(&self, entry: JournalEntry) {
        let line = match serde_json::to_string(&entry) {
            Ok(l) => l,
            Err(e) => {
                error!(error = %e, id = %entry.message_id, "Failed to serialize journal entry");
                return;
            }
        };
        let mut inner = self.inner.lock().await;
        let path = inner.path.clone();
        let write_result =
            tokio::task::spawn_blocking(move || Self::write_line_to_path(&path, &line)).await;
        match write_result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                error!(error = %e, id = %entry.message_id, "Failed to write journal entry");
                return;
            }
            Err(e) => {
                error!(error = %e, id = %entry.message_id, "spawn_blocking panicked writing journal");
                return;
            }
        }
        inner.pending.insert(entry.message_id.clone(), entry);
    }

    /// Record a terminal dispatch outcome.
    ///
    /// Routes the entry to one of three statuses based on the kernel result:
    /// * `success = true` → `Completed` (entry purged on next compaction).
    /// * `success = false` and the error string carries the
    ///   [`RATE_LIMIT_DEFER_MARKER`] suffix → `Deferred` with the parsed
    ///   retry-after window (the periodic ticker re-dispatches it once due).
    /// * Any other failure → `Failed` (counts against the 3-strike retry budget).
    pub async fn record_outcome(&self, message_id: &str, success: bool, err_str: Option<String>) {
        if success {
            self.update_status(message_id, JournalStatus::Completed, None)
                .await;
            return;
        }
        if let Some(ref s) = err_str {
            if let Some(ms) = parse_defer_marker(s) {
                self.defer(
                    message_id,
                    chrono::Duration::milliseconds(ms as i64),
                    Some(s.clone()),
                )
                .await;
                return;
            }
        }
        self.update_status(message_id, JournalStatus::Failed, err_str)
            .await;
    }

    /// Mark an entry as `Deferred` and schedule a retry.
    ///
    /// Use this when a dispatch failure is recoverable on its own
    /// (provider rate-limit / overload). The retry budget is NOT bumped —
    /// waiting for a quota window to reset is not a real failure. The
    /// entry stays on disk and is picked up by [`due_deferred_entries`]
    /// once `now >= next_retry_after`.
    ///
    /// Existing failed `attempts` count is preserved so that a Deferred
    /// entry that previously failed for non-rate-limit reasons still
    /// honors the 3-strike cap when it fails again post-retry.
    pub async fn defer(
        &self,
        message_id: &str,
        retry_after: chrono::Duration,
        last_error: Option<String>,
    ) {
        let mut inner = self.inner.lock().await;
        let path = inner.path.clone();

        let (line, updated) = {
            let entry = match inner.pending.get(message_id) {
                Some(e) => e,
                None => return,
            };
            let mut updated = entry.clone();
            updated.status = JournalStatus::Deferred;
            updated.updated_at = Utc::now();
            updated.next_retry_after = Some(Utc::now() + retry_after);
            updated.last_error = last_error;
            let line = match serde_json::to_string(&updated) {
                Ok(l) => l,
                Err(e) => {
                    error!(error = %e, id = message_id, "Failed to serialize defer update");
                    return;
                }
            };
            (line, updated)
        };

        let write_result =
            tokio::task::spawn_blocking(move || Self::write_line_to_path(&path, &line)).await;
        match write_result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                error!(error = %e, id = message_id, "Failed to write defer entry");
                return;
            }
            Err(e) => {
                error!(error = %e, id = message_id, "spawn_blocking panicked deferring journal");
                return;
            }
        }

        if let Some(entry) = inner.pending.get_mut(message_id) {
            *entry = updated;
        }
    }

    /// Return Deferred entries whose `next_retry_after` deadline has passed.
    ///
    /// Skips entries older than [`Self::MAX_RECOVERY_AGE`] — the same stale
    /// window applied by [`pending_entries`].
    pub async fn due_deferred_entries(&self) -> Vec<JournalEntry> {
        let now = Utc::now();
        let mut inner = self.inner.lock().await;
        let stale_ids: Vec<String> = inner
            .pending
            .values()
            .filter(|e| now - e.received_at > Self::MAX_RECOVERY_AGE)
            .map(|e| e.message_id.clone())
            .collect();
        for id in &stale_ids {
            debug!(
                id,
                "Discarding stale journal entry (older than MAX_RECOVERY_AGE)"
            );
            inner.pending.remove(id);
        }
        inner
            .pending
            .values()
            .filter(|e| {
                matches!(e.status, JournalStatus::Deferred)
                    && e.next_retry_after.map(|d| d <= now).unwrap_or(false)
            })
            .cloned()
            .collect()
    }

    /// Union of [`pending_entries`] (crash-recovery) and
    /// [`due_deferred_entries`] (rate-limit retries that are due now).
    ///
    /// Single-pass: takes the inner lock once, runs the stale-entry sweep
    /// once, and walks the map once. The two-call composition is correct
    /// but acquires the lock and copies entries twice, with no upside.
    pub async fn recoverable_entries(&self) -> Vec<JournalEntry> {
        let now = Utc::now();
        let mut inner = self.inner.lock().await;
        let stale_ids: Vec<String> = inner
            .pending
            .values()
            .filter(|e| now - e.received_at > Self::MAX_RECOVERY_AGE)
            .map(|e| e.message_id.clone())
            .collect();
        for id in &stale_ids {
            debug!(
                id,
                "Discarding stale journal entry (older than MAX_RECOVERY_AGE)"
            );
            inner.pending.remove(id);
        }
        inner
            .pending
            .values()
            .filter(|e| match e.status {
                JournalStatus::Pending | JournalStatus::Processing => true,
                JournalStatus::Deferred => e.next_retry_after.map(|d| d <= now).unwrap_or(false),
                JournalStatus::Completed | JournalStatus::Failed => false,
            })
            .cloned()
            .collect()
    }

    /// Update the status of an existing entry.
    ///
    /// Disk-then-memory ordering: serialize the *desired* new state, write
    /// it under the inner lock, and only mutate the in-memory entry on
    /// success.  The earlier "memory-first, release lock, write disk"
    /// shape (audit of #3967) corrupted the index on transient I/O
    /// failure: in-memory `attempts` was bumped while disk still had the
    /// old count, and after enough retries the in-memory `attempts >= 3`
    /// removed the entry from the retry pool entirely while the disk
    /// record stayed at 0.
    pub async fn update_status(
        &self,
        message_id: &str,
        status: JournalStatus,
        error: Option<String>,
    ) {
        let mut inner = self.inner.lock().await;
        let path = inner.path.clone();

        // Build the proposed updated entry from the current on-record entry
        // without mutating it yet.
        let (line, updated, should_remove) = {
            let entry = match inner.pending.get(message_id) {
                Some(e) => e,
                None => return,
            };
            let mut updated = entry.clone();
            updated.status = status;
            updated.updated_at = Utc::now();
            // Any non-Deferred transition clears the retry deadline so a
            // re-dispatched entry that lands back in Processing is not also
            // returned by `due_deferred_entries` on the next tick.
            if status != JournalStatus::Deferred {
                updated.next_retry_after = None;
            }
            if status == JournalStatus::Failed {
                updated.attempts += 1;
                updated.last_error = error;
            }
            let line = match serde_json::to_string(&updated) {
                Ok(l) => l,
                Err(e) => {
                    error!(error = %e, id = message_id, "Failed to serialize journal update");
                    return;
                }
            };
            let should_remove = status == JournalStatus::Completed
                || (status == JournalStatus::Failed && updated.attempts >= 3);
            (line, updated, should_remove)
        };

        // Write while still holding the lock (see record() doc).  On
        // failure, leave the in-memory entry untouched so the next retry
        // sees the same state the disk does.
        let write_result =
            tokio::task::spawn_blocking(move || Self::write_line_to_path(&path, &line)).await;
        match write_result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                error!(error = %e, id = message_id, "Failed to update journal entry");
                return;
            }
            Err(e) => {
                error!(error = %e, id = message_id, "spawn_blocking panicked updating journal");
                return;
            }
        }

        // Disk persisted; commit the in-memory state.
        if should_remove {
            inner.pending.remove(message_id);
        } else if let Some(entry) = inner.pending.get_mut(message_id) {
            *entry = updated;
        }
    }

    /// Atomically claim an entry for re-dispatch by transitioning its status
    /// from `Pending` or `Deferred` to `Processing`. Returns `true` if the
    /// claim was won; `false` if another caller already moved the entry
    /// past `Pending`/`Deferred` (or the entry is gone).
    ///
    /// Race this guards against: two concurrent snapshots — the boot-time
    /// initial-recovery sweep (`recoverable_entries`) and the periodic
    /// retry ticker (`due_deferred_entries`) — can each see the same
    /// `Deferred` entry whose `next_retry_after` has elapsed if the
    /// initial-recovery dispatch is still in flight when the first ticker
    /// tick fires. Without CAS, both call sites then dispatch the same
    /// `message_id`, costing a double LLM bill and producing two
    /// user-visible replies. With CAS, the second caller observes
    /// `Processing` and skips the dispatch.
    ///
    /// Disk-then-memory ordering matches `update_status`: the proposed
    /// entry is serialized and written *under* the inner lock; in-memory
    /// state advances only after the disk write succeeds. A failed disk
    /// write returns `false` so the caller does not act on a state the
    /// journal cannot persist.
    pub async fn claim(&self, message_id: &str) -> bool {
        let mut inner = self.inner.lock().await;
        let path = inner.path.clone();

        let (line, updated) = {
            let entry = match inner.pending.get(message_id) {
                Some(e) => e,
                None => return false,
            };
            // Reject the claim if another caller already advanced this
            // entry. `Completed` / `Failed` are normally already removed
            // from `pending` (so the `None` branch above catches them),
            // but match exhaustively in case a future code path keeps
            // them around.
            match entry.status {
                JournalStatus::Pending | JournalStatus::Deferred => {}
                JournalStatus::Processing | JournalStatus::Completed | JournalStatus::Failed => {
                    return false
                }
            }
            let mut updated = entry.clone();
            updated.status = JournalStatus::Processing;
            updated.updated_at = Utc::now();
            // Keep `next_retry_after` so that a later defer() that
            // observes the still-set deadline can use it as the
            // round-trip start without a clock query.
            let line = match serde_json::to_string(&updated) {
                Ok(l) => l,
                Err(e) => {
                    error!(error = %e, id = message_id, "Failed to serialize claim update");
                    return false;
                }
            };
            (line, updated)
        };

        let write_result =
            tokio::task::spawn_blocking(move || Self::write_line_to_path(&path, &line)).await;
        match write_result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                error!(error = %e, id = message_id, "Failed to write claim update");
                return false;
            }
            Err(e) => {
                error!(error = %e, id = message_id, "spawn_blocking panicked claiming journal");
                return false;
            }
        }

        if let Some(entry) = inner.pending.get_mut(message_id) {
            *entry = updated;
        }
        true
    }

    /// Maximum age of a message to be eligible for recovery.
    ///
    /// Sized to cover the longest realistic provider quota window: Claude.ai
    /// rate-limits cycle on a 5-hour window, so a Deferred entry that hits
    /// the start of a window must survive until the window resets without
    /// being silently swept. 6 hours leaves margin for clock skew and
    /// short overlap periods. Pre-PR this was 1 hour, which silently
    /// dropped messages waiting on a 5h Claude.ai window — pre-#4754
    /// behaviour was an immediate "rate limited" reply, so the 1h sweep
    /// represented a UX regression for long windows. Bumping the window
    /// keeps the deferred-retry contract honest end-to-end.
    const MAX_RECOVERY_AGE: chrono::TimeDelta = match chrono::TimeDelta::try_hours(6) {
        Some(d) => d,
        None => unreachable!(),
    };

    /// Get all entries that need (re-)processing.
    /// Returns entries with status Pending or Processing (from a previous crash).
    /// Skips entries older than `MAX_RECOVERY_AGE` — they are too stale to recover.
    pub async fn pending_entries(&self) -> Vec<JournalEntry> {
        let now = Utc::now();
        let mut inner = self.inner.lock().await;
        // Remove stale entries
        let stale_ids: Vec<String> = inner
            .pending
            .values()
            .filter(|e| now - e.received_at > Self::MAX_RECOVERY_AGE)
            .map(|e| e.message_id.clone())
            .collect();
        for id in &stale_ids {
            debug!(
                id,
                "Discarding stale journal entry (older than MAX_RECOVERY_AGE)"
            );
            inner.pending.remove(id);
        }
        inner
            .pending
            .values()
            .filter(|e| matches!(e.status, JournalStatus::Pending | JournalStatus::Processing))
            .cloned()
            .collect()
    }

    /// Check if a message is already in the journal (dedup).
    pub async fn contains(&self, message_id: &str) -> bool {
        let inner = self.inner.lock().await;
        inner.pending.contains_key(message_id)
    }

    /// Compact the journal file: rewrite only non-completed entries.
    /// Call periodically or on shutdown.
    ///
    /// Two-phase, lock-aware design:
    ///
    /// 1. **Snapshot under lock** — clone path + pending entries + their
    ///    message-ids out, then drop the lock so channel intake (`record`,
    ///    `update_status`) is not stalled while we fsync the temp file.
    /// 2. **Slow write off lock** — `File::create` + `flush` + `sync_all`
    ///    happen inside `spawn_blocking` without holding the mutex
    ///    (issue #3646: every intake awaits the same `tokio::Mutex`, so
    ///    holding it across `sync_all` serializes all channel traffic
    ///    behind the compactor regardless of which scheduler runs the I/O).
    /// 3. **Re-acquire lock for atomic rename** — before swapping
    ///    `tmp.jsonl → journal.jsonl`, take the lock again and verify no
    ///    new entries were appended to `pending` since the snapshot. If
    ///    any are present, `record()` has already appended their lines to
    ///    the live file; renaming our stale tmp over it would truncate
    ///    those lines and lose just-journaled messages on the next crash
    ///    (audit of #3967). When that happens, abort this compaction
    ///    (the next tick retries) and clean up the tmp file.
    pub async fn compact(&self) {
        use std::collections::HashSet;
        let (path, snapshot_ids, entries) = {
            let inner = self.inner.lock().await;
            let path = inner.path.clone();
            let snapshot_ids: HashSet<String> = inner.pending.keys().cloned().collect();
            let entries: Vec<JournalEntry> = inner.pending.values().cloned().collect();
            (path, snapshot_ids, entries)
        };
        let tmp_path = path.with_extension(format!("jsonl.tmp.{}", std::process::id()));
        let remaining = entries.len();

        let tmp_for_write = tmp_path.clone();
        let write_join = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            let mut file = std::fs::File::create(&tmp_for_write)?;
            for entry in &entries {
                let line = serde_json::to_string(entry).map_err(std::io::Error::other)?;
                writeln!(file, "{line}")?;
            }
            file.flush()?;
            file.sync_all()?;
            Ok(())
        })
        .await;

        match write_join {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                error!(error = %e, "Journal compaction temp write failed");
                // Best-effort cleanup of the partial tmp file.
                let cleanup = tmp_path.clone();
                let _ = tokio::task::spawn_blocking(move || std::fs::remove_file(cleanup)).await;
                return;
            }
            Err(e) => {
                error!(error = %e, "spawn_blocking panicked compacting journal");
                let cleanup = tmp_path.clone();
                let _ = tokio::task::spawn_blocking(move || std::fs::remove_file(cleanup)).await;
                return;
            }
        }

        // Re-acquire the lock for the rename. `record()` holds this same
        // mutex across its disk-append spawn_blocking, so once we own it
        // again no append can interleave with our rename. We also detect
        // any append that happened *during* the slow write window above
        // and abort if so — otherwise the rename would overwrite those
        // freshly-journaled lines on disk.
        let inner = self.inner.lock().await;
        let raced: Vec<String> = inner
            .pending
            .keys()
            .filter(|id| !snapshot_ids.contains(*id))
            .cloned()
            .collect();
        if !raced.is_empty() {
            drop(inner);
            warn!(
                appended = raced.len(),
                "Journal compaction aborted: entries were appended during compact; will retry next cycle",
            );
            let cleanup = tmp_path.clone();
            let _ = tokio::task::spawn_blocking(move || std::fs::remove_file(cleanup)).await;
            return;
        }

        let path_for_rename = path.clone();
        let tmp_for_rename = tmp_path.clone();
        let rename_join =
            tokio::task::spawn_blocking(move || std::fs::rename(&tmp_for_rename, &path_for_rename))
                .await;
        drop(inner);

        match rename_join {
            Ok(Ok(())) => debug!(remaining, "Journal compacted"),
            Ok(Err(e)) => {
                error!(error = %e, "Journal compaction rename failed");
                let cleanup = tmp_path;
                let _ = tokio::task::spawn_blocking(move || std::fs::remove_file(cleanup)).await;
            }
            Err(e) => {
                error!(error = %e, "spawn_blocking panicked renaming journal");
                let cleanup = tmp_path;
                let _ = tokio::task::spawn_blocking(move || std::fs::remove_file(cleanup)).await;
            }
        }
    }

    /// Spawn a background task that compacts the journal every hour.
    pub fn spawn_compaction_timer(&self) {
        let journal = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
            interval.tick().await; // skip first immediate tick
            loop {
                interval.tick().await;
                journal.compact().await;
            }
        });
    }

    /// Append a pre-serialized JSON line to the journal file.
    ///
    /// Intended for use inside `tokio::task::spawn_blocking` so that the
    /// sync `OpenOptions::open` + `flush` calls do not stall the async runtime.
    fn write_line_to_path(path: &Path, line: &str) -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        writeln!(file, "{line}")?;
        file.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_entry(id: &str) -> JournalEntry {
        JournalEntry {
            message_id: id.to_string(),
            channel: "telegram".to_string(),
            sender_id: "12345".to_string(),
            sender_name: "TestUser".to_string(),
            content: "Hello world".to_string(),
            agent_name: Some("ambrogio".to_string()),
            received_at: Utc::now(),
            status: JournalStatus::Pending,
            attempts: 0,
            last_error: None,
            updated_at: Utc::now(),
            is_group: false,
            thread_id: None,
            metadata: HashMap::new(),
            next_retry_after: None,
        }
    }

    #[tokio::test]
    async fn test_record_and_pending() {
        let dir = TempDir::new().unwrap();
        let journal = MessageJournal::open(dir.path()).unwrap();

        journal.record(test_entry("msg-1")).await;
        journal.record(test_entry("msg-2")).await;

        let pending = journal.pending_entries().await;
        assert_eq!(pending.len(), 2);
    }

    #[tokio::test]
    async fn test_complete_removes_from_pending() {
        let dir = TempDir::new().unwrap();
        let journal = MessageJournal::open(dir.path()).unwrap();

        journal.record(test_entry("msg-1")).await;
        journal
            .update_status("msg-1", JournalStatus::Completed, None)
            .await;

        let pending = journal.pending_entries().await;
        assert!(pending.is_empty());
    }

    #[tokio::test]
    async fn test_persistence_across_reopens() {
        let dir = TempDir::new().unwrap();

        // First session: record two messages, complete one
        {
            let journal = MessageJournal::open(dir.path()).unwrap();
            journal.record(test_entry("msg-1")).await;
            journal.record(test_entry("msg-2")).await;
            journal
                .update_status("msg-1", JournalStatus::Completed, None)
                .await;
        }

        // Second session: only msg-2 should be pending (simulates crash recovery)
        {
            let journal = MessageJournal::open(dir.path()).unwrap();
            let pending = journal.pending_entries().await;
            assert_eq!(pending.len(), 1);
            assert_eq!(pending[0].message_id, "msg-2");
        }
    }

    #[tokio::test]
    async fn test_failed_entries_retry_limit() {
        let dir = TempDir::new().unwrap();
        let journal = MessageJournal::open(dir.path()).unwrap();

        journal.record(test_entry("msg-1")).await;
        // Fail 3 times
        for _ in 0..3 {
            journal
                .update_status("msg-1", JournalStatus::Failed, Some("timeout".to_string()))
                .await;
        }

        // After 3 failures, entry is removed from pending
        let pending = journal.pending_entries().await;
        assert!(pending.is_empty());
    }

    #[tokio::test]
    async fn test_compact() {
        let dir = TempDir::new().unwrap();
        let journal = MessageJournal::open(dir.path()).unwrap();

        journal.record(test_entry("msg-1")).await;
        journal.record(test_entry("msg-2")).await;
        journal.record(test_entry("msg-3")).await;
        journal
            .update_status("msg-1", JournalStatus::Completed, None)
            .await;
        journal
            .update_status("msg-3", JournalStatus::Completed, None)
            .await;

        // Compact: file should now only contain msg-2
        journal.compact().await;

        // Reopen and verify
        let journal2 = MessageJournal::open(dir.path()).unwrap();
        let pending = journal2.pending_entries().await;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].message_id, "msg-2");
    }

    #[tokio::test]
    async fn test_contains_dedup() {
        let dir = TempDir::new().unwrap();
        let journal = MessageJournal::open(dir.path()).unwrap();

        assert!(!journal.contains("msg-1").await);
        journal.record(test_entry("msg-1")).await;
        assert!(journal.contains("msg-1").await);
    }

    #[tokio::test]
    async fn compact_does_not_lose_entries_appended_during_window() {
        // Regression for the snapshot/rename race re-introduced when
        // compact() dropped the inner mutex before the spawn_blocking
        // fsync. If `record(D)` interleaves between compact's snapshot
        // and rename, D's appended line on disk must NOT be truncated by
        // the rename of the (stale) tmp file. Compact must either
        // include D in the rewrite or abort and clean up.
        let dir = TempDir::new().unwrap();
        let journal = MessageJournal::open(dir.path()).unwrap();

        journal.record(test_entry("msg-old")).await;

        // Run compact concurrently with a record(); the record may
        // observe an empty file (compact won) or its line preserved
        // (compact aborted), but never lose the entry.
        let j2 = journal.clone();
        let compactor = tokio::spawn(async move { j2.compact().await });
        // Tiny yield to let compact take its snapshot first; even if
        // ordering varies, the invariant still holds.
        tokio::task::yield_now().await;
        journal.record(test_entry("msg-new")).await;
        compactor.await.unwrap();

        // Reopen and verify: msg-new must be recoverable. msg-old must
        // also still be there because nothing completed it.
        let journal2 = MessageJournal::open(dir.path()).unwrap();
        let pending = journal2.pending_entries().await;
        let ids: std::collections::HashSet<String> =
            pending.iter().map(|e| e.message_id.clone()).collect();
        assert!(
            ids.contains("msg-new"),
            "msg-new lost across compact race: pending = {ids:?}"
        );
        assert!(
            ids.contains("msg-old"),
            "msg-old lost across compact race: pending = {ids:?}"
        );
    }

    #[test]
    fn parse_defer_marker_extracts_ms_or_none() {
        assert_eq!(
            parse_defer_marker("Rate limited after 3 retries [rate_limit_defer_ms]=300000"),
            Some(300_000)
        );
        assert_eq!(
            parse_defer_marker("Model overloaded [rate_limit_defer_ms]= 60000 trailing"),
            Some(60_000)
        );
        // Absent marker → None (treated as hard failure by bridge)
        assert_eq!(parse_defer_marker("Some other error"), None);
        // Present marker but missing value → None (do not trigger defer)
        assert_eq!(
            parse_defer_marker("Bad [rate_limit_defer_ms]= not a number"),
            None
        );
    }

    #[tokio::test]
    async fn defer_sets_next_retry_after_and_keeps_entry_pending() {
        let dir = TempDir::new().unwrap();
        let journal = MessageJournal::open(dir.path()).unwrap();

        journal.record(test_entry("msg-1")).await;
        journal
            .defer(
                "msg-1",
                chrono::Duration::seconds(120),
                Some("rate limited".into()),
            )
            .await;

        // pending_entries() must NOT include Deferred (only Pending+Processing).
        let pending = journal.pending_entries().await;
        assert!(pending.is_empty(), "Deferred should not appear in pending");

        // due_deferred_entries() with future deadline → empty.
        let due = journal.due_deferred_entries().await;
        assert!(due.is_empty(), "deadline not yet reached");

        // Recoverable view returns nothing yet either.
        let rec = journal.recoverable_entries().await;
        assert!(rec.is_empty());
    }

    #[tokio::test]
    async fn defer_with_past_deadline_appears_in_due_and_recoverable() {
        let dir = TempDir::new().unwrap();
        let journal = MessageJournal::open(dir.path()).unwrap();

        journal.record(test_entry("msg-1")).await;
        // Deferral with negative duration → deadline already in the past,
        // entry must show up in due_deferred immediately.
        journal
            .defer("msg-1", chrono::Duration::seconds(-1), None)
            .await;

        let due = journal.due_deferred_entries().await;
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].message_id, "msg-1");
        assert_eq!(due[0].status, JournalStatus::Deferred);

        let rec = journal.recoverable_entries().await;
        assert_eq!(rec.len(), 1);
    }

    #[tokio::test]
    async fn defer_does_not_consume_retry_budget() {
        let dir = TempDir::new().unwrap();
        let journal = MessageJournal::open(dir.path()).unwrap();

        journal.record(test_entry("msg-1")).await;
        // Defer five times — attempts must remain 0; deferring is not failure.
        for _ in 0..5 {
            journal
                .defer("msg-1", chrono::Duration::seconds(60), None)
                .await;
        }

        let due = journal
            .pending_entries()
            .await
            .into_iter()
            .chain(journal.due_deferred_entries().await)
            .collect::<Vec<_>>();
        // The deadline is 60s in the future so it shouldn't be due yet.
        // Reach into pending map directly via reopen+inspect-only path.
        let _ = due;
        let inner = journal.inner.lock().await;
        let entry = inner.pending.get("msg-1").expect("entry retained");
        assert_eq!(entry.attempts, 0, "defer must not bump attempts");
        assert_eq!(entry.status, JournalStatus::Deferred);
        assert!(entry.next_retry_after.is_some());
    }

    #[tokio::test]
    async fn deferred_entry_persists_across_reopen() {
        let dir = TempDir::new().unwrap();
        {
            let journal = MessageJournal::open(dir.path()).unwrap();
            journal.record(test_entry("msg-1")).await;
            journal
                .defer("msg-1", chrono::Duration::seconds(-1), None)
                .await;
        }
        // Re-open: Deferred entry must survive (waiting on quota reset across
        // a daemon restart is the entire point of the feature).
        let journal2 = MessageJournal::open(dir.path()).unwrap();
        let due = journal2.due_deferred_entries().await;
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].message_id, "msg-1");
    }

    #[tokio::test]
    async fn processing_claim_guard_removes_entry_from_due_deferred() {
        // Regression: redispatch_journal_entry flips the entry to Processing
        // before the slow LLM call to prevent a second ticker tick from
        // re-claiming the same entry mid-flight (double-dispatch race).
        // Once Processing, due_deferred_entries() must NOT return it.
        let dir = TempDir::new().unwrap();
        let journal = MessageJournal::open(dir.path()).unwrap();
        journal.record(test_entry("msg-1")).await;
        journal
            .defer("msg-1", chrono::Duration::seconds(-1), None)
            .await;
        // Sanity: deferred entry IS due before claim.
        assert_eq!(journal.due_deferred_entries().await.len(), 1);
        // Claim guard.
        journal
            .update_status("msg-1", JournalStatus::Processing, None)
            .await;
        // After claim, it must no longer appear in the due-deferred view —
        // a concurrent ticker tick would have skipped it.
        assert!(journal.due_deferred_entries().await.is_empty());
        // It should appear in pending instead (Processing status).
        let pending = journal.pending_entries().await;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].status, JournalStatus::Processing);
        // Claim guard must also have cleared the deadline.
        assert!(pending[0].next_retry_after.is_none());
    }

    #[tokio::test]
    async fn deferred_clears_retry_deadline_on_completion() {
        let dir = TempDir::new().unwrap();
        let journal = MessageJournal::open(dir.path()).unwrap();
        journal.record(test_entry("msg-1")).await;
        journal
            .defer("msg-1", chrono::Duration::seconds(-1), None)
            .await;
        // After successful retry the entry is Completed and removed from
        // pending; reopen confirms it's not coming back as due-deferred.
        journal
            .update_status("msg-1", JournalStatus::Completed, None)
            .await;
        let journal2 = MessageJournal::open(dir.path()).unwrap();
        assert!(journal2.due_deferred_entries().await.is_empty());
        assert!(journal2.pending_entries().await.is_empty());
    }

    #[tokio::test]
    async fn test_processing_entries_recovered_on_reopen() {
        let dir = TempDir::new().unwrap();

        // First session: record and mark as processing (simulates in-flight at crash)
        {
            let journal = MessageJournal::open(dir.path()).unwrap();
            journal.record(test_entry("msg-1")).await;
            journal
                .update_status("msg-1", JournalStatus::Processing, None)
                .await;
        }

        // Second session: processing entries should appear in pending
        {
            let journal = MessageJournal::open(dir.path()).unwrap();
            let pending = journal.pending_entries().await;
            assert_eq!(pending.len(), 1);
            assert_eq!(pending[0].message_id, "msg-1");
            assert_eq!(pending[0].status, JournalStatus::Processing);
        }
    }

    #[tokio::test]
    async fn claim_rejects_already_claimed_entry() {
        // Sequential proof of CAS semantics: the second claim observes
        // Processing and bails. Concurrent variant below.
        let dir = TempDir::new().unwrap();
        let journal = MessageJournal::open(dir.path()).unwrap();
        let mut e = test_entry("msg-1");
        e.status = JournalStatus::Deferred;
        e.next_retry_after = Some(Utc::now() - chrono::Duration::seconds(5));
        journal.record(e).await;

        assert!(journal.claim("msg-1").await, "first claim must win");
        assert!(
            !journal.claim("msg-1").await,
            "second claim against the same entry must lose"
        );

        // Entry is now Processing in-memory; status reflects the won claim.
        let pending = journal.pending_entries().await;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].status, JournalStatus::Processing);
    }

    #[tokio::test]
    async fn claim_returns_false_for_unknown_message() {
        let dir = TempDir::new().unwrap();
        let journal = MessageJournal::open(dir.path()).unwrap();
        assert!(
            !journal.claim("does-not-exist").await,
            "claiming a missing message must return false, not panic"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_claims_resolve_to_exactly_one_winner() {
        // Multi-thread runtime + spawned tasks pin true contention. The
        // single-threaded sequential variant above proves CAS semantics
        // in isolation; this one proves the inner mutex serializes the
        // claim race correctly under load.
        let dir = TempDir::new().unwrap();
        let journal = Arc::new(MessageJournal::open(dir.path()).unwrap());
        let mut e = test_entry("msg-1");
        e.status = JournalStatus::Deferred;
        e.next_retry_after = Some(Utc::now() - chrono::Duration::seconds(5));
        journal.record(e).await;

        let mut handles = Vec::new();
        for _ in 0..8 {
            let j = Arc::clone(&journal);
            handles.push(tokio::spawn(async move { j.claim("msg-1").await }));
        }

        let mut wins = 0usize;
        for h in handles {
            if h.await.unwrap() {
                wins += 1;
            }
        }
        assert_eq!(
            wins, 1,
            "exactly one of 8 concurrent claims must win; the rest must observe Processing"
        );
    }
}
