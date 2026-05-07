//! MCP subsystem — connection pool, OAuth, tool cache, server catalog,
//! health monitor, and the rendered-summary cache.
//!
//! Bundles every MCP-related field that previously sat as a flat
//! cluster on `LibreFangKernel`. The `mcp_` prefix is kept on inner
//! fields so the migration is purely mechanical
//! (`self.mcp_X` → `self.mcp.mcp_X`).

use std::sync::Arc;

use arc_swap::ArcSwap;
use dashmap::DashMap;
use librefang_extensions::catalog::McpCatalog;
use librefang_extensions::health::HealthMonitor;
use librefang_runtime::mcp::McpConnection;
use librefang_runtime::mcp_oauth::{McpAuthStates, McpOAuthProvider};
use librefang_types::config::McpServerConfigEntry;
use librefang_types::tool::ToolDefinition;
use std::sync::atomic::AtomicU64;

/// Focused MCP API.
pub trait McpSubsystemApi: Send + Sync {
    /// `ArcSwap`-backed catalog handle.
    fn mcp_catalog_swap(&self) -> &ArcSwap<McpCatalog>;
    /// Cheap atomic snapshot of the catalog.
    fn mcp_catalog_load(&self) -> arc_swap::Guard<Arc<McpCatalog>>;
    /// MCP server health monitor.
    fn health(&self) -> &HealthMonitor;
    /// MCP connection pool.
    fn connections_ref(&self) -> &tokio::sync::Mutex<Vec<McpConnection>>;
    /// Per-server OAuth authentication state.
    fn auth_states_ref(&self) -> &McpAuthStates;
    /// Pluggable OAuth provider.
    fn oauth_provider_ref(&self) -> &Arc<dyn McpOAuthProvider + Send + Sync>;
    /// MCP tool definitions cache.
    fn tools_ref(&self) -> &std::sync::Mutex<Vec<ToolDefinition>>;
    /// Effective MCP server list.
    fn effective_servers_ref(&self) -> &std::sync::RwLock<Vec<McpServerConfigEntry>>;
}

/// MCP cluster — see module docs.
pub struct McpSubsystem {
    /// MCP server connections (lazily initialized at start_background_agents).
    pub(crate) mcp_connections: tokio::sync::Mutex<Vec<McpConnection>>,
    /// Per-server MCP OAuth authentication state.
    pub(crate) mcp_auth_states: McpAuthStates,
    /// Pluggable OAuth provider for MCP server authorization flows.
    pub(crate) mcp_oauth_provider: Arc<dyn McpOAuthProvider + Send + Sync>,
    /// MCP tool definitions cache (populated after connections are
    /// established).
    pub(crate) mcp_tools: std::sync::Mutex<Vec<ToolDefinition>>,
    /// Rendered MCP summary cache keyed by allowlist + mcp_generation;
    /// skips Mutex + re-render on hit.
    pub(crate) mcp_summary_cache: DashMap<String, (u64, String)>,
    /// MCP catalog — read-only set of server templates shipped by the
    /// registry. Lock-free reads via `ArcSwap`; writes use `rcu()`.
    pub(crate) mcp_catalog: ArcSwap<McpCatalog>,
    /// MCP server health monitor.
    pub(crate) mcp_health: HealthMonitor,
    /// Effective MCP server list — mirrors `config.mcp_servers`. Kept as
    /// its own field so hot-reload and tests can snapshot the list
    /// atomically.
    pub(crate) effective_mcp_servers: std::sync::RwLock<Vec<McpServerConfigEntry>>,
    /// Generation counter for MCP tool definitions — bumped whenever
    /// `mcp_tools` is modified (connect, disconnect, rebuild). Used by
    /// the tool list cache.
    pub(crate) mcp_generation: AtomicU64,
}

impl McpSubsystem {
    pub(crate) fn new(
        mcp_oauth_provider: Arc<dyn McpOAuthProvider + Send + Sync>,
        mcp_catalog: McpCatalog,
        mcp_health: HealthMonitor,
        effective_mcp_servers: Vec<McpServerConfigEntry>,
    ) -> Self {
        Self {
            mcp_connections: tokio::sync::Mutex::new(Vec::new()),
            mcp_auth_states: tokio::sync::Mutex::new(std::collections::HashMap::new()),
            mcp_oauth_provider,
            mcp_tools: std::sync::Mutex::new(Vec::new()),
            mcp_summary_cache: DashMap::new(),
            mcp_catalog: ArcSwap::from_pointee(mcp_catalog),
            mcp_health,
            effective_mcp_servers: std::sync::RwLock::new(effective_mcp_servers),
            mcp_generation: AtomicU64::new(0),
        }
    }
}

impl McpSubsystemApi for McpSubsystem {
    #[inline]
    fn mcp_catalog_swap(&self) -> &ArcSwap<McpCatalog> {
        &self.mcp_catalog
    }

    #[inline]
    fn mcp_catalog_load(&self) -> arc_swap::Guard<Arc<McpCatalog>> {
        self.mcp_catalog.load()
    }

    #[inline]
    fn health(&self) -> &HealthMonitor {
        &self.mcp_health
    }

    #[inline]
    fn connections_ref(&self) -> &tokio::sync::Mutex<Vec<McpConnection>> {
        &self.mcp_connections
    }

    #[inline]
    fn auth_states_ref(&self) -> &McpAuthStates {
        &self.mcp_auth_states
    }

    #[inline]
    fn oauth_provider_ref(&self) -> &Arc<dyn McpOAuthProvider + Send + Sync> {
        &self.mcp_oauth_provider
    }

    #[inline]
    fn tools_ref(&self) -> &std::sync::Mutex<Vec<ToolDefinition>> {
        &self.mcp_tools
    }

    #[inline]
    fn effective_servers_ref(&self) -> &std::sync::RwLock<Vec<McpServerConfigEntry>> {
        &self.effective_mcp_servers
    }
}
