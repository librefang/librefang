# EveryAPI Auto-Detection Implementation Plan

> **For Codex:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Automatically expose a locally authenticated EveryAPI account as a LibreFang OpenAI-compatible provider without copying its relay key into LibreFang-owned files.

**Architecture:** EveryAPI owns credential selection, refresh, region resolution, and invalidation through a versioned credential-process command.
LibreFang invokes that command through a bounded resolver, registers the provider in memory, resolves credentials again for live catalog requests and LLM calls, and retries one authentication failure after invalidation.
Explicit LibreFang keys and URLs keep precedence over auto-detection.

**Tech Stack:** Go CLI and SDK, Rust 2021, tokio, serde, existing OpenAI-compatible LLM driver and driver cache.

---

### Task 1: Publish the EveryAPI credential-process contract

**Files:**
- Create in EveryAPI: `specs/cli-credential-process.md`
- Modify in EveryAPI: `clients/cli/cmd/auth.go`
- Create in EveryAPI: `clients/cli/cmd/credential.go`
- Test in EveryAPI: `clients/cli/cmd/credential_test.go`

1. Write tests for versioned JSON, region-aware `/v1` URL, expiration, cached relay-key resolution, invalidation, and stable machine error codes.
2. Run the focused tests and confirm they fail because the command is absent.
3. Implement `everyapi auth credential --format=json [--invalidate]` by reusing `config.Load`, `config.ResolveAPIBaseForBase`, `api.ResolveRelayKey`, and `api.InvalidateCachedRelayKey`.
4. Keep stdout JSON-only. Keep diagnostics on stderr through the existing top-level error path.
5. Run the focused CLI tests and commit.

### Task 2: Add a bounded LibreFang credential resolver

**Files:**
- Create: `crates/librefang-kernel/src/everyapi_credentials.rs`
- Modify: `crates/librefang-kernel/src/lib.rs`
- Test: colocated unit tests in `everyapi_credentials.rs`

1. Write failing tests for schema validation, command success, timeout/error classification, candidate executable resolution, and compatibility file fallback.
2. Implement the resolver with a bounded child-process wait.
3. Prefer `EVERYAPI_CLI_PATH`, then PATH/common install paths.
4. Fall back to `credentials.json` plus `settings.json` only when the installed CLI predates the machine command.
5. Never log or persist the returned key.

### Task 3: Add the rotating EveryAPI HTTP driver

**Files:**
- Create: `crates/librefang-kernel/src/everyapi_driver.rs`
- Modify: `crates/librefang-kernel/src/kernel/llm_drivers.rs`
- Modify: `crates/librefang-kernel/src/kernel/boot.rs`
- Test: colocated driver tests and kernel tests.

1. Write failing tests proving credentials are resolved per request and a 401 causes one invalidating re-resolution and one retry.
2. Build the existing OpenAI-compatible driver from the resolved key and URL through `DriverCache`.
3. Use the managed driver only when no explicit LibreFang key or endpoint overrides EveryAPI.
4. Keep retry bounded to one authentication retry.

### Task 4: Register and refresh the auto-detected provider

**Files:**
- Modify: `crates/librefang-runtime/src/model_catalog.rs`
- Modify: `crates/librefang-kernel/src/kernel/boot.rs`
- Modify: `crates/librefang-api/src/everyapi_catalog.rs`
- Test: relevant runtime, kernel, and API unit/integration tests.

1. Write failing tests for in-memory provider registration, suppression, explicit override precedence, and live catalog credential resolution.
2. Register EveryAPI as `AutoDetected` only after usable credentials resolve.
3. Resolve current credentials for every live catalog refresh instead of reading `EVERYAPI_API_KEY`.
4. Clear availability when managed credentials disappear while preserving manual configurations.

### Task 5: Update diagnostics and verify both repositories

**Files:**
- Modify: `crates/librefang-cli/src/doctor.rs` only if required to report managed mode accurately.
- Modify: affected documentation and tests.

1. Run formatting and scoped tests in EveryAPI.
2. Run formatting, scoped crate tests, `cargo check --workspace --lib`, and relevant clippy checks in LibreFang using an external target directory.
3. Review `git diff`, `git diff --check`, and statuses.
4. Request code review, fix important findings, commit, push, and open one focused PR per repository.
