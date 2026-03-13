# --- Daemon lifecycle ---
daemon-starting = Starting daemon...
daemon-stopped = LibreFang daemon stopped.
kernel-booted = Kernel booted ({ $provider }/{ $model })
models-available = { $count } models available
agents-loaded = { $count } agent(s) loaded
daemon-started-bg = Daemon started in background
daemon-still-starting = Daemon launched in background and is still starting
daemon-stopped-ok = Daemon stopped
daemon-stopped-forced = Daemon stopped (forced)
daemon-error = Daemon error: { $error }
daemon-already-running = Daemon already running at { $url }
daemon-already-running-fix = Use `librefang status` to check it, or stop it first
daemon-not-running = Daemon is not running.
daemon-not-running-start = Daemon is not running. Start it with: librefang start
daemon-no-running-found = No running daemon found
daemon-no-running-found-fix = Is it running? Check with: librefang status
daemon-restarting = Restarting daemon...
daemon-no-running-starting = No running daemon found; starting a new daemon
daemon-bg-exited = Background daemon exited before becoming healthy ({ $status })
daemon-bg-exited-fix = Check startup logs: { $path }
daemon-bg-wait-fail = Failed while waiting for background daemon
daemon-bg-wait-fail-fix = { $error }. Check startup logs: { $path }
daemon-launch-fail = Failed to launch background daemon
daemon-no-running-auto = No daemon running — starting one now...
daemon-started = Daemon started
daemon-start-fail = Could not start daemon: { $error }
daemon-start-fail-fix = Start it manually: librefang start
shutdown-request-fail = Shutdown request failed ({ $status })
could-not-reach-daemon = Could not reach daemon: { $error }

# --- Labels ---
label-api = API
label-dashboard = Dashboard
label-provider = Provider
label-model = Model
label-pid = PID
label-log = Log
label-status = Status
label-agents = Agents
label-data-dir = Data dir
label-uptime = Uptime
label-version = Version
label-daemon = Daemon
label-id = ID
label-active-agents = Active agents
label-pairing-code = Pairing code
label-expires = Expires

# --- Hints ---
hint-open-dashboard = Open the dashboard in your browser, or run `librefang chat`
hint-stop-daemon = Use `librefang stop` to stop the daemon
hint-tail-stop = Ctrl+C stops log tailing; the daemon keeps running
hint-check-status = Run `librefang status` to check readiness
hint-start-daemon = Start it with: librefang start
hint-start-daemon-cmd = Start the daemon: librefang start
hint-or-chat = Or try `librefang chat` which works without a daemon
hint-non-interactive = Non-interactive terminal detected — running in quick mode
hint-non-interactive-wizard = For the interactive wizard, run: librefang init (in a terminal)
hint-starting-chat = Starting chat session...
hint-no-api-keys = No LLM provider API keys found
hint-groq-free = Groq offers a free tier: https://console.groq.com
hint-ollama-local = Or install Ollama for local models: https://ollama.com
hint-could-not-open-browser = Could not open a browser automatically.
hint-could-not-open-browser-visit = Could not open browser. Visit: { $url }
hint-dashboard-url = Dashboard: { $url }
hint-try-dashboard = Try: librefang dashboard
hint-install-desktop = Install it with: cargo install librefang-desktop
hint-fallback-web-dashboard = Falling back to web dashboard...
hint-then-open-dashboard = Then open: http://127.0.0.1:4545
hint-chat-with-agent = Chat: librefang chat { $name }
hint-agent-lost-on-exit = Note: Agent will be lost when this process exits
hint-persistent-agents = For persistent agents, use `librefang start` first
hint-url-copied = URL copied to clipboard
hint-doctor-repair = Run `librefang doctor --repair` to attempt auto-fix
hint-run-init = Run `librefang init` to set up the agents directory
hint-run-start = Run `librefang start` to launch the daemon
hint-config-edit = Fix with: librefang config edit
hint-set-key = Or run: librefang config set-key groq
hint-set-key-provider = Set later: librefang config set-key email (or export EMAIL_PASSWORD=...)

