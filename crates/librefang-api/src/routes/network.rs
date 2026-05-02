//! Network, peer, A2A protocol, and inter-agent communication handlers.

use super::AppState;

/// Build routes for the network/peer/A2A/communication domain.
pub fn router() -> axum::Router<std::sync::Arc<AppState>> {
    axum::Router::new()
        .route("/peers", axum::routing::get(list_peers))
        .route("/peers/{id}", axum::routing::get(get_peer))
        .route("/network/status", axum::routing::get(network_status))
        .route(
            "/network/trusted-peers",
            axum::routing::get(network_trusted_peers),
        )
        .route("/comms/topology", axum::routing::get(comms_topology))
        .route("/comms/events", axum::routing::get(comms_events))
        .route(
            "/comms/events/stream",
            axum::routing::get(comms_events_stream),
        )
        .route("/comms/send", axum::routing::post(comms_send))
        .route("/comms/task", axum::routing::post(comms_task))
        // Internal management A2A endpoints (versioned API)
        .route(
            "/a2a/agents",
            axum::routing::get(a2a_list_external_agents),
        )
        .route(
            "/a2a/agents/{id}",
            axum::routing::get(a2a_get_external_agent),
        )
        .route(
            "/a2a/discover",
            axum::routing::post(a2a_discover_external),
        )
        .route("/a2a/send", axum::routing::post(a2a_send_external))
        .route(
            "/a2a/tasks/{id}/status",
            axum::routing::get(a2a_external_task_status),
        )
        // Bug #3786: operator must explicitly approve a discovered agent before
        // it can receive tasks. POST /api/a2a/agents/{url_encoded}/approve
        // promotes the pending entry into the kernel's trusted list.
        .route(
            "/a2a/agents/{id}/approve",
            axum::routing::post(a2a_approve_external),
        )
}

/// Build protocol-level A2A routes (not versioned, mounted at the root path).
pub fn protocol_router() -> axum::Router<std::sync::Arc<AppState>> {
    axum::Router::new()
        .route(
            "/.well-known/agent.json",
            axum::routing::get(a2a_agent_card),
        )
        .route("/a2a/agents", axum::routing::get(a2a_list_agents))
        .route("/a2a/tasks/send", axum::routing::post(a2a_send_task))
        .route("/a2a/tasks/{id}", axum::routing::get(a2a_get_task))
        .route(
            "/a2a/tasks/{id}/cancel",
            axum::routing::post(a2a_cancel_task),
        )
}
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use librefang_runtime::kernel_handle::KernelHandle;
use librefang_runtime::tool_runner::builtin_tool_definitions;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;

use crate::types::ApiErrorResponse;
// ---------------------------------------------------------------------------
// Peer endpoints
// ---------------------------------------------------------------------------

/// GET /api/peers — List known OFP peers.
#[utoipa::path(
    get,
    path = "/api/peers",
    tag = "network",
    responses(
        (status = 200, description = "List known OFP peers", body = crate::types::JsonObject)
    )
)]
pub async fn list_peers(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Peers are tracked in the wire module's PeerRegistry, owned by the kernel
    // and lazily initialized when the OFP peer node starts. Read it live on every
    // request — caching at boot would return a stale (or empty) snapshot if the
    // OFP node initialized after AppState was constructed (#3644).
    //
    // Envelope is the canonical `PaginatedResponse{items,total,offset,limit}`
    // shape used by `/api/agents` (#3842). All peers are returned in a single
    // page — the registry is in-memory and small — so `offset=0` and
    // `limit=None` always.
    let items: Vec<serde_json::Value> =
        if let Some(peer_registry) = state.kernel.peer_registry_ref() {
            peer_registry
                .all_peers()
                .iter()
                .map(|p| {
                    serde_json::json!({
                        "node_id": p.node_id,
                        "node_name": p.node_name,
                        "address": p.address.to_string(),
                        "state": format!("{:?}", p.state),
                        "agents": p.agents.iter().map(|a| serde_json::json!({
                            "id": a.id,
                            "name": a.name,
                        })).collect::<Vec<_>>(),
                        "connected_at": p.connected_at.to_rfc3339(),
                        "protocol_version": p.protocol_version,
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
    let total = items.len();
    Json(crate::types::PaginatedResponse {
        items,
        total,
        offset: 0,
        limit: None,
    })
}

/// GET /api/peers/{id} — Get a single peer by node ID.
#[utoipa::path(
    get,
    path = "/api/peers/{id}",
    tag = "network",
    params(("id" = String, Path, description = "Peer node ID")),
    responses(
        (status = 200, description = "Peer details", body = crate::types::JsonObject),
        (status = 404, description = "Peer not found")
    )
)]
pub async fn get_peer(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let registry = match state.kernel.peer_registry_ref() {
        Some(r) => r,
        None => {
            return ApiErrorResponse::not_found("Peer networking is not enabled").into_json_tuple();
        }
    };

    match registry.get_peer(&id) {
        Some(p) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "node_id": p.node_id,
                "node_name": p.node_name,
                "address": p.address.to_string(),
                "state": format!("{:?}", p.state),
                "agents": p.agents.iter().map(|a| serde_json::json!({
                    "id": a.id,
                    "name": a.name,
                })).collect::<Vec<_>>(),
                "connected_at": p.connected_at.to_rfc3339(),
                "protocol_version": p.protocol_version,
            })),
        ),
        None => ApiErrorResponse::not_found("Peer not found").into_json_tuple(),
    }
}

/// GET /api/network/status — OFP network status summary.
#[utoipa::path(
    get,
    path = "/api/network/status",
    tag = "network",
    responses(
        (status = 200, description = "OFP network status summary", body = crate::types::JsonObject)
    )
)]
pub async fn network_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let cfg = state.kernel.config_ref();
    let enabled = cfg.network_enabled && !cfg.network.shared_secret.is_empty();
    drop(cfg);

    let (node_id, listen_address, connected_peers, total_peers, identity_fingerprint, pinned_peers) =
        if let Some(peer_node) = state.kernel.peer_node_ref() {
            let registry = peer_node.registry();
            (
                peer_node.node_id().to_string(),
                peer_node.local_addr().to_string(),
                registry.connected_count(),
                registry.total_count(),
                peer_node.identity_fingerprint(),
                peer_node.pinned_peer_count(),
            )
        } else {
            (String::new(), String::new(), 0, 0, None, 0)
        };

    // SECURITY (#3873): Surface this node's Ed25519 identity fingerprint
    // and the count of TOFU-pinned peers so operators can verify their
    // own identity is loaded (not silently HMAC-only) and watch the pin
    // map populate as peers are encountered. The fingerprint is the
    // out-of-band-comparable value — share it on a side channel so a
    // remote operator can check the value their kernel pinned.
    Json(serde_json::json!({
        "enabled": enabled,
        "node_id": node_id,
        "listen_address": listen_address,
        "connected_peers": connected_peers,
        "total_peers": total_peers,
        "identity_fingerprint": identity_fingerprint,
        "pinned_peers": pinned_peers,
    }))
}

/// SECURITY (#3873): GET /api/network/trusted-peers — list every TOFU-pinned
/// peer this node will accept under each `node_id`. Operators read this to
/// verify what their daemon trusts and out-of-band-compare fingerprints
/// with remote operators before federating.
#[utoipa::path(
    get,
    path = "/api/network/trusted-peers",
    tag = "network",
    responses(
        (status = 200, description = "List TOFU-pinned peers", body = crate::types::JsonObject)
    )
)]
pub async fn network_trusted_peers(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // #3842: canonical `PaginatedResponse{items,total,offset,limit}` envelope.
    // The pin store is in-memory and small, so all entries are returned in a
    // single page (`offset=0`, `limit=None`).
    let items: Vec<serde_json::Value> = match state.kernel.peer_node_ref() {
        Some(peer_node) => peer_node
            .list_pinned_peers()
            .into_iter()
            .map(|(node_id, public_key, fingerprint)| {
                serde_json::json!({
                    "node_id": node_id,
                    "public_key": public_key,
                    "fingerprint": fingerprint,
                })
            })
            .collect(),
        None => Vec::new(),
    };
    let total = items.len();
    Json(crate::types::PaginatedResponse {
        items,
        total,
        offset: 0,
        limit: None,
    })
}

