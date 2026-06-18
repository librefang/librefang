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
# Issue #4693 — after `curl install.sh | sh` upgrades the binary without
# restarting the running daemon, `librefang restart` (new CLI) hits the old
# daemon's `/api/shutdown` and is rejected with 401 because the new CLI's
# Authorization header does not match the old daemon's expected key (typical
# trigger: locked vault, rotated `[api] api_key`, or freshly enabled
# dashboard credentials). Surface the cause + auto-fall-back to PID-based
# shutdown so users can move forward without hand-editing config.
shutdown-401-detected = Shutdown request was rejected by the running daemon (401 Unauthorized).
shutdown-401-explainer = The new CLI cannot authenticate against the daemon that is currently running. This usually happens after `curl install.sh | sh` upgrades the binary without restarting the daemon — the running daemon was started with a different api_key, or the vault that holds it could not be unlocked.
shutdown-401-fallback-attempt = Falling back to a PID-based stop (PID { $pid })...
shutdown-401-fallback-success = Daemon stopped via PID { $pid }
shutdown-401-fallback-fail = PID-based stop did not work either.
shutdown-401-fallback-fix = Stop the daemon manually, then start it again:
    kill { $pid }    # or: kill -9 { $pid } if it does not exit
    librefang start
shutdown-401-no-pid-fix = Could not read the daemon PID from { $path }. Run `ps -ef | grep librefang` to find it, then `kill <pid>` and `librefang start`.

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
label-yes = yes
label-not-loaded = not loaded
label-current = Current
label-channel = Channel
label-binary = Binary
label-latest = Latest
label-target = Target
label-installed = Installed

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
hint-gemini-free = Gemini offers a free tier: https://aistudio.google.com
hint-deepseek-free = DeepSeek offers 5M free tokens: https://platform.deepseek.com
guide-title = Quick Setup
guide-free-providers-title = Pick a free provider to get started (2 min setup):
guide-get-free-key = Get your free API key
guide-paste-key-placeholder = paste your API key here
guide-setting-up = Setting up
guide-testing-key = Testing key...
guide-key-verified = ✓ Key verified!
guide-test-key-unverified = ⚠ Could not verify (may still work)
guide-help-select = ↑↓ navigate  Enter select  s/Esc skip
guide-help-paste = Paste key + Enter  Esc back
guide-help-wait = Please wait...
guide-paste-key-hint = Copy the API key from the browser and paste it below.
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
label-home = Home
label-platform = Platform
label-sessions = Sessions
label-memory = Memory
label-started = Started
label-response = Response
label-checks = Checks
section-status-locked = Restricted (requires API key)
hint-status-locked = Set `api_key` in ~/.librefang/config.toml to see agents / sessions / memory.
warn-public-bind = publicly bound
warn-key-missing = not set
section-recent-errors = Recent errors (daemon.log)
section-verbose = Details
label-auth = Auth
label-mcp = MCP servers
label-peers = OFP peers
label-channels = Channels
label-skills = Skills
label-hands = Hands
label-config-warnings = Config warnings
auth-none = none (anonymous)
auth-api-key = API key
auth-dashboard-login = dashboard login
auth-user-keys = { $count } user key(s)

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
channel-unknown-fix = Available: discord, slack, whatsapp, email, signal, matrix
channel-test-ok = Channel test passed
channel-test-fail = Channel test failed
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
vault-rotate-no-vault = No vault file found. Run `librefang vault init` first.
vault-rotate-old-key-missing = LIBREFANG_VAULT_KEY_OLD not set. Provide the current master key (base64 of 32 bytes) before rotating.
vault-rotate-new-key-missing = LIBREFANG_VAULT_KEY_NEW not set. Provide the new master key (base64 of 32 bytes), or pass --from-stdin to read it from stdin.
vault-rotate-stdin-read-failed = Failed to read new key from stdin: { $error }
vault-rotate-stdin-empty = New key read from stdin was empty.
vault-rotate-same-key = LIBREFANG_VAULT_KEY_OLD and the new key are identical — refusing to rotate to the same key.
vault-rotate-old-key-invalid = LIBREFANG_VAULT_KEY_OLD is not a valid 32-byte base64 key: { $error }
vault-rotate-new-key-invalid = New key is not a valid 32-byte base64 key: { $error }
vault-rotate-unlock-failed = Failed to unlock vault with the OLD key: { $error }. Check LIBREFANG_VAULT_KEY_OLD matches the key the vault was originally encrypted with.
vault-rotate-sentinel-failed = Vault sentinel verification failed under the OLD key: { $error }
vault-rotate-rewrap-failed = Failed to re-encrypt vault under the new key: { $error }. The original vault file is unchanged.
vault-rotate-success = Vault re-encrypted under the new master key ({ $count } user entries preserved).
vault-rotate-next-step = Next: set LIBREFANG_VAULT_KEY to the new value before restarting the daemon.

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

