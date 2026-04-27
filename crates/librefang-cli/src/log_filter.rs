//! Reloadable per-layer `EnvFilter` for the daemon's tracing stack.
//!
//! The daemon installs an `EnvFilter` as a *per-layer* filter (so the OTel
//! exporter sees the full span tree while stderr stays terse — see the
//! comment in `init_tracing_stderr`). `tracing_subscriber::reload::Layer`
//! could in principle wrap that filter, but its `Handle` carries the
//! enclosing subscriber type as a generic parameter, and the daemon's
//! subscriber stack (`Registry` + OTel reload slot + fmt layer) bakes that
//! into a `Layered<…>` chain that's both verbose and brittle to keep in a
//! `OnceLock` signature.
//!
//! Instead we hand-roll a tiny [`ReloadableEnvFilter`] backed by an
//! `ArcSwap<EnvFilter>` and forward every [`Filter`] hook to the currently
//! loaded inner filter. Hot-reload swaps the inner filter and calls
//! [`tracing_core::callsite::rebuild_interest_cache`] so the per-callsite
//! `Interest` cache and the global max-level hint are recomputed against
//! the new directive — without that, callsites whose `Interest` was
//! resolved to `Always`/`Never` under the old filter would never re-ask
//! the new one.

use arc_swap::ArcSwap;
use std::sync::{Arc, OnceLock};
use tracing::level_filters::LevelFilter;
use tracing::subscriber::Interest;
use tracing::{Event, Metadata, Subscriber};
use tracing_subscriber::layer::{Context, Filter};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::EnvFilter;

/// Process-global slot for the live filter. Set the first time
/// [`ReloadableEnvFilter::install`] runs; subsequent installs reuse the
/// existing slot (the new `initial` filter is dropped) — `OnceLock` makes
/// this race-free, and the daemon initialises tracing exactly once anyway.
static LIVE_FILTER: OnceLock<Arc<ArcSwap<EnvFilter>>> = OnceLock::new();

/// Baseline directives to reapply on every reload.
///
/// The boot-time tracing init layers per-target overrides on top of the
/// user's level (e.g. `librefang_kernel=warn` to silence kernel chatter from
/// one-shot CLI commands). Without this slot, `reload_log_level("debug")`
/// would call `EnvFilter::try_new("debug")` and silently drop those
/// overrides — a dashboard "give me debug" toggle would suddenly flood the
/// operator with kernel/runtime DEBUG noise that boot had specifically
/// masked. Set once during `install_with_baseline` and reapplied on every
/// subsequent `reload_log_level` so reload behaviour matches boot behaviour
/// for everything except the level the user actually edited.
static BASELINE_DIRECTIVES: OnceLock<Vec<String>> = OnceLock::new();

/// Per-layer filter whose inner `EnvFilter` can be replaced at runtime via
/// [`reload_log_level`].
#[derive(Clone)]
pub struct ReloadableEnvFilter {
    inner: Arc<ArcSwap<EnvFilter>>,
}

impl ReloadableEnvFilter {
    /// Install `initial` as the live filter and return a wrapper to hand to
    /// `Layer::with_filter`. Subsequent calls reuse the existing slot — the
    /// new `initial` is dropped, so callers that re-init tracing in the
    /// same process get a stable handle (test harnesses, mostly).
    pub fn install(initial: EnvFilter) -> Self {
        let cell = LIVE_FILTER.get_or_init(|| Arc::new(ArcSwap::from_pointee(initial)));
        Self {
            inner: Arc::clone(cell),
        }
    }

    /// Install `initial` as the live filter and remember `baseline` directives
    /// so [`reload_log_level`] can reapply them after every swap.
    ///
    /// Use this from the boot-time tracing init when you've layered per-target
    /// overrides on top of the user's level (see [`BASELINE_DIRECTIVES`]).
    /// `baseline` entries are stored as strings and reparsed on each reload —
    /// the parse cost is paid once per dashboard edit, which is fine.
    /// Subsequent calls reuse the existing slots (both filter and baseline);
    /// the second `baseline` is dropped to keep the in-memory state stable for
    /// re-init scenarios (test harnesses, embedded relaunches).
    pub fn install_with_baseline(initial: EnvFilter, baseline: Vec<String>) -> Self {
        let _ = BASELINE_DIRECTIVES.set(baseline);
        Self::install(initial)
    }
}

