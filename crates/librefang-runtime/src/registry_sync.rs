//! Registry sync — download the librefang-registry tarball and copy content to
//! `~/.librefang/`. Called automatically on kernel boot when the providers/
//! directory is missing, ensuring a fresh install or upgrade gets content
//! without requiring an explicit `librefang init`.
//!
//! Uses HTTP tarball download (no git dependency). Falls back to `git clone`
//! if the HTTP download fails, for users behind proxies that block GitHub
//! archive downloads.

use std::path::Path;
use std::process::Command;

/// GitHub tarball URL for the registry (no auth required).
const REGISTRY_TARBALL_URL: &str =
    "https://github.com/librefang/librefang-registry/archive/refs/heads/main.tar.gz";

/// Fallback: git clone URL.
const REGISTRY_REPO: &str = "https://github.com/librefang/librefang-registry.git";

/// Prefix inside the tarball (GitHub convention: `{repo}-{branch}/`).
const TARBALL_PREFIX: &str = "librefang-registry-main/";

/// How long (in seconds) before we re-download the registry.
const CACHE_MAX_AGE_SECS: u64 = 24 * 60 * 60; // 24 hours

/// Content directories to sync from the registry.
const SYNC_DIRS: &[(&str, &str)] = &[
    ("agents", "agent.toml"),
    ("hands", "HAND.toml"),
    ("skills", "SKILL.md"),
    ("integrations", ""), // flat .toml files
    ("providers", ""),    // flat .toml files
    ("plugins", "plugin.toml"),
];

/// Sync all content from the registry to the local librefang home directory.
///
/// Downloads the registry tarball via HTTP, extracts it, then copies items
/// that don't already exist on disk (preserves user customization).
/// Falls back to `git clone --depth 1` if the HTTP download fails.
pub fn sync_registry(home_dir: &Path) {
    let registry_cache = home_dir.join("registry");

    if !should_refresh(&registry_cache) {
        tracing::debug!("Registry cache is fresh, skipping download");
    } else if let Err(e) = download_and_extract(&registry_cache) {
        tracing::warn!("HTTP registry download failed: {e} — trying git fallback");
        if let Err(e2) = git_clone_fallback(&registry_cache) {
            tracing::warn!("Git fallback also failed: {e2}");
            // If registry_cache doesn't exist at all, nothing to sync
            if !registry_cache.exists() {
                return;
            }
        }
    }

    for &(dir_name, manifest_file) in SYNC_DIRS {
        let src_dir = registry_cache.join(dir_name);
        if !src_dir.exists() {
            continue;
        }
        let dest_dir = home_dir.join(dir_name);

        if manifest_file.is_empty() {
            sync_flat_files(&src_dir, &dest_dir, dir_name);
        } else {
            sync_subdirs(&src_dir, &dest_dir, manifest_file, dir_name);
        }
    }

    // Sync root-level files (aliases.toml, schema.toml)
    for name in &["aliases.toml", "schema.toml"] {
        let src = registry_cache.join(name);
        let dest = home_dir.join(name);
        if src.exists() && !dest.exists() {
            let _ = std::fs::copy(&src, &dest);
        }
    }
}

/// Check whether we should re-download the registry.
///
/// Returns `false` if the cache exists and the marker file is younger than
/// [`CACHE_MAX_AGE_SECS`].
fn should_refresh(registry_cache: &Path) -> bool {
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
    age.as_secs() > CACHE_MAX_AGE_SECS
}

/// Touch (create/update) the sync marker file.
fn touch_marker(registry_cache: &Path) {
    let marker = registry_cache.join(".sync_marker");
    let _ = std::fs::create_dir_all(registry_cache);
    let _ = std::fs::write(&marker, "");
}

/// Download the tarball via HTTP and extract it into `registry_cache`.
fn download_and_extract(registry_cache: &Path) -> Result<(), Box<dyn std::error::Error>> {
    tracing::info!("Downloading registry from {REGISTRY_TARBALL_URL}");

    let resp = ureq::get(REGISTRY_TARBALL_URL).call()?;
    let reader = resp.into_reader();

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
        let mut entry = entry?;
        let path = entry.path()?;
        let path_str = path.to_string_lossy();

        // Strip the tarball prefix
        let relative = match path_str.strip_prefix(TARBALL_PREFIX) {
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
fn git_clone_fallback(registry_cache: &Path) -> Result<(), Box<dyn std::error::Error>> {
    tracing::info!("Attempting git clone fallback");

    if registry_cache.join(".git").exists() {
        // Already a git repo — try pull
        let status = Command::new("git")
            .args(["pull", "--ff-only", "-q"])
            .current_dir(registry_cache)
            .status()?;
        if !status.success() {
            return Err(format!("git pull exited with {status}").into());
        }
    } else {
        // Clean slate
        if registry_cache.exists() {
            std::fs::remove_dir_all(registry_cache)?;
        }
        let status = Command::new("git")
            .args([
                "clone",
                "--depth",
                "1",
                "-q",
                REGISTRY_REPO,
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
    std::env::var("LIBREFANG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::home_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join(".librefang")
        })
}

pub fn needs_sync(home_dir: &Path) -> bool {
    !home_dir.join("providers").exists()
        || !home_dir.join("hands").exists()
        || !home_dir.join("agents").exists()
        || !home_dir.join("skills").exists()
        || !home_dir.join("integrations").exists()
}

/// Sync flat .toml files (e.g. integrations/, providers/).
fn sync_flat_files(src_dir: &Path, dest_dir: &Path, label: &str) {
    let entries = match std::fs::read_dir(src_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let mut synced = 0;
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
            skipped += 1;
            continue;
        }

        if std::fs::create_dir_all(dest_dir).is_ok() && std::fs::copy(&path, &dest_file).is_ok() {
            synced += 1;
        }
    }

    if synced > 0 || skipped > 0 {
        tracing::info!("{label} synced ({synced} new, {skipped} existing)");
    }
}

/// Sync subdirectory-based content (e.g. agents/, hands/, skills/, plugins/).
fn sync_subdirs(src_dir: &Path, dest_dir: &Path, manifest_file: &str, label: &str) {
    let entries = match std::fs::read_dir(src_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let mut synced = 0;
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
            skipped += 1;
            continue;
        }

        if copy_dir_recursive(&path, &item_dest).is_ok() {
            synced += 1;
        }
    }

    if synced > 0 || skipped > 0 {
        tracing::info!("{label} synced ({synced} new, {skipped} existing)");
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
    use super::needs_sync;

    #[test]
    fn test_needs_sync_when_agents_dir_missing() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("providers")).unwrap();
        std::fs::create_dir_all(tmp.path().join("hands")).unwrap();

        assert!(needs_sync(tmp.path()));
    }

    #[test]
    fn test_needs_sync_when_critical_dirs_exist() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("providers")).unwrap();
        std::fs::create_dir_all(tmp.path().join("hands")).unwrap();
        std::fs::create_dir_all(tmp.path().join("agents")).unwrap();
        std::fs::create_dir_all(tmp.path().join("skills")).unwrap();
        std::fs::create_dir_all(tmp.path().join("integrations")).unwrap();

        assert!(!needs_sync(tmp.path()));
    }

    #[test]
    fn test_should_refresh_no_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().join("registry");
        std::fs::create_dir_all(&cache).unwrap();
        assert!(super::should_refresh(&cache));
    }

    #[test]
    fn test_should_refresh_fresh_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().join("registry");
        std::fs::create_dir_all(&cache).unwrap();
        super::touch_marker(&cache);
        assert!(!super::should_refresh(&cache));
    }
}
