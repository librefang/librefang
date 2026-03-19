```markdown
---
title: "LibreFang 0.6.1 Released"
published: true
description: "LibreFang v0.6.1 release notes — open-source Agent OS built in Rust"
tags: rust, ai, opensource, release
canonical_url: https://github.com/librefang/librefang/releases/tag/v0.6.1-20260318
cover_image: https://raw.githubusercontent.com/librefang/librefang/main/public/assets/logo.png
---

# LibreFang 0.6.1 Released

LibreFang 0.6.1 is here! This release focuses on **polishing the CLI and API layers** with numerous fixes for real-world integration scenarios. Whether you're spinning up agents dynamically, managing budgets through the API, or deploying without an LLM provider configured, we've got you covered.

## What's New

**Graceful Degradation for Missing LLM Providers** (#1185)  
LibreFang now handles the case where no LLM provider is configured—perfect for testing, local development, or air-gapped environments.

## What We Fixed

### CLI: Agent Name Resolution & Command Parsing
The CLI now correctly resolves agent names to UUIDs across all commands, making it much more user-friendly:
- **Agent message & kill commands** now properly resolve agent names (#1170)
- **Trigger, cron, and webhook commands** parse agent names correctly (#1176)
- **Wrapped API responses** are now handled in CLI table views (#1175)
- **Cron list parsing** correctly reads nested schedule/action fields (#1186)

### API Endpoints: Consistent Responses
Standardized API response formats and fixed critical data inconsistencies:
- **Duplicate agent spawning** now returns `409 Conflict` (#1171)
- **Agent detail endpoint** includes `last_active` timestamp (#1173)
- **Cron creation** returns proper JSON (not stringified blobs) (#1184)
- **Triggers list** returns a consistently wrapped object (#1187)
- **Agent detail** now includes the `system_prompt` in responses (#1188)

### Data Handling: Parsing & Configuration
Fixed subtle bugs in model alias parsing and config updates:
- **Model aliases** now parse correctly from API responses (#1172)
- **Budget updates** accept GET response field names for read-modify-write workflows (#1182)
- **Config/set API** sends the correct field name for model changes (#1183)
- **Paginated responses** in agent lists and chat resolver are handled properly (#1169)

### Dashboard & UI
- **Internationalization** coverage is now complete across the dashboard (#1177)
- **A2A agent cards** now use service config instead of picking random agents (#1179)

### Security & Documentation
- **MongoDB example URIs** no longer expose credentials in docs (#1168)
- **Dev.to article markdown** fence wrappers removed for cleaner rendering (#1167)

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
- [GitHub Release](https://github.com/librefang/librefang/releases/tag/v0.6.1-20260318)
- [GitHub](https://github.com/librefang/librefang)
- [Discord](https://discord.gg/DzTYqAZZmc)
- [Contributing Guide](https://github.com/librefang/librefang/blob/main/docs/CONTRIBUTING.md)
```
