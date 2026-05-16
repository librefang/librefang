# librefang-runtime-sandbox-docker

Docker container sandbox for [LibreFang](https://github.com/librefang/librefang) tool execution (refs #3710 Phase 1).

OS-level isolation for agent code execution. Spawns commands inside Docker
containers with strict resource limits, network isolation, and capability
dropping. Shell metacharacters in user-supplied commands are inspected via
the helpers module (parity-tested against the parent crate's denylist —
see `crates/librefang-runtime/tests/docker_sandbox_helpers_parity.rs`).

## Where this fits

Extracted from `librefang-runtime` as part of the #3710 god-crate split.
`librefang-runtime` re-exports this crate at its historical path
(`runtime::docker_sandbox`), so downstream call sites do not need to switch
imports. Behind the parent crate's default-on `docker-sandbox` feature.

See the [workspace README](../../README.md) and `crates/librefang-runtime/README.md`.
