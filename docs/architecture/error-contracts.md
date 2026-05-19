# Error contracts

LibreFang has accreted three overlapping error styles across 24 crates: stringly-typed
`Result<String, String>` at the tool-runner boundary, structured `thiserror` enums in
the kernel / runtime / drivers, and a thin sprinkle of `anyhow` in glue code. This RFC
defines the target shape, the migration order, and the explicit per-PR scope for the
work tracked by issue [#3576](https://github.com/anthropics/librefang/issues/3576).

Refs: #3576. Earlier slices: #3711 (typed kernel→hand/sandbox/python boundaries),
#3745 (typed source chains on `LibreFangError::{Memory,LlmDriver,Network,Serialization}`),
#3541 (`LibreFangError::Unavailable` for missing-subsystem signalling).

## Current state (verified)

houko's audit on the [#3576 thread](https://github.com/anthropics/librefang/issues/3576)
(base `c0b59d65`, 2026-05-18) measured:

| Surface                                                  | Sites |
|----------------------------------------------------------|------:|
| `crates/librefang-runtime/src/tool_runner/**`            |   79  |
| `crates/librefang-channels/src/bridge.rs`                |   24  |
| `crates/librefang-api/src/channel_bridge.rs`             |   16  |
| `crates/librefang-runtime/src/browser_tools.rs`          |   10  |
| `crates/librefang-runtime/src/web_search.rs`             |    9  |
| Remainder (`librefang-cli`, scattered)                   |  ~48  |
| **Total `Result<String, String>` sites**                 | **186** |

Re-counted on this branch:

```
$ rg -c 'Result<String, String>' crates/                # 186
$ rg -lc 'use anyhow|anyhow::' crates/ | wc -l          #   4 files
$ rg -lc 'thiserror::Error' crates/ | wc -l             #  24 files
```

`anyhow` is contained — already only in 4 library files; the rule below
re-affirms that bound.

## Target convention

### Per-crate posture

| Crate                                  | Error style                                       | Notes                                                                                             |
|----------------------------------------|---------------------------------------------------|---------------------------------------------------------------------------------------------------|
| `librefang-cli`, `librefang-desktop`, `xtask`             | `anyhow::Result<T>` at the `main` boundary       | Binaries. Application-layer convenience.                                                          |
| `librefang-api`, `librefang-runtime`, `librefang-kernel`  | `Result<T, LibreFangError>` (or domain error)    | Library crates. **No `anyhow::Result` in public signatures.**                                     |
| `librefang-llm-driver`                                    | `Result<T, LlmError>`                            | Stable provider-classification surface. `LlmError` is the contract — retry/cooldown/failover all pattern-match on it. Do not flatten. |
| `librefang-channels`, `librefang-hands`, `librefang-skills`, `librefang-memory`, `librefang-runtime-{audit,mcp,media,sandbox-docker}` | Domain `*Error` enum (re-exported), convertible to `LibreFangError` | Mirrors the post-#3711 pattern: typed at the source, preserved on `source()` chains via `BoxedSource`. |
| `librefang-types`                                         | `LibreFangError` is the application enum         | Hosts the shared top-level error. No transport / storage deps.                                    |

### Tool-runner shape (new)

The tool-runner's `Result<String, String>` returns lose every kind: a "Missing
'X' parameter" (caller bug, should be a 400-class hint) is indistinguishable
from "Provider 'openai' is in cooldown" (transient infra, should be retried)
once they pass through `format!("Error: {err}")` at
`tool_runner/dispatch.rs:1282`. The next caller up (`agent_loop`) then has to
substring-match to recover the kind.

The replacement type:

```rust
// crates/librefang-runtime/src/tool_runner/error.rs
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ToolError {
    /// A required input parameter is missing or wrong-typed.
    /// Maps to "the LLM called the tool wrong — re-prompt with the schema".
    #[error("Missing parameter '{0}'")]
    MissingParameter(&'static str),

    #[error("Invalid parameter '{name}': {reason}")]
    InvalidParameter { name: &'static str, reason: String },

    /// Tool requires a runtime capability that isn't wired (kernel handle,
    /// caller agent id, web context, …). Mirrors `LibreFangError::Unavailable`.
    #[error("{0} unavailable")]
    Unavailable(&'static str),

    /// The target resource was not found OR the caller does not own it.
    /// One variant on purpose: leaking the distinction is a security smell.
    #[error("{kind} '{id}' not found")]
    NotFound { kind: &'static str, id: String },

    /// The caller lacks the right to perform the operation.
    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    /// A downstream subsystem (kernel handle, skill loader, MCP server) failed.
    /// Carries the upstream error on the `source()` chain so callers walking
    /// it can downcast back to `LibreFangError` / `KernelError` etc.
    #[error("Upstream error: {message}")]
    Upstream {
        message: String,
        #[source]
        source: Option<crate::error::BoxedSource>,
    },

    /// Serialization of the tool's response (json) failed. Distinct from
    /// `Upstream` so the agent loop can surface "the tool ran but I couldn't
    /// hand you the answer" rather than "the tool failed".
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Internal invariant violation. Use sparingly — prefer one of the above.
    #[error("Internal error: {0}")]
    Internal(String),
}

pub type ToolResult<T = String> = Result<T, ToolError>;
```

`From<ToolError> for LibreFangError` provides the bridge for callers that want
to bubble through the kernel boundary:

```rust
impl From<ToolError> for LibreFangError {
    fn from(e: ToolError) -> Self {
        match e {
            ToolError::MissingParameter(_) | ToolError::InvalidParameter { .. } => {
                LibreFangError::InvalidInput(e.to_string())
            }
            ToolError::Unavailable(cap) => LibreFangError::unavailable(cap),
            ToolError::NotFound { .. } => LibreFangError::Internal(e.to_string()),
            ToolError::PermissionDenied(_) => LibreFangError::CapabilityDenied(e.to_string()),
            ToolError::Upstream { message, source } => LibreFangError::ToolExecution {
                tool_id: "unknown".to_string(),
                reason: source
                    .as_ref()
                    .map(|s| s.to_string())
                    .unwrap_or(message),
            },
            ToolError::Serialization(msg) => LibreFangError::serialization_msg(msg),
            ToolError::Internal(msg) => LibreFangError::Internal(msg),
        }
    }
}
```

**Naming convention recap.** Domain errors live next to the trait they serve
(`HandError` next to the hands API, `SandboxError` in the sandbox crate). The
shared application enum is `LibreFangError` in `librefang-types`. Per-crate
wrapper enums (one variant per typed source + transparent `LibreFangError`)
exist when the crate needs to lift sub-crate errors at its API boundary —
`KernelError` (which is **NOT** dropped, see below) is the canonical example.

### Why `KernelError` stays

The original #3576 thread suggested dropping `KernelError` as a "2-variant
pass-through". That measurement is stale. After #3711 landed (slices 1, 2, 4
of 21), `KernelError` carries **5 typed variants**:

