#!/usr/bin/env bash
# Live kernel-side smoke test for channel progress events.
#
# Verifies that a real daemon + provider turn emits a persisted `tool_use`
# event, the prerequisite consumed by channel progress renderers. Adapter-level
# formatting (`🔧 Web Search`) requires an external channel receiver and is not
# asserted by this kernel-side script.
#
# Prerequisites (NONE of these are auto-provisioned by this script):
#   - An LLM API key in env (one of GROQ_API_KEY / OPENAI_API_KEY /
#     ANTHROPIC_API_KEY / MINIMAX_API_KEY) wired to a model that supports
#     tool calling.
#   - LIBREFANG_HOME set (defaults to ~/.librefang).
#   - target/release/librefang built (`cargo build --release -p librefang-cli`).
#
# This script:
#   1. Stops any running daemon
#   2. Starts a fresh daemon
#   3. Spawns a temporary test agent equipped with the `web_search` tool
#   4. Sends a message that the LLM will likely answer by calling web_search
#   5. Waits for completion
#   6. Reads the resulting session and requires a `tool_use` event
#   7. Cleans up
#
# Exit code 0 = the kernel emitted the progress event channels depend on.
# Exit code 1 = the event was missing or any setup step failed.

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

DAEMON_PID=""
SPAWNED_AGENT=""
cleanup() {
  if [[ -n "$SPAWNED_AGENT" ]]; then
    echo "[smoke] removing temporary agent $SPAWNED_AGENT"
    curl -fsS -X DELETE "$API_BASE/agents/${SPAWNED_AGENT}" >/dev/null 2>&1 || true
  fi
  echo "[smoke] cleaning up daemon"
  "$BIN" stop 2>/dev/null || true
  if [[ -n "$DAEMON_PID" ]] && kill -0 "$DAEMON_PID" 2>/dev/null; then
    kill "$DAEMON_PID" 2>/dev/null || true
  fi
  if [[ -n "$DAEMON_PID" ]]; then
    wait "$DAEMON_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

echo "[smoke] starting daemon on :$PORT"
"$BIN" start --foreground &
DAEMON_PID=$!

# Wait for /api/health to come up (max 30s)
HEALTHY=0
for _ in {1..30}; do
  if curl -fsS -m 1 "$API_BASE/health" >/dev/null 2>&1; then
    HEALTHY=1
    break
  fi
  sleep 1
done
if [[ "$HEALTHY" -ne 1 ]]; then
  echo "ERROR: daemon did not respond within 30s" >&2
  exit 1
fi
echo "[smoke] daemon up"

# Always spawn a dedicated agent: an arbitrary existing agent may not expose
# `web_search`, which would make a strict progress assertion meaningless.
SPAWN_NAME="channel-progress-smoke-$(date +%s)-$$"
# Pick the first available LLM key as the provider so the smoke agent can
# actually answer.
if [[ -n "${GROQ_API_KEY:-}" ]]; then PROVIDER="groq"; MODEL="llama-3.3-70b-versatile"; API_KEY_ENV="GROQ_API_KEY"
elif [[ -n "${OPENAI_API_KEY:-}" ]]; then PROVIDER="openai"; MODEL="gpt-4o-mini"; API_KEY_ENV="OPENAI_API_KEY"
elif [[ -n "${ANTHROPIC_API_KEY:-}" ]]; then PROVIDER="anthropic"; MODEL="claude-haiku-4-5"; API_KEY_ENV="ANTHROPIC_API_KEY"
else PROVIDER="minimax"; MODEL="MiniMax-M2.7"; API_KEY_ENV="MINIMAX_API_KEY"
fi

# JSON string escaping is also valid for TOML basic strings. Construct every
# interpolated value through json.dumps, then encode the complete API body.
SPAWN_BODY=$(python3 - "$SPAWN_NAME" "$PROVIDER" "$MODEL" "$API_KEY_ENV" <<'PY'
import json
import sys

name, provider, model, api_key_env = sys.argv[1:]
quote = lambda value: json.dumps(value, ensure_ascii=False)
manifest = f"""name = {quote(name)}
version = "1.0.0"
description = "Ephemeral smoke-test agent"
author = "smoke"
module = "builtin:chat"

[model]
provider = {quote(provider)}
model = {quote(model)}
api_key_env = {quote(api_key_env)}
max_tokens = 1024
temperature = 0.0
system_prompt = "You answer concisely. You must use the web_search tool exactly once."

[capabilities]
tools = ["web_search"]
"""
print(json.dumps({"manifest_toml": manifest}))
PY
)
AGENT_ID=$(curl -fsS -X POST "$API_BASE/agents" \
  -H "Content-Type: application/json" \
  -d "$SPAWN_BODY" \
  | python3 -c "import sys,json; print(json.load(sys.stdin).get('agent_id', ''))")
if [[ -z "$AGENT_ID" ]]; then
  echo "ERROR: failed to spawn temporary agent — check daemon logs" >&2
  exit 1
fi
SPAWNED_AGENT="$AGENT_ID"
echo "[smoke] spawned temporary agent $AGENT_ID ($SPAWN_NAME, $PROVIDER/$MODEL)"

# Send a message likely to trigger web_search. Result body itself is not
# the assertion — we ALSO check the agent's last conversation log for the
# 🔧 marker, which is what gets injected into channel replies.
echo "[smoke] sending message"
curl -fsS -m 60 -X POST "$API_BASE/agents/${AGENT_ID}/message" \
  -H "Content-Type: application/json" \
  -d '{"message": "You must call web_search exactly once to find the current population of Tokyo, then answer."}' \
  >/dev/null

# The /message endpoint returns the cleaned final text, while the session keeps
# the structured tool event that channel progress renderers consume.
SESSION_LOG=$(curl -fsS "$API_BASE/agents/${AGENT_ID}/session")
if grep -q '"tool_use"' <<< "$SESSION_LOG"; then
  echo "[smoke] kernel emitted tool_use events"
else
  echo "ERROR: no tool_use observed — channel progress cannot be rendered" >&2
  echo "       rerun with a model that reliably follows tool-use instructions" >&2
  exit 1
fi

echo "[smoke] kernel-side channel-progress prerequisite verified"
