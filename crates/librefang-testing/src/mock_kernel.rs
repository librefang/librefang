//! MockKernelBuilder — 构建最小化的 `LibreFangKernel` 用于测试。
//!
//! 使用内存 SQLite、临时目录，跳过网络/OFP/cron 等重量级初始化。
//! 底层通过 `LibreFangKernel::boot_with_config` 构建真实 kernel 实例。

use librefang_kernel::LibreFangKernel;
use librefang_types::config::KernelConfig;
use tempfile::TempDir;

/// 测试用 kernel 构建器。
///
/// 通过 builder 模式配置 kernel，然后调用 `.build()` 生成一个
/// 真实的 `LibreFangKernel` 实例（使用临时目录和内存数据库）。
///
/// # 示例
///
/// ```rust,ignore
/// // ignore: 需要完整 kernel 启动环境（临时目录、SQLite），详见 tests.rs 中的集成测试
/// use librefang_testing::MockKernelBuilder;
///
/// let (kernel, _tmp) = MockKernelBuilder::new().build();
/// assert!(kernel.registry.list().is_empty());
/// ```
type ConfigFn = Box<dyn FnOnce(&mut KernelConfig)>;

pub struct MockKernelBuilder {
    config: KernelConfig,
    /// 自定义 config 修改函数。
    config_fn: Option<ConfigFn>,
}

impl MockKernelBuilder {
    /// 创建一个使用默认最小配置的 builder。
    pub fn new() -> Self {
        Self {
            config: KernelConfig::default(),
            config_fn: None,
        }
    }

    /// 设置自定义的 config 修改函数。
    ///
    /// ```rust,ignore
    /// // ignore: 需要完整 kernel 启动环境（临时目录、SQLite），详见 tests.rs 中的集成测试
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

    /// 构建 kernel 实例。
    ///
    /// 返回 `(LibreFangKernel, TempDir)` — 调用方必须持有 `TempDir`，
    /// 否则临时目录会在 drop 时被删除，导致 kernel 文件路径失效。
    pub fn build(mut self) -> (LibreFangKernel, TempDir) {
        let tmp = tempfile::tempdir().expect("无法创建临时目录");
        let home_dir = tmp.path().to_path_buf();
        let data_dir = home_dir.join("data");

        // 确保必要目录存在
        std::fs::create_dir_all(&data_dir).expect("无法创建 data 目录");
        std::fs::create_dir_all(home_dir.join("skills")).expect("无法创建 skills 目录");
        std::fs::create_dir_all(home_dir.join("workspaces")).expect("无法创建 workspaces 目录");

        // 配置最小化 kernel
        self.config.home_dir = home_dir;
        self.config.data_dir = data_dir;
        self.config.network_enabled = false;
        // 使用内存 SQLite（设置路径为 :memory: 无效，boot_with_config 会用文件路径）
        // 因此使用临时目录下的文件路径即可
        self.config.memory.sqlite_path = Some(self.config.data_dir.join("test.db"));

        // 应用自定义配置
        if let Some(f) = self.config_fn.take() {
            f(&mut self.config);
        }

        let kernel = LibreFangKernel::boot_with_config(self.config).expect("测试 kernel 启动失败");

        (kernel, tmp)
    }
}

impl Default for MockKernelBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// 快速构建一个默认的测试 kernel（便捷函数）。
///
/// 等价于 `MockKernelBuilder::new().build()`。
pub fn test_kernel() -> (LibreFangKernel, TempDir) {
    MockKernelBuilder::new().build()
}
