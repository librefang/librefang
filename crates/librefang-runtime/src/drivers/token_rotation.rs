//! Token rotation driver — transparent multi-key failover for any provider.
//!
//! Wraps multiple instances of the same LLM driver (each with a different API key)
//! and transparently rotates between them when one key hits rate limits, quota
//! exhaustion, or billing errors. Uses a round-robin strategy with cooldown tracking
//! so that exhausted keys are temporarily skipped.
//!
//! This enables users to configure multiple API keys per provider via
//! `[auth_profiles.<provider>]` in config.toml and get automatic failover.

use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmDriver, LlmError, StreamEvent};
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Default cooldown period for an exhausted key (5 minutes).
const DEFAULT_COOLDOWN_MS: u64 = 5 * 60 * 1000;

/// State for a single key slot in the rotation pool.
struct KeySlot {
    /// The driver instance for this key.
    driver: Arc<dyn LlmDriver>,
    /// Profile name for logging.
    name: String,
    /// Timestamp (ms since UNIX epoch) when this key's cooldown expires.
    /// 0 means the key is available.
    cooldown_until: u64,
}

/// A driver that rotates between multiple API keys for the same provider.
///
/// On rate-limit (429), quota exhaustion, or billing errors, the driver marks
/// the current key as exhausted (with a cooldown period) and transparently
/// retries with the next available key.
pub struct TokenRotationDriver {
    /// All key slots in the pool.
    slots: RwLock<Vec<KeySlot>>,
    /// Current index for round-robin selection.
    current: AtomicUsize,
    /// Provider name for logging.
    provider: String,
}

impl TokenRotationDriver {
    /// Create a new token rotation driver from a list of (driver, profile_name) pairs.
    ///
    /// The drivers should all be for the same provider but with different API keys.
    /// At least one driver must be provided.
    pub fn new(drivers: Vec<(Arc<dyn LlmDriver>, String)>, provider: String) -> Self {
        assert!(
            !drivers.is_empty(),
            "TokenRotationDriver requires at least one driver"
        );
        info!(
            provider = %provider,
            key_count = drivers.len(),
            profiles = ?drivers.iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>(),
            "Token rotation pool initialized"
        );
        let slots = drivers
            .into_iter()
            .map(|(driver, name)| KeySlot {
                driver,
                name,
                cooldown_until: 0,
            })
            .collect();
        Self {
            slots: RwLock::new(slots),
            current: AtomicUsize::new(0),
            provider,
        }
    }

    /// Get the current time in milliseconds since UNIX epoch.
    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    /// Check if an error should trigger key rotation.
    fn should_rotate(err: &LlmError) -> bool {
        matches!(
            err,
            LlmError::RateLimited { .. } | LlmError::Overloaded { .. }
        ) || matches!(err, LlmError::Api { status, message }
            if *status == 429
                || *status == 402
                || (*status == 403 && !message.to_lowercase().contains("invalid api key"))
        )
    }

    /// Extract cooldown duration from an error, falling back to the default.
    fn cooldown_from_error(err: &LlmError) -> u64 {
        match err {
            LlmError::RateLimited { retry_after_ms } => {
                // Use the suggested retry delay, but at least 30 seconds
                (*retry_after_ms).max(30_000)
            }
            LlmError::Overloaded { retry_after_ms } => (*retry_after_ms).max(10_000),
            _ => DEFAULT_COOLDOWN_MS,
        }
    }

    /// Find the next available key slot index, skipping cooled-down keys.
    /// Returns None if all keys are in cooldown.
    async fn next_available(&self) -> Option<usize> {
        let slots = self.slots.read().await;
        let len = slots.len();
        let now = Self::now_ms();
        let start = self.current.load(Ordering::Relaxed);

        for offset in 0..len {
            let idx = start.wrapping_add(offset) % len;
            if slots[idx].cooldown_until <= now {
                return Some(idx);
            }
        }
        None
    }

    /// Mark a key slot as exhausted with a cooldown period.
    async fn mark_exhausted(&self, index: usize, cooldown_ms: u64) {
        let mut slots = self.slots.write().await;
        if let Some(slot) = slots.get_mut(index) {
            slot.cooldown_until = Self::now_ms() + cooldown_ms;
            warn!(
                provider = %self.provider,
                profile = %slot.name,
                cooldown_ms,
                "Key exhausted, entering cooldown"
            );
        }
    }

    /// Advance the round-robin index to the next slot.
    fn advance(&self) {
        self.current.fetch_add(1, Ordering::Relaxed);
    }
}

#[async_trait]
impl LlmDriver for TokenRotationDriver {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let slot_count = self.slots.read().await.len();
        let mut last_error: Option<LlmError> = None;
        let mut tried: usize = 0;

