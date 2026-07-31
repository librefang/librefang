```markdown
---
title: "LibreFang 2026.7.31 Released"
published: true
description: "LibreFang v2026.7.31 release notes — open-source Agent OS built in Rust"
tags: rust, ai, opensource, release
canonical_url: https://github.com/librefang/librefang/releases/tag/v2026.7.31
cover_image: https://raw.githubusercontent.com/librefang/librefang/main/public/assets/logo.png
---

# LibreFang 2026.7.31 Released

**58 PRs from 4 contributors since v2026.7.27.**

LibreFang 2026.7.31 ships production-ready multi-account support, Kubernetes deployment, EveryAPI auto-detection, and a security hardening pass that closes 20+ authorization and credential-handling gaps.

## Core improvements

**API security gets teeth:** Master API keys now support hashing (SHA-256 or Argon2id), environment variable indirection, and vault storage — so you can rotate the secret without touching `config.toml`. The hash-only mode closes a WebSocket/terminal auth bypass that would have re-opened a bare-key exposure path. Per-user provider credentials (added in the last release) now correctly bill and budget the user whose key is being spent, and the authorization system gained 20 fixes to ownership checks, RBAC gates, and cross-account attack surfaces.

**EveryAPI integration is automatic:** The daemon now auto-detects the EveryAPI CLI if you're logged in, sources model metadata from the gateway itself (not a stale OpenRouter snapshot), and refreshes the catalog on a configurable TTL. A new dashboard connect action means you never need a terminal to add the gateway.

**Browser automation just got richer:** Agents can now attach to browser-level Chrome DevTools Protocol endpoints via `Target.createTarget`, enabling session-wide automation instead of page-specific. Page extraction is now configurable (max characters, reported in the truncation marker), and video containers (MP4, MOV, MKV, AVI) are accepted for transcription with automatic audio extraction.

**Hands stay customized across daemon restarts:** Edits to built-in hands now persist to an operator override directory instead of getting erased by the next registry sync. The hand's live configuration also reaches agents immediately on save from the dashboard, not just on restart.

**Multi-account messaging works correctly:** Channel status indicators now reflect supervisor liveness, per-channel settings reach the right bot account, and deferred approvals route through the account-qualified delivery path.

**Kubernetes is supported out of the box:** A single-replica baseline with readiness contract, restricted Pod Security, and rootless support is shipped under `deploy/kubernetes/`. The daemon gains `LIBREFANG_API_KEY` env override and `--system` macOS LaunchDaemon support for always-on installs.

---

## Added features

- **EveryAPI gateway auto-detection and dashboard connect:** Read credentials from `~/.config/everyapi/credentials.json` on demand, fetch live model list from the gateway's `/api/pricing`, refresh on a configurable TTL, auto-detect CLI login, and surface a connect action on the dashboard (no terminal required).

- **Per-user LLM provider credentials:** Users can now store their own upstream provider API keys (encrypted in the vault), bill their own spend against their own budget, and the cost is attributed correctly per-user across all spend paths (REST, streaming, ephemeral).

- **Hand customization on disk:** Edits to registry-shipped hands now persist to `~/.librefang/hands/` instead of getting erased on sync. Hand settings changes reach live agents immediately; manifest edits are applied on the next activation.

- **Browser-level Chrome DevTools Protocol:** Agents can attach to browser-level CDP endpoints by creating a target first, enabling session-wide automation. Page extraction is now operator-configurable (`[browser] max_content_chars`) and reports the pre-truncation length so the model knows what it's missing.

- **Video container support for transcription:** `media_transcribe` and `speech_to_text` now accept MP4, MOV, MKV, AVI containers with automatic audio extraction and Ogg/Opus re-encode.

- **STT language and prompt parameters:** `language` and `prompt` parameters now reach Whisper-compatible providers (Groq, OpenAI, MiniMax, Fireworks, Together, SiliconFlow) instead of being silently dropped.

- **MCP resources primitive:** The client now implements `resources/list`, `resources/read`, and `resources/templates/list`, surfacing server resources as synthetic tools and inlining resource content into prompts.

- **Kubernetes-ready deployment:** Single-replica manifests under `deploy/kubernetes/` with restricted Pod Security, readiness probes on `/api/ready` (separate from liveness), and rootless support.

- **macOS system LaunchDaemon:** `service install --system` now registers a boot-time LaunchDaemon (instead of login-time LaunchAgent), so the daemon starts even when the user hasn't logged in yet.

- **Per-channel sidecar settings:** `dm_policy`, `group_policy`, `threading`, and `output_format` overrides per `[[sidecar_channels]]`, restoring fine-grained control lost in the sidecar migration.

- **Config schema in `GET /api/config`:** The response now includes every writable field plus a `x-non-writable` list so the dashboard can render read-only fields with hints to edit `config.toml` instead.

- **Explicit hand scheduling:** Hands now require an explicit `[autonomous]` block or `schedule` to tick — no more silent background loops inferred from `max_iterations`.

- **Registry sync toggle:** Set `[registry] auto_sync = false` to freeze the checkout and prevent edits from being erased on daemon restart.

- **Skill changelog fragments:** Write one `.md` file per changelog entry under `changelog.d/{added,fixed,changed}/` to avoid merge conflicts — fragments are auto-collected into `## [Unreleased]` on release.

