```markdown
---
title: "LibreFang 2026.11.11 Released"
published: true
description: "LibreFang v2026.11.11 release notes — open-source Agent OS built in Rust"
tags: rust, ai, opensource, release
canonical_url: https://github.com/librefang/librefang/releases/tag/v2026.11.11
cover_image: https://raw.githubusercontent.com/librefang/librefang/main/public/assets/logo.png
---

# LibreFang 2026.11.11 Released

**LibreFang v2026.11.11 is out now** — a release focused on reasoning depth, memory reliability, and the kind of production hardening that keeps agents running smoothly at scale.

_87 PRs from 5 contributors since v2026.10.16._

## What's New

### Reasoning Depth Now Tunable Per Message

Set how deep your agent thinks on each turn. The chat UI, CLI, and agent editor all gain reasoning-effort controls that let you pick from the levels your model supports (`none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max`), with:

- **Per-message override**: Click a level to arm it for the next message only, then it resets.
- **Session pinning**: Lock a level for the whole conversation via a new `/think` command or dashboard toggle.
- **Agent defaults**: Set `[thinking] reasoning_effort` in your agent manifest for all automated turns.

The backend's `offered_efforts` are mirrored in the UI so you only see what's actually available — no dead choices. Works with streaming and HTTP fallback. (#8616, #8617, #8618)

### Memory That Sticks

Two key fixes restore context across compaction and iteration limits:

- **Trimmed messages now folded into summaries**: When a session compacts, messages older than the window are no longer dropped — they fold into that session's own summary, so later turns can still reconstruct what happened. Other sessions on the same agent no longer lose their history. (#8736, #8646, #8730)
- **Capped turns record themselves**: If your agent hits the iteration limit mid-turn, that attempt is now saved to memory as a failed exchange with a clear `[Unfinished — this turn reached the iteration limit]` marker, so the next turn knows it happened. (#8688)
- **Memory misses are now hints, not dead ends**: When `memory_recall` finds no match, it surfaces similar keys that do exist in your namespace, so agents can correct typos or try variations. (#8628)

### Session Stability and Correctness

Three subtle but high-impact fixes prevent silent data loss:

- **Compaction mirrors are per-session** (#8646): The old agent-level canonical summary is now also tracked per session, so multiple conversations on the same agent no longer clobber each other's compressed history.
- **History fold respects turn boundaries** (#8653, #8731): A single long agent turn no longer compacts its own tool results mid-loop. The fold now stops at the running turn's start, and the boundary is found by tool ID so it survives session repair and compression.
- **Injections stay out of long-term storage** (#8737): Ephemeral context (current time, recalled memory, search results) is no longer persisted when inline compression merges it into adjacent messages, so it doesn't replay forever.

Combined, these mean your agent's memory is accurate, and compacting sessions doesn't accidentally erase history across the workspace.

### DeepSeek V4 and MCP OAuth Work Reliably

Two integration suites are now solid:

- **DeepSeek reasoning echoes correctly** (#8639, #8652): The runtime now sends full reasoning content on every historical turn with a thinking block (as DeepSeek's API requires), and recognizes DeepSeek's native DSML markup as text so it doesn't leak into the transcript.
- **OAuth discovery follows RFC 9728** (#8670, #8728): MCP servers behind path prefixes, self-hosted gateways, and services with dynamic client registration all work now. Discovery probes authorization-server metadata and handles both header-based and headerless variants.

### API and Routing Consistency

Reasoning effort now flows through all dispatch paths identically:

- **Channel, cron, trigger, and `agent_send` turns** now use the routed model's reasoning policy, not a pre-routing downgrade. (#8729)
- **Streaming turns apply agent routing**, `pinned_model`, and tier/cost routing the same way scheduled turns do. (#8591)
- **Session model picks win everywhere** via `/model` override.

### Security and Isolation

From a comprehensive workspace audit:

- `/api/sessions` and transcripts are now **owner-scoped** — non-admin roles can no longer read every user's conversations. (#8588)
- **`web_fetch` cache is per-agent** — one agent can't read another's authenticated response. (#8591)
- **MCP credentials never leak** in config endpoints or sidecar environment. (#8591)
- **Shell flags in exec allowlists** are now validated individually (`-lc` can't slip through as one token). (#8588)
- Three critical RustSec advisories closed: `lru` 0.18.2 (unsound eviction), `wasmtime` 48.0.8 (Cranelift jump bug on aarch64), `rmcp` 3.9 (OAuth SSRF, tool schema cache). (#8610, #8722)

### Developer Experience Improvements

- **Form lists are now chips** in the dashboard: paste a comma or newline-separated list and each value becomes a removable chip. (#8657)
- **Manifest history is readable**: The TUI and dashboard now show human labels (`Model change`, `Suspended`, `Pre-restore snapshot`) instead of wire tokens. (#8603, #8604, #8607, #8609)
- **Tab accessibility fixed**: The dashboard's tab strips now carry proper `aria-controls`, `aria-labelledby`, and keyboard semantics (Arrow/Home/End navigation, roving tabindex). (#8608)
- **Identity file locks don't deadlock** in parallel tests anymore. (#8620)
- **Telegram sidecar has a poll deadline** (50s) — no more wedged workers after a 502. (#8662)

### Under the Hood

- **`apply_patch` anchoring**: Hunks that match multiple locations are now rejected; use `@@ <line>` scope headers or add context to disambiguate. (#8588)
- **Sidecar SDK MSRV raised to Rust 1.85** to track tokio 1.54. (#8695)
- **Per-agent config overrides** now live in `agent.toml` (not `config.toml`) — the kernel emits a targeted warning if it finds them in the wrong place. (#5476)
- **Session mode resolution** is documented in `docs/architecture/session-mode-resolution.md` for triggers, cron, and channels. (#8736)

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
- [GitHub Release](https://github.com/librefang/librefang/releases/tag/v2026.11.11)
- [GitHub](https://github.com/librefang/librefang)
- [Discord](https://discord.gg/DzTYqAZZmc)
- [Contributing Guide](https://github.com/librefang/librefang/blob/main/docs/CONTRIBUTING.md)
```
