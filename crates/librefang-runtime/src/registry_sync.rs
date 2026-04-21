//! Registry sync — download the librefang-registry tarball and copy content to
//! `~/.librefang/`. Called automatically on kernel boot when the providers/
//! directory is missing, ensuring a fresh install or upgrade gets content
//! without requiring an explicit `librefang init`.
//!
//! Tries git first (incremental pull, private fork support). Falls back to HTTP
//! tarball download when git is unavailable (Docker, minimal VMs).
//! if the HTTP download fails, for users behind proxies that block GitHub
//! archive downloads.

use std::path::Path;
use std::process::Command;
use std::sync::Mutex;

/// GitHub tarball URL for the registry (no auth required).
const REGISTRY_TARBALL_URL: &str =
    "https://github.com/librefang/librefang-registry/archive/refs/heads/main.tar.gz";

/// Fallback: git clone URL.
const REGISTRY_REPO: &str = "https://github.com/librefang/librefang-registry.git";

/// Prefix inside the tarball (GitHub convention: `{repo}-{branch}/`).
const TARBALL_PREFIX: &str = "librefang-registry-main/";

/// Default cache TTL: how long (in seconds) before we re-download the registry.
/// Callers without access to `KernelConfig` can use this value directly.
pub const DEFAULT_CACHE_TTL_SECS: u64 = 24 * 60 * 60; // 24 hours

/// Serialises all writes to `~/.librefang/registry/`.
///
/// Without this, a manual `POST /api/catalog/update` firing at the same
/// time as the 24h background catalog task could have two `git pull`
/// subprocesses racing on the same working tree, which corrupts the
/// checkout. Boot-time `sync_registry` and the catalog-only
/// `refresh_registry_checkout` both acquire it. The lock is held across
/// the blocking git/tar work; callers already run these via
/// `spawn_blocking`, so a `std::sync::Mutex` is appropriate.
static SYNC_LOCK: Mutex<()> = Mutex::new(());

/// Refresh only the `~/.librefang/registry/` checkout from upstream —
/// no fan-out into `workspaces/`, `providers/`, `workflows/templates/`
/// etc. Callers like `catalog_sync` want the repo refreshed without
/// accidentally re-installing agent templates or overwriting workflow
/// templates every time the user clicks "Update catalog".
///
/// Returns `true` when the checkout is in a usable state (fresh pull,
/// fresh clone, fresh tarball extract, or the on-disk copy from a
/// previous successful run).
pub fn refresh_registry_checkout(
    home_dir: &Path,
    cache_ttl_secs: u64,
    registry_mirror: &str,
) -> bool {
    let _guard = SYNC_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let registry_cache = home_dir.join("registry");

    if !should_refresh(&registry_cache, cache_ttl_secs) {
        tracing::debug!("Registry cache is fresh, skipping download");
        return registry_cache.exists();
    }

    // Try git first (faster incremental updates, private fork support)
    let git_ok = match git_clone_fallback(&registry_cache, registry_mirror) {
        Ok(()) => true,
        Err(e) => {
            tracing::debug!("Git sync unavailable: {e} — trying HTTP download");
            false
        }
    };

    // Fall back to HTTP tarball if git failed
    if !git_ok {
        if let Err(e) = download_and_extract(&registry_cache, registry_mirror) {
            tracing::warn!("HTTP registry download also failed: {e}");
            return registry_cache.exists();
        }
    }
    true
}

