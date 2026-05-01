//! MockKernelBuilder — Builds a minimal `LibreFangKernel` for testing.
//!
//! Uses in-memory SQLite and a temp directory, skipping heavy initialization
//! like networking/OFP/cron. Internally uses `LibreFangKernel::boot_with_config`
//! to construct a real kernel instance.

use librefang_kernel::LibreFangKernel;
use librefang_types::config::KernelConfig;
use std::sync::Once;
use tempfile::TempDir;

/// Pin a deterministic vault master key for the test process the first
/// time a mock kernel is built. Without this, parallel integration tests
/// race on the process-shared `<data_local_dir>/librefang/.keyring` file
/// (or OS keyring entry): one test's `init()` overwrites another's master
/// key, and the loser's later `vault_get`/`vault_set` calls open a fresh
/// `CredentialVault` whose `resolve_master_key` then loads the wrong key
/// and fails to decrypt its own vault file (TOTP test flake on CI).
///
/// 32 zero bytes, base64-encoded — value is irrelevant, only stability is.
static VAULT_KEY_INIT: Once = Once::new();
const TEST_VAULT_KEY_B64: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

fn ensure_test_vault_key() {
    VAULT_KEY_INIT.call_once(|| {
        if std::env::var_os("LIBREFANG_VAULT_KEY").is_none() {
            // SAFETY: only runs once, before any kernel is booted in this
            // process — no other thread can be reading the env at this point
            // because all paths into the vault go through MockKernelBuilder
            // (or `LibreFangKernel::boot_with_config`, which the builder
            // owns the entry to).
            std::env::set_var("LIBREFANG_VAULT_KEY", TEST_VAULT_KEY_B64);
        }
    });
}

/// Test kernel builder.
///
/// Configures the kernel via the builder pattern, then call `.build()` to produce
/// a real `LibreFangKernel` instance (using a temp directory and in-memory database).
///
/// # Example
///
/// ```rust,ignore
/// // ignore: requires full kernel boot environment (temp directory, SQLite), see integration tests in tests.rs
/// use librefang_testing::MockKernelBuilder;
///
/// let (kernel, _tmp) = MockKernelBuilder::new().build();
/// assert!(kernel.registry.list().is_empty());
/// ```
type ConfigFn = Box<dyn FnOnce(&mut KernelConfig)>;

pub struct MockKernelBuilder {
    config: KernelConfig,
    /// Custom config modification function.
    config_fn: Option<ConfigFn>,
}

impl MockKernelBuilder {
    /// Creates a builder with the default minimal configuration.
    pub fn new() -> Self {
        Self {
            config: KernelConfig::default(),
            config_fn: None,
        }
    }

    /// Sets a custom config modification function.
    ///
    /// ```rust,ignore
    /// // ignore: requires full kernel boot environment (temp directory, SQLite), see integration tests in tests.rs
    /// use librefang_testing::MockKernelBuilder;
    ///
    /// let (kernel, _tmp) = MockKernelBuilder::new()
    ///     .with_config(|cfg| {
    ///         cfg.default_model.provider = "test".into();
    ///     })
    ///     .build();
    /// ```
    pub fn with_config<F: FnOnce(&mut KernelConfig) + 'static>(mut self, f: F) -> Self {
        self.config_fn = Some(Box::new(f));
        self
    }

    /// Builds the kernel instance.
    ///
    /// Returns `(LibreFangKernel, TempDir)` — the caller must hold onto `TempDir`,
    /// otherwise the temp directory will be deleted on drop, invalidating kernel file paths.
    pub fn build(mut self) -> (LibreFangKernel, TempDir) {
        ensure_test_vault_key();
        let tmp = tempfile::tempdir().expect("failed to create temp directory");
        let home_dir = tmp.path().to_path_buf();
        let data_dir = home_dir.join("data");

        // Ensure required directories exist
        std::fs::create_dir_all(&data_dir).expect("failed to create data directory");
        std::fs::create_dir_all(home_dir.join("skills"))
            .expect("failed to create skills directory");
        std::fs::create_dir_all(home_dir.join("workspaces").join("agents"))
            .expect("failed to create agent workspaces directory");
        std::fs::create_dir_all(home_dir.join("workspaces").join("hands"))
            .expect("failed to create hand workspaces directory");

        // Configure minimal kernel
        self.config.home_dir = home_dir;
        self.config.data_dir = data_dir;
        self.config.network_enabled = false;
        // Use in-memory SQLite (setting path to :memory: doesn't work; boot_with_config uses file paths)
        // So we use a file path under the temp directory instead
        self.config.memory.sqlite_path = Some(self.config.data_dir.join("test.db"));

        // Apply custom configuration
        if let Some(f) = self.config_fn.take() {
            f(&mut self.config);
        }

        let kernel =
            LibreFangKernel::boot_with_config(self.config).expect("failed to boot test kernel");

        (kernel, tmp)
    }
}

impl Default for MockKernelBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Quickly builds a default test kernel (convenience function).
///
/// Equivalent to `MockKernelBuilder::new().build()`.
pub fn test_kernel() -> (LibreFangKernel, TempDir) {
    MockKernelBuilder::new().build()
}
