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
    Migration {
        version: 12,
        name: "audit_user_channel_v1",
        sql: include_str!("sql/012_audit_user_channel.surql"),
    },
    Migration {
        version: 13,
        name: "usage_user_id_v1",
        sql: include_str!("sql/013_usage_user_id.surql"),
    },
    Migration {
        version: 14,
        name: "paired_devices_api_key_hash_v1",
        sql: include_str!("sql/014_paired_devices_api_key_hash.surql"),
    },
    Migration {
        version: 15,
        name: "totp_used_codes_v1",
        sql: include_str!("sql/015_totp_used_codes.surql"),
    },
    Migration {
        version: 16,
        name: "pending_approvals_v1",
        sql: include_str!("sql/016_pending_approvals.surql"),
    },
    Migration {
        version: 17,
        name: "oauth_used_nonces_v1",
        sql: include_str!("sql/017_oauth_used_nonces.surql"),
    },
    Migration {
        version: 18,
        name: "group_roster_v1",
        sql: include_str!("sql/018_group_roster.surql"),
    },
    Migration {
        version: 19,
        name: "retention_timestamps_v1",
        sql: include_str!("sql/019_retention_timestamps.surql"),
    },
    Migration {
        version: 20,
        name: "usage_events_session_id_v1",
        sql: include_str!("sql/020_usage_events_session_id.surql"),
    },
    Migration {
        version: 21,
        name: "totp_used_codes_bound_to_v1",
        sql: include_str!("sql/021_totp_used_codes_bound_to.surql"),
    },
    Migration {
        version: 22,
        name: "sessions_message_count_v1",
        sql: include_str!("sql/022_sessions_message_count.surql"),
    },
    Migration {
        version: 23,
        name: "sessions_search_analyzer_v1",
        sql: include_str!("sql/023_sessions_search_analyzer.surql"),
    },
];