#[utoipa::path(
    get,
    path = "/.well-known/agent.json",
    tag = "a2a",
    responses(
        (status = 200, description = "Get the A2A agent card", body = crate::types::JsonObject)
    )
)]
pub async fn a2a_agent_card(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let agents = state.kernel.agent_registry().list();
    let cfg = state.kernel.config_ref();
    let base_url = format!("http://{}", cfg.api_listen);

    // Use service-level A2A config for the well-known card when available.
    let (service_name, service_description) = if let Some(ref a2a_cfg) = cfg.a2a {
        let name = if a2a_cfg.name.is_empty() {
            "LibreFang Agent OS".to_string()
        } else {
            a2a_cfg.name.clone()
        };
        (name, a2a_cfg.description.clone())
    } else {
        ("LibreFang Agent OS".to_string(), String::new())
    };
    drop(cfg);

    // Aggregate skills from ALL agents.
    let skills: Vec<librefang_runtime::a2a::AgentSkill> = agents
        .iter()
        .flat_map(|entry| {
            librefang_runtime::a2a::build_agent_card(&entry.manifest, &base_url).skills
        })
        .collect();

    let card = librefang_runtime::a2a::AgentCard {
        name: service_name,
        description: service_description,
        url: format!("{base_url}/a2a"),
        version: librefang_types::VERSION.to_string(),
        capabilities: librefang_runtime::a2a::AgentCapabilities {
            streaming: true,
            push_notifications: false,
            state_transition_history: true,
        },
        skills,
        default_input_modes: vec!["text".to_string()],
        default_output_modes: vec!["text".to_string()],
    };

    (
        StatusCode::OK,
        Json(serde_json::to_value(&card).unwrap_or_default()),
    )
}

/// GET /a2a/agents — List all A2A agent cards.
#[utoipa::path(
    get,
    path = "/a2a/agents",
    tag = "a2a",
    responses(
        (status = 200, description = "List all A2A agent cards", body = crate::types::JsonObject)
    )
)]
pub async fn a2a_list_agents(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let agents = state.kernel.agent_registry().list();
    let base_url = format!("http://{}", state.kernel.config_ref().api_listen);

    let cards: Vec<serde_json::Value> = agents
        .iter()
        .map(|entry| {
            let card = librefang_runtime::a2a::build_agent_card(&entry.manifest, &base_url);
            serde_json::to_value(&card).unwrap_or_default()
        })
        .collect();

    let total = cards.len();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "agents": cards,
            "total": total,
        })),
    )
}

/// POST /a2a/tasks/send — Submit a task to an agent via A2A.
#[utoipa::path(
    post,
    path = "/a2a/tasks/send",
    tag = "a2a",
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Submit a task to an agent via A2A", body = crate::types::JsonObject)
    )
)]
pub async fn a2a_send_task(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<serde_json::Value>,
) -> impl IntoResponse {
    // Extract message text from A2A format
    let message_text = request["params"]["message"]["parts"]
        .as_array()
        .and_then(|parts| {
            parts.iter().find_map(|p| {
                if p["type"].as_str() == Some("text") {
                    p["text"].as_str().map(String::from)
                } else {
                    None
                }
            })
        })
        .unwrap_or_else(|| "No message provided".to_string());

    // Extract caller identity from A2A header (for audit / ACL).
    let caller_a2a_agent_id = headers
        .get("x-a2a-agent-id")
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    // Require an explicit target agent — refuse to silently dispatch to agents[0].
    let target_agent_id_str = request["params"]["agentId"]
        .as_str()
        .or_else(|| request["agent_id"].as_str())
        .map(String::from);

    let target_agent_id_str = match target_agent_id_str {
        Some(s) => s,
        None => {
            return ApiErrorResponse::bad_request(
                "Missing required field: params.agentId (or agent_id)",
            )
            .into_json_tuple();
        }
    };

    // Parse and validate the target agent UUID.
    let target_agent_id: librefang_types::agent::AgentId = match target_agent_id_str.parse() {
        Ok(id) => id,
        Err(_) => {
            return ApiErrorResponse::bad_request(format!(
                "Invalid agent ID: {target_agent_id_str}"
            ))
            .into_json_tuple();
        }
    };

    // Look up the agent and enforce state checks.
    let agent_entry = match state.kernel.agent_registry().get(target_agent_id) {
        Some(e) => e,
        None => {
            return ApiErrorResponse::not_found(format!("Agent not found: {target_agent_id_str}"))
                .into_json_tuple();
        }
    };

    if matches!(
        agent_entry.state,
        librefang_types::agent::AgentState::Suspended
            | librefang_types::agent::AgentState::Terminated
    ) {
        return ApiErrorResponse::bad_request(format!(
            "Agent {} is {:?} and cannot accept tasks",
            target_agent_id_str, agent_entry.state
        ))
        .into_json_tuple();
    }

    let task_id = uuid::Uuid::new_v4().to_string();
    let session_id = request["params"]["sessionId"].as_str().map(String::from);

    // Create the task in the store as Working, recording dispatch target and caller.
    let task = librefang_runtime::a2a::A2aTask {
        id: task_id.clone(),
        session_id: session_id.clone(),
        status: librefang_runtime::a2a::A2aTaskStatus::Working.into(),
        messages: vec![librefang_runtime::a2a::A2aMessage {
            role: "user".to_string(),
            parts: vec![librefang_runtime::a2a::A2aPart::Text {
                text: message_text.clone(),
            }],
        }],
        artifacts: vec![],
        agent_id: Some(target_agent_id_str),
        caller_a2a_agent_id,
    };
    state.kernel.a2a_tasks().insert(task);

    // Send message to the validated target agent.
    match state
        .kernel
        .send_message(agent_entry.id, &message_text)
        .await
    {
        Ok(result) => {
            let response_msg = librefang_runtime::a2a::A2aMessage {
                role: "agent".to_string(),
                parts: vec![librefang_runtime::a2a::A2aPart::Text {
                    text: result.response,
                }],
            };
            state
                .kernel
                .a2a_tasks()
                .complete(&task_id, response_msg, vec![]);
            match state.kernel.a2a_tasks().get(&task_id) {
                Some(completed_task) => (
                    StatusCode::OK,
                    Json(serde_json::to_value(&completed_task).unwrap_or_default()),
                ),
                None => ApiErrorResponse::internal("Task disappeared after completion")
                    .into_json_tuple(),
            }
        }
        Err(e) => {
            let error_msg = librefang_runtime::a2a::A2aMessage {
                role: "agent".to_string(),
                parts: vec![librefang_runtime::a2a::A2aPart::Text {
                    text: format!("Error: {e}"),
                }],
            };
            state.kernel.a2a_tasks().fail(&task_id, error_msg);
            match state.kernel.a2a_tasks().get(&task_id) {
                Some(failed_task) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::to_value(&failed_task).unwrap_or_default()),
                ),
                None => ApiErrorResponse::internal(format!("Agent error: {e}")).into_json_tuple(),
            }
        }
    }
}

/// GET /a2a/tasks/{id} — Get task status from the task store.
#[utoipa::path(
    get,
    path = "/a2a/tasks/{id}",
    tag = "a2a",
    params(
        ("id" = String, Path, description = "Id"),
    ),
    responses(
        (status = 200, description = "Get A2A task status", body = crate::types::JsonObject)
    )
)]
pub async fn a2a_get_task(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    match state.kernel.a2a_tasks().get(&task_id) {
        Some(task) => (
            StatusCode::OK,
            Json(serde_json::to_value(&task).unwrap_or_default()),
        ),
        None => {
            ApiErrorResponse::not_found(format!("Task '{}' not found", task_id)).into_json_tuple()
        }
    }
}

/// POST /a2a/tasks/{id}/cancel — Cancel a tracked task.
#[utoipa::path(
    post,
    path = "/a2a/tasks/{id}/cancel",
    tag = "a2a",
    params(
        ("id" = String, Path, description = "Id"),
    ),
    responses(
        (status = 200, description = "Cancel a tracked A2A task", body = crate::types::JsonObject)
    )
)]
pub async fn a2a_cancel_task(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    if state.kernel.a2a_tasks().cancel(&task_id) {
        match state.kernel.a2a_tasks().get(&task_id) {
            Some(task) => (
                StatusCode::OK,
                Json(serde_json::to_value(&task).unwrap_or_default()),
            ),
            None => {
                ApiErrorResponse::internal("Task disappeared after cancellation").into_json_tuple()
            }
        }
    } else {
        ApiErrorResponse::not_found(format!("Task '{}' not found", task_id)).into_json_tuple()
    }
}

// ── A2A Management Endpoints (outbound) ─────────────────────────────────

