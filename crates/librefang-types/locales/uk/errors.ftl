# --- API error messages (Ukrainian) ---

# Agent errors
api-error-agent-not-found = Агент не знайдений
api-error-agent-spawn-failed = Не вдалося запустити агента
api-error-agent-invalid-id = Недійсний ID агента
api-error-agent-already-exists = Агент уже існує
api-error-agent-no-workspace = Агент не має робочого простору
api-error-agent-not-found-or-terminated = Агент не знайдений або вже завершив роботу
api-error-agent-vanished = Агент зник під час оновлення
api-error-agent-no-agents-available = Немає доступних агентів
api-error-agent-no-target = Цільовий агент не знайдений. Вкажіть agent_id або спочатку запустіть агента.
api-error-agent-source-not-found = Вихідний агент не знайдений
api-error-agent-target-not-found = Цільовий агент не знайдений
api-error-agent-execution-failed = Помилка виконання агента: { $error }
api-error-agent-clone-spawn-failed = Не вдалося запустити клон: { $error }
api-error-agent-error = Помилка агента: { $error }
api-error-agent-not-found-with-id = Агента не знайдено: { $id }
api-error-agent-invalid-sort = Недійсне поле сортування '{ $field }'. Допустимі поля: { $valid }

# Message errors
api-error-message-too-large = Повідомлення занадто велике (макс. 64KB)
api-error-message-delivery-failed = Не вдалося доставити повідомлення: { $reason }
api-error-message-required = Повідомлення є обов'язковим
api-error-message-missing-field = Відсутнє поле 'message'
api-error-message-streaming-failed = Не вдалося надіслати потокове повідомлення

# Template errors
api-error-template-invalid-name = Недійсна назва темплейту
api-error-template-not-found = Темплейт '{ $name }' не знайдено
api-error-template-parse-failed = Не вдалося розпарсити темплейт: { $error }
api-error-template-required = Необхідно вказати 'manifest_toml' або 'template'
api-error-template-invalid-manifest = Недійсний маніфест темплейту
api-error-template-read-failed = Не вдалося прочитати темплейт

# Manifest errors
api-error-manifest-too-large = Маніфест занадто великий (макс. 1MB)
api-error-manifest-invalid-format = Недійсний формат маніфесту
api-error-manifest-signature-mismatch = Підписаний вміст маніфесту не відповідає manifest_toml
api-error-manifest-signature-failed = Помилка перевірки підпису маніфесту
api-error-manifest-invalid = Недійсний маніфест: { $error }

# Auth errors
api-error-auth-invalid-key = Недійсний API-ключ
api-error-auth-missing-header = Відсутній заголовок Authorization: Bearer <api_key>
api-error-auth-missing = API-ключ не налаштований для цього провайдера

# Session errors
api-error-session-load-failed = Не вдалося завантажити сесію
api-error-session-not-found = Сесію не знайдено
api-error-session-invalid-id = Недійсний ID сесії
api-error-session-no-label = Не знайдено сесії з такою міткою
api-error-session-cleanup-expired-failed = Не вдалося очистити застарілі сесії: { $error }
api-error-session-cleanup-excess-failed = Не вдалося очистити надлишкові сесії: { $error }

# Workflow errors
api-error-workflow-missing-steps = Відсутній масив 'steps'
api-error-workflow-step-needs-agent = Крок '{ $step }' потребує 'agent_id' або 'agent_name'
api-error-workflow-invalid-id = Недійсний ID воркфлоу
api-error-workflow-execution-failed = Помилка виконання воркфлоу
api-error-workflow-not-found = Воркфлоу не знайдено

# Trigger errors
api-error-trigger-missing-agent-id = Відсутній 'agent_id'
api-error-trigger-invalid-agent-id = Недійсний agent_id
api-error-trigger-invalid-pattern = Недійсний шаблон тригера
api-error-trigger-missing-pattern = Відсутній 'pattern'
api-error-trigger-registration-failed = Не вдалося зареєструвати тригер (агента не знайдено?)
api-error-trigger-invalid-id = Недійсний ID тригера
api-error-trigger-not-found = Тригер не знайдено

# Budget errors
api-error-budget-invalid-amount = Недійсна сума бюджету
api-error-budget-update-failed = Не вдалося оновити бюджет
api-error-budget-provide-at-least-one = Вкажіть хоча б один із параметрів: max_cost_per_hour_usd, max_cost_per_day_usd, max_cost_per_month_usd, max_llm_tokens_per_hour

# Config errors
api-error-config-parse-failed = Не вдалося розпарсити конфігурацію: { $error }
api-error-config-write-failed = Не вдалося записати конфігурацію: { $error }
api-error-config-save-failed = Не вдалося зберегти конфігурацію: { $error }
api-error-config-remove-failed = Не вдалося видалити конфігурацію: { $error }
api-error-config-missing-toml = Відсутнє поле toml_content

