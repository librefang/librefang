# Phase Plan: phase-4-multilang-wasm-toolchain

**Phase:** `phase-4-multilang-wasm-toolchain`
**Backend:** Native KBD (no OpenSpec, no evolver bridge)
**Date:** 2026-05-27
**Source assessment:** `assessment.md` in this directory
**Blocking dependency:** Phase 3 M2 (upstream merge of 110 commits) — Phase 4 work begins
**after** the merge lands to avoid conflicting Dockerfile edits during the upstream sync.

---

## Decisions resolved (assumptions baked into this plan)

| # | Question | Decision | Rationale |
|---|---|---|---|
| D1 | Production runtime scope | **Path 1** — wasmtime shared lib only, no compile toolchain | Preserves slim-image culture; ~30–50 MB cost; CLI tools never enter prod |
| D2 | Dev image strategy | **Extend `Dockerfile.rust-dev` in place** | Single image, one wrapper script; ~4.5 GB acceptable for CI runners |
| D3 | Rust nightly | **Install both stable + nightly; default stays stable** | Preserves CI parity for `cargo check --workspace --lib` |
| D4 | Optional tooling scope | **Include all** (user said "ALL possible tools") | Adds ~1.1 GB for Emscripten + Pyodide + wasmer; user's stated preference |

If the user redirects on any of these, the affected changes are scoped so only that change needs revision (not the whole plan).

---

## Ordered Change List

Each change is one PR. Changes are ordered so that each lands a working, reviewable slice without breaking CI for downstream changes. **C-001** must precede everything because it adds the multi-stage scaffolding the later changes layer onto.

### C-001 — Dockerfile scaffold + cargo-binstall infrastructure
**Branch:** `feat/dockerfile-wasm-scaffold`
**Files:** `Dockerfile.rust-dev`
**Effort:** S
**Recommended agent:** `claude` (direct edits)

Add the multi-stage scaffolding the later changes need:

- [ ] Switch `Dockerfile.rust-dev` to a multi-stage build so heavyweight tool installs can be parallelized via `--mount=type=cache`.
- [ ] Install `cargo-binstall` (prebuilt) — every later `cargo install` should prefer binstall to avoid 20+ min cold compiles.
- [ ] Add `--mount=type=cache,target=/root/.cache` and `--mount=type=cache,target=/usr/local/cargo/registry` to long-running install layers.
- [ ] Add a final smoke-check `RUN` that lists installed tool versions (build-time visibility; not a healthcheck).

**Done when:** `docker build -f Dockerfile.rust-dev -t librefang-rust-dev:c001 .` succeeds; existing Tauri check workflow still passes.

---

### C-002 — Rust nightly + WASM Rust toolchain in dev image
**Branch:** `feat/dockerfile-rust-wasm`
**Files:** `Dockerfile.rust-dev`
**Depends on:** C-001
**Effort:** M
**Recommended agent:** `claude`

Layer Rust nightly and the full Rust WASM CLI suite:

- [ ] `rustup toolchain install nightly --component rust-src,rust-analyzer,clippy,rustfmt` (default stays stable)
- [ ] `rustup target add wasm32-unknown-unknown wasm32-wasip1 wasm32-wasip2` for both stable and nightly
- [ ] `cargo binstall --no-confirm`:
  - `wasmtime-cli@26`
  - `wasm-tools`
  - `cargo-component@0.21`
  - `wit-bindgen-cli@0.36`
  - `wasm-bindgen-cli@0.2.120`
  - `wasm-pack`
  - `cargo-wasi`
  - `twiggy`
  - `wasmer-cli@5` (D4 inclusion)
- [ ] Drop the wasmtime C-API shared lib (`libwasmtime.so` + `wasmtime.h`) into `/usr/local/lib` and `/usr/local/include` (sourced from the wasmtime release tarball). This is the artifact C-007 will copy into the production runtime.