/// GET /api/a2a/agents — List discovered external A2A agents.
///
/// Returns both `trusted` agents (approved and able to receive tasks) and
/// `pending` agents (discovered but not yet approved by the operator).
#[utoipa::path(
    get,
    path = "/api/a2a/agents",
    tag = "a2a",
    responses(
        (status = 200, description = "List discovered external A2A agents", body = crate::types::JsonObject)
    )
)]
pub async fn a2a_list_external_agents(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let agents = state
        .kernel
        .a2a_agents()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let mut items: Vec<serde_json::Value> = agents
        .iter()
        .map(|(_, card)| {
            serde_json::json!({
                "name": card.name,
                "url": card.url,
                "description": card.description,
                "skills": card.skills,
                "version": card.version,
                "status": "trusted",
            })
        })
        .collect();
    // Include pending (unapproved) agents so the operator can see and approve them.
    for entry in state.pending_a2a_agents.iter() {
        let card = entry.value();
        items.push(serde_json::json!({
            "name": card.name,
            "url": card.url,
            "description": card.description,
            "skills": card.skills,
            "version": card.version,
            "status": "pending",
        }));
    }
    Json(serde_json::json!({"agents": items, "total": items.len()}))
}

/// Check whether a URL is safe to fetch (not targeting internal/private networks).
/// Returns `Ok(())` if the URL is safe, or `Err(message)` describing the problem.
///
/// `allowed_hosts` entries may be CIDRs (e.g. `"10.0.0.0/8"`), glob hostname
/// patterns (e.g. `"*.internal.example.com"`), or literal IPs/hostnames.
/// Cloud metadata ranges (`169.254.0.0/16`, `100.64.0.0/10`) remain blocked
/// unconditionally regardless of allowlist entries.
fn is_url_safe_for_ssrf(raw_url: &str, allowed_hosts: &[String]) -> Result<(), String> {
    let parsed = url::Url::parse(raw_url).map_err(|e| format!("Invalid URL: {e}"))?;

    // Only allow http and https schemes
    match parsed.scheme() {
        "http" | "https" => {}
        other => return Err(format!("Unsupported URL scheme: {other}")),
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| "URL has no host".to_string())?;

    // Block localhost by hostname
    if host.eq_ignore_ascii_case("localhost") {
        return Err("Requests to localhost are not allowed".to_string());
    }

    // Try to parse the host as an IP address directly, or resolve the hostname
    let addrs: Vec<IpAddr> = if let Ok(ip) = host.parse::<IpAddr>() {
        vec![ip]
    } else {
        // Resolve hostname — use port 80 as a dummy for resolution
        let socket_addr = format!("{host}:80");
        match std::net::ToSocketAddrs::to_socket_addrs(&socket_addr.as_str()) {
            Ok(iter) => iter.map(|sa| sa.ip()).collect(),
            Err(_) => {
                // If resolution fails, we still block — don't allow unresolvable hosts
                return Err(format!("Cannot resolve host: {host}"));
            }
        }
    };

    for ip in &addrs {
        // Canonicalise IPv4-mapped IPv6 (::ffff:X.X.X.X) before any safety
        // check. The OS transparently connects these to the embedded IPv4
        // target, so leaving them as IPv6 lets an attacker reach loopback /
        // private / cloud-metadata IPs via the v6 form (e.g.
        // [::ffff:169.254.169.254]) which the v6-only branches of
        // is_private_ip / is_cloud_metadata_ip do not recognise.
        let canonical = canonical_ip(ip);
        if is_private_ip(&canonical) {
            // Cloud metadata ranges are unconditionally blocked even when
            // the host appears in the allowlist.
            if !is_cloud_metadata_ip(&canonical) && is_host_allowed(host, &canonical, allowed_hosts)
            {
                continue;
            }
            return Err(format!(
                "Requests to private/internal IP addresses are not allowed ({canonical})"
            ));
        }
    }

    Ok(())
}

/// Unwrap IPv4-mapped IPv6 (`::ffff:X.X.X.X`) to its IPv4 form. All other
/// addresses are returned unchanged.
fn canonical_ip(ip: &IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(*v6),
        },
        IpAddr::V4(_) => *ip,
    }
}

/// Returns true if the IP is in a cloud metadata / CGNAT range that must be
/// blocked unconditionally (`169.254.0.0/16` or `100.64.0.0/10`).
fn is_cloud_metadata_ip(ip: &IpAddr) -> bool {
    match canonical_ip(ip) {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            (o[0] == 169 && o[1] == 254) || (o[0] == 100 && (o[1] & 0xC0) == 64)
        }
        IpAddr::V6(_) => false,
    }
}

/// Check whether a hostname or resolved IP matches any entry in `allowed_hosts`.
///
/// Entry formats:
/// - `"10.0.0.0/8"`             — CIDR; matched against the resolved `ip`
/// - `"*.internal.example.com"` — glob prefix wildcard; matched against `hostname`
/// - `"10.1.2.3"` / `"svc.local"` — literal IP or hostname exact match
fn is_host_allowed(hostname: &str, ip: &IpAddr, allowed_hosts: &[String]) -> bool {
    for entry in allowed_hosts {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        if entry.contains('/') {
            if cidr_contains(entry, ip).unwrap_or(false) {
                return true;
            }
            continue;
        }
        if let Some(suffix) = entry.strip_prefix('*') {
            if suffix.is_empty() {
                continue; // reject bare "*" — too broad
            }
            if hostname.ends_with(suffix) {
                return true;
            }
            continue;
        }
        if let Ok(entry_ip) = entry.parse::<IpAddr>() {
            if entry_ip == *ip {
                return true;
            }
            continue;
        }
        if entry.eq_ignore_ascii_case(hostname) {
            return true;
        }
    }
    false
}

/// Check if `ip` falls within the CIDR range `cidr` (e.g. `"10.0.0.0/8"`).
fn cidr_contains(cidr: &str, ip: &IpAddr) -> Result<bool, ()> {
    let (addr_str, prefix_str) = cidr.split_once('/').ok_or(())?;
    let prefix_len: u32 = prefix_str.parse().map_err(|_| ())?;
    match (addr_str.parse::<IpAddr>(), ip) {
        (Ok(IpAddr::V4(net_addr)), IpAddr::V4(v4)) => {
            if prefix_len > 32 {
                return Err(());
            }
            let mask = if prefix_len == 0 {
                0u32
            } else {
                !0u32 << (32 - prefix_len)
            };
            Ok((u32::from_be_bytes(net_addr.octets()) & mask)
                == (u32::from_be_bytes(v4.octets()) & mask))
        }
        (Ok(IpAddr::V6(net_addr)), IpAddr::V6(v6)) => {
            if prefix_len > 128 {
                return Err(());
            }
            let net_bits = u128::from_be_bytes(net_addr.octets());
            let ip_bits = u128::from_be_bytes(v6.octets());
            let mask = if prefix_len == 0 {
                0u128
            } else {
                !0u128 << (128 - prefix_len)
            };
            Ok((net_bits & mask) == (ip_bits & mask))
        }
        _ => Ok(false),
    }
}

/// Returns true if the IP address is in a private, loopback, link-local, or
/// otherwise internal range that should not be reachable from user-supplied URLs.
fn is_private_ip(ip: &IpAddr) -> bool {
    match canonical_ip(ip) {
        IpAddr::V4(v4) => {
            v4.is_loopback()              // 127.0.0.0/8
                || v4.is_private()         // 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
                || v4.is_link_local()      // 169.254.0.0/16 (cloud metadata)
                || v4.is_broadcast()       // 255.255.255.255
                || v4.is_unspecified()     // 0.0.0.0
                || v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64 // 100.64.0.0/10 (CGNAT)
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()              // ::1
                || v6.is_unspecified()     // ::
                || (v6.segments()[0] & 0xfe00) == 0xfc00 // fc00::/7 (unique local)
                || (v6.segments()[0] & 0xffc0) == 0xfe80 // fe80::/10 (link-local)
        }
    }
}

/// GET /api/a2a/agents/{id} — Get a specific external A2A agent by index, URL, or name.
#[utoipa::path(
    get,
    path = "/api/a2a/agents/{id}",
    tag = "a2a",
    params(
        ("id" = String, Path, description = "Id"),
    ),
    responses(
        (status = 200, description = "Get a specific external A2A agent", body = crate::types::JsonObject)
    )
)]
pub async fn a2a_get_external_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agents = state
        .kernel
        .a2a_agents()
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let make_response = |(_, card): &(String, librefang_runtime::a2a::AgentCard)| {
        serde_json::json!({
            "name": card.name,
            "url": card.url,
            "description": card.description,
            "skills": card.skills,
            "version": card.version,
        })
    };

    // Try by index first
    if let Ok(idx) = id.parse::<usize>() {
        if let Some(entry) = agents.get(idx) {
            return (StatusCode::OK, Json(make_response(entry)));
        }
    }

    // Try by URL match
    if let Some(entry) = agents.iter().find(|(_, c)| c.url == id) {
        return (StatusCode::OK, Json(make_response(entry)));
    }

    // Try by agent name
    if let Some(entry) = agents.iter().find(|(_, c)| c.name == id) {
        return (StatusCode::OK, Json(make_response(entry)));
    }

    ApiErrorResponse::not_found(format!("A2A agent '{}' not found", id)).into_json_tuple()
}

