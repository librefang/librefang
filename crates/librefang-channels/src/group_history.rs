//! Group-message retention buffer for messages skipped by gating.
//!
//! In `mention_required` group mode, messages that don't address
//! the agent are dropped at gating time. If they carry attachments (PDF,
//! image, voice) those attachments are lost — when the agent is finally
//! addressed and asked about prior context ("look at the attachment mum
//! sent"), it has nothing to reference.
//!
//! This module keeps a small per-group ring buffer of skipped messages
//! (text + already-downloaded `ContentBlock` extras when available) for
//! retrieval at the next gating-pass on the same group. Storage is
//! in-memory only — restart-volatile by design, since the rolling window
//! is short (24h default) and SQLite persistence + cleanup adds surface
//! we don't need yet. Move to disk only if the operator measures that
//! restart loss matters.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use librefang_types::message::ContentBlock;
use tokio::sync::RwLock;

/// Process-wide singleton, set on first `BridgeManager` construction
/// so the deep dispatch path can reach the buffer without threading
/// it through every helper signature. The buffer is stateless across
/// agents (it's keyed by `(channel_type, group_jid)`) so a single
/// shared instance is correct semantics.
static GLOBAL_BUFFER: OnceLock<Arc<GroupHistoryBuffer>> = OnceLock::new();

/// Install (or fetch) the process-wide buffer. Idempotent — calling
/// twice with different instances keeps the first one (the stored
/// closure is `get_or_init`).
pub fn install_global(default: Arc<GroupHistoryBuffer>) -> Arc<GroupHistoryBuffer> {
    GLOBAL_BUFFER.get_or_init(|| default).clone()
}

/// Read the process-wide buffer if installed. `None` in tests that
/// never construct a `BridgeManager`. Callers must handle the `None`
/// case as "no buffer wired — treat as no-op".
pub fn global() -> Option<Arc<GroupHistoryBuffer>> {
    GLOBAL_BUFFER.get().cloned()
}

/// Build the canonical group key from a `ChannelType` and the
/// platform-specific group identifier (chat JID, channel id, etc).
/// Same channel-type prefix used everywhere else in the bridge so
/// keys are interoperable with the rest of the codebase's logging.
pub fn group_key(channel_type_str: &str, group_jid: &str) -> String {
    format!("{channel_type_str}|{group_jid}")
}

/// Default retention window for buffered group messages.
///
/// 24h matches the WhatsApp gateway's own `pending_group_history` TTL
/// from the pre-Phase-07 design (history of skipped messages); long enough to
/// cover a "this morning's bolletta arrived, agent re-engaged this
/// evening" gap, short enough that the buffer doesn't grow without
/// bound on noisy groups.
pub const DEFAULT_RETENTION: Duration = Duration::from_secs(24 * 60 * 60);

/// Hard ceiling on entries retained per group. Beyond this we drop the
/// oldest on insert. Prevents a runaway noisy group from consuming all
/// memory in pathological cases (e.g. a 5k-member announcement channel
/// where the bot was added but never addressed).
pub const MAX_ENTRIES_PER_GROUP: usize = 100;

/// Hard ceiling on rendered preamble length so we don't blow the LLM
/// context with 100 buffered messages of small talk on the next address.
pub const MAX_RENDER_CHARS: usize = 4000;

/// One captured skipped message.
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub sender_display_name: String,
    pub sender_platform_id: String,
    /// Plain-text rendition of the message (caption for media, body for
    /// text). Empty string if the message had no extractable text.
    pub text: String,
    /// Media blocks the channel adapter already prepared for the LLM,
    /// most importantly `ContentBlock::ImageFile { path }` referencing
    /// files saved by the channel-download path. Empty for plain-text
    /// messages.
    pub content_blocks: Vec<ContentBlock>,
    /// Insert time — used both for ordering on render and for TTL eviction.
    pub captured_at: Instant,
}

/// Process-wide in-memory buffer of skipped group messages.
///
/// Cloneable via `Arc`; one instance is created at bridge init and
/// shared by every adapter task that needs to record skips or drain on
/// pass.
#[derive(Debug, Clone, Default)]
pub struct GroupHistoryBuffer {
    inner: Arc<RwLock<Inner>>,
    retention: Duration,
}

#[derive(Debug, Default)]
struct Inner {
    /// Keyed by `group_key = format!("{}|{}", channel_type, group_jid)`
    /// so two channels with overlapping group ids stay isolated.
    by_group: HashMap<String, Vec<HistoryEntry>>,
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
            bucket.remove(0);
        }
        bucket.push(entry);
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
    /// memory pinned.
    pub async fn evict_expired(&self) {
        let cutoff = Instant::now().checked_sub(self.retention);
        let Some(cutoff) = cutoff else { return };
        let mut inner = self.inner.write().await;
        inner.by_group.retain(|_, bucket| {
            bucket.retain(|e| e.captured_at >= cutoff);
            !bucket.is_empty()
        });
    }

    /// Test-only inspection of bucket sizes. Not exposed in production.
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

