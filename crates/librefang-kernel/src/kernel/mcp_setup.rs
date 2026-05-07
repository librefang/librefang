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

impl LibreFangKernel {
    /// Connect to all configured MCP servers and cache their tool definitions.
    ///
    /// Idempotent: servers that already have a live connection are skipped.
    /// Called at boot and after hot-reload adds/updates MCP server config.
    pub async fn connect_mcp_servers(self: &Arc<Self>) {
        use librefang_runtime::mcp::{McpConnection, McpServerConfig, McpTransport};
        use librefang_types::config::McpTransportEntry;

        let servers = self
            .mcp
            .effective_mcp_servers
            .read()
            .map(|s| s.clone())
            .unwrap_or_default();

        for server_config in &servers {
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
                oauth_provider: Some(self.oauth_provider_ref()),
                oauth_config: server_config.oauth.clone(),
                taint_scanning: server_config.taint_scanning,
                taint_policy: server_config.taint_policy.clone(),
                taint_rule_sets: self.snapshot_taint_rules(),
                roots: self.mcp_roots_for_server(server_config),
            };

            match McpConnection::connect(mcp_config).await {
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
                    if err_str == "OAUTH_NEEDS_AUTH" {
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
        use librefang_runtime::mcp::{McpConnection, McpServerConfig, McpTransport};
        use librefang_types::config::McpTransportEntry;

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
            oauth_provider: Some(self.oauth_provider_ref()),
            oauth_config: server_config.oauth.clone(),
            taint_scanning: server_config.taint_scanning,
            taint_policy: server_config.taint_policy.clone(),
            taint_rule_sets: self.snapshot_taint_rules(),
            roots: self.mcp_roots_for_server(&server_config),
        };

        match McpConnection::connect(mcp_config).await {
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
        let cfg = self.config.load_full();
        use librefang_runtime::mcp::{McpConnection, McpServerConfig, McpTransport};
        use librefang_types::config::McpTransportEntry;

        // 1. Reload the MCP catalog from disk (new templates may have landed
        //    after `registry_sync`). Atomic swap — readers never blocked.
        let catalog_count = self.mcp_catalog_reload(&cfg.home_dir);

        // 2. Effective server list == config.mcp_servers (no merge needed).
        let new_configs = cfg.mcp_servers.clone();

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
                oauth_provider: Some(self.oauth_provider_ref()),
                oauth_config: server_config.oauth.clone(),
                taint_scanning: server_config.taint_scanning,
                taint_policy: server_config.taint_policy.clone(),
                taint_rule_sets: self.snapshot_taint_rules(),
                roots: self.mcp_roots_for_server(server_config),
            };

            self.mcp.mcp_health.register(&server_config.name);

            match McpConnection::connect(mcp_config).await {
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
    pub async fn reconnect_mcp_server(self: &Arc<Self>, id: &str) -> Result<usize, String> {
        use librefang_runtime::mcp::{McpConnection, McpServerConfig, McpTransport};
        use librefang_types::config::McpTransportEntry;

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
            server_config.ok_or_else(|| format!("No MCP config found for server '{id}'"))?;

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
                return Err(format!(
                    "MCP server '{}' has no transport configured",
                    server_config.name
                ));
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
            oauth_provider: Some(self.oauth_provider_ref()),
            oauth_config: server_config.oauth.clone(),
            taint_scanning: server_config.taint_scanning,
            taint_policy: server_config.taint_policy.clone(),
            taint_rule_sets: self.snapshot_taint_rules(),
            roots: self.mcp_roots_for_server(&server_config),
        };

        match McpConnection::connect(mcp_config).await {
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
                Err(format!("Reconnect failed for '{id}': {e}"))
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
