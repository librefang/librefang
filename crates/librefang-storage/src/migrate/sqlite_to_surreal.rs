//! Copy the legacy SQLite tables into a SurrealDB instance.
//!
//! Implementation notes
//! --------------------
//!
//! - We never delete from SurrealDB; the migrator is purely additive
//!   so a partial run can be retried without losing data already
//!   written by the daemon.
//! - Every write goes through `upsert((table, record_id))` so reruns
//!   converge on the same row instead of duplicating entries.
//! - The async SurrealDB calls are bridged onto the current tokio
//!   runtime via [`tokio::task::block_in_place`], same pattern the
//!   Surreal-backed storage backends use. Callers must therefore drive
//!   the migrator from a multi-thread tokio runtime.
//! - Field shapes mirror exactly what
//!   [`librefang-runtime::backends::surreal_audit`],
//!   [`librefang-runtime::backends::surreal_trace`], and
//!   [`librefang-kernel::backends::surreal_approval`] write at runtime,
//!   so a daemon picking up the migrated database immediately rebuilds
//!   its in-memory mirrors as if the data had always been there.

use std::collections::BTreeMap;
use std::path::Path;

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OpenFlags};
use surrealdb::{engine::any::Any, Surreal};
use tokio::runtime::Handle;
use tracing::{debug, warn};

use crate::error::{StorageError, StorageResult};
use crate::migrate::{MigrationKind, MigrationOptions, MigrationReceipt};
use crate::pool::SurrealSession;

/// Tables we know how to migrate. Order matters only for receipts.
pub(super) const TABLES: &[&str] = &[
    "audit_entries",
    "hook_traces",
    "circuit_breaker_states",
    "totp_lockout",
    "agents",
    "sessions",
    "canonical_sessions",
    "kv_store",
    "task_queue",
    "usage_events",
    "paired_devices",
    "prompt_versions",
    "prompt_experiments",
];

pub(super) fn run(
    sqlite_path: &Path,
    session: &SurrealSession,
    opts: &MigrationOptions,
) -> StorageResult<MigrationReceipt> {
    if !sqlite_path.exists() {
        return Err(StorageError::Backend(format!(
            "legacy sqlite database not found at {}",
            sqlite_path.display()
        )));
    }

    let conn = Connection::open_with_flags(
        sqlite_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(|e| StorageError::Backend(format!("open sqlite {}: {e}", sqlite_path.display())))?;

    let started_at = Utc::now();
    let db = session.client().clone();
    let mut copied = BTreeMap::new();
    let mut errors = BTreeMap::new();

    for table in TABLES {
        let result = match *table {
            "audit_entries" => copy_audit_entries(&conn, &db, opts.dry_run),
            "hook_traces" => copy_hook_traces(&conn, &db, opts.dry_run),
            "circuit_breaker_states" => copy_circuit_states(&conn, &db, opts.dry_run),
            "totp_lockout" => copy_totp_lockout(&conn, &db, opts.dry_run),
            "agents" => copy_agents(&conn, &db, opts.dry_run),
            "sessions" => copy_sessions(&conn, &db, opts.dry_run),
            "canonical_sessions" => copy_canonical_sessions(&conn, &db, opts.dry_run),
            "kv_store" => copy_kv_store(&conn, &db, opts.dry_run),
            "task_queue" => copy_task_queue(&conn, &db, opts.dry_run),
            "usage_events" => copy_usage_events(&conn, &db, opts.dry_run),
            "paired_devices" => copy_paired_devices(&conn, &db, opts.dry_run),
            "prompt_versions" => copy_prompt_versions(&conn, &db, opts.dry_run),
            "prompt_experiments" => copy_prompt_experiments(&conn, &db, opts.dry_run),
            other => {
                warn!(table = other, "no migrator registered; skipping");
                Ok(0)
            }
        };
        match result {
            Ok(n) => {
                debug!(table, rows = n, dry_run = opts.dry_run, "migrated table");
                copied.insert((*table).to_string(), n);
            }
            Err(e) => {
                warn!(table, error = %e, "migration of table failed");
                copied.insert((*table).to_string(), 0);
                errors.insert((*table).to_string(), e.to_string());
            }
        }
    }

    let finished_at = Utc::now();
    let receipt = MigrationReceipt {
        kind: MigrationKind::SqliteToSurreal,
        started_at,
        finished_at,
        source: format!("sqlite:{}", sqlite_path.display()),
        target: format!(
            "surreal:ns={}/db={}",
            session.namespace(),
            session.database()
        ),
        dry_run: opts.dry_run,
        copied,
        errors,
    };

    if !opts.dry_run {
        if let Some(dir) = opts.receipt_dir.as_ref() {
            super::write_receipt(dir, &receipt)?;
        }
    }

    Ok(receipt)
}

fn block_on<F, T>(fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    tokio::task::block_in_place(|| Handle::current().block_on(fut))
}

// ── audit_entries ─────────────────────────────────────────────────────

fn copy_audit_entries(conn: &Connection, db: &Surreal<Any>, dry_run: bool) -> StorageResult<u64> {
    let mut stmt = conn
        .prepare(
            "SELECT seq, timestamp, agent_id, action, detail, outcome, prev_hash, hash \
             FROM audit_entries ORDER BY seq ASC",
        )
        .map_err(map_sql)?;
    let rows = stmt
        .query_map([], |row| {
            Ok(serde_json::json!({
                "seq": row.get::<_, i64>(0)?,
                "timestamp": row.get::<_, String>(1)?,
                "agent_id": row.get::<_, String>(2)?,
                "action": row.get::<_, String>(3)?,
                "detail": row.get::<_, String>(4)?,
                "outcome": row.get::<_, String>(5)?,
                "prev_hash": row.get::<_, String>(6)?,
                "hash": row.get::<_, String>(7)?,
            }))
        })
        .map_err(map_sql)?;

    let mut count = 0u64;
    for row in rows {
        let row = row.map_err(map_sql)?;
        if !dry_run {
            let seq = row
                .get("seq")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| StorageError::Backend("missing seq in audit row".into()))?;
            let id = format!("seq{seq}");
            block_on(async {
                let _: Option<serde_json::Value> = db
                    .upsert(("audit_entries", id.as_str()))
                    .content(row.clone())
                    .await
                    .map_err(|e| StorageError::Backend(e.to_string()))?;
                Ok::<(), StorageError>(())
            })?;
        }
        count += 1;
    }
    Ok(count)
}