/// Sync all content from the registry to the local librefang home directory.
///
/// Downloads the registry tarball via HTTP, extracts it, then copies items
/// that don't already exist on disk (preserves user customization).
/// Tries git first (incremental pull, supports private forks), falls back to
/// HTTP tarball download when git is unavailable (Docker, minimal VMs).
///
/// `cache_ttl_secs` controls how long the local cache is considered fresh
/// before triggering a re-download. Pass [`DEFAULT_CACHE_TTL_SECS`] when
/// no user-configured value is available.
///
/// `registry_mirror` is an optional proxy/mirror prefix for GitHub URLs.
/// When non-empty, all GitHub URLs are prefixed with this value (e.g.
/// `"https://ghproxy.cn"` rewrites `https://github.com/...` to
/// `https://ghproxy.cn/https://github.com/...`).
pub fn sync_registry(home_dir: &Path, cache_ttl_secs: u64, registry_mirror: &str) -> bool {
    let _guard = SYNC_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let registry_cache = home_dir.join("registry");

    if should_refresh(&registry_cache, cache_ttl_secs) {
        // Try git first (faster incremental updates, private fork support)
        let git_ok = match git_clone_fallback(&registry_cache, registry_mirror) {
            Ok(()) => true,
            Err(e) => {
                tracing::debug!("Git sync unavailable: {e} — trying HTTP download");
                false
            }
        };

        // Fall back to HTTP tarball if git failed
        if !git_ok {
            if let Err(e) = download_and_extract(&registry_cache, registry_mirror) {
                tracing::warn!("HTTP registry download also failed: {e}");
                if !registry_cache.exists() {
                    return false;
                }
            }
        }
    } else {
        tracing::debug!("Registry cache is fresh, skipping download");
    }

    // Pre-install core content users need out of the box.
    // Skills and plugins stay in registry — users install via dashboard.
    for &dir_name in &["providers", "channels"] {
        let src_dir = registry_cache.join(dir_name);
        if src_dir.exists() {
            sync_flat_files(&src_dir, &home_dir.join(dir_name), dir_name);
        }
    }
    // MCP catalog templates: upstream publishes them under `mcp/`;
    // on disk they live as the read-only catalog at `mcp/catalog/`.
    {
        let src_dir = registry_cache.join("mcp");
        if src_dir.exists() {
            sync_flat_files(
                &src_dir,
                &home_dir.join("mcp").join("catalog"),
                "mcp/catalog",
            );
        }
    }

    // Pre-install agent templates from registry (e.g. hello-world)
    let agents_src = registry_cache.join("agents");
    if agents_src.exists() {
        let agents_dest = home_dir.join("workspaces").join("agents");
        if let Ok(entries) = std::fs::read_dir(&agents_src) {
            for entry in entries.flatten() {
                let src = entry.path();
                if !src.is_dir() || !src.join("agent.toml").exists() {
                    continue;
                }
                let name = src.file_name().unwrap_or_default();
                let dest = agents_dest.join(name);
                if !dest.exists() {
                    let _ = std::fs::create_dir_all(&dest);
                    let _ = copy_dir_recursive(&src, &dest);
                }
            }
        }
    }

    // Pre-install workflow templates (always overwrite so updates land)
    let workflows_src = registry_cache.join("workflows");
    if workflows_src.is_dir() {
        let workflows_dest = home_dir.join("workflows").join("templates");
        let _ = std::fs::create_dir_all(&workflows_dest);
        let mut installed = 0usize;
        if let Ok(entries) = std::fs::read_dir(&workflows_src) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                    if let Some(name) = path.file_name() {
                        let dest = workflows_dest.join(name);
                        if std::fs::copy(&path, &dest).is_ok() {
                            installed += 1;
                        }
                    }
                }
            }
        }
        if installed > 0 {
            tracing::info!("Pre-installed {installed} workflow template(s) from registry");
        }
    }

    // Sync aliases (only on first run — user may customize)
    let aliases_src = registry_cache.join("aliases.toml");
    let aliases_dest = home_dir.join("aliases.toml");
    if aliases_src.exists() && !aliases_dest.exists() {
        let _ = std::fs::copy(&aliases_src, &aliases_dest);
    }

    // Sync schema — only overwrite when source is machine-parseable.
    // The registry may still ship the old comment-based format; copying that
    // would replace a valid schema the user (or a prior release) placed manually.
    let schema_src = registry_cache.join("schema.toml");
    let schema_dest = home_dir.join("schema.toml");
    if schema_src.exists() {
        let src_parseable = std::fs::read_to_string(&schema_src)
            .ok()
            .and_then(|c| {
                toml::from_str::<librefang_types::registry_schema::RegistrySchema>(&c).ok()
            })
            .is_some_and(|s| !s.content_types.is_empty());
        if src_parseable {
            let _ = std::fs::copy(&schema_src, &schema_dest);
        }
    }

    // Clean up stale hand directories in workspaces
    let workspaces_dir = home_dir.join("workspaces");
    if workspaces_dir.exists() {
        cleanup_stale_dirs(&workspaces_dir);
    }
    true
}

