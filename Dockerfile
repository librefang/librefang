# syntax=docker/dockerfile:1
FROM rust:1-slim-bookworm AS builder
WORKDIR /build
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    pkg-config \
    libssl-dev \
    perl \
    perl-modules \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY xtask ./xtask
COPY agents ./agents
COPY packages ./packages
RUN cargo build --release --bin librefang

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/librefang /usr/local/bin/
COPY --from=builder /build/agents /opt/librefang/agents
EXPOSE 4545
VOLUME /data
ENV LIBREFANG_HOME=/data
ENTRYPOINT ["librefang"]
CMD ["start", "--foreground"]
