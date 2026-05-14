<p align="center">
  <img src="public/assets/logo.png" width="160" alt="BossFang Logo" />
</p>

<h1 align="center">BossFang</h1>
<h3 align="center">Libre Agent Operating System — Free as in Freedom</h3>

<p align="center">
  Open-source Agent OS built in Rust. 24 crates. 2,100+ tests. Zero clippy warnings.
</p>

<p align="center">
  <a href="README.md">English</a> | <a href="i18n/README.zh.md">中文</a> | <a href="i18n/README.ja.md">日本語</a> | <a href="i18n/README.ko.md">한국어</a> | <a href="i18n/README.es.md">Español</a> | <a href="i18n/README.de.md">Deutsch</a> | <a href="i18n/README.pl.md">Polski</a> | <a href="i18n/README.fr.md">Français</a>
</p>

<p align="center">
  <a href="https://github.com/GQAdonis/librefang/">Website</a> &bull;
  <a href="https://github.com/GQAdonis/librefang/blob/main/docs">Docs</a> &bull;
  <a href="CONTRIBUTING.md">Contributing</a> &bull;
  <a href="https://discord.gg/DzTYqAZZmc">Discord</a>
</p>

<p align="center">
  <a href="https://github.com/GQAdonis/librefang/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/librefang/librefang/ci.yml?style=flat-square&label=CI" alt="CI" /></a>
  <img src="https://img.shields.io/badge/language-Rust-orange?style=flat-square" alt="Rust" />
  <img src="https://img.shields.io/badge/license-MIT-blue?style=flat-square" alt="MIT" />
  <img src="https://img.shields.io/github/stars/librefang/librefang?style=flat-square" alt="Stars" />
  <img src="https://img.shields.io/github/v/release/librefang/librefang?style=flat-square" alt="Latest Release" />
  <a href="https://discord.gg/DzTYqAZZmc"><img src="https://img.shields.io/discord/1481633471507071129?style=flat-square&logo=discord&label=Discord" alt="Discord" /></a>
  <a href="https://deepwiki.com/librefang/librefang"><img src="https://deepwiki.com/badge.svg" alt="Ask DeepWiki"></a>
</p>

---

## What is BossFang?

BossFang is an **Agent Operating System** — a full platform for running autonomous AI agents, built from scratch in Rust. Not a chatbot framework, not a Python wrapper.

Traditional agent frameworks wait for you to type something. BossFang runs **agents that work for you** — on schedules, 24/7, monitoring targets, generating leads, managing social media, and reporting to your dashboard.

