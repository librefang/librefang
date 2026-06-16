# --- Daemon lifecycle ---
daemon-starting = Запуск демона...
daemon-stopped = Демон LibreFang зупинений.
kernel-booted = Ядро завантажено ({ $provider }/{ $model })
models-available = Доступно моделей: { $count }
agents-loaded = Завантажено агентів: { $count }
daemon-started-bg = Демон запущений у фоновому режимі
daemon-still-starting = Демон запущений у фоновому режимі та все ще запускається
daemon-stopped-ok = Демон зупинений
daemon-stopped-forced = Демон зупинений (примусово)
daemon-error = Помилка демона: { $error }
daemon-already-running = Демон уже запущений на { $url }
daemon-already-running-fix = Використовуйте `librefang status` для перевірки або спочатку зупиніть його
daemon-not-running = Демон не запущений.
daemon-not-running-start = Демон не запущений. Запустіть його за допомогою: librefang start
daemon-no-running-found = Запущеного демона не знайдено
daemon-no-running-found-fix = Чи запущений він? Перевірте за допомогою: librefang status
daemon-restarting = Перезапуск демона...
daemon-no-running-starting = Запущеного демона не знайдено; запуск нового демона
daemon-bg-exited = Фоновий демон завершив роботу до того, як став працездатним ({ $status })
daemon-bg-exited-fix = Перевірте логи запуску: { $path }
daemon-bg-wait-fail = Помилка під час очікування фонового демона
daemon-bg-wait-fail-fix = { $error }. Перевірте логи запуску: { $path }
daemon-launch-fail = Не вдалося запустити фоновий демон
daemon-no-running-auto = Демон не запущений — запускаємо зараз...
daemon-started = Демон запущений
daemon-start-fail = Не вдалося запустити демона: { $error }
daemon-start-fail-fix = Запустіть його вручну: librefang start
shutdown-request-fail = Помилка запиту на вимкнення ({ $status })
could-not-reach-daemon = Не вдалося зв'язатися з демоном: { $error }
# Issue #4693 — after `curl install.sh | sh` upgrades the binary without
# restarting the running daemon, `librefang restart` (new CLI) hits the old
# daemon's `/api/shutdown` and is rejected with 401 because the new CLI's
# Authorization header does not match the old daemon's expected key (typical
# trigger: locked vault, rotated `[api] api_key`, or freshly enabled
# dashboard credentials). Surface the cause + auto-fall-back to PID-based
# shutdown so users can move forward without hand-editing config.
shutdown-401-detected = Запит на вимкнення відхилено запущеним демоном (401 Unauthorized).
shutdown-401-explainer = Новий CLI не може автентифікуватися в запущеному демоні. Зазвичай це відбувається після оновлення бінарного файлу за допомогою `curl install.sh | sh` без перезапуску демона — запущений демон було запущено з іншим api_key або не вдалося розблокувати сховище (vault), яке його містить.
shutdown-401-fallback-attempt = Перехід до зупинки на основі PID (PID { $pid })...
shutdown-401-fallback-success = Демон зупинено через PID { $pid }
shutdown-401-fallback-fail = Зупинка на основі PID також не спрацювала.
shutdown-401-fallback-fix = Зупиніть демон вручну, а потім запустіть його знову:
    kill { $pid }    # або: kill -9 { $pid } якщо він не виходить
    librefang start
shutdown-401-no-pid-fix = Не вдалося прочитати PID демона з { $path }. Виконайте `ps -ef | grep librefang`, щоб знайти його, а потім `kill <pid>` та `librefang start`.

# --- Labels ---
label-api = API
label-dashboard = Панель приладів
label-provider = Провайдер
label-model = Модель
label-pid = PID
label-log = Log
label-status = Статус
label-agents = Агенти
label-data-dir = Директорія даних
label-uptime = Час роботи
label-version = Версія
label-daemon = Демон
label-id = ID
label-active-agents = Активні агенти
label-pairing-code = Код пейрингу
label-expires = Закінчується

