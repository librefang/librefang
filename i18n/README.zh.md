<p align="center">
  <img src="../public/assets/logo.png" width="160" alt="LibreFang Logo" />
</p>

<h1 align="center">LibreFang</h1>
<h3 align="center">自由的 Agent 操作系统 — Libre 意味着自由</h3>

<p align="center">
  使用 Rust 构建的开源 Agent OS。24 个 crate。2,100+ 测试。零 clippy 警告。
</p>

<p align="center">
  <a href="../README.md">English</a> | <a href="README.zh.md">中文</a> | <a href="README.ja.md">日本語</a> | <a href="README.ko.md">한국어</a> | <a href="README.es.md">Español</a> | <a href="README.de.md">Deutsch</a> | <a href="README.pl.md">Polski</a> | <a href="README.fr.md">Français</a>
</p>

<p align="center">
  <a href="https://librefang.ai/">网站</a> &bull;
  <a href="https://docs.librefang.ai">文档</a> &bull;
  <a href="../CONTRIBUTING.md">贡献</a> &bull;
  <a href="https://discord.gg/DzTYqAZZmc">Discord</a>
</p>

<p align="center">
  <a href="https://github.com/librefang/librefang/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/librefang/librefang/ci.yml?style=flat-square&label=CI" alt="CI" /></a>
  <img src="https://img.shields.io/badge/language-Rust-orange?style=flat-square" alt="Rust" />
  <img src="https://img.shields.io/badge/license-MIT-blue?style=flat-square" alt="MIT" />
  <img src="https://img.shields.io/github/stars/librefang/librefang?style=flat-square" alt="Stars" />
  <img src="https://img.shields.io/github/v/release/librefang/librefang?style=flat-square" alt="Latest Release" />
  <a href="https://discord.gg/DzTYqAZZmc"><img src="https://img.shields.io/discord/1481633471507071129?style=flat-square&logo=discord&label=Discord" alt="Discord" /></a>
  <a href="https://deepwiki.com/librefang/librefang"><img src="https://deepwiki.com/badge.svg" alt="Ask DeepWiki"></a>
</p>

---

## 什么是 LibreFang？

LibreFang 是一个 **Agent 操作系统** — 用 Rust 从头构建的完整自主 AI 智能体运行平台。不是聊天机器人框架，不是 Python 包装器。

传统智能体框架等待你的输入。LibreFang 运行**为你工作的智能体** — 按计划全天候运行，监控目标、生成线索、管理社交媒体，并向控制台报告。

