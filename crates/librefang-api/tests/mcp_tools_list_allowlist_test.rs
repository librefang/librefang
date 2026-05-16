//! Regression test for issue #5101 — `POST /mcp` `tools/list` must honour
//! the caller agent's `tool_allowlist` / `tool_blocklist` the same way
//! `tools/call` and the kernel agent loop do.
//!
//! Background
//! ----------
//! Before #5101 was fixed, the `mcp_http` handler in
//! `crates/librefang-api/src/routes/network.rs` filtered tools only inside
//! the `tools/call` arm. Every other JSON-RPC method (including
//! `tools/list`) fell through to `mcp_server::handle_mcp_request` with the
//! unfiltered kernel-wide tool catalogue. When the `claude-code` driver
//! wired the Claude CLI to that bridge via `--mcp-config`, the CLI's
//! startup `tools/list` discovered every kernel-wide MCP tool — for large
//! Smithery catalogues (e.g. `googlesuper`, 223 tools) the resulting
//! 70-80 KB system prompt made `claude` exit 1 with no stderr.
//!
//! These tests boot a real `LibreFangKernel` via `MockKernelBuilder`,
//! seed two synthetic MCP tools into `mcp_tools_ref()`, register an
//! agent with `tool_blocklist` excluding one of them, and assert the
//! `tools/list` response only contains the un-blocked tool.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::agent::{
    AgentEntry, AgentId, AgentManifest, AgentMode, AgentState, SessionId,
};
use librefang_types::tool::ToolDefinition;
use std::sync::Arc;
use tower::ServiceExt;

const SMALL_TOOL: &str = "mcp_smallserver_ping";
const BIG_TOOL_KEEP: &str = "mcp_bigserver_keep_me";
const BIG_TOOL_DROP: &str = "mcp_bigserver_drop_me";

struct Harness {
    app: Router,
    state: Arc<AppState>,
    _test: TestAppState,
}

/// Build a router exposing only `POST /mcp` (matches the production
/// mount in `server::build_router`, which mounts `/mcp` at the root,
/// not under `/api`).
fn boot() -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new());
    let state = test.state.clone();
    let app = Router::new()
        .route("/mcp", axum::routing::post(routes::mcp_http))
        .with_state(state.clone());
    Harness {
        app,
        state,
        _test: test,
    }
}

/// Insert a synthetic `ToolDefinition` directly into the kernel's
/// MCP tool snapshot — bypasses the `McpConnection` plumbing because
/// the bridge only ever reads `mcp_tools_ref()` for `tools/list`
/// discovery, and the test wants deterministic content regardless
/// of network conditions.
fn seed_mcp_tool(state: &AppState, name: &str) {
    let mut guard = state
        .kernel
        .mcp_tools_ref()
        .lock()
        .expect("mcp_tools mutex not poisoned");
    guard.push(ToolDefinition {
        name: name.to_string(),
        description: format!("synthetic MCP tool {name}"),
        input_schema: serde_json::json!({"type": "object", "properties": {}}),
    });
}

/// Register an agent with an explicit `tool_blocklist`. Returns the
/// new agent id so tests can post it via `X-LibreFang-Agent-Id`.
///
/// Capabilities are left at defaults (`tools = []`) which the kernel
/// treats as "unrestricted access to all tools" — so the only thing
/// pruning the catalogue is the blocklist itself.
fn register_agent_with_blocklist(state: &AppState, blocklist: &[&str]) -> AgentId {
    register_agent_with_filters(state, &[], blocklist)
}

fn register_agent_with_filters(
    state: &AppState,
    allowlist: &[&str],
    blocklist: &[&str],
) -> AgentId {
    let id = AgentId::new();
    let manifest = AgentManifest {
        name: "mcp-test".to_string(),
        description: "agent for /mcp tools/list filter regression".to_string(),
        author: "test".to_string(),
        module: "builtin:chat".to_string(),
        tool_allowlist: allowlist.iter().map(|s| s.to_string()).collect(),
        tool_blocklist: blocklist.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    };
    let entry = AgentEntry {
        id,
        name: manifest.name.clone(),
        manifest,
        state: AgentState::Running,
        mode: AgentMode::default(),
        created_at: chrono::Utc::now(),
        last_active: chrono::Utc::now(),
        session_id: SessionId::new(),
        ..Default::default()
    };
    state
        .kernel
        .agent_registry()
        .register(entry)
        .expect("agent registry insert");
    id
}

