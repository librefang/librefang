//! SurrealDB-backed [`TraceBackend`] implementation.
//!
//! Persists hook traces and circuit-breaker states through the shared
//! [`librefang_storage::SurrealSession`]. Schema is owned by the
//! migration runner in `librefang-storage::migrations` (see migrations
//! v2 and v3).
//!
//! ## Design notes
//!
//! - `started_at` is stored both as the original RFC-3339 string (for
//!   readability and parity with the rusqlite path) and as
//!   `started_at_ms` (Unix-millis integer) so SurrealDB can index time
//!   ranges without paying the auto-coercion penalty documented in
//!   Phase 5.
//! - `input_preview` / `output_preview` are JSON-encoded into strings
//!   so the schema is fixed and we never depend on `serde_json::Value`
//!   surviving SurrealDB's schemaful round-trip.
//! - `insert` is fire-and-forget (just like the rusqlite version) and
//!   only logs at `warn!` on failure — traces are non-critical
//!   telemetry and must never propagate errors that would abort a hook.
//! - Pruning to the last 10 000 rows runs lazily inside `insert`; the
//!   query is `DELETE FROM hook_traces WHERE id NOT IN (latest 10 000)`,
//!   mirroring the rusqlite implementation.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use librefang_storage::SurrealSession;
use serde::{Deserialize, Serialize};
use surrealdb::{engine::any::Any, Surreal};
use tokio::runtime::Handle;
use tracing::warn;

use crate::context_engine::HookTrace;
use crate::storage_backends::{TraceBackend, TraceBackendError, TraceBackendResult};

/// Maximum number of trace rows kept in SurrealDB.
///
/// Same value as the rusqlite [`crate::trace_store::TraceStore`] uses; we
/// prune lazily inside [`SurrealTraceBackend::insert`] so a misbehaving
/// agent cannot fill the database.
const MAX_TRACE_ROWS: u64 = 10_000;

/// SurrealDB-backed implementation of [`TraceBackend`].
///
/// Construct via [`SurrealTraceBackend::open`]. The store does not keep
/// any in-memory state; every call is a SurrealDB round-trip.
pub struct SurrealTraceBackend {
    db: Surreal<Any>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CircuitRow {
    key: String,
    failures: i64,
    opened_at: Option<String>,
}

impl SurrealTraceBackend {
    /// Open the trace backend against an existing [`SurrealSession`].
    ///
    /// The session must already be signed in and have its
    /// namespace/database selected (the connection pool handles that at
    /// construction time). The schema is expected to already be present
    /// — callers run the operational migrations (see
    /// [`librefang_storage::migrations`]) before calling this.
    #[must_use]
    pub fn open(session: &SurrealSession) -> Self {
        Self {
            db: session.client().clone(),
        }
    }

