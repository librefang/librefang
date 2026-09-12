//! Cluster pulled out of mod.rs in #4713 phase 3c.
//!
//! Hosts the MCP (Model Context Protocol) server lifecycle: initial
//! connection, disconnect/retry on failure, hot-reload of the server
//! list after config changes, and the long-running health-monitor
//! loop that watches for stalled / dead connections.
//!
//! Sibling submodule of `kernel::mod`. The public methods retain their
//! existing visibility (they're called from the API crate and config-
//! reload paths). `run_mcp_health_loop` is bumped to `pub(crate)` so
//! the spawn site in `kernel::mod` can reach it after the move.

use std::sync::Arc;

use super::*;
use crate::McpSubsystemApi;

/// Sentinel `McpConnection::connect` returns instead of an error message when the server
/// answered 401 and supports OAuth: the PKCE flow is UI-driven, so the daemon only records
/// the state and waits. Not a connect failure, and both readers in this module have to
/// recognise it — named once here rather than spelled out at each site.
const OAUTH_NEEDS_AUTH: &str = "OAUTH_NEEDS_AUTH";

/// Coerce the trait-borrowed `Arc<dyn McpOAuthProvider + Send + Sync>` to
/// the unsized `Arc<dyn McpOAuthProvider>` that `McpServerConfig` expects.
fn oauth_provider_clone(
    kernel: &LibreFangKernel,
) -> Arc<dyn librefang_runtime::mcp_oauth::McpOAuthProvider> {
    let with_bounds: Arc<dyn librefang_runtime::mcp_oauth::McpOAuthProvider + Send + Sync> =
        Arc::clone(kernel.oauth_provider_ref());
    with_bounds
}

/// Why [`LibreFangKernel::reconnect_mcp_server`] did not produce a live connection.
///
/// Typed rather than a formatted `String` so the API layer can answer with the right status without matching on prose.
/// Every one of these used to arrive as `Err(String)` and left `POST /api/mcp/servers/{name}/reconnect` no choice but 500 — including [`Self::ConnectFailed`], which is an external dependency that did not answer and is not a fault of this daemon at all.
///
/// The variants carry only what is safe to hand a caller.
/// A raw MCP error can quote a URL with a token in its query or an authenticated server's response body, which is why the route still refuses to echo one; [`Self::ConnectFailed`] instead names the failure class and the endpoint an operator configured themselves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpReconnectError {
    /// No entry with this id exists in the effective server set.
    NotConfigured { id: String },
    /// The entry exists but declares no transport, so there is nothing to dial.
    NoTransport { id: String },
    /// The server is in [`McpAuthState::NeedsAuth`](librefang_runtime::mcp_oauth::McpAuthState::NeedsAuth) and the operator has not completed the OAuth flow.
    NeedsAuth { id: String },
    /// The connection attempt reached [`McpConnection::connect`](librefang_runtime::mcp::McpConnection::connect) and failed — a process that would not spawn, a handshake that never completed, an endpoint that refused.
    ///
    /// The underlying error text stays in the `ERROR` log and in `GET /api/mcp/health`'s `last_error`; what travels here is the class plus [`transport`](Self::transport_kind) and [`target`](Self::target).
    ConnectFailed {
        id: String,
        /// Transport kind as configured: `stdio`, `sse`, `http` or `http_compat`.
        transport: &'static str,
        /// The dialed endpoint with its secret-bearing parts removed — see [`connect_target`].
        target: String,
    },
}

impl McpReconnectError {
    /// Transport kind, when the failure got far enough to know one.
    pub fn transport_kind(&self) -> Option<&'static str> {
        match self {
            Self::ConnectFailed { transport, .. } => Some(transport),
            _ => None,
        }
    }

    /// The scrubbed endpoint, when the failure got far enough to have dialed one.
    pub fn target(&self) -> Option<&str> {
        match self {
            Self::ConnectFailed { target, .. } => Some(target),
            _ => None,
        }
    }
}

impl std::fmt::Display for McpReconnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConfigured { id } => write!(f, "No MCP config found for server '{id}'"),
            Self::NoTransport { id } => write!(f, "MCP server '{id}' has no transport configured"),
            Self::NeedsAuth { id } => write!(
                f,
                "MCP server '{id}' requires OAuth authorization before reconnecting"
            ),
            Self::ConnectFailed {
                id,
                transport,
                target,
            } => write!(
                f,
                "MCP server '{id}' did not answer over {transport} transport ({target})"
            ),
        }
    }
}

impl std::error::Error for McpReconnectError {}

