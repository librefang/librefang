//! Behaviour of the SurrealDB memory backends against a real store.
//!
//! Every body runs embedded (SurrealKV in a temp dir), and again over `ws://` and `http://` when `BOSSFANG_TEST_SURREAL_URL` is set (an explicit SKIP line is printed when it is not).
//! Remote configuration follows `surreal_session_roundtrip_test.rs`:
//! - `BOSSFANG_TEST_SURREAL_URL` — `ws://host:port` or `http://host:port`; the other scheme is derived from it.
//! - `BOSSFANG_TEST_SURREAL_USER` — defaults to `root`.
//! - `BOSSFANG_TEST_SURREAL_PASS_ENV` — the *name* of the env var that holds the password (defaults to `BOSSFANG_TEST_SURREAL_PASS`).
//!

#![cfg(feature = "surreal-backend")]

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use librefang_memory::proactive::EmbeddingFn;
use librefang_memory::{
    MemorySubstrate, ProactiveMemoryBackend, SemanticBackend, SurrealProactiveMemoryBackend,
    SurrealSemanticBackend,
};
use librefang_storage::config::{RemoteSurrealConfig, StorageBackendKind, StorageConfig};
use librefang_types::agent::AgentId;
use librefang_types::config::MemoryDecayConfig;
use librefang_types::error::LibreFangResult;
use librefang_types::memory::{MemoryFilter, MemorySource, VectorStore};
use surreal_memory::{Memory, MemoryStorage};

const DIMS: usize = 4;

const REMOTE_URL_ENV: &str = "BOSSFANG_TEST_SURREAL_URL";
const REMOTE_USER_ENV: &str = "BOSSFANG_TEST_SURREAL_USER";
const REMOTE_PASS_ENV_ENV: &str = "BOSSFANG_TEST_SURREAL_PASS_ENV";
const DEFAULT_REMOTE_PASS_ENV: &str = "BOSSFANG_TEST_SURREAL_PASS";

/// Maps a content string to a fixed unit axis by its first word, and counts calls so a test can prove a precomputed vector was not re-embedded.
#[derive(Default)]
struct AxisEmbedding {
    calls: AtomicUsize,
}

fn axis(word: &str) -> Vec<f32> {
    let mut v = vec![0.0_f32; DIMS];
    let i = match word {
        "alpha" => 0,
        "beta" => 1,
        "gamma" => 2,
        _ => 3,
    };
    v[i] = 1.0;
    v
}

#[async_trait::async_trait]
impl EmbeddingFn for AxisEmbedding {
    async fn embed_one(&self, text: &str) -> LibreFangResult<Vec<f32>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(axis(text.split_whitespace().next().unwrap_or_default()))
    }
}

async fn open_backend(cfg: &StorageConfig) -> (SurrealSemanticBackend, Arc<AxisEmbedding>) {
    let embedder = Arc::new(AxisEmbedding::default());
    let backend = SurrealSemanticBackend::open_with_storage(cfg, embedder.clone(), DIMS)
        .await
        .expect("open SurrealSemanticBackend");
    (backend, embedder)
}

