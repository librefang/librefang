#cloud-config

users:
  - name: librefang
    shell: /bin/bash
    sudo: ALL=(ALL) NOPASSWD:ALL
    groups: [sudo]

package_update: true
packages:
  - curl
  - jq
  - htop
  - fail2ban

write_files:
  - path: /etc/systemd/system/librefang.service
    content: |
      [Unit]
      Description=LibreFang Agent OS
      After=network-online.target
      Wants=network-online.target

      [Service]
      Type=simple
      User=librefang
      Environment=LIBREFANG_HOME=/data
      Environment=LIBREFANG_BIND=0.0.0.0:4545
      Environment=GROQ_API_KEY=${groq_api_key}
      Environment=OPENAI_API_KEY=${openai_api_key}
      Environment=ANTHROPIC_API_KEY=${anthropic_api_key}
      ExecStart=/usr/local/bin/librefang start
      Restart=on-failure
      RestartSec=5

      # Hardening
      ProtectSystem=strict
      ReadWritePaths=/data
      PrivateTmp=true
      NoNewPrivileges=true

      [Install]
      WantedBy=multi-user.target

runcmd:
  # Create data directory
  - mkdir -p /data
  - chown librefang:librefang /data

  # Download LibreFang binary
  - |
    VERSION="${librefang_version}"
    ARCH=$(uname -m)
    case "$ARCH" in
      x86_64)  TARGET="x86_64-unknown-linux-gnu" ;;
      aarch64) TARGET="aarch64-unknown-linux-gnu" ;;
      *)       echo "Unsupported arch: $ARCH"; exit 1 ;;
    esac

    if [ "$VERSION" = "latest" ]; then
      DOWNLOAD_URL=$(curl -fsSL https://api.github.com/repos/librefang/librefang/releases/latest \
        | jq -r ".assets[] | select(.name | contains(\"$TARGET\")) | select(.name | endswith(\".tar.gz\")) | .browser_download_url")
    else
      DOWNLOAD_URL="https://github.com/librefang/librefang/releases/download/$VERSION/librefang-$TARGET.tar.gz"
    fi

    curl -fsSL "$DOWNLOAD_URL" -o /tmp/librefang.tar.gz
    tar xzf /tmp/librefang.tar.gz -C /usr/local/bin/
    chmod +x /usr/local/bin/librefang
    rm -f /tmp/librefang.tar.gz

  # Start service
  - systemctl daemon-reload
  - systemctl enable librefang
  - systemctl start librefang
