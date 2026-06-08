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
//! Boot order in [`crate::server::run_daemon`] is **seed → overlay → connect**:
//! [`seed_config_store`] (C-003) populates the store from `config.toml`
//! bootstrap defaults, [`overlay_mcp_servers`] (C-004) reads the store back into
//! the kernel, then the MCP-connect task runs. The write half (UI / API edits)
//! is C-005 ([`write_mcp_servers`]).

use librefang_kernel::KernelApi;
use librefang_storage::migrations::{apply_pending, OPERATIONAL_MIGRATIONS};
use librefang_storage::{
    content_hash, shared_pool, ConfigSource, ConfigStore, StorageConfig, SurrealConfigStore,
};
use librefang_types::config::McpServerConfigEntry;

/// Config-store key under which the MCP server list is stored.
pub const MCP_SERVERS_KEY: &str = "mcp_servers";

/// Env var holding the operator-controlled bootstrap revision. Bumping it is the
/// ONLY way a `config.toml` change can override a `runtime` (UI-written) store
/// row — a content change alone never clobbers UI edits (assessment FLAW 2).
pub const BOOTSTRAP_REVISION_ENV: &str = "BOSSFANG_CONFIG_BOOTSTRAP_REVISION";

/// Outcome of a [`seed_mcp_servers`] call. Returned for logging and tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedOutcome {
    /// No row existed; the bootstrap list was seeded.
    Seeded,
    /// A `bootstrap` row existed and its content changed; it was updated.
    BootstrapUpdated,
    /// A `runtime` row existed but the bootstrap revision was bumped past it;
    /// the operator-forced bootstrap value overrode the UI edit.
    RevisionOverride,
    /// A `runtime` row existed and the bootstrap revision did NOT advance; the
    /// UI edit was left intact (the common steady-state after a UI change).
    RuntimeProtected,
    /// The stored content already matches the bootstrap value; nothing written.
    Unchanged,
}

impl SeedOutcome {
    fn wrote(self) -> bool {
        matches!(
            self,
            Self::Seeded | Self::BootstrapUpdated | Self::RevisionOverride
        )
    }
}

