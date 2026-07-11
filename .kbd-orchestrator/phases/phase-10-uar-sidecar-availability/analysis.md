# Analysis — phase-10-uar-sidecar-availability

**Date:** 2026-07-11
**Mode:** stack-specified (Rust / tokio / axum / reqwest; k8s on GKE)
**Inputs:** `assessment.md`, `docs/architecture/uar-runtime-integration-research.md`
**Verdict:** adopt UAR's existing artifacts; **build almost nothing new** — reuse librefang's own supervisor and HTTP stack.

## 1. Headline

The assessment listed four blockers. Research shows **three of the four are already solved by existing assets** — one in UAR, two inside librefang itself. The genuinely new code is small and confined.

The single most important research finding: **UAR is not a greenfield integration.** It already has a Dockerfile, a published image, a release pipeline, a live GKE Deployment, and a purpose-built sidecar binary with a documented parent-process contract. We are wiring up existing parts, not building a runtime host.

## 2. What already exists (adopt, don't build)

| Asset | Where | Consequence |
|---|---|---|
| UAR container image | `us-docker.pkg.dev/<project>/<repo>/universal-agent-runtime:<tag>`, built by UAR's `.github/workflows/build-image.yml` | We can `COPY --from=<uar-image>` the binary + assets into the BossFang image. **No UAR build in librefang's hot path.** |
| UAR live Deployment | UAR's own `k8s/base/*`; `deploy/uar` + svc `uar-svc` in ns `uar`, smoke-tested on `/readyz` + `/healthz` each deploy | Proves the server runs and is healthy in GKE. Also a viable fallback target. |
| `uar-sidecar` binary | UAR `src/bin/uar-sidecar.rs` | Purpose-built child-process contract: binds `127.0.0.1:0`, emits `READY:{port}` on stdout, exits on **stdin EOF** (cross-platform; SIGTERM is unreliable on Windows). Forces JSON logs. |
| UAR HTTP control surface | UAR `src/server.rs:694+` | `/health`, `/healthz`, `/readyz`, `/api/models`, `/api/catalog`, `/api/chat/completion`, SSE `/api/live`. Every console control we need already exists. Container healthcheck uses port **1906**. |
| **librefang's sidecar supervisor** | `crates/librefang-channels/src/sidecar.rs` | Already implements spawn, stderr classification, and **restart with backoff**: `restart`, `initial_backoff_ms`, `max_backoff_ms`, `max_retries`, `reset_after_secs` (`sidecar.rs:502-520`). |
| **librefang's bundled-binary resolver** | `resolve_sidecar_command`, `sidecar.rs:728` | Exactly the PATH fix: `current_exe()` dir → `<home>/bin/` → PATH fallback. Currently hardcoded to the Telegram stem. |
| **librefang's HTTP + SSE stack** | workspace `Cargo.toml:136` `reqwest 0.13` (features incl. `stream`); `:47` `tokio-stream`; `:156` `futures` | An SSE/streaming client for UAR needs **zero new dependencies**. |

## 3. Build-vs-adopt decisions

| Gap | Decision | Rationale |
|---|---|---|
| **G-1** no UAR binary shipped | **ADOPT** UAR's image via multi-stage `COPY --from` in librefang's Dockerfile; bundle in the release tarball next to `librefang` | UAR's image already contains the binary + `/opt/uar/{static,skills,models}`. Copying beats rebuilding: it keeps UAR's `build.rs` (network `models.dev` fetch + pnpm frontend) out of librefang's build entirely. **Requires one UAR-side change — see §4.** |
| **G-2** no sidecar container in k8s | **N/A under the chosen shape** — UAR runs as a child process inside the BossFang container, so no second container and no Service/NetworkPolicy changes | Chosen over a native k8s sidecar container because a container cannot start/stop a *sibling* container from inside the pod — which is precisely the control being asked for |
| **G-3** no spawn/supervision code | **REUSE** librefang's own supervisor | Spawn + backoff + restart + stderr classification already exist and are proven. **Do not adopt an external supervision crate** — it would duplicate working code and add a dependency. New work is limited to the `READY:{port}` stdout handshake and stdin-EOF shutdown, which the channel supervisor does not have. |
| **G-4** no bundled-binary resolution for UAR | **BUILD (small)** — generalize `resolve_sidecar_command` from a Telegram-only special case to a stem-parameterized helper, then use it for UAR | ~20 lines. The algorithm is already written and battle-tested; it is hardcoded to one stem. Generalizing is strictly safer than a second copy. |
| **G-5** `UarDriver` must become a client | **REUSE** `reqwest` + `futures`/`tokio-stream` | Already in the workspace with the `stream` feature. SSE consumption exists elsewhere in-tree (`librefang-runtime-mcp`). No new crate. |
| **G-6** no `[uar]` sidecar config | **BUILD (small)** — extend `UarConfig` (`config/types.rs:3099`) with `enabled`, `command`/binary stem, `endpoint` override, and restart/backoff knobs | Mirror the existing `SidecarChannelConfig` field names so operators see one consistent vocabulary. |
| **G-7** no console controls | **BUILD** — query/mutation hooks per `CLAUDE.md` dashboard rules | Backed entirely by UAR endpoints that already exist. |
| **G-8** opaque failure | **BUILD (small)** — fail loudly at startup, naming every path searched | Non-negotiable; this is the actual reported bug's user-facing symptom. |
| **G-9** `uar-driver` force-enabled | **BUILD (one-line)** — drop the unconditional `features = ["uar-driver"]` at `librefang-kernel/Cargo.toml:16` | See §5. |
| **G-10** `CLAUDE.md` wrong | **BUILD (docs)** | Correct the "off by default, opt-in" claim. |

