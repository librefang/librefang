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
use librefang_types::config::{DefaultModelConfig, McpServerConfigEntry};

/// Config-store key under which the MCP server list is stored.
pub const MCP_SERVERS_KEY: &str = "mcp_servers";

/// Config-store key under which the default-model selection is stored (C-005b).
pub const DEFAULT_MODEL_KEY: &str = "default_model";

/// Open the config store via the shared pool, ensuring the schema is applied.
/// Self-contained so callers work whether or not the boot overlay has run.
async fn open_config_store(storage_cfg: &StorageConfig) -> Result<SurrealConfigStore, String> {
    let session = shared_pool()
        .open(storage_cfg)
        .await
        .map_err(|e| format!("open storage session: {e}"))?;
    apply_pending(session.client(), OPERATIONAL_MIGRATIONS)
        .await
        .map_err(|e| format!("apply migrations: {e}"))?;
    SurrealConfigStore::open(&session)
        .await
        .map_err(|e| format!("open config store: {e}"))
}

/// Generic boot-seed/merge for one config-store key (shared by mcp_servers and
/// default_model). Writes are always stamped [`ConfigSource::Bootstrap`]; the
/// merge DECISION keys off the EXISTING row's source, so a `runtime` row is
/// never clobbered (`RuntimeProtected`) unless the operator bumps the bootstrap
/// revision. The one-time prod import takes a different path
/// ([`promote_or_seed`]) that writes `runtime`.
async fn seed_value(
    store: &SurrealConfigStore,
    key: &str,
    value: serde_json::Value,
    bootstrap_revision: i64,
) -> Result<SeedOutcome, String> {
    let hash = content_hash(&value);
    let existing = store
        .get(key)
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
                key,
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

/// Outcome of a one-time prod import ([`import_mcp_servers`] /
/// [`import_default_model`]). Returned for logging and tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportOutcome {
    /// No store row existed; the config.toml value was written as `runtime`.
    Seeded,
    /// A `bootstrap` row existed (e.g. written by the daemon's boot-seed); its
    /// value was **preserved** and re-stamped `runtime` so the later ConfigMap
    /// revert cannot overwrite it.
    Promoted,
    /// The row was already `runtime` (a UI edit); left untouched.
    AlreadyRuntime,
}

/// Ensure the in-scope `key` ends up `source = runtime`, **preserving the
/// store's current value** (the post-deploy source of truth), falling back to
/// the supplied `config_value` only when no row exists. This is the load-bearing
/// cutover-safety operation (C-009): every in-scope row must be `runtime` before
/// the K8s ConfigMap revert, or the next boot-seed would overwrite a `bootstrap`
/// row with the empty ConfigMap baseline (`BootstrapUpdated`) and wipe prod
/// config. Idempotent and value-preserving — it never changes a stored value.
async fn promote_or_seed(
    store: &SurrealConfigStore,
    key: &str,
    config_value: serde_json::Value,
) -> Result<ImportOutcome, String> {
    let existing = store
        .get(key)
        .await
        .map_err(|e| format!("read config store: {e}"))?;
    let (value, outcome) = match existing {
        Some(row) if row.source == ConfigSource::Runtime => {
            return Ok(ImportOutcome::AlreadyRuntime)
        }
        // Preserve the store's current value (it is the post-deploy truth, and
        // already equals config.toml for a boot-seeded row); only the provenance
        // changes to `runtime`.
        Some(row) => (row.value, ImportOutcome::Promoted),
        None => (config_value, ImportOutcome::Seeded),
    };
    let hash = content_hash(&value);
    store
        .upsert(key, value, ConfigSource::Runtime, &hash, 0)
        .await
        .map_err(|e| format!("write config store: {e}"))?;
    Ok(outcome)
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
    let store = open_config_store(storage_cfg).await?;
    let value = serde_json::to_value(bootstrap).map_err(|e| e.to_string())?;
    seed_value(&store, MCP_SERVERS_KEY, value, bootstrap_revision).await
}

/// Seed / merge the bootstrap default-model selection into the config store
/// (C-005b). Same provenance/content-hash semantics as [`seed_mcp_servers`].
///
/// # Errors
/// Returns a scrubbed message on any storage failure.
pub async fn seed_default_model(
    storage_cfg: &StorageConfig,
    bootstrap: &DefaultModelConfig,
    bootstrap_revision: i64,
) -> Result<SeedOutcome, String> {
    let store = open_config_store(storage_cfg).await?;
    let value = serde_json::to_value(bootstrap).map_err(|e| e.to_string())?;
    seed_value(&store, DEFAULT_MODEL_KEY, value, bootstrap_revision).await
}

