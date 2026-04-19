```markdown
---
title: "LibreFang 0.7.0 Released"
published: true
description: "LibreFang v0.7.0 release notes — open-source Agent OS built in Rust"
tags: rust, ai, opensource, release
canonical_url: https://github.com/librefang/librefang/releases/tag/v0.7.0-20260321
cover_image: https://raw.githubusercontent.com/librefang/librefang/main/public/assets/logo.png
---

# LibreFang 0.7.0 Released

We're thrilled to announce **LibreFang v0.7.0**—a release focused on smarter LLM routing, expanded provider support, and migration tooling for OpenFang users.

## 🤖 Core AI Improvements

The centerpiece of this release is **LLM intent routing** with a unified registry, giving you more control over how agents route requests across providers. Streaming has been rock-solid, and we've squashed internal LLM call bugs that were stripping provider prefixes unexpectedly.

## 🧩 Provider Ecosystem Expansion

You can now detect and integrate with **Gemini CLI, Codex CLI, and Aider providers** through unified CLI detection. We've also added proper support for **Qwen Code CLI** across the setup wizard and test connections. If you're running OpenFang, our new `migrate --from openfang` command makes switching over painless—we've hardened the migration inputs and completed the migration across the init wizard, API, and dashboard.

## 🔧 Operations & Configuration

This release brings finer-grained control over your deployment:
- Configurable **CORS policies** for your environment
- **Channel rate limits** to prevent runaway resource usage
- **Audit pruning** to keep log storage under control
- **Media gates** for content filtering

## 📖 Documentation

We've reorganized docs as a Next.js deployment directory for better maintainability, added a comprehensive **comparison page** to help you understand LibreFang's positioning, and cleaned up remaining artifacts.

## 🛠️ Under the Hood

- Fixed npm/PyPI publish workflows (now running via Shell)
- Synced upstream improvements
- Hardened OpenClaw migration inputs
- Various streaming and registry fixes

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
- [GitHub Release](https://github.com/librefang/librefang/releases/tag/v0.7.0-20260321)
- [GitHub](https://github.com/librefang/librefang)
- [Discord](https://discord.gg/DzTYqAZZmc)
- [Contributing Guide](https://github.com/librefang/librefang/blob/main/CONTRIBUTING.md)
```
