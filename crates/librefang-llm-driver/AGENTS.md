# librefang-llm-driver — AGENTS.md

Telegraph style. Short sentences. One idea per line.
See repo-root `CLAUDE.md` for cross-cutting rules.

## Purpose

The trait + error types. Defines the LLM driver interface. **No concrete provider implementations live here.**
Provider impls (anthropic, openai, gemini, groq, …) are in the sibling `librefang-llm-drivers` crate (note the trailing `s`).

## Boundary

- Owns: `LlmDriver` trait (or whatever the canonical name is — see `lib.rs`), `LlmError` enum (`llm_errors.rs`), shared driver-side types.
- Does NOT own: any specific provider's HTTP wiring, retry strategy, prompt formatting. Those go to `librefang-llm-drivers`.
- Depends on: `librefang-types`, `serde`, `thiserror`. Should remain dep-light.

## Why two crates

Splitting trait from impls (since this crate's inception) lets test crates depend on the trait alone — no transitive `reqwest` / TLS / vendored libs pulled into a unit test build. Don't merge the two crates "for simplicity".

## Adding a new driver

The new driver goes in `librefang-llm-drivers`, NOT here. Implementations of `LlmDriver` should not require touching this crate at all unless:

- A new method is genuinely needed on the trait (very rare — discuss in an issue first).
- A new error variant is needed in `LlmError`. Add it as a typed variant; preserve the `source()` chain (#3745).
- A new shared driver-side type is needed.

## Error types

`LlmError` is the LLM-specific error enum surfaced through the `LlmDriver` trait. Per #3541 / #3711, we're migrating callers away from `Result<_, String>` collapse at trait boundaries; **don't** add a `String` catch-all variant here.

`LlmError::*` should compose well: each variant should answer "is this retryable?", "is this a quota / auth issue?", "did the model produce something bogus?". `is_retryable()` and friends live on the enum.

Partial responses on streaming errors must be preserved (#3552 lineage) — the `Partial` variant carries the bytes-so-far so callers can settle metering.

## Testing

- Trait conformance is exercised by mock drivers in `librefang-testing` (see `MockKernelBuilder`).
- Don't add HTTP fixture tests here — those belong in `librefang-llm-drivers` next to the implementation under test.

## Taboos

- No `reqwest`, no TLS deps, no vendored client SDKs. Pure trait + types.
- No `librefang-llm-drivers` import (would be circular).
- No `librefang-runtime` / `librefang-kernel` imports. Driver trait should stand alone.
- No new `String`-typed error variants. Use a structured enum field.
- No `Box<dyn Error>` in trait return types. We have `LlmError`; use it.
