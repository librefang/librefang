# Tool-execution backends

How agents dispatch shell commands and process spawns to the right
host — local subprocess, Docker container, remote SSH host, or a
managed sandbox like Daytona. Issue #3332.

## Status

This PR (#3332) lands the **trait + concrete backend implementations
+ config plumbing**. It does **NOT** yet route the existing
`tool_runner.rs` shell / `docker_exec` / process-spawn call sites
through the trait — that migration is a deliberate follow-up because
the call-site refactor is large enough to deserve its own review.

Concretely: configuring `tool_exec.kind = "ssh"` (or `"daytona"`) in
`config.toml` resolves correctly through `resolve_backend_kind` and
materialises the corresponding `ToolExecBackend` impl, but tool calls
emitted by an LLM still flow through the legacy local / docker
helpers. **The kernel emits a `WARN` at boot when `kind != "local"`**
to make this visible. Operators experimenting with the SSH or Daytona
backend should expect the override to take effect only after the
follow-up PR migrates the call sites; until then, set `kind` to
preview the resolver and feature-flag plumbing.

## Why a trait

Historically the runtime ran every shell / `docker_exec` / process
spawn directly on the daemon host via `subprocess_sandbox.rs` and
`docker_sandbox.rs`. That works for self-hosted single-machine
deployments and CI, but it forecloses three real use cases:

1. **Workstation daemon, build farm worker.** A developer running the
   daemon on their laptop wants long-running compile / test / format
   loops to land on a beefier machine over SSH without giving up the
   workstation's identity files, MCP servers, and channel state.
2. **Throwaway sandboxes per session.** Daytona / GitHub Codespaces /
   Modal expose ephemeral container hosts on demand. An agent that
   risks `rm -rf /` should be able to do so on a sandbox with no
   blast radius back to the daemon host.
3. **Heterogeneous teams.** A fleet of agents with different security
   postures wants the "research" agents on a sandboxed host and the
   "infrastructure" agents on the local daemon host where they
   actually need to administrate it.

Trait route: `librefang_runtime::tool_exec_backend::ToolExecBackend`.

## The trait

```rust
#[async_trait]
pub trait ToolExecBackend: Send + Sync {
    fn kind(&self) -> BackendKind;
    async fn run_command(&self, spec: ExecSpec) -> Result<ExecOutcome, ExecError>;
    async fn upload(&self, path: &str, bytes: &[u8]) -> Result<(), ExecError>;
    async fn download(&self, path: &str) -> Result<Vec<u8>, ExecError>;
    async fn cleanup(&self) -> Result<(), ExecError>;
}
```

`upload` / `download` default to `ExecError::UnsupportedForBackend` —
backends that can't (yet) shuffle bytes return that error rather than
opaquely failing.

A non-zero exit code from the dispatched command is **not** an `Err` —
it surfaces in `ExecOutcome.exit_code`. `Err(ExecError)` is reserved
for failures of the dispatch path itself: missing config, auth
failure, network timeout, etc.

## Configuration

### Global default — `config.toml`

```toml
[tool_exec]
kind = "local"   # default

# [tool_exec.ssh]
# host = "build.example.com"
# port = 22
# user = "agent"
# key_path = "/home/me/.ssh/id_ed25519"
# # password_env = "BUILD_HOST_PASSWORD"
# timeout_secs = 60
# host_key_sha256 = "<hex of the server's SHA-256 host-key fingerprint>"

# [tool_exec.daytona]
# api_url = "https://app.daytona.io"
# api_key_env = "DAYTONA_API_KEY"
# image = "ubuntu:22.04"
# timeout_secs = 120
# workspace_prefix = "librefang"
```

### Per-agent override — `agent.toml`

```toml
name = "research-bot"
# … other manifest fields …
tool_exec_backend = "ssh"
```

### Resolution order

1. **Per-agent manifest** — `AgentManifest.tool_exec_backend`
   (`Some(BackendKind)`).
2. **Global config** — `KernelConfig.tool_exec.kind`.
3. **Compiled-in default** — `BackendKind::Local`.

Resolution lives in `librefang_types::tool_exec::resolve_backend_kind`
and is mirrored from the `session_mode` precedence pattern documented
elsewhere.

The matching `[tool_exec.<kind>]` subtable in `config.toml` is required
when `kind` is `ssh` or `daytona`. The factory
(`tool_exec_backend::build_backend`) returns
`ExecError::NotConfigured` if the subtable is missing or the runtime
was built without the relevant cargo feature.

## Backends

### `BackendKind::Local`

- **What:** subprocess on the daemon host, scrubbed via
  `subprocess_sandbox::sandbox_command`.
- **When:** default. Always available; no feature flag.
- **Limits honoured:** `timeout`, `max_output_bytes`.

### `BackendKind::Docker`

- **What:** create container per call, exec, destroy. Adapter over the
  existing `docker_sandbox.rs` create+exec+destroy flow.
- **Config:** uses the long-standing `[docker]` section in
  `config.toml`; no new knobs.
- **Limits honoured:** the existing `DockerSandboxConfig.memory_limit`
  / `cpu_limit` / `pids_limit` / `timeout_secs`.

### `BackendKind::Ssh` (feature `ssh-backend`)

- **What:** open a raw SSH connection per call via [`russh`], exec one
  command, capture stdout / stderr / exit code, close. Connection
  lifetime is tied to a single `run_command` call; nothing is held
  open across idle periods.
- **Auth:** `key_path` for public-key auth (PEM / OpenSSH keys);
  `password_env` for password auth from the named env var; neither
  set tries SSH none-auth (rare, mostly useless — surfaces a clear
  error otherwise).
- **Host-key verification:** `host_key_sha256` is the hex SHA-256 of
  the server's wire-form public-key blob. Three modes:
  1. **Pinned (recommended).** When set, the backend hard-rejects
     connections whose key differs.
  2. **TOFU on disk.** When empty AND
     `~/.librefang/ssh_known_hosts.toml` already records this host,
     the entry there is required to match. A mismatch raises
     `ExecError::AuthFailure`.
  3. **First connect.** When neither pin nor known-hosts entry exists,
     the backend logs the fingerprint at INFO, writes it to the
     known-hosts file, and accepts. Subsequent connects use mode 2.

  Mode 3 is the only branch open to MITM on first contact — operators
  should copy the logged fingerprint into the explicit pin once
  verified out-of-band.
- **File I/O:** `upload` / `download` return
  `ExecError::UnsupportedForBackend` in this landing. SFTP via
  `russh-sftp` is a deliberate follow-up (see "Out of scope" below).

### `BackendKind::Daytona` (feature `daytona-backend`)

- **What:** one workspace per agent (created lazily on first
  `run_command`), reused across calls. Commands POST to the
  workspace's `/exec` endpoint over the workspace `reqwest`
  client. `cleanup()` deletes the workspace; on a non-2xx or
  transport error the workspace id is restored to the cache and a
  WARN is logged so a later cleanup retries (avoids leaking
  workspaces on transient network blips). Public error messages are
  truncated to 256 chars and have `Bearer <token>` substrings
  redacted; the full body lands in `tracing::debug!` only.
- **Auth:** bearer token from the env var named in
  `tool_exec.daytona.api_key_env`. The daemon never persists the
  token.
- **BYO account setup:**
  1. Sign up at https://app.daytona.io and provision an API key.
  2. Export it on the daemon's environment, e.g. via systemd:
     ```
     [Service]
     Environment=DAYTONA_API_KEY=dt_pat_…
     ```
  3. Add the section to `config.toml` and set the agent override or
     global `kind`.
- **File I/O:** unsupported in this landing — same reasoning as SSH.

## What lives in this PR vs. what's deferred

**In this PR (#3332):**
- Trait + DTO types in `librefang-runtime/src/tool_exec_backend.rs`.
- `LocalBackend`, `DockerBackend` as adapters over the existing
  sandbox helpers — no behavior change for code paths that still call
  the helpers directly.
- `SshBackend` (feature-gated, exec-only).
- `DaytonaBackend` (feature-gated, exec-only, mocked in tests).
- Per-agent manifest field + global config field + resolver.
- Integration test in `librefang-runtime/tests/` exercising the full
  dispatch chain on the local backend.

**Deferred to follow-up PRs:**
- **Migrating the existing `tool_runner.rs` call sites** (shell tool,
  `docker_exec`, persistent process spawns) to dispatch through the
  trait. The trait is the new public seam; the call-site refactor
  stays reviewable as a separate change.
- **SFTP-backed `upload` / `download` for SSH.** Plumbing
  `russh-sftp` per call adds enough surface area (channel lifecycle,
  permissions handling) that bundling it with the trait introduction
  would have inflated this PR past usefulness.
- **Daytona file I/O.** Daytona exposes archive endpoints for tar
  upload / download; same scoping rationale as SSH.
- **Modal and Singularity backends.** The issue mentions both. Their
  shapes are similar enough to Daytona that adding them is a follow-
  up: implement `ToolExecBackend`, gate behind a new feature, plug
  into `build_backend()`.
- **Resource-cap enforcement on Daytona.** Daytona's API doesn't
  expose per-call resource caps; we rely on workspace defaults. A
  future PR can add per-call CPU / memory hints if Daytona adds them.

## Operational notes

- **The daemon does not persist credentials.** SSH keys are read off
  disk per call (small price; we don't cache the in-memory key bytes
  to keep the attack surface narrow). Daytona / future SaaS backend
  tokens come from env vars and never touch the keyring.
- **Backend selection is a security boundary.** A misconfigured SSH
  pin gives an active MITM the same blast radius as a `bash -c` on
  the daemon host. Operators should treat
  `tool_exec.ssh.host_key_sha256` as a required field in production
  deployments and enforce it via config validation in their pipeline.
- **Hot-reload caveat.** Backend instances are built once per agent
  spawn (mirrors the per-agent semaphore caveat for trigger
  dispatch). Changing `tool_exec_backend` in a manifest requires
  killing and respawning the agent — an in-place activate / status
  flip silently retains the old backend. This is intentional; future
  work can hook `build_backend` into `manifest_swap` if operators
  hit it.

## See also

- `crates/librefang-types/src/tool_exec.rs` — config types.
- `crates/librefang-runtime/src/tool_exec_backend.rs` — trait + Local
  + Docker.
- `crates/librefang-runtime/src/tool_exec_ssh.rs` — SSH backend.
- `crates/librefang-runtime/src/tool_exec_daytona.rs` — Daytona
  backend.
- `crates/librefang-runtime/tests/tool_exec_backend_selection.rs` —
  end-to-end resolution tests.
- `crates/librefang-api/tests/tool_exec_backend_selection.rs` — same
  surface, exercised from the API crate.
- Issue #3332 — original tracking issue.

[`russh`]: https://crates.io/crates/russh