// ── hook_traces ───────────────────────────────────────────────────────

fn copy_hook_traces(conn: &Connection, db: &Surreal<Any>, dry_run: bool) -> StorageResult<u64> {
    let mut stmt = conn
        .prepare(
            "SELECT id, trace_id, correlation_id, plugin, hook, started_at, elapsed_ms, \
                    success, error, input_preview, output_preview \
             FROM hook_traces ORDER BY id ASC",
        )
        .map_err(map_sql)?;
    let rows = stmt
        .query_map([], |row| {
            let success: i64 = row.get(7)?;
            Ok(SqliteHookRow {
                id: row.get(0)?,
                trace_id: row.get(1)?,
                correlation_id: row.get(2)?,
                plugin: row.get(3)?,
                hook: row.get(4)?,
                started_at: row.get(5)?,
                elapsed_ms: row.get::<_, i64>(6)?,
                success: success != 0,
                error: row.get(8)?,
                input_preview: row.get(9)?,
                output_preview: row.get(10)?,
            })
        })
        .map_err(map_sql)?;

    let mut count = 0u64;
    for row in rows {
        let row = row.map_err(map_sql)?;
        if !dry_run {
            let started_at_ms = parse_started_at_ms(&row.started_at);
            let mut payload = serde_json::Map::new();
            payload.insert("trace_id".into(), row.trace_id.clone().into());
            payload.insert("correlation_id".into(), row.correlation_id.clone().into());
            payload.insert("plugin".into(), row.plugin.clone().into());
            payload.insert("hook".into(), row.hook.clone().into());
            payload.insert("started_at".into(), row.started_at.clone().into());
            payload.insert("started_at_ms".into(), started_at_ms.into());
            payload.insert("elapsed_ms".into(), row.elapsed_ms.into());
            payload.insert("success".into(), row.success.into());
            if let Some(err) = &row.error {
                payload.insert("error".into(), err.clone().into());
            }
            if let Some(s) = &row.input_preview {
                payload.insert("input_preview".into(), s.clone().into());
            }
            if let Some(s) = &row.output_preview {
                payload.insert("output_preview".into(), s.clone().into());
            }
            let body = serde_json::Value::Object(payload);
            let id = trace_record_id(&row.trace_id, started_at_ms, row.id);
            block_on(async {
                let _: Option<serde_json::Value> = db
                    .upsert(("hook_traces", id.as_str()))
                    .content(body)
                    .await
                    .map_err(|e| StorageError::Backend(e.to_string()))?;
                Ok::<(), StorageError>(())
            })?;
        }
        count += 1;
    }
    Ok(count)
}

