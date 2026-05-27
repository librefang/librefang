# Phase Assessment: phase-4-multilang-wasm-toolchain

**Project:** LibreFang (BossFang fork) — github.com/GQAdonis/librefang
**Date:** 2026-05-27
**Status:** NEW PHASE — proposed; Phase 3 (M2 upstream merge) still pending.
**Prior context:** Existing `Dockerfile` (slim runtime) and `Dockerfile.rust-dev` (dev image with Tauri deps).
**Assessed against:** User request to extend both Dockerfiles with Rust-nightly + cranelift + wasmtime,
Python 3.13 + pyo3 + uv, Node.js 24+ + pnpm + bun + TypeScript 6, and the latest Go (1.26.3) — plus a
comprehensive set of multi-language WASM compilation toolchains, to support a forthcoming **dynamic
plugin model** where third-party code (Rust/Go/Python/JS/TS) is compiled to WASM and executed
in-process via wasmtime (AOT, JIT, or interpreted depending on configuration).

---

## Summary

The user wants the two Dockerfiles upgraded to be the foundation for a polyglot WASM plugin runtime.
The current state is **minimal-runtime / build-only**: the production `Dockerfile` is a slim release
container (Node 22 + Python 3 + librefang binary), and `Dockerfile.rust-dev` is a Rust dev image with
GTK/WebKit deps for Tauri checks. Neither has a WASM toolchain, neither has Rust nightly, neither has
Python 3.13, neither has Bun, neither has Go.

The request expands the dev image significantly (it becomes the polyglot compile host) and adds a
small, deliberate subset to the runtime image (just `wasmtime` + `wasi-sdk` runtime libs — the
runtime image must **not** become a multi-language compile environment, or it ceases to be slim).

**Gap count:** 4 toolchain stacks × 2 Dockerfiles + 1 architectural decision (runtime split:
fat-runtime vs separate `librefang-wasm-host` image vs sidecar pattern).

**Decisions required:** 3 (see end of doc).

---

## Codebase Scan Results

### Current `Dockerfile` (production runtime) — what it has today

| Stage | Base | Purpose | Toolchains present |
|---|---|---|---|
| `dashboard-builder` | `node:20.20.2-alpine` | Build React dashboard | Node 20.20.2, pnpm 10.33.0 (pinned) |
| `builder` | `rust:1.94-slim-bookworm` | Build `librefang` binary | Rust 1.94 stable, gcc, libdbus-1-dev, libssl-dev |
| final runtime | `node:22.11.0-bookworm-slim` | Run daemon | Node 22.11.0, python3 (system, ~3.11), pip3, libdbus-1-3, gosu |

**Has no Rust toolchain in the final image** (binary is copied in). Has no Go, no Bun, no
WASM tooling, no Python 3.13. `python3` is whatever bookworm ships (3.11.2). pip is invoked with
`--break-system-packages` against system Python.

### Current `Dockerfile.rust-dev` (dev image) — what it has today