**Done when:** all `--version` invocations in the dev image succeed; `cargo +nightly check --target wasm32-wasip2 -p librefang-types` succeeds (smoke-only — no actual code change to librefang).

---

### C-003 — Python 3.13 + uv + pyo3 + Python→WASM
**Branch:** `feat/dockerfile-python-wasm`
**Files:** `Dockerfile.rust-dev`
**Depends on:** C-001
**Effort:** M
**Recommended agent:** `claude`

Add Python 3.13 alongside (not replacing) system python3:

- [ ] Copy `uv` from `ghcr.io/astral-sh/uv:0.8` (multi-stage `COPY --from=...`)
- [ ] `uv python install 3.13` into `/opt/python313`; symlink as `python3.13`
- [ ] `uv tool install maturin` (for pyo3 wheel builds)
- [ ] `uv tool install componentize-py@0.16` (primary Python→WASM path)
- [ ] `pip install py2wasm` via uv pip into a dedicated venv at `/opt/py2wasm-venv` (avoids polluting system python)
- [ ] Optional Pyodide build tooling: `uv tool install pyodide-build@0.27` (D4 inclusion — ~600 MB; if dropping, gate behind a build arg)

**Done when:** `python3.13 --version`, `uv --version`, `maturin --version`, `componentize-py --version` all succeed; system `python3` unchanged at 3.11.

---

### C-004 — Node.js 24 LTS + pnpm + Bun + TypeScript 6 + JS/TS→WASM
**Branch:** `feat/dockerfile-node-wasm`
**Files:** `Dockerfile.rust-dev`
**Depends on:** C-001
**Effort:** M
**Recommended agent:** `claude`

Add a polyglot JS/TS layer (separate from the dashboard build, which lives in the prod `Dockerfile`):

- [ ] Install Node.js 24 LTS from the NodeSource tarball or `node:24-bookworm-slim` artifact (extract into `/opt/node24`, prepend to PATH).
- [ ] `corepack enable && corepack prepare pnpm@10.33.0 --activate`
- [ ] Install Bun 1.2 via `curl -fsSL https://bun.sh/install | bash` (lands in `/root/.bun/bin`; move to `/usr/local/bin/bun`).
- [ ] `pnpm add -g typescript@6 tsx assemblyscript javy-cli @bytecodealliance/jco porffor`
  - AssemblyScript (primary TS→WASM)
  - Javy CLI (JS runtime-bundled WASM)
  - jco (JS Component Model tooling)
  - Porffor (AOT JS→WASM; pre-1.0, accept failure if not yet installable globally)

**Done when:** `node --version` ≥ 24.0, `pnpm --version` ≥ 10.33, `bun --version`, `tsc --version` ≥ 6.0, `asc --version`, `javy --version` all succeed.

---

### C-005 — Go 1.26.3 + TinyGo 0.41.1 + Go→WASM
**Branch:** `feat/dockerfile-go-wasm`
**Files:** `Dockerfile.rust-dev`
**Depends on:** C-001
**Effort:** S
**Recommended agent:** `claude`

- [ ] Download Go 1.26.3 from `go.dev/dl/go1.26.3.linux-${ARCH}.tar.gz` (handle both amd64 and arm64); extract to `/usr/local/go`; add `/usr/local/go/bin` to PATH.
- [ ] Install TinyGo 0.41.1 via the official `.deb` (trixie-compatible) into `/usr/local/tinygo`.
- [ ] Smoke-build a 5-line Go program with `GOOS=wasip1 GOARCH=wasm go build` and `tinygo build -target=wasip2` at image-build time to catch silent breakage.

**Done when:** `go version` ≥ 1.26.3, `tinygo version` ≥ 0.41.1, both smoke-builds produce valid `.wasm` files.

---

### C-006 — C/C++ → WASM + Binaryen
**Branch:** `feat/dockerfile-c-wasm`
**Files:** `Dockerfile.rust-dev`
**Depends on:** C-001
**Effort:** S
**Recommended agent:** `claude`

