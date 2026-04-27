# --- API エラーメッセージ（日本語） ---

# エージェントエラー
api-error-agent-not-found = エージェントが見つかりません
api-error-agent-spawn-failed = エージェントの作成に失敗しました
api-error-agent-invalid-id = 無効なエージェント ID
api-error-agent-already-exists = エージェントは既に存在します
api-error-agent-no-workspace = エージェントにワークスペースがありません
api-error-agent-not-found-or-terminated = エージェントが見つからないか、既に終了しています
api-error-agent-vanished = 更新中にエージェントが消失しました
api-error-agent-no-agents-available = 利用可能なエージェントがありません
api-error-agent-no-target = ターゲットエージェントが見つかりません。agent_id を指定するか、先にエージェントを起動してください。
api-error-agent-source-not-found = 送信元エージェントが見つかりません
api-error-agent-target-not-found = 送信先エージェントが見つかりません
api-error-agent-execution-failed = エージェントの実行に失敗しました: { $error }
api-error-agent-clone-spawn-failed = クローンの作成に失敗しました: { $error }
api-error-agent-error = エージェントエラー: { $error }
api-error-agent-not-found-with-id = エージェントが見つかりません: { $id }

# メッセージエラー
api-error-message-too-large = メッセージが大きすぎます（最大 64KB）
api-error-message-delivery-failed = メッセージの配信に失敗しました: { $reason }
api-error-message-required = メッセージは必須です
api-error-message-missing-field = 'message' フィールドがありません
api-error-message-streaming-failed = ストリーミングメッセージの送信に失敗しました

# テンプレートエラー
api-error-template-invalid-name = 無効なテンプレート名
api-error-template-not-found = テンプレート '{ $name }' が見つかりません
api-error-template-parse-failed = テンプレートの解析に失敗しました: { $error }
api-error-template-required = 'manifest_toml' または 'template' が必要です
api-error-template-invalid-manifest = 無効なテンプレートマニフェスト
api-error-template-read-failed = テンプレートの読み込みに失敗しました

# マニフェストエラー
api-error-manifest-too-large = マニフェストが大きすぎます（最大 1MB）
api-error-manifest-invalid-format = 無効なマニフェスト形式
api-error-manifest-signature-mismatch = 署名されたマニフェストの内容が manifest_toml と一致しません
api-error-manifest-signature-failed = マニフェストの署名検証に失敗しました
api-error-manifest-invalid = 無効なマニフェスト: { $error }

# 認証エラー
api-error-auth-invalid-key = 無効な API キー
api-error-auth-missing-header = Authorization: Bearer <api_key> ヘッダーがありません
api-error-auth-missing = このプロバイダーのAPIキーが設定されていません

# セッションエラー
api-error-session-load-failed = セッションの読み込みに失敗しました
api-error-session-not-found = セッションが見つかりません
api-error-session-invalid-id = 無効なセッション ID
api-error-session-no-label = そのラベルのセッションが見つかりません
api-error-session-cleanup-expired-failed = 期限切れクリーンアップに失敗しました: { $error }
api-error-session-cleanup-excess-failed = 超過クリーンアップに失敗しました: { $error }

# ワークフローエラー
api-error-workflow-missing-steps = 'steps' 配列がありません
api-error-workflow-step-needs-agent = ステップ '{ $step }' には 'agent_id' または 'agent_name' が必要です
api-error-workflow-invalid-id = 無効なワークフロー ID
api-error-workflow-execution-failed = ワークフローの実行に失敗しました
api-error-workflow-not-found = ワークフローが見つかりません

# トリガーエラー
api-error-trigger-missing-agent-id = 'agent_id' がありません
api-error-trigger-invalid-agent-id = 無効な agent_id
api-error-trigger-invalid-pattern = 無効なトリガーパターン
api-error-trigger-missing-pattern = 'pattern' がありません
api-error-trigger-registration-failed = トリガーの登録に失敗しました（エージェントが見つかりません？）
api-error-trigger-invalid-id = 無効なトリガー ID
api-error-trigger-not-found = トリガーが見つかりません

# 予算エラー
api-error-budget-invalid-amount = 無効な予算額
api-error-budget-update-failed = 予算の更新に失敗しました
api-error-budget-provide-at-least-one = 以下のうち少なくとも 1 つを指定してください: max_cost_per_hour_usd, max_cost_per_day_usd, max_cost_per_month_usd, max_llm_tokens_per_hour

