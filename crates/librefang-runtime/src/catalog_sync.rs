//! Catalog sync — fetch model catalog updates from the remote repository.
//!
//! Clones or pulls `github.com/librefang/librefang-registry` and copies TOML
//! files to `~/.librefang/cache/catalog/`. Uses git directly to avoid CDN
//! caching delays from `raw.githubusercontent.com`.

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
/// Clones or pulls the registry repo via git, then copies TOML files to
/// `home_dir/cache/catalog/`.
///
/// `registry_mirror` is an optional proxy/mirror prefix for GitHub URLs.
/// When non-empty, the clone URL is prefixed with this value.
pub async fn sync_catalog_to(
    home_dir: &std::path::Path,
    registry_mirror: &str,
) -> Result<CatalogSyncResult, String> {
    let cache_dir = home_dir.join("cache").join("catalog");
    let providers_dir = cache_dir.join("providers");
    let repo_dir = home_dir.join("cache").join("registry");

    std::fs::create_dir_all(&providers_dir)
        .map_err(|e| format!("Failed to create cache dir: {e}"))?;

    let mirror = registry_mirror.trim_end_matches('/');
    let repo_url = if mirror.is_empty() {
        format!("https://github.com/{CATALOG_REPO}.git")
    } else {
        format!("{mirror}/https://github.com/{CATALOG_REPO}.git")
    };

    // Clone or pull the registry repo
    let git_ok = if repo_dir.join(".git").exists() {
        // Pull latest changes
        tokio::task::spawn_blocking({
            let repo_dir = repo_dir.clone();
            move || {
                let mut cmd = std::process::Command::new("git");
                cmd.args(["pull", "--ff-only", "-q"])
                    .current_dir(&repo_dir)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                    cmd.creation_flags(CREATE_NO_WINDOW);
                }
                cmd.status().map(|s| s.success()).unwrap_or(false)
            }
        })
        .await
        .unwrap_or(false)
    } else {
        // Shallow clone (depth=1) to save bandwidth
        tokio::task::spawn_blocking({
            let repo_dir = repo_dir.clone();
            let repo_url = repo_url.clone();
            move || {
                let mut cmd = std::process::Command::new("git");
                cmd.args(["clone", "--depth", "1", "-q", &repo_url])
                    .arg(&repo_dir)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                    cmd.creation_flags(CREATE_NO_WINDOW);
                }
                cmd.status().map(|s| s.success()).unwrap_or(false)
            }
        })
        .await
        .unwrap_or(false)
    };

    if !git_ok {
        // Fallback to HTTP API if git is not available
        tracing::warn!("git clone/pull failed, falling back to HTTP API");
        return sync_catalog_http(home_dir, registry_mirror).await;
    }

    // Copy TOML files from repo to cache
    let mut downloaded = 0usize;
    let mut models_count = 0usize;

    let repo_providers = repo_dir.join("providers");
    if repo_providers.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&repo_providers) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "toml") {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        let dest = providers_dir.join(entry.file_name());
                        if std::fs::write(&dest, &content).is_ok() {
                            downloaded += 1;
                            if let Ok(file) = toml::from_str::<ProviderCatalogFile>(&content) {
                                models_count += file.models.len();
                            }
                        }
                    }
                }
            }
        }
    }

    // Copy aliases.toml if present
    let aliases_src = repo_dir.join("aliases.toml");
    if aliases_src.is_file() {
        let _ = std::fs::copy(&aliases_src, cache_dir.join("aliases.toml"));
    }

    let timestamp = chrono::Utc::now().to_rfc3339();
    let _ = std::fs::write(cache_dir.join(".last_sync"), &timestamp);

    Ok(CatalogSyncResult {
        files_downloaded: downloaded,
        models_count,
        timestamp,
    })
}

/// HTTP fallback — original implementation using GitHub API + raw URLs.
/// Used when git is not available on the system.
async fn sync_catalog_http(
    home_dir: &std::path::Path,
    registry_mirror: &str,
) -> Result<CatalogSyncResult, String> {
    let cache_dir = home_dir.join("cache").join("catalog");
    let client = crate::http_client::proxied_client_builder()
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let mirror = registry_mirror.trim_end_matches('/');
    let tree_url = if mirror.is_empty() {
        format!("https://api.github.com/repos/{CATALOG_REPO}/git/trees/main?recursive=1")
    } else {
        format!("{mirror}/https://api.github.com/repos/{CATALOG_REPO}/git/trees/main?recursive=1")
    };
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
            if !path.contains("..")
                && ((path.starts_with("providers/") && path.ends_with(".toml"))
                    || path == "aliases.toml")
            {
                let raw_url = if mirror.is_empty() {
                    format!("https://raw.githubusercontent.com/{CATALOG_REPO}/main/{path}")
                } else {
                    format!("{mirror}/https://raw.githubusercontent.com/{CATALOG_REPO}/main/{path}")
                };
                match client.get(&raw_url).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        if let Ok(content) = resp.text().await {
                            let dest = cache_dir.join(path);
                            if let Some(parent) = dest.parent() {
                                let _ = std::fs::create_dir_all(parent);
                            }
                            if std::fs::write(&dest, &content).is_ok() {
                                downloaded += 1;
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
