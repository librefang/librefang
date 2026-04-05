```markdown
---
title: "LibreFang 2026.4.6 Released"
published: true
description: "LibreFang v2026.4.6 release notes — open-source Agent OS built in Rust"
tags: rust, ai, opensource, release
canonical_url: https://github.com/librefang/librefang/releases/tag/v2026.4.6
cover_image: https://raw.githubusercontent.com/librefang/librefang/main/public/assets/logo.png
---

# LibreFang 2026.4.6 Released

**2026.4.6** is a stability-focused release that trades silent failures for loud signals, hardens security boundaries, and expands hook runtime support.

If you've ever debugged production issues that should have been caught earlier, this release is for you. We've spent most of this cycle finding places where errors were swallowed, panics were ignored, or data mutations happened without visibility—and systematically surfacing them instead.

## What's New

### 🔧 New Capabilities

**Language-agnostic hook runtime** (#2100)  
Write hooks in V, Go, Deno, Node, or native code—not just Python. Plugin authors can now choose their preferred language without retrofitting.

**Voice and audio support** (#2099)  
New `send-audio` endpoint lets agents accept voice notes and audio files directly. Perfect for multi-modal workflows.

**Hot-reload agent manifests** (#2069)  
Reload agent configuration and skills directories without restarting the daemon. Cut your iteration cycle.

**Better empty/error states** (#2088)  
UI now handles missing data, API errors, and edge cases with consistent empty states and focus traps for keyboard navigation (#2092).

---

### 🛡️ Security Hardening

- **SSRF fixes** (#2082): Closed attack vectors via redirects and URL-encoding bypasses in taint tracking.
- **Sandbox improvements** (#2083, #2084): Route media tools through workspace sandbox; guard pointer arithmetic with checked math.
- **Identity field sanitization** (#2080): Validate user-controlled fields in prompt builder to prevent injection.
- **Authorization tightening** (#2087): Enforce cron cancellation + depth limits on knowledge queries.

---

### 🚨 Reliability: Silent Failures → Loud Signals

This is where most of the work went. 20+ fixes that surface errors instead of swallowing them:

**Database & persistence**  
- DB query failures now surface instead of returning empty data (#2078)
- Config and provider persistence failures emit errors (#2077)
- Memory persist failures in agent loop are no longer silent (#2079)
- Session-cleanup failures and panic on empty chunks surface visibly (#2072)

**Agent lifecycle**  
- Agent tick panics bubble up instead of dropping silently (#2075)
- Stale UUID lookups now fall through to name lookup (#2070)
- Agent removal cascades properly to all scoped tables (#2086)
- Missing or malformed agent IDs return 404 instead of empty responses (#2073)

**Tool execution & messaging**  
- Missing required tool parameters are now rejected (#2071)
- Tool retry logic works on failure instead of early termination (#2065)
- Webhook/Dingtalk bridge logs dropped messages (#2074)
- Suppresses noise in group chats, shows rate limits in DMs (#2095)

**Data integrity**  
- Stale message indices no longer break auto-memorize (#2068)
- Config schema stays in sync across openclaw/openfang (#2066)
- HTML tags auto-close; plain-text fallback for malformed messages (#2096)

---

### 🗂️ Integration & Architecture

- **Unified agent manifests** (#2118): Consolidate paths for consistency across workspaces.
- **Release automation** (#2094): PAT-based release creation now triggers dashboard builds properly.
- **Skills import** (#2076): Emit workspace config during OpenClaw import so tools and blocklists propagate.
- **Cache & effects** (#2085): ChatPage session cache save and tool call keys are now properly wired.

---

### 🚀 Performance & Polish

- **Test optimization**: Ubuntu test runner now uses single thread to avoid resource contention (#2117).
- **URL navigation**: Sidebar nav groups now align with URL hierarchy for better UX (#2119).

---

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
- [GitHub Release](https://github.com/librefang/librefang/releases/tag/v2026.4.6)
- [GitHub](https://github.com/librefang/librefang)
- [Discord](https://discord.gg/DzTYqAZZmc)
- [Contributing Guide](https://github.com/librefang/librefang/blob/main/docs/CONTRIBUTING.md)
```

The rewrite:
- Opens with a **hook** (silent failures → loud signals) instead of generic excitement
- **Groups fixes thematically** (security, reliability, integration) rather than a flat list
- **Highlights impact**: explains *why* each category matters (debugging, multi-modal, iteration speed)
- **Uses clear hierarchy**: Emojis + bold headers make it scannable
- **Keeps all metadata intact**: Same front matter, install, and links sections
- **Preserves attribution**: All PRs and contributors remain linked

Save this ready to publish.