/// POST /api/a2a/discover — Discover a new external A2A agent by URL.
#[utoipa::path(
    post,
    path = "/api/a2a/discover",
    tag = "a2a",
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Discover an external A2A agent by URL", body = crate::types::JsonObject)
    )
)]
pub async fn a2a_discover_external(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let raw_url = match body["url"].as_str() {
        Some(u) => u.to_string(),
        None => return ApiErrorResponse::bad_request("Missing 'url' field").into_json_tuple(),
    };
    // Canonicalize once at the boundary so the pending key, the trust-list
    // key inserted on approve, and every later trust-gate comparison all
    // share the same string. Otherwise `https://x.com/` and `https://x.com`
    // would split into two pending entries and the gate at /api/a2a/send
    // would reject whichever variant the caller didn't approve. (#3786)
    let url = match librefang_runtime::a2a::canonicalize_a2a_url(&raw_url) {
        Some(u) => u,
        None => {
            return ApiErrorResponse::bad_request("URL is not a valid http(s) URL with a host")
                .into_json_tuple();
        }
    };

    // SSRF protection: validate URL before making any outbound request
    let ssrf_allowed = state
        .kernel
        .config_snapshot()
        .web
        .fetch
        .ssrf_allowed_hosts
        .clone();
    if let Err(reason) = is_url_safe_for_ssrf(&url, &ssrf_allowed) {
        return ApiErrorResponse::bad_request(reason).into_json_tuple();
    }

    // Thread allowlist into client so redirects are re-validated against the same SSRF policy (#3782).
    let client = librefang_runtime::a2a::A2aClient::new_with_allowlist(ssrf_allowed);
    match client.discover(&url).await {
        Ok(card) => {
            // SECURITY (Bug #3786): Warn that we have no cryptographic proof
            // the remote agent is who it claims to be. Verification relies
            // solely on the operator reviewing the card before approving.
            tracing::warn!(
                url = %url,
                agent_name = %card.name,
                "A2A agent discovered without cryptographic verification. \
                 The returned AgentCard has NOT been signed or authenticated. \
                 Review the card carefully before approving (POST /api/a2a/agents/{{url}}/approve)."
            );

            // SECURITY (Bug #3786): Check for name collision with already-trusted agents.
            {
                let agents = state
                    .kernel
                    .a2a_agents()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                // A different URL claiming the same name as an existing trusted agent is a
                // potential impersonation attempt. Reject it to prevent confusion.
                if let Some((existing_url, _)) = agents
                    .iter()
                    .find(|(u, c)| c.name == card.name && u != &url)
                {
                    return (
                        StatusCode::CONFLICT,
                        Json(serde_json::json!({
                            "error": format!(
                                "An agent named '{}' is already trusted from a different URL ('{}').",
                                card.name, existing_url
                            ),
                            "hint": "Approve the existing agent or remove it before registering a new one with the same name."
                        })),
                    );
                }
            }
            // Also check pending agents for the same name collision.
            if let Some(entry) = state
                .pending_a2a_agents
                .iter()
                .find(|e| e.value().name == card.name && e.key() != &url)
            {
                return (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({
                        "error": format!(
                            "A pending agent named '{}' was already discovered from a different URL ('{}').",
                            card.name, entry.key()
                        ),
                        "hint": "Approve or remove the existing pending entry first."
                    })),
                );
            }

            let card_json = serde_json::to_value(&card).unwrap_or_default();

            // SECURITY (Bug #3483): cap the pending registry to prevent unbounded
            // growth. Updating an existing pending entry (same URL) is always
            // allowed; only NEW URLs are blocked once the cap is reached.
            const MAX_PENDING_A2A_AGENTS: usize = 1024;
            if !state.pending_a2a_agents.contains_key(&url)
                && state.pending_a2a_agents.len() >= MAX_PENDING_A2A_AGENTS
            {
                return (
                    StatusCode::TOO_MANY_REQUESTS,
                    Json(serde_json::json!({
                        "error": format!(
                            "Pending A2A registry full ({} entries). Approve or remove existing entries first.",
                            MAX_PENDING_A2A_AGENTS
                        )
                    })),
                );
            }

            // SECURITY (Bug #3786): Store in the PENDING list, not the trusted kernel
            // list. The agent cannot receive tasks until the operator explicitly
            // approves it via POST /api/a2a/agents/{url}/approve.
            let card_name = card.name.clone();
            state.pending_a2a_agents.insert(url.clone(), card);

            // Bug #3786: audit every discovery so silent agent enumeration is detectable.
            let audit_detail = format!("url={url} name={card_name}");
            state.kernel.audit().record_with_context(
                "system",
                librefang_runtime::audit::AuditAction::A2aDiscovered,
                &audit_detail,
                "pending",
                None,
                Some("api".to_string()),
            );

            (
                StatusCode::ACCEPTED,
                Json(serde_json::json!({
                    "url": url,
                    "status": "pending",
                    "agent": card_json,
                    "message": "Agent discovered and placed in pending state. \
                                An operator must approve it before it can receive tasks. \
                                Use POST /api/a2a/agents/{url}/approve to trust this agent.",
                })),
            )
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"error": e})),
        ),
    }
}

/// POST /api/a2a/send — Send a task to an external A2A agent.
#[utoipa::path(
    post,
    path = "/api/a2a/send",
    tag = "a2a",
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Send a task to an external A2A agent", body = crate::types::JsonObject)
    )
)]
pub async fn a2a_send_external(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let raw_url = match body["url"].as_str() {
        Some(u) => u.to_string(),
        None => return ApiErrorResponse::bad_request("Missing 'url' field").into_json_tuple(),
    };
    // Canonicalize before any trust-list comparison so case / port /
    // trailing-slash variants all match the form stored at approve time.
    let url = match librefang_runtime::a2a::canonicalize_a2a_url(&raw_url) {
        Some(u) => u,
        None => {
            return ApiErrorResponse::bad_request("URL is not a valid http(s) URL with a host")
                .into_json_tuple();
        }
    };
    let message = match body["message"].as_str() {
        Some(m) => m.to_string(),
        None => return ApiErrorResponse::bad_request("Missing 'message' field").into_json_tuple(),
    };
    let session_id = body["session_id"].as_str();

    // SECURITY (Bug #3786): Reject sends to agents that are still pending approval.
    if state.pending_a2a_agents.contains_key(&url) {
        return ApiErrorResponse::bad_request(
            "This agent is pending operator approval and cannot receive tasks. \
             Use POST /api/a2a/agents/{url}/approve to trust it first.",
        )
        .into_json_tuple();
    }

    // SECURITY (Bug #3786): Operator-approved trust gate. Without this check
    // any caller with a valid API key can dispatch tasks to arbitrary URLs as
    // long as SSRF allows them — defeating the whole approval workflow. Only
    // URLs that have been explicitly approved (via /api/a2a/agents/{id}/approve
    // or seeded via static config) may receive tasks.
    {
        let trusted = state
            .kernel
            .a2a_agents()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if !trusted.iter().any(|(u, _)| u == &url) {
            return ApiErrorResponse::bad_request(
                "Target URL is not a trusted A2A agent. \
                 Discover and approve it first via POST /api/a2a/discover \
                 followed by POST /api/a2a/agents/{url}/approve.",
            )
            .into_json_tuple();
        }
    }

    // SSRF protection: validate URL before making any outbound request
    let ssrf_allowed = state
        .kernel
        .config_snapshot()
        .web
        .fetch
        .ssrf_allowed_hosts
        .clone();
    if let Err(reason) = is_url_safe_for_ssrf(&url, &ssrf_allowed) {
        return ApiErrorResponse::bad_request(reason).into_json_tuple();
    }

    // Thread allowlist into client so redirects are re-validated against the same SSRF policy (#3782).
    let client = librefang_runtime::a2a::A2aClient::new_with_allowlist(ssrf_allowed);
    match client.send_task(&url, &message, session_id).await {
        Ok(task) => (
            StatusCode::OK,
            Json(serde_json::to_value(&task).unwrap_or_default()),
        ),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"error": e})),
        ),
    }
}

