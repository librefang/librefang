//! Kernel boot tests for `vector_backend` configuration.
//!
//! These tests verify that the kernel selects the correct semantic backend
//! based on the `vector_backend` configuration field and the compiled feature
//! flags.
//!
//! Integration tests that require a live SurrealDB instance are marked
//! `#[ignore]` and must be run explicitly:
//!
//! ```text
//! cargo test --features surreal-backend \
//!   -p librefang-kernel \
//!   --test vector_backend_boot_test \
//!   -- --ignored
//! ```

use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig, MemoryConfig};

fn base_config() -> KernelConfig {
    let tmp = std::env::temp_dir().join(format!("librefang-vb-test-{}", uuid::Uuid::new_v4()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    KernelConfig {
        home_dir: tmp.clone(),
        data_dir: tmp.join("data"),
        default_model: DefaultModelConfig {
            provider: "openai".to_string(),
            model: "gpt-4o-mini".to_string(),
            api_key_env: "OPENAI_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 30,
            extra_params: std::collections::HashMap::new(),
            cli_profile_dirs: Vec::new(),
        },
        ..KernelConfig::default()
    }
}

/// Verify that booting with `vector_backend = "surreal"` without the
/// `surreal-backend` feature returns a descriptive `KernelError::BootFailed`.
///
/// This test is only meaningful when compiled WITHOUT `surreal-backend`.
#[cfg(not(feature = "surreal-backend"))]
#[test]
fn vector_backend_surreal_requires_feature() {
    let mut cfg = base_config();
    cfg.memory = MemoryConfig {
        vector_backend: Some("surreal".to_string()),
        ..Default::default()
    };

    let result = LibreFangKernel::boot_with_config(cfg);
    assert!(
        result.is_err(),
        "Expected boot to fail when surreal-backend feature is absent"
    );
    let err_str = format!("{:?}", result.unwrap_err());
    assert!(
        err_str.contains("surreal-backend") || err_str.contains("feature"),
        "Error message should mention the missing feature: {err_str}"
    );
}

/// Verify that booting with `vector_backend = "surreal"` selects the
/// `SurrealSemanticBackend` and reports `backend_name() == "surreal"`.
///
/// Requires a running SurrealDB instance on `ws://127.0.0.1:8000`.
#[cfg(feature = "surreal-backend")]
#[test]
#[ignore = "requires a running SurrealDB instance on ws://127.0.0.1:8000"]
fn vector_backend_surreal_boot_selects_surreal_backend() {
    let mut cfg = base_config();
    cfg.memory = MemoryConfig {
        vector_backend: Some("surreal".to_string()),
        ..Default::default()
    };

    let kernel = LibreFangKernel::boot_with_config(cfg)
        .expect("Kernel should boot successfully with vector_backend = \"surreal\"");

    assert_eq!(
        kernel.semantic_backend_name(),
        "surreal",
        "SemanticBackend should report 'surreal' when surreal-backend feature is enabled"
    );
}

/// Verify that booting with `vector_backend` unset (the default "auto" path)
/// selects the `SurrealSemanticBackend` when `surreal-backend` is compiled in.
///
/// Requires a running SurrealDB instance on `ws://127.0.0.1:8000`.
#[cfg(feature = "surreal-backend")]
#[test]
#[ignore = "requires a running SurrealDB instance on ws://127.0.0.1:8000"]
fn vector_backend_auto_selects_surreal_when_feature_enabled() {
    let cfg = base_config(); // vector_backend = None → "auto"

    let kernel = LibreFangKernel::boot_with_config(cfg)
        .expect("Kernel should boot with auto/unset vector_backend");

    assert_eq!(
        kernel.semantic_backend_name(),
        "surreal",
        "auto mode with surreal-backend feature should select the SurrealDB backend"
    );
}

/// Verify that booting with `vector_backend = "sqlite"` keeps the SQLite
/// semantic backend even when `surreal-backend` is compiled in.
#[cfg(feature = "surreal-backend")]
#[test]
#[ignore = "requires a running SurrealDB instance on ws://127.0.0.1:8000"]
fn vector_backend_explicit_sqlite_bypasses_surreal() {
    let mut cfg = base_config();
    cfg.memory = MemoryConfig {
        vector_backend: Some("sqlite".to_string()),
        ..Default::default()
    };

    let kernel = LibreFangKernel::boot_with_config(cfg)
        .expect("Kernel should boot with vector_backend = \"sqlite\"");

    assert_eq!(
        kernel.semantic_backend_name(),
        "sqlite",
        "Explicit sqlite selection should bypass the surreal-backend even when feature is on"
    );
}
