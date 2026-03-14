# 技能开发

技能是可插拔的工具包，用于扩展 LibreFang 中智能体的能力。一个技能将一个或多个工具及其实现打包在一起，让智能体可以完成内置工具无法覆盖的任务。本指南涵盖技能创建、清单格式、Python 和 WASM 运行时、发布到 FangHub 以及 CLI 管理。

## 目录

- [概述](#概述)
- [技能格式](#技能格式)
- [Python 技能](#python-技能)
- [WASM 技能](#wasm-技能)
- [技能需求声明](#技能需求声明)
- [安装技能](#安装技能)
- [发布到 FangHub](#发布到-fanghub)
- [CLI 命令](#cli-命令)
- [OpenClaw 兼容性](#openclaw-兼容性)
- [最佳实践](#最佳实践)

---

## 概述

一个技能由以下部分组成：

1. 一个**清单**（`skill.toml` 或 `SKILL.md`），用于声明元数据、运行时类型、提供的工具和需求。
2. 一个**入口点**（Python 脚本、WASM 模块、Node.js 模块或仅包含 prompt 的 Markdown），用于实现工具逻辑。

技能安装到 `~/.librefang/skills/` 目录下，并通过技能注册表提供给智能体使用。LibreFang 内置了 **60 个技能**，它们被编译进二进制文件中，开箱即用。

### 支持的运行时

| 运行时 | 语言 | 沙箱化 | 说明 |
|--------|------|--------|------|
| `python` | Python 3.8+ | 否（使用 `env_clear()` 的子进程） | 最易编写。使用 stdin/stdout JSON 协议。 |
| `wasm` | Rust、C、Go 等 | 是（Wasmtime 双重计量） | 完全沙箱化。最适合安全敏感的工具。 |
| `node` | JavaScript/TypeScript | 否（子进程） | OpenClaw 兼容。 |
| `prompt_only` | Markdown | 不适用 | 将专家知识注入系统 prompt。不执行代码。 |
| `builtin` | Rust | 不适用 | 编译进二进制文件。仅用于核心工具。 |

### 60 个内置技能

LibreFang 包含 60 个编译进二进制文件的专家知识技能（无需安装）：

| 分类 | 技能 |
|------|------|
| DevOps 与基础设施 | `ci-cd`、`ansible`、`prometheus`、`nginx`、`kubernetes`、`terraform`、`helm`、`docker`、`sysadmin`、`shell-scripting`、`linux-networking` |
| 云计算 | `aws`、`gcp`、`azure` |
| 编程语言 | `rust-expert`、`python-expert`、`typescript-expert`、`golang-expert` |
| 前端 | `react-expert`、`nextjs-expert`、`css-expert` |
| 数据库 | `postgres-expert`、`redis-expert`、`sqlite-expert`、`mongodb`、`elasticsearch`、`sql-analyst` |
| API 与 Web | `graphql-expert`、`openapi-expert`、`api-tester`、`oauth-expert` |
| AI/ML | `ml-engineer`、`llm-finetuning`、`vector-db`、`prompt-engineer` |
| 安全 | `security-audit`、`crypto-expert`、`compliance` |
| 开发工具 | `github`、`git-expert`、`jira`、`linear-tools`、`sentry`、`code-reviewer`、`regex-expert` |
| 写作 | `technical-writer`、`writing-coach`、`email-writer`、`presentation` |
| 数据 | `data-analyst`、`data-pipeline` |
| 协作 | `slack-tools`、`notion`、`confluence`、`figma-expert` |
| 职业发展 | `interview-prep`、`project-manager` |
| 高级 | `wasm-expert`、`pdf-reader`、`web-search` |

这些都是使用 SKILL.md 格式的 `prompt_only` 技能——注入到智能体系统 prompt 中的专家知识。

### SKILL.md 格式

SKILL.md 格式（也被 OpenClaw 使用）使用 YAML frontmatter 和 Markdown 正文：

```markdown
---
name: rust-expert
description: Expert Rust programming knowledge
---

# Rust Expert

## Key Principles
- Ownership and borrowing rules...
- Lifetime annotations...

## Common Patterns
...
```

SKILL.md 文件会被自动解析并转换为 `prompt_only` 技能。所有 SKILL.md 文件在被包含之前，都会经过自动化的 **prompt 注入扫描器** 检测，以发现 override 尝试、数据外泄模式和 shell 引用。

---

## 技能格式

### 目录结构

```
my-skill/
  skill.toml          # 清单（必需）
  src/
    main.py           # 入口点（Python 技能）
  README.md           # 可选文档
```

### 清单（skill.toml）

```toml
[skill]
name = "web-summarizer"
version = "0.4.0"
description = "Summarizes any web page into bullet points"
author = "librefang-community"
license = "MIT"
tags = ["web", "summarizer", "research"]

[runtime]
type = "python"
entry = "src/main.py"

[[tools.provided]]
name = "summarize_url"
description = "Fetch a URL and return a concise bullet-point summary"
input_schema = { type = "object", properties = { url = { type = "string", description = "The URL to summarize" } }, required = ["url"] }

[[tools.provided]]
name = "extract_links"
description = "Extract all links from a web page"
input_schema = { type = "object", properties = { url = { type = "string" } }, required = ["url"] }

[requirements]
tools = ["web_fetch"]
capabilities = ["NetConnect(*)"]
```

### 清单各节说明

#### [skill] -- 元数据

| 字段 | 类型 | 必需 | 描述 |
|------|------|------|------|
| `name` | string | 是 | 唯一的技能名称（用作安装目录名） |
| `version` | string | 否 | 语义版本号（默认：`"0.4.0"`） |
| `description` | string | 否 | 人类可读的描述 |
| `author` | string | 否 | 作者名称或组织 |
| `license` | string | 否 | 许可证标识符（例如 `"MIT"`、`"Apache-2.0"`） |
| `tags` | array | 否 | 用于在 FangHub 上发现的标签 |

#### [runtime] -- 执行配置

| 字段 | 类型 | 必需 | 描述 |
|------|------|------|------|
| `type` | string | 是 | `"python"`、`"wasm"`、`"node"` 或 `"builtin"` |
| `entry` | string | 是 | 入口点文件的相对路径 |

#### [[tools.provided]] -- 工具定义

每个 `[[tools.provided]]` 条目定义技能提供的一个工具：

| 字段 | 类型 | 必需 | 描述 |
|------|------|------|------|
| `name` | string | 是 | 工具名称（在所有工具中必须唯一） |
| `description` | string | 是 | 展示给 LLM 的描述 |
| `input_schema` | object | 是 | 定义工具输入参数的 JSON Schema |

#### [requirements] -- 宿主需求

| 字段 | 类型 | 描述 |
|------|------|------|
| `tools` | array | 此技能需要宿主提供的内置工具 |
| `capabilities` | array | 智能体必须具备的能力字符串 |

---

## Python 技能

Python 技能是最容易编写的。它们以子进程方式运行，通过 stdin/stdout 上的 JSON 进行通信。

### 协议

1. LibreFang 向脚本的 stdin 发送 JSON 负载：

```json
{
  "tool": "summarize_url",
  "input": {
    "url": "https://example.com"
  },
  "agent_id": "uuid-...",
  "agent_name": "researcher"
}
```

2. 脚本处理输入并将 JSON 结果写入 stdout：

```json
{
  "result": "- Point one\n- Point two\n- Point three"
}
```

如果发生错误，返回一个错误对象：

```json
{
  "error": "Failed to fetch URL: connection refused"
}
```

### 示例：Web 摘要器

`src/main.py`：

```python
#!/usr/bin/env python3
"""LibreFang skill: web-summarizer"""
import json
import sys
import urllib.request


def summarize_url(url: str) -> str:
    """Fetch a URL and return a basic summary."""
    req = urllib.request.Request(url, headers={"User-Agent": "LibreFang-Skill/1.0"})
    with urllib.request.urlopen(req, timeout=30) as resp:
        content = resp.read().decode("utf-8", errors="replace")

    # Simple extraction: first 500 chars as summary
    text = content[:500].strip()
    return f"Summary of {url}:\n{text}..."


def extract_links(url: str) -> str:
    """Extract all links from a web page."""
    import re

    req = urllib.request.Request(url, headers={"User-Agent": "LibreFang-Skill/1.0"})
    with urllib.request.urlopen(req, timeout=30) as resp:
        content = resp.read().decode("utf-8", errors="replace")

    links = re.findall(r'href="(https?://[^"]+)"', content)
    unique_links = list(dict.fromkeys(links))
    return "\n".join(unique_links[:50])


def main():
    payload = json.loads(sys.stdin.read())
    tool_name = payload["tool"]
    input_data = payload["input"]

    try:
        if tool_name == "summarize_url":
            result = summarize_url(input_data["url"])
        elif tool_name == "extract_links":
            result = extract_links(input_data["url"])
        else:
            print(json.dumps({"error": f"Unknown tool: {tool_name}"}))
            return

        print(json.dumps({"result": result}))
    except Exception as e:
        print(json.dumps({"error": str(e)}))


if __name__ == "__main__":
    main()
```

### 使用 LibreFang Python SDK

对于更高级的技能，可以使用 Python SDK（`sdk/python/librefang_sdk.py`）：

```python
#!/usr/bin/env python3
from librefang_sdk import SkillHandler

handler = SkillHandler()

@handler.tool("summarize_url")
def summarize_url(url: str) -> str:
    # Your implementation here
    return "Summary..."

@handler.tool("extract_links")
def extract_links(url: str) -> str:
    # Your implementation here
    return "link1\nlink2"

if __name__ == "__main__":
    handler.run()
```

---

## WASM 技能

WASM 技能在沙箱化的 Wasmtime 环境中运行。它们非常适合安全敏感的操作，因为沙箱强制执行资源限制和能力约束。

### 构建 WASM 技能

1. 使用 Rust（或任何可编译为 WASM 的语言）编写技能：

```rust
// src/lib.rs
use std::io::{self, Read};

#[no_mangle]
pub extern "C" fn _start() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).unwrap();

    let payload: serde_json::Value = serde_json::from_str(&input).unwrap();
    let tool = payload["tool"].as_str().unwrap_or("");
    let input_data = &payload["input"];

    let result = match tool {
        "my_tool" => {
            let param = input_data["param"].as_str().unwrap_or("");
            format!("Processed: {param}")
        }
        _ => format!("Unknown tool: {tool}"),
    };

    println!("{}", serde_json::json!({"result": result}));
}
```

2. 编译为 WASM：

```bash
cargo build --target wasm32-wasi --release
```

3. 在清单中引用 `.wasm` 文件：

```toml
[runtime]
type = "wasm"
entry = "target/wasm32-wasi/release/my_skill.wasm"
```

### 沙箱限制

WASM 沙箱强制执行以下限制：

- **燃料限制**：最大计算步数（防止无限循环）。
- **内存限制**：最大内存分配量。
- **能力限制**：仅适用授予智能体的能力。

这些限制来源于智能体清单中的 `[resources]` 部分。

---

## 技能需求声明

技能可以在 `[requirements]` 部分声明需求：

### 工具需求

如果你的技能需要调用内置工具（例如，使用 `web_fetch` 在处理之前下载页面）：

```toml
[requirements]
tools = ["web_fetch", "file_read"]
```

技能注册表会在加载技能之前验证智能体是否具备这些工具。

### 能力需求

如果你的技能需要特定的能力：

```toml
[requirements]
capabilities = ["NetConnect(*)", "ShellExec(python3)"]
```

---

## 安装技能

### 从本地目录安装

```bash
librefang skill install /path/to/my-skill
```

这会读取 `skill.toml`，验证清单，并将技能复制到 `~/.librefang/skills/my-skill/`。

### 从 FangHub 安装

```bash
librefang skill install web-summarizer
```

这会从 FangHub 市场注册表下载技能。

### 从 Git 仓库安装

```bash
librefang skill install https://github.com/user/librefang-skill-example.git
```

### 列出已安装的技能

```bash
librefang skill list
```

输出：

```
3 skill(s) installed:

NAME                 VERSION    TOOLS    DESCRIPTION
----------------------------------------------------------------------
web-summarizer       0.4.0      2        Summarizes any web page into bullet points
data-analyzer        0.2.1      3        Statistical analysis tools
code-formatter       1.0.0      1        Format code in 20+ languages
```

### 移除技能

```bash
librefang skill remove web-summarizer
```

---

## 发布到 FangHub

FangHub 是 LibreFang 的社区技能市场。

### 准备你的技能

1. 确保你的 `skill.toml` 包含完整的元数据：
   - `name`、`version`、`description`、`author`、`license`、`tags`
2. 包含一个带有使用说明的 `README.md`。
3. 在本地测试你的技能：

```bash
librefang skill test /path/to/my-skill
# 可选：使用 JSON 输入执行特定工具
librefang skill test /path/to/my-skill --tool summarize_url --input '{"url":"https://example.com"}'
```

### 搜索 FangHub

```bash
librefang skill search "web scraping"
```

输出：

```
Skills matching "web scraping":

  web-summarizer (42 stars)
    Summarizes any web page into bullet points
    https://fanghub.dev/skills/web-summarizer

  page-scraper (28 stars)
    Extract structured data from web pages
    https://fanghub.dev/skills/page-scraper
```

### 发布

将技能包发布到 FangHub GitHub release：

```bash
librefang skill publish /path/to/my-skill
# 预览包内容但不上传
librefang skill publish /path/to/my-skill --dry-run
```

这会验证清单，将技能打包为 zip 包，并上传到配置的 GitHub release 仓库以进行 FangHub 分发。

---

## CLI 命令

### 完整的技能命令参考

```bash
# 安装技能（本地目录、FangHub 名称或 git URL）
librefang skill install <source>

# 列出所有已安装的技能
librefang skill list

# 移除已安装的技能
librefang skill remove <name>

# 在 FangHub 上搜索技能
librefang skill search <query>

# 本地验证技能并可选执行一个工具
librefang skill test [path] [--tool <name>] [--input <json>]

# 打包并发布技能包到 FangHub
librefang skill publish [path] [--repo <owner/name>] [--tag <tag>] [--dry-run]

# 创建新的技能脚手架（交互式）
librefang skill create
```

### 创建技能脚手架

```bash
librefang skill create
```

此交互式命令会提示输入：
- 技能名称
- 描述
- 运行时类型（python/node/wasm）

它会生成：

```
~/.librefang/skills/my-skill/
  skill.toml        # 预填充的清单
  src/
    main.py         # 初始入口点（Python 技能）
```

生成的入口点包含一个可工作的模板，从 stdin 读取 JSON 并将 JSON 写入 stdout。

### 在智能体清单中使用技能

在智能体清单的 `skills` 字段中引用技能：

```toml
name = "my-assistant"
version = "0.4.0"
description = "An assistant with extra skills"
author = "librefang"
module = "builtin:chat"
skills = ["web-summarizer", "data-analyzer"]

[model]
provider = "groq"
model = "llama-3.3-70b-versatile"

[capabilities]
tools = ["file_read", "web_fetch", "summarize_url"]
memory_read = ["*"]
memory_write = ["self.*"]
```

内核在智能体生成时加载技能工具和 prompt，并将它们与智能体的基础能力合并。

---

## OpenClaw 兼容性

LibreFang 可以安装和运行 OpenClaw 格式的技能。技能安装器会自动检测 OpenClaw 技能（通过查找 `package.json` + `index.ts`/`index.js`）并进行转换。

### 自动转换

```bash
librefang skill install /path/to/openclaw-skill
```

如果目录包含 OpenClaw 风格的技能（Node.js 包），LibreFang 会：

1. 检测 OpenClaw 格式。
2. 从 `package.json` 生成 `skill.toml` 清单。
3. 将工具名称映射为 LibreFang 约定。
4. 将技能复制到 LibreFang 技能目录。

### 手动转换

如果自动转换不起作用，可以手动创建 `skill.toml`：

```toml
[skill]
name = "my-openclaw-skill"
version = "1.0.0"
description = "Converted from OpenClaw"

[runtime]
type = "node"
entry = "index.js"

[[tools.provided]]
name = "my_tool"
description = "Tool description"
input_schema = { type = "object", properties = { input = { type = "string" } }, required = ["input"] }
```

将此文件放在现有的 `index.js`/`index.ts` 旁边并安装：

```bash
librefang skill install /path/to/skill-directory
```

通过 `librefang migrate --from openclaw` 导入的技能也会被扫描并记录在迁移报告中，附带手动重新安装的说明。

---

## 最佳实践

1. **保持技能专注** -- 一个技能应该只做好一件事。
2. **声明最少的需求** -- 只请求你的技能实际需要的工具和能力。
3. **使用描述性的工具名称** -- LLM 通过读取工具名称和描述来决定何时使用它。
4. **提供清晰的输入 schema** -- 为每个参数包含描述，以便 LLM 知道应该传递什么。
5. **优雅地处理错误** -- 始终返回 JSON 错误对象，而不是让程序崩溃。
6. **谨慎管理版本** -- 使用语义版本控制；破坏性变更需要升级主版本号。
7. **在多个智能体上测试** -- 验证你的技能在不同的智能体模板和提供商下都能正常工作。
8. **包含 README** -- 记录安装步骤、依赖项和使用示例。