/// Transport kind label for [`McpReconnectError::ConnectFailed`] and the audit entry.
///
/// Exhaustive rather than a literal at each construction site so a new `McpTransport` variant fails to compile instead of silently reporting the wrong kind.
///
/// Keyed on the runtime `McpTransport` rather than the config-side `McpTransportEntry` so the one pair of helpers serves both callers: the typed error, which still holds the config entry, and `connect_mcp_wired`, which has already translated it.
pub(super) fn transport_kind(transport: &librefang_runtime::mcp::McpTransport) -> &'static str {
    use librefang_runtime::mcp::McpTransport;
    match transport {
        McpTransport::Stdio { .. } => "stdio",
        McpTransport::Sse { .. } => "sse",
        McpTransport::Http { .. } => "http",
        McpTransport::HttpCompat { .. } => "http_compat",
    }
}

/// What was dialed, with the parts that can carry a credential removed.
///
/// For `stdio` that is the program name **without its arguments** — `npx`, not `npx -y @scope/server --token=…`.
/// For the URL transports it is scheme, host and port only: no path, no query, no userinfo.
/// Both halves are values the operator typed into their own MCP config and can already read back from `GET /api/mcp/servers`; the arguments and the query string are where a token actually lives, so they stay out.
///
/// A URL that will not parse degrades to its scheme (or `invalid-url`) rather than falling back to the raw string, because the whole point is that the raw string is the thing that may hold the secret.
///
/// **The asymmetry is deliberate**: a URL loses its path, a `stdio` command keeps its own.
/// A `command` like `/opt/secrets/mcp-abc123/bin/server` therefore travels in full.
/// The two paths are not the same kind of thing — a URL path is chosen by the server operator and routinely carries session or token segments, while the command path is what the local operator typed as the program to run and *is* the answer to "which server failed", the one detail that makes a stdio failure actionable at all.
/// Dropping it to a bare basename would leave two servers running `server` from different directories indistinguishable.
pub(super) fn connect_target(transport: &librefang_runtime::mcp::McpTransport) -> String {
    use librefang_runtime::mcp::McpTransport;
    match transport {
        McpTransport::Stdio { command, .. } => command.clone(),
        McpTransport::Sse { url } | McpTransport::Http { url } => scrub_url(url),
        McpTransport::HttpCompat { base_url, .. } => scrub_url(base_url),
    }
}

/// Reduce a URL to `scheme://host[:port]`, dropping userinfo, path, query and fragment.
///
/// Built up from `scheme()` and `host_str()` rather than taken from `Url::authority()`, which would be the obvious way to fold these three lines into one: `authority()` **includes** `user:password@`, so that refactor would silently start leaking userinfo.
/// `raw_reconnect_error_never_reaches_the_client` covers that with a userinfo fixture.
fn scrub_url(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(parsed) => match parsed.host_str() {
            Some(host) => match parsed.port() {
                Some(port) => format!("{}://{host}:{port}", parsed.scheme()),
                None => format!("{}://{host}", parsed.scheme()),
            },
            None => parsed.scheme().to_string(),
        },
        Err(_) => "invalid-url".to_string(),
    }
}

impl LibreFangKernel {
    /// Connect an MCP server and wire its transport-health reporter (#7963).
    ///
    /// Every connect site in the kernel goes through here rather than calling `McpConnection::connect` directly, because a connection built without a reporter silently loses auto-reconnect: its tool-call failures reach no health record, so `should_reconnect` never fires and the server stays wedged until the daemon restarts.
    /// That was the whole of #7963, and one choke point is what keeps a future connect site from reintroducing it.
    ///
    /// `pub(super)` because the fifth connect site — the per-agent workspace-scoped pool in `accessors.rs` — lives in a sibling module and needs the same wiring.
    pub(super) async fn connect_mcp_wired(
        &self,
        config: librefang_runtime::mcp::McpServerConfig,
    ) -> Result<librefang_runtime::mcp::McpConnection, String> {
        let reporter = crate::mcp_health_reporter::KernelMcpHealthReporter::shared(Arc::clone(
            &self.mcp.mcp_health,
        ));
        let name = config.name.clone();
        let transport = transport_kind(&config.transport);
        let target = connect_target(&config.transport);
        librefang_runtime::mcp::McpConnection::connect(config)
            .await
            .map(|conn| conn.with_health_reporter(reporter))
            .inspect_err(|e| {
                // `OAUTH_NEEDS_AUTH` is not a failure: it is the connect path's signal
                // that the server answered 401 and the PKCE flow belongs to the UI, which
                // `connect_mcp_servers` turns into `McpAuthState::NeedsAuth` one frame up.
                // Auditing it as a connect failure would mark a server that is working
                // exactly as designed as broken, once per boot and once per reload for as
                // long as the operator has not signed in.
                if e != OAUTH_NEEDS_AUTH {
                    self.audit_mcp_connect_failure(&name, transport, &target);
                }
            })
    }

