//! Resolve the agent-types directories used on both sides of a registry
//! sync: the *source* (inside a `librefang-registry` checkout) and the
//! *destination* (this installation's own live agent-type store).
//!
//! [`librefang/librefang-registry`](https://github.com/librefang/librefang-registry)
//! is renaming its `agents/` directory to `agent-types/` (upstream naming
//! cleanup). We don't control when that lands — it can happen any day — so
//! every site that resolves this directory has to work with both names
//! simultaneously, with no window where a fresh sync comes up empty.
//!
//! The source directory is read from three places — the runtime's post-sync
//! fan-out, the hands registry's `base = "<template>"` resolution, and the
//! kernel router's hand scan. Each used to open-code
//! `registry_cache.join("agents")` plus an existence check and skip its whole
//! block on a miss, so a checkout that arrived without the directory (or that
//! had already been renamed) disabled agent types with no error, no log, and
//! no way to tell that state apart from "the registry ships no agent types"
//! (#7767). [`resolve_agent_types_dir`] is the single place that fixes this:
//! prefer the canonical new name, fall back to the legacy name with a
//! warning, and log loudly (not silently return "not found") when neither
//! directory exists — that last case is exactly the failure mode that used to
//! vanish without a trace.
//!
//! Both signals are edge-triggered. The router resolves this directory on
//! every inbound message dispatch, so an unconditional log would flood rather
//! than inform: a path is logged when it first fails to resolve (or first
//! falls back to the legacy name) and becomes eligible again only after it has
//! resolved cleanly, so a genuine recovery-then-regression is still reported.
//!
//! [`installed_agent_types_dir`] resolves the other side: where THIS
//! installation keeps its own live agent-type manifests
//! (`~/.librefang/agent-types/`) — populated by a registry sync, by
//! `POST /api/agent-types`, by the `agent_type_create` tool, and by
//! `save-as-agent-type`, and read back by every agent_type-spawn resolver
//! and by `GET /api/agent-types`. It used to be `~/.librefang/templates/`,
//! which was wrong on two counts: that directory is a *different* domain
//! (starter/skeleton TOML scaffolding for agent/hand/skill/channel/workflow
//! authoring, #7758) that agent-types writes were silently contaminating,
//! and the registry's copy of it was landing agent-type *instances* inside
//! `~/.librefang/workspaces/agents/` — the deployed-agents directory — so a
//! fresh install's registry sync manufactured dozens of agents the operator
//! never asked to run. [`warn_on_agent_type_like_files_in_templates_dir`]
//! is the read-only, boot-time half of the fix for installs that already
//! have agent-type-shaped files sitting in the old, wrong location: it
//! warns and names the files, but never moves or deletes anything — moving
//! files between two unrelated domains on the operator's behalf is exactly
//! what this fix is trying to stop doing.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Canonical (post-rename) name of the agent-types directory inside a
/// registry checkout.
pub const AGENT_TYPES_DIR_NAME: &str = "agent-types";

/// Legacy name, still served by the registry until the rename lands upstream.
pub const LEGACY_AGENTS_DIR_NAME: &str = "agents";

/// Historical alias for [`LEGACY_AGENTS_DIR_NAME`], kept so call sites that
/// predate the rename keep compiling. New code should name the directory it
/// actually means: [`AGENT_TYPES_DIR_NAME`] for the canonical one.
pub const AGENT_TEMPLATES_DIR_NAME: &str = LEGACY_AGENTS_DIR_NAME;

/// Registry checkouts already reported at error level for having neither
/// candidate directory. Keyed on the checkout root so both candidate names
/// collapse to one report.
static REPORTED_MISSING: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());

/// Registry checkouts already warned about for still serving the legacy
/// directory name. Same edge-triggered discipline as [`REPORTED_MISSING`],
/// for the same reason: the router hits this path per dispatch.
static REPORTED_LEGACY: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());

