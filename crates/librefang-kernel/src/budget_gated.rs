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
        resp.actual_provider = Some(self.provider_name.clone());
        Ok(resp)
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        self.check_budget()?;
        self.inner.stream(request, tx).await
    }
}