struct SqliteHookRow {
    id: i64,
    trace_id: String,
    correlation_id: String,
    plugin: String,
    hook: String,
    started_at: String,
    elapsed_ms: i64,
    success: bool,
    error: Option<String>,
    input_preview: Option<String>,
    output_preview: Option<String>,
}

fn trace_record_id(trace_id: &str, started_at_ms: i64, fallback_id: i64) -> String {
    let base = sanitise_id(trace_id);
    if base.is_empty() {
        format!("legacy_{fallback_id}_{started_at_ms}")
    } else {
        format!("{base}_{started_at_ms}")
    }
}

fn parse_started_at_ms(started_at: &str) -> i64 {
    DateTime::parse_from_rfc3339(started_at)
        .map(|dt| dt.with_timezone(&Utc).timestamp_millis())
        .unwrap_or_else(|_| Utc::now().timestamp_millis())
}

// ── circuit_breaker_states ────────────────────────────────────────────

fn copy_circuit_states(conn: &Connection, db: &Surreal<Any>, dry_run: bool) -> StorageResult<u64> {
    let mut stmt = conn
        .prepare("SELECT key, failures, opened_at FROM circuit_breaker_states")
        .map_err(map_sql)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(map_sql)?;

    let mut count = 0u64;
    for row in rows {
        let (key, failures, opened_at) = row.map_err(map_sql)?;
        if !dry_run {
            let mut payload = serde_json::Map::new();
            payload.insert("key".into(), key.clone().into());
            payload.insert("failures".into(), failures.into());
            if let Some(ts) = &opened_at {
                payload.insert("opened_at".into(), ts.clone().into());
            }
            let id = sanitise_id(&key);
            let body = serde_json::Value::Object(payload);
            block_on(async {
                let _: Option<serde_json::Value> = db
                    .upsert(("circuit_breaker_states", id.as_str()))
                    .content(body)
                    .await
                    .map_err(|e| StorageError::Backend(e.to_string()))?;
                Ok::<(), StorageError>(())
            })?;
        }
        count += 1;
    }
    Ok(count)
}

// ── totp_lockout ──────────────────────────────────────────────────────

fn copy_totp_lockout(conn: &Connection, db: &Surreal<Any>, dry_run: bool) -> StorageResult<u64> {
    let mut stmt = conn
        .prepare("SELECT sender_id, failures, locked_at FROM totp_lockout")
        .map_err(map_sql)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<i64>>(2)?,
            ))
        })
        .map_err(map_sql)?;

    let mut count = 0u64;
    for row in rows {
        let (sender_id, failures, locked_at) = row.map_err(map_sql)?;
        if !dry_run {
            let mut payload = serde_json::Map::new();
            payload.insert("sender_id".into(), sender_id.clone().into());
            payload.insert("failures".into(), failures.into());
            if let Some(ts) = locked_at {
                payload.insert("locked_at".into(), ts.into());
            }
            let id = sanitise_id(&sender_id);
            let body = serde_json::Value::Object(payload);
            block_on(async {
                let _: Option<serde_json::Value> = db
                    .upsert(("totp_lockout", id.as_str()))
                    .content(body)
                    .await
                    .map_err(|e| StorageError::Backend(e.to_string()))?;
                Ok::<(), StorageError>(())
            })?;
        }
        count += 1;
    }
    Ok(count)
}

// ── agents (registry) ─────────────────────────────────────────────────

