//! OpenAI-compatible EveryAPI driver with CLI-managed credentials.

use crate::everyapi_credentials::{self, CredentialError, EveryApiCredential};
use async_trait::async_trait;
use librefang_llm_driver::llm_errors::ProviderErrorCode;
use librefang_llm_driver::{
    CompletionRequest, CompletionResponse, DriverConfig, LlmDriver, LlmError, LlmFamily,
    StreamEvent,
};
use librefang_llm_drivers::drivers::DriverCache;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[async_trait]
trait EveryApiCredentialSource: Send + Sync {
    async fn resolve(&self, invalidate: bool) -> Result<EveryApiCredential, CredentialError>;
}

struct LocalEveryApiCredentialSource;

#[async_trait]
impl EveryApiCredentialSource for LocalEveryApiCredentialSource {
    async fn resolve(&self, invalidate: bool) -> Result<EveryApiCredential, CredentialError> {
        tokio::task::spawn_blocking(move || everyapi_credentials::resolve(invalidate))
            .await
            .map_err(credential_task_error)?
    }
}

fn credential_task_error(error: tokio::task::JoinError) -> CredentialError {
    CredentialError::InvalidOutput(format!("credential resolver task failed: {error}"))
}

/// Resolves the current EveryAPI relay key for each request.
///
/// A rejected credential is invalidated and resolved once more.
/// The retry is deliberately bounded to one attempt so an account or gateway failure cannot create an authentication loop.
pub struct ManagedEveryApiDriver {
    source: Arc<dyn EveryApiCredentialSource>,
    cache: Arc<DriverCache>,
    base_config: DriverConfig,
    managed_gate: Option<PathBuf>,
    /// Serializes credential invalidation and records completed refreshes.
    /// Requests that fail with the same generation share one invalidation.
    credential_generation: tokio::sync::Mutex<u64>,
}

impl ManagedEveryApiDriver {
    pub fn new(base_config: DriverConfig, cache: Arc<DriverCache>) -> Self {
        Self {
            source: Arc::new(LocalEveryApiCredentialSource),
            cache,
            base_config,
            managed_gate: None,
            credential_generation: tokio::sync::Mutex::new(0),
        }
    }

    pub fn new_gated(
        base_config: DriverConfig,
        cache: Arc<DriverCache>,
        home_dir: PathBuf,
    ) -> Self {
        Self {
            source: Arc::new(LocalEveryApiCredentialSource),
            cache,
            base_config,
            managed_gate: Some(home_dir),
            credential_generation: tokio::sync::Mutex::new(0),
        }
    }

    #[cfg(test)]
    fn with_source(source: Arc<dyn EveryApiCredentialSource>, max_retries: u32) -> Self {
        Self {
            source,
            cache: Arc::new(DriverCache::new()),
            base_config: DriverConfig {
                provider: "everyapi".to_string(),
                max_retries,
                ..DriverConfig::default()
            },
            managed_gate: None,
            credential_generation: tokio::sync::Mutex::new(0),
        }
    }

    async fn resolve_driver(&self, invalidate: bool) -> Result<Arc<dyn LlmDriver>, LlmError> {
        if let Some(home_dir) = self.managed_gate.clone() {
            let disabled =
                tokio::task::spawn_blocking(move || managed_everyapi_is_disabled(&home_dir))
                    .await
                    .map_err(|error| {
                        LlmError::Http(format!("EveryAPI managed gate task failed: {error}"))
                    })?;
            if disabled {
                return Err(LlmError::MissingApiKey(
                    "EveryAPI managed provider is disabled or explicitly configured".to_string(),
                ));
            }
        }
        let credential = self
            .source
            .resolve(invalidate)
            .await
            .map_err(credential_error_to_llm)?;
        let mut config = self.base_config.clone();
        config.provider = "everyapi".to_string();
        config.api_key = Some(credential.api_key);
        config.base_url = Some(credential.base_url);
        self.cache.get_or_create(&config)
    }

