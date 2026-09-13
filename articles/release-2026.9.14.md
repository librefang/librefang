---
title: "LibreFang 2026.9.14 Released"
published: true
description: "LibreFang v2026.9.14 release notes — open-source Agent OS built in Rust"
tags: rust, ai, opensource, release
canonical_url: https://github.com/librefang/librefang/releases/tag/v2026.9.14
cover_image: https://raw.githubusercontent.com/librefang/librefang/main/public/assets/logo.png
---

# LibreFang 2026.9.14 Released

**v2026.9.14 is a milestone release** bringing autonomous goal execution, multi-instance channels, per-agent reasoning modes, and comprehensive configuration tooling to the dashboard and CLI. This release stabilizes the agent-authoring experience while closing several security boundaries and fixing 50+ bugs.

_203 PRs from 5 contributors since v2026.8.30._

## Autonomous Goal Execution & Goals System

Agents can now drive themselves through iterative tasks without human invocation. A goal defines what success looks like; LibreFang auto-spawns a disposable worker agent that runs the goal, optionally spawns a verifier that judges each iteration, and learns from tagged markers (`GOAL_LEARNED: <text>`) it encounters along the way.

**New in this release:**
- `librefang goal "<description>"` CLI command with `--loop-engineering` mode for automated verification
- Goals screen in the TUI with wizard-driven creation and live phase tracking
- `/goal` slash command in Telegram and Slack
- Dashboard Goals page with phase badges and outcome visibility
- Per-goal `evaluator_model` to judge completion, plus auto-generated skills from captured learnings

## Agent Types: Manifests as First-Class Citizens

Agent types (reusable manifests) moved from `templates/` to the canonical `agent-types/` directory and gained full CRUD support. Create, edit, and delete types from the dashboard Agent Types page instead of hand-editing TOML. Types can now be promoted directly to the public registry as a GitHub PR.

**Related improvements:**
- Dashboard Agent Types page with card grid, Quick Run for ephemeral spawns
- Shared Folders editor on agent type and instance details
- `librefang agent new` discovers types in the flat canonical store
- Registry sync resolves both legacy `agents/` and new `agent-types/` directories
- Source template origin tracked on every agent spawned from a type

## Channel Multi-Instance Support

A single channel adapter (Telegram, Slack, etc.) can now run as multiple named instances, each with its own configuration, secrets namespace, and assigned agent. Manage instances from the dashboard Channels screen or the TUI.

**Key features:**
- Per-instance secret namespacing prevents credential sharing between bots
- Collision detection rejects names that would map to the same namespace
- Add/configure/delete UI on both dashboard and TUI
- Instance name and per-instance agent binding in config forms

## Per-Agent Reasoning Mode & Extended Thinking

DeepSeek V4 and OpenRouter-routed models now respect per-agent reasoning-mode settings. Agents can be configured to run with `none`, `low`, `high`, or `max` reasoning effort. The Anthropic driver now handles current Claude models (5, Sonnet 5, Opus 4.8+) which removed sampling parameters and moved extended thinking to a new schema.

**What changed:**
- `reasoning_mode` settable per agent, globally, or per task
- Provider-aware translation: DeepSeek gets `thinking` fields, OpenRouter gets nested `reasoning`, others get top-level `reasoning_effort`
- Adaptive thinking on Claude (effort comes from `reasoning_mode` alone, no separate budget)
- Fallback chain `model = "default"` now resolves correctly

## Configuration, Credentials & Vaults

The operator experience for configuration just got significantly better. The dashboard gains a schema-driven config editor; the vault write API lets surfaces store credentials securely; and editing flows now validate and report failures instead of silently accepting bad input.

**What's new:**
- Dashboard Config page with per-field editable rows derived from `GET /api/config/schema`
- TUI Settings screen gains Configuration tab with real-time validation
- Vault write API: `GET /api/vault/keys`, `PUT /api/vault/keys/{key}`, `DELETE /api/vault/keys/{key}`
- Vault reads show effective source: `environment`, `vault`, or `unset`
- Dashboard Channels settings are writable again

## Model Management & Parameters

Every model parameter now uses the same control everywhere it appears. Context window, max output tokens, temperature, top-p, and the two penalties are now consistently editable with shared presets and bounds checking.