fn copy_agents(conn: &Connection, db: &Surreal<Any>, dry_run: bool) -> StorageResult<u64> {
    // Detect whether the legacy DB has the `agents` table at all so a
    // fresh sqlite database without the structured stack does not blow
    // up the migration.
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='agents'",
            [],
            |r| r.get(0),
        )
        .map_err(map_sql)?;
    if exists == 0 {
        return Ok(0);
    }

    let mut stmt = conn
        .prepare("SELECT id, name, manifest, state, created_at, updated_at FROM agents")
        .map_err(map_sql)?;
    let rows = stmt
        .query_map([], |row| {
            // `manifest` is stored as a BLOB containing JSON-serialised
            // `AgentManifest`. Decode lazily so we never hold the bytes
            // longer than necessary.
            let manifest_bytes: Vec<u8> = row.get(2)?;
            Ok(SqliteAgentRow {
                id: row.get(0)?,
                name: row.get(1)?,
                manifest_bytes,
                state: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })
        .map_err(map_sql)?;

    let mut count = 0u64;
    for row in rows {
        let row = row.map_err(map_sql)?;
        if !dry_run {
            let manifest_json =
                match serde_json::from_slice::<serde_json::Value>(&row.manifest_bytes) {
                    Ok(v) => v,
                    Err(e) => {
                        warn!(
                            agent = %row.id,
                            error = %e,
                            "skipping agent with unparseable manifest"
                        );
                        continue;
                    }
                };
            // Mirror the layout the Surreal-backed `MemoryBackend`
            // uses: a denormalised `id`/`name`/`updated_at_ms` triple
            // plus the full `entry` JSON so future schema evolutions
            // do not require a migration on every persisted struct
            // field.
            let updated_at_ms = parse_started_at_ms(&row.updated_at);
            let body = serde_json::json!({
                "id": row.id,
                "name": row.name,
                "updated_at_ms": updated_at_ms,
                "entry": {
                    "id": row.id,
                    "name": row.name,
                    "manifest": manifest_json,
                    "state": row.state,
                    "created_at": row.created_at,
                    "last_active": row.updated_at,
                },
            });
            let id = sanitise_id(&row.id);
            block_on(async {
                let _: Option<serde_json::Value> = db
                    .upsert(("agents", id.as_str()))
                    .content(body)
                    .await
                    .map_err(|e| StorageError::Backend(e.to_string()))?;
                Ok::<(), StorageError>(())
            })?;
        }
        count += 1;
    }
    Ok(count)
}

struct SqliteAgentRow {
    id: String,
    name: String,
    manifest_bytes: Vec<u8>,
    state: String,
    created_at: String,
    updated_at: String,
}

// ── sessions ──────────────────────────────────────────────────────────

fn copy_sessions(conn: &Connection, db: &Surreal<Any>, dry_run: bool) -> StorageResult<u64> {
    if !table_exists(conn, "sessions")? {
        return Ok(0);
    }
    let mut stmt = conn
        .prepare(
            "SELECT id, agent_id, messages, context_window_tokens, created_at, updated_at, label \
             FROM sessions ORDER BY created_at ASC",
        )
        .map_err(map_sql)?;
    let rows = stmt
        .query_map([], |row| {
            let messages_bytes: Vec<u8> = row.get(2)?;
            let messages_json = serde_json::from_slice::<serde_json::Value>(&messages_bytes)
                .unwrap_or(serde_json::Value::Array(vec![]));
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                messages_json,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
            ))
        })
        .map_err(map_sql)?;

    let mut count = 0u64;
    for row in rows {
        let (id, agent_id, messages, context_window_tokens, created_at, updated_at, label) =
            row.map_err(map_sql)?;
        if !dry_run {
            let body = serde_json::json!({
                "agent_id": agent_id,
                "messages": messages,
                "context_window_tokens": context_window_tokens,
                "created_at": created_at,
                "updated_at": updated_at,
                "label": label,
            });
            let rec_id = sanitise_id(&id);
            block_on(async {
                let _: Option<serde_json::Value> = db
                    .upsert(("sessions", rec_id.as_str()))
                    .content(body)
                    .await
                    .map_err(|e| StorageError::Backend(e.to_string()))?;
                Ok::<(), StorageError>(())
            })?;
        }
        count += 1;
    }
    Ok(count)
}

// ── canonical_sessions ────────────────────────────────────────────────