# --- Hints ---
hint-open-dashboard = Відкрийте панель приладів у браузері або виконайте `librefang chat`
hint-stop-daemon = Використовуйте `librefang stop`, щоб зупинити демона
hint-tail-stop = Ctrl+C зупиняє відстеження логів; демон продовжує працювати
hint-check-status = Виконайте `librefang status`, щоб перевірити готовність
hint-start-daemon = Запустіть його за допомогою: librefang start
hint-start-daemon-cmd = Запуск демона: librefang start
hint-or-chat = Або спробуйте `librefang chat`, що працює без демона
hint-non-interactive = Виявлено неінтерактивний термінал — запуск у швидкому режимі
hint-non-interactive-wizard = Для інтерактивного майстра виконайте: librefang init (у терміналі)
hint-starting-chat = Запуск сесії чату...
hint-no-api-keys = Не знайдено API ключів провайдерів LLM
hint-groq-free = Groq пропонує безкоштовний тариф: https://console.groq.com
hint-ollama-local = Або встановіть Ollama для локальних моделей: https://ollama.com
hint-gemini-free = Gemini пропонує безкоштовний тариф: https://aistudio.google.com
hint-deepseek-free = DeepSeek пропонує 5 млн безкоштовних токенів: https://platform.deepseek.com
guide-title = Швидке налаштування
guide-free-providers-title = Виберіть безкоштовного провайдера для початку (налаштування 2 хв):
guide-get-free-key = Отримайте безкоштовний API ключ
guide-paste-key-placeholder = вставте ваш API ключ сюди
guide-setting-up = Налаштування
guide-testing-key = Тестування ключа...
guide-key-verified = ✓ Ключ підтверджено!
guide-test-key-unverified = ⚠ Не вдалося підтвердити (може все одно працювати)
guide-help-select = ↑↓ навігація  Enter вибір  s/Esc пропустити
guide-help-paste = Вставити ключ + Enter  Esc назад
guide-help-wait = Будь ласка, зачекайте...
guide-paste-key-hint = Скопіюйте API ключ із браузера та вставте його нижче.
hint-could-not-open-browser = Не вдалося автоматично відкрити браузер.
hint-could-not-open-browser-visit = Не вдалося відкрити браузер. Відвідайте: { $url }
hint-dashboard-url = Панель приладів: { $url }
hint-try-dashboard = Спробуйте: librefang dashboard
hint-install-desktop = Встановіть його за допомогою: cargo install librefang-desktop
hint-fallback-web-dashboard = Перехід до веб-панелі приладів...
hint-then-open-dashboard = Потім відкрийте: http://127.0.0.1:4545
hint-chat-with-agent = Чат: librefang chat { $name }
hint-agent-lost-on-exit = Примітка: Агент буде втрачений після завершення цього процесу
hint-persistent-agents = Для постійних агентів спочатку запустіть `librefang start`
hint-url-copied = URL скопійовано в буфер обміну
hint-doctor-repair = Виконайте `librefang doctor --repair` для автоматичного виправлення
hint-run-init = Виконайте `librefang init` для налаштування директорії агентів
hint-run-start = Виконайте `librefang start` для запуску демона
hint-config-edit = Виправте за допомогою: librefang config edit
hint-set-key = Або виконайте: librefang config set-key groq
hint-set-key-provider = Встановити пізніше: librefang config set-key email (або export EMAIL_PASSWORD=...)

# --- Init ---
init-quick-success = LibreFang ініціалізовано (швидкий режим)
init-interactive-success = LibreFang ініціалізовано!
init-cancelled = Налаштування скасовано.
init-next-start = Запуск демона:  librefang start
init-next-chat = Чат:              librefang chat

# --- Error messages ---
error-home-dir = Не вдалося визначити домашню директорію
error-create-dir = Не вдалося створити { $path }
error-create-dir-fix = Перевірте права доступу для { $path }
error-write-config = Не вдалося записати конфігурацію
error-config-created = Створено: { $path }
error-config-exists = Конфігурація вже існує: { $path }

