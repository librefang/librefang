//! MCP (Model Context Protocol) client — connect to external MCP servers.
//!
//! Stdio transport uses the rmcp SDK for proper MCP protocol handling.
//! SSE transport uses HTTP POST with JSON-RPC for backward compatibility.
//! HttpCompat provides a built-in adapter for plain HTTP/JSON backends.
//!
//! All MCP tools are namespaced with `mcp_{server}_{tool}` to prevent collisions.

pub mod mcp_oauth;

use http::{HeaderName, HeaderValue};
use librefang_types::config::{
    HttpCompatHeaderConfig, HttpCompatMethod, HttpCompatRequestMode, HttpCompatResponseMode,
    HttpCompatToolConfig,
};
use librefang_types::config::{
    McpTaintPolicy, McpTaintRuleSetAction, McpTaintToolAction, NamedTaintRuleSet,
};
use librefang_types::taint::{detect_outbound_text_violation_with_skip, TaintRuleId, TaintSink};
use librefang_types::tool::ToolDefinition;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Maximum JSON nesting depth the taint scanner will traverse. Anything
/// deeper is rejected outright so a pathological payload can't blow the
/// stack or pin CPU. 64 is well beyond any sane tool-call shape.
const MCP_TAINT_SCAN_MAX_DEPTH: usize = 64;

/// Object keys that, when present in an MCP argument tree with a
/// non-empty string value, are treated as credential-shaped
/// regardless of what the value looks like. Catches the common
/// shape `{"headers": {"Authorization": "Bearer …"}}` that the
/// value-only text heuristic misses (whitespace + scheme word).
const MCP_SENSITIVE_KEY_NAMES: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "api_key",
    "apikey",
    "api-key",
    "x-api-key",
    "access_token",
    "accesstoken",
    "refresh_token",
    "bearer",
    "password",
    "passwd",
    "secret",
    "client_secret",
    "private_key",
];

fn is_sensitive_key_name(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    MCP_SENSITIVE_KEY_NAMES.iter().any(|k| lower == *k)
}

// ── Minimal JSONPath matching ───────────────────────────────────────────────

/// Returns `true` if a dot-separated JSONPath `pattern` (as stored in
/// `McpTaintPolicy`) matches the given `path` built by the walker.
///
/// Supported syntax:
/// - `$.a.b`   — exact nested property
/// - `$.a.*`   — any direct child of `$.a`
/// - `$.a[*]`  — any array element of `$.a`
/// - `$.*`     — any top-level property
fn jsonpath_matches(pattern: &str, path: &str) -> bool {
    if pattern == path {
        return true;
    }
    let p_segs: Vec<&str> = split_jsonpath(pattern);
    let h_segs: Vec<&str> = split_jsonpath(path);
    segs_match(&p_segs, &h_segs)
}

fn split_jsonpath(p: &str) -> Vec<&str> {
    // Split on '.' but preserve array notation like `items[0]` as one segment.
    let mut out = Vec::new();
    let mut start = 0;
    for (i, b) in p.bytes().enumerate() {
        if b == b'.' && i > 0 {
            out.push(&p[start..i]);
            start = i + 1;
        }
    }
    out.push(&p[start..]);
    out
}

fn segs_match(pattern: &[&str], path: &[&str]) -> bool {
    match (pattern, path) {
        ([], []) => true,
        ([], _) | (_, []) => false,
        ([p, pr @ ..], [h, hr @ ..]) => {
            let ok =
                *p == *h || (*p == "*" && !h.contains('[')) || seg_array_wildcard_matches(p, h);
            ok && segs_match(pr, hr)
        }
    }
}

/// Checks whether a pattern segment ending in `[*]` (e.g. `items[*]`)
/// matches a path segment with a concrete index (e.g. `items[0]`).
fn seg_array_wildcard_matches(pattern: &str, path: &str) -> bool {
    let Some(prefix) = pattern.strip_suffix("[*]") else {
        return false;
    };
    if !path.starts_with(prefix) {
        return false;
    }
    let rest = &path[prefix.len()..];
    rest.starts_with('[')
        && rest.ends_with(']')
        && rest[1..rest.len() - 1].chars().all(|c| c.is_ascii_digit())
}

/// Collect all `TaintRuleId`s that should be skipped for a specific tool +
/// argument path according to the server's `McpTaintPolicy`.
///
/// Returns an empty set when `policy` is `None` or the tool/path have no
/// matching exemption entries — i.e. all rules apply.
fn resolve_skip_rules(
    policy: Option<&McpTaintPolicy>,
    tool_name: &str,
    json_path: &str,
) -> std::collections::HashSet<TaintRuleId> {
    let mut skip = std::collections::HashSet::new();
    let Some(policy) = policy else {
        return skip;
    };
    let Some(tool_policy) = policy.tools.get(tool_name) else {
        return skip;
    };
    for (pattern, path_policy) in &tool_policy.paths {
        if jsonpath_matches(pattern, json_path) {
            for rule in &path_policy.skip_rules {
                skip.insert(rule.clone());
            }
        }
    }
    skip
}

/// Look up the [`McpTaintRuleSetAction`] (and rule set name) for a rule fired
/// during scanning. Returns the *most permissive* action across all rule sets
/// referenced by `tool_name` that contain `rule`, in order: `Log` > `Warn` >
/// `Block`. `Block` is the implicit baseline and is returned only when an
/// explicit `block`-action set names the rule (so callers can still surface
/// the rule-set name in tracing if they want).
///
/// Returns `None` when no referenced rule set covers the rule, in which case
/// the caller should block (default scanner behaviour).
fn lookup_rule_set_action<'a>(
    policy: Option<&McpTaintPolicy>,
    tool_name: &str,
    rule: &TaintRuleId,
    registry: &'a [NamedTaintRuleSet],
) -> Option<(McpTaintRuleSetAction, &'a str)> {
    let tool_policy = policy?.tools.get(tool_name)?;
    if tool_policy.rule_sets.is_empty() || registry.is_empty() {
        return None;
    }
    let mut best: Option<(McpTaintRuleSetAction, &str)> = None;
    for set_name in &tool_policy.rule_sets {
        let Some(set) = registry.iter().find(|s| &s.name == set_name) else {
            continue;
        };
        if !set.rules.contains(rule) {
            continue;
        }
        let candidate = (set.action, set.name.as_str());
        best = Some(match best {
            None => candidate,
            Some(prev) => {
                if action_priority(set.action) > action_priority(prev.0) {
                    candidate
                } else {
                    prev
                }
            }
        });
    }
    best
}

/// Higher value = more permissive (further from `block`). Used to merge
/// rule-set actions when a tool references multiple sets that cover the
/// same rule.
fn action_priority(a: McpTaintRuleSetAction) -> u8 {
    match a {
        McpTaintRuleSetAction::Block => 0,
        McpTaintRuleSetAction::Warn => 1,
        McpTaintRuleSetAction::Log => 2,
    }
}

/// Decide whether a rule fire should be downgraded from `block` and emit the
/// matching tracing event. Returns `true` to continue blocking, `false` to
/// allow the call through (warn / log).
fn apply_rule_set_action(
    policy: Option<&McpTaintPolicy>,
    tool_name: &str,
    rule: &TaintRuleId,
    json_path: &str,
    registry: &[NamedTaintRuleSet],
) -> bool {
    let Some((action, set_name)) = lookup_rule_set_action(policy, tool_name, rule, registry) else {
        return true;
    };
    match action {
        McpTaintRuleSetAction::Block => true,
        McpTaintRuleSetAction::Warn => {
            warn!(
                target: "librefang_runtime_mcp::taint",
                rule = ?rule,
                rule_set = %set_name,
                tool = %tool_name,
                path = %json_path,
                "MCP taint rule fired but downgraded by rule_set (action=warn)"
            );
            false
        }
        McpTaintRuleSetAction::Log => {
            info!(
                target: "librefang_runtime_mcp::taint",
                rule = ?rule,
                rule_set = %set_name,
                tool = %tool_name,
                path = %json_path,
                "MCP taint rule fired and audited by rule_set (action=log)"
            );
            false
        }
    }
}

// ── Taint scanner ──────────────────────────────────────────────────────────

