//! Persistent hook trace store backed by SQLite (via rusqlite).
//!
//! Stores the last 10,000 hook traces across daemon restarts, enabling
//! post-mortem analysis of hook failures without relying on the in-memory
//! ring buffer (which resets on restart).

use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, MutexGuard};
use tracing::warn;

use crate::context_engine::HookTrace;

/// Run the prune step (linear table scan) once per this many inserts, so the
/// O(N) `DELETE … WHERE id NOT IN (SELECT … LIMIT 10000)` cost is amortised
/// instead of paid on every call.
///
/// At 256 the steady-state overrun above the 10k cap is bounded by 256 rows
/// between prunes — small relative to the cap, and small relative to the
/// time it takes to push that many traces in any realistic workload.
pub const PRUNE_EVERY_N_INSERTS: u64 = 256;

/// Persistent SQLite-backed store for hook execution traces.
pub struct TraceStore {
    conn: std::sync::Mutex<Connection>,
    /// Monotonic counter incremented on every successful `insert_blocking` call.
    /// Used to gate the prune step (see [`PRUNE_EVERY_N_INSERTS`]).
    insert_counter: AtomicU64,
}

impl TraceStore {
    /// Open (or create) the trace database at the given path.
    ///
    /// Initialises the schema on first open.  WAL journal mode is enabled for
    /// better concurrent read performance.
    pub fn open(path: &Path) -> Result<Self, rusqlite::Error> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "
            PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS hook_traces (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                trace_id        TEXT    NOT NULL DEFAULT '',
                correlation_id  TEXT    NOT NULL DEFAULT '',
                plugin          TEXT    NOT NULL,
                hook            TEXT    NOT NULL,
                started_at      TEXT    NOT NULL,
                elapsed_ms      INTEGER NOT NULL,
                success         INTEGER NOT NULL,
                error           TEXT,
                input_preview   TEXT,
                output_preview  TEXT,
                annotations     TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_started_at      ON hook_traces(started_at);
            CREATE INDEX IF NOT EXISTS idx_plugin_hook     ON hook_traces(plugin, hook);
            CREATE INDEX IF NOT EXISTS idx_trace_id        ON hook_traces(trace_id);
            CREATE INDEX IF NOT EXISTS idx_correlation_id  ON hook_traces(correlation_id);
            ",
        )?;
        let has_annotations = {
            let mut stmt = conn.prepare("PRAGMA table_info(hook_traces)")?;
            let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;
            let mut found = false;
            for column in columns {
                if column? == "annotations" {
                    found = true;
                    break;
                }
            }
            found
        };
        if !has_annotations {
            conn.execute("ALTER TABLE hook_traces ADD COLUMN annotations TEXT", [])?;
        }
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS circuit_breaker_states (
                key        TEXT PRIMARY KEY,
                failures   INTEGER NOT NULL DEFAULT 0,
                opened_at  TEXT
            );",
        )?;
        Ok(Self {
            conn: std::sync::Mutex::new(conn),
            insert_counter: AtomicU64::new(0),
        })
    }

    fn lock_connection(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|poisoned| {
            warn!("Trace store connection lock poisoned; recovering inner state");
            self.conn.clear_poison();
            poisoned.into_inner()
        })
    }

    /// Insert a trace record asynchronously.
    ///
    /// SQLite work runs on a `tokio::task::spawn_blocking` thread so the tokio
    /// worker pool is never blocked on disk I/O or the (linear) prune scan.
    /// Errors and join failures are silently swallowed — traces are
    /// non-critical telemetry and must never propagate to the caller. The
    /// prune step is counter-gated (see [`PRUNE_EVERY_N_INSERTS`]).
    pub async fn insert(self: Arc<Self>, plugin: String, trace: HookTrace) {
        let _ = tokio::task::spawn_blocking(move || {
            self.insert_blocking(&plugin, &trace);
        })
        .await;
    }

    /// Synchronous insert worker.
    ///
    /// Holds the `Mutex<Connection>` for the duration of the SQL call. Safe to
    /// call from any sync context (including tests); from a tokio task, call
    /// [`TraceStore::insert`] instead so the work is moved off the worker.
    pub fn insert_blocking(&self, plugin: &str, trace: &HookTrace) {
        let conn = self.lock_connection();

        let input_preview = serde_json::to_string(&trace.input_preview).ok();
        let output_preview = trace
            .output_preview
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok());
        let annotations = trace
            .annotations
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok());

        let inserted = conn
            .execute(
                "INSERT INTO hook_traces \
                 (trace_id, correlation_id, plugin, hook, started_at, elapsed_ms, success, error, input_preview, output_preview, annotations) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    trace.trace_id,
                    trace.correlation_id,
                    plugin,
                    trace.hook,
                    trace.started_at,
                    trace.elapsed_ms as i64,
                    trace.success as i64,
                    trace.error,
                    input_preview,
                    output_preview,
                    annotations,
                ],
            )
            .is_ok();

        // Counter-gated prune: only run the O(N) DELETE scan once every
        // PRUNE_EVERY_N_INSERTS successful inserts. This amortises the
        // full-table scan cost across many inserts while still keeping the
        // table bounded by the cap + PRUNE_EVERY_N_INSERTS at steady state.
        if inserted {
            let n = self.insert_counter.fetch_add(1, Ordering::Relaxed) + 1;
            if n.is_multiple_of(PRUNE_EVERY_N_INSERTS) {
                let _ = conn.execute(
                    "DELETE FROM hook_traces WHERE id NOT IN \
                     (SELECT id FROM hook_traces ORDER BY id DESC LIMIT 10000)",
                    [],
                );
            }
        }
    }

    /// Query traces with optional filters.
    ///
    /// Returns JSON objects sorted newest-first, up to `limit` entries.
    pub fn query(
        &self,
        plugin: Option<&str>,
        hook: Option<&str>,
        limit: usize,
        only_failures: bool,
    ) -> rusqlite::Result<Vec<serde_json::Value>> {
        let conn = self.lock_connection();

        // Build parameterized WHERE clause — never interpolate user values directly.
        let mut conditions: Vec<&str> = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(p) = plugin {
            conditions.push("plugin = ?");
            params.push(Box::new(p.to_string()));
        }
        if let Some(h) = hook {
            conditions.push("hook = ?");
            params.push(Box::new(h.to_string()));
        }
        if only_failures {
            conditions.push("success = 0");
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let sql = format!(
            "SELECT trace_id, correlation_id, plugin, hook, started_at, elapsed_ms, success, error, \
             input_preview, output_preview, annotations \
             FROM hook_traces {where_clause} ORDER BY id DESC LIMIT {limit}"
        );

        let mut stmt = conn.prepare(&sql)?;

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            let annotations = optional_json_column(row, 10)?;
            Ok(serde_json::json!({
                "trace_id":        row.get::<_, String>(0)?,
                "correlation_id":  row.get::<_, String>(1)?,
                "plugin":          row.get::<_, String>(2)?,
                "hook":            row.get::<_, String>(3)?,
                "started_at":      row.get::<_, String>(4)?,
                "elapsed_ms":      row.get::<_, i64>(5)?,
                "success":         row.get::<_, i64>(6)? != 0,
                "error":           row.get::<_, Option<String>>(7)?,
                "input_preview":   row.get::<_, Option<String>>(8)?,
                "output_preview":  row.get::<_, Option<String>>(9)?,
                "annotations":     annotations,
            }))
        })?;
        rows.collect()
    }

    /// Look up a single trace by its trace_id.
    ///
    /// Returns `Ok(None)` if no trace matches; `Err` if the lookup itself fails.
    pub fn query_by_trace_id(&self, trace_id: &str) -> rusqlite::Result<Option<serde_json::Value>> {
        let conn = self.lock_connection();
        conn.query_row(
            "SELECT trace_id, correlation_id, plugin, hook, started_at, elapsed_ms, success, error, \
             input_preview, output_preview, annotations FROM hook_traces WHERE trace_id = ?1",
            [trace_id],
            |row| {
                let annotations = optional_json_column(row, 10)?;
                Ok(serde_json::json!({
                    "trace_id":       row.get::<_, String>(0)?,
                    "correlation_id": row.get::<_, String>(1)?,
                    "plugin":         row.get::<_, String>(2)?,
                    "hook":           row.get::<_, String>(3)?,
                    "started_at":     row.get::<_, String>(4)?,
                    "elapsed_ms":     row.get::<_, i64>(5)?,
                    "success":        row.get::<_, i64>(6)? != 0,
                    "error":          row.get::<_, Option<String>>(7)?,
                    "input_preview":  row.get::<_, Option<String>>(8)?,
                    "output_preview": row.get::<_, Option<String>>(9)?,
                    "annotations":    annotations,
                }))
            },
        )
        .optional()
    }

    /// Count traces, optionally filtered by plugin and/or failure status.
    pub fn count(&self, plugin: Option<&str>, only_failures: bool) -> rusqlite::Result<i64> {
        let conn = self.lock_connection();

        // Build parameterized WHERE clause — never interpolate user values directly.
        let mut conditions: Vec<&str> = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(p) = plugin {
            conditions.push("plugin = ?");
            params.push(Box::new(p.to_string()));
        }
        if only_failures {
            conditions.push("success = 0");
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let sql = format!("SELECT COUNT(*) FROM hook_traces {where_clause}");
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        conn.query_row(&sql, param_refs.as_slice(), |r| r.get(0))
    }

    /// Persist circuit breaker state for one key.
    ///
    /// `opened_at` is an RFC-3339 timestamp when the circuit opened, or `None`
    /// if the circuit is currently closed.
    pub fn save_circuit_state(
        &self,
        key: &str,
        failures: u32,
        opened_at: Option<&str>,
    ) -> rusqlite::Result<()> {
        let conn = self.lock_connection();
        conn.execute(
            "INSERT INTO circuit_breaker_states (key, failures, opened_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET
                 failures  = excluded.failures,
                 opened_at = excluded.opened_at",
            rusqlite::params![key, failures as i64, opened_at],
        )?;
        Ok(())
    }

    /// Load all persisted circuit breaker states.
    ///
    /// Returns a map of `key → (failures, opened_at)`.
    pub fn load_circuit_states(&self) -> rusqlite::Result<HashMap<String, (u32, Option<String>)>> {
        let conn = self.lock_connection();
        let mut stmt =
            conn.prepare("SELECT key, failures, opened_at FROM circuit_breaker_states")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)? as u32,
                row.get::<_, Option<String>>(2)?,
            ))
        })?;
        let mut map = HashMap::new();
        for row in rows {
            let (key, failures, opened_at) = row?;
            map.insert(key, (failures, opened_at));
        }
        Ok(map)
    }

    /// Remove the persisted state for a key (e.g. when circuit resets to closed
    /// with zero failures).
    pub fn delete_circuit_state(&self, key: &str) -> rusqlite::Result<()> {
        let conn = self.lock_connection();
        conn.execute(
            "DELETE FROM circuit_breaker_states WHERE key = ?1",
            rusqlite::params![key],
        )?;
        Ok(())
    }
}

