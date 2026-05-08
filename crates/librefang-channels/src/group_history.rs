//! Text-only retention buffer for group messages skipped by gating.
//!
//! In `mention_required` group mode, plain-text messages that don't address
//! the agent are dropped at gating time. When the agent is finally addressed
//! and asked about prior context ("did mum say something earlier?"), it has
//! nothing to reference. This buffer captures sender + text of each
//! gated-out plain-text message so a follow-up enrichment pass can stitch
//! them into the prompt.
//!
//! v1 scope is **plumbing only**: record + drain are wired in the bridge,
//! the prompt-builder is a follow-up PR. Storage is in-memory,
//! restart-volatile by design.
//!
//! Media-bearing messages (image/voice/video/file) bypass the gating skip
//! today — `dispatch_with_blocks` doesn't call `should_process_group_message`
//! — so they're not seen by this buffer. When the media path is gated in
//! a future PR, captions and `Vec<ContentBlock>` extras can be added back
//! to `HistoryEntry`.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use tokio::sync::RwLock;

/// Process-wide singleton, populated lazily on the first
/// `BridgeManager` construction.
static GLOBAL_BUFFER: OnceLock<Arc<GroupHistoryBuffer>> = OnceLock::new();

/// Install (or fetch) the process-wide buffer. The closure runs only on
/// the first call — second-and-later constructions reuse the existing
/// instance without allocating.
pub fn install_global<F>(default: F) -> Arc<GroupHistoryBuffer>
where
    F: FnOnce() -> Arc<GroupHistoryBuffer>,
{
    GLOBAL_BUFFER.get_or_init(default).clone()
}

/// Read the process-wide buffer if installed. `None` in unit tests that
/// don't construct a `BridgeManager`. Callers must handle `None` as
/// "no buffer wired — record/drain are no-ops".
pub fn global() -> Option<Arc<GroupHistoryBuffer>> {
    GLOBAL_BUFFER.get().cloned()
}

/// Build the canonical group key from a `ChannelType` string and the
/// platform-specific group identifier (chat JID, channel id, etc).
pub fn group_key(channel_type_str: &str, group_jid: &str) -> String {
    format!("{channel_type_str}|{group_jid}")
}

/// Default retention window for buffered group messages.
pub const DEFAULT_RETENTION: Duration = Duration::from_secs(24 * 60 * 60);

/// Per-bucket cap. Beyond this we drop the oldest on insert.
pub const MAX_ENTRIES_PER_GROUP: usize = 100;

/// One captured skipped message.
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub sender_display_name: String,
    /// Plain-text body. Empty entries are never recorded (the bridge
    /// skips messages that yield no extractable text).
    pub text: String,
    /// Insert time — used for chronological ordering on render and for
    /// TTL eviction.
    pub captured_at: Instant,
}

/// Process-wide in-memory buffer of skipped group messages.
#[derive(Debug, Clone)]
pub struct GroupHistoryBuffer {
    inner: Arc<RwLock<Inner>>,
    retention: Duration,
}

#[derive(Debug, Default)]
struct Inner {
    /// Keyed by `group_key = format!("{}|{}", channel_type, group_jid)`
    /// so two channels with overlapping group ids stay isolated.
    /// `VecDeque` so cap-overflow eviction is O(1) instead of O(n).
    by_group: HashMap<String, VecDeque<HistoryEntry>>,
}

impl GroupHistoryBuffer {
    pub fn new(retention: Duration) -> Self {
        Self {
            inner: Arc::new(RwLock::new(Inner::default())),
            retention,
        }
    }

    pub fn with_default_retention() -> Self {
        Self::new(DEFAULT_RETENTION)
    }

    /// Append a skipped message to the buffer for `group_key`.
    /// Drops the oldest entry when the per-group cap is hit so a
    /// pathologically active group can't OOM the daemon.
    pub async fn record(&self, group_key: &str, entry: HistoryEntry) {
        let mut inner = self.inner.write().await;
        let bucket = inner.by_group.entry(group_key.to_string()).or_default();
        if bucket.len() >= MAX_ENTRIES_PER_GROUP {
            bucket.pop_front();
        }
        bucket.push_back(entry);
    }

    /// Drain all live entries for `group_key`, evicting expired ones in
    /// the process. Returns `None` when there's nothing to render.
    pub async fn drain(&self, group_key: &str) -> Option<Vec<HistoryEntry>> {
        let mut inner = self.inner.write().await;
        let bucket = inner.by_group.remove(group_key)?;
        let cutoff = Instant::now().checked_sub(self.retention);
        let live: Vec<HistoryEntry> = bucket
            .into_iter()
            .filter(|e| match cutoff {
                Some(c) => e.captured_at >= c,
                None => true,
            })
            .collect();
        if live.is_empty() {
            None
        } else {
            Some(live)
        }
    }