# --- Extracted from Rust sources ---
init-upgrade-existing = Existing installation detected — running upgrade to preserve your settings.
init-upgrade-fresh-hint = To start fresh, remove ~/.librefang/config.toml and run `librefang init` again.
init-upgrade-no-config = Nothing to upgrade — no config.toml found. Run `librefang init` first.
init-upgrade-registry-synced = Registry synced
init-upgrade-registry-failed = Registry sync failed (network issue?) — continuing with cached content
init-upgrade-config-up-to-date = Config is already up to date — no new fields added
init-upgrade-sections-added = Added { $count } new config section(s):
init-upgrade-legacy-openclaw = Legacy ~/.openclaw installation detected.
init-upgrade-legacy-openclaw-hint = Run `librefang migrate --from openclaw` to migrate your data.
init-upgrade-approval-warning = Your require_approval list only contains "shell_exec". File operations (file_write, file_delete) now require approval by default.
init-upgrade-approval-hint = To enable: add "file_write" and "file_delete" to require_approval in config.toml
init-upgrade-success-summary = Upgrade complete!
init-upgrade-title = Upgrading LibreFang installation
init-upgrade-progress-label = Upgrading
init-upgrade-backing-up = Backing up config
init-upgrade-backup-success = Backed up config to backups/{ $name }
init-upgrade-syncing-registry = Syncing registry
init-upgrade-initializing-vault-git = Initialising vault/git
init-upgrade-merging-config = Merging config fields
init-upgrade-failed-read = Upgrade aborted: failed to read config.toml: { $error }
init-upgrade-failed-parse = Upgrade aborted: failed to parse config.toml: { $error }
init-upgrade-backup-saved-hint = Your original config was saved to backups/{ $name }
init-upgrade-failed-parse-template = Upgrade aborted: failed to parse default config template: { $error }
init-upgrade-failed-write = Upgrade aborted: failed to write config: { $error }
init-upgrade-steps-complete = Upgrade steps complete
label-backup = Backup
label-new-fields = New fields

auth-chatgpt-device-requested = Device authentication requested.
auth-chatgpt-device-open-url = Open this URL in any browser:\n  { $url }\n
auth-chatgpt-device-one-time-code = Enter this one-time code:\n  { $code }\n
auth-chatgpt-device-do-not-share = Do not share this code.
auth-chatgpt-device-waiting = Waiting for authorization...
auth-chatgpt-switching-browser = \nSwitching to the standard browser login flow...\n
auth-chatgpt-opening-browser = Opening browser for OpenAI authentication...
auth-chatgpt-open-manually-hint = If the browser does not open, visit:\n  { $url }\n
auth-chatgpt-open-browser-failed = Could not open browser automatically: { $error }
auth-chatgpt-open-manually = Please open manually: { $url }
auth-chatgpt-tokens-saved = \nChatGPT tokens saved to { $path }
auth-chatgpt-detecting-model = Detecting best available model...
auth-chatgpt-selected-model = Selected model: { $model }
auth-chatgpt-config-updated = config.toml updated: provider = "chatgpt", model = "{ $model }"
auth-chatgpt-starting-flow = Starting ChatGPT authentication flow...\n
auth-chatgpt-complete = ChatGPT authentication complete.
auth-chatgpt-failed = ChatGPT authentication failed: { $error }