/// GET /api/a2a/tasks/{id}/status — Get task status from an external A2A agent.
#[utoipa::path(
    get,
    path = "/api/a2a/tasks/{id}/status",
    tag = "a2a",
    params(
        ("id" = String, Path, description = "Id"),
        ("url" = String, Query, description = "URL of the external A2A agent"),
    ),
    responses(
        (status = 200, description = "Get external A2A task status", body = crate::types::JsonObject)
    )
)]
pub async fn a2a_external_task_status(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let raw_url = match params.get("url") {
        Some(u) => u.clone(),
        None => {
            return ApiErrorResponse::bad_request("Missing 'url' query parameter").into_json_tuple()
        }
    };
    // Canonicalize before the trust gate so cosmetic variants on the query
    // string don't split the comparison from the form stored at approve.
    let url = match librefang_runtime::a2a::canonicalize_a2a_url(&raw_url) {
        Some(u) => u,
        None => {
            return ApiErrorResponse::bad_request("URL is not a valid http(s) URL with a host")
                .into_json_tuple();
        }
    };

    // SECURITY (Bug #3786): trust gate — only query task status from
    // operator-approved A2A agents. Otherwise this endpoint doubles as an
    // SSRF probe surface against any URL the global SSRF allowlist accepts.
    {
        let trusted = state
            .kernel
            .a2a_agents()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if !trusted.iter().any(|(u, _)| u == &url) {
            return ApiErrorResponse::bad_request(
                "Target URL is not a trusted A2A agent. \
                 Discover and approve it first via POST /api/a2a/discover.",
            )
            .into_json_tuple();
        }
    }

    // SSRF protection: validate URL before making any outbound request
    let ssrf_allowed = state
        .kernel
        .config_snapshot()
        .web
        .fetch
        .ssrf_allowed_hosts
        .clone();
    if let Err(reason) = is_url_safe_for_ssrf(&url, &ssrf_allowed) {
        return ApiErrorResponse::bad_request(reason).into_json_tuple();
    }

    // Thread allowlist into client so redirects are re-validated against the same SSRF policy (#3782).
    let client = librefang_runtime::a2a::A2aClient::new_with_allowlist(ssrf_allowed);
    match client.get_task(&url, &task_id).await {
        Ok(task) => (
            StatusCode::OK,
            Json(serde_json::to_value(&task).unwrap_or_default()),
        ),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"error": e})),
        ),
    }
}

/// POST /api/a2a/agents/{id}/approve — Approve a pending external A2A agent.
///
/// Promotes the agent from the pending list into the kernel's trusted external-agent
/// list, allowing it to receive tasks via `/api/a2a/send`. The `{id}` path segment
/// should be the URL-encoded discovery URL of the agent returned by
/// `POST /api/a2a/discover`.
///
/// This endpoint exists to enforce operator oversight of newly discovered agents
/// (Bug #3786). Discovered agents are placed in a pending state and cannot be used
/// until an operator explicitly calls this endpoint.
#[utoipa::path(
    post,
    path = "/api/a2a/agents/{id}/approve",
    tag = "a2a",
    params(
        ("id" = String, Path, description = "Discovery URL of the pending agent (URL-encoded)"),
    ),
    responses(
        (status = 200, description = "Agent approved and promoted to trusted list", body = crate::types::JsonObject),
        (status = 404, description = "No pending agent found for the given URL")
    )
)]
pub async fn a2a_approve_external(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    // The path parameter may be URL-encoded; decode it for matching.
    let decoded = crate::percent_decode(&id);
    // Canonicalize so the lookup matches whatever form the discover
    // handler used as the storage key. Without this, an operator who
    // approves `https://x.com/` after discover stored `https://x.com`
    // (or vice versa) would 404.
    let url = librefang_runtime::a2a::canonicalize_a2a_url(&decoded).unwrap_or(decoded);

    match state.pending_a2a_agents.remove(&url) {
        Some((_, card)) => {
            tracing::info!(
                url = %url,
                agent_name = %card.name,
                "A2A agent approved by operator and promoted to trusted list."
            );
            let card_json = serde_json::to_value(&card).unwrap_or_default();
            let card_name = card.name.clone();
            // Promote to kernel's trusted list.
            {
                let mut agents = state
                    .kernel
                    .a2a_agents()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                // Update existing entry (same URL) or append.
                if let Some(existing) = agents.iter_mut().find(|(u, _)| u == &url) {
                    existing.1 = card;
                } else {
                    agents.push((url.clone(), card));
                }
            }
            // Bug #3786: audit the trust promotion — this is the moment the
            // agent gains the ability to receive tasks, so it must be in the
            // operator's audit trail.
            let audit_detail = format!("url={url} name={card_name}");
            state.kernel.audit().record_with_context(
                "system",
                librefang_runtime::audit::AuditAction::A2aTrusted,
                &audit_detail,
                "ok",
                None,
                Some("api".to_string()),
            );
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "url": url,
                    "status": "trusted",
                    "agent": card_json,
                    "message": "Agent approved. It can now receive tasks via POST /api/a2a/send.",
                })),
            )
        }
        None => {
            // Also check if it's already trusted (idempotent re-approval).
            let agents = state
                .kernel
                .a2a_agents()
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if agents.iter().any(|(u, _)| u == &url) {
                return (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "url": url,
                        "status": "trusted",
                        "message": "Agent is already in the trusted list.",
                    })),
                );
            }
            ApiErrorResponse::not_found(format!(
                "No pending agent found for URL '{}'. \
                 Use POST /api/a2a/discover first.",
                url
            ))
            .into_json_tuple()
        }
    }
}

// ── MCP HTTP Endpoint ───────────────────────────────────────────────────

