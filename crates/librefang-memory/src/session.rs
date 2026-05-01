//! Session management — load/save conversation history.

use chrono::Utc;
use librefang_types::agent::{AgentId, SessionId};
use librefang_types::error::{LibreFangError, LibreFangResult};
use librefang_types::message::{ContentBlock, Message, MessageContent, Role};

/// Derive a short display label for a session from its first user message.
///
/// Used as a fallback when a session has no explicit `label` set (the common
/// case — labels are only set when a user/agent explicitly names a session).
/// Returns `None` if the session has no user message yet, so callers can
/// keep the field nullable for fully empty sessions.
fn derive_session_label(messages: &[Message]) -> Option<String> {
    const MAX_LEN: usize = 60;
    let first_user = messages.iter().find(|m| m.role == Role::User)?;
    let text = match &first_user.content {
        MessageContent::Text(t) => t.clone(),
        MessageContent::Blocks(blocks) => {
            let mut buf = String::new();
            for block in blocks {
                if let ContentBlock::Text { text, .. } = block {
                    if !buf.is_empty() {
                        buf.push(' ');
                    }
                    buf.push_str(text);
                    if buf.len() >= MAX_LEN {
                        break;
                    }
                }
            }
            buf
        }
    };
    let normalized: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return None;
    }
    let truncated = if normalized.chars().count() > MAX_LEN {
        let mut t: String = normalized.chars().take(MAX_LEN).collect();
        t.push('…');
        t
    } else {
        normalized
    };
    Some(truncated)
}
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tracing::warn;

/// Result from a full-text session search.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionSearchResult {
    /// The session that matched.
    pub session_id: String,
    /// The owning agent ID.
    pub agent_id: String,
    /// A text snippet showing the matching context.
    pub snippet: String,
    /// FTS5 rank score (lower is better match).
    pub rank: f64,
}

/// A conversation session with message history.
#[derive(Debug, Clone)]
pub struct Session {
    /// Session ID.
    pub id: SessionId,
    /// Owning agent ID.
    pub agent_id: AgentId,
    /// Conversation messages.
    pub messages: Vec<Message>,
    /// Estimated token count for the context window.
    pub context_window_tokens: u64,
    /// Optional human-readable session label.
    pub label: Option<String>,
}

/// Portable session export for hibernation / session state transfer.
///
/// Contains everything needed to reconstruct a session on another instance
/// or after a context window hibernation cycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionExport {
    /// Schema version for forward compatibility.
    pub version: u32,
    /// Human-readable agent name at export time.
    pub agent_name: String,
    /// Agent ID that owned the session.
    pub agent_id: String,
    /// Original session ID.
    pub session_id: String,
    /// Full conversation messages.
    pub messages: Vec<Message>,
    /// Estimated token count at export time.
    pub context_window_tokens: u64,
    /// Optional human-readable session label.
    pub label: Option<String>,
    /// ISO-8601 timestamp when the export was created.
    pub exported_at: String,
    /// Extensible metadata (model name, provider, custom tags, etc.).
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Session store backed by SQLite.
#[derive(Clone)]
pub struct SessionStore {
    conn: Arc<Mutex<Connection>>,
}