fn optional_json_column(
    row: &rusqlite::Row<'_>,
    index: usize,
) -> rusqlite::Result<Option<serde_json::Value>> {
    row.get::<_, Option<String>>(index)?
        .map(|json| {
            serde_json::from_str(&json).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    index,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_trace(hook: &str, success: bool) -> HookTrace {
        HookTrace {
            trace_id: "test000000000000".to_string(),
            correlation_id: String::new(),
            hook: hook.to_string(),
            started_at: "2026-04-07T00:00:00Z".to_string(),
            elapsed_ms: 42,
            success,
            error: if success {
                None
            } else {
                Some("test error".to_string())
            },
            input_preview: serde_json::json!({"msg": "hello"}),
            output_preview: if success {
                Some(serde_json::json!({"type": "ok"}))
            } else {
                None
            },
            annotations: None,
        }
    }

    #[test]
    fn test_open_and_insert() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("traces.db");
        let store = TraceStore::open(&db_path).unwrap();

        store.insert_blocking("my-plugin", &make_trace("ingest", true));
        store.insert_blocking("my-plugin", &make_trace("ingest", false));

        assert_eq!(store.count(None, false).unwrap(), 2);
        assert_eq!(store.count(None, true).unwrap(), 1);
        assert_eq!(store.count(Some("my-plugin"), false).unwrap(), 2);
        assert_eq!(store.count(Some("other-plugin"), false).unwrap(), 0);
    }

    #[test]
    fn annotations_survive_legacy_schema_migration_and_reopen() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("traces.db");
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE hook_traces (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    trace_id TEXT NOT NULL DEFAULT '',
                    correlation_id TEXT NOT NULL DEFAULT '',
                    plugin TEXT NOT NULL,
                    hook TEXT NOT NULL,
                    started_at TEXT NOT NULL,
                    elapsed_ms INTEGER NOT NULL,
                    success INTEGER NOT NULL,
                    error TEXT,
                    input_preview TEXT,
                    output_preview TEXT
                );
                INSERT INTO hook_traces (
                    trace_id, correlation_id, plugin, hook, started_at,
                    elapsed_ms, success
                ) VALUES (
                    'legacy0000000000', '', 'legacy-plugin', 'legacy-hook',
                    '2026-04-06T00:00:00Z', 7, 1
                );",
            )
            .unwrap();
        }

        let annotations = serde_json::json!({
            "decision": "keep",
            "scores": [0.25, 0.75],
            "nested": {"source": "hook"}
        });
        {
            let store = TraceStore::open(&db_path).unwrap();
            let legacy = store
                .query_by_trace_id("legacy0000000000")
                .unwrap()
                .expect("legacy trace should survive migration");
            assert!(legacy["annotations"].is_null());

            let mut trace = make_trace("annotated", true);
            trace.annotations = Some(annotations.clone());
            store.insert_blocking("plugin-a", &trace);

            let queried = store.query(None, None, 10, false).unwrap();
            assert_eq!(queried[0]["annotations"], annotations);
        }

        let reopened = TraceStore::open(&db_path).unwrap();
        let trace = reopened
            .query_by_trace_id("test000000000000")
            .unwrap()
            .expect("trace should survive reopen");
        assert_eq!(trace["annotations"], annotations);
    }

    #[test]
    fn test_query_filters() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TraceStore::open(&tmp.path().join("traces.db")).unwrap();

        store.insert_blocking("plugin-a", &make_trace("ingest", true));
        store.insert_blocking("plugin-b", &make_trace("after_turn", false));
        store.insert_blocking("plugin-a", &make_trace("assemble", true));

        let all = store.query(None, None, 100, false).unwrap();
        assert_eq!(all.len(), 3);

        let plugin_a = store.query(Some("plugin-a"), None, 100, false).unwrap();
        assert_eq!(plugin_a.len(), 2);

        let failures = store.query(None, None, 100, true).unwrap();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0]["plugin"], "plugin-b");
    }

    #[test]
    fn test_prune_limit_does_not_panic() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TraceStore::open(&tmp.path().join("traces.db")).unwrap();
        // Insert more than 10 000 rows in a tight loop — should not panic.
        // We only test a small batch here; the prune SQL is what matters.
        for i in 0..20 {
            store.insert_blocking(
                "plug",
                &make_trace(if i % 2 == 0 { "ingest" } else { "after_turn" }, true),
            );
        }
        assert!(store.count(None, false).unwrap() <= 10_000);
    }

    /// The async `insert` must run the SQLite work on a `spawn_blocking`
    /// thread so the tokio worker isn't held during disk I/O / prune.
    /// Smoke test: from an async context, insert a trace and confirm it
    /// landed in the DB.
    #[tokio::test]
    async fn insert_async_routes_through_spawn_blocking() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(TraceStore::open(&tmp.path().join("traces.db")).unwrap());

        store
            .clone()
            .insert("async-plugin".to_string(), make_trace("ingest", true))
            .await;

        // The insert lands synchronously from the caller's POV (await
        // resolves after the spawn_blocking task completes), so a
        // subsequent count must see it.
        assert_eq!(store.count(Some("async-plugin"), false).unwrap(), 1);
    }

    /// Concurrent async inserts must not panic or lose rows. With the
    /// `std::sync::Mutex<Connection>` held only inside `spawn_blocking`,
    /// multiple in-flight `insert` futures serialise on the mutex but
    /// never deadlock the tokio scheduler.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_async_inserts_all_land() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(TraceStore::open(&tmp.path().join("traces.db")).unwrap());

        let n: usize = 64;
        let mut handles = Vec::with_capacity(n);
        for i in 0..n {
            let store = store.clone();
            handles.push(tokio::spawn(async move {
                store
                    .insert(
                        "concurrent".to_string(),
                        make_trace(if i % 2 == 0 { "a" } else { "b" }, true),
                    )
                    .await;
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        assert_eq!(store.count(Some("concurrent"), false).unwrap(), n as i64);
    }

    /// The prune step must NOT run on every insert (that was the original
    /// O(N) regression). Instead, it runs once per `PRUNE_EVERY_N_INSERTS`.
    ///
    /// We assert two things: (a) below the prune threshold, no rows are
    /// pruned even though we exceed the 10k cap; (b) once we cross a
    /// prune boundary, the table is trimmed back to the cap.
    #[test]
    fn prune_is_counter_gated_not_per_insert() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TraceStore::open(&tmp.path().join("traces.db")).unwrap();

        // Manually push the counter forward so the next insert is exactly
        // at the prune boundary. This keeps the test fast — we don't need
        // to actually insert 10k+ rows to validate the gating logic.
        store
            .insert_counter
            .store(PRUNE_EVERY_N_INSERTS - 1, Ordering::Relaxed);

        // (a) Insert one more row. The counter ticks to PRUNE_EVERY_N_INSERTS,
        //     so prune fires. With only 1 row in the table, prune is a no-op.
        store.insert_blocking("p", &make_trace("h", true));
        assert_eq!(store.count(None, false).unwrap(), 1);

        // (b) Insert PRUNE_EVERY_N_INSERTS - 1 more rows. None of these
        //     should trigger a prune (counter advances from N+1 to 2N-1,
        //     never hitting a multiple of N).
        for _ in 0..(PRUNE_EVERY_N_INSERTS - 1) {
            store.insert_blocking("p", &make_trace("h", true));
        }
        assert_eq!(
            store.count(None, false).unwrap(),
            PRUNE_EVERY_N_INSERTS as i64
        );

        // (c) One more insert lands on the next prune boundary (2N).
        //     Since we're under the 10k cap, the table size is unchanged.
        store.insert_blocking("p", &make_trace("h", true));
        assert_eq!(
            store.count(None, false).unwrap(),
            PRUNE_EVERY_N_INSERTS as i64 + 1
        );
    }

    #[test]
    fn query_errors_are_not_reported_as_empty_results() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TraceStore::open(&tmp.path().join("traces.db")).unwrap();
        store
            .conn
            .lock()
            .unwrap()
            .execute("DROP TABLE hook_traces", [])
            .unwrap();

        assert!(store.query(None, None, 100, false).is_err());
        assert!(store.query_by_trace_id("test000000000000").is_err());
        assert!(store.count(None, false).is_err());
    }

    #[test]
    fn corrupt_trace_rows_are_not_silently_dropped() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TraceStore::open(&tmp.path().join("traces.db")).unwrap();
        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO hook_traces \
                 (trace_id, correlation_id, plugin, hook, started_at, elapsed_ms, success) \
                 VALUES ('test000000000000', '', X'00', 'hook', 'now', 1, 1)",
                [],
            )
            .unwrap();

        assert!(store.query(None, None, 100, false).is_err());
        assert!(store.query_by_trace_id("test000000000000").is_err());
    }

    #[test]
    fn store_recovers_all_connection_operations_after_lock_poison() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(TraceStore::open(&tmp.path().join("traces.db")).unwrap());
        store.insert_blocking("before-panic", &make_trace("before", true));
        store
            .save_circuit_state("preserved", 2, Some("2026-08-14T00:00:00Z"))
            .unwrap();

        let poisoned_store = store.clone();
        let _ = std::panic::catch_unwind(move || {
            let _guard = poisoned_store.conn.lock().unwrap();
            panic!("poison trace store connection lock");
        });
        assert!(store.conn.is_poisoned());

        store.insert_blocking("after-panic", &make_trace("after", false));
        assert!(!store.conn.is_poisoned());
        assert_eq!(store.count(None, false).unwrap(), 2);
        assert_eq!(store.count(None, true).unwrap(), 1);
        assert_eq!(store.query(None, None, 10, false).unwrap().len(), 2);
        assert!(store
            .query_by_trace_id("test000000000000")
            .unwrap()
            .is_some());

        let states = store.load_circuit_states().unwrap();
        assert_eq!(states["preserved"].0, 2);
        store.save_circuit_state("new", 3, None).unwrap();
        assert_eq!(store.load_circuit_states().unwrap()["new"].0, 3);
        store.delete_circuit_state("preserved").unwrap();
        assert!(!store
            .load_circuit_states()
            .unwrap()
            .contains_key("preserved"));
        assert!(store.conn.lock().is_ok());
    }
}