/// Resolve the agent-types directory within a registry checkout.
///
/// `registry_cache` is the root of the checkout (e.g. `~/.librefang/registry`
/// or the pinned test fixture directory) — the directory that directly
/// contains `providers/`, `hands/`, `agent-types/` / `agents/`, etc. It is
/// the registry checkout itself, not the LibreFang home directory.
///
/// Resolution order:
/// 1. `{registry_cache}/agent-types/` — the canonical name. Used silently
///    when present, including when the legacy directory also exists (the new
///    name always wins so a transitional registry state where upstream ships
///    both doesn't accidentally pin callers to the old one).
/// 2. `{registry_cache}/agents/` — the legacy name. Used as a fallback with a
///    `tracing::warn!` the first time a given checkout falls back, so
///    operators get a signal that the registry they're syncing from hasn't
///    renamed yet without flooding the log.
/// 3. Neither exists — `tracing::error!` and `None`. This is the case that
///    used to be indistinguishable from "the registry genuinely ships no
///    agent types right now": callers must be able to tell "nothing to sync"
///    apart from "the sync's fan-out silently skipped its block", and this
///    log line is that signal.
pub fn resolve_agent_types_dir(registry_cache: &Path) -> Option<PathBuf> {
    let canonical = registry_cache.join(AGENT_TYPES_DIR_NAME);
    if canonical.is_dir() {
        clear_missing_report(registry_cache);
        clear_legacy_report(registry_cache);
        return Some(canonical);
    }

    let legacy = registry_cache.join(LEGACY_AGENTS_DIR_NAME);
    if legacy.is_dir() {
        clear_missing_report(registry_cache);
        report_legacy_fallback(registry_cache, &legacy);
        return Some(legacy);
    }

    report_missing(registry_cache);
    None
}

/// Historical name for [`resolve_agent_types_dir`], kept because the runtime
/// fan-out, the hands registry, and the kernel router all reference it. It
/// resolves the same directory: agent types shipped by a registry checkout,
/// under whichever of the two names that checkout currently uses.
pub fn resolve_agent_templates_dir(registry_root: &Path) -> Option<PathBuf> {
    resolve_agent_types_dir(registry_root)
}

/// Record a miss and log it if this checkout was not already known to be
/// missing both candidate directories.
///
/// Returns whether the miss was reported, which is what the unit tests below
/// assert on: the set membership is the record that the error line was
/// emitted.
fn report_missing(registry_cache: &Path) -> bool {
    let newly_missing = match REPORTED_MISSING.lock() {
        Ok(mut reported) => reported.insert(registry_cache.to_path_buf()),
        // A poisoned lock means another thread panicked mid-report; logging again is strictly better than staying silent about a degraded registry.
        Err(poisoned) => poisoned.into_inner().insert(registry_cache.to_path_buf()),
    };
    if newly_missing {
        let registry_cache_display = registry_cache.display();
        tracing::error!(
            registry_cache = %registry_cache_display,
            tried_canonical = %registry_cache.join(AGENT_TYPES_DIR_NAME).display(),
            tried_legacy = %registry_cache.join(LEGACY_AGENTS_DIR_NAME).display(),
            "registry checkout has neither '{AGENT_TYPES_DIR_NAME}/' nor legacy '{LEGACY_AGENTS_DIR_NAME}/' — \
             no agent types are available from this checkout, and hands declaring `base = \"<template>\"` \
             will not resolve. This is not necessarily an empty registry: verify the sync actually ran \
             and populated {registry_cache_display}, or re-run `librefang init` to re-sync it"
        );
    }
    newly_missing
}

/// Forget a previously reported miss so a later regression is reported again.
fn clear_missing_report(registry_cache: &Path) {
    match REPORTED_MISSING.lock() {
        Ok(mut reported) => reported.remove(registry_cache),
        Err(poisoned) => poisoned.into_inner().remove(registry_cache),
    };
}

/// Warn once per checkout that the legacy directory name is still in use.
fn report_legacy_fallback(registry_cache: &Path, legacy: &Path) -> bool {
    let newly_reported = match REPORTED_LEGACY.lock() {
        Ok(mut reported) => reported.insert(registry_cache.to_path_buf()),
        Err(poisoned) => poisoned.into_inner().insert(registry_cache.to_path_buf()),
    };
    if newly_reported {
        tracing::warn!(
            path = %legacy.display(),
            "registry checkout still serves the legacy '{LEGACY_AGENTS_DIR_NAME}/' directory name; \
             the canonical name is '{AGENT_TYPES_DIR_NAME}/' — this fallback exists for the \
             registry's in-progress rename and will keep working until it completes"
        );
    }
    newly_reported
}