/// Walk every string leaf in a JSON argument tree and check it against
/// `TaintSink::mcp_tool_call`.  Returns a *redacted* rule description
/// (JSON path + rule name) if any leaf trips the denylist, `None` otherwise.
///
/// When `taint_policy` and `tool_name` are supplied, per-path skip rules
/// from the policy are applied before calling the underlying detector.
/// Named rule sets in `rule_set_registry` referenced by the tool's policy
/// can downgrade `Block` to `Warn` / `Log` — when a downgrade applies, the
/// rule fires only as a tracing event and the call is allowed through.
///
/// If the tool's policy has `default = Skip`, scanning is bypassed
/// entirely for this tool — see [`scan_mcp_arguments_for_taint_with_policy`].
///
/// IMPORTANT: the returned string must NOT contain the offending payload.
/// It flows back to the LLM as an error and is emitted to logs — echoing
/// the secret we just blocked would defeat the filter. Only the JSON path
/// of the offending leaf is surfaced.
///
/// Non-string leaves (numbers, bools, null) are skipped.
///
/// Recursion is hard-capped at [`MCP_TAINT_SCAN_MAX_DEPTH`].
#[cfg(test)]
fn scan_mcp_arguments_for_taint(value: &serde_json::Value) -> Option<String> {
    scan_mcp_arguments_for_taint_with_policy(value, None, &[], "")
}

fn scan_mcp_arguments_for_taint_with_policy(
    value: &serde_json::Value,
    taint_policy: Option<&McpTaintPolicy>,
    rule_set_registry: &[NamedTaintRuleSet],
    tool_name: &str,
) -> Option<String> {
    // Tool-level kill switch: `default = "skip"` bypasses scanning for the
    // entire tool, including sensitive object-key blocking. This is the
    // single-line equivalent of disabling scanning on noisy tools.
    if let Some(policy) = taint_policy {
        if let Some(tool_policy) = policy.tools.get(tool_name) {
            if tool_policy.default == McpTaintToolAction::Skip {
                debug!(
                    target: "librefang_runtime_mcp::taint",
                    tool = %tool_name,
                    "MCP taint scanning bypassed: tool policy default=skip"
                );
                return None;
            }
        }
    }
    let sink = TaintSink::mcp_tool_call();
    walk_taint(
        value,
        &sink,
        "$",
        0,
        taint_policy,
        rule_set_registry,
        tool_name,
    )
}

fn walk_taint(
    v: &serde_json::Value,
    sink: &TaintSink,
    path: &str,
    depth: usize,
    policy: Option<&McpTaintPolicy>,
    rule_set_registry: &[NamedTaintRuleSet],
    tool_name: &str,
) -> Option<String> {
    if depth > MCP_TAINT_SCAN_MAX_DEPTH {
        return Some(format!(
            "taint violation: MCP argument tree exceeds max depth {} at '{}'",
            MCP_TAINT_SCAN_MAX_DEPTH, path
        ));
    }

    let skip = resolve_skip_rules(policy, tool_name, path);

    match v {
        serde_json::Value::String(s) => {
            // Discard the underlying violation string entirely — it may be
            // derived from the payload — and report only the JSON path.
            if let Some(rule) = detect_outbound_text_violation_with_skip(s, sink, &skip) {
                if apply_rule_set_action(policy, tool_name, &rule, path, rule_set_registry) {
                    return Some(format!(
                        "taint violation: sensitive value in MCP argument '{}' (blocked by sink '{}')",
                        path, sink.name
                    ));
                }
            }
            None
        }
        serde_json::Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                let child = format!("{path}[{i}]");
                if let Some(v) = walk_taint(
                    item,
                    sink,
                    &child,
                    depth + 1,
                    policy,
                    rule_set_registry,
                    tool_name,
                ) {
                    return Some(v);
                }
            }
            None
        }
        serde_json::Value::Object(obj) => {
            for (k, val) in obj {
                let child = format!("{path}.{k}");
                // SensitiveKeyName is a property of the key's own path (child),
                // not the parent path, so resolve skip rules against `child`.
                let child_skip = resolve_skip_rules(policy, tool_name, &child);
                // Credential-shaped object key with a non-empty string value
                // is an unambiguous outbound credential, regardless of the
                // value shape (e.g. `"Authorization": "Bearer sk-…"` has
                // whitespace and wouldn't trip the text heuristic alone).
                if is_sensitive_key_name(k) && !child_skip.contains(&TaintRuleId::SensitiveKeyName)
                {
                    if let serde_json::Value::String(s) = val {
                        if !s.trim().is_empty()
                            && apply_rule_set_action(
                                policy,
                                tool_name,
                                &TaintRuleId::SensitiveKeyName,
                                &child,
                                rule_set_registry,
                            )
                        {
                            return Some(format!(
                                "taint violation: sensitive MCP argument key at '{}' (blocked by sink '{}')",
                                child, sink.name
                            ));
                        }
                    }
                }
                if let Some(v) = walk_taint(
                    val,
                    sink,
                    &child,
                    depth + 1,
                    policy,
                    rule_set_registry,
                    tool_name,
                ) {
                    return Some(v);
                }
            }
            None
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Configuration types
// ---------------------------------------------------------------------------

/// Configuration for an MCP server connection.
#[derive(Serialize, Deserialize)]
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
    /// Extra HTTP headers to send with every SSE / Streamable-HTTP request.
    /// Each entry is `"Header-Name: value"`.  Useful for authentication
    /// (`Authorization: Bearer <token>`), API keys (`X-Api-Key: ...`),
    /// or any custom headers required by a remote MCP server.
    #[serde(default)]
    pub headers: Vec<String>,
    /// Optional OAuth provider for automatic authentication.
    #[serde(skip)]
    pub oauth_provider: Option<std::sync::Arc<dyn crate::mcp_oauth::McpOAuthProvider>>,
    /// Optional OAuth config from config.toml (discovery fallback).
    #[serde(default)]
    pub oauth_config: Option<librefang_types::config::McpOAuthConfig>,
    /// Enable outbound taint scanning for this MCP server (default: true).
    ///
    /// When `false`, the credential/PII heuristic is skipped for arguments
    /// sent to this server. This is an escape hatch for trusted local servers
    /// (browser automation, database adapters, …) whose tool results contain
    /// opaque session handles that would otherwise trip the credential heuristic.
    ///
    /// Key-name blocking (`Authorization`, `secret`, …) remains active even
    /// when this is `false` — only the content-based heuristic is disabled.
    #[serde(default = "default_taint_scanning")]
    pub taint_scanning: bool,
    /// Fine-grained per-tool, per-path taint exemptions.
    ///
    /// When set, individual taint rules can be disabled for specific argument
    /// paths in specific tools rather than disabling scanning entirely.
    /// Ignored when `taint_scanning = false`.
    #[serde(default)]
    pub taint_policy: Option<McpTaintPolicy>,
    /// Snapshot of the kernel's named taint rule sets, propagated at boot
    /// so the scanner can downgrade `Block` to `Warn` / `Log` for rules
    /// covered by sets referenced from this server's
    /// [`McpTaintPolicy::tools`] entries.
    ///
    /// Empty by default; the kernel populates this from
    /// [`librefang_types::config::KernelConfig::taint_rules`] before
    /// installing the server.
    #[serde(default)]
    pub taint_rule_sets: Vec<NamedTaintRuleSet>,
    /// Root directories advertised to this MCP server via the MCP Roots capability.
    ///
    /// Each entry is an absolute path (e.g. `/home/user/project`).  librefang
    /// converts these to `file://` URIs and declares `roots` in the client
    /// capabilities during the MCP `initialize` handshake. Servers that support
    /// Roots use this list to scope their file-system operations rather than
    /// falling back to their own hard-coded allowed-directories list.
    ///
    /// This field is populated at runtime by the kernel (home dir + agent
    /// workspaces dir) and is never serialised to / deserialised from config.
    #[serde(skip)]
    pub roots: Vec<String>,
}

impl std::fmt::Debug for McpServerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpServerConfig")
            .field("name", &self.name)
            .field("transport", &self.transport)
            .field("timeout_secs", &self.timeout_secs)
            .field("env", &self.env)
            .field("headers", &self.headers)
            .field(
                "oauth_provider",
                &self.oauth_provider.as_ref().map(|_| "..."),
            )
            .field("oauth_config", &self.oauth_config)
            .field("taint_scanning", &self.taint_scanning)
            .field("taint_policy", &self.taint_policy)
            .field("roots", &self.roots)
            .finish()
    }
}

impl Clone for McpServerConfig {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            transport: self.transport.clone(),
            timeout_secs: self.timeout_secs,
            env: self.env.clone(),
            headers: self.headers.clone(),
            oauth_provider: self.oauth_provider.clone(),
            oauth_config: self.oauth_config.clone(),
            taint_scanning: self.taint_scanning,
            taint_policy: self.taint_policy.clone(),
            taint_rule_sets: self.taint_rule_sets.clone(),
            roots: self.roots.clone(),
        }
    }
}

fn default_timeout() -> u64 {
    60
}

