#!/usr/bin/env bash
set -euo pipefail

# LibreFang — One-command deploy to Fly.io
# Usage: curl -sL https://raw.githubusercontent.com/librefang/librefang/main/deploy/fly/deploy.sh | bash

REPO="https://github.com/librefang/librefang.git"
REGION="nrt"

info()  { printf "\033[1;34m→\033[0m %s\n" "$1"; }
ok()    { printf "\033[1;32m✓\033[0m %s\n" "$1"; }
warn()  { printf "\033[1;33m⚠\033[0m %s\n" "$1"; }
err()   { printf "\033[1;31m✗\033[0m %s\n" "$1" >&2; exit 1; }

WORK_DIR=""
CURSOR_HIDDEN=0
cleanup() {
  if [ "$CURSOR_HIDDEN" -eq 1 ]; then
    printf "\033[?25h" > /dev/tty 2>/dev/null || true
  fi
  if [ -n "$WORK_DIR" ] && [ -d "$WORK_DIR" ]; then
    rm -rf -- "$WORK_DIR"
  fi
}
trap cleanup EXIT

# --- 1. Check / Install flyctl ---
if ! command -v flyctl &>/dev/null; then
  err "flyctl not found. Install it from https://fly.io/docs/flyctl/install/ and rerun this script."
fi
ok "flyctl $(flyctl version 2>&1 | head -1 || echo 'installed')"

# --- 2. Auth (opens browser) ---
if ! flyctl auth whoami &>/dev/null; then
  info "Opening browser for Fly.io login..."
  flyctl auth login
fi
ok "Logged in as $(flyctl auth whoami)"

# --- 3. Clone repo ---
WORK_DIR=$(mktemp -d)
info "Cloning LibreFang..."
git clone --depth 1 "$REPO" "$WORK_DIR/librefang"
cd "$WORK_DIR/librefang"

# --- 4. Name & create app ---
while true; do
  echo ""
  read -rp "$(printf '\033[1;34m→\033[0m') App name (leave empty for auto-generated): " CUSTOM_NAME < /dev/tty
  if [ -n "$CUSTOM_NAME" ]; then
    APP_NAME=$(echo "$CUSTOM_NAME" | tr '[:upper:]' '[:lower:]' | sed 's/[^a-z0-9-]/-/g; s/--*/-/g; s/^-//; s/-$//')
    if [ -z "$APP_NAME" ]; then
      warn "Invalid name. Please try again."
      continue
    fi
  else
    APP_NAME="librefang-$(openssl rand -hex 4)"
  fi

  info "Creating Fly app: $APP_NAME (region: $REGION)..."
  if create_output=$(flyctl apps create "$APP_NAME" --machines 2>&1); then
    ok "App created: $APP_NAME"
    break
  else
    warn "Could not create app '$APP_NAME':"
    printf '%s\n' "$create_output" >&2
    warn "Choose another name or fix the Fly error and retry."
  fi
done

# Update fly.toml with generated app name
sed -i.bak "s/^app = .*/app = \"$APP_NAME\"/" deploy/fly/fly.toml && rm -f deploy/fly/fly.toml.bak

# --- 5. Create persistent volume ---
info "Creating 1GB persistent volume..."
flyctl volumes create librefang_data \
  --app "$APP_NAME" \
  --region "$REGION" \
  --size 1 \
  --yes

# --- 6. Set authentication and provider secrets ---
LIBREFANG_ACCESS_KEY=$(openssl rand -hex 32)
printf 'LIBREFANG_API_KEY=%s\n' "$LIBREFANG_ACCESS_KEY" \
  | flyctl secrets import --app "$APP_NAME"

PROVIDER_NAMES=(
  "OpenAI"
  "Anthropic"
  "Google Gemini"
  "Groq"
  "DeepSeek"
  "OpenRouter"
  "Mistral"
  "xAI / Grok"
)
PROVIDER_KEYS=(
  "OPENAI_API_KEY"
  "ANTHROPIC_API_KEY"
  "GEMINI_API_KEY"
  "GROQ_API_KEY"
  "DEEPSEEK_API_KEY"
  "OPENROUTER_API_KEY"
  "MISTRAL_API_KEY"
  "XAI_API_KEY"
)

