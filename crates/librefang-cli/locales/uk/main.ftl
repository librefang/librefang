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
label-yes = так
label-no = ні
label-not-loaded = не завантажено
label-current = Поточна
label-channel = Канал
label-binary = Бінарний файл
label-latest = Остання
label-target = Цільова
label-installed = Встановлено

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
label-running = запущено
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
channel-none-configured = Не налаштовано жодного каналу.
channel-use-setup-hint = Використовуйте `librefang channel setup`, щоб додати канал.
channel-reloaded = Канали перезавантажено (запущено { $started } sidecar-процесів).
channel-registry-empty = Реєстр каналів демона порожній.
channel-install-sdk-hint = Встановіть sidecar SDK, щоб адаптери з'явилися в каталозі:
channel-install-sdk-cmd =   pip install librefang-sdk
channel-rerun-setup-hint = Потім повторно виконайте `librefang channel setup`.
channel-all-configured = Усі доступні канали вже налаштовані.
channel-see-list-hint = Використовуйте `librefang channel list`, щоб переглянути їх, або
channel-remove-entry-hint = `librefang channel rm <name>`, щоб спочатку видалити запис.
channel-pick-setup = Оберіть канал для налаштування:
channel-choice-prompt = Вибір [1]: 
channel-unknown-error = Невідомий канал: { $name }
channel-unknown-error-fix = Виконайте `librefang channel list`, щоб переглянути доступні адаптери.
channel-no-configurable-fields = `{ $name }` не має полів для налаштування — немає чого запитувати.
channel-hot-reload-manual-hint = (Ви все одно можете виконати гаряче перезавантаження за допомогою `librefang channel reload`, якщо ви вже відредагували config.toml вручну.)
channel-prompt-secret-keep =   { $label } ({ $key }) [встановлено — залиште порожнім, щоб зберегти]: 
channel-prompt-default =   { $label } ({ $key }) [{ $current }]: 
channel-prompt-required =   { $label } ({ $key }) *: 
channel-prompt-optional =   { $label } ({ $key }): 
channel-save-rejected = Збереження для `{ $name }` відхилено: { $error }
channel-save-rejected-fix = Запустіть повторно з виправленими значеннями або перевірте лог демона для отримання деталей.
channel-saved-restart-required = ✓ Збережено `{ $name }` — перезапустіть демона, щоб зміни набрали чинності.
channel-saved-hot-reload = ✓ Збережено `{ $name }` — гаряче перезавантаження застосовано.
channel-env-shadowing-warn = Попередження: змінні оточення оболонки затіняють ці токени — видаліть їх та перезапустіть, щоб нове значення набрало чинності: { $keys }
channel-config-read-fail = Не вдалося прочитати { $path }: { $error }
channel-config-read-fail-fix = Виконайте `librefang init`, щоб створити файл конфігурації.
channel-config-parse-fail = Не вдалося розібрати { $path }: { $error }
channel-config-parse-fail-fix = Виправте синтаксис TOML та спробуйте ще раз.
channel-no-entries-to-remove = У config.toml немає записів [[sidecar_channels]] — немає чого видаляти.
channel-no-entry-with-name = Немає запису [[sidecar_channels]] з name="{ $name }".
channel-config-write-fail = Не вдалося записати { $path }: { $error }
channel-config-write-fail-fix = Перевірте права доступу до файлової системи.
channel-removed-entries = ✓ Видалено { $count } запис(ів) [[sidecar_channels]] з назвою `{ $name }`.
channel-hot-reloaded-daemon =   Гаряче перезавантаження демона виконано.
channel-reload-status-warn =   Перезавантаження повернуло { $status }: зміни будуть застосовані при наступному перезапуску демона.
channel-reload-contact-fail-warn =   Не вдалося зв'язатися з демоном для перезавантаження ({ $error }); зміни будуть застосовані при наступному запуску.
channel-reload-daemon-offline =   Демон не запущений; зміни будуть застосовані при наступному запуску.
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
cron-toggled = Cron-завдання { $id } { $action ->
    [enable] увімкнен
    *[disable] вимкнен
}о.
cron-toggle-failed = Не вдалося { $action ->
    [enable] увімкнути
    *[disable] вимкнути
} Cron-завдання: { $error }

# --- Automation ---
automation-workflow-none = Немає зареєстрованих воркфлоу.
automation-workflow-file-not-found = Файл воркфлоу не знайдено: { $path }
automation-workflow-read-error = Помилка при зчитуванні файлу воркфлоу: { $error }
automation-workflow-invalid-json = Некоректний JSON: { $error }
automation-workflow-created = Воркфлоу успішно створено!
automation-workflow-created-id =   ID: { $id }
automation-workflow-create-failed = Не вдалося створити воркфлоу: { $error }
automation-workflow-completed = Воркфлоу завершено!
automation-workflow-run-id =   ID запуску: { $id }
automation-workflow-failed = Помилка воркфлоу: { $error }
automation-trigger-none = Немає зареєстрованих тригерів.
automation-trigger-invalid-pattern = Некоректний JSON шаблону: { $error }
automation-trigger-created = Тригер успішно створено!
automation-trigger-created-id =   ID тригера: { $id }
automation-trigger-created-agent =   ID агента:   { $agent_id }
automation-trigger-created-target =   Ціль:     { $target }
automation-trigger-create-failed = Не вдалося створити тригер: { $error }
automation-trigger-deleted = Тригер { $id } видалено.
automation-trigger-delete-failed = Не вдалося видалити тригер: { $error }
automation-trigger-get-failed = Не вдалося отримати тригер: { $error }
automation-trigger-update-failed = Не вдалося оновити тригер: { $error }
automation-trigger-updated = Тригер { $id } оновлено.
automation-trigger-toggle-failed = Не вдалося { $action ->
    [enable] увімкнути
    *[disable] вимкнути
} тригер: { $error }
automation-trigger-toggled = Тригер { $id } { $action ->
    [enable] увімкнен
    *[disable] вимкнен
}о.
automation-trigger-info-id = ID тригера:    { $id }
automation-trigger-info-agent = ID агента:      { $id }
automation-trigger-info-pattern = Шаблон:       { $pattern }
automation-trigger-info-prompt = Промпт:        { $prompt }
automation-trigger-info-enabled = Увімкнено:       { $enabled }
automation-trigger-info-fires = Кількість запусків:    { $count }
automation-trigger-info-max-fires = Макс. запусків:     { $count }
automation-trigger-info-target = Агент цілі:  { $agent }
automation-trigger-info-cooldown = Кулдаун:      { $secs }с
automation-trigger-info-session = Режим сесії:  { $mode }
automation-unlimited = без обмежень
automation-cron-none = Немає запланованих завдань.

label-header-steps = КРОКИ
label-header-trigger-id = ID ТРИГЕРА
label-header-agent-id = ID АГЕНТА
label-header-fires = ЗАПУСКИ
label-header-pattern = ШАБЛОН
label-header-schedule = РОЗКЛАД
label-header-prompt = ПРОМПТ

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
model-none-found = Моделей не знайдено.
model-prompt-selection =   Введіть номер або ID моделі: 

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
config-no-file-found = Не знайдено конфігурації за шляхом: { $path }
config-run-init-hint = Виконайте `librefang init`, щоб створити її.
config-read-error = Помилка читання конфігурації: { $error }
config-editor-exit = Редактор завершив роботу з кодом: { $status }
config-editor-open-fail = Не вдалося відкрити редактор '{ $editor }': { $error }
config-editor-env-hint = Встановіть змінну оточення $EDITOR на бажаний редактор.
config-val-exceeds-i64 = значення { $value } перевищує i64::MAX ({ $max }); TOML не може зберігати беззнакові цілі числа вище цієї межі
config-invalid-integer = '{ $raw }' не є коректним цілим числом
config-paste-api-key-prompt =   Вставте ваш API-ключ { $provider }: 
config-testing-key =   Тестування ключа... 
config-testing-provider-key =   Тестування { $provider } ({ $env_var })... 
config-test-ok = ОК
config-test-failed = ПОМИЛКА (401/403)
config-test-unverified = не вдалося підтвердити (може все одно працювати)