fn default_taint_scanning() -> bool {
    true
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
    /// Streamable HTTP transport (MCP 2025-03-26+).
    /// Single endpoint, client sends Accept: application/json, text/event-stream.
    /// Supports Mcp-Session-Id for session management.
    Http { url: String },
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

/// MCP client handler that declares the `roots` capability and responds to
/// `roots/list` requests with a pre-configured list of root directories.
struct RootsClientHandler {
    client_info: rmcp::model::ClientInfo,
    roots: Arc<Vec<rmcp::model::Root>>,
}

impl RootsClientHandler {
    fn new(roots: Vec<String>) -> Self {
        let mcp_roots: Vec<rmcp::model::Root> = roots
            .iter()
            .map(|path| {
                // Use the `url` crate to build a well-formed file URI so that
                // reserved characters (spaces, #, %) are percent-encoded and
                // the Windows drive-letter triple-slash form is handled
                // correctly.  Fall back to the raw string if the path is
                // already a URI or cannot be parsed as a filesystem path.
                let uri = if path.starts_with("file://") {
                    path.clone()
                } else {
                    url::Url::from_file_path(path)
                        .map(|u| u.to_string())
                        .unwrap_or_else(|_| {
                            // Url::from_file_path requires an absolute path;
                            // for relative or exotic paths fall back gracefully.
                            let forward = path.replace('\\', "/");
                            if forward.starts_with('/') {
                                format!("file://{forward}")
                            } else {
                                format!("file:///{forward}")
                            }
                        })
                };
                let name = std::path::Path::new(path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s.to_string());
                let mut root = rmcp::model::Root::new(uri);
                if let Some(n) = name {
                    root = root.with_name(n);
                }
                root
            })
            .collect();

        let mut capabilities = rmcp::model::ClientCapabilities::default();
        capabilities.roots = Some(rmcp::model::RootsCapabilities { list_changed: None });

        let client_info = rmcp::model::ClientInfo::new(
            capabilities,
            rmcp::model::Implementation::new("librefang", env!("CARGO_PKG_VERSION")),
        );

        Self {
            client_info,
            roots: Arc::new(mcp_roots),
        }
    }
}

impl rmcp::ClientHandler for RootsClientHandler {
    fn get_info(&self) -> rmcp::model::ClientInfo {
        self.client_info.clone()
    }

    fn list_roots(
        &self,
        _context: rmcp::service::RequestContext<rmcp::service::RoleClient>,
    ) -> impl std::future::Future<
        Output = Result<rmcp::model::ListRootsResult, rmcp::model::ErrorData>,
    > + Send
           + '_ {
        let roots = Arc::clone(&self.roots);
        async move { Ok(rmcp::model::ListRootsResult::new((*roots).clone())) }
    }
}

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
    /// Current OAuth authentication state for this connection.
    auth_state: crate::mcp_oauth::McpAuthState,
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
    // Python (venvs, conda)
    "PYTHONPATH",
    "VIRTUAL_ENV",
    "CONDA_PREFIX",
    // Rust
    "CARGO_HOME",
    "RUSTUP_HOME",
    // Ruby
    "GEM_HOME",
    "GEM_PATH",
    // Go
    "GOPATH",
    "GOROOT",
];

// ---------------------------------------------------------------------------
// McpConnection implementation
// ---------------------------------------------------------------------------

