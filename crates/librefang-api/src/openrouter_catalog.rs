use dashmap::DashMap;
use librefang_kernel::kernel_api::KernelApi;
use librefang_types::model_catalog::AuthStatus;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
enum RefreshAttempt {
    InProgress,
    Completed(Instant),
}

static REFRESH_ATTEMPTS: LazyLock<DashMap<String, RefreshAttempt>> = LazyLock::new(DashMap::new);
const REFRESH_RETRY_WINDOW: Duration = Duration::from_secs(60);

fn begin_refresh(base_url: &str) -> Result<(), String> {
    match REFRESH_ATTEMPTS.entry(base_url.to_string()) {
        dashmap::mapref::entry::Entry::Occupied(mut attempt) => match *attempt.get() {
            RefreshAttempt::InProgress => {
                Err("OpenRouter catalog refresh is already in progress".to_string())
            }
            RefreshAttempt::Completed(completed_at)
                if completed_at.elapsed() < REFRESH_RETRY_WINDOW =>
            {
                Err("OpenRouter catalog refresh is in the 60-second retry window".to_string())
            }
            RefreshAttempt::Completed(_) => {
                attempt.insert(RefreshAttempt::InProgress);
                Ok(())
            }
        },
        dashmap::mapref::entry::Entry::Vacant(attempt) => {
            attempt.insert(RefreshAttempt::InProgress);
            Ok(())
        }
    }
}

fn finish_refresh_failure(base_url: &str) {
    REFRESH_ATTEMPTS.remove(base_url);
}

fn finish_refresh_success(base_url: String) {
    REFRESH_ATTEMPTS.insert(base_url, RefreshAttempt::Completed(Instant::now()));
}

fn provider_is_configured(kernel: &Arc<dyn KernelApi>) -> bool {
    let catalog = kernel.model_catalog_ref().load();
    // `InvalidKey` is intentional because OpenRouter's model catalog is public.
    catalog.get_provider("openrouter").is_some_and(|provider| {
        matches!(
            provider.auth_status,
            AuthStatus::Configured
                | AuthStatus::ValidatedKey
                | AuthStatus::AutoDetected
                | AuthStatus::InvalidKey
        )
    })
}

pub(crate) fn needs_initial_refresh(kernel: &Arc<dyn KernelApi>) -> bool {
    provider_is_configured(kernel)
        && !kernel
            .model_catalog_ref()
            .load()
            .has_live_provider_models("openrouter")
}

fn needs_stale_refresh(kernel: &Arc<dyn KernelApi>) -> bool {
    provider_is_configured(kernel)
        && kernel
            .model_catalog_ref()
            .load()
            .live_provider_models_are_stale(
                "openrouter",
                librefang_kernel::model_catalog::OPENROUTER_MODEL_CATALOG_TTL,
            )
}

pub(crate) async fn refresh_if_missing(kernel: &Arc<dyn KernelApi>) -> Result<usize, String> {
    if !needs_initial_refresh(kernel) {
        return Ok(0);
    }
    refresh_now(kernel).await
}

pub(crate) async fn refresh_if_stale(kernel: &Arc<dyn KernelApi>) -> Result<usize, String> {
    if !needs_stale_refresh(kernel) {
        return Ok(0);
    }
    refresh_now(kernel).await
}

pub(crate) fn refresh_if_missing_in_background(kernel: &Arc<dyn KernelApi>) {
    if !needs_initial_refresh(kernel) {
        return;
    }
    let kernel = Arc::clone(kernel);
    tokio::spawn(async move {
        if let Err(error) = refresh_if_missing(&kernel).await {
            tracing::warn!(%error, "OpenRouter live catalog background refresh failed");
        }
    });
}

async fn refresh_now(kernel: &Arc<dyn KernelApi>) -> Result<usize, String> {
    let base_url = {
        let catalog = kernel.model_catalog_ref().load();
        catalog
            .get_provider("openrouter")
            .map(|provider| provider.base_url.clone())
            .filter(|url| !url.is_empty())
            .ok_or_else(|| "OpenRouter base URL is not configured".to_string())?
    };
    begin_refresh(&base_url)?;

    let snapshot =
        match librefang_kernel::model_catalog::fetch_openrouter_model_snapshot(&base_url).await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                finish_refresh_failure(&base_url);
                return Err(error);
            }
        };
    let model_count = snapshot.live_models.len();
    kernel.model_catalog_update(&mut move |catalog| {
        catalog.reconcile_live_provider_models(
            "openrouter",
            snapshot.available_models.clone(),
            snapshot.live_models.clone(),
        );
    });
    finish_refresh_success(base_url);
    Ok(model_count)
}

/// Helper function to clear the rate-limiting attempts cache for a specific URL during integration testing to prevent sequential port reuse contamination (#6384).
/// Only exposed when compiling for tests with the `test-util` feature enabled.
#[cfg(feature = "test-util")]
pub fn clear_refresh_attempts(base_url: &str) {
    REFRESH_ATTEMPTS.remove(base_url);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_refresh_releases_claim_but_success_starts_retry_window() {
        let base_url = "https://openrouter-refresh-state.test";
        REFRESH_ATTEMPTS.remove(base_url);

        assert!(begin_refresh(base_url).is_ok());
        assert_eq!(
            begin_refresh(base_url).unwrap_err(),
            "OpenRouter catalog refresh is already in progress"
        );

        finish_refresh_failure(base_url);
        assert!(begin_refresh(base_url).is_ok());
        finish_refresh_success(base_url.to_string());
        assert_eq!(
            begin_refresh(base_url).unwrap_err(),
            "OpenRouter catalog refresh is in the 60-second retry window"
        );

        REFRESH_ATTEMPTS.remove(base_url);
    }
}
