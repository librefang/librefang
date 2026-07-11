# UAR runtime integration: embedded library vs. sidecar process

**Date:** 2026-07-11
**Status:** research — input to phase-10 assessment / plan
**Supersedes in scope:** `docs/architecture/uar-sidecar-assessment.md` (2026-06-17), which recommended the sidecar but predates the discovery that UAR already ships a purpose-built `uar-sidecar` binary.

## 1. The reported bug, and what is actually wrong

The reported symptom is that a cloud deployment fails with an error saying the **UAR cannot be found in the PATH**, and that UAR is therefore not runnable, usable, or testable from the LibreFang web console.

The root cause is not a misconfigured `PATH`, and it is not fixable by adding a directory to `PATH`.

**LibreFang never installs, spawns, or supervises a UAR process at all.** There is no UAR binary in the container image, no UAR sidecar container in the Kubernetes manifests, and no code in the workspace that launches one. Any code path that tries to execute `universal-agent-runtime` is asking the OS to run a file that was never shipped. A "not found" error is the only possible outcome.

Three independent lines of evidence:

1. **The container compiles UAR *into* the daemon, and produces no UAR binary.**
   `Dockerfile:178` builds with `--features telemetry,surreal-backend,uar-driver`. The `uar-driver` feature links the `universal-agent-runtime` **library** (`src/lib.rs`) into `librefang`. It emits no separate executable and copies none into the runtime image.

2. **There is no UAR sidecar container in the deployment.**
   `k8s/base/bossfang-deployment.yaml` declares exactly one container (`bossfang`, line 65). There is no second container, no UAR image, and no reference to UAR anywhere under `k8s/` or `deploy/`.

3. **The architecture doc says so.**
   `docs/architecture/uar-sidecar-assessment.md` is headed *"assessment + recommendation (no migration done yet)"*.

The only process-supervision machinery that exists today is the **channel** sidecar supervisor in `crates/librefang-channels/src/sidecar.rs`. Its spawn is a bare `Command::new(&ctx.command)` (`sidecar.rs:783`), wrapped in this error (`sidecar.rs:822-827`):

```
Failed to spawn sidecar '{name}' ({command}): {e}
```

On Linux, a missing binary makes `{e}` render as `No such file or directory (os error 2)`. So configuring UAR as a `[[sidecar_channels]]` entry — the only way to make LibreFang try to *run* anything named UAR — produces exactly:

```
Failed to spawn sidecar 'uar' (universal-agent-runtime): No such file or directory (os error 2)
```

That is the reported error, and it is unavoidable: the binary was never built or copied into the image.

Worse, the bundled-binary resolution described in §4.2 **does not apply to UAR**. `resolve_sidecar_command` (`sidecar.rs:728`) only engages for the literal Telegram stem; any other program name — including `universal-agent-runtime` — is treated as explicit operator intent and falls straight through to a bare OS `PATH` lookup.

So the bug is a **missing capability**, not a broken configuration. The fix is to decide how UAR should run, then build that path.

### 1.1 A correction worth stating plainly: `uar-driver` is not optional

`CLAUDE.md` describes `uar-driver` as *"off by default, opt-in."* **That is no longer true, and the discrepancy matters.**

- `librefang-llm-drivers/Cargo.toml:10` — `default = []` ✅
- `librefang-runtime/Cargo.toml:119` — `uar-driver` not in `default` ✅
- **`librefang-kernel/Cargo.toml:16`** — `librefang-runtime = { path = "../librefang-runtime", features = ["uar-driver"] }`

That last edge is a plain, non-optional dependency feature request. Cargo unions features across the graph, and `librefang-cli` depends on `librefang-kernel` unconditionally. **Every build of the shipped `librefang` binary compiles UAR in. There is no way to turn it off.**

Two consequences:

