# --- Daemon lifecycle ---
daemon-starting = 正在启动守护进程...
daemon-stopped = LibreFang 守护进程已停止。
kernel-booted = 内核已启动 ({ $provider }/{ $model })
models-available = { $count } 个模型可用
agents-loaded = 已加载 { $count } 个智能体
daemon-started-bg = 守护进程已在后台启动
daemon-still-starting = 守护进程已在后台启动，仍在初始化中
daemon-stopped-ok = 守护进程已停止
daemon-stopped-forced = 守护进程已停止（强制）
daemon-error = 守护进程错误：{ $error }
daemon-already-running = 守护进程已在 { $url } 运行
daemon-already-running-fix = 使用 `librefang status` 检查状态，或先停止它
daemon-not-running = 守护进程未运行。
daemon-not-running-start = 守护进程未运行。请使用以下命令启动：librefang start
daemon-no-running-found = 未找到运行中的守护进程
daemon-no-running-found-fix = 是否在运行？请检查：librefang status
daemon-restarting = 正在重启守护进程...
daemon-no-running-starting = 未找到运行中的守护进程；正在启动新的守护进程
daemon-bg-exited = 后台守护进程在就绪前退出（{ $status }）
daemon-bg-exited-fix = 请查看启动日志：{ $path }
daemon-bg-wait-fail = 等待后台守护进程时失败
daemon-bg-wait-fail-fix = { $error }。请查看启动日志：{ $path }
daemon-launch-fail = 启动后台守护进程失败
daemon-no-running-auto = 没有运行中的守护进程 - 正在启动...
daemon-started = 守护进程已启动
daemon-start-fail = 无法启动守护进程：{ $error }
daemon-start-fail-fix = 请手动启动：librefang start
shutdown-request-fail = 关闭请求失败（{ $status }）
could-not-reach-daemon = 无法连接守护进程：{ $error }
# Issue #4693 — `curl install.sh | sh` 升级二进制后没有重启守护进程，
# 新版 CLI 调用旧守护进程的 /api/shutdown 时因 api_key 不一致而被拒绝
# (vault 锁定 / 密钥轮换 / 刚启用控制台凭证)。直接说明原因并退回到
# 基于 PID 的强制停止，让用户不必手动改配置。
shutdown-401-detected = 关闭请求被运行中的守护进程拒绝（401 Unauthorized）。
shutdown-401-explainer = 新版 CLI 无法对当前运行的守护进程进行身份认证。这种情况通常发生在 `curl install.sh | sh` 升级二进制后未重启守护进程 —— 旧守护进程使用了不同的 api_key，或保存该 key 的 vault 无法解锁。
shutdown-401-fallback-attempt = 退回到基于 PID 的停止方式（PID { $pid }）...
shutdown-401-fallback-success = 已通过 PID { $pid } 停止守护进程
shutdown-401-fallback-fail = 基于 PID 的停止也失败了。
shutdown-401-fallback-fix = 请手动停止守护进程，然后重新启动：
    kill { $pid }    # 或：kill -9 { $pid } 如果没有退出
    librefang start
shutdown-401-no-pid-fix = 无法从 { $path } 读取守护进程 PID。请运行 `ps -ef | grep librefang` 找到 PID，然后 `kill <pid>` 并执行 `librefang start`。

# --- Labels ---
label-api = API
label-dashboard = 控制台
label-provider = 提供商
label-model = 模型
label-pid = PID
label-log = 日志
label-status = 状态
label-agents = 智能体
label-data-dir = 数据目录
label-uptime = 运行时间
label-version = 版本
label-daemon = 守护进程
label-id = ID
label-active-agents = 活跃智能体
label-pairing-code = 配对码
label-expires = 过期时间

# --- Hints ---
hint-open-dashboard = 在浏览器中打开控制台，或运行 `librefang chat`
hint-stop-daemon = 使用 `librefang stop` 停止守护进程
hint-tail-stop = Ctrl+C 停止日志查看；守护进程将继续运行
hint-check-status = 运行 `librefang status` 检查就绪状态
hint-start-daemon = 请使用以下命令启动：librefang start
hint-start-daemon-cmd = 启动守护进程：librefang start
hint-or-chat = 或尝试 `librefang chat`，无需守护进程即可使用
hint-non-interactive = 检测到非交互式终端 - 使用快速模式运行
hint-non-interactive-wizard = 如需交互式向导，请运行：librefang init（在终端中）
hint-starting-chat = 正在启动聊天会话...
hint-no-api-keys = 未找到 LLM 提供商 API 密钥
hint-groq-free = Groq 提供免费套餐：https://console.groq.com
hint-ollama-local = 或安装 Ollama 使用本地模型：https://ollama.com
hint-gemini-free = Gemini 提供免费套餐：https://aistudio.google.com
hint-deepseek-free = DeepSeek 新号赠送 500 万免费 tokens：https://platform.deepseek.com
guide-title = 快速配置
guide-free-providers-title = 选择一个免费提供商开始使用（2 分钟完成）：
guide-get-free-key = 获取免费 API 密钥
guide-paste-key-placeholder = 在此粘贴 API 密钥
guide-setting-up = 正在配置
guide-testing-key = 正在测试密钥...
guide-key-verified = ✓ 密钥验证成功！
guide-test-key-unverified = ⚠ 无法验证（可能仍然可用）
guide-help-select = ↑↓ 导航  Enter 选择  s/Esc 跳过
guide-help-paste = 粘贴密钥 + Enter 确认  Esc 返回
guide-help-wait = 请稍候...
guide-paste-key-hint = 从浏览器复制 API 密钥，然后粘贴到下方。
hint-could-not-open-browser = 无法自动打开浏览器。
hint-could-not-open-browser-visit = 无法打开浏览器。请访问：{ $url }
hint-dashboard-url = 控制台：{ $url }
hint-try-dashboard = 请尝试：librefang dashboard
hint-install-desktop = 请使用以下命令安装：cargo install librefang-desktop
hint-fallback-web-dashboard = 回退到网页控制台...
hint-then-open-dashboard = 然后打开：http://127.0.0.1:4545
hint-chat-with-agent = 聊天：librefang chat { $name }
hint-agent-lost-on-exit = 注意：此进程退出后智能体将丢失
hint-persistent-agents = 如需持久化智能体，请先运行 `librefang start`
hint-url-copied = URL 已复制到剪贴板
hint-doctor-repair = 运行 `librefang doctor --repair` 尝试自动修复
hint-run-init = 运行 `librefang init` 设置智能体目录
hint-run-start = 运行 `librefang start` 启动守护进程
hint-config-edit = 修复方法：librefang config edit
hint-set-key = 或运行：librefang config set-key groq
hint-set-key-provider = 稍后设置：librefang config set-key email（或 export EMAIL_PASSWORD=...）

# --- Init ---
init-quick-success = LibreFang 已初始化（快速模式）
init-interactive-success = LibreFang 初始化完成！
init-cancelled = 设置已取消。
init-next-start = 启动守护进程：librefang start
init-next-chat = 聊天：          librefang chat

# --- Error messages ---
error-home-dir = 无法确定主目录
error-create-dir = 创建 { $path } 失败
error-create-dir-fix = 请检查 { $path } 的权限
error-write-config = 写入配置失败
error-config-created = 已创建：{ $path }
error-config-exists = 配置已存在：{ $path }

# --- Daemon communication errors ---
error-daemon-returned = 守护进程返回错误（{ $status }）
error-daemon-returned-fix = 请查看守护进程日志：librefang logs --follow
error-request-timeout = 请求超时
error-request-timeout-fix = 智能体可能正在处理复杂请求。请重试，或检查 `librefang status`
error-connect-refused = 无法连接守护进程
error-connect-refused-fix = 守护进程是否在运行？请使用以下命令启动：librefang start
error-daemon-comm = 守护进程通信错误：{ $error }
error-daemon-comm-fix = 请检查 `librefang status` 或重启：librefang start

# --- Boot errors ---
error-boot-config = 解析配置失败
error-boot-config-fix = 请检查 config.toml 语法：librefang config show
error-boot-db = 数据库错误（文件可能被锁定）
error-boot-db-fix = 请检查是否有其他 LibreFang 进程在运行：librefang status
error-boot-auth = LLM 提供商认证失败
error-boot-auth-fix = 运行 `librefang doctor` 检查 API 密钥配置
error-boot-generic = 内核启动失败：{ $error }
error-boot-generic-fix = 运行 `librefang doctor` 诊断问题

# --- Require daemon ---
error-require-daemon = `librefang { $command }` 需要运行中的守护进程
error-require-daemon-fix = 启动守护进程：librefang start

# --- Provider detection ---
detected-provider = 检测到 { $display }（{ $env_var }）
detected-gemini = 检测到 Gemini（GOOGLE_API_KEY）
detected-ollama = 检测到本地运行的 Ollama（无需 API 密钥）

