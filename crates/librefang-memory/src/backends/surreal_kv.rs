//! SurrealDB-backed [`crate::KvBackend`] implementation.
//!
//! Persists per-agent key-value data into the `kv_store` table defined by
//! migration v6 (`006_kv_store.surql`).
//!
//! All SurrealDB operations use `serde_json::Value` results to avoid the
//! `SurrealValue` derive requirement on internal structs.

use crate::backend::KvBackend;
use librefang_storage::pool::SurrealSession;
use librefang_types::agent::AgentId;
use librefang_types::error::{LibreFangError, LibreFangResult};
use serde_json::Value as JsonValue;
use std::sync::Arc;
use surrealdb::{engine::any::Any, Surreal};
use tokio::runtime::Handle;
use tokio::task;

/// SurrealDB-backed implementation of [`KvBackend`].
pub struct SurrealKvBackend {
    db: Arc<Surreal<Any>>,
}

impl SurrealKvBackend {
    /// Open the KV backend against an existing [`SurrealSession`].
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

    /// Derive a deterministic SurrealDB record ID from `agent_id` + `key`.
    ///
    /// Characters that are not valid in SurrealDB bare record IDs are replaced
    /// with underscores.  The separator `__k__` is chosen to minimise
    /// collisions with real agent/key strings.
    fn record_id(agent_id: &str, key: &str) -> String {
        let safe = |s: &str| {
            s.chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '-' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect::<String>()
        };
        format!("{}__k__{}", safe(agent_id), safe(key))
    }
}

impl KvBackend for SurrealKvBackend {
    fn structured_get(&self, agent_id: AgentId, key: &str) -> LibreFangResult<Option<JsonValue>> {
        let rid = Self::record_id(&agent_id.to_string(), key);
        let db = self.db.clone();
        let row: Option<JsonValue> = self.block_on(async move {
            db.select(("kv_store", rid.as_str()))
                .await
                .map_err(|e| LibreFangError::memory_msg(e.to_string()))
        })?;
        // Return the `value` sub-field (or `null` if missing).
        Ok(row.map(|v| v["value"].clone()))
    }

    fn structured_set(
        &self,
        agent_id: AgentId,
        key: &str,
        value: JsonValue,
    ) -> LibreFangResult<()> {
        let agent = agent_id.to_string();
        let key = key.to_string();
        let rid = Self::record_id(&agent, &key);
        let db = self.db.clone();
        self.block_on(async move {
            let existing: Option<JsonValue> = db
                .select(("kv_store", rid.as_str()))
                .await
                .map_err(|e| LibreFangError::memory_msg(e.to_string()))?;
            let version = existing
                .as_ref()
                .and_then(|v| v["version"].as_i64())
                .map(|v| v + 1)
                .unwrap_or(1);

            let payload = serde_json::json!({
                "agent_id": agent,
                "key": key,
                "value": value,
                "version": version,
                "updated_at": chrono::Utc::now().to_rfc3339(),
            });
            let _: Option<JsonValue> = db
                .upsert(("kv_store", rid.as_str()))
                .content(payload)
                .await
                .map_err(|e| LibreFangError::memory_msg(e.to_string()))?;
            Ok(())
        })
    }

    fn structured_delete(&self, agent_id: AgentId, key: &str) -> LibreFangResult<()> {
        let rid = Self::record_id(&agent_id.to_string(), key);
        let db = self.db.clone();
        self.block_on(async move {
            let _: Option<JsonValue> = db
                .delete(("kv_store", rid.as_str()))
                .await
                .map_err(|e| LibreFangError::memory_msg(e.to_string()))?;
            Ok(())
        })
    }
}
