# UAR: in-process library vs. sidecar binary

**Date:** 2026-06-17
**Status:** assessment + recommendation (no migration done yet)
**Dep:** `universal-agent-runtime` (GQAdonis fork), pinned at rev `0f9385d4` (main).

## What changed upstream

The GQAdonis/universal-agent-runtime `main` branch now builds **two** targets:

- a **library** (`src/lib.rs`) — what librefang links today via the optional
  `uar-driver` feature (`crates/librefang-llm-drivers/src/drivers/uar.rs`'s
  `UarDriver`), and
- a standalone **binary** (`[[bin]] universal-agent-runtime`, `src/main.rs`) —
  an **Axum HTTP server** with SSE streaming and a tonic/gRPC surface, a baked-in
  provider catalog (`build.rs` fetches `models.dev` + merges `liter-llm` schemas),
  and a Leptos/HTMX web UI. This is the **"sidecar" option**: run UAR as its own
  process and talk to it over HTTP/gRPC instead of linking its code in-process.

## The two models

| | In-process library (today) | Sidecar binary |
|---|---|---|
| Integration | `uar-driver` feature links UAR's `lib.rs`; `UarDriver` calls it directly | `UarDriver` becomes a thin HTTP/gRPC client to a running UAR process |
| Build coupling | librefang compiles UAR's **entire** transitive tree | librefang compiles **none** of it |
| Deploy | single binary | two binaries (or two containers) |

## Why the sidecar is the better fit for librefang

1. **It removes the recurring upstream-merge break surface.** Linking UAR
   in-process pulls its whole dependency tree into librefang's build: `kreuzberg`
   (which broke this very merge when a transitive `image` dep shifted), `surrealdb`
   (the `=3.0.5` lockstep pin that must match across librefang + UAR + surreal-memory),
   `liter-llm` (142 providers), `burn` (ML), `mimalloc`, tonic, and the Leptos
   frontend (whose `build.rs` runs `npm`/network fetches). Every upstream merge
   risks a version collision somewhere in that tree. A sidecar compiles
   independently — librefang stops carrying any of it.

2. **It matches an architecture librefang already has.** Channels already run as
   supervised out-of-process **sidecars** (`[[sidecar_channels]]`, `SidecarChannel*`,
   the bridge-manager restart path). UAR-as-sidecar reuses that exact supervision
   pattern instead of inventing a new one.

3. **UAR is designed to be a server.** `src/main.rs` is an Axum server with
   SSE + gRPC; the lib is essentially that server's innards. Running it as a
   service is the grain of the project, not against it.

4. **Build time + image size.** The 120-minute docker-publish timeout exists
   largely because the in-process UAR tree (wasmtime + UAR + kreuzberg + surreal)
   blows past the old 60-minute budget. A sidecar built once (or pulled as a
   prebuilt image) takes that cost out of librefang's hot build path.

5. **Independent release cadence.** UAR can update without recompiling librefang;
   the `rev` pin churn (and the kreuzberg/surreal version dance) goes away.

## Costs / what a migration entails

- **IPC overhead** — negligible for this workload: LLM calls are network-bound
  (provider round-trips dominate); local HTTP/gRPC to a co-located sidecar is in
  the noise.
- **Rewrite `UarDriver`** from in-process `lib` calls to an HTTP/gRPC client
  (reqwest or tonic-generated client). Bounded: the driver already maps to a
  request/response + stream contract.
- **Supervise the sidecar** — spawn/health/restart. Reuse the channel-sidecar
  supervisor rather than building new.
- **Deployment** — ship UAR as a second container (k8s sidecar container in the
  same pod, or a separate Deployment) or bundle its binary in the image. The
  GKE overlay already runs multi-container pods, so this is additive.
- **Config** — a `[uar] endpoint = "..."` (or a `[[sidecar_channels]]`-style
  block) instead of the `uar-driver` cargo feature.

## Recommendation

**Adopt the sidecar.** It is the higher-leverage architecture for this fork:
it deletes the dependency-coupling that breaks upstream merges, reuses the
existing sidecar-supervision machinery, and aligns with how UAR is built. The
in-process `uar-driver` feature stays available (and is the current default-off
path) until the sidecar driver lands, so this is a non-breaking, incremental
migration — not a flag day.

**Suggested next step:** a dedicated phase — (1) define the UAR sidecar IPC
contract (gRPC preferred, it's already in UAR), (2) implement an
`UarSidecarDriver` client behind a new `uar-sidecar` feature, (3) wire UAR into
the sidecar supervisor + a `[uar]` config block, (4) keep `uar-driver`
(in-process) as a fallback, then retire it once the sidecar is proven in prod.
