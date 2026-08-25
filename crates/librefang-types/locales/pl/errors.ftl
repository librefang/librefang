# --- API error messages (Polish) ---

# Agent errors
api-error-agent-not-found = Nie znaleziono agenta
api-error-agent-spawn-failed = Uruchomienie agenta nie powiodło się
api-error-agent-invalid-id = Nieprawidłowy identyfikator agenta
api-error-agent-already-exists = Agent już istnieje
api-error-agent-no-workspace = Agent nie ma obszaru roboczego
api-error-agent-not-found-or-terminated = Nie znaleziono agenta lub został już zakończony
api-error-agent-vanished = Agent zniknął podczas aktualizacji
api-error-agent-no-agents-available = Brak dostępnych agentów
api-error-agent-no-target = Nie znaleziono agenta docelowego. Podaj agent_id lub najpierw uruchom agenta.
api-error-agent-source-not-found = Nie znaleziono agenta źródłowego
api-error-agent-target-not-found = Nie znaleziono agenta docelowego
api-error-agent-execution-failed = Wykonanie agenta nie powiodło się: { $error }
api-error-agent-clone-spawn-failed = Nie udało się uruchomić klona: { $error }
api-error-agent-error = Błąd agenta: { $error }
api-error-agent-not-found-with-id = Nie znaleziono agenta: { $id }
api-error-agent-invalid-sort = Nieprawidłowe pole sortowania '{ $field }'. Prawidłowe pola: { $valid }

# Message errors
api-error-message-too-large = Wiadomość jest za duża (maks. 64 KB)
api-error-message-delivery-failed = Dostarczenie wiadomości nie powiodło się: { $reason }
api-error-message-required = Wiadomość jest wymagana
api-error-message-missing-field = Brak pola 'message'
api-error-message-streaming-failed = Nie udało się wysłać wiadomości strumieniowej

# Template errors
api-error-template-invalid-name = Nieprawidłowa nazwa szablonu
api-error-template-not-found = Nie znaleziono szablonu '{ $name }'
api-error-template-parse-failed = Nie udało się przeanalizować szablonu: { $error }
api-error-template-required = Wymagane jest 'manifest_toml' lub 'template'
api-error-template-invalid-manifest = Nieprawidłowy manifest szablonu
api-error-template-read-failed = Nie udało się odczytać szablonu
api-error-agent-type-exists = Typ agenta '{ $name }' już istnieje
api-error-agent-type-name-taken = '{ $name }' to nazwa działającego agenta; wybierz inną nazwę dla typu agenta
api-error-agent-type-not-editable = Typ agenta '{ $name }' pochodzi z obszaru roboczego działającego agenta i jest zarządzany przez /api/agents

# Manifest errors
api-error-manifest-too-large = Manifest jest za duży (maks. 1 MB)
api-error-manifest-invalid-format = Nieprawidłowy format manifestu
api-error-manifest-signature-mismatch = Treść podpisanego manifestu nie zgadza się z manifest_toml
api-error-manifest-signature-failed = Weryfikacja podpisu manifestu nie powiodła się
api-error-manifest-invalid = Nieprawidłowy manifest: { $error }

# Auth errors
api-error-auth-invalid-key = Nieprawidłowy klucz API
api-error-auth-missing-header = Brak nagłówka Authorization: Bearer <klucz_api>
api-error-auth-missing = Klucz API nie jest skonfigurowany dla tego dostawcy

# Session errors
api-error-session-load-failed = Wczytanie sesji nie powiodło się
api-error-session-not-found = Nie znaleziono sesji
api-error-session-invalid-id = Nieprawidłowy identyfikator sesji
api-error-context-report-failed = Raport kontekstu nie powiódł się
api-error-session-no-label = Nie znaleziono sesji z tą etykietą
api-error-session-cleanup-expired-failed = Nie udało się wyczyścić wygasłych sesji: { $error }
api-error-session-cleanup-excess-failed = Nie udało się wyczyścić nadmiarowych sesji: { $error }

# Workflow errors
api-error-workflow-missing-steps = Brak tablicy 'steps'
api-error-workflow-step-needs-agent = Krok '{ $step }' wymaga 'agent_id' lub 'agent_name'
api-error-workflow-invalid-id = Nieprawidłowy identyfikator przepływu pracy
api-error-workflow-execution-failed = Wykonanie przepływu pracy nie powiodło się
api-error-workflow-not-found = Nie znaleziono przepływu pracy

