//! MCP (Model Context Protocol) client — connect to external MCP servers.
//!
//! Stdio transport uses the rmcp SDK for proper MCP protocol handling.
//! SSE transport uses HTTP POST with JSON-RPC for backward compatibility.
//! HttpCompat provides a built-in adapter for plain HTTP/JSON backends.
//!
//! All MCP tools are namespaced with `mcp_{server}_{tool}` to prevent collisions.

use librefang_types::config::{
    HttpCompatHeaderConfig, HttpCompatMethod, HttpCompatRequestMode, HttpCompatResponseMode,
    HttpCompatToolConfig,
};
use librefang_types::tool::ToolDefinition;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info};

// ---------------------------------------------------------------------------
// Configuration types
// ---------------------------------------------------------------------------

/// Configuration for an MCP server connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Display name for this server (used in tool namespacing).
    pub name: String,
    /// Transport configuration.
    pub transport: McpTransport,
    /// Request timeout in seconds (default: 60).
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// Environment variables for the subprocess.
    ///
    /// Each entry should be `"KEY=VALUE"`. The subprocess does NOT inherit the
    /// parent environment — only these declared variables (plus essential system
    /// vars like PATH/HOME) are passed through.
    ///
    /// Legacy format `"KEY"` (name only, no value) will look up the value from
    /// the parent environment and pass it through.
    #[serde(default)]
    pub env: Vec<String>,
}

fn default_timeout() -> u64 {
    60
}

/// Transport type for MCP server connections.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpTransport {
    /// Subprocess with MCP protocol over stdin/stdout (via rmcp SDK).
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
    },
    /// HTTP Server-Sent Events (JSON-RPC over HTTP POST).
    Sse { url: String },
    /// Built-in compatibility adapter for plain HTTP/JSON backends.
    HttpCompat {
        base_url: String,
        #[serde(default)]
        headers: Vec<HttpCompatHeaderConfig>,
        #[serde(default)]
        tools: Vec<HttpCompatToolConfig>,
    },
}

// ---------------------------------------------------------------------------
// Connection types
// ---------------------------------------------------------------------------

/// Dynamic rmcp client type (type-erased for heterogeneous storage).
type DynRmcpClient = rmcp::service::RunningService<
    rmcp::service::RoleClient,
    Box<dyn rmcp::service::DynService<rmcp::service::RoleClient>>,
>;

/// An active connection to an MCP server.
pub struct McpConnection {
    /// Configuration for this connection.
    config: McpServerConfig,
    /// Tools discovered from the server via tools/list.
    tools: Vec<ToolDefinition>,
    /// Map from namespaced tool name → original tool name from the server.
    original_names: HashMap<String, String>,
    /// Transport-specific connection state.
    inner: McpInner,
}

/// Transport-specific connection handle.
enum McpInner {
    /// Stdio subprocess managed by the rmcp SDK.
    Rmcp(DynRmcpClient),
    /// HTTP POST with JSON-RPC (backward-compatible SSE transport).
    Sse {
        client: reqwest::Client,
        url: String,
        next_id: u64,
    },
    /// Built-in HTTP compatibility adapter.
    HttpCompat { client: reqwest::Client },
}

/// JSON-RPC 2.0 request (used by SSE transport only).
#[derive(Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str,
    id: u64,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<serde_json::Value>,
}

/// JSON-RPC 2.0 response (used by SSE transport only).
#[derive(Deserialize)]
struct JsonRpcResponse {
    #[allow(dead_code)]
    jsonrpc: String,
    #[allow(dead_code)]
    id: Option<u64>,
    result: Option<serde_json::Value>,
    error: Option<JsonRpcError>,
}

/// JSON-RPC 2.0 error object.
#[derive(Debug, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[allow(dead_code)]
    pub data: Option<serde_json::Value>,
}

impl std::fmt::Display for JsonRpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "JSON-RPC error {}: {}", self.code, self.message)
    }
}

// ---------------------------------------------------------------------------
// Environment variable allowlist for subprocess sandboxing
// ---------------------------------------------------------------------------

/// System environment variables that are safe to pass to MCP subprocesses.
const SAFE_ENV_VARS: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TERM",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TMPDIR",
    "TMP",
    "TEMP",
    "XDG_RUNTIME_DIR",
    "XDG_DATA_HOME",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    // Windows essentials
    "SystemRoot",
    "SYSTEMROOT",
    "APPDATA",
    "LOCALAPPDATA",
    "HOMEDRIVE",
    "HOMEPATH",
    "USERPROFILE",
    "COMSPEC",
    "PATHEXT",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "CommonProgramFiles",
    // Node.js / npm (needed by most MCP servers)
    "NODE_PATH",
    "NPM_CONFIG_PREFIX",
    "NVM_DIR",
    "FNM_DIR",
];

// ---------------------------------------------------------------------------
// McpConnection implementation
// ---------------------------------------------------------------------------

