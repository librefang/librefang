# RFC: LibreFang 概念全景图与设计评审

> 内部架构分析 — Agent OS 各概念如何串联、抽象层在哪里重叠、以及具体的优化方案。

---

## 1. 概念全景图

### 1.1 现状架构

```
┌──────────────────────────────────────────────────────────────┐
│                     LibreFang Agent OS                        │
│                                                              │
│  ┌───────────┐                                               │
│  │  Channel   │  设备驱动 (Telegram/Discord/Slack…47+)       │
│  └─────┬─────┘                                               │
│        │ ChannelMessage                                      │
│  ┌─────▼─────┐                                               │
│  │  Router    │  网络路由器 (binding → direct → auto)         │
│  └─────┬─────┘                                               │
│        │ dispatch                                            │
│  ┌─────▼─────────────────────────────────────────────┐       │
│  │                    Kernel                          │       │
│  │  ┌──────────┐ ┌───────────┐ ┌──────────────┐     │       │
│  │  │ Registry │ │ Scheduler │ │  Event Bus   │     │       │
│  │  │ (进程表) │ │ (crontab) │ │  (信号系统)  │     │       │
│  │  └────┬─────┘ └─────┬─────┘ └──────┬───────┘     │       │
│  │       │              │              │             │       │
│  │  ┌────▼──────────────▼──────────────▼──────┐      │       │
│  │  │              Agent (进程)                │      │       │
│  │  │  ┌─────────┐ ┌────────┐ ┌─────────────┐│      │       │
│  │  │  │Manifest │ │Session │ │  Workspace  ││      │       │
│  │  │  │ (配置)  │ │(会话)  │ │ (工作目录)  ││      │       │
│  │  │  └─────────┘ └────────┘ └─────────────┘│      │       │
│  │  │  ┌──────────────────────────────────┐   │      │       │
│  │  │  │  Capability (权限模型)            │   │      │       │
│  │  │  │  file / net / tool / shell / …   │   │      │       │
│  │  │  └──────────────────────────────────┘   │      │       │
│  │  └────────────────┬────────────────────────┘      │       │
│  └───────────────────┼───────────────────────────────┘       │
│                      │ 调用                                  │
│  ┌───────────────────▼───────────────────────────────┐       │
│  │                Runtime (执行引擎)                  │       │
│  │  ┌───────────┐ ┌────────────┐ ┌───────────────┐  │       │
│  │  │LLM Driver │ │Tool Runner │ │   Sandbox     │  │       │
│  │  │(多模型    │ │(工具执行)  │ │(WASM / Docker)│  │       │
│  │  │ 驱动)     │ │            │ │               │  │       │
│  │  └───────────┘ └──────┬─────┘ └───────────────┘  │       │
│  └───────────────────────┼───────────────────────────┘       │
│                          │ 加载                              │
│  ┌───────────────────────▼───────────────────────────┐       │
│  │  ┌────────┐  ┌─────────┐  ┌─────────────────┐    │       │
│  │  │  Tool  │  │  Skill  │  │   Extension     │    │       │
│  │  │(系统   │  │(共享库) │  │  (内核模块)     │    │       │
│  │  │ 调用)  │  │         │  │                 │    │       │
│  │  └────────┘  └─────────┘  └─────────────────┘    │       │
│  └───────────────────────────────────────────────────┘       │
│                                                              │
│  ┌──────────┐  ┌──────────┐  ┌────────────────┐             │
│  │  Memory  │  │  Budget  │  │   Approval     │             │
│  │(文件系统)│  │(资源配额)│  │    (sudo)      │             │
│  └──────────┘  └──────────┘  └────────────────┘             │
│                                                              │
│  ┌──────────┐  ┌───────────────────┐                         │
│  │   Hand   │  │   Wire / OFP      │                         │
│  │(守护服务)│  │  (网络协议栈)     │                         │
│  └──────────┘  └───────────────────┘                         │
│                                                              │
│  ┌──────────┐  ┌──────────┐                                  │
│  │   API    │  │ Desktop  │   用户界面                       │
│  │(HTTP/WS) │  │ (Tauri)  │                                  │
│  └──────────┘  └──────────┘                                  │
└──────────────────────────────────────────────────────────────┘
```

### 1.2 建议架构（Assistant 取代 Router）