impl SessionStore {
    /// Create a new session store wrapping the given connection.
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    /// Best-effort reconcile of the FTS index against `sessions`.
    ///
    /// Older releases (#3451) wrote the `sessions` row and the
    /// `sessions_fts` row in two separate statements without a transaction.
    /// A crash in between left the FTS index out of sync — either missing
    /// rows for live sessions or carrying orphaned rows for sessions that
    /// were since deleted. Run this once at substrate boot to repair both
    /// classes of drift.
    ///
    /// Failures here are logged and swallowed: the database remains
    /// usable, full-text search just degrades to whatever the index
    /// currently holds.
    pub fn reconcile_fts_index(&self) {
        let Ok(conn) = self.conn.lock() else {
            warn!("session FTS reconcile: failed to lock connection");
            return;
        };

        // 1. Drop FTS rows whose sessions row no longer exists.
        match conn.execute(
            "DELETE FROM sessions_fts \
             WHERE session_id NOT IN (SELECT id FROM sessions)",
            [],
        ) {
            Ok(n) if n > 0 => {
                warn!(removed = n, "reconciled orphan FTS rows from sessions_fts");
            }
            Ok(_) => {}
            Err(e) => warn!(error = %e, "FTS reconcile: orphan cleanup failed"),
        }

        // 2. Build FTS rows for sessions that don't yet have one. We don't
        //    repopulate `content` here — only the next `save_session`
        //    knows the up-to-date message text and will write it
        //    transactionally. Inserting an empty placeholder is enough to
        //    surface the gap to operators inspecting the index, but we
        //    skip that to avoid polluting search; instead we just log.
        let missing: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sessions s \
                 WHERE NOT EXISTS (\
                     SELECT 1 FROM sessions_fts f WHERE f.session_id = s.id\
                 )",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if missing > 0 {
            warn!(
                missing,
                "sessions_fts is missing rows for live sessions; \
                 they will be reindexed on next save_session"
            );
        }
    }

    /// Load a session from the database.
    pub fn get_session(&self, session_id: SessionId) -> LibreFangResult<Option<Session>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let mut stmt = conn
            .prepare("SELECT agent_id, messages, context_window_tokens, label FROM sessions WHERE id = ?1")
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;

        let result = stmt.query_row(rusqlite::params![session_id.0.to_string()], |row| {
            let agent_str: String = row.get(0)?;
            let messages_blob: Vec<u8> = row.get(1)?;
            let tokens: i64 = row.get(2)?;
            let label: Option<String> = row.get(3).unwrap_or(None);
            Ok((agent_str, messages_blob, tokens, label))
        });

        match result {
            Ok((agent_str, messages_blob, tokens, label)) => {
                let agent_id = uuid::Uuid::parse_str(&agent_str)
                    .map(AgentId)
                    .map_err(|e| LibreFangError::Memory(e.to_string()))?;
                let messages: Vec<Message> = rmp_serde::from_slice(&messages_blob)
                    .map_err(|e| LibreFangError::Serialization(e.to_string()))?;
                Ok(Some(Session {
                    id: session_id,
                    agent_id,
                    messages,
                    context_window_tokens: tokens as u64,
                    label,
                }))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(LibreFangError::Memory(e.to_string())),
        }
    }

    /// Load a session from the database along with its `created_at` timestamp.
    pub fn get_session_with_created_at(
        &self,
        session_id: SessionId,
    ) -> LibreFangResult<Option<(Session, String)>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let mut stmt = conn
            .prepare("SELECT agent_id, messages, context_window_tokens, label, created_at FROM sessions WHERE id = ?1")
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;

        let result = stmt.query_row(rusqlite::params![session_id.0.to_string()], |row| {
            let agent_str: String = row.get(0)?;
            let messages_blob: Vec<u8> = row.get(1)?;
            let tokens: i64 = row.get(2)?;
            let label: Option<String> = row.get(3).unwrap_or(None);
            let created_at: String = row.get(4)?;
            Ok((agent_str, messages_blob, tokens, label, created_at))
        });

        match result {
            Ok((agent_str, messages_blob, tokens, label, created_at)) => {
                let agent_id = uuid::Uuid::parse_str(&agent_str)
                    .map(AgentId)
                    .map_err(|e| LibreFangError::Memory(e.to_string()))?;
                let messages: Vec<Message> = rmp_serde::from_slice(&messages_blob)
                    .map_err(|e| LibreFangError::Serialization(e.to_string()))?;
                Ok(Some((
                    Session {
                        id: session_id,
                        agent_id,
                        messages,
                        context_window_tokens: tokens as u64,
                        label,
                    },
                    created_at,
                )))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(LibreFangError::Memory(e.to_string())),
        }
    }

    /// Maximum number of messages persisted per session.  Older messages beyond
    /// this limit are trimmed before the blob is written to SQLite.  The
    /// in-memory limit (agent_loop::DEFAULT_MAX_HISTORY_MESSAGES = 40, now
    /// overridable per-agent and globally) is much lower,
    /// so this cap only affects sessions that were built up over many turns
    /// that were individually trimmed in memory but accumulated on disk.
    const MAX_PERSISTED_MESSAGES: usize = 200;

    /// Save a session to the database and update the FTS5 index.
    ///
    /// All three writes (INSERT session, DELETE FTS, INSERT FTS) are wrapped in
    /// a single transaction so a crash between them cannot leave the session row
    /// and the FTS index inconsistent.
    pub fn save_session(&self, session: &Session) -> LibreFangResult<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        // Trim the tail of the message history before serialising so the
        // stored blob never exceeds MAX_PERSISTED_MESSAGES.  We keep the
        // *most recent* messages (slice from the end) so context is preserved.
        let messages_to_persist: &[Message] =
            if session.messages.len() > Self::MAX_PERSISTED_MESSAGES {
                &session.messages[session.messages.len() - Self::MAX_PERSISTED_MESSAGES..]
            } else {
                &session.messages
            };
        let messages_blob = rmp_serde::to_vec_named(messages_to_persist)
            .map_err(|e| LibreFangError::Serialization(e.to_string()))?;
        let now = Utc::now().to_rfc3339();

        // Extract FTS content before acquiring the transaction so we don't hold
        // the lock longer than necessary for CPU-bound work.
        let content = Self::extract_text_content(messages_to_persist);
        let session_id_str = session.id.0.to_string();
        let agent_id_str = session.agent_id.0.to_string();

        // Wrap session upsert + FTS update in a single transaction so a crash
        // between the three statements cannot leave session and FTS data
        // inconsistent. `unchecked_transaction` is safe here because we hold
        // the Mutex exclusively (no other thread can access this Connection).
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;

        tx.execute(
            "INSERT INTO sessions (id, agent_id, messages, context_window_tokens, label, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
             ON CONFLICT(id) DO UPDATE SET messages = ?3, context_window_tokens = ?4, label = ?5, updated_at = ?6",
            rusqlite::params![
                session_id_str,
                session.agent_id.0.to_string(),
                messages_blob,
                session.context_window_tokens as i64,
                session.label.as_deref(),
                now,
            ],
        )
        .map_err(|e| LibreFangError::Memory(e.to_string()))?;

        // Delete the existing FTS row and insert the fresh content. Failures
        // here MUST abort the transaction — previously they were logged and
        // swallowed, which committed the new sessions row while leaving the
        // FTS index pointing at stale or missing content (#3451). On rollback
        // the on-disk session is unchanged, so a subsequent save retries the
        // whole pair atomically.
        tx.execute(
            "DELETE FROM sessions_fts WHERE session_id = ?1",
            rusqlite::params![session_id_str],
        )
        .map_err(|e| LibreFangError::Memory(format!("FTS delete failed: {e}")))?;

        if !content.is_empty() {
            tx.execute(
                "INSERT INTO sessions_fts (session_id, agent_id, content) VALUES (?1, ?2, ?3)",
                rusqlite::params![session_id_str, agent_id_str, content],
            )
            .map_err(|e| LibreFangError::Memory(format!("FTS insert failed: {e}")))?;
        }

        tx.commit()
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(())
    }

    /// Extract concatenated text content from a list of messages.
    fn extract_text_content(messages: &[Message]) -> String {
        messages
            .iter()
            .map(|m| m.content.text_content())
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Delete a session from the database and its FTS5 index entry.
    pub fn delete_session(&self, session_id: SessionId) -> LibreFangResult<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let id_str = session_id.0.to_string();
        conn.execute(
            "DELETE FROM sessions WHERE id = ?1",
            rusqlite::params![id_str],
        )
        .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        if let Err(e) = conn.execute(
            "DELETE FROM sessions_fts WHERE session_id = ?1",
            rusqlite::params![id_str],
        ) {
            warn!(session_id = %id_str, error = %e, "Failed to delete FTS entry; orphan row left in sessions_fts");
        }
        Ok(())
    }

    /// Return all session IDs belonging to an agent.
    pub fn get_agent_session_ids(&self, agent_id: AgentId) -> LibreFangResult<Vec<SessionId>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let mut stmt = conn
            .prepare("SELECT id FROM sessions WHERE agent_id = ?1 ORDER BY created_at DESC")
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        let rows = stmt
            .query_map(rusqlite::params![agent_id.0.to_string()], |row| {
                let id_str: String = row.get(0)?;
                Ok(id_str)
            })
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        let mut ids = Vec::new();
        for id_str in rows.flatten() {
            if let Ok(uuid) = uuid::Uuid::parse_str(&id_str) {
                ids.push(SessionId(uuid));
            }
        }
        Ok(ids)
    }

    /// Count how many of this agent's sessions were last touched after the
    /// given Unix-millis timestamp. Used by auto-dream's session-count gate
    /// as a cheap proxy for "has this agent actually done anything since the
    /// last consolidation" — a no-activity dream is a waste of tokens.
    ///
    /// `since_ms = 0` means "since the epoch" (i.e. count all sessions).
    ///
    /// `exclude_session` filters out a specific session id from the count.
    /// auto-dream passes the synthetic `auto_dream` channel session id here
    /// — otherwise the dream's own turn would count as "activity" and the
    /// gate would re-open immediately, causing repeated autonomous re-dreams
    /// even when the agent has had no user or channel traffic.
    pub fn count_agent_sessions_touched_since(
        &self,
        agent_id: AgentId,
        since_ms: u64,
        exclude_session: Option<SessionId>,
    ) -> LibreFangResult<u32> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        // `updated_at` is stored as RFC3339 strings, not millis — convert
        // the timestamp on the Rust side so the comparison is a simple
        // lexicographic compare (RFC3339 sorts correctly).
        // Fall back to the epoch (not "now") if the i64 cast somehow
        // produced an out-of-range millis value — the doc comment says
        // `since_ms = 0` means "count all sessions", and an out-of-range
        // input is closer to that intent than silently returning zero
        // sessions via a "now" threshold.
        let since_rfc3339 = chrono::DateTime::<Utc>::from_timestamp_millis(since_ms as i64)
            .unwrap_or_else(|| {
                chrono::DateTime::<Utc>::from_timestamp_millis(0)
                    .expect("epoch is always a valid millis timestamp")
            })
            .to_rfc3339();
        let count: i64 = match exclude_session {
            Some(sid) => conn
                .query_row(
                    "SELECT COUNT(*) FROM sessions \
                     WHERE agent_id = ?1 AND updated_at > ?2 AND id != ?3",
                    rusqlite::params![agent_id.0.to_string(), since_rfc3339, sid.0.to_string()],
                    |row| row.get(0),
                )
                .map_err(|e| LibreFangError::Memory(e.to_string()))?,
            None => conn
                .query_row(
                    "SELECT COUNT(*) FROM sessions WHERE agent_id = ?1 AND updated_at > ?2",
                    rusqlite::params![agent_id.0.to_string(), since_rfc3339],
                    |row| row.get(0),
                )
                .map_err(|e| LibreFangError::Memory(e.to_string()))?,
        };
        Ok(count.max(0) as u32)
    }

    /// Like [`Self::count_agent_sessions_touched_since`], but returns session
    /// IDs (most-recently-touched first, capped at `limit`). Used by
    /// auto-dream to list concrete sessions in the consolidation prompt so
    /// the model can narrow its gather phase — matches libre-code's
    /// `listSessionsTouchedSince`.
    pub fn list_agent_sessions_touched_since(
        &self,
        agent_id: AgentId,
        since_ms: u64,
        limit: u32,
        exclude_session: Option<SessionId>,
    ) -> LibreFangResult<Vec<String>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        // Fall back to the epoch (not "now") if the i64 cast somehow
        // produced an out-of-range millis value — the doc comment says
        // `since_ms = 0` means "count all sessions", and an out-of-range
        // input is closer to that intent than silently returning zero
        // sessions via a "now" threshold.
        let since_rfc3339 = chrono::DateTime::<Utc>::from_timestamp_millis(since_ms as i64)
            .unwrap_or_else(|| {
                chrono::DateTime::<Utc>::from_timestamp_millis(0)
                    .expect("epoch is always a valid millis timestamp")
            })
            .to_rfc3339();
        let rows: Vec<String> = match exclude_session {
            Some(sid) => {
                let mut stmt = conn
                    .prepare(
                        "SELECT id FROM sessions \
                         WHERE agent_id = ?1 AND updated_at > ?2 AND id != ?3 \
                         ORDER BY updated_at DESC LIMIT ?4",
                    )
                    .map_err(|e| LibreFangError::Memory(e.to_string()))?;
                let mapped = stmt
                    .query_map(
                        rusqlite::params![
                            agent_id.0.to_string(),
                            since_rfc3339,
                            sid.0.to_string(),
                            limit as i64
                        ],
                        |row| row.get::<_, String>(0),
                    )
                    .map_err(|e| LibreFangError::Memory(e.to_string()))?;
                let mut ids = Vec::new();
                for row in mapped {
                    ids.push(row.map_err(|e| LibreFangError::Memory(e.to_string()))?);
                }
                ids
            }
            None => {
                let mut stmt = conn
                    .prepare(
                        "SELECT id FROM sessions WHERE agent_id = ?1 AND updated_at > ?2 \
                         ORDER BY updated_at DESC LIMIT ?3",
                    )
                    .map_err(|e| LibreFangError::Memory(e.to_string()))?;
                let mapped = stmt
                    .query_map(
                        rusqlite::params![agent_id.0.to_string(), since_rfc3339, limit as i64],
                        |row| row.get::<_, String>(0),
                    )
                    .map_err(|e| LibreFangError::Memory(e.to_string()))?;
                let mut ids = Vec::new();
                for row in mapped {
                    ids.push(row.map_err(|e| LibreFangError::Memory(e.to_string()))?);
                }
                ids
            }
        };
        Ok(rows)
    }

    /// Delete all sessions belonging to an agent and their FTS5 index entries.
    ///
    /// Both `sessions` and `sessions_fts` are removed inside the same
    /// transaction (#3470, #3501). `save_session` writes both rows
    /// atomically, and `search_sessions` reads from `sessions_fts`
    /// without joining `sessions` — so an orphan FTS row would leave
    /// the deleted agent's content searchable via `snippet(...)`. That
    /// makes write-side asymmetry a privacy regression, not just a
    /// recoverable hygiene issue.
    pub fn delete_agent_sessions(&self, agent_id: AgentId) -> LibreFangResult<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let agent_id_str = agent_id.0.to_string();
        let tx = conn
            .transaction()
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        execute_session_agent_deletes(&tx, &agent_id_str)?;
        tx.commit()
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(())
    }

    /// Delete the canonical (cross-channel) session for an agent.
    pub fn delete_canonical_session(&self, agent_id: AgentId) -> LibreFangResult<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        conn.execute(
            "DELETE FROM canonical_sessions WHERE agent_id = ?1",
            rusqlite::params![agent_id.0.to_string()],
        )
        .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(())
    }

    /// List all sessions with metadata (session_id, agent_id, message_count,
    /// created_at, label, duration_ms, cost_usd, total_tokens).
    ///
    /// `label` resolution: an explicit user-set label wins; otherwise a
    /// snippet of the first user message's text is returned so list views
    /// (Overview "Recent sessions", Sessions page, etc.) have something
    /// readable instead of "(no label)" on every row.
    ///
    /// `duration_ms` is the wall-clock span between the first and last
    /// message timestamps — `None` for empty sessions or sessions whose
    /// messages all pre-date the timestamp field. The Messages blob is
    /// already being deserialized for `message_count`, so this adds no
    /// extra round-trip per row.
    ///
    /// `cost_usd` and `total_tokens` are aggregated from `usage_events`
    /// joined on `session_id` (added in schema v30). Pre-v30 events have
    /// `session_id IS NULL` and contribute nothing — list views will show
    /// `null` for those rows, not stale data.
    pub fn list_sessions(&self) -> LibreFangResult<Vec<serde_json::Value>> {
        self.list_sessions_paginated(None, 0)
    }

    /// Total number of sessions stored.
    pub fn count_sessions(&self) -> LibreFangResult<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(total.max(0) as usize)
    }

    /// Paginated session listing. `limit = None` returns all rows from `offset` onward.
    /// Pushes LIMIT/OFFSET into SQLite so we never deserialize message blobs we won't return (#3485).
    pub fn list_sessions_paginated(
        &self,
        limit: Option<usize>,
        offset: usize,
    ) -> LibreFangResult<Vec<serde_json::Value>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        // SQLite uses -1 for "no limit"
        let lim_sql: i64 = limit.map(|n| n as i64).unwrap_or(-1);
        let off_sql: i64 = offset as i64;
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.agent_id, s.messages, s.context_window_tokens, s.created_at, s.label,
                        COALESCE(u.total_cost_usd, 0.0) AS total_cost_usd,
                        COALESCE(u.total_tokens, 0)    AS total_tokens
                 FROM sessions s
                 LEFT JOIN (
                     SELECT session_id,
                            SUM(cost_usd)                       AS total_cost_usd,
                            SUM(input_tokens + output_tokens)   AS total_tokens
                     FROM usage_events
                     WHERE session_id IS NOT NULL
                     GROUP BY session_id
                 ) u ON u.session_id = s.id
                 ORDER BY s.created_at DESC LIMIT ?1 OFFSET ?2",
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;

        let rows = stmt
            .query_map(rusqlite::params![lim_sql, off_sql], |row| {
                let session_id: String = row.get(0)?;
                let agent_id: String = row.get(1)?;
                let messages_blob: Vec<u8> = row.get(2)?;
                let context_window_tokens: i64 = row.get(3)?;
                let created_at: String = row.get(4)?;
                let label: Option<String> = row.get(5)?;
                let total_cost_usd: f64 = row.get(6)?;
                let total_tokens: i64 = row.get(7)?;
                let messages: Vec<Message> =
                    rmp_serde::from_slice(&messages_blob).unwrap_or_default();
                let msg_count = messages.len();
                let resolved_label = label.clone().or_else(|| derive_session_label(&messages));
                // Duration spans the first to the last message that carries
                // a timestamp. Skip messages with no timestamp so
                // pre-`Message::timestamp` sessions don't anchor the span
                // to "now-vs-now". `None` if fewer than two stamped
                // messages exist.
                let duration_ms: Option<i64> = {
                    let mut stamps = messages.iter().filter_map(|m| m.timestamp);
                    let first = stamps.next();
                    // next_back() on the same iterator instead of last(): the
                    // adapter is DoubleEndedIterator (Vec::iter + filter_map),
                    // so walking from the tail finds the latest stamped
                    // message in O(k) rather than scanning every remaining
                    // element. clippy::double_ended_iterator_last enforces.
                    let last = stamps.next_back().or(first);
                    match (first, last) {
                        (Some(a), Some(b)) if b > a => Some((b - a).num_milliseconds()),
                        _ => None,
                    }
                };
                // Cost / tokens default to 0 via COALESCE. Surface them as
                // numeric (not null) so the dashboard formatter can
                // distinguish "session existed but cost zero" from
                // "no metering data" using the message_count.
                Ok(serde_json::json!({
                    "session_id": session_id,
                    "agent_id": agent_id,
                    "message_count": msg_count,
                    "context_window_tokens": context_window_tokens.max(0),
                    "created_at": created_at,
                    "label": resolved_label,
                    "duration_ms": duration_ms,
                    "cost_usd": total_cost_usd,
                    "total_tokens": total_tokens.max(0),
                }))
            })
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;

        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row.map_err(|e| LibreFangError::Memory(e.to_string()))?);
        }
        Ok(sessions)
    }

    /// Create a new empty session for an agent.
    pub fn create_session(&self, agent_id: AgentId) -> LibreFangResult<Session> {
        let session = Session {
            id: SessionId::new(),
            agent_id,
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
        };
        self.save_session(&session)?;
        Ok(session)
    }

    /// Set the label on an existing session.
    pub fn set_session_label(
        &self,
        session_id: SessionId,
        label: Option<&str>,
    ) -> LibreFangResult<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        conn.execute(
            "UPDATE sessions SET label = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![label, Utc::now().to_rfc3339(), session_id.0.to_string()],
        )
        .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(())
    }

    /// Find a session by label for a given agent.
    pub fn find_session_by_label(
        &self,
        agent_id: AgentId,
        label: &str,
    ) -> LibreFangResult<Option<Session>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let mut stmt = conn
            .prepare(
                "SELECT id, messages, context_window_tokens, label FROM sessions \
                 WHERE agent_id = ?1 AND label = ?2 LIMIT 1",
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;

        let result = stmt.query_row(rusqlite::params![agent_id.0.to_string(), label], |row| {
            let id_str: String = row.get(0)?;
            let messages_blob: Vec<u8> = row.get(1)?;
            let tokens: i64 = row.get(2)?;
            let lbl: Option<String> = row.get(3).unwrap_or(None);
            Ok((id_str, messages_blob, tokens, lbl))
        });

        match result {
            Ok((id_str, messages_blob, tokens, lbl)) => {
                let session_id = uuid::Uuid::parse_str(&id_str)
                    .map(SessionId)
                    .map_err(|e| LibreFangError::Memory(e.to_string()))?;
                let messages: Vec<Message> = rmp_serde::from_slice(&messages_blob)
                    .map_err(|e| LibreFangError::Serialization(e.to_string()))?;
                Ok(Some(Session {
                    id: session_id,
                    agent_id,
                    messages,
                    context_window_tokens: tokens as u64,
                    label: lbl,
                }))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(LibreFangError::Memory(e.to_string())),
        }
    }
}