/// Check whether we should re-download the registry.
///
/// Returns `false` if the cache exists and the marker file is younger than
/// `cache_ttl_secs`.
fn should_refresh(registry_cache: &Path, cache_ttl_secs: u64) -> bool {
    let marker = registry_cache.join(".sync_marker");
    if !marker.exists() {
        return true;
    }
    let Ok(meta) = marker.metadata() else {
        return true;
    };
    let Ok(modified) = meta.modified() else {
        return true;
    };
    let Ok(age) = modified.elapsed() else {
        return true;
    };
    age.as_secs() > cache_ttl_secs
}

/// Touch (create/update) the sync marker file.
fn touch_marker(registry_cache: &Path) {
    let marker = registry_cache.join(".sync_marker");
    let _ = std::fs::create_dir_all(registry_cache);
    let _ = std::fs::write(&marker, "");
}

/// Prefix a URL with the mirror/proxy base when set.
///
/// E.g. `apply_mirror("https://ghproxy.cn", "https://github.com/foo")` →
///      `"https://ghproxy.cn/https://github.com/foo"`
fn apply_mirror(mirror: &str, url: &str) -> String {
    if mirror.is_empty() {
        url.to_string()
    } else {
        format!("{}/{}", mirror.trim_end_matches('/'), url)
    }
}

/// Download the tarball via HTTP and extract it into `registry_cache`.
fn download_and_extract(
    registry_cache: &Path,
    registry_mirror: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let url = apply_mirror(registry_mirror, REGISTRY_TARBALL_URL);
    tracing::info!("Downloading registry from {url}");

    let resp = ureq::get(&url).call()?;
    let reader = resp.into_body().into_reader();

    // Decompress gzip
    let gz = flate2::read::GzDecoder::new(reader);

    // Extract tar
    let mut archive = tar::Archive::new(gz);

    // Extract to a temporary directory first, then swap — this avoids leaving
    // a half-extracted directory on error.
    let tmp_dir = registry_cache
        .parent()
        .unwrap_or_else(|| Path::new("/tmp"))
        .join(".registry_tmp");

    // Clean up any previous failed attempt
    if tmp_dir.exists() {
        std::fs::remove_dir_all(&tmp_dir)?;
    }
    std::fs::create_dir_all(&tmp_dir)?;

    // Extract, stripping the `librefang-registry-main/` prefix
    for entry in archive.entries()? {
        let mut entry: tar::Entry<_> = entry?;
        let path = entry.path()?;
        let path_str = path.to_string_lossy();

        // Strip the tarball prefix
        let relative: String = match path_str.strip_prefix(TARBALL_PREFIX) {
            Some(r) if !r.is_empty() => r.to_string(),
            _ => continue,
        };

        let dest = tmp_dir.join(&relative);

        // Create parent directories
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Only extract files and directories
        if entry.header().entry_type().is_dir() {
            std::fs::create_dir_all(&dest)?;
        } else if entry.header().entry_type().is_file() {
            entry.unpack(&dest)?;
        }
    }

    // Swap: remove old cache, rename tmp to cache
    if registry_cache.exists() {
        std::fs::remove_dir_all(registry_cache)?;
    }
    std::fs::rename(&tmp_dir, registry_cache)?;

    touch_marker(registry_cache);
    tracing::info!("Registry downloaded and extracted successfully");

    Ok(())
}

/// Fallback: clone the registry using git (for environments where HTTP tarball
/// download fails but git is available).
fn git_clone_fallback(
    registry_cache: &Path,
    registry_mirror: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    tracing::info!("Attempting git clone fallback");

    if registry_cache.join(".git").exists() {
        // Already a git repo — fetch and reset to origin/main so that a
        // detached HEAD or local branch can never stall the sync.
        let fetch_ok = Command::new("git")
            .args(["fetch", "--depth", "1", "-q", "origin", "main"])
            .current_dir(registry_cache)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !fetch_ok {
            return Err("git fetch origin main failed".into());
        }
        let status = Command::new("git")
            .args(["reset", "--hard", "origin/main", "-q"])
            .current_dir(registry_cache)
            .status()?;
        if !status.success() {
            return Err(format!("git reset exited with {status}").into());
        }
    } else {
        // Clean slate
        if registry_cache.exists() {
            std::fs::remove_dir_all(registry_cache)?;
        }
        let repo_url = apply_mirror(registry_mirror, REGISTRY_REPO);
        let status = Command::new("git")
            .args([
                "clone",
                "--depth",
                "1",
                "-q",
                &repo_url,
                &registry_cache.display().to_string(),
            ])
            .status()?;
        if !status.success() {
            return Err(format!("git clone exited with {status}").into());
        }
    }

    touch_marker(registry_cache);
    Ok(())
}