    fn insert_inner(&self, plugin: &str, trace: &HookTrace) -> TraceBackendResult<()> {
        let started_at_ms = parse_started_at_ms(&trace.started_at);
        // SurrealDB 3.0 will not coerce JSON `null` into `NONE` for
        // `option<…>` fields, so we build the payload manually and skip
        // optional fields when they're empty.
        let mut payload = serde_json::Map::new();
        payload.insert("trace_id".into(), trace.trace_id.clone().into());
        payload.insert("correlation_id".into(), trace.correlation_id.clone().into());
        payload.insert("plugin".into(), plugin.to_string().into());
        payload.insert("hook".into(), trace.hook.clone().into());
        payload.insert("started_at".into(), trace.started_at.clone().into());
        payload.insert("started_at_ms".into(), started_at_ms.into());
        payload.insert(
            "elapsed_ms".into(),
            i64::try_from(trace.elapsed_ms).unwrap_or(i64::MAX).into(),
        );
        payload.insert("success".into(), trace.success.into());
        if let Some(err) = &trace.error {
            payload.insert("error".into(), err.clone().into());
        }
        if let Ok(s) = serde_json::to_string(&trace.input_preview) {
            payload.insert("input_preview".into(), s.into());
        }
        if let Some(out) = &trace.output_preview {
            if let Ok(s) = serde_json::to_string(out) {
                payload.insert("output_preview".into(), s.into());
            }
        }
        let row_json = serde_json::Value::Object(payload);

        // Use the trace_id as the record id so duplicate inserts (a retry
        // path that re-emits the same trace_id) become an idempotent upsert.
        let id = trace_record_id(&trace.trace_id, started_at_ms);
        block_on(async {
            let _: Option<serde_json::Value> = self
                .db
                .upsert(("hook_traces", id.as_str()))
                .content(row_json)
                .await
                .map_err(|e| TraceBackendError::Backend(e.to_string()))?;
            // Lazy prune to the most recent MAX_TRACE_ROWS rows.
            let _ = self
                .db
                .query(
                    "LET $cutoff = (
                         SELECT started_at_ms FROM hook_traces
                         ORDER BY started_at_ms DESC
                         LIMIT 1 START $skip
                     )[0].started_at_ms;
                     DELETE FROM hook_traces WHERE started_at_ms < $cutoff RETURN NONE;",
                )
                .bind(("skip", MAX_TRACE_ROWS))
                .await
                .map_err(|e| TraceBackendError::Backend(e.to_string()))?;
            Ok::<(), TraceBackendError>(())
        })
    }
}

impl TraceBackend for SurrealTraceBackend {
    fn insert(&self, plugin: &str, trace: &HookTrace) {
        if let Err(e) = self.insert_inner(plugin, trace) {
            warn!(error = %e, plugin, hook = %trace.hook, "failed to persist hook trace");
        }
    }

    fn load_circuit_states(&self) -> TraceBackendResult<HashMap<String, (u32, Option<String>)>> {
        let rows: Vec<serde_json::Value> = block_on(async {
            self.db
                .query("SELECT key, failures, opened_at FROM circuit_breaker_states")
                .await
                .map_err(|e| TraceBackendError::Backend(e.to_string()))?
                .take(0)
                .map_err(|e| TraceBackendError::Backend(e.to_string()))
        })?;

        let mut map = HashMap::with_capacity(rows.len());
        for row in rows {
            match serde_json::from_value::<CircuitRow>(row) {
                Ok(r) => {
                    let failures = u32::try_from(r.failures).unwrap_or(u32::MAX);
                    map.insert(r.key, (failures, r.opened_at));
                }
                Err(e) => warn!(error = %e, "skipping malformed circuit_breaker_states row"),
            }
        }
        Ok(map)
    }

    fn save_circuit_state(
        &self,
        key: &str,
        failures: u32,
        opened_at: Option<&str>,
    ) -> TraceBackendResult<()> {
        let mut payload = serde_json::Map::new();
        payload.insert("key".into(), key.to_string().into());
        payload.insert("failures".into(), i64::from(failures).into());
        if let Some(ts) = opened_at {
            payload.insert("opened_at".into(), ts.to_string().into());
        }
        let row = serde_json::Value::Object(payload);
        let id = sanitise_id(key);
        block_on(async {
            let _: Option<serde_json::Value> = self
                .db
                .upsert(("circuit_breaker_states", id.as_str()))
                .content(row)
                .await
                .map_err(|e| TraceBackendError::Backend(e.to_string()))?;
            Ok::<(), TraceBackendError>(())
        })
    }

    fn delete_circuit_state(&self, key: &str) -> TraceBackendResult<()> {
        let id = sanitise_id(key);
        block_on(async {
            let _: Option<serde_json::Value> = self
                .db
                .delete(("circuit_breaker_states", id.as_str()))
                .await
                .map_err(|e| TraceBackendError::Backend(e.to_string()))?;
            Ok::<(), TraceBackendError>(())
        })
    }
}