fn copy_canonical_sessions(
    conn: &Connection,
    db: &Surreal<Any>,
    dry_run: bool,
) -> StorageResult<u64> {
    if !table_exists(conn, "canonical_sessions")? {
        return Ok(0);
    }
    let mut stmt = conn
        .prepare(
            "SELECT agent_id, messages, compaction_cursor, compacted_summary, \
             compacted_summary_session_id, updated_at \
             FROM canonical_sessions",
        )
        .map_err(map_sql)?;
    let rows = stmt
        .query_map([], |row| {
            let messages_bytes: Vec<u8> = row.get(1)?;
            let messages_json = serde_json::from_slice::<serde_json::Value>(&messages_bytes)
                .unwrap_or(serde_json::Value::Array(vec![]));
            Ok((
                row.get::<_, String>(0)?,
                messages_json,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(map_sql)?;

    let mut count = 0u64;
    for row in rows {
        let (
            agent_id,
            messages,
            compaction_cursor,
            compacted_summary,
            compacted_summary_session_id,
            updated_at,
        ) = row.map_err(map_sql)?;
        if !dry_run {
            let body = serde_json::json!({
                "agent_id": agent_id,
                "messages": messages,
                "compaction_cursor": compaction_cursor,
                "compacted_summary": compacted_summary,
                "compacted_summary_session_id": compacted_summary_session_id,
                "updated_at": updated_at,
            });
            let rec_id = sanitise_id(&agent_id);
            block_on(async {
                let _: Option<serde_json::Value> = db
                    .upsert(("canonical_sessions", rec_id.as_str()))
                    .content(body)
                    .await
                    .map_err(|e| StorageError::Backend(e.to_string()))?;
                Ok::<(), StorageError>(())
            })?;
        }
        count += 1;
    }
    Ok(count)
}

// ── kv_store ──────────────────────────────────────────────────────────

fn copy_kv_store(conn: &Connection, db: &Surreal<Any>, dry_run: bool) -> StorageResult<u64> {
    if !table_exists(conn, "kv_store")? {
        return Ok(0);
    }
    let mut stmt = conn
        .prepare("SELECT agent_id, key, value, version, updated_at FROM kv_store")
        .map_err(map_sql)?;
    let rows = stmt
        .query_map([], |row| {
            let value_bytes: Vec<u8> = row.get(2)?;
            let value_json = serde_json::from_slice::<serde_json::Value>(&value_bytes)
                .unwrap_or(serde_json::Value::Null);
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                value_json,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(map_sql)?;

    let mut count = 0u64;
    for row in rows {
        let (agent_id, key, value, version, updated_at) = row.map_err(map_sql)?;
        if !dry_run {
            let body = serde_json::json!({
                "agent_id": agent_id,
                "key": key,
                "value": value,
                "version": version,
                "updated_at": updated_at,
            });
            // Use compound key as record ID
            let rec_id = format!("{}_{}", sanitise_id(&agent_id), sanitise_id(&key));
            block_on(async {
                let _: Option<serde_json::Value> = db
                    .upsert(("kv_store", rec_id.as_str()))
                    .content(body)
                    .await
                    .map_err(|e| StorageError::Backend(e.to_string()))?;
                Ok::<(), StorageError>(())
            })?;
        }
        count += 1;
    }
    Ok(count)
}

// ── task_queue ────────────────────────────────────────────────────────

fn copy_task_queue(conn: &Connection, db: &Surreal<Any>, dry_run: bool) -> StorageResult<u64> {
    if !table_exists(conn, "task_queue")? {
        return Ok(0);
    }
    let mut stmt = conn
        .prepare(
            "SELECT id, agent_id, task_type, payload, status, priority, \
                    scheduled_at, created_at, completed_at \
             FROM task_queue ORDER BY created_at ASC",
        )
        .map_err(map_sql)?;
    let rows = stmt
        .query_map([], |row| {
            let payload_bytes: Vec<u8> = row.get(3)?;
            let payload_json = serde_json::from_slice::<serde_json::Value>(&payload_bytes)
                .unwrap_or(serde_json::Value::Object(Default::default()));
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                payload_json,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, Option<String>>(8)?,
            ))
        })
        .map_err(map_sql)?;

    let mut count = 0u64;
    for row in rows {
        let (
            id,
            agent_id,
            task_type,
            payload,
            status,
            priority,
            scheduled_at,
            created_at,
            completed_at,
        ) = row.map_err(map_sql)?;
        if !dry_run {
            let body = serde_json::json!({
                "agent_id": agent_id,
                "task_type": task_type,
                "payload": payload,
                "status": status,
                "priority": priority,
                "scheduled_at": scheduled_at,
                "created_at": created_at,
                "completed_at": completed_at,
            });
            let rec_id = sanitise_id(&id);
            block_on(async {
                let _: Option<serde_json::Value> = db
                    .upsert(("task_queue", rec_id.as_str()))
                    .content(body)
                    .await
                    .map_err(|e| StorageError::Backend(e.to_string()))?;
                Ok::<(), StorageError>(())
            })?;
        }
        count += 1;
    }
    Ok(count)
}

// ── usage_events ──────────────────────────────────────────────────────

fn copy_usage_events(conn: &Connection, db: &Surreal<Any>, dry_run: bool) -> StorageResult<u64> {
    if !table_exists(conn, "usage_events")? {
        return Ok(0);
    }
    // latency_ms and provider may not exist in older DBs — use optional cols
    let has_latency = column_exists_storage(conn, "usage_events", "latency_ms");
    let has_provider = column_exists_storage(conn, "usage_events", "provider");
    let select = format!(
        "SELECT id, agent_id, timestamp, model, input_tokens, output_tokens, cost_usd, tool_calls{}{} \
         FROM usage_events ORDER BY timestamp ASC",
        if has_provider { ", provider" } else { "" },
        if has_latency { ", latency_ms" } else { "" },
    );

    let mut stmt = conn.prepare(&select).map_err(map_sql)?;
    let provider_idx: Option<usize> = if has_provider { Some(8) } else { None };
    let latency_idx: Option<usize> = if has_latency {
        Some(if has_provider { 9 } else { 8 })
    } else {
        None
    };

    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, f64>(6)?,
                row.get::<_, i64>(7)?,
                provider_idx
                    .and_then(|i| row.get::<_, Option<String>>(i).ok())
                    .flatten(),
                latency_idx
                    .and_then(|i| row.get::<_, Option<i64>>(i).ok())
                    .flatten(),
            ))
        })
        .map_err(map_sql)?;

    let mut count = 0u64;
    for row in rows {
        let (
            id,
            agent_id,
            timestamp,
            model,
            input_tokens,
            output_tokens,
            cost_usd,
            tool_calls,
            provider,
            latency_ms,
        ) = row.map_err(map_sql)?;
        if !dry_run {
            let body = serde_json::json!({
                "agent_id": agent_id,
                "timestamp": timestamp,
                "provider": provider.unwrap_or_default(),
                "model": model,
                "input_tokens": input_tokens,
                "output_tokens": output_tokens,
                "cost_usd": cost_usd,
                "tool_calls": tool_calls,
                "latency_ms": latency_ms.unwrap_or(0),
            });
            let rec_id = sanitise_id(&id);
            block_on(async {
                let _: Option<serde_json::Value> = db
                    .upsert(("usage_events", rec_id.as_str()))
                    .content(body)
                    .await
                    .map_err(|e| StorageError::Backend(e.to_string()))?;
                Ok::<(), StorageError>(())
            })?;
        }
        count += 1;
    }
    Ok(count)
}