impl McpConnection {
    /// Connect to an MCP server, perform handshake, and discover tools.
    pub async fn connect(config: McpServerConfig) -> Result<Self, String> {
        let mut initial_auth_state: Option<crate::mcp_oauth::McpAuthState> = None;

        let roots = config.roots.clone();
        let (inner, discovered_tools) = match &config.transport {
            McpTransport::Stdio { command, args } => {
                Self::connect_stdio(command, args, &config.env, roots).await?
            }
            McpTransport::Sse { url } => Self::connect_sse(url).await?,
            McpTransport::Http { url } => {
                // Only advertise local filesystem roots to local servers.
                // Remote MCP servers (GitHub, Slack, …) don't operate on
                // the local filesystem and shouldn't receive host paths.
                let http_roots = if Self::is_local_url(url) {
                    roots
                } else {
                    vec![]
                };
                let (inner, tools, auth_state) = Self::connect_streamable_http(
                    url,
                    &config.headers,
                    config.oauth_provider.as_ref(),
                    config.oauth_config.as_ref(),
                    http_roots,
                )
                .await?;
                initial_auth_state = Some(auth_state);
                (inner, tools)
            }
            McpTransport::HttpCompat {
                base_url,
                headers,
                tools,
            } => {
                // HttpCompat is a static tool-declaration protocol; it does not
                // perform an MCP initialize handshake, so roots don't apply.
                Self::validate_http_compat_config(base_url, headers, tools)?;
                Self::connect_http_compat(base_url).await?
            }
        };

        let mut conn = Self {
            config,
            tools: Vec::new(),
            original_names: HashMap::new(),
            inner,
            auth_state: initial_auth_state.unwrap_or(crate::mcp_oauth::McpAuthState::NotRequired),
        };

        match discovered_tools {
            Some(tools) => {
                // Tools already discovered during connect (rmcp handles this)
                for tool in tools {
                    let description = tool.description.as_deref().unwrap_or("");
                    let mut input_schema =
                        serde_json::Value::Object(tool.input_schema.as_ref().clone());
                    // Preserve MCP `annotations` hints by translating them into
                    // a `metadata.tool_class` entry on the schema so the
                    // runtime tool classifier can pick safe parallel candidates.
                    let ann_value = tool
                        .annotations
                        .as_ref()
                        .and_then(|a| serde_json::to_value(a).ok());
                    inject_annotation_class(&mut input_schema, ann_value.as_ref());
                    conn.register_tool(&tool.name, description, input_schema);
                }
            }
            None => {
                // HttpCompat or SSE — discover tools the old way
                if let McpTransport::HttpCompat { tools, .. } = &conn.config.transport {
                    let declared_tools = tools.clone();
                    conn.register_http_compat_tools(&declared_tools);
                } else if let McpInner::Sse { .. } = &conn.inner {
                    // SSE is a unidirectional transport (client-initiated
                    // requests only). Do NOT declare roots capability — the
                    // server cannot send a roots/list request back over SSE.
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
        roots: Vec<String>,
    ) -> Result<(McpInner, Option<Vec<rmcp::model::Tool>>), String> {
        use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
        use rmcp::ServiceExt;

        // Validate command path (no path traversal)
        if command.contains("..") {
            return Err("MCP command path contains '..': rejected".to_string());
        }

        // Block shell interpreters — MCP servers must use a specific runtime.
        const BLOCKED_SHELLS: &[&str] = &[
            "bash",
            "sh",
            "zsh",
            "fish",
            "csh",
            "tcsh",
            "ksh",
            "dash",
            "cmd",
            "cmd.exe",
            "powershell",
            "powershell.exe",
            "pwsh",
        ];
        let cmd_basename = std::path::Path::new(command)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(command);
        if BLOCKED_SHELLS
            .iter()
            .any(|&s| s.eq_ignore_ascii_case(cmd_basename))
        {
            return Err(format!(
                "MCP server command '{}' is a shell interpreter — use a specific runtime (npx, node, python) instead",
                command
            ));
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

        // Expand environment variable references ($VAR, ${VAR}) in args so
        // templates can use e.g. "$HOME" without wrapping in `sh -c`.
        let args_owned: Vec<String> = args.iter().map(|a| expand_env_vars(a)).collect();
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

        let client = if roots.is_empty() {
            ().into_dyn()
                .serve(transport)
                .await
                .map_err(|e| format!("MCP handshake failed for '{resolved_command}': {e}"))?
        } else {
            RootsClientHandler::new(roots)
                .into_dyn()
                .serve(transport)
                .await
                .map_err(|e| format!("MCP handshake failed for '{resolved_command}': {e}"))?
        };

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

        let client = librefang_http::proxied_client_builder()
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

    // --- Streamable HTTP transport (rmcp SDK) ---

    /// Connect using Streamable HTTP transport (or SSE fallback via the same endpoint).
    ///
    /// The `rmcp` SDK's `StreamableHttpClientTransport` handles the full
    /// Streamable HTTP protocol: Accept headers, Mcp-Session-Id tracking,
    /// SSE stream parsing, and content-type negotiation.
    async fn connect_streamable_http(
        url: &str,
        headers: &[String],
        oauth_provider: Option<&std::sync::Arc<dyn crate::mcp_oauth::McpOAuthProvider>>,
        oauth_config: Option<&librefang_types::config::McpOAuthConfig>,
        roots: Vec<String>,
    ) -> Result<
        (
            McpInner,
            Option<Vec<rmcp::model::Tool>>,
            crate::mcp_oauth::McpAuthState,
        ),
        String,
    > {
        use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
        use rmcp::transport::StreamableHttpClientTransport;
        use rmcp::ServiceExt;

        Self::check_ssrf(url, "Streamable HTTP")?;

        // Parse custom headers (e.g., "Authorization: Bearer <token>").
        let mut custom_headers: HashMap<HeaderName, HeaderValue> = HashMap::new();
        for header_str in headers {
            if let Some((name, value)) = header_str.split_once(':') {
                let name = name.trim();
                let value = value.trim();
                if let (Ok(hn), Ok(hv)) = (
                    HeaderName::from_bytes(name.as_bytes()),
                    HeaderValue::from_str(value),
                ) {
                    custom_headers.insert(hn, hv);
                }
            }
        }

        // Try loading a cached OAuth token and inject as Authorization header.
        let mut used_oauth_token = false;
        if let Some(provider) = oauth_provider {
            if let Some(token) = provider.load_token(url).await {
                debug!(url = %url, "Injecting cached OAuth token for MCP connection");
                if let (Ok(hn), Ok(hv)) = (
                    HeaderName::from_bytes(b"authorization"),
                    HeaderValue::from_str(&format!("Bearer {token}")),
                ) {
                    custom_headers.insert(hn, hv);
                    used_oauth_token = true;
                }
            }
        }

        let mut config = StreamableHttpClientTransportConfig::default();
        config.uri = Arc::from(url);
        config.custom_headers = custom_headers;

        let transport = StreamableHttpClientTransport::from_config(config);

        let serve_result = if roots.is_empty() {
            ().into_dyn().serve(transport).await
        } else {
            RootsClientHandler::new(roots)
                .into_dyn()
                .serve(transport)
                .await
        };
        match serve_result {
            Ok(client) => {
                // Discover tools via rmcp (with timeout)
                let timeout = std::time::Duration::from_secs(60);
                let tools = tokio::time::timeout(timeout, client.list_all_tools())
                    .await
                    .map_err(|_| {
                        "MCP tools/list timed out after 60s for Streamable HTTP".to_string()
                    })?
                    .map_err(|e| format!("MCP tools/list failed: {e}"))?;

                let auth_state = if used_oauth_token {
                    crate::mcp_oauth::McpAuthState::Authorized {
                        expires_at: None,
                        tokens: None,
                    }
                } else {
                    crate::mcp_oauth::McpAuthState::NotRequired
                };

                Ok((McpInner::Rmcp(client), Some(tools), auth_state))
            }
            Err(e) => {
                // Extract the WWW-Authenticate header directly from the
                // underlying `StreamableHttpError::AuthRequired` variant.
                //
                // rmcp's `ClientInitializeError::TransportError` wraps the
                // transport error in a `DynamicTransportError`, which
                // type-erases the inner error into a `Box<dyn Error>`.
                // `std::error::Error::source()` traversal does not reach
                // inside that box because the outer field is not annotated
                // with `#[source]`, so we match on the variant by hand and
                // `downcast_ref` the box contents.
                //
                // If anything in the chain ever changes we fall through to
                // a substring check so we don't regress on plain 401 /
                // "Unauthorized" / "Auth required" errors from future rmcp
                // versions or alternative transports.
                let www_authenticate = Self::extract_auth_header_from_error(&e);

                if www_authenticate.is_none() {
                    let error_str = e.to_string();
                    let is_auth_error = error_str.contains("401")
                        || error_str.contains("Unauthorized")
                        || error_str.contains("Auth required");
                    if !is_auth_error {
                        return Err(format!(
                            "MCP Streamable HTTP connection failed: {error_str}"
                        ));
                    }
                    debug!(
                        url = %url,
                        "401 detected via Display match — structured extraction did not reach the \
                         AuthRequired variant (rmcp chain layout may have changed)"
                    );
                }

                debug!(url = %url, "MCP server returned auth error, attempting OAuth discovery");

                // Discover OAuth metadata using three-tier resolution.
                let metadata = crate::mcp_oauth::discover_oauth_metadata(
                    url,
                    www_authenticate.as_deref(),
                    oauth_config,
                )
                .await
                .map_err(|discovery_err| {
                    format!(
                        "MCP Streamable HTTP connection failed (auth required but OAuth \
                         discovery failed): {discovery_err}"
                    )
                })?;

                // Signal that auth is needed — the API layer will drive the
                // PKCE flow via the UI instead of the daemon opening a browser.
                warn!(
                    url = %url,
                    auth_endpoint = %metadata.authorization_endpoint,
                    "MCP server requires OAuth — deferring to API layer"
                );
                Err("OAUTH_NEEDS_AUTH".to_string())
            }
        }
    }

    /// Extract the `www_authenticate_header` from a
    /// `ClientInitializeError::TransportError` whose underlying error is a
    /// `StreamableHttpError::AuthRequired`.
    ///
    /// Implementation note: walking `std::error::Error::source()` does not
    /// reach the inner variant because rmcp's
    /// `ClientInitializeError::TransportError` field is not annotated with
    /// `#[source]`, so the chain stops at `DynamicTransportError`. We match
    /// on the outer variant directly, then downcast the `Box<dyn Error>`
    /// inside `DynamicTransportError` to the concrete
    /// `StreamableHttpError<reqwest::Error>`.
    fn extract_auth_header_from_error(e: &rmcp::service::ClientInitializeError) -> Option<String> {
        use rmcp::service::ClientInitializeError;
        use rmcp::transport::streamable_http_client::{AuthRequiredError, StreamableHttpError};

        let ClientInitializeError::TransportError { error: dyn_err, .. } = e else {
            return None;
        };
        let streamable = dyn_err
            .error
            .downcast_ref::<StreamableHttpError<reqwest::Error>>()?;
        if let StreamableHttpError::AuthRequired(AuthRequiredError {
            www_authenticate_header,
            ..
        }) = streamable
        {
            Some(www_authenticate_header.clone())
        } else {
            None
        }
    }

    /// Send the MCP `initialize` handshake over SSE transport.
    ///
    /// SSE is unidirectional (client → server), so we never declare the
    /// `roots` capability here — the server has no channel to send
    /// `roots/list` back to us.
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
                    let mut input_schema = tool
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

                    // Preserve MCP `annotations` hints (readOnlyHint /
                    // destructiveHint) by translating them into a
                    // `metadata.tool_class` entry the runtime classifier can read.
                    inject_annotation_class(&mut input_schema, tool.get("annotations"));

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

        let client = librefang_http::proxied_client_builder()
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

    /// Returns `true` when `url` resolves to the local machine.
    /// Used to decide whether filesystem roots are meaningful for an HTTP MCP server.
    ///
    /// Uses proper host parsing rather than substring matching to avoid false
    /// positives on attacker-controlled domains like `127.0.0.1.evil.com`.
    fn is_local_url(url: &str) -> bool {
        // Delegate to the `url` crate so that all RFC 3986 authority components
        // (userinfo, host, port) are parsed correctly.  This prevents attacks
        // like `http://127.0.0.1@attacker.com/` and `http://localhost.evil.com/`
        // that would fool substring or naive split-based checks.
        let parsed = match url::Url::parse(url) {
            Ok(u) => u,
            Err(_) => return false,
        };
        let host = match parsed.host() {
            Some(h) => h,
            None => return false,
        };
        match host {
            url::Host::Domain(d) => d.eq_ignore_ascii_case("localhost"),
            url::Host::Ipv4(addr) => addr.octets()[0] == 127,
            url::Host::Ipv6(addr) => addr == std::net::Ipv6Addr::LOCALHOST,
        }
    }

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
}

/// Translate MCP `tools/list` annotations into a `metadata.tool_class` field
/// inside the tool's JSON Schema so `runtime/tool_classifier.rs` can pick it
/// up via `explicit_class_from_schema`.
///
/// MCP spec defaults: `readOnlyHint = false`, `destructiveHint = true`.
/// We map `(read_only=true, destructive=false)` to `readonly_search`; any
/// other combination is treated as `mutating`. When `annotations` is absent,
/// the schema is left untouched so existing heuristics still apply.
///
/// `idempotentHint` and `openWorldHint` are intentionally ignored at this
/// layer — the current `ToolApprovalClass` enum has no idempotent / open-world
/// variants, so threading them through would just mean noise that the
/// classifier discards. If the projection in
/// `runtime/tool_classifier.rs::ParallelSafety` ever grows finer-grained
/// classes (e.g. an idempotent_mutating tier for safer batch retries), wire
/// the additional hints in here.
///
/// Inputs come from server-controlled `tools/list` payloads, so the helper
/// must never panic on malformed shapes — it silently no-ops if `schema`
/// is not an object or `annotations` is not an object.
fn inject_annotation_class(
    schema: &mut serde_json::Value,
    annotations: Option<&serde_json::Value>,
) {
    let Some(ann) = annotations.and_then(|v| v.as_object()) else {
        return;
    };
    let Some(obj) = schema.as_object_mut() else {
        return;
    };

    // Spec defaults when a hint is missing.
    let read_only = ann
        .get("readOnlyHint")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let destructive = ann
        .get("destructiveHint")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let class = if read_only && !destructive {
        "readonly_search"
    } else {
        "mutating"
    };

    if !obj.contains_key("metadata") {
        obj.insert("metadata".to_string(), serde_json::json!({}));
    }
    if let Some(meta) = obj.get_mut("metadata").and_then(|v| v.as_object_mut()) {
        meta.insert(
            "tool_class".to_string(),
            serde_json::Value::String(class.to_string()),
        );
    }
}

impl McpConnection {
    /// Call a tool on the MCP server.
    pub async fn call_tool(
        &mut self,
        name: &str,
        arguments: &serde_json::Value,
    ) -> Result<String, String> {
        // Resolve raw (un-prefixed) tool name before taint check so we can
        // look it up in the per-tool policy.
        let raw_name: String = self
            .original_names
            .get(name)
            .cloned()
            .or_else(|| strip_mcp_prefix(&self.config.name, name).map(|s| s.to_string()))
            .unwrap_or_else(|| name.to_string());

        // SECURITY: best-effort taint filter before shipping arguments
        // to an out-of-process MCP server. An LLM that has been pushed
        // into smuggling credentials into tool-call arguments would
        // otherwise exfiltrate them straight through this call — the
        // MCP transport hands the JSON to whoever implements the server.
        // Walk every string leaf in the arguments tree and refuse the
        // call if anything trips `check_outbound_text_violation_with_skip`.
        // Non-string leaves (numbers, bools, null) can't carry plaintext
        // credentials in any meaningful way, so they are left alone.
        //
        // This is still a best-effort pattern match — not a full
        // information-flow tracker. Copy-pasted obfuscation still bypasses
        // it. Per-tool, per-path exemptions in `taint_policy` let operators
        // disable specific rules for known-safe fields.
        if self.config.taint_scanning {
            let policy = self.config.taint_policy.as_ref();
            if let Some(violation) = scan_mcp_arguments_for_taint_with_policy(
                arguments,
                policy,
                &self.config.taint_rule_sets,
                &raw_name,
            ) {
                // `violation` is already a redacted rule description from
                // the scanner — do NOT concatenate the raw payload or the
                // offending value into the error surface.
                return Err(violation);
            }
        }

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
                // Always send an object — MCP spec requires `arguments` to
                // be an object, and some servers (e.g. filesystem) reject
                // `undefined`/`null` even for zero-parameter tools.
                params.arguments = Some(arguments.as_object().cloned().unwrap_or_default());

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

    /// Get the current OAuth authentication state.
    pub fn auth_state(&self) -> &crate::mcp_oauth::McpAuthState {
        &self.auth_state
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

/// Expand `$VAR` and `${VAR}` references in a string using the process
/// environment. Unknown variables are left as-is. This allows MCP server
/// templates to reference `$HOME`, `$USER`, etc. without requiring a shell
/// wrapper (`sh -c`), which the security check blocks.
fn expand_env_vars(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '$' {
            let braced = chars.peek() == Some(&'{');
            if braced {
                chars.next(); // consume '{'
            }
            let mut var_name = String::new();
            while let Some(&c) = chars.peek() {
                if braced {
                    if c == '}' {
                        chars.next();
                        break;
                    }
                } else if !c.is_ascii_alphanumeric() && c != '_' {
                    break;
                }
                var_name.push(c);
                chars.next();
            }
            if var_name.is_empty() {
                result.push('$');
                if braced {
                    result.push('{');
                }
            } else if let Ok(val) = std::env::var(&var_name) {
                result.push_str(&val);
            } else {
                // Unknown var — keep original text
                result.push('$');
                if braced {
                    result.push('{');
                }
                result.push_str(&var_name);
                if braced {
                    result.push('}');
                }
            }
        } else {
            result.push(ch);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    // ── MCP outbound taint scanning ──────────────────────────────────────

    #[test]
    fn test_scan_mcp_arguments_rejects_secret_string_leaf() {
        let args = serde_json::json!({
            "repo": "libre/librefang",
            "token": "ghp_1234567890abcdefghijklmnopqrstuvwxyz",
        });
        assert!(scan_mcp_arguments_for_taint(&args).is_some());
    }

    #[test]
    fn test_scan_mcp_arguments_walks_nested_trees() {
        let args = serde_json::json!({
            "filter": {
                "headers": {
                    "Authorization": "Bearer sk-live-secret",
                }
            }
        });
        assert!(scan_mcp_arguments_for_taint(&args).is_some());
    }

    #[test]
    fn test_scan_mcp_arguments_rejects_secret_inside_array() {
        let args = serde_json::json!({
            "env": ["PATH=/usr/bin", "api_key=sk-00000"],
        });
        assert!(scan_mcp_arguments_for_taint(&args).is_some());
    }

    #[test]
    fn test_scan_mcp_arguments_allows_plain_strings() {
        let args = serde_json::json!({
            "query": "What tokens does this crate use?",
            "limit": 10,
            "include_drafts": false,
            "tags": ["rust", "security"],
        });
        assert!(scan_mcp_arguments_for_taint(&args).is_none());
    }

    #[test]
    fn test_scan_mcp_arguments_rejects_json_authorization_string_leaf() {
        let args = serde_json::json!({
            "body": r#"{"authorization": "Bearer sk-live-secret"}"#,
        });
        assert!(scan_mcp_arguments_for_taint(&args).is_some());
    }

    #[test]
    fn test_scan_mcp_arguments_rejects_pii_string_leaf() {
        let args = serde_json::json!({
            "email": "john@example.com",
        });
        assert!(scan_mcp_arguments_for_taint(&args).is_some());
    }

    #[test]
    fn test_scan_mcp_arguments_error_does_not_leak_secret() {
        // The scanner must redact: the returned error string is
        // surfaced to the LLM and to logs, and must NOT contain the
        // exact credential payload we just blocked.
        let secret = "ghp_SECRETabcdef0123456789SECRETabcdef0123";
        let args = serde_json::json!({
            "headers": { "Authorization": format!("Bearer {secret}") }
        });
        let err = scan_mcp_arguments_for_taint(&args).expect("must flag credential-shaped value");
        assert!(
            !err.contains(secret),
            "error string leaked the blocked secret: {err}"
        );
        assert!(
            !err.contains("Bearer"),
            "error string leaked the header value: {err}"
        );
        // It should still identify the offending path for debugging.
        assert!(
            err.contains("headers.Authorization") || err.contains("Authorization"),
            "error string should point at the offending path: {err}"
        );
    }

    #[test]
    fn test_scan_mcp_arguments_depth_cap() {
        // Build a 200-deep nested object. The scanner must bail out
        // at MCP_TAINT_SCAN_MAX_DEPTH rather than recursing forever.
        let mut v = serde_json::Value::String("ok".to_string());
        for _ in 0..200 {
            let mut m = serde_json::Map::new();
            m.insert("next".to_string(), v);
            v = serde_json::Value::Object(m);
        }
        let err =
            scan_mcp_arguments_for_taint(&v).expect("depth cap must reject pathological nesting");
        assert!(
            err.contains("max depth"),
            "expected depth-cap error, got: {err}"
        );
    }

    #[test]
    fn test_scan_mcp_arguments_allows_null_and_numbers() {
        let args = serde_json::json!({
            "cursor": null,
            "page": 3,
            "rate": 1.5,
        });
        assert!(scan_mcp_arguments_for_taint(&args).is_none());
    }

    #[test]
    fn test_scan_mcp_arguments_allows_date_prefixed_session_handle() {
        // Regression for issue #2652: Camofox MCP returns tabIds of the
        // form `tab-YYYY-MM-DD-<uuid-segments>`. These must pass the
        // taint scanner so the LLM can pass them to subsequent tool calls.
        let args = serde_json::json!({
            "tabId": "tab-2026-04-16-abc123-def456-ghi789",
        });
        assert!(
            scan_mcp_arguments_for_taint(&args).is_none(),
            "date-prefixed tabId must not be blocked"
        );
    }

    #[test]
    fn test_scan_mcp_arguments_still_blocks_real_token_in_tab_shaped_key() {
        // A credential-shaped VALUE under a session-like KEY must still be blocked.
        // Key-name allowlisting must NOT bypass value-content checks.
        let args = serde_json::json!({
            "tabId": "sk-proj-abcdefghijklmnopqrstuvwxyz1234567890",
        });
        assert!(
            scan_mcp_arguments_for_taint(&args).is_some(),
            "real credential under session-like key must still be blocked"
        );
    }

    // ── per-path policy tests ─────────────────────────────────────────────

    #[test]
    fn test_policy_skip_opaque_token_allows_tab_id() {
        use librefang_types::config::{McpTaintPathPolicy, McpTaintPolicy, McpTaintToolPolicy};
        use librefang_types::taint::TaintRuleId;

        let mut paths = std::collections::HashMap::new();
        paths.insert(
            "$.tabId".to_string(),
            McpTaintPathPolicy {
                skip_rules: vec![TaintRuleId::OpaqueToken],
            },
        );
        let mut tools = std::collections::HashMap::new();
        tools.insert(
            "navigate".to_string(),
            McpTaintToolPolicy {
                paths,
                ..Default::default()
            },
        );
        let policy = McpTaintPolicy { tools };

        // Opaque-looking tab handle — blocked without policy, allowed with it.
        let args = serde_json::json!({ "tabId": "xAbCdEfGhIjKlMnOpQrStUvWxYz1234567890AB" });
        assert!(
            scan_mcp_arguments_for_taint(&args).is_some(),
            "must block without policy"
        );
        assert!(
            scan_mcp_arguments_for_taint_with_policy(&args, Some(&policy), &[], "navigate")
                .is_none(),
            "OpaqueToken skip must allow browser tab ID under navigate.tabId"
        );
    }

    #[test]
    fn test_policy_skip_sensitive_key_name_uses_child_path() {
        use librefang_types::config::{McpTaintPathPolicy, McpTaintPolicy, McpTaintToolPolicy};
        use librefang_types::taint::TaintRuleId;

        // Configure skip for the child path "$.authorization", NOT the parent "$".
        // This verifies the bug fix: SensitiveKeyName resolution must use child path.
        let mut paths = std::collections::HashMap::new();
        paths.insert(
            "$.authorization".to_string(),
            McpTaintPathPolicy {
                skip_rules: vec![TaintRuleId::SensitiveKeyName],
            },
        );
        let mut tools = std::collections::HashMap::new();
        tools.insert(
            "send_request".to_string(),
            McpTaintToolPolicy {
                paths,
                ..Default::default()
            },
        );
        let policy = McpTaintPolicy { tools };

        let args = serde_json::json!({ "authorization": "some-non-empty-value" });

        // Without policy: blocked because "authorization" is a sensitive key name.
        assert!(
            scan_mcp_arguments_for_taint(&args).is_some(),
            "must block sensitive key without policy"
        );

        // With SensitiveKeyName skipped for "$.authorization": allowed.
        assert!(
            scan_mcp_arguments_for_taint_with_policy(&args, Some(&policy), &[], "send_request")
                .is_none(),
            "SensitiveKeyName skip at child path must allow the key"
        );

        // Policy on different tool must NOT apply.
        assert!(
            scan_mcp_arguments_for_taint_with_policy(&args, Some(&policy), &[], "other_tool")
                .is_some(),
            "skip for send_request must not affect other_tool"
        );
    }

    #[test]
    fn test_policy_non_skipped_rules_still_fire() {
        use librefang_types::config::{McpTaintPathPolicy, McpTaintPolicy, McpTaintToolPolicy};
        use librefang_types::taint::TaintRuleId;

        // Skip OpaqueToken for "$.token", but the value contains "api_key=secret"
        // which trips KeyValueSecret — that rule is NOT skipped.
        let mut paths = std::collections::HashMap::new();
        paths.insert(
            "$.token".to_string(),
            McpTaintPathPolicy {
                skip_rules: vec![TaintRuleId::OpaqueToken],
            },
        );
        let mut tools = std::collections::HashMap::new();
        tools.insert(
            "call".to_string(),
            McpTaintToolPolicy {
                paths,
                ..Default::default()
            },
        );
        let policy = McpTaintPolicy { tools };

        let args = serde_json::json!({ "token": "api_key=sk-not-real" });
        assert!(
            scan_mcp_arguments_for_taint_with_policy(&args, Some(&policy), &[], "call").is_some(),
            "non-skipped KeyValueSecret must still fire even when OpaqueToken is skipped"
        );
    }

    // ── tool-level `default = "skip"` kill-switch tests ───────────────────

    #[test]
    fn test_tool_default_skip_bypasses_scanning_for_target_tool() {
        use librefang_types::config::{McpTaintPolicy, McpTaintToolAction, McpTaintToolPolicy};

        let mut tools = std::collections::HashMap::new();
        tools.insert(
            "navigate".to_string(),
            McpTaintToolPolicy {
                default: McpTaintToolAction::Skip,
                ..Default::default()
            },
        );
        let policy = McpTaintPolicy { tools };

        // Heavily credential-shaped payload that would normally block.
        let args = serde_json::json!({
            "tabId": "ghp_abcdefghij1234567890abcdefghij1234567890",
            "headers": { "Authorization": "Bearer sk-zzz-not-real-but-shaped" }
        });
        assert!(
            scan_mcp_arguments_for_taint(&args).is_some(),
            "must block without policy"
        );
        assert!(
            scan_mcp_arguments_for_taint_with_policy(&args, Some(&policy), &[], "navigate")
                .is_none(),
            "tool-level default=skip must bypass scanning entirely"
        );
    }

    #[test]
    fn test_tool_default_skip_does_not_leak_to_other_tools() {
        use librefang_types::config::{McpTaintPolicy, McpTaintToolAction, McpTaintToolPolicy};

        let mut tools = std::collections::HashMap::new();
        tools.insert(
            "navigate".to_string(),
            McpTaintToolPolicy {
                default: McpTaintToolAction::Skip,
                ..Default::default()
            },
        );
        let policy = McpTaintPolicy { tools };

        // Same payload as the previous test, but called against a tool that
        // does NOT have a skip policy — must still block.
        let args = serde_json::json!({ "Authorization": "Bearer sk-not-real-token-12345" });
        assert!(
            scan_mcp_arguments_for_taint_with_policy(&args, Some(&policy), &[], "send_request")
                .is_some(),
            "default=skip on `navigate` must not affect `send_request`"
        );
    }

    // ── named rule sets / warn / log severity tests ───────────────────────

    #[test]
    fn test_rule_set_warn_action_allows_call_through() {
        use librefang_types::config::{
            McpTaintPolicy, McpTaintRuleSetAction, McpTaintToolPolicy, NamedTaintRuleSet,
        };
        use librefang_types::taint::TaintRuleId;

        let mut tools = std::collections::HashMap::new();
        tools.insert(
            "navigate".to_string(),
            McpTaintToolPolicy {
                rule_sets: vec!["browser_handles".to_string()],
                ..Default::default()
            },
        );
        let policy = McpTaintPolicy { tools };
        let registry = vec![NamedTaintRuleSet {
            name: "browser_handles".to_string(),
            action: McpTaintRuleSetAction::Warn,
            rules: vec![TaintRuleId::OpaqueToken],
        }];

        let args = serde_json::json!({ "tabId": "xAbCdEfGhIjKlMnOpQrStUvWxYz1234567890AB" });
        assert!(
            scan_mcp_arguments_for_taint(&args).is_some(),
            "must block without policy"
        );
        assert!(
            scan_mcp_arguments_for_taint_with_policy(&args, Some(&policy), &registry, "navigate")
                .is_none(),
            "rule_set with action=warn must allow the call through"
        );
    }

    #[test]
    fn test_rule_set_log_action_also_allows_through() {
        use librefang_types::config::{
            McpTaintPolicy, McpTaintRuleSetAction, McpTaintToolPolicy, NamedTaintRuleSet,
        };
        use librefang_types::taint::TaintRuleId;

        let mut tools = std::collections::HashMap::new();
        tools.insert(
            "audit_tool".to_string(),
            McpTaintToolPolicy {
                rule_sets: vec!["pii_audit".to_string()],
                ..Default::default()
            },
        );
        let policy = McpTaintPolicy { tools };
        let registry = vec![NamedTaintRuleSet {
            name: "pii_audit".to_string(),
            action: McpTaintRuleSetAction::Log,
            rules: vec![TaintRuleId::PiiEmail, TaintRuleId::PiiPhone],
        }];

        let args = serde_json::json!({ "to": "alice@example.com" });
        assert!(
            scan_mcp_arguments_for_taint_with_policy(&args, Some(&policy), &registry, "audit_tool")
                .is_none(),
            "rule_set with action=log must allow the call through"
        );
    }

    #[test]
    fn test_rule_set_block_action_is_no_op() {
        use librefang_types::config::{
            McpTaintPolicy, McpTaintRuleSetAction, McpTaintToolPolicy, NamedTaintRuleSet,
        };
        use librefang_types::taint::TaintRuleId;

        let mut tools = std::collections::HashMap::new();
        tools.insert(
            "navigate".to_string(),
            McpTaintToolPolicy {
                rule_sets: vec!["explicit_block".to_string()],
                ..Default::default()
            },
        );
        let policy = McpTaintPolicy { tools };
        let registry = vec![NamedTaintRuleSet {
            name: "explicit_block".to_string(),
            action: McpTaintRuleSetAction::Block,
            rules: vec![TaintRuleId::OpaqueToken],
        }];

        let args = serde_json::json!({ "tabId": "xAbCdEfGhIjKlMnOpQrStUvWxYz1234567890AB" });
        assert!(
            scan_mcp_arguments_for_taint_with_policy(&args, Some(&policy), &registry, "navigate")
                .is_some(),
            "rule_set with action=block must keep the call blocked"
        );
    }

    #[test]
    fn test_rule_set_warn_only_skips_listed_rules() {
        use librefang_types::config::{
            McpTaintPolicy, McpTaintRuleSetAction, McpTaintToolPolicy, NamedTaintRuleSet,
        };
        use librefang_types::taint::TaintRuleId;

        // rule_set warns OpaqueToken only — KeyValueSecret must still block.
        let mut tools = std::collections::HashMap::new();
        tools.insert(
            "do_thing".to_string(),
            McpTaintToolPolicy {
                rule_sets: vec!["browser_handles".to_string()],
                ..Default::default()
            },
        );
        let policy = McpTaintPolicy { tools };
        let registry = vec![NamedTaintRuleSet {
            name: "browser_handles".to_string(),
            action: McpTaintRuleSetAction::Warn,
            rules: vec![TaintRuleId::OpaqueToken],
        }];

        let args = serde_json::json!({ "blob": "api_key=sk-not-real" });
        assert!(
            scan_mcp_arguments_for_taint_with_policy(&args, Some(&policy), &registry, "do_thing")
                .is_some(),
            "rule_set covering OpaqueToken must not exempt KeyValueSecret"
        );
    }

    #[test]
    fn test_rule_set_warn_takes_precedence_over_block() {
        use librefang_types::config::{
            McpTaintPolicy, McpTaintRuleSetAction, McpTaintToolPolicy, NamedTaintRuleSet,
        };
        use librefang_types::taint::TaintRuleId;

        // Tool references two rule sets; the more permissive `warn` wins.
        let mut tools = std::collections::HashMap::new();
        tools.insert(
            "navigate".to_string(),
            McpTaintToolPolicy {
                rule_sets: vec!["strict".to_string(), "lenient".to_string()],
                ..Default::default()
            },
        );
        let policy = McpTaintPolicy { tools };
        let registry = vec![
            NamedTaintRuleSet {
                name: "strict".to_string(),
                action: McpTaintRuleSetAction::Block,
                rules: vec![TaintRuleId::OpaqueToken],
            },
            NamedTaintRuleSet {
                name: "lenient".to_string(),
                action: McpTaintRuleSetAction::Warn,
                rules: vec![TaintRuleId::OpaqueToken],
            },
        ];

        let args = serde_json::json!({ "tabId": "xAbCdEfGhIjKlMnOpQrStUvWxYz1234567890AB" });
        assert!(
            scan_mcp_arguments_for_taint_with_policy(&args, Some(&policy), &registry, "navigate")
                .is_none(),
            "warn must override block when both sets cover the same rule"
        );
    }

    #[test]
    fn test_rule_set_warn_downgrades_sensitive_key_name() {
        use librefang_types::config::{
            McpTaintPolicy, McpTaintRuleSetAction, McpTaintToolPolicy, NamedTaintRuleSet,
        };
        use librefang_types::taint::TaintRuleId;

        // Sensitive key-name blocking is also subject to rule_set downgrade.
        let mut tools = std::collections::HashMap::new();
        tools.insert(
            "send_request".to_string(),
            McpTaintToolPolicy {
                rule_sets: vec!["loose".to_string()],
                ..Default::default()
            },
        );
        let policy = McpTaintPolicy { tools };
        let registry = vec![NamedTaintRuleSet {
            name: "loose".to_string(),
            action: McpTaintRuleSetAction::Warn,
            rules: vec![TaintRuleId::SensitiveKeyName],
        }];

        let args = serde_json::json!({ "authorization": "anything-non-empty" });
        assert!(
            scan_mcp_arguments_for_taint_with_policy(
                &args,
                Some(&policy),
                &registry,
                "send_request"
            )
            .is_none(),
            "rule_set warn covering SensitiveKeyName must allow object key through"
        );
    }

    // ── JSONPath matcher unit tests ───────────────────────────────────────

    #[test]
    fn test_jsonpath_exact_match() {
        assert!(jsonpath_matches("$.a.b", "$.a.b"));
        assert!(jsonpath_matches("$", "$"));
        assert!(!jsonpath_matches("$.a.b", "$.a.c"));
        assert!(!jsonpath_matches("$.a.b", "$.a"));
    }

    #[test]
    fn test_jsonpath_star_wildcard() {
        assert!(jsonpath_matches("$.*", "$.foo"));
        assert!(jsonpath_matches("$.*", "$.bar"));
        assert!(
            !jsonpath_matches("$.*", "$.foo.child"),
            "star must not cross depth"
        );
        // star must not match array-index segments
        assert!(!jsonpath_matches("$.*", "$.items[0]"));
    }

    #[test]
    fn test_jsonpath_array_wildcard() {
        assert!(jsonpath_matches("$.items[*]", "$.items[0]"));
        assert!(jsonpath_matches("$.items[*]", "$.items[99]"));
        assert!(!jsonpath_matches("$.items[*]", "$.other[0]"));
        assert!(
            !jsonpath_matches("$.items[*]", "$.items[0].name"),
            "must not match deeper path"
        );
    }

    #[test]
    fn test_jsonpath_nested_star() {
        assert!(jsonpath_matches("$.a.*", "$.a.x"));
        assert!(jsonpath_matches("$.a.*", "$.a.y"));
        assert!(!jsonpath_matches("$.a.*", "$.b.x"));
        assert!(!jsonpath_matches("$.a.*", "$.a.x.z"));
    }

    /// Doc-test: validate the wildcard syntax called out in the rustdoc on
    /// `librefang_types::config::McpTaintPathPolicy`.
    #[test]
    fn test_documented_wildcards_match_expected_paths() {
        // `$.foo` — exact property.
        assert!(jsonpath_matches("$.foo", "$.foo"));
        assert!(!jsonpath_matches("$.foo", "$.foo.bar"));

        // `$.foo.*` — any direct child of `$.foo` (single segment, non-array).
        assert!(jsonpath_matches("$.foo.*", "$.foo.bar"));
        assert!(jsonpath_matches("$.foo.*", "$.foo.baz"));
        assert!(!jsonpath_matches("$.foo.*", "$.foo.bar.qux"));
        assert!(!jsonpath_matches("$.foo.*", "$.foo[0]"));

        // `$.foo[*]` — any array element of `$.foo`.
        assert!(jsonpath_matches("$.foo[*]", "$.foo[0]"));
        assert!(jsonpath_matches("$.foo[*]", "$.foo[7]"));
        assert!(!jsonpath_matches("$.foo[*]", "$.foo[0].bar"));

        // `$.*` — any top-level property.
        assert!(jsonpath_matches("$.*", "$.alpha"));
        assert!(!jsonpath_matches("$.*", "$.alpha.beta"));
    }

    #[test]
    fn test_path_wildcard_skips_apply_via_policy() {
        use librefang_types::config::{McpTaintPathPolicy, McpTaintPolicy, McpTaintToolPolicy};
        use librefang_types::taint::TaintRuleId;

        // `$.metadata.*` should exempt every direct child key of `metadata`.
        let mut paths = std::collections::HashMap::new();
        paths.insert(
            "$.metadata.*".to_string(),
            McpTaintPathPolicy {
                skip_rules: vec![TaintRuleId::SensitiveKeyName],
            },
        );
        let mut tools = std::collections::HashMap::new();
        tools.insert(
            "read_file".to_string(),
            McpTaintToolPolicy {
                paths,
                ..Default::default()
            },
        );
        let policy = McpTaintPolicy { tools };

        let args = serde_json::json!({
            "metadata": { "api_key": "x", "etag": "y" }
        });
        assert!(
            scan_mcp_arguments_for_taint_with_policy(&args, Some(&policy), &[], "read_file")
                .is_none(),
            "wildcard $.metadata.* must exempt all direct children"
        );
    }

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
            headers: vec![],
            oauth_provider: None,
            oauth_config: None,
            taint_scanning: true,
            taint_policy: None,
            taint_rule_sets: vec![],
            roots: vec![],
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
            headers: vec![],
            oauth_provider: None,
            oauth_config: None,
            taint_scanning: true,
            taint_policy: None,
            taint_rule_sets: vec![],
            roots: vec![],
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
            headers: vec![],
            oauth_provider: None,
            oauth_config: None,
            taint_scanning: true,
            taint_policy: None,
            taint_rule_sets: vec![],
            roots: vec![],
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

        // HTTP (Streamable HTTP) variant
        let http_config = McpServerConfig {
            name: "atlassian".to_string(),
            transport: McpTransport::Http {
                url: "https://mcp.atlassian.com/v1/mcp".to_string(),
            },
            timeout_secs: 120,
            env: vec![],
            headers: vec!["Authorization: Bearer test-token-456".to_string()],
            oauth_provider: None,
            oauth_config: None,
            taint_scanning: true,
            taint_policy: None,
            taint_rule_sets: vec![],
            roots: vec![],
        };
        let json = serde_json::to_string(&http_config).unwrap();
        let back: McpServerConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.headers.len(), 1);
        assert_eq!(back.headers[0], "Authorization: Bearer test-token-456");
        match back.transport {
            McpTransport::Http { url } => {
                assert_eq!(url, "https://mcp.atlassian.com/v1/mcp")
            }
            _ => panic!("Expected Http transport"),
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
                headers: vec![],
                oauth_provider: None,
                oauth_config: None,
                taint_scanning: true,
                taint_policy: None,
                taint_rule_sets: vec![],
                roots: vec![],
            },
            tools: Vec::new(),
            original_names: HashMap::new(),
            inner: McpInner::HttpCompat {
                client: librefang_http::proxied_client(),
            },
            auth_state: crate::mcp_oauth::McpAuthState::NotRequired,
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
            headers: vec![],
            oauth_provider: None,
            oauth_config: None,
            taint_scanning: true,
            taint_policy: None,
            taint_rule_sets: vec![],
            roots: vec![],
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

    #[test]
    fn test_is_local_url() {
        // Standard loopback addresses
        assert!(McpConnection::is_local_url("http://127.0.0.1:8080/mcp"));
        assert!(McpConnection::is_local_url("http://localhost/mcp"));
        assert!(McpConnection::is_local_url("http://LOCALHOST/mcp"));
        assert!(McpConnection::is_local_url("http://[::1]:3000/mcp"));
        // Full 127.0.0.0/8 range
        assert!(McpConnection::is_local_url("http://127.2.3.4/mcp"));
        assert!(McpConnection::is_local_url("http://127.255.255.255/mcp"));
        // Remote hosts
        assert!(!McpConnection::is_local_url("https://api.github.com/mcp"));
        assert!(!McpConnection::is_local_url("https://mcp.example.com/mcp"));
        // Security: domain spoofing — "127." prefix in domain name must not match
        assert!(!McpConnection::is_local_url(
            "https://127.0.0.1.evil.com/mcp"
        ));
        // Security: userinfo spoofing — "127.0.0.1@attacker.com" must not match
        assert!(!McpConnection::is_local_url(
            "http://127.0.0.1@attacker.com/mcp"
        ));
        // Security: subdomain of localhost must not match
        assert!(!McpConnection::is_local_url(
            "http://localhost.evil.com/mcp"
        ));
        // 0.0.0.0 is a listen address, not a loopback target
        assert!(!McpConnection::is_local_url("http://0.0.0.0:4545/mcp"));
    }

    /// `extract_auth_header_from_error` returns `None` for any
    /// `ClientInitializeError` variant that isn't `TransportError`. The
    /// positive path (returning `Some(header)`) requires constructing a
    /// `DynamicTransportError` holding a `StreamableHttpError::AuthRequired`,
    /// which can't be built from outside rmcp because `AuthRequiredError`
    /// is `#[non_exhaustive]`. This negative-path test pins the "bail out
    /// early on the wrong variant" invariant so the downcast chain stays
    /// correct under future rmcp shape changes.
    #[test]
    fn test_extract_auth_header_from_error_returns_none_for_non_transport_variant() {
        use rmcp::service::ClientInitializeError;

        let err = ClientInitializeError::ConnectionClosed("simulated".to_string());
        assert!(McpConnection::extract_auth_header_from_error(&err).is_none());
    }

    // ── inject_annotation_class — MCP tool annotation propagation ────────

    #[test]
    fn inject_annotation_readonly_sets_readonly_search() {
        let mut schema = serde_json::json!({"type": "object"});
        let ann = serde_json::json!({
            "readOnlyHint": true,
            "destructiveHint": false,
        });
        inject_annotation_class(&mut schema, Some(&ann));
        assert_eq!(
            schema["metadata"]["tool_class"].as_str(),
            Some("readonly_search")
        );
    }

    #[test]
    fn inject_annotation_destructive_sets_mutating() {
        let mut schema = serde_json::json!({"type": "object"});
        let ann = serde_json::json!({
            "readOnlyHint": false,
            "destructiveHint": true,
        });
        inject_annotation_class(&mut schema, Some(&ann));
        assert_eq!(schema["metadata"]["tool_class"].as_str(), Some("mutating"));
    }

    #[test]
    fn inject_annotation_default_destructive_when_missing() {
        // Per MCP spec, when destructiveHint is missing the default is true,
        // so the tool must be classified as `mutating`.
        let mut schema = serde_json::json!({"type": "object"});
        let ann = serde_json::json!({"readOnlyHint": false});
        inject_annotation_class(&mut schema, Some(&ann));
        assert_eq!(schema["metadata"]["tool_class"].as_str(), Some("mutating"));
    }

    #[test]
    fn inject_annotation_no_annotations_preserves_schema() {
        let original = serde_json::json!({
            "type": "object",
            "properties": {"q": {"type": "string"}},
        });
        let mut schema = original.clone();
        inject_annotation_class(&mut schema, None);
        assert_eq!(schema, original);
    }

    #[test]
    fn inject_annotation_preserves_existing_metadata() {
        let mut schema = serde_json::json!({
            "type": "object",
            "metadata": {"foo": "bar"},
        });
        let ann = serde_json::json!({
            "readOnlyHint": true,
            "destructiveHint": false,
        });
        inject_annotation_class(&mut schema, Some(&ann));
        assert_eq!(schema["metadata"]["foo"].as_str(), Some("bar"));
        assert_eq!(
            schema["metadata"]["tool_class"].as_str(),
            Some("readonly_search")
        );
    }

    #[test]
    fn inject_annotation_existing_tool_class_overwritten() {
        let mut schema = serde_json::json!({
            "type": "object",
            "metadata": {"tool_class": "readonly_search"},
        });
        let ann = serde_json::json!({
            "readOnlyHint": false,
            "destructiveHint": true,
        });
        inject_annotation_class(&mut schema, Some(&ann));
        assert_eq!(schema["metadata"]["tool_class"].as_str(), Some("mutating"));
    }

    #[test]
    fn inject_annotation_non_object_schema_is_noop() {
        // Defensive: a malformed schema (e.g. a bare bool) must not panic.
        let mut schema = serde_json::json!(true);
        let ann = serde_json::json!({"readOnlyHint": true});
        inject_annotation_class(&mut schema, Some(&ann));
        assert_eq!(schema, serde_json::json!(true));
    }

    #[test]
    fn inject_annotation_non_object_annotations_is_noop() {
        let original = serde_json::json!({"type": "object"});
        let mut schema = original.clone();
        let ann = serde_json::json!("not-an-object");
        inject_annotation_class(&mut schema, Some(&ann));
        assert_eq!(schema, original);
    }

    /// Producer/consumer string contract: the literals `inject_annotation_class`
    /// emits must round-trip through `ToolApprovalClass::from_snake_case` to
    /// the corresponding variants. Without this, a typo or a future rename in
    /// either crate would silently fall back to `Unknown` → `WriteShared` and
    /// the whole MCP-tool parallelisation fix becomes a no-op.
    #[test]
    fn injected_class_strings_parse_into_approval_class() {
        use librefang_types::tool_class::ToolApprovalClass;

        // readOnly + non-destructive → "readonly_search" → ReadonlySearch
        let mut schema_ro = serde_json::json!({"type": "object"});
        inject_annotation_class(
            &mut schema_ro,
            Some(&serde_json::json!({
                "readOnlyHint": true,
                "destructiveHint": false,
            })),
        );
        let class_str = schema_ro["metadata"]["tool_class"]
            .as_str()
            .expect("readonly path must produce a string");
        assert_eq!(
            ToolApprovalClass::from_snake_case(class_str),
            Some(ToolApprovalClass::ReadonlySearch),
            "producer string {class_str:?} must parse on the consumer side"
        );

        // destructive → "mutating" → Mutating
        let mut schema_mut = serde_json::json!({"type": "object"});
        inject_annotation_class(
            &mut schema_mut,
            Some(&serde_json::json!({
                "readOnlyHint": false,
                "destructiveHint": true,
            })),
        );
        let class_str = schema_mut["metadata"]["tool_class"]
            .as_str()
            .expect("mutating path must produce a string");
        assert_eq!(
            ToolApprovalClass::from_snake_case(class_str),
            Some(ToolApprovalClass::Mutating),
            "producer string {class_str:?} must parse on the consumer side"
        );
    }
}
