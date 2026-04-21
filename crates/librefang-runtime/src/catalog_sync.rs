//! Catalog sync — refresh the shared registry checkout and report what's on disk.
//!
//! Previously this module maintained its own git clone at
//! `~/.librefang/cache/registry/`, copied `.toml` files into
//! `~/.librefang/cache/catalog/providers/`, and the in-memory catalog
//! reloader read from that copy. That was two redundant layers: the
//! content was identical to what `registry_sync` already checks out at
//! `~/.librefang/registry/`.
//!
//! Post-refactor: this module just drives `registry_sync` (force-refresh)
//! and returns stats by scanning `~/.librefang/registry/providers/`. The
//! `ModelCatalog::load_cached_catalog_for` consumer reads that same dir
//! directly, so there's no intermediate copy. The only thing still
//! living under `~/.librefang/cache/catalog/` is the `.last_sync`
//! timestamp file, kept there so existing `GET /api/catalog/status`
//! behaviour is unchanged.

use librefang_types::model_catalog::ModelCatalogEntry;
use serde::{Deserialize, Serialize};

/// Result of a catalog sync operation.
#[derive(Debug, Clone, Serialize)]
pub struct CatalogSyncResult {
    pub files_downloaded: usize,
    pub models_count: usize,
    pub timestamp: String,
}

/// A provider catalog TOML file with `[[models]]` entries.
#[derive(Debug, Deserialize)]
struct ProviderCatalogFile {
    #[serde(default)]
    models: Vec<ModelCatalogEntry>,
}

/// Sync the model catalog.
///
/// Triggers `registry_sync` (force-refresh, TTL=0) so `POST /api/catalog/update`
/// and the periodic background task always see upstream's current state,
/// then returns a count of what ended up on disk under
/// `~/.librefang/registry/providers/`.
///
/// `registry_mirror` is forwarded to `registry_sync` (GitHub proxy prefix
/// for CN / air-gapped users).
pub async fn sync_catalog_to(
    home_dir: &std::path::Path,
    registry_mirror: &str,
) -> Result<CatalogSyncResult, String> {
    let cache_meta_dir = home_dir.join("cache").join("catalog");
    std::fs::create_dir_all(&cache_meta_dir)
        .map_err(|e| format!("Failed to create cache meta dir: {e}"))?;

    // Force a registry refresh. `registry_sync` is blocking (git
    // subprocess + filesystem copies), so hop to a blocking task to
    // keep the runtime responsive.
    {
        let home = home_dir.to_path_buf();
        let mirror = registry_mirror.to_string();
        let ok = tokio::task::spawn_blocking(move || {
            crate::registry_sync::sync_registry(&home, 0, &mirror)
        })
        .await
        .map_err(|e| format!("registry sync task failed: {e}"))?;
        if !ok {
            tracing::warn!(
                "registry_sync returned false; proceeding with whatever is \
                 already on disk (previous sync may still be valid)"
            );
        }
    }

    let repo_providers = home_dir.join("registry").join("providers");
    let mut file_count = 0usize;
    let mut models_count = 0usize;

    if repo_providers.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&repo_providers) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "toml") {
                    file_count += 1;
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        if let Ok(file) = toml::from_str::<ProviderCatalogFile>(&content) {
                            models_count += file.models.len();
                        }
                    }
                }
            }
        }
    } else {
        tracing::warn!(
            path = %repo_providers.display(),
            "registry/providers missing — returning empty catalog result"
        );
    }

    let timestamp = chrono::Utc::now().to_rfc3339();
    let _ = std::fs::write(cache_meta_dir.join(".last_sync"), &timestamp);

    Ok(CatalogSyncResult {
        files_downloaded: file_count,
        models_count,
        timestamp,
    })
}

/// Check when the catalog was last synced.
pub fn last_sync_time_for(home_dir: &std::path::Path) -> Option<String> {
    let path = home_dir.join("cache").join("catalog").join(".last_sync");
    std::fs::read_to_string(path).ok()
}

/// Return the cache metadata directory for the catalog.
pub fn cache_dir_for(home_dir: &std::path::Path) -> std::path::PathBuf {
    home_dir.join("cache").join("catalog")
}