```
┌──────────────────────────────────────────────────────────────┐
│                     LibreFang Agent OS                        │
│                                                              │
│  ┌───────────┐                                               │
│  │  Channel   │  设备驱动 (Telegram/Discord/Slack…47+)       │
│  └─────┬─────┘                                               │
│        │ ChannelMessage                                      │
│  ┌─────▼──────────────────────┐                              │
│  │  Bridge                    │                              │
│  │  ├── 策略检查              │                              │
│  │  ├── Binding 精确匹配?     │                              │
│  │  │   └── 是 → 直接发到     │                              │
│  │  │       目标 agent        │                              │
│  │  └── 否 → 发到 Assistant   │                              │
│  └─────┬──────────────────────┘                              │
│        │                                                     │
│  ┌─────▼─────────────────────────────────────────────┐       │
│  │                    Kernel                          │       │
│  │  ┌──────────┐ ┌───────────┐ ┌──────────────┐     │       │
│  │  │ Registry │ │ Scheduler │ │  Event Bus   │     │       │
│  │  │ (进程表) │ │ (crontab) │ │  (信号系统)  │     │       │
│  │  └────┬─────┘ └─────┬─────┘ └──────┬───────┘     │       │
│  │       │              │              │             │       │
│  │  ┌────▼──────────────▼──────────────▼──────┐      │       │
│  │  │         Assistant (init 进程)            │      │       │
│  │  │                                         │      │       │
│  │  │  常驻，持有完整对话上下文                │      │       │
│  │  │  具备委派工具：                          │      │       │
│  │  │  • delegate_to_service(id, msg)         │      │       │
│  │  │  • spawn_specialist(template, msg)      │      │       │
│  │  │  • 常规工具 (file, web, shell…)         │      │       │
│  │  │                                         │      │       │
│  │  │  决策：                                  │      │       │
│  │  │  ├── 简单问题 → 自己回答                │      │       │
│  │  │  ├── 专业问题 → spawn 专家              │      │       │
│  │  │  ├── 自治任务 → 委派给 Service agent    │      │       │
│  │  │  └── 多意图   → 分步委派多个 agent      │      │       │
│  │  └────────────────┬────────────────────────┘      │       │
│  │                   │                               │       │
│  │  ┌────────────────▼─────────────────────┐         │       │
│  │  │  Agent (统一概念，按 lifecycle 区分)  │         │       │
│  │  │                                      │         │       │
│  │  │  Ephemeral  ── 临时专家，用完回收     │         │       │
│  │  │  Ttl        ── 带存活时间，超时回收   │         │       │
│  │  │  Persistent ── 手动管理，长期存活     │         │       │
│  │  │  Service    ── 守护服务 (原 Hand)     │         │       │
│  │  └──────────────────────────────────────┘         │       │
│  └───────────────────────────────────────────────────┘       │
│                                                              │
│  ┌───────────────────────────────────────────────────┐       │
│  │                Runtime (执行引擎)                  │       │
│  │  ┌───────────┐ ┌────────────┐ ┌───────────────┐  │       │
│  │  │LLM Driver │ │Tool Runner │ │   Sandbox     │  │       │
│  │  │(多模型    │ │(工具执行)  │ │(WASM / Docker)│  │       │
│  │  │ 驱动)     │ │            │ │               │  │       │
│  │  └───────────┘ └──────┬─────┘ └───────────────┘  │       │
│  └───────────────────────┼───────────────────────────┘       │
│                          │                                   │
│  ┌───────────────────────▼───────────────────────────┐       │
│  │              统一 ToolProvider                     │       │
│  │  ┌──────────┐ ┌──────────┐ ┌──────────┐          │       │
│  │  │ Builtin  │ │  Skill   │ │   MCP    │          │       │
│  │  │(系统调用)│ │ (共享库) │ │(远程协议)│          │       │
│  │  └──────────┘ └──────────┘ └──────────┘          │       │
│  └───────────────────────────────────────────────────┘       │
│                                                              │
│  ┌──────────┐  ┌──────────┐  ┌────────────────┐             │
│  │  Memory  │  │  Budget  │  │   Approval     │             │
│  │(文件系统)│  │(资源配额)│  │    (sudo)      │             │
│  └──────────┘  └──────────┘  └────────────────┘             │
│                                                              │
│  ┌───────────────────┐                                       │
│  │   Wire / OFP      │                                       │
│  │  (网络协议栈)     │                                       │
│  └───────────────────┘                                       │
│                                                              │
│  ┌──────────┐  ┌──────────┐                                  │
│  │   API    │  │ Desktop  │   用户界面                       │
│  │(HTTP/WS) │  │ (Tauri)  │                                  │
│  └──────────┘  └──────────┘                                  │
└──────────────────────────────────────────────────────────────┘
```

**核心变化：**
- Router 消失，**Assistant 成为 init 进程**（PID 1），所有无精确绑定的消息都发给它
- Hand 概念合并回 Agent，通过 **`AgentLifecycle::Service`** 区分
- Assistant 通过 **LLM 理解意图** 而非关键词匹配，自行决定回答还是委派
- 对话上下文天然保持（所有消息经过同一个 Assistant）

---

## 2. OS 类比对照表

### 2.1 现状