    /// Record a failed MCP connect in the audit trail so it reaches the dashboard's Logs page.
    ///
    /// Until this existed the only trace of an MCP server that would not start was a `tracing` `ERROR` in the daemon's journal.
    /// The Logs page reads `GET /api/audit/recent`, and `/api/logs/stream` streams the same audit entries — neither has ever carried the daemon's `tracing` output — so the screen an operator opens when something breaks was empty by construction while the daemon was writing errors.
    /// Emitting here rather than at the HTTP handler covers every connect path at once: boot, `reload_mcp_servers`, the operator's reconnect, the health loop's auto-reconnect and the per-agent pool in `accessors.rs` all funnel through this one method, which the #7963 drift guard already forces them to.
    ///
    /// `outcome` leads with `error` because that is what the dashboard's `auditLogLevel` reads to colour a row — the level is derived from the outcome text, not from the action.
    ///
    /// **This does not flood the audit log**, which is the reason it needs no rate limiting: `reconnect_attempts` is reset only by `mark_ok`, and `should_reconnect` requires it to stay under `max_reconnect_attempts` (10), so a server that is permanently broken contributes roughly eleven entries per daemon lifetime plus one per operator click.
    fn audit_mcp_connect_failure(&self, name: &str, transport: &'static str, target: &str) {
        use crate::MeteringSubsystemApi;
        MeteringSubsystemApi::audit_log(self).record(
            "system",
            crate::audit::AuditAction::McpConnect,
            format!("MCP server '{name}' ({transport}: {target})"),
            "error: connect failed",
        );
    }

    async fn mcp_connection_requires_auth(&self, server_name: &str) -> bool {
        matches!(
            self.mcp.mcp_auth_states.lock().await.get(server_name),
            Some(librefang_runtime::mcp_oauth::McpAuthState::NeedsAuth)
        )
    }