- [ ] Install `wasi-sdk` 24+ from GitHub releases into `/opt/wasi-sdk`; symlink `clang` for WASI as `wasi-clang`.
- [ ] Install Binaryen (provides `wasm-opt`, `wasm-as`, `wasm-dis`) — prefer apt if trixie has a sufficiently recent package, else download release tarball.
- [ ] (D4) Install Emscripten 4.0.x via `emsdk` into `/opt/emsdk` — ~400 MB; if dropping, gate behind `ARG INSTALL_EMSCRIPTEN=1`.

**Done when:** `wasi-clang --version`, `wasm-opt --version` succeed; (if D4) `emcc --version` succeeds.

---

### C-007 — Production runtime: ship wasmtime runtime libs only
**Branch:** `feat/dockerfile-runtime-wasm-libs`
**Files:** `Dockerfile`
**Depends on:** C-002 (which produces the libwasmtime.so artifact pattern)
**Effort:** S
**Recommended agent:** `claude`

Path 1 from D1: production image gains **only** the wasmtime shared library and `wasi-sdk` runtime sysroot — no CLI tools, no compilers.

- [ ] In the final-stage image, `COPY --from=builder /usr/local/lib/libwasmtime.so* /usr/local/lib/` and `COPY --from=builder /usr/local/include/wasmtime.h /usr/local/include/` (or pull from a pinned wasmtime release tarball at build time — pick whichever produces a smaller layer).
- [ ] Add `ldconfig` after the copy.
- [ ] Do **not** install any cargo / pnpm / bun / go / python3.13 tooling into the runtime image.
- [ ] Add a build-time size assertion to `deploy/docker-entrypoint.sh` or a CI step: final image size must be within 100 MB of the pre-change baseline.

**Done when:** `docker images bossfang:test` size grew by < 100 MB; `ldd /usr/local/lib/libwasmtime.so` resolves cleanly inside the image; existing GCR deployment still pulls and boots.

---

### C-008 — Verification: per-language WASM smoke-test corpus
**Branch:** `feat/dockerfile-wasm-smoke-tests`
**Files:** `scripts/test-wasm-toolchain.sh` (new), CI workflow integration
**Depends on:** C-002 … C-006 all merged
**Effort:** M
**Recommended agent:** `claude`

A single shell script + matching CI job that, inside the dev image, compiles a trivial "hello world" component in each language and runs it through `wasmtime run --wasi`. This is the long-term regression guard for the toolchain.

- [ ] `scripts/test-wasm-toolchain.sh` — one function per language:
  - `test_rust_wasm` → `cargo +nightly build --target wasm32-wasip2`
  - `test_python_wasm` → `componentize-py componentize hello -o hello.wasm`
  - `test_typescript_wasm` → `asc hello.ts -o hello.wasm`
  - `test_javascript_wasm` → `javy compile hello.js -o hello.wasm`
  - `test_go_wasm` → `tinygo build -target=wasip2 -o hello.wasm hello.go`
  - `test_c_wasm` → `wasi-clang hello.c -o hello.wasm`
- [ ] Each function must run `wasmtime run hello.wasm` and assert stdout.
- [ ] Wire as a CI job that runs only when `Dockerfile.rust-dev` changes.

**Done when:** `./scripts/test-wasm-toolchain.sh` exits 0 inside a fresh dev image; CI job passes.

---

### C-009 — Documentation: polyglot dev image + plugin compile recipes
**Branch:** `docs/dockerfile-polyglot-dev`
**Files:** `docs/development/polyglot-dev-image.md` (new), update top-level Dockerfile headers
**Depends on:** C-008
**Effort:** S
**Recommended agent:** `claude`

- [ ] Document the wrapper-script flow (`LIBREFANG_RUST_IMAGE=librefang-rust-dev:latest cargo ...`) extended for `wasmtime`, `tinygo`, `componentize-py`, etc.
- [ ] Recipe per language: "compile a librefang plugin in <lang>" — minimum-viable hello-world.
- [ ] Note that this image is **not** the production image; plugins must be pre-compiled before being shipped.

