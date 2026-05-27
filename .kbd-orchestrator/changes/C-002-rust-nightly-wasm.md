# Change C-002 — Rust nightly + WASM Rust toolchain

**Phase:** phase-4-multilang-wasm-toolchain
**Status:** DONE
**Completed:** 2026-05-27
**Files touched:** `Dockerfile.rust-dev`

## What landed

Layered onto C-001's scaffold:

- **`rustup toolchain install nightly`** with `rust-src`, `rust-analyzer`,
  `clippy`, `rustfmt`. Default toolchain remains stable. Opt in to nightly
  via `cargo +nightly`.
- **WASM targets on BOTH stable and nightly**: `wasm32-unknown-unknown`,
  `wasm32-wasip1`, `wasm32-wasip2`.
- **Rust WASM CLI suite** via `cargo binstall --no-confirm --locked`:
  - `wasmtime-cli` → wasmtime 45.0.0
  - `wasm-tools` 1.250.0
  - `cargo-component` 0.21.1
  - `wit-bindgen-cli` 0.57.1
  - `wasm-bindgen-cli` 0.2.122
  - `wasm-pack` 0.15.0
  - `cargo-wasi`
  - `twiggy` 0.8.0
  - `wasmer-cli` 7.1.0 (cross-runtime sanity check)
- **wasmtime C-API shared lib + headers** from the v45.0.0 release tarball:
  `/usr/local/lib/libwasmtime.so*` + `/usr/local/include/wasmtime.h`.
  `ldconfig` run after copy. C-007 (prod runtime) will `COPY --from=builder`
  these artefacts.

## Verification

```
DOCKER_BUILDKIT=1 docker build -f Dockerfile.rust-dev -t librefang-rust-dev:c002-final .
```

Exit 0. Smoke-check confirmed:

| Tool | Reported version |
|---|---|
| rustc (stable) | 1.95.0 (2026-04-14) |
| cargo (stable) | 1.95.0 (2026-03-21) |
| cargo +nightly | 1.98.0-nightly (2026-05-15) |
| stable WASM targets | wasm32-unknown-unknown, wasm32-wasip1, wasm32-wasip2 |
| nightly WASM targets | wasm32-unknown-unknown, wasm32-wasip1, wasm32-wasip2 |
| cargo-binstall | 1.19.1 |
| wasmtime CLI | 45.0.0 (2026-05-21) |
| wasm-tools | 1.250.0 (2026-05-21) |
| cargo-component | 0.21.1 |
| wit-bindgen-cli | 0.57.1 (2026-04-17) |
| wasm-bindgen | 0.2.122 |
| wasm-pack | 0.15.0 |
| twiggy | 0.8.0 |
| wasmer | 7.1.0 |
| libwasmtime.so | present at /usr/local/lib |
| wasmtime.h | present at /usr/local/include |

## Issues hit + fixed during execution

1. **`rustup toolchain install --component` parses bare args after the
   first as additional toolchain names** — initial run errored with
   `invalid value 'rust-analyzer' for '[TOOLCHAIN]...'`. Fix: pass each
   component as its own `--component` flag.
2. **WASMTIME_VERSION pin drifted from cargo-binstall's actual install
   target** — initial pin at 26.0.1 mismatched the binstall'd 45.0.0
   wasmtime CLI. Fix: bump ARG to 45.0.0 so the CLI and C-API tarball
   share a major.

## QA-gate note

`/refine-validate` skip rule: this change touched 1 file. QA skipped per
the "fewer than 3 files" rule in kbd-execute. The next QA-required change
is C-003 (Python 3.13 + uv + componentize-py + py2wasm + pyodide-build —
touches >=3 files via multi-stage uv `COPY --from`).
