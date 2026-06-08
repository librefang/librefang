//! Phase 9: the database config-store overlay (C-004) replaces the kernel's
//! effective MCP server list at boot, and runtime writes (C-005) persist to the
//! store and survive a restart.
#![cfg(feature = "surreal-backend")]

use librefang_api::config_store_overlay::{
    overlay_mcp_servers, write_mcp_servers, MCP_SERVERS_KEY,
};
use librefang_kernel::LibreFangKernel;
use librefang_storage::migrations::{apply_pending, OPERATIONAL_MIGRATIONS};
use librefang_storage::{
    content_hash, shared_pool, ConfigSource, ConfigStore, StorageConfig, SurrealConfigStore,
};
use librefang_types::config::{DefaultModelConfig, KernelConfig};

/// Build a minimal KernelConfig pointing at an isolated embedded store.
fn test_config(tmp: &std::path::Path, storage: StorageConfig) -> KernelConfig {
    KernelConfig {
        home_dir: tmp.to_path_buf(),
        data_dir: tmp.join("data"),
        storage,
        mcp_servers: Vec::new(),
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::BTreeMap::new(),
            cli_profile_dirs: Vec::new(),
        },
        ..KernelConfig::default()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn overlay_replaces_effective_mcp_servers_from_db() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = StorageConfig::embedded_default(tmp.path().join("operational"));

    // 1. Seed the config store with an mcp_servers entry via the SHARED pool —
    //    the same pool the overlay uses, so the embedded RocksDB transport is
    //    reused (a fresh pool would deadlock on the per-path lock).
    {
        let session = shared_pool().open(&storage).await.expect("open session");
        apply_pending(session.client(), OPERATIONAL_MIGRATIONS)
            .await
            .expect("migrations");
        let store = SurrealConfigStore::open(&session)
            .await
            .expect("open store");
        let servers = serde_json::json!([{ "name": "db-server", "transport": null }]);
        store
            .upsert(
                "mcp_servers",
                servers.clone(),
                ConfigSource::Runtime,
                &content_hash(&servers),
                0,
            )
            .await
            .expect("upsert");
    }

    // 2. Boot a kernel pointing at the same storage, with NO bootstrap MCP
    //    servers — so any post-overlay entry can only have come from the DB.
    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        storage: storage.clone(),
        mcp_servers: Vec::new(),
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::BTreeMap::new(),
            cli_profile_dirs: Vec::new(),
        },
        ..KernelConfig::default()
    };
    let kernel = LibreFangKernel::boot_with_config(config).expect("kernel boots");

    assert!(
        kernel.effective_mcp_servers().is_empty(),
        "bootstrap list must start empty"
    );

    // 3. Overlay reads the DB and replaces the effective list.
    overlay_mcp_servers(&kernel).await;

    let effective = kernel.effective_mcp_servers();
    assert_eq!(effective.len(), 1, "DB entry must be overlaid");
    assert_eq!(effective[0].name, "db-server");
}

#[tokio::test(flavor = "multi_thread")]
async fn overlay_is_noop_when_store_has_no_mcp_servers() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = StorageConfig::embedded_default(tmp.path().join("operational"));

    // Bootstrap list with one server; store has no mcp_servers entry → keep it.
    let bootstrap = serde_json::from_value::<librefang_types::config::McpServerConfigEntry>(
        serde_json::json!({ "name": "boot-server", "transport": null }),
    )
    .unwrap();
    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        storage: storage.clone(),
        mcp_servers: vec![bootstrap],
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::BTreeMap::new(),
            cli_profile_dirs: Vec::new(),
        },
        ..KernelConfig::default()
    };
    let kernel = LibreFangKernel::boot_with_config(config).expect("kernel boots");

    overlay_mcp_servers(&kernel).await;

    let effective = kernel.effective_mcp_servers();
    assert_eq!(effective.len(), 1, "bootstrap list must be preserved");
    assert_eq!(effective[0].name, "boot-server");
}

// ── C-005: runtime writes persist to the store and survive a restart ────────