**Improvements:**
- `context_window` and `max_output_tokens` are editable per agent from the API, CLI, TUI, and dashboard
- Model parameter overrides survive registry syncs (schema v56)
- Per-model limits show effective value vs. catalog figure
- Unsupported parameters (e.g., `reasoning_effort` on a gateway that doesn't support it) are stripped and retried transparently

## Memory System Hardening

Automatic memory scoping now closes three holes that let memories leak across boundaries:

- `capabilities.memory_read = []` now blocks substrate recall and context-engine injection (not just auto-retrieve)
- `capabilities.memory_write = []` blocks the per-turn writer, not just extraction
- Per-turn writer stamps both `chat_scope` and `session_scope` so raw dialogue respects agent isolation
- Full-text-index rebuilds on UPDATEs that cannot change indexed content (access-count bumps, decay sweeps)
- Session and cron compaction run on the agent's own model chain, not the kernel default

## Dashboard & TUI Enhancements

The UX got smoother everywhere. Failed operations now report what the daemon said instead of generic lines. The dashboard shows real values for configuration settings instead of cached placeholders. The TUI's goal detail panel, memory config editor, and every error message is now precise and actionable.

**Highlights:**
- Dashboard login survives daemon restart (session tokens now restored correctly)
- Agent detail drawer shows injected token footprint and last five LLM calls
- Memory tab shows resolved extraction model and provenance
- Workflow run detail shows step progress (`current_step_index`, `total_steps`, per-step variables)
- Goals page shows autonomous run phase badges (finished, rate-limited, stopped, etc.)
- Slider tick legends now position labels at their true values, not evenly spaced
- Provider page shows real availability status, not a stale cached state
- Add button re-enabled on Channels page after first instance is configured

## Bug Fixes & Stability

**50+ bugs fixed**, including critical issues that affected real deployments:

- **Memory consolidation** no longer merges memories across different peers, chats, or sessions
- **Tool loop guard** now detects repeated identical results and refuses the call after three runs
- **Streaming replies** with malformed tool-call markup are replaced with an honest sentence instead of sent raw to channels
- **Network byte quota** now actually enforces `[resources] max_network_bytes_per_hour` (was accepted but ignored)
- **Docker sandbox** container pooling now works (was created and destroyed per call)
- **Workflow step prompts** expanded in single pass (order-independent, no iteration on substituted values)
- **Cron schedule editing** no longer reverts to defaults when no changes made
- **Sidecar channel delete** now removes every adapter instance, not guessed ones
- **Config reload** correctly reports what actually took effect (not applied work it withheld)
- **Session streaming** no longer wedges the daemon on message-history compaction
- **Approval routing** now properly scoped to requesting agent's configured channels
- **Windows launcher** (librefang.exe) no longer stack-overflows on startup

## Security Fixes

- Approval field payload injection blocked: config writes can no longer smuggle credential fields into tables
- MCP server listing now requires Admin role (was exposing tokens in SSE/HTTP URLs)
- API key revocation on `POST /api/config/reload` now applies to REST surface (was only effective on WebSocket)
- Passkey login no longer accepts credentials filed under the wrong principal
- Dashboard root path protected by auth (was unprotected even with password configured)
- Dangerous-command gate no longer bypassed by quoted program names (`"rm" -rf /` now blocked)
- Sidecar secrets no longer shared between sibling instances
- Plugin `[env]` expansion can no longer reference reserved secrets
- Chat WebSocket now verifies caller owns the agent (not just exists)
- Rich HTML sanitization moved to structured parsing (no longer regex-based escaping that can be evaded)

## Performance

- Sidecar schema probes run concurrently (cold-start boot time reduced 10–30x on deployments with many adapters)
- Per-adapter send backpressure prevents queuing under burst load
- Memory index rebuilt only on content changes, not on every access-count bump or decay sweep
- Reduced lock contention in concurrent retry scenarios

## Migration Notes

- **Operator-visible behavior change**: `max_network_bytes_per_hour` now enforces the 100 MB default if unset. Agents moving more traffic need an explicit larger value or `0` for unlimited.
- **Workflow approval gates** now actually pause runs and wait for humans (previously ran unattended). Anything behind an approval gate will now require explicit approval to proceed.
- **Agent session isolation**: agents running `session_mode = "new"` no longer recall raw dialogue from earlier invocations; set `session_scoped_recall = false` in `agent.toml` to restore prior behavior.
- **Configuration includes** with unknown fields are now rejected when `strict_config = true` (was silently ignoring them).

## Install / Upgrade

```bash
# Binary
curl -fsSL https://get.librefang.ai | sh

# Rust SDK
cargo add librefang

# JavaScript SDK
npm install @librefang/sdk

# Python SDK
pip install librefang-sdk
```

## Links

- [Full Changelog](https://github.com/librefang/librefang/blob/main/CHANGELOG.md)
- [GitHub Release](https://github.com/librefang/librefang/releases/tag/v2026.9.14)
- [GitHub](https://github.com/librefang/librefang)
- [Discord](https://discord.gg/DzTYqAZZmc)
- [Contributing Guide](https://github.com/librefang/librefang/blob/main/docs/CONTRIBUTING.md)