# --- Desktop app ---
desktop-launching = 正在启动 LibreFang 桌面应用...
desktop-started = 桌面应用已启动。
desktop-launch-fail = 启动桌面应用失败：{ $error }
desktop-not-found = 未找到桌面应用。

# --- Dashboard ---
dashboard-opening = 正在打开控制台 { $url }

# --- Agent commands ---
agent-spawned = 智能体 '{ $name }' 已创建
agent-spawned-inprocess = 智能体 '{ $name }' 已创建（进程内模式）
agent-spawn-failed = 创建失败：{ $error }
agent-spawn-agent-failed = 创建智能体失败：{ $error }
agent-template-not-found = 未找到模板 '{ $name }'
agent-template-not-found-fix = 运行 `librefang agent new` 查看可用模板
agent-no-templates = 未找到智能体模板
agent-no-templates-fix = 运行 `librefang init` 设置智能体目录
agent-template-parse-fail = 解析模板 '{ $name }' 失败：{ $error }
agent-template-parse-fail-fix = 模板清单文件可能已损坏
agent-killed = 智能体 { $id } 已终止。
agent-kill-failed = 终止智能体失败：{ $error }
agent-invalid-id = 无效的智能体 ID：{ $id }
agent-model-set = 智能体 { $id } 模型已设置为 { $value }。
agent-set-model-failed = 设置模型失败：{ $error }
agent-no-daemon-for-set = 未找到运行中的守护进程。请使用以下命令启动：librefang start
agent-unknown-field = 未知字段：{ $field }。支持的字段：model
agent-no-agents = 没有运行中的智能体。
agent-spawn-success = 智能体创建成功！
agent-spawn-inprocess-mode = 智能体已创建（进程内模式）。
agent-note-lost = 注意：此进程退出后智能体将丢失。
agent-note-persistent = 如需持久化智能体，请先运行 `librefang start`。
section-agent-templates = 可用智能体模板

# --- Manifest errors ---
manifest-not-found = 未找到清单文件：{ $path }
manifest-not-found-fix = 请使用 `librefang agent new` 从模板创建
error-reading-manifest = 读取清单错误：{ $error }
error-parsing-manifest = 解析清单错误：{ $error }

# --- Status ---
section-daemon-status = LibreFang 守护进程状态
section-status-inprocess = LibreFang 状态（进程内）
section-active-agents = 活跃智能体
section-persisted-agents = 已持久化智能体
label-daemon-not-running = 未运行
label-home = 主目录
label-platform = 平台
label-sessions = 会话
label-memory = 内存
label-started = 启动时间
label-response = 响应延迟
label-checks = 检查
section-status-locked = 受限信息（需要 API 密钥）
hint-status-locked = 在 ~/.librefang/config.toml 中设置 `api_key` 以查看智能体 / 会话 / 内存信息。
warn-public-bind = 公网暴露
warn-key-missing = 未设置
section-recent-errors = 最近错误（daemon.log）
section-verbose = 详细信息
label-auth = 认证
label-mcp = MCP 服务器
label-peers = OFP 节点
label-channels = 通道
label-skills = 技能
label-hands = Hands
label-config-warnings = 配置警告
auth-none = 无（匿名）
auth-api-key = API 密钥
auth-dashboard-login = 控制台登录
auth-user-keys = { $count } 个用户密钥

# --- Doctor ---
doctor-title = LibreFang 诊断
doctor-all-passed = 所有检查已通过！LibreFang 已就绪。
doctor-repairs-applied = 已应用修复。请重新运行 `librefang doctor` 以验证。
doctor-some-failed = 部分检查未通过。
doctor-no-api-keys = 未找到 LLM 提供商 API 密钥！
section-getting-api-key = 获取 API 密钥（免费套餐）

# --- Security ---
section-security-status = 安全状态
label-audit-trail = 审计追踪
label-taint-tracking = 污点追踪
label-wasm-sandbox = WASM 沙箱
label-wire-protocol = 通信协议
label-api-keys = API 密钥
label-manifests = 清单文件
value-audit-trail = Merkle 哈希链 (SHA-256)
value-taint-tracking = 信息流标签
value-wasm-sandbox = 双重计量（fuel + epoch）
value-wire-protocol = OFP HMAC-SHA256 双向认证
value-api-keys = Zeroizing<String>（丢弃时自动清除）
value-manifests = Ed25519 签名
audit-verified = 审计追踪完整性已验证（Merkle 链有效）。
audit-failed = 审计追踪完整性检查失败。

# --- Health ---
health-ok = 守护进程运行正常
health-not-running = 守护进程未运行。

# --- Channel setup ---
section-channel-setup = 通道设置
channel-configured = { $name } 已配置
channel-no-token = 未提供令牌。设置已取消。
channel-no-email = 未提供邮箱。设置已取消。
channel-token-saved = 令牌已保存至 ~/.librefang/.env
channel-app-token-saved = 应用令牌已保存至 ~/.librefang/.env
channel-bot-token-saved = 机器人令牌已保存至 ~/.librefang/.env
channel-password-saved = 密码已保存至 ~/.librefang/.env
channel-phone-saved = 手机号已保存至 ~/.librefang/.env
channel-key-saved = { $key } 已保存至 ~/.librefang/.env
channel-unknown = 未知通道：{ $name }
channel-unknown-fix = 可用通道：discord、slack、whatsapp、email、signal、matrix
channel-test-ok = 通道测试通过
channel-test-fail = 通道测试失败
section-setup-discord = 设置 Discord
section-setup-slack = 设置 Slack
section-setup-whatsapp = 设置 WhatsApp
section-setup-email = 设置邮箱
section-setup-signal = 设置 Signal
section-setup-matrix = 设置 Matrix

# --- Vault ---
vault-initialized = 凭据保险库已初始化。
vault-not-initialized = 保险库未初始化。
vault-not-init-run = 保险库未初始化。请运行：librefang vault init
vault-unlock-failed = 无法解锁保险库：{ $error }
vault-empty-value = 空值 - 未存储。
vault-stored = 已将 '{ $key }' 存入保险库。
vault-store-failed = 存储失败：{ $error }
vault-removed = 已从保险库中移除 '{ $key }'。
vault-key-not-found = 在保险库中未找到密钥 '{ $key }'。
vault-remove-failed = 移除失败：{ $error }
vault-rotate-no-vault = 未找到保险库文件。请先运行 `librefang vault init`。
vault-rotate-old-key-missing = 未设置 LIBREFANG_VAULT_KEY_OLD。请在轮换前提供当前主密钥（32 字节的 base64）。
vault-rotate-new-key-missing = 未设置 LIBREFANG_VAULT_KEY_NEW。请提供新的主密钥（32 字节的 base64），或使用 --from-stdin 从标准输入读取。
vault-rotate-stdin-read-failed = 从标准输入读取新密钥失败：{ $error }
vault-rotate-stdin-empty = 从标准输入读取的新密钥为空。
vault-rotate-same-key = LIBREFANG_VAULT_KEY_OLD 与新密钥相同 — 拒绝轮换到相同的密钥。
vault-rotate-old-key-invalid = LIBREFANG_VAULT_KEY_OLD 不是有效的 32 字节 base64 密钥：{ $error }
vault-rotate-new-key-invalid = 新密钥不是有效的 32 字节 base64 密钥：{ $error }
vault-rotate-unlock-failed = 使用旧密钥解锁保险库失败：{ $error }。请检查 LIBREFANG_VAULT_KEY_OLD 是否与最初加密保险库时使用的密钥一致。
vault-rotate-sentinel-failed = 使用旧密钥验证保险库哨兵值失败：{ $error }
vault-rotate-rewrap-failed = 使用新密钥重新加密保险库失败：{ $error }。原始保险库文件未被修改。
vault-rotate-success = 已使用新主密钥重新加密保险库（保留了 { $count } 条用户条目）。
vault-rotate-next-step = 下一步：在重启守护进程前，将 LIBREFANG_VAULT_KEY 设置为新值。

# --- Cron ---
cron-created = 定时任务已创建：{ $id }
cron-create-failed = 创建定时任务失败：{ $error }
cron-deleted = 定时任务 { $id } 已删除。
cron-delete-failed = 删除定时任务失败：{ $error }
cron-toggled = 定时任务 { $id } 已{ $action }。
cron-toggle-failed = { $action }定时任务失败：{ $error }

# --- Approvals ---
approval-responded = 审批 { $id } 已{ $action }。
approval-failed = { $action }审批失败：{ $error }

# --- Memory ---
memory-set = 已为智能体 '{ $agent }' 设置 { $key }。
memory-set-failed = 设置记忆失败：{ $error }
memory-deleted = 已删除智能体 '{ $agent }' 的密钥 '{ $key }'。
memory-delete-failed = 删除记忆失败：{ $error }

# --- Devices ---
section-device-pairing = 设备配对
device-scan-qr = 请使用 LibreFang 移动应用扫描此二维码：
device-removed = 设备 { $id } 已移除。
device-remove-failed = 移除设备失败：{ $error }