| LibreFang 概念 | OS 对应物 | 职责 |
|---|---|---|
| **Kernel** | 内核 | 进程管理、调度、事件分发、安全执行 |
| **Agent** | 进程 | 最小执行单元；状态机 (Created → Running → Terminated) |
| **Hand** | 守护服务 (systemd service) | 长期运行的 agent + 生命周期管理 + 仪表盘 + 市场 |
| **Channel** | 设备驱动 | 适配 47+ 外部平台，统一转换为 `ChannelMessage` |
| **Router** | 网络路由器 | 多级匹配：binding → direct → channel default → auto-select |
| **Tool** | 系统调用 (syscall) | 原子操作 (file_read, shell_exec, web_fetch…) |
| **Skill** | 共享库 (.so/.dylib) | 打包一组 tool + manifest；可热加载 (Python / WASM / Node / PromptOnly) |
| **Extension** | 内核模块 | 系统级集成：OAuth、Vault、MCP server 配置 |
| **Memory** | 文件系统 + RAM | 三层：User (持久) / Session (临时) / Agent (学习) |
| **Capability** | 文件权限 + SELinux | 细粒度授权：file / net / tool / shell / spawn / econ |
| **Approval** | sudo | 危险操作需人类确认；风险等级 + 自动拒绝超时 |
| **Scheduler** | crontab | At / Every / Cron 调度 → AgentTurn / Webhook / SystemEvent 动作 |
| **Trigger** | 信号处理器 | 基于条件的事件触发 |
| **Budget** | ulimit / cgroups | LLM token、网络流量、CPU 时间、美元成本上限 |
| **Wire / OFP** | TCP/IP 协议栈 | 跨机器 agent-to-agent RPC、节点发现、HMAC 认证 |
| **A2A** | HTTP API 网关 | Google A2A 协议，跨系统 agent 通信 |
| **Workspace** | 进程工作目录 | 每个 agent 的 SOUL.md / AGENT.json / memory/ |
| **Session** | 终端会话 | 绑定到 agent 的对话上下文 |
| **Identity** | /etc/passwd + profile | SOUL.md (人格)，视觉标识 (emoji/颜色) |
| **Runtime** | 进程运行时 (libc) | Agent loop、LLM 驱动、工具执行、沙箱 |
| **Registry** | 进程表 (/proc) | 追踪所有 agent、用户、会话 |
| **Event Bus** | D-Bus / 信号系统 | 发布-订阅：生命周期、内存、网络、系统事件 |

### 2.2 建议调整

| 变更 | 现状 | 建议 |
|------|------|------|
| **Router** | 独立的关键词路由器 | 移除；Assistant 通过 LLM 理解意图，自行委派 |
| **Assistant** | 兜底 agent | 升级为 **init 进程 (PID 1)**，所有消息的默认入口 |
| **Hand** | 独立的守护服务概念 | 合并回 Agent，变为 `AgentLifecycle::Service` |
| **Template Agent** | 用完即杀 | 支持 `Ttl` 生命周期，空闲后自动回收 |

---

## 3. 概念关系

### 3.1 现状消息生命周期

```
用户在 Telegram 发送消息
    │
    ▼
Channel Adapter ─── 转换为 ChannelMessage
    │
    ▼
BridgeManager.dispatch_message()
    ├── 策略检查 (DM/群聊过滤、频率限制)
    ├── 斜杠命令？→ 本地处理，不走 agent
    ├── 广播路由？→ 扇出到多个 agent
    └── 标准路由（按优先级）：
         ① Agent Bindings (规则匹配，最具体的优先)
         ② Direct Routes  (channel + user → 固定 agent)
         ③ User Defaults  (用户级默认)
         ④ Channel Default (频道级默认)
         ⑤ System Default  (最终兜底)
    │
    ▼
RBAC 权限验证
    │
    ▼
Auto-reply 检查（命中则直接回复，短路）
    │
    ▼
Kernel.send_message() ─── 按 agent 模块类型分发：
    ├── builtin:router  → auto_select_hand / auto_select_template
    ├── builtin:chat    → 标准 LLM agent loop
    ├── wasm:           → WASM 沙箱执行
    └── python:         → Python 子进程
    │
    ▼
Agent Loop (runtime):
    ├── 构建 system prompt + 对话历史
    ├── 调用 LLM，携带可用工具列表
    ├── LLM 返回文本 + tool calls
    ├── 对每个 tool call：
    │   ├── Capability 权限检查
    │   ├── Approval 请求（如为危险操作）
    │   ├── 通过 ToolRunner 执行
    │   └── 记录 DecisionTrace (审计追踪)
    ├── 重复直到 LLM 不再调用工具
    └── 格式化响应
    │
    ▼
响应 → Channel Adapter → 平台 → 用户
```

### 3.2 建议消息生命周期（Assistant 取代 Router）

```
用户在 Telegram 发送消息
    │
    ▼
Channel Adapter ─── 转换为 ChannelMessage
    │
    ▼
BridgeManager.dispatch_message()
    ├── 策略检查 (DM/群聊过滤、频率限制)
    ├── 斜杠命令？→ 本地处理
    ├── 广播路由？→ 扇出到多个 agent
    └── 路由匹配：
         ① Agent Bindings (精确规则匹配)
         ② Direct Routes  (channel + user → 固定 agent)
         ③ 无匹配 → 发给 Assistant（默认入口）
    │
    ▼
RBAC 权限验证 → Auto-reply 检查
    │
    ▼
Assistant (init 进程, 常驻) 收到消息：
    │
    ▼
LLM 理解意图 + 对话上下文，做出决策：
    │
    ├── 简单问题 → 直接回答
    │   "今天天气怎么样" → Assistant 自己调 web_fetch 回答
    │
    ├── 专业问题 → spawn_specialist(template, msg)
    │   "帮我优化这段 SQL" → spawn coder (Ttl 10min)
    │   结果回到 Assistant 上下文
    │
    ├── 自治任务 → delegate_to_service(hand_id, msg)
    │   "帮我发条推文" → 委派给 Twitter Service agent
    │   确认信息回到 Assistant
    │
    └── 多意图 → 分步委派
        "分析 API 性能，然后写个测试"
        → 先 spawn performance analyst
        → 拿到结果后 spawn test writer
        → 汇总回复用户
    │
    ▼
响应 → Channel Adapter → 平台 → 用户

优势：
✓ 对话上下文天然保持（同一个 Assistant）
✓ 多意图支持（LLM 理解复合请求）
✓ 路由准确率高（LLM 语义理解 vs 关键词匹配）
✓ 用户追问自然衔接（"改一下" → Assistant 知道在说什么）
```