impl SessionStore {
    /// List all sessions for a specific agent.
    pub fn list_agent_sessions(
        &self,
        agent_id: AgentId,
    ) -> LibreFangResult<Vec<serde_json::Value>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let mut stmt = conn
            .prepare(
                "SELECT id, messages, created_at, label FROM sessions WHERE agent_id = ?1 ORDER BY created_at DESC",
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;

        let rows = stmt
            .query_map(rusqlite::params![agent_id.0.to_string()], |row| {
                let session_id: String = row.get(0)?;
                let messages_blob: Vec<u8> = row.get(1)?;
                let created_at: String = row.get(2)?;
                let label: Option<String> = row.get(3)?;
                let messages: Vec<Message> =
                    rmp_serde::from_slice(&messages_blob).unwrap_or_default();
                let msg_count = messages.len();
                let resolved_label = label.clone().or_else(|| derive_session_label(&messages));
                Ok(serde_json::json!({
                    "session_id": session_id,
                    "message_count": msg_count,
                    "created_at": created_at,
                    "label": resolved_label,
                }))
            })
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;

        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row.map_err(|e| LibreFangError::Memory(e.to_string()))?);
        }
        Ok(sessions)
    }

    /// Create a new session with an optional label.
    pub fn create_session_with_label(
        &self,
        agent_id: AgentId,
        label: Option<&str>,
    ) -> LibreFangResult<Session> {
        let session = Session {
            id: SessionId::new(),
            agent_id,
            messages: Vec::new(),
            context_window_tokens: 0,
            label: label.map(|s| s.to_string()),
        };
        self.save_session(&session)?;
        Ok(session)
    }

    /// Store an LLM-generated summary, replacing older messages with the summary
    /// and keeping only the specified recent messages.
    ///
    /// This is used by the LLM-based compactor to replace text-truncation compaction
    /// with an intelligent, LLM-generated summary of older conversation history.
    pub fn store_llm_summary(
        &self,
        agent_id: AgentId,
        summary: &str,
        kept_messages: Vec<Message>,
    ) -> LibreFangResult<()> {
        let mut canonical = self.load_canonical(agent_id)?;
        canonical.compacted_summary = Some(summary.to_string());
        canonical.messages = kept_messages
            .into_iter()
            .map(|message| CanonicalEntry {
                message,
                session_id: None,
            })
            .collect();
        canonical.compaction_cursor = 0;
        canonical.updated_at = Utc::now().to_rfc3339();
        self.save_canonical(&canonical)
    }
}

