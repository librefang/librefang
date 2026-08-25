//! Integration tests for the network/peers/comms slice of `routes::network`.
//!
//! Refs #3571 — most registered HTTP routes have no integration test, and
//! the `network.rs` module is one of the largest uncovered surfaces. This
//! file mounts the real `routes::network::router()` against a freshly-booted
//! mock kernel and exercises the read-side peers/network endpoints plus the
//! happy-and-error paths of `/api/comms/*` that are safe to drive without
//! real LLM credentials or a live OFP socket.
//!
//! The A2A endpoints (`/api/a2a/*` and the protocol router) are intentionally
//! out of scope — covered by a separate slice.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::agent::AgentManifest;
use librefang_types::config::DefaultModelConfig;
use librefang_wire::registry::{PeerEntry, PeerRegistry, PeerState};
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: Router,
    state: Arc<AppState>,
    _test: TestAppState,
}

/// Boot a harness with the bare network router mounted under `/api`.
fn boot() -> Harness {
    boot_with(|_| {})
}

/// Boot a harness, allowing the caller to configure the freshly-built state
/// (e.g. to install a peer registry on the kernel) before mounting the router.
fn boot_with<F: FnOnce(&AppState)>(configure: F) -> Harness {
    boot_with_builder(MockKernelBuilder::new(), configure)
}

fn boot_with_builder<F: FnOnce(&AppState)>(builder: MockKernelBuilder, configure: F) -> Harness {
    let test = TestAppState::with_builder(builder);
    configure(&test.state);

    let state = test.state.clone();
    let app = Router::new()
        .nest("/api", routes::network::router())
        .with_state(state.clone());

    Harness {
        app,
        state,
        _test: test,
    }
}

async fn json_request(
    h: &Harness,
    method: Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    json_request_with_user(h, method, path, body, None).await
}

async fn json_request_as_owner(
    h: &Harness,
    method: Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    json_request_with_user(
        h,
        method,
        path,
        body,
        Some(librefang_api::middleware::AuthenticatedApiUser {
            name: "root".to_string(),
            role: librefang_api::middleware::UserRole::Owner,
            user_id: librefang_types::agent::UserId::from_name("root-test"),
        }),
    )
    .await
}

async fn json_request_with_user(
    h: &Harness,
    method: Method,
    path: &str,
    body: Option<serde_json::Value>,
    user: Option<librefang_api::middleware::AuthenticatedApiUser>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder().method(method).uri(path);
    let body_bytes = match body {
        Some(v) => {
            builder = builder.header("content-type", "application/json");
            serde_json::to_vec(&v).unwrap()
        }
        None => Vec::new(),
    };
    let mut req = builder.body(Body::from(body_bytes)).unwrap();
    if let Some(user) = user {
        req.extensions_mut().insert(user);
    }
    let resp = h.app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let value: serde_json::Value = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, value)
}