### 3.3 Router 现状 vs Assistant 方案对比

```
                    现状 (Router)                  建议 (Assistant)
                    ─────────────                  ────────────────
意图理解:          关键词 ×3/×1                    LLM 语义理解
                   + 余弦相似度 0–3                + 完整对话上下文

延迟:              ~0ms (确定性匹配)               +1-3s (额外 LLM 调用)

成本:              零                              每条消息多一次 LLM 调用

准确率:            低 (关键词脆弱,                 高 (LLM 理解语义,
                   多语言差)                       天然支持多语言)

对话连续性:        无 (每次新 agent)               有 (Assistant 记住上下文)

多意图:            不支持 (只选一个)               支持 (LLM 分步委派)

降级策略:          Hand → Template → assistant     Assistant 自己处理
                                                   (spawn 失败则自行回答)

跟进对话:          断裂 ("改一下" 没上下文)        自然 (同一个 Assistant)
```

### 3.4 成本优化：可选关键词快速路由

对于高流量场景，可保留关键词路由作为**可选优化**（而非必经之路）：

```toml
[channel.telegram]
# 高流量频道：用关键词快速路由省钱
routing_mode = "keyword"

[channel.slack]
# 低流量频道：走 Assistant 获得更好体验
routing_mode = "assistant"   # 默认值
```

```
routing_mode = "keyword" 时:
    消息 → 关键词匹配 → 命中则直接转发 → 未命中则发给 Assistant

routing_mode = "assistant" 时（默认）:
    消息 → 直接发给 Assistant → Assistant 决定如何处理
```

Router 从**必经的中间层**降级为**可选的成本优化手段**。

### 3.5 Hand 与 Agent 统一后的生命周期

```
统一 Agent 概念，通过 AgentLifecycle 区分：

┌──────────────────────────────────────────────────────┐
│                    AgentLifecycle                      │
│                                                      │
│  Ephemeral ─── 用完即回收                            │
│  │              Assistant spawn 的临时专家            │
│  │                                                   │
│  Ttl { idle_timeout } ─── 空闲超时后回收             │
│  │              解决 "接着聊" 的问题                  │
│  │              默认 10 分钟，收到新消息则重置        │
│  │                                                   │
│  Persistent ─── 手动管理，显式杀死                    │
│  │              用户主动创建的 agent                  │
│  │                                                   │
│  Service { requirements, settings, dashboard, ... }  │
│                 守护服务（原 Hand）                   │
│                 市场安装、依赖检查、仪表盘监控        │
│                 确定性 ID (UUID v5)                   │
└──────────────────────────────────────────────────────┘
```

### 3.6 Tool 提供者层级

```
Agent 需要调用一个工具
    │
    ▼
ToolRunner 统一解析提供者：
    ├── Builtin tools     ── 编译进二进制 (file_read, shell_exec…)
    ├── Skill tools       ── 已安装的技能 (Python/WASM/Node 子进程)
    ├── MCP server tools  ── 远程 MCP 协议调用
    └── (未来: OFP 远程 agent 提供的工具)
```

### 3.7 Memory 流转

```
Agent 产生响应 + 决策追踪
    │
    ▼
ProactiveMemory 提取事实/洞察
    │
    ▼
创建 MemoryFragment (分类、评分)
    │
    ▼
Memory 基底层按级别存储：
    ├── User 层   ── 跨会话事实 (持久)
    ├── Session 层 ── 当前对话 (临时)
    └── Agent 层   ── 学习到的行为 (持久)
    │
    ▼
下次 agent 调用时：
    ├── 从所有层级检索
    ├── 通过 ContextEngine 注入上下文
    └── 溢出管理 (裁剪低重要度片段)
```

### 3.8 安全模型

```
Agent 尝试执行某个操作
    │
    ▼
Capability 权限检查：
    ├── 已授权？→ 放行
    ├── 未授权？→ 拒绝
    └── 需要审批？
        │
        ▼
        创建 ApprovalRequest
        ├── RiskLevel: Low / Medium / High / Critical
        ├── 自动拒绝倒计时
        └── Agent 挂起，等待
            │
            ▼
        人类通过 dashboard/API 批准或拒绝
            │
            ▼
        Agent 恢复或中止操作
```

---

## 4. Crate 依赖图

```
librefang-types              (核心类型 — 零依赖)
    ▲
    │
    ├── librefang-memory     (SQLite 内存基底层)
    ├── librefang-runtime    (agent loop, LLM 驱动, 工具执行, 沙箱)
    ├── librefang-skills     (技能注册, 市场, ClawHub)
    ├── librefang-hands      (Hand 定义, 注册表) → 建议合并回 types + kernel
    ├── librefang-extensions (OAuth, Vault, MCP 配置)
    ├── librefang-channels   (47+ 频道适配器, Bridge, 路由)
    ├── librefang-wire       (OFP 点对点协议)
    └── librefang-migrate    (OpenClaw/ClawHub 导入)
         ▲
         │
    librefang-kernel         (组装所有子系统)
         ▲
         │
    librefang-api            (HTTP/WS/SSE 服务, 仪表盘)
         ▲
         │
    ├── librefang-cli        (CLI 二进制 + MCP 服务模式)
    └── librefang-desktop    (Tauri 2.0 桌面应用)
```

