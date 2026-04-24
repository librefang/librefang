//! SurrealDB-backed [`crate::MemoryBackend`] implementation.
//!
//! This is the Phase 5 deliverable of the `surrealdb-storage-swap` plan. It
//! provides the same narrow agent-CRUD surface as the legacy
//! [`crate::MemorySubstrate`] but persists into a SurrealDB 3.0 instance
//! opened through [`librefang_storage::pool::SurrealConnectionPool`].
//!
//! The richer [`surreal_memory::MemoryStorage`] surface (TaskStreams, hybrid
//! BM25+HNSW search, knowledge graph, scoped memory, etc.) is exposed via
//! [`SurrealMemoryBackend::extended`] for callers that opt in. We keep the
//! extended handle optional because constructing `SurrealStorage` requires
//! an embedding service, which librefang wires up separately from the basic
//! agent registry.

use crate::backend::MemoryBackend;
use async_trait::async_trait;
use librefang_storage::pool::SurrealSession;
use librefang_types::agent::{AgentEntry, AgentId};
use librefang_types::error::{LibreFangError, LibreFangResult};
use serde_json::Value as JsonValue;
use std::sync::Arc;
use surrealdb::{engine::any::Any, Surreal};
use tokio::runtime::Handle;
use tokio::task;

/// SurrealDB-backed implementation of [`MemoryBackend`].
///
/// Holds a [`SurrealSession`] dedicated to librefang's operational tables
/// (currently only `agents`). When the optional [`surreal_memory::SurrealStorage`]
/// handle is attached via [`SurrealMemoryBackend::with_extended`], richer
/// memory APIs become available through [`SurrealMemoryBackend::extended`].
///
/// # Multi-tenancy
///
/// The backing [`SurrealSession`] is the only ns/db scope this struct uses.
/// Two `SurrealMemoryBackend`s opened on the same pool with different
/// sessions can coexist on a single SurrealDB instance — see Phase 4b of
/// the plan for the cross-tenant story.
pub struct SurrealMemoryBackend {
    session: SurrealSession,
    db: Arc<Surreal<Any>>,
    extended: Option<Arc<surreal_memory::SurrealStorage>>,
}

impl SurrealMemoryBackend {
    /// Build a new backend over an opened [`SurrealSession`] and ensure the
    /// `agents` table exists.
    ///
    /// # Errors
    ///
    /// Returns [`LibreFangError::Memory`] if the schema bootstrap query fails.
    pub async fn new(session: SurrealSession) -> LibreFangResult<Self> {
        let db = Arc::new(session.client().clone());
        // SCHEMALESS so the `AgentEntry` struct can evolve without a
        // migration on every field addition. Persistence still goes through
        // serde so we keep typed round-trips.
        // SCHEMALESS lets us add fields without a migration; we only declare
        // a secondary index for `name` lookups. Strict TYPE enforcement on
        // `updated_at` is intentionally omitted because the JSON content path
        // sends RFC3339 strings, not `datetime` literals — see
        // <https://surrealdb.com/docs/surrealdb/surrealql/datamodel/datetimes>
        // for the coercion rules.
        let bootstrap = "
            DEFINE TABLE IF NOT EXISTS agents SCHEMALESS;
            DEFINE INDEX IF NOT EXISTS agents_name_idx ON agents COLUMNS name;
        ";
        db.query(bootstrap)
            .await
            .map_err(|e| LibreFangError::Memory(format!("agents bootstrap: {e}")))?;
        Ok(Self {
            session,
            db,
            extended: None,
        })
    }

    /// Attach a [`surreal_memory::SurrealStorage`] handle for the richer
    /// memory APIs. Idempotent — the last call wins.
    #[must_use]
    pub fn with_extended(mut self, extended: Arc<surreal_memory::SurrealStorage>) -> Self {
        self.extended = Some(extended);
        self
    }

    /// Borrow the underlying SurrealDB session metadata. Useful when callers
    /// need the active namespace/database labels for diagnostics.
    #[must_use]
    pub fn session(&self) -> &SurrealSession {
        &self.session
    }

    /// Borrow the extended [`surreal_memory::MemoryStorage`] handle, when
    /// configured. Returns `None` when the backend was constructed without
    /// an embedding service.
    ///
    /// Phase 6+ will widen the [`MemoryBackend`] trait so most callers can
    /// stop reaching for this directly.
    #[must_use]
    pub fn extended(&self) -> Option<&surreal_memory::SurrealStorage> {
        self.extended.as_deref()
    }

    /// Run an async block to completion from a sync trait method. Requires a
    /// multi-threaded tokio runtime (which librefang always uses). This is
    /// the same pattern Phase 5 uses for every sync wrapper around an async
    /// SurrealDB call. Falls back to a temporary runtime when called from a
    /// plain `#[test]` thread that has no active Tokio context.
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
}

/// Build the JSON row that we persist. We keep the full [`AgentEntry`] under
/// `entry` plus a denormalised `name` column for cheap indexed lookups.
/// Storing `serde_json::Value` (which implements
/// [`surrealdb::types::SurrealValue`]) avoids needing a `SurrealValue` derive
/// on every upstream type.
///
/// We deliberately store the modification time as a Unix epoch-millis integer
/// in `updated_at_ms` rather than an RFC3339 string. SurrealDB 3.0's
/// schemaless content path auto-coerces ISO-8601 strings into the native
/// `datetime` type and then refuses to round-trip them as JSON, so an
/// integer side-steps the coercion entirely. The authoritative timestamps
/// for an agent live inside the embedded `entry` JSON itself
/// (`created_at` / `last_active`).
fn row_for(entry: &AgentEntry) -> LibreFangResult<JsonValue> {
    let entry_json =
        serde_json::to_value(entry).map_err(|e| LibreFangError::Serialization(e.to_string()))?;
    let updated_at_ms = chrono::Utc::now().timestamp_millis();
    Ok(serde_json::json!({
        "id": entry.id.0.to_string(),
        "name": entry.name,
        "entry": entry_json,
        "updated_at_ms": updated_at_ms,
    }))
}

