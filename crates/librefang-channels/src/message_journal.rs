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
                            JournalStatus::Failed => {
                                // Keep failed entries if under retry limit
                                if entry.attempts < 3 {
                                    pending.insert(entry.message_id.clone(), entry);
                                } else {
                                    pending.remove(&entry.message_id);
                                }
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

    /// Maximum age of a message to be eligible for recovery (1 hour).
    const MAX_RECOVERY_AGE: chrono::TimeDelta = match chrono::TimeDelta::try_hours(1) {
        Some(d) => d,
        None => unreachable!(),
    };

    /// Get all entries that need (re-)processing.
    /// Returns entries with status Pending or Processing (from a previous crash).
    /// Skips entries older than 1 hour — they are too stale to recover.
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
            debug!(id, "Discarding stale journal entry (>1h old)");
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
}
