---
title: "LibreFang 0.6.0 Released"
published: true
description: "LibreFang v0.6.0 release notes — open-source Agent OS built in Rust"
tags: rust, ai, opensource, release
canonical_url: https://github.com/librefang/librefang/releases/tag/v0.6.0-20260318
cover_image: https://raw.githubusercontent.com/librefang/librefang/main/public/assets/logo.png
---

# LibreFang 0.6.0 Released

We're thrilled to release **LibreFang v0.6.0**—packed with enterprise-grade features, expanded LLM provider support, and a more powerful API for building intelligent agents. Here's what changed.

## 🧠 Memory & Context Intelligence

LibreFang now ships with **mem0-style proactive memory**, letting agents learn and evolve from past conversations. We've also significantly improved the **context engine**—it's now more accurate, more resilient, and ships with a plugin management system that lets you customize how agents retrieve and reason over context. For advanced use cases, you can register multiple custom plugin registries.

## 🤖 LLM Provider Ecosystem

This release dramatically expands LLM flexibility:

- **NVIDIA NIM** is now a first-class provider—ideal for organizations running local or on-prem inference
- **Vertex AI** joins the lineup with full OAuth2 authentication support
- **Intelligent LLM fallback**: Web search key rotation, data-driven routing, and health-aware LLM selection ensure your agents gracefully degrade if a provider goes down

## 📡 Channels & Real-Time Messaging

Agents can now reach more users through richer channels:

- **Telegram streaming** with progressive message updates (no more "typing..." delays)
- **Multimedia support** for Telegram and Discord—images, video, audio all work seamlessly
- **Improved dispatch reliability**—silent message drops are a thing of the past
- **Sender identity propagation**—channels now forward the original sender's identity to agent context, enabling personalized responses

## ⚙️ Workflows & Automation Overhaul

Workflows are now deeply integrated with your infrastructure:

- **Cron-triggered workflows**: Schedule agents to wake up on a timer and execute tasks automatically
- **Auto-registration at startup**: Local workflow definitions load without manual wiring
- **New workflow REST endpoints**: Full CRUD operations on workflows with `/api/workflows/:id`

## 📊 API Expansion

We added **8 new REST endpoints** to give you finer-grained control:

- `GET /api/providers/:name`, `GET /api/workflows/:id`, `GET /api/channels/:name`, `GET /api/cron/jobs/:id`, `GET /api/mcp_servers/:name`
- `PUT/DELETE /api/workflows/:id` for workflow lifecycle management
- `DELETE /api/agents/:id/files/:filename` for file cleanup
- **Agent list improvements**: filtering, pagination, and sorting for handling thousands of agents

## 🌐 Infrastructure & Developer Experience

- **HTTP proxy support** for all outbound connections (essential for enterprise environments)
- **Docker image now bundles Python and Node.js runtimes**—no more installing dependencies after pulling the image
- **Streaming improvements**: Better handling of chunked responses from thinking models (Gemini's `o1` style)
- **SSL/TLS reliability**: Falls back to bundled Mozilla CA roots if system certificates are unavailable

## 🔧 Bug Fixes & Maintenance

- Fixed Slack thread detection for proper conversation threading
- Corrected MIME type handling for vision models on images
- SHA-256 Nostr pubkey derivation for security compliance
- 10 upstream parity bug fixes to stay in sync with the broader ecosystem
- Full validation of agent list operations

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
- [GitHub Release](https://github.com/librefang/librefang/releases/tag/v0.6.0-20260318)
- [GitHub](https://github.com/librefang/librefang)
- [Discord](https://discord.gg/DzTYqAZZmc)
- [Contributing Guide](https://github.com/librefang/librefang/blob/main/docs/CONTRIBUTING.md)
