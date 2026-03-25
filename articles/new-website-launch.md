---
title: "We just launched the new LibreFang website"
published: true
description: "LibreFang is an open-source Agent OS built in Rust. We redesigned our website from scratch with 7 languages, real-time GitHub stats, and one-command install."
tags: opensource, rust, ai, webdev
canonical_url: https://librefang.ai
cover_image: https://raw.githubusercontent.com/librefang/librefang/main/public/assets/logo.png
---

We just shipped the new [librefang.ai](https://librefang.ai) — a complete redesign of the official website for [LibreFang](https://github.com/librefang/librefang), our open-source Agent Operating System built in Rust.

If you haven't heard of LibreFang: it's a production-grade runtime for autonomous AI agents. Single binary, 180ms cold start, 40MB memory. 15 built-in autonomous capability units ("Hands"), 44 channel adapters, 50 LLM providers. It runs agents 24/7 on a schedule — no user prompts needed.

Here's what we built for the new site.

## 7 languages, day one

The site ships with full translations in English, 简体中文, 繁體中文, 日本語, 한국어, Deutsch, and Español. Not machine-translated placeholders — every string was reviewed for grammar and natural phrasing.

Language detection is automatic based on your browser, and you can switch anytime from the nav.

## Architecture visualized

The homepage walks through LibreFang's five-layer architecture:

- **Channels** — 44 adapters: Telegram, Slack, Discord, WhatsApp, Signal, Matrix, Teams, and more
- **Hands** — 15 autonomous units, each with its own model, tools, and workflow
- **Kernel** — agent lifecycle, workflow orchestration, budget control, scheduling
- **Runtime** — Tokio async, WASM sandbox, Merkle audit chain, SSRF protection
- **Hardware** — runs everywhere: laptop, VPS, Raspberry Pi, bare metal

Each layer is interactive — click to explore the components inside.

## Performance benchmarks, not marketing claims

We put a comparison table right on the homepage:

| Metric | Others | LibreFang |
|--------|--------|-----------|
| Cold Start | 2.5 ~ 4s | **180ms** |
| Idle Memory | 180 ~ 250MB | **40MB** |
| Binary Size | 100 ~ 200MB | **32MB** |
| Security Layers | 2 ~ 3 | **16** |
| Channel Adapters | 8 ~ 15 | **44** |
| Built-in Hands | 0 | **15** |

Rust, not TypeScript. Production, not prototype.

## One command install with OS detection

The install section detects your OS automatically and shows the right command:

**macOS / Linux:**

```bash
curl -fsSL https://librefang.ai/install | sh
```

**Windows (PowerShell):**

```powershell
irm https://librefang.ai/install.ps1 | iex
```

Tabs let you switch between macOS, Windows, and Linux manually too.

## Downloads & changelog

The downloads section pulls release data in real-time through our own API proxy. Desktop apps, CLI binaries, everything categorized by platform.

The changelog page shows a full timeline of releases with:

- Category filters (features, fixes, etc.)
- Download counts per asset
- Auto-linked `#issue` references and `@username` mentions in release notes

## Tech stack

- React + TypeScript + Vite
- Tailwind CSS for styling
- Framer Motion for animations
- TanStack Query for data fetching
- Cloudflare Workers for the GitHub stats proxy

No heavy frameworks. No CMS. Just a fast SPA that gets out of your way.

## What's next

LibreFang itself is moving fast — WhatsApp bidirectional routing just landed, prompt versioning and A/B experiments are in, and we're working toward the v1.0 milestone.

The website will keep evolving with the project. Docs are at [docs.librefang.ai](https://docs.librefang.ai), and the deploy page at [deploy.librefang.ai](https://deploy.librefang.ai) has one-click options for Fly.io and more.

---

If this sounds interesting, check out the repo: **[github.com/librefang/librefang](https://github.com/librefang/librefang)**

We're open source, merge-first PR policy, and all contributions are welcome. Drop a ⭐ if you like what you see.
