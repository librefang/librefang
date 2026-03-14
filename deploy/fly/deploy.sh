#!/usr/bin/env bash
set -euo pipefail

# LibreFang — One-command deploy to Fly.io
# Usage: curl -sL https://raw.githubusercontent.com/librefang/librefang/main/deploy/fly/deploy.sh | bash

REPO="https://github.com/librefang/librefang.git"
APP_NAME="librefang-$(openssl rand -hex 4)"
REGION="nrt"

info()  { printf "\033[1;34m→\033[0m %s\n" "$1"; }
ok()    { printf "\033[1;32m✓\033[0m %s\n" "$1"; }
err()   { printf "\033[1;31m✗\033[0m %s\n" "$1" >&2; exit 1; }

# --- 1. Check / Install flyctl ---
if ! command -v flyctl &>/dev/null; then
  info "Installing flyctl..."
  curl -sL https://fly.io/install.sh | sh
  export PATH="$HOME/.fly/bin:$PATH"
fi
ok "flyctl $(flyctl version --short 2>/dev/null || echo 'installed')"

# --- 2. Auth (opens browser) ---
if ! flyctl auth whoami &>/dev/null; then
  info "Opening browser for Fly.io login..."
  flyctl auth login
fi
ok "Logged in as $(flyctl auth whoami)"

# --- 3. Clone repo ---
TMPDIR=$(mktemp -d)
info "Cloning LibreFang..."
git clone --depth 1 "$REPO" "$TMPDIR/librefang"
cd "$TMPDIR/librefang"

# --- 4. Create app ---
info "Creating Fly app: $APP_NAME (region: $REGION)..."
flyctl apps create "$APP_NAME" --machines

# Update fly.toml with generated app name
sed -i.bak "s/^app = .*/app = \"$APP_NAME\"/" fly.toml && rm -f fly.toml.bak

# --- 5. Create persistent volume ---
info "Creating 1GB persistent volume..."
flyctl volumes create librefang_data \
  --app "$APP_NAME" \
  --region "$REGION" \
  --size 1 \
  --yes

# --- 6. Set secrets (optional) ---
echo ""
info "Optional: Set your LLM API key (press Enter to skip)"
read -rp "  GROQ_API_KEY: " GROQ_KEY
if [ -n "$GROQ_KEY" ]; then
  flyctl secrets set GROQ_API_KEY="$GROQ_KEY" --app "$APP_NAME"
fi
read -rp "  OPENAI_API_KEY: " OPENAI_KEY
if [ -n "$OPENAI_KEY" ]; then
  flyctl secrets set OPENAI_API_KEY="$OPENAI_KEY" --app "$APP_NAME"
fi
read -rp "  ANTHROPIC_API_KEY: " ANTHROPIC_KEY
if [ -n "$ANTHROPIC_KEY" ]; then
  flyctl secrets set ANTHROPIC_API_KEY="$ANTHROPIC_KEY" --app "$APP_NAME"
fi

# --- 7. Deploy ---
echo ""
info "Deploying LibreFang (this may take a few minutes on first build)..."
flyctl deploy --app "$APP_NAME" --remote-only

# --- 8. Done ---
echo ""
APP_URL="https://$APP_NAME.fly.dev"
ok "LibreFang is live!"
echo ""
echo "  Dashboard:  $APP_URL"
echo "  API:        $APP_URL/api/health"
echo "  Manage:     flyctl dashboard --app $APP_NAME"
echo ""
echo "  To add more API keys later:"
echo "    flyctl secrets set GROQ_API_KEY=your-key --app $APP_NAME"
echo ""

# Cleanup
rm -rf "$TMPDIR"
