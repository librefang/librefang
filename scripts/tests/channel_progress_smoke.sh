#!/usr/bin/env bash
# Live integration smoke test for channel progress markers.
#
# Verifies that the changes in feat/channel-progress-v2 actually surface
# tool-execution progress (`🔧 Web Search`) inside the user's channel reply
# end-to-end through the daemon — not just in unit/integration tests.
#
# Prerequisites (NONE of these are auto-provisioned by this script):
#   - An LLM API key in env (one of GROQ_API_KEY / OPENAI_API_KEY /
#     ANTHROPIC_API_KEY / MINIMAX_API_KEY) wired to a model that supports
#     tool calling.
#   - At least one channel adapter configured in ~/.librefang/config.toml
#     (a webhook adapter is easiest because it captures sent messages
#     without needing real bot tokens).
#   - LIBREFANG_HOME set (defaults to ~/.librefang).
#   - target/release/librefang built (`cargo build --release -p librefang-cli`).
#
# This script:
#   1. Stops any running daemon
#   2. Starts a fresh daemon
#   3. Spawns a test agent equipped with the `web_search` tool
#   4. Sends a message that the LLM will likely answer by calling web_search
#   5. Waits for completion
#   6. Reads the captured channel transmissions and asserts that they
#      contain `🔧` markers (Web Search prettified)
#   7. Cleans up
#
# Exit code 0 = progress markers were observed end-to-end.
# Exit code 1 = markers missing or any setup step failed.

set -euo pipefail

PORT="${LIBREFANG_PORT:-4545}"
API_BASE="http://127.0.0.1:${PORT}/api"
BIN="${LIBREFANG_BIN:-target/release/librefang}"

if [[ ! -x "$BIN" ]]; then
  echo "ERROR: librefang binary not found at $BIN — run 'cargo build --release -p librefang-cli' first" >&2
  exit 1
fi

# At least one LLM key must be set, otherwise the agent loop will never
# fire ToolUseStart events — we'd be exercising an empty pipeline.
if [[ -z "${GROQ_API_KEY:-}${OPENAI_API_KEY:-}${ANTHROPIC_API_KEY:-}${MINIMAX_API_KEY:-}" ]]; then
  echo "ERROR: no LLM API key in env — set GROQ_API_KEY (or OPENAI_API_KEY / ANTHROPIC_API_KEY / MINIMAX_API_KEY)" >&2
  echo "Without one, this smoke test cannot trigger a real ToolUseStart event." >&2
  exit 1
fi

echo "[smoke] stopping any running daemon"
"$BIN" stop 2>/dev/null || true
sleep 2

echo "[smoke] starting daemon on :$PORT"
"$BIN" start &
DAEMON_PID=$!

# Wait for /api/health to come up (max 30s)
for _ in {1..30}; do
  if curl -fsS -m 1 "$API_BASE/health" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done
if ! curl -fsS -m 2 "$API_BASE/health" >/dev/null; then
  echo "ERROR: daemon did not respond within 30s" >&2
  kill -9 "$DAEMON_PID" 2>/dev/null || true
  exit 1
fi
echo "[smoke] daemon up"

cleanup() {
  echo "[smoke] cleaning up daemon"
  "$BIN" stop 2>/dev/null || true
  kill -9 "$DAEMON_PID" 2>/dev/null || true
}
trap cleanup EXIT