# Profile errors
api-error-profile-not-found = Профіль '{ $name }' не знайдено

# Cron errors
api-error-cron-invalid-id = Недійсний ID cron-завдання
api-error-cron-not-found = Cron-завдання не знайдено
api-error-cron-create-failed = Не вдалося створити cron-завдання: { $error }
api-error-cron-invalid-expression = Недійсний cron-вираз
api-error-cron-invalid-expression-detail = Недійсний cron-вираз: потрібно 5 полів (хвилина година день місяць день_тижня)
api-error-cron-missing-field = Відсутнє поле 'cron'

# Goal errors
api-error-goal-not-found = Ціль не знайдено
api-error-goal-not-found-with-id = Ціль '{ $id }' не знайдено
api-error-goal-missing-title = Відсутнє або порожнє поле 'title'
api-error-goal-title-too-long = Заголовок занадто довгий (макс. 256 символів)
api-error-goal-description-too-long = Опис занадто довгий (макс. 4096 символів)
api-error-goal-invalid-status = Недійсний статус. Має бути один із: pending, in_progress, completed, cancelled
api-error-goal-progress-range = Прогрес має бути в діапазоні 0-100
api-error-goal-parent-not-found = Батьківську ціль '{ $id }' не знайдено
api-error-goal-self-parent = Ціль не може бути своєю власною батьківською ціллю
api-error-goal-circular-parent = Виявлено циклічне посилання на батьківську ціль
api-error-goal-save-failed = Не вдалося зберегти ціль: { $error }
api-error-goal-update-failed = Не вдалося оновити ціль: { $error }
api-error-goal-delete-failed = Не вдалося видалити ціль: { $error }
api-error-goal-load-failed = Не вдалося завантажити цілі: { $error }
api-error-goal-title-empty = Заголовок не може бути порожнім
api-error-goal-status-invalid = Недійсний статус

# Memory errors
api-error-memory-not-enabled = Проактивна пам'ять не увімкнена
api-error-memory-not-found = Пам'ять не знайдено
api-error-memory-operation-failed = Помилка операції з пам'яттю
api-error-memory-export-failed = Не вдалося експортувати пам'ять
api-error-memory-import-failed = Не вдалося імпортувати пам'ять під час очищення
api-error-memory-key-not-found = Ключ не знайдено
api-error-memory-missing-kv = Тіло запиту відсутнє або містить недійсний об'єкт 'kv'
api-error-memory-serialization-error = Помилка серіалізації
api-error-memory-missing-ids = Відсутній масив 'ids'

# Network / A2A errors
api-error-network-not-enabled = Мережа пірів не увімкнена
api-error-network-peer-not-found = Пір не знайдений
api-error-network-a2a-not-found = A2A-агент '{ $url }' не знайдений
api-error-network-connection-failed = Помилка підключення: { $error }
api-error-network-auth-failed = Помилка автентифікації (HTTP { $status })
api-error-network-task-post-failed = Не вдалося опублікувати таску: { $error }
api-error-network-missing-url = Відсутній query-параметр 'url'

# Plugin errors
api-error-plugin-missing-name = Відсутнє поле 'name'
api-error-plugin-missing-name-registry = Відсутнє поле 'name' для встановлення з реєстру
api-error-plugin-missing-path = Відсутнє поле 'path' для локального встановлення
api-error-plugin-missing-url = Відсутнє поле 'url' для встановлення з git
api-error-plugin-invalid-source = Недійсне джерело. Використовуйте одне з: 'registry', 'local', 'git'

# Channel errors
api-error-channel-unknown = Невідомий канал
api-error-channel-missing-agent-id = Відсутнє обов'язкове поле: agent_id
api-error-channel-invalid-from = Недійсний from_agent_id
api-error-channel-invalid-to = Недійсний to_agent_id

