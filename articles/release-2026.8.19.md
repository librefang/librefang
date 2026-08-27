---
title: "LibreFang 2026.8.19 Released"
published: true
description: "LibreFang v2026.8.19 release notes — open-source Agent OS built in Rust"
tags: rust, ai, opensource, release
canonical_url: https://github.com/librefang/librefang/releases/tag/v2026.8.19
cover_image: https://raw.githubusercontent.com/librefang/librefang/main/public/assets/logo.png
---

# LibreFang 2026.8.19 Released

**474 PRs from 5 contributors since v2026.7.31.**

This release is a massive stability and security push. We've closed dozens of data-loss vectors, locked down SSRF, hardened atomic file writes throughout the daemon, and fixed a ton of edge cases that were silently failing or corrupting state. On the feature side: managed configuration mode for self-hosted deployments, long-form audio transcription that doesn't drop your 20-minute recordings, and Polish language support. Performance-wise we've moved a bunch of blocking I/O off async workers and bolted on some overdue mutex poison recovery so a single panic doesn't disable half the daemon.

## What's New

### Security & Reliability

This release closes a lot of holes. We've sealed path-traversal in skill/channel IDs, fixed SSRF vectors in link context and webhook callbacks, escaped untrusted data in OAuth responses, and re-locked the auth scoping so non-owners can't probe other agents' sessions. On the data side: we now fsync parent directories after atomic file writes, serialize concurrent writes to shared files (e.g. secrets.env), and validate every SQLite row decode so corruption surfaces instead of silently dropping data. Credentials get redacted consistently before export, and the desktop app no longer bypasses shell security when uninstalling.

**Most impactful:** we've recovered poison recovery for ~40 locks throughout the daemon. A single panic in a critical section used to permanently disable feature areas (metering, routing, audit, skills, memory) for the entire process lifetime. Now they recover and log what went wrong, so one bad turn doesn't ghost you.

### Managed Configuration

New `LIBREFANG_CONFIG_MODE=managed` lets you own `config.toml` instead of the daemon. Set it in your environment, and every API route that would mutate config answers `423 Locked`. Works great with Kubernetes ConfigMaps or read-only mounts. The API now also reports the config's SHA-256, write status, and last-modified time so your dashboard can render it read-only server-side. Plus custom providers can opt into live model discovery if you're running self-hosted Ollama or vLLM and want the model list to refresh automatically.

### Long-Form Audio & Video

`media_transcribe` can now process recordings in windowed chunks and write to a file. Your 30-minute recording won't drop anymore — it was hitting the spill threshold and the tool-result size cap together. Windows also get deduplicated on their Ogg granule position so you don't lose audio at boundaries. Transcription of `.mp4` and `.mov` now works (they need seeking, so we stage to disk first).

### Async Agent Messaging & Task Waking

`agent_send` is now non-blocking by default — returns a task ID you can poll. Previous default made you predict whether a delegation would be slow and opt in to `async: true`, which was a coin flip. `false` only when you need the answer in-turn.

Tasks assigned to agents now wake them automatically, no manual trigger required. If nobody's listening to `task_posted` events, the task still reaches its owner via a synthesized wake. Dead triggers can no longer shadow real work.

### Localization Fixes

Polish (`pl`) is now a first-class supported language. We also restored missing diacritics in Spanish and French (`válido`, `déjà`, `¿agente no encontrado?`, etc.) and added missing Japanese error keys.

### Performance

A lot of blocking filesystem and database work has moved off Tokio's async workers. Config reads/writes, SQLite queries, WASM module loads, and PDF extraction now run on the blocking pool so they don't park your request handler. Desktop app SPA installs, agent-template discovery, and Chromium binary probing also got offloaded. We also offloaded the image vulnerability scanner (Trivy) to run on every build — no user-facing gate yet (waiting on remediation), but full JSON/SARIF artifacts are retained for CI review.

### Bug Fixes

- **Link extraction** wasn't recursing into list items, and the page-selector was picking the first `<article>` instead of the container holding multiple articles. Fixed.
- **Windows tests** got disabled by a desktop app linking failure. Re-enabled with a workaround, keeping the compile+link coverage.
- **Slack messages** with lots of whitespace rendered as walls of blanks; fixed. Also fixed sections overshooting the 3000-character limit and getting dropped entirely.
- **Trigger cooldown** was per-trigger instead of per-subject, so a second distinct thing happening fast got discarded. Now scoped correctly.
- **Image generation** against OpenAI's `gpt-image-*` models failed on `response_format` rejection. Removed it for that family.
- **`browser_read_page`** now deduplicates links and renders them as `⟨n⟩` markers + a separate table, cutting link payload 70–84% on dense pages and keeping markers clickable.

Grab the [full list](https://github.com/librefang/librefang/releases/tag/v2026.8.19) if you want the gory details.

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
- [GitHub Release](https://github.com/librefang/librefang/releases/tag/v2026.8.19)
- [GitHub](https://github.com/librefang/librefang)
- [Discord](https://discord.gg/DzTYqAZZmc)
- [Contributing Guide](https://github.com/librefang/librefang/blob/main/docs/CONTRIBUTING.md)
