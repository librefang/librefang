<p align="center">
  <img src="public/assets/logo.png" width="160" alt="LibreFang Logo" />
</p>

<h1 align="center">LibreFang</h1>
<h3 align="center">社区维护的 Agent 操作系统</h3>

<p align="center">
  使用 Rust 编写的开源 Agent OS。137K 代码行。14 个 crate。1767+ 测试。零 clippy 警告。<br/>
  <strong>派生自 `RightNow-AI/openfang`。透明治理。保留署名。与现有 `librefang` CLI 兼容。</strong>
</p>

<p align="center">
  <strong>多语言版本：</strong> <a href="README.md">English</a> | <a href="README.zh.md">中文</a> | <a href="README.ja.md">日本語</a> | <a href="README.ko.md">한국어</a> | <a href="README.es.md">Español</a> | <a href="README.de.md">Deutsch</a>
</p>

<p align="center">
  <a href="https://librefang.ai/">网站</a> &bull;
  <a href="https://github.com/librefang/librefang">GitHub</a> &bull;
  <a href="GOVERNANCE.md">治理</a> &bull;
  <a href="CONTRIBUTING.md">贡献</a> &bull;
  <a href="SECURITY.md">安全</a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/language-Rust-orange?style=flat-square" alt="Rust" />
  <img src="https://img.shields.io/badge/license-MIT-blue?style=flat-square" alt="MIT" />
  <img src="https://img.shields.io/badge/community-maintained-brightgreen?style=flat-square" alt="社区维护" />
  <img src="https://img.shields.io/github/stars/librefang/librefang?style=flat-square" alt="Stars" />
  <img src="https://img.shields.io/github/forks/librefang/librefang?style=flat-square" alt="Forks" />
</p>

---

