//! Shared resolution of paths inside a registry checkout.
//!
//! The registry's agent-templates directory is read from three places — the runtime's post-sync fan-out, the hands registry's `base = "<template>"` resolution, and the kernel router's hand scan.
//! Each used to open-code `join("agents")` plus an existence check and skip its whole block on a miss, so a checkout that arrived without the directory disabled agent templates with no error, no log, and no way to tell that state apart from "the registry ships no templates" (#7767).
//! Routing all three through one resolver keeps them from drifting apart again and gives the miss a single place to be loud.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Directory name the registry checkout uses for agent templates.
pub const AGENT_TEMPLATES_DIR_NAME: &str = "agents";

/// Missing agent-template directories already reported at error level.
///
/// The router resolves this directory on every inbound message dispatch, so an unconditional log would flood rather than inform.
/// Reporting is edge-triggered instead: a path is logged when it first fails to resolve and becomes eligible again only after it has resolved successfully, so a genuine recovery-then-regression is still reported.
static REPORTED_MISSING: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());

/// Resolve `<registry_root>/agents`, the directory holding the registry's agent templates.
///
/// `registry_root` is the registry checkout itself (`$LIBREFANG_HOME/registry`), not the LibreFang home directory.
/// Returns `None` when the directory does not exist or is not a directory, logging at error level the first time a given path misses — callers treat the miss as "no agent templates", which is a degraded state an operator has to be able to see.
pub fn resolve_agent_templates_dir(registry_root: &Path) -> Option<PathBuf> {
    let dir = registry_root.join(AGENT_TEMPLATES_DIR_NAME);
    if dir.is_dir() {
        clear_missing_report(&dir);
        return Some(dir);
    }
    report_missing(&dir);
    None
}

/// Record a miss and log it if this path was not already known to be missing.
///
/// Returns whether the miss was reported, which is what the unit tests below assert on: the set membership is the record that the error line was emitted.
fn report_missing(dir: &Path) -> bool {
    let newly_missing = match REPORTED_MISSING.lock() {
        Ok(mut reported) => reported.insert(dir.to_path_buf()),
        // A poisoned lock means another thread panicked mid-report; logging again is strictly better than staying silent about a degraded registry.
        Err(poisoned) => poisoned.into_inner().insert(dir.to_path_buf()),
    };
    if newly_missing {
        tracing::error!(
            path = %dir.display(),
            "Registry checkout has no agent-templates directory — agent templates will not be pre-installed and hands declaring `base = \"<template>\"` will not resolve. Re-run `librefang init` to re-sync the registry.",
        );
    }
    newly_missing
}

/// Forget a previously reported miss so a later regression is reported again.
fn clear_missing_report(dir: &Path) {
    match REPORTED_MISSING.lock() {
        Ok(mut reported) => reported.remove(dir),
        Err(poisoned) => poisoned.into_inner().remove(dir),
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether `dir` is currently recorded as a reported miss.
    ///
    /// Reaching into the private static is deliberate: it is the same condition that gates the `tracing::error!` call, so asserting on it asserts the error-level signal fired without pulling a subscriber into this crate's dependency tree.
    fn was_reported(dir: &Path) -> bool {
        match REPORTED_MISSING.lock() {
            Ok(reported) => reported.contains(dir),
            Err(poisoned) => poisoned.into_inner().contains(dir),
        }
    }

    #[test]
    fn missing_agents_dir_returns_none_and_reports_at_error_level() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let expected = root.join("agents");

        assert_eq!(resolve_agent_templates_dir(root), None);
        assert!(
            was_reported(&expected),
            "a missing agent-templates directory must produce the error-level signal",
        );
    }

    #[test]
    fn repeated_misses_report_once_until_the_directory_comes_back() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let dir = root.join("agents");

        assert!(report_missing(&dir), "the first miss is reported");
        assert!(
            !report_missing(&dir),
            "a repeated miss is not reported again"
        );

        std::fs::create_dir_all(&dir).expect("create agents dir");
        assert_eq!(resolve_agent_templates_dir(root), Some(dir.clone()));
        assert!(
            !was_reported(&dir),
            "a successful resolution clears the reported miss",
        );

        std::fs::remove_dir_all(&dir).expect("remove agents dir");
        assert_eq!(resolve_agent_templates_dir(root), None);
        assert!(
            was_reported(&dir),
            "a regression after a recovery is reported again",
        );
    }

    #[test]
    fn present_agents_dir_resolves_without_reporting() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let dir = root.join("agents");
        std::fs::create_dir_all(&dir).expect("create agents dir");

        assert_eq!(resolve_agent_templates_dir(root), Some(dir.clone()));
        assert!(
            !was_reported(&dir),
            "the normal case must not emit an error line",
        );
    }

    #[test]
    fn a_file_named_agents_is_treated_as_missing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let dir = root.join("agents");
        std::fs::write(&dir, b"not a directory").expect("write agents file");

        assert_eq!(resolve_agent_templates_dir(root), None);
        assert!(
            was_reported(&dir),
            "a non-directory at the expected path is a miss, not a resolution",
        );
    }
}