# --- Init ---
init-quick-success = LibreFang initialized (quick mode)
init-interactive-success = LibreFang initialized!
init-cancelled = Setup cancelled.
init-next-start = Start the daemon:  librefang start
init-next-chat = Chat:              librefang chat

# --- Error messages ---
error-home-dir = Could not determine home directory
error-create-dir = Failed to create { $path }
error-create-dir-fix = Check permissions on { $path }
error-write-config = Failed to write config
error-config-created = Created: { $path }
error-config-exists = Config already exists: { $path }

# --- Daemon communication errors ---
error-daemon-returned = Daemon returned error ({ $status })
error-daemon-returned-fix = Check daemon logs with: librefang logs --follow
error-request-timeout = Request timed out
error-request-timeout-fix = The agent may be processing a complex request. Try again, or check `librefang status`
error-connect-refused = Cannot connect to daemon
error-connect-refused-fix = Is the daemon running? Start it with: librefang start
error-daemon-comm = Daemon communication error: { $error }
error-daemon-comm-fix = Check `librefang status` or restart: librefang start

# --- Boot errors ---
error-boot-config = Failed to parse configuration
error-boot-config-fix = Check your config.toml syntax: librefang config show
error-boot-db = Database error (file may be locked)
error-boot-db-fix = Check if another LibreFang process is running: librefang status
error-boot-auth = LLM provider authentication failed
error-boot-auth-fix = Run `librefang doctor` to check your API key configuration
error-boot-generic = Failed to boot kernel: { $error }
error-boot-generic-fix = Run `librefang doctor` to diagnose the issue

# --- Require daemon ---
error-require-daemon = `librefang { $command }` requires a running daemon
error-require-daemon-fix = Start the daemon: librefang start

# --- Provider detection ---
detected-provider = Detected { $display } ({ $env_var })
detected-gemini = Detected Gemini (GOOGLE_API_KEY)
detected-ollama = Detected Ollama running locally (no API key needed)

# --- Desktop app ---
desktop-launching = Launching LibreFang Desktop...
desktop-started = Desktop app started.
desktop-launch-fail = Failed to launch desktop app: { $error }
desktop-not-found = Desktop app not found.

# --- Dashboard ---
dashboard-opening = Opening dashboard at { $url }

# --- Agent commands ---
agent-spawned = Agent '{ $name }' spawned
agent-spawned-inprocess = Agent '{ $name }' spawned (in-process)
agent-spawn-failed = Failed to spawn: { $error }
agent-spawn-agent-failed = Failed to spawn agent: { $error }
agent-template-not-found = Template '{ $name }' not found
agent-template-not-found-fix = Run `librefang agent new` to see available templates
agent-no-templates = No agent templates found
agent-no-templates-fix = Run `librefang init` to set up the agents directory
agent-template-parse-fail = Failed to parse template '{ $name }': { $error }
agent-template-parse-fail-fix = The template manifest may be corrupted
agent-killed = Agent { $id } killed.
agent-kill-failed = Failed to kill agent: { $error }
agent-invalid-id = Invalid agent ID: { $id }
agent-model-set = Agent { $id } model set to { $value }.
agent-set-model-failed = Failed to set model: { $error }
agent-no-daemon-for-set = No running daemon found. Start one with: librefang start
agent-unknown-field = Unknown field: { $field }. Supported fields: model
agent-no-agents = No agents running.
agent-spawn-success = Agent spawned successfully!
agent-spawn-inprocess-mode = Agent spawned (in-process mode).
agent-note-lost = Note: Agent will be lost when this process exits.
agent-note-persistent = For persistent agents, use `librefang start` first.
section-agent-templates = Available Agent Templates

