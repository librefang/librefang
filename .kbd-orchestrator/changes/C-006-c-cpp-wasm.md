# Change C-006 — C/C++ → WASM (wasi-sdk + Binaryen + Emscripten)

**Phase:** phase-4-multilang-wasm-toolchain
**Status:** DONE
**Completed:** 2026-05-27
**Files touched:** `Dockerfile.rust-dev`

## What landed

- **wasi-sdk 25** at `/opt/wasi-sdk` (clang 19.1.5, target `wasm32-unknown-wasi`)
  + `wasi-clang` PATH wrapper preselecting the WASI sysroot.
- **Binaryen 123** at `/opt/binaryen` + symlinks for `wasm-opt`, `wasm-as`,
  `wasm-dis`, `wasm-merge` on `/usr/local/bin`.
- **Emscripten 4.0.7** via `emsdk` at `/opt/emsdk` + symlinks for `emcc`,
  `em++`, `emar`, `emranlib` on `/usr/local/bin`. `EMSDK_NODE` points at
  the Node 24 installed in C-004 so emscripten doesn't pull a second Node.

## Verification

```
cd /Users/gqadonis/.claude/worktrees/confident-wilbur-c27abe && \
  DOCKER_BUILDKIT=1 docker build -f Dockerfile.rust-dev -t librefang-rust-dev:c006-test .
```

Exit 0. Smoke check confirmed every tool above (clang/wasi-clang 19.1.5,
wasm-opt 123, emcc 4.0.7).

## Issues hit + fixed

1. **BINARYEN_VERSION drift** — set ARG to `121` but upstream's release
   tarball redirected to `123`. The build succeeded but recorded the wrong
   pin; bumped the ARG to 123 to reflect what the layer actually ships.

## Image growth (C-006 layer)

| Component | Approx size |
|---|---|
| wasi-sdk 25 (clang + sysroot) | ~600 MB |
| Binaryen 123 | ~80 MB |
| Emscripten 4.0.7 + node bits | ~400 MB |
| **C-006 total** | **~1.1 GB** |

Dev image now in the 4–5 GB range as predicted in the assessment.

## QA-gate note

Single file → `<3 files` skip rule applies.