**Net:** one adopt (UAR's image), two reuses (supervisor, HTTP stack), and a handful of small, well-scoped builds. No new third-party dependency is required anywhere in this phase.

## 4. The one hard external dependency

**UAR's release/image pipeline does not currently build `uar-sidecar`.** `grep -c uar-sidecar .github/workflows/release.yml` → **0**; the Dockerfile builds only `--bin universal-agent-runtime` (`Dockerfile:225`) and the release matrix ships only `universal-agent-runtime-{linux-x64,macos-x64,macos-arm64,windows-x64.exe}`.

This is a **blocking upstream change in `GQAdonis/universal-agent-runtime`**: add `--bin uar-sidecar` to the Docker build and the release matrix so the binary we intend to spawn is actually published. It is a small, additive change to a repo we control, on the same branch family we already cut this session (`sync-gqadonis-8c7377a1`).

Also note UAR's release workflow flags cross-platform prebuilt binaries as *"an aspirational extra whose toolchain..."* (`release.yml:160`) — i.e. the non-Linux binaries may not be reliably produced. **For Linux/container (the reported bug) this is not on the path**, since we take the binary from the image. It *is* a risk for the desktop/local shape and must be verified before we promise local support.

## 5. The dependency-coupling finding (decisive, and independently verified)

`cargo tree -i surrealdb` shows exactly two external crates constraining `surrealdb`:

- `surreal-memory` → **`3.2.0`, a caret range** (`^3.2.0`) — satisfied by 3.2.1 and any future 3.x. **Flexible.**
- `universal-agent-runtime` → **`=3.2.1`, an exact pin.** **Rigid.**

**UAR is the sole source of the lockstep pin.** `surreal-memory` does not re-impose it. This settles assessment open-question 3: retiring the in-process UAR link *does* free librefang to bump `surrealdb` unilaterally, ending the coordinated three-repo dance (which we paid again during this session's merge, and which — because `uar-driver` is force-enabled — would otherwise have broken *every* build).

## 6. Decisions taken (user-confirmed)

- **D-1 — Integration shape: child process, in-container.** librefang spawns UAR's `uar-sidecar` as a supervised child. The UAR binary is baked into the BossFang image via `COPY --from=<uar-image>` and resolved via `current_exe()` → `~/.librefang/bin/` → PATH, so **PATH is never load-bearing**. Chosen over a native k8s sidecar container (cannot start/stop a sibling container from inside the pod) and over client-of-existing-Deployment (no lifecycle control at all). This is the only shape that delivers real run/stop/restart from the console *and* behaves identically on a laptop and in the cloud.
- **D-2 — `uar-driver`: un-force now, delete later.** Drop the unconditional feature edge at `librefang-kernel/Cargo.toml:16` immediately, making the feature genuinely opt-in (as `CLAUDE.md` already wrongly claims). Keep the driver code in-tree, unbuilt, for one release; delete it and the `universal-agent-runtime` git dependency once the sidecar path is proven in production.

## 7. Open questions carried into `/kbd-spec` → `/kbd-plan`

1. **UAR assets.** The image ships `/opt/uar/{static,skills,models}` alongside the binary. Which of these does `uar-sidecar` actually require at runtime, and where must they land in the BossFang image? (Determines the `COPY --from` set and image size.)
2. **Image-size budget.** Baking UAR's binary + models into the BossFang image grows it. Quantify before committing; if the models dir is large, consider fetching at first run or a shared volume.
3. **Desktop/local support in v1?** Depends on UAR's cross-platform release binaries, which its own workflow calls "aspirational" (§4). Linux/container works regardless.
4. **Version compatibility.** With the cargo pin gone, librefang's HTTP client and UAR can drift. Pin the UAR image tag and add a startup version/capability check.
5. **Loopback auth.** `uar-sidecar` binds loopback-only inside the container. Confirm that is sufficient, or add a shared token.
6. **Does the existing `uar` Deployment stay?** It is operator-managed and live. If BossFang now runs its own in-container UAR, the standalone Deployment may become redundant — or remain, serving other consumers. Not ours to delete unilaterally.

## 8. Budget

Tier 1 (repo/code search) and Tier 3 (dependency-graph verification via `cargo tree`) were sufficient; Tier 2 (Context7) and Tier 4 (web comparison) were unnecessary because both integration targets are first-party repos whose source is directly readable. One external doc consulted (Kubernetes native sidecar semantics) to justify rejecting the sidecar-container shape. Well inside the query and time caps.