    /// Connect to all configured MCP servers and cache their tool definitions.
    ///
    /// Idempotent: servers that already have a live connection are skipped.
    /// Called at boot and after hot-reload adds/updates MCP server config.
    pub async fn connect_mcp_servers(self: &Arc<Self>) {
        use librefang_runtime::mcp::{McpServerConfig, McpTransport};
        use librefang_types::config::McpTransportEntry;

        let _connection_op = self.mcp.mcp_connection_ops.lock().await;
        let servers = self
            .mcp
            .effective_mcp_servers
            .read()
            .map(|s| s.clone())
            .unwrap_or_default();

        for server_config in &servers {
            if self.mcp_connection_requires_auth(&server_config.name).await {
                continue;
            }
            // Skip servers that already have a live connection (idempotent).
            {
                let conns = self.mcp.mcp_connections.lock().await;
                if conns.iter().any(|c| c.name() == server_config.name) {
                    continue;
                }
            }

            let transport_entry = match &server_config.transport {
                Some(t) => t,
                None => {
                    tracing::warn!(name = %server_config.name, "MCP server has no transport configured, skipping");
                    continue;
                }
            };
            let transport = match transport_entry {
                McpTransportEntry::Stdio { command, args } => McpTransport::Stdio {
                    command: command.clone(),
                    args: args.clone(),
                },
                McpTransportEntry::Sse { url } => McpTransport::Sse { url: url.clone() },
                McpTransportEntry::Http { url } => McpTransport::Http { url: url.clone() },
                McpTransportEntry::HttpCompat {
                    base_url,
                    headers,
                    tools,
                } => McpTransport::HttpCompat {
                    base_url: base_url.clone(),
                    headers: headers.clone(),
                    tools: tools.clone(),
                },
            };

            let mcp_config = McpServerConfig {
                name: server_config.name.clone(),
                transport,
                timeout_secs: server_config.timeout_secs,
                env: server_config.env.clone(),
                headers: server_config.headers.clone(),
                oauth_provider: Some(oauth_provider_clone(self)),
                oauth_config: server_config.oauth.clone(),
                taint_scanning: server_config.taint_scanning,
                taint_policy: server_config.taint_policy.clone(),
                taint_rule_sets: self.snapshot_taint_rules(),
                roots: self.mcp_roots_for_server(server_config),
            };

            match self.connect_mcp_wired(mcp_config).await {
                Ok(conn) => {
                    let tool_count = conn.tools().len();
                    // Cache tool definitions
                    if let Ok(mut tools) = self.mcp.mcp_tools.lock() {
                        tools.extend(conn.tools().iter().cloned());
                        self.mcp
                            .mcp_generation
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                    info!(
                        server = %server_config.name,
                        tools = tool_count,
                        "MCP server connected"
                    );
                    // Update extension health if this is an extension-provided server
                    self.mcp
                        .mcp_health
                        .report_ok(&server_config.name, tool_count);
                    self.mcp.mcp_connections.lock().await.push(conn);
                }
                Err(e) => {
                    let err_str = e.to_string();

                    // Check if this is an OAuth-needed signal (HTTP 401 from an
                    // MCP server that supports OAuth). The MCP connection layer
                    // returns "OAUTH_NEEDS_AUTH" when auth is required but defers
                    // the actual PKCE flow to the API layer.
                    if err_str == OAUTH_NEEDS_AUTH {
                        info!(
                            server = %server_config.name,
                            "MCP server requires OAuth — waiting for UI-driven auth"
                        );
                        self.mcp.mcp_auth_states.lock().await.insert(
                            server_config.name.clone(),
                            librefang_runtime::mcp_oauth::McpAuthState::NeedsAuth,
                        );
                    } else {
                        warn!(
                            server = %server_config.name,
                            error = %e,
                            "Failed to connect to MCP server"
                        );
                    }
                    self.mcp
                        .mcp_health
                        .report_error(&server_config.name, err_str);
                }
            }
        }

        let tool_count = self.mcp.mcp_tools.lock().map(|t| t.len()).unwrap_or(0);
        if tool_count > 0 {
            info!(
                "MCP: {tool_count} tools available from {} server(s)",
                self.mcp.mcp_connections.lock().await.len()
            );
        }
    }

    /// Disconnect an MCP server by name, removing it from the live connection list.
    ///
    /// The dropped `McpConnection` will shut down the underlying transport.
    /// Returns `true` if a connection was found and removed.
    pub async fn disconnect_mcp_server(&self, name: &str) -> bool {
        let _connection_op = self.mcp.mcp_connection_ops.lock().await;
        // Extract the matching connection(s) so we can close them explicitly
        // rather than relying on the implicit Drop path.  Explicit close ensures
        // the underlying stdio child process is reaped before we return, which
        // prevents subprocess leaks on hot-reload. (#3800)
        let removed_conns: Vec<librefang_runtime::mcp::McpConnection> = {
            let mut conns = self.mcp.mcp_connections.lock().await;
            let mut extracted = Vec::new();
            let mut i = 0;
            while i < conns.len() {
                if conns[i].name() == name {
                    extracted.push(conns.remove(i));
                } else {
                    i += 1;
                }
            }
            extracted
        };

        let removed = !removed_conns.is_empty();
        if removed {
            // Remove cached tools from this server and bump generation.
            // MCP tools are prefixed: mcp_{normalized_server_name}_{tool_name}
            let prefix = format!("mcp_{}_", librefang_runtime::mcp::normalize_name(name));
            if let Ok(mut tools) = self.mcp.mcp_tools.lock() {
                tools.retain(|t| !t.name.starts_with(&prefix));
            }
            self.mcp
                .mcp_generation
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            info!(server = %name, "MCP server disconnected");

            // Close each extracted connection after releasing the lock.
            // For stdio connections this waits for the rmcp service task to
            // finish and the child process to be killed. (#3800)
            for conn in removed_conns {
                conn.close().await;
            }
        }
        removed
    }

    /// Watch for OAuth completion by polling the vault for a stored access token.
    ///
    /// Polls every 10 seconds for up to 5 minutes. When a token appears, calls
    /// `retry_mcp_connection` to establish the MCP connection.
    ///
    /// Note: Currently unused — the API layer drives OAuth completion via the
    /// callback endpoint. Retained for potential future use by non-API flows.
    /// Retry connecting to a specific MCP server by name.
    ///
    /// Looks up the server config, builds an `McpServerConfig`, and attempts
    /// to connect. On success, adds the connection and updates auth state.
    pub async fn retry_mcp_connection(self: &Arc<Self>, server_name: &str) {
        use librefang_runtime::mcp::{McpServerConfig, McpTransport};
        use librefang_types::config::McpTransportEntry;

        let _connection_op = self.mcp.mcp_connection_ops.lock().await;
        if self.mcp_connection_requires_auth(server_name).await {
            return;
        }
        if self
            .mcp
            .mcp_connections
            .lock()
            .await
            .iter()
            .any(|connection| connection.name() == server_name)
        {
            return;
        }

        let server_config = {
            let servers = self
                .mcp
                .effective_mcp_servers
                .read()
                .map(|s| s.clone())
                .unwrap_or_default();
            servers.into_iter().find(|s| s.name == server_name)
        };

        let server_config = match server_config {
            Some(c) => c,
            None => {
                warn!(server = %server_name, "MCP server config not found for retry");
                return;
            }
        };

        let transport_entry = match &server_config.transport {
            Some(t) => t,
            None => {
                warn!(server = %server_name, "MCP server has no transport for retry");
                return;
            }
        };

        let transport = match transport_entry {
            McpTransportEntry::Stdio { command, args } => McpTransport::Stdio {
                command: command.clone(),
                args: args.clone(),
            },
            McpTransportEntry::Sse { url } => McpTransport::Sse { url: url.clone() },
            McpTransportEntry::Http { url } => McpTransport::Http { url: url.clone() },
            McpTransportEntry::HttpCompat {
                base_url,
                headers,
                tools,
            } => McpTransport::HttpCompat {
                base_url: base_url.clone(),
                headers: headers.clone(),
                tools: tools.clone(),
            },
        };

        let mcp_config = McpServerConfig {
            name: server_config.name.clone(),
            transport,
            timeout_secs: server_config.timeout_secs,
            env: server_config.env.clone(),
            headers: server_config.headers.clone(),
            oauth_provider: Some(oauth_provider_clone(self)),
            oauth_config: server_config.oauth.clone(),
            taint_scanning: server_config.taint_scanning,
            taint_policy: server_config.taint_policy.clone(),
            taint_rule_sets: self.snapshot_taint_rules(),
            roots: self.mcp_roots_for_server(&server_config),
        };

        match self.connect_mcp_wired(mcp_config).await {
            Ok(conn) => {
                let tool_count = conn.tools().len();
                if let Ok(mut tools) = self.mcp.mcp_tools.lock() {
                    tools.extend(conn.tools().iter().cloned());
                    self.mcp
                        .mcp_generation
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                info!(
                    server = %server_name,
                    tools = tool_count,
                    "MCP server connected after OAuth"
                );
                self.mcp
                    .mcp_health
                    .report_ok(&server_config.name, tool_count);
                self.mcp.mcp_connections.lock().await.push(conn);

                // Update auth state to Authorized
                self.mcp.mcp_auth_states.lock().await.insert(
                    server_name.to_string(),
                    librefang_runtime::mcp_oauth::McpAuthState::Authorized {
                        expires_at: None,
                        tokens: None,
                    },
                );
            }
            Err(e) => {
                warn!(
                    server = %server_name,
                    error = %e,
                    "MCP server retry after OAuth failed"
                );
                self.mcp
                    .mcp_health
                    .report_error(&server_config.name, e.to_string());
                self.mcp.mcp_auth_states.lock().await.insert(
                    server_name.to_string(),
                    librefang_runtime::mcp_oauth::McpAuthState::Error {
                        message: format!("Connection failed after auth: {e}"),
                    },
                );
            }
        }
    }

    /// Reload MCP server configs and (re)connect every server in config.toml.
    ///
    /// Called by `POST /api/mcp/reload` and by the API handlers for
    /// `POST/PUT/DELETE /api/mcp/servers[/{id}]` after they mutate config.toml.
    ///
    /// Returns the number of *newly connected* servers (not the total count).
    pub async fn reload_mcp_servers(self: &Arc<Self>) -> Result<usize, String> {
        use librefang_runtime::mcp::{McpServerConfig, McpTransport};
        use librefang_types::config::McpTransportEntry;

        let _connection_op = self.mcp.mcp_connection_ops.lock().await;
        let cfg = self.config.load_full();
        // 1. Reload the MCP catalog from disk (new templates may have landed
        //    after `registry_sync`). Atomic swap — readers never blocked.
        let catalog_count = self.mcp_catalog_reload(&cfg.home_dir);

        // 2. Effective server list = config.mcp_servers merged with the
        //    DB-backed `mcp_server_configs` table (DB wins by name), mirroring
        //    the boot-time overlay via the shared `merge_over` helper. Without
        //    this, a runtime DB write (`mcp_runtime_store = "db"`, #6113) would
        //    only take effect after a restart, and a hot-reload would drop the
        //    DB-backed servers the boot merge had applied. Empty table = the
        //    file-only list, unchanged.
        let new_configs = {
            let store = librefang_memory::McpConfigStore::new(self.memory.substrate.pool());
            match store.merge_over(cfg.mcp_servers.clone()) {
                Ok((merged, _added, _overridden)) => merged,
                Err(e) => {
                    warn!("reload_mcp_servers: failed to merge DB-backed MCP configs: {e}");
                    cfg.mcp_servers.clone()
                }
            }
        };

        let old_configs = self
            .mcp
            .effective_mcp_servers
            .read()
            .map(|servers| servers.clone())
            .unwrap_or_default();
        let old_by_name: std::collections::HashMap<&str, _> = old_configs
            .iter()
            .map(|server| (server.name.as_str(), server))
            .collect();
        let new_by_name: std::collections::HashMap<&str, _> = new_configs
            .iter()
            .map(|server| (server.name.as_str(), server))
            .collect();
        let mut auth_state_resets = std::collections::HashSet::new();
        for old in &old_configs {
            match new_by_name.get(old.name.as_str()) {
                None => {
                    auth_state_resets.insert(old.name.clone());
                }
                Some(new) if serde_json::to_value(old).ok() != serde_json::to_value(new).ok() => {
                    auth_state_resets.insert(old.name.clone());
                }
                Some(_) => {}
            }
        }
        for new in &new_configs {
            if !old_by_name.contains_key(new.name.as_str()) {
                auth_state_resets.insert(new.name.clone());
            }
        }
        if !auth_state_resets.is_empty() {
            let mut auth_states = self.mcp.mcp_auth_states.lock().await;
            for name in &auth_state_resets {
                auth_states.remove(name);
            }
        }

        // 3. Find servers that aren't already connected
        let already_connected: Vec<String> = self
            .mcp
            .mcp_connections
            .lock()
            .await
            .iter()
            .map(|c| c.name().to_string())
            .collect();

        let new_servers: Vec<_> = new_configs
            .iter()
            .filter(|s| !already_connected.contains(&s.name))
            .cloned()
            .collect();

        // 4. Update effective list; bump mcp_generation inside the same write lock so cached summaries invalidate atomically.
        if let Ok(mut effective) = self.mcp.effective_mcp_servers.write() {
            *effective = new_configs;
            self.mcp
                .mcp_generation
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }

        // 5. Connect new servers
        let mut connected_count = 0;
        for server_config in &new_servers {
            if self.mcp_connection_requires_auth(&server_config.name).await {
                continue;
            }
            let transport_entry = match &server_config.transport {
                Some(t) => t,
                None => {
                    continue;
                }
            };
            let transport = match transport_entry {
                McpTransportEntry::Stdio { command, args } => McpTransport::Stdio {
                    command: command.clone(),
                    args: args.clone(),
                },
                McpTransportEntry::Sse { url } => McpTransport::Sse { url: url.clone() },
                McpTransportEntry::Http { url } => McpTransport::Http { url: url.clone() },
                McpTransportEntry::HttpCompat {
                    base_url,
                    headers,
                    tools,
                } => McpTransport::HttpCompat {
                    base_url: base_url.clone(),
                    headers: headers.clone(),
                    tools: tools.clone(),
                },
            };

            let mcp_config = McpServerConfig {
                name: server_config.name.clone(),
                transport,
                timeout_secs: server_config.timeout_secs,
                env: server_config.env.clone(),
                headers: server_config.headers.clone(),
                oauth_provider: Some(oauth_provider_clone(self)),
                oauth_config: server_config.oauth.clone(),
                taint_scanning: server_config.taint_scanning,
                taint_policy: server_config.taint_policy.clone(),
                taint_rule_sets: self.snapshot_taint_rules(),
                roots: self.mcp_roots_for_server(server_config),
            };

            self.mcp.mcp_health.register(&server_config.name);

            match self.connect_mcp_wired(mcp_config).await {
                Ok(conn) => {
                    let tool_count = conn.tools().len();
                    if let Ok(mut tools) = self.mcp.mcp_tools.lock() {
                        tools.extend(conn.tools().iter().cloned());
                        self.mcp
                            .mcp_generation
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                    self.mcp
                        .mcp_health
                        .report_ok(&server_config.name, tool_count);
                    info!(
                        server = %server_config.name,
                        tools = tool_count,
                        "MCP server connected (hot-reload)"
                    );
                    self.mcp.mcp_connections.lock().await.push(conn);
                    connected_count += 1;
                }
                Err(e) => {
                    self.mcp
                        .mcp_health
                        .report_error(&server_config.name, e.to_string());
                    warn!(
                        server = %server_config.name,
                        error = %e,
                        "Failed to connect MCP server"
                    );
                }
            }
        }

        // 6. Remove connections for servers no longer in config
        let removed: Vec<String> = already_connected
            .iter()
            .filter(|name| {
                let effective = self
                    .mcp
                    .effective_mcp_servers
                    .read()
                    .unwrap_or_else(|e| e.into_inner());
                !effective.iter().any(|s| &s.name == *name)
            })
            .cloned()
            .collect();

        if !removed.is_empty() {
            // Extract the connections to remove so we can close them explicitly
            // after releasing the lock, preventing subprocess leaks on hot-reload. (#3800)
            let conns_to_close: Vec<librefang_runtime::mcp::McpConnection> = {
                let mut conns = self.mcp.mcp_connections.lock().await;
                let mut extracted = Vec::new();
                let mut i = 0;
                while i < conns.len() {
                    if removed.contains(&conns[i].name().to_string()) {
                        extracted.push(conns.remove(i));
                    } else {
                        i += 1;
                    }
                }
                // Rebuild tool cache with remaining connections.
                if let Ok(mut tools) = self.mcp.mcp_tools.lock() {
                    tools.clear();
                    for conn in conns.iter() {
                        tools.extend(conn.tools().iter().cloned());
                    }
                    self.mcp
                        .mcp_generation
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                extracted
            };
            for name in &removed {
                self.mcp.mcp_health.unregister(name);
                info!(server = %name, "MCP server disconnected (removed)");
            }
            // Close extracted connections after releasing the lock. (#3800)
            for conn in conns_to_close {
                conn.close().await;
            }
        }

        info!(
            "MCP reload: catalog={catalog_count}, {connected_count} new connections, {} removed",
            removed.len()
        );
        Ok(connected_count)
    }

    /// Reconnect a single MCP server by id.
    ///
    /// The error is [`McpReconnectError`] rather than a formatted string so callers can tell an external server that would not answer from a config problem on this side — see that type for why the distinction reaches HTTP.
    pub async fn reconnect_mcp_server(
        self: &Arc<Self>,
        id: &str,
    ) -> Result<usize, McpReconnectError> {
        use librefang_runtime::mcp::{McpServerConfig, McpTransport};
        use librefang_types::config::McpTransportEntry;

        let _connection_op = self.mcp.mcp_connection_ops.lock().await;
        // Find the config for this server
        let server_config = {
            let effective = self
                .mcp
                .effective_mcp_servers
                .read()
                .unwrap_or_else(|e| e.into_inner());
            effective.iter().find(|s| s.name == id).cloned()
        };

        let server_config =
            server_config.ok_or_else(|| McpReconnectError::NotConfigured { id: id.to_string() })?;
        if self.mcp_connection_requires_auth(id).await {
            return Err(McpReconnectError::NeedsAuth { id: id.to_string() });
        }

        // Disconnect existing connection if any
        {
            let mut conns = self.mcp.mcp_connections.lock().await;
            let old_len = conns.len();
            conns.retain(|c| c.name() != id);
            if conns.len() < old_len {
                // Rebuild tool cache
                if let Ok(mut tools) = self.mcp.mcp_tools.lock() {
                    tools.clear();
                    for conn in conns.iter() {
                        tools.extend(conn.tools().iter().cloned());
                    }
                    self.mcp
                        .mcp_generation
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }

        self.mcp.mcp_health.mark_reconnecting(id);

        let transport_entry = match &server_config.transport {
            Some(t) => t,
            None => {
                let error = McpReconnectError::NoTransport { id: id.to_string() };
                self.mcp.mcp_health.report_error(id, error.to_string());
                return Err(error);
            }
        };
        let dialed = match transport_entry {
            McpTransportEntry::Stdio { command, args } => McpTransport::Stdio {
                command: command.clone(),
                args: args.clone(),
            },
            McpTransportEntry::Sse { url } => McpTransport::Sse { url: url.clone() },
            McpTransportEntry::Http { url } => McpTransport::Http { url: url.clone() },
            McpTransportEntry::HttpCompat {
                base_url,
                headers,
                tools,
            } => McpTransport::HttpCompat {
                base_url: base_url.clone(),
                headers: headers.clone(),
                tools: tools.clone(),
            },
        };

        // Captured before `dialed` moves into the config: the failure arm needs them, and re-deriving there would be a second match to keep in step with this one.
        let failed_transport = transport_kind(&dialed);
        let failed_target = connect_target(&dialed);

        let mcp_config = McpServerConfig {
            name: server_config.name.clone(),
            transport: dialed,
            timeout_secs: server_config.timeout_secs,
            env: server_config.env.clone(),
            headers: server_config.headers.clone(),
            oauth_provider: Some(oauth_provider_clone(self)),
            oauth_config: server_config.oauth.clone(),
            taint_scanning: server_config.taint_scanning,
            taint_policy: server_config.taint_policy.clone(),
            taint_rule_sets: self.snapshot_taint_rules(),
            roots: self.mcp_roots_for_server(&server_config),
        };

        match self.connect_mcp_wired(mcp_config).await {
            Ok(conn) => {
                let tool_count = conn.tools().len();
                if let Ok(mut tools) = self.mcp.mcp_tools.lock() {
                    tools.extend(conn.tools().iter().cloned());
                    self.mcp
                        .mcp_generation
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                self.mcp.mcp_health.report_ok(id, tool_count);
                info!(
                    server = %id,
                    tools = tool_count,
                    "MCP server reconnected"
                );
                self.mcp.mcp_connections.lock().await.push(conn);
                // Cardinality: server label is the operator-configured MCP
                // server id (bounded set), outcome is one of two fixed
                // values. (#3495)
                metrics::counter!(
                    "librefang_mcp_reconnect_total",
                    "server" => id.to_string(),
                    "outcome" => "success",
                )
                .increment(1);
                Ok(tool_count)
            }
            Err(e) => {
                self.mcp.mcp_health.report_error(id, e.to_string());
                metrics::counter!(
                    "librefang_mcp_reconnect_total",
                    "server" => id.to_string(),
                    "outcome" => "failure",
                )
                .increment(1);
                // The same sentinel `connect_mcp_servers` special-cases twenty lines up:
                // the server answered 401 and wants the UI-driven PKCE flow. Reporting
                // that as "the server did not answer" would send the operator hunting a
                // network fault when what they owe it is a sign-in. The pre-check above
                // only catches a server *already* in `NeedsAuth`; this is the reconnect
                // that discovers it.
                if e == OAUTH_NEEDS_AUTH {
                    self.mcp.mcp_auth_states.lock().await.insert(
                        id.to_string(),
                        librefang_runtime::mcp_oauth::McpAuthState::NeedsAuth,
                    );
                    return Err(McpReconnectError::NeedsAuth { id: id.to_string() });
                }
                Err(McpReconnectError::ConnectFailed {
                    id: id.to_string(),
                    transport: failed_transport,
                    target: failed_target,
                })
            }
        }
    }

    /// Background loop that checks MCP server health and auto-reconnects.
    pub(crate) async fn run_mcp_health_loop(self: &Arc<Self>) {
        let interval_secs = self.mcp.mcp_health.config().check_interval_secs;
        if interval_secs == 0 {
            return;
        }

        let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
        interval.tick().await; // skip first immediate tick

        loop {
            interval.tick().await;

            // Check each registered server
            let health_entries = self.mcp.mcp_health.all_health();
            for entry in health_entries {
                // Try reconnect for errored servers
                if self.mcp.mcp_health.should_reconnect(&entry.id) {
                    let backoff = self
                        .mcp
                        .mcp_health
                        .backoff_duration(entry.reconnect_attempts);
                    debug!(
                        server = %entry.id,
                        attempt = entry.reconnect_attempts + 1,
                        backoff_secs = backoff.as_secs(),
                        "Auto-reconnecting MCP server"
                    );
                    tokio::time::sleep(backoff).await;

                    if let Err(e) = self.reconnect_mcp_server(&entry.id).await {
                        debug!(server = %entry.id, error = %e, "Auto-reconnect failed");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    /// Every `.rs` file under this crate's `src/`, read from disk at test time.
    ///
    /// Read from disk rather than `include_str!` because the invariant below is about files this module has never heard of — a `include_str!` list can only pin the connect sites someone already remembered to add to it, which is precisely the drift being guarded against.
    fn crate_sources() -> Vec<(PathBuf, String)> {
        fn walk(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
            let entries = std::fs::read_dir(dir)
                .unwrap_or_else(|e| panic!("reading {} failed: {e}", dir.display()));
            for entry in entries {
                let path = entry.expect("dir entry").path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let body = std::fs::read_to_string(&path)
                        .unwrap_or_else(|e| panic!("reading {} failed: {e}", path.display()));
                    out.push((path, body));
                }
            }
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut out = Vec::new();
        walk(&root, &mut out);
        out
    }

    /// Drift guard for #7963: every MCP connect site in this crate must go through `connect_mcp_wired`, which attaches the transport-health reporter.
    ///
    /// A connection built by a bare `connect` compiles and works — it just silently has no health reporter, so its tool-call failures reach no health record and auto-reconnect never fires for that server.
    /// That failure mode is invisible at runtime until a server wedges and stays wedged, which is exactly the two days of broken tool calls #7963 reported, so it is pinned here instead.
    ///
    /// The guard scans the whole crate rather than a hand-maintained module list, because a hand-maintained list is the same class of omission it is meant to catch: the first version of this test scanned only `mcp_setup.rs` and so did not notice that `accessors.rs::build_agent_mcp_pool` — the per-agent workspace-scoped pool `execute_llm_agent`, `ephemeral_spawn` and the messaging path *prefer* over the daemon-global one — still had its own bare `connect`.
    ///
    /// Needles are assembled from fragments so this test's own source does not count as a match against the files it scans.
    #[test]
    fn every_connect_site_wires_the_transport_health_reporter() {
        let direct_connect = concat!("McpConnection", "::connect(");
        let helper_call = concat!("self.connect_mcp", "_wired(");
        let helper_decl = concat!("fn connect_mcp", "_wired");
        let attach = concat!("with_health", "_reporter");

        let sources = crate_sources();
        assert!(
            sources.len() > 20,
            "the source walk found only {} files — it is not scanning the crate, so every \
             assertion below would pass vacuously",
            sources.len()
        );

        let mut direct_sites: Vec<String> = Vec::new();
        let mut helper_calls = 0usize;
        let mut helper_declared_in = None;
        for (path, body) in &sources {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            for _ in 0..body.matches(direct_connect).count() {
                direct_sites.push(name.clone());
            }
            helper_calls += body.matches(helper_call).count();
            if body.contains(helper_decl) && body.contains(attach) {
                helper_declared_in = Some(name);
            }
        }

        assert_eq!(
            helper_declared_in.as_deref(),
            Some("mcp_setup.rs"),
            "the wiring helper must be declared, and must attach the health reporter, in \
             mcp_setup.rs — without it the counts below mean nothing"
        );
        assert_eq!(
            direct_sites,
            vec!["mcp_setup.rs".to_string()],
            "exactly one bare `connect` may exist in this crate: the one inside the wiring \
             helper in mcp_setup.rs. Route every other connect site through \
             `connect_mcp_wired` so the #7963 health reporter is always attached."
        );
        assert!(
            helper_calls >= 5,
            "expected at least five wired connect sites (boot connect, retry, reload, \
             reconnect, per-agent pool), found {helper_calls}"
        );
    }
}
