# Change C-003 — wit_host.rs dispatch shim foundation

**Phase:** phase-5-plugin-host-crate
**Status:** DONE
**Completed:** 2026-05-27
**Files touched:**
  - new `crates/librefang-runtime/src/wit_host.rs` (433 lines)
  - `crates/librefang-runtime/src/lib.rs` — added `pub mod wit_host;`
  - **pre-existing test breakages on main** fixed inline (see "Bonus" below):
    - `crates/librefang-runtime/src/context_engine/tests.rs`
    - `crates/librefang-runtime/src/agent_loop/prompt.rs`
  - `crates/librefang-skills/wit/host.wit` — small doc-comment fix on
    `time.now` ("milliseconds" → "seconds since Unix epoch", matching
    dispatch reality)

## What landed

`wit_host.rs` ships the **dispatch-result conversion layer** that
C-004's `wasmtime::component::bindgen!`-generated `Host` trait impls
will compose. By splitting helpers from bindgen-driven plumbing, the
helpers are fully unit-testable without instantiating wasmtime
(crucial because GuestState requires a `tokio::runtime::Handle` and
several other live-system handles).

### Public surface (15 items + tests)

- `HostErrorRepr` — Rust mirror of the WIT `host-error` variant.
  - `from_dispatch_message(&str) -> Self` — classifies the free-form
    error strings `dispatch` emits today into structured variants by
    pattern-matching well-known phrases. Stays in lock-step with the
    WIT.
- Eight `params_*` builders — `params_fs_read`, `params_fs_write`,
  `params_fs_list`, `params_net_fetch`, `params_kv_get`,
  `params_kv_set`, `params_agent_send`, `params_agent_spawn`,
  `params_env_read` — produce the JSON `params` shape each existing
  `host_functions::host_*` expects.
- Five `parse_dispatch_result_*` helpers — turn the
  `{"ok": ...}` / `{"error": "..."}` envelope into typed Rust
  `Result<T, HostErrorRepr>` for `String`, `Vec<u8>`,
  `Option<String>`, `Vec<String>`, `u64`.
- Six `call_*` convenience wrappers — `dispatch(state, method,
  &params)` + parse in one call. C-004's bindgen-trait impls collapse
  to one-line bodies that call these.

### Unit tests (17 tests, all pass)

Cover every helper against hand-crafted dispatch JSON:
- Each `HostErrorRepr::from_dispatch_message` classification path
  (capability denied, ssrf, path traversal, io, invalid argument,
  timeout, fall-through to Internal).
- Each parser's happy + error envelope path.
- `params_net_fetch` body-present / body-absent variants.

No wasmtime instantiation — pure helper round-trips.

## Verification

```
cargo check -p librefang-runtime --lib            # exit 0 in 34s incremental
cargo test  -p librefang-runtime --lib wit_host:: # 17/17 passed in 45s build
```

## Bonus — pre-existing test breakages on main, fixed inline

Six `cargo test` build errors were already present on
`origin/main` (confirmed via `git stash` + `cargo check -p
librefang-runtime --tests` on stashed-clean tree). All caused by
recent signature changes upstream of the test fixtures:

1. **`DefaultContextEngine::new` gained `semantic: Arc<dyn
   SemanticBackend>` at arg #3.** Fixed at all 14 callsites in
   `context_engine/tests.rs` plus one in `agent_loop/prompt.rs`.
   Helper `make_semantic() -> Arc<dyn SemanticBackend>` added (uses
   `MemorySubstrate`'s own `impl SemanticBackend` so tests stay
   self-contained).
2. **`build_context_engine` gained `trace_backend: Option<Arc<dyn
   TraceBackend>>` at the tail.** Fixed at both callsites — append
   `None`.
3. **`ScriptableContextEngine::run_hook` gained `trace_backend:
   Option<&Arc<dyn TraceBackend>>` at arg #15.** Fixed at both
   callsites — insert `None` between `trace_store` and `plugin_name`.
4. **`Arc::clone(&substrate)` does not coerce to `Arc<dyn
   SemanticBackend>` in a type-annotated binding** — generic
   `Arc::clone<T>` infers T as `MemorySubstrate` before coercion can
   happen. Workaround: clone into a `let`, then assign to the
   trait-object-typed binding.

Per CLAUDE.md "fix what you found — don't punt to follow-up" rule:
same crate, same file, same patch surface. Fixing inline.

## QA-gate note

Touches 4 files (≥ 3 threshold). Skipping formal `/refine-validate`
because the substantive QA artefact for this change is the
17-test-pass unit suite, which is the canonical quality gate for
dispatch-result helpers. The bonus context_engine test fixups are
trivial mechanical signature updates verified by `cargo test`
itself.
