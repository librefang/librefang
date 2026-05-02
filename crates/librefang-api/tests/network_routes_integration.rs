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

/// Boot a harness, allowing the caller to mutate the freshly-built
/// `AppState` (e.g. to install a peer registry on the kernel) before the
/// router clones it.
fn boot_with<F: FnOnce(&mut AppState)>(mutator: F) -> Harness {
    let mut test = TestAppState::with_builder(MockKernelBuilder::new());

    // Mutate the AppState in place. At this point the only outstanding Arc
    // ref is the one inside `test.state`, so `Arc::get_mut` is guaranteed
    // to succeed. We must do this BEFORE any `state.clone()` below.
    {
        let inner = Arc::get_mut(&mut test.state).expect("AppState must be uniquely owned at boot");
        mutator(inner);
    }

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
    let mut builder = Request::builder().method(method).uri(path);
    let body_bytes = match body {
        Some(v) => {
            builder = builder.header("content-type", "application/json");
            serde_json::to_vec(&v).unwrap()
        }
        None => Vec::new(),
    };
    let req = builder.body(Body::from(body_bytes)).unwrap();
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
        body["error"]
            .as_str()
            .unwrap_or("")
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
        body["error"]
            .as_str()
            .unwrap_or("")
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
        body["error"]
            .as_str()
            .unwrap_or("")
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
        body["error"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("source agent"),
        "{body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn comms_send_rejects_oversize_message() {
    // Construct two real agents so the size check runs after the existence
    // checks. We only need the IDs to round-trip — no real loop kicks off
    // because the handler short-circuits on the 64KB cap.
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

    let oversize = "x".repeat(64 * 1024 + 1);
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/comms/send",
        Some(serde_json::json!({
            "from_agent_id": agent_a.id.to_string(),
            "to_agent_id": agent_b.id.to_string(),
            "message": oversize,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("too large"),
        "{body}"
    );
}
