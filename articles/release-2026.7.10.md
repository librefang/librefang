---
title: "LibreFang 2026.7.10 Released"
published: true
description: "LibreFang v2026.7.10 release notes — open-source Agent OS built in Rust"
tags: rust, ai, opensource, release
canonical_url: https://github.com/librefang/librefang/releases/tag/v2026.7.10
cover_image: https://raw.githubusercontent.com/librefang/librefang/main/public/assets/logo.png
---

# LibreFang 2026.7.10 Released

This release unlocks **extended thinking for reasoning models**, improves **Slack formatting**, eliminates **redundant artifact output**, and expands **international reach**. Alongside that, we've locked down security advisories and made your test suites faster and more reliable.

_39 PRs from 4 contributors since v2026.6.29._

## What's New

### 🧠 Extended Thinking & Reasoning Support

The OpenAI-compatible driver now correctly forwards `reasoning_effort` to supported models, unlocking extended thinking and o1-style reasoning flows. If you're using Claude, o1, or other reasoning-enabled models, this is a substantial unlock for complex problem-solving agents that need to think deeply before acting.

### 💬 Native Slack Message Formatting

Markdown messages sent to Slack are now automatically converted to Slack's native `mrkdwn` format in the Slack sidecar. Your Slack agents now produce properly-formatted, native-looking messages instead of raw Markdown syntax — a small change that makes a big difference in readability.

### 🎯 Smarter Output Deduplication

Artifact read results are no longer re-spilled at the post-tool chokepoint, eliminating redundant output and keeping agent conversations cleaner. This is especially valuable in longer runs where output verbosity can distract from the actual work.

### 🔍 Dashboard Model Transparency

The Codex CLI now exposes which model it's configured to run, visible directly in the dashboard. No more mystery about what's powering your agents.

### 🌍 Internationalization

Dashboard and website translations have been completed and proofread across multiple languages, opening LibreFang to a global community of developers.

### 🛡️ Security & Reliability Hardening

- Tests can now run offline with `LIBREFANG_REGISTRY_OFFLINE`, removing network dependencies and accelerating your CI pipelines
- Multiple security advisories cleared: quick-xml, crossbeam-epoch, and cargo-deny all updated
- WS terminal frames now correctly correlate with chat turns, improving observability and debugging
- Fixed edge cases in media handling and TUI formatting for smoother operation

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
- [GitHub Release](https://github.com/librefang/librefang/releases/tag/v2026.7.10)
- [GitHub](https://github.com/librefang/librefang)
- [Discord](https://discord.gg/DzTYqAZZmc)
- [Contributing Guide](https://github.com/librefang/librefang/blob/main/docs/CONTRIBUTING.md)