# Trigger errors
api-error-trigger-missing-agent-id = Brak 'agent_id'
api-error-trigger-invalid-agent-id = Nieprawidłowy agent_id
api-error-trigger-invalid-pattern = Nieprawidłowy wzorzec wyzwalacza
api-error-trigger-missing-pattern = Brak 'pattern'
api-error-trigger-registration-failed = Rejestracja wyzwalacza nie powiodła się (nie znaleziono agenta?)
api-error-trigger-invalid-id = Nieprawidłowy identyfikator wyzwalacza
api-error-trigger-not-found = Nie znaleziono wyzwalacza

# Budget errors
api-error-budget-invalid-amount = Nieprawidłowa kwota budżetu
api-error-budget-update-failed = Aktualizacja budżetu nie powiodła się
api-error-budget-provide-at-least-one = Podaj co najmniej jedno z: max_cost_per_hour_usd, max_cost_per_day_usd, max_cost_per_month_usd, max_llm_tokens_per_hour

# Config errors
api-error-config-parse-failed = Nie udało się przeanalizować konfiguracji: { $error }
api-error-config-write-failed = Nie udało się zapisać konfiguracji: { $error }
api-error-config-save-failed = Nie udało się zapisać konfiguracji: { $error }
api-error-config-remove-failed = Nie udało się usunąć konfiguracji: { $error }
api-error-config-missing-toml = Brak pola toml_content

# Profile errors
api-error-profile-not-found = Nie znaleziono profilu '{ $name }'

# Cron errors
api-error-cron-invalid-id = Nieprawidłowy identyfikator zadania cron
api-error-cron-not-found = Nie znaleziono zadania cron
api-error-cron-create-failed = Nie udało się utworzyć zadania cron: { $error }
api-error-cron-invalid-expression = Nieprawidłowe wyrażenie cron
api-error-cron-invalid-expression-detail = Nieprawidłowe wyrażenie cron: wymaga 5 pól (minuta godzina dzień miesiąc dzień_tygodnia)
api-error-cron-missing-field = Brak pola 'cron'

# Goal errors
api-error-goal-not-found = Nie znaleziono celu
api-error-goal-not-found-with-id = Nie znaleziono celu '{ $id }'
api-error-goal-missing-title = Brak lub puste pole 'title'
api-error-goal-title-too-long = Tytuł jest za długi (maks. 256 znaków)
api-error-goal-description-too-long = Opis jest za długi (maks. 4096 znaków)
api-error-goal-invalid-status = Nieprawidłowy status. Musi być jednym z: pending, in_progress, completed, cancelled
api-error-goal-progress-range = Postęp musi być w zakresie 0–100
api-error-goal-parent-not-found = Nie znaleziono celu nadrzędnego '{ $id }'
api-error-goal-self-parent = Cel nie może być własnym celem nadrzędnym
api-error-goal-circular-parent = Wykryto cykliczne odwołanie nadrzędne
api-error-goal-save-failed = Nie udało się zapisać celu: { $error }
api-error-goal-update-failed = Nie udało się zaktualizować celu: { $error }
api-error-goal-delete-failed = Nie udało się usunąć celu: { $error }
api-error-goal-load-failed = Nie udało się wczytać celów: { $error }
api-error-goal-title-empty = Tytuł nie może być pusty
api-error-goal-status-invalid = Nieprawidłowy status

# Memory errors
api-error-memory-not-enabled = Proaktywna pamięć nie jest włączona
api-error-memory-not-found = Nie znaleziono pamięci
api-error-memory-operation-failed = Operacja pamięci nie powiodła się
api-error-memory-export-failed = Nie udało się wyeksportować pamięci
api-error-memory-import-failed = Nie udało się zaimportować pamięci podczas czyszczenia
api-error-memory-key-not-found = Nie znaleziono klucza
api-error-memory-missing-kv = W treści żądania brakuje obiektu 'kv' lub jest on nieprawidłowy
api-error-memory-serialization-error = Błąd serializacji
api-error-memory-missing-ids = Brak tablicy 'ids'

