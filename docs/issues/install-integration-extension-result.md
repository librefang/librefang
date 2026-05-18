# [Medium] `KernelApi::install_integration` returns `librefang_extensions::ExtensionResult`

**Severity:** Medium · **Domain:** Architecture · **Source:** `audit-06-architecture.md`

## Location
`crates/librefang-kernel/src/kernel_api.rs:174`

## Problem
The HTTP-layer trait returns a concrete extension-crate `Result` type. Reimplementers (mocks, alternate kernels) are forced to depend on `librefang-extensions` even when they don't otherwise need it. Same root cause as "kernel-depends-on-extensions".

## Fix
Define a typed error in `librefang-types` (e.g. `IntegrationError`) and return that. `librefang-extensions` can `From::from` for its internal errors.

## Tests
- Mock kernel for tests can be built without depending on `librefang-extensions`.
