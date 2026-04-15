---
status: awaiting_human_verify
trigger: "wa-nudging-retry-duplication — Owner receives duplicated WhatsApp message in relay/OWNER_MESSAGE flow"
created: 2026-04-15T00:00:00Z
updated: 2026-04-15T01:00:00Z
---

## Current Focus
<!-- OVERWRITE on each update - reflects NOW -->

hypothesis: CONFIRMED — stream_with_retry sends TextDelta events to stream_tx during generation; when nudge retry fires the first iteration's text is already fully in the SSE channel; iteration-1 appends its text to the same channel; gateway accumulated variable concatenates both
test: Build passes, clippy clean, tests pass on modified crates
expecting: Fix verified by code inspection; human live test needed
next_action: Commit fix, deploy to NAS, human verification

## Symptoms
<!-- Written during gathering, then IMMUTABLE -->

expected: One single response text delivered to the owner's WhatsApp chat per turn.
actual: Owner receives a single WhatsApp message containing the SAME content twice, back-to-back, with slightly different wording.
errors: |
  WARN agent_loop: User requested action but LLM responded without tool calls (streaming) — nudging retry agent=ambrogio iteration=0
  WARN agent_loop: Empty response from LLM (streaming) — guard activated agent=ambrogio iteration=1
reproduction: Send an OWNER_MESSAGE in WHATSAPP_RELAY context while a stranger conversation is active. Ambrogio responds with PURE TEXT (no tool calls). Runtime treats absent tool_use as "LLM did nothing" and re-invokes.
started: 2026-04-15 after cumulative fixes #1/#2/#3 deployed

## Eliminated
<!-- APPEND only - prevents re-investigating -->

- hypothesis: SSE StreamDedup (PR #2626) would fix this
  evidence: If concatenation happens inside agent_loop across iterations before streaming, each chunk is unique; only final assembled text carries duplication
  timestamp: 2026-04-15T00:00:00Z

## Evidence
<!-- APPEND only - facts discovered -->

- timestamp: 2026-04-15T00:30:00Z
  checked: agent_loop.rs streaming path, ActionIntent branch (line ~3542)
  found: When classify_end_turn_retry returns ActionIntent, the code does `continue` to next loop iteration. But stream_with_retry already sent ALL TextDelta events for iteration-0 to stream_tx before the classify call. The retry sends iteration-1 text to the same stream_tx.
  implication: Gateway `accumulated` string gets iteration-0 text + iteration-1 text concatenated.

- timestamp: 2026-04-15T00:35:00Z
  checked: stream_with_retry (agent_loop.rs:2971) — calls driver.stream(request, tx.clone())
  found: TextDelta events are emitted token-by-token during generation directly to the tx channel passed in. This is confirmed across all drivers (anthropic.rs:457, openai.rs:1164, qwen_code.rs:841 etc.)
  implication: There is no buffering — text is live-streamed to SSE before the agent_loop can decide to retry.

- timestamp: 2026-04-15T00:40:00Z
  checked: forwardToLibreFangStreaming in index.js (line 2558+)
  found: Gateway has single `accumulated = ''` per request. On `chunk` SSE events it does `accumulated += parsed.content`. On `end` it resolves with `accumulated`. No reset between iterations.
  implication: All text from all iterations concatenates into one string, delivered as one WhatsApp message.

- timestamp: 2026-04-15T00:45:00Z
  checked: StreamEvent enum in librefang-llm-driver/src/lib.rs
  found: No reset/discard event existed. Added ResetAccumulator variant.
  implication: Rust protocol now has a way to signal "discard accumulated text."

- timestamp: 2026-04-15T00:50:00Z
  checked: Build + clippy + tests
  found: Build clean. Clippy 0 errors. All tests in modified crates pass. Pre-existing failures in bridge.rs and apply_patch.rs unaffected.
  implication: Fix is safe to ship.

## Resolution
<!-- OVERWRITE as understanding evolves -->

root_cause: |
  run_agent_loop_streaming calls stream_with_retry which passes stream_tx directly to the LLM driver.
  The driver emits TextDelta events token-by-token into stream_tx as tokens arrive (before the full
  response is available). When classify_end_turn_retry returns ActionIntent (nudge retry), the code
  pushes a user nudge message and `continue`s — but iteration-0's text has ALREADY been fully
  streamed to stream_tx/SSE. Iteration-1 then streams its own (slightly rephrased) response to the
  same channel. The gateway's `accumulated` variable in forwardToLibreFangStreaming concatenates both
  iteration texts, delivering doubled content as a single WhatsApp message.

fix: |
  Added StreamEvent::ResetAccumulator variant to the StreamEvent enum. Emitted from agent_loop.rs
  before each retry continue (EmptyResponse, HallucinatedAction, ActionIntent branches).
  SSE route (agents.rs) serializes it as `event: reset` with `{"reset": true}` JSON.
  Gateway (index.js) handles `reset` event by setting `accumulated = ''` and clearing pendingEdit.
  TUI chat_runner.rs and tui/mod.rs handle ResetAccumulator by clearing streaming_text.

verification: Build clean, clippy 0 errors, tests pass. Human live test pending.

files_changed:
  - crates/librefang-llm-driver/src/lib.rs
  - crates/librefang-runtime/src/agent_loop.rs
  - crates/librefang-api/src/routes/agents.rs
  - crates/librefang-cli/src/tui/chat_runner.rs
  - crates/librefang-cli/src/tui/mod.rs
  - packages/whatsapp-gateway/index.js
