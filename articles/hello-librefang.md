---
title: "Hello, LibreFang! — オープンソース Agent OS の紹介"
emoji: "🦊"
type: "tech"
topics: ["rust", "ai", "agent", "opensource", "librefang"]
published: true
---

# Hello, LibreFang!

LibreFang は Rust で構築されたオープンソースの Agent Operating System です。

## なぜ LibreFang？

"Libre" は自由を意味します。オープンソースプロジェクトは、ライセンスだけでなく、ガバナンス、コントリビューション、コラボレーションにおいても真にオープンであるべきだと考えています。

## 特徴

- **14 の Rust crate** — モジュラー、高速、メモリ安全
- **40 のメッセージングチャネル** — Telegram, Discord, Slack など
- **60 のバンドルスキル** — すぐに使える
- **16 のセキュリティレイヤー** — WASM サンドボックス、RBAC、監査証跡
- **2,100+ テスト** — Clippy 警告ゼロ

## クイックスタート

```bash
export GROQ_API_KEY="your-key"
librefang init && librefang start
# http://127.0.0.1:4545 を開く
```

## コントリビューション

Rust の経験がなくても貢献できます：

| 種類 | Rust 必要？ |
|------|-----------|
| Agent テンプレート (TOML) | 不要 |
| スキル (Python/JS) | 不要 |
| ドキュメント / 翻訳 | 不要 |
| チャネルアダプター | 必要 |
| コア機能 | 必要 |

## リンク

- [GitHub](https://github.com/librefang/librefang)
- [Discord](https://discord.gg/DzTYqAZZmc)
- [Contributing Guide](https://github.com/librefang/librefang/blob/main/CONTRIBUTING.md)
- [Good First Issues](https://github.com/librefang/librefang/labels/good%20first%20issue)
