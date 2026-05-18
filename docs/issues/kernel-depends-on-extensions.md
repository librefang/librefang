# [High] Kernel architecture refactor roadmap — extension coupling, KernelApi god-object, typed errors, driver-crate coupling, field bloat

**Severity:** High
**Category:** Architecture
**Status:** Merges 5 earlier issues into a single tracking item. This is a roadmap PR series, not a one-shot fix.

## Sub-findings rollup

| Origin | Severity | Description |
|--------|----------|-------------|
| this | High | Kernel crate imports `librefang-extensions` — violates CLAUDE.md's "runtime / kernel must not depend on extensions" rule (should go through the `KernelHandle` trait inversion instead) |
| KernelApi god-object | High | `KernelApi` is a god-object: 1457 lines / 18 modules / heavily mixed responsibilities |
| typed errors | High | `KernelApi` returns `Result<_, String>` in 12 places — should be typed errors |
| AppState bloat | Medium | `AppState` is a 24-field god-bag; every new field forces a re-wire of the entire axum router |
| llm-drivers concrete dep | Medium | Kernel imports concrete `librefang-llm-drivers` (not HTTP primitives), contradicting the driver-trait split |
| LibreFangKernel fields | Medium | `LibreFangKernel` has 30+ fields, blocking the #3746 split |

## Affected files

- `crates/librefang-kernel/Cargo.toml` (dependency graph)
- `crates/librefang-kernel/src/kernel_api.rs` (1457 lines — god-object)
- `crates/librefang-kernel/src/kernel/mod.rs` (30+ fields)
- `crates/librefang-api/src/server.rs` (AppState 24 fields)
- `crates/librefang-extensions/src/*` and its boundary with the kernel
- `crates/librefang-kernel/src/kernel/pooled_driver.rs` (concrete `llm-drivers` import)
- Cross-boundary types: `ExtensionError`, `CredentialVault`, `McpCatalog`, `HealthMonitor`

## Why merged

All 6 are facets of the same architectural problem — "kernel is bloating at a single point, and its boundaries with the surrounding crates have become muddy." No single PR can fix all of this; tracking it as a roadmap is more useful than 6 independent issues.

## Refactor roadmap

### Phase 1 — Boundary inversion (this, llm-drivers concrete dep)
- Invert `librefang-kernel → librefang-extensions`: define a kernel-internal trait (`ExtensionHandle`, analogous to `KernelHandle`); the extensions crate implements that trait and is injected at boot.
- Push shared types (`CredentialVault`, `McpCatalog`, `HealthMonitor`, `ExtensionError`) down into `librefang-types` or a new `librefang-shared-extensions` crate.
- Extend the `librefang-kernel-handle` pattern to `llm-drivers`: kernel depends only on `librefang-llm-driver` (the trait crate); the concrete `librefang-llm-drivers` is wired only at the binary crate.

### Phase 2 — Typed errors (typed errors)
- Replace the 12 `Result<_, String>` sites with a `thiserror`-derived `KernelApiError`; keep a `String` fallback variant during the transition.

### Phase 3 — God-object split (KernelApi god-object, LibreFangKernel fields)
- Split `KernelApi` by domain: `KernelApi::sessions`, `KernelApi::agents`, `KernelApi::triggers`, `KernelApi::credentials`, etc., each < 300 lines.
- Group `LibreFangKernel`'s 30+ fields into subsystem structs (`SessionSubsystem` / `TriggerSubsystem` / etc.), following the #3746 pattern.

### Phase 4 — AppState consolidation (AppState bloat)
- Group the 24 `AppState` fields into sub-domains (`AppState::kernel`, `AppState::auth`, `AppState::dashboard`, etc.); the axum router injects sub-slices rather than the whole god-bag.

## Tests / acceptance

- **Phase 1**: `cargo tree -p librefang-kernel | grep librefang-extensions` returns empty (same for `librefang-llm-drivers`). Add a `forbidden-deps.toml` or `cargo-deny` rule to lock the invariant in.
- **Phase 2**: `rg 'Result<.*, String>' crates/librefang-kernel/src/kernel_api.rs` → 0 matches.
- **Phase 3**: `wc -l crates/librefang-kernel/src/kernel_api.rs` < 500; `grep '^pub struct LibreFangKernel' -A 60` shows ≤ 12 subsystem fields.
- **Phase 4**: `AppState` has ≤ 8 fields (top-level groupings).

## Notes

This is not a one-PR effort; break it into 4 phased PRs, each independently reviewable with the acceptance criteria above. This issue is the roadmap tracker.

## Related

Concrete bugs in the same subsystems are tracked separately (e.g. [pooled-driver-no-invalidate](pooled-driver-no-invalidate.md), [roundrobin-index-desync](roundrobin-index-desync.md)) and are not subsumed by this roadmap.