/// Check if the registry content appears to be populated.
///
/// Returns `false` if any critical directories are missing, meaning
/// auto-sync should run.
/// Resolve the default home directory (for tests and standalone usage).
pub fn resolve_home_dir_for_tests() -> std::path::PathBuf {
    // OnceLock ensures the registry sync runs exactly once per process,
    // preventing concurrent git clone races when tests run in parallel threads.
    use std::sync::OnceLock;
    static HOME: OnceLock<std::path::PathBuf> = OnceLock::new();
    HOME.get_or_init(|| {
        let home = std::env::var("LIBREFANG_HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| {
                // Use process-unique dir to avoid git lock conflicts
                // when nextest runs tests in parallel processes.
                std::env::temp_dir().join(format!("librefang-test-{}", std::process::id()))
            });
        // Auto-sync if the providers dir is empty (fresh CI environment)
        if !home.join("providers").exists()
            || std::fs::read_dir(home.join("providers"))
                .map(|d| d.count() == 0)
                .unwrap_or(true)
        {
            sync_registry(&home, DEFAULT_CACHE_TTL_SECS, "");
        }
        home
    })
    .clone()
}

pub fn needs_sync(home_dir: &Path) -> bool {
    // Only check if the registry cache is populated
    !home_dir.join("registry").join("providers").exists()
}

/// Sync flat .toml files (e.g. integrations/, providers/).
fn sync_flat_files(src_dir: &Path, dest_dir: &Path, label: &str) {
    let entries = match std::fs::read_dir(src_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let mut synced = 0;
    let mut updated = 0;
    let mut skipped = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) if n.ends_with(".toml") => n.to_string(),
            _ => continue,
        };

        let dest_file = dest_dir.join(&name);
        if dest_file.exists() {
            // Update if content differs — keeps builtin provider metadata (e.g.
            // supports_thinking, new model entries) in sync with the registry.
            // User API key config lives in config.toml, not in these TOML files.
            let src_content = std::fs::read(&path).unwrap_or_default();
            let dst_content = std::fs::read(&dest_file).unwrap_or_default();
            if src_content == dst_content {
                skipped += 1;
            } else if std::fs::create_dir_all(dest_dir).is_ok()
                && std::fs::write(&dest_file, &src_content).is_ok()
            {
                updated += 1;
            }
            continue;
        }

        if std::fs::create_dir_all(dest_dir).is_ok() && std::fs::copy(&path, &dest_file).is_ok() {
            synced += 1;
        }
    }

    // Remove local files that no longer exist in the registry source.
    // This cleans up defunct providers/integrations after upstream pruning.
    let mut removed = 0usize;
    if let Ok(dest_entries) = std::fs::read_dir(dest_dir) {
        for entry in dest_entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) if n.ends_with(".toml") => n.to_string(),
                _ => continue,
            };
            if !src_dir.join(&name).exists() && std::fs::remove_file(&path).is_ok() {
                removed += 1;
            }
        }
    }

    if synced > 0 || updated > 0 || removed > 0 || skipped > 0 {
        tracing::info!("{label} synced ({synced} new, {updated} updated, {removed} removed, {skipped} unchanged)");
    }
}

/// Extract the `version = "X.Y.Z"` value from a manifest file via line scan.
///
/// Avoids full TOML parse (which may fail on new-format files that older code
/// can't deserialize). Returns `None` if the file can't be read or has no
/// version field.
#[cfg(test)]
fn extract_version(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("version") {
            let rest = rest.trim_start();
            if let Some(rest) = rest.strip_prefix('=') {
                let rest = rest.trim();
                // Strip surrounding quotes
                let ver = rest.trim_matches('"').trim_matches('\'');
                if !ver.is_empty() {
                    return Some(ver.to_string());
                }
            }
        }
    }
    None
}

