```markdown
---
title: "LibreFang 2026.3.21 Released"
published: true
description: "LibreFang v2026.3.21 release notes — open-source Agent OS built in Rust"
tags: rust, ai, opensource, release
canonical_url: https://github.com/librefang/librefang/releases/tag/v2026.3.2123-rc1
cover_image: https://raw.githubusercontent.com/librefang/librefang/main/public/assets/logo.png
---

# LibreFang 2026.3.21 Released

We're thrilled to announce **LibreFang v2026.3.21** — a major release packed with new provider support, a completely redesigned dashboard, granular configuration control, and 30+ stability fixes.

This release also marks our switch to **CalVer** (YYYY.M.DDHH) for clearer versioning. Here's what's new:

## 🚀 Expanded LLM Provider Ecosystem

Connect to more models than ever before:
- **Azure OpenAI** — Enterprise deployment ready
- **DeepInfra** — Access a wide range of open-source models
- **Qwen International & US** — Regional endpoints for lower latency
- **ChatGPT Device Auth** — New authentication flow for ChatGPT integration

All providers now include enriched error handling with provider/model context baked into `ClassifiedError` for faster debugging.

## ⚙️ Configuration & Control You Demanded

This release delivers fine-grained control over agent behavior:
- **Config Validation with Tolerant Mode** — Validate configs before startup with graceful fallback options
- **Per-Agent Plugin Scoping** — Control which plugins each agent can access via `allowed_plugins`
- **Configurable Embedding Dimensions** — Tune embedding models for your use case
- **FTS-Only Memory Indexing** — Skip embeddings if you only need full-text search
- **Custom Log Directory** — Point logs wherever you want
- **Configurable Session Reset Prompt** — Customize agent context reset behavior
- **Arbitrary Config Keys in Skills** — Add custom metadata to skill entries
- **Global Workspace Directory** — Share state across sessions

## 🎨 Dashboard & UI Overhaul

A **comprehensive React redesign** makes agent management delightful:
- New layout and navigation flow
- Better mobile responsiveness
- **Japanese localization** support
- Expanded i18n coverage for goals and analytics
- Fixed sidebar navigation and broken links
- Corrected provider/model counts

Plus a fresh promotional banner! 🎉

## 🤖 Agent Workflows & Automation

Build more sophisticated agents with new tools:
- **Pipeline Runner Agents** — Execute multi-step workflows with built-in IMAP email reader for email automation
- **WorkflowTemplate Types** — In-memory registry for reusable workflow definitions
- **/reboot Slash Command** — Graceful context reset without restarting
- **Workflow Editor Improvements** — Better handling of nested mode/error_mode definitions from the UI

## 🔌 Slack & Discord Integration Enhancements

More control over how agents interact with chat platforms:
- `unfurl_links` for Slack — Disable link previews when needed
- `force_flat_replies` for Slack — Keep messages in flat mode
- `mention_patterns` for Discord — Define custom mention behavior

## 🐛 Stability & Performance Improvements

30+ bug fixes and optimizations under the hood:
- **Streaming Reliability** — Prevent interrupts during multi-tool sequences
- **Infinite Retry Guard** — Dead branch cleanup and body size limits
- **LoopGuard Improvements** — Replaced fragile heuristics with robust poll detection
- **Knowledge Queries** — Fixed JOINs to match entities by name/ID with proper agent scoping
- **GitHub Stats** — Resolved zero values and optimized KV operations
- **MCP Improvements** — Now supports hyphens in server names
- **Desktop App** — Properly load .env files at startup
- **Docker Compose** — Fixed admin interface port binding
- **Windows Stability** — Fixed browser hand connection issues
- **Skill Resolution** — Fixed file path resolution for installed skill execution
- **Cache Performance** — Metadata caching reduces per-message overhead
- **Session History** — Replace processed images with text placeholders

## 📚 Documentation & Infrastructure

- Comprehensive docs review with updated numbers and new sections
- Auto-updating PR branches on main push
- GitHub Stats Worker in deploy workflow
- Improved Homebrew Cask Formula generation

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
- [GitHub Release](https://github.com/librefang/librefang/releases/tag/v2026.3.2123-rc1)
- [GitHub](https://github.com/librefang/librefang)
- [Discord](https://discord.gg/DzTYqAZZmc)
- [Contributing Guide](https://github.com/librefang/librefang/blob/main/CONTRIBUTING.md)
```