---

## 5. 设计亮点

### 5.1 OS 隐喻一致性

操作系统类比贯彻得非常彻底 — 从 capability-based security 到 approval (sudo)
到 cron scheduler 到 event bus，每个概念都能映射到成熟的 OS 原语。这使得整体
心智模型统一，新贡献者容易上手。

### 5.2 Channel 层

系统中最干净的抽象。47+ 平台适配器统一在 `ChannelAdapter` trait 之后。Bridge
负责格式转换和策略执行。新增平台只需实现 trait。关注点分离清晰。

### 5.3 安全模型

不是简单的 allow/deny，而是细粒度的 capability 授权 + 风险等级审批 +
自动拒绝超时。污点追踪 (taint) 系统标记经过 tool call 的不受信数据流向。
这个级别的安全设计在 agent 系统中很必要，在竞品中少见。

### 5.4 内存架构

三层模型 (User / Session / Agent) 精准映射真实使用场景。重要度评分的溢出管理
是务实的工程选择，避免了 "上下文窗口满了" 的断崖效应。

### 5.5 确定性 Agent ID（原 Hand ID）

UUID v5 从 service_id 派生，让 cron job、trigger 和外部引用在守护进程重启后自动
归位，不需要迁移逻辑。一个小巧但优雅的工程方案。合并 Hand 后此设计应保留在
`AgentLifecycle::Service` 中。

### 5.6 优雅降级链

当前 Router 的降级链（Hand → Template → assistant）设计稳健。在 Assistant
方案中，降级更自然：Assistant 委派失败时直接自己回答，无需多级降级逻辑。

---

## 6. 设计问题与优化方案

### 6.1 用 Assistant 取代 Router（核心提案）

**问题：** Router 用关键词匹配做意图分类，但 LLM 天生就是做这个的，而且做得
更好。Router 作为必经中间层带来了多个问题：

1. **对话断裂** — Template agent 用完即杀，追问时丢失上下文
2. **路由不准** — 关键词匹配在多语言、模糊表达下脆弱
3. **不支持多意图** — "分析性能然后写测试" 只能选一个目标
4. **额外概念负担** — 用户需要理解 Router、Hand、Template 三个概念

**方案：** Assistant 升级为 **init 进程**（PID 1），成为所有无精确绑定消息的
默认入口。通过委派工具与其他 agent 协作：

```rust
// Assistant 的工具列表中增加委派能力
ToolDefinition {
    name: "delegate_to_service",
    description: "将任务委派给已激活的 Service agent (如 Twitter、Browser 等)",
    input_schema: json!({
        "type": "object",
        "properties": {
            "service_id": { "type": "string", "description": "Service agent ID" },
            "message": { "type": "string", "description": "要发送的消息" }
        }
    })
}

ToolDefinition {
    name: "spawn_specialist",
    description: "创建临时专家 agent 处理专业问题",
    input_schema: json!({
        "type": "object",
        "properties": {
            "template": { "type": "string", "description": "agent 模板名" },
            "message": { "type": "string", "description": "要发送的消息" }
        }
    })
}
```

**执行流程：**
- 简单问题 → Assistant 直接回答（零额外开销）
- 专业问题 → Assistant 调 `spawn_specialist`，结果回到 Assistant 上下文
- 自治任务 → Assistant 调 `delegate_to_service`，确认回到 Assistant
- 多意图 → Assistant 分步调用多个委派工具，汇总结果

**成本控制：** 高流量场景可配置 `routing_mode = "keyword"` 回退到关键词路由。

**影响：**
- 移除 `builtin:router` 模块和 `auto_select_hand/template` 逻辑
- Router 相关代码降级为可选优化模块
- 对话连续性、多意图、路由准确率同时提升
- 用户概念从 5 个 (Router/Hand/Template/Agent/Assistant) 简化为 2 个
  (Assistant + Agent)

### 6.2 Hand 概念保留，内部实现简化

**原则：** Hand 是产品卖点（"你有很多只手在帮你做事"），在用户层面保留为
一等概念。但内部实现去重，消除双重注册表和冗余结构体。

**产品层定位：**

```
用户看到的：                          系统管理的：
┌──────────────────────┐             ┌───────────────────┐
│  Hand（你的"手"）     │             │  Agent（执行引擎） │
│  - Marketplace 安装   │  ──激活──→  │  - AgentEntry      │
│  - Dashboard 监控     │             │  - AgentManifest   │
│  - Settings 配置      │             │  - Lifecycle 管理  │
│  - Requirements 检查  │             │                    │
└──────────────────────┘             └───────────────────┘
```

| | Agent | Hand |
|---|---|---|
| 交互模式 | **你跟它聊** (interactive) | **它替你干** (autonomous) |
| 用户关系 | 对话伙伴 | 雇员/助手 |
| 生命周期 | 按需启停 | 常驻后台 |
| 可见度 | 聊天窗口 | Dashboard 仪表盘 |