impl SessionStore {
    /// Delete sessions that have not been updated within `retention_days`.
    ///
    /// Returns the number of sessions deleted.
    pub fn cleanup_expired_sessions(&self, retention_days: u32) -> LibreFangResult<u64> {
        if retention_days == 0 {
            return Ok(0);
        }
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let cutoff = Utc::now() - chrono::Duration::days(i64::from(retention_days));
        let cutoff_str = cutoff.to_rfc3339();
        let deleted = conn
            .execute(
                "DELETE FROM sessions WHERE updated_at < ?1",
                rusqlite::params![cutoff_str],
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(deleted as u64)
    }

    /// For each agent, keep only the newest `max_per_agent` sessions, deleting the rest.
    ///
    /// Returns the total number of sessions deleted across all agents.
    pub fn cleanup_excess_sessions(&self, max_per_agent: u32) -> LibreFangResult<u64> {
        if max_per_agent == 0 {
            return Ok(0);
        }
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        // Single-query approach using window functions (SQLite 3.25+).
        // ROW_NUMBER partitions by agent and ranks by recency; rows beyond
        // the limit are deleted in one pass — no N+1 per-agent queries.
        let deleted = conn
            .execute(
                "DELETE FROM sessions WHERE id IN (
                    SELECT id FROM (
                        SELECT id, ROW_NUMBER() OVER (
                            PARTITION BY agent_id ORDER BY updated_at DESC
                        ) AS rn
                        FROM sessions
                    ) WHERE rn > ?1
                )",
                rusqlite::params![max_per_agent],
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;

        Ok(deleted as u64)
    }

    /// Delete sessions whose agent_id is not in the provided live set.
    ///
    /// Returns the number of orphan sessions deleted.
    pub fn cleanup_orphan_sessions(&self, live_agent_ids: &[AgentId]) -> LibreFangResult<u64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        if live_agent_ids.is_empty() {
            return Ok(0);
        }

        let placeholders: Vec<String> = live_agent_ids
            .iter()
            .map(|id| format!("'{}'", id.0))
            .collect();
        let in_clause = placeholders.join(",");
        let sql = format!("DELETE FROM sessions WHERE agent_id NOT IN ({in_clause})");
        let deleted = conn
            .execute(&sql, [])
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;

        Ok(deleted as u64)
    }
}

impl SessionStore {
    /// Full-text search across session content using FTS5.
    ///
    /// Returns matching sessions ranked by relevance. Optionally filter by agent.
    pub fn search_sessions(
        &self,
        query: &str,
        agent_id: Option<&AgentId>,
    ) -> LibreFangResult<Vec<SessionSearchResult>> {
        // Backwards-compatible default: keep historical 50-row cap for
        // callers that don't care about pagination (tests, internal kernel
        // use). New paginated callers should use `search_sessions_paginated`.
        self.search_sessions_paginated(query, agent_id, Some(50), 0)
    }

    /// Full-text search across session content using FTS5, with pagination.
    ///
    /// Pagination is pushed into SQLite via `LIMIT ?/OFFSET ?`, so unbounded
    /// result sets never materialize in memory (#3691). Pass `limit = None`
    /// to skip the LIMIT clause; callers MUST clamp to a sane upper bound
    /// before doing so.
    pub fn search_sessions_paginated(
        &self,
        query: &str,
        agent_id: Option<&AgentId>,
        limit: Option<usize>,
        offset: usize,
    ) -> LibreFangResult<Vec<SessionSearchResult>> {
        if query.is_empty() {
            return Ok(Vec::new());
        }

        // Sanitize FTS5 query: escape special characters to prevent injection.
        // FTS5 treats `*`, `"`, `NEAR`, `OR`, `AND`, `NOT` as operators.
        // Wrap each word in double quotes to treat as literal phrase tokens.
        let sanitized: String = query
            .split_whitespace()
            .map(|word| {
                let escaped = word.replace('"', "\"\"");
                format!("\"{escaped}\"")
            })
            .collect::<Vec<_>>()
            .join(" ");

        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        // SQLite treats LIMIT < 0 as "no limit" — encode `None` that way so
        // the query plan stays a single prepared statement either way.
        let limit_param: i64 = limit.map(|n| n as i64).unwrap_or(-1);
        let offset_param: i64 = offset as i64;

        let results = if let Some(aid) = agent_id {
            let mut stmt = conn
                .prepare(
                    "SELECT session_id, agent_id, snippet(sessions_fts, 2, '<b>', '</b>', '...', 32), rank
                     FROM sessions_fts
                     WHERE content MATCH ?1 AND agent_id = ?2
                     ORDER BY rank
                     LIMIT ?3 OFFSET ?4",
                )
                .map_err(|e| LibreFangError::Memory(e.to_string()))?;

            let rows = stmt
                .query_map(
                    rusqlite::params![sanitized, aid.0.to_string(), limit_param, offset_param],
                    |row| {
                        Ok(SessionSearchResult {
                            session_id: row.get(0)?,
                            agent_id: row.get(1)?,
                            snippet: row.get(2)?,
                            rank: row.get(3)?,
                        })
                    },
                )
                .map_err(|e| LibreFangError::Memory(e.to_string()))?;

            rows.filter_map(|r| r.ok()).collect()
        } else {
            let mut stmt = conn
                .prepare(
                    "SELECT session_id, agent_id, snippet(sessions_fts, 2, '<b>', '</b>', '...', 32), rank
                     FROM sessions_fts
                     WHERE content MATCH ?1
                     ORDER BY rank
                     LIMIT ?2 OFFSET ?3",
                )
                .map_err(|e| LibreFangError::Memory(e.to_string()))?;

            let rows = stmt
                .query_map(
                    rusqlite::params![sanitized, limit_param, offset_param],
                    |row| {
                        Ok(SessionSearchResult {
                            session_id: row.get(0)?,
                            agent_id: row.get(1)?,
                            snippet: row.get(2)?,
                            rank: row.get(3)?,
                        })
                    },
                )
                .map_err(|e| LibreFangError::Memory(e.to_string()))?;

            rows.filter_map(|r| r.ok()).collect()
        };

        Ok(results)
    }
}

