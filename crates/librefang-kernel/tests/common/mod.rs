use librefang_kernel::LibreFangKernel;
use librefang_types::config::{KernelConfig, MemoryConfig, UserConfig};

pub fn boot_kernel() -> (LibreFangKernel, tempfile::TempDir) {
    boot_kernel_with_users(Vec::new())
}

pub fn boot_kernel_with_users(users: Vec<UserConfig>) -> (LibreFangKernel, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("failed to create temp directory");
    let home_dir = tmp.path().to_path_buf();
    let data_dir = home_dir.join("data");
    std::fs::create_dir_all(&data_dir).expect("failed to create data directory");
    std::fs::create_dir_all(home_dir.join("skills")).unwrap();
    std::fs::create_dir_all(home_dir.join("workspaces").join("agents")).unwrap();
    std::fs::create_dir_all(home_dir.join("workspaces").join("hands")).unwrap();

    let config = KernelConfig {
        home_dir,
        data_dir: data_dir.clone(),
        network_enabled: false,
        memory: MemoryConfig {
            sqlite_path: Some(data_dir.join("test.db")),
            ..Default::default()
        },
        users,
        ..KernelConfig::default()
    };

    let kernel = LibreFangKernel::boot_with_config(config).expect("failed to boot test kernel");
    (kernel, tmp)
}
