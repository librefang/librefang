```markdown
---
title: "LibreFang 0.5.6 Released"
published: true
description: "LibreFang v0.5.6 release notes — open-source Agent OS built in Rust"
tags: rust, ai, opensource, release
canonical_url: https://github.com/librefang/librefang/releases/tag/v0.5.6-20260317
cover_image: https://raw.githubusercontent.com/librefang/librefang/main/public/assets/logo.png
---

# LibreFang 0.5.6 Released

We're thrilled to ship **LibreFang v0.5.6**—a release focused on improving the developer experience and strengthening integrations with messaging platforms.

## 🌍 Global Developer Experience

**Multi-language Support for CLI & API Errors** — Error messages across the CLI and API are now localized, making LibreFang more accessible to developers worldwide. Debug with confidence in your preferred language.

## 💬 Enhanced Messaging & Notifications

**Telegram Reply-to Improvements** — Better handling of Telegram reply-to messages with proper truncation and metadata support. Keep your bot conversations clean and contextual, even in long message chains.

**Bluesky Notifications** — Fixed reliability issues in the Bluesky notification workflow to ensure your automated announcements reach their destination consistently.

## 🚀 Automation & SDK Updates

**Automated Release Announcements** — LibreFang now automatically generates Dev.to articles for releases, keeping your community in the loop without manual effort.

**SDK Publishing Fixes** — Resolved distribution issues across the Rust, JavaScript, and Python SDKs for smoother installation and package management.

## 🔧 Bug Fixes

- Fixed YAML syntax error in Bluesky notification workflow configuration

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
- [GitHub Release](https://github.com/librefang/librefang/releases/tag/v0.5.6-20260317)
- [GitHub](https://github.com/librefang/librefang)
- [Discord](https://discord.gg/DzTYqAZZmc)
- [Contributing Guide](https://github.com/librefang/librefang/blob/main/CONTRIBUTING.md)
```
