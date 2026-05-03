# syntax=docker/dockerfile:1

# Stage 1: Build React dashboard
# Pinned to a specific minor (not floating `node:20-alpine`) so rebuilds of a
# tagged release months later produce a bit-for-bit identical builder image.
# Track Node 20 LTS — CI's setup-node also uses node-version: 20
# (.github/workflows/ci.yml, .github/workflows/dashboard-build.yml).
FROM node:20.18.1-alpine AS dashboard-builder
WORKDIR /build
COPY crates/librefang-api/dashboard ./dashboard
WORKDIR /build/dashboard
RUN corepack enable \
    && pnpm install --frozen-lockfile --ignore-scripts \
    && pnpm run build

# Stage 2: Build Rust binary
# Pinned to a specific minor (not floating `rust:1-slim-bookworm`). Tracks the
# workspace MSRV declared in Cargo.toml's [workspace.package].rust-version
# (currently 1.94.1) so the build image is guaranteed to satisfy it.
FROM rust:1.94-slim-bookworm AS builder
WORKDIR /build
# libdbus-1-dev is required by libdbus-sys (transitive dep of keyring's
# sync-secret-service feature, added in #3180). Without it the cargo build
# panics with exit 101 in the build script — same root cause as #3259, and
# why the v2026.4.27-beta6 docker image was never published.
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    pkg-config \
    libssl-dev \
    libdbus-1-dev \
    perl \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY xtask ./xtask
COPY packages ./packages
# librefang-api uses include_str!("../../../deploy/...") to embed the
# observability stack (prometheus / tempo / otel-collector / grafana
# configs) at compile time — added in #3062. Without this COPY the
# build fails with "couldn't read deploy/grafana/...". flake.nix
# already lists the same paths in its source fileset.
COPY deploy ./deploy
COPY --from=dashboard-builder /build/static/react ./crates/librefang-api/static/react

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/build/target \
    # `--features all-channels`: the published Docker image is the full
    # daemon, so opt back in to every channel adapter. The CLI's `default`
    # was slimmed to a "core-channels" subset (see #3655 / #3688) to keep
    # developer cold-build time low; release/packaging pipelines re-enable
    # the full set explicitly.
    cargo build --release --bin librefang --features all-channels && \
    cp target/release/librefang /usr/local/bin/librefang

# Pinned to a specific Node 22 LTS minor (not floating `node:lts-bookworm-slim`)
# so a rebuild months later doesn't quietly land on a new major when the
# `lts` alias rolls forward. `curl` is added for the HEALTHCHECK below.
FROM node:22.11.0-bookworm-slim
# libdbus-1-3 = runtime SO that libdbus-sys links against. Without it the
# binary fails to start (the keyring init path runs early in boot and
# exits 101 if the .so can't be resolved).
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    python3 \
    python3-venv \
    libicu72 \
    libdbus-1-3 \
    gosu \
    && rm -rf /var/lib/apt/lists/*
RUN addgroup --system --gid 1001 librefang && \
    adduser --system --uid 1001 --ingroup librefang librefang
COPY --from=builder /usr/local/bin/librefang /usr/local/bin/
COPY --from=builder /build/packages /opt/librefang/packages
COPY deploy/docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-entrypoint.sh
# CIS Docker Benchmark §4.1: run the service as a dedicated non-root
# user with no login shell.  The user `librefang` (uid/gid 1001) is
# already created above via addgroup/adduser; the redundant
# `groupadd -r librefang && useradd -r ...` block introduced by #3948
# collides with that user — `groupadd` exits with code 9 ('group
# already exists'), breaking `docker build` on every clean tree.
# Apply the CIS shell-restriction with `usermod` instead, then chown
# /opt/librefang/packages so the runtime user can read its own asset
# tree (the COPYs above land as root:root by default).
RUN usermod -s /sbin/nologin librefang && \
    chown -R librefang:librefang /opt/librefang/packages
EXPOSE 4545
ENV LIBREFANG_HOME=/data
# Native restart-on-failure signal for orchestrators (Docker/Swarm/Compose;
# Kubernetes uses its own probes and ignores this). 20 s start-period gives
# the daemon time to bind, run `librefang init` on first boot, and start the
# axum server. The shell form is required so ${PORT:-4545} expands at
# runtime — Railway/Render/Fly inject $PORT and the entrypoint rewrites
# api_listen accordingly (see deploy/docker-entrypoint.sh).
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s \
  CMD curl -fsS http://127.0.0.1:${PORT:-4545}/api/health || exit 1
# docker-entrypoint.sh uses gosu to exec as the librefang user, so we
# keep the entrypoint itself running as root to allow bind-mount chown
# and data-dir initialisation before privilege drop.
ENTRYPOINT ["docker-entrypoint.sh"]
CMD ["librefang", "start", "--foreground"]
