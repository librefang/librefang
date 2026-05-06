//! SurrealDB-backed [`crate::SessionBackend`] implementation.
//!
//! Persists session history and canonical sessions into SurrealDB tables
//! defined by migration v5 (`005_sessions.surql`).
//!
//! ## Design notes
//!
//! - All SurrealDB round-trips use `serde_json::Value` (which implements
//!   `SurrealValue`) to avoid requiring the `surrealdb::sql::Thing` derive
//!   on internal row types.
//! - Session record IDs are stored as the session UUID string.
//! - `canonical_sessions` record IDs are the agent UUID string so there is
//!   exactly one canonical session per agent.

use crate::backend::SessionBackend;
use librefang_storage::pool::SurrealSession;
use librefang_types::agent::{AgentId, SessionId};
use librefang_types::error::{LibreFangError, LibreFangResult};
use librefang_types::message::Message;
use serde_json::Value as JsonValue;
use std::sync::Arc;
use surrealdb::{engine::any::Any, Surreal};
use tokio::runtime::Handle;
use tokio::task;

use crate::session::Session;

/// SurrealDB-backed implementation of [`SessionBackend`].
pub struct SurrealSessionBackend {
    db: Arc<Surreal<Any>>,
}

impl SurrealSessionBackend {
    /// Open the session backend against an existing [`SurrealSession`].
    #[must_use]
    pub fn open(session: &SurrealSession) -> Self {
        Self {
            db: Arc::new(session.client().clone()),
        }
    }

    fn block_on<F, T>(&self, fut: F) -> T
    where
        F: std::future::Future<Output = T>,
    {
        match Handle::try_current() {
            Ok(handle) => task::block_in_place(|| handle.block_on(fut)),
            Err(_) => tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to build temporary tokio runtime")
                .block_on(fut),
        }
    }

    fn row_to_session(row: &JsonValue) -> LibreFangResult<Session> {
        let id_raw = row["id"]
            .as_str()
            .ok_or_else(|| LibreFangError::memory_msg("session row missing id"))?;
        // SurrealDB returns record IDs as "sessions:UUID" — strip the table prefix.
        let id_str = id_raw.strip_prefix("sessions:").unwrap_or(id_raw);
        let session_id: SessionId = id_str
            .parse()
            .map_err(|_| LibreFangError::memory_msg(format!("invalid session id: {id_str}")))?;

        let agent_raw = row["agent_id"]
            .as_str()
            .ok_or_else(|| LibreFangError::memory_msg("session row missing agent_id"))?;
        let agent_id: AgentId = agent_raw
            .parse()
            .map_err(|_| LibreFangError::memory_msg(format!("invalid agent id: {agent_raw}")))?;

        let messages: Vec<Message> =
            serde_json::from_value(row["messages"].clone()).unwrap_or_default();

        let context_window_tokens = row["context_window_tokens"].as_u64().unwrap_or(0);
        let label = row["label"].as_str().map(str::to_string);

        Ok(Session {
            id: session_id,
            agent_id,
            messages,
            context_window_tokens,
            label,
            // Generation counter starts at 0 on cold-load; the repair pass
            // will set last_repaired_generation once it runs.
            messages_generation: 0,
            last_repaired_generation: None,
        })
    }
}

impl SessionBackend for SurrealSessionBackend {
    fn get_session(&self, session_id: SessionId) -> LibreFangResult<Option<Session>> {
        let id = session_id.to_string();
        let db = self.db.clone();
        let row: Option<JsonValue> = self.block_on(async move {
            db.select(("sessions", id.as_str()))
                .await
                .map_err(|e| LibreFangError::memory_msg(e.to_string()))
        })?;
        row.as_ref().map(Self::row_to_session).transpose()
    }

