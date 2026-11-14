//! Migrations v50 to v52 (`050_upstream_sqlite_v62_v64_parity`, `051_sessions_reasoning_effort`, `052_upstream_sqlite_column_parity`) against a real SurrealDB.
//!
//! The database is first brought to v49, the last migration before this set, and seeded with rows in the shape a pre-v50 writer produced.
//! The test then proves the upgrade applies cleanly on that populated database, is a no-op when re-run (through the runner and by replaying each script), keeps the seeded rows readable and writable, and that every field and index the three migrations add is enforced.
//! The embedded variant runs on `kv-surrealkv` in a temp dir; the remote variant runs the same body over `ws://` and `http://` when `LIBREFANG_TEST_SURREAL_URL` is set, and prints an explicit SKIP line when it is not.

#![cfg(feature = "surreal-backend")]

use librefang_storage::config::{RemoteSurrealConfig, StorageBackendKind, StorageConfig};
use librefang_storage::migrations::{apply_pending, OPERATIONAL_MIGRATIONS};
use librefang_storage::{SurrealConnectionPool, SurrealSession};
use serde_json::{json, Value};

/// The first migration this test upgrades through; everything below it is the pre-existing schema.
const FIRST_NEW: u32 = 50;
const NEW_VERSIONS: [u32; 3] = [50, 51, 52];

async fn rows(session: &SurrealSession, sql: &str) -> Vec<Value> {
    session
        .client()
        .query(sql)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
        .take(0)
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
}

/// Run `sql` and report whether every statement in it succeeded.
async fn accepts(session: &SurrealSession, sql: &str) -> bool {
    match session.client().query(sql).await {
        Ok(res) => res.check().is_ok(),
        Err(_) => false,
    }
}

async fn assert_index(session: &SurrealSession, table: &str, index: &str) {
    let info: Option<Value> = session
        .client()
        .query(format!("INFO FOR TABLE {table}"))
        .await
        .expect("info")
        .take(0)
        .expect("info row");
    let info = info.expect("table info");
    assert!(
        info["indexes"].get(index).is_some(),
        "{table} is missing {index}: {info}"
    );
}

/// Bring the database to v49 and write rows the way a pre-v50 writer did.
async fn seed_pre_v50(session: &SurrealSession) {
    let pre: Vec<_> = OPERATIONAL_MIGRATIONS
        .iter()
        .filter(|m| m.version < FIRST_NEW)
        .cloned()
        .collect();
    apply_pending(session.client(), &pre)
        .await
        .expect("migrate to v49");

    // Populated tables are what SurrealDB 4 guards: a new non-optional field or a changed ASSERT must say what existing rows hold.
    for seed in [
        "CREATE prompt_experiments:legacy CONTENT { name: 'exp', agent_id: 'a', status: 'running', \
         traffic_split: {}, success_criteria: {}, created_at: '2026-01-01' }",
        "CREATE task_queue:legacy CONTENT { agent_id: 'a', task_type: 'x', status: 'pending', priority: 0, \
         created_at: '2026-01-01' }",
        "CREATE ephemeral_runs:legacy CONTENT { parent_agent_id: 'p', label: 'l', worker_name: 'w', \
         status: 'completed', started_at: '2026-01-01', finished_at: '2026-01-01', created_at: '2026-01-01' }",
        "CREATE sessions:legacy CONTENT { agent_id: 'a', context_window_tokens: 0, created_at: 't', updated_at: 't' }",
        "CREATE group_roster:legacy CONTENT { channel_type: 'tg', chat_id: 'old', user_id: 'u', display_name: 'U' }",
        "CREATE workflow_runs:legacy CONTENT { workflow_id: 'w', state: 'failed', started_at: '2026-01-01', \
         created_at: '2026-01-01' }",
        "CREATE workflow_runs:done CONTENT { workflow_id: 'w', state: 'completed', started_at: '2026-01-01', \
         created_at: '2026-01-01' }",
        "CREATE workflow_runs:live CONTENT { workflow_id: 'w', state: 'running', started_at: '2026-01-01', \
         created_at: '2026-01-01' }",
    ] {
        assert!(accepts(session, seed).await, "v49 seed: {seed}");
    }
}