impl McpConnection {
    /// Connect to an MCP server, perform handshake, and discover tools.
    pub async fn connect(config: McpServerConfig) -> Result<Self, String> {
        let (inner, discovered_tools) = match &config.transport {
            McpTransport::Stdio { command, args } => {
                Self::connect_stdio(command, args, &config.env).await?
            }
            McpTransport::Sse { url } => Self::connect_sse(url).await?,
            McpTransport::HttpCompat {
                base_url,
                headers,
                tools,
            } => {
                Self::validate_http_compat_config(base_url, headers, tools)?;
                Self::connect_http_compat(base_url).await?
            }
        };

        let mut conn = Self {
            config,
            tools: Vec::new(),
            original_names: HashMap::new(),
            inner,
        };

        match discovered_tools {
            Some(tools) => {
                // Tools already discovered during connect (rmcp handles this)
                for tool in tools {
                    let description = tool.description.as_deref().unwrap_or("");
                    let input_schema =
                        serde_json::Value::Object(tool.input_schema.as_ref().clone());
                    conn.register_tool(&tool.name, description, input_schema);
                }
            }
            None => {
                // HttpCompat or SSE — discover tools the old way
                if let McpTransport::HttpCompat { tools, .. } = &conn.config.transport {
                    let declared_tools = tools.clone();
                    conn.register_http_compat_tools(&declared_tools);
                } else if let McpInner::Sse { .. } = &conn.inner {
                    conn.sse_initialize().await?;
                    conn.sse_discover_tools().await?;
                }
            }
        }

        info!(
            server = %conn.config.name,
            tools = conn.tools.len(),
            "MCP server connected"
        );

        Ok(conn)
    }

    // --- Stdio transport (rmcp SDK) ---

    async fn connect_stdio(
        command: &str,
        args: &[String],
        extra_env: &[String],
    ) -> Result<(McpInner, Option<Vec<rmcp::model::Tool>>), String> {
        use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
        use rmcp::ServiceExt;

        // Validate command path (no path traversal)
        if command.contains("..") {
            return Err("MCP command path contains '..': rejected".to_string());
        }

        // On Windows, npm/npx install as .cmd batch wrappers. Detect and adapt.
        let resolved_command: String = if cfg!(windows) {
            if command.ends_with(".cmd") || command.ends_with(".bat") {
                command.to_string()
            } else {
                let cmd_variant = format!("{command}.cmd");
                let has_cmd = std::env::var("PATH")
                    .unwrap_or_default()
                    .split(';')
                    .any(|dir| std::path::Path::new(dir).join(&cmd_variant).exists());
                if has_cmd {
                    cmd_variant
                } else {
                    command.to_string()
                }
            }
        } else {
            command.to_string()
        };

        let args_owned: Vec<String> = args.to_vec();
        let env_owned: Vec<String> = extra_env.to_vec();

        let transport = TokioChildProcess::new(
            tokio::process::Command::new(&resolved_command).configure(|cmd| {
                cmd.args(&args_owned);

                // SECURITY: Do NOT inherit the full parent environment.
                // Only pass through safe system vars + explicitly declared vars.
                cmd.env_clear();

                // Pass safe system environment variables
                for &var in SAFE_ENV_VARS {
                    if let Ok(val) = std::env::var(var) {
                        cmd.env(var, val);
                    }
                }

                // Pass declared environment variables from config
                for entry in &env_owned {
                    if let Some((key, value)) = entry.split_once('=') {
                        cmd.env(key, value);
                    } else {
                        // Legacy format: plain name — look up from parent env
                        if let Ok(value) = std::env::var(entry) {
                            cmd.env(entry, value);
                        }
                    }
                }
            }),
        )
        .map_err(|e| format!("Failed to spawn MCP server '{resolved_command}': {e}"))?;

        let client = ()
            .into_dyn()
            .serve(transport)
            .await
            .map_err(|e| format!("MCP handshake failed for '{resolved_command}': {e}"))?;

        // Discover tools via rmcp (with timeout)
        let timeout = std::time::Duration::from_secs(60);
        let tools = tokio::time::timeout(timeout, client.list_all_tools())
            .await
            .map_err(|_| format!("MCP tools/list timed out after 60s for '{resolved_command}'"))?
            .map_err(|e| format!("MCP tools/list failed: {e}"))?;

        Ok((McpInner::Rmcp(client), Some(tools)))
    }

    // --- SSE transport (JSON-RPC over HTTP POST) ---

    async fn connect_sse(url: &str) -> Result<(McpInner, Option<Vec<rmcp::model::Tool>>), String> {
        Self::check_ssrf(url, "SSE")?;

        let client = crate::http_client::proxied_client_builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

        Ok((
            McpInner::Sse {
                client,
                url: url.to_string(),
                next_id: 1,
            },
            None, // Tools discovered later via sse_initialize + sse_discover_tools
        ))
    }

