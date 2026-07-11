# Decision log — phase-10-uar-sidecar-availability

## 2026-07-11 — Contested integration shape

**Options:** (A) child process in-container · (B) native k8s sidecar container · (C) client of the existing standalone `uar` Deployment

The three shapes were genuinely contested because they differ on the single property the phase exists to deliver — **lifecycle control from the BossFang console** — and research showed a working UAR Deployment already exists, making (C) nearly free.

| | Control | Local/desktop | Effort |
|---|---|---|---|
| A — child process in-container | **full** (start/stop/restart) | **yes** (identical behaviour) | L |
| B — native k8s sidecar container | health + use + test only — a container cannot start/stop a *sibling* container from inside the pod | no | M |
| C — client of existing Deployment | none ("use and test" only) | no | S |

**Decision: (A) child process in-container.** | **Provenance: user** (`AskUserQuestion`, kbd-analyze)

Rationale: only (A) satisfies "run, used, and tested from the web console" — (B) and (C) deliver *use* and *test* but not *run*. (A) is also the only shape that behaves identically on a laptop and in the cloud, and it needs no Service DNS or NetworkPolicy changes.

Mechanism: librefang spawns UAR's `uar-sidecar` binary as a supervised child (`READY:{port}` stdout handshake; stdin-EOF shutdown). The binary is baked into the BossFang image via multi-stage `COPY --from=<uar-image>` and resolved with `current_exe()` → `~/.librefang/bin/` → PATH, so **PATH is never load-bearing** — which is the root cause of the reported bug.

Cost accepted: UAR must add `uar-sidecar` to its release/image build (see below), and UAR's `/opt/uar` assets must be carried into the image.

## 2026-07-11 — `uar-driver` retirement posture

**Options:** un-force now + delete later · delete outright · leave as-is

**Decision: un-force now, delete later.** | **Provenance: user** (`AskUserQuestion`, kbd-analyze)

Drop the unconditional `features = ["uar-driver"]` edge at `librefang-kernel/Cargo.toml:16`, making the feature genuinely opt-in (as `CLAUDE.md` already — wrongly — claims it is). Keep the driver code in-tree but unbuilt for one release; delete it and the `universal-agent-runtime` git dependency once the sidecar path is proven in production.

Backing evidence (`cargo tree -i surrealdb`): `universal-agent-runtime` pins `surrealdb = "=3.2.1"` (**exact, rigid**) while `surreal-memory` pins `3.2.0` (**caret, flexible**). **UAR is the sole source of the lockstep pin.** Un-forcing it removes that constraint from the default build — ending the coordinated three-repo version dance that this session's upstream merge had to pay again.

## 2026-07-11 — Build-vs-adopt: no new dependencies

**Decision: adopt UAR's published image; reuse librefang's own supervisor and HTTP stack; build nothing third-party.** | **Provenance: research**

- Supervision — **reuse** `librefang-channels/src/sidecar.rs`, which already implements spawn, stderr classification, and restart-with-backoff (`sidecar.rs:502-520`). An external supervision crate was rejected as duplicating proven in-tree code.
- HTTP + SSE — **reuse** `reqwest 0.13` (`stream` feature), `futures`, `tokio-stream`, all already in the workspace. `reqwest-eventsource` and `tonic`/gRPC were rejected as unnecessary.
- Binary resolution — **generalize** the existing `resolve_sidecar_command` (`sidecar.rs:728`) rather than copy-pasting a UAR-specific variant. Notably, a `which`-style crate was rejected on principle: PATH lookup is the mechanism we are deliberately removing.

Net: **zero new third-party dependencies** in this phase.

## 2026-07-11 — Blocking upstream change identified

`GQAdonis/universal-agent-runtime` does not currently build or publish the `uar-sidecar` binary — `grep -c uar-sidecar .github/workflows/release.yml` → **0**, and `Dockerfile:225` builds only `--bin universal-agent-runtime`. Adding it is small and additive, in a repo we control, on the branch family already cut this session (`sync-gqadonis-8c7377a1`). **This blocks G-1 and must land before librefang's spawn path can be tested end-to-end.**
