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
/// for CN / air-gapped users). `base_url` overrides the default registry
/// repository URL when non-empty.
pub async fn sync_catalog_to(
    home_dir: &std::path::Path,
    registry_mirror: &str,
    base_url: &str,
) -> Result<CatalogSyncResult, String> {
    let cache_meta_dir = home_dir.join("cache").join("catalog");
    std::fs::create_dir_all(&cache_meta_dir)
        .map_err(|e| format!("Failed to create cache meta dir: {e}"))?;

    // Force a registry refresh. We call `refresh_registry_checkout`
    // (not `sync_registry`) because the catalog consumer only needs the
    // upstream checkout refreshed — not the full fan-out into
    // `workspaces/agents/`, `workflows/templates/`, etc. The broader
    // fan-out belongs to `kernel::boot_with_config`; clicking
    // "Update catalog" shouldn't, for example, overwrite user-managed
    // workflow templates. `refresh_registry_checkout` also acquires
    // the module's internal lock so concurrent callers (boot, 24h
    // background task, manual trigger) don't race on the same
    // working tree. It's blocking (git subprocess), so hop to a
    // blocking task to keep the runtime responsive.
    {
        let home = home_dir.to_path_buf();
        let mirror = registry_mirror.to_string();
        let base = base_url.to_string();
        let ok = tokio::task::spawn_blocking(move || {
            crate::registry_sync::refresh_registry_checkout(&home, 0, &mirror, &base)
        })
        .await
        .map_err(|e| format!("registry sync task failed: {e}"))?;
        if !ok {
            tracing::warn!(
                "refresh_registry_checkout returned false; proceeding with \
                 whatever is already on disk (previous sync may still be valid)"
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

/// Reclaim disk from the pre-unify cache layout.
///
/// Called from `LibreFangKernel::boot_with_config` — idempotent so it's
/// safe on every boot (each branch no-ops when the path is absent).
///
/// Removes three paths the old two-checkout design maintained, all of
/// which are now pure duplicates of `~/.librefang/registry/`:
/// - `~/.librefang/cache/registry/` — second git clone of the upstream repo
/// - `~/.librefang/cache/catalog/providers/` — copy of `registry/providers/`
/// - `~/.librefang/cache/catalog/aliases.toml` — copy of `registry/aliases.toml`
///
/// `cache/catalog/.last_sync` is deliberately preserved so
/// `GET /api/catalog/status` keeps returning the user's last-sync
/// timestamp across the upgrade.
pub fn remove_legacy_cache_dirs(home_dir: &std::path::Path) {
    fn remove_dir(path: &std::path::Path, label: &str) {
        if !path.exists() {
            return;
        }
        match std::fs::remove_dir_all(path) {
            Ok(()) => tracing::info!(path = %path.display(), "Removed legacy {label}"),
            Err(e) => tracing::warn!(
                path = %path.display(),
                error = %e,
                "Failed to remove legacy {label}"
            ),
        }
    }

    fn remove_file(path: &std::path::Path, label: &str) {
        if !path.is_file() {
            return;
        }
        match std::fs::remove_file(path) {
            Ok(()) => tracing::info!(path = %path.display(), "Removed legacy {label}"),
            Err(e) => tracing::warn!(
                path = %path.display(),
                error = %e,
                "Failed to remove legacy {label}"
            ),
        }
    }

    remove_dir(
        &home_dir.join("cache").join("registry"),
        "duplicate registry checkout",
    );
    remove_dir(
        &home_dir.join("cache").join("catalog").join("providers"),
        "cached catalog providers copy",
    );
    remove_file(
        &home_dir.join("cache").join("catalog").join("aliases.toml"),
        "cached aliases.toml copy",
    );
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
    fn test_provider_catalog_parse_mixed_text_and_image() {
        // Regression: an image-modality entry (no context_window /
        // max_output_tokens) must not cause the whole file to fail parsing.
        // Pre-#3074 this would error at the first image entry, daemon would
        // fall back to "Ok(file) = ..." catching nothing, and the entire
        // provider (e.g. all of openai.toml) would be silently dropped.
        let toml_str = r#"
[[models]]
id = "gpt-4.1"
display_name = "GPT-4.1"
provider = "openai"
tier = "frontier"
context_window = 1047576
max_output_tokens = 32768
input_cost_per_m = 2.0
output_cost_per_m = 8.0
supports_tools = true
supports_vision = true
supports_streaming = true

[[models]]
id = "gpt-image-2"
display_name = "GPT Image 2"
provider = "openai"
tier = "frontier"
modality = "image"
input_cost_per_m = 5.0
output_cost_per_m = 10.0
image_input_cost_per_m = 8.0
image_output_cost_per_m = 30.0
supports_tools = false
supports_vision = true
supports_streaming = false
"#;
        let file: ProviderCatalogFile =
            toml::from_str(toml_str).expect("mixed text+image catalog must parse");
        assert_eq!(file.models.len(), 2);
        assert_eq!(file.models[0].id, "gpt-4.1");
        assert_eq!(file.models[1].id, "gpt-image-2");
        assert_eq!(file.models[1].context_window, 0);
        assert!(file.models[1].is_image_generation());
        assert_eq!(file.models[1].image_output_cost_per_m, Some(30.0));
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
        remove_legacy_cache_dirs(tmp.path());
        assert!(!tmp.path().join("cache").join("registry").exists());
    }

    #[test]
    fn test_remove_legacy_deletes_duplicate_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let legacy = tmp.path().join("cache").join("registry").join("sub");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("file.toml"), "x").unwrap();
        remove_legacy_cache_dirs(tmp.path());
        assert!(!tmp.path().join("cache").join("registry").exists());
    }

    #[test]
    fn test_remove_legacy_deletes_cached_providers_copy() {
        let tmp = tempfile::tempdir().unwrap();
        let providers = tmp.path().join("cache").join("catalog").join("providers");
        std::fs::create_dir_all(&providers).unwrap();
        std::fs::write(providers.join("ollama.toml"), "x").unwrap();
        remove_legacy_cache_dirs(tmp.path());
        assert!(!providers.exists());
    }

    #[test]
    fn test_remove_legacy_preserves_last_sync_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_catalog = tmp.path().join("cache").join("catalog");
        std::fs::create_dir_all(&cache_catalog).unwrap();
        std::fs::write(cache_catalog.join(".last_sync"), "2026-04-21").unwrap();
        remove_legacy_cache_dirs(tmp.path());
        assert!(cache_catalog.join(".last_sync").exists());
    }

    #[test]
    fn test_remove_legacy_deletes_aliases_copy() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_catalog = tmp.path().join("cache").join("catalog");
        std::fs::create_dir_all(&cache_catalog).unwrap();
        let aliases = cache_catalog.join("aliases.toml");
        std::fs::write(&aliases, "[aliases]\nfoo = \"bar\"").unwrap();
        remove_legacy_cache_dirs(tmp.path());
        assert!(!aliases.exists());
        // Removing the file must not take the containing directory with it —
        // `.last_sync` lives in the same dir and must survive.
        assert!(cache_catalog.exists());
    }
}