    /// Send the MCP `initialize` handshake over SSE transport.
    async fn sse_initialize(&mut self) -> Result<(), String> {
        let params = serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "librefang",
                "version": env!("CARGO_PKG_VERSION")
            }
        });

        let response = self.sse_send_request("initialize", Some(params)).await?;

        if let Some(result) = response {
            debug!(
                server = %self.config.name,
                server_info = %result,
                "MCP SSE initialize response"
            );
        }

        self.sse_send_notification("notifications/initialized", None)
            .await?;

        Ok(())
    }

    /// Discover available tools via `tools/list` over SSE transport.
    async fn sse_discover_tools(&mut self) -> Result<(), String> {
        let response = self.sse_send_request("tools/list", None).await?;

        if let Some(result) = response {
            if let Some(tools_array) = result.get("tools").and_then(|t| t.as_array()) {
                for tool in tools_array {
                    let raw_name = tool["name"].as_str().unwrap_or("unnamed");
                    let description = tool["description"].as_str().unwrap_or("");
                    let input_schema = tool
                        .get("inputSchema")
                        .cloned()
                        .and_then(|v| match &v {
                            serde_json::Value::Object(_) => Some(v),
                            serde_json::Value::String(s) => {
                                serde_json::from_str::<serde_json::Value>(s)
                                    .ok()
                                    .filter(|p| p.is_object())
                            }
                            _ => None,
                        })
                        .unwrap_or(serde_json::json!({"type": "object"}));

                    self.register_tool(raw_name, description, input_schema);
                }
            }
        }

        Ok(())
    }

    async fn sse_send_request(
        &mut self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, String> {
        // Extract owned copies of the values we need before any async work,
        // so we don't hold a borrow of `self.inner` across an await point
        // (which would conflict with the concurrent borrow of `self.config`).
        let (client, url, id) = match &mut self.inner {
            McpInner::Sse {
                client,
                url,
                next_id,
            } => {
                let id = *next_id;
                *next_id += 1;
                (client.clone(), url.clone(), id)
            }
            _ => return Err("sse_send_request called on non-SSE transport".to_string()),
        };
        let timeout_secs = self.config.timeout_secs;

        let request = JsonRpcRequest {
            jsonrpc: "2.0",
            id,
            method: method.to_string(),
            params,
        };

        debug!(method, id, "MCP SSE request");

        let response = client
            .post(url.as_str())
            .json(&request)
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .send()
            .await
            .map_err(|e| format!("MCP SSE request failed: {e}"))?;

        if !response.status().is_success() {
            return Err(format!("MCP SSE returned {}", response.status()));
        }

        let body = response
            .text()
            .await
            .map_err(|e| format!("Failed to read SSE response: {e}"))?;

        let rpc_response: JsonRpcResponse = serde_json::from_str(&body)
            .map_err(|e| format!("Invalid MCP SSE JSON-RPC response: {e}"))?;

        if let Some(err) = rpc_response.error {
            return Err(format!("{err}"));
        }

        Ok(rpc_response.result)
    }

    async fn sse_send_notification(
        &mut self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<(), String> {
        let McpInner::Sse { client, url, .. } = &self.inner else {
            return Ok(());
        };

        let notification = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params.unwrap_or(serde_json::json!({})),
        });

        let _ = client.post(url.as_str()).json(&notification).send().await;
        Ok(())
    }

    // --- HttpCompat transport ---

    async fn connect_http_compat(
        base_url: &str,
    ) -> Result<(McpInner, Option<Vec<rmcp::model::Tool>>), String> {
        Self::check_ssrf(base_url, "HTTP compatibility backend")?;

        let client = crate::http_client::proxied_client_builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

        let probe = base_url.trim_end_matches('/').to_string();
        let probe_result = client
            .get(probe.as_str())
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await;

        if let Err(e) = &probe_result {
            debug!(base_url = %probe, error = %e, "HTTP compatibility backend probe failed, continuing anyway");
        } else if let Ok(response) = &probe_result {
            debug!(
                base_url = %probe,
                status = %response.status(),
                "HTTP compatibility backend reachable"
            );
        }

        Ok((McpInner::HttpCompat { client }, None))
    }

    // --- Shared ---

    fn check_ssrf(url: &str, label: &str) -> Result<(), String> {
        let lower = url.to_lowercase();
        if lower.contains("169.254.169.254") || lower.contains("metadata.google") {
            return Err(format!("SSRF: {label} URL targets metadata endpoint"));
        }
        Ok(())
    }

    fn register_http_compat_tools(&mut self, tools: &[HttpCompatToolConfig]) {
        for tool in tools {
            let description = if tool.description.trim().is_empty() {
                format!("HTTP compatibility tool {}", tool.name)
            } else {
                tool.description.clone()
            };

            let input_schema = if tool.input_schema.is_object() {
                tool.input_schema.clone()
            } else {
                serde_json::json!({"type": "object"})
            };

            self.register_tool(&tool.name, &description, input_schema);
        }
    }

    fn register_tool(
        &mut self,
        raw_name: &str,
        description: &str,
        input_schema: serde_json::Value,
    ) {
        let server_name = &self.config.name;
        let namespaced = format_mcp_tool_name(server_name, raw_name);
        self.original_names
            .insert(namespaced.clone(), raw_name.to_string());
        self.tools.push(ToolDefinition {
            name: namespaced,
            description: format!("[MCP:{server_name}] {description}"),
            input_schema,
        });
    }

    /// Call a tool on the MCP server.
    pub async fn call_tool(
        &mut self,
        name: &str,
        arguments: &serde_json::Value,
    ) -> Result<String, String> {
        // Resolve to an owned String immediately so the borrow of self.original_names
        // and self.config.name ends before any mutable operations below.
        let raw_name: String = self
            .original_names
            .get(name)
            .cloned()
            .or_else(|| strip_mcp_prefix(&self.config.name, name).map(|s| s.to_string()))
            .unwrap_or_else(|| name.to_string());

        // Determine the transport kind without holding any reference into self.inner
        // across an await or across a mutable reborrow of self.  Using a simple
        // tag enum avoids E0502 / E0521 caused by overlapping borrows.
        enum TransportKind {
            Rmcp,
            Sse,
            HttpCompat,
        }
        let kind = match &self.inner {
            McpInner::Rmcp(_) => TransportKind::Rmcp,
            McpInner::Sse { .. } => TransportKind::Sse,
            McpInner::HttpCompat { .. } => TransportKind::HttpCompat,
        };
        // `self.inner` borrow from the match above ends here.

        match kind {
            TransportKind::Rmcp => {
                let McpInner::Rmcp(client) = &mut self.inner else {
                    unreachable!()
                };

                let mut params = rmcp::model::CallToolRequestParams::new(raw_name.clone());
                if let Some(obj) = arguments.as_object() {
                    if !obj.is_empty() {
                        params.arguments = Some(obj.clone());
                    }
                }

                let timeout = std::time::Duration::from_secs(self.config.timeout_secs);
                let result: rmcp::model::CallToolResult =
                    tokio::time::timeout(timeout, client.call_tool(params))
                        .await
                        .map_err(|_| {
                            format!(
                                "MCP tool call timed out after {}s",
                                self.config.timeout_secs
                            )
                        })?
                        .map_err(|e| format!("MCP tool call failed: {e}"))?;

                // Extract text content from response
                let texts: Vec<String> = result
                    .content
                    .iter()
                    .filter_map(|item| match &item.raw {
                        rmcp::model::RawContent::Text(text) => Some(text.text.clone()),
                        _ => None,
                    })
                    .collect();

                let output = if texts.is_empty() {
                    serde_json::to_string(&result.content)
                        .unwrap_or_else(|_| "No content".to_string())
                } else {
                    texts.join("\n")
                };

                // Check if the server reported an error via is_error flag
                if result.is_error == Some(true) {
                    Err(output)
                } else {
                    Ok(output)
                }
            }

            TransportKind::Sse => {
                // `self.inner` is no longer borrowed here, so calling
                // `self.sse_send_request` (which takes `&mut self`) is safe.
                let params = serde_json::json!({
                    "name": raw_name,
                    "arguments": arguments,
                });

                let response = self.sse_send_request("tools/call", Some(params)).await?;

                match response {
                    Some(result) => {
                        if let Some(content) = result.get("content").and_then(|c| c.as_array()) {
                            let texts: Vec<&str> = content
                                .iter()
                                .filter_map(|item| {
                                    if item["type"].as_str() == Some("text") {
                                        item["text"].as_str()
                                    } else {
                                        None
                                    }
                                })
                                .collect();
                            Ok(texts.join("\n"))
                        } else {
                            Ok(result.to_string())
                        }
                    }
                    None => Err("No result from MCP tools/call".to_string()),
                }
            }

            TransportKind::HttpCompat => {
                // Clone the reqwest::Client so we can release the borrow of
                // self.inner before borrowing self.config (avoids E0502).
                let client = match &self.inner {
                    McpInner::HttpCompat { client } => client.clone(),
                    _ => unreachable!(),
                };

                if let McpTransport::HttpCompat {
                    base_url,
                    headers,
                    tools,
                } = &self.config.transport
                {
                    Self::call_http_compat_tool(
                        &client,
                        base_url,
                        headers,
                        tools,
                        raw_name.as_str(),
                        arguments,
                        self.config.timeout_secs,
                    )
                    .await
                } else {
                    Err("HttpCompat inner with non-HttpCompat transport config".to_string())
                }
            }
        }
    }

    /// Get the discovered tool definitions.
    pub fn tools(&self) -> &[ToolDefinition] {
        &self.tools
    }

    /// Get the server name.
    pub fn name(&self) -> &str {
        &self.config.name
    }

    // --- HttpCompat tool execution (unchanged) ---

    fn validate_http_compat_config(
        base_url: &str,
        headers: &[HttpCompatHeaderConfig],
        tools: &[HttpCompatToolConfig],
    ) -> Result<(), String> {
        if base_url.trim().is_empty() {
            return Err("HTTP compatibility transport requires non-empty base_url".to_string());
        }

        if tools.is_empty() {
            return Err("HTTP compatibility transport requires at least one tool".to_string());
        }

        for header in headers {
            if header.name.trim().is_empty() {
                return Err("HTTP compatibility headers must have non-empty names".to_string());
            }

            let has_static_value = header
                .value
                .as_ref()
                .is_some_and(|value| !value.trim().is_empty());
            let has_env_value = header
                .value_env
                .as_ref()
                .is_some_and(|value| !value.trim().is_empty());
            if !has_static_value && !has_env_value {
                return Err(format!(
                    "HTTP compatibility header '{}' must define either 'value' or 'value_env'",
                    header.name
                ));
            }
        }

        for tool in tools {
            if tool.name.trim().is_empty() {
                return Err("HTTP compatibility tools must have non-empty names".to_string());
            }
            if tool.path.trim().is_empty() {
                return Err(format!(
                    "HTTP compatibility tool '{}' must have a non-empty path",
                    tool.name
                ));
            }
        }

        Ok(())
    }

    async fn call_http_compat_tool(
        client: &reqwest::Client,
        base_url: &str,
        headers: &[HttpCompatHeaderConfig],
        tools: &[HttpCompatToolConfig],
        raw_name: &str,
        arguments: &serde_json::Value,
        timeout_secs: u64,
    ) -> Result<String, String> {
        let tool = tools
            .iter()
            .find(|tool| tool.name == raw_name)
            .ok_or_else(|| format!("HTTP compatibility tool not found: {raw_name}"))?;

        let (path, remaining_args) = Self::render_http_compat_path(&tool.path, arguments);
        let base = base_url.trim_end_matches('/');
        let full_url = if path.starts_with("http://") || path.starts_with("https://") {
            path
        } else if path.starts_with('/') {
            format!("{base}{path}")
        } else {
            format!("{base}/{path}")
        };

        let mut request = match tool.method {
            HttpCompatMethod::Get => client.get(full_url.as_str()),
            HttpCompatMethod::Post => client.post(full_url.as_str()),
            HttpCompatMethod::Put => client.put(full_url.as_str()),
            HttpCompatMethod::Patch => client.patch(full_url.as_str()),
            HttpCompatMethod::Delete => client.delete(full_url.as_str()),
        };

        request = request.timeout(std::time::Duration::from_secs(timeout_secs));
        request = Self::apply_http_compat_headers(request, headers)?;

        match tool.request_mode {
            HttpCompatRequestMode::JsonBody => {
                if !Self::is_empty_json_object(&remaining_args) {
                    request = request.json(&remaining_args);
                }
            }
            HttpCompatRequestMode::Query => {
                let pairs = Self::json_value_to_query_pairs(&remaining_args)?;
                if !pairs.is_empty() {
                    request = request.query(&pairs);
                }
            }
            HttpCompatRequestMode::None => {}
        }

        let response = request
            .send()
            .await
            .map_err(|e| format!("HTTP compatibility request failed: {e}"))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| format!("Failed to read HTTP compatibility response: {e}"))?;

        if !status.is_success() {
            return Err(format!(
                "{} {} -> HTTP {}: {}",
                Self::http_method_name(&tool.method),
                full_url,
                status.as_u16(),
                body
            ));
        }

        Ok(Self::format_http_compat_response(
            &body,
            &tool.response_mode,
        ))
    }

    fn render_http_compat_path(
        path_template: &str,
        arguments: &serde_json::Value,
    ) -> (String, serde_json::Value) {
        let Some(args_obj) = arguments.as_object() else {
            return (path_template.to_string(), arguments.clone());
        };

        let mut rendered = path_template.to_string();
        let mut remaining = args_obj.clone();

        for (key, value) in args_obj {
            let placeholder = format!("{{{key}}}");
            if rendered.contains(&placeholder) {
                let replacement = match value {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                let encoded = Self::encode_http_compat_path_value(&replacement);
                rendered = rendered.replace(&placeholder, &encoded);
                remaining.remove(key);
            }
        }

        (rendered, serde_json::Value::Object(remaining))
    }

    fn encode_http_compat_path_value(value: &str) -> String {
        let mut encoded = String::with_capacity(value.len());
        for byte in value.bytes() {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                    encoded.push(char::from(byte))
                }
                _ => {
                    const HEX: &[u8; 16] = b"0123456789ABCDEF";
                    encoded.push('%');
                    encoded.push(char::from(HEX[(byte >> 4) as usize]));
                    encoded.push(char::from(HEX[(byte & 0x0F) as usize]));
                }
            }
        }
        encoded
    }

    fn apply_http_compat_headers(
        mut request: reqwest::RequestBuilder,
        headers: &[HttpCompatHeaderConfig],
    ) -> Result<reqwest::RequestBuilder, String> {
        for header in headers {
            let value = if let Some(value) = &header.value {
                value.clone()
            } else if let Some(value_env) = &header.value_env {
                std::env::var(value_env).map_err(|_| {
                    format!(
                        "Missing environment variable '{}' for HTTP compatibility header '{}'",
                        value_env, header.name
                    )
                })?
            } else {
                return Err(format!(
                    "HTTP compatibility header '{}' must define either 'value' or 'value_env'",
                    header.name
                ));
            };

            request = request.header(header.name.as_str(), value);
        }

        Ok(request)
    }

    fn json_value_to_query_pairs(
        value: &serde_json::Value,
    ) -> Result<Vec<(String, String)>, String> {
        let Some(args_obj) = value.as_object() else {
            if value.is_null() {
                return Ok(Vec::new());
            }
            return Err("HTTP compatibility query mode requires object arguments".to_string());
        };

        let mut pairs = Vec::with_capacity(args_obj.len());
        for (key, value) in args_obj {
            if value.is_null() {
                continue;
            }
            let rendered = match value {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::Bool(b) => b.to_string(),
                other => serde_json::to_string(other)
                    .map_err(|e| format!("Failed to serialize query value for '{key}': {e}"))?,
            };
            pairs.push((key.clone(), rendered));
        }
        Ok(pairs)
    }

    fn format_http_compat_response(body: &str, response_mode: &HttpCompatResponseMode) -> String {
        if body.trim().is_empty() {
            return "{}".to_string();
        }

        match response_mode {
            HttpCompatResponseMode::Text => body.to_string(),
            HttpCompatResponseMode::Json => serde_json::from_str::<serde_json::Value>(body)
                .ok()
                .and_then(|value| serde_json::to_string_pretty(&value).ok())
                .unwrap_or_else(|| body.to_string()),
        }
    }

    fn is_empty_json_object(value: &serde_json::Value) -> bool {
        value.is_null() || value.as_object().is_some_and(|obj| obj.is_empty())
    }

    fn http_method_name(method: &HttpCompatMethod) -> &'static str {
        match method {
            HttpCompatMethod::Get => "GET",
            HttpCompatMethod::Post => "POST",
            HttpCompatMethod::Put => "PUT",
            HttpCompatMethod::Patch => "PATCH",
            HttpCompatMethod::Delete => "DELETE",
        }
    }
}

