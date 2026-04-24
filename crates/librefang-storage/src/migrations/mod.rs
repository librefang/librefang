//! Versioned, idempotent SurrealDB DDL migrations.
//!
//! Phase 6 of the `surrealdb-storage-swap` plan introduces this module so the
//! operational tables (audit, traces, approvals, etc.) all share one
//! migration runner — same pattern that `surreal-memory-server` already
//! proved out. Every migration is `IF NOT EXISTS`-safe so re-running on a
//! healthy database is a no-op, and a `_schema_version` table records which
//! versions have been applied.
//!
//! ## Invariant: schema ⇄ struct sync
//!
//! Every Rust struct field that is persisted via [`surrealdb::Surreal`]
//! against a `SCHEMAFULL` table here MUST have a matching `DEFINE FIELD`.
//! Schemaless tables (used for the agent registry in [`librefang-memory`])
//! are exempt. Mixed JSON-bearing fields use
//! `TYPE option<object> FLEXIBLE` (SurrealDB 3.x — FLEXIBLE follows TYPE).

#[cfg(feature = "surreal-backend")]
mod runner;

#[cfg(feature = "surreal-backend")]
pub use runner::{apply_pending, Migration, MigrationError, APPLIED_TABLE};

/// Migration set covering all operational SurrealDB stores.
///
/// New migrations append to this list with strictly increasing `version`.
/// Never re-order or rewrite past entries — the runner refuses to apply a
/// migration whose checksum no longer matches the recorded one.
#[cfg(feature = "surreal-backend")]
pub const OPERATIONAL_MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "audit_entries_v1",
        sql: include_str!("sql/001_audit_entries.surql"),
    },
    Migration {
        version: 2,
        name: "hook_traces_v1",
        sql: include_str!("sql/002_hook_traces.surql"),
    },
    Migration {
        version: 3,
        name: "circuit_breaker_states_v1",
        sql: include_str!("sql/003_circuit_breaker_states.surql"),
    },
    Migration {
        version: 4,
        name: "totp_lockout_v1",
        sql: include_str!("sql/004_totp_lockout.surql"),
    },
    Migration {
        version: 5,
        name: "sessions_v1",
        sql: include_str!("sql/005_sessions.surql"),
    },
    Migration {
        version: 6,
        name: "kv_store_v1",
        sql: include_str!("sql/006_kv_store.surql"),
    },
    Migration {
        version: 7,
        name: "task_queue_v1",
        sql: include_str!("sql/007_task_queue.surql"),
    },
    Migration {
        version: 8,
        name: "usage_events_v1",
        sql: include_str!("sql/008_usage_events.surql"),
    },
    Migration {
        version: 9,
        name: "paired_devices_v1",
        sql: include_str!("sql/009_paired_devices.surql"),
    },
    Migration {
        version: 10,
        name: "prompt_management_v1",
        sql: include_str!("sql/010_prompt_management.surql"),
    },
    Migration {
        version: 11,
        name: "knowledge_graph_v1",
        sql: include_str!("sql/011_knowledge_graph.surql"),
    },
];
