<p align="center">
  <img src="public/assets/logo.png" width="160" alt="LibreFang Logo" />
</p>

<h1 align="center">LibreFang</h1>
<h3 align="center">コミュニティメンテナンス型 Agent OS</h3>

<p align="center">
  Rustで書かれたオープンソースAgent OS。137Kコード行。14個のcrate。1767+テスト。ゼロclippy警告。<br/>
  <strong>`RightNow-AI/openfang`からフォーク。透明なガバナンス。帰属を保持。既存の`librefang` CLIと互換性あり。</strong>
</p>

<p align="center">
  <strong>多言語バージョン：</strong> <a href="README.md">English</a> | <a href="README.zh.md">中文</a> | <a href="README.ja.md">日本語</a> | <a href="README.ko.md">한국어</a> | <a href="README.es.md">Español</a> | <a href="README.de.md">Deutsch</a>
</p>

<p align="center">
  <a href="https://librefang.ai/">ウェブサイト</a> &bull;
  <a href="https://github.com/librefang/librefang">GitHub</a> &bull;
  <a href="GOVERNANCE.md">ガバナンス</a> &bull;
  <a href="CONTRIBUTING.md">コントリビューション</a> &bull;
  <a href="SECURITY.md">セキュリティ</a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/language-Rust-orange?style=flat-square" alt="Rust" />
  <img src="https://img.shields.io/badge/license-MIT-blue?style=flat-square" alt="MIT" />
  <img src="https://img.shields.io/badge/community-maintained-brightgreen?style=flat-square" alt="コミュニティメンテナンス" />
  <img src="https://img.shields.io/github/stars/librefang/librefang?style=flat-square" alt="Stars" />
  <img src="https://img.shields.io/github/forks/librefang/librefang?style=flat-square" alt="Forks" />
</p>

---