# Network / A2A errors
api-error-network-not-enabled = Sieć równorzędna nie jest włączona
api-error-network-peer-not-found = Nie znaleziono węzła
api-error-network-a2a-not-found = Nie znaleziono agenta A2A '{ $url }'
api-error-network-connection-failed = Połączenie nie powiodło się: { $error }
api-error-network-auth-failed = Uwierzytelnianie nie powiodło się (HTTP { $status })
api-error-network-task-post-failed = Nie udało się wysłać zadania: { $error }
api-error-network-missing-url = Brak parametru zapytania 'url'

# Plugin errors
api-error-plugin-missing-name = Brak 'name'
api-error-plugin-missing-name-registry = Brak 'name' dla instalacji z rejestru
api-error-plugin-missing-path = Brak 'path' dla instalacji lokalnej
api-error-plugin-missing-url = Brak 'url' dla instalacji z git
api-error-plugin-invalid-source = Nieprawidłowe źródło. Użyj jednego z: 'registry', 'local', 'git'

# Channel errors
api-error-channel-unknown = Nieznany kanał
api-error-channel-missing-agent-id = Brak wymaganego pola: agent_id
api-error-channel-invalid-from = Nieprawidłowy from_agent_id
api-error-channel-invalid-to = Nieprawidłowy to_agent_id

# Provider errors
api-error-provider-missing-alias = Brak wymaganego pola: alias
api-error-provider-missing-model-id = Brak wymaganego pola: model_id
api-error-provider-missing-id = Brak wymaganego pola: id
api-error-provider-missing-key = Brak lub puste pole 'key'
api-error-provider-alias-exists = Alias '{ $alias }' już istnieje
api-error-provider-alias-not-found = Nie znaleziono aliasu '{ $alias }'
api-error-provider-model-not-found = Nie znaleziono modelu '{ $id }'
api-error-provider-not-found = Nie znaleziono dostawcy '{ $name }'
api-error-provider-model-exists = Model '{ $id }' już istnieje u dostawcy '{ $provider }'
api-error-provider-custom-model-not-found = Nie znaleziono modelu niestandardowego '{ $id }'
api-error-provider-no-key-required = Ten dostawca nie wymaga klucza API
api-error-provider-key-not-configured = Klucz API dostawcy nie jest skonfigurowany
api-error-provider-secrets-write-failed = Nie udało się zapisać secrets.env: { $error }
api-error-provider-secrets-update-failed = Nie udało się zaktualizować secrets.env: { $error }
api-error-provider-invalid-url = Nieprawidłowy format adresu URL
api-error-provider-missing-url = Brak lub puste 'url'
api-error-provider-missing-base-url = Brak lub puste pole 'base_url'
api-error-provider-unknown = Nieznany dostawca '{ $name }'
api-error-provider-base-url-invalid = base_url musi zaczynać się od http:// lub https://
api-error-provider-missing-model = Brak pola 'model'
api-error-provider-token-save-failed = Nie udało się zapisać tokena: { $error }
api-error-provider-unknown-poll = Nieznany poll_id
api-error-provider-secret-write-failed = Nie udało się zapisać tajnego klucza: { $error }

# Skill errors
api-error-skill-missing-name = Brak lub puste pole 'name'
api-error-skill-invalid-name = Nazwa umiejętności może zawierać tylko znaki alfanumeryczne, myślniki i znaki podkreślenia
api-error-skill-not-found-source = Nie znaleziono kodu źródłowego dla tej umiejętności
api-error-skill-only-prompt = Z interfejsu WWW można tworzyć tylko umiejętności oparte na prompcie
api-error-skill-name-too-long = Nazwa przekracza maksymalną długość (256 znaków)
api-error-skill-description-too-long = Opis przekracza maksymalną długość ({ $max } znaków)
api-error-skill-dir-create-failed = Nie udało się utworzyć katalogu umiejętności: { $error }
api-error-skill-toml-write-failed = Nie udało się zapisać skill.toml: { $error }
api-error-skill-install-failed = Instalacja nie powiodła się: { $error }

# Hand errors
api-error-hand-not-found = Nie znaleziono dłoni: { $id }
api-error-hand-definition-not-found = Nie znaleziono definicji dłoni
api-error-hand-instance-not-found = Nie znaleziono instancji