/// Compare two semver-like version strings numerically.
///
/// Returns `true` if `a` is strictly newer than `b`. Non-numeric segments
/// compare as 0 to avoid panics on malformed versions.
#[cfg(test)]
fn version_newer_than(a: &str, b: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.split('.')
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect()
    };
    let va = parse(a);
    let vb = parse(b);
    let len = va.len().max(vb.len());
    for i in 0..len {
        let pa = va.get(i).copied().unwrap_or(0);
        let pb = vb.get(i).copied().unwrap_or(0);
        if pa != pb {
            return pa > pb;
        }
    }
    false
}

/// Sync subdirectory-based content (e.g. hands/).
///
/// When a destination manifest already exists, compares `version` fields.
/// If the source has a newer version, replaces the destination directory
/// (user settings live in `hand_state.json`, not in the manifest).
#[cfg(test)]
fn sync_subdirs(src_dir: &Path, dest_dir: &Path, manifest_file: &str, label: &str) {
    let entries = match std::fs::read_dir(src_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let mut synced = 0;
    let mut updated = 0;
    let mut skipped = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let src_manifest = path.join(manifest_file);
        if !src_manifest.exists() {
            continue;
        }

        let item_dest = dest_dir.join(&name);
        let dest_manifest = item_dest.join(manifest_file);

        if dest_manifest.exists() {
            // Check if source version is newer
            let src_ver = extract_version(&src_manifest).unwrap_or_default();
            let dest_ver = extract_version(&dest_manifest).unwrap_or_default();

            if !version_newer_than(&src_ver, &dest_ver) {
                skipped += 1;
                continue;
            }

            // Source is newer — replace destination
            tracing::debug!("{label}/{name}: updating {dest_ver} → {src_ver}");
            if std::fs::remove_dir_all(&item_dest).is_err() {
                skipped += 1;
                continue;
            }
            if copy_dir_recursive(&path, &item_dest).is_ok() {
                updated += 1;
            }
        } else if copy_dir_recursive(&path, &item_dest).is_ok() {
            synced += 1;
        }
    }

    if synced > 0 || updated > 0 || skipped > 0 {
        tracing::info!("{label} synced ({synced} new, {updated} updated, {skipped} unchanged)");
    }
}

/// Remove stale hand directories that have `agent.toml` but no `HAND.toml`.
///
/// These are remnants of the old `*-hand` naming convention where each hand
/// was a plain agent directory. Now every hand must have a `HAND.toml`.
fn cleanup_stale_dirs(hands_dir: &Path) {
    let entries = match std::fs::read_dir(hands_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let has_hand_toml = path.join("HAND.toml").exists();
        let has_agent_toml = path.join("agent.toml").exists();

        if has_agent_toml && !has_hand_toml {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
            tracing::info!("Removing stale hand directory: {name}");
            if std::fs::remove_dir_all(&path).is_ok() {
                removed += 1;
            }
        }
    }

    if removed > 0 {
        tracing::info!("Cleaned up {removed} stale hand directories");
    }
}

