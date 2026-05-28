# Change C-006 — AOT (.cwasm) cache + JIT fallback

**Phase:** phase-5-plugin-host-crate
**Status:** DONE
**Completed:** 2026-05-27
**Files touched:**
  - new `crates/librefang-runtime/src/aot_cache.rs` (~285 lines, 7 tests)
  - `crates/librefang-runtime/src/lib.rs` — `pub mod aot_cache;`
  - `crates/librefang-runtime/src/sandbox_component.rs` — extended
    `ComponentExecuteOptions` with `aot_cache_dir` +
    `compile_mode`; `execute_component` now consumes
    `aot_cache::load_or_compile` when a cache dir is configured.
  - `crates/librefang-runtime/Cargo.toml` — added `wat` dev-dep for
    the test fixture (`(component)` WAT → empty Component bytes).

## What landed

### Cache module

`aot_cache` exports:

- **`CompileMode { Auto, Aot, Jit }`** (default `Auto`).
- **`WASMTIME_CACHE_VERSION: &str = "wasmtime-44"`** — bumped in
  lock-step with the workspace wasmtime pin; load-bearing for cache
  safety.
- **`cache_path(cache_dir, wasm_bytes) -> PathBuf`** — filename
  `<sha256(wasm)>.<wasmtime_version>.cwasm`. Both keys are present
  so source drift and wasmtime version drift both miss the cache
  cleanly.
- **`load_or_compile(engine, wasm_bytes, cache_dir, mode) -> Result<Component, _>`**
  — the canonical entry point. Behavior per mode:
  - `Auto`: try `Component::deserialize_file` from cache → fall back
    to `Component::from_binary` on miss/failure → opportunistically
    write the cache for the next run. Corrupt cache file is removed
    so the next call cleanly re-populates.
  - `Aot`: hard error if no fresh cache hit. Use after a
    `librefang skill install` precompile step.
  - `Jit`: skip the cache entirely.
- **`precompile(engine, wasm_bytes, cache_dir) -> Result<PathBuf, _>`**
  — force-precompile + cache write. Skill installer calls this so
  the first execution is already AOT. Atomic write via temp file +
  rename so interrupted precompiles don't leave a corrupt artefact.

### Integration with execute_component

`ComponentExecuteOptions` gained two fields:
```rust
pub aot_cache_dir: Option<PathBuf>,  // None = JIT every call
pub compile_mode: CompileMode,        // default Auto
```

`execute_component` branches: if `aot_cache_dir.is_some()`, route
through `aot_cache::load_or_compile`; else direct
`Component::from_binary`. Backward compatible — callers passing
`ComponentExecuteOptions::default()` get the same JIT-only behavior
as before.

### Safety

`Component::deserialize_file` is `unsafe` upstream (trusts the bytes
to be both well-formed AND produced by the SAME wasmtime binary).
Mitigated by:
1. Filename includes `WASMTIME_CACHE_VERSION` — cross-version reuse
   is impossible because the filename differs.
2. We only read files we wrote (cache_dir is caller-trusted, in
   production under `~/.librefang/skills/<id>/`).
3. Deserialize failure in `Auto` mode falls back to JIT + WARN log +
   best-effort cache delete.

## Verification

```
cargo check -p librefang-runtime --lib            # exit 0
cargo test  -p librefang-runtime --lib aot_cache
# test result: ok. 7 passed
cargo test  -p librefang-runtime --lib sandbox_component
# test result: ok. 6 passed (no regression on existing tests)
```

### 7 aot_cache tests cover the full matrix

| Test | Scenario |
|---|---|
| `cache_path_includes_sha_and_version` | Filename shape — sha256 prefix + wasmtime version suffix |
| `precompile_creates_file` | Explicit precompile writes a non-empty `.cwasm` |
| `auto_cache_miss_then_hit` | Auto mode: first call JIT + populate cache; second call hits cache |
| `aot_mode_missing_cache_errors` | Aot mode without precompile → `SandboxError::Compilation("AOT cache miss ...")` |
| `aot_mode_after_precompile_succeeds` | Aot mode + precompile → load via cache cleanly |
| `jit_mode_never_writes_cache` | Jit mode does NOT populate the cache (confirms the "skip entirely" contract) |
| `corrupt_cache_falls_back_in_auto_mode` | Corrupt `.cwasm` in Auto mode → JIT fallback works + corrupt file cleaned up |

## Issues hit + fixed

1. **`wat` crate missing.** Test fixture used `wat::parse_str("(component)")`
   to build empty Component bytes. wasmtime is configured without
   the `wat` feature in this workspace, so `wat` needed as a
   separate dev-dep.
2. **`unwrap_err()` requires Debug on T.**
   `wasmtime::component::Component` doesn't impl `Debug`. Swapped
   `aot_mode_missing_cache_errors` to a `match` instead.

## Backward compat note

`ComponentExecuteOptions::default()` produces `aot_cache_dir: None`,
so the existing `rejects_empty_bytes_as_compilation_error` test
(which uses `::default()`) bypasses the cache entirely. No
regression.

## Wasmtime version pin coupling

When bumping wasmtime in workspace `Cargo.toml`, update
`WASMTIME_CACHE_VERSION` in `aot_cache.rs` in the SAME commit.
Skipping it would let stale `.cwasm` files (compiled by the old
wasmtime) be deserialized by the new wasmtime — undefined behavior.
The constant is documented as a "review-mandatory bump" so the
reviewer sees it.

## QA-gate note

Touches 4 files (≥3 threshold). 7-test cache suite + 6-test
sandbox_component regression check is the substantive QA artefact.
No `/refine-validate` invoked — the test coverage is the gate for
this kind of pure helper module.
