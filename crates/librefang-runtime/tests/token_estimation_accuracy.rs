//! Token-estimation accuracy benchmark (offline harness).
//!
//! `compactor::estimate_token_count` drives compaction triggers, the
//! context-usage report, and cost estimation, yet it is a character-weighted
//! heuristic (CJK ~1.5 tokens, other chars/4) with no ground truth behind it.
//! This harness measures that estimator against real provider `input_tokens`
//! so the error is a number instead of a guess, and so a later tuning PR can
//! prove an improvement rather than assert one.
//!
//! Two halves, deliberately split so CI stays offline and deterministic:
//!   - Corpus (`fixtures/token_estimation/corpus.json`) — message samples
//!     bucketed by content shape (English prose, CJK, mixed, JSON-heavy tool
//!     steps). Committed.
//!   - Ground truth (`fixtures/token_estimation/tokens_truth*.json`) — real
//!     `input_tokens` per sample, captured against live providers by the
//!     `capture_token_truth` example (see the fixtures README). Each file is a
//!     separate provider/tokenizer baseline and is reported on its own; the
//!     committed baselines are Zhipu GLM (`glm-4-flash`) and `openai/gpt-oss`
//!     (o200k tokenizer family).
//!
//! The estimator runs fully offline here. When ground-truth files are present
//! the harness additionally reports per-bucket error for each; when none exist
//! it prints how to generate one and still exercises the offline invariants.

use librefang_runtime::compactor::estimate_token_count;
use librefang_types::message::{ContentBlock, Message, MessageContent, Role};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

// --- Corpus schema ---------------------------------------------------------

#[derive(Debug, Deserialize)]
struct Corpus {
    samples: Vec<Sample>,
}

#[derive(Debug, Deserialize)]
struct Sample {
    id: String,
    bucket: String,
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

/// Build the `(messages, system)` pair a sample represents. Kept intentionally
/// simple and mirrored by the `capture_token_truth` example so the bytes the
/// estimator sees match the bytes sent to the provider for ground truth.
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

// --- Ground truth schema ---------------------------------------------------

#[derive(Debug, Deserialize)]
struct GroundTruth {
    samples: BTreeMap<String, TruthEntry>,
}

#[derive(Debug, Deserialize)]
struct TruthEntry {
    #[allow(dead_code)]
    provider: String,
    #[allow(dead_code)]
    model: String,
    input_tokens: u64,
}

// --- Fixture loading -------------------------------------------------------

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/token_estimation")
}

fn load_corpus() -> Corpus {
    let path = fixtures_dir().join("corpus.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read corpus {}: {e}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse corpus {}: {e}", path.display()))
}

/// Discover every captured ground-truth file (`tokens_truth*.json`, excluding
/// the `.example.json` placeholder), sorted by name for deterministic output.
/// Each file is a separate provider/tokenizer baseline, reported on its own.
fn load_ground_truth_files() -> Vec<(String, GroundTruth)> {
    let dir = fixtures_dir();
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return out;
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            name.starts_with("tokens_truth")
                && name.ends_with(".json")
                && !name.contains(".example.")
        })
        .collect();
    paths.sort();
    for path in paths {
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let truth: GroundTruth =
            serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
        let name = path
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        out.push((name, truth));
    }
    out
}

// --- Per-bucket error accounting -------------------------------------------

#[derive(Default)]
struct BucketStats {
    n: usize,
    sum_signed_pct: f64,
    sum_abs_pct: f64,
    sum_abs_tokens: u64,
}

impl BucketStats {
    fn record(&mut self, estimate: usize, truth: u64) {
        let truth_f = truth.max(1) as f64;
        let signed = (estimate as f64 - truth as f64) / truth_f * 100.0;
        self.n += 1;
        self.sum_signed_pct += signed;
        self.sum_abs_pct += signed.abs();
        self.sum_abs_tokens += (estimate as i64 - truth as i64).unsigned_abs();
    }
    fn mean_signed_pct(&self) -> f64 {
        self.sum_signed_pct / self.n.max(1) as f64
    }
    fn mae_pct(&self) -> f64 {
        self.sum_abs_pct / self.n.max(1) as f64
    }
    fn mae_tokens(&self) -> f64 {
        self.sum_abs_tokens as f64 / self.n.max(1) as f64
    }
}