# --- Daemon communication errors ---
error-daemon-returned = Демон повернув помилку ({ $status })
error-daemon-returned-fix = Перевірте логи демона за допомогою: librefang logs --follow
error-request-timeout = Час очікування запиту минув
error-request-timeout-fix = Агент може обробляти складний запит. Спробуйте ще раз або перевірте `librefang status`
error-connect-refused = Не вдалося підключитися до демона
error-connect-refused-fix = Чи запущений демон? Запустіть його за допомогою: librefang start
error-daemon-comm = Помилка зв'язку з демоном: { $error }
error-daemon-comm-fix = Перевірте `librefang status` або перезапустіть: librefang start

# --- Boot errors ---
error-boot-config = Не вдалося розібрати конфігурацію
error-boot-config-fix = Перевірте синтаксис вашого config.toml: librefang config show
error-boot-db = Помилка бази даних (файл може бути заблокований)
error-boot-db-fix = Перевірте, чи запущений інший процес LibreFang: librefang status
error-boot-auth = Помилка автентифікації провайдера LLM
error-boot-auth-fix = Виконайте `librefang doctor`, щоб перевірити конфігурацію API-ключів
error-boot-generic = Не вдалося завантажити ядро: { $error }
error-boot-generic-fix = Виконайте `librefang doctor`, щоб діагностувати проблему

# --- Require daemon ---
error-require-daemon = `librefang { $command }` вимагає запущеного демона
error-require-daemon-fix = Запустіть демона: librefang start

# --- Provider detection ---
detected-provider = Виявлено { $display } ({ $env_var })
detected-gemini = Виявлено Gemini (GOOGLE_API_KEY)
detected-ollama = Виявлено Ollama, що працює локально (API-ключ не потрібен)

# --- Desktop app ---
desktop-launching = Запуск LibreFang Desktop...
desktop-started = Десктопний додаток запущений.
desktop-launch-fail = Не вдалося запустити десктопний додаток: { $error }
desktop-not-found = Десктопний додаток не знайдено.

# --- Dashboard ---
dashboard-opening = Відкриття панелі приладів на { $url }

# --- Agent commands ---
agent-spawned = Агент '{ $name }' запущений
agent-spawned-inprocess = Агент '{ $name }' запущений (у процесі)
agent-spawn-failed = Не вдалося запустити: { $error }
agent-spawn-agent-failed = Не вдалося запустити агента: { $error }
agent-template-not-found = Темплейт '{ $name }' не знайдено
agent-template-not-found-fix = Виконайте `librefang agent new`, щоб переглянути доступні темплейти
agent-no-templates = Темплейтів агентів не знайдено
agent-no-templates-fix = Виконайте `librefang init`, щоб налаштувати директорії агентів
agent-template-parse-fail = Не вдалося розібрати темплейт '{ $name }': { $error }
agent-template-parse-fail-fix = Маніфест темплейту може бути пошкоджений
agent-killed = Агент { $id } зупинений.
agent-kill-failed = Не вдалося зупинити агента: { $error }
agent-invalid-id = Некоректний ID агента: { $id }
agent-model-set = Модель агента { $id } встановлено на { $value }.
agent-set-model-failed = Не вдалося встановити модель: { $error }
agent-no-daemon-for-set = Запущеного демона не знайдено. Запустіть його за допомогою: librefang start
agent-unknown-field = Невідоме поле: { $field }. Підтримувані поля: model
agent-no-agents = Немає запущених агентів.
agent-spawn-success = Агента успішно запущено!
agent-spawn-inprocess-mode = Агента запущено (внутрішньопроцесний режим).
agent-note-lost = Примітка: Агент буде втрачений після завершення цього процесу.
agent-note-persistent = Для постійних агентів спочатку запустіть `librefang start`.
section-agent-templates = Доступні темплейти агентів

# --- Manifest errors ---
manifest-not-found = Файл маніфесту не знайдено: { $path }
manifest-not-found-fix = Використовуйте `librefang agent new`, щоб запустити з темплейту
error-reading-manifest = Помилка читання маніфесту: { $error }
error-parsing-manifest = Помилка парсингу маніфесту: { $error }