        // Try each available slot up to the total number of slots.
        while tried < slot_count {
            let idx = match self.next_available().await {
                Some(i) => i,
                None => break, // All keys in cooldown
            };

            let driver = {
                let slots = self.slots.read().await;
                let slot = &slots[idx];
                debug!(
                    provider = %self.provider,
                    profile = %slot.name,
                    slot_index = idx,
                    "Trying key slot"
                );
                slot.driver.clone()
            };

            match driver.complete(request.clone()).await {
                Ok(response) => {
                    // Update the current index for next call
                    self.current.store(idx, Ordering::Relaxed);
                    self.advance();
                    return Ok(response);
                }
                Err(err) if Self::should_rotate(&err) => {
                    let cooldown = Self::cooldown_from_error(&err);
                    self.mark_exhausted(idx, cooldown).await;
                    self.advance();
                    last_error = Some(err);
                    tried += 1;
                }
                Err(err) => {
                    // Non-rotatable error — return immediately
                    return Err(err);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| LlmError::Api {
            status: 429,
            message: format!(
                "All {} API keys for provider '{}' are rate-limited or in cooldown",
                slot_count, self.provider
            ),
        }))
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        let slot_count = self.slots.read().await.len();
        let mut last_error: Option<LlmError> = None;
        let mut tried: usize = 0;

        while tried < slot_count {
            let idx = match self.next_available().await {
                Some(i) => i,
                None => break,
            };

            let driver = {
                let slots = self.slots.read().await;
                let slot = &slots[idx];
                debug!(
                    provider = %self.provider,
                    profile = %slot.name,
                    slot_index = idx,
                    "Trying key slot (stream)"
                );
                slot.driver.clone()
            };

            match driver.stream(request.clone(), tx.clone()).await {
                Ok(response) => {
                    self.current.store(idx, Ordering::Relaxed);
                    self.advance();
                    return Ok(response);
                }
                Err(err) if Self::should_rotate(&err) => {
                    let cooldown = Self::cooldown_from_error(&err);
                    self.mark_exhausted(idx, cooldown).await;
                    self.advance();
                    last_error = Some(err);
                    tried += 1;
                }
                Err(err) => {
                    return Err(err);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| LlmError::Api {
            status: 429,
            message: format!(
                "All {} API keys for provider '{}' are rate-limited or in cooldown (stream)",
                slot_count, self.provider
            ),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm_driver::CompletionResponse;
    use librefang_types::message::{ContentBlock, StopReason, TokenUsage};
    use std::sync::atomic::AtomicU32;

    fn test_request() -> CompletionRequest {
        CompletionRequest {
            model: "test".to_string(),
            messages: vec![],
            tools: vec![],
            max_tokens: 100,
            temperature: 0.0,
            system: None,
            thinking: None,
            prompt_caching: false,
        }
    }

    fn ok_response(text: &str) -> CompletionResponse {
        CompletionResponse {
            content: vec![ContentBlock::Text {
                text: text.to_string(),
                provider_metadata: None,
            }],
            stop_reason: StopReason::EndTurn,
            tool_calls: vec![],
            usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 5,
            },
        }
    }

    struct OkDriver {
        label: String,
    }

    #[async_trait]
    impl LlmDriver for OkDriver {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            Ok(ok_response(&self.label))
        }
    }

    struct RateLimitDriver {
        call_count: AtomicU32,
    }

    #[async_trait]
    impl LlmDriver for RateLimitDriver {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            Err(LlmError::RateLimited {
                retry_after_ms: 60_000,
            })
        }
    }

    struct CallCountingOkDriver {
        call_count: AtomicU32,
    }

    #[async_trait]
    impl LlmDriver for CallCountingOkDriver {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            Ok(ok_response("unused"))
        }
    }

    #[tokio::test]
    async fn test_single_key_passes_through() {
        let driver = TokenRotationDriver::new(
            vec![(
                Arc::new(OkDriver {
                    label: "key1".to_string(),
                }),
                "primary".to_string(),
            )],
            "test-provider".to_string(),
        );

        let result = driver.complete(test_request()).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().text(), "key1");
    }

