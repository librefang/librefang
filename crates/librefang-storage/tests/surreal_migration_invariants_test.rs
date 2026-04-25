#![cfg(feature = "surreal-backend")]

use librefang_storage::config::{
    RemoteSurrealConfig, StorageBackendKind, StorageConfig, DEFAULT_MEMORY_DATABASE_NAME,
};
use librefang_storage::migrations::OPERATIONAL_MIGRATIONS;

#[test]
fn memory_storage_config_separates_embedded_operational_and_memory_paths() {
    let cfg = StorageConfig::embedded_default("/tmp/librefang-data");
    let memory_cfg = cfg.memory_storage_config();

    let StorageBackendKind::Embedded { path: ref op_path } = cfg.backend else {
        panic!("expected embedded operational storage");
    };
    let StorageBackendKind::Embedded {
        path: ref memory_path,
    } = memory_cfg.backend
    else {
        panic!("expected embedded memory storage");
    };

    assert_ne!(
        op_path, memory_path,
        "embedded operational and memory stores must not share a RocksDB lock path"
    );
    assert_eq!(memory_path.file_name().unwrap(), "librefang-memory.surreal");
    assert_eq!(memory_cfg.database, DEFAULT_MEMORY_DATABASE_NAME);
    assert_eq!(
        memory_cfg.effective_database(),
        DEFAULT_MEMORY_DATABASE_NAME
    );
}

#[test]
fn memory_storage_config_separates_remote_database() {
    let cfg = StorageConfig {
        backend: StorageBackendKind::Remote(RemoteSurrealConfig {
            url: "ws://127.0.0.1:8000".into(),
            namespace: "librefang".into(),
            database: "main".into(),
            username: "root".into(),
            password_env: "SURREALDB_PASS".into(),
            tls_skip_verify: false,
        }),
        namespace: "ignored".into(),
        database: "ignored".into(),
        legacy_sqlite_path: None,
    };
    let memory_cfg = cfg.memory_storage_config();

    assert_eq!(cfg.effective_database(), "main");
    assert_eq!(memory_cfg.effective_namespace(), "librefang");
    assert_eq!(
        memory_cfg.effective_database(),
        DEFAULT_MEMORY_DATABASE_NAME
    );
}

#[test]
fn operational_migrations_are_ordered_and_surreal_3_flexible_syntax_safe() {
    let mut last_version = 0;
    for migration in OPERATIONAL_MIGRATIONS {
        assert!(
            migration.version > last_version,
            "migration versions must be strictly increasing"
        );
        last_version = migration.version;

        assert!(
            !migration.sql.contains("FLEXIBLE TYPE"),
            "migration {} uses pre-SurrealDB-3 FLEXIBLE TYPE syntax",
            migration.name
        );
    }
}