/// Build the `[GROUP_CONTEXT]` preamble injected before the user
/// message on a gating-pass that has buffered prior skipped messages.
///
/// Keeps the rendering deterministic (chronological order, capped at
/// `MAX_RENDER_CHARS`) so the LLM prompt cache stays warm across turns.
pub fn render_group_context(entries: &[HistoryEntry]) -> Option<String> {
    if entries.is_empty() {
        return None;
    }

    let mut out = String::new();
    out.push_str("[GROUP_CONTEXT — messages received while you were not addressed]\n");

    let mut sorted: Vec<&HistoryEntry> = entries.iter().collect();
    sorted.sort_by_key(|e| e.captured_at);

    for entry in sorted {
        let line = format_entry(entry);
        if out.len() + line.len() > MAX_RENDER_CHARS {
            out.push_str(&format!(
                "• ... ({} more messages truncated)\n",
                entries.len() - count_in_buffer(&out)
            ));
            break;
        }
        out.push_str(&line);
    }
    out.push_str("[/GROUP_CONTEXT]\n");
    Some(out)
}

fn format_entry(entry: &HistoryEntry) -> String {
    let mut bullet = format!("• {}: ", entry.sender_display_name);
    if !entry.text.is_empty() {
        bullet.push_str(&entry.text);
    }
    if !entry.content_blocks.is_empty() {
        let media_count = entry
            .content_blocks
            .iter()
            .filter(|b| {
                matches!(
                    b,
                    ContentBlock::Image { .. } | ContentBlock::ImageFile { .. }
                )
            })
            .count();
        if media_count > 0 {
            if !entry.text.is_empty() {
                bullet.push(' ');
            }
            bullet.push_str(&format!("[+{media_count} attachment(s)]"));
        }
    }
    bullet.push('\n');
    bullet
}

fn count_in_buffer(rendered: &str) -> usize {
    rendered.lines().filter(|l| l.starts_with("• ")).count()
}

/// Extract the live `ContentBlock`s (image/imagefile) from a list of
/// drained entries — the bridge then prepends them to the user message
/// so the agent has the actual media available, not just the text
/// reference.
pub fn collect_media_blocks(entries: &[HistoryEntry]) -> Vec<ContentBlock> {
    entries
        .iter()
        .flat_map(|e| e.content_blocks.iter().cloned())
        .filter(|b| {
            matches!(
                b,
                ContentBlock::Image { .. } | ContentBlock::ImageFile { .. }
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(sender: &str, text: &str) -> HistoryEntry {
        HistoryEntry {
            sender_display_name: sender.into(),
            sender_platform_id: format!("{sender}-id"),
            text: text.into(),
            content_blocks: Vec::new(),
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
        // First 5 inserted should have been dropped — oldest survivor is m5.
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

    #[test]
    fn render_group_context_empty_returns_none() {
        assert!(render_group_context(&[]).is_none());
    }

    #[test]
    fn render_group_context_chronological_ordering() {
        let mut e1 = entry("Alice", "first");
        let mut e2 = entry("Bob", "second");
        // Force ordering: e2 must sort after e1.
        e1.captured_at = Instant::now();
        e2.captured_at = e1.captured_at + Duration::from_millis(1);
        let rendered = render_group_context(&[e2, e1]).expect("rendered");
        // first should appear before second despite reversed input order.
        let first_idx = rendered.find("first").expect("first present");
        let second_idx = rendered.find("second").expect("second present");
        assert!(first_idx < second_idx);
    }

    #[test]
    fn render_group_context_marks_attachments() {
        let entry_with_image = HistoryEntry {
            sender_display_name: "Patrizia".into(),
            sender_platform_id: "patrizia-id".into(),
            text: "bolletta".into(),
            content_blocks: vec![ContentBlock::ImageFile {
                path: "/tmp/librefang_uploads/x.jpg".into(),
                media_type: "image/jpeg".into(),
            }],
            captured_at: Instant::now(),
        };
        let out = render_group_context(&[entry_with_image]).expect("rendered");
        assert!(out.contains("[+1 attachment(s)]"));
    }

    #[test]
    fn collect_media_blocks_filters_text() {
        let mut e = entry("Alice", "with image");
        e.content_blocks = vec![
            ContentBlock::Text {
                text: "should be filtered out".into(),
                provider_metadata: None,
            },
            ContentBlock::ImageFile {
                path: "/tmp/x.jpg".into(),
                media_type: "image/jpeg".into(),
            },
        ];
        let blocks = collect_media_blocks(&[e]);
        assert_eq!(blocks.len(), 1, "Text blocks excluded");
        assert!(matches!(blocks[0], ContentBlock::ImageFile { .. }));
    }
}
