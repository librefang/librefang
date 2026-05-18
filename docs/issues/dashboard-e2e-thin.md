# [Low] Test coverage Low — Dashboard E2E thin, `MockLlmDriver` idle, channel webhook happy-path only, vault key foot-gun

**Severity:** Low · **Domain:** Test coverage
**Status:** Merges 3 earlier issues into a single tracking item.

## Sub-findings rollup

| Origin | Description | Location |
|--------|-------------|----------|
| this | Dashboard E2E is a single 56-line file with no mutation-invalidation tests | `dashboard/e2e/dashboard.spec.ts` |
| MockLlmDriver idle | `MockLlmDriver` is used only by kernel unit tests — integration tests should be able to simulate LLM behaviour through it, but don't | unit vs integration tests |
| webhook happy-path | Channel webhook tests cover only happy paths — bad signatures, replays, and oversized bodies are not exercised | `tests/channel_webhook_*.rs` |
| vault key foot-gun | The `LIBREFANG_VAULT_KEY` 32-ASCII vs 32-bytes pitfall has no unit-test coverage | vault key boot path |

## Combined fix plan

1. (this) Add Playwright tests for each major mutation: create agent / send message / config change; assert that the next dashboard read reflects the change without a manual refresh.
2. (MockLlmDriver idle) At minimum, migrate LLM-dependent cases in `tests/api_integration_test.rs` over to `MockLlmDriver`, removing the dependency on a real provider.
3. (webhook happy-path) Add negative-path integration tests to each channel webhook: bad signature / replay / oversized body → the correct status code in each case.
4. (vault key foot-gun) Unit test: feeding 32 ASCII characters (not base64) → boot returns a structured error "expected base64 of 32 bytes."