# --- Status ---
section-daemon-status = Статус демона LibreFang
section-status-inprocess = Статус LibreFang (внутрішньопроцесний)
section-active-agents = Active Agents
section-persisted-agents = Persisted Agents
label-daemon-not-running = НЕ ЗАПУЩЕНИЙ
label-home = Домашня директорія
label-platform = Платформа
label-sessions = Сесії
label-memory = Пам'ять
label-started = Запущено
label-response = Відповідь
label-checks = Перевірки
section-status-locked = Обмежено (потрібен API-ключ)
hint-status-locked = Встановіть `api_key` у ~/.librefang/config.toml, щоб бачити агентів / сесії / пам'ять.
warn-public-bind = публічно прив'язано
warn-key-missing = не встановлено
section-recent-errors = Останні помилки (daemon.log)
section-verbose = Деталі
label-auth = Автентифікація
label-mcp = MCP-сервери
label-peers = OFP-піри
label-channels = Канали
label-skills = Скіли
label-hands = Hands
label-config-warnings = Попередження конфігурації
auth-none = немає (анонімно)
auth-api-key = API-ключ
auth-dashboard-login = логін панелі приладів
auth-user-keys = Ключів користувачів: { $count }

# --- Doctor ---
doctor-title = LibreFang Doctor
doctor-all-passed = Усі перевірки пройдено! LibreFang готовий до роботи.
doctor-repairs-applied = Виправлення застосовано. Запустіть `librefang doctor` знову для перевірки.
doctor-some-failed = Деякі перевірки не пройдено.
doctor-no-api-keys = Не знайдено API-ключів провайдерів LLM!
section-getting-api-key = Отримання API-ключа (безкоштовні тарифи)

# --- Security ---
section-security-status = Стан безпеки
label-audit-trail = Аудиторський слід
label-taint-tracking = Відстеження міток
label-wasm-sandbox = WASM-пісочниця
label-wire-protocol = Мережевий протокол
label-api-keys = API-ключі
label-manifests = Маніфести
value-audit-trail = Ланцюжок хешів Merkle (SHA-256)
value-taint-tracking = Мітки потоку інформації
value-wasm-sandbox = Подвійний облік (паливо + епоха)
value-wire-protocol = Взаємна автентифікація OFP HMAC-SHA256
value-api-keys = Zeroizing<String> (автоочищення при видаленні)
value-manifests = Підписано Ed25519
audit-verified = Цілісність аудиторського сліду підтверджено (ланцюжок Merkle валідний).
audit-failed = Перевірка цілісності аудиторського сліду НЕ ВДАЛАСЯ.

# --- Health ---
health-ok = Демон здоровий
health-not-running = Демон не запущений.

# --- Channel setup ---
section-channel-setup = Налаштування каналу
channel-configured = Канал { $name } налаштовано
channel-no-token = Токен не надано. Налаштування скасовано.
channel-no-email = Email не надано. Налаштування скасовано.
channel-token-saved = Токен збережено в ~/.librefang/.env
channel-app-token-saved = Токен додатка збережено в ~/.librefang/.env
channel-bot-token-saved = Токен бота збережено в ~/.librefang/.env
channel-password-saved = Пароль збережено в ~/.librefang/.env
channel-phone-saved = Телефон збережено в ~/.librefang/.env
channel-key-saved = { $key } збережено в ~/.librefang/.env
channel-unknown = Невідомий канал: { $name }
channel-unknown-fix = Доступні: discord, slack, whatsapp, email, signal, matrix
channel-test-ok = Тест каналу пройдено
channel-test-fail = Тест каналу не пройдено
section-setup-discord = Налаштування Discord
section-setup-slack = Налаштування Slack
section-setup-whatsapp = Налаштування WhatsApp
section-setup-email = Налаштування Email
section-setup-signal = Налаштування Signal
section-setup-matrix = Налаштування Matrix