1. The embedded model's costs (§3, Option A) are not paid by an opt-in minority — they are paid by *every* build, on every platform, in every CI lane.
2. It explains why this merge's `surrealdb` version conflict was load-bearing rather than cosmetic. librefang pins `surrealdb = "=3.2.1"`; UAR pinned `=3.2.0`. Cargo cannot unify two exact `=` pins, so that mismatch would have failed dep-resolution on **every** build — not merely under an opt-in feature. It was fixed by cutting a UAR branch that bumps its pin to match.

The docs should be corrected regardless of which option is chosen.

## 2. What UAR actually offers (this changes the decision)

The previous assessment assumed a migration would mean writing a sidecar host from scratch. That is not the case. UAR already ships **two** binary targets:

```toml
[[bin]]
name = "universal-agent-runtime"
path = "src/main.rs"

[[bin]]
name = "uar-sidecar"
path = "src/bin/uar-sidecar.rs"
```

### 2.1 `uar-sidecar` — a purpose-built child-process contract

`src/bin/uar-sidecar.rs` exists specifically to be supervised by a parent process. Its contract, read directly from the source:

| Behaviour | Detail |
|---|---|
| **Binding** | Binds `127.0.0.1:0` — the OS assigns a free ephemeral port. Loopback-only; never exposed off-host. |
| **Readiness** | Emits exactly one line `READY:{port}\n` to **stdout** after the listener is bound and *before* it begins accepting connections. The parent reads this line to learn the port and to know the child is up. |
| **Shutdown** | Reads stdin; **stdin EOF terminates the process cleanly.** This is the deliberate cross-platform shutdown contract — the source notes it is used *"because SIGTERM is unreliable on Windows."* The parent simply closes the child's stdin pipe. |
| **Logging** | Forces JSON log format to avoid ANSI escape noise in the parent's log capture. |
| **Mode flag** | Sets `UAR_SIDECAR=1` so the server can detect sidecar mode (e.g. to relax CORS). |

This is a mature, well-specified integration surface. It was written for an Electron host, but nothing about it is Electron-specific — it is a generic "supervised child process" protocol, and it maps cleanly onto LibreFang's existing sidecar supervisor.

### 2.2 The HTTP control surface

Both binaries serve the same Axum router (`src/server.rs:694+`):

| Endpoint | Purpose |
|---|---|
| `GET /health`, `GET /healthz` | Liveness — for restart/backoff decisions |
| `GET /readyz` | Readiness — gate traffic until the model catalog is loaded |
| `GET /api/models`, `GET /api/catalog` | The 142-provider model catalog |
| `POST /api/chat/completion` | The completion call `UarDriver` needs |
| `POST /api/uar/route` | Model routing |
| `GET /api/live`, `GET /api/live/{topic}` | **SSE** streaming |

There is also a tonic/gRPC surface (`tonic = "0.14"`). For LibreFang's needs — completions plus token streaming — the HTTP + SSE surface is sufficient and far cheaper to consume; gRPC is not required.

Critically, **every control the web console needs already exists as an HTTP endpoint.** "Run / use / test the UAR from the console" reduces to: show `/healthz` + `/readyz`, list `/api/models`, and issue a test `POST /api/chat/completion`.

## 3. The two integration models, honestly compared

### Option A — Embedded library (what we do today)

`uar-driver` is a cargo feature on `librefang-llm-drivers`; `UarDriver` calls the UAR crate in-process.

**Pluses**

- **Zero IPC.** No serialization hop, no port, no process to supervise, no restart logic, no health check. In-process calls cannot fail with a connection error.
- **Single artifact.** One binary, one container, one thing to deploy and version. Nothing can "not be found" — the code is *in* the executable.
- **No lifecycle bugs.** No orphaned children, no zombie processes, no startup race between parent and child, no port conflicts.
- **It already works** for the in-process code path, and is already wired into the Docker build. `UarDriver` (`drivers/uar.rs:34-103`) constructs `LiterLlmDriver` as a direct library call — no socket, no PID, nothing that can be "not found".

**Minuses**

