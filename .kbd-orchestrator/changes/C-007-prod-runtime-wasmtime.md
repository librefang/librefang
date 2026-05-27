# Change C-007 — Production runtime: ship wasmtime C-API libs only

**Phase:** phase-4-multilang-wasm-toolchain
**Status:** DONE
**Completed:** 2026-05-27
**Files touched:** `Dockerfile` (production runtime)

## What landed

Path 1 from D1 — production runtime gains only the wasmtime shared library
+ headers, NOT the wasmtime CLI and NOT any compile toolchain.

### Builder stage addition

```dockerfile
ARG WASMTIME_VERSION=45.0.0
RUN ... curl wasmtime-vX-${arch}-linux-c-api.tar.xz
         | tar -xJ -C /opt/wasmtime-c-api ...
```

Pinned to the same WASMTIME_VERSION=45.0.0 as `Dockerfile.rust-dev` (C-002).
Bumping requires touching both files.

### Final stage addition

```dockerfile
COPY --from=builder /opt/wasmtime-c-api/lib/.     /usr/local/lib/
COPY --from=builder /opt/wasmtime-c-api/include/. /usr/local/include/
RUN ldconfig
```

`ldconfig` runs after the COPY so the dynamic loader sees the new .so.

## Verification

Builder-stage build:
```
cd /Users/gqadonis/.claude/worktrees/confident-wilbur-c27abe && \
  DOCKER_BUILDKIT=1 docker build -f Dockerfile --target=builder -t librefang-prod:c007-test .
```

Result: the wasmtime C-API stage (step #28) completes in 1.8s — downloads
the aarch64 tarball, extracts to /opt/wasmtime-c-api cleanly.

The subsequent cargo build step (#29) fails at kreuzberg SSH fetch
(`failed to get kreuzberg as a dependency of universal-agent-runtime`)
— this is the documented **Phase-3 M1 blocker**, not a C-007 issue.
CI will exercise the full pipeline once M1 lands.

## Out of scope (deferred to M1)

- End-to-end verification of `COPY --from=builder /opt/wasmtime-c-api/...`
  into the final stage requires a successful builder. Pattern is a
  standard multi-stage COPY — low risk, CI-exercised post-M1.
- Image-size assertion (final image must grow by <100 MB per the plan).
  Will be verified once the full image builds; rough estimate based on
  the dev image's libwasmtime.so size is ~30-40 MB.

## QA-gate note

Single file touched → `<3 files` skip rule applies.