    /// Periodic sweep: drop expired entries from every bucket so a group
    /// that goes quiet without ever being addressed doesn't keep stale
    /// memory pinned. Driven by the bridge's evictor task; safe to call
    /// from anywhere.
    pub async fn evict_expired(&self) {
        let cutoff = Instant::now().checked_sub(self.retention);
        let Some(cutoff) = cutoff else { return };
        let mut inner = self.inner.write().await;
        inner.by_group.retain(|_, bucket| {
            bucket.retain(|e| e.captured_at >= cutoff);
            !bucket.is_empty()
        });
    }

    /// Number of live buckets (one per `(channel, group_jid)`).
    /// Exposed for ops metrics.
    pub async fn bucket_count(&self) -> usize {
        self.inner.read().await.by_group.len()
    }

    /// Total entries across all buckets. Exposed for ops metrics.
    pub async fn entries_total(&self) -> usize {
        self.inner
            .read()
            .await
            .by_group
            .values()
            .map(|b| b.len())
            .sum()
    }

    #[cfg(test)]
    pub async fn bucket_size(&self, group_key: &str) -> usize {
        self.inner
            .read()
            .await
            .by_group
            .get(group_key)
            .map(|v| v.len())
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(sender: &str, text: &str) -> HistoryEntry {
        HistoryEntry {
            sender_display_name: sender.into(),
            text: text.into(),
            captured_at: Instant::now(),
        }
    }

    #[tokio::test]
    async fn record_and_drain_round_trips() {
        let buf = GroupHistoryBuffer::with_default_retention();
        buf.record("ct|grp1", entry("Alice", "ciao")).await;
        buf.record("ct|grp1", entry("Bob", "tutto bene?")).await;
        let drained = buf.drain("ct|grp1").await.expect("drained");
        assert_eq!(drained.len(), 2);
        assert_eq!(buf.bucket_size("ct|grp1").await, 0);
    }

    #[tokio::test]
    async fn drain_isolates_groups() {
        let buf = GroupHistoryBuffer::with_default_retention();
        buf.record("ct|grp1", entry("Alice", "msg1")).await;
        buf.record("ct|grp2", entry("Bob", "msg2")).await;
        let g1 = buf.drain("ct|grp1").await.expect("g1");
        assert_eq!(g1.len(), 1);
        assert_eq!(g1[0].sender_display_name, "Alice");
        let g2 = buf.drain("ct|grp2").await.expect("g2");
        assert_eq!(g2.len(), 1);
        assert_eq!(g2[0].sender_display_name, "Bob");
    }

    #[tokio::test]
    async fn drain_returns_none_when_empty() {
        let buf = GroupHistoryBuffer::with_default_retention();
        assert!(buf.drain("ct|grp1").await.is_none());
    }

    #[tokio::test]
    async fn cap_drops_oldest_on_overflow() {
        let buf = GroupHistoryBuffer::with_default_retention();
        for i in 0..(MAX_ENTRIES_PER_GROUP + 5) {
            buf.record("ct|grp1", entry("X", &format!("m{i}"))).await;
        }
        assert_eq!(
            buf.bucket_size("ct|grp1").await,
            MAX_ENTRIES_PER_GROUP,
            "bucket bounded at MAX_ENTRIES_PER_GROUP",
        );
        let drained = buf.drain("ct|grp1").await.expect("drained");
        assert_eq!(drained.first().unwrap().text, "m5");
    }

    #[tokio::test]
    async fn evict_expired_clears_old_entries() {
        let buf = GroupHistoryBuffer::new(Duration::from_millis(50));
        buf.record("ct|grp1", entry("Alice", "old")).await;
        tokio::time::sleep(Duration::from_millis(80)).await;
        buf.evict_expired().await;
        assert_eq!(buf.bucket_size("ct|grp1").await, 0);
    }

    #[tokio::test]
    async fn metrics_reflect_buffer_state() {
        let buf = GroupHistoryBuffer::with_default_retention();
        assert_eq!(buf.bucket_count().await, 0);
        assert_eq!(buf.entries_total().await, 0);
        buf.record("ct|grp1", entry("Alice", "a")).await;
        buf.record("ct|grp1", entry("Bob", "b")).await;
        buf.record("ct|grp2", entry("Carol", "c")).await;
        assert_eq!(buf.bucket_count().await, 2);
        assert_eq!(buf.entries_total().await, 3);
    }
}