/// Default number of recent messages to include from canonical session.
const DEFAULT_CANONICAL_WINDOW: usize = 50;

/// Default compaction threshold: when message count exceeds this, compact older messages.
const DEFAULT_COMPACTION_THRESHOLD: usize = 100;

/// A canonical message tagged with its originating session id for chat-scoped filtering.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CanonicalEntry {
    pub message: Message,
    #[serde(default)]
    pub session_id: Option<SessionId>,
}

/// A canonical session stores persistent cross-channel context for an agent.
///
/// Unlike regular sessions (one per channel interaction), there is one canonical
/// session per agent. All channels contribute to it, so what a user tells an agent
/// on Telegram is remembered on Discord.
#[derive(Debug, Clone)]
pub struct CanonicalSession {
    /// The agent this session belongs to.
    pub agent_id: AgentId,
    /// Full message history (post-compaction window), tagged by originating session.
    pub messages: Vec<CanonicalEntry>,
    /// Index marking how far compaction has processed.
    pub compaction_cursor: usize,
    /// Summary of compacted (older) messages.
    pub compacted_summary: Option<String>,
    /// Last update time.
    pub updated_at: String,
}

impl SessionStore {
    /// Load the canonical session for an agent, creating one if it doesn't exist.
    pub fn load_canonical(&self, agent_id: AgentId) -> LibreFangResult<CanonicalSession> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        load_canonical_in_tx(&conn, agent_id)
    }

    /// Append new messages to the canonical session and compact if over threshold.
    ///
    /// Compaction summarizes old messages into a text summary and trims the
    /// message list. The `compaction_threshold` controls when this happens
    /// (default: 100 messages).
    pub fn append_canonical(
        &self,
        agent_id: AgentId,
        new_messages: &[Message],
        compaction_threshold: Option<usize>,
        session_id: Option<SessionId>,
    ) -> LibreFangResult<CanonicalSession> {
        // Hold the connection lock across the entire read-modify-write so concurrent
        // sessions of the same agent cannot lose each other's appended messages
        // (canonical_sessions is keyed by agent_id and stored as a single blob).
        // BEGIN IMMEDIATE escalates to a write lock at the SQLite layer too, so
        // any future cross-process caller is also serialized.
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;

        let mut canonical = load_canonical_in_tx(&tx, agent_id)?;
        canonical
            .messages
            .extend(new_messages.iter().cloned().map(|message| CanonicalEntry {
                message,
                session_id,
            }));

        let threshold = compaction_threshold.unwrap_or(DEFAULT_COMPACTION_THRESHOLD);

        // Compact if over threshold
        if canonical.messages.len() > threshold {
            let keep_count = DEFAULT_CANONICAL_WINDOW;
            let to_compact = canonical.messages.len().saturating_sub(keep_count);
            if to_compact > canonical.compaction_cursor {
                // Build a summary from the messages being compacted
                let compacting = &canonical.messages[canonical.compaction_cursor..to_compact];
                let mut summary_parts: Vec<String> = Vec::new();
                if let Some(ref existing) = canonical.compacted_summary {
                    summary_parts.push(existing.clone());
                }
                for entry in compacting {
                    let msg = &entry.message;
                    let role = match msg.role {
                        librefang_types::message::Role::User => "User",
                        librefang_types::message::Role::Assistant => "Assistant",
                        librefang_types::message::Role::System => "System",
                    };
                    let text = msg.content.text_content();
                    if !text.is_empty() {
                        // Truncate individual messages in summary to keep it compact (UTF-8 safe)
                        let truncated = if text.len() > 200 {
                            format!("{}...", librefang_types::truncate_str(&text, 200))
                        } else {
                            text
                        };
                        summary_parts.push(format!("{role}: {truncated}"));
                    }
                }
                // Keep summary under ~4000 chars (UTF-8 safe)
                let mut full_summary = summary_parts.join("\n");
                if full_summary.len() > 4000 {
                    let start = full_summary.len() - 4000;
                    // Find the next char boundary at or after `start`
                    let safe_start = (start..full_summary.len())
                        .find(|&i| full_summary.is_char_boundary(i))
                        .unwrap_or(full_summary.len());
                    full_summary = full_summary[safe_start..].to_string();
                }
                canonical.compacted_summary = Some(full_summary);
                canonical.compaction_cursor = to_compact;
                // Trim messages: keep only the recent window
                canonical.messages = canonical.messages.split_off(to_compact);
                canonical.compaction_cursor = 0; // reset cursor since we trimmed
            }
        }

        canonical.updated_at = Utc::now().to_rfc3339();
        save_canonical_in_tx(&tx, &canonical)?;
        tx.commit()
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(canonical)
    }

    /// Get recent messages from canonical session for context injection.
    ///
    /// Returns up to `window_size` recent messages (default 50), plus
    /// the compacted summary if available.
    pub fn canonical_context(
        &self,
        agent_id: AgentId,
        session_id: Option<SessionId>,
        window_size: Option<usize>,
    ) -> LibreFangResult<(Option<String>, Vec<Message>)> {
        let canonical = self.load_canonical(agent_id)?;
        let window = window_size.unwrap_or(DEFAULT_CANONICAL_WINDOW);
        // Filter by session_id: include matching entries and untagged (legacy) entries.
        let filtered: Vec<Message> = canonical
            .messages
            .iter()
            .filter(|e| match (&session_id, &e.session_id) {
                (Some(want), Some(got)) => want == got,
                (Some(_), None) => true,
                (None, _) => true,
            })
            .map(|e| e.message.clone())
            .collect();
        let start = filtered.len().saturating_sub(window);
        let recent = filtered[start..].to_vec();
        Ok((canonical.compacted_summary.clone(), recent))
    }

    /// Persist a canonical session to SQLite.
    fn save_canonical(&self, canonical: &CanonicalSession) -> LibreFangResult<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        save_canonical_in_tx(&conn, canonical)
    }
}

/// Load a canonical session using an already-acquired connection (e.g. inside a transaction).
fn load_canonical_in_tx(conn: &Connection, agent_id: AgentId) -> LibreFangResult<CanonicalSession> {
    let mut stmt = conn
        .prepare(
            "SELECT messages, compaction_cursor, compacted_summary, updated_at \
             FROM canonical_sessions WHERE agent_id = ?1",
        )
        .map_err(|e| LibreFangError::Memory(e.to_string()))?;

    let result = stmt.query_row(rusqlite::params![agent_id.0.to_string()], |row| {
        let messages_blob: Vec<u8> = row.get(0)?;
        let cursor: i64 = row.get(1)?;
        let summary: Option<String> = row.get(2)?;
        let updated_at: String = row.get(3)?;
        Ok((messages_blob, cursor, summary, updated_at))
    });

    match result {
        Ok((messages_blob, cursor, summary, updated_at)) => {
            // Try new format (tagged entries); fall back to legacy Vec<Message> for pre-fix rows.
            let messages: Vec<CanonicalEntry> =
                match rmp_serde::from_slice::<Vec<CanonicalEntry>>(&messages_blob) {
                    Ok(entries) => entries,
                    Err(_) => {
                        let legacy: Vec<Message> = rmp_serde::from_slice(&messages_blob)
                            .map_err(|e| LibreFangError::Serialization(e.to_string()))?;
                        legacy
                            .into_iter()
                            .map(|message| CanonicalEntry {
                                message,
                                session_id: None,
                            })
                            .collect()
                    }
                };
            Ok(CanonicalSession {
                agent_id,
                messages,
                compaction_cursor: cursor as usize,
                compacted_summary: summary,
                updated_at,
            })
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            let now = Utc::now().to_rfc3339();
            Ok(CanonicalSession {
                agent_id,
                messages: Vec::new(),
                compaction_cursor: 0,
                compacted_summary: None,
                updated_at: now,
            })
        }
        Err(e) => Err(LibreFangError::Memory(e.to_string())),
    }
}