/// POST /mcp — Handle MCP JSON-RPC requests over HTTP.
///
/// Exposes the same MCP protocol normally served via stdio, allowing
/// external MCP clients to connect over HTTP instead.
#[utoipa::path(
    post,
    path = "/mcp",
    tag = "mcp",
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Handle MCP JSON-RPC requests over HTTP", body = crate::types::JsonObject)
    )
)]
pub async fn mcp_http(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<serde_json::Value>,
) -> impl IntoResponse {
    // Gather all available tools (builtin + skills + MCP)
    let mut tools = builtin_tool_definitions();
    {
        let registry = state
            .kernel
            .skill_registry_ref()
            .read()
            .unwrap_or_else(|e| e.into_inner());
        for skill_tool in registry.all_tool_definitions() {
            tools.push(librefang_types::tool::ToolDefinition {
                name: skill_tool.name.clone(),
                description: skill_tool.description.clone(),
                input_schema: skill_tool.input_schema.clone(),
            });
        }
    }
    if let Ok(mcp_tools) = state.kernel.mcp_tools_ref().lock() {
        tools.extend(mcp_tools.iter().cloned());
    }

    // Check if this is a tools/call that needs real execution
    let method = request["method"].as_str().unwrap_or("");
    if method == "tools/call" {
        let tool_name = request["params"]["name"].as_str().unwrap_or("");
        let arguments = request["params"]
            .get("arguments")
            .cloned()
            .unwrap_or(serde_json::json!({}));

        // Verify the tool exists
        if !tools.iter().any(|t| t.name == tool_name) {
            return Json(serde_json::json!({
                "jsonrpc": "2.0",
                "id": request.get("id").cloned(),
                "error": {"code": -32602, "message": format!("Unknown tool: {tool_name}")}
            }));
        }

        // Snapshot skill registry before async call (RwLockReadGuard is !Send)
        let skill_snapshot = state
            .kernel
            .skill_registry_ref()
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .snapshot();

        // Resolve the caller agent from the `X-LibreFang-Agent-Id` header,
        // if any. When a CLI driver (e.g. claude-code's `--mcp-config`)
        // re-exposes LibreFang tools to a spawned CLI, the driver writes
        // the owning agent's ID into this header so we can rehydrate the
        // ToolExecContext fields that the direct agent-loop path would
        // populate (workspace_root, allowed_tools, allowed_skills,
        // exec_policy, hand_allowed_env). Without it, every file/media/
        // cron/schedule tool fails with "workspace sandbox not configured"
        // or "Agent ID required" — issue #2699.
        //
        // Unauthenticated external MCP clients do not set this header and
        // continue to run with `None` context: the fallback behaviour is
        // unchanged.
        let caller_entry = headers
            .get("x-librefang-agent-id")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<librefang_types::agent::AgentId>().ok())
            .and_then(|id| state.kernel.agent_registry().get(id));

        let caller_agent_id_string = caller_entry.as_ref().map(|e| e.id.to_string());
        let workspace_root = caller_entry
            .as_ref()
            .and_then(|e| e.manifest.workspace.as_deref());
        // Build the allowed-tool-name list the same way the direct agent-loop
        // path does: `kernel.available_tools(id)` already resolves declared
        // tools + ToolProfile expansion + skill-evolution defaults + MCP
        // server scoping + `tool_allowlist`/`tool_blocklist` + global
        // `tool_policy` + the `ToolAll` capability + the browser toggle.
        // Then mirror the kernel's per-message mode filter (Observe/Assist/
        // Full) that `send_message` applies before handing tools to
        // `run_agent_loop` (kernel/mod.rs:3997, 5148, 6852).
        //
        // Using `manifest.capabilities.tools` raw would silently break every
        // agent that declares `capabilities.tools = []` (the common
        // "unrestricted" default) because `execute_tool` treats `Some([])`
        // as "deny all" — the exact symptom would be every tool coming back
        // as "Permission denied" through the bridge even though the agent
        // was allowed everything on the direct path.
        let allowed_tools_vec = caller_entry.as_ref().map(|e| {
            let tools = state.kernel.available_tools(e.id);
            e.mode
                .filter_tools((*tools).clone())
                .into_iter()
                .map(|t| t.name)
                .collect::<Vec<String>>()
        });
        let allowed_skills_vec = caller_entry.as_ref().map(|e| e.manifest.skills.clone());
        let exec_policy = caller_entry
            .as_ref()
            .and_then(|e| e.manifest.exec_policy.as_ref());
        let hand_allowed_env: Option<Vec<String>> = caller_entry
            .as_ref()
            .and_then(|e| e.manifest.metadata.get("hand_allowed_env"))
            .and_then(|v| serde_json::from_value(v.clone()).ok());

        // Execute the tool via the kernel's tool runner
        let kernel_handle: Arc<dyn librefang_runtime::kernel_handle::KernelHandle> =
            state.kernel.clone() as Arc<dyn librefang_runtime::kernel_handle::KernelHandle>;
        // Snapshot config before async call — Guard is !Send and cannot cross .await
        let cfg = state.kernel.config_snapshot();
        let tts_opt = if cfg.tts.enabled {
            Some(state.kernel.tts())
        } else {
            None
        };
        let docker_opt = if cfg.docker.enabled {
            Some(&cfg.docker)
        } else {
            None
        };
        let result = librefang_runtime::tool_runner::execute_tool(
            "mcp-http",
            tool_name,
            &arguments,
            Some(&kernel_handle),
            allowed_tools_vec.as_deref(),
            caller_agent_id_string.as_deref(),
            Some(&skill_snapshot),
            allowed_skills_vec.as_deref(),
            Some(state.kernel.mcp_connections_ref()),
            Some(state.kernel.web_tools()),
            Some(state.kernel.browser()),
            hand_allowed_env.as_deref(),
            workspace_root,
            Some(state.kernel.media()),
            Some(state.kernel.media_drivers()),
            exec_policy,
            tts_opt,
            docker_opt,
            Some(state.kernel.processes()),
            None, // process_registry (network bridge doesn't run agent tools)
            None, // sender_id (MCP HTTP has no sender context)
            None, // channel
            None, // checkpoint_manager (network bridge doesn't run agent tools)
            None, // interrupt (MCP HTTP calls have no session-scoped cancellation)
            None, // session_id (MCP HTTP is not tied to a live session)
            None, // dangerous_command_checker (no session-scoped checker here)
            None, // available_tools (lazy-load pool not applicable to MCP bridge)
        )
        .await;

        return Json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": request.get("id").cloned(),
            "result": {
                "content": [{"type": "text", "text": result.content}],
                "isError": result.is_error,
            }
        }));
    }

    // For non-tools/call methods (initialize, tools/list, etc.), delegate to the handler
    let response = librefang_runtime::mcp_server::handle_mcp_request(&request, &tools).await;
    Json(response)
}

// ── Multi-Session Endpoints ─────────────────────────────────────────────

// ---------------------------------------------------------------------------
// Agent Communication (Comms) endpoints
// ---------------------------------------------------------------------------

/// GET /api/comms/topology — Build agent topology graph from registry.
#[utoipa::path(
    get,
    path = "/api/comms/topology",
    tag = "network",
    responses(
        (status = 200, description = "Build agent topology graph", body = crate::types::JsonObject)
    )
)]
pub async fn comms_topology(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    use librefang_types::comms::{EdgeKind, TopoEdge, TopoNode, Topology};

    let agents = state.kernel.agent_registry().list();

    let nodes: Vec<TopoNode> = agents
        .iter()
        .map(|e| TopoNode {
            id: e.id.to_string(),
            name: e.name.clone(),
            state: format!("{:?}", e.state),
            model: e.manifest.model.model.clone(),
        })
        .collect();

    let mut edges: Vec<TopoEdge> = Vec::new();

    // Parent-child edges from registry
    for agent in &agents {
        for child_id in &agent.children {
            edges.push(TopoEdge {
                from: agent.id.to_string(),
                to: child_id.to_string(),
                kind: EdgeKind::ParentChild,
            });
        }
    }

    // Peer message edges from event bus history
    let events = state.kernel.event_bus_ref().history(500).await;
    let mut peer_pairs = std::collections::HashSet::new();
    for event in &events {
        if let librefang_types::event::EventPayload::Message(_) = &event.payload {
            if let librefang_types::event::EventTarget::Agent(target_id) = &event.target {
                let from = event.source.to_string();
                let to = target_id.to_string();
                // Deduplicate: only one edge per pair, skip self-loops
                if from != to {
                    let key = if from < to {
                        (from.clone(), to.clone())
                    } else {
                        (to.clone(), from.clone())
                    };
                    if peer_pairs.insert(key) {
                        edges.push(TopoEdge {
                            from,
                            to,
                            kind: EdgeKind::Peer,
                        });
                    }
                }
            }
        }
    }

    Json(serde_json::to_value(Topology { nodes, edges }).unwrap_or_default())
}

