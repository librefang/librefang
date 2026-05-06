//! SurrealDB-backed [`crate::TaskBackend`] implementation.
//!
//! Manages the per-agent task queue via the `task_queue` table defined by
//! migration v7 (`007_task_queue.surql`).

use crate::backend::TaskBackend;
use async_trait::async_trait;
use librefang_storage::pool::SurrealSession;
use librefang_types::error::{LibreFangError, LibreFangResult};
use serde_json::Value as JsonValue;
use std::sync::Arc;
use surrealdb::{engine::any::Any, Surreal};

/// SurrealDB-backed implementation of [`TaskBackend`].
pub struct SurrealTaskBackend {
    db: Arc<Surreal<Any>>,
}

impl SurrealTaskBackend {
    /// Open the task backend against an existing [`SurrealSession`].
    #[must_use]
    pub fn open(session: &SurrealSession) -> Self {
        Self {
            db: Arc::new(session.client().clone()),
        }
    }
}

#[async_trait]
impl TaskBackend for SurrealTaskBackend {
    async fn task_reset_stuck(
        &self,
        ttl_secs: u64,
        _max_retries: u32,
    ) -> LibreFangResult<Vec<String>> {
        let cutoff = (chrono::Utc::now()
            - chrono::Duration::from_std(std::time::Duration::from_secs(ttl_secs))
                .unwrap_or(chrono::Duration::seconds(ttl_secs as i64)))
        .to_rfc3339();

        let rows: Vec<JsonValue> = self
            .db
            .query(
                "UPDATE task_queue SET status = 'pending' \
                 WHERE status = 'running' AND updated_at < $cutoff \
                 RETURN id",
            )
            .bind(("cutoff", cutoff))
            .await
            .map_err(|e| LibreFangError::memory_msg(e.to_string()))?
            .take(0)
            .map_err(|e| LibreFangError::memory_msg(e.to_string()))?;

        let ids: Vec<String> = rows
            .into_iter()
            .filter_map(|v| {
                v["id"]
                    .as_str()
                    .map(|s| s.strip_prefix("task_queue:").unwrap_or(s).to_string())
            })
            .collect();

        Ok(ids)
    }
}
