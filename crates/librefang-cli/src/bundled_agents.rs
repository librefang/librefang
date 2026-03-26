//! Registry-based content sync.
//!
//! Delegates to `librefang_runtime::registry_sync` for the actual sync logic.
//! This wrapper exists for backwards compatibility with CLI callers.

use std::path::Path;

/// Sync all content from the registry to the local librefang home directory.
pub fn sync_registry_agents(home_dir: &Path) {
    librefang_runtime::registry_sync::sync_registry(
        home_dir,
        librefang_runtime::registry_sync::DEFAULT_CACHE_TTL_SECS,
    );
}