```
$ grep -c '#\[error' crates/librefang-kernel/src/error.rs
5
```

— `LibreFang`, `Hand`, `WasmSandbox`, `Python`, `Backpressure`. The
`Hand` / `WasmSandbox` / `Python` variants exist *specifically* so upstream
HTTP / CLI callers can map `HandError::AlreadyActive` → 409,
`SandboxError::FuelExhausted` → 408 / quota, `PythonError::Timeout` → 408,
etc., without re-parsing strings. The tests in
`crates/librefang-kernel/src/error.rs` (`hand_error_kind_survives_kernel_boundary`,
`sandbox_error_kind_survives_kernel_boundary`,
`python_error_kind_survives_kernel_boundary`) are the contract.

**Dropping `KernelError` would un-do #3711.** It stays.

What this RFC *does* drop from the issue body's wish-list:

- `KernelError::BootFailed(String)` was already dropped in an earlier slice
  of #3576 — see the comment on `LibreFangError::BootFailed` and its
  regression test `boot_failed_display_matches_dropped_kernel_variant`.
- No further kernel-side wrapper variants are scheduled for removal.

### `anyhow` ban

Add (in a follow-up PR) a `clippy.toml` at the workspace root:

```toml
disallowed-types = [
  { path = "anyhow::Result", reason = "Use Result<T, LibreFangError> in library crates. anyhow is for binaries (librefang-cli, librefang-desktop, xtask) only." },
  { path = "anyhow::Error", reason = "Use LibreFangError in library crates." },
]
```

