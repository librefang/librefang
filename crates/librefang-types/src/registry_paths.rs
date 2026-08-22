//! Resolve the agent-templates directory inside a `librefang-registry` checkout.
//!
//! Three call sites resolve this directory, and before this module existed each
//! did its own bare `registry_cache.join("agents")` + existence check that
//! returns `None` / skips its whole block when the directory is not there:
//! `registry_sync::fanout_registry_content`, `librefang_hands::registry::resolve_agents_dir`
//! and `librefang_kernel_router::load_hand_route_candidates`.
//! None of them logs anything in that case, so "the registry genuinely ships no
//! agent templates" and "the sync produced a checkout this code cannot read" are
//! indistinguishable from the outside: a fresh install silently comes up with
//! zero preinstalled agent templates, and hands declaring `base = "<template>"`
//! silently drop out of routing, with nothing in the logs to point at why.
//! Making that case loud is the load-bearing half of this module.
//!
//! The other half is forward compatibility on the directory name.
//! [`librefang/librefang-registry`](https://github.com/librefang/librefang-registry)
//! currently serves `agents/`, and there is no announced rename — but the core
//! is converging on "agent type" as the name for this concept, and a
//! single-name-hardcoded lookup is exactly the shape that fails silently if the
//! registry layout ever moves.
//! Accepting either name costs one `is_dir` call on a path already being
//! stat-ed, so [`resolve_agent_types_dir`] takes the canonical `agent-types/`
//! when present, falls back to `agents/` with a warning, and errors when
//! neither exists.
//! If the registry never renames, the fallback branch is simply the one that
//! always runs and the warning is the only visible cost — see
//! [`resolve_agent_types_dir`] for why the warning is once-per-sync rather than
//! once-per-message.

use std::path::{Path, PathBuf};

/// Forward-compatible name of the agent-templates directory inside a registry
/// checkout, matching the core's "agent type" terminology.
/// Not currently served by `librefang/librefang-registry`; accepted first so a
/// future layout change cannot silently zero out the fan-out.
pub const AGENT_TYPES_DIR_NAME: &str = "agent-types";

/// The name `librefang/librefang-registry` serves today, and therefore the
/// branch that normally runs.
pub const LEGACY_AGENTS_DIR_NAME: &str = "agents";

/// Resolve the agent-templates directory within a registry checkout.
///
/// `registry_cache` is the root of the checkout (e.g. `~/.librefang/registry`
/// or the pinned test fixture directory) — the directory that directly
/// contains `providers/`, `hands/`, `agent-types/` / `agents/`, etc.
///
/// Resolution order:
/// 1. `{registry_cache}/agent-types/` — taken when present, including when
///    `agents/` also exists, so a registry that ever ships both during a
///    transition does not pin callers to the older layout.
/// 2. `{registry_cache}/agents/` — what the registry serves today, so this is
///    the branch that normally runs. Deliberately **not** warned about: it is
///    the current correct state, and warning here would fire on every sync for
///    every install, which is how a log line teaches operators to ignore it.
///    It logs at `debug` so the resolution is still recoverable when
///    diagnosing an empty fan-out.
/// 3. Neither exists — `tracing::error!` and `None`. This is the case that was
///    previously indistinguishable from "the registry genuinely ships no agent
///    templates right now": callers must be able to tell "nothing to sync"
///    apart from "the fan-out skipped its whole block", and this log line is
///    that signal. It is the reason this module exists.
///
/// Called once per sync / cache rebuild by each caller, never once per agent or
/// per routed message, so the log levels above do not compound.
pub fn resolve_agent_types_dir(registry_cache: &Path) -> Option<PathBuf> {
    let canonical = registry_cache.join(AGENT_TYPES_DIR_NAME);
    if canonical.is_dir() {
        return Some(canonical);
    }

    let legacy = registry_cache.join(LEGACY_AGENTS_DIR_NAME);
    if legacy.is_dir() {
        tracing::debug!(
            path = %legacy.display(),
            "registry checkout serves agent templates under '{LEGACY_AGENTS_DIR_NAME}/'"
        );
        return Some(legacy);
    }

    let registry_cache_display = registry_cache.display();
    tracing::error!(
        registry_cache = %registry_cache_display,
        tried_canonical = %canonical.display(),
        tried_legacy = %legacy.display(),
        "registry checkout has neither '{AGENT_TYPES_DIR_NAME}/' nor legacy '{LEGACY_AGENTS_DIR_NAME}/' — \
         no agent templates are available from this checkout. This is not necessarily an empty \
         registry: verify the sync actually ran and populated {registry_cache_display}"
    );
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::sync::{Arc, Mutex};
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

    /// `agents/` is what the registry serves today, so resolving it is the
    /// normal path and must stay quiet: a WARN here would fire on every sync
    /// for every install and train operators to ignore the log.
    #[test]
    fn only_legacy_agents_dir_resolves_without_warning() {
        let tmp = tempfile::tempdir().unwrap();
        let legacy = tmp.path().join("agents");
        std::fs::create_dir_all(&legacy).unwrap();

        let logs = capture_logs(|| {
            let resolved = resolve_agent_types_dir(tmp.path());
            assert_eq!(resolved, Some(legacy.clone()), "must resolve legacy dir");
        });

        assert!(
            !logs.contains("WARN") && !logs.contains("ERROR"),
            "resolving the directory name the registry currently serves must not warn, got: {logs}"
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
}