# TUI multi-select: arrow keys to move, space to toggle, enter to confirm
tui_multiselect() {
  local count=${#PROVIDER_NAMES[@]}
  local cursor=0
  local selected=()
  for ((i = 0; i < count; i++)); do selected+=(0); done

  # Hide cursor
  printf "\033[?25l" > /dev/tty
  CURSOR_HIDDEN=1

  draw_menu() {
    for ((i = 0; i < count; i++)); do
      local checkbox="[ ]"
      if [ "${selected[$i]}" -eq 1 ]; then checkbox="[\033[1;32m✓\033[0m]"; fi

      local pointer="  "
      if [ "$i" -eq "$cursor" ]; then pointer="\033[1;36m❯\033[0m "; fi

      if [ "$i" -eq "$cursor" ]; then
        printf "\033[K  ${pointer}${checkbox} \033[1m%-16s\033[0m  \033[2m%s\033[0m\n" "${PROVIDER_NAMES[$i]}" "${PROVIDER_KEYS[$i]}" > /dev/tty
      else
        printf "\033[K  ${pointer}${checkbox} %-16s  \033[2m%s\033[0m\n" "${PROVIDER_NAMES[$i]}" "${PROVIDER_KEYS[$i]}" > /dev/tty
      fi
    done
  }

  echo "" > /dev/tty
  info "Select LLM providers to configure:" > /dev/tty
  echo "" > /dev/tty
  printf "  \033[1;33m↑/↓\033[0m navigate  \033[1;33mspace\033[0m toggle  \033[1;33menter\033[0m confirm  \033[1;33mesc\033[0m skip\n" > /dev/tty
  echo "" > /dev/tty
  draw_menu

  while true; do
    IFS= read -rsn1 key < /dev/tty

    if [[ "$key" == $'\x1b' ]]; then
      read -rsn1 -t 0.1 k2 < /dev/tty || true
      read -rsn1 -t 0.1 k3 < /dev/tty || true
      key="${key}${k2}${k3}"
    fi

    case "$key" in
      $'\x1b[A' | k)  [[ $cursor -gt 0 ]] && ((cursor--)) || true ;;
      $'\x1b[B' | j)  [[ $cursor -lt $((count - 1)) ]] && ((cursor++)) || true ;;
      " ")  # Space — toggle current item (for multi-select)
        if [ "${selected[$cursor]}" -eq 0 ]; then
          selected[$cursor]=1
        else
          selected[$cursor]=0
        fi
        ;;
      "")  # Enter — select current item (if nothing toggled) & confirm
        local has_selection=0
        for ((i = 0; i < count; i++)); do
          if [ "${selected[$i]}" -eq 1 ]; then has_selection=1; break; fi
        done
        if [ "$has_selection" -eq 0 ]; then
          selected[$cursor]=1
        fi
        break
        ;;
      q | $'\x1b')  # q or Esc — skip without selecting anything
        for ((i = 0; i < count; i++)); do selected[$i]=0; done
        break
        ;;
    esac

    printf "\033[%dA" "$count" > /dev/tty
    draw_menu
  done
  echo "" > /dev/tty
  printf "\033[?25h" > /dev/tty
  CURSOR_HIDDEN=0

  SELECTED_INDICES=()
  for ((i = 0; i < count; i++)); do
    if [ "${selected[$i]}" -eq 1 ]; then
      SELECTED_INDICES+=("$i")
    fi
  done
}

tui_multiselect

for idx in "${SELECTED_INDICES[@]}"; do
  name="${PROVIDER_NAMES[$idx]}"
  env_var="${PROVIDER_KEYS[$idx]}"
  read -rsp "  $name ($env_var): " KEY_VAL < /dev/tty
  echo "" > /dev/tty
  if [ -n "$KEY_VAL" ]; then
    printf '%s=%s\n' "$env_var" "$KEY_VAL" | flyctl secrets import --app "$APP_NAME"
    KEY_VAL=""
  fi
done

# --- 7. Deploy ---
echo ""
info "Deploying LibreFang (this may take a few minutes on first build)..."
flyctl deploy --app "$APP_NAME" --config deploy/fly/fly.toml --remote-only

# --- 8. Done ---
echo ""
APP_URL="https://$APP_NAME.fly.dev"
ok "LibreFang is live!"
echo ""
echo "  Dashboard:  $APP_URL"
echo "  API:        $APP_URL/api/health"
echo "  API key:    $LIBREFANG_ACCESS_KEY"
echo "  Manage:     flyctl dashboard --app $APP_NAME"
echo ""
warn "Save the API key now; it is stored as a Fly secret and cannot be read back."
echo ""
echo "  To add or change API keys later:"
echo "    flyctl secrets set <PROVIDER>_API_KEY=your-key --app $APP_NAME"
echo ""
