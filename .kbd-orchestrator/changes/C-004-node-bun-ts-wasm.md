# Change C-004 — Node 24 LTS + pnpm + Bun + TypeScript 6 + JS/TS→WASM

**Phase:** phase-4-multilang-wasm-toolchain
**Status:** DONE
**Completed:** 2026-05-27
**Files touched:** `Dockerfile.rust-dev`

## What landed

- **Node.js 24.10.0** (Active LTS) from upstream tarball → `/opt/node24`,
  symlinked onto `/usr/local/bin/{node,npm,npx,corepack}`.
- **pnpm 10.33.0** installed via `npm install -g pnpm@10.33.0` (corepack's
  shim creation was unreliable — see issue note below) + `PNPM_HOME=/usr/local/bin`.
- **Bun 1.3.14** via upstream installer, copied to `/usr/local/bin/bun`.
- **Globals via pnpm**: TypeScript 6.0.3, tsx 4.22.3, AssemblyScript 0.28.17,
  @bytecodealliance/jco 1.19.0.
- **Javy 5.0.4** from GitHub release gz (not on npm).

## Verification

```
cd /Users/gqadonis/.claude/worktrees/confident-wilbur-c27abe && \
  DOCKER_BUILDKIT=1 docker build -f Dockerfile.rust-dev -t librefang-rust-dev:c004-test .
```

Exit 0; smoke-check confirmed every tool above.

## Issues hit + fixed

1. **`pnpm: not found` after corepack** — corepack does not pre-create the
   `/opt/node24/bin/pnpm` shim on `enable`; the static `ln -sf` resolved to
   nothing. Switched to `npm install -g pnpm@${PNPM_VERSION}` which
   deterministically lands the binary at `/opt/node24/bin/pnpm`.
2. **`ERR_PNPM_NO_GLOBAL_BIN_DIR`** — pnpm refuses `add -g` until
   `PNPM_HOME` is set to an on-PATH directory. Added `ENV PNPM_HOME=/usr/local/bin`.
3. **`@bytecodealliance/javy-cli` 404 on npm** — Javy isn't published to
   npm; ships as a GitHub release binary. Split it into a dedicated
   `RUN` that downloads `javy-<arch>-linux-v${JAVY_VERSION}.gz`.
4. **Wrong cwd after worktree move** — the harness's tracked cwd lagged
   one turn. Subsequent docker builds use explicit `cd /Users/gqadonis/.claude/worktrees/confident-wilbur-c27abe`.

## QA-gate note

Single file → `<3 files` skip rule applies.