**内部改动：**

1. **`AgentEntry` 增加字段，删除 `HandInstance`：**
```rust
pub struct AgentEntry {
    // ... 现有字段保持不变 ...

    /// 来源 Hand 定义 ID（替代通过 hand_registry 反查）
    pub hand_id: Option<String>,

    /// 生命周期策略（从 HandDefinition 或 template 继承）
    pub lifecycle: AgentLifecycle,

    /// 用户配置覆盖（原 HandInstance.config）
    pub config_overrides: HashMap<String, serde_json::Value>,

    /// Dashboard 指标 schema（原 HandDashboard，可选）
    pub dashboard: Option<DashboardConfig>,
}

enum AgentLifecycle {
    Ephemeral,                              // 用完即杀
    Ttl { idle_timeout: Duration },         // 空闲超时回收
    Persistent,                             // 手动管理
    Service {                               // 守护服务（原 Hand 行为）
        auto_restart: bool,
        max_restarts: u32,
        health_check_interval_secs: u64,
    },
}
```

2. **合并为单一注册表：**
```
之前：两个注册表
  kernel.registry       → AgentEntry
  kernel.hand_registry  → HandDefinition + HandInstance

之后：一个注册表 + 一个定义库
  kernel.definitions    → HashMap<String, HandDefinition>  ← 所有已安装的 Hand
  kernel.registry       → HashMap<AgentId, AgentEntry>     ← 所有运行中的 agent
```

3. **统一 spawn/reuse 逻辑：**
```rust
fn get_or_spawn(&self, definition_id: &str) -> KernelResult<AgentId> {
    // 1. 查找已有运行中 agent（通过 hand_id 字段）
    if let Some(entry) = self.registry.find_by_hand_id(definition_id) {
        if entry.state == AgentState::Running { return Ok(entry.id); }
    }
    // 2. 加载 Hand 定义、检查需求、spawn agent
    let def = self.definitions.get(definition_id)?;
    self.check_requirements(&def.requires)?;
    self.spawn_from_definition(def)
}
```

**保留不变的：**
- HAND.toml 格式和命名
- Marketplace UI
- Dashboard 监控界面
- HandDefinition 结构体（作为打包格式）
- 激活/去激活 API 端点

**删除的：**
- HandInstance 结构体（合并到 AgentEntry）
- hand_registry（合并到 definitions + registry）
- 双注册表交叉查找逻辑（active_hand_agent_id）

**影响：** 用户体验不变，内部减少约 2000 行重复代码，单一注册表简化查询逻辑。

### 6.3 统一 Tool 提供者

**问题：** Skill、Extension (MCP) 和内建工具都向 agent 提供工具，但走的是不同的
代码路径。MCP 工具通过 runtime 的 MCP 客户端流转，Skill 工具通过
SkillRegistry → ToolRunner 流转。两条并行的发现和执行路径。

**方案：** 引入统一的 `ToolProvider` trait：

```rust
trait ToolProvider: Send + Sync {
    fn name(&self) -> &str;
    fn list_tools(&self) -> Vec<ToolDefinition>;
    async fn execute(&self, call: ToolCall) -> ToolResult;
}

enum ToolProviderKind {
    Builtin,                          // 编译进二进制 (file_read, shell_exec…)
    Skill { runtime: SkillRuntime },  // Python / WASM / Node
    Mcp { server: McpServerConfig },  // MCP 协议
    Remote { peer: PeerId },          // OFP 远程 agent (未来)
}
```

- Extension crate 只保留非工具类集成 (OAuth、Vault)。
- MCP 工具作为 ToolProvider 注册，与 Skill 并列。
- ToolRunner 统一查询所有 provider。
- Agent 看到的是扁平的工具命名空间，不区分来源。

### 6.4 拆解 Kernel

**问题：** kernel.rs 是一个上帝对象 — agent 生命周期、内存协调、工具授权、
审批处理、调度器、事件总线、路由集成、计量、配置热加载、认证、自动回复、
配对、向导，全都挂在 `&self` (Kernel) 上。

**方案：** 提取独立的子系统管理器：

```rust
struct Kernel {
    process_mgr:  ProcessManager,   // spawn / kill / suspend / 状态机
    scheduler:    SchedulerManager, // cron + trigger
    security:     SecurityManager,  // capability + approval + auth + RBAC
    event_bus:    EventBus,         // 发布-订阅分发
    memory_mgr:   MemoryManager,    // 基底层协调 + 溢出
    resource_mgr: ResourceManager,  // budget + metering + 配额
}
```

每个 manager 拥有自己的状态 (DashMap、config)，暴露聚焦的 trait。
Kernel 变成薄的协调层，只负责把 manager 串联起来。好处：
- 各子系统可独立单元测试
- 所有权边界清晰
- 未来更容易拆成独立 crate

### 6.5 Manifest 合并

**问题：** HAND.toml 的 `[agent]` 部分重复了 AgentManifest 的大部分字段
(name, description, module, model, system_prompt, max_tokens, temperature,
resources, capabilities)。修改 AgentManifest 可能需要同步修改 HandAgentConfig。