impl<S> Filter<S> for ReloadableEnvFilter
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn enabled(&self, meta: &Metadata<'_>, cx: &Context<'_, S>) -> bool {
        Filter::<S>::enabled(self.inner.load().as_ref(), meta, cx)
    }

    fn callsite_enabled(&self, meta: &'static Metadata<'static>) -> Interest {
        Filter::<S>::callsite_enabled(self.inner.load().as_ref(), meta)
    }

    fn max_level_hint(&self) -> Option<LevelFilter> {
        Filter::<S>::max_level_hint(self.inner.load().as_ref())
    }

    fn event_enabled(&self, event: &Event<'_>, cx: &Context<'_, S>) -> bool {
        Filter::<S>::event_enabled(self.inner.load().as_ref(), event, cx)
    }

    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        id: &tracing::Id,
        ctx: Context<'_, S>,
    ) {
        Filter::<S>::on_new_span(self.inner.load().as_ref(), attrs, id, ctx);
    }

    fn on_record(&self, id: &tracing::Id, values: &tracing::span::Record<'_>, ctx: Context<'_, S>) {
        Filter::<S>::on_record(self.inner.load().as_ref(), id, values, ctx);
    }

    fn on_enter(&self, id: &tracing::Id, ctx: Context<'_, S>) {
        Filter::<S>::on_enter(self.inner.load().as_ref(), id, ctx);
    }

    fn on_exit(&self, id: &tracing::Id, ctx: Context<'_, S>) {
        Filter::<S>::on_exit(self.inner.load().as_ref(), id, ctx);
    }

    fn on_close(&self, id: tracing::Id, ctx: Context<'_, S>) {
        Filter::<S>::on_close(self.inner.load().as_ref(), id, ctx);
    }
}

/// Replace the live `EnvFilter` with one parsed from `directive` and
/// invalidate the callsite `Interest` cache.
///
/// Reapplies any baseline directives stored via
/// [`ReloadableEnvFilter::install_with_baseline`] so per-target overrides
/// from boot (e.g. `librefang_kernel=warn`) survive a dashboard
/// `log_level` edit. Without this, swapping in a fresh `EnvFilter` from
/// just the directive string would silently drop those overrides — boot
/// would mask kernel chatter while reload would unmask it, giving two
/// different log experiences for "the same" `log_level` value.
///
/// Returns `Err` when the filter slot has not been installed (no daemon
/// tracing init has run) or when `directive` fails to parse. A baseline
/// directive that fails to reparse is logged at warn and skipped — that
/// would only happen if someone changed boot-time directive syntax to
/// something invalid, in which case the new filter is still better than
/// no reload at all.
pub fn reload_log_level(directive: &str) -> Result<(), String> {
    let cell = LIVE_FILTER
        .get()
        .ok_or_else(|| "log filter not installed".to_string())?;
    let mut new_filter = EnvFilter::try_new(directive)
        .map_err(|e| format!("invalid log directive {directive:?}: {e}"))?;
    if let Some(baseline) = BASELINE_DIRECTIVES.get() {
        for d in baseline {
            match d.parse() {
                Ok(parsed) => new_filter = new_filter.add_directive(parsed),
                Err(e) => {
                    tracing::warn!(
                        directive = %d,
                        error = %e,
                        "Skipping unparseable baseline log directive on reload",
                    );
                }
            }
        }
    }
    cell.store(Arc::new(new_filter));
    tracing_core::callsite::rebuild_interest_cache();
    Ok(())
}

/// Adapter that hands [`reload_log_level`] to the kernel via the
/// [`librefang_kernel::log_reload::LogLevelReloader`] trait.
pub struct CliLogLevelReloader;