// ---------------------------------------------------------------------------
// Tool namespacing helpers
// ---------------------------------------------------------------------------

/// Format a namespaced MCP tool name: `mcp_{server}_{tool}`.
pub fn format_mcp_tool_name(server: &str, tool: &str) -> String {
    format!("mcp_{}_{}", normalize_name(server), normalize_name(tool))
}

/// Check if a tool name is an MCP-namespaced tool.
pub fn is_mcp_tool(name: &str) -> bool {
    name.starts_with("mcp_")
}

/// Extract the normalized server name from an MCP tool name.
///
/// **Warning**: This heuristic splits on the first `_` after the `mcp_` prefix,
/// so it only works for single-word server names (e.g. `"github"`). For server
/// names that contain hyphens or underscores (e.g. `"my-server"` →
/// `"mcp_my_server_tool"`), this returns only the first segment (`"my"`).
///
/// Prefer [`resolve_mcp_server_from_known`] when the list of configured server
/// names is available.
pub fn extract_mcp_server(tool_name: &str) -> Option<&str> {
    if !tool_name.starts_with("mcp_") {
        return None;
    }
    let rest = &tool_name[4..];
    rest.find('_').map(|pos| &rest[..pos])
}

/// Strip the MCP namespace prefix from a tool name.
fn strip_mcp_prefix<'a>(server: &str, tool_name: &'a str) -> Option<&'a str> {
    let prefix = format!("mcp_{}_", normalize_name(server));
    tool_name.strip_prefix(&prefix)
}