/// Forget a previously warned legacy fallback so a regression warns again.
fn clear_legacy_report(registry_cache: &Path) {
    match REPORTED_LEGACY.lock() {
        Ok(mut reported) => reported.remove(registry_cache),
        Err(poisoned) => poisoned.into_inner().remove(registry_cache),
    };
}

/// Directory name for this installation's own agent-type manifest store.
/// Deliberately the same string as [`AGENT_TYPES_DIR_NAME`] — it names the
/// same *concept* (agent-type manifests) on the destination side of a sync,
/// just rooted at `home_dir` (`~/.librefang/`) instead of a registry
/// checkout.
pub const INSTALLED_AGENT_TYPES_DIR_NAME: &str = AGENT_TYPES_DIR_NAME;

/// Resolve this installation's canonical agent-type manifest directory
/// (`~/.librefang/agent-types/`).
///
/// This is the single place every reader and writer of agent-type manifests
/// should call instead of hand-rolling `home_dir.join("agent-types")` (or,
/// worse, `home_dir.join("templates")` / `home_dir.join("workspaces").join("agents")`
/// — both wrong locations this directory replaces). See the module doc
/// comment for why those two were wrong.
///
/// Agent-type manifests are stored flat: `agent-types/<name>.toml` — one
/// file per type, matching what `GET/POST/PUT/DELETE /api/agent-types`, the
/// `agent_type_create` tool, and every ephemeral/persistent spawn resolver
/// already expect. This is deliberately NOT the registry checkout's own
/// directory-per-type layout (`agent-types/<name>/agent.toml`, see
/// [`resolve_agent_types_dir`]) — a registry sync flattens on copy.
pub fn installed_agent_types_dir(home_dir: &Path) -> PathBuf {
    home_dir.join(INSTALLED_AGENT_TYPES_DIR_NAME)
}