    async fn resolve_initial_driver(&self) -> Result<(Arc<dyn LlmDriver>, u64), LlmError> {
        loop {
            let generation = *self.credential_generation.lock().await;
            let driver = self.resolve_driver(false).await?;
            if *self.credential_generation.lock().await == generation {
                return Ok((driver, generation));
            }
            // A refresh completed while the credential source was resolving.
            // Resolve again so the returned driver and observed generation
            // describe the same credential snapshot.
        }
    }

    async fn resolve_after_authentication_failure(
        &self,
        observed_generation: u64,
    ) -> Result<Arc<dyn LlmDriver>, LlmError> {
        let mut generation = self.credential_generation.lock().await;
        if *generation != observed_generation {
            return self.resolve_driver(false).await;
        }

        let driver = self.resolve_driver(true).await?;
        *generation = (*generation).wrapping_add(1);
        Ok(driver)
    }
}

fn managed_everyapi_is_disabled(home_dir: &std::path::Path) -> bool {
    if std::env::var("EVERYAPI_API_KEY").is_ok_and(|key| !key.trim().is_empty())
        || home_dir.join("providers").join("everyapi.toml").exists()
    {
        return true;
    }
    let path = home_dir.join("data").join("suppressed_providers.json");
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Vec<String>>(&bytes).ok())
        .is_some_and(|providers| providers.iter().any(|provider| provider == "everyapi"))
}

fn credential_error_to_llm(error: CredentialError) -> LlmError {
    match error {
        CredentialError::NotInstalled
        | CredentialError::NotLoggedIn
        | CredentialError::NoRelayKey => LlmError::MissingApiKey(error.to_string()),
        _ => LlmError::Http(error.to_string()),
    }
}

fn is_authentication_error(error: &LlmError) -> bool {
    matches!(
        error,
        LlmError::AuthenticationFailed(_)
            | LlmError::Api { status: 401, .. }
            | LlmError::Api {
                code: Some(ProviderErrorCode::AuthError),
                ..
            }
    )
}

fn should_retry_stream(error: &LlmError, event_emitted: bool) -> bool {
    !event_emitted && is_authentication_error(error)
}

async fn stream_attempt(
    driver: Arc<dyn LlmDriver>,
    request: CompletionRequest,
    tx: tokio::sync::mpsc::Sender<StreamEvent>,
) -> (Result<CompletionResponse, LlmError>, bool) {
    let (intercept_tx, mut intercept_rx) = tokio::sync::mpsc::channel::<StreamEvent>(32);
    let event_emitted = Arc::new(AtomicBool::new(false));
    let relay_emitted = Arc::clone(&event_emitted);
    let relay = tokio::spawn(async move {
        while let Some(event) = intercept_rx.recv().await {
            relay_emitted.store(true, Ordering::Release);
            if tx.send(event).await.is_err() {
                intercept_rx.close();
                break;
            }
        }
    });

    let result = driver.stream(request, intercept_tx).await;
    let _ = relay.await;
    (result, event_emitted.load(Ordering::Acquire))
}