// ── paired_devices ────────────────────────────────────────────────────

fn copy_paired_devices(conn: &Connection, db: &Surreal<Any>, dry_run: bool) -> StorageResult<u64> {
    if !table_exists(conn, "paired_devices")? {
        return Ok(0);
    }
    let mut stmt = conn
        .prepare(
            "SELECT device_id, display_name, platform, paired_at, last_seen, push_token \
             FROM paired_devices",
        )
        .map_err(map_sql)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })
        .map_err(map_sql)?;

    let mut count = 0u64;
    for row in rows {
        let (device_id, display_name, platform, paired_at, last_seen, push_token) =
            row.map_err(map_sql)?;
        if !dry_run {
            let body = serde_json::json!({
                "device_id": device_id,
                "display_name": display_name,
                "platform": platform,
                "paired_at": paired_at,
                "last_seen": last_seen,
                "push_token": push_token,
            });
            let rec_id = sanitise_id(&device_id);
            block_on(async {
                let _: Option<serde_json::Value> = db
                    .upsert(("paired_devices", rec_id.as_str()))
                    .content(body)
                    .await
                    .map_err(|e| StorageError::Backend(e.to_string()))?;
                Ok::<(), StorageError>(())
            })?;
        }
        count += 1;
    }
    Ok(count)
}

// ── prompt_versions ───────────────────────────────────────────────────

fn copy_prompt_versions(conn: &Connection, db: &Surreal<Any>, dry_run: bool) -> StorageResult<u64> {
    if !table_exists(conn, "prompt_versions")? {
        return Ok(0);
    }
    let mut stmt = conn
        .prepare(
            "SELECT id, agent_id, version, content_hash, system_prompt, tools, variables, \
                    created_at, created_by, is_active, description \
             FROM prompt_versions ORDER BY version ASC",
        )
        .map_err(map_sql)?;
    let rows = stmt
        .query_map([], |row| {
            let tools_str: String = row.get(5)?;
            let variables_str: String = row.get(6)?;
            let tools_json = serde_json::from_str::<serde_json::Value>(&tools_str)
                .unwrap_or(serde_json::Value::Array(vec![]));
            let variables_json = serde_json::from_str::<serde_json::Value>(&variables_str)
                .unwrap_or(serde_json::Value::Array(vec![]));
            let is_active: i64 = row.get(9)?;
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                tools_json,
                variables_json,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                is_active != 0,
                row.get::<_, Option<String>>(10)?,
            ))
        })
        .map_err(map_sql)?;

    let mut count = 0u64;
    for row in rows {
        let (
            id,
            agent_id,
            version,
            content_hash,
            system_prompt,
            tools,
            variables,
            created_at,
            created_by,
            is_active,
            description,
        ) = row.map_err(map_sql)?;
        if !dry_run {
            let body = serde_json::json!({
                "agent_id": agent_id,
                "version": version,
                "content_hash": content_hash,
                "system_prompt": system_prompt,
                "tools": tools,
                "variables": variables,
                "created_at": created_at,
                "created_by": created_by,
                "is_active": is_active,
                "description": description,
            });
            let rec_id = sanitise_id(&id);
            block_on(async {
                let _: Option<serde_json::Value> = db
                    .upsert(("prompt_versions", rec_id.as_str()))
                    .content(body)
                    .await
                    .map_err(|e| StorageError::Backend(e.to_string()))?;
                Ok::<(), StorageError>(())
            })?;
        }
        count += 1;
    }
    Ok(count)
}