/// Recursively copy a directory.
fn copy_dir_recursive(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dest_path = dest.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dest_path)?;
        } else {
            std::fs::copy(&src_path, &dest_path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_needs_sync_when_registry_cache_missing() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(needs_sync(tmp.path()));
    }

    #[test]
    fn test_needs_sync_when_registry_cache_exists() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("registry").join("providers")).unwrap();
        assert!(!needs_sync(tmp.path()));
    }

    #[test]
    fn test_should_refresh_no_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().join("registry");
        std::fs::create_dir_all(&cache).unwrap();
        assert!(super::should_refresh(&cache, super::DEFAULT_CACHE_TTL_SECS));
    }

    #[test]
    fn test_should_refresh_fresh_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().join("registry");
        std::fs::create_dir_all(&cache).unwrap();
        super::touch_marker(&cache);
        assert!(!super::should_refresh(
            &cache,
            super::DEFAULT_CACHE_TTL_SECS
        ));
    }

    #[test]
    fn test_extract_version() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("HAND.toml");

        std::fs::write(&path, "id = \"test\"\nversion = \"1.2.3\"\nname = \"Test\"").unwrap();
        assert_eq!(extract_version(&path), Some("1.2.3".to_string()));

        std::fs::write(&path, "id = \"test\"\nname = \"Test\"").unwrap();
        assert_eq!(extract_version(&path), None);

        std::fs::write(&path, "  version  =  \"0.1.0\"  ").unwrap();
        assert_eq!(extract_version(&path), Some("0.1.0".to_string()));
    }

    #[test]
    fn test_version_newer_than() {
        assert!(version_newer_than("1.0.0", "0.9.9"));
        assert!(version_newer_than("2.0.0", "1.99.99"));
        assert!(version_newer_than("1.1.0", "1.0.9"));
        assert!(version_newer_than("1.0.1", "1.0.0"));

        assert!(!version_newer_than("1.0.0", "1.0.0"));
        assert!(!version_newer_than("0.9.0", "1.0.0"));
        assert!(!version_newer_than("", "0.0.1"));

        // Different segment counts
        assert!(version_newer_than("1.0.0", "0.9"));
        assert!(!version_newer_than("1.0", "1.0.0"));
    }

    #[test]
    fn test_sync_subdirs_updates_newer_version() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src_hands");
        let dest = tmp.path().join("dest_hands");

        // Source: v2.0.0
        let src_hand = src.join("clip");
        std::fs::create_dir_all(&src_hand).unwrap();
        std::fs::write(
            src_hand.join("HAND.toml"),
            "id = \"clip\"\nversion = \"2.0.0\"\nname = \"Clip v2\"",
        )
        .unwrap();

        // Dest: v1.0.0
        let dest_hand = dest.join("clip");
        std::fs::create_dir_all(&dest_hand).unwrap();
        std::fs::write(
            dest_hand.join("HAND.toml"),
            "id = \"clip\"\nversion = \"1.0.0\"\nname = \"Clip v1\"",
        )
        .unwrap();

        sync_subdirs(&src, &dest, "HAND.toml", "hands");

        let content = std::fs::read_to_string(dest_hand.join("HAND.toml")).unwrap();
        assert!(content.contains("2.0.0"), "should have been updated to v2");
        assert!(content.contains("Clip v2"));
    }

    #[test]
    fn test_sync_subdirs_skips_same_version() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src_hands");
        let dest = tmp.path().join("dest_hands");

        let src_hand = src.join("clip");
        std::fs::create_dir_all(&src_hand).unwrap();
        std::fs::write(
            src_hand.join("HAND.toml"),
            "id = \"clip\"\nversion = \"1.0.0\"\nname = \"Clip src\"",
        )
        .unwrap();

        let dest_hand = dest.join("clip");
        std::fs::create_dir_all(&dest_hand).unwrap();
        std::fs::write(
            dest_hand.join("HAND.toml"),
            "id = \"clip\"\nversion = \"1.0.0\"\nname = \"Clip dest\"",
        )
        .unwrap();

        sync_subdirs(&src, &dest, "HAND.toml", "hands");

        let content = std::fs::read_to_string(dest_hand.join("HAND.toml")).unwrap();
        assert!(
            content.contains("Clip dest"),
            "should NOT have been overwritten"
        );
    }

    #[test]
    fn test_cleanup_stale_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let hands = tmp.path().join("workspaces");

        // Stale: has agent.toml but no HAND.toml
        let stale = hands.join("old-hand");
        std::fs::create_dir_all(&stale).unwrap();
        std::fs::write(stale.join("agent.toml"), "name = \"old\"").unwrap();

        // Valid: has HAND.toml
        let valid = hands.join("new-hand");
        std::fs::create_dir_all(&valid).unwrap();
        std::fs::write(valid.join("HAND.toml"), "id = \"new\"").unwrap();

        // Has both — should NOT be removed
        let both = hands.join("migrated-hand");
        std::fs::create_dir_all(&both).unwrap();
        std::fs::write(both.join("agent.toml"), "name = \"m\"").unwrap();
        std::fs::write(both.join("HAND.toml"), "id = \"m\"").unwrap();

        cleanup_stale_dirs(&hands);

        assert!(!stale.exists(), "stale dir should be removed");
        assert!(valid.exists(), "valid dir should remain");
        assert!(both.exists(), "dir with both files should remain");
    }
}