/// One-time prod **import** of the MCP server list (C-008 / C-009 / C-009b).
/// Ensures the `mcp_servers` row is `source = runtime`, preserving the store's
/// current value (or seeding config.toml's when absent) — see [`promote_or_seed`].
/// This protects prod's MCP servers across the K8s ConfigMap revert regardless
/// of whether the daemon already boot-seeded them as `bootstrap`.
///
/// # Errors
/// Returns a scrubbed message on any storage failure.
pub async fn import_mcp_servers(
    storage_cfg: &StorageConfig,
    servers: &[McpServerConfigEntry],
) -> Result<ImportOutcome, String> {
    let store = open_config_store(storage_cfg).await?;
    let value = serde_json::to_value(servers).map_err(|e| e.to_string())?;
    promote_or_seed(&store, MCP_SERVERS_KEY, value).await
}

/// One-time prod **import** of the default-model selection (C-008 / C-009 /
/// C-009b). Same value-preserving `runtime` promotion as [`import_mcp_servers`].
///
/// # Errors
/// Returns a scrubbed message on any storage failure.
pub async fn import_default_model(
    storage_cfg: &StorageConfig,
    default_model: &DefaultModelConfig,
) -> Result<ImportOutcome, String> {
    let store = open_config_store(storage_cfg).await?;
    let value = serde_json::to_value(default_model).map_err(|e| e.to_string())?;
    promote_or_seed(&store, DEFAULT_MODEL_KEY, value).await
}

/// Seed the config store from the kernel's bootstrap config at daemon boot.
/// Best-effort: failures log and leave the store untouched (never block boot).
/// Runs BEFORE the overlays so they read a populated store.
pub async fn seed_config_store(kernel: &dyn KernelApi) {
    let storage_cfg = kernel.config_ref().storage.clone();
    let revision = bootstrap_revision();

    let mcp = kernel.config_ref().mcp_servers.clone();
    match seed_mcp_servers(&storage_cfg, &mcp, revision).await {
        Ok(outcome) => tracing::info!(
            ?outcome,
            bootstrap_revision = revision,
            count = mcp.len(),
            "config-store seed: bootstrap MCP servers reconciled"
        ),
        Err(e) => tracing::warn!(error = %e, "config-store seed (mcp_servers) failed"),
    }

    let dm = kernel.config_ref().default_model.clone();
    match seed_default_model(&storage_cfg, &dm, revision).await {
        Ok(outcome) => tracing::info!(
            ?outcome,
            bootstrap_revision = revision,
            "config-store seed: bootstrap default_model reconciled"
        ),
        Err(e) => tracing::warn!(error = %e, "config-store seed (default_model) failed"),
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
    let store = open_config_store(storage_cfg).await?;
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

/// Persist the default-model selection to the config store under
/// [`DEFAULT_MODEL_KEY`] (C-005b). UI/API writes use [`ConfigSource::Runtime`].
///
/// # Errors
/// Returns a scrubbed message on any storage failure.
pub async fn write_default_model(
    storage_cfg: &StorageConfig,
    default_model: &DefaultModelConfig,
    source: ConfigSource,
) -> Result<(), String> {
    let store = open_config_store(storage_cfg).await?;
    let value = serde_json::to_value(default_model).map_err(|e| e.to_string())?;
    let hash = content_hash(&value);
    store
        .upsert(DEFAULT_MODEL_KEY, value, source, &hash, 0)
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

/// Overlay the database config store's default-model selection onto the
/// kernel's runtime override (C-005b). Best-effort: any failure logs and leaves
/// the bootstrap `default_model` intact — it must never block daemon startup.
pub async fn overlay_default_model(kernel: &dyn KernelApi) {
    let storage_cfg = kernel.config_ref().storage.clone();
    let store = match open_config_store(&storage_cfg).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "config-store overlay (default_model): open failed");
            return;
        }
    };
    match store.get(DEFAULT_MODEL_KEY).await {
        Ok(Some(entry)) => match serde_json::from_value::<DefaultModelConfig>(entry.value) {
            Ok(dm) => {
                let provider = dm.provider.clone();
                if let Ok(mut guard) = kernel.default_model_override_ref().write() {
                    *guard = Some(dm);
                }
                tracing::info!(
                    %provider,
                    source = entry.source.as_str(),
                    "config-store overlay: applied default_model from database"
                );
            }
            Err(e) => tracing::warn!(
                error = %e,
                "config-store overlay: '{DEFAULT_MODEL_KEY}' entry malformed; keeping bootstrap"
            ),
        },
        Ok(None) => {
            tracing::debug!(
                "config-store overlay: no '{DEFAULT_MODEL_KEY}' entry; keeping bootstrap"
            )
        }
        Err(e) => {
            tracing::warn!(error = %e, "config-store overlay (default_model): read failed")
        }
    }
}
