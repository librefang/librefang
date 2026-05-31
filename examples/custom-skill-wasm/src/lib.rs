//! Word-counter WASM skill — the WASM twin of `examples/custom-skill-python`,
//! written with the `librefang-skill` SDK.
//!
//! Pure compute, no host calls, so it needs no capabilities and runs under
//! `librefang skill test` end to end.

use librefang_skill::{skill, Request};
use serde_json::{json, Value};

fn handle(req: Request) -> Result<Value, String> {
    match req.tool.as_str() {
        "count_words" => {
            let text = req.input.get("text").and_then(Value::as_str).unwrap_or("");
            let words = text.split_whitespace().count();
            let sentences = text
                .split(['.', '!', '?'])
                .filter(|s| !s.trim().is_empty())
                .count();
            let characters = text.chars().count();
            Ok(json!({
                "words": words,
                "sentences": sentences,
                "characters": characters,
            }))
        }
        other => Err(format!("unknown tool: {other}")),
    }
}

skill!(handle);
