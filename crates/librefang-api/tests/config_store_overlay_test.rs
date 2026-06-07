//! Phase 9 (C-004): the database config-store overlay replaces the kernel's
//! effective MCP server list at boot.
#![cfg(feature = "surreal-backend")]

use librefang_api::config_store_overlay::overlay_mcp_servers;
use librefang_kernel::LibreFangKernel;
use librefang_storage::migrations::{apply_pending, OPERATIONAL_MIGRATIONS};
use librefang_storage::{
    content_hash, shared_pool, ConfigSource, ConfigStore, StorageConfig, SurrealConfigStore,
};
use librefang_types::config::{DefaultModelConfig, KernelConfig};

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
