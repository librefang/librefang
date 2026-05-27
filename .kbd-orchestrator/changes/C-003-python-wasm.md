# Change C-003 — Python 3.13 + uv + pyo3 + Python→WASM

**Phase:** phase-4-multilang-wasm-toolchain
**Status:** DONE
**Completed:** 2026-05-27
**Files touched:** `Dockerfile.rust-dev`

## What landed

- **`uv` 0.8.24** copied in from `ghcr.io/astral-sh/uv:0.8` multi-stage.
- **UV layout** via env: `UV_PYTHON_INSTALL_DIR=/opt/uv/python`,
  `UV_TOOL_DIR=/opt/uv/tools`, `UV_TOOL_BIN_DIR=/usr/local/bin`,
  `UV_COMPILE_BYTECODE=1`, `UV_LINK_MODE=copy`. Tool shims land on PATH;
  no shell-init required.
- **Python 3.13.7** installed via uv; exposed at `/usr/local/bin/python3.13`.
  System `python3` (trixie ships 3.13.5) left untouched.
- **Plugin-author tools** via `uv tool install --python 3.13`:
  - `maturin` 1.13.3
  - `componentize-py` 0.23.0 (primary Python → WASI 0.2 Component path)
  - `py2wasm` (argparse CLI — no `--version`; auto-bundles wasi-sdk 21 ~75 MB)
  - `pyodide-cli` 0.5.0 (pulls `pyodide-build` 0.34.4 as a dep)

## Verification

```
DOCKER_BUILDKIT=1 docker build -f Dockerfile.rust-dev -t librefang-rust-dev:c003-final .
```

Exit 0. Smoke check confirmed every tool above.

## Issues hit + fixed

1. **`uv tool install pyodide-build` failed** — that package has no
   `console_scripts` entry point. The binary lives in the `pyodide-cli`
   package, which depends on `pyodide-build`. Fix: install `pyodide-cli`
   so we get the build toolchain transitively + the `pyodide` binary on PATH.
2. **`py2wasm --version` failed** — argparse-only CLI; `--version` is
   parsed as an unknown flag and falls through to required-args error.
   Fix: use `py2wasm --help >/dev/null` as the presence probe.
3. **Stale comment about trixie python** — said "trixie 3.12.x"; trixie
   actually ships 3.13.5. Corrected in the Dockerfile.

## Image growth

This change adds substantial weight to the dev image:

| Component | Approx size |
|---|---|
| uv binary | 30 MB |
| CPython 3.13 (uv-managed) | 110 MB |
| maturin venv | 30 MB |
| componentize-py venv (bundles Rust wasm libs) | 120 MB |
| py2wasm venv (Nuitka + wasi-sdk 21 bundled) | 200 MB |
| pyodide-cli venv | 70 MB |
| **Total C-003 layer** | **~560 MB** |

Within the budget called out in the assessment (dev image trending toward ~4.5 GB).

## QA-gate note

Single file touched → `<3 files` skip rule applies. C-004 will also be
single-file. The QA-required changes will be C-008 (multi-file smoke test
script + CI workflow integration) and C-009 (docs).
