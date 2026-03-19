---
title: "LibreFang 0.6.3 Released"
published: true
description: "LibreFang v0.6.3 release notes — open-source Agent OS built in Rust"
tags: rust, ai, opensource, release
canonical_url: https://github.com/librefang/librefang/releases/tag/v0.6.3-20260319
cover_image: https://raw.githubusercontent.com/librefang/librefang/main/public/assets/logo.png
---

# LibreFang 0.6.3 Released

We're excited to announce **LibreFang v0.6.3** — another solid release packed with improvements across the board! This version brings better security, smoother onboarding, and important fixes that make your Agent OS experience more reliable.

## What's New

### 🚀 Better Onboarding & Token Management

The init experience just got smarter. LibreFang now **auto-initializes the vault** during `librefang init`, so you can start building agents right away without extra setup steps. We've also added **token consumption metadata** to help you track usage more precisely, and reduced the default "hands" (concurrent agent slots) to give you more conservative resource allocation out of the box.

### 🔒 Security & Encryption

A big one for webhook users — encrypted webhook payloads are now **properly decrypted** before processing, so your integrations with external services stay secure. The context engine also now bootstraps correctly during startup, ensuring your agent configurations are available from the moment the system comes up.

### 🐛 Bug Fixes

- **Channel tests** now properly support target IDs for more accurate testing scenarios
- **Linux installer** is now POSIX-compatible, fixing installation issues on various Linux distributions
- General stability improvements across the board

### 📦 Under the Hood

- Cleaned up the skills subsystem for better maintainability
- Tidied up the repository structure for a more organized codebase
- Updated contributor records

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
- [GitHub Release](https://github.com/librefang/librefang/releases/tag/v0.6.3-20260319)
- [GitHub](https://github.com/librefang/librefang)
- [Discord](https://discord.gg/DzTYqAZZmc)
- [Contributing Guide](https://github.com/librefang/librefang/blob/main/docs/CONTRIBUTING.md)
