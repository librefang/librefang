# ADR-014: Vendor Code Ownership — Move vendor/ to First-Party crates/

**Status**: Accepted
**Phase**: Cross-cutting (applies to all phases)
**Date**: 2026-03-21
**Authors**: Daniel Alberttis

---

## Context

OpenFang-AI currently has 6 crates frozen in `vendor/`:

| Vendor path | What it is |
|-------------|-----------|
| `vendor/rvf/rvf-runtime` | HNSW vector store engine (core storage) |
| `vendor/rvf/rvf-types` | Shared types for the RVF format |
| `vendor/rvf/rvf-crypto` | Witness chain / content hashing |
| `vendor/rvf/rvf-adapters/sona` | SONA learning adapter (trajectory/experience/pattern stores) |
| `vendor/rvf/rvf-federation` | Federated learning export/import pipeline |
| `vendor/ruvllm` | Model routing intelligence (placeholder skeleton, T-10 target) |

These were vendored from `ruvnet/ruvector-upstream` to decouple OpenFang from upstream release cadence. However, the current approach has a structural problem: **the code lives in `vendor/`, which signals "third-party, frozen, don't customize"** — exactly the opposite of what is needed for a production SaaS.

Specific risks of the current vendor pattern:

1. **API lock-in**: We work around vendor APIs rather than improving them.
2. **Upgrade friction**: Syncing upstream means manually patching `vendor/` with no clear ownership boundary.
3. **SaaS dependency risk**: If `ruvnet/ruvector-upstream` changes license, breaks API, or is abandoned, our core storage and routing layers are affected.
4. **Product logic in vendor/**: The `sona`, `federation`, and routing components are **product logic** (not generic infrastructure) — they belong in `crates/`, not `vendor/`.
5. **T-10 blocker**: `vendor/ruvllm` is a skeleton with no types. Vendoring the full upstream ruvllm would require also vendoring `ruvector-core` (a massive, separate dep not in our workspace). The routing intelligence is ~300 lines of pure algorithmic Rust that is simpler to own outright.

---

## Decision

**Move all vendor code into first-party crates under `crates/`.** We own the implementation, the API surface, and the evolution roadmap. Upstream syncs are a deliberate pull decision (swarm-reviewed diff), not a forced dependency.

### Crate mapping

| Current vendor path | New first-party crate | Rationale |
|--------------------|-----------------------|-----------|
| `vendor/rvf/rvf-runtime` | `crates/openfang-store` | Core vector storage — owned infrastructure |
| `vendor/rvf/rvf-types` | merge into `crates/openfang-types` | Already re-exported; eliminate indirection |
| `vendor/rvf/rvf-crypto` | `crates/openfang-crypto` | Witness chain / hashing — owned primitive |
| `vendor/rvf/rvf-adapters/sona` | `crates/openfang-sona` | Product learning layer |
| `vendor/rvf/rvf-federation` | `crates/openfang-federation` | Product federation layer |
| `vendor/ruvllm` | `crates/openfang-routing` | Model routing intelligence (implement from scratch — see below) |

### `crates/openfang-routing` — implement, don't copy

`vendor/ruvllm` is a placeholder skeleton. The upstream ruvllm has an unconditional `ruvector-core` path dependency that cannot be resolved in our workspace without also vendoring `ruvector-core` and its deps. The routing intelligence (7-factor `ComplexityAnalyzer`, `ModelRouter`, tier thresholds) is ~300 lines of pure Rust with no external deps beyond `serde`. We implement it as a first-party crate rather than copying the upstream.

ADR-010 remains authoritative for the routing logic design (`ComplexityAnalyzer` 7 factors, `ModelTier` thresholds, `WitnessLog`). The implementation lives in `crates/openfang-routing/`.

### What stays in vendor/

Nothing. `vendor/` is removed entirely once the migration is complete. Any future third-party code that cannot be owned (e.g., external cryptography primitives) will use published crates.io deps, not a local vendor directory.

### Upstream sync process (post-migration)

When ruvnet/ruvector-upstream ships improvements worth pulling:
1. Spawn a swarm agent to review the upstream diff
2. Port the specific algorithm or type change into the relevant `crates/openfang-*/`
3. Write a test that exercises the ported behavior
4. Merge into main — no vendor lockfile to update

---

## Consequences

### Positive

- Full ownership of storage, crypto, learning, federation, and routing layers
- API can be refactored freely (rename, extend, break) without upstream permission
- No single-vendor risk — the SaaS does not depend on ruvnet maintaining any particular crate
- `openfang-routing` is purpose-built for OpenFang's routing needs (no dead code from candle/metal/cuda inference features)
- `vendor/` directory removed — workspace is self-contained under `crates/`

### Negative

- Migration effort: ~6 crates to move and re-path (estimated 1 round per crate)
- All existing `use rvf_runtime::`, `use rvf_federation::` etc. imports must be updated across `openfang-memory`, `openfang-runtime`
- Any upstream improvements to rvf-runtime HNSW need to be ported manually

### Neutral

- All existing tests pass unchanged (code moves, not changes)
- `Cargo.toml` path deps change from `vendor/rvf/...` to `crates/openfang-...` — no behavioral change
- PLAN-002 T-10 through T-12 proceed using `crates/openfang-routing` instead of `vendor/ruvllm`

---

## Supersedes

- ADR-010 §8 "Crate Vendoring" section — `vendor/ruvllm` approach replaced by `crates/openfang-routing`
- All PLAN-002 references to `vendor/ruvllm` — updated in PLAN-002 and PLAN-003

---

## References

- `docs/plans/PLAN-003-vendor-to-first-party-migration.md` — execution plan
- `docs/specs/SPEC-009-vendor-to-first-party-migration.md` — acceptance criteria
- `docs/adr/ADR-010-llm-intelligence-layer.md` — routing design (still authoritative)
- `ruvnet/ruvector-upstream` (external) — reference implementation, not a dependency
