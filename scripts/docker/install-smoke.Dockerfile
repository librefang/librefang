# Smoke test for install.sh
# Verifies the installer works in a clean environment.
#
# Usage (CI):
#   docker build -f scripts/docker/install-smoke.Dockerfile .
#
# Usage (full E2E — requires a published release):
#   docker build -f scripts/docker/install-smoke.Dockerfile \
#     --build-arg LIBREFANG_SMOKE_FULL=1 .

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    curl \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Create a non-root user (simulates real user install)
RUN useradd -m -s /bin/bash testuser
USER testuser
WORKDIR /home/testuser

# Copy the install script from the build context
COPY web/public/install.sh /tmp/install.sh

ARG LIBREFANG_SMOKE_FULL=0
RUN if [ "$LIBREFANG_SMOKE_FULL" = "1" ]; then \
        sh /tmp/install.sh; \
    else \
        # 1. Syntax check
        sh -n /tmp/install.sh && \
        echo "PASS: install.sh syntax is valid" && \
        # 2. Verify detect_platform works by extracting the function
        sh -c ' \
            eval "$(sed -n "/^detect_platform/,/^}/p" /tmp/install.sh)" && \
            detect_platform && \
            echo "PASS: platform detected as $PLATFORM" \
        ' && \
        # 3. Verify target matches release naming (musl preferred, gnu fallback)
        sh -c ' \
            eval "$(sed -n "/^detect_platform/,/^}/p" /tmp/install.sh)" && \
            detect_platform && \
            echo "$PLATFORM" | grep -Eq "linux-(musl|gnu)" && \
            echo "PASS: target is linux-musl or linux-gnu" \
        '; \
    fi

# If full install succeeded, verify the binary works
RUN if [ "$LIBREFANG_SMOKE_FULL" = "1" ] && [ -f "$HOME/.librefang/bin/librefang" ]; then \
        $HOME/.librefang/bin/librefang --version && \
        echo "PASS: librefang binary works"; \
    else \
        echo "SKIP: binary verification (no full install)"; \
    fi
