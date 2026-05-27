# syntax=docker/dockerfile:1

# ─────────────────────────────────────────────────────────────────────────────
# Stage 1 — Dashboard builder (Node.js 24 LTS)
# Updated from Node 20 to Node 24 (Active LTS as of 2026-05).
# Pin to a specific 24.x.x patch at release time for bit-for-bit reproducibility
# (check hub.docker.com/r/library/node for the current 24-alpine tag).
# Node ≥20.19.0 required by vite 8 / rolldown's optional native bindings.
FROM node:24-alpine AS dashboard-builder
# ─────────────────────────────────────────────────────────────────────────────

# Required for pnpm to run non-interactively (no TTY in docker build).
ENV CI=true
WORKDIR /build
COPY crates/librefang-api/dashboard ./dashboard
WORKDIR /build/dashboard
# corepack is refreshed first to avoid stale keyring errors when pnpm rotates
# signing keys. pnpm@10.33.0 matches the packageManager field in package.json.
RUN npm install --global corepack@latest \
    && corepack enable \
    && corepack prepare pnpm@10.33.0 --activate \
    && pnpm install --frozen-lockfile --ignore-scripts \
    && pnpm run build \
    # JS → WASM tools available in the dashboard / plugin build context
    && npm install -g \
        assemblyscript \
        @bytecodealliance/componentize-js \
        wabt

# ─────────────────────────────────────────────────────────────────────────────
# Stage 2 — Python 3.13 provider
# Debian bookworm's apt does not ship python3.13. We copy the official Python
# 3.13 build (binary, stdlib, shared library, dev headers) into later stages
# without changing their Debian base. This stage is a copy-source only.
FROM python:3.13-bookworm AS python-provider
# ─────────────────────────────────────────────────────────────────────────────

