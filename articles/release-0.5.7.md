```markdown
---
title: "LibreFang 0.5.7 Released"
published: true
description: "LibreFang v0.5.7 release notes — open-source Agent OS built in Rust"
tags: rust, ai, opensource, release
canonical_url: https://github.com/librefang/librefang/releases/tag/v0.5.7-20260318
cover_image: https://raw.githubusercontent.com/librefang/librefang/main/public/assets/logo.png
---

# LibreFang 0.5.7 Released

We're thrilled to ship **v0.5.7** — a release packed with production-critical reliability features, powerful new APIs, and deep multi-agent orchestration foundations.

## 🎯 Highlights

### Production Reliability: Multi-Token Fallback & Quota Rotation

The standout feature this release is **transparent multi-token fallback with automatic quota rotation** (#441). If your primary API key hits rate limits or quota, LibreFang now seamlessly rotates to backup tokens without your code needing to know about it. Perfect for scaling production deployments without downtime.

### API Ecosystem Expansion

We've landed a complete event-driven architecture layer:
- **Event webhooks** (#394) — React to system events in real-time
- **New REST endpoints** — Query integrations, approvals, and sessions individually (#1088, #1089, #1096)
- **Task queue management API** (#395) — Full programmatic control over task lifecycle
- **Agent monitoring & metrics** (#396) — Deep visibility into agent performance and resource usage
- **Input validation middleware** (#398) — Automatic request validation across all endpoints
- **Webhooks management** (#400) — Configure delivery, retries, and filtering

### Multi-Agent Orchestration Foundation

We've laid the groundwork for coordinated multi-agent systems with new orchestration primitives (#323, #437) — future releases will build on this to enable agent-to-agent communication patterns.

### Integrations & LLM Enhancements

- **Telegram streaming** — Agents now progressively type responses in real-time, making interactions feel snappier (fixes #317)
- **Feishu approval flows** — Interactive cards for agent permission requests, bringing approval workflows to Feishu users (#439)
- **Google Chat JWT auth** — Service account authentication for more secure integrations (#443)
- **Agent cloning** — Export agent configs with skills and tools included (#366)
- **6 new hand-crafted templates** (#413) — More starter patterns for common agent architectures

## Bug Fixes & Quality

- Fixed YAML parsing in auto-posted release articles (#567)
- MCP subprocesses now inherit parent environment (better plugin compatibility) (#1098)
- Parameterless tool calls now send empty objects instead of null, fixing downstream serialization (#1108)
- TokenUsage fields properly included in token rotation tests (#1114)

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
- [GitHub Release](https://github.com/librefang/librefang/releases/tag/v0.5.7-20260318)
- [GitHub](https://github.com/librefang/librefang)
- [Discord](https://discord.gg/DzTYqAZZmc)
- [Contributing Guide](https://github.com/librefang/librefang/blob/main/CONTRIBUTING.md)
```