/// Boot-time diagnostic (never a mutation): warn when
/// `{home_dir}/templates/` — the unrelated starter/skeleton TOML directory
/// for agent/hand/skill/channel/workflow authoring (#7758) — contains files
/// that look like agent-type manifests rather than plain skeletons.
///
/// A now-fixed bug used to write operator- and tool-created agent-types
/// into `templates/` instead of the canonical [`installed_agent_types_dir`].
/// This function does not move, copy, or delete anything — `templates/` and
/// `agent-types/` are different domains, and silently relocating files
/// between them on the operator's behalf is exactly the mistake this fix is
/// correcting. It only surfaces a `tracing::warn!` naming the suspect files
/// and the canonical directory, so the operator can decide.
///
/// Heuristic: a legitimate `templates/` skeleton (e.g. a minimal
/// `name = "…"` / `module = "…"` starter) does not declare a `[model]`
/// table — every agent-type manifest written by `POST /api/agent-types`,
/// the `agent_type_create` tool, or `save-as-agent-type` does, because all
/// three serialize a full `AgentManifest`, which always carries a `model`
/// field. A `[model]` table is therefore a reliable (if imperfect) signal
/// that a `templates/*.toml` file belongs in `agent-types/` instead.
pub fn warn_on_agent_type_like_files_in_templates_dir(home_dir: &Path) {
    let templates_dir = home_dir.join("templates");
    let Ok(entries) = std::fs::read_dir(&templates_dir) else {
        return;
    };

    let mut suspects: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("toml") {
                return None;
            }
            let content = std::fs::read_to_string(&path).ok()?;
            let value: toml::Value = toml::from_str(&content).ok()?;
            value
                .get("model")
                .and_then(|m| m.as_table())
                .map(|_| path.display().to_string())
        })
        .collect();

    if suspects.is_empty() {
        return;
    }
    suspects.sort();

    let canonical = installed_agent_types_dir(home_dir);
    tracing::warn!(
        files = ?suspects,
        canonical_dir = %canonical.display(),
        templates_dir = %templates_dir.display(),
        "found {} file(s) in the 'templates/' scaffold directory that look like agent-type \
         manifests (each declares a [model] table) — 'templates/' holds starter TOML skeletons \
         for agent/hand/skill/channel/workflow authoring, not agent-type storage; the canonical \
         location for agent-types is 'agent-types/'. This is a diagnostic only: nothing was \
         moved or deleted. If these files are meant to be agent-types, move them yourself.",
        suspects.len()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::sync::Arc;
    use tracing_subscriber::fmt::MakeWriter;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    /// Minimal in-memory writer so tests can assert on emitted log lines
    /// without touching stdout/stderr. Mirrors the pattern already used in
    /// `librefang-runtime/tests/instrument_span_fields.rs`.
    #[derive(Clone, Default)]
    struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

    impl io::Write for CaptureWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for CaptureWriter {
        type Writer = CaptureWriter;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// Run `f` under a fresh tracing subscriber and return everything it logged.
    fn capture_logs(f: impl FnOnce()) -> String {
        let writer = CaptureWriter::default();
        let buf = writer.0.clone();
        let layer = tracing_subscriber::fmt::layer()
            .with_writer(writer)
            .with_ansi(false)
            .with_target(false);
        let _guard = tracing_subscriber::registry().with(layer).set_default();
        f();
        let captured = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        captured
    }

    /// Whether `registry_cache` is currently recorded as a reported miss.
    ///
    /// Reaching into the private static is deliberate: it is the same
    /// condition that gates the `tracing::error!` call, so asserting on it
    /// asserts the error-level signal fired.
    fn was_reported(registry_cache: &Path) -> bool {
        match REPORTED_MISSING.lock() {
            Ok(reported) => reported.contains(registry_cache),
            Err(poisoned) => poisoned.into_inner().contains(registry_cache),
        }
    }

    #[test]
    fn only_legacy_agents_dir_resolves_and_warns() {
        let tmp = tempfile::tempdir().unwrap();
        let legacy = tmp.path().join("agents");
        std::fs::create_dir_all(&legacy).unwrap();

        let logs = capture_logs(|| {
            let resolved = resolve_agent_types_dir(tmp.path());
            assert_eq!(resolved, Some(legacy.clone()), "must resolve legacy dir");
        });

        assert!(
            logs.contains("legacy") && logs.contains("agent-types"),
            "expected a warning naming both the legacy fallback and the canonical name, got: {logs}"
        );
    }

    #[test]
    fn only_agent_types_dir_resolves_without_warning() {
        let tmp = tempfile::tempdir().unwrap();
        let canonical = tmp.path().join("agent-types");
        std::fs::create_dir_all(&canonical).unwrap();

        let logs = capture_logs(|| {
            let resolved = resolve_agent_types_dir(tmp.path());
            assert_eq!(
                resolved,
                Some(canonical.clone()),
                "must resolve canonical dir"
            );
        });

        assert!(
            logs.is_empty(),
            "canonical-only resolution must not warn, got: {logs}"
        );
    }

    #[test]
    fn both_present_prefers_agent_types() {
        let tmp = tempfile::tempdir().unwrap();
        let canonical = tmp.path().join("agent-types");
        let legacy = tmp.path().join("agents");
        std::fs::create_dir_all(&canonical).unwrap();
        std::fs::create_dir_all(&legacy).unwrap();

        let logs = capture_logs(|| {
            let resolved = resolve_agent_types_dir(tmp.path());
            assert_eq!(
                resolved,
                Some(canonical.clone()),
                "canonical name must win when both exist"
            );
        });

        assert!(
            logs.is_empty(),
            "no warning expected when the canonical name is present, got: {logs}"
        );
    }

    #[test]
    fn neither_present_errors_loudly() {
        let tmp = tempfile::tempdir().unwrap();

        let logs = capture_logs(|| {
            let resolved = resolve_agent_types_dir(tmp.path());
            assert_eq!(resolved, None, "must return None when neither dir exists");
        });

        assert!(
            logs.contains("ERROR"),
            "missing-both case must log at error level, got: {logs}"
        );
        assert!(
            logs.contains("agent-types") && logs.contains("agents"),
            "error must name both directories that were tried, got: {logs}"
        );
    }

    #[test]
    fn missing_agents_dir_returns_none_and_reports_at_error_level() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        assert_eq!(resolve_agent_types_dir(root), None);
        assert!(
            was_reported(root),
            "a missing agent-types directory must produce the error-level signal",
        );
    }

    #[test]
    fn repeated_misses_report_once_until_the_directory_comes_back() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let dir = root.join("agents");

        assert!(report_missing(root), "the first miss is reported");
        assert!(
            !report_missing(root),
            "a repeated miss is not reported again"
        );

        std::fs::create_dir_all(&dir).expect("create agents dir");
        assert_eq!(resolve_agent_types_dir(root), Some(dir.clone()));
        assert!(
            !was_reported(root),
            "a successful resolution clears the reported miss",
        );

        std::fs::remove_dir_all(&dir).expect("remove agents dir");
        assert_eq!(resolve_agent_types_dir(root), None);
        assert!(
            was_reported(root),
            "a regression after a recovery is reported again",
        );
    }

    #[test]
    fn present_agents_dir_resolves_without_reporting() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let dir = root.join("agents");
        std::fs::create_dir_all(&dir).expect("create agents dir");

        assert_eq!(resolve_agent_types_dir(root), Some(dir.clone()));
        assert!(
            !was_reported(root),
            "the normal case must not emit an error line",
        );
    }

    #[test]
    fn a_file_named_agents_is_treated_as_missing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let dir = root.join("agents");
        std::fs::write(&dir, b"not a directory").expect("write agents file");

        assert_eq!(resolve_agent_types_dir(root), None);
        assert!(
            was_reported(root),
            "a non-directory at the expected path is a miss, not a resolution",
        );
    }

    #[test]
    fn resolve_agent_templates_dir_is_an_alias_for_agent_types() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let canonical = tmp.path().join("agent-types");
        std::fs::create_dir_all(&canonical).expect("create agent-types dir");

        assert_eq!(
            resolve_agent_templates_dir(tmp.path()),
            resolve_agent_types_dir(tmp.path()),
            "the historical name must resolve the same directory",
        );
    }

    // ---- installed_agent_types_dir ----------------------------------

    #[test]
    fn installed_agent_types_dir_is_home_join_agent_types() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            installed_agent_types_dir(tmp.path()),
            tmp.path().join("agent-types")
        );
    }

    // ---- warn_on_agent_type_like_files_in_templates_dir --------------

    #[test]
    fn templates_dir_missing_is_a_silent_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let logs = capture_logs(|| {
            warn_on_agent_type_like_files_in_templates_dir(tmp.path());
        });
        assert!(
            logs.is_empty(),
            "no templates/ dir at all must not warn, got: {logs}"
        );
    }

    #[test]
    fn plain_skeleton_without_model_table_does_not_warn() {
        let tmp = tempfile::tempdir().unwrap();
        let templates_dir = tmp.path().join("templates");
        std::fs::create_dir_all(&templates_dir).unwrap();
        std::fs::write(
            templates_dir.join("tooled-mission.toml"),
            "name = \"tooled-mission\"\nmodule = \"builtin:chat\"\n",
        )
        .unwrap();

        let logs = capture_logs(|| {
            warn_on_agent_type_like_files_in_templates_dir(tmp.path());
        });
        assert!(
            logs.is_empty(),
            "a skeleton with no [model] table must not be flagged, got: {logs}"
        );
    }

    #[test]
    fn file_with_model_table_warns_and_names_it_without_moving_anything() {
        let tmp = tempfile::tempdir().unwrap();
        let templates_dir = tmp.path().join("templates");
        std::fs::create_dir_all(&templates_dir).unwrap();
        let misplaced = templates_dir.join("misplaced-agent-type.toml");
        std::fs::write(
            &misplaced,
            "name = \"misplaced-agent-type\"\ndescription = \"oops\"\n\n[model]\nprovider = \"default\"\nmodel = \"default\"\n",
        )
        .unwrap();

        let logs = capture_logs(|| {
            warn_on_agent_type_like_files_in_templates_dir(tmp.path());
        });

        assert!(
            logs.contains("WARN") && logs.contains("misplaced-agent-type.toml"),
            "must warn and name the suspect file, got: {logs}"
        );
        assert!(
            logs.contains("agent-types"),
            "warning must point at the canonical directory, got: {logs}"
        );
        assert!(
            misplaced.exists(),
            "the file must never be moved or deleted — diagnostic only"
        );
        assert!(
            !tmp.path().join("agent-types").exists(),
            "no agent-types/ directory should be created as a side effect"
        );
    }
}
