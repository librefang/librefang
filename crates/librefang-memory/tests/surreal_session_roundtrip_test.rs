//! Round-trip tests for [`SurrealSessionBackend`] against the real operational schema.
//!
//! Every test applies the full `OPERATIONAL_MIGRATIONS` set (twice, to prove idempotence) before exercising the backend, so a schema/backend type mismatch fails here rather than at runtime.
//! The embedded variant runs on `kv-surrealkv` in a temp dir.
//! The remote variant runs the same body over `ws://` and `http://` when `BOSSFANG_TEST_SURREAL_URL` is set, and prints an explicit SKIP line when it is not.
//!
//! Remote knobs:
//! - `BOSSFANG_TEST_SURREAL_URL` — `ws://host:port` or `http://host:port`; the other scheme is derived from it.
//! - `BOSSFANG_TEST_SURREAL_USER` — defaults to `root`.
//! - `BOSSFANG_TEST_SURREAL_PASS_ENV` — the *name* of the env var that holds the password (defaults to `BOSSFANG_TEST_SURREAL_PASS`), following the `password_env` convention.

#![cfg(feature = "surreal-backend")]

use librefang_memory::session::Session;
use librefang_memory::{SessionBackend, SurrealSessionBackend};
use librefang_storage::config::{RemoteSurrealConfig, StorageBackendKind, StorageConfig};
use librefang_storage::migrations::{apply_pending, OPERATIONAL_MIGRATIONS};
use librefang_storage::{SurrealConnectionPool, SurrealSession};
use librefang_types::agent::{AgentId, SessionId};
use librefang_types::message::{ContentBlock, Message, MessageContent, Role};
use serde_json::Value as JsonValue;

const REMOTE_URL_ENV: &str = "BOSSFANG_TEST_SURREAL_URL";
const REMOTE_USER_ENV: &str = "BOSSFANG_TEST_SURREAL_USER";
const REMOTE_PASS_ENV_ENV: &str = "BOSSFANG_TEST_SURREAL_PASS_ENV";
const DEFAULT_REMOTE_PASS_ENV: &str = "BOSSFANG_TEST_SURREAL_PASS";

fn session(agent_id: AgentId, messages: Vec<Message>, parent: Option<SessionId>) -> Session {
    Session {
        id: SessionId(uuid::Uuid::new_v4()),
        agent_id,
        messages,
        context_window_tokens: 1234,
        label: Some("round-trip".to_string()),
        parent_session_id: parent,
        model_override: None,
        messages_generation: 0,
        last_repaired_generation: None,
        peer_id: None,
    }
}

/// A conversation that exercises every nesting level the schema has to keep: plain text, content blocks, and an arbitrary JSON tool input.
fn conversation() -> Vec<Message> {
    vec![
        Message::user("hello"),
        Message {
            role: Role::Assistant,
            content: MessageContent::Blocks(vec![
                ContentBlock::Text {
                    text: "calling a tool".to_string(),
                    provider_metadata: None,
                },
                ContentBlock::ToolUse {
                    id: "tool-1".to_string(),
                    name: "search".to_string(),
                    input: serde_json::json!({"query": "surreal", "opts": {"limit": 3, "tags": ["a", "b"]}}),
                    provider_metadata: Some(serde_json::json!({"thoughtSignature": "sig"})),
                },
            ]),
            pinned: true,
            timestamp: None,
        },
        Message::assistant("done"),
    ]
}

fn as_json(messages: &[Message]) -> JsonValue {
    serde_json::to_value(messages).expect("serialise messages")
}

async fn migrate(session: &SurrealSession) {
    apply_pending(session.client(), OPERATIONAL_MIGRATIONS)
        .await
        .expect("first migration run");
    let second = apply_pending(session.client(), OPERATIONAL_MIGRATIONS)
        .await
        .expect("second migration run");
    assert!(
        second.is_empty(),
        "re-applying migrations must be a no-op, applied {second:?}"
    );
}

