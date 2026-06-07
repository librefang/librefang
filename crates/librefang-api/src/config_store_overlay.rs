//! Database config-store overlay at daemon boot (phase 9 / C-004).
//!
//! The kernel holds no operational SurrealDB session — every operational-store
//! access in the daemon is opened on-demand by the API / CLI layers. So the API
//! layer owns config-store access and pushes the resolved configuration into the
//! kernel, rather than the kernel reaching into SurrealDB itself.
//!
//! [`overlay_mcp_servers`] runs once, between `set_self_handle()` and
//! `start_background_agents()` in [`crate::server::run_daemon`], so the kernel's
//! `effective_mcp_servers` already reflects the database before the MCP-connect
//! background task spawns. It opens an on-demand session, reads, and drops it —
//! never holding the embedded RocksDB lock, matching the existing on-demand
//! pattern (assessment R-2).
//!
//! This is the read half. The seed-once + provenance merge that populates the
//! store from `config.toml` bootstrap defaults lands in C-003; the write half
//! (UI / API edits) lands in C-005.

use librefang_kernel::LibreFangKernel;
use librefang_storage::{shared_pool, ConfigStore, SurrealConfigStore};
use librefang_types::config::McpServerConfigEntry;

/// Config-store key under which the MCP server list is stored.
pub const MCP_SERVERS_KEY: &str = "mcp_servers";

/// Overlay the database config store's MCP server list onto the kernel's
/// effective list at boot. Best-effort: any failure (SurrealDB unreachable,
/// missing entry, malformed value) logs and leaves the bootstrap list intact —
/// it must never block daemon startup.
pub async fn overlay_mcp_servers(kernel: &LibreFangKernel) {
    let storage_cfg = kernel.config_ref().storage.clone();

    // Shared process-global pool — embedded RocksDB holds one lock per path per
    // process, so the overlay, the storage routes, and config writes must all
    // reuse the same cached transport (see librefang_storage::shared_pool).
    let pool = shared_pool();
    let session = match pool.open(&storage_cfg).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "config-store overlay: could not open storage session; \
                 keeping bootstrap MCP servers"
            );
            return;
        }
    };

    // Ensure the config_store table exists. apply_pending is idempotent and
    // checksum-guarded; this is the first daemon-side consumer of the
    // operational store, so it carries the responsibility of bringing the
    // schema up before reading.
    if let Err(e) = librefang_storage::migrations::apply_pending(
        session.client(),
        librefang_storage::migrations::OPERATIONAL_MIGRATIONS,
    )
    .await
    {
        tracing::warn!(
            error = %e,
            "config-store overlay: migration apply failed; keeping bootstrap MCP servers"
        );
        return;
    }

    let store = match SurrealConfigStore::open(&session).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "config-store overlay: open store failed");
            return;
        }
    };

    match store.get(MCP_SERVERS_KEY).await {
        Ok(Some(entry)) => match serde_json::from_value::<Vec<McpServerConfigEntry>>(entry.value) {
            Ok(servers) => {
                let count = servers.len();
                kernel.replace_effective_mcp_servers(servers);
                tracing::info!(
                    count,
                    source = entry.source.as_str(),
                    "config-store overlay: applied MCP servers from database"
                );
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "config-store overlay: '{MCP_SERVERS_KEY}' entry is malformed; \
                     keeping bootstrap MCP servers"
                );
            }
        },
        Ok(None) => {
            tracing::debug!(
                "config-store overlay: no '{MCP_SERVERS_KEY}' entry; keeping bootstrap"
            );
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "config-store overlay: read failed; keeping bootstrap MCP servers"
            );
        }
    }
    // `session` / `pool` drop here → embedded RocksDB lock released.
}