/// Apply 050 to 052 once, then prove a second run and a raw replay of each script change nothing.
async fn upgrade_and_rerun(session: &SurrealSession) {
    let db = session.client();
    let applied = apply_pending(db, OPERATIONAL_MIGRATIONS)
        .await
        .expect("apply v50..v52");
    assert_eq!(applied, NEW_VERSIONS.to_vec());
    let again = apply_pending(db, OPERATIONAL_MIGRATIONS)
        .await
        .expect("re-run");
    assert!(again.is_empty(), "re-run applied {again:?}");
    // Replaying the scripts is also harmless: every DEFINE is IF NOT EXISTS / OVERWRITE and the UPDATE converges.
    for version in NEW_VERSIONS {
        let m = OPERATIONAL_MIGRATIONS
            .iter()
            .find(|m| m.version == version)
            .unwrap_or_else(|| panic!("v{version} registered"));
        db.query(m.sql)
            .await
            .unwrap_or_else(|e| panic!("replay v{version}: {e}"))
            .check()
            .unwrap_or_else(|e| panic!("replayed v{version} statements: {e}"));
    }
}

/// Rows written before v50 read back with SQLite's backfill values and still accept writes.
async fn check_seeded_rows(session: &SurrealSession) {
    let task = rows(session, "SELECT * FROM task_queue:legacy").await;
    assert_eq!(task[0]["retry_count"], 0, "{task:?}");
    assert!(task[0].get("timeout_secs").is_none(), "{task:?}");
    let run = rows(session, "SELECT * FROM ephemeral_runs:legacy").await;
    assert!(
        matches!(run[0].get("truncated"), None | Some(Value::Bool(false))),
        "a pre-v50 run reads as not truncated: {run:?}"
    );
    let roster = rows(session, "SELECT * FROM group_roster:legacy").await;
    assert_eq!(roster[0]["source"], "observed", "{roster:?}");
    // The `state` migration is the identity: every stored state passes the widened ASSERT unchanged, and the restored index still finds them.
    for (record, state) in [
        ("legacy", "failed"),
        ("done", "completed"),
        ("live", "running"),
    ] {
        let run = rows(
            session,
            &format!("SELECT state FROM workflow_runs:{record}"),
        )
        .await;
        assert_eq!(run[0]["state"], state, "{record}: {run:?}");
        let by_state = rows(
            session,
            &format!("SELECT VALUE id FROM workflow_runs WHERE state = '{state}'"),
        )
        .await;
        assert_eq!(by_state.len(), 1, "{state}: {by_state:?}");
    }
    for update in [
        "UPDATE task_queue:legacy SET status = 'in_progress', claimed_at = '2026-01-02T00:00:00Z', timeout_secs = 30",
        "UPDATE ephemeral_runs:legacy SET label = 'm'",
        "UPDATE sessions:legacy SET reasoning_effort = 'high', peer_id = 'tg:chat'",
        "UPDATE group_roster:legacy SET display_name = 'V'",
        "UPDATE workflow_runs:legacy SET state = 'cancelled'",
    ] {
        assert!(accepts(session, update).await, "post-v52 write: {update}");
    }
    let session_row = rows(session, "SELECT * FROM sessions:legacy").await;
    assert_eq!(
        session_row[0]["reasoning_effort"], "high",
        "{session_row:?}"
    );
    assert_eq!(session_row[0]["peer_id"], "tg:chat", "{session_row:?}");

    // The `{}` placeholder was cleared, so a later update re-validates against the new type.
    let legacy = rows(session, "SELECT * FROM prompt_experiments:legacy").await;
    assert!(legacy[0].get("traffic_split").is_none(), "{legacy:?}");
    assert!(legacy[0].get("success_criteria").is_none(), "{legacy:?}");
    for (update, ok) in [
        (
            "UPDATE prompt_experiments:legacy MERGE { status: 'paused' }",
            true,
        ),
        (
            "UPDATE prompt_experiments:legacy SET traffic_split = [70, 30]",
            true,
        ),
        (
            "UPDATE prompt_experiments:legacy SET traffic_split = { a: 1 }",
            false,
        ),
    ] {
        assert_eq!(accepts(session, update).await, ok, "{update}");
    }
}

