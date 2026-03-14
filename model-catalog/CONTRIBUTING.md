# Contributing to LibreFang Model Catalog

Thank you for helping keep the model catalog up to date! This guide explains how to add or update model entries.

## How to Add a New Model

### 1. Fork & Clone

```bash
git clone https://github.com/<your-fork>/model-catalog.git
cd model-catalog
```

### 2. Find the Right Provider File

Each provider has its own file in `providers/`. For example:
- OpenAI models go in `providers/openai.toml`
- Anthropic models go in `providers/anthropic.toml`

If the provider doesn't exist yet, create a new file (e.g. `providers/newprovider.toml`) with the `[provider]` section and your `[[models]]` entries.

### 3. Add Your Model Entry

Append a `[[models]]` block to the provider file:

```toml
[[models]]
id = "new-model-id"              # The exact API model ID
display_name = "New Model Name"  # Human-readable name
tier = "smart"                   # frontier | smart | balanced | fast | local
context_window = 128000
max_output_tokens = 16384
input_cost_per_m = 2.50          # USD per million input tokens
output_cost_per_m = 10.0         # USD per million output tokens
supports_tools = true
supports_vision = false
supports_streaming = true
aliases = []                     # Optional short names
```

### 4. Validate

```bash
python scripts/validate.py
```

This checks:
- All TOML files parse correctly
- Required fields are present
- Tier values are valid
- Costs are non-negative
- No duplicate model IDs

### 5. Submit a Pull Request

Push your branch and open a PR. The PR template will guide you through the checklist.

## Where to Find Model Information

- **OpenAI**: https://openai.com/pricing
- **Anthropic**: https://docs.anthropic.com/en/docs/about-claude/models
- **Google Gemini**: https://ai.google.dev/pricing
- **DeepSeek**: https://platform.deepseek.com/api-docs/pricing
- **Mistral**: https://mistral.ai/technology/#pricing
- **Groq**: https://wow.groq.com/
- **xAI**: https://docs.x.ai/docs
- **Cohere**: https://cohere.com/pricing
- **Together**: https://www.together.ai/pricing
- **Fireworks**: https://fireworks.ai/pricing
- **Perplexity**: https://docs.perplexity.ai/guides/pricing

## Tier Definitions

| Tier | Description | Examples |
|------|-------------|----------|
| `frontier` | Most capable, cutting-edge | Claude Opus, GPT-4.1, Gemini 2.5 Pro |
| `smart` | Smart and cost-effective | Claude Sonnet, GPT-4o, Gemini 2.5 Flash |
| `balanced` | Balanced speed and cost | GPT-4.1 Mini, Llama 3.3 70B |
| `fast` | Fastest, cheapest | GPT-4o Mini, Claude Haiku, Gemma 2 9B |
| `local` | Local models, zero cost | Ollama, vLLM, LM Studio |

## Guidelines

- **Pricing must be in USD per million tokens** -- convert from other units if needed
- **Use the exact model ID** that the provider's API expects
- **Don't guess** -- only add data you can verify from official sources
- **One provider per file** -- don't mix providers in a single TOML file
- **Keep aliases short** -- 1-3 word abbreviations that users would naturally type

---

# 贡献指南

感谢您帮助维护模型目录！以下是添加或更新模型条目的方法。

## 如何添加新模型

1. Fork 并克隆此仓库
2. 在 `providers/` 目录中找到对应的提供商文件
3. 添加 `[[models]]` 条目（参考上方英文模板）
4. 运行 `python scripts/validate.py` 验证
5. 提交 Pull Request

## 定价说明

- 所有价格均以 **美元/百万 tokens** 为单位
- 免费模型和本地模型的价格为 `0.0`
- 请从官方定价页面获取准确数据
