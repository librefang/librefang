//! Per-session `file_read` deduplication tracker (#4971).
//!
//! Agents frequently re-read the same config / source file across multiple
//! turns. Each `file_read` tool result stores the full body in history, so a
//! 10 KB file read four times consumes ~40 KB of context. This module tracks
//! `(path, sha256, turn_id)` per session and rewrites repeat reads into short
//! stubs:
//!
//! - Same path, same hash → `[File already read — content unchanged since
//!   turn N. See above for full content.]`
//! - Same path, different hash → original content prefixed with
//!   `[File updated since last read at turn N]\n\n`
//! - New path → return content unchanged and record.
//!
//! State is keyed by [`SessionId`] and cleared when the context compressor
//! summarises history (the prior bodies are gone, so the stub would point at
//! nothing). Insertion / eviction also keep a small recency-driven cap to
//! bound memory for long-lived sessions.
//!
//! # Determinism
//!
//! Tracker maps use `BTreeMap` so iteration is stable across processes — this
//! aligns with the prompt-cache invariants documented in `CLAUDE.md`.

use librefang_types::agent::SessionId;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Hard cap on tracked paths per session. Bounded purely for memory safety on
/// pathological agents that touch tens of thousands of unique files; under
/// the cap older entries are evicted by `turn_id` so the freshest reads win.
const MAX_TRACKED_PATHS: usize = 512;

/// One recorded read.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FileReadEntry {
    /// sha256 of the file content at the time of the recorded read.
    content_hash: [u8; 32],
    /// 1-based turn / read counter at the time of the recorded read.
    turn_id: u64,
}

/// What [`FileReadTracker::observe`] decided to do with this read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadOutcome {
    /// First time we have seen this path in this session (or dedup disabled,
    /// or after a reset). Caller returns the full content unchanged.
    First,
    /// Same path, identical content hash. Caller returns the
    /// `[File already read — content unchanged since turn N. See above for
    /// full content.]` stub.
    Unchanged { first_turn: u64 },
    /// Same path, content changed since the previous read. Caller returns the
    /// full content prefixed with `[File updated since last read at turn N]`.
    Changed { previous_turn: u64 },
}

/// Per-session tracker — see module docs.
#[derive(Debug)]
pub struct FileReadTracker {
    entries: BTreeMap<PathBuf, FileReadEntry>,
    /// Monotonic counter — incremented on every `observe` call regardless of
    /// outcome so each recorded `turn_id` is unique within the session and the
    /// human-readable "turn N" stub stays meaningful even across re-reads.
    next_turn: u64,
}

impl Default for FileReadTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl FileReadTracker {
    /// Empty tracker.
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            next_turn: 1,
        }
    }

    /// Wipe all state. Called when context compression fires (#4971: prior
    /// bodies are no longer in history so stubs would dangle).
    pub fn reset(&mut self) {
        self.entries.clear();
        self.next_turn = 1;
    }

    /// Number of paths currently tracked. Test helper.
    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }

    /// Record a read of `path` containing `content`, returning what the caller
    /// should send back to the model.
    pub fn observe(&mut self, path: &Path, content: &str) -> ReadOutcome {
        let hash = Self::hash(content);
        let turn = self.next_turn;
        self.next_turn = self.next_turn.saturating_add(1);

        let key = path.to_path_buf();
        let outcome = match self.entries.get(&key) {
            Some(prev) if prev.content_hash == hash => ReadOutcome::Unchanged {
                first_turn: prev.turn_id,
            },
            Some(prev) => ReadOutcome::Changed {
                previous_turn: prev.turn_id,
            },
            None => ReadOutcome::First,
        };

        // Only First and Changed write a new entry. Unchanged must leave the
        // recorded turn_id anchored to the turn that actually carried the
        // file body — otherwise the stub on call N points at call N-1, which
        // is itself a stub, and the "see above for full content" trail
        // drifts forward away from the real content.
        if !matches!(outcome, ReadOutcome::Unchanged { .. }) {
            self.entries.insert(
                key,
                FileReadEntry {
                    content_hash: hash,
                    turn_id: turn,
                },
            );
        }

        // Bound memory growth: when above the cap, drop the lowest-turn (i.e.
        // oldest-recorded) entry. BTreeMap iteration is by key, not insertion
        // order, so we have to scan to find the oldest — fine at this size.
        while self.entries.len() > MAX_TRACKED_PATHS {
            if let Some(oldest_key) = self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.turn_id)
                .map(|(k, _)| k.clone())
            {
                self.entries.remove(&oldest_key);
            } else {
                break;
            }
        }

        outcome
    }

    fn hash(content: &str) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(content.as_bytes());
        h.finalize().into()
    }
}

