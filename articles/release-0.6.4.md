---
title: "LibreFang 0.6.4 Released"
published: true
description: "LibreFang v0.6.4 release notes — open-source Agent OS built in Rust"
tags: rust, ai, opensource, release
canonical_url: https://github.com/librefang/librefang/releases/tag/v0.6.4-20260320
cover_image: https://raw.githubusercontent.com/librefang/librefang/main/public/assets/logo.png
---

# LibreFang 0.6.4 Released

We're excited to announce **LibreFang v0.6.4** — another solid update to your open-source Agent Operating System! This release brings new LLM providers, important stability improvements, and a fresh coat of paint. Let's dive in.

## What's New

### Expanded LLM Support

LibreFang now supports **Qwen Code CLI** as a new LLM provider (#1224), giving you even more options for powering your agents. We also aligned the init defaults with **OpenRouter Stepfun** (#1262), so new users get better out-of-the-box configuration.

### Image Pipeline & Subprocess Management

The new **image pipeline and subprocess management** feature (#1223) enables richer agent capabilities — think image processing workflows and better control over external tool execution.

### Fresh Visual Identity

We replaced all icons with new **LibreFang branding** (#1263), giving the project a more cohesive and professional look. It's the same great functionality, now easier on the eyes.

## Stability & Security

### Web Deployment Overhaul

This release tackles a long-standing issue: web deployment reliability. We landed multiple fixes (#1236, #1237, #1238, #1239, #1243) that should make deploying and running the web interface much smoother. If you've had trouble with web deployment before, this update is for you.

### Hardened Install Flow

The one-line install script (`curl -fsSL https://get.librefang.ai | sh`) got a security hardening pass (#1259). We also addressed code-scanning path-injection findings (#1260) — small things that make a big difference for security-conscious deployments.

### Bug Fixes

- Fixed web UI chat input line break issue (#1245) — no more broken messages
- Repaired the install smoke script and dropped an outdated workflow (#1255)

## Developer Experience

### Better Documentation

SDK usage examples now appear in **all README files** (#1229), so whether you're using Rust, JavaScript, or Python, you'll find clear guidance getting started.

### Infrastructure Improvements

- Added `.nvmrc` for web Node.js version consistency (#1234)
- Fixed Dockerfile path issues (#1234)

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
- [GitHub Release](https://github.com/librefang/librefang/releases/tag/v0.6.4-20260320)
- [GitHub](https://github.com/librefang/librefang)
- [Discord](https://discord.gg/DzTYqAZZmc)
- [Contributing Guide](https://github.com/librefang/librefang/blob/main/CONTRIBUTING.md)