> **LibreFang 是 [`RightNow-AI/openfang`](https://github.com/RightNow-AI/openfang) 的社区维护分支。**
>
> 代码库、二进制文件、crate 名称和配置路径已使用 `librefang`。LibreFang 采用不同的项目治理方式：我们积极接受社区贡献，在公开场合审查它们，通过正常的 GitHub 流程合并被接受的工作，并在改编代码时保留贡献者署名。

> **社区状态：** 欢迎提交 issue、PR、reviewer 和维护者。

---

## 什么是 LibreFang？

LibreFang 是一个**开源 Agent 操作系统**——不是聊天机器人框架，不是围绕 LLM 的 Python 包装器，也不是"多智能体编排器"。它是一个为自主智能体构建的完整操作系统，使用 Rust 从头构建并公开维护。

传统的智能体框架等待你输入内容。LibreFang 运行**为你工作的自主智能体**——按计划运行，7x24 小时，构建知识图谱、监控目标、生成潜在客户、管理你的社交媒体，并向你的仪表板报告结果。

项目网站已在 [librefang.ai](https://librefang.ai/) 上线。今天，最快的试用 LibreFang 的方式仍然是从源码安装。

```bash
cargo install --git https://github.com/librefang/librefang librefang-cli
librefang init
librefang start
# 仪表板地址：http://localhost:4545
```

**或者使用 Homebrew 安装：**
```bash
brew tap librefang/tap
brew install librefang
```

---

## 核心特性

### 🤖 Hands：真正做事的智能体

*"传统智能体等待你输入。Hands 为你工作。"*

**Hands** 是 LibreFang 的核心创新——预构建的自主能力包，独立运行，按计划执行，无需你提示。它不是聊天机器人。这是一个在早上 6 点醒来的智能体，研究你的竞争对手，构建知识图谱，对发现进行评分，在你有咖啡之前将报告发送到你的 Telegram。

每个 Hand 包含：
- **HAND.toml** — 声明工具、设置、要求和仪表板指标的清单
- **System Prompt** — 多阶段操作手册（不是一句话——这些是 500+ 字的专家程序）
- **SKILL.md** — 运行时注入上下文的领域专业知识参考
- **Guardrails** — 敏感操作的批准门（例如，Browser Hand 需要在任何购买前获得批准）

全部编译进二进制文件。无需下载，无需 pip install，无需 Docker pull。

### 7 个内置 Hands

| Hand | 功能 |
|------|------|
| **Clip** | 获取 YouTube URL，下载，识别最佳时刻，裁剪成带字幕和缩略图的短视频，可选添加 AI 配音，发布到 Telegram 和 WhatsApp。8 阶段管道。FFmpeg + yt-dlp + 5 个 STT 后端。 |
| **Lead** | 每日运行。发现符合你的 ICP 的潜在客户，用网络研究丰富它们，评分 0-100，与现有数据库去重，以 CSV/JSON/Markdown 交付合格线索。随着时间推移构建 ICP 档案。 |
| **Collector** | OSINT 级情报。你给一个目标（公司、人、主题）。它持续监控——变化检测、情感追踪、知识图谱构建，在重要变化时发送关键警报。 |
| **Predictor** | 超级预测引擎。从多个来源收集信号，构建校准的推理链，用置信区间进行预测，使用 Brier 分数跟踪自己的准确性。有反向模式——故意与共识争辩。 |
| **Researcher** | 深度自主研究员。交叉引用多个来源，使用 CRAAP 标准评估可信度（货币性、相关性、权威性、准确性、目的），生成带引用的 APA 格式报告，支持多语言。 |
| **Twitter** | 自主 Twitter/X 账户管理器。以 7 种轮换格式创建内容，为最佳参与度安排帖子，回复提及，跟踪绩效指标。有批准队列——未经你确认 nothing posts。 |
| **Browser** | Web 自动化智能体。导航网站，填写表单，点击按钮，处理多步骤工作流。使用 Playwright 桥接和会话持久化。**强制购买批准门**——未经明确确认永远不会花你的钱。 |

---

## 16 层安全系统 — 纵深防御

LibreFang 不是事后才添加安全。每一层都是独立可测试的，无单点故障运行。

| # | 系统 | 功能 |
|---|------|------|
| 1 | **WASM 双重计量沙箱** | 工具代码在 WebAssembly 中运行，带燃料计量 + epoch 中断。看门狗线程杀死失控代码。 |
| 2 | **Merkle 哈希链审计追踪** | 每个操作都加密链接到前一个。篡改一条记录整个链就断裂。 |
| 3 | **信息流污染追踪** | 标签在执行中传播——从源到汇跟踪 secrets。 |
| 4 | **Ed25519 签名智能体清单** | 每个智能体身份和能力集都是加密签名的。 |
| 5 | **SSRF 保护** | 阻止私有 IP、云元数据端点和 DNS 重新绑定攻击。 |
| 6 | **Secret 零化** | `Zeroizing<String>` 在不再需要时立即从内存中擦除 API 密钥。 |
| 7 | **OFP 双向认证** | HMAC-SHA256 nonce-based，常数时间验证用于 P2P 网络。 |
| 8 | **能力门** | 基于角色的访问控制——智能体声明所需工具，内核强制执行。 |
| 9 | **安全头** | CSP、X-Frame-Options、HSTS、X-Content-Type-Options 在每个响应上。 |
| 10 | **健康端点编辑** | 公共健康检查返回最少信息。完整诊断需要认证。 |
| 11 | **子进程沙箱** | `env_clear()` + 选择性变量传递。进程树隔离与跨平台 kill。 |
| 12 | **提示注入扫描器** | 检测 override 尝试、数据外泄模式和技能中的 shell 引用注入。 |
| 13 | **循环守卫** | 基于 SHA256 的工具调用循环检测与断路器。处理 ping-pong 模式。 |
| 14 | **会话修复** | 7 阶段消息历史验证和自动从损坏中恢复。 |
| 15 | **路径遍历防护** | 规范化与符号链接转义预防。`../` 在这里不起作用。 |
| 16 | **GCRA 速率限制器** | 成本感知的令牌桶速率限制，带 per-IP 追踪和过期清理。 |

---

## 架构

14 个 Rust crate。137,728 行代码。模块化内核设计。

```
librefang-kernel      编排、工作流、计量、RBAC、调度、预算追踪
librefang-runtime     智能体循环、3 个 LLM 驱动、53 个工具、WASM 沙箱、MCP、A2A
librefang-api         140+ REST/WS/SSE 端点、OpenAI 兼容 API、仪表板
librefang-channels    40 个消息适配器，带速率限制
librefang-memory      SQLite 持久化、向量嵌入、规范会话、压缩
librefang-types       核心类型、污染追踪、Ed25519 清单签名、模型目录
librefang-skills      60 个内置技能、SKILL.md 解析器、FangHub 市场
librefang-hands       7 个自主 Hands、HAND.toml 解析器、生命周期管理
librefang-extensions  25 个 MCP 模板、AES-256-GCM 凭据保险库、OAuth2 PKCE
librefang-wire        OFP P2P 协议，带 HMAC-SHA256 双向认证
librefang-cli         CLI，带守护进程管理、TUI 仪表板、MCP 服务器模式
librefang-desktop     Tauri 2.0 原生应用（系统托盘、通知、全局快捷键）
librefang-migrate     OpenClaw、LangChain、AutoGPT 迁移引擎
xtask                构建自动化
```

---

## 快速开始

```bash
# 1. 安装
cargo install --git https://github.com/librefang/librefang librefang-cli

# 2. 初始化 — 引导你完成提供商设置
librefang init

# 3. 启动守护进程
librefang start

# 4. 仪表板地址：http://localhost:4545

# 5. 激活一个 Hand — 它开始为你工作
librefang hand activate researcher

# 6. 与智能体聊天
librefang chat researcher
> "AI 智能体框架有哪些新兴趋势？"

# 7. 生成一个预构建的智能体
librefang agent spawn coder
```

---

## 开发

```bash
# 构建工作空间
cargo build --workspace --lib

# 运行所有测试 (1767+)
cargo test --workspace

# Lint（必须是 0 警告）
cargo clippy --workspace --all-targets -- -D warnings

# 格式化
cargo fmt --all -- --check
```

---

## 稳定性说明

LibreFang 是 pre-1.0。架构稳固，测试套件全面，安全模型全面。也就是说：

- **破坏性变更** 可能在 minor 版本之间发生，直到 v1.0
- **一些 Hands** 比其他的更成熟（Browser 和 Researcher 是经过实战检验的）
- **边缘情况** 存在——如果你发现了一个，[开 issue](https://github.com/librefang/librefang/issues)
- 在生产部署中**锁定到特定 commit**，直到 v1.0

我们快速发布，快速修复。目标是 2026 年中发布可靠的 v1.0。

---

## 安全

要报告安全漏洞，请遵循 [SECURITY.md](SECURITY.md) 中的私人报告流程。

---

## 许可证

MIT 许可证。详见 LICENSE 文件。

---

## 链接

- [GitHub](https://github.com/librefang/librefang)
- [网站](https://librefang.ai/)
- [文档](https://docs.librefang.ai)
- [贡献指南](CONTRIBUTING.md)
- [治理](GOVERNANCE.md)
- [维护者](MAINTAINERS.md)
- [安全策略](SECURITY.md)

---

<p align="center">
  <strong>使用 Rust 构建。16 层安全保障。真正为你工作的智能体。</strong>
</p>