/// One-shot cleanup for directories obsoleted by the registry-unify
/// refactor. Called from `LibreFangKernel::boot_with_config` so existing
/// installs reclaim disk without any manual step.
///
/// Removes:
/// - `~/.librefang/cache/registry/` — duplicate checkout of the upstream
///   registry repo (now read from `~/.librefang/registry/`).
/// - `~/.librefang/cache/catalog/providers/` — copy of
///   `~/.librefang/registry/providers/` (consumer now reads the source).
///
/// Safe to call every boot — each step no-ops when the path is absent.
pub fn remove_legacy_registry_checkout(home_dir: &std::path::Path) {
    let legacy_repo = home_dir.join("cache").join("registry");
    if legacy_repo.exists() {
        match std::fs::remove_dir_all(&legacy_repo) {
            Ok(()) => tracing::info!(
                path = %legacy_repo.display(),
                "Removed legacy duplicate registry checkout"
            ),
            Err(e) => tracing::warn!(
                path = %legacy_repo.display(),
                error = %e,
                "Failed to remove legacy duplicate registry checkout"
            ),
        }
    }

    let legacy_providers = home_dir.join("cache").join("catalog").join("providers");
    if legacy_providers.exists() {
        match std::fs::remove_dir_all(&legacy_providers) {
            Ok(()) => tracing::info!(
                path = %legacy_providers.display(),
                "Removed legacy cached catalog providers copy"
            ),
            Err(e) => tracing::warn!(
                path = %legacy_providers.display(),
                error = %e,
                "Failed to remove legacy cached catalog providers copy"
            ),
        }
    }

    // `~/.librefang/cache/catalog/aliases.toml` was also a copy of
    // registry/aliases.toml; the model catalog's alias loader already
    // handles either location, so drop the stale copy too.
    let legacy_aliases = home_dir.join("cache").join("catalog").join("aliases.toml");
    if legacy_aliases.is_file() {
        let _ = std::fs::remove_file(&legacy_aliases);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_catalog_parse() {
        let toml_str = r#"
[[models]]
id = "test-model"
display_name = "Test Model"
provider = "test"
tier = "balanced"
context_window = 4096
max_output_tokens = 1024
input_cost_per_m = 1.0
output_cost_per_m = 2.0
supports_tools = true
supports_vision = false
supports_streaming = true
"#;
        let file: ProviderCatalogFile = toml::from_str(toml_str).unwrap();
        assert_eq!(file.models.len(), 1);
        assert_eq!(file.models[0].id, "test-model");
    }

    #[test]
    fn test_last_sync_time_missing() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(last_sync_time_for(tmp.path()).is_none());
    }

    #[test]
    fn test_cache_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let d = cache_dir_for(tmp.path());
        assert!(d.ends_with("cache/catalog") || d.ends_with("cache\\catalog"));
    }

    #[test]
    fn test_remove_legacy_noop_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        remove_legacy_registry_checkout(tmp.path());
        assert!(!tmp.path().join("cache").join("registry").exists());
    }

    #[test]
    fn test_remove_legacy_deletes_duplicate_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let legacy = tmp.path().join("cache").join("registry").join("sub");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("file.toml"), "x").unwrap();
        remove_legacy_registry_checkout(tmp.path());
        assert!(!tmp.path().join("cache").join("registry").exists());
    }

    #[test]
    fn test_remove_legacy_deletes_cached_providers_copy() {
        let tmp = tempfile::tempdir().unwrap();
        let providers = tmp.path().join("cache").join("catalog").join("providers");
        std::fs::create_dir_all(&providers).unwrap();
        std::fs::write(providers.join("ollama.toml"), "x").unwrap();
        remove_legacy_registry_checkout(tmp.path());
        assert!(!providers.exists());
    }

    #[test]
    fn test_remove_legacy_preserves_last_sync_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_catalog = tmp.path().join("cache").join("catalog");
        std::fs::create_dir_all(&cache_catalog).unwrap();
        std::fs::write(cache_catalog.join(".last_sync"), "2026-04-21").unwrap();
        remove_legacy_registry_checkout(tmp.path());
        assert!(cache_catalog.join(".last_sync").exists());
    }
}