/// Resolve the original server name for a namespaced MCP tool using known servers.
///
/// This is the robust variant for runtime dispatch because server names are normalized
/// into the tool namespace and may themselves contain underscores.
pub fn resolve_mcp_server_from_known<'a>(
    tool_name: &str,
    server_names: impl IntoIterator<Item = &'a str>,
) -> Option<&'a str> {
    let mut best_match: Option<&'a str> = None;
    let mut best_len = 0usize;

    for server_name in server_names {
        let normalized = normalize_name(server_name);
        let prefix = format!("mcp_{}_", normalized);
        if tool_name.starts_with(&prefix) && prefix.len() > best_len {
            best_len = prefix.len();
            best_match = Some(server_name);
        }
    }

    best_match
}

/// Normalize a name for use in tool namespacing (lowercase, replace hyphens).
pub fn normalize_name(name: &str) -> String {
    name.to_lowercase().replace('-', "_")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn test_mcp_tool_namespacing() {
        assert_eq!(
            format_mcp_tool_name("github", "create_issue"),
            "mcp_github_create_issue"
        );
        assert_eq!(
            format_mcp_tool_name("my-server", "do_thing"),
            "mcp_my_server_do_thing"
        );
    }

    #[test]
    fn test_is_mcp_tool() {
        assert!(is_mcp_tool("mcp_github_create_issue"));
        assert!(!is_mcp_tool("file_read"));
        assert!(!is_mcp_tool(""));
    }

    #[test]
    fn test_hyphenated_tool_name_preserved() {
        let namespaced = format_mcp_tool_name("sqlcl", "list-connections");
        assert_eq!(namespaced, "mcp_sqlcl_list_connections");

        let mut original_names = HashMap::new();
        original_names.insert(namespaced.clone(), "list-connections".to_string());

        let raw = original_names
            .get(&namespaced)
            .map(|s| s.as_str())
            .unwrap_or("list_connections");
        assert_eq!(raw, "list-connections");
    }

    #[test]
    fn test_extract_mcp_server() {
        assert_eq!(
            extract_mcp_server("mcp_github_create_issue"),
            Some("github")
        );
        assert_eq!(extract_mcp_server("file_read"), None);
    }

    #[test]
    fn test_resolve_mcp_server_from_known_prefers_longest_prefix() {
        let server = resolve_mcp_server_from_known(
            "mcp_http_tools_fetch_item",
            ["http", "http-tools", "http-tools-extra"],
        );
        assert_eq!(server, Some("http-tools"));
    }

    #[test]
    fn test_resolve_mcp_server_hyphenated_name() {
        let server =
            resolve_mcp_server_from_known("mcp_bocha_test_search", ["github", "bocha-test"]);
        assert_eq!(server, Some("bocha-test"));

        let server =
            resolve_mcp_server_from_known("mcp_github_create_issue", ["github", "bocha-test"]);
        assert_eq!(server, Some("github"));
    }

    #[test]
    fn test_hyphenated_server_tool_namespacing_roundtrip() {
        let servers = ["my-server", "another-mcp-server", "simple"];
        let tool_name = format_mcp_tool_name("my-server", "do_thing");
        assert_eq!(tool_name, "mcp_my_server_do_thing");

        let resolved = resolve_mcp_server_from_known(&tool_name, servers);
        assert_eq!(resolved, Some("my-server"));

        let tool_name = format_mcp_tool_name("another-mcp-server", "action");
        assert_eq!(tool_name, "mcp_another_mcp_server_action");

        let resolved = resolve_mcp_server_from_known(&tool_name, servers);
        assert_eq!(resolved, Some("another-mcp-server"));
    }

    #[test]
    fn test_mcp_jsonrpc_initialize() {
        let request = JsonRpcRequest {
            jsonrpc: "2.0",
            id: 1,
            method: "initialize".to_string(),
            params: Some(serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "librefang",
                    "version": librefang_types::VERSION
                }
            })),
        };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("initialize"));
        assert!(json.contains("protocolVersion"));
        assert!(json.contains("librefang"));
    }

    #[test]
    fn test_mcp_jsonrpc_tools_list() {
        let response_json = r#"{
            "jsonrpc": "2.0",
            "id": 2,
            "result": {
                "tools": [
                    {
                        "name": "create_issue",
                        "description": "Create a GitHub issue",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "title": {"type": "string"},
                                "body": {"type": "string"}
                            },
                            "required": ["title"]
                        }
                    }
                ]
            }
        }"#;

        let response: JsonRpcResponse = serde_json::from_str(response_json).unwrap();
        assert!(response.error.is_none());
        let result = response.result.unwrap();
        let tools = result["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"].as_str().unwrap(), "create_issue");
    }

    #[test]
    fn test_mcp_transport_config_serde() {
        let config = McpServerConfig {
            name: "github".to_string(),
            transport: McpTransport::Stdio {
                command: "npx".to_string(),
                args: vec![
                    "-y".to_string(),
                    "@modelcontextprotocol/server-github".to_string(),
                ],
            },
            timeout_secs: 30,
            env: vec![
                "GITHUB_PERSONAL_ACCESS_TOKEN=ghp_test123".to_string(),
                "LEGACY_NAME_ONLY".to_string(),
            ],
        };

        let json = serde_json::to_string(&config).unwrap();
        let back: McpServerConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, "github");
        assert_eq!(back.timeout_secs, 30);
        assert_eq!(back.env.len(), 2);
        assert_eq!(back.env[0], "GITHUB_PERSONAL_ACCESS_TOKEN=ghp_test123");
        assert_eq!(back.env[1], "LEGACY_NAME_ONLY");

        match back.transport {
            McpTransport::Stdio { command, args } => {
                assert_eq!(command, "npx");
                assert_eq!(args.len(), 2);
            }
            _ => panic!("Expected Stdio transport"),
        }

        // SSE variant
        let sse_config = McpServerConfig {
            name: "test".to_string(),
            transport: McpTransport::Sse {
                url: "https://example.com/mcp".to_string(),
            },
            timeout_secs: 60,
            env: vec![],
        };
        let json = serde_json::to_string(&sse_config).unwrap();
        let back: McpServerConfig = serde_json::from_str(&json).unwrap();
        match back.transport {
            McpTransport::Sse { url } => assert_eq!(url, "https://example.com/mcp"),
            _ => panic!("Expected SSE transport"),
        }

        // HTTP compatibility variant
        let http_compat_config = McpServerConfig {
            name: "http-tools".to_string(),
            transport: McpTransport::HttpCompat {
                base_url: "http://127.0.0.1:11235".to_string(),
                headers: vec![HttpCompatHeaderConfig {
                    name: "Authorization".to_string(),
                    value: None,
                    value_env: Some("HTTP_TOOLS_TOKEN".to_string()),
                }],
                tools: vec![HttpCompatToolConfig {
                    name: "search".to_string(),
                    description: "Search over an HTTP backend".to_string(),
                    path: "/search".to_string(),
                    method: HttpCompatMethod::Get,
                    request_mode: HttpCompatRequestMode::Query,
                    response_mode: HttpCompatResponseMode::Json,
                    input_schema: serde_json::json!({"type": "object"}),
                }],
            },
            timeout_secs: 45,
            env: vec![],
        };
        let json = serde_json::to_string(&http_compat_config).unwrap();
        let back: McpServerConfig = serde_json::from_str(&json).unwrap();
        match back.transport {
            McpTransport::HttpCompat {
                base_url,
                headers,
                tools,
            } => {
                assert_eq!(base_url, "http://127.0.0.1:11235");
                assert_eq!(headers.len(), 1);
                assert_eq!(tools.len(), 1);
                assert_eq!(tools[0].name, "search");
            }
            _ => panic!("Expected HttpCompat transport"),
        }
    }

    #[test]
    fn test_env_key_value_parsing() {
        let entry = "MY_KEY=my_value";
        let (key, value) = entry.split_once('=').unwrap();
        assert_eq!(key, "MY_KEY");
        assert_eq!(value, "my_value");

        let entry = "TOKEN=abc=def==";
        let (key, value) = entry.split_once('=').unwrap();
        assert_eq!(key, "TOKEN");
        assert_eq!(value, "abc=def==");

        let entry = "PLAIN_NAME";
        assert!(entry.split_once('=').is_none());
    }

    #[test]
    fn test_http_compat_tool_registration() {
        let mut conn = McpConnection {
            config: McpServerConfig {
                name: "http-tools".to_string(),
                transport: McpTransport::HttpCompat {
                    base_url: "http://127.0.0.1:8080".to_string(),
                    headers: vec![],
                    tools: vec![],
                },
                timeout_secs: 30,
                env: vec![],
            },
            tools: Vec::new(),
            original_names: HashMap::new(),
            inner: McpInner::HttpCompat {
                client: crate::http_client::proxied_client(),
            },
        };

        conn.register_http_compat_tools(&[
            HttpCompatToolConfig {
                name: "search".to_string(),
                description: "Search backend".to_string(),
                path: "/search".to_string(),
                method: HttpCompatMethod::Get,
                request_mode: HttpCompatRequestMode::Query,
                response_mode: HttpCompatResponseMode::Json,
                input_schema: serde_json::json!({"type": "object"}),
            },
            HttpCompatToolConfig {
                name: "create_item".to_string(),
                description: String::new(),
                path: "/items".to_string(),
                method: HttpCompatMethod::Post,
                request_mode: HttpCompatRequestMode::JsonBody,
                response_mode: HttpCompatResponseMode::Json,
                input_schema: serde_json::json!({"type": "object"}),
            },
        ]);

        let tool_names: Vec<&str> = conn.tools.iter().map(|tool| tool.name.as_str()).collect();
        assert!(tool_names.contains(&"mcp_http_tools_search"));
        assert!(tool_names.contains(&"mcp_http_tools_create_item"));
        assert_eq!(
            conn.original_names
                .get("mcp_http_tools_create_item")
                .map(String::as_str),
            Some("create_item")
        );
    }

    #[test]
    fn test_http_compat_path_rendering() {
        let arguments = serde_json::json!({
            "team_id": "core platform",
            "doc_id": "folder/42",
            "include_meta": true,
        });

        let (path, remaining) =
            McpConnection::render_http_compat_path("/teams/{team_id}/docs/{doc_id}", &arguments);

        assert_eq!(path, "/teams/core%20platform/docs/folder%2F42");
        assert_eq!(remaining, serde_json::json!({ "include_meta": true }));
    }

    #[test]
    fn test_http_compat_query_pairs() {
        let pairs = McpConnection::json_value_to_query_pairs(&serde_json::json!({
            "q": "hello",
            "limit": 10,
            "exact": false,
        }))
        .unwrap();

        assert!(pairs.contains(&(String::from("q"), String::from("hello"))));
        assert!(pairs.contains(&(String::from("limit"), String::from("10"))));
        assert!(pairs.contains(&(String::from("exact"), String::from("false"))));
    }

    #[test]
    fn test_http_compat_invalid_config_rejected() {
        let err = McpConnection::validate_http_compat_config(
            "http://127.0.0.1:8080",
            &[HttpCompatHeaderConfig {
                name: "Authorization".to_string(),
                value: None,
                value_env: None,
            }],
            &[HttpCompatToolConfig {
                name: "search".to_string(),
                description: String::new(),
                path: "/search".to_string(),
                method: HttpCompatMethod::Get,
                request_mode: HttpCompatRequestMode::Query,
                response_mode: HttpCompatResponseMode::Json,
                input_schema: serde_json::json!({"type": "object"}),
            }],
        )
        .unwrap_err();

        assert!(err.contains("value") || err.contains("value_env"));
    }

    #[tokio::test]
    async fn test_http_compat_end_to_end() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            for request_index in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut buffer = vec![0_u8; 4096];
                let bytes = stream.read(&mut buffer).await.unwrap();
                let request = String::from_utf8_lossy(&buffer[..bytes]).to_string();
                let request_line = request.lines().next().unwrap_or_default().to_string();

                if request_index == 0 {
                    assert_eq!(request_line, "GET / HTTP/1.1");
                    stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                        )
                        .await
                        .unwrap();
                    continue;
                }

                assert!(request_line.starts_with("GET /items/folder%2F42?"));
                assert!(request_line.contains("q=hello+world"));
                assert!(request_line.contains("limit=2"));
                assert!(request.to_ascii_lowercase().contains("x-test: yes\r\n"));

                let body = r#"{"ok":true,"source":"http_compat"}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });

        let mut conn = McpConnection::connect(McpServerConfig {
            name: "http-tools".to_string(),
            transport: McpTransport::HttpCompat {
                base_url: format!("http://{}", addr),
                headers: vec![HttpCompatHeaderConfig {
                    name: "X-Test".to_string(),
                    value: Some("yes".to_string()),
                    value_env: None,
                }],
                tools: vec![HttpCompatToolConfig {
                    name: "fetch_item".to_string(),
                    description: "Fetch item over HTTP".to_string(),
                    path: "/items/{id}".to_string(),
                    method: HttpCompatMethod::Get,
                    request_mode: HttpCompatRequestMode::Query,
                    response_mode: HttpCompatResponseMode::Json,
                    input_schema: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "id": { "type": "string" },
                            "q": { "type": "string" },
                            "limit": { "type": "integer" }
                        },
                        "required": ["id"]
                    }),
                }],
            },
            timeout_secs: 5,
            env: vec![],
        })
        .await
        .unwrap();

        let result = conn
            .call_tool(
                "mcp_http_tools_fetch_item",
                &serde_json::json!({
                    "id": "folder/42",
                    "q": "hello world",
                    "limit": 2
                }),
            )
            .await
            .unwrap();

        assert!(result.contains("\"ok\": true"));
        assert!(result.contains("\"source\": \"http_compat\""));

        server.await.unwrap();
    }

    #[test]
    fn test_safe_env_vars_contains_essentials() {
        assert!(SAFE_ENV_VARS.contains(&"PATH"));
        assert!(SAFE_ENV_VARS.contains(&"HOME"));
        assert!(SAFE_ENV_VARS.contains(&"TERM"));
    }

    #[test]
    fn test_ssrf_check() {
        assert!(
            McpConnection::check_ssrf("http://169.254.169.254/latest/meta-data", "test").is_err()
        );
        assert!(McpConnection::check_ssrf("http://metadata.google.internal/v1/", "test").is_err());
        assert!(McpConnection::check_ssrf("https://api.example.com/mcp", "test").is_ok());
    }
}