/// New rows carry every field the three migrations add, with SQLite's defaults where SQLite has one.
async fn check_new_rows(session: &SurrealSession) {
    // ephemeral_runs.truncated (v50) defaults to false and keeps an explicit true.
    for (id, extra, expected) in [("e1", "", false), ("e2", ", truncated: true", true)] {
        let create = format!(
            "CREATE ephemeral_runs:{id} CONTENT {{ parent_agent_id: 'p', label: 'l', worker_name: 'w', \
             status: 'completed', started_at: 's', finished_at: 'f', created_at: 'c'{extra} }}"
        );
        assert!(accepts(session, &create).await, "{create}");
        let row = rows(
            session,
            &format!("SELECT truncated FROM ephemeral_runs:{id}"),
        )
        .await;
        assert_eq!(row[0]["truncated"], expected, "{id}: {row:?}");
    }

    // workflow_runs.state accepts 'cancelled' and still rejects unknown states; owner_agent_id is kept.
    let run = "CREATE workflow_runs CONTENT { id: 'r1', workflow_id: 'w', state: 'cancelled', \
               started_at: '2026-01-01', created_at: '2026-01-01', owner_agent_id: 'owner-1' }";
    assert!(accepts(session, run).await, "cancelled is a valid state");
    let owner = rows(session, "SELECT owner_agent_id FROM workflow_runs:r1").await;
    assert_eq!(owner[0]["owner_agent_id"], "owner-1");
    assert!(
        !accepts(
            session,
            "CREATE workflow_runs CONTENT { id: 'r2', workflow_id: 'w', state: 'bogus', \
             started_at: '2026-01-01', created_at: '2026-01-01' }"
        )
        .await,
        "an unknown state is rejected"
    );

    // group_roster.source defaults like SQLite's DEFAULT 'observed'.
    assert!(
        accepts(
            session,
            "CREATE group_roster:new CONTENT { channel_type: 'tg', chat_id: 'c', user_id: 'u', display_name: 'U' }"
        )
        .await
    );
    let roster = rows(session, "SELECT source FROM group_roster:new").await;
    assert_eq!(roster[0]["source"], "observed");

    // task_queue carries the v2 / v20 / v21 / v62 columns; retry_count defaults to 0.
    assert!(
        accepts(
            session,
            "CREATE task_queue:t1 CONTENT { agent_id: 'a', task_type: 'x', status: 'in_progress', priority: 0, \
             created_at: '2026-01-01', title: 'T', description: 'D', created_by: 'user', result: 'R', \
             claimed_at: '2026-01-01T00:00:00Z', timeout_secs: 0 }"
        )
        .await
    );
    let task = rows(session, "SELECT * FROM task_queue:t1").await;
    assert_eq!(task[0]["retry_count"], 0);
    assert_eq!(task[0]["timeout_secs"], 0);
    assert_eq!(task[0]["claimed_at"], "2026-01-01T00:00:00Z");
    assert_eq!(task[0]["result"], "R");

    // The remaining new fields are kept by their SCHEMAFULL tables instead of being dropped.
    for (create, select, field, expected) in [
        (
            "CREATE usage_events:u1 CONTENT { agent_id: 'w', timestamp: 't', provider: 'p', model: 'm', input_tokens: 0, \
             output_tokens: 0, cost_usd: 0.0, tool_calls: 0, latency_ms: 0, billed_agent_id: 'parent' }",
            "SELECT * FROM usage_events:u1",
            "billed_agent_id",
            json!("parent"),
        ),
        (
            "CREATE sessions:s1 CONTENT { agent_id: 'a', context_window_tokens: 0, created_at: 't', updated_at: 't', \
             peer_id: 'tg:chat', reasoning_effort: 'low' }",
            "SELECT * FROM sessions:s1",
            "reasoning_effort",
            json!("low"),
        ),
        (
            "CREATE hook_traces:h1 CONTENT { trace_id: 't', correlation_id: 'c', plugin: 'p', hook: 'h', started_at: 's', \
             started_at_ms: 0, elapsed_ms: 1, success: true, annotations: '{\"k\":1}' }",
            "SELECT * FROM hook_traces:h1",
            "annotations",
            json!("{\"k\":1}"),
        ),
    ] {
        assert!(accepts(session, create).await, "{create}");
        let row = rows(session, select).await;
        assert_eq!(row[0][field], expected, "{field}: {row:?}");
    }
}