fn nested_error_message(body: &serde_json::Value) -> &str {
    body["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("expected canonical nested error envelope: {body}"))
}

fn flat_error_message(body: &serde_json::Value) -> &str {
    body["error"]
        .as_str()
        .unwrap_or_else(|| panic!("expected flat error envelope: {body}"))
}

const MOCK_LLM_REPLY: &str = "message accepted by mock provider";

async fn spawn_mock_openai() -> (String, tokio::task::JoinHandle<()>) {
    async fn completions_handler() -> axum::Json<serde_json::Value> {
        axum::Json(serde_json::json!({
            "id": "chatcmpl-network-test",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": MOCK_LLM_REPLY },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 5, "completion_tokens": 4, "total_tokens": 9 }
        }))
    }

    let app = Router::new().route(
        "/chat/completions",
        axum::routing::post(completions_handler),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock LLM");
    let addr = listener.local_addr().expect("mock LLM address");
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}"), handle)
}

fn sample_peer(node_id: &str, name: &str) -> PeerEntry {
    PeerEntry {
        node_id: node_id.to_string(),
        node_name: name.to_string(),
        address: "127.0.0.1:9000".parse().unwrap(),
        agents: Vec::new(),
        state: PeerState::Connected,
        connected_at: chrono::Utc::now(),
        protocol_version: 1,
    }
}

// ---------------------------------------------------------------------------
// /api/peers — list_peers
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn peers_list_returns_empty_envelope_when_no_registry() {
    let h = boot();
    let (status, body) = json_request(&h, Method::GET, "/api/peers", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"], serde_json::json!([]));
    assert_eq!(body["total"], 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn peers_list_surfaces_seeded_registry() {
    let registry = PeerRegistry::new();
    registry.add_peer(sample_peer("node-a", "Node A"));
    registry.add_peer(sample_peer("node-b", "Node B"));

    let h = {
        let cloned = registry.clone();
        boot_with(move |s| {
            s.kernel
                .install_peer_registry_for_test(cloned)
                .expect("registry not yet set");
        })
    };

    let (status, body) = json_request(&h, Method::GET, "/api/peers", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 2);
    let peers = body["items"].as_array().expect("items array");
    assert_eq!(peers.len(), 2);
    let names: Vec<&str> = peers
        .iter()
        .map(|p| p["node_name"].as_str().unwrap_or(""))
        .collect();
    assert!(names.contains(&"Node A"), "{body}");
    assert!(names.contains(&"Node B"), "{body}");
    // Each peer entry must carry the dashboard-required fields.
    for p in peers {
        for key in [
            "node_id",
            "node_name",
            "address",
            "state",
            "agents",
            "connected_at",
            "protocol_version",
        ] {
            assert!(p.get(key).is_some(), "peer entry missing field {key}: {p}");
        }
    }
}

// ---------------------------------------------------------------------------
// /api/peers/{id} — get_peer
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn peers_get_returns_404_when_no_registry() {
    let h = boot();
    let (status, body) = json_request(&h, Method::GET, "/api/peers/anything", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        nested_error_message(&body)
            .to_lowercase()
            .contains("peer networking"),
        "expected 'peer networking' phrase: {body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn peers_get_returns_404_for_unknown_id() {
    let registry = PeerRegistry::new();
    let h = {
        let cloned = registry.clone();
        boot_with(move |s| {
            s.kernel
                .install_peer_registry_for_test(cloned)
                .expect("registry not yet set");
        })
    };
    let (status, body) = json_request(&h, Method::GET, "/api/peers/missing", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        nested_error_message(&body)
            .to_lowercase()
            .contains("not found"),
        "{body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn peers_get_returns_seeded_peer() {
    let registry = PeerRegistry::new();
    registry.add_peer(sample_peer("node-x", "Node X"));
    let h = {
        let cloned = registry.clone();
        boot_with(move |s| {
            s.kernel
                .install_peer_registry_for_test(cloned)
                .expect("registry not yet set");
        })
    };

    let (status, body) = json_request(&h, Method::GET, "/api/peers/node-x", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["node_id"], "node-x");
    assert_eq!(body["node_name"], "Node X");
    assert_eq!(body["protocol_version"], 1);
    // Connection state is rendered with Debug formatting (`Connected`).
    assert_eq!(body["state"], "Connected");
}

// ---------------------------------------------------------------------------
// /api/network/status
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn network_status_disabled_when_secret_empty() {
    let h = boot();
    let (status, body) = json_request(&h, Method::GET, "/api/network/status", None).await;
    assert_eq!(status, StatusCode::OK);
    // Default mock kernel has no network secret + no peer node, so the
    // surface must report a disabled, zeroed-out summary rather than
    // crashing on a missing `peer_node`.
    assert_eq!(body["enabled"], false, "{body}");
    assert_eq!(body["connected_peers"], 0);
    assert_eq!(body["total_peers"], 0);
    assert_eq!(body["pinned_peers"], 0);
    assert_eq!(body["node_id"], "");
    assert_eq!(body["listen_address"], "");
    assert!(body["identity_fingerprint"].is_null());

    // #3873 follow-up: dashboard reads `online` / `listen_addr` /
    // `peer_count` / `protocol_version`. Lock those names so a future
    // rename or removal can't silently re-break the network page
    // (NetworkPage.tsx referenced them while the daemon shipped only
    // their legacy aliases — the badge stayed "offline" for months).
    assert_eq!(body["online"], false);
    assert_eq!(body["listen_addr"], "");
    assert_eq!(body["peer_count"], 0);
    assert_eq!(body["protocol_version"], "ofp/1");
}

// ---------------------------------------------------------------------------
// /api/network/trusted-peers
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn network_trusted_peers_empty_when_no_peer_node() {
    // #3842: canonical `PaginatedResponse{items,total,offset,limit}` envelope.
    let h = boot();
    let (status, body) = json_request(&h, Method::GET, "/api/network/trusted-peers", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"], serde_json::json!([]));
    assert_eq!(body["total"], 0);
    assert_eq!(body["offset"], 0);
    assert!(body["limit"].is_null());
}

// ---------------------------------------------------------------------------
// /api/comms/topology
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn comms_topology_returns_nodes_and_edges_arrays() {
    let h = boot();
    let (status, body) = json_request(&h, Method::GET, "/api/comms/topology", None).await;
    assert_eq!(status, StatusCode::OK);
    // The dashboard relies on shape, not contents — both keys must be
    // arrays. Each `TopoNode` must carry the full set of fields the SPA
    // renders (id / name / state / model).
    let nodes = body["nodes"].as_array().expect("nodes array");
    assert!(body["edges"].is_array(), "edges must be an array: {body}");
    for n in nodes {
        for key in ["id", "name", "state", "model"] {
            assert!(n.get(key).is_some(), "node missing {key}: {n}");
        }
    }
}

// ---------------------------------------------------------------------------
// /api/comms/events
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn comms_events_returns_paginated_envelope_with_default_limit() {
    let h = boot();
    let (status, body) = json_request(&h, Method::GET, "/api/comms/events", None).await;
    assert_eq!(status, StatusCode::OK);
    // #3842 canonical envelope: PaginatedResponse{items,total,offset,limit}.
    let items = body
        .get("items")
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("events response must have items array: {body}"));
    let total = body.get("total").and_then(|v| v.as_u64()).expect("total");
    assert_eq!(total as usize, items.len(), "total must match items length");
    assert_eq!(body.get("offset").and_then(|v| v.as_u64()), Some(0));
    assert_eq!(body.get("limit").and_then(|v| v.as_u64()), Some(100));
}

#[tokio::test(flavor = "multi_thread")]
async fn comms_events_honours_explicit_limit_query() {
    let h = boot();
    let (status, body) = json_request(&h, Method::GET, "/api/comms/events?limit=5", None).await;
    assert_eq!(status, StatusCode::OK);
    let items = body
        .get("items")
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("events response must have items array: {body}"));
    // Empty kernel has no events; the limit cap simply must not over-yield.
    assert!(
        items.len() <= 5,
        "limit=5 must not be exceeded, got {} entries: {body}",
        items.len()
    );
    assert_eq!(body.get("limit").and_then(|v| v.as_u64()), Some(5));
}

// ---------------------------------------------------------------------------
// /api/comms/send — error paths only (success requires a live agent loop +
// real LLM creds, which the kernel-side handler `send_message` would invoke)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn comms_send_rejects_invalid_from_agent_id() {
    let h = boot();
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/comms/send",
        Some(serde_json::json!({
            "from_agent_id": "not-a-uuid",
            "to_agent_id": "00000000-0000-0000-0000-000000000000",
            "message": "hi",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        nested_error_message(&body)
            .to_lowercase()
            .contains("from_agent_id"),
        "{body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn comms_send_rejects_unknown_from_agent() {
    let h = boot();
    // Well-formed UUID but no such agent in the registry.
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/comms/send",
        Some(serde_json::json!({
            "from_agent_id": "00000000-0000-0000-0000-000000000001",
            "to_agent_id": "00000000-0000-0000-0000-000000000002",
            "message": "hi",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(
        nested_error_message(&body)
            .to_lowercase()
            .contains("source agent"),
        "{body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn comms_send_rejects_oversize_message() {
    // Construct two real agents so the size check runs after the existence
    // checks. We only need the IDs to round-trip — no real loop kicks off
    // because the handler short-circuits on the byte cap
    // (`validation::MAX_MESSAGE_BYTES`).
    let h = boot();

    // Register two minimal agents directly via the kernel registry. The
    // full LLM agent loop is never started, but the registry entries are
    // enough for the existence checks the handler performs before the
    // size guard short-circuits with 413.
    let agent_a = librefang_types::agent::AgentEntry {
        id: librefang_types::agent::AgentId::new(),
        name: "alice".into(),
        state: librefang_types::agent::AgentState::Running,
        ..Default::default()
    };
    let agent_b = librefang_types::agent::AgentEntry {
        id: librefang_types::agent::AgentId::new(),
        name: "bob".into(),
        state: librefang_types::agent::AgentState::Running,
        ..Default::default()
    };
    h.state
        .kernel
        .agent_registry()
        .register(agent_a.clone())
        .expect("register alice");
    h.state
        .kernel
        .agent_registry()
        .register(agent_b.clone())
        .expect("register bob");

    // One byte past the byte cap so `check_message_size` rejects with 413,
    // regardless of how the cap is tuned over time (byte-vs-char-cap audit).
    let oversize = "x".repeat(librefang_api::validation::MAX_MESSAGE_BYTES + 1);
    let request_body = serde_json::json!({
        "from_agent_id": agent_a.id.to_string(),
        "to_agent_id": agent_b.id.to_string(),
        "message": oversize,
    });
    let (anonymous_status, anonymous_body) = json_request(
        &h,
        Method::POST,
        "/api/comms/send",
        Some(request_body.clone()),
    )
    .await;
    assert_eq!(anonymous_status, StatusCode::FORBIDDEN, "{anonymous_body}");

    let (status, body) =
        json_request_as_owner(&h, Method::POST, "/api/comms/send", Some(request_body)).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
    assert!(
        flat_error_message(&body)
            .to_lowercase()
            .contains("too large"),
        "{body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn comms_send_refuses_impersonation_of_owned_from_agent() {
    // SECURITY (audit: comms-send-impersonation). An agent carrying a
    // non-empty `manifest.author` is owned by a named human. This bare
    // router has no auth middleware, so `comms_send` sees no
    // `AuthenticatedApiUser` — the loopback / `require_auth = false`
    // path. On that path the handler must refuse to mint a message from every
    // agent; production trusted no-auth mode injects a synthetic Owner rather
    // than reaching this unattributed branch.
    let h = boot();

    let owned = librefang_types::agent::AgentEntry {
        id: librefang_types::agent::AgentId::new(),
        name: "alice".into(),
        state: librefang_types::agent::AgentState::Running,
        manifest: librefang_types::agent::AgentManifest {
            author: "alice-the-human".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    let target = librefang_types::agent::AgentEntry {
        id: librefang_types::agent::AgentId::new(),
        name: "bob".into(),
        state: librefang_types::agent::AgentState::Running,
        ..Default::default()
    };
    h.state
        .kernel
        .agent_registry()
        .register(owned.clone())
        .expect("register owned");
    h.state
        .kernel
        .agent_registry()
        .register(target.clone())
        .expect("register target");

    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/comms/send",
        Some(serde_json::json!({
            "from_agent_id": owned.id.to_string(),
            "to_agent_id": target.id.to_string(),
            "message": "forged inter-agent message",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(
        nested_error_message(&body)
            .to_lowercase()
            .contains("own from_agent_id"),
        "{body}"
    );
}

/// SECURITY (audit: comms-send-no-audit-log).
///
/// `comms_send` records every successful cross-agent send into the
/// hash-chained audit log so a forensic reviewer can answer "which
/// agent talked to which?" with tamper-evident evidence. The kernel's
/// own `AgentMessage` row only records token usage for the receiver —
/// it does not capture the from→to relationship.
///
/// This test exercises the route end-to-end with an Owner extension matching
/// the production auth middleware and a local deterministic LLM endpoint.
/// The target is a real spawned agent, so the success-only audit branch must
/// execute rather than being optional based on ambient provider state.
#[tokio::test(flavor = "multi_thread")]
async fn comms_send_records_audit_entry_on_success() {
    let (llm_base_url, _llm_server) = spawn_mock_openai().await;
    let h = boot_with_builder(
        MockKernelBuilder::new().with_config(move |cfg| {
            cfg.default_model = DefaultModelConfig {
                provider: "librefang-network-test".to_string(),
                model: "mock-model".to_string(),
                base_url: Some(llm_base_url),
                ..Default::default()
            };
        }),
        |_| {},
    );

    let agent_a = librefang_types::agent::AgentEntry {
        id: librefang_types::agent::AgentId::new(),
        name: "alice".into(),
        state: librefang_types::agent::AgentState::Running,
        ..Default::default()
    };
    h.state
        .kernel
        .agent_registry()
        .register(agent_a.clone())
        .expect("register alice");
    let agent_b_id = h
        .state
        .kernel
        .spawn_agent_typed(AgentManifest {
            name: "bob".into(),
            ..Default::default()
        })
        .expect("spawn bob");

    // Snapshot the audit log before — there should be no `comms_send`
    // entries yet. We snapshot to avoid coupling to whatever the
    // kernel records during boot (e.g. retention-trim metadata).
    let before_count = h
        .state
        .kernel
        .audit()
        .recent(usize::MAX)
        .into_iter()
        .filter(|e| e.detail.contains("comms_send"))
        .count();
    assert_eq!(
        before_count, 0,
        "no comms_send audit entries should exist before the call"
    );

    let msg = "héllo 漢字 🎉"; // multi-byte; len-in-bytes != chars-count
    let expected_chars = msg.chars().count();
    let (status, body) = json_request_as_owner(
        &h,
        Method::POST,
        "/api/comms/send",
        Some(serde_json::json!({
            "from_agent_id": agent_a.id.to_string(),
            "to_agent_id": agent_b_id.to_string(),
            "message": msg,
        })),
    )
    .await;

    let comms_entries: Vec<_> = h
        .state
        .kernel
        .audit()
        .recent(usize::MAX)
        .into_iter()
        .filter(|e| e.detail.contains("comms_send"))
        .collect();

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ok"], true);
    assert_eq!(body["response"], MOCK_LLM_REPLY);
    assert_eq!(comms_entries.len(), 1, "exactly one audit row expected");

    let entry = &comms_entries[0];
    assert_eq!(entry.outcome, "ok");
    assert_eq!(entry.channel.as_deref(), Some("api"));
    let from_str = agent_a.id.to_string();
    let to_str = agent_b_id.to_string();
    assert!(entry.detail.contains(&from_str), "{}", entry.detail);
    assert!(entry.detail.contains(&to_str), "{}", entry.detail);
    assert!(
        entry.detail.contains(&format!("\"len\":{expected_chars}")),
        "audit detail must record character count ({expected_chars}, NOT byte count {}); got: {}",
        msg.len(),
        entry.detail,
    );
}
