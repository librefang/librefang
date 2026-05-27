# Change C-005 — Go 1.26.3 + TinyGo 0.41.1 + Go→WASM

**Phase:** phase-4-multilang-wasm-toolchain
**Status:** DONE
**Completed:** 2026-05-27
**Files touched:** `Dockerfile.rust-dev`

## What landed

- **Go 1.26.3** (released 2026-05-07) from go.dev tarball → `/usr/local/go`,
  symlinked `go` + `gofmt` onto `/usr/local/bin`.
- **TinyGo 0.41.1** (released 2026-04-22) via the upstream `.deb` package
  (pulls LLVM 20.1.1 + lld transitively).
- **Image-build-time smoke build**: both `GOOS=wasip1 GOARCH=wasm go build`
  and `tinygo build -target=wasip2` produce non-empty `.wasm` files.

## Verification

```
cd /Users/gqadonis/.claude/worktrees/confident-wilbur-c27abe && \
  DOCKER_BUILDKIT=1 docker build -f Dockerfile.rust-dev -t librefang-rust-dev:c005-test .
```

Exit 0. Smoke check confirmed:
- `go version` → go1.26.3 linux/arm64
- `tinygo version` → 0.41.1 linux/arm64 (Go 1.26.3, LLVM 20.1.1)

## Issues hit + fixed

1. **Self-referential symlink on tinygo** — I added `ln -sf /usr/local/bin/tinygo /usr/local/bin/tinygo` "for parity" with the Go install. The TinyGo .deb already places the binary at /usr/local/bin/tinygo, so the ln overwrote a real binary with a self-targeting symlink → `Too many levels of symbolic links` at first invocation. Removed the redundant ln.

## QA-gate note

Single file → `<3 files` skip rule applies.
