//! Opt-in MoA trace persistence.
//!
//! When `moa.save_traces` is enabled, each cache-MISS turn appends one JSON
//! line to `<trace_dir>/<sanitized_session_id>.jsonl`. Best-effort: errors are
//! logged and swallowed so tracing never breaks a live turn.

use std::path::PathBuf;

use librefang_types::message::TokenUsage;
use serde::Serialize;
use tracing::warn;

use super::fanout::AdvisorResult;

/// A persisted trace record for one MoA turn.
#[derive(Debug, Serialize)]
pub struct MoaTraceRecord {
    /// RFC 3339 timestamp.
    pub ts: String,
    /// Session identifier.
    pub session_id: String,
    /// Preset name used.
    pub preset: String,
    /// Per-advisor outcomes.
    pub references: Vec<AdvisorTrace>,
    /// Aggregator outcome.
    pub aggregator: AggregatorTrace,
}

/// Per-advisor trace entry.
#[derive(Debug, Serialize)]
pub struct AdvisorTrace {
    pub label: String,
    pub model: String,
    pub provider: String,
    pub temperature: f32,
    pub input_messages: usize,
    pub output: String,
    pub usage: TokenUsage,
    pub cost: f64,
}

/// Aggregator trace entry.
#[derive(Debug, Serialize)]
pub struct AggregatorTrace {
    pub label: String,
    pub model: String,
    pub provider: String,
    pub temperature: f32,
    pub input_messages: usize,
    pub output: String,
    pub streamed: bool,
    pub output_location: String,
}

impl AdvisorTrace {
    /// Build from an advisor result.
    pub fn from_result(result: &AdvisorResult) -> Self {
        Self {
            label: result.label.clone(),
            model: result.model.clone(),
            provider: result.provider.clone(),
            temperature: result.temperature,
            input_messages: result.input_messages,
            output: result.text.clone(),
            usage: result.usage,
            cost: result.cost,
        }
    }
}

/// Sanitize a session id into a safe file stem.
fn sanitize_session_id(session_id: &str) -> String {
    session_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Resolve the trace directory, defaulting to `~/.librefang/moa-traces/`.
pub fn resolve_trace_dir(configured: Option<&str>) -> PathBuf {
    if let Some(dir) = configured {
        return PathBuf::from(dir);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".librefang").join("moa-traces")
}

/// Persist a trace record on a blocking thread. Best-effort.
pub fn persist_trace(trace_dir: PathBuf, session_id: &str, record: MoaTraceRecord) {
    let file_stem = sanitize_session_id(session_id);
    tokio::task::spawn_blocking(move || {
        if let Err(e) = write_trace_line(&trace_dir, &file_stem, &record) {
            warn!(error = %e, "MoA trace persistence failed");
        }
    });
}

fn write_trace_line(
    trace_dir: &PathBuf,
    file_stem: &str,
    record: &MoaTraceRecord,
) -> std::io::Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(trace_dir)?;
    let path = trace_dir.join(format!("{file_stem}.jsonl"));
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let line = serde_json::to_string(record).map_err(std::io::Error::other)?;
    writeln!(file, "{line}")?;
    Ok(())
}
