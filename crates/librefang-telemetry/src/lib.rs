//! OpenTelemetry + Prometheus metrics instrumentation for LibreFang.
//!
//! This crate provides centralized telemetry (metrics + tracing) for monitoring
//! the LibreFang Agent OS across all 14 crates.

pub mod config;
pub mod metrics;

pub use metrics::{get_http_metrics_summary, normalize_path, record_http_request};
