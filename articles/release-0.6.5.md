# LibreFang 0.6.5 Released

---
title: "LibreFang 0.6.5 Released"
published: true
description: "LibreFang v0.6.5 release notes — open-source Agent OS built in Rust"
tags: rust, ai, opensource, release
canonical_url: https://github.com/librefang/librefang/releases/tag/v0.6.5-20260320
cover_image: https://raw.githubusercontent.com/librefang/librefang/main/public/assets/logo.png
---

# LibreFang 0.6.5 Released

We're thrilled to announce **LibreFang v0.6.5**! This release focuses on **security hardening**, **multi-provider LLM support**, and **improved deployment reliability**. Whether you're running on Linux, deploying to the web, or integrating with external LLM providers, this update has something for you.

## 🚀 New Features & Improvements

### LLM Provider Expansion & Cost Tracking
- **Add Qwen Code CLI as LLM provider** (#1224) — expand your model options with Qwen's code-focused offerings
- **Align init defaults with OpenRouter Stepfun** (#1262) — streamlined configuration for one of the fastest open models
- **Add token consumption metadata** (#1215) — better visibility into model usage and cost attribution

### Infrastructure & Core Features
- **Auto-initialize vault during `librefang init`** (#1206) — faster, zero-config setup for secure credential storage
- **Add image pipeline and subprocess management** (#1223) — enhanced capabilities for task-based workflows
- **Improved shell compatibility** (#1270) — smoother terminal experience across platforms

### UI & Brand Refresh
- **Replace all icons with new LibreFang branding** (#1263) — fresh visual identity across the dashboard and CLI

## 🔒 Security & Stability Fixes

### Security Hardening
- **Harden curl|sh install flow** (#1259) — safer installation from the web with improved source verification
- **Initialize rustls crypto provider for TLS connections** (#1294) — hardened TLS stack for secure communication
- **Decrypt encrypted webhook payloads** (#1208) — proper handling of sensitive inbound data
- **Address code-scanning path-injection findings** (#1260) — proactive security remediation

### Deployment & Reliability
- **Fix web deployment issues** (#1236, #1237, #1239, #1243) — resolved multiple asset serving and CI/CD blockers
- **Make shell installer POSIX-compatible for Linux** (#1226) — universal installer support
- **Bootstrap context engine during startup** (#1209) — more reliable initialization
- **Repair install smoke script** (#1255) — confidence in installation success

## 📚 Documentation & Maintenance

- **SDK usage examples in all READMEs** (#1229) — get started faster with working code samples
- **Updated contributor list** — thank you to everyone who made this release possible
- **Repository structure improvements** (#1211) — cleaner codebase organization
- **Node.js version standardization** (#1234) — consistent build environments
- **13 dependency updates** — keep the Rust ecosystem fresh (criterion, rand, tokio, rumqttc, and more)

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
- [GitHub Release](https://github.com/librefang/librefang/releases/tag/v0.6.5-20260320)
- [GitHub](https://github.com/librefang/librefang)
- [Discord](https://discord.gg/DzTYqAZZmc)
- [Contributing Guide](https://github.com/librefang/librefang/blob/main/docs/CONTRIBUTING.md)