# --- Webhooks ---
webhook-created = Webhook 已创建：{ $id }
webhook-create-failed = 创建 Webhook 失败：{ $error }
webhook-deleted = Webhook { $id } 已删除。
webhook-delete-failed = 删除 Webhook 失败：{ $error }
webhook-test-ok = Webhook { $id } 测试载荷已成功发送。
webhook-test-failed = 测试 Webhook 失败：{ $error }

# --- Models ---
model-set-success = 默认模型已设置为：{ $model }
model-set-failed = 设置模型失败：{ $error }
model-no-catalog = 模型目录为空。
section-select-model = 选择模型
model-out-of-range = 数字超出范围（1-{ $max }）

# --- Config ---
config-set-success = 配置值已设置。
config-unset-success = 配置键已移除。
config-no-file = 未找到配置文件
config-no-file-fix = 请先运行 `librefang init`
config-read-failed = 读取配置失败：{ $error }
config-parse-error = 配置解析错误：{ $error }
config-parse-fix = 请修复 config.toml 语法，或运行 `librefang config edit`
config-parse-fix-alt = 请先修复 config.toml 语法
config-key-not-found = 未找到键：{ $key }
config-key-path-not-found = 未找到键路径：{ $key }
config-empty-key = 空键名
config-section-not-scalar = '{ $key }' 是一个分区，不是标量值
config-section-not-scalar-fix = 请使用点分记法：{ $key }.field_name
config-parent-not-table = '{ $key }' 的父级不是表
config-serialize-failed = 序列化配置失败：{ $error }
config-write-failed = 写入配置失败：{ $error }
config-set-kv = 已设置 { $key } = { $value }
config-removed-key = 已移除键：{ $key }
config-no-key = 未提供密钥。已取消。
config-saved-key = 已将 { $env_var } 保存到 ~/.librefang/.env
config-save-key-failed = 保存密钥失败：{ $error }
config-removed-env = 已从 ~/.librefang/.env 移除 { $env_var }
config-remove-key-failed = 移除密钥失败：{ $error }
config-env-not-set = { $env_var } 未设置
config-set-key-hint = 设置方法：librefang config set-key { $provider }
config-update-key-hint = 更新密钥：librefang config set-key { $provider }

# --- Hand commands ---
hand-install-deps-success = 已为 hand '{ $id }' 安装依赖。
hand-paused = Hand 实例 '{ $id }' 已暂停。
hand-resumed = Hand 实例 '{ $id }' 已恢复。

# --- Daemon notify ---
daemon-restart-notify = 重启守护进程以应用更改：librefang restart

# --- System info ---
section-system-info = LibreFang 系统信息

# --- Uninstall ---
uninstall-goodbye = LibreFang 已卸载。再见！
uninstall-cancelled = 已取消。
uninstall-stopping-daemon = 正在停止运行中的守护进程...
uninstall-removed = 已移除 { $path }
uninstall-remove-failed = 移除 { $path } 失败：{ $error }
uninstall-removed-data-kept = 已移除数据（保留配置文件）
uninstall-removed-autostart-win = 已移除 Windows 自启动注册表项
uninstall-removed-launch-agent = 已移除 macOS 启动代理
uninstall-remove-launch-fail = 移除启动代理失败：{ $error }
uninstall-removed-autostart-linux = 已移除 Linux 自启动项
uninstall-remove-autostart-fail = 移除自启动项失败：{ $error }
uninstall-removed-systemd = 已移除 systemd 用户服务
uninstall-remove-systemd-fail = 移除 systemd 服务失败：{ $error }
uninstall-cleaned-path = 已从 { $path } 清理 PATH
uninstall-cleaned-path-win = 已从 Windows 用户环境清理 PATH

# --- Reset ---
reset-success = 已移除 { $path }
reset-fail = 移除 { $path } 失败：{ $error }

# --- Logs ---
log-following = --- 正在跟踪 { $path }（Ctrl+C 停止）---
log-path-hint = 日志文件：{ $path }

# --- Brand/proper names ---
brand-openai = OpenAI
brand-openrouter = OpenRouter
brand-deepseek = DeepSeek
brand-deepinfra = DeepInfra
brand-byteplus = BytePlus
brand-azure-openai = Azure OpenAI
brand-github-copilot = GitHub Copilot
brand-huggingface = Hugging Face
brand-openai-codex = OpenAI Codex
brand-claude-code = Claude Code
brand-vertex-ai = Vertex AI
brand-nvidia-nim = NVIDIA NIM
brand-zai = Z.ai
brand-kimi-coding = Kimi Coding
brand-alibaba-coding-plan = Alibaba Coding Plan
brand-slack-app = Slack App
brand-slack-bot = Slack Bot
brand-telegram = Telegram
brand-discord = Discord
brand-openclaw-openfang = OpenClaw / OpenFang
brand-openclaw = OpenClaw
brand-openfang = OpenFang

# --- Number/unit formatting templates ---
format-bytes-gib = { $value } GiB
format-bytes-mib = { $value } MiB
format-bytes-kib = { $value } KiB
format-bytes-b = { $value } B
format-size-mb = ({ $value } MB)

format-uptime-s = { $secs }s
format-uptime-ms = { $mins }m { $secs }s
format-uptime-hm = { $hours }h { $mins }m
format-uptime-hms = { $hours }h { $mins }m { $secs }s
format-uptime-dh = { $days }d { $hours }h
format-uptime-dhm = { $days }d { $hours }h { $mins }m

# --- Desktop install & Update errors ---
desktop-install-unsupported-platform = 不支持的平台
desktop-install-error-hdiutil-attach = hdiutil 挂载失败：{ $error }
desktop-install-error-app-not-found = 在 DMG 中未找到 LibreFang.app
desktop-install-error-remove-old = 无法移除旧安装：{ $error }
desktop-install-error-cp = cp 失败：{ $error }
desktop-install-error-copy-applications = 复制到 /Applications 失败：{ $error }
desktop-install-error-run-installer = 启动安装程序失败：{ $error }
desktop-install-error-installer-status = 安装程序已退出：{ $status }
desktop-install-error-localappdata = 无法确定 %LOCALAPPDATA%
desktop-install-error-binary-not-found = 安装程序已完成，但在预期位置未找到二进制文件
desktop-install-error-home-dir = 无法确定用户主目录
desktop-install-error-create-dir = 创建 { $path } 失败：{ $error }
desktop-install-error-copy-appimage = 复制 AppImage 失败：{ $error }
desktop-install-error-http = HTTP 请求失败：{ $error }
desktop-install-error-create = 无法创建 { $path }：{ $error }
desktop-install-error-write = 写入错误：{ $error }

maintenance-error-github-request = GitHub 请求失败：{ $error }
maintenance-error-github-status = GitHub API 返回了 { $status }
maintenance-error-decode-release = 解码发布版元数据失败：{ $error }
maintenance-error-missing-tag = 发布版元数据中缺少 `tag_name`
maintenance-error-decode-list = 解码发布版列表失败：{ $error }
maintenance-error-no-release = 未找到适用于 '{ $channel }' 频道的发布版本
maintenance-error-http-client = 构建 HTTP 客户端失败：{ $error }
maintenance-error-powershell-updater = 无法启动 PowerShell 更新程序：{ $error }
maintenance-error-run-installer = 启动安装程序失败：{ $error }
maintenance-error-installer-status = 安装程序以状态 { $status } 退出
maintenance-error-download-fail = 下载失败：{ $error }
maintenance-error-download-status = 下载返回了 { $status }
maintenance-error-read-response = 读取响应主体失败：{ $error }
maintenance-error-create-dir = 创建更新程序目录失败：{ $error }
maintenance-error-create-script = 创建更新程序脚本失败：{ $error }
maintenance-error-write-script = 写入更新程序脚本失败：{ $error }

common-error-find-exe = 找不到可执行文件：{ $error }
common-error-spawn-daemon = 无法启动后台守护进程：{ $error }
common-error-daemon-timeout = 守护进程未在 10 秒内就绪

ui-brand-title = LibreFang Agent OS
progress-fail = [失败]

# acp.rs
acp-attached-uds = librefang acp: 已连接到守护进程 (UDS { $path })
acp-attached-pipe = librefang acp: 已连接到守护进程 (命名管道)
acp-in-process = librefang acp: 进程内内核 (未检测到守护进程)
acp-error-boot-kernel = 启动内核失败: { $error }
acp-error-resolve-agent = 解析智能体 '{ $name }' 失败: { $error }
acp-error-server = ACP 服务器错误: { $error }
acp-error-uds-connect = ACP UDS 连接失败 { $path }: { $error }
acp-error-pipe-connect = ACP 命名管道连接失败 { $name }: { $error }