# 設定エラー
api-error-config-parse-failed = 設定の解析に失敗しました: { $error }
api-error-config-write-failed = 設定の書き込みに失敗しました: { $error }
api-error-config-save-failed = 設定の保存に失敗しました: { $error }
api-error-config-remove-failed = 設定の削除に失敗しました: { $error }
api-error-config-missing-toml = toml_content フィールドがありません

# プロファイルエラー
api-error-profile-not-found = プロファイル '{ $name }' が見つかりません

# スケジュールタスクエラー
api-error-cron-invalid-id = 無効なスケジュールタスク ID
api-error-cron-not-found = スケジュールタスクが見つかりません
api-error-cron-create-failed = スケジュールタスクの作成に失敗しました: { $error }
api-error-cron-invalid-expression = 無効な cron 式
api-error-cron-invalid-expression-detail = 無効な cron 式: 5 つのフィールドが必要です（分 時 日 月 曜日）
api-error-cron-missing-field = 'cron' フィールドがありません

# 目標エラー
api-error-goal-not-found = 目標が見つかりません
api-error-goal-not-found-with-id = 目標 '{ $id }' が見つかりません
api-error-goal-missing-title = 'title' フィールドがないか空です
api-error-goal-title-too-long = タイトルが長すぎます（最大 256 文字）
api-error-goal-description-too-long = 説明が長すぎます（最大 4096 文字）
api-error-goal-invalid-status = 無効なステータス。pending、in_progress、completed、cancelled のいずれかである必要があります
api-error-goal-progress-range = 進捗は 0-100 の範囲である必要があります
api-error-goal-parent-not-found = 親目標 '{ $id }' が見つかりません
api-error-goal-self-parent = 目標を自身の親にすることはできません
api-error-goal-circular-parent = 循環する親参照が検出されました
api-error-goal-save-failed = 目標の保存に失敗しました: { $error }
api-error-goal-update-failed = 目標の更新に失敗しました: { $error }
api-error-goal-delete-failed = 目標の削除に失敗しました: { $error }
api-error-goal-load-failed = 目標の読み込みに失敗しました: { $error }
api-error-goal-title-empty = タイトルは空にできません
api-error-goal-status-invalid = 無効なステータス

# メモリエラー
api-error-memory-not-enabled = プロアクティブメモリが有効になっていません
api-error-memory-not-found = メモリが見つかりません
api-error-memory-operation-failed = メモリ操作に失敗しました
api-error-memory-export-failed = メモリのエクスポートに失敗しました
api-error-memory-import-failed = クリア中にメモリのインポートに失敗しました
api-error-memory-key-not-found = キーが見つかりません
api-error-memory-missing-kv = リクエストボディに 'kv' オブジェクトがないか無効です
api-error-memory-serialization-error = シリアライズエラー
api-error-memory-missing-ids = 'ids' 配列がありません

# ネットワーク / A2A エラー
api-error-network-not-enabled = ピアネットワークが有効になっていません
api-error-network-peer-not-found = ピアが見つかりません
api-error-network-a2a-not-found = A2A エージェント '{ $url }' が見つかりません
api-error-network-connection-failed = 接続に失敗しました: { $error }
api-error-network-auth-failed = 認証に失敗しました (HTTP { $status })
api-error-network-task-post-failed = タスクの送信に失敗しました: { $error }
api-error-network-missing-url = 'url' クエリパラメータがありません

# プラグインエラー
api-error-plugin-missing-name = 'name' がありません
api-error-plugin-missing-name-registry = レジストリインストールに 'name' がありません
api-error-plugin-missing-path = ローカルインストールに 'path' がありません
api-error-plugin-missing-url = Git インストールに 'url' がありません
api-error-plugin-invalid-source = 無効なソース。'registry'、'local'、'git' のいずれかを使用してください

# チャネルエラー
api-error-channel-unknown = 不明なチャネル
api-error-channel-missing-agent-id = 必須フィールドがありません: agent_id
api-error-channel-invalid-from = 無効な from_agent_id
api-error-channel-invalid-to = 無効な to_agent_id