# ─────────────────────────────────────────────────────────────────────────────
# Stage 3 — Rust builder + full WASM toolchain
# Pinned to a specific minor matching the workspace MSRV (1.94.1).
FROM rust:1.94-slim-bookworm AS builder
# ─────────────────────────────────────────────────────────────────────────────
WORKDIR /build

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    pkg-config \
    libssl-dev \
    libdbus-1-dev \
    perl \
    ca-certificates \
    openssh-client \
    clang \
    libclang-dev \
    curl \
    # binaryen: wasm-opt optimizer; wabt: wat2wasm/wasm2wat/wasm-objdump
    binaryen \
    wabt \
    && rm -rf /var/lib/apt/lists/*

# ── Python 3.13 (headers + shared lib for pyo3 compilation) ──────────────────
COPY --from=python-provider /usr/local/bin/python3.13     /usr/local/bin/python3.13
COPY --from=python-provider /usr/local/bin/python3        /usr/local/bin/python3
COPY --from=python-provider /usr/local/lib/python3.13     /usr/local/lib/python3.13
COPY --from=python-provider /usr/local/include/python3.13 /usr/local/include/python3.13
COPY --from=python-provider /usr/local/lib/libpython3.13.so.1.0 \
                              /usr/local/lib/libpython3.13.so.1.0
RUN ln -sf /usr/local/lib/libpython3.13.so.1.0 /usr/local/lib/libpython3.13.so \
    && ldconfig
ENV PYO3_PYTHON=/usr/local/bin/python3.13

# ── Rust nightly + WASM targets ───────────────────────────────────────────────
# rustup is present in the rust: base image.
# Nightly is installed alongside stable; rust-toolchain.toml keeps stable as the
# default for the main daemon build. Nightly is used for plugin compilation and
# cutting-edge wasmtime/cranelift features.
RUN rustup toolchain install nightly \
        --profile minimal \
        --component rust-src \
    && rustup target add \
        wasm32-unknown-unknown \
        wasm32-wasip1 \
        wasm32-wasip2 \
    && rustup target add \
        wasm32-unknown-unknown \
        wasm32-wasip1 \
        wasm32-wasip2 \
        --toolchain nightly

# ── Go 1.26.3 ────────────────────────────────────────────────────────────────
RUN curl -fsSL https://go.dev/dl/go1.26.3.linux-amd64.tar.gz \
    | tar -xz -C /usr/local
ENV PATH="/usr/local/go/bin:${PATH}"

# ── TinyGo 0.41.1 (Go → WASM/WASI optimised compiler) ────────────────────────
RUN curl -fsSL \
    https://github.com/tinygo-org/tinygo/releases/download/v0.41.1/tinygo_0.41.1_amd64.deb \
    -o /tmp/tinygo.deb \
    && dpkg -i /tmp/tinygo.deb \
    && rm /tmp/tinygo.deb

# ── WASI-SDK 27.0 (C/C++ → WASM/WASI toolchain) ──────────────────────────────
ENV WASI_SDK_PATH=/opt/wasi-sdk
RUN curl -fsSL \
    https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-27/wasi-sdk-27.0-x86_64-linux.tar.gz \
    | tar -xz -C /opt \
    && mv /opt/wasi-sdk-27.0-x86_64-linux /opt/wasi-sdk
ENV CC_wasm32_wasip1="${WASI_SDK_PATH}/bin/clang --sysroot=${WASI_SDK_PATH}/share/wasi-sysroot"

# ── Cargo WASM tools ──────────────────────────────────────────────────────────
# Installed before source COPY so this expensive layer is cached independently
# of source changes. Binaries land at /usr/local/cargo/bin/ and are later
# copied into the runtime image.
#   wasm-pack          — Rust → WASM → npm workflow
#   wasm-bindgen-cli   — Rust ↔ JS interop glue generator
#   cargo-component    — WASM Component Model (.wit) builds
#   wit-bindgen-cli    — Generate language bindings from .wit interfaces
#   wasm-tools         — Merge/compose/validate/inspect WASM components
#   wasmtime-cli       — Run WASM/WASI modules; standalone runtime CLI
#   wasm-opt           — Binaryen WASM optimizer (cargo-built variant)
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo install \
        wasm-pack \
        wasm-bindgen-cli \
        cargo-component \
        wit-bindgen-cli \
        wasm-tools \
        wasmtime-cli \
        wasm-opt

# ── Source + daemon build ─────────────────────────────────────────────────────
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY xtask ./xtask
COPY packages ./packages
# librefang-channels embeds the Python SDK tree at compile time via
# include_dir! — without this COPY the proc macro panics.
COPY sdk/python/librefang ./sdk/python/librefang
COPY sdk/python/setup.py sdk/python/pyproject.toml ./sdk/python/
# librefang-api embeds deploy/ configs at compile time via include_str!.
COPY deploy ./deploy
COPY --from=dashboard-builder /build/static/react ./crates/librefang-api/static/react

RUN mkdir -p -m 0700 /root/.ssh \
    && ssh-keyscan github.com >> /root/.ssh/known_hosts

# wasmtime C-API shared library — staged here so the final runtime stage
# can `COPY --from=builder` libwasmtime.so + wasmtime.h without pulling
# the entire compile toolchain. Phase-4 (multilang plugin model) lands a
# plugin host that may dlopen wasmtime at runtime; shipping the C-API
# now means the artifact pattern is in place even though the librefang
# binary statically links wasmtime via Cargo and doesn't need it yet.
# Path 1 from D1 in the phase plan — wasmtime libs only, no CLI.
# Track WASMTIME_VERSION in lock-step with Dockerfile.rust-dev's pin.
ARG WASMTIME_VERSION=45.0.0
RUN set -eux; \
    arch="$(uname -m)"; \
    case "$arch" in \
        x86_64)  wt_arch="x86_64" ;; \
        aarch64) wt_arch="aarch64" ;; \
        *) echo "unsupported arch: $arch" >&2; exit 1 ;; \
    esac; \
    url="https://github.com/bytecodealliance/wasmtime/releases/download/v${WASMTIME_VERSION}/wasmtime-v${WASMTIME_VERSION}-${wt_arch}-linux-c-api.tar.xz"; \
    curl -fsSL "$url" -o /tmp/wasmtime-c-api.tar.xz; \
    mkdir -p /opt/wasmtime-c-api; \
    tar -xJf /tmp/wasmtime-c-api.tar.xz -C /opt/wasmtime-c-api --strip-components=1; \
    rm /tmp/wasmtime-c-api.tar.xz

RUN --mount=type=ssh \
    --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/build/target \
    SKIP_FRONTEND_BUILD=1 \
    SKIP_DASHBOARD_BUILD=1 \
    cargo build --release --bin librefang \
        --features telemetry,surreal-backend,uar-driver && \
    cp target/release/librefang /usr/local/bin/librefang

# ─────────────────────────────────────────────────────────────────────────────
# Stage 4 — Runtime image (Node.js 24 LTS)
# Updated from Node 22 to Node 24 (Active LTS as of 2026-05).
# Pin to a specific 24.x.x patch for bit-for-bit reproducibility
# (check hub.docker.com/r/library/node for the current bookworm-slim tag).
FROM node:24-bookworm-slim
# ─────────────────────────────────────────────────────────────────────────────

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    # libicu72: Unicode/i18n support for the Rust binary
    libicu72 \
    # libdbus-1-3: runtime SO for the keyring crate
    libdbus-1-3 \
    # gosu: privilege-drop helper for docker-entrypoint.sh
    gosu \
    # ncurses-term: provides xterm-256color terminfo for the terminal PTY
    ncurses-term \
    # Python 3.13 runtime system library dependencies
    libexpat1 \
    zlib1g \
    libbz2-1.0 \
    libffi8 \
    liblzma5 \
    libsqlite3-0 \
    libreadline8 \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# ── Python 3.13 ───────────────────────────────────────────────────────────────
COPY --from=python-provider /usr/local/bin/python3.13      /usr/local/bin/python3.13
COPY --from=python-provider /usr/local/lib/python3.13      /usr/local/lib/python3.13
COPY --from=python-provider /usr/local/lib/libpython3.13.so.1.0 \
                              /usr/local/lib/libpython3.13.so.1.0
RUN ln -sf /usr/local/bin/python3.13 /usr/local/bin/python3 \
    && ln -sf /usr/local/bin/python3.13 /usr/local/bin/python \
    && ln -sf /usr/local/lib/libpython3.13.so.1.0 /usr/local/lib/libpython3.13.so \
    && ldconfig

# ── uv (Python package manager) ───────────────────────────────────────────────
COPY --from=ghcr.io/astral-sh/uv:latest /uv  /usr/local/bin/uv
COPY --from=ghcr.io/astral-sh/uv:latest /uvx /usr/local/bin/uvx
ENV UV_SYSTEM_PYTHON=1

# ── Go 1.26.3 (copy from builder — avoids re-download) ────────────────────────
COPY --from=builder /usr/local/go /usr/local/go
ENV PATH="/usr/local/go/bin:${PATH}"

# ── TinyGo (copy from builder) ────────────────────────────────────────────────
COPY --from=builder /usr/lib/tinygo /usr/lib/tinygo
RUN ln -sf /usr/lib/tinygo/bin/tinygo /usr/local/bin/tinygo

# ── WASI-SDK (copy from builder) ──────────────────────────────────────────────
COPY --from=builder /opt/wasi-sdk /opt/wasi-sdk
ENV WASI_SDK_PATH=/opt/wasi-sdk

# ── WASM cargo tools (copy pre-built binaries; no Rust install needed for these)
COPY --from=builder \
    /usr/local/cargo/bin/wasm-pack \
    /usr/local/cargo/bin/wasm-bindgen \
    /usr/local/cargo/bin/cargo-component \
    /usr/local/cargo/bin/wit-bindgen \
    /usr/local/cargo/bin/wasm-tools \
    /usr/local/cargo/bin/wasmtime \
    /usr/local/cargo/bin/wasm-opt \
    /usr/local/bin/

# ── WABT tools (copy from builder's apt-installed binaryen/wabt) ──────────────
COPY --from=builder /usr/bin/wasm-opt     /usr/local/bin/wasm-opt-binaryen
COPY --from=builder /usr/bin/wat2wasm     /usr/local/bin/wat2wasm
COPY --from=builder /usr/bin/wasm2wat     /usr/local/bin/wasm2wat
COPY --from=builder /usr/bin/wasm-objdump /usr/local/bin/wasm-objdump

# ── Rust nightly for runtime plugin compilation ────────────────────────────────
# Installed under /opt/rust (not $HOME) so it is accessible to both root
# (entrypoint) and the librefang service user (daemon). RUSTUP_HOME / CARGO_HOME
# are set as image-level ENV vars; docker-entrypoint.sh and any plugin compiler
# code inherit them automatically.
ENV RUSTUP_HOME=/opt/rust/rustup \
    CARGO_HOME=/opt/rust/cargo
RUN curl https://sh.rustup.rs -sSf | sh -s -- -y \
        --default-toolchain none \
        --no-modify-path \
    && /opt/rust/cargo/bin/rustup toolchain install nightly \
        --profile minimal \
        --component rust-src \
    && /opt/rust/cargo/bin/rustup target add \
        wasm32-unknown-unknown \
        wasm32-wasip1 \
        wasm32-wasip2
ENV PATH="/opt/rust/cargo/bin:${PATH}"

# ── pnpm + JS WASM tooling ────────────────────────────────────────────────────
# Node is already on PATH from the base image.
RUN npm install --global corepack@latest \
    && corepack enable \
    && corepack prepare pnpm@10.33.0 --activate \
    && npm install -g \
        assemblyscript \
        @bytecodealliance/componentize-js

# ── bun ───────────────────────────────────────────────────────────────────────
RUN curl -fsSL https://bun.sh/install | bash \
    && mv /root/.bun/bin/bun /usr/local/bin/bun \
    && chmod +x /usr/local/bin/bun \
    && rm -rf /root/.bun

# ── Application setup ─────────────────────────────────────────────────────────
# Install the librefang Python SDK so sidecar adapters can run --describe at
# daemon boot and populate the channel configuration schema cache.
COPY --from=builder /build/sdk/python /opt/librefang/sdk/python
RUN uv pip install --no-cache /opt/librefang/sdk/python

RUN addgroup --system --gid 1001 librefang && \
    adduser --system --uid 1001 --ingroup librefang librefang && \
    # Allow the daemon user to invoke rustup/cargo for plugin compilation
    chown -R librefang:librefang /opt/rust

COPY --from=builder /usr/local/bin/librefang /usr/local/bin/
COPY --from=builder /build/packages /opt/librefang/packages
# wasmtime C-API runtime libs + headers — staged by the builder stage above.
# Path 1 from the phase-4 plan: ship the shared lib + header so a future
# plugin host can dlopen wasmtime, but DO NOT ship the wasmtime CLI or any
# compile toolchain. Re-run ldconfig so the loader sees the new .so.
COPY --from=builder /opt/wasmtime-c-api/lib/. /usr/local/lib/
COPY --from=builder /opt/wasmtime-c-api/include/. /usr/local/include/
RUN ldconfig
COPY deploy/docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-entrypoint.sh
# CIS Docker Benchmark §4.1: restrict shell; chown package assets.
RUN usermod -s /sbin/nologin librefang && \
    chown -R librefang:librefang /opt/librefang/packages

EXPOSE 4545
ENV LIBREFANG_HOME=/data
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s \
  CMD curl -fsS http://127.0.0.1:${PORT:-4545}/api/health || exit 1
# docker-entrypoint.sh runs as root for bind-mount chown/init, then gosu drops
# to the librefang user before executing the daemon binary.
ENTRYPOINT ["docker-entrypoint.sh"]
CMD ["librefang", "start", "--foreground"]
