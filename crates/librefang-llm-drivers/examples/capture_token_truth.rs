//! Capture real provider `input_tokens` for the token-estimation corpus.
//!
//! This is the live, human-run half of the token-estimation accuracy benchmark
//! (the offline half is `librefang-runtime/tests/token_estimation_accuracy.rs`).
//! It reads the committed corpus, sends each sample once with `max_tokens = 1`
//! and prompt caching disabled, and records the provider-reported
//! `usage.input_tokens` as ground truth.
//!
//! Run once and commit the output; CI never invokes this.
//!
//! ```bash
//! OPENAI_API_KEY=<key> cargo run -p librefang-llm-drivers \
//!   --example capture_token_truth -- \
//!   --provider openai --model gpt-4o-mini \
//!   --out crates/librefang-runtime/tests/fixtures/token_estimation/tokens_truth.json
//! ```

use librefang_llm_drivers::drivers::anthropic::AnthropicDriver;
use librefang_llm_drivers::drivers::openai::OpenAIDriver;
use librefang_llm_drivers::llm_driver::{CompletionRequest, LlmDriver, LlmError};
use librefang_llm_drivers::llm_errors::{self, ProviderErrorCode};
use librefang_types::message::{ContentBlock, Message, MessageContent, Role};
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Max attempts per sample before giving up.
const MAX_RETRIES: u32 = 6;
/// Base backoff (seconds), multiplied by attempt number. Free-tier limits
/// typically reset within a minute, so the first retry alone clears most.
const RETRY_BACKOFF_SECS: u64 = 25;
/// Pause between successful requests to stay under per-minute rate caps.
const INTER_REQUEST_SECS: u64 = 5;

#[derive(Debug, Deserialize)]
struct Corpus {
    samples: Vec<Sample>,
}