/// Persist a canonical session using an already-acquired connection.
fn save_canonical_in_tx(conn: &Connection, canonical: &CanonicalSession) -> LibreFangResult<()> {
    let messages_blob = rmp_serde::to_vec(&canonical.messages)
        .map_err(|e| LibreFangError::Serialization(e.to_string()))?;
    conn.execute(
        "INSERT INTO canonical_sessions (agent_id, messages, compaction_cursor, compacted_summary, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(agent_id) DO UPDATE SET messages = ?2, compaction_cursor = ?3, compacted_summary = ?4, updated_at = ?5",
        rusqlite::params![
            canonical.agent_id.0.to_string(),
            messages_blob,
            canonical.compaction_cursor as i64,
            canonical.compacted_summary,
            canonical.updated_at,
        ],
    )
    .map_err(|e| LibreFangError::Memory(e.to_string()))?;
    Ok(())
}

/// A single JSONL line in the session mirror file.
#[derive(serde::Serialize)]
struct JsonlLine {
    timestamp: String,
    role: String,
    content: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_use: Option<serde_json::Value>,
}

impl SessionStore {
    /// Write a human-readable JSONL mirror of a session to disk.
    ///
    /// Best-effort: errors are returned but should be logged and never
    /// affect the primary SQLite store.
    pub fn write_jsonl_mirror(
        &self,
        session: &Session,
        sessions_dir: &Path,
    ) -> Result<(), std::io::Error> {
        std::fs::create_dir_all(sessions_dir)?;
        let path = sessions_dir.join(format!("{}.jsonl", session.id.0));
        let mut file = std::fs::File::create(&path)?;
        let now = Utc::now().to_rfc3339();

        for msg in &session.messages {
            let role_str = match msg.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::System => "system",
            };

            let mut text_parts: Vec<String> = Vec::new();
            let mut tool_parts: Vec<serde_json::Value> = Vec::new();

            match &msg.content {
                MessageContent::Text(t) => {
                    text_parts.push(t.clone());
                }
                MessageContent::Blocks(blocks) => {
                    for block in blocks {
                        match block {
                            ContentBlock::Text { text, .. } => {
                                text_parts.push(text.clone());
                            }
                            ContentBlock::ToolUse {
                                id, name, input, ..
                            } => {
                                tool_parts.push(serde_json::json!({
                                    "type": "tool_use",
                                    "id": id,
                                    "name": name,
                                    "input": input,
                                }));
                            }
                            ContentBlock::ToolResult {
                                tool_use_id,
                                tool_name: _,
                                content,
                                is_error,
                                ..
                            } => {
                                tool_parts.push(serde_json::json!({
                                    "type": "tool_result",
                                    "tool_use_id": tool_use_id,
                                    "content": content,
                                    "is_error": is_error,
                                }));
                            }
                            ContentBlock::Image { media_type, .. }
                            | ContentBlock::ImageFile { media_type, .. } => {
                                text_parts.push(format!("[image: {media_type}]"));
                            }
                            ContentBlock::Thinking { thinking, .. } => {
                                text_parts.push(format!(
                                    "[thinking: {}]",
                                    librefang_types::truncate_str(thinking, 200)
                                ));
                            }
                            ContentBlock::Unknown => {}
                        }
                    }
                }
            }

            let line = JsonlLine {
                timestamp: now.clone(),
                role: role_str.to_string(),
                content: serde_json::Value::String(text_parts.join("\n")),
                tool_use: if tool_parts.is_empty() {
                    None
                } else {
                    Some(serde_json::Value::Array(tool_parts))
                },
            };

            serde_json::to_writer(&mut file, &line).map_err(std::io::Error::other)?;
            file.write_all(b"\n")?;
        }

        Ok(())
    }
}

