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
/// [`ReloadableEnvFilter::install`] runs; subsequent installs would race a
/// re-init of the tracing subscriber, which we don't support.
static LIVE_FILTER: OnceLock<Arc<ArcSwap<EnvFilter>>> = OnceLock::new();

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
/// Returns `Err` when the filter slot has not been installed (no daemon
/// tracing init has run) or when `directive` fails to parse.
pub fn reload_log_level(directive: &str) -> Result<(), String> {
    let cell = LIVE_FILTER
        .get()
        .ok_or_else(|| "log filter not installed".to_string())?;
    let new_filter = EnvFilter::try_new(directive)
        .map_err(|e| format!("invalid log directive {directive:?}: {e}"))?;
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