// ── prompt_experiments ────────────────────────────────────────────────

fn copy_prompt_experiments(
    conn: &Connection,
    db: &Surreal<Any>,
    dry_run: bool,
) -> StorageResult<u64> {
    if !table_exists(conn, "prompt_experiments")? {
        return Ok(0);
    }
    let mut stmt = conn
        .prepare(
            "SELECT id, name, agent_id, status, traffic_split, success_criteria, \
                    started_at, ended_at, created_at \
             FROM prompt_experiments ORDER BY created_at ASC",
        )
        .map_err(map_sql)?;
    let rows = stmt
        .query_map([], |row| {
            let traffic_str: String = row.get(4)?;
            let criteria_str: String = row.get(5)?;
            let traffic_json = serde_json::from_str::<serde_json::Value>(&traffic_str)
                .unwrap_or(serde_json::Value::Object(Default::default()));
            let criteria_json = serde_json::from_str::<serde_json::Value>(&criteria_str)
                .unwrap_or(serde_json::Value::Object(Default::default()));
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                traffic_json,
                criteria_json,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, String>(8)?,
            ))
        })
        .map_err(map_sql)?;

    let mut count = 0u64;
    for row in rows {
        let (
            id,
            name,
            agent_id,
            status,
            traffic_split,
            success_criteria,
            started_at,
            ended_at,
            created_at,
        ) = row.map_err(map_sql)?;
        if !dry_run {
            let body = serde_json::json!({
                "name": name,
                "agent_id": agent_id,
                "status": status,
                "traffic_split": traffic_split,
                "success_criteria": success_criteria,
                "started_at": started_at,
                "ended_at": ended_at,
                "created_at": created_at,
            });
            let rec_id = sanitise_id(&id);
            block_on(async {
                let _: Option<serde_json::Value> = db
                    .upsert(("prompt_experiments", rec_id.as_str()))
                    .content(body)
                    .await
                    .map_err(|e| StorageError::Backend(e.to_string()))?;
                Ok::<(), StorageError>(())
            })?;
        }
        count += 1;
    }
    Ok(count)
}

// ── shared helpers ────────────────────────────────────────────────────

fn map_sql(e: rusqlite::Error) -> StorageError {
    StorageError::Backend(format!("sqlite: {e}"))
}

/// Returns `true` if `table` exists in the SQLite database.
fn table_exists(conn: &Connection, table: &str) -> StorageResult<bool> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
            rusqlite::params![table],
            |r| r.get(0),
        )
        .map_err(map_sql)?;
    Ok(count > 0)
}