# tui/chat_runner.rs
chat-runner-owner-notice = [owner_notice] { $preview }
chat-runner-error-prefix = 错误: { $error }
chat-runner-no-active-connection = 无活动连接
chat-runner-unknown-command = 未知命令: { $command }。输入 /help
chat-runner-status-mode-daemon = 模式: 守护进程 ({ $url })
chat-runner-status-agent = 智能体: { $name }
chat-runner-status-mode-inprocess = 模式: 进程内
chat-runner-status-agents-count = 智能体数量: { $count }
chat-runner-status-mode-disconnected = 模式: 已断开连接
chat-runner-chat-history-cleared = 聊天历史记录已清除。
chat-runner-agent-killed = 智能体 "{ $name }" 已终止。
chat-runner-failed-kill-agent = 终止智能体 "{ $name }" 失败。
chat-runner-kill-failed = 终止失败: { $error }
chat-runner-no-backend-connected = 未连接后端。
chat-runner-no-models-available = 无可用模型。
chat-runner-switched-model = 已切换到 { $model }
chat-runner-failed-switch-model = 切换到 { $model } 失败
chat-runner-switch-failed = 切换失败: { $error }
chat-runner-welcome-help-hint = /help 获取命令列表 • /exit 退出
chat-runner-spawning-agent = 正在生成智能体 '{ $name }'…
chat-runner-no-agent-templates = 未找到智能体模板。请运行 `librefang init`。
chat-runner-invalid-template = 智能体模板 '{ $name }' 无效: { $error }
chat-runner-spawn-failed = 生成失败: { $error }
chat-runner-booting-kernel = 正在启动内核…
chat-runner-booting-kernel-hint =   内核初始化可能需要一些时间。
chat-runner-failed-start = 启动失败
chat-runner-press-esc-to-exit =   按 Esc 键退出。


# tui/event.rs
tui-event-workflow-completed = 工作流已完成
tui-event-workflow-exec-not-available-in-process = 进程内模式下工作流执行不可用
tui-event-workflow-create-not-available-in-process = 进程内模式下工作流创建不可用
tui-event-trigger-create-not-available-in-process = 进程内模式下触发器创建不可用
tui-event-trigger-delete-failed = 删除触发器 { $trigger_id } 失败
tui-event-trigger-delete-not-available-in-process = 进程内模式下触发器删除不可用
tui-event-agent-kill-failed = 终止智能体 { $agent_id } 失败
tui-event-agent-invalid-id = 无效的智能体 ID: { $agent_id }
tui-event-skills-fetch-failed = 获取技能失败
tui-event-mcp-fetch-failed = 获取 MCP 服务失败
tui-event-skills-update-failed = 更新技能失败
tui-event-skills-update-error = 技能更新: { $error }
tui-event-mcp-update-failed = 更新 MCP 服务失败
tui-event-mcp-update-error = MCP 更新: { $error }
tui-event-session-delete-failed = 删除会话 { $session_id } 失败
tui-event-session-management-not-available-in-process = 进程内模式下会话管理不可用
tui-event-kv-save-failed = 保存键值对失败
tui-event-kv-not-available-in-process = 进程内模式下内存 KV 不可用
tui-event-kv-delete-failed = 删除键值对失败
tui-event-skill-install-failed = 安装 { $slug } 失败
tui-event-skill-install-not-available-in-process = 进程内模式下技能安装不可用
tui-event-skill-uninstall-failed = 卸载 { $name } 失败
tui-event-skill-uninstall-not-available-in-process = 进程内模式下技能卸载不可用
tui-event-security-verification-complete = 验证完成
tui-event-security-chain-not-applicable = 进程内模式：链不适用
tui-event-provider-save-key-failed = 保存 { $name } 的密钥失败
tui-event-provider-key-management-not-available-in-process = 进程内模式下提供商密钥管理不可用
tui-event-provider-delete-key-failed = 删除 { $name } 的密钥失败
tui-event-provider-connection-ok = 连接成功
tui-event-provider-test-failed = 测试失败
tui-event-provider-test-not-available-in-process = 进程内模式下提供商测试不可用
tui-event-hand-activation-failed = 激活失败
tui-event-hand-activate-failed-error = 激活失败: { $error }
tui-event-hand-activation-failed-error = 激活失败: { $error }
tui-event-hand-deactivate-failed = 停用 { $instance_id } 失败
tui-event-hand-deactivate-failed-error = 停用失败: { $error }
tui-event-hand-invalid-instance-id = 无效的实例 ID: { $error }
tui-event-hand-pause-failed = 暂停 { $instance_id } 失败
tui-event-hand-pause-failed-error = 暂停失败: { $error }
tui-event-hand-resume-failed = 恢复 { $instance_id } 失败
tui-event-hand-resume-failed-error = 恢复失败: { $error }
tui-event-extension-install-failed = 安装 { $id } 失败
tui-event-extension-install-failed-error = 安装失败: { $error }
tui-event-extension-install-not-supported = 不支持通过进程内模式安装 — 请使用 CLI
tui-event-extension-remove-failed = 移除 { $id } 失败
tui-event-extension-remove-not-supported = 不支持通过进程内模式移除 — 请使用 CLI
tui-event-extension-reconnect-failed = 重新连接 { $id } 失败
tui-event-extension-reconnect-not-supported = 不支持通过进程内模式重新连接
tui-event-comms-message-sent = 消息已发送
tui-event-comms-send-failed = 发送失败
tui-event-comms-send-not-supported-in-process = 进程内模式下不支持发送
tui-event-comms-task-posted = 任务已发布
tui-event-comms-post-failed = 发布失败
tui-event-comms-post-not-supported-in-process = 进程内模式下不支持任务发布
tui-event-stream-runtime-error = 运行时错误: { $error }
tui-event-stream-connection-failed = 连接失败: { $error }
tui-event-agent-spawn-failed-fallback = 生成智能体失败

# tui/mod.rs
tui-mod-session-deleted = 会话 { $id } 已删除。
tui-mod-saved-key = 已保存密钥: { $key }
tui-mod-deleted-key = 已删除密钥: { $key }
tui-mod-skill-installed = 已安装: { $name }
tui-mod-skill-uninstalled = 已卸载: { $name }
tui-mod-key-saved-for = 已保存 { $name } 的密钥
tui-mod-key-deleted-for = 已删除 { $name } 的密钥
tui-mod-hand-activated = 已激活: { $name }
tui-mod-hand-deactivated = 已停用: { $id }
tui-mod-hand-paused = Hands 已暂停
tui-mod-hand-resumed = Hands 已恢复
tui-mod-extension-installed = 已安装: { $id }
tui-mod-extension-removed = 已移除: { $id }
tui-mod-extension-reconnected = 已重新连接 { $id }: { $tools } 个工具
tui-mod-no-agents-running = 无正在运行的智能体。
tui-mod-agent-killed = 智能体 "{ $name }" 已终止。
tui-mod-failed-kill-agent = 终止智能体 "{ $name }" 失败。
tui-mod-missing-api-key = 缺少 API 密钥
tui-mod-invalid-manifest = 无效的清单: { $error }
tui-mod-spawn-failed = 生成失败: { $error }
tui-mod-help-help = /help         — 显示此帮助信息
tui-mod-help-model = /model        — 打开模型选择器 (Ctrl+M)
tui-mod-help-model-arg = /model <name> — 直接切换到模型
tui-mod-help-status = /status       — 连接与智能体信息
tui-mod-help-agents = /agents       — 列出运行中的智能体
tui-mod-help-clear = /clear        — 清除聊天历史记录
tui-mod-help-kill = /kill         — 终止当前智能体
tui-mod-help-exit = /exit         — 结束聊天会话
tui-mod-status-mode-daemon = 模式: 守护进程 ({ $url })
tui-mod-status-agent = 智能体: { $name }
tui-mod-status-mode-inprocess = 模式: 进程内
tui-mod-status-agents-count = 智能体数量: { $count }
tui-mod-status-mode-disconnected = 模式: 已断开连接
tui-mod-chat-history-cleared = 聊天历史记录已清除。
tui-mod-available-hands = 可用 Hands ({ $count }):
tui-mod-active-hands = 活跃 Hands ({ $count }):
tui-mod-hands-info-requires-inprocess = Hands 信息需要进程内模式。请改用 Hands 标签页。
tui-mod-unknown-command = 未知命令: { $command }。输入 /help
tui-mod-error-symbol =  ✘ { $error }
tui-mod-press-ctrl-c-again-to-quit = 再次按下 Ctrl+C 退出
tui-mod-ctrl-c-status-bar = Ctrl+C×2 退出  Tab/Ctrl+←→ 切换标签
tui-mod-trigger-deleted = 触发器 { $id } 已删除。
tui-mod-agent-killed-status = 智能体 { $id } 已终止。
tui-mod-agent-kill-failed = 终止失败: { $error }
tui-mod-agent-skills-updated = 已更新智能体 { $id } 的技能。
tui-mod-agent-mcp-updated = 已更新智能体 { $id } 的 MCP 服务端。
tui-mod-ready = 准备就绪
tui-mod-setup = 待设置
tui-mod-workflow-created = 工作流已创建！
tui-mod-trigger-created = 触发器已创建！
tui-tab-dashboard = 仪表盘
tui-tab-agents = 智能体
tui-tab-chat = 聊天
tui-tab-sessions = 会话
tui-tab-workflows = 工作流
tui-tab-triggers = 触发器
tui-tab-memory = 记忆
tui-tab-skills = 技能
tui-tab-hands = Hands
tui-tab-extensions = 扩展
tui-tab-templates = 模板
tui-tab-peers = 对等节点
tui-tab-comms = 通信
tui-tab-security = 安全
tui-tab-audit = 审计
tui-tab-usage = 使用量
tui-tab-settings = 配置
tui-tab-logs = 日志
# welcome.rs
tui-welcome-menu-connect = 连接到守护进程
tui-welcome-menu-connect-hint = 通过 API 与运行中的智能体通信
tui-welcome-menu-chat = 快速聊天
tui-welcome-menu-chat-hint = 本地启动内核，无需守护进程
tui-welcome-menu-setup = 设置向导
tui-welcome-menu-setup-hint = 配置提供商与通道
tui-welcome-menu-exit = 退出
tui-welcome-menu-exit-hint = 退出 LibreFang
tui-welcome-tagline = 智能体操作系统
tui-welcome-ctrl-c-quit = 再次按下 Ctrl+C 退出
tui-welcome-hint-bar = ↑↓ 导航  enter 选择  q 退出
tui-welcome-checking-daemon = 正在检查守护进程…
tui-welcome-agent-count =  • { $count } 个智能体
tui-welcome-daemon-status = 守护进程 { $url }
tui-welcome-no-daemon = 无正在运行的守护进程
tui-welcome-provider = 提供商: { $provider }
tui-welcome-no-api-keys = 缺少 API 密钥
tui-welcome-run-hint-prefix =  — 运行 
tui-welcome-setup-complete = 设置完成！

