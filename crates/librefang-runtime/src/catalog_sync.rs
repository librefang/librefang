//! Catalog sync — fetch model catalog updates from the remote repository.
//!
//! Downloads TOML files from `github.com/librefang/librefang-registry` and caches
//! them under `~/.librefang/cache/catalog/`. The cached catalog is loaded at
//! startup between the builtin fallback and the user's local overrides.

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

/// Default remote repository for the model catalog.
const CATALOG_REPO: &str = "librefang/librefang-registry";

/// Sync the model catalog from the remote repository.
///
/// Downloads TOML files from GitHub and saves to `home_dir/cache/catalog/`.
pub async fn sync_catalog_to(home_dir: &std::path::Path) -> Result<CatalogSyncResult, String> {
    let cache_dir = home_dir.join("cache").join("catalog");
    let providers_dir = cache_dir.join("providers");

    // Create directories
    std::fs::create_dir_all(&providers_dir)
        .map_err(|e| format!("Failed to create cache dir: {e}"))?;

    let client = crate::http_client::proxied_client_builder()
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    // Get the repo tree to find all TOML files
    let tree_url =
        format!("https://api.github.com/repos/{CATALOG_REPO}/git/trees/main?recursive=1");
    let tree_resp = client
        .get(&tree_url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch repo tree: {e}"))?;

    if !tree_resp.status().is_success() {
        return Err(format!("GitHub API returned {}", tree_resp.status()));
    }

    let tree: serde_json::Value = tree_resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse tree: {e}"))?;

    let mut downloaded = 0usize;
    let mut models_count = 0usize;

    if let Some(items) = tree["tree"].as_array() {
        for item in items {
            let path = item["path"].as_str().unwrap_or("");
            // Download provider TOML files and aliases.toml
            if (path.starts_with("providers/") && path.ends_with(".toml")) || path == "aliases.toml"
            {
                let raw_url =
                    format!("https://raw.githubusercontent.com/{CATALOG_REPO}/main/{path}");
                match client.get(&raw_url).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        if let Ok(content) = resp.text().await {
                            let dest = cache_dir.join(path);
                            if let Some(parent) = dest.parent() {
                                let _ = std::fs::create_dir_all(parent);
                            }
                            if std::fs::write(&dest, &content).is_ok() {
                                downloaded += 1;
                                // Count models in provider files
                                if path.starts_with("providers/") {
                                    if let Ok(file) =
                                        toml::from_str::<ProviderCatalogFile>(&content)
                                    {
                                        models_count += file.models.len();
                                    }
                                }
                            }
                        }
                    }
                    _ => {
                        tracing::warn!("Failed to download catalog file: {path}");
                    }
                }
            }
        }
    }

    // Write a timestamp so we know when we last synced
    let timestamp = chrono::Utc::now().to_rfc3339();
    let _ = std::fs::write(cache_dir.join(".last_sync"), &timestamp);

    Ok(CatalogSyncResult {
        files_downloaded: downloaded,
        models_count,
        timestamp,
    })
}

/// Check when the catalog was last synced.
pub fn last_sync_time_for(home_dir: &std::path::Path) -> Option<String> {
    let path = home_dir.join("cache").join("catalog").join(".last_sync");
    std::fs::read_to_string(path).ok()
}

/// Return the cache directory for the catalog.
pub fn cache_dir_for(home_dir: &std::path::Path) -> std::path::PathBuf {
    home_dir.join("cache").join("catalog")
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
    fn test_alias_catalog_parse() {
        #[derive(serde::Deserialize)]
        struct AliasFile {
            #[serde(default)]
            aliases: std::collections::HashMap<String, String>,
        }

        let toml_str = r#"
[aliases]
sonnet = "claude-sonnet-4-20250514"
gpt4 = "gpt-4o"
"#;
        let file: AliasFile = toml::from_str(toml_str).unwrap();
        assert_eq!(file.aliases.len(), 2);
        assert_eq!(file.aliases["sonnet"], "claude-sonnet-4-20250514");
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
}
