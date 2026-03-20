```markdown
---
title: "LibreFang 0.6.7 Released"
published: true
description: "LibreFang v0.6.7 release notes — open-source Agent OS built in Rust"
tags: rust, ai, opensource, release
canonical_url: https://github.com/librefang/librefang/releases/tag/v0.6.7-20260320
cover_image: https://raw.githubusercontent.com/librefang/librefang/main/public/assets/logo.png
---

# LibreFang 0.6.7 Released

We're thrilled to release **LibreFang v0.6.7**—a focused update that strengthens extensibility, improves developer experience, and hardens our release pipeline. If you're building custom agents with HAND manifests or relying on context hooks, this one's for you.

## Extensibility & Scripting Improvements

The foundation of LibreFang's power is its flexibility. This release fixes critical gaps in how your custom code integrates:

- **Include user-installed HAND manifests in hand routing** (#1205) — Custom HAND agent manifests are now properly discovered and included in the routing system, eliminating the friction of manual registration.
- **Pass raw JSON payloads to context hook scripts** (#1207) — Context hooks now receive unprocessed JSON instead of serialized strings, giving you full control and eliminating unnecessary parsing overhead.

## Better Developer Experience

- **Self-heal fish config PATH entries** (#1303) — Your shell configuration now automatically recovers from corrupted PATH entries in fish, keeping your development environment stable without manual fixes.

## Community & Documentation

- **Add GitHub Discussions link to dashboard sidebar** (#1302) — Find answers and connect with the community without leaving the dashboard. We're making it easier for everyone to share knowledge and get help.

## Release & CI Reliability

Behind the scenes, we've tightened our release process to ensure smooth deployments:

- **Pass GITHUB_TOKEN to contributor/star-history scripts** (#1300) — Improved authentication for release automation, reducing friction in contributor tracking.
- **Fix 3 release workflow failures from v0.6.6** (#1309) — Resolved critical blockers in the release pipeline for more reliable deployments.

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
- [GitHub Release](https://github.com/librefang/librefang/releases/tag/v0.6.7-20260320)
- [GitHub](https://github.com/librefang/librefang)
- [Discord](https://discord.gg/DzTYqAZZmc)
- [Contributing Guide](https://github.com/librefang/librefang/blob/main/docs/CONTRIBUTING.md)
```