# sessions.rs
tui-sessions-title = 会话
tui-sessions-filter = (过滤: "{ $query }")
tui-sessions-count = { $count } 个会话
tui-sessions-header-agent = 智能体
tui-sessions-header-id = 会话 ID
tui-sessions-header-msgs = 消息数
tui-sessions-header-created = 创建时间
tui-sessions-loading = 正在加载会话…
tui-sessions-empty = 暂无会话。开启聊天以创建。
tui-sessions-delete-confirm = 删除此会话？[y] 是  [任意键] 取消
tui-sessions-hints = ↑↓ 导航  Enter 打开  d 删除  / 搜索  r 刷新

# peers.rs
tui-peers-title = 对等节点
tui-peers-network = OFP 对等网络
tui-peers-count = { $count } 个对等节点
tui-peers-header-node-id = 节点 ID
tui-peers-header-name = 名称
tui-peers-header-address = 地址
tui-peers-header-status = 状态
tui-peers-header-agents = 智能体
tui-peers-header-protocol = 协议
tui-peers-status-active = 活跃
tui-peers-status-offline = 离线
tui-peers-status-pending = 等待中
tui-peers-loading = 正在寻找对等节点…
tui-peers-empty = 无连接的对等节点。请在 config.toml 中启用 OFP 网络。
tui-peers-hints = ↑↓ 导航  r 刷新 (每 15 秒自动刷新)

# usage.rs
tui-usage-title = 使用量
tui-usage-hints = [1] 概览  [2] 按模型  [3] 按智能体  [r] 刷新
tui-usage-tab-summary = 1 概览
tui-usage-tab-model = 2 按模型
tui-usage-tab-agent = 3 按智能体
tui-usage-loading = 正在加载使用量数据…
tui-usage-loading-simple = 正在加载…
tui-usage-card-input = 输入 Token
tui-usage-card-output = 输出 Token
tui-usage-card-cost = 总花费
tui-usage-card-calls = API 调用数
tui-usage-header-model = 模型
tui-usage-header-input = 输入 Token
tui-usage-header-output = 输出 Token
tui-usage-header-cost = 花费
tui-usage-header-calls = 调用数
tui-usage-header-agent = 智能体
tui-usage-header-total-tokens = 总 Token
tui-usage-header-tool-calls = 工具调用数
tui-usage-empty = 暂无使用量数据。发送消息以查看 Token 统计信息。

# hands.rs
tui-hands-title = Hands
tui-hands-tab-marketplace = 应用市场
tui-hands-tab-active = 已启用
tui-hands-loading = 正在加载 Hands…
tui-hands-loading-active = 正在加载活动中的 Hands…
tui-hands-empty-marketplace = 未加载 Hands 定义。
tui-hands-empty-active = 无活动中的 Hands。按 [1] 浏览应用市场。
tui-hands-status-ready = 已就绪
tui-hands-status-setup = 设置
tui-hands-status-active = 活动中
tui-hands-status-paused = 已暂停
tui-hands-status-unknown = 未知
tui-hands-hints-marketplace =   [↑↓] 导航  [a/Enter] 启用  [r] 刷新
tui-hands-hints-active =   [↑↓] 导航  [p] 暂停/恢复  [d] 停用  [r] 刷新
tui-hands-confirm-deactivate =   停用此 Hands？[y] 是  [任意键] 取消
tui-hands-header-name = 名称
tui-hands-header-category = 类别
tui-hands-header-status = 状态
tui-hands-header-description = 描述
tui-hands-header-agent = 智能体
tui-hands-header-hand = Hand
tui-hands-header-since = 运行时间
tui-hands-category-content = 内容
tui-hands-category-security = 安全
tui-hands-category-development = 开发
tui-hands-category-productivity = 效率

# logs.rs
tui-logs-title = 日志
tui-logs-badge-auto = 自动
tui-logs-badge-paused = 暂停
tui-logs-label-level = 级别
tui-logs-filter-all = 全部
tui-logs-filter-error = 错误
tui-logs-filter-warn = 警告
tui-logs-filter-info = 信息
tui-logs-filter-active =   │ 过滤: "{ $query }"
tui-logs-entries-count =   │ { $count } 条记录
tui-logs-header-timestamp = 时间戳
tui-logs-header-level = 级别
tui-logs-header-action = 操作
tui-logs-header-agent = 智能体
tui-logs-header-detail = 详情
tui-logs-loading = 正在加载日志…
tui-logs-empty = 暂无日志记录。启动后台程序以查看日志。
tui-logs-hints =   [↑↓] 导航  [f] 过滤级别  [/] 搜索  [a] 自动刷新  [r] 刷新

# security.rs
tui-security-title = 安全
tui-security-active-features =   已启用 { $active }/{ $total } 项安全特性
tui-security-sections-sub =   │  核心 · 可配置 · 监控
tui-security-section-core = 核心安全
tui-security-section-configurable = 可配置组件
tui-security-section-monitoring = 监控指标
tui-security-header-feature = 特性
tui-security-header-status = 状态
tui-security-header-description = 描述
tui-security-status-active = 活动中
tui-security-status-inactive = 未启用
tui-security-verifying = 正在验证审计链…
tui-security-verify-prompt = 按 [v] 验证审计链完整性
tui-security-verify-success = 审计链已验证
tui-security-verify-failed = 审计链验证失败
tui-security-hints =   [↑↓] 滚动  [v] 验证链  [r] 刷新
tui-security-feat-path-traversal-name = 路径遍历防御
tui-security-feat-path-traversal-desc = safe_resolve_path 阻止 ../../ 攻击
tui-security-feat-ssrf-name = SSRF 防御
tui-security-feat-ssrf-desc = 阻止 HTTP 获取中的私有 IP 和元数据端点
tui-security-feat-subprocess-name = 子进程隔离
tui-security-feat-subprocess-desc = 在子进程上执行 env_clear() + 选择性变量
tui-security-feat-wasm-name = WASM 双重计量
tui-security-feat-wasm-desc = 通过看门狗线程进行 Fuel 和 Epoch 中断
tui-security-feat-capability-name = 能力继承
tui-security-feat-capability-desc = validate_capability_inheritance 防止权限提升
tui-security-feat-secret-name = 密钥置零
tui-security-feat-secret-desc = Zeroizing<String> 自动清除内存中的 API 密钥
tui-security-feat-ed25519-name = Ed25519 配置清单签名
tui-security-feat-ed25519-desc = 带 Ed25519 验证的已签名智能体配置清单
tui-security-feat-taint-name = 污点追踪
tui-security-feat-taint-desc = 跨工具边界的信息流追踪
tui-security-feat-ofp-name = OFP 传输认证
tui-security-feat-ofp-desc = 带随机数的 HMAC-SHA256 双向认证
tui-security-feat-rbac-name = 基于角色的多用户控制
tui-security-feat-rbac-desc = 带用户层级基于角色的访问控制
tui-security-feat-rate-name = 速率限制
tui-security-feat-rate-desc = 具有成本感知 Token 的 GCRA 速率限制器
tui-security-feat-headers-name = 安全响应头
tui-security-feat-headers-desc = CSP, X-Frame-Options, HSTS 中间件
tui-security-feat-merkle-name = 默克尔审计跟踪
tui-security-feat-merkle-desc = 带防篡改检测的的哈希链审计日志
tui-security-feat-heartbeat-name = 心跳监控
tui-security-feat-heartbeat-desc = 带重启限制的的后台健康检查
tui-security-feat-prompt-name = 提示词注入扫描器
tui-security-feat-prompt-desc = 检测覆盖尝试和数据外泄