/// Issue a `tools/list` JSON-RPC call against `/mcp`. When
/// `agent_id` is `Some`, attach the `X-LibreFang-Agent-Id` header
/// the same way `claude_code::write_mcp_config` does in production.
async fn list_tools(h: &Harness, agent_id: Option<AgentId>) -> serde_json::Value {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/list",
        "params": {},
    });
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri("/mcp")
        .header("content-type", "application/json");
    if let Some(id) = agent_id {
        builder = builder.header("X-LibreFang-Agent-Id", id.to_string());
    }
    let req = builder
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn tool_names(body: &serde_json::Value) -> Vec<String> {
    body["result"]["tools"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|t| t["name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Regression coverage
// ---------------------------------------------------------------------------

/// Core regression: with the agent header set, `tools/list` must hide
/// any tool the agent's `tool_blocklist` excludes. Before #5101 this
/// returned the full kernel catalogue including `BIG_TOOL_DROP`.
#[tokio::test(flavor = "multi_thread")]
async fn tools_list_honours_agent_blocklist() {
    let h = boot();
    seed_mcp_tool(&h.state, SMALL_TOOL);
    seed_mcp_tool(&h.state, BIG_TOOL_KEEP);
    seed_mcp_tool(&h.state, BIG_TOOL_DROP);

    let agent_id = register_agent_with_blocklist(&h.state, &[BIG_TOOL_DROP]);

    let body = list_tools(&h, Some(agent_id)).await;
    let names = tool_names(&body);

    assert!(
        names.iter().any(|n| n == SMALL_TOOL),
        "small-server tool must remain visible; got {names:?}"
    );
    assert!(
        names.iter().any(|n| n == BIG_TOOL_KEEP),
        "non-blocked big-server tool must remain visible; got {names:?}"
    );
    assert!(
        !names.iter().any(|n| n == BIG_TOOL_DROP),
        "blocked big-server tool must be hidden from tools/list; got {names:?}"
    );
}

/// Allowlist path: when the agent declares an explicit
/// `tool_allowlist`, `tools/list` returns only those entries.
#[tokio::test(flavor = "multi_thread")]
async fn tools_list_honours_agent_allowlist() {
    let h = boot();
    seed_mcp_tool(&h.state, SMALL_TOOL);
    seed_mcp_tool(&h.state, BIG_TOOL_KEEP);
    seed_mcp_tool(&h.state, BIG_TOOL_DROP);

    let agent_id = register_agent_with_filters(&h.state, &[SMALL_TOOL], &[]);

    let body = list_tools(&h, Some(agent_id)).await;
    let names = tool_names(&body);

    assert!(
        names.iter().any(|n| n == SMALL_TOOL),
        "allowlisted tool must be visible; got {names:?}"
    );
    assert!(
        !names
            .iter()
            .any(|n| n == BIG_TOOL_KEEP || n == BIG_TOOL_DROP),
        "non-allowlisted MCP tools must be hidden from tools/list; got {names:?}"
    );
}

/// Header-less fallback: external MCP clients that don't set
/// `X-LibreFang-Agent-Id` keep the pre-#5101 behaviour and see the
/// full kernel-wide catalogue. This guards against the fix going
/// too far and breaking unauthenticated discovery flows.
#[tokio::test(flavor = "multi_thread")]
async fn tools_list_falls_back_to_kernel_catalogue_when_no_header() {
    let h = boot();
    seed_mcp_tool(&h.state, SMALL_TOOL);
    seed_mcp_tool(&h.state, BIG_TOOL_KEEP);
    seed_mcp_tool(&h.state, BIG_TOOL_DROP);

    // Register an agent with a restrictive blocklist, but DON'T pass
    // the header — the bridge must not apply that agent's filter.
    let _agent_id = register_agent_with_blocklist(&h.state, &[BIG_TOOL_DROP]);

    let body = list_tools(&h, None).await;
    let names = tool_names(&body);

    assert!(
        names.iter().any(|n| n == SMALL_TOOL),
        "header-less tools/list must include all kernel MCP tools; got {names:?}"
    );
    assert!(
        names.iter().any(|n| n == BIG_TOOL_KEEP),
        "header-less tools/list must include all kernel MCP tools; got {names:?}"
    );
    assert!(
        names.iter().any(|n| n == BIG_TOOL_DROP),
        "header-less tools/list must not apply any agent's blocklist; got {names:?}"
    );
}

/// Unknown / unparseable agent header is treated identically to a
/// missing header: the bridge falls back to the unfiltered catalogue
/// rather than silently erroring or returning an empty list.
#[tokio::test(flavor = "multi_thread")]
async fn tools_list_unknown_agent_header_falls_back_to_kernel_catalogue() {
    let h = boot();
    seed_mcp_tool(&h.state, SMALL_TOOL);
    seed_mcp_tool(&h.state, BIG_TOOL_DROP);

    // Pass a syntactically-valid but unregistered agent id.
    let bogus = AgentId::new();
    let body = list_tools(&h, Some(bogus)).await;
    let names = tool_names(&body);

    assert!(
        names.iter().any(|n| n == SMALL_TOOL) && names.iter().any(|n| n == BIG_TOOL_DROP),
        "unknown agent header must fall back to the unfiltered catalogue; got {names:?}"
    );
}