// --- Tests -----------------------------------------------------------------

/// Offline invariants that hold without any provider call: every sample
/// estimates to a positive token count and the estimator is deterministic
/// (the prompt-cache correctness rule, #3298, depends on stable estimates).
#[test]
fn estimator_is_positive_and_deterministic_over_corpus() {
    let corpus = load_corpus();
    assert!(!corpus.samples.is_empty(), "corpus must not be empty");
    for sample in &corpus.samples {
        let (messages, system) = build_messages(sample);
        let a = estimate_token_count(&messages, system.as_deref(), None);
        let b = estimate_token_count(&messages, system.as_deref(), None);
        assert!(a > 0, "sample {} estimated 0 tokens", sample.id);
        assert_eq!(a, b, "sample {} estimate not deterministic", sample.id);
    }
}

/// Report estimator error against every captured ground-truth baseline,
/// bucketed by content shape. Each `tokens_truth*.json` file is a distinct
/// provider/tokenizer; the per-bucket signs across files are what reveal which
/// errors are tokenizer-specific (e.g. CJK) versus cross-provider (e.g. the
/// JSON-heavy `tool_json` undercount). Prints a table per baseline (visible
/// with `--nocapture`).
#[test]
fn report_estimator_error_against_ground_truth() {
    let corpus = load_corpus();
    let baselines = load_ground_truth_files();
    if baselines.is_empty() {
        eprintln!(
            "\n[token-estimation] no tokens_truth*.json yet — offline-only run.\n\
             Generate one against a real provider:\n  \
             cargo run -p librefang-llm-drivers --example capture_token_truth -- \\\n    \
             --provider openai --model gpt-4o-mini --out \
             crates/librefang-runtime/tests/fixtures/token_estimation/tokens_truth.json\n\
             See crates/librefang-runtime/tests/fixtures/token_estimation/README.md.\n"
        );
        return;
    }

    let ceiling = std::env::var("LIBREFANG_TOKEN_EST_MAX_MAE_PCT")
        .ok()
        .map(|v| {
            v.parse::<f64>()
                .expect("LIBREFANG_TOKEN_EST_MAX_MAE_PCT must be a number")
        });

    for (name, truth) in &baselines {
        let mut per_bucket: BTreeMap<String, BucketStats> = BTreeMap::new();
        let mut overall = BucketStats::default();
        let mut missing = Vec::new();

        for sample in &corpus.samples {
            let Some(entry) = truth.samples.get(&sample.id) else {
                missing.push(sample.id.clone());
                continue;
            };
            let (messages, system) = build_messages(sample);
            let estimate = estimate_token_count(&messages, system.as_deref(), None);
            per_bucket
                .entry(sample.bucket.clone())
                .or_default()
                .record(estimate, entry.input_tokens);
            overall.record(estimate, entry.input_tokens);
        }

        eprintln!("\n[token-estimation] {name}: estimator error vs provider input_tokens");
        eprintln!(
            "  {:<18} {:>5} {:>12} {:>10} {:>11}",
            "bucket", "n", "mean signed%", "MAE%", "MAE tokens"
        );
        for (bucket, s) in &per_bucket {
            eprintln!(
                "  {:<18} {:>5} {:>11.1}% {:>9.1}% {:>11.1}",
                bucket,
                s.n,
                s.mean_signed_pct(),
                s.mae_pct(),
                s.mae_tokens()
            );
        }
        eprintln!(
            "  {:<18} {:>5} {:>11.1}% {:>9.1}% {:>11.1}",
            "ALL",
            overall.n,
            overall.mean_signed_pct(),
            overall.mae_pct(),
            overall.mae_tokens()
        );
        if !missing.is_empty() {
            eprintln!("  (no ground truth for: {})", missing.join(", "));
        }

        assert!(
            overall.n > 0,
            "{name}: ground truth present but no sample ids matched the corpus"
        );

        // Regression ceiling: opt-in via env, applied per baseline. Off by
        // default because the error is provider-specific and no cross-provider
        // ceiling has been agreed yet.
        if let Some(ceiling) = ceiling {
            assert!(
                overall.mae_pct() <= ceiling,
                "{name}: overall MAE {:.1}% exceeds ceiling {:.1}%",
                overall.mae_pct(),
                ceiling
            );
        }
    }
}
