//! ChatGPT session-based authentication driver.
//!
//! Uses a ChatGPT Plus/Ultra session token (obtained via browser login) to
//! access the ChatGPT backend API. The session token is used as a bearer token
//! with the OpenAI-compatible API endpoint.
//!
//! Token lifecycle:
//! - Session token provided via env var `CHATGPT_SESSION_TOKEN` or browser auth flow
//! - Token is cached and reused until it expires or is invalidated
//! - On expiry, user is prompted to re-authenticate via browser

use std::sync::Mutex;
use std::time::{Duration, Instant};
use tracing::debug;
use zeroize::Zeroizing;

use crate::chatgpt_oauth::CHATGPT_BASE_URL;

/// How long a ChatGPT session token is valid (conservative estimate).
/// ChatGPT session tokens typically last ~2 weeks, but we refresh at 7 days.
const SESSION_TOKEN_TTL_SECS: u64 = 7 * 24 * 3600; // 7 days

/// Refresh buffer — refresh this many seconds before estimated expiry.
const REFRESH_BUFFER_SECS: u64 = 3600; // 1 hour

/// Cached ChatGPT session token with estimated expiry.
#[derive(Clone)]
pub struct CachedSessionToken {
    /// The bearer token (zeroized on drop).
    pub token: Zeroizing<String>,
    /// Estimated expiry time.
    pub expires_at: Instant,
}

impl CachedSessionToken {
    /// Check if the token is still considered valid (with refresh buffer).
    pub fn is_valid(&self) -> bool {
        self.expires_at > Instant::now() + Duration::from_secs(REFRESH_BUFFER_SECS)
    }
}

/// Thread-safe token cache for a ChatGPT session.
pub struct ChatGptTokenCache {
    cached: Mutex<Option<CachedSessionToken>>,
}

impl ChatGptTokenCache {
    pub fn new() -> Self {
        Self {
            cached: Mutex::new(None),
        }
    }

    /// Get a valid cached token, or None if expired/missing.
    pub fn get(&self) -> Option<CachedSessionToken> {
        let lock = self.cached.lock().unwrap_or_else(|e| e.into_inner());
        lock.as_ref().filter(|t| t.is_valid()).cloned()
    }

    /// Store a new token in the cache.
    pub fn set(&self, token: CachedSessionToken) {
        let mut lock = self.cached.lock().unwrap_or_else(|e| e.into_inner());
        *lock = Some(token);
    }
}

impl Default for ChatGptTokenCache {
    fn default() -> Self {
        Self::new()
    }
}

/// LLM driver that wraps OpenAI-compatible with ChatGPT session token.
///
/// On each API call, ensures a valid session token is available,
/// then delegates to an OpenAI-compatible driver.
pub struct ChatGptDriver {
    /// The session token (provided at construction or via env).
    session_token: Zeroizing<String>,
    /// Base URL override.
    base_url: String,
    /// Token cache.
    token_cache: ChatGptTokenCache,
}

impl ChatGptDriver {
    pub fn new(session_token: String, base_url: String) -> Self {
        Self {
            session_token: Zeroizing::new(session_token),
            base_url: if base_url.is_empty() {
                CHATGPT_BASE_URL.to_string()
            } else {
                base_url
            },
            token_cache: ChatGptTokenCache::new(),
        }
    }

    /// Get a valid session token, caching it with an estimated TTL.
    fn ensure_token(&self) -> Result<CachedSessionToken, crate::llm_driver::LlmError> {
        // Check cache first
        if let Some(cached) = self.token_cache.get() {
            return Ok(cached);
        }

        // Use the session token directly (it's a bearer token)
        if self.session_token.is_empty() {
            return Err(crate::llm_driver::LlmError::MissingApiKey(
                "ChatGPT session token not set. Run browser auth flow or set CHATGPT_SESSION_TOKEN"
                    .to_string(),
            ));
        }

        debug!("Caching ChatGPT session token");
        let token = CachedSessionToken {
            token: self.session_token.clone(),
            expires_at: Instant::now() + Duration::from_secs(SESSION_TOKEN_TTL_SECS),
        };

        self.token_cache.set(token.clone());
        Ok(token)
    }

    /// Create a fresh OpenAI driver with the current session token.
    fn make_inner_driver(
        &self,
        token: &CachedSessionToken,
    ) -> super::openai::OpenAIDriver {
        super::openai::OpenAIDriver::new(token.token.to_string(), self.base_url.clone())
    }
}

#[async_trait::async_trait]
impl crate::llm_driver::LlmDriver for ChatGptDriver {
    async fn complete(
        &self,
        request: crate::llm_driver::CompletionRequest,
    ) -> Result<crate::llm_driver::CompletionResponse, crate::llm_driver::LlmError> {
        let token = self.ensure_token()?;
        let driver = self.make_inner_driver(&token);
        driver.complete(request).await
    }

    async fn stream(
        &self,
        request: crate::llm_driver::CompletionRequest,
        tx: tokio::sync::mpsc::Sender<crate::llm_driver::StreamEvent>,
    ) -> Result<crate::llm_driver::CompletionResponse, crate::llm_driver::LlmError> {
        let token = self.ensure_token()?;
        let driver = self.make_inner_driver(&token);
        driver.stream(request, tx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_cache_empty() {
        let cache = ChatGptTokenCache::new();
        assert!(cache.get().is_none());
    }

    #[test]
    fn test_token_cache_set_get() {
        let cache = ChatGptTokenCache::new();
        let token = CachedSessionToken {
            token: Zeroizing::new("test-session-token".to_string()),
            expires_at: Instant::now() + Duration::from_secs(86400),
        };
        cache.set(token);
        let cached = cache.get();
        assert!(cached.is_some());
        assert_eq!(*cached.unwrap().token, "test-session-token");
    }

    #[test]
    fn test_token_validity_check() {
        // Valid token (expires in 1 day)
        let valid = CachedSessionToken {
            token: Zeroizing::new("t".to_string()),
            expires_at: Instant::now() + Duration::from_secs(86400),
        };
        assert!(valid.is_valid());

        // Token that expires in < 1 hour should be considered expired
        let almost_expired = CachedSessionToken {
            token: Zeroizing::new("t".to_string()),
            expires_at: Instant::now() + Duration::from_secs(60),
        };
        assert!(!almost_expired.is_valid());
    }

    #[test]
    fn test_chatgpt_driver_new_default_url() {
        let driver = ChatGptDriver::new("tok".to_string(), String::new());
        assert_eq!(driver.base_url, CHATGPT_BASE_URL);
    }

    #[test]
    fn test_chatgpt_driver_new_custom_url() {
        let driver =
            ChatGptDriver::new("tok".to_string(), "https://custom.api.com/v1".to_string());
        assert_eq!(driver.base_url, "https://custom.api.com/v1");
    }

    #[test]
    fn test_ensure_token_empty_errors() {
        let driver = ChatGptDriver::new(String::new(), String::new());
        let result = driver.ensure_token();
        assert!(result.is_err());
    }

    #[test]
    fn test_ensure_token_caches() {
        let driver = ChatGptDriver::new("my-token".to_string(), String::new());
        let tok1 = driver.ensure_token().unwrap();
        let tok2 = driver.ensure_token().unwrap();
        assert_eq!(*tok1.token, *tok2.token);
    }
}
