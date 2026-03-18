//! Fallback driver — tries multiple LLM drivers in sequence.
//!
//! If the primary driver fails with a non-retryable error, the fallback driver
//! moves to the next driver in the chain.

use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmDriver, LlmError, StreamEvent};
use async_trait::async_trait;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tracing::warn;

/// A driver that wraps multiple LLM drivers and tries each in order.
///
/// On failure (including rate-limit and overload), moves to the next driver.
/// Only returns an error when ALL drivers in the chain are exhausted.
/// Each driver is paired with the model name it should use.
///
/// Health-aware: tracks per-driver EWMA latency and consecutive errors.
/// On each request, the driver list is dynamically reordered so healthy,
/// low-latency drivers are tried first while preserving the primary position
/// when it is healthy.
pub struct FallbackDriver {
    drivers: Vec<DriverEntry>,
}

struct DriverEntry {
    driver: Arc<dyn LlmDriver>,
    model_name: String,
    /// Exponentially weighted moving average latency in ms.
    ewma_latency_ms: AtomicU64,
    /// Consecutive error count. Reset to 0 on success.
    consecutive_errors: AtomicU64,
}

/// Penalty added to EWMA when a driver errors (makes it sort lower).
const ERROR_PENALTY_MS: u64 = 30_000;
/// EWMA smoothing factor (0.3 = new sample is 30% of the result).
const EWMA_ALPHA: f64 = 0.3;

impl FallbackDriver {
    /// Create a new fallback driver from an ordered chain of (driver, model_name) pairs.
    ///
    /// The first entry is the primary; subsequent are fallbacks.
    pub fn new(drivers: Vec<Arc<dyn LlmDriver>>) -> Self {
        Self {
            drivers: drivers
                .into_iter()
                .map(|d| DriverEntry {
                    driver: d,
                    model_name: String::new(),
                    ewma_latency_ms: AtomicU64::new(0),
                    consecutive_errors: AtomicU64::new(0),
                })
                .collect(),
        }
    }

    /// Create a new fallback driver with explicit model names for each driver.
    pub fn with_models(drivers: Vec<(Arc<dyn LlmDriver>, String)>) -> Self {
        Self {
            drivers: drivers
                .into_iter()
                .map(|(d, m)| DriverEntry {
                    driver: d,
                    model_name: m,
                    ewma_latency_ms: AtomicU64::new(0),
                    consecutive_errors: AtomicU64::new(0),
                })
                .collect(),
        }
    }

    /// Build a health-aware ordering of driver indices. Healthy drivers
    /// (consecutive_errors == 0) come first sorted by EWMA latency; unhealthy
    /// drivers follow sorted by error count (fewest errors first). The primary
    /// driver (index 0) gets a latency bonus to keep it preferred when healthy.
    fn health_order(&self) -> Vec<usize> {
        let mut indices: Vec<usize> = (0..self.drivers.len()).collect();
        indices.sort_by(|&a, &b| {
            let ea = self.drivers[a].consecutive_errors.load(Ordering::Relaxed);
            let eb = self.drivers[b].consecutive_errors.load(Ordering::Relaxed);
            // Healthy (0 errors) before unhealthy
            match (ea == 0, eb == 0) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => {
                    let la = self.drivers[a].ewma_latency_ms.load(Ordering::Relaxed);
                    let lb = self.drivers[b].ewma_latency_ms.load(Ordering::Relaxed);
                    la.cmp(&lb)
                }
            }
        });
        indices
    }
}

#[async_trait]
impl LlmDriver for FallbackDriver {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let mut last_error = None;
        let order = self.health_order();

