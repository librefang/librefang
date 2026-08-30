---
title: "LibreFang 2026.8.30 Released"
published: true
description: "LibreFang v2026.8.30 release notes — open-source Agent OS built in Rust"
tags: rust, ai, opensource, release
canonical_url: https://github.com/librefang/librefang/releases/tag/v2026.8.30
cover_image: https://raw.githubusercontent.com/librefang/librefang/main/public/assets/logo.png
---

# LibreFang 2026.8.30 Released

554 PRs from 2 contributors have landed since v2026.8.19, bringing major improvements to agent memory, workflow composition, identity management, and operational reliability. Here's what you need to know.

## What's New

### Agents Can Now Think for Themselves

For the first time, agents can search, add, forget, and consolidate their own semantic memory without going through the REST API. Four new tools—`memory_semantic_search`, `memory_semantic_add`, `memory_semantic_forget`, and `memory_semantic_stats`—give agents direct access to what they've learned. This is the unlock for self-correcting memory: if a stale "service X is down" entry keeps being recalled, the agent can now find it, fix it, or merge near-duplicates reinforcing bad beliefs. Recall is also now split between extracted facts and raw dialogue, so memory works as a two-class system that allocates budget fairly instead of letting one class starve the other.

Session-scoped memory isolation is now configurable per agent, and per-capability access controls let you decide which agents can read vs. write their own stores—essential for shared or public agents where one visitor's context must stay separate from the next.

### Workflows Are Now Composable, Portable, and Inspectable

Agents can define workflows mid-conversation with the new `workflow_create` tool. Workflows now support named agent bindings—pick a specific running instance, an agent name resolved at step time, or an agent type spawned from a template—so the same workflow runs on any LibreFang instance without pre-registration. The Canvas editor makes this visible and editable, with per-step session mode and required-skills validation before dispatch.

Workflow runs now record their owner (a user or group), and every step's cost rolls up to the agent that asked for it, so spend attribution stays honest even when workflows fan out across shared agent types.

### Agents Can Spawn Ephemeral Workers

Most delegation is task-shaped, not colleague-shaped: hand work to a fresh context for one turn, get an answer back, and clean up. `agent_spawn` with `ephemeral: true` now does this without creating agent records, sessions, or workspaces. Workers inherit the spawning agent's tool set and spend cap, and nesting is bounded by the same recursion limit multi-step workflows use. `POST /api/agents/spawn-ephemeral` and the Quick Run button on the Agent Types page let you test this without writing an agent.

Worker runs leave a record with their task, answer, model, tokens, cost, and outcome—readable at `GET /api/agents/{id}/ephemeral-runs` so you can inspect what agents delegated and what it cost them.

### Identity and Access Control Are Now Group-Aware

Groups are now a first-class entity. Define a team in `config.toml` with `[[groups]]`, assign roles to every member, and use that group as the owner of workflows, cron jobs, or access policies. Identity-provider groups can now map onto local groups via `[external_auth.group_map]`, so removing someone from your Okta team or Keycloak group instantly revokes their LibreFang access. OIDC role claims can authorize API access through `[external_auth.role_map]`, completing the SSO picture.

### Slack Channels Got Smarter

Agents can now see who's in a channel with the `channel_members` tool, answer one person privately with `channel_dm`, and receive images and files that visitors upload. Display names are resolved through the Slack API with configurable caching, so your agent names people by handle instead of by ID. All of this is behind configurable flags—bulk enumeration, display-name resolution, and file forwarding—so you control the tradeoff between capability and data persistence.

### Backups Are Selective and Restorable

Restore a backup onto a running machine instead of only onto a blank slate, keep your current config while adopting everything else, and choose which components to restore—agents, cron jobs, memory, data directory. The TUI's new Backups tab lets you do this over SSH without hand-writing HTTP requests. Archive manifests carry component metadata, so the client always matches what it sends against what the archive actually contains.

Audit hash chains that have broken can now be repaired without destroying the evidence: `librefang security audit-reanchor` archives the broken rows to a tamper-evident JSON file before re-anchoring the chain, so compliance stays intact.

### Kubernetes and Mounted Configuration Just Work