# プロバイダーエラー
api-error-provider-missing-alias = 必須フィールドがありません: alias
api-error-provider-missing-model-id = 必須フィールドがありません: model_id
api-error-provider-missing-id = 必須フィールドがありません: id
api-error-provider-missing-key = 'key' フィールドがないか空です
api-error-provider-alias-exists = エイリアス '{ $alias }' は既に存在します
api-error-provider-alias-not-found = エイリアス '{ $alias }' が見つかりません
api-error-provider-model-not-found = モデル '{ $id }' が見つかりません
api-error-provider-not-found = プロバイダー '{ $name }' が見つかりません
api-error-provider-model-exists = モデル '{ $id }' はプロバイダー '{ $provider }' に既に存在します
api-error-provider-custom-model-not-found = カスタムモデル '{ $id }' が見つかりません
api-error-provider-no-key-required = このプロバイダーは API キーを必要としません
api-error-provider-key-not-configured = プロバイダーの API キーが設定されていません
api-error-provider-secrets-write-failed = secrets.env の書き込みに失敗しました: { $error }
api-error-provider-secrets-update-failed = secrets.env の更新に失敗しました: { $error }
api-error-provider-invalid-url = 無効な URL 形式
api-error-provider-missing-url = 'url' がないか空です
api-error-provider-missing-base-url = 'base_url' フィールドがないか空です
api-error-provider-unknown = 不明なプロバイダー '{ $name }'
api-error-provider-base-url-invalid = base_url は http:// または https:// で始まる必要があります
api-error-provider-missing-model = 'model' フィールドがありません
api-error-provider-token-save-failed = トークンの保存に失敗しました: { $error }
api-error-provider-unknown-poll = 不明な poll_id
api-error-provider-secret-write-failed = シークレットの書き込みに失敗しました: { $error }

# スキルエラー
api-error-skill-missing-name = 'name' フィールドがないか空です
api-error-skill-invalid-name = スキル名には英数字、ハイフン、アンダースコアのみ使用できます
api-error-skill-not-found-source = このスキルのソースコードが見つかりません
api-error-skill-only-prompt = Web UI からはプロンプトのみのスキルだけ作成できます
api-error-skill-name-too-long = 名前が最大長を超えています（256 文字）
api-error-skill-description-too-long = 説明が最大長を超えています（{ $max } 文字）
api-error-skill-dir-create-failed = スキルディレクトリの作成に失敗しました: { $error }
api-error-skill-toml-write-failed = skill.toml の書き込みに失敗しました: { $error }
api-error-skill-install-failed = インストールに失敗しました: { $error }

# ハンドエラー
api-error-hand-not-found = ハンドが見つかりません: { $id }
api-error-hand-definition-not-found = ハンド定義が見つかりません
api-error-hand-instance-not-found = インスタンスが見つかりません

# MCP エラー
api-error-mcp-missing-name = 'name' フィールドがありません
api-error-mcp-missing-transport = 'transport' フィールドがありません
api-error-mcp-invalid-config = 無効な MCP サーバー設定: { $error }
api-error-mcp-not-found = MCP サーバー '{ $name }' が見つかりません
api-error-mcp-write-failed = 設定の書き込みに失敗しました: { $error }

# 統合/拡張エラー
api-error-integration-not-found = 統合 '{ $id }' が見つかりません
api-error-integration-missing-id = 'id' フィールドがありません
api-error-extension-not-found = 拡張 '{ $id }' が見つかりません

# システムエラー
api-error-system-cli-not-found = PATH に CLI が見つかりません

# KV / 構造化メモリエラー
api-error-kv-missing-fields = 'fields' オブジェクトがありません
api-error-kv-missing-value = 'value' フィールドがありません
api-error-kv-array-empty = 配列は空にできません
api-error-kv-missing-path = 'path' フィールドがありません

# 承認エラー
api-error-approval-invalid-id = 無効な承認 ID
api-error-approval-not-found = 承認が見つかりません

# Webhook エラー
api-error-webhook-not-enabled = Webhook トリガーが有効になっていません
api-error-webhook-invalid-id = 無効な Webhook ID
api-error-webhook-not-found = Webhook が見つかりません
api-error-webhook-missing-url = 'url' フィールドがありません
api-error-webhook-missing-events = 'events' 配列がありません
api-error-webhook-invalid-events = イベントタイプは文字列である必要があります
api-error-webhook-event-types-required = 少なくとも 1 つのイベントタイプが必要です
api-error-webhook-url-unreachable = Webhook URL に到達できません: { $error }
api-error-webhook-event-publish-failed = イベントの発行に失敗しました: { $error }