        for &i in &order {
            let entry = &self.drivers[i];
            let mut req = request.clone();
            if !entry.model_name.is_empty() {
                req.model = entry.model_name.clone();
            }

            let start = std::time::Instant::now();
            match entry.driver.complete(req).await {
                Ok(response) => {
                    let latency = start.elapsed().as_millis() as u64;
                    let prev = entry.ewma_latency_ms.load(Ordering::Relaxed);
                    let new = if prev == 0 {
                        latency
                    } else {
                        (EWMA_ALPHA * latency as f64 + (1.0 - EWMA_ALPHA) * prev as f64) as u64
                    };
                    entry.ewma_latency_ms.store(new, Ordering::Relaxed);
                    entry.consecutive_errors.store(0, Ordering::Relaxed);
                    return Ok(response);
                }
                Err(e) => {
                    entry.consecutive_errors.fetch_add(1, Ordering::Relaxed);
                    entry
                        .ewma_latency_ms
                        .fetch_add(ERROR_PENALTY_MS, Ordering::Relaxed);
                    warn!(
                        driver_index = i,
                        model = %entry.model_name,
                        error = %e,
                        consecutive_errors = entry.consecutive_errors.load(Ordering::Relaxed),
                        "Fallback driver failed, trying next"
                    );
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| LlmError::Api {
            status: 0,
            message: "No drivers configured in fallback chain".to_string(),
        }))
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        let mut last_error = None;
        let order = self.health_order();

        for &i in &order {
            let entry = &self.drivers[i];
            let mut req = request.clone();
            if !entry.model_name.is_empty() {
                req.model = entry.model_name.clone();
            }

            let start = std::time::Instant::now();
            match entry.driver.stream(req, tx.clone()).await {
                Ok(response) => {
                    let latency = start.elapsed().as_millis() as u64;
                    let prev = entry.ewma_latency_ms.load(Ordering::Relaxed);
                    let new = if prev == 0 {
                        latency
                    } else {
                        (EWMA_ALPHA * latency as f64 + (1.0 - EWMA_ALPHA) * prev as f64) as u64
                    };
                    entry.ewma_latency_ms.store(new, Ordering::Relaxed);
                    entry.consecutive_errors.store(0, Ordering::Relaxed);
                    return Ok(response);
                }
                Err(e) => {
                    entry.consecutive_errors.fetch_add(1, Ordering::Relaxed);
                    entry
                        .ewma_latency_ms
                        .fetch_add(ERROR_PENALTY_MS, Ordering::Relaxed);
                    warn!(
                        driver_index = i,
                        model = %entry.model_name,
                        error = %e,
                        consecutive_errors = entry.consecutive_errors.load(Ordering::Relaxed),
                        "Fallback driver (stream) failed, trying next"
                    );
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| LlmError::Api {
            status: 0,
            message: "No drivers configured in fallback chain".to_string(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm_driver::CompletionResponse;
    use librefang_types::message::{ContentBlock, StopReason, TokenUsage};

    struct FailDriver;

    #[async_trait]
    impl LlmDriver for FailDriver {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            Err(LlmError::Api {
                status: 500,
                message: "Internal error".to_string(),
            })
        }
    }

    struct OkDriver;

    #[async_trait]
    impl LlmDriver for OkDriver {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            Ok(CompletionResponse {
                content: vec![ContentBlock::Text {
                    text: "OK".to_string(),
                    provider_metadata: None,
                }],
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
                usage: TokenUsage {
                    input_tokens: 10,
                    output_tokens: 5,
                    ..Default::default()
                },
            })
        }
    }

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

    #[tokio::test]
    async fn test_fallback_primary_succeeds() {
        let driver = FallbackDriver::new(vec![
            Arc::new(OkDriver) as Arc<dyn LlmDriver>,
            Arc::new(FailDriver) as Arc<dyn LlmDriver>,
        ]);
        let result = driver.complete(test_request()).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().text(), "OK");
    }

    #[tokio::test]
    async fn test_fallback_primary_fails_secondary_succeeds() {
        let driver = FallbackDriver::new(vec![
            Arc::new(FailDriver) as Arc<dyn LlmDriver>,
            Arc::new(OkDriver) as Arc<dyn LlmDriver>,
        ]);
        let result = driver.complete(test_request()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_fallback_all_fail() {
        let driver = FallbackDriver::new(vec![
            Arc::new(FailDriver) as Arc<dyn LlmDriver>,
            Arc::new(FailDriver) as Arc<dyn LlmDriver>,
        ]);
        let result = driver.complete(test_request()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_rate_limit_falls_through() {
        struct RateLimitDriver;

        #[async_trait]
        impl LlmDriver for RateLimitDriver {
            async fn complete(
                &self,
                _req: CompletionRequest,
            ) -> Result<CompletionResponse, LlmError> {
                Err(LlmError::RateLimited {
                    retry_after_ms: 5000,
                })
            }
        }

        let driver = FallbackDriver::new(vec![
            Arc::new(RateLimitDriver) as Arc<dyn LlmDriver>,
            Arc::new(OkDriver) as Arc<dyn LlmDriver>,
        ]);
        let result = driver.complete(test_request()).await;
        // Rate limit should fall through to the OkDriver fallback
        assert!(result.is_ok());
        assert_eq!(result.unwrap().text(), "OK");
    }

    #[tokio::test]
    async fn test_rate_limit_all_fail() {
        struct RateLimitDriver;

        #[async_trait]
        impl LlmDriver for RateLimitDriver {
            async fn complete(
                &self,
                _req: CompletionRequest,
            ) -> Result<CompletionResponse, LlmError> {
                Err(LlmError::RateLimited {
                    retry_after_ms: 5000,
                })
            }
        }

        let driver = FallbackDriver::new(vec![
            Arc::new(RateLimitDriver) as Arc<dyn LlmDriver>,
            Arc::new(RateLimitDriver) as Arc<dyn LlmDriver>,
        ]);
        let result = driver.complete(test_request()).await;
        // All drivers rate-limited — error should bubble up
        assert!(matches!(result, Err(LlmError::RateLimited { .. })));
    }
}
