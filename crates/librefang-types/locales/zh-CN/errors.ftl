# --- API error messages (Simplified Chinese) ---

# Agent errors
api-error-agent-not-found = 未找到智能体
api-error-agent-spawn-failed = 创建智能体失败
api-error-agent-invalid-id = 无效的智能体 ID
api-error-session-invalid-id = 无效的会话 ID
api-error-agent-already-exists = 智能体已存在

# Message errors
api-error-message-too-large = 消息过大（最大 64KB）
api-error-message-delivery-failed = 消息发送失败：{ $reason }

# Template errors
api-error-template-invalid-name = 无效的模板名称
api-error-template-not-found = 未找到模板 '{ $name }'
api-error-template-parse-failed = 解析模板失败：{ $error }
api-error-template-required = 必须提供 'manifest_toml' 或 'template'

# Manifest errors
api-error-manifest-too-large = 清单文件过大（最大 1MB）
api-error-manifest-invalid-format = 无效的清单格式
api-error-manifest-signature-mismatch = 签名清单内容与 manifest_toml 不匹配
api-error-manifest-signature-failed = 清单签名验证失败

# Auth errors
api-error-auth-invalid-key = 无效的 API 密钥
api-error-auth-missing-header = 缺少 Authorization: Bearer <api_key> 请求头
api-error-auth-missing = 该供应商的 API Key 未配置

# Session errors
api-error-session-load-failed = 加载会话失败
api-error-session-not-found = 未找到会话

# Workflow errors
api-error-workflow-missing-steps = 缺少 'steps' 数组
api-error-workflow-step-needs-agent = 步骤 '{ $step }' 需要 'agent_id' 或 'agent_name'
api-error-workflow-invalid-id = 无效的工作流 ID
api-error-workflow-execution-failed = 工作流执行失败

# Trigger errors
api-error-trigger-missing-agent-id = 缺少 'agent_id'
api-error-trigger-invalid-agent-id = 无效的 agent_id
api-error-trigger-invalid-pattern = 无效的触发器模式
api-error-trigger-missing-pattern = 缺少 'pattern'
api-error-trigger-registration-failed = 触发器注册失败（智能体未找到？）
api-error-trigger-invalid-id = 无效的触发器 ID
api-error-trigger-not-found = 未找到触发器

# Budget errors
api-error-budget-invalid-amount = 无效的预算金额
api-error-budget-update-failed = 更新预算失败

# Config errors
api-error-config-parse-failed = 解析配置失败：{ $error }
api-error-config-write-failed = 写入配置失败：{ $error }

# Profile errors
api-error-profile-not-found = 未找到配置文件 '{ $name }'

# Cron errors
api-error-cron-invalid-id = 无效的定时任务 ID
api-error-cron-not-found = 未找到定时任务
api-error-cron-create-failed = 创建定时任务失败：{ $error }

# Tool errors
api-error-tool-not-found = 未找到工具：{ $name }
api-error-tool-invoke-disabled = 直接调用工具已禁用。请在配置中设置 '[tool_invoke] enabled = true'，并把工具名加入 'allowlist'。
api-error-tool-invoke-denied = 工具 '{ $name }' 不在 '[tool_invoke] allowlist' 白名单中
api-error-tool-requires-agent = 工具 '{ $name }' 需要人工审批，无法在无智能体上下文的情况下调用；请通过智能体发起调用

# General errors
api-error-not-found = 未找到资源
api-error-internal = 内部服务器错误
api-error-bad-request = 请求无效：{ $reason }
api-error-rate-limited = 请求频率超限，请稍后重试。