> BossFang is a community fork of [`RightNow-AI/openfang`](https://github.com/RightNow-AI/openfang) with open governance and a merge-first PR policy. See [GOVERNANCE.md](GOVERNANCE.md) for details.

<p align="center">
  <img src="public/assets/dashboard.png" width="800" alt="BossFang Dashboard" />
</p>

## Quick Start

```bash
# Install (Linux/macOS/WSL)
curl -fsSL https://github.com/GQAdonis/librefang/raw/main/install.sh | sh

# Or install via Cargo
cargo install --git https://github.com/GQAdonis/librefang librefang-cli

# Start — auto-initializes on first run, dashboard at http://localhost:4545
bossfang start

# Or run the setup wizard manually for interactive provider selection
# bossfang init
```

<details>
<summary><strong>Homebrew</strong></summary>

```bash
brew tap librefang/tap
brew install librefang              # CLI (stable)
brew install --cask librefang       # Desktop (stable)
# Beta/RC channels also available:
# brew install librefang-beta       # or librefang-rc
# brew install --cask librefang-rc  # or librefang-beta
```

</details>

<details>
<summary><strong>Docker</strong></summary>

```bash
docker run -p 4545:4545 ghcr.io/librefang/librefang
```

</details>

<details>
<summary><strong>Cloud Deploy</strong></summary>

[![Deploy Hub](https://img.shields.io/badge/Deploy%20Hub-000?style=for-the-badge&logo=rocket)](https://github.com/GQAdonis/librefang/tree/main/deploy) [![Fly.io](https://img.shields.io/badge/Fly.io-purple?style=for-the-badge&logo=fly.io)](https://github.com/GQAdonis/librefang/tree/main/deploy) [![Render](https://img.shields.io/badge/Render-46E3B7?style=for-the-badge&logo=render)](https://render.com/deploy?repo=https://github.com/GQAdonis/librefang) [![Railway](https://img.shields.io/badge/Railway-0B0D0E?style=for-the-badge&logo=railway)](https://railway.app/template/librefang) [![GCP](https://img.shields.io/badge/GCP-4285F4?style=for-the-badge&logo=googlecloud)](deploy/gcp/README.md)

</details>

## Hands: Agents That Work for You

**Hands** are autonomous capability packages that run independently, on schedules, without prompting. Each Hand is defined by a `HAND.toml` manifest, a system prompt, and optional `SKILL.md` files loaded from your configured `hands_dir`.

Example Hand definitions (Researcher, Collector, Predictor, Strategist, Analytics, Trader, Lead, Twitter, Reddit, LinkedIn, Clip, Browser, API Tester, DevOps) are available in the [community hands repository](https://github.com/librefang-registry/hands).

```bash
# Install a community Hand, then:
bossfang hand activate researcher   # Starts working immediately
bossfang hand status researcher     # Check progress
bossfang hand list                  # See all installed Hands
```

Build your own: define a `HAND.toml` + system prompt + `SKILL.md`. [Guide](https://github.com/GQAdonis/librefang/blob/main/docs/agent/skills)

## Architecture

24 Rust crates + xtask, modular kernel design.

```
librefang-kernel            Orchestration, workflows, metering, RBAC, scheduler, budget
librefang-runtime           Agent loop, tool execution, WASM sandbox, MCP, A2A
librefang-api               140+ REST/WS/SSE endpoints, OpenAI-compatible API, dashboard
librefang-channels          45 messaging adapters with rate limiting, DM/group policies
librefang-memory            SQLite persistence, vector embeddings, sessions, compaction
librefang-types             Core types, taint tracking, Ed25519 signing, model catalog
librefang-skills            60 bundled skills, SKILL.md parser, FangHub marketplace
librefang-hands             HAND.toml parser, Hand registry, lifecycle management
librefang-extensions        25 MCP templates, AES-256-GCM vault, OAuth2 PKCE
librefang-wire              OFP P2P protocol, HMAC-SHA256 mutual auth (see note)
librefang-cli               CLI, daemon management, TUI dashboard, MCP server mode
librefang-desktop           Tauri 2.0 native app (tray, notifications, shortcuts)
librefang-migrate           OpenClaw, LangChain, AutoGPT migration engine
librefang-http              Shared HTTP client builder, proxy, TLS fallback
librefang-testing           Test infrastructure: mock kernel, mock LLM driver and API route test utilities
librefang-telemetry         OpenTelemetry + Prometheus metrics instrumentation for BossFang
librefang-llm-driver        LLM driver trait and shared types for BossFang
librefang-llm-drivers       Concrete LLM provider drivers (anthropic, openai, gemini, …) implementing librefang-llm-driver trait
librefang-runtime-mcp       MCP (Model Context Protocol) client for BossFang runtime
librefang-kernel-handle     KernelHandle trait for in-process callers into the BossFang kernel
librefang-runtime-wasm      WASM skill sandbox for BossFang runtime
librefang-kernel-router     Hand/Template routing engine for the BossFang kernel
librefang-runtime-oauth     OAuth flows (ChatGPT, GitHub Copilot) for BossFang runtime drivers
librefang-kernel-metering   Cost metering, quota enforcement for the BossFang kernel
xtask                       Build automation
```

> **OFP wire is plaintext-by-design.** HMAC-SHA256 mutual auth + per-message
> HMAC + nonce replay protection cover *active* attackers, but frame contents
> are not encrypted. For cross-network federation, run OFP behind a private
> overlay (WireGuard, Tailscale, SSH tunnel) or a service-mesh mTLS layer.
> Details: [github.com/GQAdonis/librefang/blob/main/docs/architecture/ofp-wire](https://github.com/GQAdonis/librefang/blob/main/docs/architecture/ofp-wire)

## Key Features

**45 Channel Adapters** — Telegram, Discord, Slack, WhatsApp, Signal, Matrix, Email, Teams, Google Chat, Feishu, LINE, Mastodon, Bluesky, and 32 more. [Full list](https://github.com/GQAdonis/librefang/blob/main/docs/integrations/channels)

**28 LLM Providers** — Anthropic, Gemini, OpenAI, Groq, DeepSeek, OpenRouter, Ollama, Alibaba Coding Plan, and 20 more. Intelligent routing, automatic fallback, cost tracking. [Details](https://github.com/GQAdonis/librefang/blob/main/docs/configuration/providers)

**16 Security Layers** — WASM sandbox, Merkle audit trail, taint tracking, Ed25519 signing, SSRF protection, secret zeroization, and more. [Details](https://github.com/GQAdonis/librefang/blob/main/docs/getting-started/comparison#16-security-systems--defense-in-depth)

**OpenAI-Compatible API** — Drop-in `/v1/chat/completions` endpoint. 140+ REST/WS/SSE endpoints. [API Reference](https://github.com/GQAdonis/librefang/blob/main/docs/integrations/api)

**Client SDKs** — Full REST client with streaming support.

```javascript
// JavaScript/TypeScript
npm install @bossfang/sdk
const { BossFang } = require("@bossfang/sdk");
const client = new BossFang("http://localhost:4545");
const agent = await client.agents.create({ template: "assistant" });
const reply = await client.agents.message(agent.id, "Hello!");
```

```python
# Python
pip install librefang
from librefang import Client
client = Client("http://localhost:4545")
agent = client.agents.create(template="assistant")
reply = client.agents.message(agent["id"], "Hello!")
```

```rust
// Rust
cargo add librefang
use librefang::BossFang;
let client = BossFang::new("http://localhost:4545");
let agent = client.agents().create(CreateAgentRequest { template: Some("assistant".into()), .. }).await?;
```

```go
// Go
go get github.com/GQAdonis/librefang/sdk/go
import "github.com/GQAdonis/librefang/sdk/go"
client := librefang.New("http://localhost:4545")
agent, _ := client.Agents.Create(map[string]interface{}{"template": "assistant"})
```

**MCP Support** — Built-in MCP client and server. Connect to IDEs, extend with custom tools, compose agent pipelines. [Details](https://github.com/GQAdonis/librefang/blob/main/docs/integrations/mcp-a2a)

**A2A Protocol** — Google Agent-to-Agent protocol support. Discover, communicate, and delegate tasks across agent systems. [Details](https://github.com/GQAdonis/librefang/blob/main/docs/integrations/mcp-a2a)

**Desktop App** — Tauri 2.0 native app with system tray, notifications, and global shortcuts.

**OpenClaw Migration** — `librefang migrate --from openclaw` imports agents, history, skills, and config.

## Development

```bash
cargo build --workspace --lib                            # Build
cargo test --workspace                                   # 2,100+ tests
cargo clippy --workspace --all-targets -- -D warnings    # Zero warnings
cargo fmt --all -- --check                               # Format check
```

### Committing changes

Use `scripts/commit.sh` instead of `git commit` directly so staged Rust
files are rustfmt-clean before the pre-commit hook gates them:

```bash
scripts/commit.sh -m "feat: add foo"
scripts/commit.sh -F .git/COMMIT_EDITMSG
```

The wrapper runs `cargo fmt` on staged `*.rs` files, re-stages them, and
holds a soft lock against parallel commits in the same worktree. All flags
are forwarded to `git commit` unchanged. If `cargo` is unavailable the
script skips formatting and warns; the pre-commit hook still gates the
commit.

## Comparison

See [Comparison](https://github.com/GQAdonis/librefang/blob/main/docs/getting-started/comparison#16-security-systems--defense-in-depth) for benchmarks and feature-by-feature comparison vs OpenClaw, ZeroClaw, CrewAI, AutoGen, and LangGraph.

## Links

- [Documentation](https://github.com/GQAdonis/librefang/blob/main/docs) &bull; [API Reference](https://github.com/GQAdonis/librefang/blob/main/docs/integrations/api) &bull; [Getting Started](https://github.com/GQAdonis/librefang/blob/main/docs/getting-started) &bull; [Troubleshooting](https://github.com/GQAdonis/librefang/blob/main/docs/operations/troubleshooting)
- [Contributing](CONTRIBUTING.md) &bull; [Governance](GOVERNANCE.md) &bull; [Security](SECURITY.md)
- Discussions: [Q&A](https://github.com/GQAdonis/librefang/discussions/categories/q-a) &bull; [Use Cases](https://github.com/GQAdonis/librefang/discussions/categories/show-and-tell) &bull; [Feature Votes](https://github.com/GQAdonis/librefang/discussions/categories/ideas) &bull; [Announcements](https://github.com/GQAdonis/librefang/discussions/categories/announcements) &bull; [Discord](https://discord.gg/DzTYqAZZmc)

## Contributors

<a href="https://github.com/GQAdonis/librefang/graphs/contributors">
  <img src="web/public/assets/contributors.svg" alt="Contributors" />
</a>

<p align="center">
  We welcome contributions of all kinds — code, docs, translations, bug reports.<br/>
  Check the <a href="CONTRIBUTING.md">Contributing Guide</a> and pick a <a href="https://github.com/GQAdonis/librefang/issues?q=is%3Aissue+is%3Aopen+label%3A%22good+first+issue%22">good first issue</a> to get started!<br/>
  You can also visit the <a href="https://leszek3737.github.io/librefang-WIki/">unofficial wiki</a>, which is updated with helpful information for new contributors.
</p>

<p align="center">
  <a href="https://github.com/GQAdonis/librefang/stargazers">
    <img src="web/public/assets/star-history.svg" alt="Star History" />
  </a>
</p>

---

<p align="center">MIT License</p>
