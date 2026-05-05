//! Integration tests for the MCP HttpCompat transport (#3696).
//!
//! `HttpCompat` is the simplest MCP transport: a static tool-declaration
//! protocol that maps tool calls onto plain HTTP/JSON requests against a
//! user-supplied base URL. It does NOT perform an MCP `initialize`
//! handshake, so it is the ideal entry point for an integration test that
//! does not require an actual MCP-protocol-speaking peer.
//!
//! These tests verify, end-to-end against a real `wiremock` server:
//!
//! 1. `McpConnection::connect` succeeds and registers the declared tools
//!    under the namespaced `mcp_<server>_<tool>` name.
//! 2. `call_tool` issues a real HTTP request to the configured backend,
//!    interpolates path parameters, and returns the response body.
//! 3. Path-template arguments are URL-encoded and consumed before the
//!    remaining args are sent as a JSON body / query string.

use librefang_runtime_mcp::{
    empty_taint_rule_sets_handle, format_mcp_tool_name, McpConnection, McpServerConfig,
    McpTransport,
};
use librefang_types::config::{
    HttpCompatHeaderConfig, HttpCompatMethod, HttpCompatRequestMode, HttpCompatResponseMode,
    HttpCompatToolConfig,
};
use serde_json::json;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn http_compat_config(base_url: String, tools: Vec<HttpCompatToolConfig>) -> McpServerConfig {
    McpServerConfig {
        name: "test-server".to_string(),
        transport: McpTransport::HttpCompat {
            base_url,
            headers: vec![HttpCompatHeaderConfig {
                name: "x-test-token".to_string(),
                value: Some("integration-fixture".to_string()),
                value_env: None,
            }],
            tools,
        },
        timeout_secs: 5,
        env: vec![],
        headers: vec![],
        oauth_provider: None,
        oauth_config: None,
        taint_scanning: false,
        taint_policy: None,
        taint_rule_sets: empty_taint_rule_sets_handle(),
        roots: vec![],
    }
}

fn weather_tool() -> HttpCompatToolConfig {
    HttpCompatToolConfig {
        name: "get_weather".to_string(),
        description: "Look up current weather by city".to_string(),
        path: "/weather/{city}".to_string(),
        method: HttpCompatMethod::Get,
        request_mode: HttpCompatRequestMode::Query,
        response_mode: HttpCompatResponseMode::Json,
        input_schema: json!({
            "type": "object",
            "properties": {
                "city": {"type": "string"},
                "units": {"type": "string"}
            },
            "required": ["city"]
        }),
    }
}

#[tokio::test]
async fn http_compat_connect_registers_namespaced_tools() {
    let server = MockServer::start().await;

    // Probe (GET /) hits during connect — respond with anything; failure is
    // also fine because connect ignores the probe error.
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let config = http_compat_config(server.uri(), vec![weather_tool()]);
    let conn = McpConnection::connect(config)
        .await
        .expect("HttpCompat connect should succeed");

    // The tool list must contain the namespaced name `mcp_<server>_<tool>`
    // — the API consumers (agent loop, dashboard) all key off the prefixed
    // form, so a regression here breaks tool dispatch end-to-end.
    let names: Vec<&str> = conn.tools().iter().map(|t| t.name.as_str()).collect();
    let expected = format_mcp_tool_name("test-server", "get_weather");
    assert!(
        names.iter().any(|n| *n == expected),
        "expected tools to include {expected:?}, got {names:?}"
    );
    assert_eq!(conn.name(), "test-server");
}

#[tokio::test]
async fn http_compat_call_tool_renders_path_and_returns_body() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    // The driver renders `{city}` from the args and consumes that key, so
    // only `units` should arrive as a query param. The custom header from
    // `http_compat_config` must be on the call too.
    Mock::given(method("GET"))
        .and(path("/weather/Paris"))
        .and(query_param("units", "metric"))
        .and(header("x-test-token", "integration-fixture"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"city": "Paris", "tempC": 18})),
        )
        .expect(1)
        .mount(&server)
        .await;

    let config = http_compat_config(server.uri(), vec![weather_tool()]);
    let mut conn = McpConnection::connect(config)
        .await
        .expect("HttpCompat connect should succeed");

    let namespaced = format_mcp_tool_name("test-server", "get_weather");
    let response = conn
        .call_tool(&namespaced, &json!({"city": "Paris", "units": "metric"}))
        .await
        .expect("call_tool should succeed against the wiremock backend");

    // Response is forwarded verbatim (response_mode = Json) — at minimum the
    // city/temp values must round-trip so the agent loop sees the backend
    // payload.
    assert!(
        response.contains("\"city\""),
        "response must include city field, got {response}"
    );
    assert!(
        response.contains("Paris"),
        "response must include the looked-up value, got {response}"
    );
    assert!(
        response.contains("18"),
        "response must include the temperature, got {response}"
    );
}

#[tokio::test]
async fn http_compat_call_tool_unknown_name_errors() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let config = http_compat_config(server.uri(), vec![weather_tool()]);
    let mut conn = McpConnection::connect(config).await.expect("connect");

    // Calling a tool that was never registered must fail rather than
    // silently issuing an unrelated request.
    let err = conn
        .call_tool("mcp_test-server_does_not_exist", &json!({}))
        .await
        .expect_err("unknown tool must error");
    assert!(
        err.to_lowercase().contains("not found")
            || err.to_lowercase().contains("unknown")
            || err.to_lowercase().contains("does not exist"),
        "error should mention the missing tool name, got {err}"
    );
}