# MCP errors
api-error-mcp-missing-name = Brak pola 'name'
api-error-mcp-missing-transport = Brak pola 'transport'
api-error-mcp-invalid-config = Nieprawidłowa konfiguracja serwera MCP: { $error }
api-error-mcp-not-found = Nie znaleziono serwera MCP '{ $name }'
api-error-mcp-write-failed = Nie udało się zapisać konfiguracji: { $error }

# Integration/Extension errors
api-error-integration-not-found = Nie znaleziono integracji '{ $id }'
api-error-integration-missing-id = Brak pola 'id'
api-error-extension-not-found = Nie znaleziono rozszerzenia '{ $id }'

# System errors
api-error-system-cli-not-found = Nie znaleziono CLI w PATH

# KV / Structured memory errors
api-error-kv-missing-fields = Brak obiektu 'fields'
api-error-kv-missing-value = Brak pola 'value'
api-error-kv-array-empty = Tablica nie może być pusta
api-error-kv-missing-path = Brak pola 'path'

# Approval errors
api-error-approval-invalid-id = Nieprawidłowy identyfikator zatwierdzenia
api-error-approval-not-found = Nie znaleziono zatwierdzenia

# Webhook errors
api-error-webhook-not-enabled = Wyzwalacze webhook nie są włączone
api-error-webhook-invalid-id = Nieprawidłowy identyfikator webhooka
api-error-webhook-not-found = Nie znaleziono webhooka
api-error-webhook-missing-url = Brak pola 'url'
api-error-webhook-missing-events = Brak tablicy 'events'
api-error-webhook-invalid-events = Typy zdarzeń muszą być ciągami znaków
api-error-webhook-event-types-required = Wymagany jest co najmniej jeden typ zdarzenia
api-error-webhook-url-unreachable = Adres URL webhooka jest nieosiągalny: { $error }
api-error-webhook-event-publish-failed = Nie udało się opublikować zdarzenia: { $error }
api-error-webhook-invalid-url = Nieprawidłowy format adresu URL webhooka
api-error-webhook-agent-exec-failed = Wykonanie agenta webhooka nie powiodło się: { $error }
api-error-webhook-reach-failed = Nie udało się dotrzeć do adresu URL webhooka: { $error }
api-error-webhook-unknown-event = Nieznany typ zdarzenia '{ $event }'. Prawidłowe typy: { $valid }

# Backup errors
api-error-backup-not-found = Nie znaleziono kopii zapasowej
api-error-backup-file-not-found = Nie znaleziono pliku kopii zapasowej
api-error-backup-invalid-filename = Nieprawidłowa nazwa pliku kopii zapasowej
api-error-backup-invalid-filename-zip = Nieprawidłowa nazwa pliku kopii zapasowej — musi być plikiem .zip
api-error-backup-missing-manifest = W archiwum kopii zapasowej brakuje manifest.json — to nie jest prawidłowa kopia zapasowa LibreFang
api-error-backup-dir-create-failed = Nie udało się utworzyć katalogu kopii zapasowej: { $error }
api-error-backup-file-create-failed = Nie udało się utworzyć pliku kopii zapasowej: { $error }
api-error-backup-finalize-failed = Nie udało się sfinalizować kopii zapasowej: { $error }
api-error-backup-open-failed = Nie udało się otworzyć kopii zapasowej: { $error }
api-error-backup-invalid-archive = Nieprawidłowe archiwum kopii zapasowej: { $error }
api-error-backup-delete-failed = Nie udało się usunąć kopii zapasowej: { $error }
api-error-backup-invalid-keep-config = Nieprawidłowe 'keep_config' — musi być wartością logiczną
api-error-backup-invalid-components = Nieprawidłowe 'components' — musi być tablicą nazw komponentów
api-error-backup-empty-components = 'components' nie może być puste — pomiń to pole, aby przywrócić wszystkie komponenty
api-error-backup-unknown-component = Nieznany komponent kopii zapasowej '{ $component }'. Prawidłowe komponenty: { $valid }