fn parse_started_at_ms(started_at: &str) -> i64 {
    DateTime::parse_from_rfc3339(started_at)
        .map(|dt| dt.with_timezone(&Utc).timestamp_millis())
        .unwrap_or_else(|_| Utc::now().timestamp_millis())
}

/// Build a SurrealDB record id for a trace.
///
/// The rusqlite path used an autoincrementing integer; here we combine
/// `trace_id` and `started_at_ms` so re-inserts of the same `trace_id`
/// converge to the same row (idempotent retries) while inserts that
/// somehow share a `trace_id` across different timestamps keep their own
/// rows.
fn trace_record_id(trace_id: &str, started_at_ms: i64) -> String {
    format!("{}_{}", sanitise_id(trace_id), started_at_ms)
}

/// SurrealDB record ids accept a constrained character set without
/// quoting. Replace the few characters that would force the runtime to
/// add backticks (and thereby pollute the on-disk id) with `_`.
fn sanitise_id(input: &str) -> String {
    input
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn block_on<F, T>(fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    match Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| handle.block_on(fut)),
        Err(_) => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build temporary tokio runtime")
            .block_on(fut),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_storage::{StorageBackendKind, StorageConfig, SurrealConnectionPool};
    use tempfile::tempdir;

    async fn open_backend(path: &std::path::Path) -> SurrealTraceBackend {
        let pool = SurrealConnectionPool::new();
        let cfg = StorageConfig {
            backend: StorageBackendKind::embedded(path.to_path_buf()),
            namespace: "librefang".into(),
            database: "main".into(),
            legacy_sqlite_path: None,
        };
        let session = pool.open(&cfg).await.expect("open session");
        librefang_storage::migrations::apply_pending(
            session.client(),
            librefang_storage::migrations::OPERATIONAL_MIGRATIONS,
        )
        .await
        .expect("migrations");
        SurrealTraceBackend::open(&session)
    }

    fn make_trace(hook: &str, success: bool) -> HookTrace {
        HookTrace {
            trace_id: format!("t_{hook}_{success}"),
            correlation_id: "corr-1".to_string(),
            hook: hook.to_string(),
            started_at: "2026-04-22T00:00:00Z".to_string(),
            elapsed_ms: 7,
            success,
            error: if success {
                None
            } else {
                Some("boom".to_string())
            },
            input_preview: serde_json::json!({"hello": "world"}),
            output_preview: if success {
                Some(serde_json::json!({"ok": true}))
            } else {
                None
            },
            annotations: None,
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn circuit_state_round_trip() {
        let dir = tempdir().unwrap();
        let backend = open_backend(&dir.path().join("trace.surreal")).await;

        backend
            .save_circuit_state("plugin::ingest", 3, Some("2026-04-22T00:00:00Z"))
            .expect("save");
        backend
            .save_circuit_state("plugin::after_turn", 0, None)
            .expect("save closed");

        let states = backend.load_circuit_states().expect("load");
        assert_eq!(states.len(), 2);
        let ingest = states.get("plugin::ingest").expect("ingest present");
        assert_eq!(ingest.0, 3);
        assert_eq!(ingest.1.as_deref(), Some("2026-04-22T00:00:00Z"));
        let after_turn = states
            .get("plugin::after_turn")
            .expect("after_turn present");
        assert_eq!(after_turn.0, 0);
        assert!(after_turn.1.is_none());

        backend
            .delete_circuit_state("plugin::ingest")
            .expect("delete");
        let states = backend.load_circuit_states().expect("load again");
        assert_eq!(states.len(), 1);
        assert!(states.contains_key("plugin::after_turn"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn insert_does_not_panic_or_propagate() {
        let dir = tempdir().unwrap();
        let backend = open_backend(&dir.path().join("trace.surreal")).await;

        // Nothing checked here — the contract is "never panic, never raise".
        backend.insert("plugin-a", &make_trace("ingest", true));
        backend.insert("plugin-a", &make_trace("ingest", false));
        backend.insert("plugin-b", &make_trace("after_turn", true));
    }
}