auth-pool-config-not-array = config.toml `credential_pools` exists but is not an array of tables
auth-pool-daemon-error-fallback = Daemon returned HTTP { $status } — falling back to config.toml view
auth-pool-daemon-connect-fallback = Failed to query daemon at { $url }: { $error } — falling back to config.toml view
auth-pool-no-config-offline = No config at { $path } and daemon is not running.
auth-pool-config-load-failed = Failed to load config: { $error }
auth-pool-none-configured = No credential pools configured.
auth-pool-invalid-env-name = `{ $env_var }` is not a valid env var name. Expected uppercase letters, digits, and underscores (e.g. OPENAI_API_KEY_2).
auth-pool-env-empty = env var `{ $env_var }` is set but empty.
auth-pool-env-empty-fix = Set it to your API key before adding the pool entry, e.g.\n  export { $env_var }=sk-…\nThen retry.
auth-pool-env-not-set = env var `{ $env_var }` is not set in the current shell.
auth-pool-env-not-set-fix = Export it before adding the pool entry, e.g.\n  export { $env_var }=sk-…\nThen retry. (The daemon will read it from its own environment at boot time — make sure it's exported there too.)
auth-pool-keys-not-array = Pool for `{ $provider }` has a `keys` field that is not an array of tables.
auth-pool-key-duplicate = Key with env_var `{ $env_var }` already exists in pool for provider `{ $provider }`.
auth-pool-key-added = Added key `{ $label }` (env={ $env_var }, priority={ $priority }) to pool for `{ $provider }`. Restart the daemon or hot-reload config to apply.
auth-pool-not-configured = No credential pool configured for provider `{ $provider }`.
auth-pool-no-keys-field = Pool for `{ $provider }` has no keys array.
auth-pool-key-not-found = No key with env_var `{ $env_var }` found in pool for `{ $provider }`.
auth-pool-key-removed-pool-empty = Removed key `{ $env_var }` from pool for `{ $provider }`. Pool is now empty and has been removed entirely. Restart the daemon or hot-reload config to apply.
auth-pool-key-removed = Removed key `{ $env_var }` from pool for `{ $provider }`. Restart the daemon or hot-reload config to apply.
auth-pool-unknown-strategy = Unknown strategy `{ $strategy }`. Valid: fill_first, round_robin, random, least_used.
auth-pool-strategy-set = Set pool strategy for `{ $provider }` to `{ $strategy }`. Restart the daemon or hot-reload config to apply.
vault-empty = Vault is empty.
vault-stored-count = Stored credentials ({ $count }):

# --- Scanned & Extracted keys ---
# init.rs
init-upgrade-failed-create-backups-dir = Failed to create backups dir: { $error }
init-upgrade-failed-backup-config = Failed to backup config: { $error }
init-error-write-config-example = Could not write config.example.toml: { $error }

# auth.rs
auth-write-failed = Failed to write { $path }: { $error }
auth-password-empty = Password cannot be empty.
auth-passwords-mismatch = Passwords do not match.
auth-password-hash-failed = Failed to hash password: { $error }
vault-enter-value-prompt = Enter value for { $key }: 
auth-enter-password-prompt = Enter password: 
auth-confirm-password-prompt = Confirm password: 

# agent.rs
agent-spawn-choose-target-or-template = Choose either a positional target or `--template`, not both.
agent-spawn-choose-target-or-template-fix = Use `librefang spawn coder` or `librefang spawn --template agents/custom/my-agent.toml`.
agent-spawn-name-requires-template = `--name` requires a template name or manifest path.
agent-spawn-name-requires-template-fix = Use `librefang spawn coder --name backend-coder` or `librefang spawn --template path/to/agent.toml --name backend-coder`.
agent-spawn-dry-run-requires-template = Dry run needs a template name or manifest path.
agent-spawn-dry-run-requires-template-fix = Use `librefang spawn coder --dry-run` or `librefang spawn --template path/to/agent.toml --dry-run`.
agent-spawn-template-or-path-not-found = Template or manifest path not found: { $target }
agent-spawn-template-or-path-not-found-fix = Run `librefang agent new` to browse templates, or pass a valid manifest path.
agent-manifest-parse-failed = Failed to parse agent manifest from { $source }: { $error }
agent-manifest-parse-failed-fix = Check the manifest TOML syntax and required fields.
agent-manifest-serialize-failed = Failed to serialize updated manifest: { $error }
agent-dry-run-title = Agent Dry Run
agent-dry-run-success = Manifest parsed successfully. No agent was spawned.
agent-delete-warning-text = WARNING: Deleting agent "{ $name }" will permanently remove its canonical UUID
    and all associated memories and sessions.
    This action cannot be undone.
label-confirm-prompt = Confirm?
label-aborted = Aborted.
agent-delete-no-uuid = No canonical UUID recorded for agent name '{ $name }' — nothing to delete.
agent-deleted-success = Agent "{ $name }" deleted (canonical UUID purged).
agent-delete-failed-with-reason = Failed to delete agent: { $error }
agent-reset-uuid-warning-text = WARNING: Resetting the canonical UUID for "{ $name }" will orphan all sessions
    and memories tied to its current UUID. The next spawn under this
    name will start with a fresh UUID. This action cannot be undone.
agent-reset-uuid-success = Canonical UUID for "{ $name }" reset (was { $previous }).
agent-reset-uuid-failed-with-reason = Failed to reset canonical UUID: { $error }
agent-reset-uuid-not-found = No canonical UUID recorded for agent name '{ $name }'.
agent-merge-history-not-implemented = merge-history is not yet implemented (refs #4614 follow-up).
    Reassignment of sessions / memories from { $from } to the canonical UUID
    for agent "{ $name }" requires cross-table SQL surgery in the memory
    substrate that is being tracked separately.
agent-set-model-success = Agent { $id } model set to { $value }.
agent-set-model-failed-with-reason = Failed to set model: { $error }
agent-set-no-daemon = No running daemon found. Start one with: librefang start
agent-set-unknown-field = Unknown field: { $field }. Supported fields: model
agent-new-no-templates = No agent templates found
agent-new-no-templates-fix = Run `librefang init` to set up the agents directory
agent-new-template-not-found = Template '{ $name }' not found
agent-new-template-not-found-fix = Run `librefang agent new` to see available templates
agent-new-choose-template-prompt =   Choose template [1]: 
agent-sessions-none-active = No active sessions.
agent-sessions-none-found = No sessions found.

label-source = Source
label-name = Name
label-module = Module
label-tools = Tools
label-tags = Tags
label-description = Description

# daemon.rs
daemon-first-run-setup = First run detected — running quick setup...
daemon-config-not-found = Config file not found: { $path }
daemon-config-not-found-fix = Run `librefang init` to create a default config at ~/.librefang/config.toml, or check the --config path.
daemon-log-file-not-found = Log file not found
daemon-log-file-not-found-fix = Expected at: { $path }
daemon-log-not-found-showing-tui = Daemon log not found; showing TUI log at { $path }

# hand.rs
hand-install-error-no-toml = Error: No HAND.toml found in { $path }
hand-install-error-read-toml = Error reading { $path }: { $error }
hand-error-prefix = Error: { $error }
hand-installed-success = Installed hand: { $name } ({ $id })
hand-activate-hint = Use `librefang hand activate { $id }` to start it.
hand-none-available = No hands available.
hand-list-activate-hint =
    Use `librefang hand activate <id>` to activate a hand.
hand-none-active = No active hands.
label-hand = Hand
label-instance = Instance
label-agent = Agent
hand-status-title = Hand Status
label-status-inactive = inactive
hand-not-found = No active hand or installed hand found for '{ $id }'.
hand-activated-success = Hand '{ $id }' activated (instance: { $instance }, agent: { $agent })
hand-activate-failed = Failed to activate hand '{ $id }': { $error }
hand-deactivated-success = Hand '{ $id }' deactivated.
label-failed-reason = Failed: { $error }
hand-no-active-instance = No active hand instance found for '{ $id }'.
hand-info-not-found = Hand not found: { $error }
hand-no-settings = Hand '{ $id }' has no configurable settings.
hand-settings-title = Settings for '{ $id }'
hand-set-setting-success = Set { $key }={ $value } for hand '{ $id }'.
hand-reloaded-summary = Reloaded hands: { $added } added, { $updated } updated, { $total } total.
hand-chat-welcome = Chat with { $name } (type /quit to exit)

# mcp_cmds.rs
mcp-catalog-unknown-entry = Unknown MCP catalog entry: '{ $name }'
mcp-catalog-available-header =
    Available MCP servers (catalog):
mcp-failed-read-config = Failed to read { $path }: { $error }
mcp-invalid-toml = { $path } is not valid TOML: { $error }
mcp-already-configured = MCP server '{ $name }' is already configured. Run `librefang mcp remove { $name }` first if you want to re-install.
mcp-failed-write-config = Failed to write config.toml: { $error }
mcp-add-credentials-hint =
    To add credentials:
mcp-get-it-here =   Get it here: { $url }
mcp-not-configured = MCP server '{ $name }' is not configured
mcp-failed-update-config = Failed to update config.toml: { $error }
mcp-removed-success = { $name } removed.
mcp-catalog-no-matches = No MCP catalog entries matching '{ $query }'.
mcp-catalog-none-available = No MCP catalog entries available.
mcp-catalog-summary =   { $total } catalog entries ({ $installed } installed)
mcp-catalog-install-hint =   Use `librefang mcp add <id>` to install an MCP server.
mcp-none-configured = No MCP servers configured.
mcp-list-catalog-hint =   Use `librefang mcp catalog` to list installable entries.

# monitoring.rs
monitoring-audit-reset-destructive = audit reset is destructive — re-run with `--confirm` to proceed
monitoring-db-not-found = database not found at { $path }
monitoring-db-open-failed = failed to open { $path }: { $error }
monitoring-db-truncate-failed = failed to truncate audit_entries: { $error }
monitoring-audit-reset-anchor-deleted = , deleted anchor at { $path }
monitoring-audit-reset-anchor-none =  (no anchor file to remove)
monitoring-audit-reset-success = Audit trail reset: removed { $count } row(s) from audit_entries{ $anchor_detail }.
monitoring-audit-reset-would-header =   Would:
monitoring-audit-reset-would-delete =     1. DELETE all rows from `audit_entries` in { $path }
monitoring-audit-reset-would-remove-anchor =     2. Remove anchor file { $path }
monitoring-audit-reset-would-restart =   The Merkle chain will restart from the next audit event.
monitoring-daemon-running-error = daemon is running at { $url }; refusing to touch the audit database
monitoring-daemon-running-error-fix = stop the daemon first: `librefang stop`
monitoring-anchor-remove-failed = failed to remove anchor { $path }: { $error }
monitoring-audit-reset-seed-fresh = The next daemon boot will seed a fresh Merkle chain from the current tip.
monitoring-memory-no-entries = No memory entries for agent '{ $agent }'.
monitoring-devices-none-paired = No paired devices.
monitoring-webhooks-none-configured = No webhooks configured.

# skill.rs
skill-install-progress = Installing { $source }

# system.rs
migrate-error-home-dir = Error: Could not determine home directory
migrate-start-msg = Migrating from { $source } ({ $path })...
migrate-dry-run-hint =   (dry run — no changes will be made)
migrate-progress-label = Running migration
migrate-complete-msg = Migration complete
migrate-warn-report-save-failed = Warning: Could not save migration report: { $error }
migrate-report-saved =
      Report saved to: { $path }
migrate-failed-msg = Migration failed: { $error }

# maintenance.rs
maintenance-service-install-root-error = Running as root — the service will be installed for the root account, not your user. Run without sudo instead.
maintenance-service-unsupported = Auto-start service is not supported on this platform.
maintenance-failed-create-dir = Failed to create { $path }: { $error }
maintenance-failed-write-file = Failed to write { $path }: { $error }
maintenance-wrote-file = Wrote { $path }
maintenance-systemctl-reload-failed = systemctl --user daemon-reload failed
maintenance-service-enabled = Service enabled (will start on next login)
maintenance-service-start-hint = Start now with: systemctl --user start librefang.service
maintenance-service-linger-hint = For headless servers, also run: loginctl enable-linger
maintenance-systemctl-enable-failed = systemctl --user enable librefang.service failed
maintenance-launchagent-loaded = LaunchAgent loaded (will start on login and now)
maintenance-launchctl-load-failed = launchctl load failed: { $error }
maintenance-launchctl-run-failed = Failed to run launchctl: { $error }
maintenance-windows-startup-added = Added to Windows startup (HKCU\Software\Microsoft\Windows\CurrentVersion\Run)
maintenance-windows-registry-write-failed = Failed to write registry: { $error }
maintenance-windows-reg-run-failed = Failed to run reg.exe: { $error }
maintenance-systemd-removed = Removed systemd user service
maintenance-systemd-remove-failed = Failed to remove service file: { $error }
maintenance-systemd-not-found = No systemd user service found — nothing to remove.
maintenance-launchagent-removed = Removed LaunchAgent
maintenance-launchagent-remove-failed = Failed to remove plist: { $error }
maintenance-launchagent-not-found = No LaunchAgent found — nothing to remove.
maintenance-windows-startup-removed = Removed from Windows startup
maintenance-windows-startup-not-found = No startup entry found — nothing to remove.
maintenance-systemd-status-registered = Systemd user service is registered
maintenance-status-label-enabled =   Enabled
maintenance-status-label-active =   Active
maintenance-systemd-status-not-registered = No systemd user service registered.
maintenance-service-install-hint = Run `librefang service install` to set it up.
maintenance-launchagent-status-registered = LaunchAgent is registered
maintenance-status-label-loaded =   Loaded
maintenance-launchagent-status-not-registered = No LaunchAgent registered.
maintenance-windows-status-registered = Windows startup entry is registered
maintenance-windows-status-not-registered = No startup entry registered.
reset-confirm-message =   This will delete all data in { $path }
      Including: config, database, agent manifests, credentials.
reset-confirm-prompt =   Are you sure? Type 'yes' to confirm: 
reset-not-needed = Nothing to reset — { $path } does not exist.
maintenance-update-section = Update
maintenance-update-error-exe-path = Cannot determine current executable path: { $error }
maintenance-update-error-check-release = Failed to check latest release: { $error }
maintenance-update-warn-resolve-release = Could not resolve the latest published release: { $error }
maintenance-update-warn-resolve-release-fix = Retry later, or pass `--version <tag>` to target a specific release.
maintenance-update-available = A newer published release is available: { $tag }
maintenance-update-run-hint = Run `librefang update` to install it.
maintenance-update-same-core = The published release { $tag } uses the same CLI version core as the current binary ({ $current }).
maintenance-update-same-core-hint = Run `librefang update` if you want the latest published build for this version line.
maintenance-update-ahead = Current binary version { $current } is ahead of the published release { $tag }.
maintenance-update-compare-unknown = Could not compare the current binary with release tag { $tag }.
maintenance-update-compare-unknown-hint = If you want that exact release, run `librefang update --version <tag>`.
maintenance-update-unable-to-determine = Unable to determine whether an update is available.
maintenance-update-unable-to-determine-hint = Retry later when GitHub Releases is reachable.
maintenance-update-cannot-compare-safely = Could not safely compare the current binary against release tag { $tag }.
maintenance-update-cannot-compare-safely-hint = Re-run with `librefang update --version { $tag }` to install it explicitly.
maintenance-update-windows-daemon-running-error = Stop the running daemon before updating on Windows.
maintenance-update-windows-daemon-running-error-fix = Run `librefang stop`, then `librefang update`, then `librefang start`.
maintenance-update-cli-success = LibreFang CLI updated.
maintenance-update-merging-config-defaults = Merging new config defaults...
maintenance-update-restart-daemon-hint = If the daemon is running, restart it with `librefang restart`.
maintenance-update-background-launched = Update launched in the background.
maintenance-update-background-hint-terminal = Open a new terminal after it finishes and run `librefang --version`.
maintenance-update-background-hint-restart = If the daemon is running, restart it after the update completes.
maintenance-update-failed-error = Update failed: { $error }
maintenance-update-cargo-blocked = This binary was installed with cargo. Running `cargo install` from inside the active executable is intentionally blocked.
maintenance-update-unofficial-path = Automatic update only supports the official install path ({ $path }). This binary is running from a different location.
maintenance-update-package-manager-hint = If this binary came from another package manager, update it with that package manager instead.

# doctor_cmd.rs
doctor-check-librefang-dir-ok = LibreFang directory: { $path }
doctor-check-librefang-dir-fail = LibreFang directory not found.
doctor-check-librefang-dir-created = Created LibreFang directory
doctor-check-librefang-dir-create-fail = Failed to create directory
doctor-check-librefang-dir-not-found-init = LibreFang directory not found. Run `librefang init` first.
doctor-check-env-ok = .env file (permissions OK)
doctor-check-env-fixed = .env file (permissions fixed to 0600)
doctor-check-env-ok-generic = .env file
doctor-check-env-loose-warn = .env file has loose permissions ({ $mode }), should be 0600
doctor-check-env-not-found-warn = .env file not found (create with: librefang config set-key <provider>)
doctor-check-config-ok = Config file: { $path }
doctor-check-config-syntax-fail = Config file has syntax errors: { $error }
doctor-check-config-not-found = Config file not found.
doctor-check-config-created = Created default config.toml
doctor-check-config-create-fail = Failed to create config.toml
doctor-check-cli-version = CLI version: { $version } (channel: { $channel })
doctor-check-update-available-warn = Update available: { $current } -> { $latest } (see https://github.com/librefang/librefang/releases)
doctor-check-cli-up-to-date = CLI is up to date
doctor-check-update-fail-warn = Could not check for updates (network unavailable)
doctor-check-daemon-running = Daemon running at { $url }
doctor-check-daemon-not-running-warn = Daemon not running (start with `librefang start`)
doctor-check-port-available = Port { $address } is available
doctor-check-port-in-use-warn = Port { $address } is in use by another process
doctor-check-stale-daemon-json-removed = Removed stale daemon.json
doctor-check-stale-daemon-json-warn = Stale daemon.json found (daemon not running). Run with --repair to clean up.
doctor-check-db-ok = Database file (valid SQLite)
doctor-check-db-invalid-fail = Database file exists but is not valid SQLite
doctor-check-db-not-found-warn = No database file (will be created on first run)
doctor-check-disk-space-low-warn = Low disk space: { $count }MB available
doctor-check-disk-space-ok = Disk space: { $count }MB available
doctor-check-manifests-ok = Agent manifests are valid
doctor-check-manifest-invalid-fail = Invalid manifest { $file }: { $error }
doctor-check-home-dir-fail = Could not determine home directory
doctor-check-provider-key-rejected-warn = { $name } ({ $env_var }) - key rejected (401/403)
doctor-check-endpoint-reachable = { $name } endpoint reachable ({ $endpoint })
doctor-check-endpoint-unreachable-warn = { $name } endpoint unreachable ({ $endpoint })
doctor-check-channel-token-format-warn = { $name } ({ $env_var }) - unexpected token format
doctor-check-config-env-missing-warn = Config references { $env_var } but it is not set in env or .env
doctor-check-config-deser-ok = Config deserializes into KernelConfig
doctor-check-exec-policy = Exec policy: mode={ $mode }, safe_bins={ $count }
doctor-check-include-file-ok = Include file: { $path }
doctor-check-include-file-missing-warn = Include file missing: { $path }
doctor-check-include-file-not-found-fail = Include file not found: { $path }
doctor-check-mcp-servers-count = MCP servers configured: { $count }
doctor-check-mcp-empty-command-warn = MCP server '{ $name }' has empty command
doctor-check-mcp-empty-url-warn = MCP server '{ $name }' has empty URL
doctor-check-mcp-empty-base-url-warn = MCP server '{ $name }' has empty base_url
doctor-check-mcp-no-compat-tools-warn = MCP server '{ $name }' has no http_compat tools configured
doctor-check-mcp-compat-header-empty-name-warn = MCP server '{ $name }' has an http_compat header with empty name
doctor-check-mcp-compat-header-no-value-warn = MCP server '{ $name }' has an http_compat header without value/value_env
doctor-check-mcp-compat-tool-empty-name-warn = MCP server '{ $name }' has an http_compat tool with empty name
doctor-check-mcp-compat-tool-empty-path-warn = MCP server '{ $name }' has an http_compat tool with empty path
doctor-check-config-deser-fail = Config fails KernelConfig deserialization: { $error }
doctor-check-skills-loaded = Skills loaded: { $count }
doctor-check-skills-load-fail-warn = Failed to load skills: { $error }
doctor-check-skills-injection-ok = All skills pass prompt injection scan
doctor-check-mcp-catalog-templates = MCP catalog templates: { $templates }
doctor-check-mcp-configured-servers = Configured MCP servers: { $configured }
doctor-check-running-agents = Running agents: { $count }
doctor-check-daemon-uptime = Daemon uptime: { $hours }h { $mins }m
doctor-check-db-connectivity-ok = Database connectivity: OK
doctor-check-db-status-fail = Database status: { $status }
doctor-check-health-detail-status-warn = Health detail returned { $status }
doctor-check-health-detail-fail-warn = Failed to query daemon health: { $error }
doctor-check-skills-loaded-daemon = Skills loaded in daemon: { $count }
doctor-check-rust-version = Rust: { $version }
doctor-check-rust-not-found-fail = Rust toolchain not found
doctor-check-python-version = Python: { $version }
doctor-check-python-not-found-warn = Python not found (needed for Python skill runtime)
doctor-check-node-version = Node.js: { $version }
doctor-check-node-not-found-warn = Node.js not found (needed for Node skill runtime)
doctor-prompt-create-dir =     Create it now? [Y/n] 
doctor-prompt-create-config =     Create default config? [Y/n] 
doctor-section-providers =   LLM Providers:
doctor-section-connectivity = 

  Network Connectivity:
doctor-section-channels = 

  Channel Integrations:
doctor-section-config-val = 

  Config Validation:
doctor-section-skills = 

  Skills:
doctor-check-skills-injection-critical-warn = Skill '{ $name }' has { $count } critical warning(s):
doctor-check-skills-injection-warn = Prompt injection warning in skill: { $name }
doctor-section-mcp-servers =
  MCP servers:
doctor-section-daemon-health =
  Daemon Health:
doctor-check-daemon-mcp-status = MCP servers: { $configured } configured, { $connected } connected
doctor-check-daemon-mcp-health = MCP server health: { $healthy }/{ $total } healthy

doctor-suggest-groq = https://console.groq.com       (free, fast)
doctor-suggest-gemini = https://aistudio.google.com    (free tier)
doctor-suggest-deepseek = https://platform.deepseek.com  (low cost)

desktop-install-launched = Desktop app launched.
desktop-install-launch-fail = Failed to launch { $path }: { $error }
desktop-install-launch-fail-generic = Failed to launch desktop app: { $error }
desktop-install-not-installed = LibreFang Desktop is not installed.
desktop-install-prompt =   Download and install it now? [Y/n] 
desktop-install-skipped = Skipped. You can install it later:
desktop-install-skipped-brew =   brew install --cask librefang   (macOS)
desktop-install-skipped-manual =   Or download from https://github.com/librefang/librefang/releases
desktop-install-fetching = Fetching latest release info...
desktop-install-unsupported = Unsupported platform for automatic desktop install.
desktop-install-download-manual = Download manually: https://github.com/librefang/librefang/releases
desktop-install-github-fail = Failed to reach GitHub: { $error }
desktop-install-parse-fail = Failed to parse release info: { $error }
desktop-install-kv-asset = Asset
desktop-install-downloading = Downloading...
desktop-install-download-fail = Download failed: { $error }
desktop-install-download-complete = Download complete.
desktop-install-installing = Installing...
desktop-install-success = LibreFang Desktop installed successfully.
desktop-install-fail = Installation failed: { $error }
desktop-install-running-installer = Running installer...

doctor-audit-vault-key-unset = LIBREFANG_VAULT_KEY not set — vault encryption disabled.
doctor-audit-vault-key-invalid-base64 = LIBREFANG_VAULT_KEY is not valid base64: { $error }
doctor-audit-vault-key-invalid-base64-hint = Generate one with: openssl rand -base64 32
doctor-audit-vault-key-wrong-length = LIBREFANG_VAULT_KEY decodes to { $count } bytes; must be exactly 32. Note that 32 ASCII characters is NOT 32 bytes after base64 decode.
doctor-audit-vault-key-wrong-length-hint = Generate a fresh 32-byte key: openssl rand -base64 32 (44-char output)
doctor-audit-vault-key-ok = LIBREFANG_VAULT_KEY decodes to 32 bytes.

doctor-audit-api-listen-no-config = config.toml not found — skipping api_listen check.
doctor-audit-api-listen-invalid-toml = config.toml is not valid TOML: { $error }
doctor-audit-api-listen-invalid-toml-hint = Edit ~/.librefang/config.toml or run `librefang doctor --repair`.
doctor-audit-api-listen-unset = api_listen not set in config — kernel will use the default.
doctor-audit-api-listen-invalid-addr = api_listen `{ $address }` is not a valid socket address: { $error }
doctor-audit-api-listen-invalid-addr-hint = Use `host:port` form, e.g. `127.0.0.1:4545` or `[::1]:4545`.
doctor-audit-api-listen-port-zero = api_listen `{ $address }` uses port 0 (OS-assigned ephemeral); clients can't discover the daemon URL after bind.
doctor-audit-api-listen-port-zero-hint = Pick an explicit port (default 4545), e.g. `127.0.0.1:4545`.
doctor-audit-api-listen-privileged = api_listen port { $port } is privileged (<1024); daemon will fail to bind without root.
doctor-audit-api-listen-privileged-hint = Use a port >= 1024 (default 4545) unless you intentionally need root.
doctor-audit-api-listen-ok = api_listen `{ $address }` parses cleanly.

doctor-audit-config-not-found = { $path } does not exist.
doctor-audit-config-not-found-hint = Run `librefang init` to create a default config.
doctor-audit-config-read-fail = Failed to read { $path }: { $error }
doctor-audit-config-ok = { $path } parses as TOML.
doctor-audit-config-syntax-error = { $path } has TOML syntax errors: { $error }
doctor-audit-config-syntax-error-hint = Edit { $path } or restore from a backup.

# launcher menu items
launcher-menu-get-started = Get started
launcher-menu-get-started-hint = Providers, API keys, models, migration
launcher-menu-settings = Settings
launcher-menu-settings-hint = Providers, API keys, models, routing
launcher-menu-chat = Chat with an agent
launcher-menu-chat-hint = Quick chat in the terminal
launcher-menu-dashboard = Open dashboard
launcher-menu-dashboard-hint = Launch the web UI in your browser
launcher-menu-desktop = Open desktop app
launcher-menu-desktop-hint = Launch the native desktop app
launcher-menu-tui = Launch terminal UI
launcher-menu-tui-hint = Full interactive TUI dashboard
launcher-menu-help = Show all commands
launcher-menu-help-hint = Print full --help output

# launcher screen strings
launcher-welcome = Welcome! Let's get you set up.
launcher-checking-daemon = Checking for daemon…
launcher-daemon-running = Daemon running at { $url }
launcher-daemon-no-running = No daemon running
launcher-provider = Provider: { $provider }
launcher-no-keys = No API keys detected
launcher-hint-re-run =   Run 'Re-run setup' to configure a provider
launcher-hint-get-started =   Select 'Get started' to configure
launcher-migration-question = Coming from { $source }? 
launcher-migration-hint = 'Get started' includes automatic migration.
launcher-menu-hints = ↑↓/jk navigate  1-9 quick select  enter confirm  q quit
launcher-help-title = All commands
launcher-help-subtitle =   — q/Esc to go back
launcher-help-hints = ↑↓/jk scroll  PgUp/PgDn  g/G top/bottom  q back

# CLI shared UI strings
ui-brand-tagline = The open-source agent operating system
ui-label-hint = hint:
ui-label-next-steps = Next steps
ui-label-fix = fix:
ui-label-try = try:
ui-provider-not-set = { $env_var } not set


