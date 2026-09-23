//! Shared bookkeeping for the typed sampling parameters on `CompletionRequest` (#8290).
//!
//! `top_p`, `frequency_penalty` and `presence_penalty` are typed fields, and each driver decides for itself where on its wire they go.
//! A driver whose wire has no field for one — or whose target model rejects it — drops it rather than turning a tuning preference into a 400 on every turn.
//! This module only makes those drops visible, so that "I set `top_p` and nothing changed" has an answer in the debug log.

/// Log, at `debug`, each parameter in `dropped` that the caller actually set.
///
/// Entries whose value is `None` were never requested and are skipped, so drivers can pass every parameter they do not send without filtering first.
pub(crate) fn log_dropped(
    provider: &'static str,
    model: &str,
    dropped: &[(&'static str, Option<f32>)],
) {
    for (param, value) in dropped {
        if let Some(value) = value {
            tracing::debug!(
                provider,
                model,
                param,
                value,
                "sampling parameter not accepted by this provider/model; not sent"
            );
        }
    }
}
