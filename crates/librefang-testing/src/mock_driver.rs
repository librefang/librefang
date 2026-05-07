//! Mock LLM driver — a configurable fake LLM provider for testing.
//!
//! Supports:
//! - Returning canned responses
//! - Recording all requests for assertions
//! - Simulating streaming responses

use async_trait::async_trait;
use librefang_runtime::llm_driver::{
    CompletionRequest, CompletionResponse, LlmDriver, LlmError, StreamEvent,
};
use librefang_types::message::{ContentBlock, StopReason, TokenUsage};
use std::sync::{Arc, Mutex};

/// Recorded LLM call information.
#[derive(Debug, Clone)]
pub struct RecordedCall {
    /// Model name from the request.
    pub model: String,
    /// Number of messages.
    pub message_count: usize,
    /// Number of tool definitions.
    pub tool_count: usize,
    /// System prompt (if any).
    pub system: Option<String>,
}

/// Mock LLM driver — returns configurable canned responses and records all calls.
pub struct MockLlmDriver {
    /// List of canned response texts, returned in order. Wraps around to the last one when exhausted.
    responses: Vec<String>,
    /// Recorded calls.
    calls: Arc<Mutex<Vec<RecordedCall>>>,
    /// Current response index.
    index: Arc<Mutex<usize>>,
    /// Custom input token count (default 10).
    input_tokens: u64,
    /// Custom output token count (default 5).
    output_tokens: u64,
    /// Custom stop reason (default EndTurn).
    stop_reason: StopReason,
}

impl MockLlmDriver {
    /// Creates a mock driver that returns canned responses.
    ///
    /// ```rust
    /// use librefang_testing::MockLlmDriver;
    ///
    /// let driver = MockLlmDriver::new(vec!["Hello!".into()]);
    /// ```
    pub fn new(responses: Vec<String>) -> Self {
        assert!(
            !responses.is_empty(),
            "MockLlmDriver requires at least one canned response"
        );
        Self {
            responses,
            calls: Arc::new(Mutex::new(Vec::new())),
            index: Arc::new(Mutex::new(0)),
            input_tokens: 10,
            output_tokens: 5,
            stop_reason: StopReason::EndTurn,
        }
    }

    /// Creates a mock driver that always returns the same response.
    pub fn with_response(response: impl Into<String>) -> Self {
        Self::new(vec![response.into()])
    }

    /// Sets custom token usage (overrides the default input=10, output=5).
    ///
    /// ```rust
    /// use librefang_testing::MockLlmDriver;
    ///
    /// let driver = MockLlmDriver::with_response("hi").with_tokens(100, 50);
    /// ```
    pub fn with_tokens(mut self, input: u64, output: u64) -> Self {
        self.input_tokens = input;
        self.output_tokens = output;
        self
    }

    /// Sets a custom stop reason (overrides the default EndTurn).
    ///
    /// ```rust
    /// use librefang_testing::MockLlmDriver;
    /// use librefang_types::message::StopReason;
    ///
    /// let driver = MockLlmDriver::with_response("hi").with_stop_reason(StopReason::MaxTokens);
    /// ```
    pub fn with_stop_reason(mut self, reason: StopReason) -> Self {
        self.stop_reason = reason;
        self
    }

    /// Returns all recorded calls.
    pub fn recorded_calls(&self) -> Vec<RecordedCall> {
        self.calls.lock().unwrap().clone()
    }

    /// Returns the number of calls made.
    pub fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    /// Gets the next response text.
    fn next_response(&self) -> String {
        let mut idx = self.index.lock().unwrap();
        let response = if *idx < self.responses.len() {
            self.responses[*idx].clone()
        } else {
            // Wrap around to the last response when exhausted
            self.responses.last().unwrap().clone()
        };
        *idx += 1;
        response
    }
}

#[async_trait]
impl LlmDriver for MockLlmDriver {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        // Record the call
        {
            let call = RecordedCall {
                model: request.model.clone(),
                message_count: request.messages.len(),
                tool_count: request.tools.len(),
                system: request.system.clone(),
            };
            self.calls.lock().unwrap().push(call);
        }

        let text = self.next_response();
        Ok(CompletionResponse {
            content: vec![ContentBlock::Text {
                text,
                provider_metadata: None,
            }],
            stop_reason: self.stop_reason,
            tool_calls: vec![],
            usage: TokenUsage {
                input_tokens: self.input_tokens,
                output_tokens: self.output_tokens,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            },
            actual_provider: None,
        })
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        // Simulate streaming: send TextDelta first, then ContentComplete
        let response = self.complete(request).await?;
        let text = response.text();
        if !text.is_empty() {
            let _ = tx.send(StreamEvent::TextDelta { text }).await;
        }
        let _ = tx
            .send(StreamEvent::ContentComplete {
                stop_reason: response.stop_reason,
                usage: response.usage,
            })
            .await;
        Ok(response)
    }

    fn is_configured(&self) -> bool {
        true
    }
}

/// A mock driver that always returns errors, used for testing error handling.
pub struct FailingLlmDriver {
    error_message: String,
}

impl FailingLlmDriver {
    /// Creates a driver that always returns the specified error.
    pub fn new(error_message: impl Into<String>) -> Self {
        Self {
            error_message: error_message.into(),
        }
    }
}

#[async_trait]
impl LlmDriver for FailingLlmDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Err(LlmError::Api {
            status: 500,
            message: self.error_message.clone(),
            code: None,
        })
    }

    fn is_configured(&self) -> bool {
        false
    }
}
