# Build & verify

The short rules live in [`CLAUDE.md`](../../CLAUDE.md): never `cargo build` / `cargo run` locally, always scope `cargo test` with `-p <crate>`, and let the full workspace build run in CI.
This page carries the reasoning and the procedures that do not fit there.

## Why local builds are restricted

Several agent sessions and the user's own shell share one `target/` directory.
An unscoped workspace-wide build or test run locks it and stalls everyone else, so the shared cost outweighs the local signal.
`cargo check --workspace --lib` is cheap enough to stay allowed; anything that links is not.

## CI test lanes (refs #3696)

CI splits tests into two jobs so a unit failure surfaces quickly.

- **Unit-fast** — `Test / Unit (lib+bin)`, roughly 2 minutes.
  `cargo nextest run --workspace -E 'kind(lib) | kind(bin)' --no-fail-fast`.
  Lib and binary unit tests only, no integration test binaries.
  Run this locally for quick iteration.
- **Integration** — `Test / Ubuntu (shard N/4)`, roughly 10–20 minutes.
  Sharded across 4 Ubuntu runners via `--partition hash:N/4`, plus a single macOS job and a 2-way-sharded Windows job, both main-push only.
  Runs all `--tests` targets.

Local equivalents:

```bash
# Fast lane — unit tests only:
cargo nextest run --workspace -E 'kind(lib) | kind(bin)' --no-fail-fast

# Full validation — integration tests (mirrors the Ubuntu shard lane):
cargo nextest run --workspace --no-fail-fast
```

### Why the nextest filter expression, not `--lib --bins`

`--lib --bins` errors with "no library targets found" when a `-p <crate>` selector targets a binary-only crate such as `librefang-cli`.
The `-E 'kind(lib) | kind(bin)'` expression matches whichever kinds the selected crates actually have, so the selective CI lane stays green when a PR touches only `librefang-cli/main.rs` — or when a stale-base diff drags it in.

`librefang-desktop` is not binary-only: it has a lib target carrying 11 tests, so `-p librefang-desktop` resolves fine.

### Why the Windows lane excludes `librefang-desktop` (#6729)

The crate's test binary links on Windows but aborts at load with `0xc0000139` (`STATUS_ENTRYPOINT_NOT_FOUND`).
That kills nextest's list phase, takes the whole lane down, and with it the CI Gate.

A dedicated `cargo test -p librefang-desktop --all-targets --no-run` step keeps the Windows compile+link coverage.
The tests themselves are platform-independent and still run on Ubuntu, macOS and the unit-fast lane.
Drop both the exclusion and the extra step once the missing DLL export is identified.

## Verifying without a native toolchain (Docker)

When the host has no native `cargo` — for example a macOS box where Rust was never installed — do **not** declare a change unverified.
Run the build inside the repo's sanctioned dev image (`Dockerfile.rust-dev`, kept in sync with CI's Linux package set).
It compiles into a named volume rather than the host's shared `target/`, so it does not contend with the user's sessions and the "don't run cargo locally" rule still holds.

```bash
# 1. Build the dev image once (cached afterwards):
docker build -t librefang-rust-dev:latest -f Dockerfile.rust-dev .

# 2. Run any scoped cargo command through it.
WORKTREE="$(git rev-parse --show-toplevel)"
TARGET_VOL="librefang-target-$(basename "$WORKTREE")"
docker run --rm \
  -v "$WORKTREE":/work \
  -v librefang-cargo:/cargo -v "$TARGET_VOL":/target \
  -e CARGO_HOME=/cargo -e CARGO_TARGET_DIR=/target -w /work \
  librefang-rust-dev:latest \
  sh -c 'export PATH=/usr/local/cargo/bin:$PATH; mold -run cargo test -p librefang-api --lib'

# Reclaim a finished worktree's cache:
docker volume rm "$TARGET_VOL"

# On a small Docker VM, back each volume with a host dir on a big disk first:
docker volume create --opt type=none --opt o=bind \
  --opt device=/big/disk/target-<wt> librefang-target-<wt>
```

Four things about that invocation are load-bearing:

- **Set `PATH` explicitly.** A login shell (`bash -l`) drops the image's `/usr/local/cargo/bin` from `PATH`.
- **Prefix link-producing commands with `mold -run`** — build and test, not `check`.
  The image ships the mold linker, and `mold -run` intercepts the child `ld` *without* touching `RUSTFLAGS`, so the cached target dir stays valid while the link phase every incremental build pays gets cut.
  Measured on a warm cache, a one-line edit plus relink of the kernel-router test binary went 7.1 s → 1.7 s (4.1×).
  `cargo check` has no link step, so there is nothing to prefix there.
- **Use a per-worktree target volume.**
  Sharing one target dir across worktrees on different branches corrupts cargo's incremental cache: cargo reuses compiled metadata from another code state and emits phantom errors, such as `missing field 'x'` for a field the current source does not even define.
  Deriving the volume name from the worktree keeps each branch isolated.
  The cargo *download* cache (`librefang-cargo`) is safe to share — it holds fetched `.crate` files, not compiled artifacts.
- **Scope is still mandatory**: `-p <crate>`, or the `kind(lib)|kind(bin)` nextest filter, never the unscoped workspace form.

