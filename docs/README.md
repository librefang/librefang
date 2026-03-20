# LibreFang Docs

LibreFang 官方文档网站。

## 开发

```bash
# 安装依赖
pnpm install

# 开发服务器
pnpm dev

# 构建静态站点
pnpm build
```

## 添加新文档

1. 在 `src/app/` 下创建新目录，如 `src/app/new-page/`
2. 添加 `page.mdx` 文件
3. 添加 `export const sections = [];` 到文件末尾用于导航

## 多语言

- `/` - 中文（默认）
- `/en/` - 英文（从 LibreFang 仓库同步）

## 部署

自动部署到 Cloudflare Pages。
