```markdown
---
title: "LibreFang 0.6.8 Released"
published: true
description: "LibreFang v0.6.8 release notes — open-source Agent OS built in Rust"
tags: rust, ai, opensource, release
canonical_url: https://github.com/librefang/librefang/releases/tag/v0.6.8-20260320
cover_image: https://raw.githubusercontent.com/librefang/librefang/main/public/assets/logo.png
---

# LibreFang 0.6.8 Released

**LibreFang v0.6.8** is here, and we've made it easier than ever to get started! This release focuses on **accessibility** — our biggest win is distributing the CLI binary through npm and PyPI, so you can install LibreFang alongside your other dev tools. We've also improved multi-agent communication and tightened up our CI/CD pipeline.

## 🚀 Game-Changer: CLI Now on npm & PyPI

The standout feature of this release: **you can now install the LibreFang CLI directly from npm and PyPI** (#1323). No more hunting for pre-built binaries or compiling from source unless you want to. If you're a JavaScript or Python developer, it's as simple as `npm install -g librefang-cli` or `pip install librefang-cli`. This dramatically lowers the barrier to entry for teams already using these ecosystems.

## 💬 Better Multi-Agent Communication

We've enhanced how LibreFang handles external DM routing. Your agents can now intelligently route owner responses when handling external direct messages (#1266) — meaning cleaner, more predictable conversation flows across federated agent networks.

## 🛠️ Under the Hood

- **Smarter automation**: We now use the GitHub API directly to create Go SDK tags (#1321), making our release process more reliable and less error-prone.
- **Cleaner CI/CD**: Removed wasteful workflows and fixed several bugs to keep the build system lean (#1320).

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
- [GitHub Release](https://github.com/librefang/librefang/releases/tag/v0.6.8-20260320)
- [GitHub](https://github.com/librefang/librefang)
- [Discord](https://discord.gg/DzTYqAZZmc)
- [Contributing Guide](https://github.com/librefang/librefang/blob/main/CONTRIBUTING.md)
```