/// Returns `true` if `column` exists in `table` (storage-error variant).
fn column_exists_storage(conn: &Connection, table: &str, column: &str) -> bool {
    conn.prepare(&format!("SELECT {column} FROM {table} LIMIT 0"))
        .is_ok()
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{StorageBackendKind, StorageConfig};
    use crate::pool::SurrealConnectionPool;
    use rusqlite::params;
    use tempfile::tempdir;

    fn seed_sqlite(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE audit_entries (
                seq INTEGER PRIMARY KEY,
                timestamp TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                action TEXT NOT NULL,
                detail TEXT NOT NULL,
                outcome TEXT NOT NULL,
                prev_hash TEXT NOT NULL,
                hash TEXT NOT NULL
            );
            CREATE TABLE hook_traces (
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
            CREATE TABLE circuit_breaker_states (
                key TEXT PRIMARY KEY,
                failures INTEGER NOT NULL DEFAULT 0,
                opened_at TEXT
            );
            CREATE TABLE totp_lockout (
                sender_id TEXT PRIMARY KEY,
                failures INTEGER NOT NULL,
                locked_at INTEGER
            );",
        )
        .unwrap();

        conn.execute(
            "INSERT INTO audit_entries (seq, timestamp, agent_id, action, detail, outcome, prev_hash, hash) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                0i64,
                "2026-04-21T00:00:00Z",
                "agent-1",
                "AgentSpawn",
                "spawn",
                "ok",
                "0".repeat(64),
                "a".repeat(64),
            ],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO hook_traces (trace_id, correlation_id, plugin, hook, started_at, elapsed_ms, success) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params!["trace-1", "corr-1", "plugin", "ingest", "2026-04-21T00:00:01Z", 12i64, 1i64],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO circuit_breaker_states (key, failures, opened_at) VALUES (?1, ?2, ?3)",
            params!["plugin/ingest", 3i64, Some("2026-04-21T00:00:02Z")],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO totp_lockout (sender_id, failures, locked_at) VALUES (?1, ?2, ?3)",
            params!["slack:U12345", 5i64, Some(1_700_000_000i64)],
        )
        .unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dry_run_counts_rows_without_writing() {
        let dir = tempdir().unwrap();
        let sqlite_path = dir.path().join("librefang.db");
        seed_sqlite(&sqlite_path);

        let surreal_dir = dir.path().join("surreal");
        let pool = SurrealConnectionPool::new();
        let cfg = StorageConfig {
            backend: StorageBackendKind::embedded(surreal_dir),
            namespace: "librefang".into(),
            database: "main".into(),
            legacy_sqlite_path: None,
        };
        let session = pool.open(&cfg).await.expect("open surreal");
        // Run schema migrations so the post-flight assertion below can
        // query a real table (otherwise SurrealDB 3.0 returns
        // `NotFound` rather than an empty result set).
        crate::migrations::apply_pending(
            session.client(),
            crate::migrations::OPERATIONAL_MIGRATIONS,
        )
        .await
        .expect("migrations");

        let opts = MigrationOptions {
            dry_run: true,
            receipt_dir: None,
        };
        let receipt = run(&sqlite_path, &session, &opts).expect("dry run");
        assert!(receipt.dry_run);
        assert_eq!(receipt.copied.get("audit_entries"), Some(&1));
        assert_eq!(receipt.copied.get("hook_traces"), Some(&1));
        assert_eq!(receipt.copied.get("circuit_breaker_states"), Some(&1));
        assert_eq!(receipt.copied.get("totp_lockout"), Some(&1));

        // Surreal tables stay empty under dry-run.
        let rows: Vec<serde_json::Value> = session
            .client()
            .query("SELECT seq FROM audit_entries")
            .await
            .unwrap()
            .take(0)
            .unwrap();
        assert!(rows.is_empty(), "dry-run wrote rows: {rows:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn live_run_copies_rows_and_is_idempotent() {
        let dir = tempdir().unwrap();
        let sqlite_path = dir.path().join("librefang.db");
        seed_sqlite(&sqlite_path);

        let surreal_dir = dir.path().join("surreal");
        let pool = SurrealConnectionPool::new();
        let cfg = StorageConfig {
            backend: StorageBackendKind::embedded(surreal_dir),
            namespace: "librefang".into(),
            database: "main".into(),
            legacy_sqlite_path: None,
        };
        let session = pool.open(&cfg).await.expect("open surreal");
        crate::migrations::apply_pending(
            session.client(),
            crate::migrations::OPERATIONAL_MIGRATIONS,
        )
        .await
        .expect("migrations");

        let receipts_dir = dir.path().join("migrations");
        let opts = MigrationOptions {
            dry_run: false,
            receipt_dir: Some(receipts_dir.clone()),
        };
        let receipt = run(&sqlite_path, &session, &opts).expect("first run");
        assert!(!receipt.dry_run);
        assert!(receipt.is_clean(), "errors: {:?}", receipt.errors);

        let count_audit: Vec<serde_json::Value> = session
            .client()
            .query("SELECT seq FROM audit_entries")
            .await
            .unwrap()
            .take(0)
            .unwrap();
        assert_eq!(count_audit.len(), 1);

        // Re-running must converge, not duplicate.
        let second = run(&sqlite_path, &session, &opts).expect("second run");
        assert!(second.is_clean());
        let still_one: Vec<serde_json::Value> = session
            .client()
            .query("SELECT seq FROM audit_entries")
            .await
            .unwrap()
            .take(0)
            .unwrap();
        assert_eq!(still_one.len(), 1);

        // Receipt files exist (one per run).
        let entries: Vec<_> = std::fs::read_dir(&receipts_dir)
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert!(entries.len() >= 2, "expected receipts, got {entries:?}");
    }
}