# --- Vault ---
vault-initialized = Зашифроване сховище ініціалізовано.
vault-not-initialized = Сховище не ініціалізовано.
vault-not-init-run = Сховище не ініціалізовано. Виконайте: librefang vault init
vault-unlock-failed = Не вдалося розблокувати сховище: { $error }
vault-empty-value = Порожнє значення — не збережено.
vault-stored = Збережено '{ $key }' у сховіщі.
vault-store-failed = Не вдалося зберегти: { $error }
vault-removed = Видалено '{ $key }' зі сховища.
vault-key-not-found = Ключ '{ $key }' не знайдено в сховищі.
vault-remove-failed = Не вдалося видалити: { $error }
vault-rotate-no-vault = Файл сховища не знайдено. Спочатку виконайте `librefang vault init`.
vault-rotate-old-key-missing = LIBREFANG_VAULT_KEY_OLD не встановлено. Надайте поточний майстер-ключ (base64 від 32 байтів) перед ротацією.
vault-rotate-new-key-missing = LIBREFANG_VAULT_KEY_NEW не встановлено. Надайте новий майстер-ключ (base64 від 32 байтів) або передайте --from-stdin, щоб зчитати його з stdin.
vault-rotate-stdin-read-failed = Не вдалося зчитати новий ключ із stdin: { $error }
vault-rotate-stdin-empty = Новий ключ, зчитаний із stdin, виявився порожнім.
vault-rotate-same-key = LIBREFANG_VAULT_KEY_OLD та новий ключ ідентичні — відмова від ротації на той самий ключ.
vault-rotate-old-key-invalid = LIBREFANG_VAULT_KEY_OLD не є валідним 32-байтовим ключем base64: { $error }
vault-rotate-new-key-invalid = Новий ключ не є валідним 32-байтовим ключем base64: { $error }
vault-rotate-unlock-failed = Не вдалося розблокувати сховище за допомогою СТАРОГО ключа: { $error }. Перевірте, чи відповідає LIBREFANG_VAULT_KEY_OLD ключу, яким сховище було спочатку зашифровано.
vault-rotate-sentinel-failed = Перевірка сентингеля сховища не вдалася під СТАРИМ ключем: { $error }
vault-rotate-rewrap-failed = Не вдалося перешифрувати сховище новим ключем: { $error }. Оригінальний файл сховища не змінено.
vault-rotate-success = Сховище перешифровано під новим майстер-ключем (збережено користувацьких записів: { $count }).
vault-rotate-next-step = Далі: встановіть LIBREFANG_VAULT_KEY у нове значення перед перезапуском демона.

# --- Cron ---
cron-created = Створено Cron-завдання: { $id }
cron-create-failed = Не вдалося створити Cron-завдання: { $error }
cron-deleted = Cron-завдання { $id } видалено.
cron-delete-failed = Не вдалося видалити Cron-завдання: { $error }
cron-toggled = Cron-завдання { $id } { $action }о.
cron-toggle-failed = Не вдалося { $action } Cron-завдання: { $error }

# --- Approvals ---
approval-responded = Апрув { $id } { $action }о.
approval-failed = Не вдалося { $action } апрув: { $error }

# --- Memory ---
memory-set = Встановлено { $key } для агента '{ $agent }'.
memory-set-failed = Не вдалося встановити пам'ять: { $error }
memory-deleted = Видалено ключ '{ $key }' для агента '{ $agent }'.
memory-delete-failed = Не вдалося видалити пам'ять: { $error }

# --- Devices ---
section-device-pairing = Пейринг пристроїв
device-scan-qr = Відскануйте цей QR-код за допомогою мобільного додатка LibreFang:
device-removed = Пристрій { $id } видалено.
device-remove-failed = Не вдалося видалити пристрій: { $error }

# --- Webhooks ---
webhook-created = Вебхук створено: { $id }
webhook-create-failed = Не вдалося створити вебхук: { $error }
webhook-deleted = Вебхук { $id } видалено.
webhook-delete-failed = Не вдалося видалити вебхук: { $error }
webhook-test-ok = Тестове корисне навантаження для вебхуку { $id } успішно надіслано.
webhook-test-failed = Не вдалося протестувати вебхук: { $error }

