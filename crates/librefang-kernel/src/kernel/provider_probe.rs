//! Local-provider liveness probing. Periodically reaches out to local LLM
//! servers (ollama, LM Studio, Open WebUI, …), updates their auth status
//! and discovered model list in the model catalog, and is exposed
//! method-style on `LibreFangKernel::probe_local_provider` for crates that
//! shouldn't reach into `librefang_kernel::kernel::*` directly.

use std::sync::Arc;

use tracing::{debug, info, warn};

use super::LibreFangKernel;

pub async fn probe_and_update_local_provider(
    kernel: &Arc<LibreFangKernel>,
    provider_id: &str,
    base_url: &str,
    log_offline_as_warn: bool,
) -> librefang_runtime::provider_health::ProbeResult {
    // Forward the provider's api_key (when configured) so reverse-proxy
    // frontends like Open WebUI accept the listing request. Without this,
    // the probe always 401s and the catalog flips to LocalOffline even
    // when the underlying ollama is healthy.
    let api_key = {
        let catalog = kernel.model_catalog.load();
        let env_var = catalog
            .get_provider(provider_id)
            .map(|p| p.api_key_env.clone())
            .filter(|env| !env.trim().is_empty())
            .unwrap_or_else(|| format!("{}_API_KEY", provider_id.to_uppercase().replace('-', "_")));
        std::env::var(env_var).ok().filter(|v| !v.trim().is_empty())
    };
    let result = librefang_runtime::provider_health::probe_provider(
        provider_id,
        base_url,
        api_key.as_deref(),
    )
    .await;
    if result.reachable {
        info!(
            provider = %provider_id,
            models = result.discovered_models.len(),
            latency_ms = result.latency_ms,
            "Local provider online"
        );
        // Pre-compute the merged info outside the RCU closure so it's not
        // recomputed on retry (the closure may run multiple times if a
        // concurrent updater wins the CAS).
        let merged_info: Option<Vec<librefang_runtime::provider_health::DiscoveredModelInfo>> =
            if result.discovered_models.is_empty() {
                None
            } else if result.discovered_model_info.is_empty() {
                Some(
                    result
                        .discovered_models
                        .iter()
                        .map(
                            |name| librefang_runtime::provider_health::DiscoveredModelInfo {
                                name: name.clone(),
                                parameter_size: None,
                                quantization_level: None,
                                family: None,
                                families: None,
                                size: None,
                                capabilities: vec![],
                            },
                        )
                        .collect(),
                )
            } else {
                Some(result.discovered_model_info.clone())
            };
        kernel.model_catalog_update(|catalog| {
            catalog.set_provider_auth_status(
                provider_id,
                librefang_types::model_catalog::AuthStatus::NotRequired,
            );
            if let Some(ref info) = merged_info {
                catalog.merge_discovered_models(provider_id, info);
            }
        });
    } else {
        let err = result.error.as_deref().unwrap_or("unknown");
        if log_offline_as_warn {
            warn!(
                provider = %provider_id,
                error = err,
                "Configured local provider offline"
            );
        } else {
            debug!(
                provider = %provider_id,
                error = err,
                "Local provider offline (not configured as default/fallback)"
            );
        }
        // Mark unreachable local providers as LocalOffline (not Missing).
        // Using Missing would cause detect_auth() to reset the status back
        // to NotRequired on the next unrelated auth check, making offline
        // providers reappear in the model switcher.
        kernel.model_catalog_update(|catalog| {
            catalog.set_provider_auth_status(
                provider_id,
                librefang_types::model_catalog::AuthStatus::LocalOffline,
            );
        });
    }
    result
}

/// Probe every local provider once and update the catalog. Called from the
/// periodic loop in `start_background_agents`.
///
/// Probes run concurrently via `join_all`. The total wall time of one cycle
/// is bounded by the slowest probe (≤ 2 s per provider — see
/// `PROBE_TIMEOUT_SECS` in `provider_health`) instead of the sum across
/// providers, which matters when a local server is hung rather than simply
/// offline.
pub(super) async fn probe_all_local_providers_once(
    kernel: &Arc<LibreFangKernel>,
    relevant_providers: &std::collections::HashSet<String>,
) {
    let local_providers: Vec<(String, String)> = {
        let catalog = kernel.model_catalog.load();
        catalog
            .list_providers()
            .iter()
            .filter(|p| {
                librefang_runtime::provider_health::is_local_provider(&p.id)
                    && !p.base_url.is_empty()
            })
            .map(|p| (p.id.clone(), p.base_url.clone()))
            .collect()
    };
    let tasks = local_providers.into_iter().map(|(provider_id, base_url)| {
        let kernel = Arc::clone(kernel);
        let is_relevant = relevant_providers.contains(&provider_id.to_lowercase());
        async move {
            let _ = probe_and_update_local_provider(&kernel, &provider_id, &base_url, is_relevant)
                .await;
        }
    });
    futures::future::join_all(tasks).await;
}
