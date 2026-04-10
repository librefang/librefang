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
    cargo clean -p librefang-runtime -p librefang-kernel --release 2>/dev/null; \
    cargo build --release --bin librefang && \
    cp target/release/librefang /usr/local/bin/librefang

# Stage 3: Runtime — Lazycat NAS customized
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates curl git ffmpeg python3 python3-pip chromium gosu sudo procps build-essential jq && rm -rf /var/lib/apt/lists/*
RUN ln -s /usr/bin/python3 /usr/bin/python
RUN curl -LsSf https://astral.sh/uv/install.sh | sh
RUN (type -p wget >/dev/null || (apt-get update && apt-get install -y wget)) && \
    mkdir -p -m 755 /etc/apt/keyrings && \
    out=$(mktemp) && wget -qO "$out" https://cli.github.com/packages/githubcli-archive-keyring.gpg && \
    cat "$out" | tee /etc/apt/keyrings/githubcli-archive-keyring.gpg > /dev/null && \
    chmod go+r /etc/apt/keyrings/githubcli-archive-keyring.gpg && \
    echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" | tee /etc/apt/sources.list.d/github-cli.list > /dev/null && \
    apt-get update && apt-get install -y gh && rm -rf /var/lib/apt/lists/*
RUN curl -fsSL https://deb.nodesource.com/setup_22.x | bash - && \
    apt-get install -y nodejs && \
    npm install -g @anthropic-ai/claude-code @qwen-code/qwen-code && \
    rm -rf /var/lib/apt/lists/*
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    python3 \
    python3-venv \
    pkg-config \
    libssl-dev \
    libicu72 \
    gosu \
    && rm -rf /var/lib/apt/lists/*
RUN useradd -m -s /bin/bash librefang && echo "librefang ALL=(ALL) NOPASSWD:ALL" > /etc/sudoers.d/librefang
USER librefang
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/home/librefang/.cargo/bin:${PATH}"
RUN NONINTERACTIVE=1 /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
RUN eval "$(/home/linuxbrew/.linuxbrew/bin/brew shellenv)" && brew install steipete/tap/gogcli
USER root
# Symlink brew-installed gog to /usr/local/bin so it's available in non-login
# shells (e.g. when Claude Code runs `sh -c "gog ..."`)
RUN ln -sf /home/linuxbrew/.linuxbrew/bin/gog /usr/local/bin/gog
RUN echo 'eval "$(/home/linuxbrew/.linuxbrew/bin/brew shellenv)"' >> /home/librefang/.bashrc && \
    echo 'eval "$(/home/linuxbrew/.linuxbrew/bin/brew shellenv)"' >> /root/.bashrc && \
    echo 'export PATH="/data/npm-global/bin:$PATH"' >> /home/librefang/.bashrc && \
    echo 'export PATH="/data/npm-global/bin:$PATH"' >> /root/.bashrc && \
    echo 'source /home/librefang/.cargo/env 2>/dev/null' >> /home/librefang/.bashrc
COPY --from=builder /usr/local/bin/librefang /usr/local/bin/
COPY --from=builder /build/packages /opt/librefang/packages
RUN mkdir -p /data && chown librefang:librefang /data
COPY entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh
EXPOSE 4545
VOLUME /data
ENV LIBREFANG_HOME=/data
ENTRYPOINT ["entrypoint.sh"]
