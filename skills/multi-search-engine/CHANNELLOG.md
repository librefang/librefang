# Multi Search Engine

## 基本信息

- **名称**: multi-search-engine
- **版本**: v2.0.1
- **描述**: 集成16个搜索引擎（7国内+9国际），支持高级搜索语法
- **发布时间**: 2026-02-06

## 搜索引擎

**国内（7个）**: 百度、必应CN、必应INT、360、搜狗、微信、神马
**国际（9个）**: Google、Google HK、DuckDuckGo、Yahoo、Startpage、Brave、Ecosia、Qwant、WolframAlpha

## 核心功能

- 高级搜索操作符（site:, filetype:, intitle:等）
- DuckDuckGo Bangs快捷命令
- 时间筛选（小时/天/周/月/年）
- 隐私保护搜索
- WolframAlpha知识计算

## 更新记录

### v2.0.1 (2026-02-06)
- 精简文档，优化发布

### v2.0.0 (2026-02-06)
- 新增9个国际搜索引擎
- 强化深度搜索能力

### v1.0.0 (2026-02-04)
- 初始版本：8个国内搜索引擎

## 使用示例

```javascript
// Google搜索
web_fetch({"url": "https://www.google.com/search?q=python"})

// 隐私搜索
web_fetch({"url": "https://duckduckgo.com/html/?q=privacy"})

// 站内搜索
web_fetch({"url": "https://www.google.com/search?q=site:github.com+python"})
```

MIT License