**方案：** 随 §6.2 (Hand 内部简化) 自动解决。marketplace 的 manifest
就是 AgentManifest + `lifecycle = Service { ... }`，不需要单独的
HandAgentConfig。

### 6.6 概念分层：Hand / Agent / Skill / Extension

**原则：** 四个概念处于不同层级，不应合并为同一个东西。

```
┌─────────────────────────────────────────────────────┐
│  Hand（应用包）                                      │
│  组合下层所有能力，面向用户的完整解决方案             │
│  ├── 引用 1 个 Agent 配置                            │
│  ├── 引用 N 个 Skills                                │
│  └── 引用 N 个 Extensions                            │
│  例：clip hand = agent config + whisper skill        │
│       + ffmpeg skill + youtube extension              │
├─────────────────────────────────────────────────────┤
│  Agent（运行时引擎）                                  │
│  执行单元，调用下层的工具                             │
├─────────────────────────────────────────────────────┤
│  Skill（工具层）          Extension（连接层）          │
│  Python/WASM/Node 代码     MCP Server 进程            │
│  提供 tool 给 agent        提供外部服务接入            │
└─────────────────────────────────────────────────────┘
```

**OS 类比：**

| LibreFang | Linux | 粒度 |
|-----------|-------|------|
| **Hand** | .app / snap package | 应用 |
| **Agent** | process | 进程 |
| **Skill** | .so / .dylib | 共享库 |
| **Extension** | kernel module / driver | 驱动 |

**设计决策：** 概念不合并，分发层统一（见 §6.7）。

### 6.7 内容分发：librefang-registry 拆分

**问题：** 当前 60+ bundled skills、25 integration templates、Hand 定义全部
通过 `include_str!()` 编译进二进制。导致：
- 更新 Hand prompt 需要重新发 release
- 社区贡献 Hand/Skill 需要碰 Rust 代码
- binary 体积膨胀

**方案：** 框架代码留核心仓库，内容定义拆到 `librefang-registry` 仓库。

**拆分原则 — 框架 vs 内容：**

| 概念 | 框架（留 librefang 核心仓库） | 内容（拆到 librefang-registry） |
|------|------|------|
| **Hand** | HandRegistry, 激活逻辑, 需求检查, 设置解析 | 各个 HAND.toml 定义 |
| **Skill** | SkillRegistry, tool dispatch, 安全扫描 | 各个 skill.toml + 脚本 |
| **Extension** | IntegrationRegistry, Vault, 健康监控 | 各个 integration.toml |
| **Agent** | AgentEntry, 状态机, agent loop | 各个模板 .toml |
| **Provider** | Rust HTTP 客户端代码 (留核心) | provider 配置 TOML (拆) |
| **MCP** | MCP 客户端运行时 (全部留核心) | — |

**一句话：拆的是"菜谱"（TOML + 脚本），留的是"厨房"（Rust 框架）。**

**librefang-registry 仓库结构（已存在）：**

```
librefang-registry/
├── hands/              ← Hand 定义 (HAND.toml)
├── skills/             ← Skill 实现 (skill.toml + 代码)
├── agents/             ← Agent 模板 (.toml)
├── integrations/       ← Extension 模板 (.toml)
├── plugins/            ← 插件 (echo-memory 等)
├── providers/          ← Provider 配置 (42 个 TOML)
├── aliases.toml        ← 别名映射
├── schema.toml         ← Schema 定义
├── scripts/
│   └── validate.py     ← 验证脚本
└── README.md
```

**核心仓库对接方式：**

```bash
librefang init
  ├── 创建 ~/.librefang/
  ├── 生成 config.toml
  ├── 拉取 librefang-registry → ~/.librefang/registry/
  │   ├── hands/     → 可用的 Hand 定义
  │   ├── skills/    → 可用的 Skill
  │   ├── providers/ → Provider 配置
  │   └── ...
  └── 启动 daemon

librefang update
  └── 拉取 registry 最新版本，对比本地，提示可更新的包

librefang install <package>
  └── 从 registry 安装指定的 hand/skill/extension
```

**核心仓库需要的改动：**

1. 去掉 `include_str!()` 的 bundled 内容 → 改为从 `~/.librefang/` 加载
2. 保留 assistant 一个内置 → 保证零网络也能启动（init 进程不能依赖外部）
3. `init` 命令增加拉取 registry 步骤
4. 各 Registry 的 `load_bundled()` 改为 `load_from_dir()`

**不拆 MCP 和 Provider 框架代码的理由：**

- **MCP 客户端**：协议实现 + 连接管理 + 健康检查，是内核的"网络栈"，
  agent 运行时直接依赖
- **Provider 框架**：LLM API 客户端（Anthropic/OpenAI/Groq/Ollama），
  数量有限（~10 个），需要 Rust 编译保证性能，streaming/token counting
  跟运行时紧耦合。但 Provider 的**配置**（endpoint URL、model 列表）
  拆到 registry（已有 42 个 TOML）

### 6.8 依赖解析与统一分发（未来）

**问题：** Hand 引用 Skill 和 Extension，但当前无自动依赖解析。
用户激活 clip hand 后发现缺 whisper skill，需要手动安装。

**方案（v1.0 规划）：** 统一打包格式 + 依赖声明：

```toml
# HAND.toml 增加依赖声明
[dependencies]
skills = ["whisper-transcribe", "ffmpeg-tools"]
extensions = ["youtube"]
```

