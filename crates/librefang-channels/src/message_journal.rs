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
    /// The file write is executed via `spawn_blocking` so that slow disk I/O
    /// (e.g. fsync on a busy volume) does not stall the async runtime while
    /// the mutex is held.  The in-memory index is updated only after the
    /// write completes, preserving the WAL invariant.
    pub async fn record(&self, entry: JournalEntry) {
        let path = {
            let inner = self.inner.lock().await;
            inner.path.clone()
        };
        let line = match serde_json::to_string(&entry) {
            Ok(l) => l,
            Err(e) => {
                error!(error = %e, id = %entry.message_id, "Failed to serialize journal entry");
                return;
            }
        };
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
        let mut inner = self.inner.lock().await;
        inner.pending.insert(entry.message_id.clone(), entry);
    }

    /// Update the status of an existing entry.
    ///
    /// Like `record`, the disk write runs in `spawn_blocking` so that a slow
    /// fsync cannot block the async runtime while the mutex is held.
    pub async fn update_status(
        &self,
        message_id: &str,
        status: JournalStatus,
        error: Option<String>,
    ) {
        // Snapshot the updated entry under the lock, then release before I/O.
        let (updated, path, should_remove) = {
            let mut inner = self.inner.lock().await;
            let entry = match inner.pending.get_mut(message_id) {
                Some(e) => e,
                None => return,
            };
            entry.status = status;
            entry.updated_at = Utc::now();
            if status == JournalStatus::Failed {
                entry.attempts += 1;
                entry.last_error = error;
            }
            let updated = entry.clone();
            let should_remove = status == JournalStatus::Completed
                || (status == JournalStatus::Failed && updated.attempts >= 3);
            (updated, inner.path.clone(), should_remove)
        };

        // Write without holding the mutex.
        let line = match serde_json::to_string(&updated) {
            Ok(l) => l,
            Err(e) => {
                error!(error = %e, id = message_id, "Failed to serialize journal update");
                return;
            }
        };
        let write_result =
            tokio::task::spawn_blocking(move || Self::write_line_to_path(&path, &line)).await;
        match write_result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                error!(error = %e, id = message_id, "Failed to update journal entry");
            }
            Err(e) => {
                error!(error = %e, id = message_id, "spawn_blocking panicked updating journal");
            }
        }

        // Remove from in-memory index if terminal.
        if should_remove {
            let mut inner = self.inner.lock().await;
            inner.pending.remove(message_id);
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
    pub async fn compact(&self) {
        let inner = self.inner.lock().await;
        let tmp_path = inner.path.with_extension("jsonl.tmp");
        let result = (|| -> std::io::Result<()> {
            let mut file = std::fs::File::create(&tmp_path)?;
            for entry in inner.pending.values() {
                let line = serde_json::to_string(entry).map_err(std::io::Error::other)?;
                writeln!(file, "{line}")?;
            }
            file.flush()?;
            std::fs::rename(&tmp_path, &inner.path)?;
            Ok(())
        })();
        match result {
            Ok(()) => debug!(remaining = inner.pending.len(), "Journal compacted"),
            Err(e) => error!(error = %e, "Journal compaction failed"),
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