# templates.rs
tui-templates-title = 模板
tui-templates-cat-all = 全部
tui-templates-cat-general = 通用
tui-templates-cat-development = 开发
tui-templates-cat-research = 研究
tui-templates-cat-writing = 写作
tui-templates-cat-business = 商务
tui-templates-header-template = 模板
tui-templates-header-category = 分类
tui-templates-header-provider-model = 提供商/模型
tui-templates-header-description = 描述
tui-templates-loading = 正在加载模板…
tui-templates-empty = 无可用模板。
tui-templates-detail-provider =   提供商: { $provider }/{ $model }  
tui-templates-hints =   [↑↓] 导航  [Enter] 生成智能体  [f] 过滤分类  [r] 刷新
tui-templates-provider-not-configured = 提供商 '{ $provider }' 未配置。请先在“设置”中设置 API 密钥。
tui-templates-name-general-assistant = 通用助手
tui-templates-desc-general-assistant = 适用于日常任务的通用 AI 助手
tui-templates-name-code-helper = 代码助手
tui-templates-desc-code-helper = 具有代码审查和调试功能的编程助手
tui-templates-name-researcher = 研究员
tui-templates-desc-researcher = 带网页搜索的深度研究与分析
tui-templates-name-writer = 写作助手
tui-templates-desc-writer = 创意和技术写作助手
tui-templates-name-data-analyst = 数据分析师
tui-templates-desc-data-analyst = 数据分析、可视化和 SQL 查询
tui-templates-name-devops-engineer = DevOps 工程师
tui-templates-desc-devops-engineer = 基础设施、CI/CD 和部署协助
tui-templates-name-customer-support = 客户支持
tui-templates-desc-customer-support = 专业客户服务智能体
tui-templates-name-tutor = 导师
tui-templates-desc-tutor = 针对任何学科的耐心教学助手
tui-templates-name-api-designer = API 设计师
tui-templates-desc-api-designer = REST/GraphQL API 设计与文档编写
tui-templates-name-meeting-notes = 会议纪要
tui-templates-desc-meeting-notes = 会议转录、总结 and 待办事项

# audit.rs
tui-audit-title = 审计日志
tui-audit-filter-all = 全部
tui-audit-filter-spawn = 智能体已创建
tui-audit-filter-kill = 智能体已结束
tui-audit-filter-tool = 工具已使用
tui-audit-filter-network = 网络
tui-audit-filter-shell = Shell 执行
tui-audit-action-spawn = 智能体已创建
tui-audit-action-kill = 智能体已结束
tui-audit-action-tool = 工具已使用
tui-audit-action-network = 网络访问
tui-audit-action-shell = Shell 执行
tui-audit-action-denied = 访问被拒绝
tui-audit-action-config = 配置已更改
tui-audit-label-filter = 过滤:
tui-audit-entries-count = { $count } 条记录
tui-audit-header-timestamp = 时间戳
tui-audit-header-action = 操作
tui-audit-header-agent = 智能体
tui-audit-header-hash = 哈希
tui-audit-header-detail = 详情
tui-audit-loading = 正在加载审计日志…
tui-audit-empty = 暂无审计记录。智能体的操作将显示在此处。
tui-audit-chain-unverified = 审计链: 未验证
tui-audit-chain-verified = 审计链: 验证通过
tui-audit-chain-failed = 审计链: 验证失败
tui-audit-hints =   [↑↓] 导航  [f] 过滤  [v] 验证链  [r] 刷新

# dashboard.rs
tui-dashboard-title = 控制台
tui-dashboard-hints =   [r] 刷新  [a] 智能体  [↑↓] 滚动  [PgUp/PgDn] 快速滚动
tui-dashboard-dreams-title = DREAMS
tui-dashboard-auto-dream-enabled = auto-dream 已启用
tui-dashboard-auto-dream-disabled = auto-dream 已禁用
tui-dashboard-dream-details = 阶段={ $phase }  工具={ $tools }  记忆={ $mems }
tui-dashboard-stat-agents = 智能体
tui-dashboard-stat-uptime = 在线时间
tui-dashboard-stat-provider = 提供商
tui-dashboard-stat-model = 模型
tui-dashboard-audit-time = 时间
tui-dashboard-audit-agent = 智能体
tui-dashboard-audit-action = 操作
tui-dashboard-audit-detail = 详情
tui-dashboard-loading = 正在加载…
tui-dashboard-no-audit = 暂无审计记录。

# comms.rs
tui-comms-title = 通信
tui-comms-tab-topology = 拓扑关系 ({ $agents } 智能体, { $edges } 连接)
tui-comms-tab-events = 事件 ({ $count })
tui-comms-hints =   [s] 发送  [t] 任务  [r] 刷新  [Tab] 切换焦点  [↑↓] 滚动
tui-comms-loading = 正在加载拓扑结构…
tui-comms-empty = 没有正在运行的智能体。启动智能体以查看通信。
tui-comms-events-empty = 暂无智能体间的事件。活动将在此处显示。
tui-comms-modal-send-title =  发送消息 
tui-comms-modal-send-from = 发送方 (智能体 ID):
tui-comms-modal-send-to = 接收方 (智能体 ID):
tui-comms-modal-send-msg = 消息:
tui-comms-modal-send-hints = [Tab] 切换输入框  [Enter] 发送  [Esc] 取消
tui-comms-modal-task-title =  发布任务 
tui-comms-modal-task-title-field = 标题:
tui-comms-modal-task-desc = 描述:
tui-comms-modal-task-assign = 指派给 (智能体 ID, 可选):
tui-comms-modal-task-hints = [Tab] 切换输入框  [Enter] 发布  [Esc] 取消

# settings.rs
tui-settings-title = 设置
tui-settings-hints-input =   [Enter] 保存  [Esc] 取消
tui-settings-hints-providers =   [↑↓] 导航  [e] 设置密钥  [d] 删除密钥  [t] 测试  [r] 刷新
tui-settings-hints-models =   [↑↓] 导航  [r] 刷新
tui-settings-hints-tools =   [↑↓] 导航  [r] 刷新
tui-settings-tab-providers = 1 提供商
tui-settings-tab-models = 2 模型
tui-settings-tab-tools = 3 工具
tui-settings-providers-header-provider = 提供商
tui-settings-providers-header-status = 状态
tui-settings-providers-header-env = 环境变量
tui-settings-providers-loading = 正在加载提供商…
tui-settings-providers-empty = 未配置提供商。运行 `librefang init` 进行设置。
tui-settings-providers-status-online = 在线 ({ $ms }ms)
tui-settings-providers-status-offline = 离线
tui-settings-providers-status-local = 本地
tui-settings-providers-status-configured = 已配置
tui-settings-providers-status-notset = 未设置
tui-settings-providers-input-prompt = 输入 { $provider } 的 API 密钥: 
tui-settings-providers-latency = 延迟: { $ms }ms
tui-settings-models-header-id = 模型 ID
tui-settings-models-header-provider = 提供商
tui-settings-models-header-tier = 层级
tui-settings-models-header-context = 上下文
tui-settings-models-header-cost = 成本 (输入/输出 每百万)
tui-settings-models-loading = 正在加载模型…
tui-settings-models-empty = 没有可用模型。
tui-settings-tools-header-name = 工具名称
tui-settings-tools-header-desc = 描述
tui-settings-tools-empty = 没有可用工具。
# chat.rs
tui-chat-input-staged =   (已暂存 { $count } 个)
tui-chat-hints-modelpicker =     [↑↓] 导航  [Enter] 选择  [Esc] 关闭  [输入] 过滤
tui-chat-hints-streaming =     [Enter] 暂存  [↑↓] 滚动  [Esc] 停止
tui-chat-hints-history =     [Enter] 发送  [↑↓] 历史  [PgUp/PgDn] 滚动  [Esc] 返回
tui-chat-hints-normal =     [Enter] 发送  [Ctrl+M] 模型  [↑↓] 历史  [PgUp/PgDn] 滚动  [Esc] 返回
tui-chat-modelpicker-title =  切换模型 
tui-chat-modelpicker-empty = 没有匹配的模型
tui-chat-welcome-ready = 准备就绪
tui-chat-welcome-suggest =   尝试提问:
tui-chat-welcome-q1 = "解释这个代码库"
tui-chat-welcome-q2 = "写一个单元测试..."
tui-chat-welcome-q3 = "这个函数是做什么的？"
tui-chat-welcome-footer =   输入 /help 查看命令  •  Ctrl+M 切换模型
tui-chat-tokens-estimated =   约 { $count } 个 Token
tui-chat-tokens-detail =   [Token: 输入 { $in } / 输出 { $out }{ $cost }]
tui-chat-tool-input = 输入: 
tui-chat-tool-error = 错误: 
tui-chat-tool-result = 结果: 
tui-chat-tool-running = 正在运行…
tui-chat-thinking = 思考中…
tui-chat-mode-daemon = 守护进程
tui-chat-mode-inprocess = 进程内

