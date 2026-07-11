# Assessment — phase-10-uar-sidecar-availability

**Date:** 2026-07-11
**Phase:** phase-10-uar-sidecar-availability
**Previous phase:** phase-9-config-store-migration (code-complete, awaiting `/kbd-reflect`)
**Verdict:** `proceed-with-architecture-change`
**Research input:** `docs/architecture/uar-runtime-integration-research.md`

## 1. Goal

Guarantee that the Universal Agent Runtime (UAR) is **available to be run, used, and tested from the LibreFang/BossFang web console**, in cloud deployments as well as locally. Today a cloud deploy fails with an error stating UAR cannot be found on the `PATH`.

## 2. Headline finding

**The reported bug is a missing capability, not a misconfiguration. It cannot be fixed by changing `PATH`, config, or the image's environment.**

LibreFang never installs, spawns, or supervises a UAR process anywhere. There is no UAR binary in the container image, no UAR sidecar container in the Kubernetes manifests, and no code in the workspace that launches one. The web console has no control that starts, stops, or tests a UAR runtime.

The user's mental model — "UAR is deployed as a sidecar controlled from BossFang" — describes an architecture that exists only as a **written proposal** (`docs/architecture/uar-sidecar-assessment.md`, 2026-06-17, status: *"no migration done yet"*). It has never been implemented.

## 3. Evidence

| # | Finding | Evidence |
|---|---|---|
| E-1 | UAR is compiled **in-process** as a library; no UAR binary is ever produced or shipped | `Dockerfile:177-178` builds `--features telemetry,surreal-backend,uar-driver`; runtime stage copies only `/usr/local/bin/librefang` |
| E-2 | No UAR sidecar container exists in the deployment | `k8s/base/bossfang-deployment.yaml:64-68` — a single container, `bossfang`. No UAR reference anywhere under `k8s/` or `deploy/` |
| E-3 | No code spawns or supervises UAR | Exhaustive search: zero `Command::new`/`spawn`/`which` call references `uar` or `universal-agent-runtime` |
| E-4 | The only spawn path that can emit the error is the **generic channel** supervisor | `sidecar.rs:783` `Command::new(&ctx.command)`; error wrapper `sidecar.rs:822-827` → `Failed to spawn sidecar 'uar' (universal-agent-runtime): No such file or directory (os error 2)` |
| E-5 | The existing bundled-binary resolver does **not** cover UAR | `resolve_sidecar_command` (`sidecar.rs:728`) engages only for the literal Telegram stem; any other program name falls through to a bare OS `PATH` lookup |
| E-6 | `UarDriver` is an in-process library call, not a client | `drivers/uar.rs:34-103` constructs `LiterLlmDriver` directly. Its `base_url` is an **LLM-provider** override, *not* a UAR endpoint |
| E-7 | The console's UAR affordances do not touch a process | `POST /api/agents/uar` = manifest import; `POST /api/storage/link-uar` = SurrealDB namespace provisioning. Neither spawns anything |
| E-8 | **`uar-driver` is force-enabled, contradicting `CLAUDE.md`** | `librefang-kernel/Cargo.toml:16` requests `features = ["uar-driver"]` unconditionally; `librefang-cli` → `librefang-kernel` is not feature-gated. **Every** shipped binary compiles UAR in |
| E-9 | UAR already ships a purpose-built sidecar binary | UAR `Cargo.toml`: `[[bin]] uar-sidecar` (`src/bin/uar-sidecar.rs`) — binds `127.0.0.1:0`, emits `READY:{port}` on stdout, exits on stdin EOF |
| E-10 | UAR exposes a complete supervision + control HTTP surface | UAR `src/server.rs:694+`: `/health`, `/healthz`, `/readyz`, `/api/models`, `/api/catalog`, `/api/chat/completion`, SSE `/api/live` |

## 4. Gap analysis

| ID | Gap | Severity |
|---|---|---|
| G-1 | No UAR binary is built, shipped, or copied into the release artifact / container image | **blocker** |
| G-2 | No UAR sidecar container in the k8s deployment; nothing for librefang to talk to in-cluster | **blocker** |
| G-3 | No spawn/supervision code for a UAR child process (spawn, `READY:{port}` handshake, health-check, restart/backoff, graceful shutdown) | **blocker** |
| G-4 | No bundled-binary resolution for UAR — `resolve_sidecar_command` is Telegram-only, so UAR always falls back to bare `PATH` | **blocker** |
| G-5 | `UarDriver` is an in-process library call and would have to become an HTTP/SSE client | high |
| G-6 | No `[uar]` config surface for a sidecar (`UarConfig` at `config/types.rs:3099` has `api_key`/`model`/`base_url`/`remote` — **no binary path, no endpoint-to-spawn, no enable flag**) | high |
| G-7 | Web console has no run/stop/restart/test controls for UAR | high |
| G-8 | Failure mode is opaque — a raw OS spawn error, not an actionable message naming the paths searched | medium |
| G-9 | `uar-driver` force-enabled at `librefang-kernel/Cargo.toml:16`, taxing **every** build with UAR's full transitive tree and the `surrealdb` lockstep pin | medium |
| G-10 | `CLAUDE.md` documents `uar-driver` as "off by default, opt-in" — factually wrong (see E-8) | low (docs) |

