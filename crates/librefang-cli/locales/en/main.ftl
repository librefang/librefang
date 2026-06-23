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
label-no = no
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
hint-could-not-open-browser-visit = Could not open browser. Visit: { $url }
hint-chat-with-agent = Chat: librefang chat { $name }
hint-agent-lost-on-exit = Note: Agent will be lost when this process exits
hint-persistent-agents = For persistent agents, use `librefang start` first
hint-url-copied = URL copied to clipboard
hint-doctor-repair = Run `librefang doctor --repair` to attempt auto-fix
hint-run-start = Run `librefang start` to launch the daemon
hint-config-edit = Fix with: librefang config edit
hint-set-key = Or run: librefang config set-key groq

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
detected-ollama = Detected Ollama running locally (no API key needed)

# --- Desktop app ---

# --- Dashboard ---
dashboard-opening = Opening dashboard at { $url }

# --- Agent commands ---
agent-spawned = Agent '{ $name }' spawned
agent-spawned-inprocess = Agent '{ $name }' spawned (in-process)
agent-spawn-failed = Failed to spawn: { $error }
agent-spawn-agent-failed = Failed to spawn agent: { $error }
agent-killed = Agent { $id } killed.
agent-kill-failed = Failed to kill agent: { $error }
agent-invalid-id = Invalid agent ID: { $id }
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
label-running = running
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
channel-none-configured = No channels configured.
channel-use-setup-hint = Use `librefang channel setup` to add one.
channel-reloaded = Channels reloaded ({ $started } sidecar(s) started).
channel-registry-empty = Daemon's channel registry is empty.
channel-install-sdk-hint = Install the sidecar SDK so adapters appear in the catalog:
channel-install-sdk-cmd =   pip install librefang-sdk
channel-rerun-setup-hint = Then re-run `librefang channel setup`.
channel-all-configured = Every available channel is already configured.
channel-see-list-hint = Use `librefang channel list` to see them, or
channel-remove-entry-hint = `librefang channel rm <name>` to remove an entry first.
channel-pick-setup = Pick a channel to set up:
channel-choice-prompt = Choice [1]: 
channel-unknown-error = Unknown channel: { $name }
channel-unknown-error-fix = Run `librefang channel list` to see the available adapters.
channel-no-configurable-fields = `{ $name }` exposes no configurable fields — nothing to prompt for.
channel-hot-reload-manual-hint = (Hot-reload anyway with `librefang channel reload` if you've already edited config.toml by hand.)
channel-prompt-secret-keep =   { $label } ({ $key }) [set — leave blank to keep]: 
channel-prompt-default =   { $label } ({ $key }) [{ $current }]: 
channel-prompt-required =   { $label } ({ $key }) *: 
channel-prompt-optional =   { $label } ({ $key }): 
channel-save-rejected = Save for `{ $name }` rejected: { $error }
channel-save-rejected-fix = Re-run with corrected values, or check the daemon log for details.
channel-saved-restart-required = ✓ Saved `{ $name }` — restart the daemon for changes to apply.
channel-saved-hot-reload = ✓ Saved `{ $name }` — hot-reload applied.
channel-env-shadowing-warn = Warning: shell environment variables shadow these tokens — unset them and restart for the new value to take effect: { $keys }
channel-config-read-fail = Cannot read { $path }: { $error }
channel-config-read-fail-fix = Run `librefang init` to create the config file.
channel-config-parse-fail = Cannot parse { $path }: { $error }
channel-config-parse-fail-fix = Fix the TOML syntax and retry.
channel-no-entries-to-remove = No [[sidecar_channels]] entries in config.toml — nothing to remove.
channel-no-entry-with-name = No [[sidecar_channels]] entry with name="{ $name }".
channel-config-write-fail = Failed to write { $path }: { $error }
channel-config-write-fail-fix = Check filesystem permissions.
channel-removed-entries = ✓ Removed { $count } [[sidecar_channels]] entry/entries named `{ $name }`.
channel-hot-reloaded-daemon =   Hot-reloaded daemon.
channel-reload-status-warn =   Reload returned { $status }: change will apply on next daemon restart.
channel-reload-contact-fail-warn =   Could not contact daemon for reload ({ $error }); change will apply on next start.
channel-reload-daemon-offline =   Daemon not running; change will apply on next start.
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

# --- Automation ---
automation-workflow-none = No workflows registered.
automation-workflow-file-not-found = Workflow file not found: { $path }
automation-workflow-read-error = Error reading workflow file: { $error }
automation-workflow-invalid-json = Invalid JSON: { $error }
automation-workflow-created = Workflow created successfully!
automation-workflow-created-id =   ID: { $id }
automation-workflow-create-failed = Failed to create workflow: { $error }
automation-workflow-completed = Workflow completed!
automation-workflow-run-id =   Run ID: { $id }
automation-workflow-failed = Workflow failed: { $error }
automation-trigger-none = No triggers registered.
automation-trigger-invalid-pattern = Invalid pattern JSON: { $error }
automation-trigger-created = Trigger created successfully!
automation-trigger-created-id =   Trigger ID: { $id }
automation-trigger-created-agent =   Agent ID:   { $agent_id }
automation-trigger-created-target =   Target:     { $target }
automation-trigger-create-failed = Failed to create trigger: { $error }
automation-trigger-deleted = Trigger { $id } deleted.
automation-trigger-delete-failed = Failed to delete trigger: { $error }
automation-trigger-get-failed = Failed to get trigger: { $error }
automation-trigger-update-failed = Failed to update trigger: { $error }
automation-trigger-updated = Trigger { $id } updated.
automation-trigger-toggle-failed = Failed to { $action } trigger: { $error }
automation-trigger-toggled = Trigger { $id } { $action }d.
automation-trigger-info-id = Trigger ID:    { $id }
automation-trigger-info-agent = Agent ID:      { $id }
automation-trigger-info-pattern = Pattern:       { $pattern }
automation-trigger-info-prompt = Prompt:        { $prompt }
automation-trigger-info-enabled = Enabled:       { $enabled }
automation-trigger-info-fires = Fire count:    { $count }
automation-trigger-info-max-fires = Max fires:     { $count }
automation-trigger-info-target = Target agent:  { $agent }
automation-trigger-info-cooldown = Cooldown:      { $secs }s
automation-trigger-info-session = Session mode:  { $mode }
automation-unlimited = unlimited
automation-cron-none = No scheduled jobs.

label-header-steps = STEPS
label-header-trigger-id = TRIGGER ID
label-header-agent-id = AGENT ID
label-header-fires = FIRES
label-header-pattern = PATTERN
label-header-schedule = SCHEDULE
label-header-prompt = PROMPT

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
model-none-found = No models found.
model-prompt-selection =   Enter number or model ID: 


# --- Config ---
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
config-no-file-found = No configuration found at: { $path }
config-run-init-hint = Run `librefang init` to create one.
config-read-error = Error reading config: { $error }
config-editor-exit = Editor exited with: { $status }
config-editor-open-fail = Failed to open editor '{ $editor }': { $error }
config-editor-env-hint = Set $EDITOR to your preferred editor.
config-val-exceeds-i64 = value { $value } exceeds i64::MAX ({ $max }); TOML cannot store unsigned integers above this bound
config-invalid-integer = '{ $raw }' is not a valid integer
config-paste-api-key-prompt =   Paste your { $provider } API key: 
config-testing-key =   Testing key... 
config-testing-provider-key =   Testing { $provider } ({ $env_var })... 
config-test-ok = OK
config-test-failed = FAILED (401/403)
config-test-unverified = could not verify (may still work)


# --- Hand commands ---
hand-install-deps-success = Dependencies installed for hand '{ $id }'.
hand-paused = Hand instance '{ $label } (instance: { $instance_id })' paused.
hand-resumed = Hand instance '{ $label } (instance: { $instance_id })' resumed.

# --- Daemon notify ---

# --- System info ---
section-system-info = LibreFang System Info

# --- Uninstall ---
uninstall-warning = This will completely uninstall LibreFang from your system.
uninstall-remove-data-kept =   • Remove data in { $path } (keeping config files)
uninstall-remove-all =   • Remove { $path }
uninstall-remove-binary =   • Remove binary: { $path }
uninstall-remove-cargo-binary =   • Remove cargo binary: { $path }
uninstall-remove-autostart =   • Remove auto-start entries (if any)
uninstall-clean-path =   • Clean PATH from shell configs (if any)
uninstall-confirm-prompt =   Type 'uninstall' to confirm: 
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

# --- Extracted from Rust sources ---
init-error-create-data-dir = Error creating data dir: { $error }
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

# acp.rs
acp-attached-uds = librefang acp: attached to daemon (UDS { $path })
acp-attached-pipe = librefang acp: attached to daemon (named pipe)
acp-in-process = librefang acp: in-process kernel (no daemon detected)
acp-error-boot-kernel = Failed to boot kernel: { $error }
acp-error-resolve-agent = Failed to resolve agent '{ $name }': { $error }
acp-error-server = ACP server error: { $error }
acp-error-uds-connect = ACP UDS connect failed at { $path }: { $error }
acp-error-pipe-connect = ACP named-pipe connect failed at { $name }: { $error }


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
label-captured = Captured
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
label-chat-with = Chat with
hand-chat-quit-hint = (type /quit to exit)
hand-chat-prompt-you = you >
label-no-response = [no response]
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
mcp-vault-set-hint =   librefang vault set { $name }  # { $help }
mcp-header-name = name
mcp-header-template-id = template_id
mcp-header-transport = transport
mcp-header-details = details

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
launcher-daemon-agents = { $count ->
    [one]  ({ $count } agent)
   *[other]  ({ $count } agents)
}
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
common-warning-config-default = warning: { $error }; using default config values for this command
ui-brand-tagline = The open-source agent operating system
ui-brand-title = LibreFang Agent OS
ui-label-hint = hint:
ui-label-next-steps = Next steps
ui-label-fix = fix:
ui-label-try = try:
ui-provider-not-set = { $env_var } not set
progress-fail = [FAIL]

# Table headers / Shared labels
label-header-name = NAME
label-header-kind = KIND
label-header-configured = CONFIGURED
label-header-token = TOKEN
label-header-alias = ALIAS
label-header-provider = PROVIDER
label-header-id = ID
label-header-agent = AGENT
label-header-category = CATEGORY
label-header-description = DESCRIPTION
label-header-hand = HAND
label-header-instance = INSTANCE
label-header-model = MODEL
label-header-status = STATUS
label-header-type = TYPE
label-header-timestamp = TIMESTAMP
label-header-event = EVENT
label-header-key = KEY
label-header-value = VALUE
label-header-enabled = ENABLED
label-header-url = URL

# Channel command specific keys
channel-header-msgs-24h = 24H MSGS
channel-error-save-failed-no-body = save failed (no error body)

# Models command specific keys
model-none-in-catalog = No models in catalog.
model-header-model = MODEL
model-header-tier = TIER
model-header-context = CONTEXT
model-header-resolves-to = RESOLVES TO
model-header-auth = AUTH
model-header-models = MODELS
model-header-base-url = BASE URL
model-picker-item =     { $idx }. { $id } { $tier }

# Approvals command specific keys
approval-none-pending = No pending approvals.
approval-header-request = REQUEST

# Auth command specific keys
auth-error-create-home-dir = Failed to create LibreFang home directory: { $error }
auth-error-write-secrets = Failed to write secrets.env: { $error }
auth-error-parse-config = Failed to parse config.toml: { $error }
auth-error-default-model-not-table = default_model is not a table
auth-error-write-config = Failed to write config.toml: { $error }
auth-pool-add-hint = Add one with:
auth-pool-add-example =   librefang auth pool add openai OPENAI_API_KEY_1 --label Primary --priority 10
auth-pool-header = { $provider }  ({ $strategy })
auth-pool-keys-available =   keys: { $available }/{ $total } available
auth-pool-cooldown-left = ({ $secs }s left)
auth-pool-status-invalid = invalid
auth-pool-status-exhausted = exhausted
auth-pool-status-cooldown = cooldown
auth-pool-status-env-missing = env-missing
auth-pool-status-healthy = healthy
auth-pool-key-requests = requests={ $count }
auth-pool-key-item =     - [{ $label }] { $key_display }  priority={ $pri }{ $reqs_str }  status={ $status }
auth-hash-add-config-hint = Add to config.toml:
auth-hash-config-entry =   dashboard_pass_hash = "{ $hash }"

# Agent command specific keys
agent-spawn-id-label =   ID:   { $id }
agent-spawn-name-label =   Name: { $name }
error-unknown = Unknown error
label-unknown = <unknown>
label-header-state = STATE
label-header-created = CREATED
label-header-msgs = MSGS
label-header-last-active = LAST ACTIVE
label-session-state-running = running
label-session-state-idle = idle

# Daemon command specific keys
daemon-error-resolve-exe = resolve current executable: { $error }
daemon-error-create-log-dir = create log directory { $path }: { $error }
daemon-error-open-log = open daemon log { $path }: { $error }
daemon-error-clone-log-handle = clone daemon log handle { $path }: { $error }
daemon-error-spawn-detached = spawn detached daemon: { $error }
daemon-error-failed-create-log-dir = Failed to create log directory { $path }: { $error }
daemon-error-failed-open-log = Failed to open daemon log file { $path }: { $error }

# --- Skill commands ---
skill-name-empty = skill name is empty
skill-name-unsafe = unsafe skill name '{ $name }': must be a single path component (no '/', '\', '..' or absolute path)
skill-hand-not-found = Hand '{ $hand }' not found at { $path }
skill-openclaw-detected = Detected OpenClaw skill format. Converting...
skill-install-refused = Refusing to install skill: { $error }
skill-write-manifest-failed = Failed to write manifest: { $error }
skill-openclaw-installed-to-hand = Installed OpenClaw skill '{ $name }' to hand '{ $hand }'
skill-openclaw-installed = Installed OpenClaw skill: { $name }
skill-openclaw-convert-failed = Failed to convert OpenClaw skill: { $error }
skill-no-toml = No skill.toml found in { $path }
skill-read-toml-failed = Error reading skill.toml: { $error }
skill-parse-toml-failed = Error parsing skill.toml: { $error }
skill-installed-to-hand = Installed skill '{ $name }' v{ $version } to hand '{ $hand }'
skill-installed = Installed skill: { $name } v{ $version }
skill-installed-hub-to-hand = Installed { $source } { $version } to hand '{ $hand }'
skill-installed-hub = Installed { $source } { $version }
skill-install-failed = Failed to install skill: { $error }
skill-list-none-hand = No skills installed for hand '{ $hand }'.
skill-list-none = No skills installed.
skill-list-count-hand = { $count } skill(s) installed for hand '{ $hand }':
skill-list-count = { $count } skill(s) installed:
skill-list-load-failed = Error loading skills: { $error }
skill-removed-from-hand = Removed skill '{ $name }' from hand '{ $hand }'
skill-removed = Removed skill: { $name }
skill-remove-failed = Failed to remove skill: { $error }
skill-search-none = No skills found for "{ $query }".
skill-search-results-header = Skills matching "{ $query }":
skill-search-failed = Search failed: { $error }
skill-validation-failed = Skill validation failed: { $error }
skill-execution-failed = Skill execution failed: { $error }
skill-package-failed = Failed to package skill: { $error }
skill-determine-dir-failed = Could not determine current directory: { $error }
skill-unsupported-runtime = Unsupported runtime '{ $runtime }'. Choose one of: python, node, wasm.
skill-create-dir-failed = Error creating skill directory: { $error }
skill-registry-load-failed = Error loading skill registry: { $error }
skill-not-found = Skill '{ $name }' not found in { $path }
skill-read-file-failed = Failed to read { $path }: { $error }
skill-create-skills-dir-failed = Failed to create skills dir: { $error }
skill-create-failed = Create failed: { $error }
skill-update-failed = Update failed: { $error }
skill-patch-failed = Patch failed: { $error }
skill-delete-failed = Delete failed: { $error }
skill-rollback-failed = Rollback failed: { $error }
skill-write-file-failed = Write-file failed: { $error }
skill-remove-file-failed = Remove-file failed: { $error }
skill-serialize-history-failed = Failed to serialize history: { $error }
skill-read-pending-failed = Failed to read pending directory: { $error }
skill-load-candidate-failed = Failed to load candidate: { $error }
skill-render-candidate-failed = Failed to render candidate as TOML: { $error }
skill-approve-candidate-failed = Approve failed: { $error }
skill-reject-candidate-failed = Reject failed: { $error }
skill-publish-failed = Publish failed: { $error }
skill-evolution-label = Skill: { $name }
skill-version-label = Current version: { $version }
skill-use-count-label = Use count: { $count }
skill-evolution-count-label = Evolution count: { $count }
skill-no-history = No version history recorded.
skill-no-pending = No pending skill candidates.{ $filter }
skill-pending-filter =  (filter: agent { $agent })
skill-approved-candidate = Approved candidate { $id } → installed skill '{ $name }' (v{ $version }).
skill-rejected-candidate = Rejected and removed candidate { $id }.
skill-validated = Validated skill: { $name } v{ $version }
skill-validated-runtime =   Runtime: { $runtime }
skill-validated-source =   Source: { $path }
skill-validated-description =   Description: { $description }
skill-validated-tools =   Tools: { $tools }
skill-refusing-warnings = Refusing to execute a skill with critical validation warnings.
skill-validated-only = Validation only: no tool declared to execute.
skill-invalid-input-json = Invalid --input JSON: { $error }
skill-tool-result-header = Tool result ({ $name }):
skill-validation-complete = Validation complete.
skill-execution-skipped = Execution skipped: { $message }
skill-preparing = Preparing skill: { $name } v{ $version }
skill-refusing-publish = Refusing to publish a skill with critical validation warnings.
skill-bundle-created = Bundle created: { $path }
skill-bundle-sha =   SHA256: { $sha }
skill-bundle-size =   Size: { $size } bytes
skill-dry-run = Dry run only.
skill-dry-run-repo =   Repo: { $repo }
skill-dry-run-tag =   Tag: { $tag }
skill-github-token-required = Set GITHUB_TOKEN or GH_TOKEN to publish, or re-run with --dry-run.
skill-publishing-progress = Publishing { $name }@{ $tag }
skill-publish-success = Published { $name } to { $repo }@{ $tag }
skill-publish-release-url = Release: { $url }
skill-warnings-none =   Warnings: none
skill-warnings-header =   Warnings:
skill-prompt-name = Skill name: 
skill-prompt-description = Description: 
skill-prompt-runtime = Runtime (python/node/wasm) [python]: 
skill-created = Skill created: { $path }
skill-created-files-header = Files:
skill-created-next-steps-header = Next steps:
skill-created-step-edit =   { $step }. Edit the entry point to implement your skill logic
skill-created-step-test =   { $step }. Test locally: librefang skill test { $path }
skill-created-step-install =   { $step }. Install: librefang skill install { $path }

# --- Monitoring & Status ---
monitoring-no-audit = No audit entries.
monitoring-no-memory = No memory entries for agent '{ $agent }'.
monitoring-no-devices = No paired devices.
monitoring-no-webhooks = No webhooks configured.
label-last-seen = LAST SEEN
status-watch-header =   { $status } (refreshing every { $interval }s, Ctrl+C to exit)
status-warning-config = warning: { $error }; using default config values for status display
status-summary-up = librefang { $version } { $state } uptime={ $uptime } { $auth } ({ $base })
status-peers-connected = { $connected } connected / { $total } known
status-agents-active = { $active } running / { $total } total
status-mb = { $mb } MB
status-summary-down = librefang down home={ $home } default={ $provider }/{ $model }
status-uptime-format = { $hours }h { $mins }m
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
desktop-install-unsupported-platform = Unsupported platform
desktop-install-error-hdiutil-attach = hdiutil attach failed: { $error }
desktop-install-error-app-not-found = LibreFang.app not found in DMG
desktop-install-error-remove-old = Failed to remove old installation: { $error }
desktop-install-error-cp = cp failed: { $error }
desktop-install-error-copy-applications = Copy to /Applications failed: { $error }
desktop-install-error-run-installer = Failed to run installer: { $error }
desktop-install-error-installer-status = Installer exited with: { $status }
desktop-install-error-localappdata = Cannot determine %LOCALAPPDATA%
desktop-install-error-binary-not-found = Installer completed but binary not found at expected location
desktop-install-error-home-dir = Cannot determine home directory
desktop-install-error-create-dir = Failed to create { $path }: { $error }
desktop-install-error-copy-appimage = Failed to copy AppImage: { $error }
desktop-install-error-http = HTTP request failed: { $error }
desktop-install-error-create = Cannot create { $path }: { $error }
desktop-install-error-write = Write error: { $error }

maintenance-error-github-request = GitHub request failed: { $error }
maintenance-error-github-status = GitHub API returned { $status }
maintenance-error-decode-release = Failed to decode release metadata: { $error }
maintenance-error-missing-tag = Release metadata is missing `tag_name`
maintenance-error-decode-list = Failed to decode releases list: { $error }
maintenance-error-no-release = No matching release found for the '{ $channel }' channel
maintenance-error-http-client = Failed to build HTTP client: { $error }
maintenance-error-powershell-updater = Failed to launch PowerShell updater: { $error }
maintenance-error-run-installer = Failed to run installer: { $error }
maintenance-error-installer-status = Installer exited with status { $status }
maintenance-error-download-fail = Download failed: { $error }
maintenance-error-download-status = Download returned { $status }
maintenance-error-read-response = Failed to read response body: { $error }
maintenance-error-create-dir = Failed to create updater dir: { $error }
maintenance-error-create-script = Failed to create updater script: { $error }
maintenance-error-write-script = Failed to write updater script: { $error }

common-error-find-exe = Cannot find executable: { $error }
common-error-spawn-daemon = Failed to spawn daemon: { $error }
common-error-daemon-timeout = Daemon did not become ready within 10 seconds

# tui/chat_runner.rs
chat-runner-owner-notice = [owner_notice] { $preview }
chat-runner-error-prefix = Error: { $error }
chat-runner-no-active-connection = No active connection
chat-runner-unknown-command = Unknown command: { $command }. Type /help
chat-runner-status-mode-daemon = Mode: daemon ({ $url })
chat-runner-status-agent = Agent: { $name }
chat-runner-status-mode-inprocess = Mode: in-process
chat-runner-status-agents-count = Agents: { $count }
chat-runner-status-mode-disconnected = Mode: disconnected
chat-runner-chat-history-cleared = Chat history cleared.
chat-runner-session-reset = Session reset — starting fresh.
chat-runner-session-reset-failed = Could not reset the session.
chat-runner-agent-killed = Agent "{ $name }" killed.
chat-runner-failed-kill-agent = Failed to kill agent "{ $name }".
chat-runner-kill-failed = Kill failed: { $error }
chat-runner-no-backend-connected = No backend connected.
chat-runner-no-models-available = No models available.
chat-runner-switched-model = Switched to { $model }
chat-runner-failed-switch-model = Failed to switch to { $model }
chat-runner-switch-failed = Switch failed: { $error }
chat-runner-welcome-help-hint = /help for commands • /exit to quit
chat-runner-spawning-agent = Spawning '{ $name }' agent…
chat-runner-no-agent-templates = No agent templates found. Run `librefang init`.
chat-runner-invalid-template = Invalid template '{ $name }': { $error }
chat-runner-spawn-failed = Spawn failed: { $error }
chat-runner-booting-kernel = Booting kernel…
chat-runner-booting-kernel-hint =   This may take a moment while the kernel initializes.
chat-runner-failed-start = Failed to start
chat-runner-press-esc-to-exit =   Press Esc to exit.

# tui/event.rs
tui-event-workflow-completed = Workflow completed
tui-event-workflow-exec-not-available-in-process = Workflow execution not available in in-process mode
tui-event-workflow-create-not-available-in-process = Workflow creation not available in in-process mode
tui-event-trigger-create-not-available-in-process = Trigger creation not available in in-process mode
tui-event-trigger-delete-failed = Failed to delete trigger { $trigger_id }
tui-event-trigger-delete-not-available-in-process = Trigger deletion not available in in-process mode
tui-event-agent-kill-failed = Failed to kill agent { $agent_id }
tui-event-agent-invalid-id = Invalid agent ID: { $agent_id }
tui-event-skills-fetch-failed = Failed to fetch skills
tui-event-mcp-fetch-failed = Failed to fetch MCP servers
tui-event-skills-update-failed = Failed to update skills
tui-event-skills-update-error = Skills update: { $error }
tui-event-mcp-update-failed = Failed to update MCP servers
tui-event-mcp-update-error = MCP update: { $error }
tui-event-session-delete-failed = Failed to delete session { $session_id }
tui-event-session-management-not-available-in-process = Session management not available in in-process mode
tui-event-kv-save-failed = Failed to save KV pair
tui-event-kv-not-available-in-process = Memory KV not available in in-process mode
tui-event-kv-delete-failed = Failed to delete KV pair
tui-event-skill-install-failed = Failed to install { $slug }
tui-event-skill-install-not-available-in-process = Skill installation not available in in-process mode
tui-event-skill-uninstall-failed = Failed to uninstall { $name }
tui-event-skill-uninstall-not-available-in-process = Skill uninstall not available in in-process mode
tui-event-security-verification-complete = Verification complete
tui-event-security-chain-not-applicable = In-process mode: chain not applicable
tui-event-provider-save-key-failed = Failed to save key for { $name }
tui-event-provider-key-management-not-available-in-process = Provider key management not available in in-process mode
tui-event-provider-delete-key-failed = Failed to delete key for { $name }
tui-event-provider-connection-ok = Connection OK
tui-event-provider-test-failed = Test failed
tui-event-provider-test-not-available-in-process = Provider test not available in in-process mode
tui-event-hand-activation-failed = Activation failed
tui-event-hand-activate-failed-error = Failed to activate: { $error }
tui-event-hand-activation-failed-error = Activation failed: { $error }
tui-event-hand-deactivate-failed = Failed to deactivate { $instance_id }
tui-event-hand-deactivate-failed-error = Deactivate failed: { $error }
tui-event-hand-invalid-instance-id = Invalid instance ID: { $error }
tui-event-hand-pause-failed = Failed to pause { $instance_id }
tui-event-hand-pause-failed-error = Pause failed: { $error }
tui-event-hand-resume-failed = Failed to resume { $instance_id }
tui-event-hand-resume-failed-error = Resume failed: { $error }
tui-event-extension-install-failed = Failed to install { $id }
tui-event-extension-install-failed-error = Install failed: { $error }
tui-event-extension-install-not-supported = Install via in-process mode not supported — use CLI
tui-event-extension-remove-failed = Failed to remove { $id }
tui-event-extension-remove-not-supported = Remove via in-process mode not supported — use CLI
tui-event-extension-reconnect-failed = Failed to reconnect { $id }
tui-event-extension-reconnect-not-supported = Reconnect via in-process mode not supported
tui-event-comms-message-sent = Message sent
tui-event-comms-send-failed = Send failed
tui-event-comms-send-not-supported-in-process = Send not supported in-process
tui-event-comms-task-posted = Task posted
tui-event-comms-post-failed = Post failed
tui-event-comms-post-not-supported-in-process = Task post not supported in-process
tui-event-stream-runtime-error = Runtime error: { $error }
tui-event-stream-connection-failed = Connection failed: { $error }
tui-event-agent-spawn-failed-fallback = Failed to spawn agent

# tui/mod.rs
tui-mod-session-deleted = Session { $id } deleted.
tui-mod-saved-key = Saved key: { $key }
tui-mod-deleted-key = Deleted key: { $key }
tui-mod-skill-installed = Installed: { $name }
tui-mod-skill-uninstalled = Uninstalled: { $name }
tui-mod-key-saved-for = Key saved for { $name }
tui-mod-key-deleted-for = Key deleted for { $name }
tui-mod-hand-activated = Activated: { $name }
tui-mod-hand-deactivated = Deactivated: { $id }
tui-mod-hand-paused = Hand paused
tui-mod-hand-resumed = Hand resumed
tui-mod-extension-installed = Installed: { $id }
tui-mod-extension-removed = Removed: { $id }
tui-mod-extension-reconnected = Reconnected { $id }: { $tools } tools
tui-mod-no-agents-running = No agents running.
tui-mod-agent-killed = Agent "{ $name }" killed.
tui-mod-failed-kill-agent = Failed to kill agent "{ $name }".
tui-mod-invalid-manifest = Invalid manifest: { $error }
tui-mod-spawn-failed = Spawn failed: { $error }
tui-mod-help-help = /help         — show this help
tui-mod-help-model = /model        — open model picker (Ctrl+M)
tui-mod-help-model-arg = /model <name> — switch to model directly
tui-mod-help-status = /status       — connection & agent info
tui-mod-help-agents = /agents       — list running agents
tui-mod-help-clear = /clear        — clear chat history
tui-mod-help-new = /new          — reset session (start fresh)
tui-mod-help-kill = /kill         — kill the current agent
tui-mod-help-exit = /exit         — end chat session
tui-mod-status-mode-daemon = Mode: daemon ({ $url })
tui-mod-status-agent = Agent: { $name }
tui-mod-status-mode-inprocess = Mode: in-process
tui-mod-status-agents-count = Agents: { $count }
tui-mod-status-mode-disconnected = Mode: disconnected
tui-mod-chat-history-cleared = Chat history cleared.
tui-mod-session-reset = Session reset — starting fresh.
tui-mod-session-reset-failed = Could not reset the session.
tui-mod-available-hands = Available hands ({ $count }):
tui-mod-active-hands = Active hands ({ $count }):
tui-mod-hands-info-requires-inprocess = Hands info requires in-process mode. Use the Hands tab instead.
tui-mod-unknown-command = Unknown command: { $command }. Type /help
tui-mod-error-symbol =  ✘ { $error }
tui-mod-press-ctrl-c-again-to-quit = Press Ctrl+C again to quit
tui-mod-ctrl-c-status-bar = Ctrl+C×2 quit  Tab/Ctrl+←→ switch
tui-mod-trigger-deleted = Trigger { $id } deleted.
tui-mod-agent-killed-status = Agent { $id } killed.
tui-mod-agent-kill-failed = Kill failed: { $error }
tui-mod-agent-skills-updated = Skills updated for agent { $id }.
tui-mod-agent-mcp-updated = MCP servers updated for agent { $id }.
tui-mod-ready = Ready
tui-mod-setup = Setup
tui-mod-workflow-created = Workflow created!
tui-mod-trigger-created = Trigger created!
tui-tab-dashboard = Dash
tui-tab-agents = Agents
tui-tab-chat = Chat
tui-tab-sessions = Sessions
tui-tab-workflows = Flows
tui-tab-triggers = Triggers
tui-tab-memory = Memory
tui-tab-skills = Skills
tui-tab-hands = Hands
tui-tab-extensions = Ext
tui-tab-templates = Tpl
tui-tab-peers = Peers
tui-tab-comms = Comms
tui-tab-security = Sec
tui-tab-audit = Audit
tui-tab-usage = Usage
tui-tab-settings = Config
tui-tab-logs = Logs
# welcome.rs
tui-welcome-menu-connect = Connect to daemon
tui-welcome-menu-connect-hint = talk to running agents via API
tui-welcome-menu-chat = Quick chat
tui-welcome-menu-chat-hint = boot kernel locally, no daemon needed
tui-welcome-menu-setup = Setup wizard
tui-welcome-menu-setup-hint = configure providers & channels
tui-welcome-menu-exit = Exit
tui-welcome-menu-exit-hint = quit LibreFang
tui-welcome-tagline = Agent Operating System
tui-welcome-ctrl-c-quit = Press Ctrl+C again to exit
tui-welcome-hint-bar = ↑↓ navigate  enter select  q quit
tui-welcome-checking-daemon = Checking for daemon…
tui-welcome-agent-count =
    { $count ->
        [one]  • { $count } agent
       *[other]  • { $count } agents
    }
tui-welcome-daemon-status = Daemon { $url }
tui-welcome-no-daemon = No daemon running
tui-welcome-provider = Provider: { $provider }
tui-welcome-no-api-keys = No API keys
tui-welcome-run-hint-prefix =  — run 
tui-welcome-setup-complete = Setup complete!

# sessions.rs
tui-sessions-title = Sessions
tui-sessions-filter = (filter: "{ $query }")
tui-sessions-count =
    { $count ->
        [one] 1 session
       *[other] { $count } sessions
    }
tui-sessions-header-agent = Agent
tui-sessions-header-id = Session ID
tui-sessions-header-msgs = Msgs
tui-sessions-header-created = Created
tui-sessions-loading = Loading sessions…
tui-sessions-empty = No sessions yet. Start a chat to create one.
tui-sessions-delete-confirm = Delete this session? [y] Yes  [any] Cancel
tui-sessions-hints = ↑↓ Navigate  Enter Open  d Delete  / Search  r Refresh

# peers.rs
tui-peers-title = Peers
tui-peers-network = OFP Peer Network
tui-peers-count =
    { $count ->
        [one] 1 peer
       *[other] { $count } peers
    }
tui-peers-header-node-id = Node ID
tui-peers-header-name = Name
tui-peers-header-address = Address
tui-peers-header-status = Status
tui-peers-header-agents = Agents
tui-peers-header-protocol = Protocol
tui-peers-status-active = Active
tui-peers-status-offline = Offline
tui-peers-status-pending = Pending
tui-peers-loading = Discovering peers…
tui-peers-empty = No peers connected. Enable OFP networking in config.toml.
tui-peers-hints = ↑↓ Navigate  r Refresh  (auto-refreshes every 15s)

# usage.rs
tui-usage-title = Usage
tui-usage-hints = [1] Summary  [2] By Model  [3] By Agent  [r] Refresh
tui-usage-tab-summary = 1 Summary
tui-usage-tab-model = 2 By Model
tui-usage-tab-agent = 3 By Agent
tui-usage-loading = Loading usage data…
tui-usage-loading-simple = Loading…
tui-usage-card-input = Input Tokens
tui-usage-card-output = Output Tokens
tui-usage-card-cost = Total Cost
tui-usage-card-calls = API Calls
tui-usage-header-model = Model
tui-usage-header-input = Input Tokens
tui-usage-header-output = Output Tokens
tui-usage-header-cost = Cost
tui-usage-header-calls = Calls
tui-usage-header-agent = Agent
tui-usage-header-total-tokens = Total Tokens
tui-usage-header-tool-calls = Tool Calls
tui-usage-empty = No usage data. Send messages to see token stats.

# hands.rs
tui-hands-title = Hands
tui-hands-tab-marketplace = Marketplace
tui-hands-tab-active = Active
tui-hands-loading = Loading Hands…
tui-hands-loading-active = Loading active Hands…
tui-hands-empty-marketplace = No Hand definitions loaded.
tui-hands-empty-active = No active Hands. Press [1] to browse the marketplace.
tui-hands-status-ready = Ready
tui-hands-status-setup = Setup
tui-hands-status-active = Active
tui-hands-status-paused = Paused
tui-hands-status-unknown = Unknown
tui-hands-hints-marketplace =   [↑↓] Navigate  [a/Enter] Activate  [r] Refresh
tui-hands-hints-active =   [↑↓] Navigate  [p] Pause/Resume  [d] Deactivate  [r] Refresh
tui-hands-confirm-deactivate =   Deactivate this Hand? [y] Yes  [any] Cancel
tui-hands-header-name = Name
tui-hands-header-category = Category
tui-hands-header-status = Status
tui-hands-header-description = Description
tui-hands-header-agent = Agent
tui-hands-header-hand = Hand
tui-hands-header-since = Since
tui-hands-category-content = Content
tui-hands-category-security = Security
tui-hands-category-development = Development
tui-hands-category-productivity = Productivity

# logs.rs
tui-logs-title = Logs
tui-logs-badge-auto = auto
tui-logs-badge-paused = paused
tui-logs-label-level = Level
tui-logs-filter-all = All
tui-logs-filter-error = Error
tui-logs-filter-warn = Warn
tui-logs-filter-info = Info
tui-logs-filter-active =   │ filter: "{ $query }"
tui-logs-entries-count =   │ { $count } entries
tui-logs-header-timestamp = Timestamp
tui-logs-header-level = Level
tui-logs-header-action = Action
tui-logs-header-agent = Agent
tui-logs-header-detail = Detail
tui-logs-loading = Loading logs…
tui-logs-empty = No log entries. Start the daemon to see logs.
tui-logs-hints =   [↑↓] Navigate  [f] Filter Level  [/] Search  [a] Auto-refresh  [r] Refresh

# security.rs
tui-security-title = Security
tui-security-active-features =   { $active }/{ $total } features active
tui-security-sections-sub =   │  Core · Configurable · Monitoring
tui-security-section-core = Core Security
tui-security-section-configurable = Configurable
tui-security-section-monitoring = Monitoring
tui-security-header-feature = Feature
tui-security-header-status = Status
tui-security-header-description = Description
tui-security-status-active = Active
tui-security-status-inactive = Inactive
tui-security-verifying = Verifying audit chain…
tui-security-verify-prompt = Press [v] to verify audit chain integrity
tui-security-verify-success = Audit chain verified
tui-security-verify-failed = Audit chain verification failed
tui-security-hints =   [↑↓] Scroll  [v] Verify Chain  [r] Refresh
tui-security-feat-path-traversal-name = Path Traversal Prevention
tui-security-feat-path-traversal-desc = safe_resolve_path blocks ../../ attacks
tui-security-feat-ssrf-name = SSRF Protection
tui-security-feat-ssrf-desc = Blocks private IPs and metadata endpoints in HTTP fetches
tui-security-feat-subprocess-name = Subprocess Isolation
tui-security-feat-subprocess-desc = env_clear() + selective vars on child processes
tui-security-feat-wasm-name = WASM Dual Metering
tui-security-feat-wasm-desc = Fuel + epoch interruption with watchdog thread
tui-security-feat-capability-name = Capability Inheritance
tui-security-feat-capability-desc = validate_capability_inheritance prevents privilege escalation
tui-security-feat-secret-name = Secret Zeroization
tui-security-feat-secret-desc = Zeroizing<String> auto-wipes API keys from memory
tui-security-feat-ed25519-name = Ed25519 Manifest Signing
tui-security-feat-ed25519-desc = Signed agent manifests with Ed25519 verification
tui-security-feat-taint-name = Taint Tracking
tui-security-feat-taint-desc = Information flow tracking across tool boundaries
tui-security-feat-ofp-name = OFP Wire Auth
tui-security-feat-ofp-desc = HMAC-SHA256 mutual authentication with nonce
tui-security-feat-rbac-name = RBAC Multi-User
tui-security-feat-rbac-desc = Role-based access control with user hierarchy
tui-security-feat-rate-name = Rate Limiting
tui-security-feat-rate-desc = GCRA rate limiter with cost-aware tokens
tui-security-feat-headers-name = Security Headers
tui-security-feat-headers-desc = CSP, X-Frame-Options, HSTS middleware
tui-security-feat-merkle-name = Merkle Audit Trail
tui-security-feat-merkle-desc = Hash chain audit log with tamper detection
tui-security-feat-heartbeat-name = Heartbeat Monitor
tui-security-feat-heartbeat-desc = Background health checks with restart limits
tui-security-feat-prompt-name = Prompt Injection Scanner
tui-security-feat-prompt-desc = Detects override attempts and data exfiltration

# templates.rs
tui-templates-title = Templates
tui-templates-cat-all = All
tui-templates-cat-general = General
tui-templates-cat-development = Development
tui-templates-cat-research = Research
tui-templates-cat-writing = Writing
tui-templates-cat-business = Business
tui-templates-header-template = Template
tui-templates-header-category = Category
tui-templates-header-provider-model = Provider/Model
tui-templates-header-description = Description
tui-templates-loading = Loading templates…
tui-templates-empty = No templates available.
tui-templates-detail-provider =   Provider: { $provider }/{ $model }  
tui-templates-hints =   [↑↓] Navigate  [Enter] Spawn Agent  [f] Filter Category  [r] Refresh
tui-templates-provider-not-configured = Provider '{ $provider }' not configured. Set API key in Settings first.
tui-templates-name-general-assistant = General Assistant
tui-templates-desc-general-assistant = Versatile AI assistant for everyday tasks
tui-templates-name-code-helper = Code Helper
tui-templates-desc-code-helper = Programming assistant with code review and debugging
tui-templates-name-researcher = Researcher
tui-templates-desc-researcher = Deep research and analysis with web search
tui-templates-name-writer = Writer
tui-templates-desc-writer = Creative and technical writing assistant
tui-templates-name-data-analyst = Data Analyst
tui-templates-desc-data-analyst = Data analysis, visualization, and SQL queries
tui-templates-name-devops-engineer = DevOps Engineer
tui-templates-desc-devops-engineer = Infrastructure, CI/CD, and deployment assistance
tui-templates-name-customer-support = Customer Support
tui-templates-desc-customer-support = Professional customer service agent
tui-templates-name-tutor = Tutor
tui-templates-desc-tutor = Patient educational assistant for learning any subject
tui-templates-name-api-designer = API Designer
tui-templates-desc-api-designer = REST/GraphQL API design and documentation
tui-templates-name-meeting-notes = Meeting Notes
tui-templates-desc-meeting-notes = Meeting transcription, summary, and action items

# audit.rs
tui-audit-title = Audit Trail
tui-audit-filter-all = All
tui-audit-filter-spawn = Agent Created
tui-audit-filter-kill = Agent Killed
tui-audit-filter-tool = Tool Used
tui-audit-filter-network = Network
tui-audit-filter-shell = Shell Exec
tui-audit-action-spawn = Agent Created
tui-audit-action-kill = Agent Killed
tui-audit-action-tool = Tool Used
tui-audit-action-network = Network Access
tui-audit-action-shell = Shell Exec
tui-audit-action-denied = Access Denied
tui-audit-action-config = Config Changed
tui-audit-label-filter = Filter:
tui-audit-entries-count = { $count } entries
tui-audit-header-timestamp = Timestamp
tui-audit-header-action = Action
tui-audit-header-agent = Agent
tui-audit-header-hash = Hash
tui-audit-header-detail = Detail
tui-audit-loading = Loading audit trail…
tui-audit-empty = No audit entries yet. Agent actions will appear here.
tui-audit-chain-unverified = Chain: not verified
tui-audit-chain-verified = Chain: Verified
tui-audit-chain-failed = Chain: Verification failed
tui-audit-hints =   [↑↓] Navigate  [f] Filter  [v] Verify Chain  [r] Refresh

# dashboard.rs
tui-dashboard-title = Dashboard
tui-dashboard-hints =   [r] Refresh  [a] Agents  [↑↓] Scroll  [PgUp/PgDn] Fast scroll
tui-dashboard-dreams-title = DREAMS
tui-dashboard-auto-dream-enabled = auto-dream enabled
tui-dashboard-auto-dream-disabled = auto-dream disabled
tui-dashboard-dream-details = phase={ $phase }  tools={ $tools }  mems={ $mems }
tui-dashboard-stat-agents = AGENTS
tui-dashboard-stat-uptime = UPTIME
tui-dashboard-stat-provider = PROVIDER
tui-dashboard-stat-model = MODEL
tui-dashboard-audit-time = Time
tui-dashboard-audit-agent = Agent
tui-dashboard-audit-action = Action
tui-dashboard-audit-detail = Detail
tui-dashboard-loading = Loading…
tui-dashboard-no-audit = No audit entries yet.

# comms.rs
tui-comms-title = Comms
tui-comms-tab-topology = Topology ({ $agents } agents, { $edges } edges)
tui-comms-tab-events = Events ({ $count })
tui-comms-hints =   [s]end  [t]ask  [r]efresh  [Tab] focus  [↑↓] scroll
tui-comms-loading = Loading topology…
tui-comms-empty = No agents running. Start agents to see communication.
tui-comms-events-empty = No inter-agent events yet. Activity will appear here.
tui-comms-modal-send-title =  Send Message 
tui-comms-modal-send-from = From (agent ID):
tui-comms-modal-send-to = To (agent ID):
tui-comms-modal-send-msg = Message:
tui-comms-modal-send-hints = [Tab] field  [Enter] send  [Esc] cancel
tui-comms-modal-task-title =  Post Task 
tui-comms-modal-task-title-field = Title:
tui-comms-modal-task-desc = Description:
tui-comms-modal-task-assign = Assign to (agent ID, optional):
tui-comms-modal-task-hints = [Tab] field  [Enter] post  [Esc] cancel

# settings.rs
tui-settings-title = Settings
tui-settings-hints-input =   [Enter] Save  [Esc] Cancel
tui-settings-hints-providers =   [↑↓] Navigate  [e] Set Key  [d] Delete Key  [t] Test  [r] Refresh
tui-settings-hints-models =   [↑↓] Navigate  [r] Refresh
tui-settings-hints-tools =   [↑↓] Navigate  [r] Refresh
tui-settings-tab-providers = 1 Providers
tui-settings-tab-models = 2 Models
tui-settings-tab-tools = 3 Tools
tui-settings-providers-header-provider = Provider
tui-settings-providers-header-status = Status
tui-settings-providers-header-env = Env Variable
tui-settings-providers-loading = Loading providers…
tui-settings-providers-empty = No providers configured. Run `librefang init` to set up.
tui-settings-providers-status-online = Online ({ $ms }ms)
tui-settings-providers-status-offline = Offline
tui-settings-providers-status-local = Local
tui-settings-providers-status-configured = Configured
tui-settings-providers-status-notset = Not set
tui-settings-providers-input-prompt = Enter API key for { $provider }: 
tui-settings-providers-latency = Latency: { $ms }ms
tui-settings-models-header-id = Model ID
tui-settings-models-header-provider = Provider
tui-settings-models-header-tier = Tier
tui-settings-models-header-context = Context
tui-settings-models-header-cost = Cost (in/out per 1M)
tui-settings-models-loading = Loading models…
tui-settings-models-empty = No models available.
tui-settings-tools-header-name = Tool Name
tui-settings-tools-header-desc = Description
tui-settings-tools-empty = No tools available.
# chat.rs
tui-chat-input-staged =   ({ $count } staged)
tui-chat-hints-modelpicker =     [↑↓] Navigate  [Enter] Select  [Esc] Close  [type] Filter
tui-chat-hints-streaming =     [Enter] Stage  [↑↓] Scroll  [Esc] Stop
tui-chat-hints-history =     [Enter] Send  [↑↓] History  [PgUp/PgDn] Scroll  [Esc] Back
tui-chat-hints-normal =     [Enter] Send  [Ctrl+M] Models  [↑↓] History  [PgUp/PgDn] Scroll  [Esc] Back
tui-chat-modelpicker-title =  Switch Model 
tui-chat-modelpicker-empty = No models match
tui-chat-welcome-ready = Ready to chat
tui-chat-welcome-suggest =   Try asking:
tui-chat-welcome-q1 = "Explain this codebase"
tui-chat-welcome-q2 = "Write a unit test for..."
tui-chat-welcome-q3 = "What does this function do?"
tui-chat-welcome-footer =   Type /help for commands  •  Ctrl+M to switch models
tui-chat-tokens-estimated =   ~{ $count } tokens
tui-chat-tokens-detail =   [tokens: { $in } in / { $out } out{ $cost }]
tui-chat-tool-input = input: 
tui-chat-tool-error = error: 
tui-chat-tool-result = result: 
tui-chat-tool-running = running…
tui-chat-thinking = thinking…
tui-chat-mode-daemon = daemon
tui-chat-mode-inprocess = in-process

# free_provider_guide.rs
tui-guide-hint-groq = free tier, blazing fast inference
tui-guide-hint-gemini = free tier, generous quota (Google account)
tui-guide-hint-deepseek = 5M free tokens for new accounts
tui-guide-label-apikey =  API Key 
tui-guide-warn-env = .env: { $error }

# init_wizard.rs
tui-init-welcome-tagline = Agent Operating System
tui-init-welcome-sec1 = Sandboxed execution, WASM isolation, SSRF protection
tui-init-welcome-sec2 = Signed manifests, audit trail, taint tracking
tui-init-welcome-sec3 = RBAC, capability checks, secret zeroization
tui-init-welcome-sec4 = API keys never logged, 0600 file permissions
tui-init-welcome-resp1 = Agents can execute code, access the network, and act
tui-init-welcome-resp2 = on your behalf.
tui-init-welcome-resp-warn = You are responsible for what they do.
tui-init-welcome-hints =   [Enter] I understand    [Esc] Cancel
tui-init-migrate-checking =   Checking for existing installations...
tui-init-migrate-openfang-detected =   OpenFang Installation Detected
tui-init-migrate-openclaw-detected =   OpenClaw Installation Detected
tui-init-migrate-openfang-summary = OpenFang configuration and data
tui-init-migrate-openclaw-agents = { $count } agents ({ $names })
tui-init-migrate-openclaw-no-agents = No agents
tui-init-migrate-openclaw-channels = { $count } channels ({ $names })
tui-init-migrate-openclaw-no-channels = No channels
tui-init-migrate-openclaw-skills = { $count } skills
tui-init-migrate-openclaw-no-skills = No skills
tui-init-migrate-openclaw-memory = Memory files
tui-init-migrate-openclaw-no-memory = No memory files
tui-init-migrate-openclaw-config = Configuration
tui-init-migrate-opt-yes = Yes
tui-init-migrate-opt-yes-desc = migrate settings and data
tui-init-migrate-opt-no = No
tui-init-migrate-opt-no-desc = start fresh
tui-init-migrate-hints =   [↑↓] Navigate  [Enter] Select  [Esc] Skip
tui-init-migrate-running-openfang =  Migrating from OpenFang...
tui-init-migrate-running-openclaw =  Migrating from OpenClaw...
tui-init-migrate-done-failed = Migration failed: { $error }
tui-init-migrate-done-config = Config migrated
tui-init-migrate-done-agents = { $count } agents imported ({ $names })
tui-init-migrate-done-channels = { $count } channels ({ $names })
tui-init-migrate-done-memory = Memory files copied
tui-init-migrate-done-skills = { $count } skills imported
tui-init-migrate-done-sessions = { $count } sessions imported
tui-init-migrate-done-skipped = { $name} skipped ({ $reason })
tui-init-migrate-done-summary =   { $imported } imported, { $skipped } skipped, { $warnings } warnings
tui-init-migrate-done-continue =   [Enter] Continue  
tui-init-migrate-done-autoadvancing = (auto-advancing...)
tui-init-provider-prompt =   Choose your LLM provider:
tui-init-provider-cli-detected = CLI detected
tui-init-provider-no-key-needed = no API key needed
tui-init-provider-local-no-key = local, no key needed
tui-init-provider-requires-with-hint = requires { $env_var } ({ $hint })
tui-init-provider-requires = requires { $env_var }
tui-init-provider-hints =   [↑↓/jk] Navigate  [Enter] Select  [Esc] Cancel
tui-init-hint-freetier = free tier
tui-init-hint-cheap = cheap
tui-init-hint-fast = fast inference
tui-init-hint-pat = via PAT
tui-init-hint-nokey = no API key
tui-init-hint-local = local
tui-init-apikey-prompt =   Enter your { $provider } API key:
tui-init-apikey-env-hint =     Or set { $env_var } environment variable
tui-init-apikey-testing =  Testing API key...
tui-init-apikey-verified = API key verified
tui-init-apikey-saved =     Saved to ~/.librefang/.env
tui-init-apikey-verify-failed = Could not verify (may still work)
tui-init-apikey-save-failed = Verified, but saving to .env failed
tui-init-apikey-save-failed-hints =     [Enter] retry save  ·  [Esc] edit key  (key already verified — nothing on disk)
tui-init-apikey-hints =   [Enter] Confirm  [Esc] Back
tui-init-model-prompt =   Choose default model for { $provider }:
tui-init-model-hints =   [↑↓/jk] Navigate  [Enter] Select  [Esc] Back    * = default
tui-init-routing-title =   Smart Model Routing
tui-init-routing-desc1 =   Automatically picks the right model per task complexity.
tui-init-routing-desc2 =   Simple tasks use cheap/fast models, complex tasks use
tui-init-routing-desc3 =   frontier models. Saves cost without sacrificing quality.
tui-init-routing-opt-yes = Yes
tui-init-routing-opt-yes-desc = pick 3 models (fast / balanced / frontier)
tui-init-routing-opt-no = No
tui-init-routing-opt-no-desc = use one model for everything
tui-init-routing-hints =   [↑↓] Navigate  [Enter] Select  [Esc] Back
tui-init-routing-pick-hints =   [↑↓/jk] Navigate  [Enter] Select  [Esc] Back
tui-init-routing-tier-fast = Fast
tui-init-routing-tier-balanced = Balanced
tui-init-routing-tier-frontier = Frontier
tui-init-routing-tier-fast-desc = quick lookups, greetings, simple Q&A
tui-init-routing-tier-balanced-desc = standard conversation, general tasks
tui-init-routing-tier-frontier-desc = multi-step reasoning, code generation
tui-init-complete-success-daemon = Setup complete — daemon running
tui-init-complete-success = Setup complete!
tui-init-complete-label-provider =   Provider:    
tui-init-complete-label-model =   Model:       
tui-init-complete-label-daemon =   Daemon:      
tui-init-complete-daemon-running = running at { $url }
tui-init-complete-daemon-not-running = not running
tui-init-complete-daemon-pending = pending
tui-init-complete-question =   How do you want to use LibreFang?
tui-init-complete-desktop-desc-installed = native window with system tray
tui-init-complete-desktop-desc-not-installed = not installed
tui-init-complete-opt-desktop = Desktop app
tui-init-complete-opt-desktop-badge = (recommended)
tui-init-complete-opt-dashboard = Web dashboard
tui-init-complete-opt-dashboard-desc = opens in your default browser
tui-init-complete-opt-chat = Terminal chat
tui-init-complete-opt-chat-desc = interactive chat right here
tui-init-complete-hints =   [↑↓/jk] Navigate  [Enter] Launch  [1/2/3] Quick select
tui-init-step-label = { $current } of { $total }
tui-init-complete-err-no-provider = No provider selected
tui-init-complete-err-home-dir = Could not determine home directory
tui-init-complete-err-write-config = Failed to write config: { $error }
tui-init-complete-err-daemon-failed = Daemon failed: { $error }
tui-init-routing-pick-prefix = Pick
tui-init-routing-pick-suffix = model ({ $step }/3):
tui-init-complete-setup-prefix = Setup complete — 

# agents.rs
tui-agents-tool-file-read-desc = Read files
tui-agents-tool-file-write-desc = Write files
tui-agents-tool-file-list-desc = List directory contents
tui-agents-tool-memory-store-desc = Store data in agent memory
tui-agents-tool-memory-recall-desc = Recall data from memory
tui-agents-tool-memory-list-desc = List all stored memory keys
tui-agents-tool-web-fetch-desc = Fetch web pages
tui-agents-tool-shell-exec-desc = Execute shell commands
tui-agents-tool-agent-send-desc = Send messages to other agents
tui-agents-tool-agent-list-desc = List running agents

tui-agents-title-create-method = Create Agent
tui-agents-title-templates = Templates
tui-agents-title-custom-name = Custom — Name
tui-agents-title-custom-desc = Custom — Description
tui-agents-title-custom-prompt = Custom — System Prompt
tui-agents-title-custom-tools = Custom — Tools
tui-agents-title-custom-skills = Custom — Skills
tui-agents-title-custom-mcp = Custom — MCP Servers
tui-agents-title-spawning = Spawning...
tui-agents-title-screen = Agents
tui-agents-title-detail = Agent Detail

tui-agents-prompt-create-method =   How would you like to create your agent?
tui-agents-prompt-name = Agent name:
tui-agents-prompt-desc = Description:
tui-agents-prompt-prompt = System prompt:
tui-agents-prompt-tools =   Select tools (Space to toggle):
tui-agents-prompt-skills =   Select skills (none checked = all skills):
tui-agents-prompt-mcp =   Select MCP servers (none checked = all servers):
tui-agents-prompt-edit-skills =   Space to toggle, Enter to save (none checked = all):
tui-agents-prompt-spawning =   Spawning agent...
tui-agents-label-no-agent-selected = No agent selected.
tui-agents-label-none-available = (none available)

tui-agents-opt-templates =   Choose from templates
tui-agents-opt-templates-desc =   (pre-built agents)
tui-agents-opt-custom =   Build custom agent
tui-agents-opt-custom-desc =   (pick name, tools, prompt)

tui-agents-header-state = State
tui-agents-header-name = Name
tui-agents-header-model = Model
tui-agents-header-id = ID
tui-agents-opt-create-new = Create new agent

tui-agents-hints-filter =   [Type] Filter  [Enter] Accept  [Esc] Cancel search
tui-agents-hints-list =   [↑↓] Navigate  [Enter] Detail  [/] Search  [Esc] Back
tui-agents-hints-detail =   [s] Edit skills  [m] Edit MCP  [c] Chat  [k] Kill  [Esc] Back
tui-agents-hints-navigate =     [↑↓] Navigate  [Enter] Select  [Esc] Back
tui-agents-hints-input =     [Enter] Next  [Esc] Back
tui-agents-hints-tools =     [↑↓] Navigate  [Space] Toggle  [Enter] Create  [Esc] Back
tui-agents-hints-skills =     [↑↓] Navigate  [Space] Toggle  [Enter] Next  [Esc] Back
tui-agents-hints-mcp =     [↑↓] Navigate  [Space] Toggle  [Enter] Create  [Esc] Back
tui-agents-hints-save =     [↑↓] Navigate  [Space] Toggle  [Enter] Save  [Esc] Cancel

tui-agents-placeholder-name = my-agent
tui-agents-placeholder-desc = A custom agent
tui-agents-placeholder-prompt = You are a helpful agent.
tui-agents-label-placeholder =     placeholder: { $placeholder }

tui-agents-detail-id =   ID:       
tui-agents-detail-name =   Name:     
tui-agents-detail-state =   State:    
tui-agents-detail-provider =   Provider: 
tui-agents-detail-model =   Model:    
tui-agents-detail-created =   Created:  
tui-agents-detail-active =   Active:   
tui-agents-detail-tags =   Tags:     
tui-agents-detail-caps =   Caps:     
tui-agents-detail-parent =   Parent:   
tui-agents-detail-children =   Children: 
tui-agents-detail-skills =   Skills:   
tui-agents-detail-mcp =   MCP:      
tui-agents-detail-all-skills = [All skills]
tui-agents-detail-all-servers = [All servers]
tui-agents-detail-none = [None]
tui-agents-default-desc = A custom { $name } agent
tui-agents-default-prompt = You are { $name }, a helpful agent.

# --- Workflows screen ---
tui-workflows-title-screen = Workflows
tui-workflows-header-id = ID
tui-workflows-header-name = Name
tui-workflows-header-steps = Steps
tui-workflows-header-created = Created
tui-workflows-loading = Loading workflows...
tui-workflows-empty-state = No workflows defined. Create one with [n].
tui-workflows-create-new-option =   + Create new workflow
tui-workflows-hints-list =   [↑↓] Navigate  [Enter] View runs  [x] Run  [n] New  [r] Refresh
tui-workflows-title-runs = Runs for: { $name }
tui-workflows-header-run-id = Run ID
tui-workflows-header-state = State
tui-workflows-header-duration = Duration
tui-workflows-header-output = Output
tui-workflows-runs-empty = No runs yet. Press [x] from the list to run.
tui-workflows-hints-runs =   [↑↓] Navigate  [r] Refresh  [Esc] Back
tui-workflows-title-create = Create New Workflow
tui-workflows-create-step =   Step { $current } of { $total }
tui-workflows-label-name = Workflow name:
tui-workflows-placeholder-name = my-workflow
tui-workflows-label-desc = Description:
tui-workflows-placeholder-desc = What this workflow does
tui-workflows-label-steps = Steps (JSON array):
tui-workflows-placeholder-steps = {"[{\"action\":\"...\"}]"}
tui-workflows-label-review = Review — press Enter to create
tui-workflows-review-name =   Name:  
tui-workflows-review-desc =   Desc:  
tui-workflows-hints-create-submit =   [Enter] Create  [Esc] Back
tui-workflows-hints-create-next =   [Enter] Next  [Esc] Back
tui-workflows-title-run-input = Run: { $name }
tui-workflows-label-run-input =   Input (JSON or text):
tui-workflows-placeholder-run-input = enter workflow input...
tui-workflows-hints-run-input =   [Enter] Run  [Esc] Cancel
tui-workflows-title-run-result = Workflow Run Result
tui-workflows-running = Running workflow...
tui-workflows-result-complete = Complete
tui-workflows-result-empty = No result.
tui-workflows-hints-run-result =   [Enter/Esc] Back

# --- Triggers screen ---
tui-triggers-title-screen = Triggers
tui-triggers-header-agent = Agent
tui-triggers-header-pattern = Pattern
tui-triggers-header-fires = Fires
tui-triggers-header-status = Status
tui-triggers-loading = Loading triggers...
tui-triggers-empty-state = No triggers configured. Create one with [n].
tui-triggers-status-active = ● Active
tui-triggers-status-off = ○ Off
tui-triggers-create-new-option =   + Create new trigger
tui-triggers-hints-list =   [↑↓] Navigate  [Enter] Create  [d] Delete  [r] Refresh
tui-triggers-title-create = Create New Trigger
tui-triggers-create-step =   Step { $current } of { $total }
tui-triggers-label-agent-id = Agent ID:
tui-triggers-placeholder-agent-id = agent-uuid
tui-triggers-label-pattern-picker =   Select pattern type:
tui-triggers-prompt-param = Pattern param for { $type }:
tui-triggers-placeholder-pattern-param = e.g. .*error.*
tui-triggers-label-prompt = Prompt template:
tui-triggers-placeholder-prompt = Handle this: {"{"}event{"}"}
tui-triggers-label-max-fires = Max fires (0 = unlimited):
tui-triggers-placeholder-max-fires = 0
tui-triggers-review-agent =   Agent:   
tui-triggers-review-pattern =   Pattern: 
tui-triggers-review-prompt =   Prompt:  
tui-triggers-review-max =   Max:     
tui-triggers-review-unlimited = unlimited
tui-triggers-review-confirm = Press Enter to create this trigger.
tui-triggers-hints-create-submit =   [Enter] Create  [Esc] Back
tui-triggers-hints-create-select =   [↑↓] Navigate  [Enter] Select  [Esc] Back
tui-triggers-hints-create-next =   [Enter] Next  [Esc] Back

tui-triggers-type-lifecycle-name = Lifecycle
tui-triggers-type-lifecycle-desc = Agent lifecycle events (start, stop, error)
tui-triggers-type-agentspawned-name = AgentSpawned
tui-triggers-type-agentspawned-desc = Fires when a new agent is spawned
tui-triggers-type-contentmatch-name = ContentMatch
tui-triggers-type-contentmatch-desc = Match on message content (regex)
tui-triggers-type-schedule-name = Schedule
tui-triggers-type-schedule-desc = Cron-like schedule trigger
tui-triggers-type-webhook-name = Webhook
tui-triggers-type-webhook-desc = HTTP webhook trigger
tui-triggers-type-channelmessage-name = ChannelMessage
tui-triggers-type-channelmessage-desc = Message received on a channel

# --- Memory screen ---
tui-memory-title-screen = Memory
tui-memory-label-select-agent =   Select an agent to browse its memory:
tui-memory-header-agent-name = Agent Name
tui-memory-header-id = ID
tui-memory-loading-agents = Loading agents...
tui-memory-empty-agents = No memory entries. Agents store data here automatically.
tui-memory-hints-agent-select =   ↑↓ Navigate  Enter Browse KV  r Refresh
tui-memory-pairs-count =   │ { $count } pairs
tui-memory-header-key = Key
tui-memory-header-value = Value
tui-memory-loading = Loading...
tui-memory-empty-kv = No key-value pairs. Press a to add one.
tui-memory-confirm-delete =   Delete this key? [y] Yes  [any] Cancel
tui-memory-hints-kv-browser =   ↑↓ Navigate  a Add  e Edit  d Delete  Esc Back  r Refresh
tui-memory-title-add = ┼ Add Key-Value Pair
tui-memory-title-edit = ✎ Edit Value
tui-memory-field-key = Key:
tui-memory-placeholder-key = enter key...
tui-memory-field-value = Value:
tui-memory-placeholder-value = enter value...
tui-memory-hints-edit =   Tab Switch field  Enter Save  Esc Cancel

# --- Extensions screen ---
tui-extensions-title-screen = Extensions
tui-extensions-tab-browse = 1 Browse
tui-extensions-tab-installed = 2 Installed
tui-extensions-tab-health = 3 Health
tui-extensions-status-ready = Ready
tui-extensions-status-setup = Setup
tui-extensions-status-error = Error
tui-extensions-status-off = Off
tui-extensions-status-installed = Installed
tui-extensions-status-available = Available
tui-extensions-header-name = Name
tui-extensions-header-category = Category
tui-extensions-header-status = Status
tui-extensions-header-desc = Description
tui-extensions-header-id = ID
tui-extensions-header-server = Server
tui-extensions-header-tools = Tools
tui-extensions-header-connected = Connected
tui-extensions-header-fails = Fails
tui-extensions-header-last-error = Last Error
tui-extensions-loading = Loading MCP servers...
tui-extensions-empty = No extensions installed. Browse the marketplace with [b].
tui-extensions-remove-confirm =   Press y to confirm removal, any other key to cancel
tui-extensions-hints-search =   Type to search • Esc cancel • Enter confirm
tui-extensions-hints-browse =   j/k navigate • Enter install • / search • r refresh
tui-extensions-hints-installed =   j/k navigate • d remove • r refresh
tui-extensions-hints-health =   j/k navigate • r/Enter reconnect • auto-reconnect active

# --- Skills screen ---
tui-skills-title-screen = Skills
tui-skills-tab-installed = 1 Installed
tui-skills-tab-clawhub = 2 ClawHub
tui-skills-tab-mcp = 3 MCP Servers
tui-skills-header-name = Name
tui-skills-header-runtime = Runtime
tui-skills-header-source = Source
tui-skills-header-desc = Description
tui-skills-header-downloads = Downloads
tui-skills-header-server = Server
tui-skills-header-status = Status
tui-skills-header-tools = Tools
tui-skills-loading = Loading skills...
tui-skills-empty = No skills installed. Browse ClawHub to find skills.
tui-skills-uninstall-confirm =   Uninstall this skill? [y] Yes  [any] Cancel
tui-skills-hints-installed =   [↑↓] Navigate  [u] Uninstall  [r] Refresh
tui-skills-sort =   Sort: { $sort }
tui-skills-sort-trending = trending
tui-skills-sort-popular = popular
tui-skills-sort-recent = recent
tui-skills-searching = Searching ClawHub...
tui-skills-empty-clawhub = No results. Press [/] to search or [s] to change sort.
tui-skills-hints-clawhub =   [↑↓] Navigate  [i] Install  [/] Search  [s] Sort  [r] Refresh
tui-skills-loading-mcp = Loading MCP servers...
tui-skills-empty-mcp = No MCP servers configured. Add servers in config.toml.
tui-skills-hints-mcp =   [↑↓] Navigate  [r] Refresh
tui-skills-mcp-status-connected = Connected
tui-skills-mcp-status-disconnected = Disconnected
tui-skills-mcp-tools-count = { $count } tools

# --- Setup Wizard screen ---
tui-wizard-title = Setup
tui-wizard-step-1 = Step 1 of 3
tui-wizard-step-2 = Step 2 of 3
tui-wizard-step-3 = Step 3 of 3
tui-wizard-step-saving = Saving...
tui-wizard-step-complete = Complete
tui-wizard-prompt-provider = Choose your LLM provider:
tui-wizard-hint-cli-detected = CLI detected
tui-wizard-hint-no-key-needed = no API key needed
tui-wizard-hint-local-no-key = local, no key needed
tui-wizard-hint-env-detected = { $env } detected
tui-wizard-hint-env-required = requires { $env }
tui-wizard-hints-provider =     [↑↓] Navigate  [Enter] Select  [Esc] Cancel
tui-wizard-prompt-api-key = Enter your { $provider } API key:
tui-wizard-hint-env-alternative = Or set { $env } environment variable
tui-wizard-hints-confirm-back =     [Enter] Confirm  [Esc] Back
tui-wizard-prompt-model-name = Model name:
tui-wizard-hint-model-default = default: { $model }
tui-wizard-status-no-provider = No provider selected
tui-wizard-status-no-home = Could not determine home directory
tui-wizard-status-saved = Config saved — { $provider } / { $model }
tui-wizard-status-save-fail = Failed to save config: { $error }
tui-wizard-status-continuing = Continuing...