/// Render the "unchanged" stub message that the caller substitutes for the
/// full file body.
pub fn unchanged_stub(first_turn: u64) -> String {
    format!(
        "[File already read — content unchanged since turn {first_turn}. See above for full content.]"
    )
}

/// Prefix prepended to the full body when the file changed since the previous
/// read in this session.
pub fn changed_header(previous_turn: u64) -> String {
    format!("[File updated since last read at turn {previous_turn}]")
}

// ─── Process-wide registry ───────────────────────────────────────────────

// The per-session registry is keyed by [`SessionId`] (a UUID newtype that is
// `Hash + Eq` but not `Ord`). Iteration order doesn't influence prompts —
// callers always look up a single session — so `HashMap` is fine here. The
// inner `FileReadTracker` keeps its own `BTreeMap` for prompt-stable behaviour.
fn registry() -> &'static Mutex<HashMap<SessionId, FileReadTracker>> {
    static REG: OnceLock<Mutex<HashMap<SessionId, FileReadTracker>>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Run `f` against the tracker for `session_id`, creating an empty tracker on
/// first access. The mutex is held across `f` so callers should keep the
/// closure short.
pub fn with_session<R>(session_id: SessionId, f: impl FnOnce(&mut FileReadTracker) -> R) -> R {
    let mut guard = registry().lock().unwrap_or_else(|p| p.into_inner());
    let tracker = guard.entry(session_id).or_default();
    f(tracker)
}