> LibreFang 是 [`RightNow-AI/openfang`](https://github.com/RightNow-AI/openfang) 的社区分支，采用开放治理和合并优先的 PR 政策。详见 [GOVERNANCE.md](../GOVERNANCE.md)。

<p align="center">
  <img src="../public/assets/dashboard.png" width="800" alt="LibreFang 控制台" />
</p>

## 快速开始

```bash
# 安装 (Linux/macOS/WSL)
curl -fsSL https://librefang.ai/install.sh | sh

# 或通过 Cargo 安装
cargo install --git https://github.com/librefang/librefang librefang-cli

# 启动 — 首次运行时自动初始化，仪表盘位于 http://localhost:4545
librefang start

# 或者手动运行设置向导以进行交互式的提供商选择
# librefang init
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
<summary><strong>云部署</strong></summary>

[![Deploy Hub](https://img.shields.io/badge/Deploy%20Hub-000?style=for-the-badge&logo=rocket)](https://deploy.librefang.ai) [![Fly.io](https://img.shields.io/badge/Fly.io-purple?style=for-the-badge&logo=fly.io)](https://deploy.librefang.ai) [![Render](https://img.shields.io/badge/Render-46E3B7?style=for-the-badge&logo=render)](https://render.com/deploy?repo=https://github.com/librefang/librefang) [![Railway](https://img.shields.io/badge/Railway-0B0D0E?style=for-the-badge&logo=railway)](https://railway.app/template/librefang) [![GCP](https://img.shields.io/badge/GCP-4285F4?style=for-the-badge&logo=googlecloud)](../deploy/gcp/README.md)

</details>

## Hands：为你工作的智能体

**Hands** 是一种自主的能力包，无需输入提示词即可按计划独立运行。每个 Hand 由一个 `HAND.toml` 清单、一个系统提示词（system prompt）以及从您配置的环境中加载的可选 `SKILL.md` 文件来定义 `hands_dir`。

Hand 的定义示例（Researcher, Collector, Predictor, Strategist, Analytics, Trader, Lead, Twitter, Reddit, LinkedIn, Clip, Browser, API Tester, DevOps）可以在[社区的 Hands 仓库](https://github.com/librefang-registry/hands)中找到。

```bash
# 安装一个社区 Hand，然后：
librefang hand activate researcher   # 立即开始工作
librefang hand status researcher     # 查看进度
librefang hand list                  # 查看所有 Hands
```

自定义 Hand：定义 `HAND.toml` + 系统提示词 + `SKILL.md`。[指南](https://docs.librefang.ai/agent/skills)

## 架构

24 个 Rust crate + xtask，模块化内核设计。

```
librefang-kernel            编排、工作流、计量、RBAC、调度、预算
librefang-runtime           智能体循环、工具执行、WASM 沙箱、MCP、A2A
librefang-api               140+ REST/WS/SSE 端点、OpenAI 兼容 API、控制台
librefang-channels          45 个消息适配器，速率限制、DM/群组策略
librefang-memory            SQLite 持久化、向量嵌入、会话、压缩
librefang-types             核心类型、污点追踪、Ed25519 签名、模型目录
librefang-skills            60 个内置技能、SKILL.md 解析器、FangHub 市场
librefang-hands             个自主 Hands、HAND.toml 解析器、生命周期管理
librefang-extensions        25 个 MCP 模板、AES-256-GCM 保险库、OAuth2 PKCE
librefang-wire              OFP P2P 协议、HMAC-SHA256 双向认证
librefang-cli               CLI、守护进程管理、TUI 控制台、MCP 服务器模式
librefang-desktop           Tauri 2.0 原生应用（托盘、通知、快捷键）
librefang-migrate           OpenClaw、LangChain、AutoGPT 迁移引擎
librefang-http              共享 HTTP 客户端构建器、代理、TLS 回退（fallback）
librefang-testing           测试基础设施：模拟（mock）内核、模拟 LLM 驱动程序以及 API 路由测试工具
librefang-telemetry         的 OpenTelemetry + Prometheus 指标埋点（instrumentation）
librefang-llm-driver        的 LLM 驱动程序 trait 及共享类型
librefang-llm-drivers       trait 的具体 LLM 提供商驱动程序（anthropic、openai、gemini 等）
librefang-runtime-mcp       运行时的 MCP（Model Context Protocol）客户端
librefang-kernel-handle     内核进行进程内调用的 KernelHandle trait
librefang-kernel-router     内核的 Hand/Template 路由引擎
librefang-kernel-metering   内核的成本计量和配额执行
xtask                       构建自动化
```
> **OFP wire 是 plaintext-by-design。** HMAC-SHA256 双向认证 + 每条消息的
> HMAC + nonce 重放保护涵盖了针对 *主动* 攻击者的防御，但帧的内容
> 未经加密。对于跨网络联邦，请在私有
> 覆盖网络（如 WireGuard、Tailscale、SSH 隧道）或服务网格 mTLS 层之后运行 OFP。
> 详情: [docs.librefang.ai/architecture/ofp-wire](https://docs.librefang.ai/architecture/ofp-wire)

## 核心特性

**45 个渠道适配器** — Telegram、Discord、Slack、WhatsApp、Signal、Matrix、Email、Teams、Google Chat、飞书、LINE、Mastodon、Bluesky 等。[完整列表](https://docs.librefang.ai/integrations/channels)

**28 个 LLM 服务商** — Anthropic、Gemini、OpenAI、Groq、DeepSeek、OpenRouter、Ollama 等。智能路由、自动回退、成本追踪。[详情](https://docs.librefang.ai/configuration/providers)

**16 层安全体系** — WASM 沙箱、Merkle 审计链、污点追踪、Ed25519 签名、SSRF 防护、密钥清零等。[详情](https://docs.librefang.ai/getting-started/comparison#16-security-systems--defense-in-depth)

**OpenAI 兼容 API** — 即插即用的 `/v1/chat/completions` 端点。140+ REST/WS/SSE 端点。[API 参考](https://docs.librefang.ai/integrations/api)

**客户端 SDK** — 完整 REST 客户端，支持流式传输。

```javascript
// JavaScript/TypeScript
npm install @librefang/sdk
const { LibreFang } = require("@librefang/sdk");
const client = new LibreFang("http://localhost:4545");
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
use librefang::LibreFang;
let client = LibreFang::new("http://localhost:4545");
let agent = client.agents().create(CreateAgentRequest { template: Some("assistant".into()), .. }).await?;
```

```go
// Go
go get github.com/librefang/librefang/sdk/go
import "github.com/librefang/librefang/sdk/go"
client := librefang.New("http://localhost:4545")
agent, _ := client.Agents.Create(map[string]interface{}{"template": "assistant"})
```

**MCP 支持** — 内置 MCP 客户端和服务器。连接 IDE、扩展自定义工具、组合智能体管道。[详情](https://docs.librefang.ai/integrations/mcp-a2a)

**A2A 协议** — 支持 Google Agent-to-Agent 协议。跨智能体系统发现、通信和任务委派。[详情](https://docs.librefang.ai/integrations/mcp-a2a)

**桌面应用** — Tauri 2.0 原生应用，支持系统托盘、通知和全局快捷键。

**OpenClaw 迁移** — `librefang migrate --from openclaw` 导入智能体、历史、技能和配置。

## 开发

```bash
cargo build --workspace --lib                            # 构建
cargo test --workspace                                   # 2,100+ 测试
cargo clippy --workspace --all-targets -- -D warnings    # 零警告
cargo fmt --all -- --check                               # 格式化检查
```

## 对比

查看 [对比](https://docs.librefang.ai/getting-started/comparison#16-security-systems--defense-in-depth) 了解 LibreFang 与 OpenClaw、ZeroClaw、CrewAI、AutoGen、LangGraph 的基准测试和功能对比。

## 中文文档

- [技能开发指南（中文）](skill-development.zh.md) — 技能格式、Python/WASM 运行时、FangHub 发布与 CLI 管理
- [官方文档](https://docs.librefang.ai)（英文）

## 链接

- [文档](https://docs.librefang.ai) &bull; [API 参考](https://docs.librefang.ai/integrations/api) &bull; [入门指南](https://docs.librefang.ai/getting-started) &bull; [故障排除](https://docs.librefang.ai/operations/troubleshooting)
- [贡献](../CONTRIBUTING.md) &bull; [治理](../GOVERNANCE.md) &bull; [安全](../SECURITY.md)
- 讨论: [问答](https://github.com/librefang/librefang/discussions/categories/q-a) &bull; [用例展示](https://github.com/librefang/librefang/discussions/categories/show-and-tell) &bull; [功能投票](https://github.com/librefang/librefang/discussions/categories/ideas) &bull; [公告](https://github.com/librefang/librefang/discussions/categories/announcements) &bull; [Discord](https://discord.gg/DzTYqAZZmc)

## 贡献者

<a href="https://github.com/librefang/librefang/graphs/contributors">
  <img src="../web/public/assets/contributors.svg" alt="Contributors" />
</a>

<p align="center">
  我们欢迎各种形式的贡献 — 代码、文档、翻译、Bug 报告。<br/>
  查看 <a href="../CONTRIBUTING.md">贡献指南</a>，从一个 <a href="https://github.com/librefang/librefang/issues?q=is%3Aissue+is%3Aopen+label%3A%22good+first+issue%22">good first issue</a> 开始吧！<br/>
  您也可以访问 <a href="https://leszek3737.github.io/librefang-WIki/">非官方 wiki</a>，其中更新了面向新贡献者的有用信息。
</p>

<p align="center">
  <a href="https://github.com/librefang/librefang/stargazers">
    <img src="../web/public/assets/star-history.svg" alt="Star History" />
  </a>
</p>

---

<p align="center">MIT 许可证</p>
