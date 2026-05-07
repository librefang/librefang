//! Budget-gated driver wrapper.
//!
//! Wraps an inner LLM driver and checks the provider's budget gate
//! before every call. Returns a rate-limit error when exhausted so
//! the outer fallback driver skips to the next provider.

use async_trait::async_trait;
use librefang_kernel_metering::provider_gate::ProviderBudgetGate;
use librefang_llm_driver::llm_errors::ProviderErrorCode;
use librefang_llm_driver::{
    CompletionRequest, CompletionResponse, LlmDriver, LlmError, StreamEvent,
};
use std::sync::Arc;

pub struct BudgetGatedDriver {
    inner: Arc<dyn LlmDriver>,
    gate: Arc<ProviderBudgetGate>,
    provider_name: String,
}

impl BudgetGatedDriver {
    pub fn new(
        inner: Arc<dyn LlmDriver>,
        gate: Arc<ProviderBudgetGate>,
        provider_name: String,
    ) -> Self {
        Self {
            inner,
            gate,
            provider_name,
        }
    }

    fn check_budget(&self) -> Result<(), LlmError> {
        self.gate
            .check(&self.provider_name)
            .map_err(|e| LlmError::Api {
                status: 429,
                message: e.to_string(),
                code: Some(ProviderErrorCode::RateLimit),
            })
    }
}

#[async_trait]
impl LlmDriver for BudgetGatedDriver {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        self.check_budget()?;
        let mut resp = self.inner.complete(request).await?;
        // get_or_insert_with: do not stomp a provider name set by an inner
        // chain (e.g. nested FallbackChain) — only fill when the inner did
        // not already attribute the call to a specific provider.
        resp.actual_provider
            .get_or_insert_with(|| self.provider_name.clone());
        Ok(resp)
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        self.check_budget()?;
        let mut resp = self.inner.stream(request, tx).await?;
        resp.actual_provider
            .get_or_insert_with(|| self.provider_name.clone());
        Ok(resp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_kernel_metering::MeteringEngine;
    use librefang_llm_driver::{CompletionRequest, CompletionResponse, LlmDriver, LlmError};
    use librefang_memory::usage::UsageStore;
    use librefang_types::config::ProviderBudget;
    use librefang_types::message::{StopReason, TokenUsage};
    use std::collections::HashMap;
    use std::sync::Arc;

    fn make_request() -> CompletionRequest {
        CompletionRequest {
            model: "test-model".to_string(),
            messages: Arc::new(vec![]),
            tools: Arc::new(vec![]),
            max_tokens: 64,
            temperature: 0.0,
            system: None,
            thinking: None,
            prompt_caching: false,
            cache_ttl: None,
            response_format: None,
            timeout_secs: None,
            extra_body: None,
            agent_id: None,
            session_id: None,
            step_id: None,
        }
    }

    fn ok_response(provider: Option<&str>) -> CompletionResponse {
        CompletionResponse {
            content: vec![],
            stop_reason: StopReason::EndTurn,
            tool_calls: vec![],
            usage: TokenUsage {
                input_tokens: 1,
                output_tokens: 1,
                ..Default::default()
            },
            actual_provider: provider.map(|s| s.to_string()),
        }
    }

    struct OkInner {
        attribution: Option<String>,
    }

    #[async_trait]
    impl LlmDriver for OkInner {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            Ok(ok_response(self.attribution.as_deref()))
        }
    }

    fn in_memory_metering() -> Arc<MeteringEngine> {
        let substrate = librefang_memory::MemorySubstrate::open_in_memory(0.1)
            .expect("in-memory memory substrate");
        let store = Arc::new(UsageStore::new(substrate.pool()));
        Arc::new(MeteringEngine::new(store))
    }

    #[tokio::test]
    async fn complete_returns_429_when_gate_denies() {
        let metering = in_memory_metering();
        // Pre-record enough spend that an hourly cap of $0.01 is already exhausted.
        let record = librefang_memory::usage::UsageRecord {
            agent_id: librefang_types::agent::AgentId::new(),
            provider: "anthropic".to_string(),
            model: "claude-3".to_string(),
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
            cost_usd: 100.0,
            tool_calls: 0,
            latency_ms: 0,
            user_id: None,
            channel: None,
            session_id: None,
        };
        metering.record(&record).expect("record seed usage");

        let mut budgets = HashMap::new();
        budgets.insert(
            "anthropic".to_string(),
            ProviderBudget {
                max_cost_per_hour_usd: 0.01,
                ..Default::default()
            },
        );
        let gate = Arc::new(ProviderBudgetGate::new(metering, budgets));

        let inner: Arc<dyn LlmDriver> = Arc::new(OkInner { attribution: None });
        let gated = BudgetGatedDriver::new(inner, gate, "anthropic".to_string());

        let err = gated
            .complete(make_request())
            .await
            .expect_err("expected gate denial");
        match err {
            LlmError::Api { status, code, .. } => {
                assert_eq!(status, 429, "gate must surface 429 so fallback skips");
                assert!(matches!(code, Some(ProviderErrorCode::RateLimit)));
            }
            other => panic!("expected LlmError::Api, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn complete_does_not_clobber_inner_attribution() {
        let metering = in_memory_metering();
        let gate = Arc::new(ProviderBudgetGate::new(metering, HashMap::new()));
        // Inner already set actual_provider="inner-chain-pick"; the wrapper
        // must not overwrite it (the inner path knows better which leaf served).
        let inner: Arc<dyn LlmDriver> = Arc::new(OkInner {
            attribution: Some("inner-chain-pick".to_string()),
        });
        let gated = BudgetGatedDriver::new(inner, gate, "wrapper-default".to_string());
        let resp = gated.complete(make_request()).await.expect("ok");
        assert_eq!(resp.actual_provider.as_deref(), Some("inner-chain-pick"));
    }

    #[tokio::test]
    async fn complete_fills_attribution_when_inner_left_it_blank() {
        let metering = in_memory_metering();
        let gate = Arc::new(ProviderBudgetGate::new(metering, HashMap::new()));
        let inner: Arc<dyn LlmDriver> = Arc::new(OkInner { attribution: None });
        let gated = BudgetGatedDriver::new(inner, gate, "anthropic".to_string());
        let resp = gated.complete(make_request()).await.expect("ok");
        assert_eq!(resp.actual_provider.as_deref(), Some("anthropic"));
    }
}
