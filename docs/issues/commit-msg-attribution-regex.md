# [Low] CI hooks Low — commit-msg zero-space regex, openapi-drift `[skip ci]`, xtask dev `pkill` too broad, global `RUST_TEST_THREADS`

**Severity:** Low · **Domain:** CI / hooks
**Status:** Merges 3 earlier issues into a single tracking item.

## Sub-findings rollup

| Origin | Description | Location |
|--------|-------------|----------|
| this | `commit-msg` regex `Claude[[:space:]]+Code` requires at least one space — `ClaudeCode` slips through | `scripts/hooks/commit-msg:17` |
| openapi-drift skip-ci | The `openapi-drift` auto-commit uses `[skip ci]` — subsequent CI is skipped, and the next PR is the first to notice problems | `.github/workflows/openapi-drift.yml` |
| pkill too broad | `xtask dev` uses `pkill -f librefang`, which is too broad and can kill same-named processes | `xtask/src/dev.rs` |
| RUST_TEST_THREADS global | `RUST_TEST_THREADS: 1` is set at the workflow top level — should be scoped to the specific test suites that need serialization | `.github/workflows/test.yml` |

## Combined fix plan

1. (this) Change the regex to `Claude[[:space:]]*Code` and add a corpus test `scripts/tests/commit-msg-attribution.sh`.
2. (openapi-drift skip-ci) Remove `[skip ci]` from the openapi-drift auto-commit; let subsequent CI run so a drift fix doesn't introduce a different break.
3. (pkill too broad) Change `pkill -f librefang` to `pkill -f 'target/release/librefang start'`, or use a PID-based kill (record PID at startup).
4. (RUST_TEST_THREADS global) Scope `RUST_TEST_THREADS: 1` down to a specific step: `cargo nextest run --test-threads 1 -p kernel`; leave the rest unset.