# Schedule errors
api-error-schedule-not-found = Nie znaleziono harmonogramu
api-error-schedule-missing-cron = Brak pola 'cron'
api-error-schedule-missing-enabled = Brak pola 'enabled'
api-error-schedule-invalid-cron = Nieprawidłowe wyrażenie cron
api-error-schedule-invalid-cron-detail = Nieprawidłowe wyrażenie cron: wymaga 5 pól (minuta godzina dzień miesiąc dzień_tygodnia)
api-error-schedule-save-failed = Nie udało się zapisać harmonogramu: { $error }
api-error-schedule-update-failed = Nie udało się zaktualizować harmonogramu: { $error }
api-error-schedule-delete-failed = Nie udało się usunąć harmonogramu: { $error }
api-error-schedule-load-failed = Nie udało się wczytać harmonogramu: { $error }

# Job errors
api-error-job-invalid-id = Nieprawidłowy identyfikator zadania
api-error-job-not-found = Nie znaleziono zadania
api-error-job-not-retryable = Nie znaleziono zadania lub nie jest w stanie umożliwiającym ponowienie (musi być ukończone lub nieudane)
api-error-job-disappeared-cancel = Zadanie zniknęło po anulowaniu
api-error-job-disappeared-complete = Zadanie zniknęło po ukończeniu

# Task errors
api-error-task-not-found = Nie znaleziono zadania
api-error-task-disappeared = Zadanie zniknęło

# Pairing errors
api-error-pairing-not-enabled = Parowanie nie jest włączone
api-error-pairing-invalid-token = Nieprawidłowy lub brakujący token

# Binding errors
api-error-binding-out-of-range = Indeks powiązania jest poza zakresem

# Command errors
api-error-command-not-found = Nie znaleziono polecenia '{ $name }'

# File/Upload errors
api-error-file-not-found = Nie znaleziono pliku
api-error-file-not-in-whitelist = Plik nie znajduje się na białej liście
api-error-file-too-large = Plik jest za duży (maks. { $max })
api-error-file-content-too-large = Treść pliku jest za duża (maks. 32 KB)
api-error-file-empty-body = Pusta treść pliku
api-error-file-save-failed = Nie udało się zapisać pliku
api-error-file-missing-filename = Brak pola 'filename'
api-error-file-missing-path = Brak pola 'path'
api-error-file-path-too-deep = Ścieżka jest za głęboka (maks. 3 poziomy)
api-error-file-path-traversal = Odmowa przejścia ścieżki (path traversal)
api-error-file-unsupported-type = Nieobsługiwany typ treści. Dozwolone: image/*, text/*, audio/*, application/pdf
api-error-file-upload-dir-failed = Nie udało się utworzyć katalogu przesyłania
api-error-file-dir-not-found = Nie znaleziono katalogu
api-error-file-workspace-error = Błąd ścieżki obszaru roboczego

# Tool errors
api-error-tool-provide-allowlist = Podaj 'tool_allowlist' i/lub 'tool_blocklist'
api-error-tool-not-found = Nie znaleziono narzędzia: { $name }
api-error-tool-invoke-disabled = Bezpośrednie wywoływanie narzędzi jest wyłączone. Włącz '[tool_invoke] enabled = true' i dodaj narzędzie do 'allowlist'.
api-error-tool-invoke-denied = Narzędzie '{ $name }' nie znajduje się w '[tool_invoke] allowlist'
api-error-tool-requires-agent = Narzędzie '{ $name }' wymaga zatwierdzenia przez człowieka i nie może być wywołane bez kontekstu agenta; wywołaj je przez agenta

# Validation errors
api-error-validation-content-empty = Treść nie może być pusta
api-error-validation-name-empty = new_name nie może być puste
api-error-validation-title-required = Tytuł jest wymagany
api-error-validation-avatar-url-invalid = Adres URL awatara musi być http/https lub identyfikatorem URI danych
api-error-validation-color-invalid = Kolor musi być kodem szesnastkowym zaczynającym się od '#'

# General errors
api-error-not-found = Nie znaleziono zasobu
api-error-internal = Wewnętrzny błąd serwera
api-error-bad-request = Nieprawidłowe żądanie: { $reason }
api-error-rate-limited = Przekroczono limit żądań. Spróbuj ponownie później.

# Generic catch-all — interpolates the underlying error string verbatim.
# Used by 41+ HTTP 500 handlers as a stopgap until each route is moved to a
# typed MemoryRouteError-style helper. Without this key, every `t_args("api-error-generic", …)`
# call returns the literal key as the response body and `$error` interpolation never runs.
api-error-generic = Błąd: { $error }