fn entry_from_row(row: &JsonValue) -> LibreFangResult<AgentEntry> {
    let inner = row
        .get("entry")
        .ok_or_else(|| LibreFangError::Serialization("missing entry field".into()))?;
    serde_json::from_value(inner.clone()).map_err(|e| LibreFangError::Serialization(e.to_string()))
}

#[async_trait]
impl MemoryBackend for SurrealMemoryBackend {
    fn save_agent(&self, entry: &AgentEntry) -> LibreFangResult<()> {
        let row = row_for(entry)?;
        let id = entry.id.0.to_string();
        let db = Arc::clone(&self.db);
        self.block_on(async move {
            let _: Option<JsonValue> = db
                .upsert(("agents", id.as_str()))
                .content(row)
                .await
                .map_err(|e| LibreFangError::Memory(format!("save_agent: {e}")))?;
            Ok::<_, LibreFangError>(())
        })
    }

    fn load_agent(&self, agent_id: AgentId) -> LibreFangResult<Option<AgentEntry>> {
        let id = agent_id.0.to_string();
        let db = Arc::clone(&self.db);
        let row: Option<JsonValue> = self.block_on(async move {
            db.select(("agents", id.as_str()))
                .await
                .map_err(|e| LibreFangError::Memory(format!("load_agent: {e}")))
        })?;
        row.as_ref().map(entry_from_row).transpose()
    }

    fn remove_agent(&self, agent_id: AgentId) -> LibreFangResult<()> {
        let id = agent_id.0.to_string();
        let db = Arc::clone(&self.db);
        self.block_on(async move {
            let _: Option<JsonValue> = db
                .delete(("agents", id.as_str()))
                .await
                .map_err(|e| LibreFangError::Memory(format!("remove_agent: {e}")))?;
            Ok::<_, LibreFangError>(())
        })
    }

    fn load_all_agents(&self) -> LibreFangResult<Vec<AgentEntry>> {
        let db = Arc::clone(&self.db);
        let rows: Vec<JsonValue> = self.block_on(async move {
            db.select("agents")
                .await
                .map_err(|e| LibreFangError::Memory(format!("load_all_agents: {e}")))
        })?;
        rows.iter().map(entry_from_row).collect()
    }
}

impl std::fmt::Debug for SurrealMemoryBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SurrealMemoryBackend")
            .field("session", &self.session)
            .field("extended_attached", &self.extended.is_some())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_storage::config::{StorageBackendKind, StorageConfig};
    use librefang_storage::pool::SurrealConnectionPool;
    use librefang_types::agent::{AgentId, AgentManifest, AgentMode, AgentState, SessionId};
    use tempfile::tempdir;

    fn make_entry(name: &str) -> AgentEntry {
        let id = AgentId(uuid::Uuid::new_v4());
        let session_id = SessionId(uuid::Uuid::new_v4());
        AgentEntry {
            id,
            name: name.to_string(),
            manifest: AgentManifest::default(),
            state: AgentState::Created,
            mode: AgentMode::default(),
            created_at: chrono::Utc::now(),
            last_active: chrono::Utc::now(),
            parent: None,
            children: vec![],
            session_id,
            source_toml_path: None,
            tags: vec!["test".into()],
            identity: Default::default(),
            onboarding_completed: false,
            onboarding_completed_at: None,
            is_hand: false,
            force_session_wipe: false,
            resume_pending: false,
            reset_reason: None,
        }
    }

    async fn fresh_backend() -> SurrealMemoryBackend {
        let dir = tempdir().unwrap();
        let cfg = StorageConfig {
            backend: StorageBackendKind::embedded(dir.path().join("agents.surreal")),
            namespace: "librefang".into(),
            database: "main".into(),
            legacy_sqlite_path: None,
        };
        // Leak the tempdir so the path stays alive for the lifetime of the
        // test process — we don't care about cleanup, just isolation.
        std::mem::forget(dir);
        let pool = SurrealConnectionPool::new();
        let session = pool.open(&cfg).await.expect("open embedded");
        SurrealMemoryBackend::new(session)
            .await
            .expect("bootstrap agents table")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn round_trip_agent_entry() {
        let backend = fresh_backend().await;
        let entry = make_entry("alice");
        backend.save_agent(&entry).expect("save");
        let loaded = backend
            .load_agent(entry.id)
            .expect("load")
            .expect("agent present");
        assert_eq!(loaded.id, entry.id);
        assert_eq!(loaded.name, "alice");
        assert_eq!(loaded.tags, vec!["test".to_string()]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn list_then_remove_agent() {
        let backend = fresh_backend().await;
        let a = make_entry("alpha");
        let b = make_entry("beta");
        backend.save_agent(&a).unwrap();
        backend.save_agent(&b).unwrap();

        let mut listed = backend.load_all_agents().unwrap();
        listed.sort_by(|x, y| x.name.cmp(&y.name));
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].name, "alpha");
        assert_eq!(listed[1].name, "beta");

        backend.remove_agent(a.id).unwrap();
        assert!(backend.load_agent(a.id).unwrap().is_none());
        assert_eq!(backend.load_all_agents().unwrap().len(), 1);
    }
}
