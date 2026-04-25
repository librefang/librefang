#![cfg(feature = "surreal-backend")]

use librefang_memory::open_shared_memory_storage;
use librefang_storage::config::{StorageBackendKind, StorageConfig, DEFAULT_MEMORY_DATABASE_NAME};

#[test]
fn derived_memory_store_uses_dedicated_database_and_lock_path() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cfg = StorageConfig::embedded_default(tmp.path());
    let memory_cfg = cfg.memory_storage_config();

    let StorageBackendKind::Embedded {
        path: ref operational_path,
    } = cfg.backend
    else {
        panic!("expected embedded operational storage");
    };
    let StorageBackendKind::Embedded {
        path: ref memory_path,
    } = memory_cfg.backend
    else {
        panic!("expected embedded memory storage");
    };

    assert_ne!(operational_path, memory_path);
    assert_eq!(memory_path.file_name().unwrap(), "librefang-memory.surreal");
    assert_eq!(
        memory_cfg.effective_database(),
        DEFAULT_MEMORY_DATABASE_NAME
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "opens embedded SurrealDB RocksDB storage; run in the feature-gated Surreal/vector CI lane"]
async fn shared_memory_storage_opens_once_and_can_be_shared_by_backends() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cfg = StorageConfig::embedded_default(tmp.path());

    let shared = open_shared_memory_storage(&cfg)
        .await
        .expect("shared memory storage should open");
    let clone = shared.clone();

    assert!(
        std::sync::Arc::ptr_eq(&shared, &clone),
        "kernel should share a single SurrealStorage Arc across memory backends"
    );
}
