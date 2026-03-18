# --- API error messages (Japanese) ---

# Agent errors
api-error-agent-not-found = エージェントが見つかりません
api-error-agent-spawn-failed = エージェントの作成に失敗しました
api-error-agent-invalid-id = 無効なエージェント ID
api-error-agent-already-exists = エージェントは既に存在します

# Message errors
api-error-message-too-large = メッセージが大きすぎます（最大 64KB）
api-error-message-delivery-failed = メッセージの配信に失敗しました: { $reason }

# Template errors
api-error-template-invalid-name = 無効なテンプレート名
api-error-template-not-found = テンプレート '{ $name }' が見つかりません
api-error-template-parse-failed = テンプレートの解析に失敗しました: { $error }
api-error-template-required = 'manifest_toml' または 'template' が必要です

# Manifest errors
api-error-manifest-too-large = マニフェストが大きすぎます（最大 1MB）
api-error-manifest-invalid-format = 無効なマニフェスト形式
api-error-manifest-signature-mismatch = 署名されたマニフェストの内容が manifest_toml と一致しません
api-error-manifest-signature-failed = マニフェストの署名検証に失敗しました

# Auth errors
api-error-auth-invalid-key = 無効な API キー
api-error-auth-missing-header = Authorization: Bearer <api_key> ヘッダーがありません

# Session errors
api-error-session-load-failed = セッションの読み込みに失敗しました
api-error-session-not-found = セッションが見つかりません

# Workflow errors
api-error-workflow-missing-steps = 'steps' 配列がありません
api-error-workflow-step-needs-agent = ステップ '{ $step }' には 'agent_id' または 'agent_name' が必要です
api-error-workflow-invalid-id = 無効なワークフロー ID
api-error-workflow-execution-failed = ワークフローの実行に失敗しました

# Trigger errors
api-error-trigger-missing-agent-id = 'agent_id' がありません
api-error-trigger-invalid-agent-id = 無効な agent_id
api-error-trigger-invalid-pattern = 無効なトリガーパターン
api-error-trigger-missing-pattern = 'pattern' がありません
api-error-trigger-registration-failed = トリガーの登録に失敗しました（エージェントが見つかりません？）
api-error-trigger-invalid-id = 無効なトリガー ID
api-error-trigger-not-found = トリガーが見つかりません

# Budget errors
api-error-budget-invalid-amount = 無効な予算額
api-error-budget-update-failed = 予算の更新に失敗しました

# Config errors
api-error-config-parse-failed = 設定の解析に失敗しました: { $error }
api-error-config-write-failed = 設定の書き込みに失敗しました: { $error }

# Profile errors
api-error-profile-not-found = プロファイル '{ $name }' が見つかりません

# Cron errors
api-error-cron-invalid-id = 無効なスケジュールタスク ID
api-error-cron-not-found = スケジュールタスクが見つかりません
api-error-cron-create-failed = スケジュールタスクの作成に失敗しました: { $error }

# General errors
api-error-not-found = リソースが見つかりません
api-error-internal = 内部サーバーエラー
api-error-bad-request = 不正なリクエスト: { $reason }
api-error-rate-limited = リクエスト制限を超えました。しばらくしてから再試行してください。