# --- Hand commands ---
hand-install-deps-success = Залежності для Hands '{ $id }' встановлено.
hand-paused = Екземпляр Hands '{ $label } (екземпляр: { $instance_id })' призупинено.
hand-resumed = Екземпляр Hands '{ $label } (екземпляр: { $instance_id })' відновлено.

# --- Daemon notify ---
daemon-restart-notify = Перезапустіть демона, щоб застосувати: librefang restart

# --- System info ---
section-system-info = Системна інформація LibreFang

# --- Uninstall ---
uninstall-warning = Це повністю видалить LibreFang з вашої системи.
uninstall-remove-data-kept =   • Видалити дані в { $path } (зберегти файли конфігурації)
uninstall-remove-all =   • Видалити { $path }
uninstall-remove-binary =   • Видалити бінарний файл: { $path }
uninstall-remove-cargo-binary =   • Видалити бінарний файл cargo: { $path }
uninstall-remove-autostart =   • Видалити записи автозапуску (якщо є)
uninstall-clean-path =   • Очистити PATH у конфігураціях оболонки (якщо є)
uninstall-confirm-prompt =   Введіть 'uninstall' для підтвердження: 
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

# --- Extracted from Rust sources ---
init-error-create-data-dir = Помилка створення директорії даних: { $error }
init-upgrade-existing = Виявлено наявне встановлення — виконується оновлення для збереження ваших налаштувань.
init-upgrade-fresh-hint = Щоб почати спочатку, видаліть ~/.librefang/config.toml та запустіть `librefang init` знову.
init-upgrade-no-config = Немає чого оновлювати — config.toml не знайдено. Спочатку запустіть `librefang init`.
init-upgrade-registry-synced = Реєстр синхронізовано
init-upgrade-registry-failed = Помилка синхронізації реєстру (проблема з мережею?) — продовжуємо з кешованим вмістом
init-upgrade-config-up-to-date = Конфігурація вже актуальна — нових полів не додано
init-upgrade-sections-added = Додано { $count } нових розділів конфігурації:
init-upgrade-legacy-openclaw = Виявлено застаріле встановлення ~/.openclaw.
init-upgrade-legacy-openclaw-hint = Виконайте `librefang migrate --from openclaw`, щоб перенести ваші дані.
init-upgrade-approval-warning = Ваш список require_approval містить лише "shell_exec". Файлові операції (file_write, file_delete) тепер вимагають схвалення за замовчуванням.
init-upgrade-approval-hint = Щоб увімкнути: додайте "file_write" та "file_delete" до require_approval у config.toml
init-upgrade-success-summary = Оновлення завершено!
init-upgrade-title = Оновлення встановлення LibreFang
init-upgrade-progress-label = Оновлення
init-upgrade-backing-up = Резервне копіювання конфігурації
init-upgrade-backup-success = Резервну копію конфігурації збережено у backups/{ $name }
init-upgrade-syncing-registry = Синхронізація реєстру
init-upgrade-initializing-vault-git = Ініціалізація сховища/git
init-upgrade-merging-config = Об'єднання полів конфігурації
init-upgrade-failed-read = Оновлення перервано: не вдалося прочитати config.toml: { $error }
init-upgrade-failed-parse = Оновлення перервано: не вдалося розібрати config.toml: { $error }
init-upgrade-backup-saved-hint = Вашу оригінальну конфігурацію було збережено у backups/{ $name }
init-upgrade-failed-parse-template = Оновлення перервано: не вдалося розібрати шаблон конфігурації за замовчуванням: { $error }
init-upgrade-failed-write = Оновлення перервано: не вдалося записати конфігурацію: { $error }
init-upgrade-steps-complete = Кроки оновлення завершено
label-backup = Бекап
label-new-fields = Нові поля

auth-chatgpt-device-requested = Запитано автентифікацію пристрою.
auth-chatgpt-device-open-url = Відкрийте цю URL-адресу в будь-кому браузері:\n  { $url }\n
auth-chatgpt-device-one-time-code = Введіть цей одноразовий код:\n  { $code }\n
auth-chatgpt-device-do-not-share = Не діліться цим кодом.
auth-chatgpt-device-waiting = Очікування авторизації...
auth-chatgpt-switching-browser = \nПеремикання на стандартний процес входу через браузер...\n
auth-chatgpt-opening-browser = Відкриття браузера для автентифікації OpenAI...
auth-chatgpt-open-manually-hint = Якщо браузер не відкрився, відвідайте:\n  { $url }\n
auth-chatgpt-open-browser-failed = Не вдалося автоматично відкрити браузер: { $error }
auth-chatgpt-open-manually = Будь ласка, відкрийте вручну: { $url }
auth-chatgpt-tokens-saved = \nТокени ChatGPT збережено в { $path }
auth-chatgpt-detecting-model = Визначення найкращої доступної моделі...
auth-chatgpt-selected-model = Вибрана модель: { $model }
auth-chatgpt-config-updated = config.toml оновлено: provider = "chatgpt", model = "{ $model }"
auth-chatgpt-starting-flow = Запуск процесу автентифікації ChatGPT...\n
auth-chatgpt-complete = Автентифікацію ChatGPT завершено.
auth-chatgpt-failed = Помилка автентифікації ChatGPT: { $error }

auth-pool-config-not-array = config.toml `credential_pools` існує, але не є масивом таблиць
auth-pool-daemon-error-fallback = Демон повернув HTTP { $status } — перехід до перегляду config.toml
auth-pool-daemon-connect-fallback = Не вдалося зробити запит до демона на { $url }: { $error } — перехід до перегляду config.toml
auth-pool-no-config-offline = Немає конфігурації в { $path } і демон не запущений.
auth-pool-config-load-failed = Не вдалося завантажити конфігурацію: { $error }
auth-pool-none-configured = Не налаштовано жодного пулу облікових даних.
auth-pool-invalid-env-name = `{ $env_var }` не є коректним ім'ям змінної оточення. Очікуються великі літери, цифри та підкреслення (наприклад, OPENAI_API_KEY_2).
auth-pool-env-empty = змінна оточення `{ $env_var }` встановлена, але порожня.
auth-pool-env-empty-fix = Встановіть її у ваш API-ключ перед додаванням запису до пулу, наприклад:\n  export { $env_var }=sk-…\nПотім повторіть спробу.
auth-pool-env-not-set = змінна оточення `{ $env_var }` не встановлена в поточному шеллі.
auth-pool-env-not-set-fix = Експортуйте її перед додаванням запису до пулу, наприклад:\n  export { $env_var }=sk-…\nПотім повторіть спробу. (Демон зчитуватиме її зі свого власного оточення під час запуску — переконайтеся, що вона експортована і там.)
auth-pool-keys-not-array = Пул для `{ $provider }` має поле `keys`, яке не є масивом таблиць.
auth-pool-key-duplicate = Ключ зі змінною оточення `{ $env_var }` уже існує в пулі для провайдера `{ $provider }`.
auth-pool-key-added = Додано ключ `{ $label }` (env={ $env_var }, priority={ $priority }) до пулу для `{ $provider }`. Перезапустіть демона або оновіть конфігурацію для застосування.
auth-pool-not-configured = Не налаштовано жодного пулу облікових даних для провайдера `{ $provider }`.
auth-pool-no-keys-field = Пул для `{ $provider }` не має масиву ключів.
auth-pool-key-not-found = Не знайдено ключа зі змінною оточення `{ $env_var }` у пулі для `{ $provider }`.
auth-pool-key-removed-pool-empty = Вилучено ключ `{ $env_var }` з пулу для `{ $provider }`. Тепер пул порожній і його було повністю видалено. Перезапустіть демона або оновіть конфігурацію для застосування.
auth-pool-key-removed = Вилучено ключ `{ $env_var }` з пулу для `{ $provider }`. Перезапустіть демона або оновіть конфігурацію для застосування.
auth-pool-unknown-strategy = Невідома стратегія `{ $strategy }`. Допустимі: fill_first, round_robin, random, least_used.
auth-pool-strategy-set = Встановлено стратегію пулу для `{ $provider }` на `{ $strategy }`. Перезапустіть демона або оновіть конфігурацію для застосування.
vault-empty = Сховище порожнє.
vault-stored-count = Збережені облікові дані ({ $count }):

