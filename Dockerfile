# syntax=docker/dockerfile:1
FROM rust:1.85-slim-bookworm AS builder
WORKDIR /build
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    pkg-config \
    libssl-dev \
    perl \
    perl-modules \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Build dependencies first (cached unless Cargo.toml/Cargo.lock change)
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY xtask ./xtask
COPY agents ./agents
COPY catalog ./catalog
COPY packages ./packages
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/build/target \
    cargo build --release --bin librefang \
    && cp target/release/librefang /usr/local/bin/librefang

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    python3 \
    python3-venv \
    nodejs \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/local/bin/librefang /usr/local/bin/
COPY --from=builder /build/agents /opt/librefang/agents
COPY --from=builder /build/packages /opt/librefang/packages
COPY deploy/docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-entrypoint.sh
EXPOSE 4545
ENV LIBREFANG_HOME=/data
ENTRYPOINT ["docker-entrypoint.sh"]
CMD ["librefang", "start", "--foreground"]
