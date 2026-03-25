//! Network, peer, A2A protocol, and inter-agent communication handlers.

use super::AppState;

/// Build routes for the network/peer/A2A/communication domain.
pub fn router() -> axum::Router<std::sync::Arc<AppState>> {
    axum::Router::new()
        .route("/peers", axum::routing::get(list_peers))
        .route("/peers/{id}", axum::routing::get(get_peer))
        .route("/network/status", axum::routing::get(network_status))
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
use axum::http::StatusCode;
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
        (status = 200, description = "List known OFP peers", body = serde_json::Value)
    )
)]
pub async fn list_peers(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Peers are tracked in the wire module's PeerRegistry.
    // The kernel doesn't directly hold a PeerRegistry, so we return an empty list
    // unless one is available. The API server can be extended to inject a registry.
    if let Some(ref peer_registry) = state.peer_registry {
        let peers: Vec<serde_json::Value> = peer_registry
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
            .collect();
        Json(serde_json::json!({"peers": peers, "total": peers.len()}))
    } else {
        Json(serde_json::json!({"peers": [], "total": 0}))
    }
}

/// GET /api/peers/{id} — Get a single peer by node ID.
#[utoipa::path(
    get,
    path = "/api/peers/{id}",
    tag = "network",
    params(("id" = String, Path, description = "Peer node ID")),
    responses(
        (status = 200, description = "Peer details", body = serde_json::Value),
        (status = 404, description = "Peer not found")
    )
)]
pub async fn get_peer(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let registry = match state.peer_registry {
        Some(ref r) => r,
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
        (status = 200, description = "OFP network status summary", body = serde_json::Value)
    )
)]
pub async fn network_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let enabled = state.kernel.config_ref().network_enabled
        && !state.kernel.config_ref().network.shared_secret.is_empty();

    let (node_id, listen_address, connected_peers, total_peers) =
        if let Some(peer_node) = state.kernel.peer_node_ref() {
            let registry = peer_node.registry();
            (
                peer_node.node_id().to_string(),
                peer_node.local_addr().to_string(),
                registry.connected_count(),
                registry.total_count(),
            )
        } else {
            (String::new(), String::new(), 0, 0)
        };

    Json(serde_json::json!({
        "enabled": enabled,
        "node_id": node_id,
        "listen_address": listen_address,
        "connected_peers": connected_peers,
        "total_peers": total_peers,
    }))
}

#[utoipa::path(
    get,
    path = "/.well-known/agent.json",
    tag = "a2a",
    responses(
        (status = 200, description = "Get the A2A agent card", body = serde_json::Value)
    )
)]
pub async fn a2a_agent_card(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let agents = state.kernel.agent_registry().list();
    let base_url = format!("http://{}", state.kernel.config_ref().api_listen);

    // Use service-level A2A config for the well-known card when available.
    let (service_name, service_description) =
        if let Some(ref a2a_cfg) = state.kernel.config_ref().a2a {
            let name = if a2a_cfg.name.is_empty() {
                "LibreFang Agent OS".to_string()
            } else {
                a2a_cfg.name.clone()
            };
            (name, a2a_cfg.description.clone())
        } else {
            ("LibreFang Agent OS".to_string(), String::new())
        };

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
        (status = 200, description = "List all A2A agent cards", body = serde_json::Value)
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
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Submit a task to an agent via A2A", body = serde_json::Value)
    )
)]
pub async fn a2a_send_task(
    State(state): State<Arc<AppState>>,
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

    // Find target agent (use first available or specified)
    let agents = state.kernel.agent_registry().list();
    if agents.is_empty() {
        return ApiErrorResponse::not_found("No agents available").into_json_tuple();
    }

    let agent = &agents[0];
    let task_id = uuid::Uuid::new_v4().to_string();
    let session_id = request["params"]["sessionId"].as_str().map(String::from);

    // Create the task in the store as Working
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
    };
    state.kernel.a2a_tasks().insert(task);

    // Send message to agent
    match state.kernel.send_message(agent.id, &message_text).await {
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
                None => ApiErrorResponse::internal("Task disappeared after completion").into_json_tuple(),
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
        (status = 200, description = "Get A2A task status", body = serde_json::Value)
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
        None => ApiErrorResponse::not_found(format!("Task '{}' not found", task_id)).into_json_tuple(),
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
        (status = 200, description = "Cancel a tracked A2A task", body = serde_json::Value)
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
            None => ApiErrorResponse::internal("Task disappeared after cancellation").into_json_tuple(),
        }
    } else {
        ApiErrorResponse::not_found(format!("Task '{}' not found", task_id)).into_json_tuple()
    }
}

// ── A2A Management Endpoints (outbound) ─────────────────────────────────

/// GET /api/a2a/agents — List discovered external A2A agents.
#[utoipa::path(
    get,
    path = "/api/a2a/agents",
    tag = "a2a",
    responses(
        (status = 200, description = "List discovered external A2A agents", body = serde_json::Value)
    )
)]
pub async fn a2a_list_external_agents(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let agents = state
        .kernel
        .a2a_agents()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let items: Vec<serde_json::Value> = agents
        .iter()
        .map(|(_, card)| {
            serde_json::json!({
                "name": card.name,
                "url": card.url,
                "description": card.description,
                "skills": card.skills,
                "version": card.version,
            })
        })
        .collect();
    Json(serde_json::json!({"agents": items, "total": items.len()}))
}

/// Check whether a URL is safe to fetch (not targeting internal/private networks).
/// Returns `Ok(())` if the URL is safe, or `Err(message)` describing the problem.
fn is_url_safe_for_ssrf(raw_url: &str) -> Result<(), String> {
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
        if is_private_ip(ip) {
            return Err(format!(
                "Requests to private/internal IP addresses are not allowed ({ip})"
            ));
        }
    }

    Ok(())
}