#[async_trait]
impl LlmDriver for ManagedEveryApiDriver {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let (driver, generation) = self.resolve_initial_driver().await?;
        match driver.complete(request.clone()).await {
            Err(error) if is_authentication_error(&error) => {
                self.resolve_after_authentication_failure(generation)
                    .await?
                    .complete(request)
                    .await
            }
            result => result,
        }
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        let (driver, generation) = self.resolve_initial_driver().await?;
        let (result, event_emitted) = stream_attempt(driver, request.clone(), tx.clone()).await;
        match result {
            Err(error) if should_retry_stream(&error, event_emitted) => {
                self.resolve_after_authentication_failure(generation)
                    .await?
                    .stream(request, tx)
                    .await
            }
            result => result,
        }
    }

    fn family(&self) -> LlmFamily {
        LlmFamily::OpenAi
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::everyapi_credentials::{CredentialError, EveryApiCredential};
    use async_trait::async_trait;
    use chrono::Utc;
    use librefang_llm_driver::{CompletionRequest, LlmDriver};
    use librefang_types::message::Message;
    use std::sync::atomic::AtomicUsize;
    use std::sync::{Arc, Mutex};
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    struct SequenceSource {
        invalidations: Mutex<Vec<bool>>,
        base_url: String,
    }

    #[async_trait]
    impl EveryApiCredentialSource for SequenceSource {
        async fn resolve(&self, invalidate: bool) -> Result<EveryApiCredential, CredentialError> {
            self.invalidations.lock().unwrap().push(invalidate);
            Ok(EveryApiCredential {
                base_url: self.base_url.clone(),
                api_key: if invalidate { "fresh-key" } else { "old-key" }.to_string(),
                expires_at: Some(Utc::now() + chrono::Duration::hours(1)),
            })
        }
    }

    struct RotatingSource {
        invalidations: Mutex<Vec<bool>>,
        fresh: AtomicBool,
        base_url: String,
    }

    #[async_trait]
    impl EveryApiCredentialSource for RotatingSource {
        async fn resolve(&self, invalidate: bool) -> Result<EveryApiCredential, CredentialError> {
            self.invalidations.lock().unwrap().push(invalidate);
            if invalidate {
                self.fresh.store(true, Ordering::Release);
            }
            Ok(EveryApiCredential {
                base_url: self.base_url.clone(),
                api_key: if self.fresh.load(Ordering::Acquire) {
                    "fresh-key"
                } else {
                    "old-key"
                }
                .to_string(),
                expires_at: Some(Utc::now() + chrono::Duration::hours(1)),
            })
        }
    }

    struct InterleavingSource {
        invalidations: Mutex<Vec<bool>>,
        fresh: AtomicBool,
        non_invalidating_lookups: AtomicUsize,
        initial_started: tokio::sync::Notify,
        release_initial: tokio::sync::Notify,
        base_url: String,
    }

    #[async_trait]
    impl EveryApiCredentialSource for InterleavingSource {
        async fn resolve(&self, invalidate: bool) -> Result<EveryApiCredential, CredentialError> {
            self.invalidations.lock().unwrap().push(invalidate);
            if invalidate {
                self.fresh.store(true, Ordering::Release);
            } else if self.non_invalidating_lookups.fetch_add(1, Ordering::AcqRel) == 0 {
                self.initial_started.notify_one();
                self.release_initial.notified().await;
            }
            Ok(EveryApiCredential {
                base_url: self.base_url.clone(),
                api_key: if self.fresh.load(Ordering::Acquire) {
                    "fresh-key"
                } else {
                    "old-key"
                }
                .to_string(),
                expires_at: Some(Utc::now() + chrono::Duration::hours(1)),
            })
        }
    }

    struct AuthenticationStreamDriver {
        emit_event: bool,
    }

    #[async_trait]
    impl LlmDriver for AuthenticationStreamDriver {
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            Err(LlmError::AuthenticationFailed("rejected".to_string()))
        }

        async fn stream(
            &self,
            _request: CompletionRequest,
            tx: tokio::sync::mpsc::Sender<StreamEvent>,
        ) -> Result<CompletionResponse, LlmError> {
            if self.emit_event {
                let _ = tx
                    .send(StreamEvent::TextDelta {
                        text: "partial".to_string(),
                    })
                    .await;
            }
            Err(LlmError::AuthenticationFailed("rejected".to_string()))
        }

        fn family(&self) -> LlmFamily {
            LlmFamily::OpenAi
        }
    }

    fn request() -> CompletionRequest {
        CompletionRequest {
            model: "gpt-test".to_string(),
            messages: Arc::new(vec![Message::user("hello")]),
            ..Default::default()
        }
    }

    fn success_body() -> serde_json::Value {
        serde_json::json!({
            "id": "chatcmpl-test",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "ok"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
        })
    }

    #[tokio::test]
    async fn authentication_failure_invalidates_and_retries_once() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("authorization", "Bearer old-key"))
            .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
                "error": {"code": "invalid_api_key", "message": "rejected"}
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("authorization", "Bearer fresh-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(success_body()))
            .expect(1)
            .mount(&server)
            .await;

        let source = Arc::new(SequenceSource {
            invalidations: Mutex::new(Vec::new()),
            base_url: format!("{}/v1", server.uri()),
        });
        let driver = ManagedEveryApiDriver::with_source(source.clone(), 0);
        let response = driver.complete(request()).await.unwrap();

        assert_eq!(response.text(), "ok");
        assert_eq!(*source.invalidations.lock().unwrap(), [false, true]);
    }

    #[tokio::test]
    async fn concurrent_auth_failures_share_one_invalidation() {
        let source = Arc::new(RotatingSource {
            invalidations: Mutex::new(Vec::new()),
            fresh: AtomicBool::new(false),
            base_url: "http://127.0.0.1:9/v1".to_string(),
        });
        let driver = ManagedEveryApiDriver::with_source(source.clone(), 0);

        let (_, first_generation) = driver.resolve_initial_driver().await.unwrap();
        let (_, second_generation) = driver.resolve_initial_driver().await.unwrap();
        assert_eq!(first_generation, second_generation);

        let (first, second) = tokio::join!(
            driver.resolve_after_authentication_failure(first_generation),
            driver.resolve_after_authentication_failure(second_generation)
        );
        assert!(first.is_ok());
        assert!(second.is_ok());
        assert_eq!(
            *source.invalidations.lock().unwrap(),
            [false, false, true, false],
            "the second stale failure must reuse the completed refresh"
        );
    }

    #[tokio::test]
    async fn initial_resolution_retries_when_refresh_changes_generation() {
        let source = Arc::new(InterleavingSource {
            invalidations: Mutex::new(Vec::new()),
            fresh: AtomicBool::new(false),
            non_invalidating_lookups: AtomicUsize::new(0),
            initial_started: tokio::sync::Notify::new(),
            release_initial: tokio::sync::Notify::new(),
            base_url: "http://127.0.0.1:9/v1".to_string(),
        });
        let driver = Arc::new(ManagedEveryApiDriver::with_source(source.clone(), 0));
        let initial = {
            let driver = Arc::clone(&driver);
            tokio::spawn(async move { driver.resolve_initial_driver().await })
        };

        source.initial_started.notified().await;
        driver
            .resolve_after_authentication_failure(0)
            .await
            .unwrap();
        source.release_initial.notify_one();

        let (_, generation) = initial.await.unwrap().unwrap();
        assert_eq!(generation, 1);
        assert_eq!(
            *source.invalidations.lock().unwrap(),
            [false, true, false],
            "the overlapping initial lookup must be repeated after refresh"
        );
    }

    #[tokio::test]
    async fn stream_attempt_reports_any_forwarded_event() {
        for (emit_event, expected) in [(false, false), (true, true)] {
            let (tx, mut rx) = tokio::sync::mpsc::channel(4);
            let (result, emitted) = stream_attempt(
                Arc::new(AuthenticationStreamDriver { emit_event }),
                request(),
                tx,
            )
            .await;

            assert!(matches!(result, Err(LlmError::AuthenticationFailed(_))));
            assert_eq!(emitted, expected);
            assert_eq!(rx.try_recv().is_ok(), expected);
        }
    }

    #[tokio::test]
    async fn credential_resolver_join_error_keeps_panic_context() {
        let join_error = tokio::spawn(async { panic!("credential resolver boom") })
            .await
            .unwrap_err();
        let error = credential_task_error(join_error).to_string();
        assert!(error.contains("credential resolver boom"), "{error}");
    }

    #[test]
    fn quota_or_policy_forbidden_does_not_rotate_credentials() {
        assert!(!is_authentication_error(&LlmError::Api {
            status: 403,
            message: "quota or policy rejection".to_string(),
            code: None,
        }));
        assert!(is_authentication_error(&LlmError::Api {
            status: 403,
            message: "credential rejected".to_string(),
            code: Some(ProviderErrorCode::AuthError),
        }));
    }

    #[test]
    fn stream_authentication_retry_requires_a_clean_event_stream() {
        let error = LlmError::AuthenticationFailed("rejected".to_string());
        assert!(should_retry_stream(&error, false));
        assert!(!should_retry_stream(&error, true));
    }

    #[test]
    fn persisted_suppression_disables_boot_time_managed_driver() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join("data")).unwrap();
        std::fs::write(
            home.path().join("data/suppressed_providers.json"),
            br#"["everyapi"]"#,
        )
        .unwrap();
        assert!(managed_everyapi_is_disabled(home.path()));
    }
}