# --- Models ---
model-set-success = Модель за замовчуванням встановлена на: { $model }
model-set-failed = Не вдалося встановити model: { $error }
model-no-catalog = У каталозі немає моделей.
section-select-model = Виберіть модель
model-out-of-range = Номер поза діапазоном (1-{ $max })

# --- Config ---
config-set-success = Значення конфігурації встановлено.
config-unset-success = Ключ конфігурації видалено.
config-no-file = Файл конфігурації не знайдено
config-no-file-fix = Run `librefang init` first
config-read-failed = Не вдалося прочитати конфігурацію: { $error }
config-parse-error = Помилка парсингу конфігурації: { $error }
config-parse-fix = Виправте синтаксис вашого config.toml або виконайте `librefang config edit`
config-parse-fix-alt = Спочатку виправте синтаксис вашого config.toml
config-key-not-found = Ключ не знайдено: { $key }
config-key-path-not-found = Шлях до ключа не знайдено: { $key }
config-empty-key = Порожній ключ
config-section-not-scalar = '{ $key }' є розділом, а не скаляром
config-section-not-scalar-fix = Використовуйте крапкову нотацію: { $key }.field_name
config-parent-not-table = Батьківський елемент для '{ $key }' не є таблицею
config-serialize-failed = Не вдалося серіалізувати конфігурацію: { $error }
config-write-failed = Не вдалося записати конфігурацію: { $error }
config-set-kv = Встановлено { $key } = { $value }
config-removed-key = Видалено ключ: { $key }
config-no-key = Ключ не надано. Скасовано.
config-saved-key = Збережено { $env_var } у ~/.librefang/.env
config-save-key-failed = Не вдалося зберегти ключ: { $error }
config-removed-env = Видалено { $env_var } з ~/.librefang/.env
config-remove-key-failed = Не вдалося видалити ключ: { $error }
config-env-not-set = { $env_var } не встановлено
config-set-key-hint = Встановіть його: librefang config set-key { $provider }
config-update-key-hint = Оновіть ключ: librefang config set-key { $provider }

# --- Hand commands ---
hand-install-deps-success = Залежності для Hands '{ $id }' встановлено.
hand-paused = Екземпляр Hands '{ $id }' призупинено.
hand-resumed = Екземпляр Hands '{ $id }' відновлено.

# --- Daemon notify ---
daemon-restart-notify = Перезапустіть демона, щоб застосувати: librefang restart

# --- System info ---
section-system-info = Системна інформація LibreFang

# --- Uninstall ---
uninstall-goodbye = LibreFang було видалено. Бувайте!
uninstall-cancelled = Скасовано.
uninstall-stopping-daemon = Зупинка запущеного демона...
uninstall-removed = Видалено { $path }
uninstall-remove-failed = Не вдалося видалити { $path }: { $error }
uninstall-removed-data-kept = Дані видалено (файли конфігурації збережено)
uninstall-removed-autostart-win = Видалено запис автозапуску з реєстру Windows
uninstall-removed-launch-agent = Видалено macOS launch agent
uninstall-remove-launch-fail = Не вдалося видалити launch agent: { $error }
uninstall-removed-autostart-linux = Видалено Linux autostart запис
uninstall-remove-autostart-fail = Не вдалося видалити autostart запис: { $error }
uninstall-removed-systemd = Видалено службу користувача systemd
uninstall-remove-systemd-fail = Не вдалося видалити службу systemd: { $error }
uninstall-cleaned-path = Очищено PATH від { $path }
uninstall-cleaned-path-win = Очищено PATH в користувацькому оточенні Windows

# --- Reset ---
reset-success = Видалено { $path }
reset-fail = Не вдалося видалити { $path }: { $error }

# --- Logs ---
log-following = --- Стеження за { $path } (Ctrl+C для зупинки) ---
log-path-hint = Файл логу: { $path }