/// Returns true if the IP address is in a private, loopback, link-local, or
/// otherwise internal range that should not be reachable from user-supplied URLs.
fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
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
        (status = 200, description = "Get a specific external A2A agent", body = serde_json::Value)
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
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Discover an external A2A agent by URL", body = serde_json::Value)
    )
)]
pub async fn a2a_discover_external(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let url = match body["url"].as_str() {
        Some(u) => u.to_string(),
        None => {
            return ApiErrorResponse::bad_request("Missing 'url' field").into_json_tuple()
        }
    };

    // SSRF protection: validate URL before making any outbound request
    if let Err(reason) = is_url_safe_for_ssrf(&url) {
        return ApiErrorResponse::bad_request(reason).into_json_tuple();
    }

    let client = librefang_runtime::a2a::A2aClient::new();
    match client.discover(&url).await {
        Ok(card) => {
            let card_json = serde_json::to_value(&card).unwrap_or_default();
            // Store in kernel's external agents list
            {
                let mut agents = state
                    .kernel
                    .a2a_agents()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                // Update or add
                if let Some(existing) = agents.iter_mut().find(|(u, _)| u == &url) {
                    existing.1 = card;
                } else {
                    agents.push((url.clone(), card));
                }
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "url": url,
                    "agent": card_json,
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
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Send a task to an external A2A agent", body = serde_json::Value)
    )
)]
pub async fn a2a_send_external(
    State(_state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let url = match body["url"].as_str() {
        Some(u) => u.to_string(),
        None => {
            return ApiErrorResponse::bad_request("Missing 'url' field").into_json_tuple()
        }
    };
    let message = match body["message"].as_str() {
        Some(m) => m.to_string(),
        None => {
            return ApiErrorResponse::bad_request("Missing 'message' field").into_json_tuple()
        }
    };
    let session_id = body["session_id"].as_str();

    let client = librefang_runtime::a2a::A2aClient::new();
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
        (status = 200, description = "Get external A2A task status", body = serde_json::Value)
    )
)]
pub async fn a2a_external_task_status(
    State(_state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let url = match params.get("url") {
        Some(u) => u.clone(),
        None => {
            return ApiErrorResponse::bad_request("Missing 'url' query parameter").into_json_tuple()
        }
    };

    let client = librefang_runtime::a2a::A2aClient::new();
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

// ── MCP HTTP Endpoint ───────────────────────────────────────────────────

/// POST /mcp — Handle MCP JSON-RPC requests over HTTP.
///
/// Exposes the same MCP protocol normally served via stdio, allowing
/// external MCP clients to connect over HTTP instead.
#[utoipa::path(
    post,
    path = "/mcp",
    tag = "mcp",
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Handle MCP JSON-RPC requests over HTTP", body = serde_json::Value)
    )
)]
pub async fn mcp_http(
    State(state): State<Arc<AppState>>,
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

        // Execute the tool via the kernel's tool runner
        let kernel_handle: Arc<dyn librefang_runtime::kernel_handle::KernelHandle> =
            state.kernel.clone() as Arc<dyn librefang_runtime::kernel_handle::KernelHandle>;
        let result = librefang_runtime::tool_runner::execute_tool(
            "mcp-http",
            tool_name,
            &arguments,
            Some(&kernel_handle),
            None,
            None,
            Some(&skill_snapshot),
            Some(state.kernel.mcp_connections_ref()),
            Some(state.kernel.web_tools()),
            Some(state.kernel.browser()),
            None,
            None,
            Some(state.kernel.media()),
            None, // media_drivers
            None, // exec_policy
            if state.kernel.config_ref().tts.enabled {
                Some(state.kernel.tts())
            } else {
                None
            },
            if state.kernel.config_ref().docker.enabled {
                Some(&state.kernel.config_ref().docker)
            } else {
                None
            },
            Some(state.kernel.processes()),
            None, // sender_id (MCP HTTP has no sender context)
            None, // channel
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
        (status = 200, description = "Build agent topology graph", body = serde_json::Value)
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

    Json(comms_events)
}

/// GET /api/comms/events/stream — SSE stream of inter-agent communication events.
///
/// Polls the audit log every 500ms for new inter-agent events.
#[utoipa::path(
    get,
    path = "/api/comms/events/stream",
    tag = "network",
    responses(
        (status = 200, description = "SSE stream of inter-agent events", body = serde_json::Value)
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
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Send a message between agents", body = serde_json::Value)
    )
)]
pub async fn comms_send(
    State(state): State<Arc<AppState>>,
    Json(req): Json<librefang_types::comms::CommsSendRequest>,
) -> impl IntoResponse {
    // Validate from agent exists
    let from_id: librefang_types::agent::AgentId = match req.from_agent_id.parse() {
        Ok(id) => id,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid from_agent_id").into_json_tuple()
        }
    };
    if state.kernel.agent_registry().get(from_id).is_none() {
        return ApiErrorResponse::not_found("Source agent not found").into_json_tuple();
    }

    // Validate to agent exists
    let to_id: librefang_types::agent::AgentId = match req.to_agent_id.parse() {
        Ok(id) => id,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid to_agent_id").into_json_tuple()
        }
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
        Err(e) => ApiErrorResponse::internal(format!("Message delivery failed: {e}")).into_json_tuple(),
    }
}

/// POST /api/comms/task — Post a task to the agent task queue.
#[utoipa::path(
    post,
    path = "/api/comms/task",
    tag = "network",
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Post a task to the agent task queue", body = serde_json::Value)
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
        .memory_substrate()
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
