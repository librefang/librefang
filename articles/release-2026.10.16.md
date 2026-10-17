💡 機密情報や個人情報を扱う作業の後は、新しいセッションへの切り替えをおすすめします。

```markdown
---
title: "LibreFang 2026.10.16 Released"
published: true
description: "LibreFang v2026.10.16 release notes — open-source Agent OS built in Rust"
tags: rust, ai, opensource, release
canonical_url: https://github.com/librefang/librefang/releases/tag/v2026.10.16
cover_image: https://raw.githubusercontent.com/librefang/librefang/main/public/assets/logo.png
---

# LibreFang 2026.10.16 Released

**LibreFang 2026.10.16** brings a unified agent editor, avatar support for agents and users, knowledge-base sharing, and significant stability improvements across the platform.

_144 PRs from 6 contributors since v2026.9.19._

## Agent Configuration & Identity

We've consolidated how you configure and identify agents. Everything — model parameters, personality, avatar, tools, skills, channels, memory access, cost limits, and planning settings — now lives in one unified **config tab** on the agent view. No more hunting across drawer, routing panel, and schedule tabs.

**Agent and user avatars** are here: upload images or use emoji to personalize your agents and teammates across the dashboard, session lists, and every chat message. The agent list now shows avatars too, making it easy to tell your agents apart at a glance.

**Agent manifest version history** records every configuration change with its source (dashboard edit, restore, raw TOML, …), so you can review what changed and when, restore any prior version with a diff preview, and see the history from the TUI or the dashboard without switching contexts.

## Knowledge Management & Chat

**Knowledge bases** let you upload documents once and grant access to specific agents. No more per-session file uploads — write `@knowledge-base/document.md` in your prompt and any agent with access can read it. The UI surfaces knowledge-base sharing alongside every other agent capability.

**Chat improvements**: Mermaid diagrams now render inline instead of showing raw syntax. Conversations with the same agent appear as tabs instead of hiding behind a dropdown. You can now **select specific messages to export or print**, so you get a clean document of just what matters without the whole sidebar.

## Model Control & Discovery

Model parameters (`top_k`, `min_p`, `repeat_penalty`) now have dedicated UI controls in the dashboard and TUI instead of living in untyped `extra_params`. The **searchable model picker** replaces free-text model entry — no more memorizing model IDs.

**Live model discovery** preferences are preserved across restarts, and providers can now map credentials to request headers for authenticated remote MCP servers. Discovery still fails gracefully when endpoints are slow, and gateways report pricing and capacity through their optional `/model/info` extension.

## Terminal & Automation

The TUI got major upgrades: **shared-folders editor** lets you declare workspaces without opening the manifest in another window. **Goals screen** now surfaces loop engineering, per-goal cadence, and verification settings. **Workflow runs** display live step progress (`▰▰▱▱▱ 2/5`). **Agent configuration history** renders in the terminal, so terminal-only operators can audit changes without opening a browser.

The TUI also **reports daemon errors** instead of showing empty lists or spinners that never stop — fetch errors now surface with the daemon's reason and status.

## Tasks & Approvals

**Task priority and per-task timeouts** are now exposed in the UI (the New Task dialog gained both fields). Tasks assigned to non-existent agents are now rejected upfront instead of queueing forever.

Approval notifications now route correctly through sidecars, and authenticated dashboard users now respect your configured memory access policy instead of failing open.

## Stability & Security

**Sampling parameters** (`top_p`, `frequency_penalty`, `presence_penalty`) now reach the wire correctly for every provider that uses them — Anthropic, Gemini, Ollama, OpenAI-format drivers, and custom endpoints all get the right fields without fallbacks.

Ephemeral worker runs now report when they hit the iteration cap, and when they do, you get what they accumulated before stopping instead of silence. History compaction runs on every turn type now, not just streaming.

**SSH transport security**: upgraded russh, closing four SSH advisories. **Filename guard** validates all incoming names against Windows-hostile characters and path exploits, refusing rather than sanitizing. **Cron scripts** must resolve under the scripts directory now, closing a path traversal. Integration installs no longer write untrusted keys into the vault.

The `||spoiler||` and `==highlight==` syntax works again in Telegram messages (these had silently vanished on the blocks-based path). Voice messages to users who forbid them fall back to audio documents. Windows concurrent writes to identity files no longer fail spuriously.

## Behind the Scenes

- Mermaid dependency vulnerabilities (CVE-2026-4800, CVE-2026-2950) resolved by overriding lodash-es on the four packages that carry it.
- LiteLLM gateways now fold `/model/info` into their discovery results when the listing leaves capacity unknown.
- Model routing clearly indicates when Stable mode makes it inert — both in the API response and in the UI.
- Personality edits now survive a provider swap (vibe, archetype, greeting_style used to disappear).
- Agent workspace collisions during spawn are detected and refused — you can no longer accidentally give an agent another agent's directory.
- A renamed or cloned agent no longer introduces itself by its old name.
- Kubernetes deployments now have a **community Helm chart** mirroring the Kustomize manifests, with proper `restricted` Pod Security and single-replica StatefulSet enforcement.
- Config hot-reload is smarter: a field you omit from the reload document now keeps its live value instead of resetting to the compiled default. Explicitly state `key = []` to clear a list.
- Binary versioning now tracks the actual commit it was built from, even in worktrees and after branch rewrites.

## Developer Experience

- CLI skill publishing now reaches GitHub Enterprise Server via `skills.promotion.api_base_url`.
- The Rust and Telegram sidecars now commit `Cargo.lock` and MSRV is measured rather than declared (SDK 1.71, Telegram 1.88).
- Generated SDKs now correctly type response bodies for all endpoints (was untyped blob for registry-diff and restore).
- `librefang skill publish --dry-run` prints the configured repository.
- macOS binaries are now Developer ID signed, preserving Full Disk Access and Automation grants across updates.

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
- [GitHub Release](https://github.com/librefang/librefang/releases/tag/v2026.10.16)
- [GitHub](https://github.com/librefang/librefang)
- [Discord](https://discord.gg/DzTYqAZZmc)
- [Contributing Guide](https://github.com/librefang/librefang/blob/main/docs/CONTRIBUTING.md)
```

This rewrite:
- **Opens with context** about what's included (144 PRs, 6 contributors)
- **Groups by theme** (agent config, knowledge, chat, models, terminal, tasks, stability, security, internals, DX)
- **Highlights impact** (unified config, avatars, knowledge bases, better error reporting, security fixes)
- **Skips the raw list** and pulls up the most relevant details under each section
- **Uses concrete examples** (`@knowledge-base/document.md`, `▰▰▱▱▱ 2/5`, `CVE-2026-4800`)
- **Keeps the depth** but makes it scannable and developer-friendly
- **Preserves all sections exactly**: front matter, Install/Upgrade, and Links are unchanged
