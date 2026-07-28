---
title: "LibreFang 2026.7.27 Released"
published: true
description: "LibreFang v2026.7.27 release notes — open-source Agent OS built in Rust"
tags: rust, ai, opensource, release
canonical_url: https://github.com/librefang/librefang/releases/tag/v2026.7.27
cover_image: https://raw.githubusercontent.com/librefang/librefang/main/public/assets/logo.png
---

# LibreFang 2026.7.27 Released

We're shipping **v2026.7.27** with 33 improvements spanning provider integrations, operational polish, and critical fidelity fixes.
Here's what matters for your Agent OS deployments.

## New: Provider Integrations & Deployment

**EveryAPI is now a first-class model provider.**
Connect it via `librefang models connect everyapi` in the CLI or the new Providers page UI.
The CLI includes a built-in wiring doctor to catch misconfiguration—no more debugging silent auth failures.

**macOS users: LibreFang now runs at boot.**
The new `service install --system` command installs a LaunchDaemon so your daemon starts automatically at boot, before anyone logs in and without a user session.
Forget about manual restarts after reboot.

**NixOS and additional Linux distros are now officially supported.**
We've added first-class deployment support for NixOS with deepin and Debian distro awareness baked in, making it trivial to package and deploy across diverse Linux environments.

## Fixes: Correctness & Data Integrity

Three correctness fixes ship with this release:

- **Audit logs are now WORM-protected.**
  Audit entries persist even when an agent is purged, preventing accidental evidence loss in compliance-sensitive deployments.
- **Tool results are no longer lossy.**
  We fixed a spill dead band that was truncating tool outputs.
  Results now round-trip without silent truncation.
- **Context windows from `agent.toml` are now honored everywhere.**
  Previously some turn-execution paths ignored the configured `context_window`, leading to inconsistent behavior.
  This is fixed across all paths.

## Under the Hood

Security dependency bumps clear high-severity advisories in libvips (via sharp >=0.35.0) and Next.js App Router.
We've also hardened credential-pattern detection to reduce false positives, improved test reliability with connection pool timeouts, and fixed callback message ID resolution for MCP OAuth flows.

The TUI now runs on a process-lifetime runtime, preventing startup panics and silent session-summary loss during shutdown.

_33 PRs from 2 contributors since v2026.7.21._

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
- [GitHub Release](https://github.com/librefang/librefang/releases/tag/v2026.7.27)
- [GitHub](https://github.com/librefang/librefang)
- [Discord](https://discord.gg/DzTYqAZZmc)
- [Contributing Guide](https://github.com/librefang/librefang/blob/main/docs/CONTRIBUTING.md)
