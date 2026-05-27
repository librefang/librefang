# Change C-008 — Per-language WASM smoke-test corpus + CI workflow

**Phase:** phase-4-multilang-wasm-toolchain
**Status:** DONE
**Completed:** 2026-05-27
**Files touched:** `scripts/test-wasm-toolchain.sh` (new),
                   `.github/workflows/wasm-toolchain.yml` (new)

## What landed

### `scripts/test-wasm-toolchain.sh`
Single-file regression guard for the polyglot WASM dev image. Per
language: compile a "hello-${lang}-wasm" program to a `.wasm` artefact,
run it through wasmtime (or `wasm-tools validate` for componentize-py),
and assert the expected output / shape.

Languages exercised:
- **Rust** — `cargo +nightly build --target wasm32-wasip2`
- **Python** — `componentize-py -d . -w hello componentize hello` (built +
  validated via `wasm-tools validate` + `wasm-tools component wit`;
  invocation requires generated host bindings which is out of scope for
  a smoke test).
- **TypeScript** — `asc --runtime stub --use abort=` direct WASI fd_write
- **JavaScript** — `javy build`
- **Go** — `tinygo build -target=wasip2`
- **C** — `wasi-clang`

Failure aggregation: all six tests run unconditionally so CI shows the
full picture per build; the script exits non-zero only at the end if
any failed.

### `.github/workflows/wasm-toolchain.yml`
GitHub Actions workflow that:
1. Triggers only on changes to `Dockerfile.rust-dev`,
   `scripts/test-wasm-toolchain.sh`, or the workflow itself
   (multi-GB image rebuild — don't run on unrelated PRs).
2. Builds `librefang-rust-dev:ci` with GHA cache (`type=gha`,
   `scope=wasm-toolchain`, `mode=max`).
3. Runs the smoke script inside the freshly-built image.

## Verification

```
docker run --rm -v "$PWD":/workspace -w /workspace \
    librefang-rust-dev:c006-test \
    /workspace/scripts/test-wasm-toolchain.sh
```

Final result: **All 6 language WASM toolchains green.**

| Language | Toolchain | Output |
|---|---|---|
| rust | rustc 1.98.0-nightly + cargo + wasm32-wasip2 | `hello-rust-wasm` |
| python | Python 3.13.7 + componentize-py 0.23.0 | component validated |
| typescript | AssemblyScript 0.28.17 | `hello-typescript-wasm` |
| javascript | Javy 5.0.4 | `hello-javascript-wasm` |
| go | TinyGo 0.41.1 (wasip2) | `hello-go-wasm` |
| c | wasi-clang 19.1.5-wasi-sdk | `hello-c-wasm` |

## Issues fixed during iteration

1. **componentize-py: missing `-d`** — `-w <world>` requires `-d <wit-dir>`
   as a sibling flag. Without `-d`, componentize-py errors with "failed
   to read path for WIT".
2. **componentize-py: expects class, not function** — the world's exports
   are looked up as attributes of a `WitWorld` class inside the python
   module, not as top-level functions.
3. **componentize-py: can't `wasmtime run --invoke`** — produced
   components don't target `wasi:cli/run`, so wasmtime can't auto-invoke
   them. Generic invocation requires generated host bindings. Smoke
   test verifies build + `wasm-tools validate` instead.
4. **AssemblyScript stdlib + wasi-shim** — `import "@assemblyscript/wasi-shim"`
   fails AS module resolution (it's not a normal node module). Replaced
   with raw `wasi_snapshot_preview1.fd_write` at fixed linear-memory
   offsets, plus `--runtime stub` + `--use abort=` to strip the GC
   runtime and stub the abort import.

## QA-gate note

This change touches 2 new files (`<3` threshold not crossed). QA-gate skipped.