/// Read the operator-controlled bootstrap revision from the environment
/// ([`BOOTSTRAP_REVISION_ENV`]); defaults to 0 when unset or unparseable.
#[must_use]
pub fn bootstrap_revision() -> i64 {
    std::env::var(BOOTSTRAP_REVISION_ENV)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

/// Seed / merge the bootstrap MCP server list into the config store
/// (phase 9 / C-003).
///
/// Provenance- and content-hash-aware; **never reads a file mtime** (a
/// ConfigMap-projected file's mtime changes on every kubelet sync even when the
/// content is identical — assessment FLAW 1). Per-key, not whole-file
/// (FLAW 2): a `runtime` (UI-written) row is only overridden when the operator
/// explicitly advances `bootstrap_revision` past the stored revision.
///
/// Decision table given the on-disk bootstrap value vs the stored row:
/// - no row                      → seed (`source = bootstrap`)
/// - same content hash           → unchanged (no write)
/// - changed, stored = bootstrap → update (bootstrap owns its own value)
/// - changed, stored = runtime, `revision > stored.revision` → override
/// - changed, stored = runtime, otherwise                    → protected (keep UI)
///
/// # Errors
/// Returns a scrubbed message on any storage failure.
pub async fn seed_mcp_servers(
    storage_cfg: &StorageConfig,
    bootstrap: &[McpServerConfigEntry],
    bootstrap_revision: i64,
) -> Result<SeedOutcome, String> {
    let session = shared_pool()
        .open(storage_cfg)
        .await
        .map_err(|e| format!("open storage session: {e}"))?;
    apply_pending(session.client(), OPERATIONAL_MIGRATIONS)
        .await
        .map_err(|e| format!("apply migrations: {e}"))?;
    let store = SurrealConfigStore::open(&session)
        .await
        .map_err(|e| format!("open config store: {e}"))?;

    let value = serde_json::to_value(bootstrap).map_err(|e| e.to_string())?;
    let hash = content_hash(&value);

    let existing = store
        .get(MCP_SERVERS_KEY)
        .await
        .map_err(|e| format!("read config store: {e}"))?;

    let outcome = match existing {
        None => SeedOutcome::Seeded,
        Some(row) if row.content_hash == hash => SeedOutcome::Unchanged,
        Some(row) => match row.source {
            ConfigSource::Bootstrap => SeedOutcome::BootstrapUpdated,
            ConfigSource::Runtime => {
                if bootstrap_revision > row.revision {
                    SeedOutcome::RevisionOverride
                } else {
                    SeedOutcome::RuntimeProtected
                }
            }
        },
    };

    if outcome.wrote() {
        store
            .upsert(
                MCP_SERVERS_KEY,
                value,
                ConfigSource::Bootstrap,
                &hash,
                bootstrap_revision,
            )
            .await
            .map_err(|e| format!("write config store: {e}"))?;
    }

    Ok(outcome)
}

/// Seed the config store from the kernel's bootstrap config at daemon boot.
/// Best-effort: failures log and leave the store untouched (never block boot).
/// Runs BEFORE [`overlay_mcp_servers`] so the overlay reads a populated store.
pub async fn seed_config_store(kernel: &dyn KernelApi) {
    let storage_cfg = kernel.config_ref().storage.clone();
    let bootstrap = kernel.config_ref().mcp_servers.clone();
    let revision = bootstrap_revision();
    match seed_mcp_servers(&storage_cfg, &bootstrap, revision).await {
        Ok(outcome) => tracing::info!(
            ?outcome,
            bootstrap_revision = revision,
            count = bootstrap.len(),
            "config-store seed: bootstrap MCP servers reconciled"
        ),
        Err(e) => tracing::warn!(
            error = %e,
            "config-store seed failed; leaving store untouched"
        ),
    }
}

/// Persist the full MCP server list to the config store under
/// [`MCP_SERVERS_KEY`], stamping `source` (C-005: UI/API writes use
/// [`ConfigSource::Runtime`]).
///
/// Opens via the shared pool and applies pending migrations first so the call
/// is self-contained — it works whether or not the boot-time overlay has run
/// (e.g. in tests that build the router directly without `run_daemon`).
///
/// # Errors
/// Returns a scrubbed message on any storage failure (open / migrate / write).
pub async fn write_mcp_servers(
    storage_cfg: &StorageConfig,
    servers: &[McpServerConfigEntry],
    source: ConfigSource,
) -> Result<(), String> {
    let session = shared_pool()
        .open(storage_cfg)
        .await
        .map_err(|e| format!("open storage session: {e}"))?;
    apply_pending(session.client(), OPERATIONAL_MIGRATIONS)
        .await
        .map_err(|e| format!("apply migrations: {e}"))?;
    let store = SurrealConfigStore::open(&session)
        .await
        .map_err(|e| format!("open config store: {e}"))?;
    let value = serde_json::to_value(servers).map_err(|e| e.to_string())?;
    let hash = content_hash(&value);
    // revision is only meaningful for bootstrap precedence (C-003); runtime
    // rows are never overwritten by a bootstrap re-sync on a mere hash diff.
    store
        .upsert(MCP_SERVERS_KEY, value, source, &hash, 0)
        .await
        .map_err(|e| format!("write config store: {e}"))?;
    Ok(())
}

/// Overlay the database config store's MCP server list onto the kernel's
/// effective list at boot. Best-effort: any failure (SurrealDB unreachable,
/// missing entry, malformed value) logs and leaves the bootstrap list intact —
/// it must never block daemon startup.
pub async fn overlay_mcp_servers(kernel: &dyn KernelApi) {
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
                kernel.replace_mcp_servers(servers);
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