The container runs **Linux only**.
It cannot reproduce a Windows- or macOS-specific failure, so for a platform-divergent bug either write a platform-independent regression test — resolve a nonexistent path rather than relying on a host path like `/etc` existing, see #5716 — or hand the exact command to the human for the missing OS.

## Integration testing (refs #3721)

Primary verification is automated.
The repo has comprehensive `#[tokio::test]` coverage in `crates/librefang-api/tests/`, landed via the #3571 PR series of roughly 30 PRs.
Every major route domain is exercised against a real axum router via `TestServer` (see `start_test_server*` in `tests/api_integration_test.rs`); the canonical list is `ls crates/librefang-api/tests/`.
CI runs these on every push.

For any route or wiring change:

1. **Add a `#[tokio::test]` against `TestServer`** in the matching `tests/*.rs` file.
   Spawn the router via `start_test_server()`, hit the endpoint with `reqwest`, assert status and response shape; for write endpoints, follow up with a read and assert the side effect.
   This catches missing `server.rs` registrations, un-deserialized config fields, kernel↔API type drift, and empty or null payloads.
2. **Run scoped tests locally**: `cargo test -p librefang-api`.
3. **Reviewers gate PRs** on the presence of an integration test for each new endpoint.
   A PR that changes route shape without one should be sent back.

### What the automated coverage replaced

- The old 8-step manual curl checklist is gone.
  Steps 4 and 6 are now `#[tokio::test]` cases.
  Step 7 (dashboard `grep -c newComponentName`) is dropped outright — it broke under Vite minification, and dashboard UI verification is the dashboard test suite's responsibility (`crates/librefang-api/dashboard/`).
- The "Key API Endpoints for Testing" table is gone.
  The canonical enumeration is the OpenAPI spec (`openapi.json`, regenerated by the pre-commit hook) plus the integration tests themselves.

### A test kernel must be explicitly driverless (refs #7743)

`KernelConfig::default()` sets `default_model.provider = "auto"`, and `"auto"` tells `boot_with_config` to interrogate the machine it is running on: provider API-key env vars, a TCP probe for a local Ollama, then a logged-in coding-agent CLI on `PATH`.
Whatever it finds becomes the kernel's live driver, and boot rewrites `config.default_model` to match.

So a test built on `KernelConfig::default()` is not driverless — it is driverless *only on a machine with no provider credentials*.
On a CI runner that is true and the test passes; on the laptop of anyone who develops LibreFang with Claude Code installed or `OPENAI_API_KEY` exported, the same test wires a live driver and any agent turn it dispatches makes a real, billable call — spawning the Anthropic `claude` CLI against the checkout, or hitting the provider API.
Pinning a deliberately nonexistent provider name does not help: boot classifies an unknown provider as a misconfiguration to recover from and falls through to the same host probe.

Say it outright instead:

```rust
let config = KernelConfig {
    home_dir,
    data_dir,
    default_model: DefaultModelConfig::driverless(),   // provider = "none"
    ..KernelConfig::default()
};
```

Boot honours the sentinel by installing `StubDriver` directly — no driver construction, no credential-helper subprocess, no fallback slot, no auto-detection — and per-turn resolution short-circuits to the same stub.
An agent turn then returns `AgentLoopResult { provider_not_configured: true, iterations: 0, .. }` on every machine, which is a specific outcome a test can assert instead of settling for "an error happened".

`MockKernelBuilder` (`librefang-testing`) applies this before its `with_config` closure runs, so every kernel booted through it is driverless unless the test asks for a driver.
A test that *does* need one should point `default_model` at a local mock provider (see `crates/librefang-api/tests/hooks_commands_routes_integration.rs`), never at `"auto"`.

## Live LLM verification (human-only)

A live daemon with a real LLM is needed **only** when the change touches an LLM call path or end-to-end prompt/metering wiring that integration tests cannot simulate — real provider streaming, real Groq token accounting, dashboard HTML smoke.

Claude must **not** execute these steps.
They require `cargo build --release` and a long-lived daemon on port 4545, both blocked by `.claude/hooks/`.
Prepare the commands and payloads for the user, who pastes the output back.

```bash
# Stop any running daemon:
#   Linux/macOS:        pkill -f librefang ; sleep 3
#   Windows / Git Bash: tasklist | grep -i librefang && taskkill //PID <pid> //F && sleep 3

# Build + start with provider key (binary suffix is .exe only on Windows):
cargo build --release -p librefang-cli
GROQ_API_KEY=<key> target/release/librefang start &
sleep 6 && curl -s http://127.0.0.1:4545/api/health

# Real LLM round-trip + side-effect check:
AGENT_ID=$(curl -s http://127.0.0.1:4545/api/agents | python3 -c "import sys,json; print(json.load(sys.stdin)[0]['id'])")
curl -s -X POST "http://127.0.0.1:4545/api/agents/$AGENT_ID/message" \
  -H "Content-Type: application/json" -d '{"message":"Say hello in 5 words."}'
curl -s http://127.0.0.1:4545/api/budget          # cost should have increased
curl -s http://127.0.0.1:4545/api/budget/agents   # per-agent spend visible

# Cleanup: same OS-specific kill command as above.
```

The daemon subcommand is `start`, not `daemon`.
