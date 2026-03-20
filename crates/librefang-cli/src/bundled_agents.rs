//! Registry-based content sync.
//!
//! All content definitions (agents, hands, skills, integrations, providers,
//! plugins) are maintained in the librefang-registry repo.
//! `sync_registry` clones or pulls the registry and copies content
//! to `~/.librefang/`.

use std::path::Path;
use std::process::Command;

const REGISTRY_REPO: &str = "https://github.com/librefang/librefang-registry.git";

/// Content directories to sync from the registry.
const SYNC_DIRS: &[(&str, &str)] = &[
    ("agents", "agent.toml"),
    ("hands", "HAND.toml"),
    ("skills", "SKILL.md"),
    ("integrations", ""), // flat .toml files, not subdirectories
    ("providers", ""),    // flat .toml files
    ("plugins", "plugin.toml"),
];

/// Sync all content from the registry to the local librefang home directory.
///
/// Clones the registry (shallow) on first run, pulls on subsequent runs.
/// Only copies items that don't already exist on disk (preserves user customization).
pub fn sync_registry_agents(home_dir: &Path) {
    let registry_cache = home_dir.join("registry");

    // Clone or pull the registry
    if registry_cache.join(".git").exists() {
        let status = Command::new("git")
            .args(["pull", "--ff-only", "-q"])
            .current_dir(&registry_cache)
            .status();
        if let Err(e) = status {
            eprintln!("  ⚠ Failed to pull registry: {e}");
        }
    } else {
        let status = Command::new("git")
            .args([
                "clone",
                "--depth",
                "1",
                "-q",
                REGISTRY_REPO,
                &registry_cache.display().to_string(),
            ])
            .status();
        match status {
            Ok(s) if s.success() => {}
            Ok(s) => eprintln!("  ⚠ git clone exited with {s}"),
            Err(e) => {
                eprintln!("  ⚠ Failed to clone registry (is git installed?): {e}");
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
            // Flat directory: sync individual .toml files
            sync_flat_files(&src_dir, &dest_dir, dir_name);
        } else {
            // Subdirectory-based: each item is a folder with a manifest file
            sync_subdirs(&src_dir, &dest_dir, manifest_file, dir_name);
        }
    }
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
        println!("  ✔ {label} synced ({synced} new, {skipped} existing)");
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

        // Copy entire subdirectory (may contain README, docs, etc.)
        if copy_dir_recursive(&path, &item_dest).is_ok() {
            synced += 1;
        }
    }

    if synced > 0 || skipped > 0 {
        println!("  ✔ {label} synced ({synced} new, {skipped} existing)");
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
