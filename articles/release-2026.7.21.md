---
title: "LibreFang 2026.7.21 Released"
published: true
description: "LibreFang v2026.7.21 release notes — open-source Agent OS built in Rust"
tags: rust, ai, opensource, release
canonical_url: https://github.com/librefang/librefang/releases/tag/v2026.7.21
cover_image: https://raw.githubusercontent.com/librefang/librefang/main/public/assets/logo.png
---

# LibreFang 2026.7.21 Released

61 PRs from 4 contributors since v2026.7.11.

## What's New

This release brings **critical security improvements**, **multi-user isolation fixes**, and **powerful credential management** — all while expanding MCP integration and shipping quality-of-life wins across the dashboard.

### 🔐 Security & Multi-User Data Isolation

If you're running LibreFang with multiple users or on shared infrastructure, pay close attention here.

- **Per-user LLM provider credentials** — Users can now bring their own API keys with transparent per-owner spend tracking. Operators set org-wide provider allowlists and enforce per-user budgets directly at the API level.
- **Knowledge graph per-user scoping** — Fixed a cross-user data leak: the knowledge graph is now partitioned by user on multi-user agents, ensuring shared agents don't expose one user's memory to another.
- **Cross-account channel guards** — Enforced through the MCP bridge to prevent unauthorized agent-to-channel sends.
- **Session-owned compacted summaries** — Prevents prompt leaks across user sessions.
- **Four-pass security audit** — Caught and fixed 15+ bugs ranging from authorization gaps to token-quota races.

### 🚀 Better Developer Experience

- **MCP resources primitive** — Agents can now access MCP-exposed resources, completing the MCP integration beyond tools alone.
- **Live Slack progress for long tasks** — Long-running agent tasks now post real-time phase updates in Slack using Block Kit message edits, so users see progress instead of waiting for a single final reply.
- **HAND.toml online editing** — Edit hand manifests directly from the Hands panel in the dashboard — no file editing required.
- **Expanded delivery-target channel presets** — All sidecar adapters now support shortcut presets for common targets.

### 🛠️ Infrastructure & Provider Robustness

- **Multi-provider improvements** — Added response_format support for Gemini and Vertex AI; prefer live model catalog with build-time fallback.
- **Token quota safety** — Release reservations on drop to prevent self-imposed quota DoS.
- **Provider alias safety** — Canonicalize providers before per-user key lookup to block chargeback leaks.
- **Better error visibility** — Surface mid-stream provider errors instead of garbled or empty turns.

### Added

