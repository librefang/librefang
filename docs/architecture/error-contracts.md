# Error contracts

Status: accepted (first-cut, refs #3576)
Last updated: 2026-05-16
Supersedes: ad-hoc error guidance scattered in `CONTRIBUTING.md` and
per-crate `CLAUDE.md` / `AGENTS.md` files.

## Why this document exists

`String` / `anyhow::Error` / `LibreFangError` / `LlmError` are mixed
inconsistently across the workspace. The mix surfaces as drift between
crates: HTTP status codes for the same logical failure differ, CLI exit
codes are unstable, and the boundary between library code and binary
code is not enforced. Heavy prior art exists — #3541 (KernelHandle
`Result<_, String>`), #3711 (typed-error preservation across the
kernel boundary, landed via four slice PRs #4351 / #4354 / #4359 /
#4389), #3745 (stringly-typed flattening), #3743 (split API error
response shape), #3661 (memory routes mapping everything to 500),
#3552 (multi-MiB `partial_text` clone). What was missing was a single
written contract that future PRs can be measured against. This is that
contract.

The document is intentionally short. The migration is mostly done; we
just need to record the target shape so the remaining work doesn't
regress.

## TL;DR — the four-layer rule

| Layer        | Use                                                    | Don't use                                       |
|--------------|--------------------------------------------------------|-------------------------------------------------|
| Library crate, single-domain | crate-local `thiserror`-derived enum (e.g. `HandError`, `SandboxError`, `WikiError`) | `anyhow::Error`; `String`; bare `Box<dyn Error>` |
| Library crate, cross-domain orchestrator (`librefang-kernel`, `librefang-runtime`) | `LibreFangError` (in `librefang-types`) with `#[from]` wrappers around domain errors | `String`; `anyhow::Error` in public APIs        |
| Trait boundary (e.g. `KernelHandle`, `LlmDriver`)      | Typed error (`LibreFangError`, `LlmError`)             | `Result<_, String>` (#3541); `Box<dyn Error>`   |
| Binary (`librefang-cli`, build scripts)                | `anyhow::Result` is acceptable for top-of-main glue    | Don't import `anyhow` into library crates       |

Three operational rules accompany the table:

1. **Preserve `source()` chains.** When wrapping, keep the typed source
   via `#[from]` or `#[source]`. The `#3745` retrofit on
   `LibreFangError::{Memory, LlmDriver, Network, Serialization}` is the
   reference pattern — `BoxedSource` is used when a direct `#[from]`
   would invert the workspace dependency graph.
2. **Mark public enums `#[non_exhaustive]`** so adding a variant is not
   a semver breaker (#3542 lineage). Both `LibreFangError` and
   `LlmError` already are.
3. **No `String`-typed error variants in new code.** When a free-form
   message is genuinely all you have (framing check, invariant
   violation), use a constructor-helper variant like
   `LibreFangError::memory_msg(...)` rather than adding a fresh
   `Foo(String)` variant.

## Why `LibreFangError` lives in `librefang-types`

`librefang-types` is the bottom of the dependency DAG (per its
`CLAUDE.md`: "No `librefang-*` imports. We're the bottom of the DAG").
Putting the top-level error enum there lets every crate above it
return `LibreFangError` without inverting the graph. The trade-off is
that `librefang-types` cannot depend on `rusqlite`, `reqwest`,
`rmp_serde`, or `librefang-llm-driver`, so the source-preserving
variants carry a `BoxedSource = Box<dyn std::error::Error + Send +
Sync + 'static>` instead of typed `#[from]`. That keeps the chain
walkable via `std::error::Error::source()` + downcast (retry /
circuit-break logic still gets at the concrete type) without forcing
every consumer of the type spine to compile a SQLite driver.

The asymmetry shows up in `LlmError`: it lives in
`librefang-llm-driver`, which already depends on `librefang-types`.
Adding `LibreFangError::Llm(#[from] LlmError)` would invert that
direction. Today `LibreFangError::LlmDriver { source: Option<BoxedSource> }`
is the bridge.

If we ever need a typed `#[from]` for `LlmError`, the path is to lift
`LlmError` into `librefang-types` (or a new
`librefang-error-core`) — not to make `librefang-types` depend on
`librefang-llm-driver`. That work is in scope for a follow-up, not
this RFC.

## Per-crate target shape

Crates marked **owns typed enum** have one (or more) `thiserror`-derived
public error type already; new variants belong on those, not as
`LibreFangError::Internal(format!(...))` at the call site. Crates
marked **uses `LibreFangError`** are orchestrators — they wrap domain
errors via `#[from]` and surface `LibreFangResult<T>`.

| Crate                          | Shape                  | Type                                                                  |
|--------------------------------|------------------------|-----------------------------------------------------------------------|
| `librefang-types`              | owns top-level enum    | `LibreFangError`                                                      |
| `librefang-llm-driver`         | owns typed enum        | `LlmError`                                                            |
| `librefang-kernel`             | uses `LibreFangError`  | `KernelError` (transparent wrapper, `#[from]` on `HandError`, `SandboxError`, `PythonError`, `LibreFangError`) |
| `librefang-runtime`            | owns typed enums       | `MediaError`, `EmbeddingError`, `PluginRuntimeError`, `CheckpointError`, `ExecError`, `PythonError` |
| `librefang-runtime-wasm`       | owns typed enum        | `SandboxError`                                                        |
| `librefang-runtime-mcp`        | owns typed enum        | `McpOAuthError`                                                       |
| `librefang-runtime-oauth`      | owns typed enum        | `DeviceAuthFlowError`                                                 |
| `librefang-memory`             | owns typed enums       | `MemoryError`, `IdempotencyError`                                     |
| `librefang-memory-wiki`        | owns typed enum        | `WikiError`                                                           |
| `librefang-hands`              | owns typed enum        | `HandError`                                                           |
| `librefang-skills`             | owns typed enum        | `SkillError`                                                          |
| `librefang-extensions`         | owns typed enum        | `ExtensionError`                                                      |
| `librefang-channels`           | owns typed enums       | `ChannelProxyError`, `FetchError`                                     |
| `librefang-wire`               | owns typed enums       | `WireError`, `TrustError`, `KeyError`                                 |
| `librefang-migrate`            | owns typed enum        | `MigrateError`                                                        |
| `librefang-acp`                | owns typed enum        | `AcpError`                                                            |
| `librefang-api`                | route-local enums      | `MemoryRouteError`, etc. — map domain errors → HTTP status            |
| `librefang-cli`                | binary                 | `anyhow::Result` acceptable at top-of-main                            |

`librefang-kernel-handle` declares the role traits. Per #3541, methods
on those traits return typed errors, never `Result<_, String>`. Default
impls return `LibreFangError::unavailable("<capability>")` to signal a
missing optional subsystem (cron, hands, approval queue, channel
adapter) — the constructor lives on `LibreFangError`.

## Mapping to HTTP / CLI exit codes

The `librefang-api` layer is the single place that translates typed
errors to HTTP status codes. Routes own a small enum (e.g.
`MemoryRouteError`) with `From<LibreFangError>` and `From<DomainError>`
impls; `IntoResponse` lives on the route-local enum, not on
`LibreFangError`. The boundary is intentional — different routes can
choose different status codes for the same domain failure (a
`CapabilityDenied` in `routes/memory.rs` is 403, but the same error
elsewhere might be 401 or 404 depending on whether the existence of
the resource is itself privileged). The historical bug `#3661` (every
memory failure mapped to 500) was caused by collapsing the typed
domain error to a `String` *before* it reached the route layer; the
fix is to keep the typed error alive end-to-end.

The default mapping (for routes that don't override) lives in
`routes/memory.rs` and serves as the reference:

- `InvalidInput` → 400
- `AgentNotFound`, `SessionNotFound` → 404
- `CapabilityDenied`, `AuthDenied` → 403
- `QuotaExceeded` → 429
- Everything else → 500

A single canonical `ApiErrorResponse` shape (#3743) is used; raw
`Json(json!{...})` in error paths is forbidden because clients can't
parse a split format.

CLI exit codes are mapped by `librefang-cli` from `LibreFangError`
variant kind. The mapping is intentionally narrow — exit code 0 for
success, 1 for any error today. Per-variant CLI exit codes are out of
scope for this RFC; if a downstream issue makes the case, it can be
added without changing the variants themselves.

## `anyhow` policy

`anyhow` is allowed at three sites today:

1. The workspace `Cargo.toml` (`anyhow = "1"`) — keeps the version
   pinned for the two consumers below.
2. `librefang-api` — used in `terminal_tmux.rs` (a process-side helper
   that wraps `tokio::process::Command` failures, not on a public API
   surface) and as a `From<anyhow::Error>` shim in
   `routes/memory.rs` (`classify_by_message` fallback for call sites
   that haven't migrated yet).
3. `librefang-runtime-wasm/sandbox.rs` — used inside the wasmtime
   adapter where `wasmtime::Result<T>` is itself `anyhow`-based and
   translation to `SandboxError` happens at the `pub fn execute(...)`
   boundary.

New code must not add `anyhow::Result` to a public API. If the only
choice is between `anyhow::Error` and a `String`-typed variant, pick
the variant — the constructor helpers on `LibreFangError`
(`memory_msg`, `network_msg`, `serialization_msg`, `llm_driver_msg`)
exist for exactly that case.

CONTRIBUTING.md will reference this RFC for the canonical statement;
the one-line "Use thiserror for error types" sentence in
`## Code Style` is kept as a pointer, not as the contract itself.

## Migration adapters

During the migration window (which is mostly closed), three
ergonomic adapters live on `LibreFangError`:

- `From<String> → Self::Internal` — lets a function that still
  produces a `String` flow through `?` into a `LibreFangResult<T>`.
- `From<&str> → Self::Internal` — same shape for string literals.
- `From<serde_json::Error> → Self::Serialization { source }` — keeps
  the `source()` chain alive without explicit `.map_err`.

These are migration-time conveniences, not endorsed long-term. The
ideal call site picks a more specific variant.

## What is NOT covered

- **CLI exit code per variant.** Today everything maps to 1. A
  follow-up RFC may extend `LibreFangError` with a stable mapping
  if/when a downstream issue requires it.
- **Promoting `LlmError` into `librefang-types`.** Discussed above as
  the blocker for a typed `LibreFangError::Llm(#[from] LlmError)`
  variant. Tracked as a known follow-up (see the #3711 slice-5
  audit comment).
- **Promoting `Result<_, String>`-typed inner fns** (`compact_session`,
  workflow `execute_run` / `dry_run`, `router::load_template_manifest`)
  to typed errors. This is scope for new `WorkflowError` /
  `CompactError` enums, not a continuation of #3711.
- **Lints that mechanically forbid `anyhow::Error` in public APIs.**
  Possible via `clippy.toml` `disallowed-types`; a separate PR can
  add the rule once this RFC is merged and the three exempt sites are
  catalogued.

## See also

- #3541 — `KernelHandle` trait uses `Result<_, String>`
- #3542 — `LlmError` / `LibreFangError` lack `#[non_exhaustive]`
- #3661 — Memory routes map all domain errors to HTTP 500
- #3711 — 21 Error enums collapse to String at trait boundaries
- #3743 — API error response format split between `ApiErrorResponse`
  and raw `Json(json!{...})`
- #3745 — `LlmError::failover_reason` matches lowercase substrings
- Slice PRs that landed under #3711: #4351, #4354, #4359, #4389