# --- Manifest errors ---
manifest-not-found = Manifest file not found: { $path }
manifest-not-found-fix = Use `librefang agent new` to spawn from a template instead
error-reading-manifest = Error reading manifest: { $error }
error-parsing-manifest = Error parsing manifest: { $error }

# --- Status ---
section-daemon-status = LibreFang Daemon Status
section-status-inprocess = LibreFang Status (in-process)
section-active-agents = Active Agents
section-persisted-agents = Persisted Agents
label-daemon-not-running = NOT RUNNING

# --- Doctor ---
doctor-title = LibreFang Doctor
doctor-all-passed = All checks passed! LibreFang is ready.
doctor-repairs-applied = Repairs applied. Re-run `librefang doctor` to verify.
doctor-some-failed = Some checks failed.
doctor-no-api-keys = No LLM provider API keys found!
section-getting-api-key = Getting an API key (free tiers)

# --- Security ---
section-security-status = Security Status
label-audit-trail = Audit trail
label-taint-tracking = Taint tracking
label-wasm-sandbox = WASM sandbox
label-wire-protocol = Wire protocol
label-api-keys = API keys
label-manifests = Manifests
value-audit-trail = Merkle hash chain (SHA-256)
value-taint-tracking = Information flow labels
value-wasm-sandbox = Dual metering (fuel + epoch)
value-wire-protocol = OFP HMAC-SHA256 mutual auth
value-api-keys = Zeroizing<String> (auto-wipe on drop)
value-manifests = Ed25519 signed
audit-verified = Audit trail integrity verified (Merkle chain valid).
audit-failed = Audit trail integrity check FAILED.

# --- Health ---
health-ok = Daemon is healthy
health-not-running = Daemon is not running.

# --- Channel setup ---
section-channel-setup = Channel Setup
channel-configured = { $name } configured
channel-no-token = No token provided. Setup cancelled.
channel-no-email = No email provided. Setup cancelled.
channel-token-saved = Token saved to ~/.librefang/.env
channel-app-token-saved = App token saved to ~/.librefang/.env
channel-bot-token-saved = Bot token saved to ~/.librefang/.env
channel-password-saved = Password saved to ~/.librefang/.env
channel-phone-saved = Phone saved to ~/.librefang/.env
channel-key-saved = { $key } saved to ~/.librefang/.env
channel-unknown = Unknown channel: { $name }
channel-unknown-fix = Available: telegram, discord, slack, whatsapp, email, signal, matrix
channel-test-ok = Channel test passed
channel-test-fail = Channel test failed
section-setup-telegram = Setting up Telegram
section-setup-discord = Setting up Discord
section-setup-slack = Setting up Slack
section-setup-whatsapp = Setting up WhatsApp
section-setup-email = Setting up Email
section-setup-signal = Setting up Signal
section-setup-matrix = Setting up Matrix

# --- Vault ---
vault-initialized = Credential vault initialized.
vault-not-initialized = Vault not initialized.
vault-not-init-run = Vault not initialized. Run: librefang vault init
vault-unlock-failed = Could not unlock vault: { $error }
vault-empty-value = Empty value — not stored.
vault-stored = Stored '{ $key }' in vault.
vault-store-failed = Failed to store: { $error }
vault-removed = Removed '{ $key }' from vault.
vault-key-not-found = Key '{ $key }' not found in vault.
vault-remove-failed = Failed to remove: { $error }

# --- Cron ---
cron-created = Cron job created: { $id }
cron-create-failed = Failed to create cron job: { $error }
cron-deleted = Cron job { $id } deleted.
cron-delete-failed = Failed to delete cron job: { $error }
cron-toggled = Cron job { $id } { $action }d.
cron-toggle-failed = Failed to { $action } cron job: { $error }

# --- Approvals ---
approval-responded = Approval { $id } { $action }d.
approval-failed = Failed to { $action } approval: { $error }

# --- Memory ---
memory-set = Set { $key } for agent '{ $agent }'.
memory-set-failed = Failed to set memory: { $error }
memory-deleted = Deleted key '{ $key }' for agent '{ $agent }'.
memory-delete-failed = Failed to delete memory: { $error }

