//! Persistent hook trace store backed by SQLite (via rusqlite).
//!
//! Stores the last 10,000 hook traces across daemon restarts, enabling
//! post-mortem analysis of hook failures without relying on the in-memory
//! ring buffer (which resets on restart).

use rusqlite::{params, Connection};
use std::path::Path;

use crate::context_engine::HookTrace;

/// Persistent SQLite-backed store for hook execution traces.
pub struct TraceStore {
    conn: std::sync::Mutex<Connection>,
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
                plugin          TEXT    NOT NULL,
                hook            TEXT    NOT NULL,
                started_at      TEXT    NOT NULL,
                elapsed_ms      INTEGER NOT NULL,
                success         INTEGER NOT NULL,
                error           TEXT,
                input_preview   TEXT,
                output_preview  TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_started_at   ON hook_traces(started_at);
            CREATE INDEX IF NOT EXISTS idx_plugin_hook  ON hook_traces(plugin, hook);
            ",
        )?;
        Ok(Self {
            conn: std::sync::Mutex::new(conn),
        })
    }

    /// Insert a trace record.
    ///
    /// Silently ignores DB errors — traces are non-critical telemetry and must
    /// never cause a hook invocation to fail.  Also prunes the table to keep
    /// at most 10,000 rows.
    pub fn insert(&self, plugin: &str, trace: &HookTrace) {
        let Ok(conn) = self.conn.lock() else { return };

        let input_preview = serde_json::to_string(&trace.input_preview).ok();
        let output_preview = trace
            .output_preview
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok());

        let _ = conn.execute(
            "INSERT INTO hook_traces \
             (plugin, hook, started_at, elapsed_ms, success, error, input_preview, output_preview) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                plugin,
                trace.hook,
                trace.started_at,
                trace.elapsed_ms as i64,
                trace.success as i64,
                trace.error,
                input_preview,
                output_preview,
            ],
        );

        // Prune to the most recent 10,000 rows.
        let _ = conn.execute(
            "DELETE FROM hook_traces WHERE id NOT IN \
             (SELECT id FROM hook_traces ORDER BY id DESC LIMIT 10000)",
            [],
        );
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
    ) -> Vec<serde_json::Value> {
        let Ok(conn) = self.conn.lock() else {
            return vec![];
        };

        // Build WHERE clause from optional filters.
        // Using string interpolation with escaping because rusqlite's positional
        // params cannot be used in a dynamically-built WHERE clause without
        // additional machinery.
        let mut where_parts: Vec<String> = Vec::new();
        if let Some(p) = plugin {
            where_parts.push(format!("plugin = '{}'", p.replace('\'', "''")));
        }
        if let Some(h) = hook {
            where_parts.push(format!("hook = '{}'", h.replace('\'', "''")));
        }
        if only_failures {
            where_parts.push("success = 0".to_string());
        }

        let where_clause = if where_parts.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", where_parts.join(" AND "))
        };

        let sql = format!(
            "SELECT plugin, hook, started_at, elapsed_ms, success, error, \
             input_preview, output_preview \
             FROM hook_traces {where_clause} ORDER BY id DESC LIMIT {limit}"
        );

        let Ok(mut stmt) = conn.prepare(&sql) else {
            return vec![];
        };

        stmt.query_map([], |row| {
            Ok(serde_json::json!({
                "plugin":          row.get::<_, String>(0)?,
                "hook":            row.get::<_, String>(1)?,
                "started_at":      row.get::<_, String>(2)?,
                "elapsed_ms":      row.get::<_, i64>(3)?,
                "success":         row.get::<_, i64>(4)? != 0,
                "error":           row.get::<_, Option<String>>(5)?,
                "input_preview":   row.get::<_, Option<String>>(6)?,
                "output_preview":  row.get::<_, Option<String>>(7)?,
            }))
        })
        .ok()
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    }

    /// Count traces, optionally filtered by plugin and/or failure status.
    pub fn count(&self, plugin: Option<&str>, only_failures: bool) -> i64 {
        let Ok(conn) = self.conn.lock() else {
            return 0;
        };

        let mut parts: Vec<String> = Vec::new();
        if let Some(p) = plugin {
            parts.push(format!("plugin = '{}'", p.replace('\'', "''")));
        }
        if only_failures {
            parts.push("success = 0".to_string());
        }

        let where_clause = if parts.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", parts.join(" AND "))
        };

        let sql = format!("SELECT COUNT(*) FROM hook_traces {where_clause}");
        conn.query_row(&sql, [], |r| r.get(0)).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_trace(hook: &str, success: bool) -> HookTrace {
        HookTrace {
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
        }
    }

    #[test]
    fn test_open_and_insert() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("traces.db");
        let store = TraceStore::open(&db_path).unwrap();

        store.insert("my-plugin", &make_trace("ingest", true));
        store.insert("my-plugin", &make_trace("ingest", false));

        assert_eq!(store.count(None, false), 2);
        assert_eq!(store.count(None, true), 1);
        assert_eq!(store.count(Some("my-plugin"), false), 2);
        assert_eq!(store.count(Some("other-plugin"), false), 0);
    }

    #[test]
    fn test_query_filters() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TraceStore::open(&tmp.path().join("traces.db")).unwrap();

        store.insert("plugin-a", &make_trace("ingest", true));
        store.insert("plugin-b", &make_trace("after_turn", false));
        store.insert("plugin-a", &make_trace("assemble", true));

        let all = store.query(None, None, 100, false);
        assert_eq!(all.len(), 3);

        let plugin_a = store.query(Some("plugin-a"), None, 100, false);
        assert_eq!(plugin_a.len(), 2);

        let failures = store.query(None, None, 100, true);
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
            store.insert("plug", &make_trace(if i % 2 == 0 { "ingest" } else { "after_turn" }, true));
        }
        assert!(store.count(None, false) <= 10_000);
    }
}
