# LibreFang Model Catalog

Community-maintained model metadata catalog for [LibreFang](https://github.com/librefang/librefang) -- the open-source Agent Operating System.

This repository is the source of truth for model metadata (pricing, context windows, capabilities). When new models are released (e.g. GPT-5.5, Claude 5), anyone can submit a PR here without touching the LibreFang binary.

## Structure

```
model-catalog/
├── providers/          # One TOML file per provider
│   ├── anthropic.toml
│   ├── openai.toml
│   ├── gemini.toml
│   └── ...
├── aliases.toml        # Global alias mappings (e.g. "sonnet" -> "claude-sonnet-4-6")
├── schema.toml         # Reference schema documenting all fields
├── scripts/
│   └── validate.py     # Validation script
├── CONTRIBUTING.md     # How to add a new model
└── LICENSE             # MIT
```

## How LibreFang Uses This Catalog

LibreFang ships with a built-in model catalog compiled into the binary. This repository serves as the upstream source. To update your local catalog:

```bash
librefang catalog update
```

This fetches the latest TOML files from this repository and merges them into your local catalog.

### Custom Local Models

You can also add custom models locally without submitting a PR:

```bash
# Add to your personal config
# ~/.librefang/model_catalog.toml

[[models]]
id = "my-custom-model"
display_name = "My Custom Model"
provider = "ollama"
tier = "local"
context_window = 32768
max_output_tokens = 4096
input_cost_per_m = 0.0
output_cost_per_m = 0.0
supports_tools = true
supports_vision = false
supports_streaming = true
```

## Schema Reference

Each provider file contains a `[provider]` section and one or more `[[models]]` entries:

```toml
[provider]
id = "provider-id"                  # Unique provider identifier
display_name = "Provider Name"      # Human-readable name
api_key_env = "PROVIDER_API_KEY"    # Environment variable for API key
base_url = "https://api.example.com"  # Default API endpoint
key_required = true                 # Whether an API key is needed

[[models]]
id = "model-id"                    # Unique model identifier (API model ID)
display_name = "Human Name"        # Human-readable display name
tier = "smart"                     # frontier | smart | balanced | fast | local
context_window = 128000            # Maximum input tokens
max_output_tokens = 16384          # Maximum output tokens
input_cost_per_m = 2.50            # USD per million input tokens
output_cost_per_m = 10.0           # USD per million output tokens
supports_tools = true              # Tool/function calling support
supports_vision = true             # Vision/image input support
supports_streaming = true          # Streaming response support
aliases = ["alias1", "alias2"]     # Short names for this model
```

### Tier Definitions

| Tier | Description | Examples |
|------|-------------|----------|
| `frontier` | Most capable, cutting-edge models | Claude Opus, GPT-4.1, Gemini 2.5 Pro |
| `smart` | Smart, cost-effective models | Claude Sonnet, GPT-4o, Gemini 2.5 Flash |
| `balanced` | Balanced speed/cost | GPT-4.1 Mini, Llama 3.3 70B |
| `fast` | Fastest, cheapest | GPT-4o Mini, Claude Haiku |
| `local` | Local models (zero cost) | Ollama, vLLM, LM Studio |

## How to Add a New Model

1. Edit the appropriate provider file in `providers/`
2. Run validation: `python scripts/validate.py`
3. Submit a Pull Request

See [CONTRIBUTING.md](CONTRIBUTING.md) for detailed instructions.

## Validation

```bash
python scripts/validate.py
```

This checks all TOML files for correctness: required fields, valid tiers, non-negative costs, no duplicate IDs.

## Current Stats

- **30+ providers** including Anthropic, OpenAI, Google, DeepSeek, Groq, Mistral, xAI, and more
- **190+ models** with pricing, context windows, and capability flags
- **80+ aliases** for quick model selection

## License

MIT License. See [LICENSE](LICENSE).

---

# LibreFang 模型目录

社区维护的 [LibreFang](https://github.com/librefang/librefang) 模型元数据目录 -- 开源 Agent 操作系统。

本仓库是模型元数据（定价、上下文窗口、能力标记）的唯一数据源。当新模型发布时（如 GPT-5.5、Claude 5），任何人都可以在这里提交 PR，而无需修改 LibreFang 二进制文件。

## 目录结构

```
model-catalog/
├── providers/          # 每个提供商一个 TOML 文件
│   ├── anthropic.toml
│   ├── openai.toml
│   ├── gemini.toml
│   └── ...
├── aliases.toml        # 全局别名映射（如 "sonnet" -> "claude-sonnet-4-6"）
├── schema.toml         # 字段定义参考
├── scripts/
│   └── validate.py     # 验证脚本
├── CONTRIBUTING.md     # 如何添加新模型
└── LICENSE             # MIT 许可证
```

## LibreFang 如何使用此目录

LibreFang 内置了编译到二进制文件中的模型目录。本仓库作为上游数据源。更新本地目录：

```bash
librefang catalog update
```

### 本地自定义模型

您也可以在本地添加自定义模型，无需提交 PR：

```bash
# 编辑个人配置文件
# ~/.librefang/model_catalog.toml

[[models]]
id = "my-custom-model"
display_name = "我的自定义模型"
provider = "ollama"
tier = "local"
context_window = 32768
max_output_tokens = 4096
input_cost_per_m = 0.0
output_cost_per_m = 0.0
```

## 如何添加新模型

1. 编辑 `providers/` 中对应的提供商文件
2. 运行验证：`python scripts/validate.py`
3. 提交 Pull Request

详细说明请参考 [CONTRIBUTING.md](CONTRIBUTING.md)。

## 验证

```bash
python scripts/validate.py
```

检查所有 TOML 文件的正确性：必填字段、有效的层级值、非负成本、无重复 ID。

## 许可证

MIT 许可证。详见 [LICENSE](LICENSE)。
