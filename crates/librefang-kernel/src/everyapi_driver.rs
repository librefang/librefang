//! OpenAI-compatible EveryAPI driver with CLI-managed credentials.

use crate::everyapi_credentials::{self, CredentialError, EveryApiCredential};
use async_trait::async_trait;
use librefang_llm_driver::{
    CompletionRequest, CompletionResponse, DriverConfig, LlmDriver, LlmError, LlmFamily,
    StreamEvent,
};
use librefang_llm_drivers::drivers::DriverCache;
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
            .map_err(|_| CredentialError::Unavailable)?
    }
}

/// Resolves the current EveryAPI relay key for each request.
///
/// A rejected credential is invalidated and resolved once more. The retry is
/// deliberately bounded to one attempt so an account or gateway failure cannot
/// create an authentication loop.
pub struct ManagedEveryApiDriver {
    source: Arc<dyn EveryApiCredentialSource>,
    cache: Arc<DriverCache>,
    base_config: DriverConfig,
}

impl ManagedEveryApiDriver {
    pub fn new(base_config: DriverConfig, cache: Arc<DriverCache>) -> Self {
        Self {
            source: Arc::new(LocalEveryApiCredentialSource),
            cache,
            base_config,
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
        }
    }

    async fn resolve_driver(&self, invalidate: bool) -> Result<Arc<dyn LlmDriver>, LlmError> {
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
        LlmError::AuthenticationFailed(_) | LlmError::Api { status: 401, .. }
    )
}

#[async_trait]
impl LlmDriver for ManagedEveryApiDriver {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let driver = self.resolve_driver(false).await?;
        match driver.complete(request.clone()).await {
            Err(error) if is_authentication_error(&error) => {
                self.resolve_driver(true).await?.complete(request).await
            }
            result => result,
        }
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        let driver = self.resolve_driver(false).await?;
        match driver.stream(request.clone(), tx.clone()).await {
            Err(error) if is_authentication_error(&error) => {
                self.resolve_driver(true).await?.stream(request, tx).await
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

    #[test]
    fn quota_or_policy_forbidden_does_not_rotate_credentials() {
        assert!(!is_authentication_error(&LlmError::Api {
            status: 403,
            message: "quota or policy rejection".to_string(),
            code: None,
        }));
    }
}