# --- Scanned & Extracted keys ---
# init.rs
init-upgrade-failed-create-backups-dir = Не вдалося створити директорію бекапів: { $error }
init-upgrade-failed-backup-config = Не вдалося створити бекап конфігурації: { $error }
init-error-write-config-example = Не вдалося записати config.example.toml: { $error }

# acp.rs
acp-attached-uds = librefang acp: приєднано до демона (UDS { $path })
acp-attached-pipe = librefang acp: приєднано до демона (іменований канал)
acp-in-process = librefang acp: запуск вбудованого ядра (демона не виявлено)
acp-error-boot-kernel = Не вдалося запустити ядро: { $error }
acp-error-resolve-agent = Не вдалося розпізнати агента '{ $name }': { $error }
acp-error-server = Помилка сервера ACP: { $error }
acp-error-uds-connect = Помилка підключення ACP UDS за шляхом { $path }: { $error }
acp-error-pipe-connect = Помилка підключення до іменованого каналу ACP { $name }: { $error }


# auth.rs
auth-write-failed = Не вдалося записати { $path }: { $error }
auth-password-empty = Пароль не може бути порожнім.
auth-passwords-mismatch = Паролі не збігаються.
auth-password-hash-failed = Не вдалося захешувати пароль: { $error }
vault-enter-value-prompt = Введіть значення для { $key }: 
auth-enter-password-prompt = Введіть пароль: 
auth-confirm-password-prompt = Підтвердіть пароль: 

# agent.rs
agent-spawn-choose-target-or-template = Виберіть або позиційну ціль, або `--template`, але не обидва.
agent-spawn-choose-target-or-template-fix = Використовуйте `librefang spawn coder` або `librefang spawn --template agents/custom/my-agent.toml`.
agent-spawn-name-requires-template = `--name` вимагає імені темплейту або шляху до маніфесту.
agent-spawn-name-requires-template-fix = Використовуйте `librefang spawn coder --name backend-coder` або `librefang spawn --template path/to/agent.toml --name backend-coder`.
agent-spawn-dry-run-requires-template = Холостий запуск потребує імені темплейту або шляху до маніфесту.
agent-spawn-dry-run-requires-template-fix = Використовуйте `librefang spawn coder --dry-run` or `librefang spawn --template path/to/agent.toml --dry-run`.
agent-spawn-template-or-path-not-found = Темплейт або шлях до маніфесту не знайдено: { $target }
agent-spawn-template-or-path-not-found-fix = Запустіть `librefang agent new`, щоб переглянути темплейти, або вкажіть правильний шлях до маніфесту.
agent-manifest-parse-failed = Не вдалося розібрати маніфест агента з { $source }: { $error }
agent-manifest-parse-failed-fix = Перевірте синтаксис TOML маніфесту та обов'язкові поля.
agent-manifest-serialize-failed = Не вдалося серіалізувати оновлений маніфест: { $error }
agent-dry-run-title = Холостий запуск агента
agent-dry-run-success = Маніфест успішно розібрано. Жодного агента не було створено.
agent-delete-warning-text = УВАГА: Видалення агента "{ $name }" назавжди вилучить його канонічний UUID
    та всі пов'язані спогади й сесії.
    Цю дію неможливо скасувати.
label-confirm-prompt = Підтвердити?
label-aborted = Скасовано.
agent-delete-no-uuid = Не знайдено запису канонічного UUID для імені агента '{ $name }' — немає чого видаляти.
agent-deleted-success = Агента "{ $name }" видалено (канонічний UUID очищено).
agent-delete-failed-with-reason = Не вдалося видалити агента: { $error }
agent-reset-uuid-warning-text = УВАГА: Скидання канонічного UUID для "{ $name }" призведе до втрати зв'язку з усіма сесіями
    та спогадами, пов'язаними з поточним UUID. Наступний запуск під цим
    іменем розпочнеться з новим UUID. Цю дію неможливо скасувати.