/// Filter a kernel event into a CommsEvent, if it represents inter-agent communication.
fn filter_to_comms_event(
    event: &librefang_types::event::Event,
    agents: &[librefang_types::agent::AgentEntry],
) -> Option<librefang_types::comms::CommsEvent> {
    use librefang_types::comms::{CommsEvent, CommsEventKind};
    use librefang_types::event::{EventPayload, EventTarget, LifecycleEvent};

    let resolve_name = |id: &str| -> String {
        agents
            .iter()
            .find(|a| a.id.to_string() == id)
            .map(|a| a.name.clone())
            .unwrap_or_else(|| id.to_string())
    };

    match &event.payload {
        EventPayload::Message(msg) => {
            let target_id = match &event.target {
                EventTarget::Agent(id) => id.to_string(),
                _ => String::new(),
            };
            Some(CommsEvent {
                id: event.id.to_string(),
                timestamp: event.timestamp.to_rfc3339(),
                kind: CommsEventKind::AgentMessage,
                source_id: event.source.to_string(),
                source_name: resolve_name(&event.source.to_string()),
                target_id: target_id.clone(),
                target_name: resolve_name(&target_id),
                detail: librefang_types::truncate_str(&msg.content, 200).to_string(),
            })
        }
        EventPayload::Lifecycle(lifecycle) => match lifecycle {
            LifecycleEvent::Spawned { agent_id, name } => Some(CommsEvent {
                id: event.id.to_string(),
                timestamp: event.timestamp.to_rfc3339(),
                kind: CommsEventKind::AgentSpawned,
                source_id: event.source.to_string(),
                source_name: resolve_name(&event.source.to_string()),
                target_id: agent_id.to_string(),
                target_name: name.clone(),
                detail: format!("Agent '{}' spawned", name),
            }),
            LifecycleEvent::Terminated { agent_id, reason } => Some(CommsEvent {
                id: event.id.to_string(),
                timestamp: event.timestamp.to_rfc3339(),
                kind: CommsEventKind::AgentTerminated,
                source_id: event.source.to_string(),
                source_name: resolve_name(&event.source.to_string()),
                target_id: agent_id.to_string(),
                target_name: resolve_name(&agent_id.to_string()),
                detail: format!("Terminated: {}", reason),
            }),
            _ => None,
        },
        EventPayload::System(sys) => {
            use librefang_types::event::SystemEvent;
            match sys {
                SystemEvent::TaskPosted {
                    task_id,
                    title,
                    assigned_to,
                    created_by,
                } => {
                    let target_id = assigned_to.clone().unwrap_or_default();
                    let source_id = created_by.clone().unwrap_or_default();
                    Some(CommsEvent {
                        id: event.id.to_string(),
                        timestamp: event.timestamp.to_rfc3339(),
                        kind: CommsEventKind::TaskPosted,
                        source_id: source_id.clone(),
                        source_name: resolve_name(&source_id),
                        target_id: target_id.clone(),
                        target_name: resolve_name(&target_id),
                        detail: format!("Task posted: {} ({})", title, task_id),
                    })
                }
                SystemEvent::TaskClaimed {
                    task_id,
                    claimed_by,
                } => Some(CommsEvent {
                    id: event.id.to_string(),
                    timestamp: event.timestamp.to_rfc3339(),
                    kind: CommsEventKind::TaskClaimed,
                    source_id: claimed_by.clone(),
                    source_name: resolve_name(claimed_by),
                    target_id: String::new(),
                    target_name: String::new(),
                    detail: format!("Task claimed: {}", task_id),
                }),
                SystemEvent::TaskCompleted {
                    task_id,
                    completed_by,
                    result,
                } => Some(CommsEvent {
                    id: event.id.to_string(),
                    timestamp: event.timestamp.to_rfc3339(),
                    kind: CommsEventKind::TaskCompleted,
                    source_id: completed_by.clone(),
                    source_name: resolve_name(completed_by),
                    target_id: String::new(),
                    target_name: String::new(),
                    detail: format!(
                        "Task completed: {} — {}",
                        task_id,
                        librefang_types::truncate_str(result, 200)
                    ),
                }),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Convert an audit entry into a CommsEvent if it represents inter-agent activity.
fn audit_to_comms_event(
    entry: &librefang_runtime::audit::AuditEntry,
    agents: &[librefang_types::agent::AgentEntry],
) -> Option<librefang_types::comms::CommsEvent> {
    use librefang_types::comms::{CommsEvent, CommsEventKind};

    let resolve_name = |id: &str| -> String {
        agents
            .iter()
            .find(|a| a.id.to_string() == id)
            .map(|a| a.name.clone())
            .unwrap_or_else(|| {
                if id.is_empty() || id == "system" {
                    "system".to_string()
                } else {
                    librefang_types::truncate_str(id, 12).to_string()
                }
            })
    };

    let action_str = format!("{:?}", entry.action);
    let (kind, detail, target_label) = match action_str.as_str() {
        "AgentMessage" => {
            // Format detail: "tokens_in=X, tokens_out=Y" → readable summary
            let detail = if entry.detail.starts_with("tokens_in=") {
                let parts: Vec<&str> = entry.detail.split(", ").collect();
                let in_tok = parts
                    .first()
                    .and_then(|p| p.strip_prefix("tokens_in="))
                    .unwrap_or("?");
                let out_tok = parts
                    .get(1)
                    .and_then(|p| p.strip_prefix("tokens_out="))
                    .unwrap_or("?");
                if entry.outcome == "ok" {
                    format!("{} in / {} out tokens", in_tok, out_tok)
                } else {
                    format!(
                        "{} in / {} out — {}",
                        in_tok,
                        out_tok,
                        librefang_types::truncate_str(&entry.outcome, 80)
                    )
                }
            } else if entry.outcome != "ok" {
                format!(
                    "{} — {}",
                    librefang_types::truncate_str(&entry.detail, 80),
                    librefang_types::truncate_str(&entry.outcome, 80)
                )
            } else {
                librefang_types::truncate_str(&entry.detail, 200).to_string()
            };
            (CommsEventKind::AgentMessage, detail, "user")
        }
        "AgentSpawn" => (
            CommsEventKind::AgentSpawned,
            format!(
                "Agent spawned: {}",
                librefang_types::truncate_str(&entry.detail, 100)
            ),
            "",
        ),
        "AgentKill" => (
            CommsEventKind::AgentTerminated,
            format!(
                "Agent killed: {}",
                librefang_types::truncate_str(&entry.detail, 100)
            ),
            "",
        ),
        _ => return None,
    };

    Some(CommsEvent {
        id: format!("audit-{}", entry.seq),
        timestamp: entry.timestamp.clone(),
        kind,
        source_id: entry.agent_id.clone(),
        source_name: resolve_name(&entry.agent_id),
        target_id: if target_label.is_empty() {
            String::new()
        } else {
            target_label.to_string()
        },
        target_name: if target_label.is_empty() {
            String::new()
        } else {
            target_label.to_string()
        },
        detail,
    })
}

/// GET /api/comms/events — Return recent inter-agent communication events.
///
/// Sources from both the event bus (for lifecycle events with full context)
/// and the audit log (for message/spawn/kill events that are always captured).
///
/// Envelope is the canonical `PaginatedResponse{items,total,offset,limit}`
/// shape used by `/api/agents` (#3842). Events are returned in a single
/// page capped by `limit` (default 100, max 500); `offset` is always 0.
#[utoipa::path(
    get,
    path = "/api/comms/events",
    tag = "network",
    params(
        ("limit" = Option<usize>, Query, description = "Maximum number of results"),
    ),
    responses(
        (status = 200, description = "Recent inter-agent communication events", body = serde_json::Value)
    )
)]
pub async fn comms_events(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(100)
        .min(500);

    let agents = state.kernel.agent_registry().list();

    // Primary source: event bus (has full source/target context)
    let bus_events = state.kernel.event_bus_ref().history(500).await;
    let mut comms_events: Vec<librefang_types::comms::CommsEvent> = bus_events
        .iter()
        .filter_map(|e| filter_to_comms_event(e, &agents))
        .collect();

    // Secondary source: audit log (always populated, wider coverage)
    let audit_entries = state.kernel.audit().recent(500);
    let seen_ids: std::collections::HashSet<String> =
        comms_events.iter().map(|e| e.id.clone()).collect();

    for entry in audit_entries.iter().rev() {
        if let Some(ev) = audit_to_comms_event(entry, &agents) {
            if !seen_ids.contains(&ev.id) {
                comms_events.push(ev);
            }
        }
    }

    // Sort by timestamp descending (newest first)
    comms_events.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    comms_events.truncate(limit);

    let total = comms_events.len();
    Json(crate::types::PaginatedResponse {
        items: comms_events,
        total,
        offset: 0,
        limit: Some(limit),
    })
}

/// GET /api/comms/events/stream — SSE stream of inter-agent communication events.
///
/// Polls the audit log every 500ms for new inter-agent events.
#[utoipa::path(
    get,
    path = "/api/comms/events/stream",
    tag = "network",
    responses(
        (status = 200, description = "SSE stream of inter-agent events", body = crate::types::JsonObject)
    )
)]
pub async fn comms_events_stream(State(state): State<Arc<AppState>>) -> axum::response::Response {
    use axum::response::sse::{Event, KeepAlive, Sse};

    let (tx, rx) = tokio::sync::mpsc::channel::<
        Result<axum::response::sse::Event, std::convert::Infallible>,
    >(256);

    tokio::spawn(async move {
        let mut last_seq: u64 = {
            let entries = state.kernel.audit().recent(1);
            entries.last().map(|e| e.seq).unwrap_or(0)
        };

        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;

            let agents = state.kernel.agent_registry().list();
            let entries = state.kernel.audit().recent(50);

            for entry in &entries {
                if entry.seq <= last_seq {
                    continue;
                }
                if let Some(comms_event) = audit_to_comms_event(entry, &agents) {
                    let data = serde_json::to_string(&comms_event).unwrap_or_default();
                    if tx.send(Ok(Event::default().data(data))).await.is_err() {
                        return; // Client disconnected
                    }
                }
            }

            if let Some(last) = entries.last() {
                last_seq = last.seq;
            }
        }
    });

    let rx_stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Sse::new(rx_stream)
        .keep_alive(
            KeepAlive::new()
                .interval(std::time::Duration::from_secs(15))
                .text("ping"),
        )
        .into_response()
}