    fn save_session(&self, session: &Session) -> LibreFangResult<()> {
        let id = session.id.to_string();
        let agent_id = session.agent_id.to_string();
        let messages = serde_json::to_value(&session.messages)
            .map_err(|e| LibreFangError::memory_msg(format!("serialise messages: {e}")))?;
        let label = session.label.clone();
        let context_window_tokens = session.context_window_tokens;
        let message_count = session.messages.len() as i64;
        let now = chrono::Utc::now().to_rfc3339();
        let db = self.db.clone();

        self.block_on(async move {
            let existing: Option<JsonValue> = db
                .select(("sessions", id.as_str()))
                .await
                .map_err(|e| LibreFangError::memory_msg(e.to_string()))?;
            let created_at = existing
                .as_ref()
                .and_then(|v| v["created_at"].as_str())
                .unwrap_or(&now)
                .to_string();

            let payload = serde_json::json!({
                "agent_id": agent_id,
                "messages": messages,
                "context_window_tokens": context_window_tokens,
                "message_count": message_count,
                "label": label,
                "created_at": created_at,
                "updated_at": now,
            });
            let _: Option<JsonValue> = db
                .upsert(("sessions", id.as_str()))
                .content(payload)
                .await
                .map_err(|e| LibreFangError::memory_msg(e.to_string()))?;
            Ok(())
        })
    }

    fn get_agent_session_ids(&self, agent_id: AgentId) -> LibreFangResult<Vec<SessionId>> {
        let agent = agent_id.to_string();
        let db = self.db.clone();
        self.block_on(async move {
            let rows: Vec<JsonValue> = db
                .query("SELECT id FROM sessions WHERE agent_id = $agent_id")
                .bind(("agent_id", agent))
                .await
                .map_err(|e| LibreFangError::memory_msg(e.to_string()))?
                .take(0)
                .map_err(|e| LibreFangError::memory_msg(e.to_string()))?;

            let mut ids = Vec::new();
            for row in rows {
                if let Some(id_str) = row["id"].as_str() {
                    let raw = id_str.strip_prefix("sessions:").unwrap_or(id_str);
                    if let Ok(sid) = raw.parse::<SessionId>() {
                        ids.push(sid);
                    }
                }
            }
            Ok(ids)
        })
    }

    fn delete_agent_sessions(&self, agent_id: AgentId) -> LibreFangResult<()> {
        let agent = agent_id.to_string();
        let db = self.db.clone();
        self.block_on(async move {
            db.query("DELETE sessions WHERE agent_id = $agent_id")
                .bind(("agent_id", agent))
                .await
                .map_err(|e| LibreFangError::memory_msg(e.to_string()))?;
            Ok(())
        })
    }

    fn append_canonical(
        &self,
        agent_id: AgentId,
        messages: &[Message],
        _compaction_threshold: Option<usize>,
        _session_id: Option<SessionId>,
    ) -> LibreFangResult<()> {
        let agent = agent_id.to_string();
        let new_msgs = messages.to_vec();
        let db = self.db.clone();
        self.block_on(async move {
            let existing: Option<JsonValue> = db
                .select(("canonical_sessions", agent.as_str()))
                .await
                .map_err(|e| LibreFangError::memory_msg(e.to_string()))?;

            let mut all_msgs: Vec<Message> = existing
                .as_ref()
                .map(|v| {
                    serde_json::from_value::<Vec<Message>>(v["messages"].clone())
                        .unwrap_or_default()
                })
                .unwrap_or_default();
            let compaction_cursor = existing
                .as_ref()
                .and_then(|v| v["compaction_cursor"].as_u64())
                .unwrap_or(0);
            let compacted_summary = existing
                .as_ref()
                .and_then(|v| v["compacted_summary"].as_str())
                .map(str::to_string);

            all_msgs.extend(new_msgs);

            let msgs_value = serde_json::to_value(&all_msgs)
                .map_err(|e| LibreFangError::memory_msg(format!("serialise: {e}")))?;

            let payload = serde_json::json!({
                "agent_id": agent,
                "messages": msgs_value,
                "compaction_cursor": compaction_cursor,
                "compacted_summary": compacted_summary,
                "updated_at": chrono::Utc::now().to_rfc3339(),
            });

            let _: Option<JsonValue> = db
                .upsert(("canonical_sessions", agent.as_str()))
                .content(payload)
                .await
                .map_err(|e| LibreFangError::memory_msg(e.to_string()))?;
            Ok(())
        })
    }

    fn delete_canonical_session(&self, agent_id: AgentId) -> LibreFangResult<()> {
        let agent = agent_id.to_string();
        let db = self.db.clone();
        self.block_on(async move {
            let _: Option<JsonValue> = db
                .delete(("canonical_sessions", agent.as_str()))
                .await
                .map_err(|e| LibreFangError::memory_msg(e.to_string()))?;
            Ok(())
        })
    }
}