/// The shared body: runs on whatever SurrealDB the session points at.
async fn exercise_backend(store: &SurrealSession) {
    migrate(store).await;
    let backend = SurrealSessionBackend::open(store);
    let agent = AgentId(uuid::Uuid::new_v4());

    let parent = session(agent, conversation(), None);
    backend.save_session(&parent).expect("save parent");
    let mut child = session(agent, vec![Message::user("sub-task")], Some(parent.id));
    child.label = None;
    child.model_override = Some("provider/model".to_string());
    backend.save_session(&child).expect("save child");

    let loaded = backend
        .get_session(parent.id)
        .expect("get parent")
        .expect("parent exists");
    assert_eq!(loaded.id, parent.id);
    assert_eq!(loaded.agent_id, agent);
    assert_eq!(
        as_json(&loaded.messages),
        as_json(&parent.messages),
        "messages must round-trip unchanged"
    );
    assert_eq!(loaded.context_window_tokens, 1234);
    assert_eq!(loaded.label.as_deref(), Some("round-trip"));
    assert_eq!(loaded.parent_session_id, None);

    let loaded_child = backend
        .get_session(child.id)
        .expect("get child")
        .expect("child exists");
    assert_eq!(loaded_child.parent_session_id, Some(parent.id));
    assert_eq!(loaded_child.label, None);
    assert_eq!(
        loaded_child.model_override.as_deref(),
        Some("provider/model")
    );
    assert_eq!(as_json(&loaded_child.messages), as_json(&child.messages));

    // Re-saving keeps created_at and replaces the history.
    let mut updated = loaded;
    updated.messages.push(Message::user("again"));
    backend.save_session(&updated).expect("re-save parent");
    let reloaded = backend
        .get_session(parent.id)
        .expect("get parent again")
        .expect("parent still exists");
    assert_eq!(reloaded.messages.len(), 4);

    // The v43 index serves the parent -> children lookup.
    let children: Vec<JsonValue> = store
        .client()
        .query("SELECT id FROM sessions WHERE parent_session_id = $parent")
        .bind(("parent", parent.id.to_string()))
        .await
        .expect("children query")
        .take(0)
        .expect("children rows");
    assert_eq!(children.len(), 1, "exactly one child: {children:?}");

    let mut ids = backend.get_agent_session_ids(agent).expect("list sessions");
    ids.sort_by_key(|id| id.0);
    let mut expected = vec![parent.id, child.id];
    expected.sort_by_key(|id| id.0);
    assert_eq!(ids, expected);
    assert!(backend
        .get_agent_session_ids(AgentId(uuid::Uuid::new_v4()))
        .expect("list other agent")
        .is_empty());

    // Canonical sessions: append twice, the history accumulates.
    backend
        .append_canonical(agent, &conversation(), None, None)
        .expect("first canonical append");
    backend
        .append_canonical(agent, &[Message::user("more")], None, None)
        .expect("second canonical append");
    let canonical: Option<JsonValue> = store
        .client()
        .select(("canonical_sessions", agent.to_string().as_str()))
        .await
        .expect("select canonical");
    let canonical = canonical.expect("canonical row exists");
    let mut expected_canonical = conversation();
    expected_canonical.push(Message::user("more"));
    let stored: Vec<Message> =
        serde_json::from_value(canonical["messages"].clone()).expect("canonical messages decode");
    assert_eq!(stored.len(), expected_canonical.len());
    assert_eq!(
        as_json(&stored)[1]["content"],
        as_json(&expected_canonical)[1]["content"],
        "nested content blocks must survive the canonical round trip"
    );

    backend
        .delete_canonical_session(agent)
        .expect("delete canonical");
    let gone: Option<JsonValue> = store
        .client()
        .select(("canonical_sessions", agent.to_string().as_str()))
        .await
        .expect("select canonical after delete");
    assert!(gone.is_none());

    backend
        .delete_agent_sessions(agent)
        .expect("delete sessions");
    assert!(backend
        .get_agent_session_ids(agent)
        .expect("list after delete")
        .is_empty());
    assert!(backend
        .get_session(parent.id)
        .expect("get deleted")
        .is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn surreal_session_backend_round_trips_embedded() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cfg = StorageConfig {
        backend: StorageBackendKind::embedded(dir.path().join("sessions.surreal")),
        namespace: "librefang".into(),
        database: "main".into(),
        legacy_sqlite_path: None,
    };
    let store = SurrealConnectionPool::new()
        .open(&cfg)
        .await
        .expect("open embedded");
    exercise_backend(&store).await;
}

/// The `ws://` and `http://` URLs for one server, derived from whichever scheme the operator gave.
fn remote_urls(url: &str) -> Vec<String> {
    let pairs = [("ws://", "http://"), ("wss://", "https://")];
    for (ws, http) in pairs {
        if let Some(rest) = url.strip_prefix(ws).or_else(|| url.strip_prefix(http)) {
            return vec![format!("{ws}{rest}"), format!("{http}{rest}")];
        }
    }
    vec![url.to_string()]
}

#[tokio::test(flavor = "multi_thread")]
async fn surreal_session_backend_round_trips_remote() {
    let Ok(url) = std::env::var(REMOTE_URL_ENV) else {
        eprintln!("SKIP remote surreal: {REMOTE_URL_ENV} unset");
        return;
    };
    let username = std::env::var(REMOTE_USER_ENV).unwrap_or_else(|_| "root".to_string());
    let password_env =
        std::env::var(REMOTE_PASS_ENV_ENV).unwrap_or_else(|_| DEFAULT_REMOTE_PASS_ENV.to_string());

    for url in remote_urls(&url) {
        // A fresh database per run and transport, so repeated runs never see each other's rows or migration ledger.
        let database = format!("sessions_rt_{}", uuid::Uuid::new_v4().simple());
        let remote = RemoteSurrealConfig {
            url: url.clone(),
            namespace: "bossfang_test".into(),
            database: database.clone(),
            username: username.clone(),
            password_env: password_env.clone(),
            tls_skip_verify: false,
        };
        let store = SurrealConnectionPool::new()
            .open_remote(&remote)
            .await
            .unwrap_or_else(|e| panic!("open remote {url}: {e}"));
        eprintln!("remote surreal: running session round trip against {url} db={database}");
        exercise_backend(&store).await;
        store
            .client()
            .query(format!("REMOVE DATABASE IF EXISTS {database}"))
            .await
            .expect("drop test database");
    }
}

#[test]
fn remote_urls_cover_both_transports() {
    assert_eq!(
        remote_urls("ws://127.0.0.1:8000"),
        vec!["ws://127.0.0.1:8000", "http://127.0.0.1:8000"]
    );
    assert_eq!(
        remote_urls("https://db.example"),
        vec!["wss://db.example", "https://db.example"]
    );
}