- Env opt-out (TELEGRAM_STREAMING) for the streaming path (#6482) (@houko)
- Per-user LLM provider credentials with per-owner usage attribution (initial) (#6483) (@houko)
- Org-wide LLM provider allowlist (fail-closed at driver resolution) (#6484) (@houko)
- Slack multi-step progress display via AgentPhase-driven Block Kit updates (#6487) (@houko)
- Per-user attribution survey + API-level user filtering of audit queries (#6488) (@houko)
- Process_start completion notification via the async task tracker (#6489) (@houko)
- Edit HAND.toml online from the Hands panel (#6490) (@houko)
- Scope the knowledge graph per user (peer_id) on multi-user agents (#6494) (#6502) (@houko)
- Expand delivery-target channel presets to all sidecar adapters (#6506) (@houko)
- Owner-gated CRUD for per-user provider credentials (#6460) (#6509) (@houko)
- Implement the MCP resources primitive (#6501) (#6532) (@houko)

### Fixed

- Prefer the live model catalog with a build fallback (#6384) (@pavver)
- Security and correctness hardening from repo-wide audit (#6438) (@houko)
- Second-pass security and correctness hardening from repo-wide audit (#6439) (@houko)
- Third-pass security and correctness hardening from repo-wide audit (#6441) (@houko)
- Fourth-pass security and correctness hardening from repo-wide audit (#6446) (@houko)
- Resolve four reported bugs (#6423, #6442, #6443, #6444) (#6449) (@houko)
- Enforce cross-account channel_send guard through the /mcp bridge (#6443) (#6455) (@houko)
- Trust operator env allowlist in sandbox_command (#6465) (@houko)
- Treat retired pnpm audit endpoint as skip, not a dependency issue (#6466) (@houko)
- Field-scope dm_policy/group_policy so a partial override stops silently gating groups, and expose them on [[sidecar_channels]] (#6445) (#6468) (@houko)
- Distinguish context and budget limits (#6479) (@houko)
- Allow dashboard login script under CSP (#6480) (@houko)
- Treat auto_dream fork tool calls as system-internal so RBAC does not gate them (#6485) (@houko)
- Login page unreadable in light theme (CSS cascade source-order bug) (#6486) (@houko)
- Pin login_page.html to LF so the CSP-hash test passes on Windows (#6481) (#6496) (@houko)
- Gate compacted summary by owning session to stop cross-user prompt leak (#6493) (#6497) (@houko)
- Honour glob patterns in per-agent tool_allowlist/tool_blocklist (#6495) (#6498) (@houko)
- Approvals approve 415 false-success + status column in approvals list (#6492) (#6500) (@houko)
- Stop over-blocking MCP arguments that carry a long numeric id (#6499) (#6503) (@houko)
- Route post-approval reply through account-qualified outbound (#6492) (#6511) (@houko)
- Surface mid-stream provider errors instead of empty/garbled turns (#6512) (@houko)
- Release the token reservation on drop to stop a quota self-DoS (#6513) (@houko)
- Attribute owner-key spend and enforce per-user budget on authenticated API turns (#6514) (@houko)
- Honor response_format in the Gemini and Vertex AI drivers (#6515) (@houko)
- Treat [browser] config-reload as restart-required, not a false hot-reload (#6516) (@houko)
- Canonicalize provider before the per-user key lookup to stop an alias chargeback leak (#6517) (@houko)
- Serialize a canonical-session override on the per-agent lock to stop a lost-update race (#6518) (@houko)
- Scope MCP knowledge_add_* writes to the calling agent, not agent_id="" (#6519) (@houko)
- Recognize CHANGELOG attribution on a bullet's continuation lines (#6520) (@houko)
- Don't orphan another agent's relations when deleting a shared entity's first-writer (#6522) (@houko)
- Honor auto_approve and return 409 on double-resolve (#6492) (#6528) (@houko)
- Surface on-disk upload path to agents for every file type (#6531) (@neo-wanderer)

### Changed

- Normalize on-disk upload naming to <uuid>.<ext> (#6530) (#6536) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Documentation

- Explain trusted_senders vs [[users]] RBAC composition (#6492) (#6507) (@houko)
- Document per-user key precedence over operator rotation (#6460) (#6510) (@houko)

### Maintenance

- Bump the cargo-minor-patch group with 10 updates (#6452) (@app/dependabot)
- Bump tokio-tungstenite from 0.29.0 to 0.30.0 (#6453) (@app/dependabot)
- Update yanked spin 0.9.8 to 0.9.9 (#6454) (@houko)
- Bump the actions-minor-patch group with 3 updates (#6456) (@app/dependabot)
- Bump actions/setup-node from 6.4.0 to 7.0.0 (#6457) (@app/dependabot)
- Lock in env trust split across defer→approve→resume (follow-up to #6465) (#6467) (@houko)
- Bump the web-minor-patch group in /web with 5 updates (#6472) (@app/dependabot)
- Bump the dashboard-minor-patch group in /crates/librefang-api/dashboard with 6 updates (#6473) (@app/dependabot)
- Bump @eslint/js from 9.39.4 to 9.39.5 in /crates/librefang-api/dashboard (#6474) (@app/dependabot)
- Bump serde_with from 3.18.0 to 3.21.0 (#6475) (@app/dependabot)
- Bump the docs-minor-patch group in /docs with 4 updates (#6491) (@app/dependabot)
- Update model snapshot (#6523) (@houko)
- Bump wasmtime from 46.0.1 to 47.0.1 (#6527) (@app/dependabot)
- Bump the cargo-minor-patch group across 1 directory with 17 updates (#6533) (@app/dependabot)
- Migrate librefang-acp to agent-client-protocol 1.3.0 (supersedes #6526) (#6534) (@houko)

</details>

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
- [GitHub Release](https://github.com/librefang/librefang/releases/tag/v2026.7.21)
- [GitHub](https://github.com/librefang/librefang)
- [Discord](https://discord.gg/DzTYqAZZmc)
- [Contributing Guide](https://github.com/librefang/librefang/blob/main/docs/CONTRIBUTING.md)
