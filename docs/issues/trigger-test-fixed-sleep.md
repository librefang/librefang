# [Medium] Test infrastructure roundup — fixed sleep, test-only `pub fn`, missing integration, unit-fast lane heavy I/O, idle proptest, hand-curated allowlist

**Severity:** Medium · **Domain:** Test coverage
**Status:** Merges 5 earlier issues into a single tracking item.

## Sub-findings rollup

| Origin | Description | Location |
|--------|-------------|----------|
| this | `trigger_workflow_test.rs` uses a fixed 150ms `sleep`; flaky on CI under load | `tests/trigger_workflow_test.rs:187, 456` |
| test helper leak | `pub fn install_peer_registry_for_test` lacks a `#[cfg(test)]` gate and leaks into the production API | (kernel internals) |
| rate limiter integration | The rate limiter has no live-router integration test | missing in `tests/` |
| unit-fast heavy I/O | The CI Unit-fast lane runs kernel-heavy I/O tests, defeating the #3696 lane split | `.github/workflows/test.yml` |
| proptest idle | `proptest` dependency is present but used by only 2 modules | `Cargo.toml` + `rg "proptest"` |
| hand-curated allowlist | Hand-curated allowlist tables (auth path, plugin manifests) are drift-prone | multiple sites |

## Why merged

All six are test-infrastructure hygiene; a single sweep is more effective than tackling them in isolation.

## Combined fix plan

1. **(this) Condition-based polling** instead of fixed sleeps: use the `condition-based-waiting` skill pattern, 5s deadline + 25ms poll.
2. **(test helper leak) Gate test helpers**:
   ```rust
   #[cfg(any(test, feature = "test-helpers"))]
   pub fn install_peer_registry_for_test(...) { ... }
   ```
   Or seal inside `#[cfg(test)] mod`.
3. **(rate limiter integration) Live-router test for the rate limiter**: under `start_full_router` (see "integration-tests-mock-router"), send N requests over the GCRA threshold and assert 429.
4. **(unit-fast heavy I/O) Tighten Unit-fast**: in addition to `-E 'kind(lib) | kind(bin)'`, exclude test names containing `tests::heavy_*`; or physically split heavy-I/O cases into the integration lane.
5. **(proptest idle) Promote or remove proptest**: add proptest coverage to 3–5 high-value boundaries (messages trim / metering / config reload); otherwise drop the dependency to cut compile time.
6. **(hand-curated allowlist) Derive allowlists automatically**: derive allowlists from manifests or runtime reflection to replace hand-maintained tables; CI verifies the derived output matches the hard-coded one.

## Tests

- (this) 100×`--test-threads=16` runs with no flakes.
- (test helper leak) `cargo doc --no-deps` does not expose the test helper (gate confirmed).
- (rate limiter integration) New `tests/rate_limiter_live_test.rs`.
- (unit-fast heavy I/O) Unit-fast average duration stays within the original target (< 5 min).
- (proptest idle) `cargo tree | grep proptest` shows N real dev-only consumers.
- (hand-curated allowlist) Drift test: derived table == hand-coded table.