/// A runtime MCP write (the path POST/PUT/DELETE /api/mcp/servers takes) lands
/// in the config store with `source = runtime`, syncs BOTH kernel views
/// (`config.mcp_servers` + the effective list), and is re-applied on the next
/// boot's overlay — i.e. it survives a restart without touching config.toml.
#[tokio::test(flavor = "multi_thread")]
async fn runtime_write_persists_and_survives_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = StorageConfig::embedded_default(tmp.path().join("operational"));

    // Boot a kernel with NO bootstrap servers.
    let kernel =
        LibreFangKernel::boot_with_config(test_config(tmp.path(), storage.clone())).expect("boots");
    assert!(kernel.effective_mcp_servers().is_empty());

    // Simulate the handler write path: persist the new full list to the store
    // (source = runtime), then sync it into the kernel.
    let entry: librefang_types::config::McpServerConfigEntry =
        serde_json::from_value(serde_json::json!({ "name": "ui-server", "transport": null }))
            .unwrap();
    let servers = vec![entry];
    write_mcp_servers(&storage, &servers, ConfigSource::Runtime)
        .await
        .expect("write");
    kernel.replace_mcp_servers(servers);

    // Both kernel views reflect the write (so dup-checks / GET list stay correct).
    assert_eq!(kernel.effective_mcp_servers().len(), 1);
    assert_eq!(kernel.config_ref().mcp_servers.len(), 1);
    assert_eq!(kernel.config_ref().mcp_servers[0].name, "ui-server");

    // The store row is tagged runtime (so a later bootstrap re-sync won't clobber it).
    {
        let session = shared_pool().open(&storage).await.expect("open");
        let store = SurrealConfigStore::open(&session).await.expect("store");
        let row = store
            .get(MCP_SERVERS_KEY)
            .await
            .expect("read")
            .expect("present");
        assert_eq!(row.source, ConfigSource::Runtime);
    }

    // Restart: a fresh kernel at the same storage boots empty, then the overlay
    // re-applies the persisted write — no config.toml involved.
    drop(kernel);
    let kernel2 =
        LibreFangKernel::boot_with_config(test_config(tmp.path(), storage.clone())).expect("boots");
    assert!(
        kernel2.effective_mcp_servers().is_empty(),
        "fresh boot is empty"
    );
    overlay_mcp_servers(&kernel2).await;
    let after = kernel2.effective_mcp_servers();
    assert_eq!(after.len(), 1, "runtime write survives restart");
    assert_eq!(after[0].name, "ui-server");
}

// ── C-003: seed-once + provenance/content-hash merge ────────────────────────

use librefang_api::config_store_overlay::{seed_mcp_servers, SeedOutcome};

fn entry(name: &str) -> librefang_types::config::McpServerConfigEntry {
    serde_json::from_value(serde_json::json!({ "name": name, "transport": null })).unwrap()
}

async fn stored_servers(storage: &StorageConfig) -> (Vec<String>, ConfigSource, i64) {
    let session = shared_pool().open(storage).await.expect("open");
    let store = SurrealConfigStore::open(&session).await.expect("store");
    let row = store
        .get("mcp_servers")
        .await
        .expect("read")
        .expect("present");
    let names =
        serde_json::from_value::<Vec<librefang_types::config::McpServerConfigEntry>>(row.value)
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
    (names, row.source, row.revision)
}

#[tokio::test(flavor = "multi_thread")]
async fn seed_writes_bootstrap_on_fresh_store() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = StorageConfig::embedded_default(tmp.path().join("operational"));

    let outcome = seed_mcp_servers(&storage, &[entry("a")], 0).await.unwrap();
    assert_eq!(outcome, SeedOutcome::Seeded);

    let (names, source, rev) = stored_servers(&storage).await;
    assert_eq!(names, vec!["a"]);
    assert_eq!(source, ConfigSource::Bootstrap);
    assert_eq!(rev, 0);

    // Re-seeding identical bootstrap is a no-op.
    let again = seed_mcp_servers(&storage, &[entry("a")], 0).await.unwrap();
    assert_eq!(again, SeedOutcome::Unchanged);
}