# free_provider_guide.rs
tui-guide-hint-groq = 免费层，极速推理
tui-guide-hint-gemini = 免费层，额度慷慨 (需 Google 账号)
tui-guide-hint-deepseek = 新用户免费 500 万 Token
tui-guide-label-apikey =  API 密钥 
tui-guide-warn-env = .env: { $error }

# init_wizard.rs
tui-init-welcome-tagline = Agent 操作系统
tui-init-welcome-sec1 = 沙箱运行, WASM 隔离, SSRF 防护
tui-init-welcome-sec2 = 签名清单, 审计追踪, 污点追踪
tui-init-welcome-sec3 = RBAC, 能力检查, 机密清除
tui-init-welcome-sec4 = API 密钥从不记录, 0600 文件权限
tui-init-welcome-resp1 = Agent 可以执行代码、访问网络，并
tui-init-welcome-resp2 = 代表您执行操作。
tui-init-welcome-resp-warn = 您需要对它们的行为负责。
tui-init-welcome-hints =   [Enter] 我理解    [Esc] 取消
tui-init-migrate-checking =   正在检查现有安装...
tui-init-migrate-openfang-detected =   检测到 OpenFang 安装
tui-init-migrate-openclaw-detected =   检测到 OpenClaw 安装
tui-init-migrate-openfang-summary = OpenFang 配置与数据
tui-init-migrate-openclaw-agents = { $count } 个 Agent ({ $names })
tui-init-migrate-openclaw-no-agents = 无 Agent
tui-init-migrate-openclaw-channels = { $count } 个通道 ({ $names })
tui-init-migrate-openclaw-no-channels = 无通道
ui-init-migrate-openclaw-skills = { $count } 个 Skill
tui-init-migrate-openclaw-no-skills = 无 Skill
tui-init-migrate-openclaw-memory = 内存文件
tui-init-migrate-openclaw-no-memory = 无内存文件
tui-init-migrate-openclaw-config = 配置
tui-init-migrate-opt-yes = 是
tui-init-migrate-opt-yes-desc = 迁移设置与数据
tui-init-migrate-opt-no = 否
tui-init-migrate-opt-no-desc = 全新开始
tui-init-migrate-hints =   [↑↓] 导航  [Enter] 选择  [Esc] 跳过
tui-init-migrate-running-openfang =  正在从 OpenFang 迁移...
tui-init-migrate-running-openclaw =  正在从 OpenClaw 迁移...
tui-init-migrate-done-failed = 迁移失败: { $error }
tui-init-migrate-done-config = 配置已迁移
tui-init-migrate-done-agents = 已导入 { $count } 个 Agent ({ $names })
tui-init-migrate-done-channels = { $count } 个通道 ({ $names })
tui-init-migrate-done-memory = 内存文件已复制
tui-init-migrate-done-skills = 已导入 { $count } 个 Skill
tui-init-migrate-done-sessions = 已导入 { $count } 个会话
tui-init-migrate-done-skipped = { $name } 已跳过 ({ $reason })
tui-init-migrate-done-summary =   已导入 { $imported }，跳过 { $skipped }，{ $warnings } 个警告
tui-init-migrate-done-continue =   [Enter] 继续  
tui-init-migrate-done-autoadvancing = (自动跳转...)
tui-init-provider-prompt =   选择您的 LLM 提供商:
tui-init-provider-cli-detected = 检测到 CLI
tui-init-provider-no-key-needed = 无需 API 密钥
tui-init-provider-local-no-key = 本地，无需密钥
tui-init-provider-requires-with-hint = 需要 { $env_var } ({ $hint })
tui-init-provider-requires = 需要 { $env_var }
tui-init-provider-hints =   [↑↓/jk] 导航  [Enter] 选择  [Esc] 取消
tui-init-hint-freetier = 免费层级
tui-init-hint-cheap = 廉价
tui-init-hint-fast = 快速推理
tui-init-hint-pat = 通过 PAT
tui-init-hint-nokey = 无需 API 密钥
tui-init-hint-local = 本地
tui-init-apikey-prompt =   输入您的 { $provider } API 密钥:
tui-init-apikey-env-hint =     或设置 { $env_var } 环境变量
tui-init-apikey-testing =  正在测试 API 密钥...
tui-init-apikey-verified = API 密钥验证成功
tui-init-apikey-saved =     已保存至 ~/.librefang/.env
tui-init-apikey-verify-failed = 无法验证 (可能仍可使用)
tui-init-apikey-save-failed = 验证成功，但保存到 .env 失败
tui-init-apikey-save-failed-hints =     [Enter] 重试保存 · [Esc] 编辑密钥 (密钥已验证 — 磁盘上无内容)
tui-init-apikey-hints =   [Enter] 确认  [Esc] 返回
tui-init-model-prompt =   选择 { $provider } 的默认模型:
tui-init-model-hints =   [↑↓/jk] 导航  [Enter] 选择  [Esc] 返回    * = 默认
tui-init-routing-title =   智能模型路由
tui-init-routing-desc1 =   根据任务复杂度自动选择合适的模型。
tui-init-routing-desc2 =   简单任务使用廉价/快速模型，复杂任务使用
tui-init-routing-desc3 =   前沿模型。在不牺牲质量的前提下节省成本。
tui-init-routing-opt-yes = 是
tui-init-routing-opt-yes-desc = 选择 3 个模型 (快速 / 平衡 / 前沿)
tui-init-routing-opt-no = 否
tui-init-routing-opt-no-desc = 所有任务使用单一模型
tui-init-routing-hints =   [↑↓] 导航  [Enter] 选择  [Esc] 返回
tui-init-routing-pick-title =   选择 { $tier } 模型 ({ $step }/3):
tui-init-routing-pick-hints =   [↑↓/jk] 导航  [Enter] 选择  [Esc] 返回
tui-init-routing-tier-fast = 快速
tui-init-routing-tier-balanced = 平衡
tui-init-routing-tier-frontier = 前沿
tui-init-routing-tier-fast-desc = 快速查询、问候、简单问答
tui-init-routing-tier-balanced-desc = 标准对话、一般任务
tui-init-routing-tier-frontier-desc = 多步推理、代码生成
tui-init-complete-success-daemon = 设置完成 — 守护进程运行中
tui-init-complete-success = 设置完成！
tui-init-complete-label-provider =   提供商:    
tui-init-complete-label-model =   模型:       
tui-init-complete-label-daemon =   守护进程:    
tui-init-complete-daemon-running = 运行于 { $url }
tui-init-complete-daemon-not-running = 未运行
tui-init-complete-daemon-pending = 等待中
tui-init-complete-question =   您想如何使用 LibreFang？
tui-init-complete-desktop-desc-installed = 带系统托盘的原生窗口
tui-init-complete-desktop-desc-not-installed = 未安装
tui-init-complete-opt-desktop = 桌面应用
tui-init-complete-opt-desktop-badge = (推荐)
tui-init-complete-opt-dashboard = 网页控制台
tui-init-complete-opt-dashboard-desc = 在默认浏览器中打开
tui-init-complete-opt-chat = 终端聊天
tui-init-complete-opt-chat-desc = 在此处进行交互式聊天
tui-init-complete-hints =   [↑↓/jk] 导航  [Enter] 启动  [1/2/3] 快速选择
tui-init-step-label = 第 { $current } 步，共 { $total } 步
tui-init-complete-err-no-provider = 未选择提供商
tui-init-complete-err-home-dir = 无法确定用户主目录
tui-init-complete-err-write-config = 写入配置失败: { $error }
tui-init-complete-err-daemon-failed = 守护进程启动失败: { $error }
tui-init-routing-pick-prefix = 选择
tui-init-routing-pick-suffix = 模型 ({ $step }/3):
tui-init-complete-setup-prefix = 设置完成 — 

# agents.rs
tui-agents-tool-file-read-desc = 读取文件
tui-agents-tool-file-write-desc = 写入文件
tui-agents-tool-file-list-desc = 列出目录内容
tui-agents-tool-memory-store-desc = 在 Agent 内存中存储数据
tui-agents-tool-memory-recall-desc = 从内存中恢复数据
tui-agents-tool-memory-list-desc = 列出所有存储的内存键
tui-agents-tool-web-fetch-desc = 获取网页
tui-agents-tool-shell-exec-desc = 执行 Shell 命令
tui-agents-tool-agent-send-desc = 向其他 Agent 发送消息
tui-agents-tool-agent-list-desc = 列出运行中的 Agent