The 4 remaining `use anyhow` library uses must be migrated first; otherwise
the lint fires immediately. **This PR introduces the `clippy::disallowed_types`
mechanism in scope-discovery mode only** — see "What this PR ships" below.

## Migration order

Smallest-blast-radius first. Each module is independently reviewable.

1. **`tool_runner/cron.rs`** — 3 fns, 9 sites, 1 caller (`dispatch.rs`).
   The canonical first migration. **Done in this PR.**
2. **`tool_runner/{event,artifact,goal,spill,notify,sandbox,system}.rs`** —
   small, mostly-pure (no kernel handle plumbing beyond `require_kernel`).
3. **`tool_runner/{memory,a2a,task,process}.rs`** — kernel-handle-heavy, need
   the `From<LibreFangError>` impl exercised.
4. **`tool_runner/{shell,knowledge,image,meta,canvas,wiki,web_legacy,hand}.rs`** —
   the long tail. Each PR migrates one file + adds tests.
5. **`tool_runner/{a2a,fs,dispatch}.rs` and the rest** — last, because
   `dispatch.rs` is the boundary that finally upgrades from
   `match result { Ok(s) => …, Err(s) => … }` to
   `match result { Ok(s) => …, Err(e) => match e.kind() … }`.
6. **`librefang-channels::bridge`** — 24 sites. After tool_runner is done so the
   shared `ToolError` shape is settled.
7. **`librefang-api::channel_bridge`** — 16 sites. Same reason.
8. **`librefang-runtime::{browser_tools,web_search}`** — 19 sites total.
9. **`librefang-cli` remainder** — last, narrowest blast radius (binary only).
10. **`clippy::disallowed_types` enforcement broadens** to every library crate
    as each falls below the threshold.

Estimated runway: ~10–14 PRs after this one. Tracked under the umbrella issue
#3576; one follow-up issue per group is fine.

## What this PR ships

1. This document.
2. `crates/librefang-runtime/src/tool_runner/error.rs` (new) — the `ToolError`
   enum, `ToolResult<T>` alias, `From<ToolError> for LibreFangError` impl,
   constructor unit tests.
3. `crates/librefang-runtime/src/tool_runner/cron.rs` — migrated from
   `Result<String, String>` to `Result<String, ToolError>`. Direct unit
   tests added (none existed before).
4. `crates/librefang-runtime/src/tool_runner/dispatch.rs` — the three
   `cron_*` arms call `.map_err(|e: ToolError| e.to_string())` at the
   dispatch-side boundary so the change does not cascade across the other
   ~180 sites in this PR. (That cascade is the work of follow-up PRs.)
5. Unit tests on `cron.rs` covering each new `ToolError` variant returned
   by the three cron fns. Note that the rendered error *strings* the LLM
   sees change with this migration (e.g. `"Missing 'job_id' parameter"`
   becomes `"Missing required parameter 'job_id'"`) — that is the whole
   point of the structured shape and is desirable, but it means downstream
   code that substring-matched the legacy phrasing must be re-pointed at
   the structured variant. The cron arm has only one caller (`dispatch.rs`),
   which renders the error via `format!("Error: {err}")` without parsing it,
   so no string-matching consumer exists.

## What this PR explicitly does NOT ship

- The other 177 `Result<String, String>` sites. Each is the subject of a
  follow-up PR per the migration order above.
- The `clippy::disallowed_types` enforcement. Introducing it before
  migration is finished would create a wall of opt-outs; better to switch
  on after the migration order completes per-crate.
- Any change to `KernelError` (see "Why `KernelError` stays" above).
- Any change to `LlmError` (the retry path pattern-matches on the
  concrete variants; reshaping is out of scope and not desired).
- Any change to `LibreFangError`. The `From<ToolError>` impl is additive.

## Verification

The migration is verified at two levels per slice:

- Compile-clean: `cargo check --workspace --lib`,
  `cargo clippy --workspace --all-targets -- -D warnings`.
- Behaviour-clean: scoped `cargo test -p librefang-runtime` (and the
  destination crate's scoped tests for slices ≥ 6).

Per `CLAUDE.md`, workspace-wide `cargo test` (unscoped) is forbidden in this
workspace; CI runs that lane.
