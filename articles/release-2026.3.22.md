```markdown
---
title: "LibreFang 2026.3.22 Released"
published: true
description: "LibreFang v2026.3.22 release notes — open-source Agent OS built in Rust"
tags: rust, ai, opensource, release
canonical_url: https://github.com/librefang/librefang/releases/tag/v2026.3.2201
cover_image: https://raw.githubusercontent.com/librefang/librefang/main/public/assets/logo.png
---

# LibreFang 2026.3.22 Released

We're thrilled to announce **LibreFang v2026.3.22** — a massive release with expanded provider support, powerful new capabilities, and a completely overhauled dashboard. This release marks our evolution to Calendar Versioning and brings 50+ improvements across the entire platform.

## What's New

### 📊 Major Architecture Update: Calendar Versioning
We've switched to **CalVer (YYYY.M.DDHH)** for more transparent and predictable release cycles. This change makes it easier to track what's fresh and plan your upgrades accordingly.

### 🔌 Expanded Provider Ecosystem
Connect to virtually any LLM provider now:
- **New providers**: DeepInfra, Azure OpenAI, Qwen International/US endpoints
- **ChatGPT device auth flow** for seamless authentication
- **Custom model support** with proper alias registration and provider routing
- **Embedding dimensions** configurable per agent — fine-tune costs and quality

### 🤖 Agent & Workflow Superpowers
- **Pipeline runner agents** let you orchestrate complex multi-step workflows
- **IMAP email reader script** for email-triggered automation
- **WorkflowTemplate registry** (in-memory) for reusable workflow definitions
- **Per-agent plugin scoping** with granular `allowed_plugins` config
- **Graceful context reset** via `/reboot` slash command

### ⚙️ Configuration & Persistence
- **Global workspace directory** for cross-session state persistence
- **Custom log directory config** — store logs anywhere you want
- **Config validation with tolerant mode** — cleaner error messages, safer defaults
- **Arbitrary config keys in skills** — unlimited flexibility for custom skill parameters
- **Configurable session reset prompt** — personalize context resets
- **Knowledge query improvements** — JOIN matches entities by name or ID, indexed by agent

### 💬 Chat Platform Enhancements
- **Slack**: Unfurl links option, force-flat replies config for cleaner threads
- **Discord**: Configurable mention patterns for smarter mentions
- **HAND.toml format** now accepts flexible [hand] wrappers for better compatibility

### 🚀 Performance & Stability
- **Infinite retry guard** — prevents runaway loops
- **Streaming safety** — prevent interrupts during multi-tool sequences
- **Memory caching** — workspace and skill metadata cached to reduce per-message overhead
- **Optimized KV operations** — faster, more efficient state management
- **Migration tools** — refresh param for cache bypassing, sparse chart data handling
- **Improved polling detection** — replaced fragile heuristics with robust LoopGuard

### 🎨 Dashboard & UX
- **Complete React UI/UX overhaul** — modern, responsive, beautiful
- **Japanese localization** + comprehensive i18n coverage for goals/analytics
- **SVG branding banners** — fresh promotional materials
- **Fixed navigation** — better link structure and mobile indicators
- **Dev server integration** — `just api` now starts dashboard + API together

### 🛠️ Developer Experience
- **Rustfmt.toml** for consistent code formatting across the project
- **Version & git hash in startup logs** — instant transparency on what's running
- **Desktop app `.env` support** — easier local configuration
- **Homebrew Cask CI sync** — streamlined distribution
- **Improved Qwen Code CLI detection** — smoother integration

### 📚 Documentation
Comprehensive review — fixed errors, updated all numbers, filled missing gaps.

### 🔧 Infrastructure & CI/CD
- **GitHub Stats Worker** integration for better analytics
- **Auto-update PR branches** on main push
- **Docker Compose fixes** for admin interface port binding
- **MCP server names** now allow hyphens
- **Release.sh macOS compatibility** — no more grep headaches
- **Cloudflare Pages SPA fallback** — corrected _redirects format
- **PWA manifest & OG image** fixes for better sharing

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
- [GitHub Release](https://github.com/librefang/librefang/releases/tag/v2026.3.2201)
- [GitHub](https://github.com/librefang/librefang)
- [Discord](https://discord.gg/DzTYqAZZmc)
- [Contributing Guide](https://github.com/librefang/librefang/blob/main/docs/CONTRIBUTING.md)
```