- **It cannot satisfy the actual requirement.** The user wants UAR *"run, used, and tested"* as a controllable runtime from the web console. An in-process library has no process to start, stop, restart, or health-check. There is nothing for the console to control. This is the decisive point: the embedded model is not a worse way to meet the goal, it is *unable* to meet it. (Today the dashboard's only UAR affordances are **agent-manifest import** (`POST /api/agents/uar`) and **SurrealDB namespace linking** (`POST /api/storage/link-uar`). Neither starts a process. There is no run/stop/test control to fix — it has to be built.)
- **It forces a lockstep dependency pin across three repositories, on every build.** Because librefang and UAR link the *same* `surrealdb` crate into one binary, their versions must match exactly — and per §1.1 this is not opt-in. This session's merge required pinning `surrealdb = "=3.2.1"` in the librefang workspace **and** cutting a new UAR branch to bump its `=3.2.0` → `=3.2.1`, because cargo cannot unify two exact `=` pins. Every surrealdb bump, forever, is a coordinated multi-repo change that gates *all* builds.
- **It drags UAR's entire transitive tree into librefang's build**: `liter-llm` (142 providers), `burn` (ML), `tonic`, `mimalloc`, plus a `build.rs` that fetches `models.dev` over the network and builds a frontend with pnpm. Upstream merges break on collisions in this tree.
- **It inherits UAR's git-submodule fragility.** UAR declares submodules over `git@github.com:` SSH. Cargo resolves UAR as a git dependency on CI runners with no deploy key, so librefang must consume a *forked* UAR branch that rewrites those URLs to HTTPS. That fork must be re-cut on every UAR bump (`sync-gqadonis-<sha>`).
- **Build cost.** The 120-minute docker-publish timeout exists largely to accommodate this tree.

### Option B — Sidecar process

UAR runs as its own process. `UarDriver` becomes a thin HTTP client.

**Pluses**

- **It is the only model that satisfies the requirement.** A process can be started, stopped, restarted, health-checked, and test-pinged from the web console. `/healthz`, `/readyz`, `/api/models`, and `/api/chat/completion` are exactly the controls the console needs — they already exist.
- **It severs the dependency coupling.** librefang stops compiling UAR entirely. The `surrealdb` lockstep pin, the `liter-llm`/`burn`/`tonic` tree, the `models.dev` network fetch in `build.rs`, the pnpm frontend build, and the SSH-submodule fork *all disappear from librefang's build*. The dependency pain this session was spent on is the same problem as this bug.
- **UAR is designed for it.** `uar-sidecar` is a first-class binary target with a documented readiness and shutdown protocol. We would be using the grain of the project, not fighting it.
- **It reuses machinery LibreFang already has.** The channel sidecar supervisor already does spawn / stderr-classify / restart. We extend a proven pattern rather than inventing one.
- **Independent release cadence and build time.** UAR updates without recompiling librefang.

**Minuses**

- **A process can fail to start.** This is the real cost: we take on binary-locatability, port handling, health-checking, restart/backoff, and graceful shutdown. Done carelessly, this *reintroduces* the very "not found" class of bug we are fixing. Section 4 is about making that failure mode structurally impossible.
- **Two artifacts to ship and version.** The UAR binary must be built, shipped in the image (or as a second container), and kept version-compatible with librefang's client.
- **IPC overhead.** Negligible in practice — LLM calls are network-bound on the provider round-trip; a loopback HTTP hop is in the noise.
- **`UarDriver` must be rewritten** from in-process calls to an HTTP/SSE client. Bounded work: the driver already maps onto a request/response + stream contract.

### Recommendation

**Adopt the sidecar (Option B), and retire the in-process `uar-driver` link.**

The comparison is not close, because the two options are not competing on quality — the embedded model *cannot express the feature being asked for*. There is no process for a console to run, test, or supervise. Everything else is confirmation: the sidecar also removes the multi-repo `surrealdb` lockstep pin, the SSH-submodule fork, and the bulk of librefang's build cost, which are ongoing recurring taxes we paid again this session.

The one legitimate objection to a sidecar — "a process can fail to start, and that's how we got here" — is answered by making the binary's location a build-time guarantee rather than a runtime hope. That is the next section, and it is the part that must not be got wrong.

Sequencing note: retiring the embedded link is a *two*-step change, because `uar-driver` is currently force-enabled. First drop the unconditional `features = ["uar-driver"]` edge at `librefang-kernel/Cargo.toml:16` so the feature becomes genuinely opt-in (this alone removes the `surrealdb` lockstep pin from the default build and should be measurable as a large build-time win). Keep the driver code in-tree, unbuilt, until the sidecar path is proven in production; then delete it and the `universal-agent-runtime` git dependency together.

## 4. Guaranteeing UAR is always locatable (the actual fix)

This is the heart of the bug. **`PATH` must not be load-bearing.** The two runtime shapes need different guarantees.

### 4.1 Cloud / Kubernetes — a native sidecar container

In Kubernetes the binary-location problem dissolves: UAR ships as its **own container image**, so it is on *its own* filesystem and cannot be "missing from PATH". Containers in a pod share a network namespace, so librefang reaches it at `http://127.0.0.1:<uar-port>` with no service, no DNS, and no network policy change.

Use a **native sidecar container** — an entry in `initContainers` with `restartPolicy: Always` — not an ordinary second entry in `spec.containers`. Native sidecars are **GA/stable in Kubernetes v1.33** (the `SidecarContainers` feature gate has been on by default since v1.29). They give three properties an ordinary container does not:

- **Startup ordering.** The kubelet marks the sidecar `started` before proceeding; the main container is guaranteed to start *after* UAR is up. An ordinary second container starts concurrently, so librefang would race UAR's boot and log connection-refused on every cold start.
- **Shutdown ordering.** On pod termination the kubelet *postpones* terminating sidecars until the main container has fully stopped, then shuts them down in reverse order. Ordinary containers terminate concurrently, so in-flight LLM calls would be cut off mid-request.
- **Job semantics.** A native sidecar does not prevent a Job from completing once the main container exits.

For a GKE deployment this is decisively the right shape. Sketch:

```yaml
spec:
  initContainers:
    - name: uar
      image: <registry>/universal-agent-runtime:<pinned-tag>
      restartPolicy: Always          # ← this is what makes it a native sidecar
      args: ["--host", "127.0.0.1", "--port", "8088"]
      startupProbe:                  # gate librefang on real readiness
        httpGet: { path: /readyz, port: 8088 }
      livenessProbe:
        httpGet: { path: /healthz, port: 8088 }
  containers:
    - name: bossfang
      env:
        - name: LIBREFANG_UAR_ENDPOINT
          value: "http://127.0.0.1:8088"
```

Note this uses the **`universal-agent-runtime`** binary (fixed host/port), *not* `uar-sidecar` — the stdout `READY:{port}` handshake is meaningless across a container boundary, where the kubelet's probes serve the same purpose. The two binaries are for the two different shapes.

### 4.2 Local / desktop / single-binary — bundle and resolve, never PATH

For the non-Kubernetes case (desktop, `librefang start` on a VM, dev machines), UAR runs as a **child process** using the `uar-sidecar` binary and its `READY:{port}` handshake.

Here the binary *must* be shipped alongside the daemon and located deterministically. **LibreFang already has exactly this pattern, and it is proven.** `crates/librefang-channels/src/sidecar.rs:728` resolves the bundled Telegram sidecar (`resolve_sidecar_command`, refs #5936):

```
Search order, first hit wins:
  1. the daemon's own executable directory  — std::env::current_exe()?.parent()
  2. <home_dir>/bin/                        — the `librefang update` install location
  3. the original command                   — PATH lookup (historical fallback)
```

Explicit operator intent always wins: only an *empty* command or the bare program stem is eligible for resolution; anything path-shaped is returned unchanged.

**Replicate this verbatim for UAR.** `std::env::current_exe()` is the key primitive: binaries shipped side-by-side in the same release tarball land in the same directory, so step 1 hits in the common case and `PATH` is never consulted. Remember the platform extension (`.exe` on Windows).

Two build-time obligations make step 1 reliable:

- The release tarball / container image **must** contain the UAR binary next to `librefang`. Add it to the release workflow and `COPY` it in the Dockerfile.
- **Fail loudly at startup**, not lazily at first use. If UAR is configured as the provider and the binary cannot be resolved, the daemon should surface a clear, actionable error naming every path it searched — not a bare OS "no such file or directory". The current failure is opaque precisely because it bubbles up a raw spawn error.

### 4.3 Supervision (child-process case)

Reuse the channel-sidecar supervisor's shape:

- **Spawn** with piped stdin/stdout/stderr. Read stdout until the `READY:{port}` line; **time out** the wait (a child that never prints READY must not hang boot).
- **Health-check** `GET /healthz` on the captured port; gate readiness on `/readyz`.
- **Restart with exponential backoff** and a cap; classify stderr lines to distinguish a crash-loop (bad config, missing API key) from a transient fault, and surface the former to the operator instead of silently retrying.
- **Graceful shutdown** by **closing the child's stdin** — that is UAR's documented contract and works on Windows. Follow with a SIGTERM/kill escalation on a timeout.
- Never leak the child: kill it on daemon exit.

## 5. Web-console surface

With a supervised process and its HTTP API, the console controls fall out naturally:

| Console control | Backing call |
|---|---|
| Status pill (running / degraded / down) | supervisor state + `GET /healthz`, `GET /readyz` |
| Start / Stop / Restart | supervisor commands |
| "Test the UAR" button | `POST /api/chat/completion` with a canned prompt; render the reply and latency |
| Model picker | `GET /api/models`, `GET /api/catalog` |
| Live logs / token stream | child stderr (JSON-formatted) and/or SSE `GET /api/live` |

Per the dashboard rules in `CLAUDE.md`, all of these must go through hooks in `dashboard/src/lib/queries/` and `dashboard/src/lib/mutations/` with hierarchical query-key factories — no inline `fetch()` in pages.

## 6. Open questions for the plan

1. **Who builds and publishes the UAR image/binary?** UAR's `build.rs` fetches `models.dev` over the network and builds a pnpm frontend. That cost moves out of librefang's build, but it has to live *somewhere* — a UAR release pipeline producing a pinned image tag is the cleanest answer.
2. **Version compatibility.** With the cargo-level pin gone, librefang's HTTP client and UAR's API can drift independently. We need a pinned image tag plus a version/capability check at startup.
3. **Auth on the loopback surface.** `uar-sidecar` binds loopback-only. The k8s container binds `127.0.0.1` inside the shared pod netns. Confirm nothing else in the pod can reach it, and decide whether a shared token is warranted.
4. **Do we keep `uar-driver` as a build-time fallback**, or delete it once the sidecar ships? Recommend: keep unbuilt for one release, then delete.
5. **Does the surrealdb pin actually go away?** UAR-as-sidecar owns its own SurrealDB linkage. Confirm `surreal-memory` alone does not re-impose a lockstep constraint on librefang.

## 7. Sources

- UAR source, `sync-gqadonis-8c7377a1` (`0cd664ce`): `Cargo.toml` (`[[bin]]` targets), `src/bin/uar-sidecar.rs` (READY/stdin-EOF contract), `src/server.rs:694+` (HTTP routes).
- LibreFang: `Dockerfile:178`; `k8s/base/bossfang-deployment.yaml:64-68`; `crates/librefang-channels/src/sidecar.rs:697-757` (`resolve_sidecar_command`); `crates/librefang-llm-drivers/Cargo.toml` (uar-driver, fork rationale); `docs/architecture/uar-sidecar-assessment.md`.
- Kubernetes documentation, *Sidecar Containers* — native sidecars as `initContainers` with `restartPolicy: Always`; stable in v1.33, feature gate on by default since v1.29; startup ordering, shutdown postponement, and Job-completion semantics. <https://kubernetes.io/docs/concepts/workloads/pods/sidecar-containers/>
- Rust `std::env::current_exe` — the primitive underpinning sibling-binary resolution.