#[derive(Debug, Deserialize)]
struct Sample {
    id: String,
    #[serde(default)]
    system: Option<String>,
    turns: Vec<Turn>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
enum Turn {
    User {
        text: String,
    },
    Assistant {
        text: String,
    },
    ToolUse {
        tool_id: String,
        tool_name: String,
        tool_input: serde_json::Value,
    },
    ToolResult {
        tool_id: String,
        tool_name: String,
        content: String,
    },
}

/// Mirror of the benchmark's builder so the bytes sent for ground truth match
/// the bytes the estimator scores.
fn build_messages(sample: &Sample) -> (Vec<Message>, Option<String>) {
    let mut messages = Vec::with_capacity(sample.turns.len());
    for turn in &sample.turns {
        match turn {
            Turn::User { text } => messages.push(Message::user(text.clone())),
            Turn::Assistant { text } => messages.push(Message::assistant(text.clone())),
            Turn::ToolUse {
                tool_id,
                tool_name,
                tool_input,
            } => messages.push(Message {
                role: Role::Assistant,
                content: MessageContent::Blocks(vec![ContentBlock::ToolUse {
                    id: tool_id.clone(),
                    name: tool_name.clone(),
                    input: tool_input.clone(),
                    provider_metadata: None,
                }]),
                pinned: false,
                timestamp: None,
            }),
            Turn::ToolResult {
                tool_id,
                tool_name,
                content,
            } => messages.push(Message::user_with_blocks(vec![ContentBlock::ToolResult {
                tool_use_id: tool_id.clone(),
                tool_name: tool_name.clone(),
                content: content.clone(),
                is_error: false,
                status: Default::default(),
                approval_request_id: None,
            }])),
        }
    }
    (messages, sample.system.clone())
}

/// Return whether repeating the same capture request can plausibly succeed.
///
/// Provider throttling, temporary server failures, timeouts, and recognizable
/// transport interruptions are transient. Authentication, billing, malformed
/// requests, missing models, parse failures, and opaque transport errors must
/// fail immediately instead of sleeping through the full retry budget.
fn is_retryable_capture_error(error: &LlmError) -> bool {
    match error {
        LlmError::RateLimited { .. } | LlmError::Overloaded { .. } | LlmError::TimedOut { .. } => {
            true
        }
        LlmError::Api { status, code, .. } => {
            if matches!(*status, 401 | 402 | 404 | 413) {
                return false;
            }
            if *status == 403 {
                // Some OpenAI-compatible providers use 403 for throttling.
                // Only an explicit typed rate-limit code makes that otherwise
                // permanent status safe to retry.
                return matches!(code, Some(ProviderErrorCode::RateLimit));
            }
            match code {
                Some(
                    ProviderErrorCode::RateLimit
                    | ProviderErrorCode::ServerUnavailable
                    | ProviderErrorCode::ServerError,
                ) => true,
                // A typed permanent code is more precise than a contradictory
                // generic 5xx status and must not consume the retry budget.
                Some(_) => false,
                None => matches!(*status, 408 | 429 | 500 | 502 | 503 | 504),
            }
        }
        LlmError::Http(message) => llm_errors::is_transient(message),
        LlmError::AllProvidersExhausted {
            cause: Some(cause), ..
        } => is_retryable_capture_error(cause),
        _ => false,
    }
}

struct Args {
    provider: String,
    /// Provenance label written into the truth file. Defaults to `provider`,
    /// but lets an OpenAI-compatible backend (Zhipu/GLM, Groq, Moonshot, …)
    /// record its real identity even though it is driven via `--provider openai`.
    label: String,
    model: String,
    base_url: Option<String>,
    corpus: String,
    out: String,
}

fn parse_args() -> Args {
    let mut provider = None;
    let mut label = None;
    let mut model = None;
    let mut base_url = None;
    let mut corpus =
        "crates/librefang-runtime/tests/fixtures/token_estimation/corpus.json".to_string();
    let mut out =
        "crates/librefang-runtime/tests/fixtures/token_estimation/tokens_truth.json".to_string();

    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut next = || {
            it.next()
                .unwrap_or_else(|| panic!("missing value for {flag}"))
        };
        match flag.as_str() {
            "--provider" => provider = Some(next()),
            "--label" => label = Some(next()),
            "--model" => model = Some(next()),
            "--base-url" => base_url = Some(next()),
            "--corpus" => corpus = next(),
            "--out" => out = next(),
            other => panic!("unknown flag: {other}"),
        }
    }
    let provider = provider.expect("--provider is required (openai|anthropic)");
    Args {
        label: label.unwrap_or_else(|| provider.clone()),
        provider,
        model: model.expect("--model is required"),
        base_url,
        corpus,
        out,
    }
}

