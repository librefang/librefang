---
title: "Hello, LibreFang! — The Open-Source Agent Operating System"
emoji: "🦊"
type: "tech"
topics: ["rust", "ai", "agent", "opensource", "librefang"]
published: true
---

# Hello, LibreFang!

LibreFang is an open-source Agent Operating System built in Rust.

## Why LibreFang?

"Libre" means freedom. We believe an open-source project should be truly open — not just in license, but in governance, contribution, and collaboration.

## Features

- **14 Rust crates** — modular, fast, and memory-safe
- **40 messaging channels** — Telegram, Discord, Slack, and 37 more
- **60 bundled skills** — ready to use out of the box
- **16 security layers** — WASM sandbox, RBAC, audit trails, and more
- **2,100+ tests** — zero clippy warnings

## Quick Start

```bash
export GROQ_API_KEY="your-key"
librefang init && librefang start
# Open http://127.0.0.1:4545
```

## Contributing

You don't need Rust experience to contribute:

| Type | Rust required? |
|------|---------------|
| Agent templates (TOML) | No |
| Skills (Python/JS) | No |
| Documentation / Translation | No |
| Channel adapters | Yes |
| Core features | Yes |

## Links

- [GitHub](https://github.com/librefang/librefang)
- [Discord](https://discord.gg/DzTYqAZZmc)
- [Contributing Guide](https://github.com/librefang/librefang/blob/main/CONTRIBUTING.md)
- [Good First Issues](https://github.com/librefang/librefang/labels/good%20first%20issue)
