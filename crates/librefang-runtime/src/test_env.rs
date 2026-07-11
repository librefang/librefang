//! Shared lock for tests that read or mutate process-global environment variables.
//!
//! Per-module `ENV_LOCK` statics don't compose: a writer in one module races a reader in another when `cargo test` runs threads in a single process (observed: `model_catalog`'s `GOOGLE_API_KEY` setter vs `tts`'s provider detection).
//! Every env-touching test across the crate must take this one lock.
//! nextest runs each test in its own process, so this only matters under plain `cargo test`.

pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
