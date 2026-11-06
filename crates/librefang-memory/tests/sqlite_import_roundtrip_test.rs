//! End-to-end SQLite → SurrealDB import with the real stores on both sides.
//!
//! The SQLite side is written through `MemorySubstrate`, so every BLOB is encoded exactly as a live daemon encodes it: MessagePack session histories (named), canonical histories (positional), agent manifests (named) and JSON kv values.
//! The SurrealDB side is read back through the Surreal backends the kernel uses, so the test fails if the importer writes a shape those backends cannot read.
//! The embedded variant runs on `kv-surrealkv` in a temp dir; the remote variant runs the same body over `ws://` and `http://` when `BOSSFANG_TEST_SURREAL_URL` is set, and prints an explicit SKIP line when it is not.
//!
//! Run with `cargo test -p librefang-memory --features sqlite-backend --test sqlite_import_roundtrip_test`.

#![cfg(all(feature = "surreal-backend", feature = "sqlite-backend"))]

use std::path::Path;

use librefang_memory::migration::TypedLegacyBlobDecoder;
use librefang_memory::session::Session;
use librefang_memory::{
    KvBackend, MemoryBackend, MemorySubstrate, SessionBackend, SurrealKvBackend,
    SurrealMemoryBackend, SurrealSessionBackend,
};
use librefang_storage::config::{RemoteSurrealConfig, StorageBackendKind, StorageConfig};
use librefang_storage::migrate::{
    migrate_sqlite_to_surreal, migrate_sqlite_to_surreal_with_decoder, MigrationOptions,
};
use librefang_storage::migrations::{apply_pending, OPERATIONAL_MIGRATIONS};
use librefang_storage::{SurrealConnectionPool, SurrealSession};
use librefang_types::agent::{AgentEntry, AgentId, AgentManifest, AgentState, SessionId};
use librefang_types::message::{ContentBlock, Message, MessageContent, Role};
use serde_json::{json, Value};

struct Fixture {
    agent: AgentId,
    session: Session,
    canonical: Vec<Message>,
}

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
                    input: json!({"query": "surreal", "opts": {"limit": 3, "tags": ["a", "b"]}}),
                    provider_metadata: Some(json!({"thoughtSignature": "sig"})),
                },
            ]),
            pinned: true,
            timestamp: None,
        },
        Message::assistant("done"),
    ]
}

/// Write a legacy SQLite database the way a running daemon does.
fn seed_sqlite(path: &Path) -> Fixture {
    let substrate = MemorySubstrate::open(path, 0.05).expect("open sqlite substrate");
    let agent = AgentId::new();

    let entry = AgentEntry {
        id: agent,
        name: "importer-probe".to_string(),
        manifest: AgentManifest {
            name: "importer-probe".to_string(),
            ..AgentManifest::default()
        },
        state: AgentState::Running,
        created_at: chrono::Utc::now(),
        last_active: chrono::Utc::now(),
        session_id: SessionId::new(),
        ..Default::default()
    };
    substrate.save_agent(&entry).expect("save agent");

    let session = Session {
        id: SessionId::new(),
        agent_id: agent,
        messages: conversation(),
        context_window_tokens: 4321,
        label: None,
        parent_session_id: None,
        model_override: Some("provider/model".to_string()),
        messages_generation: 0,
        last_repaired_generation: None,
        peer_id: None,
    };
    substrate.save_session(&session).expect("save session");
    // `save_session` does not write the override column; the kernel sets it separately.
    substrate
        .set_session_model_override(session.id, Some("provider/model"))
        .expect("set model override");

    let canonical = vec![Message::user("remember this"), Message::assistant("noted")];
    substrate
        .append_canonical(agent, &canonical, None, Some(session.id))
        .expect("append canonical");

    substrate
        .structured_set(agent, "greeting", json!("hello"))
        .expect("kv string");
    substrate
        .structured_set(agent, "config", json!({"nested": {"k": [1, 2]}}))
        .expect("kv object");

    Fixture {
        agent,
        session,
        canonical,
    }
}

async fn migrated(session: &SurrealSession) {
    apply_pending(session.client(), OPERATIONAL_MIGRATIONS)
        .await
        .expect("operational migrations");
}

fn opts() -> MigrationOptions {
    MigrationOptions {
        dry_run: false,
        receipt_dir: None,
    }
}