    #[tokio::test]
    async fn test_rotates_on_rate_limit() {
        let driver = TokenRotationDriver::new(
            vec![
                (
                    Arc::new(RateLimitDriver {
                        call_count: AtomicU32::new(0),
                    }),
                    "key-a".to_string(),
                ),
                (
                    Arc::new(OkDriver {
                        label: "key-b".to_string(),
                    }),
                    "key-b".to_string(),
                ),
            ],
            "test-provider".to_string(),
        );

        let result = driver.complete(test_request()).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().text(), "key-b");
    }

    #[tokio::test]
    async fn test_all_keys_exhausted() {
        let driver = TokenRotationDriver::new(
            vec![
                (
                    Arc::new(RateLimitDriver {
                        call_count: AtomicU32::new(0),
                    }),
                    "key-a".to_string(),
                ),
                (
                    Arc::new(RateLimitDriver {
                        call_count: AtomicU32::new(0),
                    }),
                    "key-b".to_string(),
                ),
            ],
            "test-provider".to_string(),
        );

        let result = driver.complete(test_request()).await;
        assert!(matches!(result, Err(LlmError::RateLimited { .. })));
    }

    #[tokio::test]
    async fn test_non_rotatable_error_returns_immediately() {
        struct AuthFailDriver;

        #[async_trait]
        impl LlmDriver for AuthFailDriver {
            async fn complete(
                &self,
                _req: CompletionRequest,
            ) -> Result<CompletionResponse, LlmError> {
                Err(LlmError::ModelNotFound("no-such-model".to_string()))
            }
        }

        let driver = TokenRotationDriver::new(
            vec![
                (Arc::new(AuthFailDriver), "key-a".to_string()),
                (
                    Arc::new(OkDriver {
                        label: "key-b".to_string(),
                    }),
                    "key-b".to_string(),
                ),
            ],
            "test-provider".to_string(),
        );

        // ModelNotFound is not rotatable — should return error without trying key-b
        let result = driver.complete(test_request()).await;
        assert!(matches!(result, Err(LlmError::ModelNotFound(_))));
    }

    #[tokio::test]
    async fn test_authentication_failed_returns_immediately() {
        struct AuthFailDriver;

        #[async_trait]
        impl LlmDriver for AuthFailDriver {
            async fn complete(
                &self,
                _req: CompletionRequest,
            ) -> Result<CompletionResponse, LlmError> {
                Err(LlmError::AuthenticationFailed(
                    "invalid api key".to_string(),
                ))
            }
        }

        let fallback = Arc::new(CallCountingOkDriver {
            call_count: AtomicU32::new(0),
        });
        let driver = TokenRotationDriver::new(
            vec![
                (Arc::new(AuthFailDriver), "key-a".to_string()),
                (fallback.clone(), "key-b".to_string()),
            ],
            "test-provider".to_string(),
        );

        let result = driver.complete(test_request()).await;
        assert!(matches!(result, Err(LlmError::AuthenticationFailed(_))));
        assert_eq!(fallback.call_count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn test_should_rotate_classification() {
        assert!(TokenRotationDriver::should_rotate(&LlmError::RateLimited {
            retry_after_ms: 1000
        }));
        assert!(TokenRotationDriver::should_rotate(&LlmError::Overloaded {
            retry_after_ms: 1000
        }));
        assert!(TokenRotationDriver::should_rotate(&LlmError::Api {
            status: 429,
            message: "too many requests".to_string()
        }));
        assert!(TokenRotationDriver::should_rotate(&LlmError::Api {
            status: 402,
            message: "billing issue".to_string()
        }));
        assert!(TokenRotationDriver::should_rotate(&LlmError::Api {
            status: 403,
            message: "credit balance is too low".to_string()
        }));
        // Non-rotatable errors
        assert!(!TokenRotationDriver::should_rotate(
            &LlmError::ModelNotFound("x".to_string())
        ));
        assert!(!TokenRotationDriver::should_rotate(
            &LlmError::AuthenticationFailed("invalid api key".to_string())
        ));
        assert!(!TokenRotationDriver::should_rotate(&LlmError::Parse(
            "bad json".to_string()
        )));
        assert!(!TokenRotationDriver::should_rotate(&LlmError::Http(
            "connection failed".to_string()
        )));
        assert!(!TokenRotationDriver::should_rotate(&LlmError::Api {
            status: 403,
            message: "invalid api key".to_string()
        }));
    }

    #[test]
    fn test_cooldown_extraction() {
        assert_eq!(
            TokenRotationDriver::cooldown_from_error(&LlmError::RateLimited {
                retry_after_ms: 60_000
            }),
            60_000
        );
        // Small retry_after should be clamped to minimum 30s
        assert_eq!(
            TokenRotationDriver::cooldown_from_error(&LlmError::RateLimited {
                retry_after_ms: 100
            }),
            30_000
        );
        // Default cooldown for other errors
        assert_eq!(
            TokenRotationDriver::cooldown_from_error(&LlmError::Api {
                status: 429,
                message: "rate limited".to_string()
            }),
            DEFAULT_COOLDOWN_MS
        );
    }
}