/// Storage configs to run a body against: embedded always, plus both remote transports when configured.
/// Each remote run gets its own namespace so repeated runs never see each other's rows; the returned names are removed afterwards.
fn configs(tmp: &std::path::Path, label: &str) -> Vec<(StorageConfig, Option<(String, String)>)> {
    let mut out = vec![(StorageConfig::embedded_default(tmp), None)];
    let Ok(url) = std::env::var(REMOTE_URL_ENV) else {
        eprintln!("SKIP remote surreal ({label}): {REMOTE_URL_ENV} unset");
        return out;
    };
    let username = std::env::var(REMOTE_USER_ENV).unwrap_or_else(|_| "root".to_string());
    let password_env =
        std::env::var(REMOTE_PASS_ENV_ENV).unwrap_or_else(|_| DEFAULT_REMOTE_PASS_ENV.to_string());
    for url in remote_urls(&url) {
        let namespace = format!("bossfang_mem_{}", uuid::Uuid::new_v4().simple());
        eprintln!("remote surreal ({label}): {url} ns={namespace}");
        out.push((
            StorageConfig {
                backend: StorageBackendKind::Remote(RemoteSurrealConfig {
                    url: url.clone(),
                    namespace: namespace.clone(),
                    database: "main".into(),
                    username: username.clone(),
                    password_env: password_env.clone(),
                    tls_skip_verify: false,
                }),
                namespace: namespace.clone(),
                database: "main".into(),
                legacy_sqlite_path: None,
            },
            Some((url, namespace)),
        ));
    }
    out
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

async fn drop_namespace(storage: &Arc<surreal_memory::SurrealStorage>, namespace: &str) {
    storage
        .db()
        .expect("memory store connection")
        .query(format!("REMOVE NAMESPACE IF EXISTS {namespace}"))
        .await
        .expect("drop test namespace");
}

// ── VectorStore mirror ────────────────────────────────────────────────────────

async fn exercise_vector_store(backend: &SurrealSemanticBackend) {
    let a = uuid::Uuid::new_v4().to_string();
    let b = uuid::Uuid::new_v4().to_string();
    let c = uuid::Uuid::new_v4().to_string();
    backend
        .insert(&a, &[1.0, 0.0, 0.0, 0.0], "a", HashMap::new())
        .await
        .expect("insert a");
    backend
        .insert(&b, &[0.8, 0.6, 0.0, 0.0], "b", HashMap::new())
        .await
        .expect("insert b");
    backend
        .insert(&c, &[0.0, 0.0, 1.0, 0.0], "c", HashMap::new())
        .await
        .expect("insert c");

    let hits = backend
        .search(&[1.0, 0.0, 0.0, 0.0], 3, None)
        .await
        .expect("search");
    let ids: Vec<&str> = hits.iter().map(|h| h.id.as_str()).collect();
    assert_eq!(
        ids,
        vec![a.as_str(), b.as_str(), c.as_str()],
        "nearest first"
    );
    assert!(
        (hits[0].score - 1.0).abs() < 1e-4,
        "score is the cosine similarity: {}",
        hits[0].score
    );
    assert!(
        (hits[1].score - 0.8).abs() < 1e-4,
        "score is the cosine similarity: {}",
        hits[1].score
    );

    // Mirror rows carry no owner, so an agent-filtered search still reaches them; the SemanticStore re-filters after hydrating.
    let filtered = backend
        .search(
            &[1.0, 0.0, 0.0, 0.0],
            1,
            Some(MemoryFilter::agent(AgentId::new())),
        )
        .await
        .expect("filtered search");
    assert_eq!(filtered.first().map(|h| h.id.as_str()), Some(a.as_str()));

    let missing = uuid::Uuid::new_v4().to_string();
    let got = backend
        .get_embeddings(&[&a, &missing])
        .await
        .expect("get_embeddings");
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[&a], vec![1.0, 0.0, 0.0, 0.0]);

    // Insert is an upsert: `update_embedding` relies on it to replace a stale vector.
    backend
        .insert(&a, &[0.0, 1.0, 0.0, 0.0], "a2", HashMap::new())
        .await
        .expect("upsert a");
    let got = backend
        .get_embeddings(&[&a])
        .await
        .expect("get_embeddings after upsert");
    assert_eq!(got[&a], vec![0.0, 1.0, 0.0, 0.0]);

    backend.delete(&c).await.expect("delete c");
    backend.delete(&c).await.expect("delete is idempotent");
    assert!(backend.get_embeddings(&[&c]).await.expect("get").is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn vector_store_mirror_round_trips() {
    let tmp = tempfile::tempdir().expect("tempdir");
    for (cfg, remote) in configs(tmp.path(), "vector store") {
        let (backend, embedder) = open_backend(&cfg).await;
        exercise_vector_store(&backend).await;
        assert_eq!(
            embedder.calls.load(Ordering::SeqCst),
            0,
            "the mirror never embeds"
        );
        if let Some((_, ns)) = remote {
            drop_namespace(&backend.storage(), &ns).await;
        }
    }
}

/// The path the kernel wires: SQLite stays the system of record, the Surreal backend ranks vectors, and hits are hydrated from SQLite.
async fn exercise_substrate_mirror(backend: Arc<SurrealSemanticBackend>) {
    let substrate = MemorySubstrate::open_in_memory(0.0).expect("substrate");
    let agent = AgentId::new();
    let other = AgentId::new();

    // Written before the backend is attached: only the backfill can make it recallable.
    let before = substrate
        .remember_with_embedding(
            agent,
            "alpha written before the switch",
            MemorySource::Conversation,
            "episodic",
            HashMap::new(),
            Some(&axis("alpha")),
            None,
        )
        .expect("remember before attach");

    substrate.set_vector_store(backend.clone());
    assert_eq!(substrate.vector_backend_name().as_deref(), Some("surreal"));

    assert_eq!(
        substrate.sync_vector_store(DIMS).await.expect("backfill"),
        1
    );
    assert_eq!(
        substrate
            .sync_vector_store(DIMS)
            .await
            .expect("second backfill"),
        0,
        "the backfill never copies a vector twice"
    );

    let after = substrate
        .remember_with_embedding(
            agent,
            "beta written after the switch",
            MemorySource::Conversation,
            "episodic",
            HashMap::new(),
            Some(&axis("beta")),
            None,
        )
        .expect("remember after attach");
    // Another agent's nearest-neighbour row must never surface in this agent's recall.
    substrate
        .remember_with_embedding(
            other,
            "alpha from another agent",
            MemorySource::Conversation,
            "episodic",
            HashMap::new(),
            Some(&axis("alpha")),
            None,
        )
        .expect("remember other agent");

    let mirrored = backend
        .get_embeddings(&[&before.0.to_string(), &after.0.to_string()])
        .await
        .expect("mirror lookup");
    assert_eq!(mirrored.len(), 2, "remember writes through to the mirror");

    let recalled = substrate
        .recall_with_embedding(
            "",
            2,
            Some(MemoryFilter::agent(agent)),
            Some(&axis("alpha")),
        )
        .expect("recall");
    assert_eq!(recalled.first().map(|f| f.id), Some(before), "{recalled:?}");
    assert!(recalled.iter().all(|f| f.agent_id == agent), "{recalled:?}");
    assert!(
        recalled[0]
            .similarity
            .is_some_and(|s| (s - 1.0).abs() < 1e-4),
        "similarity comes from the Surreal score: {:?}",
        recalled[0].similarity
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn substrate_recall_goes_through_the_surreal_mirror() {
    let tmp = tempfile::tempdir().expect("tempdir");
    for (cfg, remote) in configs(tmp.path(), "substrate mirror") {
        let (backend, _) = open_backend(&cfg).await;
        let backend = Arc::new(backend);
        exercise_substrate_mirror(backend.clone()).await;
        if let Some((_, ns)) = remote {
            drop_namespace(&backend.storage(), &ns).await;
        }
    }
}

// ── Proactive backend ─────────────────────────────────────────────────────────

async fn exercise_proactive(backend: &SurrealSemanticBackend) {
    let storage = backend.storage();
    let proactive = SurrealProactiveMemoryBackend::new(storage.clone()).expect("proactive backend");
    let old = chrono::Utc::now() - chrono::Duration::days(40);

    let row = |content: &str, scope: &str, accessed: Option<chrono::DateTime<chrono::Utc>>| {
        let mut m = Memory::new(content, None, None, None, vec![scope.to_string()]);
        m.last_accessed_at = accessed.map(Into::into);
        m
    };
    let stale_episodic = uuid::Uuid::new_v4().to_string();
    let fresh_episodic = uuid::Uuid::new_v4().to_string();
    let stale_user = uuid::Uuid::new_v4().to_string();
    for (key, mem) in [
        (
            &stale_episodic,
            row("stale episodic", "episodic", Some(old)),
        ),
        (&fresh_episodic, row("fresh episodic", "episodic", None)),
        (&stale_user, row("stale user", "user_memory", Some(old))),
    ] {
        storage
            .store_indexed_memory(key, mem, axis("delta"), None)
            .await
            .expect("store row");
    }

    let config = MemoryDecayConfig {
        enabled: true,
        episodic_ttl_days: 30,
        ..MemoryDecayConfig::default()
    };
    // `run_decay` is sync and blocks in place on the caller's (multi-thread) runtime, as the kernel's decay sweep does.
    let removed = proactive.run_decay(&config);
    assert_eq!(removed.expect("run_decay"), 1);
    assert!(storage
        .get_memory(&stale_episodic)
        .await
        .expect("get")
        .is_none());
    assert!(storage
        .get_memory(&fresh_episodic)
        .await
        .expect("get")
        .is_some());
    assert!(
        storage
            .get_memory(&stale_user)
            .await
            .expect("get")
            .is_some(),
        "user-scope memories never decay"
    );

    let disabled = MemoryDecayConfig {
        enabled: false,
        ..config
    };
    assert_eq!(proactive.run_decay(&disabled).expect("disabled decay"), 0);

    let mut expiring = Memory::new("expired fact", None, None, None, Vec::new());
    // surreal-memory hides expired rows at read time (since #52), so a row that is
    // already expired at write time cannot be stored. Expire it shortly after instead.
    expiring.valid_until = Some((chrono::Utc::now() + chrono::Duration::milliseconds(500)).into());
    let expiring_key = uuid::Uuid::new_v4().to_string();
    storage
        .store_indexed_memory(&expiring_key, expiring, axis("delta"), None)
        .await
        .expect("store expiring row");
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    let report = proactive.consolidate().await.expect("consolidate");
    assert_eq!(report.memories_decayed, 1, "{report:?}");
    assert!(storage
        .get_memory(&expiring_key)
        .await
        .expect("get")
        .is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn proactive_backend_decays_and_expires_memory_rows() {
    let tmp = tempfile::tempdir().expect("tempdir");
    for (cfg, remote) in configs(tmp.path(), "proactive") {
        let (backend, _) = open_backend(&cfg).await;
        exercise_proactive(&backend).await;
        if let Some((_, ns)) = remote {
            drop_namespace(&backend.storage(), &ns).await;
        }
    }
}

// ── SemanticBackend face ──────────────────────────────────────────────────────

async fn exercise_semantic_backend(backend: &SurrealSemanticBackend, embedder: &AxisEmbedding) {
    let agent = AgentId::new();
    let other = AgentId::new();
    let mut meta = HashMap::new();
    meta.insert("topic".to_string(), serde_json::json!("coffee"));

    // A precomputed vector is stored as given: the content says "gamma", the vector says "alpha".
    let a = backend
        .remember(
            agent,
            "gamma words",
            MemorySource::Conversation,
            "episodic",
            meta,
            Some(axis("alpha")),
        )
        .await
        .expect("remember with vector");
    assert_eq!(
        embedder.calls.load(Ordering::SeqCst),
        0,
        "a supplied vector is not re-embedded"
    );
    let b = backend
        .remember(
            agent,
            "beta words",
            MemorySource::Document,
            "agent_memory",
            HashMap::new(),
            None,
        )
        .await
        .expect("remember without vector");
    assert_eq!(
        embedder.calls.load(Ordering::SeqCst),
        1,
        "a missing vector is embedded once"
    );
    backend
        .remember(
            other,
            "alpha other",
            MemorySource::Conversation,
            "episodic",
            HashMap::new(),
            Some(axis("alpha")),
        )
        .await
        .expect("remember other agent");
    // A mirror row has no owner and must never surface through this face.
    backend
        .insert(
            &uuid::Uuid::new_v4().to_string(),
            &axis("alpha"),
            "mirror",
            HashMap::new(),
        )
        .await
        .expect("mirror insert");

    let hits = backend
        .recall("", 5, Some(MemoryFilter::agent(agent)), Some(axis("alpha")))
        .await
        .expect("vector recall");
    assert_eq!(
        hits.iter().map(|f| f.id).collect::<Vec<_>>(),
        vec![a, b],
        "{hits:?}"
    );
    assert_eq!(hits[0].agent_id, agent);
    assert_eq!(hits[0].scope, "episodic");
    assert_eq!(
        hits[0].metadata.get("topic"),
        Some(&serde_json::json!("coffee"))
    );
    assert!(hits[0].similarity.is_some_and(|s| (s - 1.0).abs() < 1e-4));
    assert_eq!(hits[1].source, MemorySource::Document);

    assert_eq!(
        backend
            .count(MemoryFilter::agent(agent))
            .await
            .expect("count"),
        2
    );
    assert_eq!(
        backend
            .count(MemoryFilter::agent(other))
            .await
            .expect("count other"),
        1
    );

    backend.update_access(a).await.expect("update_access");
    let hits = backend
        .recall("", 1, Some(MemoryFilter::agent(agent)), Some(axis("alpha")))
        .await
        .expect("recall after access");
    assert_eq!(hits[0].access_count, 1);

    assert!(backend.forget(a).await.expect("forget"));
    assert!(
        !backend.forget(a).await.expect("forget twice"),
        "a second forget finds nothing"
    );
    assert_eq!(
        backend
            .count(MemoryFilter::agent(agent))
            .await
            .expect("count"),
        1
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn semantic_backend_remember_recall_count_forget() {
    let tmp = tempfile::tempdir().expect("tempdir");
    for (cfg, remote) in configs(tmp.path(), "semantic backend") {
        let (backend, embedder) = open_backend(&cfg).await;
        exercise_semantic_backend(&backend, &embedder).await;
        if let Some((_, ns)) = remote {
            drop_namespace(&backend.storage(), &ns).await;
        }
    }
}