async fn import_and_verify(store: &SurrealSession, sqlite_path: &Path, fx: &Fixture) {
    migrated(store).await;

    let receipt = migrate_sqlite_to_surreal_with_decoder(
        sqlite_path,
        store,
        &opts(),
        &TypedLegacyBlobDecoder,
    )
    .expect("import");
    assert!(receipt.is_clean(), "errors: {:?}", receipt.errors);
    assert_eq!(receipt.copied.get("sessions"), Some(&1));
    assert_eq!(receipt.copied.get("canonical_sessions"), Some(&1));
    assert_eq!(receipt.copied.get("agents"), Some(&1));
    assert_eq!(receipt.copied.get("kv_store"), Some(&2));

    let sessions = SurrealSessionBackend::open(store);
    let loaded = sessions
        .get_session(fx.session.id)
        .expect("get imported session")
        .expect("imported session exists");
    assert_eq!(loaded.agent_id, fx.agent);
    assert_eq!(
        serde_json::to_value(&loaded.messages).unwrap(),
        serde_json::to_value(&fx.session.messages).unwrap(),
        "the MessagePack history must survive the import unchanged"
    );
    assert_eq!(loaded.context_window_tokens, 4321);
    assert_eq!(loaded.label, None);
    assert_eq!(loaded.model_override.as_deref(), Some("provider/model"));
    assert_eq!(
        sessions.get_agent_session_ids(fx.agent).expect("list"),
        vec![fx.session.id]
    );

    let canonical: Option<Value> = store
        .client()
        .select(("canonical_sessions", fx.agent.to_string().as_str()))
        .await
        .expect("select canonical");
    let canonical = canonical.expect("canonical row imported");
    let stored: Vec<Message> =
        serde_json::from_value(canonical["messages"].clone()).expect("canonical decodes");
    assert_eq!(
        serde_json::to_value(&stored).unwrap(),
        serde_json::to_value(&fx.canonical).unwrap(),
        "the positional canonical history must survive the import"
    );

    let agents = SurrealMemoryBackend::new(store.clone())
        .await
        .expect("agent backend");
    let agent = agents
        .load_agent(fx.agent)
        .expect("load imported agent")
        .expect("imported agent exists");
    assert_eq!(agent.name, "importer-probe");
    assert_eq!(agent.manifest.name, "importer-probe");

    let kv = SurrealKvBackend::open(store);
    assert_eq!(
        kv.structured_get(fx.agent, "greeting").expect("kv get"),
        Some(json!("hello"))
    );
    assert_eq!(
        kv.structured_get(fx.agent, "config").expect("kv get"),
        Some(json!({"nested": {"k": [1, 2]}}))
    );

    // Idempotent: importing again converges on the same rows.
    let again = migrate_sqlite_to_surreal_with_decoder(
        sqlite_path,
        store,
        &opts(),
        &TypedLegacyBlobDecoder,
    )
    .expect("re-import");
    assert!(again.is_clean(), "errors: {:?}", again.errors);
    assert_eq!(
        sessions.get_agent_session_ids(fx.agent).expect("list"),
        vec![fx.session.id]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_import_round_trips_through_surreal_backends_embedded() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sqlite_path = dir.path().join("librefang.db");
    let fx = seed_sqlite(&sqlite_path);
    let cfg = StorageConfig {
        backend: StorageBackendKind::embedded(dir.path().join("librefang.surreal")),
        namespace: "librefang".into(),
        database: "main".into(),
        legacy_sqlite_path: None,
    };
    let store = SurrealConnectionPool::new()
        .open(&cfg)
        .await
        .expect("open embedded");
    import_and_verify(&store, &sqlite_path, &fx).await;
}

/// Without the typed decoder the positional canonical history is reported, never imported as an empty list.
#[tokio::test(flavor = "multi_thread")]
async fn generic_import_reports_positional_canonical_history() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sqlite_path = dir.path().join("librefang.db");
    let fx = seed_sqlite(&sqlite_path);
    let cfg = StorageConfig {
        backend: StorageBackendKind::embedded(dir.path().join("librefang.surreal")),
        namespace: "librefang".into(),
        database: "main".into(),
        legacy_sqlite_path: None,
    };
    let store = SurrealConnectionPool::new()
        .open(&cfg)
        .await
        .expect("open embedded");
    migrated(&store).await;
    let receipt = migrate_sqlite_to_surreal(&sqlite_path, &store, &opts()).expect("import");
    assert_eq!(
        receipt.copied.get("sessions"),
        Some(&1),
        "named history imports"
    );
    assert_eq!(receipt.copied.get("canonical_sessions"), Some(&0));
    assert!(
        receipt.errors["canonical_sessions"].contains("typed LegacyBlobDecoder"),
        "{:?}",
        receipt.errors
    );
    let canonical: Option<Value> = store
        .client()
        .select(("canonical_sessions", fx.agent.to_string().as_str()))
        .await
        .expect("select canonical");
    assert!(canonical.is_none());
}

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
async fn sqlite_import_round_trips_through_surreal_backends_remote() {
    let Ok(url) = std::env::var("BOSSFANG_TEST_SURREAL_URL") else {
        eprintln!("SKIP remote surreal: BOSSFANG_TEST_SURREAL_URL unset");
        return;
    };
    let username = std::env::var("BOSSFANG_TEST_SURREAL_USER").unwrap_or_else(|_| "root".into());
    let password_env = std::env::var("BOSSFANG_TEST_SURREAL_PASS_ENV")
        .unwrap_or_else(|_| "BOSSFANG_TEST_SURREAL_PASS".into());
    for url in remote_urls(&url) {
        let dir = tempfile::tempdir().expect("tempdir");
        let sqlite_path = dir.path().join("librefang.db");
        let fx = seed_sqlite(&sqlite_path);
        let database = format!("import_rt_{}", uuid::Uuid::new_v4().simple());
        eprintln!("remote surreal: sqlite import round trip against {url} db={database}");
        let store = SurrealConnectionPool::new()
            .open_remote(&RemoteSurrealConfig {
                url: url.clone(),
                namespace: "bossfang_test".into(),
                database: database.clone(),
                username: username.clone(),
                password_env: password_env.clone(),
                tls_skip_verify: false,
            })
            .await
            .unwrap_or_else(|e| panic!("open remote {url}: {e}"));
        import_and_verify(&store, &sqlite_path, &fx).await;
        store
            .client()
            .query(format!("REMOVE DATABASE IF EXISTS {database}"))
            .await
            .expect("drop test database");
    }
}