# バックアップエラー
api-error-backup-not-found = バックアップが見つかりません
api-error-backup-file-not-found = バックアップファイルが見つかりません
api-error-backup-invalid-filename = 無効なバックアップファイル名
api-error-backup-invalid-filename-zip = 無効なバックアップファイル名 — .zip ファイルである必要があります
api-error-backup-missing-manifest = バックアップアーカイブに manifest.json がありません — 有効な LibreFang バックアップではありません
api-error-backup-dir-create-failed = バックアップディレクトリの作成に失敗しました: { $error }
api-error-backup-file-create-failed = バックアップファイルの作成に失敗しました: { $error }
api-error-backup-finalize-failed = バックアップの完了に失敗しました: { $error }
api-error-backup-open-failed = バックアップの開封に失敗しました: { $error }
api-error-backup-invalid-archive = 無効なバックアップアーカイブ: { $error }
api-error-backup-delete-failed = バックアップの削除に失敗しました: { $error }

# スケジュールエラー
api-error-schedule-not-found = スケジュールが見つかりません
api-error-schedule-missing-cron = 'cron' フィールドがありません
api-error-schedule-missing-enabled = 'enabled' フィールドがありません
api-error-schedule-invalid-cron = 無効な cron 式
api-error-schedule-invalid-cron-detail = 無効な cron 式: 5 つのフィールドが必要です（分 時 日 月 曜日）
api-error-schedule-save-failed = スケジュールの保存に失敗しました: { $error }
api-error-schedule-update-failed = スケジュールの更新に失敗しました: { $error }
api-error-schedule-delete-failed = スケジュールの削除に失敗しました: { $error }
api-error-schedule-load-failed = スケジュールの読み込みに失敗しました: { $error }

# ジョブエラー
api-error-job-invalid-id = 無効なジョブ ID
api-error-job-not-found = ジョブが見つかりません
api-error-job-not-retryable = タスクが見つからないか、再試行可能な状態ではありません（完了または失敗である必要があります）
api-error-job-disappeared-cancel = キャンセル後にタスクが消失しました
api-error-job-disappeared-complete = 完了後にタスクが消失しました

# タスクエラー
api-error-task-not-found = タスクが見つかりません
api-error-task-disappeared = タスクが消失しました

# ペアリングエラー
api-error-pairing-not-enabled = ペアリングが有効になっていません
api-error-pairing-invalid-token = 無効またはトークンがありません

# バインディングエラー
api-error-binding-out-of-range = バインディングインデックスが範囲外です

# コマンドエラー
api-error-command-not-found = コマンド '{ $name }' が見つかりません

# ファイル/アップロードエラー
api-error-file-not-found = ファイルが見つかりません
api-error-file-not-in-whitelist = ファイルがホワイトリストにありません
api-error-file-too-large = ファイルが大きすぎます（最大 { $max }）
api-error-file-content-too-large = ファイルの内容が大きすぎます（最大 32KB）
api-error-file-empty-body = 空のファイル本文
api-error-file-save-failed = ファイルの保存に失敗しました
api-error-file-missing-filename = 'filename' フィールドがありません
api-error-file-missing-path = 'path' フィールドがありません
api-error-file-path-too-deep = パスが深すぎます（最大 3 階層）
api-error-file-path-traversal = パストラバーサルが拒否されました
api-error-file-unsupported-type = サポートされていないコンテンツタイプ。許可: image/*、text/*、audio/*、application/pdf
api-error-file-upload-dir-failed = アップロードディレクトリの作成に失敗しました
api-error-file-dir-not-found = ディレクトリが見つかりません
api-error-file-workspace-error = ワークスペースパスエラー

# ツールエラー
api-error-tool-provide-allowlist = 'tool_allowlist' および/または 'tool_blocklist' を指定してください
api-error-tool-unknown = 不明なツール: { $name }
api-error-tool-not-found = ツールが見つかりません: { $name }
api-error-tool-invoke-disabled = 直接ツール呼び出しが無効です。'[tool_invoke] enabled = true' を設定し、ツール名を 'allowlist' に追加してください。
api-error-tool-invoke-denied = ツール '{ $name }' は '[tool_invoke] allowlist' に含まれていません
api-error-tool-requires-agent = ツール '{ $name }' は承認が必要であり、エージェントコンテキストなしでは呼び出せません。エージェント経由で実行してください

# バリデーションエラー
api-error-validation-content-empty = 内容は空にできません
api-error-validation-name-empty = new_name は空にできません
api-error-validation-title-required = タイトルは必須です
api-error-validation-avatar-url-invalid = アバター URL は http/https または data URI である必要があります
api-error-validation-color-invalid = 色は '#' で始まる 16 進コードである必要があります

# 一般エラー
api-error-not-found = リソースが見つかりません
api-error-internal = 内部サーバーエラー
api-error-bad-request = 不正なリクエスト: { $reason }
api-error-rate-limited = リクエスト制限を超えました。しばらくしてから再試行してください。