- **Process completion notifications:** `process_start` with `notify_on_completion: true` triggers a `TaskCompletionEvent` when the process exits, reusing the async-task delivery path (no polling needed).

- **All 25 channel type adapters in scheduler:** The scheduler delivery-target dropdown now lists WeChat, WeCom, Feishu, DingTalk, Teams, Google Chat, Matrix, Mattermost, WhatsApp, QQ, LINE, and the rest by name.

- **MCP OAuth via dashboard:** MCP servers can now be configured with OAuth credentials through the dashboard, and the MCP bridge carries sender account context for proper access control.

- **NixOS first-class support:** Flake gains `nixosModules.default` with `services.librefang` configuration, `overlays.default`, and evaluation-time assertions (e.g., no `environmentFile` inside `/nix/store`).

- **Distributed debug symbols:** Release binaries ship with stripped executables and separate `.dSYM` / `.dwp` debug symbols, so crash reports can be symbolized without including symbols in the download.

---

## Fixed bugs & security issues

**Authorization & access control (20+ fixes):**
- Owner-gate `GET /api/config/export` (previously exposed plaintext master key to any Admin).
- Authorize channel slash-commands (`/approve`, `/switch-agent`, etc.) through RBAC before dispatch.
- Role-gate terminal and agent WebSocket upgrades (Admin+ for PTY, User+ for turns).
- Enforce ownership on `process_poll` / `process_write` / `process_kill`.
- Gate MCP-server config mutations to Owner (RCE risk via plugin lifecycle scripts).
- Scrub daemon environment before `process_start` spawns (was exposing `LIBREFANG_VAULT_KEY` and provider keys).
- Stop persistent plugins from inheriting full daemon environment.
- Bind `POST /api/auth/refresh` to the session it can prove it owns (was leaking other users' upstream tokens to Admin callers).
- Apply dangerous-command blocklist to `process_start` (was bypassing `shell_exec` allowlist checks).
- Enforce `webhook_triggers.max_payload_bytes` at the wire level.

**Multi-user data isolation (6+ fixes):**
- Scope knowledge graph (entities/relations) per user, mirroring the memory scoping from the prior release.
- Stop compacted-session summaries from leaking across users on multi-user agents.
- Correctly attribute per-user provider-key spend to the user and enforce their budget.
- Scoped `GET /api/memory/agents/{id}/relations` to the path's agent.
- Prevent cross-account `channel_send` tool calls.
- Key channel message deduplication on per-user sender ID.

**Credential & secret handling (8+ fixes):**
- Accept `api_key_hash` (`$sha256$...`) in place of plaintext master key.
- Route master API key through env/vault indirection without re-reading `config.toml` on reload.
- Hash-only credentials now work for WebSocket, terminal, and HTTP (previously were a bypass).
- Stop EveryAPI gateway base URL from being repointed to whatever account the CLI is logged into.
- Canonicalize provider name before per-user key lookup (so aliases bill the user's key, not fallback).
- Stop `GET /api/mcp/servers` from serializing MCP environment values (was exposing inline credentials).
- Reject cross-account `channel_send` on the `/mcp` bridge path.
- Extended thinking now emitted through OpenAI-compatible driver (was parsed on receive only).

**Streaming & error handling:**
- Surface mid-stream LLM errors (overload, rate-limit) instead of silently truncating to an empty "success" turn.
- Apply content-emitted guard at both streaming layers so a provider error doesn't concatenate partial responses.
- Classify `insufficient_quota` as a billing error (not a malformed request) and apply long retry backoff.
- Terminal frames are now correlated with their chat turns (late frames no longer overwrite newer answers).

**Configuration & state:**
- Config-reload now correctly gates `[registry] auto_sync` to live-read (was falsely reporting restart-required).
- `api_key_hash` regenerates upgrade hints and warns if no transmittable key exists.
- Distinguish `json.null` from unset fields in the `http_compat` server round-trip (was corrupting redacted header values).
- Reject unknown keys in `[mcp_servers.transport]` and both `HttpCompat` variants (was silently discarding typos).
- Compare canonicalized JSON values in config-reload diffs (was spuriously reporting changes on every reload for maps).
- Parse `frequency = "reactive"` without error (was a validation error while having no effect anyway).

**Session & message routing:**
- Serialize `session_id_override = canonical_entry` on the same lock as no-override persistent dispatch (was losing history on concurrent writes).
- Resolve model context window provider-aware (OpenRouter agents now show real window, not 8192-token fallback).
- Clear persistent `compacted_summary_session_id` on model switch so budget math uses the right window.
- Cron fires with `session_mode = "new"` now isolate per fire (was sharing one `for_sender_scope` session).
- `GoalRunner::start` now atomic (was leaving orphaned, unstoppable loops on concurrent starts).
- Stop non-passive agents from being flagged unresponsive (the heartbeat monitor was using the wrong schedule check).

**Tools & skill execution:**
- Apply `tool_allowlist` when rendering MCP tools (was ignoring glob patterns like `mcp_notion__*`).
- Apply `tool_allowlist` on MCP tool execution, not just display.
- Stop skill `description` and tool names from bypassing the prompt-injection scanner.
- Stop the taint heuristic from false-positive-blocking numeric IDs (`note-12345-67890.pdf`) and long runs of digits.
- Tool allowlist entries that cannot grant are now named in the response (not silent no-ops).
- `read_artifact` results are exempt from re-spill gates (was looping on artifact fetch for results larger than `spill_threshold_bytes`).

**Data correctness:**
- Session detail endpoint now reports `cost_usd`, `total_tokens`, `duration_ms`, and `label` (was missing all four).
- Goal creation now normalizes blank `parent_id` / `agent_id` to "absent" and validates non-blank UUIDs up front.
- Stop agent deletion from silently orphaning shared knowledge entities (was breaking entity ownership model).
- Stop hand activation from fabricating `mode = "full"` exec policy (was silently bypassing approval queues).
- Stop `agent kill` from deleting audit trail rows (was breaking the WORM Merkle chain).
- Stop a partial `[channel_overrides]` table from gating group messages (was materializing defaults for unset fields).

**Network & storage:**
- Stop symlinks in skill installers from copying external tree contents.
- Bound ClawHub/Skillhub skill-zip extraction (was vulnerable to decompression bombs).
- Stream attachment-URL fetches with running size cap instead of buffering (was OOMing on unbounded bodies).
- Per-IP WebSocket connection tracker guard-drop TOCTOU fixed (was deleting live connection entries).
- Tolerate empty-string error objects in OpenAI-compatible streaming (was aborting stream on benign `"error": ""`).

**CLI & integration:**
- Telegram menu button callbacks now work (was silently dropping on both Rust and Python sidecars).
- Matrix sidecar inbound no longer silently dropped under default `GroupPolicy::MentionOnly`.
- Correct the channel docs' claim about env var names and formats.
- `librefang skill install ./dir` now accepts `SKILL.md`-only directories and Git URLs.
- `librefang skill search` and `install` now point at the local synced registry (was pointing at nonexistent GitHub org).
- `librefang doctor` EveryAPI check now warns on stale credentials (was reporting green after logout).
- Dashboard Clone button now opens a dialog (was posting empty body and always failing).

---

## Changed

- Bump `agent-client-protocol` to 2.0.0 and update call sites.
- Normalize on-disk upload naming to `<uuid>.<ext>` across all producers.
- Thread user identity into audit trail for human-initiated API actions.
- Migrate Linux system tray from `tray-icon` to pure D-Bus `ksni` implementation.
- Docker entrypoint now detects root vs. rootless mode and adapts (works in Kubernetes without a second image tag).
- Consolidate invisible/format code-point lists into a single source of truth.
- Stop releasing stable CLI to homebrew-tap (now ships from homebrew-core; tap carries `beta` / `rc` / versioned formulas only).
- Refresh OpenRouter model snapshot.
- Move all CI/release workflows from Node 20 to Node 24.
- Split release debug symbols into a separate asset (`.dSYM` / `.dwp`).

---

## Security

This release closes 30+ authorization, credential-handling, and data-isolation gaps discovered in the prior release's multi-user and per-user credential work. Key hardening:

- Owner-gated MCP-server mutations, plugin installation, and `/api/config/export`.
- RBAC now gates terminal and WebSocket upgrades by role (Admin+ for PTY, User+ for turns).
- Per-user budget enforcement on provider-key spend; spend is correctly attributed and audited.
- Hash-only API key now works for WebSocket/terminal (closes a previous bypass).
- Credential environment values no longer leak from `GET /api/mcp/servers`.
- Canonical provider names used in per-user key vault lookups (aliases now bill user keys correctly).
- Dangerous-command blocklist now applied to `process_start`.
- Daemon environment scrubbed before spawning persistent processes.
- Cross-account `channel_send` blocked on both HTTP and `/mcp` bridge paths.
- Knowledge graph scoped per user on multi-user agents.
- Compacted-session summaries no longer leak across users.
- Unknown keys in MCP config rejected at boot (was silently dropping typos).

---

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
- [GitHub Release](https://github.com/librefang/librefang/releases/tag/v2026.7.31)
- [GitHub](https://github.com/librefang/librefang)
- [Discord](https://discord.gg/DzTYqAZZmc)
- [Contributing Guide](https://github.com/librefang/librefang/blob/main/docs/CONTRIBUTING.md)
```