> **LibreFangは[`RightNow-AI/openfang`](https://github.com/RightNow-AI/openfang)のコミュニティメンテナンスフォークです。**
>
> コードベース、バイナリ、crate名、設定パスは`librefang`を使用します。LibreFangは異なるプロジェクトガバナンスを行っています：私たちはコミュニティのコントリビューションを積極的に受け入れ、公開でレビューし、通常のGitHubフローでマージし、コードを改変したときはコントリビューターの帰属を保持します。

> **コミュニティステータス：** issue、PR、レビュアー、メンテナーの募集を歓迎します。

---

## LibreFangとは？

LibreFangは**オープンソースAgent OS**です — チャットbotフレームワークではなく、LLMをラップしたPythonでもなく、「マルチエージェントオーケストレータ」。Rustでゼロから構築された自律型エージェントのための完全なオペレーティングシステムです。

従来のエージェントフレームワークはあなたの入力を待ちます。LibreFangは**あなたのために働く自律型エージェント**を実行します — スケジュールに従って、24時間365日稼働し、ナレッジグラフを構築し、ターゲットを監視し、リードを生成し、ソーシャルメディアを管理し、ダッシュボードに結果を報告します。

プロジェクトウェブサイトは[librefang.ai](https://librefang.ai/)で公開中です。LibreFangを試す最快の方法は今もソースからのインストールです。

```bash
cargo install --git https://github.com/librefang/librefang librefang-cli
librefang init
librefang start
# ダッシュボード：http://localhost:4545
```

**または Homebrew でインストール：**
```bash
brew tap librefang/tap
brew install librefang
```

---

## コア機能

### 🤖 Hands：実際にタスクを実行するエージェント

*"従来のエージェントはあなたの入力を待ちます。Handsはあなたのために働きます。"*

**Hands**はLibreFangのコアイノベーション — 事前に構築された自律型能力パッケージで、独立して実行され、スケジュールに従って、あなたにプロンプトを入力させることなく動作します。これはチャットbotではありません。これは朝6時に起きて、競合他社を研究し、ナレッジグラフを構築し、発見を評価し、あなたのコーヒーを飲む前にレポートをTelegramに送ってくるエージェントです。

各Handには以下が含まれます：
- **HAND.toml** — ツール、要件、ダッシュボード指標を宣言するマニフェスト
- **System Prompt** — 多段階オペレーションマニュアル（一行ではなく、500語以上の専門家手続き）
- **SKILL.md** — ランタイムにコンテキストに注入されるドメイン専門知識リファレンス
- **Guardrails** — 機密操作の承認ゲート（例：Browser Handは購入前に承認が必要）

すべてバイナリにコンパイルされます。ダウンロード不要、pip install不要、Docker pull不要。

### 7つのバンドルされたHands

| Hand | 機能 |
|------|------|
| **Clip** | YouTube URLを取得、ダウンロード、最高瞬間を識別、字幕とサムネイル付きの短い縦型ビデオに裁断、オプションでAIナレーションを追加、TelegramとWhatsAppに公開。8段階パイプライン。FFmpeg + yt-dlp + 5 STTバックエンド。 |
| **Lead** | 毎日実行。ICPに一致する潜在顧客を発見、Webリサーチでエンリッチ、0-100でスコア付け、既存データベースと重複排除、CSV/JSON/Markdownで適格リードを配信。時間とともにICPプロファイルを構築。 |
| **Collector** | OSINTグレードのインテリジェンス。ターゲットを与える（会社、人、トピック）。継続的に監視 — 変更検出、センチメント追跡、ナレッジグラフ構築、重要な変化時にクリティカルアラートを配信。 |
| **Predictor** | スーパフォーキャスティングエンジン。複数のソースから信号を収集、校准推理チェーンを構築、置信区間で予測、独自の精度をBrierスコアで追跡。反対モードあり — 意図的にコンセンサスに異議を唱える。 |
| **Researcher** | 深い自律的研究者。複数のソースを相互参照、CRAAP基準（通貨性、相関性、権威性、正確性、目的）で信頼性を評価、引用付きAPAフォーマットレポートを生成、多言語サポート。 |
| **Twitter** | 自律的Twitter/Xアカウントマネージャー。7つのローテーション形式でコンテンツを作成、最適なエンゲージメントのために投稿をスケジュール、メンションに返信、パフォーマンス指標を跟踪。承認キューあり — あなたのOKなしでは投稿しません。 |
| **Browser** | Web自動化エージェント。サイトをナビゲート、フォームに入力、ボタンをクリック、複数ステップワークフローを処理。Playwrightブリッジとセッション永続化を使用。**強制購入承認ゲート** — 明確な確認なしにあなたのお金を使うことはありません。 |

---

## 16層のセキュリティシステム — 多層防御

LibreFangは後付けでセキュリティを追加しません。每一層が独立してテスト可能で、単一障害点なしで動作します。

| # | システム | 機能 |
|---|---------|------|
| 1 | **WASM二層メーターサンドボックス** | ツールコードは燃料メーター + epoch中断付きのWebAssemblyで実行。ウォッチドレッドが暴走コードをkill。 |
| 2 | **Merkleハッシュチェーン監査トレイル** | 各操作は暗号化で前のものにリンク。1つのエントリを改ざんするとチェーン全体が破損。 |
| 3 | **情報フローテイント追跡** | ラベルが実行中传播 — ソースからシンクまでsecretsを追跡。 |
| 4 | **Ed25519署名エージェントマニフェスト** | 各エージェントのアイデンティティと能力セットは暗号化署名済み。 |
| 5 | **SSRF保護** | プライベートIP、クラウドメタデータエンドポイント、DNS rebinding攻撃をブロック。 |
| 6 | **Secretゼロ化** | `Zeroizing<String>`が不要になった瞬間にAPIキーをメモリから即座にワイプ。 |
| 7 | **OFP相互認証** | HMAC-SHA256 nonceベース、P2Pネットワーキング用の定数時間検証。 |
| 8 | **キャパビリティゲート** | 役割ベースアクセス制御 — エージェントが所需ツールを宣言、カーネルが強制。 |
| 9 | **セキュリティヘッダー** | CSP、X-Frame-Options、HSTS、X-Content-Type-Options、すべてのレスポンスに適用。 |
| 10 | **ヘルスエンドポイント修整** | パブリックヘルスチェックは最小情報を返す。完全診断には認証が必要。 |
| 11 | **サブプロセスサンドボックス** | `env_clear()` + 選択的変数パススルー。クロスプラットフォームkillを持つプロセスツリー分離。 |
| 12 | **プロンプトインジェクションスキャナー** | オーバーライド試行、データ抽出パターン、スキル内のシェル参照インジェクションを検出。 |
| 13 | **ループガード** | SHA256ベースのツール呼び出しループ検出とサーキットブレーカー。ping-pongパターンを処理。 |
| 14 | **セッション修復** | 7段階メッセージ履歴検証と破損からの自動回復。 |
| 15 | **パストラバーサル防止** | 正規化とシンボリックリンクエスケープ防止。`../`はここでは機能しません。 |
| 16 | **GCRAレートリミッター** | コスト認識のトークンバケットレートリミット、per-IP追跡と古いクリーンアップ付き。 |

---

## アーキテクチャ

14個のRust crate。137,728行のコード。モジュール式カーネルデザイン。

```
librefang-kernel      オーケストレーション、ワークフロー、计量、RBAC、スケジューラー、予算追跡
librefang-runtime     エージェントループ、3つのLLM驱动、53ツール、WASMサンドボックス、MCP、A2A
librefang-api         140+ REST/WS/SSEエンドポイント、OpenAI互換API、ダッシュボード
librefang-channels    40メッセージアダプター、レートリミット付き
librefang-memory      SQLite永続化、ベクトル埋め込み、カノニカルセッション、compaction
librefang-types       コアタイプ、テイント追跡、Ed25519マニフェスト署名、モデルカタログ
librefang-skills      60バンドルスキル、SKILL.mdパーサー、FangHubマーケットプレイス
librefang-hands       7つの自律型Hands、HAND.tomlパーサー、ライフサイクル管理
librefang-extensions  25 MCPテンプレート、AES-256-GCM資格情報ボールト、OAuth2 PKCE
librefang-wire        OFP P2Pプロトコル、HMAC-SHA256相互認証付き
librefang-cli         CLI、Daemon管理、TUIダッシュボード、MCPサーバーモード
librefang-desktop     Tauri 2.0ネイティブアプリ（システムトレイ、通知、グローバルショートカット）
librefang-migrate     OpenClaw、LangChain、AutoGPT移行エンジン
xtask                ビルド自動化
```

---

## クイックスタート

```bash
# 1. インストール
cargo install --git https://github.com/librefang/librefang librefang-cli

# 2. 初期化 — プロバイダー設定ウォークスルー
librefang init

# 3. デーモン起動
librefang start

# 4. ダッシュボード：http://localhost:4545

# 5. Handをアクティブ化 — あなたのために働き始める
librefang hand activate researcher

# 6. エージェントとチャット
librefang chat researcher
> "AIエージェントフレームワークの新兴トレンドは？"

# 7. 事前構築エージェントをスポーン
librefang agent spawn coder
```

---

## 開発

```bash
# ワークスペースビルド
cargo build --workspace --lib

# 全テスト実行 (1767+)
cargo test --workspace

# Lint（警告ゼロ必須）
cargo clippy --workspace --all-targets -- -D warnings

# フォーマット
cargo fmt --all -- --check
```

---

## 安定性に関する注意

LibreFangはpre-1.0です。アーキテクチャは堅実、テストスイートは包括的、セキュリティモデルは包括的。也就是：

- **破壊的変更**はv1.0までのマイナーバージョン間で発生する可能性あり
- **一部のHands**は他よりも成熟している（BrowserとResearcherが最も实战経験済み）
- **エッジケース**は存在します — 発見したら[issueを開いて](https://github.com/librefang/librefang/issues)
- v1.0まで本番デプロイでは**特定のコミットにピン留め**を

私たちは快速リリース、快速修正。目標：2026年中に堅実なv1.0をリリース。

---

## セキュリティ

セキュリティ脆弱性を報告するには[SECURITY.md](SECURITY.md)の私人レポート流程に従ってください。

---

## ライセンス

MITライセンス。LICENSEファイルを参照してください。

---

## リンク

- [GitHub](https://github.com/librefang/librefang)
- [ウェブサイト](https://librefang.ai/)
- [ドキュメント](https://docs.librefang.ai)
- [コントリビューションガイド](CONTRIBUTING.md)
- [ガバナンス](GOVERNANCE.md)
- [メンテナー](MAINTAINERS.md)
- [セキュリティポリシー](SECURITY.md)

---

<p align="center">
  <strong>Rustで構築。16層セキュリティ。実際にあなたのために働くエージェント。</strong>
</p>
