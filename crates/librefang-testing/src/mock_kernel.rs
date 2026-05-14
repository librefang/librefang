//! MockKernelBuilder — Builds a minimal `LibreFangKernel` for testing.
//!
//! Uses in-memory SQLite and a temp directory, skipping heavy initialization
//! like networking/OFP/cron. Internally uses `LibreFangKernel::boot_with_config`
//! to construct a real kernel instance.

use librefang_kernel::LibreFangKernel;
use librefang_runtime::model_catalog::ModelCatalog;
use librefang_types::config::KernelConfig;
use librefang_types::model_catalog::{
    AuthStatus, Modality, ModelCatalogEntry, ModelTier, ProviderInfo,
};
use std::sync::{Arc, Once};
use tempfile::TempDir;

/// Catalog seed pair: `(providers, models)` in the same order
/// `ModelCatalog::from_entries` accepts.
pub type CatalogSeed = (Vec<ProviderInfo>, Vec<ModelCatalogEntry>);

/// A minimal, deterministic catalog covering ids referenced by the
/// `librefang-api` integration test suite (`gpt-4o-mini` under `openai`).
///
/// Use this when a test asserts on a specific model id and you need a
/// stable baseline regardless of network conditions. `MockKernelBuilder`
/// boots through the real `LibreFangKernel::boot_with_config`, which
/// calls `librefang_runtime::registry_sync::sync_registry` — that talks
/// to `github.com/librefang-registry`, and CI runners that flake or
/// rate-limit produce an empty (or partially populated) catalog. Tests
/// referencing specific ids then panic with 404 in one shard while
/// passing in another. Seeding via the builder bypasses the network
/// dependency.
///
/// Add entries here as other tests grow demands — keep the list small
/// and intentional.
pub fn test_catalog_baseline() -> CatalogSeed {
    let providers = vec![ProviderInfo {
        id: "openai".to_string(),
        display_name: "OpenAI".to_string(),
        api_key_env: "OPENAI_API_KEY".to_string(),
        base_url: "https://api.openai.com/v1".to_string(),
        key_required: true,
        auth_status: AuthStatus::default(),
        model_count: 1,
        ..ProviderInfo::default()
    }];
    let models = vec![ModelCatalogEntry {
        id: "gpt-4o-mini".to_string(),
        display_name: "GPT-4o mini (test fixture)".to_string(),
        provider: "openai".to_string(),
        tier: ModelTier::Custom,
        modality: Modality::default(),
        context_window: 128_000,
        max_output_tokens: 16_384,
        input_cost_per_m: 0.15,
        output_cost_per_m: 0.6,
        image_input_cost_per_m: None,
        image_output_cost_per_m: None,
        supports_tools: true,
        supports_vision: true,
        supports_streaming: true,
        supports_thinking: false,
        reasoning_echo_policy: librefang_types::model_catalog::ReasoningEchoPolicy::default(),
        aliases: Vec::new(),
    }];
    (providers, models)
}

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
        // BossFang fork: vault.rs reads BOSSFANG_VAULT_KEY first and falls
        // back to LIBREFANG_VAULT_KEY. Set both so tests pass regardless
        // of which name a future caller checks. If either is already set
        // by the developer's shell, respect that value — tests should
        // not silently overwrite an operator-provided key.
        //
        // SAFETY: only runs once, before any kernel is booted in this
        // process — no other thread can be reading the env at this point
        // because all paths into the vault go through MockKernelBuilder
        // (or `LibreFangKernel::boot_with_config`, which the builder
        // owns the entry to).
        let bossfang_set = std::env::var_os("BOSSFANG_VAULT_KEY").is_some();
        let librefang_set = std::env::var_os("LIBREFANG_VAULT_KEY").is_some();
        if !bossfang_set && !librefang_set {
            std::env::set_var("BOSSFANG_VAULT_KEY", TEST_VAULT_KEY_B64);
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
    /// Optional model-catalog seed applied after boot. When `Some`, replaces
    /// whatever catalog `boot_with_config` produced (including any partial
    /// state left behind by `sync_registry`'s network fetch) with a
    /// deterministic baseline.
    catalog_seed: Option<CatalogSeed>,
}

impl MockKernelBuilder {
    /// Creates a builder with the default minimal configuration.
    pub fn new() -> Self {
        Self {
            config: KernelConfig::default(),
            config_fn: None,
            catalog_seed: None,
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

    /// Seed the model catalog with the given providers and models, replacing
    /// whatever `LibreFangKernel::boot_with_config` produced.
    ///
    /// Use this when tests assert on specific model ids. Without seeding,
    /// the catalog is whatever `librefang_runtime::registry_sync::sync_registry`
    /// fetched from `github.com/librefang-registry` — flaky on CI when the
    /// runner is rate-limited or the network is partitioned, and entirely
    /// undefined when no network is available at all.
    ///
    /// Pass [`test_catalog_baseline()`] for a sane minimum that covers the
    /// `librefang-api` integration test suite, or build your own pair when
    /// you need provider/model shapes the baseline doesn't include.
    pub fn with_catalog_seed(mut self, seed: CatalogSeed) -> Self {
        self.catalog_seed = Some(seed);
        self
    }

    /// Builds the kernel instance.
    ///
    /// Returns `(Arc<LibreFangKernel>, TempDir)` — the caller must hold onto
    /// `TempDir`, otherwise the temp directory will be deleted on drop,
    /// invalidating kernel file paths. The kernel is wrapped in `Arc` and has
    /// `set_self_handle` called on it so internal `kernel_handle()` lookups
    /// (used by `send_message_*`, agent forking, etc.) succeed in tests the
    /// same way they do in production (#3652).
    pub fn build(mut self) -> (Arc<LibreFangKernel>, TempDir) {
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

        let kernel = Arc::new(
            LibreFangKernel::boot_with_config(self.config).expect("failed to boot test kernel"),
        );
        kernel.set_self_handle();

        if let Some((providers, models)) = self.catalog_seed.take() {
            kernel.model_catalog_update(|cat| {
                *cat = ModelCatalog::from_entries(models.clone(), providers.clone());
            });
        }

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
pub fn test_kernel() -> (Arc<LibreFangKernel>, TempDir) {
    MockKernelBuilder::new().build()
}