# Provider errors
api-error-provider-missing-alias = Відсутнє обов'язкове поле: alias
api-error-provider-missing-model-id = Відсутнє обов'язкове поле: model_id
api-error-provider-missing-id = Відсутнє обов'язкове поле: id
api-error-provider-missing-key = Відсутнє або порожнє поле 'key'
api-error-provider-alias-exists = Аліас '{ $alias }' уже існує
api-error-provider-alias-not-found = Аліас '{ $alias }' не знайдено
api-error-provider-model-not-found = Модель '{ $id }' не знайдено
api-error-provider-not-found = Провайдер '{ $name }' не знайдений
api-error-provider-model-exists = Модель '{ $id }' уже існує у провайдері '{ $provider }'
api-error-provider-custom-model-not-found = Кастомну модель '{ $id }' не знайдено
api-error-provider-no-key-required = Цей провайдер не потребує API-ключа
api-error-provider-key-not-configured = API-ключ провайдера не налаштований
api-error-provider-secrets-write-failed = Не вдалося записати secrets.env: { $error }
api-error-provider-secrets-update-failed = Не вдалося оновити secrets.env: { $error }
api-error-provider-invalid-url = Недійсний формат URL
api-error-provider-missing-url = Відсутнє або порожнє поле 'url'
api-error-provider-missing-base-url = Відсутнє або порожнє поле 'base_url'
api-error-provider-unknown = Невідомий провайдер '{ $name }'
api-error-provider-base-url-invalid = base_url має починатися з http:// або https://
api-error-provider-missing-model = Відсутнє поле 'model'
api-error-provider-token-save-failed = Не вдалося зберегти токен: { $error }
api-error-provider-unknown-poll = Невідомий poll_id
api-error-provider-secret-write-failed = Не вдалося записати секрет: { $error }

# Skill errors
api-error-skill-missing-name = Відсутнє або порожнє поле 'name'
api-error-skill-invalid-name = Назва скіла може містити лише буквено-цифрові символи, дефіси та підкреслення
api-error-skill-not-found-source = Вихідний код для цього скіла не знайдено
api-error-skill-only-prompt = З веб-інтерфейсу можна створювати лише скіли типу prompt-only
api-error-skill-name-too-long = Назва перевищує максимальну довжину (256 символів)
api-error-skill-description-too-long = Опис перевищує максимальну довжину ({ $max } символів)
api-error-skill-dir-create-failed = Не вдалося створити директорію скіла: { $error }
api-error-skill-toml-write-failed = Не вдалося записати skill.toml: { $error }
api-error-skill-install-failed = Помилка встановлення: { $error }

# Hand errors
api-error-hand-not-found = Hand не знайдено: { $id }
api-error-hand-definition-not-found = Визначення Hand не знайдено
api-error-hand-instance-not-found = Екземпляр Hand не знайдено

# MCP errors
api-error-mcp-missing-name = Відсутнє поле 'name'
api-error-mcp-missing-transport = Відсутнє поле 'transport'
api-error-mcp-invalid-config = Недійсна конфігурація MCP-сервера: { $error }
api-error-mcp-not-found = MCP-сервер '{ $name }' не знайдено
api-error-mcp-write-failed = Не вдалося записати конфігурацію: { $error }

# Integration/Extension errors
api-error-integration-not-found = Інтеграцію '{ $id }' не знайдено
api-error-integration-missing-id = Відсутнє поле 'id'
api-error-extension-not-found = Розширення '{ $id }' не знайдено

# System errors
api-error-system-cli-not-found = CLI не знайдено в PATH

# KV / Structured memory errors
api-error-kv-missing-fields = Відсутній об'єкт 'fields'
api-error-kv-missing-value = Відсутнє поле 'value'
api-error-kv-array-empty = Масив не може бути порожнім
api-error-kv-missing-path = Відсутнє поле 'path'

# Approval errors
api-error-approval-invalid-id = Недійсний ID апруву
api-error-approval-not-found = Апрув не знайдено

# Webhook errors
api-error-webhook-not-enabled = Тригери вебхуків не увімкнені
api-error-webhook-invalid-id = Недійсний ID вебхука
api-error-webhook-not-found = Вебхук не знайдено
api-error-webhook-missing-url = Відсутнє поле 'url'
api-error-webhook-missing-events = Відсутній масив 'events'
api-error-webhook-invalid-events = Типи подій мають бути рядками
api-error-webhook-event-types-required = Необхідно вказати хоча б один тип події
api-error-webhook-url-unreachable = URL вебхука недоступний: { $error }
api-error-webhook-event-publish-failed = Не вдалося опублікувати подію: { $error }
api-error-webhook-invalid-url = Недійсний формат URL вебхука
api-error-webhook-agent-exec-failed = Помилка виконання агента вебхука: { $error }
api-error-webhook-reach-failed = Не вдалося зв'язатися з URL вебхука: { $error }
api-error-webhook-unknown-event = Невідомий тип події '{ $event }'. Допустимі типи: { $valid }

# Backup errors
api-error-backup-not-found = Бекап не знайдений
api-error-backup-file-not-found = Файл бекапу не знайдений
api-error-backup-invalid-filename = Недійсна назва файлу бекапу
api-error-backup-invalid-filename-zip = Недійсна назва файлу бекапу — має бути .zip файлом
api-error-backup-missing-manifest = В архіві бекапу відсутній manifest.json — це недійсний бекап LibreFang
api-error-backup-dir-create-failed = Не вдалося створити директорію бекапу: { $error }
api-error-backup-file-create-failed = Не вдалося створити файл бекапу: { $error }
api-error-backup-finalize-failed = Не вдалося фіналізувати бекап: { $error }
api-error-backup-open-failed = Не вдалося відкрити бекап: { $error }
api-error-backup-invalid-archive = Недійсний архів бекапу: { $error }
api-error-backup-delete-failed = Не вдалося видалити бекап: { $error }

