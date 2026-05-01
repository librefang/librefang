//! LLM driver implementations for LibreFang runtime.
//!
//! Re-exports `librefang_llm_driver` as `llm_driver` so the existing
//! `crate::llm_driver::*` paths inside driver source keep working.

pub use librefang_llm_driver as llm_driver;
pub use librefang_llm_driver::llm_errors;
pub mod backoff;
pub use librefang_llm_driver::FailoverReason;
pub mod credential_pool;
pub mod drivers;
pub mod rate_limit_tracker;
pub mod shared_rate_guard;
pub mod think_filter;
pub mod utf8_stream;

pub use credential_pool::{
    new_arc_pool, ArcCredentialPool, CredentialPool, PoolStrategy, PooledCredential,
};
pub use drivers::fallback_chain::{ChainEntry, FallbackChain};
pub use rate_limit_tracker::{RateLimitBucket, RateLimitSnapshot};