| Layer | Content |
|---|---|
| Base | `rust:1-trixie` (Debian trixie, rolls forward as rustup stable advances) |
| Toolchain | Rust **stable** (whatever rust-toolchain.toml's `stable` resolves to on first container boot) |
| System deps | build-essential, pkg-config, libssl-dev, libdbus-1-dev, libsecret-1-dev, perl |
| Tauri Linux deps | libwebkit2gtk-4.1-dev, libgtk-3-dev, libayatana-appindicator3-dev, librsvg2-dev, patchelf |

**Has no nightly, no cranelift component beyond what rustc ships, no wasmtime, no wasm-tools,
no cargo-component, no wit-bindgen, no Python beyond what trixie pulls transitively, no Node,
no Go, no Bun, no uv.**

### Target architecture (what the plugin model needs)

The user's stated plugin model needs **two distinct capabilities**:

1. **Plugin compile-time** (dev image, CI image, possibly an extension-author-facing image):
   compile plugin source (Rust / Go / Python / JS / TS) → `.wasm` (core module) or `.wasm`
   (Component Model component).
2. **Plugin run-time** (production runtime image, embedded in `librefang` itself or beside it):
   load `.wasm` from disk / registry, instantiate, execute with sandboxing.

These are not the same surface. The compile-time toolchain is hundreds of MB. The runtime needs
**only the wasmtime runtime library** (already linkable into the Rust binary as a crate, plus
optionally `wasi-sdk` shared libs if we want to dlopen host modules). Conflating the two would
balloon the production image from ~400 MB to ~3 GB.

The strong signal from the existing repo culture (slim release image, separate dev image
already in place) is: **runtime stays slim; dev image becomes the polyglot host**.

---

## Toolchain Inventory — what to add, with researched-current versions

### 1. Rust nightly + WASM ecosystem

**Why nightly:** cranelift-as-a-library is exposed via Wasmtime stable; nightly is only needed for
(a) `-Zbuild-std` when shrinking wasm binaries, (b) experimental Component Model features in
`wasm-bindgen` 0.2.120+, and (c) emerging `wasm32-wasip3` target work. Stable suffices for plugin
loading via wasmtime; nightly is requested for build-time experiments. Pin via
`rust-toolchain.toml`-style **install both** rather than swap the default.

| Component | Channel | Latest (2026-05) | Purpose |
|---|---|---|---|
| `rustc` / `cargo` stable | stable | 1.94.x (existing) | Production builds |
| `rustc` / `cargo` nightly | nightly | nightly-2026-05-26 | Plugin-build experiments, `-Zbuild-std` |
| `rust-src`, `rust-analyzer` | both | — | Tooling |
| Targets | both | `wasm32-unknown-unknown`, `wasm32-wasip1`, `wasm32-wasip2` | WASM compilation |

WASM CLI tools to install via `cargo install --locked`:

| Tool | Version | Purpose |
|---|---|---|
| `wasmtime-cli` | 26.x | AOT compile + execute WASM/WASI/Component Model |
| `wasm-tools` | 1.x | wasm dump, validate, component new, strip, opt |
| `cargo-component` | 0.21.x | Build Rust → WASM components (replaces wit-bindgen invocation) |
| `wit-bindgen-cli` | 0.36.x | Generate language bindings from WIT |
| `wasm-bindgen-cli` | 0.2.120 | JS interop bindings (browser-style WASM) |
| `wasm-pack` | 0.13.x | npm-package WASM builds |
| `cargo-wasi` | 0.1.x | Convenience wrapper for wasm32-wasi target |
| `twiggy` | 0.7.x | WASM size profiling |
| `wasmer` (optional) | 5.x | Alternative runtime for cross-validation |

System WASM runtime libs (so `wasmtime::Engine` can load AOT-compiled `.cwasm` files from a
runtime image without rebuilding Cranelift): wasmtime exposes a stable C API
(`libwasmtime.so` / `wasmtime.h`) shipped via the wasmtime release tarball.

### 2. Python 3.13 + pyo3 + uv + WASM

| Item | Latest | Install path |
|---|---|---|
| Python | 3.13.x (CPython, replaces system 3.11) | `uv python install 3.13` |
| `uv` | 0.8.x (Astral) | Copy from `ghcr.io/astral-sh/uv:latest` multi-stage |
| `pyo3` | 0.22.x → 0.23.x (workspace-pinned) | Already a Cargo dep; no apt change |
| `maturin` | 1.7.x | `uv tool install maturin` — builds pyo3 wheels |
| `pip` | via `uv pip` | uv replaces pip |

Python → WASM toolchain (for plugins authored in Python):

| Tool | Version | Strategy |
|---|---|---|
| `py2wasm` (Wasmer) | latest | AOT-compile Python (Nuitka-based) to WASM, ~70% native speed |
| `componentize-py` (Bytecode Alliance) | 0.16.x | Convert Python apps to **WASI 0.2 Components** — preferred for plugin model |
| `pyodide-build` | 0.27.x | Build CPython + pkg stack to WASM (browser focus; useful for validation) |

**Recommended primary:** `componentize-py`. It targets the Component Model directly, which is the
native fit for a dynamic plugin host. `py2wasm` is a fallback for plugins that need raw speed.

### 3. Node.js 24 LTS + pnpm + bun + TypeScript 6 + WASM

Per the Node.js release schedule, **24.x is the current Active LTS** (LTS through 2026-10-28,
Maintenance through 2028-04-30). Node 26 is the current dev release. The user asked for "LTS
(24+)" — pin Node 24.x.

| Item | Version | Source |
|---|---|---|
| Node.js | 24.x LTS (e.g. 24.10.0) | `nodejs.org` tarball or `node:24-bookworm-slim` base |
| pnpm | 10.x (pinned via corepack, current 10.33.0) | `corepack prepare pnpm@10.33.0 --activate` |
| Bun | 1.2.x | Astral curl installer `curl -fsSL https://bun.sh/install` |
| TypeScript | **6.0** (GA March 2026) | `pnpm add -g typescript@6` |
| `tsx` | 4.x | Optional, for ts script execution |

JS/TS → WASM toolchain:

| Tool | Version | Strategy |
|---|---|---|
| AssemblyScript | 0.27.x | TypeScript-like subset → WASM (Binaryen-based) — primary path |
| Javy (Shopify / Bytecode Alliance) | 5.x | QuickJS + script → WASM (runtime-bundled) |
| Porffor | 0.49+ | Ahead-of-time JS → WASM (still pre-1.0; install as a stretch) |
| `wasm-bindgen` JS glue | (Rust-side) | For Rust↔JS WASM bridging |
| Emscripten | 4.0.x | C/C++ → WASM, also useful when Node-native deps need a WASM fallback |

**Recommended primary:** AssemblyScript for first-party TS plugins; Javy for "I have arbitrary JS
that I want to run in the sandbox".

### 4. Go (latest) + WASM

Per web search: **Go 1.26.3** released 2026-05-07 (Green Tea GC default; ~30% lower cgo overhead).

| Item | Version | Source |
|---|---|---|
| Go | 1.26.3 | `go.dev/dl/go1.26.3.linux-${ARCH}.tar.gz` |
| TinyGo | 0.41.1 (2026-04-22) | TinyGo deb package or tarball |

Go → WASM toolchain:

| Tool | Target | Notes |
|---|---|---|
| `go build` with `GOOS=js GOARCH=wasm` | browser WASM | Larger binaries; cgo limited |
| `go build` with `GOOS=wasip1 GOARCH=wasm` | WASI Preview 1 | Server-side |
| TinyGo with `-target=wasi` / `-target=wasip2` | WASI 0.1 / 0.2 components | Preferred for plugin model — small binaries, Component Model support |
| `wasm-opt` (binaryen) | post-process | Size reduction |

**Recommended primary:** TinyGo with `wasip2`. The mainline `go` compiler is included for
compatibility but TinyGo is the path that actually fits a plugin model.

### 5. C/C++ → WASM (low priority but cheap to include)

| Tool | Version | Notes |
|---|---|---|
| `wasi-sdk` | 24+ | Clang + sysroot + libc for WASI; required by many plugin examples |
| Emscripten | 4.0.x | C/C++ → browser WASM (already listed above) |
| Binaryen (`wasm-opt`, `wasm-as`, `wasm-dis`) | 119+ | Optimization and disassembly |

### 6. Optional: cross-runtime validators (very small, helpful for CI)

- `wasmer` 5.x — sanity-check that a compiled `.wasm` runs on at least one other runtime besides wasmtime.
- `wasmedge` 0.14.x — only if we want AI/tensor extension validation later. Skip for now.

---

## Architectural Decision Points

### D1. Should the **production** `Dockerfile` ship a WASM toolchain at all?

Three viable paths:

1. **Runtime-only WASM in production** *(recommended)*: include the wasmtime shared library
   (`libwasmtime.so`) and `wasi-sdk` sysroot libs. The librefang Rust binary links wasmtime
   statically (Cargo dep); the only thing that needs to be present at runtime is whatever the
   plugin loader needs to `dlopen` — for most cases, nothing. Image grows by ~30-50 MB.
   Plugins arrive pre-compiled (`.cwasm` AOT or `.wasm`).
2. **Fat runtime**: bundle the full polyglot compile toolchain inside the production image so
   plugins can be compiled in-place. Image balloons to ~3 GB. Strong "no" from existing repo
   culture (slim release image is a deliberate design property — see Dockerfile header comments).
3. **Sidecar/builder image**: ship a separate `librefang-plugin-builder` image that contains the
   full polyglot toolchain. Production image only embeds wasmtime. Cleanest split.

**Recommendation:** Path 1 for the immediate change; design Path 3 as the next iteration (when
the plugin registry & build pipeline land).

### D2. Should `Dockerfile.rust-dev` become the polyglot dev image, or do we split into
`Dockerfile.plugin-dev` for the polyglot WASM compile stack?

Two paths:

A. **Extend `Dockerfile.rust-dev` in place** (single dev image, ~4-5 GB final size). One image to
   maintain. Aligns with "the wrapper script in `scripts/.local/bin/cargo`" — extension authors
   already use this image.

B. **Add a second `Dockerfile.plugin-dev`** that inherits from `librefang-rust-dev` and layers
   Python/Node/Go/Bun/WASM tools. Smaller blast radius on the existing dev image.

**Recommendation:** Path A *unless* sizing pushes the image past CI runner disk limits (GitHub's
`ubuntu-latest` has ~14 GB free on `/`). Initial estimate: ~4.5 GB. Acceptable.

### D3. Rust nightly handling: install both stable + nightly, or switch default to nightly?

The repo's `rust-toolchain.toml` pins `channel = "stable"` and is load-bearing — `cargo check
--workspace --lib` against trixie + nightly will exercise different code paths than CI's stable
lane and risks "works on my machine" drift.

**Recommendation:** Install both via `rustup toolchain install nightly` and `rustup target add
... --toolchain nightly`; **default stays stable**. Plugin builds opt-in via `cargo +nightly`.

---

## Verification Plan (what "done" looks like)

After the change is implemented (Plan phase), the following checks must pass:

1. `docker build -f Dockerfile -t bossfang:test .` succeeds and the final image size is within
   ~50 MB of the current image (runtime WASM lib is the only addition).
2. `docker build -f Dockerfile.rust-dev -t librefang-rust-dev:test .` succeeds and resulting
   image contains:
   ```
   rustc +stable --version
   rustc +nightly --version
   wasmtime --version
   wasm-tools --version
   cargo-component --version
   wit-bindgen --version
   python3.13 --version
   uv --version
   maturin --version
   componentize-py --version
   node --version              # ≥ 24.0
   pnpm --version              # ≥ 10
   bun --version
   tsc --version               # ≥ 6.0
   asc --version               # AssemblyScript
   javy --version
   go version                  # ≥ 1.26.3
   tinygo version              # ≥ 0.41.1
   clang --version             # from wasi-sdk
   wasm-opt --version
   ```
3. A smoke test plugin in each language compiles to a `.wasm` Component and is loaded + executed
   by a hand-rolled `wasmtime::component::Linker` test program in the image.
4. `python3 scripts/enforce-branding.py --check` still passes (Dockerfile changes don't touch
   brand tokens — sanity check only).
5. Image is pushed to the GCR registry and the existing `bossfang` deployment pulls and starts
   successfully (production-runtime image only — the dev image is not deployed).

---

## Out-of-scope (deferred)

- Designing the actual plugin loader Rust crate (`librefang-plugin-host` or similar). The
  Dockerfile change only makes the toolchain available; the plugin model itself is a separate
  KBD change.
- Registry / distribution channel for plugins. Will piggyback on the existing
  `librefang-registry` infrastructure.
- WASI 0.3 (async) — still pre-stable as of 2026-05. Track upstream.
- AI/tensor WASM extensions (WasmEdge plugins). Defer until there's a concrete use case.

---

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Dev image size > CI runner disk | Medium | Medium | Multi-stage build; prune apt cache and cargo registry after each install. |
| `cargo install` of many WASM CLIs takes 20+ min on a cold build | High | Low | Use `cargo-binstall` to pull prebuilt binaries; layer cache them. |
| Python 3.13 + system python3 (3.11) conflict in dev image | Medium | Medium | Install 3.13 via uv into `/opt/python313`; leave system python3 alone. |
| TypeScript 6 type-check breaks dashboard build (currently TS 5.x) | Low | Low | Install TS 6 globally for plugin authoring; dashboard `pnpm install` keeps its own pinned TS in `node_modules`. |
| Bun 1.2 ships its own SQLite/JSC that may clash with system libs | Low | Low | Bun is self-contained at `/usr/local/bin/bun`; no apt deps. |
| Adding wasmtime CLI to production image (Path 1) creeps toward Path 2 over time | Medium | Medium | Keep production additions to **shared libs only** (no `wasmtime` CLI). Enforce via Dockerfile review. |

---

## Decisions Required (before Plan phase)

1. **D1**: Confirm production runtime gets runtime libs only (Path 1), not the full toolchain.
2. **D2**: Confirm we extend `Dockerfile.rust-dev` in place rather than introducing
   `Dockerfile.plugin-dev`.
3. **D3**: Confirm nightly is installed alongside stable (not as default).
4. Scope of WASM CLI tools — accept the recommended full list above, or trim? Specifically:
   - Keep `wasmer` CLI for cross-runtime validation? (adds ~80 MB)
   - Keep Emscripten? (adds ~400 MB — only justified if C/C++ plugin authoring is a real path)
   - Keep Pyodide build tooling? (adds ~600 MB — defer unless browser-side Python plugins are in scope)

---

## Sources (web research, 2026-05-27)

- Go 1.26.3 release info — [go.dev/doc/devel/release](https://go.dev/doc/devel/release)
- TinyGo 0.41 release — [tinygo.org/blog/2026/tinygo-0-41-the-big-release/](https://tinygo.org/blog/2026/tinygo-0-41-the-big-release/)
- Python WASM tooling (py2wasm, componentize-py, pyodide) —
  [wasmer.io/posts/py2wasm-a-python-to-wasm-compiler](https://wasmer.io/posts/py2wasm-a-python-to-wasm-compiler),
  [github.com/bytecodealliance/componentize-py](https://github.com/bytecodealliance/componentize-py),
  [pyodide.org](https://pyodide.org)
- TypeScript/JS WASM (AssemblyScript, Javy, Porffor) —
  [assemblyscript.org/introduction.html](https://www.assemblyscript.org/introduction.html),
  [porffor.dev](https://porffor.dev/)
- WASM runtime comparison 2026 — [wasmruntime.com](https://wasmruntime.com/en/blog/wasm-runtime-complete-list-2026)
- Wasmtime + cargo-component — [bytecodealliance/wasmtime](https://github.com/bytecodealliance/wasmtime),
  [crates.io/crates/cargo-component](https://crates.io/crates/cargo-component)
- Node.js LTS 24 release schedule — [nodejs.org/en/about/previous-releases](https://nodejs.org/en/about/previous-releases)
- uv + Python 3.13 in Docker — [docs.astral.sh/uv/guides/integration/docker/](https://docs.astral.sh/uv/guides/integration/docker/)