# Schedule errors
api-error-schedule-not-found = Розклад не знайдено
api-error-schedule-missing-cron = Відсутнє поле 'cron'
api-error-schedule-missing-enabled = Відсутнє поле 'enabled'
api-error-schedule-invalid-cron = Недійсний cron-вираз
api-error-schedule-invalid-cron-detail = Недійсний cron-вираз: потрібно 5 полів (хвилина година день місяць день_тижня)
api-error-schedule-save-failed = Не вдалося зберегти розклад: { $error }
api-error-schedule-update-failed = Не вдалося оновити розклад: { $error }
api-error-schedule-delete-failed = Не вдалося видалити розклад: { $error }
api-error-schedule-load-failed = Не вдалося завантажити розклад: { $error }

# Job errors
api-error-job-invalid-id = Недійсний ID джоби
api-error-job-not-found = Джоби не знайдено
api-error-job-not-retryable = Таску не знайдено або вона перебуває в стані, який не підлягає повторній спробі (має бути завершена або неуспішна)
api-error-job-disappeared-cancel = Таска зникла після скасування
api-error-job-disappeared-complete = Таска зникла після завершення

# Task errors
api-error-task-not-found = Таску не знайдено
api-error-task-disappeared = Таска зникла

# Pairing errors
api-error-pairing-not-enabled = Пейринг не увімкнений
api-error-pairing-invalid-token = Недійсний або відсутній токен

# Binding errors
api-error-binding-out-of-range = Індекс байндингу поза діапазоном

# Command errors
api-error-command-not-found = Команду '{ $name }' не знайдено

# File/Upload errors
api-error-file-not-found = Файл не знайдений
api-error-file-not-in-whitelist = Файл не входить до білого списку (whitelist)
api-error-file-too-large = Файл занадто великий (макс. { $max })
api-error-file-content-too-large = Вміст файлу занадто великий (макс. 32KB)
api-error-file-empty-body = Порожнє тіло файлу
api-error-file-save-failed = Не вдалося зберегти файл
api-error-file-missing-filename = Відсутнє поле 'filename'
api-error-file-missing-path = Відсутнє поле 'path'
api-error-file-path-too-deep = Шлях занадто глибокий (макс. 3 рівні)
api-error-file-path-traversal = Обхід шляху (path traversal) заборонений
api-error-file-unsupported-type = Непідтримуваний тип контенту. Дозволено: image/*, text/*, audio/*, application/pdf
api-error-file-upload-dir-failed = Не вдалося створити директорію завантаження
api-error-file-dir-not-found = Директорію не знайдено
api-error-file-workspace-error = Помилка шляху робочого простору

# Tool errors
api-error-tool-provide-allowlist = Вкажіть 'tool_allowlist' та/або 'tool_blocklist'
api-error-tool-not-found = Тулу не знайдено: { $name }
api-error-tool-invoke-disabled = Прямий виклик тули вимкнено. Увімкніть '[tool_invoke] enabled = true' та додайте тулу до 'allowlist'.
api-error-tool-invoke-denied = Виклик тули '{ $name }' заборонено у '[tool_invoke] allowlist'
api-error-tool-requires-agent = Тула '{ $name }' потребує підтвердження людиною та не може бути викликана без контексту агента; викликайте її через агента

# Validation errors
api-error-validation-content-empty = Вміст не може бути порожнім
api-error-validation-name-empty = new_name не може бути порожнім
api-error-validation-title-required = Заголовок (title) є обов'язковим
api-error-validation-avatar-url-invalid = URL аватара має бути http/https або data URI
api-error-validation-color-invalid = Колір має бути шістнадцятковим кодом (hex), що починається з '#'

# General errors
api-error-not-found = Ресурс не знайдений
api-error-internal = Внутрішня помилка сервера
api-error-bad-request = Некоректний запит: { $reason }
api-error-rate-limited = Перевищено ліміт запитів. Спробуйте пізніше.

# Generic catch-all — interpolates the underlying error string verbatim.
# Used by 41+ HTTP 500 handlers as a stopgap until each route is moved to a
# typed MemoryRouteError-style helper. Without this key, every `t_args("api-error-generic", …)`
# call returns the literal key as the response body and `$error` interpolation never runs.
api-error-generic = Помилка: { $error }
