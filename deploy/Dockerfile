# syntax=docker/dockerfile:1

# Stage 1: Build React dashboard
FROM node:20-alpine AS dashboard-builder
WORKDIR /build
COPY crates/librefang-api/dashboard ./dashboard
WORKDIR /build/dashboard
RUN npm install && npm run build

# Stage 2: Build Rust binary
FROM rust:1-slim-bookworm AS builder
WORKDIR /build
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    pkg-config \
    libssl-dev \
    perl \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY xtask ./xtask
COPY packages ./packages
COPY --from=dashboard-builder /build/static/react ./crates/librefang-api/static/react

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/build/target \
    cargo build --release --bin librefang && \
    cp target/release/librefang /usr/local/bin/librefang

FROM node:lts-bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    python3 \
    python3-venv \
    libicu72 \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/local/bin/librefang /usr/local/bin/
COPY --from=builder /build/packages /opt/librefang/packages
COPY deploy/docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-entrypoint.sh
RUN mkdir -p /data && chown node:node /data
EXPOSE 4545
ENV LIBREFANG_HOME=/data
USER node
ENTRYPOINT ["docker-entrypoint.sh"]
CMD ["librefang", "start", "--foreground"]
