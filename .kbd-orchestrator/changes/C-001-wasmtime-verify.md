# Change C-001 — wasmtime workspace version verification (was: bump 44→45)

**Phase:** phase-5-plugin-host-crate
**Status:** DONE — collapsed to verification, no version bump
**Completed:** 2026-05-27
**Files touched:** none

## What landed

Per the post-merge adjustment recorded in `progress.json`, the wasmtime
44 → 45 bump originally planned for this change became unnecessary
once PR #55 enabled the relevant feature flags on the v44 line.
Current `Cargo.toml`:

```toml
wasmtime = { version = "44", features = [
    "cranelift",
    "component-model",
    "cache",
    "parallel-compilation",
    "async",
] }
```

C-001 became a pure verification: confirm `librefang-runtime` builds
with these features. `cargo check -p librefang-runtime --lib` ran exit
0 in 7m 49s (cold workspace, no warnings about wasmtime API drift).

`wasmtime::component::{Component, Linker, bindgen}` are in scope for
C-003+ via the `component-model` feature.

## Implication for the plan

C-002 onward proceeds as planned. The "wasmtime bump is the one
unavoidable cross-cut" risk from the upstream-sync-hygiene section is
defused — no version bump means no merge conflict surface on the
workspace `Cargo.toml` against future upstream wasmtime work.

A future C-001-redux ("bump to wasmtime 45") may still happen later in
phase 5 if a 45-only API is needed (e.g. some Component Model
ergonomics landed only post-44). Defer until forced.
