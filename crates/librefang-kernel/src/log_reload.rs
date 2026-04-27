//! Trait for swapping the global tracing log filter at runtime.
//!
//! `librefang-kernel` deliberately does not depend on `tracing-subscriber`,
//! so the binary that owns the subscriber (`librefang-cli` for the daemon)
//! implements this trait and injects it into the kernel after boot via
//! [`crate::LibreFangKernel::set_log_reloader`]. The hot-reload path
//! ([`crate::config_reload::HotAction::ReloadLogLevel`]) then calls
//! [`LogLevelReloader::reload`] to swap the live `EnvFilter` directive.

use std::sync::Arc;

/// Replace the active tracing log filter with one parsed from `level`.
///
/// `level` is the raw string from `KernelConfig::log_level`
/// (e.g. `"debug"`, `"info"`, or a full directive like
/// `"librefang_kernel=debug,info"`). Implementations should return an error
/// when the directive fails to parse so the caller can surface a useful
/// reload-failure reason instead of silently dropping the change.
pub trait LogLevelReloader: Send + Sync {
    fn reload(&self, level: &str) -> Result<(), String>;
}

pub type LogLevelReloaderArc = Arc<dyn LogLevelReloader>;
