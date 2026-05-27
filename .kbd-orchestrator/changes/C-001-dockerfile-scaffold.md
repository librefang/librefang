# Change C-001 — Dockerfile.rust-dev scaffold + cargo-binstall

**Phase:** phase-4-multilang-wasm-toolchain
**Branch:** working in linked worktree `claude/confident-wilbur-c27abe`
  (plan suggested `feat/dockerfile-wasm-scaffold` — worktree branch is fine for this single-change flow; rename at PR time).
**Status:** DONE
**Completed:** 2026-05-27
**Verification:** `DOCKER_BUILDKIT=1 docker build -f Dockerfile.rust-dev -t librefang-rust-dev:c001-test .` → exit 0; rustc 1.95.0, cargo 1.95.0, cargo-binstall 1.19.1, pkg-config 1.8.1.
**Started:** 2026-05-27
**Files touched:** `Dockerfile.rust-dev`

## Intent

Establish the scaffolding the rest of phase-4 (C-002 through C-006) layers
toolchains onto:

1. BuildKit cache mounts on apt so parallel install stages don't repeatedly
   download .deb files.
2. `curl`, `xz-utils`, `unzip` in the base layer — every later installer
   (cargo-binstall, wasi-sdk tarball, Bun, TinyGo .deb, Node tarball,
   Go tarball, uv multi-stage) needs at least one of these.
3. `cargo-binstall` installed up-front — turns C-002's WASM CLI install
   layer from a ~20-minute compile pass into a ~2-minute download pass.
4. A final smoke-check `RUN` listing tool versions. C-002…C-006 will append
   their own version lines so a single failed `docker build` pinpoints
   which layer regressed.

Intentionally **not** in C-001:

- Multi-stage build. Keeping it single-stage until there are toolchains
  worth parallelizing across stages (C-002+ era). Avoids speculative
  scaffolding.
- New toolchains. C-001 is infrastructure only.

## Verification

```bash
DOCKER_BUILDKIT=1 docker build -f Dockerfile.rust-dev -t librefang-rust-dev:c001-test .
```

Acceptance:

- Exit 0.
- Smoke-check RUN prints `rustc`, `cargo`, `cargo binstall`, `pkg-config`
  versions.
- Existing Tauri workflow (the `cargo` wrapper script that invokes this
  image) still works for `cargo check --workspace --lib`.

## QA-gate note

`/refine-validate` is skipped for this change per the kbd-execute QA rule:
"fewer than 3 files modified" (this change touches 1 file). Re-enable QA
starting at C-002, which begins introducing real toolchain payloads.
