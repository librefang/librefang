//! Registry-based agent sync.
//!
//! Agent manifests are maintained in the librefang-registry repo.
//! `sync_registry_agents` clones or pulls the registry and copies
//! agent definitions to `~/.librefang/agents/`.

use std::path::Path;
use std::process::Command;

const REGISTRY_REPO: &str = "https://github.com/librefang/librefang-registry.git";

/// Sync agent definitions from the registry to the local agents directory.
///
/// Clones the registry (shallow) on first run, pulls on subsequent runs.
/// Only copies agents that don't already exist on disk (preserves user customization).
pub fn sync_registry_agents(home_dir: &Path) {
    let registry_cache = home_dir.join("registry");
    let agents_dir = home_dir.join("agents");

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

    // Copy agents from registry to ~/.librefang/agents/
    let registry_agents = registry_cache.join("agents");
    if !registry_agents.exists() {
        eprintln!("  ⚠ Registry cloned but agents/ directory not found");
        return;
    }

    let entries = match std::fs::read_dir(&registry_agents) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("  ⚠ Failed to read registry agents: {e}");
            return;
        }
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
        let src = path.join("agent.toml");
        if !src.exists() {
            continue;
        }

        let dest_dir = agents_dir.join(&name);
        let dest_file = dest_dir.join("agent.toml");
        if dest_file.exists() {
            skipped += 1;
            continue; // Preserve user customization
        }

        if std::fs::create_dir_all(&dest_dir).is_ok() && std::fs::copy(&src, &dest_file).is_ok() {
            synced += 1;
        }
    }

    if synced > 0 || skipped > 0 {
        println!("  ✔ Agents synced from registry ({synced} new, {skipped} existing)");
    }
}
