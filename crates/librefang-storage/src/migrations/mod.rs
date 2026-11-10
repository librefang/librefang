//! Versioned, idempotent SurrealDB DDL migrations.
//!
//! The operational tables (audit, traces, approvals, etc.) all share one
//! migration runner. Every migration is `IF NOT EXISTS`-safe so re-running on a
//! healthy database is a no-op, and a `_schema_version` table records which
//! versions have been applied.
//!
//! ## Invariant: schema ⇄ struct sync
//!
//! Every Rust struct field that is persisted via [`surrealdb::Surreal`]
//! against a `SCHEMAFULL` table here MUST have a matching `DEFINE FIELD`.
//! Schemaless tables
//! are exempt. Mixed JSON-bearing fields use
//! `TYPE option<object> FLEXIBLE` (SurrealDB 3.x and later — FLEXIBLE follows TYPE).

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
    Migration {
        version: 24,
        name: "idempotency_keys_v1",
        sql: include_str!("sql/024_idempotency_keys.surql"),
    },
    Migration {
        version: 25,
        name: "approvals_tool_use_id_v1",
        sql: include_str!("sql/025_approvals_tool_use_id.surql"),
    },
    Migration {
        version: 26,
        name: "approvals_deferred_payload_v1",
        sql: include_str!("sql/026_approvals_deferred_payload.surql"),
    },
    Migration {
        version: 27,
        name: "workflow_runs_v1",
        sql: include_str!("sql/027_workflow_runs.surql"),
    },
    Migration {
        version: 28,
        name: "approval_audit_second_factor_used_v1",
        sql: include_str!("sql/028_approval_audit_second_factor_used.surql"),
    },
    Migration {
        version: 29,
        name: "sessions_model_override_v1",
        sql: include_str!("sql/029_sessions_model_override.surql"),
    },
    Migration {
        version: 30,
        name: "composite_indexes_v1",
        sql: include_str!("sql/030_composite_indexes.surql"),
    },
    Migration {
        version: 32,
        name: "canonical_sessions_compacted_summary_session_id_v1",
        sql: include_str!("sql/032_canonical_sessions_compacted_summary_session_id.surql"),
    },
    Migration {
        version: 33,
        name: "knowledge_graph_peer_scope_v1",
        sql: include_str!("sql/033_knowledge_graph_peer_scope.surql"),
    },
    Migration {
        version: 34,
        name: "workflow_runs_total_steps_v1",
        sql: include_str!("sql/034_workflow_runs_total_steps.surql"),
    },
    Migration {
        version: 35,
        name: "manifest_versions_v1",
        sql: include_str!("sql/035_manifest_versions.surql"),
    },
    Migration {
        version: 36,
        name: "template_versions_v1",
        sql: include_str!("sql/036_template_versions.surql"),
    },
    Migration {
        version: 37,
        name: "goal_runs_v1",
        sql: include_str!("sql/037_goal_runs.surql"),
    },
    Migration {
        version: 38,
        name: "webauthn_credentials_v1",
        sql: include_str!("sql/038_webauthn_credentials.surql"),
    },
    Migration {
        version: 39,
        name: "channel_bindings_v1",
        sql: include_str!("sql/039_channel_bindings.surql"),
    },
    Migration {
        version: 40,
        name: "mcp_server_configs_v1",
        sql: include_str!("sql/040_mcp_server_configs.surql"),
    },
    Migration {
        version: 41,
        name: "a2a_tasks_v1",
        sql: include_str!("sql/041_a2a_tasks.surql"),
    },
    Migration {
        version: 42,
        name: "ephemeral_runs_v1",
        sql: include_str!("sql/042_ephemeral_runs.surql"),
    },
    Migration {
        version: 43,
        name: "sessions_parent_session_id_v1",
        sql: include_str!("sql/043_sessions_parent_session_id.surql"),
    },
    Migration {
        version: 44,
        name: "sessions_messages_array_v1",
        sql: include_str!("sql/044_sessions_messages_array.surql"),
    },
    Migration {
        version: 49,
        name: "json_value_columns_v1",
        sql: include_str!("sql/049_json_value_columns.surql"),
    },
];

/// Helpers shared by the tests that run against a remote SurrealDB.
#[cfg(all(test, feature = "surreal-backend"))]
pub(crate) mod test_support {
    /// The `ws://` and `http://` URLs for one server, derived from whichever scheme the operator gave.
    pub(crate) fn remote_urls(url: &str) -> Vec<String> {
        let pairs = [("ws://", "http://"), ("wss://", "https://")];
        for (ws, http) in pairs {
            if let Some(rest) = url.strip_prefix(ws).or_else(|| url.strip_prefix(http)) {
                return vec![format!("{ws}{rest}"), format!("{http}{rest}")];
            }
        }
        vec![url.to_string()]
    }
}