**Done when:** docs render cleanly; each recipe has been executed at least once against the actual image.

---

## Out of scope for this phase

Not part of these 9 changes — separate KBD changes when the time comes:

- The actual Rust crate `librefang-plugin-host` that loads `.wasm` plugins. Toolchain only.
- Plugin registry / distribution channel (will land alongside `librefang-registry`).
- A sidecar `librefang-plugin-builder` image (D1 Path 3). Defer until the polyglot dev image proves load-bearing.
- WASI 0.3 async support (upstream still pre-stable).
- WasmEdge AI/tensor plugins.

---

## Risks to manage during execution

| Risk | Affects | Mitigation |
|---|---|---|
| Dev image > 14 GB CI disk | C-002…C-006 | Aggressive multi-stage; prune `/var/lib/apt/lists`, `/root/.cache/cargo` per layer; consider matrix split if a single image still bloats. |
| `cargo install` time-outs in CI | C-002 | `cargo-binstall` for every CLI; install in parallel where Dockerfile syntax permits via separate RUN layers. |
| Bun 1.2 SQLite collisions | C-004 | Bun is self-contained at `/usr/local/bin/bun`; no apt deps. |
| TypeScript 6 breaks dashboard build | C-004, C-007 boundary | Dashboard image (`dashboard-builder` stage in prod Dockerfile) keeps its own pinned TS in `node_modules`. Global TS 6 only present in dev image. |
| Porffor pre-1.0 install fails | C-004 | Soft-fail: if `pnpm add porffor` errors, log warning and continue (the other JS→WASM paths cover the need). |
| Emscripten install bloats image | C-006 (D4) | Gate behind `ARG INSTALL_EMSCRIPTEN=1`, default ON; allows easy revert if size becomes painful. |
| Production runtime image creep | C-007 | Reviewer checklist: no `apt-get install` of any compiler / language runtime in the final stage of `Dockerfile`. |
| Phase 3 M2 merge conflict | All | Hold the entire phase until Phase 3 M2 lands. Phase 4 work starts from post-merge HEAD. |

---

## Verification matrix (re-stated from assessment)

After all 9 changes merge:

| Check | Command | Acceptance |
|---|---|---|
| Prod image builds | `docker build -f Dockerfile -t bossfang:test .` | exit 0; size ≤ baseline + 100 MB |
| Dev image builds | `docker build -f Dockerfile.rust-dev -t librefang-rust-dev:test .` | exit 0 |
| All toolchains present | `./scripts/test-wasm-toolchain.sh` (inside dev image) | exit 0 |
| Brand audit | `python3 scripts/enforce-branding.py --check` | exit 0 |
| Compile-check | `cargo check --workspace --lib` (via wrapper using new dev image) | exit 0 |
| BossFang exclusives | `cargo check -p librefang-storage -p librefang-uar-spec -p librefang-memory` | exit 0 |
| Deployment | Push prod image to GCR; existing `bossfang` workload restarts | Pod ready |

---

## Suggested execution order summary

```
phase-3 M2 lands
       │
       ▼
   C-001 (scaffold) ──┬── C-002 (Rust+WASM Rust)
                      ├── C-003 (Python)
                      ├── C-004 (Node/Bun/TS)
                      ├── C-005 (Go)
                      └── C-006 (C/C++)
                            │
                            ▼
                         C-007 (prod runtime libs) — depends on C-002
                            │
                            ▼
                         C-008 (smoke tests)
                            │
                            ▼
                         C-009 (docs)
```

C-002 through C-006 can land in any order once C-001 is merged. C-007 needs the libwasmtime.so artifact pattern established in C-002.

---

## Next action

1. User confirms (or redirects) D1–D4 decisions baked above.
2. After Phase 3 M2 lands, run `/kbd-execute C-001` to start the scaffold change.
