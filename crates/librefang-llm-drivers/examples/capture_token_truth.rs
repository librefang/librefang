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
use librefang_llm_drivers::llm_driver::{CompletionRequest, LlmDriver};
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

        // Free tiers (OpenRouter, …) rate-limit aggressively. Retry on any
        // error with backoff rather than aborting the whole run, and pace
        // requests so we stay under per-minute caps.
        let mut input = None;
        for attempt in 0..MAX_RETRIES {
            match driver.complete(make_request()).await {
                Ok(resp) => {
                    input = Some(resp.usage.input_tokens);
                    break;
                }
                Err(e) if attempt + 1 < MAX_RETRIES => {
                    let backoff = RETRY_BACKOFF_SECS * (attempt + 1) as u64;
                    eprintln!(
                        "  {:<18} attempt {} failed ({e}); retrying in {backoff}s",
                        sample.id,
                        attempt + 1
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
                }
                Err(e) => panic!(
                    "sample {} failed after {MAX_RETRIES} attempts: {e}",
                    sample.id
                ),
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
