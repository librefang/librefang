// ============================================================================
// 15. ApiAuth — raw auth-config values needed by the HTTP server layer to
//     build middleware token tables and bind-safety checks at startup.
//
//     Deliberately returns *raw* (unresolved) config strings so the API
//     server can apply its own credential-resolution logic (env-var override,
//     vault: prefix, literal) without pulling KernelConfig into that layer.
// ============================================================================

/// A snapshot of the user-config values needed for API-key table construction.
#[derive(Debug, Clone, Default)]
pub struct ApiUserConfigSnapshot {
    pub name: String,
    pub role: String,
    pub api_key_hash: Option<String>,
}

/// Raw dashboard credential strings from config (before env-var / vault
/// resolution). The HTTP server resolves them with `LIBREFANG_DASHBOARD_USER`,
/// `LIBREFANG_DASHBOARD_PASS`, and the `vault:KEY` prefix logic.
#[derive(Debug, Clone, Default)]
pub struct DashboardRawConfig {
    pub user: String,
    pub pass: String,
    pub pass_hash: String,
}

/// One-shot snapshot of every auth-relevant config field. Returned by
/// [`ApiAuth::auth_snapshot`] from a single `config.load()` so all fields
/// observe the same hot-reload generation — preventing per-request
/// middleware (`valid_api_tokens`, `paired_device_user_keys`) from mixing
/// pre-reload and post-reload config when a reload races with the request.
#[derive(Debug, Clone, Default)]
pub struct ApiAuthSnapshot {
    /// Raw `api_key` value from config (may be empty when auth is open).
    pub api_key: String,
    /// Raw dashboard credential strings (before env-var / vault resolution).
    pub dashboard: DashboardRawConfig,
    /// Absolute path to the daemon home directory (owned so the snapshot
    /// is fully self-contained and not tied to the kernel's lifetime).
    pub home_dir: std::path::PathBuf,
    /// Paired-device (mobile) API key hashes: `(device_id, api_key_hash)`.
    pub device_api_keys: Vec<(String, String)>,
    /// Per-user config entries used to build the user API-key table.
    pub config_users: Vec<ApiUserConfigSnapshot>,
}

pub trait ApiAuth: Send + Sync {
    /// Atomic snapshot of every auth-relevant config field. Implementations
    /// MUST acquire all values from a single config snapshot so callers see
    /// a consistent view across hot-reload boundaries.
    fn auth_snapshot(&self) -> ApiAuthSnapshot;
}