```bash
librefang install clip
# 自动安装：clip hand + whisper-transcribe skill + ffmpeg-tools skill + youtube extension
```

此方案不在 0.x 实施，但 HAND.toml 格式应预留 `[dependencies]` 字段。

---

## 7. 版本路线图

### v0.7 — Registry 拆分

| 任务 | 说明 |
|------|------|
| 去除 `include_str!()` bundled 内容 | Skills、Integrations、Hands 改为文件系统加载 |
| `init` 命令拉取 registry | 从 librefang-registry 仓库下载到 ~/.librefang/ |
| 保留 assistant 内置 | 零网络启动保障 |
| `load_bundled()` → `load_from_dir()` | 各 Registry 加载逻辑改造 |
| `update` 命令 | 拉取 registry 最新版本 |

### v0.8 — Hand/Agent 内部合并

| 任务 | 说明 |
|------|------|
| AgentEntry 增加 `hand_id`, `lifecycle`, `config_overrides` | 统一运行时表示 |
| 删除 HandInstance | 合并到 AgentEntry |
| 合并双注册表 | hand_registry → definitions + registry |
| 新增 LifecycleManager | 统一管理 Ephemeral/Ttl/Persistent/Service |
| Dashboard API 改造 | 从 registry 查询，不再查 hand_registry |

### v0.9 — Assistant 取代 Router

| 任务 | 说明 |
|------|------|
| Assistant 升级为默认入口 | 所有无精确绑定的消息发给 Assistant |
| 增加委派工具 | delegate_to_service, spawn_specialist |
| Router 降级为可选优化 | routing_mode = "keyword" \| "assistant" |
| 移除 builtin:router 模块 | 或保留为 legacy 兼容 |

### v1.0 — 接口冻结

| 任务 | 说明 |
|------|------|
| HAND.toml / skill.toml / agent.toml 格式冻结 | 对外承诺向后兼容 |
| 依赖解析器 | Hand 自动安装所需 Skill/Extension |
| 统一 ToolProvider trait | Builtin / Skill / MCP 统一接口 |
| Kernel 子系统拆解 | ProcessManager / SecurityManager / ... |
| 对外稳定 API | HTTP/WS 端点版本化 |

---

## 8. 优先级排序

| # | 方案 | 工作量 | 影响 | 版本 | 优先级 |
|---|------|--------|------|------|--------|
| 6.7 | Registry 拆分（内容外移） | 中 | 高 (社区贡献 + binary 瘦身) | v0.7 | **P0** |
| 6.2 | Hand 内部简化（保留品牌） | 高 | 高 (消除双注册表) | v0.8 | **P0** |
| 6.1 | Assistant 取代 Router | 中 | 高 (用户体验 + 架构简化) | v0.9 | **P1** |
| 6.4 | 拆解 Kernel | 高 | 高 (可维护性) | v1.0 | **P1** |
| 6.3 | 统一 Tool 提供者 | 中 | 中 (一致性) | v1.0 | **P2** |
| 6.5 | Manifest 合并 | 低 | 低 (随 6.2 自动完成) | v0.8 | **P3** |
| 6.6 | 概念分层 | — | — (设计原则，不需要实施) | — | — |
| 6.8 | 依赖解析 | 中 | 中 (安装体验) | v1.0 | **P2** |

**核心原则：0.x 做大手术，1.0 冻结接口。**

在 0.x 阶段用户对 breaking change 有预期，是做架构调整的最佳时机。
等到 1.0 再动就意味着 breaking change，会影响已有用户。

---

## 9. 总结

LibreFang 的 OS 隐喻和整体架构是扎实的。概念体系完整（进程 → 权限 →
文件系统 → 网络协议栈），安全模型成熟，Channel 层干净。

**核心优化方向：**

1. **分层清晰化** — Hand(应用包) > Agent(进程) > Skill(共享库) + Extension(驱动)，
   各层概念独立，不合并
2. **内容外移** — 框架留核心仓库，Hand/Skill/Extension/Provider 定义
   拆到 librefang-registry，`init` 时拉取
3. **内部去重** — Hand 品牌保留，删除 HandInstance，合并双注册表，
   AgentEntry 加 lifecycle 字段
4. **路由升级** — Router 降级为可选优化，Assistant 成为 init 进程
5. **接口统一** — ToolProvider trait 统一 Builtin/Skill/MCP 三条路径

**用户概念变化：**
- 之前：Router + Hand + Template + Agent + Assistant（5 个）
- 之后：**Hand**（替你干活的）+ **Agent**（你跟它聊的）+ **Assistant**（默认入口）（3 个）

**Hand 是产品卖点，不改。内部实现简化，外部体验提升。**

OS 类比一句话总结：

> **Channel** = 设备驱动, **Assistant** = init 进程 (PID 1),
> **Hand** = 应用包 (.app), **Agent** = 进程 (四种生命周期),
> **Tool** = 系统调用, **Skill** = 共享库, **Extension** = 内核模块,
> **Memory** = 文件系统, **Capability** = 权限, **Approval** = sudo,
> **Scheduler** = crontab, **Wire/OFP** = TCP/IP,
> **Kernel** = 内核, **API/Desktop** = 用户界面。
