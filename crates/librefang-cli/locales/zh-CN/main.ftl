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
channel-unknown-fix = 可用通道：telegram、discord、slack、whatsapp、email、signal、matrix
channel-test-ok = 通道测试通过
channel-test-fail = 通道测试失败
section-setup-telegram = 设置 Telegram
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