impl librefang_kernel::log_reload::LogLevelReloader for CliLogLevelReloader {
    fn reload(&self, level: &str) -> Result<(), String> {
        reload_log_level(level)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn current_max_level() -> Option<LevelFilter> {
        let cell = LIVE_FILTER
            .get()
            .expect("LIVE_FILTER must be installed before reading max_level");
        cell.load().max_level_hint()
    }

    fn current_filter_repr() -> String {
        let cell = LIVE_FILTER
            .get()
            .expect("LIVE_FILTER must be installed before reading repr");
        format!("{}", cell.load())
    }

    /// All reload assertions live in one test because `LIVE_FILTER` and
    /// `BASELINE_DIRECTIVES` are process-wide `OnceLock`s — running them as
    /// separate `#[test]` fns would let cargo's parallel test harness race on
    /// the same slots, and the `OnceLock` semantics mean only the first
    /// `install*()` actually seeds them. Driving the slots through a known
    /// sequence here is both deterministic and exercises the same code path
    /// that `apply_hot_actions_inner` triggers in production.
    ///
    /// We use `install_with_baseline` because it's the strict superset:
    /// asserting baseline survival also implicitly covers the simpler
    /// `install` path (the filter swap and Err-handling logic are shared).
    #[test]
    fn install_then_reload_swaps_inner_filter_and_keeps_baseline() {
        // Idempotent installer — we don't care whether some other test
        // already seeded the slot; we just need a wrapper to hold and a
        // valid starting point before our first `reload_log_level`. The
        // baseline mirrors the per-target overrides `init_tracing_stderr`
        // applies on the daemon path so the assertions below match prod.
        let _filter = ReloadableEnvFilter::install_with_baseline(
            EnvFilter::new("warn"),
            vec![
                "librefang_kernel=warn".to_string(),
                "librefang_runtime=warn".to_string(),
            ],
        );

        // Raising the level above the baseline (debug/trace > warn) — the
        // overall max_level_hint is dominated by the user-requested level
        // and the baseline doesn't move it.
        reload_log_level("debug").expect("debug reload");
        assert_eq!(current_max_level(), Some(LevelFilter::DEBUG));

        reload_log_level("trace").expect("trace reload");
        assert_eq!(current_max_level(), Some(LevelFilter::TRACE));

        // Baseline survival check (regression for Codex P2-1 #3200): even
        // though the user reloaded to `trace`, the kernel/runtime overrides
        // installed at boot must still be present in the live filter, or
        // the dashboard "give me trace" toggle would silently flood with
        // noise that boot had specifically masked.
        let repr = current_filter_repr();
        assert!(
            repr.contains("librefang_kernel=warn"),
            "baseline directive lost after reload: {repr}"
        );
        assert!(
            repr.contains("librefang_runtime=warn"),
            "baseline directive lost after reload: {repr}"
        );

        // Lowering below the baseline — `error < warn`, but the baseline
        // pins kernel/runtime at WARN, so the global max_level_hint is
        // WARN (not ERROR). This is the *positive* baseline-presence test:
        // if reload had wiped the baseline, max_level would collapse to
        // ERROR. Asserting WARN here proves the baseline is being applied.
        reload_log_level("error").expect("error reload");
        assert_eq!(
            current_max_level(),
            Some(LevelFilter::WARN),
            "baseline (warn) must dominate when user level (error) is lower"
        );

        // Invalid directives surface as `Err` and must leave the live
        // filter untouched — otherwise a typo from the dashboard would
        // silently disable logging until the next valid reload.
        let err = reload_log_level("foo=bogus").expect_err("must reject");
        assert!(
            err.contains("invalid log directive"),
            "error message changed shape: {err}"
        );
        assert_eq!(
            current_max_level(),
            Some(LevelFilter::WARN),
            "failed reload must not mutate the live filter"
        );
        assert!(
            current_filter_repr().contains("librefang_kernel=warn"),
            "failed reload must not drop baseline either"
        );
    }
}
