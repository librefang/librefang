#!/usr/bin/env bash
set -euo pipefail

# LibreFang — Uninstall from Fly.io
# Usage: curl -sL https://raw.githubusercontent.com/librefang/librefang/main/deploy/fly/uninstall.sh | bash

info()  { printf "\033[1;34m→\033[0m %s\n" "$1"; }
ok()    { printf "\033[1;32m✓\033[0m %s\n" "$1"; }
warn()  { printf "\033[1;33m⚠\033[0m %s\n" "$1"; }
err()   { printf "\033[1;31m✗\033[0m %s\n" "$1" >&2; exit 1; }

# --- 1. Check flyctl ---
command -v flyctl &>/dev/null || err "flyctl not found. Install it first: curl -sL https://fly.io/install.sh | sh"

# --- 2. Auth ---
if ! flyctl auth whoami &>/dev/null; then
  info "Opening browser for Fly.io login..."
  flyctl auth login
fi
ok "Logged in as $(flyctl auth whoami)"

# --- 3. Fetch LibreFang apps ---
info "Fetching your LibreFang apps..."
APPS=()
while IFS= read -r line; do
  [[ -n "$line" ]] && APPS+=("$line")
done < <(flyctl apps list --json 2>/dev/null | python3 -c "
import sys, json
apps = json.load(sys.stdin)
for a in apps:
    name = a.get('Name', a.get('name', ''))
    if name.startswith('librefang'):
        print(name)
" 2>/dev/null || true)

if [ ${#APPS[@]} -eq 0 ]; then
  ok "No LibreFang apps found on your account."
  exit 0
fi

echo ""
ok "Found ${#APPS[@]} LibreFang app(s)"

# --- 4. TUI multi-select ---
TUI_ITEMS=("${APPS[@]}")

tui_multiselect() {
  local count=${#TUI_ITEMS[@]}
  local cursor=0
  local selected=()
  for ((i = 0; i < count; i++)); do selected+=(0); done

  printf "\033[?25l" > /dev/tty
  trap 'printf "\033[?25h" > /dev/tty' RETURN

  draw_menu() {
    for ((i = 0; i < count; i++)); do
      local checkbox="[ ]"
      if [ "${selected[$i]}" -eq 1 ]; then checkbox="[\033[1;32m✓\033[0m]"; fi

      local pointer="  "
      if [ "$i" -eq "$cursor" ]; then pointer="\033[1;36m❯\033[0m "; fi

      if [ "$i" -eq "$cursor" ]; then
        printf "\033[K  ${pointer}${checkbox} \033[1m%s\033[0m\n" "${TUI_ITEMS[$i]}" > /dev/tty
      else
        printf "\033[K  ${pointer}${checkbox} %s\n" "${TUI_ITEMS[$i]}" > /dev/tty
      fi
    done
  }

  echo "" > /dev/tty
  info "Select apps to destroy:" > /dev/tty
  echo "" > /dev/tty
  printf "  \033[1;33m↑/↓\033[0m navigate  \033[1;33mspace\033[0m toggle  \033[1;33menter\033[0m confirm  \033[1;33mesc\033[0m cancel\n" > /dev/tty
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
      " ")
        if [ "${selected[$cursor]}" -eq 0 ]; then
          selected[$cursor]=1
        else
          selected[$cursor]=0
        fi
        ;;
      "")  # Enter — select current if nothing toggled, then confirm
        local has_selection=0
        for ((i = 0; i < count; i++)); do
          if [ "${selected[$i]}" -eq 1 ]; then has_selection=1; break; fi
        done
        if [ "$has_selection" -eq 0 ]; then
          selected[$cursor]=1
        fi
        break
        ;;
      q | $'\x1b')  # Esc or q — cancel
        for ((i = 0; i < count; i++)); do selected[$i]=0; done
        break
        ;;
    esac

    printf "\033[%dA" "$count" > /dev/tty
    draw_menu
  done
  echo "" > /dev/tty

  SELECTED_INDICES=()
  for ((i = 0; i < count; i++)); do
    if [ "${selected[$i]}" -eq 1 ]; then
      SELECTED_INDICES+=("$i")
    fi
  done
}

tui_multiselect

if [ ${#SELECTED_INDICES[@]} -eq 0 ]; then
  ok "Cancelled. No apps were destroyed."
  exit 0
fi

# --- 5. Confirm destruction ---
echo ""
warn "The following apps will be permanently destroyed:"
for idx in "${SELECTED_INDICES[@]}"; do
  printf "    \033[1;31m%s\033[0m\n" "${TUI_ITEMS[$idx]}"
done
echo ""
printf "  \033[1;31mThis will delete all data, volumes, and secrets. This cannot be undone.\033[0m\n"
echo ""
read -rp "  Type 'yes' to confirm: " CONFIRM < /dev/tty

if [ "$CONFIRM" != "yes" ]; then
  ok "Cancelled. No apps were destroyed."
  exit 0
fi

# --- 6. Destroy selected apps ---
echo ""
for idx in "${SELECTED_INDICES[@]}"; do
  app="${TUI_ITEMS[$idx]}"
  info "Destroying $app..."
  if flyctl apps destroy "$app" --yes 2>/dev/null; then
    ok "Destroyed $app"
  else
    warn "Failed to destroy $app (may already be deleted)"
  fi
done

echo ""
ok "Done! All selected apps have been destroyed."