#[tokio::test(flavor = "multi_thread")]
async fn seed_updates_changed_bootstrap_row() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = StorageConfig::embedded_default(tmp.path().join("operational"));

    seed_mcp_servers(&storage, &[entry("a")], 0).await.unwrap();
    // config.toml changed (bootstrap row), no UI edit in between → update.
    let outcome = seed_mcp_servers(&storage, &[entry("a"), entry("b")], 0)
        .await
        .unwrap();
    assert_eq!(outcome, SeedOutcome::BootstrapUpdated);
    let (names, source, _) = stored_servers(&storage).await;
    assert_eq!(names, vec!["a", "b"]);
    assert_eq!(source, ConfigSource::Bootstrap);
}

#[tokio::test(flavor = "multi_thread")]
async fn seed_never_clobbers_runtime_row_without_revision_bump() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = StorageConfig::embedded_default(tmp.path().join("operational"));

    // A UI write lands first (source = runtime).
    write_mcp_servers(&storage, &[entry("ui")], ConfigSource::Runtime)
        .await
        .unwrap();

    // Operator edits config.toml but does NOT bump the revision → UI wins.
    let outcome = seed_mcp_servers(&storage, &[entry("boot")], 0)
        .await
        .unwrap();
    assert_eq!(outcome, SeedOutcome::RuntimeProtected);
    let (names, source, _) = stored_servers(&storage).await;
    assert_eq!(names, vec!["ui"], "UI edit must be preserved");
    assert_eq!(source, ConfigSource::Runtime);
}

#[tokio::test(flavor = "multi_thread")]
async fn seed_revision_bump_overrides_runtime_row() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = StorageConfig::embedded_default(tmp.path().join("operational"));

    write_mcp_servers(&storage, &[entry("ui")], ConfigSource::Runtime)
        .await
        .unwrap();

    // Operator bumps the bootstrap revision past the stored row (0) → override.
    let outcome = seed_mcp_servers(&storage, &[entry("boot")], 1)
        .await
        .unwrap();
    assert_eq!(outcome, SeedOutcome::RevisionOverride);
    let (names, source, rev) = stored_servers(&storage).await;
    assert_eq!(names, vec!["boot"], "operator-forced bootstrap wins");
    assert_eq!(source, ConfigSource::Bootstrap);
    assert_eq!(rev, 1);
}

// ── C-006: a config reload re-resolves from the store (no clobber) ───────────

use librefang_api::config_store_overlay::seed_config_store;

/// The config-reload path re-runs seed → overlay, so a reload that re-reads
/// config.toml does NOT revert a DB-resolved `runtime` (UI) value back to the
/// bootstrap file value. We simulate `reload_config`'s effect (it transiently
/// resets the kernel's MCP list to the bootstrap file) and assert the C-006
/// re-resolve restores the runtime value from the store.
#[tokio::test(flavor = "multi_thread")]
async fn reload_reresolve_preserves_runtime_over_bootstrap() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = StorageConfig::embedded_default(tmp.path().join("operational"));

    let kernel =
        LibreFangKernel::boot_with_config(test_config(tmp.path(), storage.clone())).expect("boots");

    // A prior UI edit lives in the store as a runtime value.
    write_mcp_servers(&storage, &[entry("ui-server")], ConfigSource::Runtime)
        .await
        .unwrap();

    // Simulate reload_config re-reading config.toml: it resets the kernel's MCP
    // list to the bootstrap file values (here, a different server).
    kernel.replace_mcp_servers(vec![entry("boot-server")]);
    assert_eq!(kernel.effective_mcp_servers()[0].name, "boot-server");

    // C-006 re-resolve (exactly what config_reload runs after reload_config):
    seed_config_store(&kernel).await;
    overlay_mcp_servers(&kernel).await;

    // The runtime value wins — the reload did not clobber the UI edit.
    let effective = kernel.effective_mcp_servers();
    assert_eq!(effective.len(), 1);
    assert_eq!(
        effective[0].name, "ui-server",
        "reload must not revert a DB runtime value to the config.toml bootstrap"
    );
    // And the store row stayed runtime (seed left it protected).
    let (_, source, _) = stored_servers(&storage).await;
    assert_eq!(source, ConfigSource::Runtime);
}