/// POST /api/comms/send — Send a message from one agent to another.
#[utoipa::path(
    post,
    path = "/api/comms/send",
    tag = "network",
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Send a message between agents", body = crate::types::JsonObject)
    )
)]
pub async fn comms_send(
    State(state): State<Arc<AppState>>,
    Json(req): Json<librefang_types::comms::CommsSendRequest>,
) -> impl IntoResponse {
    // Validate from agent exists
    let from_id: librefang_types::agent::AgentId = match req.from_agent_id.parse() {
        Ok(id) => id,
        Err(_) => return ApiErrorResponse::bad_request("Invalid from_agent_id").into_json_tuple(),
    };
    if state.kernel.agent_registry().get(from_id).is_none() {
        return ApiErrorResponse::not_found("Source agent not found").into_json_tuple();
    }

    // Validate to agent exists
    let to_id: librefang_types::agent::AgentId = match req.to_agent_id.parse() {
        Ok(id) => id,
        Err(_) => return ApiErrorResponse::bad_request("Invalid to_agent_id").into_json_tuple(),
    };
    if state.kernel.agent_registry().get(to_id).is_none() {
        return ApiErrorResponse::not_found("Target agent not found").into_json_tuple();
    }

    // SECURITY: Limit message size
    if req.message.len() > 64 * 1024 {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"error": "Message too large (max 64KB)"})),
        );
    }

    // Resolve URL-based attachments into image content blocks
    let content_blocks = if req.attachments.is_empty() {
        None
    } else {
        let blocks = super::agents::resolve_url_attachments(&req.attachments).await;
        if blocks.is_empty() {
            None
        } else {
            Some(blocks)
        }
    };

    let kernel_handle: Arc<dyn KernelHandle> = state.kernel.clone() as Arc<dyn KernelHandle>;
    match state
        .kernel
        .send_message_with_handle_and_blocks(
            to_id,
            &req.message,
            Some(kernel_handle),
            content_blocks,
        )
        .await
    {
        Ok(result) => {
            let mut resp = serde_json::json!({
                "ok": true,
                "response": result.response,
                "input_tokens": result.total_usage.input_tokens,
                "output_tokens": result.total_usage.output_tokens,
            });
            if let Some(tid) = &req.thread_id {
                resp["thread_id"] = serde_json::json!(tid);
            }
            (StatusCode::OK, Json(resp))
        }
        Err(e) => {
            ApiErrorResponse::internal(format!("Message delivery failed: {e}")).into_json_tuple()
        }
    }
}

/// POST /api/comms/task — Post a task to the agent task queue.
#[utoipa::path(
    post,
    path = "/api/comms/task",
    tag = "network",
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Post a task to the agent task queue", body = crate::types::JsonObject)
    )
)]
pub async fn comms_task(
    State(state): State<Arc<AppState>>,
    Json(req): Json<librefang_types::comms::CommsTaskRequest>,
) -> impl IntoResponse {
    if req.title.is_empty() {
        return ApiErrorResponse::bad_request("Title is required").into_json_tuple();
    }

    match state
        .kernel
        .task_post(
            &req.title,
            &req.description,
            req.assigned_to.as_deref(),
            Some("ui-user"),
        )
        .await
    {
        Ok(task_id) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "ok": true,
                "task_id": task_id,
            })),
        ),
        Err(e) => ApiErrorResponse::internal(format!("Failed to post task: {e}")).into_json_tuple(),
    }
}

#[allow(dead_code)]
pub(crate) fn remove_toml_section(content: &str, section: &str) -> String {
    let header = format!("[{}]", section);
    let mut result = String::new();
    let mut skipping = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == header {
            skipping = true;
            continue;
        }
        if skipping && trimmed.starts_with('[') {
            skipping = false;
        }
        if !skipping {
            result.push_str(line);
            result.push('\n');
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{canonical_ip, is_cloud_metadata_ip, is_private_ip};
    use std::net::{IpAddr, Ipv4Addr};

    // -----------------------------------------------------------------
    // Regression test for #3644: /api/peers must reflect the live kernel
    // PeerRegistry, not a boot-time snapshot. Previously AppState held
    // `peer_registry: Option<Arc<PeerRegistry>>` populated once in
    // `serve()` from `kernel.peer_registry_ref()`. If the OFP node
    // initialized the registry *after* AppState was built (or never),
    // the cached `Option::None` was permanent and `/api/peers` always
    // returned an empty list even after peers connected.
    //
    // The fix removes the cache and reads `state.kernel.peer_registry_ref()`
    // live in the handler. This test:
    //   1. Boots a kernel with no OFP node started (registry == None).
    //   2. Builds AppState.
    //   3. Installs a registry into the kernel (simulating OFP startup
    //      AFTER AppState construction).
    //   4. Adds a peer to that registry.
    //   5. Calls `list_peers` and asserts the peer is visible.
    // Pre-fix this test would see `peers: []`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn list_peers_reflects_peers_added_after_appstate_boot() {
        use crate::routes::AppState;
        use axum::extract::State;
        use axum::response::IntoResponse;
        use chrono::Utc;
        use http_body_util::BodyExt;
        use librefang_types::config::KernelConfig;
        use librefang_wire::registry::{PeerEntry, PeerRegistry, PeerState};
        use std::sync::Arc;

        let tmp = tempfile::tempdir().unwrap();
        let home_dir = tmp.path().join("librefang-api-peer-test");
        std::fs::create_dir_all(&home_dir).unwrap();
        let config = KernelConfig {
            home_dir: home_dir.clone(),
            data_dir: home_dir.join("data"),
            ..KernelConfig::default()
        };
        let kernel = Arc::new(librefang_kernel::LibreFangKernel::boot_with_config(config).unwrap());

        // No OFP node => registry is None at AppState-build time.
        assert!(kernel.peer_registry_ref().is_none());

        let state = Arc::new(AppState {
            kernel: kernel.clone(),
            started_at: std::time::Instant::now(),
            bridge_manager: tokio::sync::Mutex::new(None),
            channels_config: tokio::sync::RwLock::new(Default::default()),
            shutdown_notify: Arc::new(tokio::sync::Notify::new()),
            clawhub_cache: dashmap::DashMap::new(),
            skillhub_cache: dashmap::DashMap::new(),
            provider_probe_cache: librefang_runtime::provider_health::ProbeCache::new(),
            provider_test_cache: dashmap::DashMap::new(),
            webhook_store: crate::webhook_store::WebhookStore::load(
                home_dir.join("data").join("webhooks.json"),
            ),
            active_sessions: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            media_drivers: librefang_runtime::media::MediaDriverCache::new(),
            webhook_router: Arc::new(tokio::sync::RwLock::new(Arc::new(axum::Router::new()))),
            api_key_lock: Arc::new(tokio::sync::RwLock::new(String::new())),
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            config_write_lock: tokio::sync::Mutex::new(()),
            pending_a2a_agents: dashmap::DashMap::new(),
            auth_login_limiter: Arc::new(crate::rate_limiter::AuthLoginLimiter::new()),
            gcra_limiter: crate::rate_limiter::create_rate_limiter(0),
        });

        // Simulate OFP startup happening AFTER AppState construction.
        let registry = PeerRegistry::new();
        kernel
            .install_peer_registry_for_test(registry.clone())
            .expect("registry not yet set");

        // Register a peer post-boot — the bug was these never appeared.
        registry.add_peer(PeerEntry {
            node_id: "node-abc".to_string(),
            node_name: "test-peer".to_string(),
            address: "127.0.0.1:9090".parse().unwrap(),
            agents: Vec::new(),
            state: PeerState::Connected,
            connected_at: Utc::now(),
            protocol_version: 1,
        });

        let resp = super::list_peers(State(state)).await.into_response();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            json["total"], 1,
            "expected post-boot peer to appear, got {json}"
        );
        assert_eq!(json["items"][0]["node_id"], "node-abc");
    }

    #[test]
    fn canonical_ip_unwraps_ipv4_mapped_v6() {
        let mapped: IpAddr = "::ffff:169.254.169.254".parse().unwrap();
        assert_eq!(
            canonical_ip(&mapped),
            IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254))
        );
        // Real IPv6 is left alone.
        let real_v6: IpAddr = "2001:db8::1".parse().unwrap();
        assert_eq!(canonical_ip(&real_v6), real_v6);
    }

    #[test]
    fn is_private_ip_recognises_ipv4_mapped_v6() {
        // Without canonicalisation the V6 arms only cover fc00::/7 + fe80::/10,
        // letting ::ffff:X.X.X.X slip past as "public". These must be blocked.
        assert!(is_private_ip(&"::ffff:10.0.0.1".parse().unwrap()));
        assert!(is_private_ip(&"::ffff:127.0.0.1".parse().unwrap()));
        assert!(is_private_ip(&"::ffff:169.254.169.254".parse().unwrap()));
        assert!(is_private_ip(&"::ffff:192.168.1.1".parse().unwrap()));
        assert!(is_private_ip(&"::ffff:100.64.0.1".parse().unwrap()));
    }

    #[test]
    fn is_cloud_metadata_ip_recognises_ipv4_mapped_v6() {
        // AWS IMDS + Alibaba IMDS (CGNAT) expressed as IPv4-mapped IPv6 must
        // unconditionally be blocked — this is the exact reproduction from
        // PR #2396 but exercising the network.rs copy of the guard.
        assert!(is_cloud_metadata_ip(
            &"::ffff:169.254.169.254".parse().unwrap()
        ));
        assert!(is_cloud_metadata_ip(&"::ffff:a9fe:a9fe".parse().unwrap()));
        assert!(is_cloud_metadata_ip(&"::ffff:100.64.0.1".parse().unwrap()));
        assert!(is_cloud_metadata_ip(
            &"::ffff:100.100.100.200".parse().unwrap()
        ));
    }
}
