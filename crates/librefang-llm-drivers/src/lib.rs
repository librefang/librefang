//! LLM driver implementations for LibreFang runtime.
//!
//! Re-exports `librefang_llm_driver` as `llm_driver` so the existing
//! `crate::llm_driver::*` paths inside driver source keep working.

pub use librefang_llm_driver as llm_driver;
pub use librefang_llm_driver::llm_errors;
pub mod backoff;
pub use librefang_llm_driver::FailoverReason;
pub mod drivers;
pub mod rate_limit_tracker;
pub mod think_filter;

pub use drivers::fallback_chain::{ChainEntry, FallbackChain};
pub use rate_limit_tracker::{RateLimitBucket, RateLimitSnapshot};