agent-reset-uuid-success = Канонічний UUID для "{ $name }" скинуто (був { $previous }).
agent-reset-uuid-failed-with-reason = Не вдалося скинути канонічний UUID: { $error }
agent-reset-uuid-not-found = Не знайдено запису канонічного UUID для імені агента '{ $name }'.
agent-merge-history-not-implemented = merge-history ще не реалізовано (слідкуючий тикет #4614).
    Перепризначення сесій / спогадів з { $from } до канонічного UUID
    для агента "{ $name }" вимагає крос-табличного SQL-втручання в субстрат
    пам'яті, що відстежується окремо.
agent-set-model-success = Для агента { $id } встановлено модель { $value }.
agent-set-model-failed-with-reason = Не вдалося встановити модель: { $error }
agent-set-no-daemon = Запущеного демона не знайдено. Запустіть його за допомогою: librefang start
agent-set-unknown-field = Невідоме поле: { $field }. Підтримувані поля: model
agent-new-no-templates = Не знайдено темплейтів агентів
agent-new-no-templates-fix = Запустіть `librefang init`, щоб налаштувати директорію агентів
agent-new-template-not-found = Темплейт '{ $name }' не знайдено
agent-new-template-not-found-fix = Запустіть `librefang agent new`, щоб переглянути доступні темплейти
agent-new-choose-template-prompt =   Оберіть темплейт [1]: 
agent-sessions-none-active = Немає активних сесій.
agent-sessions-none-found = Сесій не знайдено.

label-source = Джерело
label-name = Ім'я
label-captured = Зафіксовано
label-module = Модуль
label-tools = Інструменти
label-tags = Теги
label-description = Опис

# daemon.rs
daemon-first-run-setup = Виявлено перший запуск — виконується швидке налаштування...
daemon-config-not-found = Файл конфігурації не знайдено: { $path }
daemon-config-not-found-fix = Запустіть `librefang init`, щоб створити конфігурацію за замовчуванням у ~/.librefang/config.toml, або перевірте шлях у --config.
daemon-log-file-not-found = Файл логів не знайдено
daemon-log-file-not-found-fix = Очікувався за шляхом: { $path }
daemon-log-not-found-showing-tui = Лог демона не знайдено; показуємо лог TUI за шляхом { $path }

# hand.rs
hand-install-error-no-toml = Помилка: HAND.toml не знайдено в { $path }
hand-install-error-read-toml = Помилка читання { $path }: { $error }
hand-error-prefix = Помилка: { $error }
hand-installed-success = Встановлено Hands: { $name } ({ $id })
hand-activate-hint = Використовуйте `librefang hand activate { $id }`, щоб запустити його.
hand-none-available = Немає доступних Hands.
hand-list-activate-hint =
    Використовуйте `librefang hand activate <id>`, щоб активувати Hands.
hand-none-active = Немає активних Hands.
label-hand = Hands
label-instance = Інстанс
label-agent = Агент
hand-status-title = Статус Hands
label-status-inactive = неактивний
hand-not-found = Не знайдено активного або встановленого Hands для '{ $id }'.
hand-activated-success = Hands '{ $id }' активовано (інстанс: { $instance }, агент: { $agent })
hand-activate-failed = Не вдалося активувати Hands '{ $id }': { $error }
hand-deactivated-success = Hands '{ $id }' деактивовано.
label-failed-reason = Помилка: { $error }
hand-no-active-instance = Не знайдено активного інстансу для Hands '{ $id }'.
hand-info-not-found = Hands не знайдено: { $error }
hand-no-settings = Hands '{ $id }' не має конфігурованих налаштувань.
hand-settings-title = Налаштування для '{ $id }'
hand-set-setting-success = Встановлено { $key }={ $value } для Hands '{ $id }'.
hand-reloaded-summary = Перезавантажено Hands: { $added } додано, { $updated } оновлено, всього { $total }.
hand-chat-welcome = Чат із { $name } (введіть /quit для виходу)

# mcp_cmds.rs
mcp-catalog-unknown-entry = Невідомий елемент каталогу MCP: '{ $name }'
mcp-catalog-available-header =
    Доступні сервери MCP (каталог):
mcp-failed-read-config = Не вдалося прочитати { $path }: { $error }
mcp-invalid-toml = { $path } не є коректним файлом TOML: { $error }
mcp-already-configured = Сервер MCP '{ $name }' уже налаштований. Спочатку запустіть `librefang mcp remove { $name }`, якщо хочете перевстановити.
mcp-failed-write-config = Не вдалося записати config.toml: { $error }
mcp-add-credentials-hint =
    Щоб додати облікові дані:
mcp-get-it-here =   Отримайте тут: { $url }
mcp-not-configured = Сервер MCP '{ $name }' не налаштований
mcp-failed-update-config = Не вдалося оновити config.toml: { $error }
mcp-removed-success = { $name } видалено.
mcp-catalog-no-matches = Не знайдено елементів каталогу MCP, що відповідають '{ $query }'.
mcp-catalog-none-available = Немає доступних елементів каталогу MCP.
mcp-catalog-summary =   { $total } елементів каталогу ({ $installed } встановлено)
mcp-catalog-install-hint =   Використовуйте `librefang mcp add <id>`, щоб встановити сервер MCP.
mcp-none-configured = Немає налаштованих серверів MCP.
mcp-list-catalog-hint =   Використовуйте `librefang mcp catalog`, щоб переглянути список доступних для встановлення серверів.
mcp-vault-set-hint =   librefang vault set { $name }  # { $help }
mcp-header-name = назва
mcp-header-template-id = id_шаблону
mcp-header-transport = транспорт
mcp-header-details = деталі

# monitoring.rs
monitoring-audit-reset-destructive = скидання аудиту є руйнівним — запустіть знову з `--confirm` для продовження
monitoring-db-not-found = базу даних не знайдено за шляхом { $path }
monitoring-db-open-failed = не вдалося відкрити { $path }: { $error }
monitoring-db-truncate-failed = не вдалося очистити audit_entries: { $error }
monitoring-audit-reset-anchor-deleted = , видалено якір за шляхом { $path }
monitoring-audit-reset-anchor-none =  (немає якірного файлу для видалення)
monitoring-audit-reset-success = Скинуто слід аудиту: вилучено { $count } рядків з audit_entries{ $anchor_detail }.
monitoring-audit-reset-would-header =   Буде виконано:
monitoring-audit-reset-would-delete =     1. ВИДАЛЕНО всі рядки з `audit_entries` у { $path }
monitoring-audit-reset-would-remove-anchor =     2. Вилучено якірний файл { $path }
monitoring-audit-reset-would-restart =   Ланцюг Меркла розпочнеться заново з наступної події аудиту.
monitoring-daemon-running-error = демон запущений за адресою { $url }; відмовлено у зміні бази даних аудиту
monitoring-daemon-running-error-fix = спочатку зупиніть демона: `librefang stop`
monitoring-anchor-remove-failed = не вдалося вилучити якір { $path }: { $error }
monitoring-audit-reset-seed-fresh = Наступний запуск демона створить свіжий ланцюг Меркла з поточного кінця.
monitoring-memory-no-entries = Не знайдено записів пам'яті для агента '{ $agent }'.
monitoring-devices-none-paired = Немає спарених пристроїв.
monitoring-webhooks-none-configured = Вебхуки не налаштовані.

# skill.rs
skill-install-progress = Встановлення { $source }

# system.rs
migrate-error-home-dir = Помилка: не вдалося визначити домашню директорію
migrate-start-msg = Міграція з { $source } ({ $path })...
migrate-dry-run-hint =   (холостий запуск — жодних змін не буде внесено)
migrate-progress-label = Виконання міграції
migrate-complete-msg = Міграцію завершено
migrate-warn-report-save-failed = Попередження: не вдалося зберегти звіт міграції: { $error }
migrate-report-saved =
      Звіт збережено за шляхом: { $path }
migrate-failed-msg = Міграція завершилась невдачею: { $error }

# maintenance.rs
maintenance-service-install-root-error = Запущено від імені root — службу буде встановлено для облікового запису root, а не для вашого користувача. Запустіть без sudo.
maintenance-service-unsupported = Автозапуск служби не підтримується на цій платформі.
maintenance-failed-create-dir = Не вдалося створити { $path }: { $error }
maintenance-failed-write-file = Не вдалося записати { $path }: { $error }
maintenance-wrote-file = Записано { $path }
maintenance-systemctl-reload-failed = помилка виконання systemctl --user daemon-reload
maintenance-service-enabled = Службу увімкнено (запуститься при наступному вході в систему)
maintenance-service-start-hint = Запустіть зараз за допомогою: systemctl --user start librefang.service
maintenance-service-linger-hint = Для серверів без графічної оболонки також виконайте: loginctl enable-linger
maintenance-systemctl-enable-failed = помилка виконання systemctl --user enable librefang.service
maintenance-launchagent-loaded = LaunchAgent завантажено (запускатиметься при вході в систему та зараз)
maintenance-launchctl-load-failed = помилка виконання launchctl load: { $error }
maintenance-launchctl-run-failed = Не вдалося запустити launchctl: { $error }
maintenance-windows-startup-added = Додано до автозавантаження Windows (HKCU\Software\Microsoft\Windows\CurrentVersion\Run)
maintenance-windows-registry-write-failed = Не вдалося записати до реєстру Windows: { $error }
maintenance-windows-reg-run-failed = Не вдалося запустити reg.exe: { $error }
maintenance-systemd-removed = Вилучено користувацьку службу systemd
maintenance-systemd-remove-failed = Не вдалося вилучити файл служби: { $error }
maintenance-systemd-not-found = Користувацької служби systemd не знайдено — немає чого вилучати.
maintenance-launchagent-removed = Вилучено LaunchAgent
maintenance-launchagent-remove-failed = Не вдалося вилучити файл plist: { $error }
maintenance-launchagent-not-found = LaunchAgent не знайдено — немає чого вилучати.
maintenance-windows-startup-removed = Вилучено з автозавантаження Windows
maintenance-windows-startup-not-found = Запису автозавантаження не знайдено — немає чого вилучати.
maintenance-systemd-status-registered = Користувацьку службу systemd зареєстровано
maintenance-status-label-enabled =   Увімкнено
maintenance-status-label-active =   Активно
maintenance-systemd-status-not-registered = Користувацьку службу systemd не зареєстровано.
maintenance-service-install-hint = Запустіть `librefang service install`, щоб налаштувати її.
maintenance-launchagent-status-registered = LaunchAgent зареєстровано
maintenance-status-label-loaded =   Завантажено
maintenance-launchagent-status-not-registered = LaunchAgent не зареєстровано.
maintenance-windows-status-registered = Запис автозавантаження Windows зареєстровано
maintenance-windows-status-not-registered = Запис автозавантаження не зареєстровано.
reset-confirm-message =   Це видалить всі дані в { $path }
      Включаючи: конфігурацію, базу даних, маніфести агентів, облікові дані.
reset-confirm-prompt =   Ви впевнені? Введіть 'yes' для підтвердження: 
reset-not-needed = Немає чого скидати — { $path } не існує.
maintenance-update-section = Оновлення
maintenance-update-error-exe-path = Не вдалося визначити шлях до поточного виконуваного файлу: { $error }
maintenance-update-error-check-release = Не вдалося перевірити останній реліз: { $error }
maintenance-update-warn-resolve-release = Не вдалося визначити останній опублікований реліз: { $error }
maintenance-update-warn-resolve-release-fix = Спробуйте пізніше або передайте `--version <tag>`, щоб вказати конкретний реліз.
maintenance-update-available = Доступний новіший опублікований реліз: { $tag }
maintenance-update-run-hint = Запустіть `librefang update`, щоб встановити його.
maintenance-update-same-core = Опублікований реліз { $tag } використовує ту саму версію ядра CLI, що й поточний виконуваний файл ({ $current }).
maintenance-update-same-core-hint = Запустіть `librefang update`, якщо хочете отримати останню опубліковану збірку для цієї версії.
maintenance-update-ahead = Поточна версія виконуваного файлу { $current } випереджає опублікований реліз { $tag }.
maintenance-update-compare-unknown = Не вдалося порівняти поточну версію виконуваного файлу з тегом релізу { $tag }.
maintenance-update-compare-unknown-hint = Якщо вам потрібен саме цей реліз, запустіть `librefang update --version <tag>`.
maintenance-update-unable-to-determine = Не вдалося визначити наявність оновлення.
maintenance-update-unable-to-determine-hint = Спробуйте пізніше, коли сервіс GitHub Releases буде доступним.
maintenance-update-cannot-compare-safely = Не вдалося безпечно порівняти поточний виконуваний файл з тегом релізу { $tag }.
maintenance-update-cannot-compare-safely-hint = Запустіть знову як `librefang update --version { $tag }`, щоб встановити його явно.
maintenance-update-windows-daemon-running-error = Перед оновленням у Windows зупиніть працюючого демона.
maintenance-update-windows-daemon-running-error-fix = Виконайте `librefang stop`, потім `librefang update`, а тоді `librefang start`.
maintenance-update-cli-success = Локальний LibreFang CLI оновлено.
maintenance-update-merging-config-defaults = Об'єднання нових налаштувань конфігурації за замовчуванням...
maintenance-update-restart-daemon-hint = Якщо демон запущений, перезапустіть його за допомогою `librefang restart`.
maintenance-update-background-launched = Оновлення запущено у фоновому режимі.
maintenance-update-background-hint-terminal = Після завершення оновлення відкрийте новий термінал та запустіть `librefang --version`.
maintenance-update-background-hint-restart = Якщо демон запущений, перезапустіть його після завершення оновлення.
maintenance-update-failed-error = Помилка оновлення: { $error }
maintenance-update-cargo-blocked = Цей виконуваний файл було встановлено через cargo. Запуск `cargo install` зсередини активного виконуваного файлу навмисно заблокований.
maintenance-update-unofficial-path = Автоматичне оновлення підтримує лише офіційний шлях встановлення ({ $path }). Цей виконуваний файл запущено з іншого місця.
maintenance-update-package-manager-hint = Якщо цей файл було встановлено через інший менеджер пакетів, оновіть його за допомогою цього менеджера.

# doctor_cmd.rs
doctor-check-librefang-dir-ok = Директорія LibreFang: { $path }
doctor-check-librefang-dir-fail = Директорію LibreFang не знайдено.
doctor-check-librefang-dir-created = Створено директорію LibreFang
doctor-check-librefang-dir-create-fail = Не вдалося створити директорію
doctor-check-librefang-dir-not-found-init = Директорію LibreFang не знайдено. Спочатку запустіть `librefang init`.
doctor-check-env-ok = Файл .env (права доступу в нормі)
doctor-check-env-fixed = Файл .env (права доступу виправлено на 0600)
doctor-check-env-ok-generic = Файл .env
doctor-check-env-loose-warn = Файл .env має занадто відкриті права доступу ({ $mode }), має бути 0600
doctor-check-env-not-found-warn = Файл .env не знайдено (створіть за допомогою: librefang config set-key <provider>)
doctor-check-config-ok = Файл конфігурації: { $path }
doctor-check-config-syntax-fail = Файл конфігурації містить синтаксичні помилки: { $error }
doctor-check-config-not-found = Файл конфігурації не знайдено.
doctor-check-config-created = Створено config.toml за замовчуванням
doctor-check-config-create-fail = Не вдалося створити config.toml
doctor-check-cli-version = Версія CLI: { $version } (канал: { $channel })
doctor-check-update-available-warn = Доступне оновлення: { $current } -> { $latest } (див. https://github.com/librefang/librefang/releases)
doctor-check-cli-up-to-date = CLI оновлений до останньої версії
doctor-check-update-fail-warn = Не вдалося перевірити наявність оновлень (мережа недоступна)
doctor-check-daemon-running = Демон запущений за адресою { $url }
doctor-check-daemon-not-running-warn = Демон не запущений (запустіть за допомогою `librefang start`)
doctor-check-port-available = Порт { $address } вільний
doctor-check-port-in-use-warn = Порт { $address } використовується іншим процесом
doctor-check-stale-daemon-json-removed = Вилучено застарілий daemon.json
doctor-check-stale-daemon-json-warn = Знайдено застарілий daemon.json (демон не запущений). Запустіть з --repair для очищення.
doctor-check-db-ok = Файл бази даних (коректний SQLite)
doctor-check-db-invalid-fail = Файл бази даних існує, але не є коректним SQLite
doctor-check-db-not-found-warn = Файлу бази даних немає (буде створено при першому запуску)
doctor-check-disk-space-low-warn = Мало вільного місця на диску: доступно { $count }МБ
doctor-check-disk-space-ok = Вільне місце на диску: доступно { $count }МБ
doctor-check-manifests-ok = Маніфести агентів коректні
doctor-check-manifest-invalid-fail = Некоректний маніфест { $file }: { $error }
doctor-check-home-dir-fail = Не вдалося визначити домашню директорію
doctor-check-provider-key-rejected-warn = { $name } ({ $env_var }) - ключ відхилено (401/403)
doctor-check-endpoint-reachable = Ендпоінт { $name } доступний ({ $endpoint })
doctor-check-endpoint-unreachable-warn = Ендпоінт { $name } недоступний ({ $endpoint })
doctor-check-channel-token-format-warn = { $name } ({ $env_var }) - неочікуваний формат токена
doctor-check-config-env-missing-warn = Конфігурація посилається на { $env_var }, але його не встановлено в оточенні або у .env
doctor-check-config-deser-ok = Конфігурація успішно десеріалізується в KernelConfig
doctor-check-exec-policy = Політика виконання: mode={ $mode }, safe_bins={ $count }
doctor-check-include-file-ok = Підключений файл: { $path }
doctor-check-include-file-missing-warn = Підключений файл відсутній: { $path }
doctor-check-include-file-not-found-fail = Підключений файл не знайдено: { $path }
doctor-check-mcp-servers-count = Налаштовано серверів MCP: { $count }
doctor-check-mcp-empty-command-warn = Сервер MCP '{ $name }' має порожню команду
doctor-check-mcp-empty-url-warn = Сервер MCP '{ $name }' має порожню URL-адресу
doctor-check-mcp-empty-base-url-warn = Сервер MCP '{ $name }' має порожній base_url
doctor-check-mcp-no-compat-tools-warn = Сервер MCP '{ $name }' не має налаштованих інструментів http_compat
doctor-check-mcp-compat-header-empty-name-warn = Сервер MCP '{ $name }' має заголовок http_compat з порожнім ім'ям
doctor-check-mcp-compat-header-no-value-warn = Сервер MCP '{ $name }' має заголовок http_compat без value/value_env
doctor-check-mcp-compat-tool-empty-name-warn = Сервер MCP '{ $name }' має інструмент http_compat з порожнім ім'ям
doctor-check-mcp-compat-tool-empty-path-warn = Сервер MCP '{ $name }' має інструмент http_compat з порожнім шляхом
doctor-check-config-deser-fail = Помилка десеріалізації конфігурації в KernelConfig: { $error }
doctor-check-skills-loaded = Завантажено скілів: { $count }
doctor-check-skills-load-fail-warn = Не вдалося завантажити скіли: { $error }
doctor-check-skills-injection-ok = Усі скіли пройшли перевірку на ін'єкції промптів
doctor-check-mcp-catalog-templates = Шаблони каталогу MCP: { $templates }
doctor-check-mcp-configured-servers = Налаштовано серверів MCP: { $configured }
doctor-check-running-agents = Запущено агентів: { $count }
doctor-check-daemon-uptime = Час роботи демона: { $hours }год { $mins }хв
doctor-check-db-connectivity-ok = Підключення до бази даних: OK
doctor-check-db-status-fail = Стан бази даних: { $status }
doctor-check-health-detail-status-warn = Ендпоінт стану повернув { $status }
doctor-check-health-detail-fail-warn = Не вдалося запитати стан демона: { $error }
doctor-check-skills-loaded-daemon = Завантажено скілів у демоні: { $count }
doctor-check-rust-version = Rust: { $version }
doctor-check-rust-not-found-fail = Інструментарій Rust не знайдено
doctor-check-python-version = Python: { $version }
doctor-check-python-not-found-warn = Python не знайдено (необхідний для скілів на Python)
doctor-check-node-version = Node.js: { $version }
doctor-check-node-not-found-warn = Node.js не знайдено (необхідний для скілів на Node.js)
doctor-prompt-create-dir =     Створити зараз? [Y/n] 
doctor-prompt-create-config =     Створити конфігурацію за замовчуванням? [Y/n] 
doctor-section-providers =   LLM Провайдери:
doctor-section-connectivity = 

  Підключення до Мережі:
doctor-section-channels = 

  Інтеграція Каналів:
doctor-section-config-val = 

  Валідація Конфігурації:
doctor-section-skills = 

  Скіли:
doctor-check-skills-injection-critical-warn = Скіл '{ $name }' має { $count } критичних попереджень:
doctor-check-skills-injection-warn = Попередження про ін'єкцію промпту в скілі: { $name }
doctor-section-mcp-servers =
  MCP-сервери:
doctor-section-daemon-health =
  Здоров'я демона:
doctor-check-daemon-mcp-status = MCP-сервери: { $configured } налаштовано, { $connected } підключено
doctor-check-daemon-mcp-health = Здоров'я MCP-серверів: { $healthy }/{ $total } здорових

doctor-suggest-groq = https://console.groq.com       (безкоштовно, швидко)
doctor-suggest-gemini = https://aistudio.google.com    (безкоштовний тариф)
doctor-suggest-deepseek = https://platform.deepseek.com  (дешево)

desktop-install-launched = Десктопний додаток запущено.
desktop-install-launch-fail = Не вдалося запустити { $path }: { $error }
desktop-install-launch-fail-generic = Не вдалося запустити десктопний додаток: { $error }
desktop-install-not-installed = LibreFang Desktop не встановлено.
desktop-install-prompt =   Завантажити та встановити зараз? [Y/n] 
desktop-install-skipped = Пропущено. Ви можете встановити його пізніше:
desktop-install-skipped-brew =   brew install --cask librefang   (macOS)
desktop-install-skipped-manual =   Або завантажте з https://github.com/librefang/librefang/releases
desktop-install-fetching = Отримання інформації про останній реліз...
desktop-install-unsupported = Непідтримувана платформа для автоматичного встановлення десктопної версії.
desktop-install-download-manual = Завантажте вручну: https://github.com/librefang/librefang/releases
desktop-install-github-fail = Не вдалося зв'язатися з GitHub: { $error }
desktop-install-parse-fail = Не вдалося розібрати інформацію про реліз: { $error }
desktop-install-kv-asset = Ресурс
desktop-install-downloading = Завантаження...
desktop-install-download-fail = Завантаження не вдалося: { $error }
desktop-install-download-complete = Завантаження завершено.
desktop-install-installing = Встановлення...
desktop-install-success = LibreFang Desktop успішно встановлено.
desktop-install-fail = Помилка встановлення: { $error }
desktop-install-running-installer = Запуск інсталятора...

doctor-audit-vault-key-unset = LIBREFANG_VAULT_KEY не встановлено — шифрування сховища вимкнено.
doctor-audit-vault-key-invalid-base64 = LIBREFANG_VAULT_KEY не є коректним base64: { $error }
doctor-audit-vault-key-invalid-base64-hint = Згенеруйте його за допомогою: openssl rand -base64 32
doctor-audit-vault-key-wrong-length = LIBREFANG_VAULT_KEY розкодовується в { $count } байтів; має бути рівно 32. Зверніть увагу, що 32 ASCII символи НЕ дорівнюють 32 байтам після декодування base64.
doctor-audit-vault-key-wrong-length-hint = Згенеруйте новий 32-байтний ключ: openssl rand -base64 32 (результат матиме 44 символи)
doctor-audit-vault-key-ok = LIBREFANG_VAULT_KEY успішно розкодовується в 32 байти.

doctor-audit-api-listen-no-config = config.toml не знайдено — пропуск перевірки api_listen.
doctor-audit-api-listen-invalid-toml = config.toml не є коректним TOML: { $error }
doctor-audit-api-listen-invalid-toml-hint = Відредагуйте ~/.librefang/config.toml або запустіть `librefang doctor --repair`.
doctor-audit-api-listen-unset = api_listen не встановлено в конфігурації — ядро використовуватиме значення за замовчуванням.
doctor-audit-api-listen-invalid-addr = api_listen `{ $address }` не є коректною сокет-адресою: { $error }
doctor-audit-api-listen-invalid-addr-hint = Використовуйте форму `хост:порт`, наприклад `127.0.0.1:4545` або `[::1]:4545`.
doctor-audit-api-listen-port-zero = api_listen `{ $address }` використовує порт 0 (тимчасовий, призначений ОС); клієнти не зможуть визначити URL-адресу демона після прив'язки.
doctor-audit-api-listen-port-zero-hint = Оберіть явний порт (за замовчуванням 4545), наприклад `127.0.0.1:4545`.
doctor-audit-api-listen-privileged = Порт api_listen { $port } є привілейованим (<1024); демон не зможе прив'язатися без прав root.
doctor-audit-api-listen-privileged-hint = Використовуйте порт >= 1024 (за замовчуванням 4545), якщо вам не потрібні права root навмисно.
doctor-audit-api-listen-ok = api_listen `{ $address }` успішно розібрано.

doctor-audit-config-not-found = { $path } не існує.
doctor-audit-config-not-found-hint = Запустіть `librefang init`, щоб створити конфігурацію за замовчуванням.
doctor-audit-config-read-fail = Не вдалося прочитати { $path }: { $error }
doctor-audit-config-ok = { $path } успішно розбирається як TOML.
doctor-audit-config-syntax-error = { $path } містить синтаксичні помилки TOML: { $error }
doctor-audit-config-syntax-error-hint = Відредагуйте { $path } або відновіть її з бекапу.

# launcher menu items
launcher-menu-get-started = Налаштувати та запустити
launcher-menu-get-started-hint = Провайдери, API-ключі, моделі, міграція
launcher-menu-settings = Налаштування
launcher-menu-settings-hint = Провайдери, API-ключі, моделі, воркфлоу
launcher-menu-chat = Чат з агентом
launcher-menu-chat-hint = Швидкий чат у терміналі
launcher-menu-dashboard = Відкрити панель приладів
launcher-menu-dashboard-hint = Запустити веб-інтерфейс у браузері
launcher-menu-desktop = Відкрити десктопний додаток
launcher-menu-desktop-hint = Запустити нативний десктопний додаток
launcher-menu-tui = Запустити термінальний UI
launcher-menu-tui-hint = Інтерактивна консольна панель керування
launcher-menu-help = Показати всі команди
launcher-menu-help-hint = Вивести повну довідку --help

# launcher screen strings
launcher-welcome = Ласкаво просимо! Давайте налаштуємо систему.
launcher-checking-daemon = Перевірка стану демона…
launcher-daemon-running = Демон запущений за адресою { $url }
launcher-daemon-agents = { $count ->
    [one]  ({ $count } агент)
    [few]  ({ $count } агенти)
   *[other]  ({ $count } агентів)
}
launcher-daemon-no-running = Демон не запущений
launcher-provider = Провайдер: { $provider }
launcher-no-keys = API-ключі не виявлено
launcher-hint-re-run =   Запустіть 'Налаштування' для вибору провайдера
launcher-hint-get-started =   Оберіть 'Налаштувати та запустити' для конфігурації
launcher-migration-question = Переходите з { $source }? 
launcher-migration-hint = 'Налаштувати та запустити' включає автоматичну міграцію.
launcher-menu-hints = ↑↓/jk навігація  1-9 швидкий вибір  enter підтвердити  q вихід
launcher-help-title = Усі команди
launcher-help-subtitle =   — q/Esc для повернення
launcher-help-hints = ↑↓/jk прокрутка  PgUp/PgDn  g/G вгору/вниз  q назад

# CLI shared UI strings
common-warning-config-default = попередження: { $error }; використовуються стандартні значення конфігурації для цієї команди
ui-brand-tagline = Відкрита операційна система для агентів
ui-brand-title = LibreFang Agent OS
ui-label-hint = підказка:
ui-label-next-steps = Наступні кроки
ui-label-fix = виправити:
ui-label-try = спробувати:
ui-provider-not-set = { $env_var } не налаштовано
progress-fail = [ПОМИЛКА]

# Table headers / Shared labels
label-header-name = НАЗВА
label-header-kind = ТИП
label-header-configured = НАЛАШТОВАНО
label-header-token = ТОКЕН
label-header-alias = АЛІАС
label-header-provider = ПРОВАЙДЕР
label-header-id = ID
label-header-agent = АГЕНТ
label-header-type = ТИП
label-header-timestamp = ЧАС
label-header-event = ПОДІЯ
label-header-key = КЛЮЧ
label-header-value = ЗНАЧЕННЯ
label-header-enabled = УВІМКНЕНО
label-header-url = URL

# Channel command specific keys
channel-header-msgs-24h = ПОВІДОМЛЕНЬ ЗА 24 ГОД
channel-error-save-failed-no-body = збереження відхилено (тіло помилки відсутнє)

# Models command specific keys
model-none-in-catalog = У каталозі немає моделей.
model-header-model = МОДЕЛЬ
model-header-tier = ТІР
model-header-context = КОНТЕКСТ
model-header-resolves-to = ВКАЗУЄ НА
model-header-auth = АВТОРИЗАЦІЯ
model-header-models = МОДЕЛІ
model-header-base-url = БАЗОВИЙ URL
model-picker-item =     { $idx }. { $id } { $tier }

# Approvals command specific keys
approval-none-pending = Немає апрувів у черзі.
approval-header-request = ЗАПИТ

# Auth command specific keys
auth-error-create-home-dir = Не вдалося створити домашню директорію LibreFang: { $error }
auth-error-write-secrets = Не вдалося записати secrets.env: { $error }
auth-error-parse-config = Не вдалося розібрати config.toml: { $error }
auth-error-default-model-not-table = default_model не є таблицею
auth-error-write-config = Не вдалося записати config.toml: { $error }
auth-pool-add-hint = Додайте його за допомогою:
auth-pool-add-example =   librefang auth pool add openai OPENAI_API_KEY_1 --label Primary --priority 10
auth-pool-header = { $provider }  ({ $strategy })
auth-pool-keys-available =   доступно ключів: { $available }/{ $total }
auth-pool-cooldown-left = (залишилось { $secs }с)
auth-pool-status-invalid = некоректний
auth-pool-status-exhausted = вичерпаний
auth-pool-status-cooldown = охолодження
auth-pool-status-env-missing = відсутня змінна оточення
auth-pool-status-healthy = здоровий
auth-pool-key-requests = запитів={ $count }
auth-pool-key-item =     - [{ $label }] { $key_display }  priority={ $pri }{ $reqs_str }  status={ $status }
auth-hash-add-config-hint = Додайте до config.toml:
auth-hash-config-entry =   dashboard_pass_hash = "{ $hash }"

# Agent command specific keys
agent-spawn-id-label =   ID:   { $id }
agent-spawn-name-label =   Ім'я: { $name }
error-unknown = Невідома помилка
label-unknown = <невідомо>
label-header-state = СТАТУС
label-header-created = СТВОРЕНО
label-header-msgs = ПОВІД.
label-header-last-active = ВОСТАННЄ АКТИВНИЙ
label-session-state-running = активна
label-session-state-idle = очікування

# Daemon command specific keys
daemon-error-resolve-exe = визначення поточного виконуваного файлу: { $error }
daemon-error-create-log-dir = створення директорії логів { $path }: { $error }
daemon-error-open-log = відкриття логу демона { $path }: { $error }
daemon-error-clone-log-handle = клонування дескриптора логу демона { $path }: { $error }
daemon-error-spawn-detached = запуск фонового процесу демона: { $error }
daemon-error-failed-create-log-dir = Не вдалося створити директорію логів { $path }: { $error }
daemon-error-failed-open-log = Не вдалося відкрити файл логу демона { $path }: { $error }

# --- Skill commands ---
skill-name-empty = назва скіла порожня
skill-name-unsafe = небезпечна назва скіла '{ $name }': повинна бути одним компонентом шляху (без '/', '\', '..' або абсолютного шляху)
skill-hand-not-found = Hand '{ $hand }' не знайдено у { $path }
skill-openclaw-detected = Виявлено формат скіла OpenClaw. Конвертація...
skill-install-refused = Відмова у встановленні скіла: { $error }
skill-write-manifest-failed = Не вдалося записати маніфест: { $error }
skill-openclaw-installed-to-hand = Встановлено OpenClaw скіл '{ $name }' до hand '{ $hand }'
skill-openclaw-installed = Встановлено OpenClaw скіл: { $name }
skill-openclaw-convert-failed = Не вдалося конвертувати OpenClaw скіл: { $error }
skill-no-toml = Не знайдено skill.toml у { $path }
skill-read-toml-failed = Помилка читання skill.toml: { $error }
skill-parse-toml-failed = Помилка парсингу skill.toml: { $error }
skill-installed-to-hand = Встановлено скіл '{ $name }' v{ $version } до hand '{ $hand }'
skill-installed = Встановлено скіл: { $name } v{ $version }
skill-installed-hub-to-hand = Встановлено { $source } { $version } до hand '{ $hand }'
skill-installed-hub = Встановлено { $source } { $version }
skill-install-failed = Не вдалося встановити скіл: { $error }
skill-list-none-hand = Немає встановлених скілів для hand '{ $hand }'.
skill-list-none = Немає встановлених скілів.
skill-list-count-hand = Встановлено скілів ({ $count }) для hand '{ $hand }':
skill-list-count = Встановлено скілів ({ $count }):
skill-list-load-failed = Помилка завантаження скілів: { $error }
skill-removed-from-hand = Видалено скіл '{ $name }' з hand '{ $hand }'
skill-removed = Видалено скіл: { $name }
skill-remove-failed = Не вдалося видалити скіл: { $error }
skill-search-none = Не знайдено скілів за запитом "{ $query }".
skill-search-results-header = Знайдено скіли за запитом "{ $query }":
skill-search-failed = Помилка пошуку: { $error }
skill-validation-failed = Помилка валідації скіла: { $error }
skill-execution-failed = Помилка виконання скіла: { $error }
skill-package-failed = Не вдалося упакувати скіл: { $error }
skill-determine-dir-failed = Не вдалося визначити поточну директорію: { $error }
skill-unsupported-runtime = Непідтримуване середовище виконання '{ $runtime }'. Оберіть одне з: python, node, wasm.
skill-create-dir-failed = Помилка створення директорії скіла: { $error }
skill-registry-load-failed = Помилка завантаження реєстру скілів: { $error }
skill-not-found = Скіл '{ $name }' не знайдено у { $path }
skill-read-file-failed = Не вдалося прочитати { $path }: { $error }
skill-create-skills-dir-failed = Не вдалося створити директорію скілів: { $error }
skill-create-failed = Помилка створення: { $error }
skill-update-failed = Помилка оновлення: { $error }
skill-patch-failed = Помилка накладання патчу: { $error }
skill-delete-failed = Помилка видалення: { $error }
skill-rollback-failed = Помилка відкату змін: { $error }
skill-write-file-failed = Помилка запису файлу: { $error }
skill-remove-file-failed = Помилка видалення файлу: { $error }
skill-serialize-history-failed = Не вдалося серіалізувати історію: { $error }
skill-read-pending-failed = Не вдалося прочитати директорію очікуваних скілів: { $error }
skill-load-candidate-failed = Не вдалося завантажити кандидата: { $error }
skill-render-candidate-failed = Не вдалося згенерувати кандидата як TOML: { $error }
skill-approve-candidate-failed = Помилка апруву: { $error }
skill-reject-candidate-failed = Помилка відхилення: { $error }
skill-publish-failed = Помилка публікації: { $error }
skill-evolution-label = Скіл: { $name }
skill-version-label = Поточна версія: { $version }
skill-use-count-label = Кількість використання: { $count }
skill-evolution-count-label = Кількість еволюцій: { $count }
skill-no-history = Історію версій не записано.
skill-no-pending = Немає очікуваних кандидатів скілів.{ $filter }
skill-pending-filter =  (фільтр: агент { $agent })
skill-approved-candidate = Апрувнуто кандидата { $id } → встановлено скіл '{ $name }' (v{ $version }).
skill-rejected-candidate = Відхилено та видалено кандидата { $id }.
skill-validated = Валідовано скіл: { $name } v{ $version }
skill-validated-runtime =   Середовище: { $runtime }
skill-validated-source =   Джерело: { $path }
skill-validated-description =   Опис: { $description }
skill-validated-tools =   Тули: { $tools }
skill-refusing-warnings = Відмова у виконанні скіла з критичними попередженнями валідації.
skill-validated-only = Тільки валідація: не оголошено тулу для виконання.
skill-invalid-input-json = Некоректний --input JSON: { $error }
skill-tool-result-header = Результат тули ({ $name }):
skill-validation-complete = Валідацію завершено.
skill-execution-skipped = Виконання пропущено: { $message }
skill-preparing = Підготовка скіла: { $name } v{ $version }
skill-refusing-publish = Відмова в публікації скіла з критичними попередженнями валідації.
skill-bundle-created = Архів створено: { $path }
skill-bundle-sha =   SHA256: { $sha }
skill-bundle-size =   Розмір: { $size } байт
skill-dry-run = Тільки сухий запуск (dry run).
skill-dry-run-repo =   Репозиторій: { $repo }
skill-dry-run-tag =   Тег: { $tag }
skill-github-token-required = Встановіть GITHUB_TOKEN або GH_TOKEN для публікації, або перезапустіть з параметром --dry-run.
skill-publishing-progress = Публікація { $name }@{ $tag }
skill-publish-success = Опубліковано { $name } в { $repo }@{ $tag }
skill-publish-release-url = Реліз: { $url }
skill-warnings-none =   Попередження: немає
skill-warnings-header =   Попередження:
skill-prompt-name = Назва скіла: 
skill-prompt-description = Опис: 
skill-prompt-runtime = Середовище (python/node/wasm) [python]: 
skill-created = Скіл створено: { $path }
skill-created-files-header = Файли:
skill-created-next-steps-header = Наступні кроки:
skill-created-step-edit =   { $step }. Відредагуйте точку входу для реалізації логіки скіла
skill-created-step-test =   { $step }. Протестуйте локально: librefang skill test { $path }
skill-created-step-install =   { $step }. Встановіть: librefang skill install { $path }

# --- Monitoring & Status ---
monitoring-no-audit = Немає записів аудиту безпеки.
monitoring-no-memory = Немає записів пам'яті для агента '{ $agent }'.
monitoring-no-devices = Немає підключених пристроїв.
monitoring-no-webhooks = Вебхуки не налаштовані.
label-last-seen = ВОСТАННЄ БАЧИЛИ
status-watch-header =   { $status } (оновлення кожні { $interval }с, Ctrl+C для виходу)
status-warning-config = попередження: { $error }; використовуються стандартні значення конфігурації для показу статусу
status-summary-up = librefang { $version } { $state } час роботи={ $uptime } { $auth } ({ $base })
status-peers-connected = { $connected } підключено / { $total } відомо
status-agents-active = { $active } запущено / { $total } всього
status-mb = { $mb } МБ
status-summary-down = librefang зупинено home={ $home } default={ $provider }/{ $model }
status-uptime-format = { $hours }год { $mins }хв
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
format-bytes-gib = { $value } ГіБ
format-bytes-mib = { $value } МіБ
format-bytes-kib = { $value } КіБ
format-bytes-b = { $value } Б
format-size-mb = ({ $value } МБ)

format-uptime-s = { $secs }с
format-uptime-ms = { $mins }хв { $secs }с
format-uptime-hm = { $hours }г { $mins }хв
format-uptime-hms = { $hours }г { $mins }хв { $secs }с
format-uptime-dh = { $days }д { $hours }г
format-uptime-dhm = { $days }д { $hours }г { $mins }хв

# --- Desktop install & Update errors ---
desktop-install-unsupported-platform = Непідтримувана платформа
desktop-install-error-hdiutil-attach = Помилка підключення hdiutil attach: { $error }
desktop-install-error-app-not-found = LibreFang.app не знайдено в DMG
desktop-install-error-remove-old = Не вдалося видалити попередню інсталяцію: { $error }
desktop-install-error-cp = Помилка виконання cp: { $error }
desktop-install-error-copy-applications = Помилка копіювання до /Applications: { $error }
desktop-install-error-run-installer = Не вдалося запустити інсталятор: { $error }
desktop-install-error-installer-status =  Інсталятор завершив роботу з помилкою: { $status }
desktop-install-error-localappdata = Не вдалося визначити %LOCALAPPDATA%
desktop-install-error-binary-not-found = Інсталятор завершив роботу, але бінарний файл не знайдено за очікуваним шляхом
desktop-install-error-home-dir = Не вдалося визначити домашню директорію
desktop-install-error-create-dir = Не вдалося створити { $path }: { $error }
desktop-install-error-copy-appimage =  Не вдалося скопіювати AppImage: { $error }
desktop-install-error-http = HTTP-запит завершився помилкою: { $error }
desktop-install-error-create = Не вдалося створити { $path }: { $error }
desktop-install-error-write = Помилка запису: { $error }

maintenance-error-github-request = GitHub-запит завершився помилкою: { $error }
maintenance-error-github-status = GitHub API повернув статус { $status }
maintenance-error-decode-release = Не вдалося декодувати метадані релізу: { $error }
maintenance-error-missing-tag = У метаданих релізу відсутній `tag_name`
maintenance-error-decode-list = Не вдалося декодувати список релізів: { $error }
maintenance-error-no-release = Не знайдено відповідного релізу для каналу '{ $channel }'
maintenance-error-http-client = Не вдалося створити HTTP-клієнт: { $error }
maintenance-error-powershell-updater = Не вдалося запустити оновлення через PowerShell: { $error }
maintenance-error-run-installer = Не вдалося запустити інсталятор: { $error }
maintenance-error-installer-status = Інсталятор завершив роботу зі статусом { $status }
maintenance-error-download-fail = Помилка завантаження: { $error }
maintenance-error-download-status = Завантаження повернуло статус { $status }
maintenance-error-read-response = Не вдалося прочитати тіло відповіді: { $error }
maintenance-error-create-dir = Не вдалося створити директорію оновлень: { $error }
maintenance-error-create-script = Не вдалося створити скрипт оновлення: { $error }
maintenance-error-write-script = Не вдалося записати скрипт оновлення: { $error }

common-error-find-exe = Не вдалося знайти виконуваний файл: { $error }
common-error-spawn-daemon = Не вдалося запустити демон: { $error }
common-error-daemon-timeout = Демон не став готовим протягом 10 секунд

# tui/chat_runner.rs
chat-runner-owner-notice = [owner_notice] { $preview }
chat-runner-error-prefix = Помилка: { $error }
chat-runner-no-active-connection = Немає активного з'єднання
chat-runner-unknown-command = Невідома команда: { $command }. Введіть /help
chat-runner-status-mode-daemon = Режим: демон ({ $url })
chat-runner-status-agent = Агент: { $name }
chat-runner-status-mode-inprocess = Режим: вбудоване ядро
chat-runner-status-agents-count = Агенти: { $count }
chat-runner-status-mode-disconnected = Режим: відключено
chat-runner-chat-history-cleared = Історію чату очищено.
chat-runner-agent-killed = Агента "{ $name }" примусово зупинено.
chat-runner-failed-kill-agent = Не вдалося примусово зупинити агента "{ $name }".
chat-runner-kill-failed = Не вдалося примусово зупинити: { $error }
chat-runner-no-backend-connected = Не підключено до бекенду.
chat-runner-no-models-available = Немає доступних моделей.
chat-runner-switched-model = Переключено на { $model }
chat-runner-failed-switch-model = Не вдалося переключитися на { $model }
chat-runner-switch-failed = Помилка перемикання: { $error }
chat-runner-welcome-help-hint = /help для довідки • /exit для виходу
chat-runner-spawning-agent = Запуск агента '{ $name }'…
chat-runner-no-agent-templates = Шаблонів агентів не знайдено. Запустіть `librefang init`.
chat-runner-invalid-template = Некоректний шаблон '{ $name }': { $error }
chat-runner-spawn-failed = Не вдалося запустити: { $error }
chat-runner-booting-kernel = Завантаження ядра…
chat-runner-booting-kernel-hint =   Це може зайняти деякий час, поки ядро ініціалізується.
chat-runner-failed-start = Не вдалося запустити
chat-runner-press-esc-to-exit =   Натисніть Esc для виходу.