async fn exercise(session: &SurrealSession) {
    seed_pre_v50(session).await;
    upgrade_and_rerun(session).await;
    check_seeded_rows(session).await;
    check_new_rows(session).await;

    // The indexes the SQLite read paths rely on exist.
    for (table, index) in [
        ("sessions", "idx_sessions_created"),
        ("sessions", "idx_sessions_agent_created"),
        ("sessions", "idx_sessions_peer"),
        ("task_queue", "idx_task_status_claimed_at"),
        ("usage_events", "usage_provider_time_idx"),
        ("usage_events", "usage_channel_time_idx"),
        ("audit_entries", "idx_audit_timestamp"),
        ("audit_entries", "idx_audit_action"),
        ("audit_entries", "idx_audit_channel"),
        ("workflow_runs", "idx_workflow_runs_state"),
    ] {
        assert_index(session, table, index).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn migrations_50_to_52_upgrade_a_populated_v49_database_embedded() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cfg = StorageConfig {
        backend: StorageBackendKind::embedded(dir.path().join("v52.surreal")),
        namespace: "librefang".into(),
        database: "main".into(),
        legacy_sqlite_path: None,
    };
    let session = SurrealConnectionPool::new()
        .open(&cfg)
        .await
        .expect("open embedded");
    exercise(&session).await;
}

fn remote_urls(url: &str) -> Vec<String> {
    for (ws, http) in [("ws://", "http://"), ("wss://", "https://")] {
        if let Some(rest) = url.strip_prefix(ws).or_else(|| url.strip_prefix(http)) {
            return vec![format!("{ws}{rest}"), format!("{http}{rest}")];
        }
    }
    vec![url.to_string()]
}

#[tokio::test(flavor = "multi_thread")]
async fn migrations_50_to_52_upgrade_a_populated_v49_database_remote() {
    let Ok(url) = std::env::var("LIBREFANG_TEST_SURREAL_URL") else {
        eprintln!("SKIP remote surreal: LIBREFANG_TEST_SURREAL_URL unset");
        return;
    };
    let username = std::env::var("LIBREFANG_TEST_SURREAL_USER").unwrap_or_else(|_| "root".into());
    let password_env = std::env::var("LIBREFANG_TEST_SURREAL_PASS_ENV")
        .unwrap_or_else(|_| "LIBREFANG_TEST_SURREAL_PASS".into());
    for url in remote_urls(&url) {
        let database = format!("v52_{}", uuid::Uuid::new_v4().simple());
        eprintln!("remote surreal: migrations v50..v52 against {url} db={database}");
        let session = SurrealConnectionPool::new()
            .open_remote(&RemoteSurrealConfig {
                url: url.clone(),
                namespace: "librefang_test".into(),
                database: database.clone(),
                username: username.clone(),
                password_env: password_env.clone(),
                tls_skip_verify: false,
            })
            .await
            .unwrap_or_else(|e| panic!("open remote {url}: {e}"));
        exercise(&session).await;
        session
            .client()
            .query(format!("REMOVE DATABASE IF EXISTS {database}"))
            .await
            .expect("drop test database");
    }
}