/// Run every session-store DELETE for an agent inside the caller's
/// transaction. Both `sessions` and `sessions_fts` MUST be cleared
/// together — `search_sessions` reads from `sessions_fts` without
/// joining `sessions`, so an orphan FTS row leaves the deleted agent's
/// content searchable (a privacy regression, see #3501).
///
/// Shared by [`SessionStore::delete_agent_sessions`] and
/// [`crate::substrate::MemorySubstrate::remove_agent`] so the cascade
/// stays consistent across both entry points.
pub(crate) fn execute_session_agent_deletes(
    tx: &rusqlite::Transaction<'_>,
    agent_id: &str,
) -> LibreFangResult<()> {
    tx.execute(
        "DELETE FROM sessions WHERE agent_id = ?1",
        rusqlite::params![agent_id],
    )
    .map_err(|e| LibreFangError::Memory(e.to_string()))?;
    tx.execute(
        "DELETE FROM sessions_fts WHERE agent_id = ?1",
        rusqlite::params![agent_id],
    )
    .map_err(|e| LibreFangError::Memory(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migrations;

    fn setup() -> SessionStore {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        SessionStore::new(Arc::new(Mutex::new(conn)))
    }

    #[test]
    fn test_create_and_load_session() {
        let store = setup();
        let agent_id = AgentId::new();
        let session = store.create_session(agent_id).unwrap();

        let loaded = store.get_session(session.id).unwrap().unwrap();
        assert_eq!(loaded.agent_id, agent_id);
        assert!(loaded.messages.is_empty());
    }

    #[test]
    fn test_save_and_load_with_messages() {
        let store = setup();
        let agent_id = AgentId::new();
        let mut session = store.create_session(agent_id).unwrap();
        session.messages.push(Message::user("Hello"));
        session.messages.push(Message::assistant("Hi there!"));
        store.save_session(&session).unwrap();

        let loaded = store.get_session(session.id).unwrap().unwrap();
        assert_eq!(loaded.messages.len(), 2);
    }

    #[test]
    fn test_get_missing_session() {
        let store = setup();
        let result = store.get_session(SessionId::new()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_delete_session() {
        let store = setup();
        let agent_id = AgentId::new();
        let session = store.create_session(agent_id).unwrap();
        let sid = session.id;
        assert!(store.get_session(sid).unwrap().is_some());
        store.delete_session(sid).unwrap();
        assert!(store.get_session(sid).unwrap().is_none());
    }

    #[test]
    fn test_delete_agent_sessions() {
        let store = setup();
        let agent_id = AgentId::new();
        let s1 = store.create_session(agent_id).unwrap();
        let s2 = store.create_session(agent_id).unwrap();
        assert!(store.get_session(s1.id).unwrap().is_some());
        assert!(store.get_session(s2.id).unwrap().is_some());
        store.delete_agent_sessions(agent_id).unwrap();
        assert!(store.get_session(s1.id).unwrap().is_none());
        assert!(store.get_session(s2.id).unwrap().is_none());
    }

    /// `delete_agent_sessions` must wipe `sessions_fts` in the same
    /// transaction as `sessions`. `search_sessions` reads from the FTS
    /// table without joining `sessions`, so any orphan FTS row remains
    /// searchable (and snippets leak content) after the owning agent
    /// is gone — a privacy regression, not a recoverable hygiene issue.
    #[test]
    fn test_delete_agent_sessions_clears_fts() {
        let store = setup();
        let agent_id = AgentId::new();

        // Seed a session whose content goes through the FTS index.
        let mut session = store.create_session(agent_id).unwrap();
        let needle = "thequickbrownfoxnonceabc123";
        session.messages.push(Message::user(needle));
        store.save_session(&session).unwrap();

        // FTS sees it.
        let pre = store.search_sessions(needle, Some(&agent_id)).unwrap();
        assert!(
            !pre.is_empty(),
            "FTS index must be populated by save_session"
        );

        store.delete_agent_sessions(agent_id).unwrap();

        // After cascade delete, the FTS index must NOT still return the
        // content. Pre-fix the FTS DELETE was best-effort outside the tx,
        // so a partial failure (or any out-of-tx race) could leave the
        // searchable orphan visible here.
        let post = store.search_sessions(needle, Some(&agent_id)).unwrap();
        assert!(
            post.is_empty(),
            "search_sessions must not return orphan FTS rows after delete_agent_sessions"
        );
    }

    #[test]
    fn test_canonical_load_creates_empty() {
        let store = setup();
        let agent_id = AgentId::new();
        let canonical = store.load_canonical(agent_id).unwrap();
        assert_eq!(canonical.agent_id, agent_id);
        assert!(canonical.messages.is_empty());
        assert!(canonical.compacted_summary.is_none());
        assert_eq!(canonical.compaction_cursor, 0);
    }

    #[test]
    fn test_canonical_append_and_load() {
        let store = setup();
        let agent_id = AgentId::new();

        // Append from "Telegram"
        let msgs1 = vec![
            Message::user("Hello from Telegram"),
            Message::assistant("Hi! I'm your agent."),
        ];
        store
            .append_canonical(agent_id, &msgs1, None, None)
            .unwrap();

        // Append from "Discord"
        let msgs2 = vec![
            Message::user("Now I'm on Discord"),
            Message::assistant("I remember you from Telegram!"),
        ];
        let canonical = store
            .append_canonical(agent_id, &msgs2, None, None)
            .unwrap();

        // Should have all 4 messages
        assert_eq!(canonical.messages.len(), 4);
    }

    #[test]
    fn test_canonical_context_window() {
        let store = setup();
        let agent_id = AgentId::new();

        // Add 10 messages
        let msgs: Vec<Message> = (0..10)
            .map(|i| Message::user(format!("Message {i}")))
            .collect();
        store.append_canonical(agent_id, &msgs, None, None).unwrap();

        // Request window of 3
        let (summary, recent) = store.canonical_context(agent_id, None, Some(3)).unwrap();
        assert_eq!(recent.len(), 3);
        assert!(summary.is_none()); // No compaction yet
    }

    #[test]
    fn test_canonical_compaction() {
        let store = setup();
        let agent_id = AgentId::new();

        // Add 120 messages (over the default 100 threshold)
        let msgs: Vec<Message> = (0..120)
            .map(|i| Message::user(format!("Message number {i} with some content")))
            .collect();
        let canonical = store
            .append_canonical(agent_id, &msgs, Some(100), None)
            .unwrap();

        // After compaction: should keep DEFAULT_CANONICAL_WINDOW (50) messages
        assert!(canonical.messages.len() <= 60); // some tolerance
        assert!(canonical.compacted_summary.is_some());
    }

    #[test]
    fn test_canonical_cross_channel_roundtrip() {
        let store = setup();
        let agent_id = AgentId::new();

        // Channel 1: user tells agent their name
        store
            .append_canonical(
                agent_id,
                &[
                    Message::user("My name is Jaber"),
                    Message::assistant("Nice to meet you, Jaber!"),
                ],
                None,
                None,
            )
            .unwrap();

        // Channel 2: different channel queries same agent
        let (summary, recent) = store.canonical_context(agent_id, None, None).unwrap();
        // The agent should have context about "Jaber" from the previous channel
        let all_text: String = recent.iter().map(|m| m.content.text_content()).collect();
        assert!(all_text.contains("Jaber"));
        assert!(summary.is_none()); // Only 2 messages, no compaction
    }

    #[test]
    fn test_canonical_context_session_scoped() {
        let store = setup();
        let agent_id = AgentId::new();
        let sid_a = SessionId::new();
        let sid_b = SessionId::new();

        store
            .append_canonical(
                agent_id,
                &[Message::user("from A-1"), Message::assistant("reply A-1")],
                None,
                Some(sid_a),
            )
            .unwrap();
        store
            .append_canonical(
                agent_id,
                &[Message::user("from B-1"), Message::assistant("reply B-1")],
                None,
                Some(sid_b),
            )
            .unwrap();

        let (_, recent_a) = store
            .canonical_context(agent_id, Some(sid_a), None)
            .unwrap();
        let text_a: String = recent_a.iter().map(|m| m.content.text_content()).collect();
        assert!(text_a.contains("A-1"));
        assert!(!text_a.contains("B-1"));

        let (_, recent_all) = store.canonical_context(agent_id, None, None).unwrap();
        assert_eq!(recent_all.len(), 4);
    }

    #[test]
    fn test_canonical_backward_compat_legacy_blob() {
        let store = setup();
        let agent_id = AgentId::new();

        let legacy: Vec<Message> = vec![
            Message::user("legacy user"),
            Message::assistant("legacy reply"),
        ];
        let blob = rmp_serde::to_vec(&legacy).unwrap();
        let now = Utc::now().to_rfc3339();
        {
            let conn = store.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO canonical_sessions (agent_id, messages, compaction_cursor, compacted_summary, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![agent_id.0.to_string(), blob, 0_i64, Option::<String>::None, now],
            )
            .unwrap();
        }

        let canonical = store.load_canonical(agent_id).unwrap();
        assert_eq!(canonical.messages.len(), 2);
        assert!(canonical.messages.iter().all(|e| e.session_id.is_none()));

        let (_, recent) = store.canonical_context(agent_id, None, None).unwrap();
        let text: String = recent.iter().map(|m| m.content.text_content()).collect();
        assert!(text.contains("legacy"));
    }

    #[test]
    fn test_canonical_append_concurrent_no_message_loss() {
        // Regression for #3559: two sessions of the same agent appending
        // concurrently must not silently overwrite each other.
        let store = setup();
        let agent_id = AgentId::new();
        let sid_a = SessionId::new();
        let sid_b = SessionId::new();

        const PER_THREAD: usize = 50;
        let store_a = store.clone();
        let store_b = store.clone();
        let h_a = std::thread::spawn(move || {
            for i in 0..PER_THREAD {
                store_a
                    .append_canonical(
                        agent_id,
                        &[Message::user(format!("A-{i}"))],
                        Some(10_000), // disable compaction so all messages are observable
                        Some(sid_a),
                    )
                    .expect("append A");
            }
        });
        let h_b = std::thread::spawn(move || {
            for i in 0..PER_THREAD {
                store_b
                    .append_canonical(
                        agent_id,
                        &[Message::user(format!("B-{i}"))],
                        Some(10_000),
                        Some(sid_b),
                    )
                    .expect("append B");
            }
        });
        h_a.join().unwrap();
        h_b.join().unwrap();

        let canonical = store.load_canonical(agent_id).unwrap();
        assert_eq!(
            canonical.messages.len(),
            PER_THREAD * 2,
            "expected all {} messages from both sessions to be persisted",
            PER_THREAD * 2,
        );
        let count_a = canonical
            .messages
            .iter()
            .filter(|e| e.session_id == Some(sid_a))
            .count();
        let count_b = canonical
            .messages
            .iter()
            .filter(|e| e.session_id == Some(sid_b))
            .count();
        assert_eq!(count_a, PER_THREAD, "session A messages dropped");
        assert_eq!(count_b, PER_THREAD, "session B messages dropped");
    }

    #[test]
    fn test_cleanup_expired_sessions() {
        let store = setup();
        let agent_id = AgentId::new();

        // Create two sessions
        let s1 = store.create_session(agent_id).unwrap();
        let s2 = store.create_session(agent_id).unwrap();

        // Manually backdate s1 to 60 days ago
        {
            let conn = store.conn.lock().unwrap();
            let old_date = (Utc::now() - chrono::Duration::days(60)).to_rfc3339();
            conn.execute(
                "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
                rusqlite::params![old_date, s1.id.0.to_string()],
            )
            .unwrap();
        }

        // Cleanup with 30-day retention
        let deleted = store.cleanup_expired_sessions(30).unwrap();
        assert_eq!(deleted, 1);

        // s1 should be gone, s2 should remain
        assert!(store.get_session(s1.id).unwrap().is_none());
        assert!(store.get_session(s2.id).unwrap().is_some());
    }

    #[test]
    fn test_cleanup_expired_sessions_zero_noop() {
        let store = setup();
        let agent_id = AgentId::new();
        store.create_session(agent_id).unwrap();

        // retention_days=0 should be a no-op
        let deleted = store.cleanup_expired_sessions(0).unwrap();
        assert_eq!(deleted, 0);
    }

    #[test]
    fn test_cleanup_excess_sessions() {
        let store = setup();
        let agent_id = AgentId::new();

        // Create 5 sessions, staggering updated_at so ordering is deterministic
        let mut session_ids = Vec::new();
        for i in 0..5 {
            let s = store.create_session(agent_id).unwrap();
            let conn = store.conn.lock().unwrap();
            let date = (Utc::now() + chrono::Duration::seconds(i)).to_rfc3339();
            conn.execute(
                "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
                rusqlite::params![date, s.id.0.to_string()],
            )
            .unwrap();
            session_ids.push(s.id);
        }

        // Keep only 2 per agent
        let deleted = store.cleanup_excess_sessions(2).unwrap();
        assert_eq!(deleted, 3);

        // The 3 oldest should be gone, the 2 newest should remain
        assert!(store.get_session(session_ids[0]).unwrap().is_none());
        assert!(store.get_session(session_ids[1]).unwrap().is_none());
        assert!(store.get_session(session_ids[2]).unwrap().is_none());
        assert!(store.get_session(session_ids[3]).unwrap().is_some());
        assert!(store.get_session(session_ids[4]).unwrap().is_some());
    }

    #[test]
    fn test_cleanup_excess_sessions_zero_noop() {
        let store = setup();
        let agent_id = AgentId::new();
        store.create_session(agent_id).unwrap();

        // max_per_agent=0 should be a no-op
        let deleted = store.cleanup_excess_sessions(0).unwrap();
        assert_eq!(deleted, 0);
    }

    #[test]
    fn test_jsonl_mirror_write() {
        let store = setup();
        let agent_id = AgentId::new();
        let mut session = store.create_session(agent_id).unwrap();
        session
            .messages
            .push(librefang_types::message::Message::user("Hello"));
        session
            .messages
            .push(librefang_types::message::Message::assistant("Hi there!"));
        store.save_session(&session).unwrap();

        let dir = tempfile::TempDir::new().unwrap();
        let sessions_dir = dir.path().join("sessions");
        store.write_jsonl_mirror(&session, &sessions_dir).unwrap();

        let jsonl_path = sessions_dir.join(format!("{}.jsonl", session.id.0));
        assert!(jsonl_path.exists());

        let content = std::fs::read_to_string(&jsonl_path).unwrap();
        let lines: Vec<&str> = content.trim().split('\n').collect();
        assert_eq!(lines.len(), 2);

        // Verify first line is user message
        let line1: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(line1["role"], "user");
        assert_eq!(line1["content"], "Hello");

        // Verify second line is assistant message
        let line2: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(line2["role"], "assistant");
        assert_eq!(line2["content"], "Hi there!");
        assert!(line2.get("tool_use").is_none());
    }

    #[test]
    fn test_fts_search_sessions() {
        let store = setup();
        let agent_id = AgentId::new();
        let mut session = store.create_session(agent_id).unwrap();
        session
            .messages
            .push(Message::user("The quick brown fox jumps over the lazy dog"));
        session
            .messages
            .push(Message::assistant("That is a classic pangram!"));
        store.save_session(&session).unwrap();

        // Search for existing content
        let results = store.search_sessions("fox", None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].session_id, session.id.0.to_string());

        // Search with agent filter
        let results = store.search_sessions("pangram", Some(&agent_id)).unwrap();
        assert_eq!(results.len(), 1);

        // Search with wrong agent should return nothing
        let other_agent = AgentId::new();
        let results = store.search_sessions("fox", Some(&other_agent)).unwrap();
        assert!(results.is_empty());

        // Search for non-existent content
        let results = store.search_sessions("elephant", None).unwrap();
        assert!(results.is_empty());

        // Empty query should return nothing
        let results = store.search_sessions("", None).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_fts_updates_on_save() {
        let store = setup();
        let agent_id = AgentId::new();
        let mut session = store.create_session(agent_id).unwrap();
        session.messages.push(Message::user("alpha beta gamma"));
        store.save_session(&session).unwrap();

        let results = store.search_sessions("alpha", None).unwrap();
        assert_eq!(results.len(), 1);

        // Update session with different content
        session.messages.clear();
        session.messages.push(Message::user("delta epsilon zeta"));
        store.save_session(&session).unwrap();

        // Old content should no longer match
        let results = store.search_sessions("alpha", None).unwrap();
        assert!(results.is_empty());

        // New content should match
        let results = store.search_sessions("delta", None).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_fts_cleaned_on_delete() {
        let store = setup();
        let agent_id = AgentId::new();
        let mut session = store.create_session(agent_id).unwrap();
        session
            .messages
            .push(Message::user("searchable content here"));
        store.save_session(&session).unwrap();

        let results = store.search_sessions("searchable", None).unwrap();
        assert_eq!(results.len(), 1);

        store.delete_session(session.id).unwrap();

        let results = store.search_sessions("searchable", None).unwrap();
        assert!(results.is_empty());
    }

    /// list_sessions surfaces per-session aggregates (cost_usd, total_tokens,
    /// duration_ms) so the Overview "Recent sessions" table can render them
    /// without a follow-up round-trip per row. Cost/tokens come from a
    /// LEFT JOIN on usage_events keyed by session_id (schema v30); duration
    /// is computed from the message timestamps already in the messages blob.
    #[test]
    fn list_sessions_includes_cost_tokens_duration_aggregates() {
        let conn = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        run_migrations(&conn.lock().unwrap()).unwrap();
        let store = SessionStore::new(Arc::clone(&conn));

        let agent_id = AgentId::new();
        let mut session = store.create_session(agent_id).unwrap();

        // Two messages 5s apart so duration_ms = 5000. We override the
        // timestamps Message::* would otherwise stamp at "now".
        let t0 = chrono::Utc::now();
        let t1 = t0 + chrono::Duration::seconds(5);
        let mut user = Message::user("hi");
        user.timestamp = Some(t0);
        let mut asst = Message::assistant("ack");
        asst.timestamp = Some(t1);
        session.messages.push(user);
        session.messages.push(asst);
        store.save_session(&session).unwrap();

        // Two usage_events tagged to this session: one tagged, one NULL.
        // The aggregate must include only the tagged one.
        {
            let c = conn.lock().unwrap();
            c.execute(
                "INSERT INTO usage_events (id, agent_id, timestamp, model, provider, input_tokens, output_tokens, cost_usd, tool_calls, latency_ms, session_id)
                 VALUES (?1, ?2, datetime('now'), 'm', 'p', 100, 50, 0.012, 0, 0, ?3)",
                rusqlite::params![
                    uuid::Uuid::new_v4().to_string(),
                    agent_id.0.to_string(),
                    session.id.0.to_string(),
                ],
            )
            .unwrap();
            c.execute(
                "INSERT INTO usage_events (id, agent_id, timestamp, model, provider, input_tokens, output_tokens, cost_usd, tool_calls, latency_ms, session_id)
                 VALUES (?1, ?2, datetime('now'), 'm', 'p', 999, 999, 9.99, 0, 0, NULL)",
                rusqlite::params![
                    uuid::Uuid::new_v4().to_string(),
                    agent_id.0.to_string(),
                ],
            )
            .unwrap();
        }

        let listed = store.list_sessions().unwrap();
        let row = listed
            .iter()
            .find(|v| v["session_id"].as_str() == Some(&session.id.0.to_string()))
            .expect("created session must be listed");

        assert_eq!(row["message_count"].as_u64(), Some(2));
        // 5s span between the two stamped messages — within ~50ms tolerance.
        let duration_ms = row["duration_ms"].as_i64().expect("duration present");
        assert!(
            (4900..=5100).contains(&duration_ms),
            "duration_ms = {} not within tolerance",
            duration_ms,
        );
        assert!((row["cost_usd"].as_f64().unwrap() - 0.012).abs() < 1e-9);
        assert_eq!(row["total_tokens"].as_u64(), Some(150));
    }

    /// Sessions with no usage_events still list with cost_usd=0 and
    /// total_tokens=0 (NOT null). duration_ms is null when fewer than two
    /// timestamped messages exist.
    #[test]
    fn list_sessions_zero_aggregates_for_unmetered_session() {
        let store = setup();
        let agent_id = AgentId::new();
        let session = store.create_session(agent_id).unwrap();

        let listed = store.list_sessions().unwrap();
        let row = listed
            .iter()
            .find(|v| v["session_id"].as_str() == Some(&session.id.0.to_string()))
            .expect("created session must be listed");

        assert_eq!(row["cost_usd"].as_f64(), Some(0.0));
        assert_eq!(row["total_tokens"].as_u64(), Some(0));
        assert!(row["duration_ms"].is_null());
    }
}