## 5. Options considered

Full pros/cons in the research doc, §3. Summary:

- **Option A — embedded library (status quo).** Zero IPC, single artifact, nothing to supervise. **But it cannot satisfy the goal**: an in-process library has no process to run, stop, or test, so there is nothing for a console to control. It also imposes a three-repo `surrealdb` lockstep pin and UAR's whole dependency tree on every build.
- **Option B — sidecar process.** The only model that can express the feature. Severs the dependency coupling (the `surrealdb` pin, the SSH-submodule fork, the build cost). UAR is *designed* for it (E-9, E-10). Cost: we take on binary-locatability, health, restart, and shutdown — the very failure class we are fixing, so it must be engineered deliberately.

## 6. Recommendation

**Adopt the sidecar (Option B). Retire the in-process `uar-driver` link.**

The two options are not competing on quality — the embedded model is *unable* to express "run, use, and test UAR from the console". Everything else confirms the choice: the sidecar also removes recurring dependency taxes we paid again during this very merge.

Two runtime shapes, because they have different guarantees:

- **Cloud / Kubernetes** — run the `universal-agent-runtime` binary as a **native sidecar container** (`initContainers` + `restartPolicy: Always`; GA in k8s v1.33, gate on by default since v1.29) on a fixed loopback port. Binary-locatability dissolves: UAR ships in its own image. Native sidecars (vs. an ordinary second container) additionally guarantee **startup ordering** (no cold-start connection-refused race) and **shutdown postponement** (no in-flight LLM call cut off).
- **Local / desktop / single-binary** — spawn UAR's `uar-sidecar` binary as a supervised child, using its `READY:{port}` stdout handshake and stdin-EOF shutdown contract. Resolve the binary by **replicating `resolve_sidecar_command`** (`current_exe()` dir → `~/.librefang/bin/` → `PATH` fallback) so `PATH` is never load-bearing, and **ship the binary in the release tarball**.

**Non-negotiable:** whichever path, resolution failure must fail loudly at startup with a message naming every path searched (G-8). The current bug is opaque precisely because it surfaces a raw OS error.

## 7. Risks

| ID | Risk | Mitigation |
|---|---|---|
| R-1 | A sidecar reintroduces the "can't find the binary" failure class | Bundle + `current_exe()` resolution (never `PATH`); loud startup failure; a CI check asserting the binary is present in the image/tarball |
| R-2 | Version drift once the cargo-level pin is gone | Pin the UAR image tag / binary version; capability check at startup |
| R-3 | UAR's `build.rs` (network `models.dev` fetch + pnpm frontend) has to run *somewhere* | Move to a UAR release pipeline producing a pinned artifact — do not leave it in librefang's hot build path |
| R-4 | Dropping `uar-driver` is a behavioural change for anyone relying on the in-process driver | Stage it: make the feature genuinely opt-in first (drop the kernel edge), keep code in-tree one release, then delete |
| R-5 | Loopback surface has no auth | Confirm nothing else in the pod netns can reach it; consider a shared token |

## 8. Open questions for `/kbd-analyze` → `/kbd-plan`

1. Who builds and publishes the UAR binary/image, and at what cadence? (Blocks G-1/G-2.)
2. Do we ship **both** runtime shapes (container sidecar + local child), or is cloud-only acceptable for v1?
3. Does removing the in-process UAR link actually drop the `surrealdb` lockstep pin, or does `surreal-memory` re-impose it independently?
4. Keep `uar-driver` as an opt-in fallback for one release, or delete outright?
5. Scope of the console surface for v1 — is status + test-ping enough, or is start/stop/restart required?

## 9. Verification criteria for this phase

- A cloud deploy exposes a reachable, healthy UAR (`/readyz` green) with **no** `PATH` dependency anywhere in the resolution chain.
- The web console shows UAR status and can issue a successful test completion.
- Killing the UAR process/container results in an automatic, backed-off restart.
- A deliberately-removed UAR binary produces an actionable startup error naming the searched paths — not `No such file or directory (os error 2)`.
- `cargo check --workspace --lib` clean; `python3 scripts/enforce-branding.py --check` exit 0.
