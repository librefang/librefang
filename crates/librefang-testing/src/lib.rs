//! # librefang-testing — Test Infrastructure
//!
//! Provides mock infrastructure for unit testing API routes without starting a full daemon.
//!
//! ## Main Components
//!
//! - [`MockKernelBuilder`] — Builds a minimal `LibreFangKernel` (in-memory SQLite, temp directory)
//! - [`MockLlmDriver`] — Configurable LLM driver mock with call recording and canned responses
//! - [`TestAppState`] — Builds an `AppState` suitable for axum testing
//! - Helper functions — `test_request`, `assert_json_ok`, `assert_json_error`

pub mod helpers;
pub mod mock_driver;
pub mod mock_kernel;
pub mod test_app;

pub use helpers::{assert_json_error, assert_json_ok, test_request};
pub use mock_driver::{FailingLlmDriver, MockLlmDriver};
pub use mock_kernel::{test_catalog_baseline, CatalogSeed, MockKernelBuilder};
pub use test_app::TestAppState;

#[cfg(test)]
mod tests;