tui-agents-title-create-method = 创建 Agent
tui-agents-title-templates = 模板
tui-agents-title-custom-name = 自定义 — 名称
tui-agents-title-custom-desc = 自定义 — 描述
tui-agents-title-custom-prompt = 自定义 — 系统提示词
tui-agents-title-custom-tools = 自定义 — 工具
tui-agents-title-custom-skills = 自定义 — Skill
tui-agents-title-custom-mcp = 自定义 — MCP 服务
tui-agents-title-spawning = 正在创建...
tui-agents-title-screen = Agent
tui-agents-title-detail = Agent 详情

tui-agents-prompt-create-method =   您想如何创建您的 Agent？
tui-agents-prompt-name = Agent 名称:
tui-agents-prompt-desc = 描述:
tui-agents-prompt-prompt = 系统提示词:
tui-agents-prompt-tools =   选择工具 (空格键切换):
tui-agents-prompt-skills =   选择 Skill (未勾选 = 所有 Skill):
tui-agents-prompt-mcp =   选择 MCP 服务 (未勾选 = 所有服务):
tui-agents-prompt-edit-skills =   空格键切换，Enter 保存 (未勾选 = 所有):
tui-agents-prompt-spawning =   正在创建 Agent...
tui-agents-label-no-agent-selected = 未选择 Agent。
tui-agents-label-none-available = (无可用项)

tui-agents-opt-templates =   从模板中选择
tui-agents-opt-templates-desc =   (预构建的 Agent)
tui-agents-opt-custom =   构建自定义 Agent
tui-agents-opt-custom-desc =   (选择名称、工具、提示词)

tui-agents-header-state = 状态
tui-agents-header-name = 名称
tui-agents-header-model = 模型
tui-agents-header-id = ID
tui-agents-opt-create-new = 创建新 Agent

tui-agents-hints-filter =   [输入] 过滤  [Enter] 应用  [Esc] 取消搜索
tui-agents-hints-list =   [↑↓] 导航  [Enter] 详情  [/] 搜索  [Esc] 返回
tui-agents-hints-detail =   [s] 编辑 Skill  [m] 编辑 MCP  [c] 聊天  [k] 停止  [Esc] 返回
tui-agents-hints-navigate =     [↑↓] 导航  [Enter] 选择  [Esc] 返回
tui-agents-hints-input =     [Enter] 下一步  [Esc] 返回
tui-agents-hints-tools =     [↑↓] 导航  [Space] 切换  [Enter] 创建  [Esc] 返回
tui-agents-hints-skills =     [↑↓] 导航  [Space] 切换  [Enter] 下一步  [Esc] 返回
tui-agents-hints-mcp =     [↑↓] 导航  [Space] 切换  [Enter] 创建  [Esc] 返回
tui-agents-hints-save =     [↑↓] 导航  [Space] 切换  [Enter] 保存  [Esc] 取消

tui-agents-placeholder-name = my-agent
tui-agents-placeholder-desc = 自定义 Agent
tui-agents-placeholder-prompt = 您是一个有用的助手。
tui-agents-label-placeholder =     占位符: { $placeholder }

tui-agents-detail-id =   ID:       
tui-agents-detail-name =   名称:     
tui-agents-detail-state =   状态:     
tui-agents-detail-provider =   提供商:   
tui-agents-detail-model =   模型:     
tui-agents-detail-created =   创建时间: 
tui-agents-detail-active =   活跃时间: 
tui-agents-detail-tags =   标签:     
tui-agents-detail-caps =   功能:     
tui-agents-detail-parent =   父级:     
tui-agents-detail-children =   子级:     
tui-agents-detail-skills =   Skill:    
tui-agents-detail-mcp =   MCP:      
tui-agents-detail-all-skills = [所有 Skill]
tui-agents-detail-all-servers = [所有服务]
tui-agents-detail-none = [无]
tui-agents-default-desc = 自定义 { $name } Agent
tui-agents-default-prompt = 您是 { $name }，一个有用的助手。

# --- Workflows screen ---
tui-workflows-title-screen = 工作流
tui-workflows-header-id = ID
tui-workflows-header-name = 名称
tui-workflows-header-steps = 步骤数
tui-workflows-header-created = 创建时间
tui-workflows-loading = 正在加载工作流...
tui-workflows-empty-state = 未定义工作流。使用 [n] 创建一个。
tui-workflows-create-new-option =   + 创建新工作流
tui-workflows-hints-list =   [↑↓] 导航  [Enter] 查看运行  [x] 运行  [n] 新建  [r] 刷新
tui-workflows-title-runs = 运行历史: { $name }
tui-workflows-header-run-id = 运行 ID
tui-workflows-header-state = 状态
tui-workflows-header-duration = 耗时
tui-workflows-header-output = 输出
tui-workflows-runs-empty = 尚无运行记录。在列表中按 [x] 运行。
tui-workflows-hints-runs =   [↑↓] 导航  [r] 刷新  [Esc] 返回
tui-workflows-title-create = 创建新工作流
tui-workflows-create-step =   第 { $current } 步，共 { $total } 步
tui-workflows-label-name = 工作流名称:
tui-workflows-placeholder-name = my-workflow
tui-workflows-label-desc = 描述:
tui-workflows-placeholder-desc = 此工作流的作用
tui-workflows-label-steps = 步骤 (JSON 数组):
tui-workflows-placeholder-steps = [{"action":"..."}]
tui-workflows-label-review = 确认 — 按 Enter 创建
tui-workflows-review-name =   名称:  
tui-workflows-review-desc =   描述:  
tui-workflows-hints-create-submit =   [Enter] 创建  [Esc] 返回
tui-workflows-hints-create-next =   [Enter] 下一步  [Esc] 返回
tui-workflows-title-run-input = 运行: { $name }
tui-workflows-label-run-input =   输入 (JSON 或文本):
tui-workflows-placeholder-run-input = 输入工作流输入...
tui-workflows-hints-run-input =   [Enter] 运行  [Esc] 取消
tui-workflows-title-run-result = 工作流运行结果
tui-workflows-running = 正在运行工作流...
tui-workflows-result-complete = 完成
tui-workflows-result-empty = 无结果。
tui-workflows-hints-run-result =   [Enter/Esc] 返回

# --- Triggers screen ---
tui-triggers-title-screen = 触发器
tui-triggers-header-agent = Agent
tui-triggers-header-pattern = 模式
tui-triggers-header-fires = 触发次数
tui-triggers-header-status = 状态
tui-triggers-loading = 正在加载触发器...
tui-triggers-empty-state = 未配置触发器。使用 [n] 创建一个。
tui-triggers-status-active = ● 激活
tui-triggers-status-off = ○ 关闭
tui-triggers-create-new-option =   + 创建新触发器
tui-triggers-hints-list =   [↑↓] 导航  [Enter] 创建  [d] 删除  [r] 刷新
tui-triggers-title-create = 创建新触发器
tui-triggers-create-step =   第 { $current } 步，共 { $total } 步
tui-triggers-label-agent-id = Agent ID:
tui-triggers-placeholder-agent-id = agent-uuid
tui-triggers-label-pattern-picker =   选择模式类型:
tui-triggers-prompt-param = { $type } 的模式参数:
tui-triggers-placeholder-pattern-param = 例如 .*error.*
tui-triggers-label-prompt = 提示词模板:
tui-triggers-placeholder-prompt = 处理此项: {"{"}event{"}"}
tui-triggers-label-max-fires = 最大触发次数 (0 = 无限制):
tui-triggers-placeholder-max-fires = 0
tui-triggers-review-agent =   Agent:   
tui-triggers-review-pattern =   模式:   
tui-triggers-review-prompt =   提示词: 
tui-triggers-review-max =   最大:    
tui-triggers-review-unlimited = 无限制
tui-triggers-review-confirm = 按 Enter 创建此触发器。
tui-triggers-hints-create-submit =   [Enter] 创建  [Esc] 返回
tui-triggers-hints-create-select =   [↑↓] 导航  [Enter] 选择  [Esc] 返回
tui-triggers-hints-create-next =   [Enter] 下一步  [Esc] 返回

tui-triggers-type-lifecycle-name = Lifecycle
tui-triggers-type-lifecycle-desc = Agent 生命周期事件 (start, stop, error)
tui-triggers-type-agentspawned-name = AgentSpawned
tui-triggers-type-agentspawned-desc = 新 Agent 创建时触发
tui-triggers-type-contentmatch-name = ContentMatch
tui-triggers-type-contentmatch-desc = 匹配消息内容 (正则表达式)
tui-triggers-type-schedule-name = Schedule
tui-triggers-type-schedule-desc = 类 Cron 调度触发器
tui-triggers-type-webhook-name = Webhook
tui-triggers-type-webhook-desc = HTTP Webhook 触发器
tui-triggers-type-channelmessage-name = ChannelMessage
tui-triggers-type-channelmessage-desc = 通道收到消息时触发



