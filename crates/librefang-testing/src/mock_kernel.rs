//! MockKernelBuilder — Builds a minimal `LibreFangKernel` for testing.
//!
//! Uses in-memory SQLite and a temp directory, skipping heavy initialization
//! like networking/OFP/cron. Internally uses `LibreFangKernel::boot_with_config`
//! to construct a real kernel instance.

use librefang_kernel::LibreFangKernel;
use librefang_types::config::KernelConfig;
use tempfile::TempDir;

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
        let tmp = tempfile::tempdir().expect("failed to create temp directory");
        let home_dir = tmp.path().to_path_buf();
        let data_dir = home_dir.join("data");

        // Ensure required directories exist
        std::fs::create_dir_all(&data_dir).expect("failed to create data directory");
        std::fs::create_dir_all(home_dir.join("skills"))
            .expect("failed to create skills directory");
        std::fs::create_dir_all(home_dir.join("workspaces").join("agents"))
            .expect("failed to create workspaces directory");

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
