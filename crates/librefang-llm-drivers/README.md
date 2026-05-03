# librefang-llm-drivers

Concrete LLM provider drivers for [LibreFang](https://github.com/librefang/librefang).

Implements the `librefang-llm-driver` trait for Anthropic, OpenAI,
Gemini, Groq, Ollama, and other providers. Adds a credential pool, a
fallback chain, a shared rate-limit tracker, retry/backoff with
`Retry-After`, and stream backpressure / UTF-8 handling.

## Public API entry points

- `drivers::*` — per-provider driver modules.
- `drivers::fallback_chain::FallbackChain`, `ChainEntry` — multi-provider
  failover composition.
- `credential_pool::{ArcCredentialPool, CredentialPool, PoolStrategy,
  PooledCredential, new_arc_pool}` — shared API-key pool.
- `rate_limit_tracker::{RateLimitBucket, RateLimitSnapshot}` — provider
  rate-limit observability.
- `backoff`, `retry_after`, `shared_rate_guard`,
  `stream_backpressure`, `think_filter`, `utf8_stream` — supporting
  utilities.
- Re-exports: `llm_driver` (trait + error types from
  `librefang-llm-driver`), `llm_errors`, `FailoverReason`.

## Key dependencies

`librefang-types`, `librefang-llm-driver`, `librefang-http`,
`librefang-runtime-oauth`, `reqwest`, `tokio`, `serde`.

See the [workspace README](../../README.md).