# --- Devices ---
section-device-pairing = Device Pairing
device-scan-qr = Scan this QR code with the LibreFang mobile app:
device-removed = Device { $id } removed.
device-remove-failed = Failed to remove device: { $error }

# --- Webhooks ---
webhook-created = Webhook created: { $id }
webhook-create-failed = Failed to create webhook: { $error }
webhook-deleted = Webhook { $id } deleted.
webhook-delete-failed = Failed to delete webhook: { $error }
webhook-test-ok = Webhook { $id } test payload sent successfully.
webhook-test-failed = Failed to test webhook: { $error }

# --- Models ---
model-set-success = Default model set to: { $model }
model-set-failed = Failed to set model: { $error }
model-no-catalog = No models in catalog.
section-select-model = Select a model
model-out-of-range = Number out of range (1-{ $max })

# --- Config ---
config-set-success = Config value set.
config-unset-success = Config key removed.
config-no-file = No config file found
config-no-file-fix = Run `librefang init` first
config-read-failed = Failed to read config: { $error }
config-parse-error = Config parse error: { $error }
config-parse-fix = Fix your config.toml syntax, or run `librefang config edit`
config-parse-fix-alt = Fix your config.toml syntax first
config-key-not-found = Key not found: { $key }
config-key-path-not-found = Key path not found: { $key }
config-empty-key = Empty key
config-section-not-scalar = '{ $key }' is a section, not a scalar
config-section-not-scalar-fix = Use dotted notation: { $key }.field_name
config-parent-not-table = Parent of '{ $key }' is not a table
config-serialize-failed = Failed to serialize config: { $error }
config-write-failed = Failed to write config: { $error }
config-set-kv = Set { $key } = { $value }
config-removed-key = Removed key: { $key }
config-no-key = No key provided. Cancelled.
config-saved-key = Saved { $env_var } to ~/.librefang/.env
config-save-key-failed = Failed to save key: { $error }
config-removed-env = Removed { $env_var } from ~/.librefang/.env
config-remove-key-failed = Failed to remove key: { $error }
config-env-not-set = { $env_var } not set
config-set-key-hint = Set it: librefang config set-key { $provider }
config-update-key-hint = Update key: librefang config set-key { $provider }

# --- Hand commands ---
hand-install-deps-success = Dependencies installed for hand '{ $id }'.
hand-paused = Hand instance '{ $id }' paused.
hand-resumed = Hand instance '{ $id }' resumed.

# --- Daemon notify ---
daemon-restart-notify = Restart the daemon to apply: librefang restart

# --- System info ---
section-system-info = LibreFang System Info

# --- Uninstall ---
uninstall-goodbye = LibreFang has been uninstalled. Goodbye!
uninstall-cancelled = Cancelled.
uninstall-stopping-daemon = Stopping running daemon...
uninstall-removed = Removed { $path }
uninstall-remove-failed = Failed to remove { $path }: { $error }
uninstall-removed-data-kept = Removed data (kept config files)
uninstall-removed-autostart-win = Removed Windows auto-start registry entry
uninstall-removed-launch-agent = Removed macOS launch agent
uninstall-remove-launch-fail = Failed to remove launch agent: { $error }
uninstall-removed-autostart-linux = Removed Linux autostart entry
uninstall-remove-autostart-fail = Failed to remove autostart entry: { $error }
uninstall-removed-systemd = Removed systemd user service
uninstall-remove-systemd-fail = Failed to remove systemd service: { $error }
uninstall-cleaned-path = Cleaned PATH from { $path }
uninstall-cleaned-path-win = Cleaned PATH from Windows user environment

# --- Reset ---
reset-success = Removed { $path }
reset-fail = Failed to remove { $path }: { $error }

# --- Logs ---
log-following = --- Following { $path } (Ctrl+C to stop) ---
log-path-hint = Log file: { $path }