#[tokio::main]
async fn main() {
    let args = parse_args();

    let (driver, default_base): (Box<dyn LlmDriver>, &str) = match args.provider.as_str() {
        "openai" => {
            let key = std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY not set");
            let base = args
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.openai.com/v1".into());
            (
                Box::new(OpenAIDriver::new(key, base.clone())),
                "https://api.openai.com/v1",
            )
        }
        "anthropic" => {
            let key = std::env::var("ANTHROPIC_API_KEY").expect("ANTHROPIC_API_KEY not set");
            let base = args
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.anthropic.com".into());
            (
                Box::new(AnthropicDriver::new(key, base.clone())),
                "https://api.anthropic.com",
            )
        }
        other => panic!("unsupported --provider {other} (expected openai|anthropic)"),
    };
    let base_url = args
        .base_url
        .clone()
        .unwrap_or_else(|| default_base.to_string());

    let raw = std::fs::read_to_string(&args.corpus)
        .unwrap_or_else(|e| panic!("read corpus {}: {e}", args.corpus));
    let corpus: Corpus = serde_json::from_str(&raw).expect("parse corpus.json");

    let mut samples: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    for sample in &corpus.samples {
        let (messages, system) = build_messages(sample);
        let make_request = || CompletionRequest {
            model: args.model.clone(),
            messages: Arc::new(messages.clone()),
            tools: Arc::new(vec![]),
            max_tokens: 1,
            temperature: 0.0,
            system: system.clone(),
            prompt_caching: false,
            ..Default::default()
        };

        // Free tiers (OpenRouter, …) rate-limit aggressively. Retry transient
        // failures with backoff, fail fast on permanent errors, and pace
        // successful requests so we stay under per-minute caps.
        let mut input = None;
        for attempt in 0..MAX_RETRIES {
            match driver.complete(make_request()).await {
                Ok(resp) => {
                    input = Some(resp.usage.input_tokens);
                    break;
                }
                Err(e) => {
                    if !is_retryable_capture_error(&e) {
                        panic!(
                            "sample {} failed with a non-retryable error on attempt {}: {e}",
                            sample.id,
                            attempt + 1
                        );
                    }
                    if attempt + 1 == MAX_RETRIES {
                        panic!(
                            "sample {} failed after {MAX_RETRIES} attempts: {e}",
                            sample.id
                        );
                    }
                    let backoff = RETRY_BACKOFF_SECS * (attempt + 1) as u64;
                    eprintln!(
                        "  {:<18} attempt {} failed ({e}); retrying in {backoff}s",
                        sample.id,
                        attempt + 1
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
                }
            }
        }
        let input = input.expect("retry loop guarantees a value or panics");
        eprintln!("  {:<18} input_tokens = {input}", sample.id);
        samples.insert(
            sample.id.clone(),
            json!({ "provider": args.label, "model": args.model, "input_tokens": input }),
        );
        tokio::time::sleep(std::time::Duration::from_secs(INTER_REQUEST_SECS)).await;
    }

    let doc = json!({
        "captured_with": {
            "provider": args.label,
            "driver": args.provider,
            "model": args.model,
            "base_url": base_url,
            "prompt_caching": false,
        },
        "samples": samples,
    });
    let pretty = serde_json::to_string_pretty(&doc).expect("serialize truth");
    std::fs::write(&args.out, pretty + "\n").unwrap_or_else(|e| panic!("write {}: {e}", args.out));
    eprintln!("\nWrote {} samples to {}", samples.len(), args.out);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api_error(status: u16, code: Option<ProviderErrorCode>) -> LlmError {
        LlmError::Api {
            status,
            message: "provider error".to_string(),
            code,
        }
    }

    #[test]
    fn capture_retries_only_transient_provider_failures() {
        for error in [
            LlmError::RateLimited {
                retry_after_ms: 1,
                message: None,
            },
            LlmError::Overloaded { retry_after_ms: 1 },
            LlmError::TimedOut {
                inactivity_secs: 30,
                partial_text: None,
                partial_text_len: 0,
                last_activity: "request sent".to_string(),
            },
            api_error(408, None),
            api_error(429, None),
            api_error(500, None),
            api_error(502, None),
            api_error(503, None),
            api_error(504, None),
            api_error(400, Some(ProviderErrorCode::ServerUnavailable)),
            api_error(400, Some(ProviderErrorCode::ServerError)),
            api_error(403, Some(ProviderErrorCode::RateLimit)),
            LlmError::Http("connection reset by peer".to_string()),
        ] {
            assert!(is_retryable_capture_error(&error), "{error:?}");
        }
    }

    #[test]
    fn capture_fails_fast_on_permanent_or_opaque_failures() {
        for error in [
            LlmError::AuthenticationFailed("bad key".to_string()),
            LlmError::MissingApiKey("missing".to_string()),
            LlmError::ModelNotFound("missing-model".to_string()),
            LlmError::Parse("invalid response".to_string()),
            api_error(400, None),
            api_error(401, None),
            api_error(403, None),
            api_error(404, None),
            api_error(501, None),
            api_error(400, Some(ProviderErrorCode::BadRequest)),
            api_error(401, Some(ProviderErrorCode::ServerError)),
            api_error(500, Some(ProviderErrorCode::AuthError)),
            api_error(503, Some(ProviderErrorCode::BadRequest)),
            LlmError::Http("invalid base URL".to_string()),
        ] {
            assert!(!is_retryable_capture_error(&error), "{error:?}");
        }
    }
}
