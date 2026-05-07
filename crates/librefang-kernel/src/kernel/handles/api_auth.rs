//! [`kernel_handle::ApiAuth`] — snapshot of the auth-relevant config + paired
//! device API keys + per-user role/api_key_hash entries the API auth
//! middleware needs on every request. Single `config.load()` so every field
//! comes from the same hot-reload generation (#3744).

use librefang_runtime::kernel_handle;

use super::super::LibreFangKernel;

impl kernel_handle::ApiAuth for LibreFangKernel {
    fn auth_snapshot(&self) -> kernel_handle::ApiAuthSnapshot {
        // Single `config.load()` so every field in the snapshot comes from
        // the same hot-reload generation. Per-request middleware would
        // otherwise race the reload barrier and observe e.g. an old
        // `api_key` alongside a new `dashboard_pass_hash`.
        let cfg = self.config.load();
        kernel_handle::ApiAuthSnapshot {
            api_key: cfg.api_key.clone(),
            dashboard: kernel_handle::DashboardRawConfig {
                user: cfg.dashboard_user.clone(),
                pass: cfg.dashboard_pass.clone(),
                pass_hash: cfg.dashboard_pass_hash.clone(),
            },
            home_dir: self.home_dir().to_path_buf(),
            device_api_keys: self.pairing.device_api_keys(),
            config_users: cfg
                .users
                .iter()
                .map(|u| kernel_handle::ApiUserConfigSnapshot {
                    name: u.name.clone(),
                    role: u.role.clone(),
                    api_key_hash: u.api_key_hash.clone(),
                })
                .collect(),
        }
    }
}