Managed configuration mode now reaches every surface. The daemon resolves `config.toml` exactly once at boot and every interface reads that one path, so a Kubernetes ConfigMap or Docker bind mount is no longer a footgun. Pod rollouts are confirmed by comparing checksums—the same value the API reports back—instead of inferring from restart counts. A Kubernetes overlay at `deploy/kubernetes/overlays/managed-config/` shows the exact shape this was built for.

Provisioned agent directories let you declare agents in a tree the deployment owns, and the daemon prevents rewrites while keeping agent operations (suspend, resume, messages) fully available. Removal of a declaration releases the agent back to runtime ownership, so moving between managed and manual is reversible.

## Under the Hood

### Performance and Reliability

- Memory extraction no longer runs on turns that don't need it, and the extraction model is now visible and configurable instead of silently inherited.
- Confidence decay now charges exactly once per interval instead of accumulating quadratically, so memory doesn't mysteriously evaporate after a week of disuse.
- Recalled memory bullets are cut at sentence boundaries instead of mid-word, preserving readability and preventing the model from inventing missing text.
- Context windows for self-hosted gateways are now discovered and editable, so conservative fallback assumptions don't evict your actual content.
- The agent loop's memory section now splits budget between facts and dialogue fairly, and every clipped bullet is marked so you see what was omitted.
- Model overrides are editable from the dashboard and survive registry syncs, so one operator correction applies to every agent on that model.

### Developer Experience

- The CLI now accepts `--session-id` on `librefang message`, so scripted callers can address one conversation among many that a single agent serves.
- Workflow templates now appear in `librefang workflow run` output, and the TUI's workflow creator names all three agent binding modes it accepts.
- Agent-created skills now record the agent that made them instead of a placeholder, so the provenance is visible in the registry.
- The dashboard, REST API, and TUI all surface pending skills and MCP servers—declarations nothing has installed yet—so gaps in a workflow are visible before it runs.
- Usage and budget endpoints now accept date ranges and export to CSV, so monthly reporting doesn't require a client-side workaround.
- Error messages now name the budget cap that was hit and what it's set to, instead of generic "raise a limit" guidance.

### Security and Hardening

- Audio extracted from video is now bounded by both its container size and its own decoded size, preventing a large container from holding much more audio than the cap allows.
- Backup restore entry counts, decompression ratios, and extracted sizes are all bounded, and symlink escapes are rejected.
- Automatic memory respects session boundaries and per-capability access controls, so one visitor's context never leaks into the next.
- Agent manifests destined for a shared registry are automatically sanitized of instance-specific details—absolute paths, credential env vars, private URLs, free-text hostnames.
- The TUI templates screen no longer invents a capability declaration when spawning agents; operator-created types run with their actual declared permissions.
- Dashboard caller authentication is now moved off async workers and scrubbed from HTTP 500 responses while retained in server logs for diagnosis.

### Fixes

Over 200 fixes landed this release. Key themes:

**Concurrency and state**: Fixed lock poisoning in audit chains, credential vaults, MCP reloads, and a dozen other stateful paths. Serialized session refreshes, token refreshes, and config writes so concurrent mutations don't race.

**Type safety**: Added missing schema updates, fixed API contracts that had drifted from reality, and corrected generated client code.

**Gateway compatibility**: Models served through OpenAI-compatible proxies (vLLM, LM Studio, llama.cpp, LiteLLM) now discover context windows, handle reasoning-effort parameter mismatches, and accept cached discovery across restarts.

**Windows platform**: Fixed path separators in template checks, database reservations, and archive restoration so CI and deployments work identically on every OS.

**Channel adapters**: Slack, Telegram, Discord, WhatsApp, WeChat, Bluesky, Mastodon, Reddit, and a dozen others all got hardening against transient failures, malformed states, and protocol edge cases.

**Dashboard**: Nearly 100 UI fixes—cache invalidation, focus management, state initialization, async task cleanup, and accessibility improvements.

**Memory correctness**: Stopped memories from forking under concurrent writes, fixed embedding dimensionality checks, and corrected the class-blind recall that was evicting facts in favor of raw dialogue.

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
- [GitHub Release](https://github.com/librefang/librefang/releases/tag/v2026.8.30)
- [GitHub](https://github.com/librefang/librefang)
- [Discord](https://discord.gg/DzTYqAZZmc)
- [Contributing Guide](https://github.com/librefang/librefang/blob/main/docs/CONTRIBUTING.md)