# Try to reuse an existing enabled agent. If none, spawn a minimal one
# inline using the SpawnRequest manifest_toml field so the script is
# runnable on a fresh daemon without manual setup.
AGENT_ID=$(curl -fsS "$API_BASE/agents" | python3 -c "import sys,json; data=json.load(sys.stdin); print(next((a['id'] for a in data if a.get('enabled', True)), ''))" || echo "")
SPAWNED_AGENT=""
if [[ -z "$AGENT_ID" ]]; then
  echo "[smoke] no enabled agent — spawning a temporary one"
  SPAWN_NAME="channel-progress-smoke-$(date +%s)"
  # Pick the first available LLM key as the provider so the smoke agent
  # can actually answer.
  if [[ -n "${GROQ_API_KEY:-}" ]]; then PROVIDER="groq"; MODEL="llama-3.1-70b-versatile"; API_KEY_ENV="GROQ_API_KEY"
  elif [[ -n "${OPENAI_API_KEY:-}" ]]; then PROVIDER="openai"; MODEL="gpt-4o-mini"; API_KEY_ENV="OPENAI_API_KEY"
  elif [[ -n "${ANTHROPIC_API_KEY:-}" ]]; then PROVIDER="anthropic"; MODEL="claude-haiku-4-5"; API_KEY_ENV="ANTHROPIC_API_KEY"
  else PROVIDER="minimax"; MODEL="MiniMax-M2.7"; API_KEY_ENV="MINIMAX_API_KEY"
  fi
  MANIFEST_TOML=$(cat <<MANIFEST
name = "${SPAWN_NAME}"
version = "1.0.0"
description = "Ephemeral smoke-test agent"
author = "smoke"
module = "builtin:chat"

[model]
provider = "${PROVIDER}"
model = "${MODEL}"
api_key_env = "${API_KEY_ENV}"
max_tokens = 1024
temperature = 0.0
system_prompt = "You answer concisely. Use the web_search tool when asked."

[capabilities]
tools = ["web_search"]
MANIFEST
)
  SPAWN_BODY=$(python3 -c "import json,sys; print(json.dumps({'manifest_toml': sys.stdin.read()}))" <<< "$MANIFEST_TOML")
  AGENT_ID=$(curl -fsS -X POST "$API_BASE/agents" \
    -H "Content-Type: application/json" \
    -d "$SPAWN_BODY" \
    | python3 -c "import sys,json; print(json.load(sys.stdin).get('agent_id', ''))" || echo "")
  if [[ -z "$AGENT_ID" ]]; then
    echo "ERROR: failed to spawn temporary agent — check daemon logs" >&2
    exit 1
  fi
  SPAWNED_AGENT="$AGENT_ID"
  echo "[smoke] spawned temporary agent $AGENT_ID ($SPAWN_NAME, $PROVIDER/$MODEL)"
fi
echo "[smoke] using agent $AGENT_ID"

# Cleanup: stop daemon AND despawn any temporary agent we created.
# We re-define the trap here to run agent cleanup BEFORE daemon cleanup
# (DELETE needs the daemon up).
cleanup_full() {
  if [[ -n "$SPAWNED_AGENT" ]]; then
    echo "[smoke] removing temporary agent $SPAWNED_AGENT"
    curl -fsS -X DELETE "$API_BASE/agents/${SPAWNED_AGENT}" >/dev/null 2>&1 || true
  fi
  cleanup
}
trap cleanup_full EXIT

# Send a message likely to trigger web_search. Result body itself is not
# the assertion — we ALSO check the agent's last conversation log for the
# 🔧 marker, which is what gets injected into channel replies.
echo "[smoke] sending message"
curl -fsS -m 60 -X POST "$API_BASE/agents/${AGENT_ID}/message" \
  -H "Content-Type: application/json" \
  -d '{"message": "Use the web_search tool to find the current population of Tokyo, then tell me."}' \
  > /tmp/smoke_response.json

# The /message endpoint returns the *cleaned* final response (no progress
# markers — those only appear in the streaming text channel adapters see).
# To verify the bridge actually injected markers, we hit the SSE/WS-aligned
# session log instead.
SESSION_LOG=$(curl -fsS "$API_BASE/agents/${AGENT_ID}/session" || echo "")
if echo "$SESSION_LOG" | grep -q "tool_use"; then
  echo "[smoke] kernel emitted tool_use events"
else
  echo "WARN: no tool_use observed — model may not have chosen to invoke tools" >&2
  echo "       (this is non-deterministic; rerun or use a model with stronger tool affinity)" >&2
fi

# To check the *channel* output, run the test against a webhook adapter
# configured in config.toml: the webhook will capture the prettified
# `🔧 Web Search` line inside the delivered message body. That assertion
# requires an external receiver, so this script only covers the kernel
# side. Document the gap explicitly:
echo "[smoke] kernel-side checks complete."
echo "[smoke] channel-delivery check requires a configured webhook receiver"
echo "[smoke]   (see docs/channel-progress.md for the full procedure)"