/// Clear the tracker for `session_id`. No-op if it doesn't exist.
///
/// Called from the context compressor (#4971) after a successful compression
/// pass: the prior full file bodies have been summarised away, so the
/// "see above for full content" stub would dangle if we kept the state.
pub fn reset_session(session_id: SessionId) {
    let mut guard = registry().lock().unwrap_or_else(|p| p.into_inner());
    if let Some(t) = guard.get_mut(&session_id) {
        t.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn first_read_is_first_outcome() {
        let mut t = FileReadTracker::new();
        let outcome = t.observe(&PathBuf::from("/tmp/a"), "hello");
        assert_eq!(outcome, ReadOutcome::First);
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn unchanged_read_returns_unchanged_with_first_turn() {
        let mut t = FileReadTracker::new();
        let first = t.observe(&PathBuf::from("/tmp/a"), "hello");
        assert_eq!(first, ReadOutcome::First);
        let second = t.observe(&PathBuf::from("/tmp/a"), "hello");
        // First call recorded turn=1; second call sees that entry.
        assert_eq!(second, ReadOutcome::Unchanged { first_turn: 1 });
    }

    #[test]
    fn unchanged_anchor_does_not_drift_forward() {
        // Repeated unchanged reads must all point back to the FIRST turn
        // that actually stored the file body. If the anchor advanced on each
        // unchanged observation, the stub on call N would reference call N-1
        // (itself a stub), losing the trail to the turn that carries the
        // real content.
        let mut t = FileReadTracker::new();
        let r1 = t.observe(&PathBuf::from("/tmp/a"), "hello");
        assert_eq!(r1, ReadOutcome::First);
        let r2 = t.observe(&PathBuf::from("/tmp/a"), "hello");
        assert_eq!(r2, ReadOutcome::Unchanged { first_turn: 1 });
        let r3 = t.observe(&PathBuf::from("/tmp/a"), "hello");
        assert_eq!(
            r3,
            ReadOutcome::Unchanged { first_turn: 1 },
            "anchor must stay on turn 1 — the only turn carrying full content"
        );
        let r4 = t.observe(&PathBuf::from("/tmp/a"), "hello");
        assert_eq!(r4, ReadOutcome::Unchanged { first_turn: 1 });
    }

    #[test]
    fn changed_read_returns_changed_with_previous_turn() {
        let mut t = FileReadTracker::new();
        t.observe(&PathBuf::from("/tmp/a"), "hello");
        let second = t.observe(&PathBuf::from("/tmp/a"), "world");
        assert_eq!(second, ReadOutcome::Changed { previous_turn: 1 });
        // Third read with the new content is now "unchanged since turn 2",
        // proving the entry was overwritten on the previous call.
        let third = t.observe(&PathBuf::from("/tmp/a"), "world");
        assert_eq!(third, ReadOutcome::Unchanged { first_turn: 2 });
    }

    #[test]
    fn different_paths_tracked_separately() {
        let mut t = FileReadTracker::new();
        assert_eq!(
            t.observe(&PathBuf::from("/tmp/a"), "aaa"),
            ReadOutcome::First
        );
        assert_eq!(
            t.observe(&PathBuf::from("/tmp/b"), "aaa"),
            ReadOutcome::First
        );
        assert_eq!(
            t.observe(&PathBuf::from("/tmp/a"), "aaa"),
            ReadOutcome::Unchanged { first_turn: 1 }
        );
    }

    #[test]
    fn reset_clears_state() {
        let mut t = FileReadTracker::new();
        t.observe(&PathBuf::from("/tmp/a"), "hello");
        t.reset();
        assert_eq!(t.len(), 0);
        // After reset the next read is treated as first.
        assert_eq!(
            t.observe(&PathBuf::from("/tmp/a"), "hello"),
            ReadOutcome::First
        );
    }

    #[test]
    fn cap_evicts_oldest_entries() {
        let mut t = FileReadTracker::new();
        for i in 0..MAX_TRACKED_PATHS + 5 {
            t.observe(&PathBuf::from(format!("/tmp/{i}")), "x");
        }
        assert_eq!(t.len(), MAX_TRACKED_PATHS);
        // Oldest entries (low i) should have been evicted; newest survives.
        let last = format!("/tmp/{}", MAX_TRACKED_PATHS + 4);
        assert_eq!(
            t.observe(&PathBuf::from(last), "x"),
            ReadOutcome::Unchanged {
                first_turn: (MAX_TRACKED_PATHS + 5) as u64
            }
        );
    }

    #[test]
    fn stub_text_includes_turn_number() {
        let s = unchanged_stub(7);
        assert!(s.contains("turn 7"));
        assert!(s.contains("unchanged"));
    }

    #[test]
    fn changed_header_includes_turn_number() {
        let h = changed_header(3);
        assert!(h.contains("turn 3"));
        assert!(h.contains("updated"));
    }

    #[test]
    fn session_registry_isolates_sessions() {
        let s1 = SessionId::new();
        let s2 = SessionId::new();
        with_session(s1, |t| {
            t.observe(&PathBuf::from("/tmp/x"), "v1");
        });
        // s2's first read is unaffected by s1's history.
        let outcome = with_session(s2, |t| t.observe(&PathBuf::from("/tmp/x"), "v1"));
        assert_eq!(outcome, ReadOutcome::First);
        // s1 still remembers.
        let outcome = with_session(s1, |t| t.observe(&PathBuf::from("/tmp/x"), "v1"));
        assert_eq!(outcome, ReadOutcome::Unchanged { first_turn: 1 });
    }

    #[test]
    fn reset_session_clears_only_target() {
        let s1 = SessionId::new();
        let s2 = SessionId::new();
        with_session(s1, |t| {
            t.observe(&PathBuf::from("/tmp/x"), "v1");
        });
        with_session(s2, |t| {
            t.observe(&PathBuf::from("/tmp/x"), "v1");
        });
        reset_session(s1);
        // s1 is cleared.
        let after = with_session(s1, |t| t.observe(&PathBuf::from("/tmp/x"), "v1"));
        assert_eq!(after, ReadOutcome::First);
        // s2 untouched.
        let after = with_session(s2, |t| t.observe(&PathBuf::from("/tmp/x"), "v1"));
        assert_eq!(after, ReadOutcome::Unchanged { first_turn: 1 });
    }
}
