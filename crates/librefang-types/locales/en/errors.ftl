# --- API error messages (English) ---

# Agent errors
api-error-agent-not-found = Agent not found
api-error-agent-spawn-failed = Agent spawn failed
api-error-agent-invalid-id = Invalid agent ID
api-error-agent-already-exists = Agent already exists
api-error-agent-no-workspace = Agent has no workspace
api-error-agent-not-found-or-terminated = Agent not found or already terminated
api-error-agent-vanished = Agent vanished during update
api-error-agent-no-agents-available = No agents available
api-error-agent-no-target = Target agent not found. Specify agent_id or start an agent first.
api-error-agent-source-not-found = Source agent not found
api-error-agent-target-not-found = Target agent not found
api-error-agent-execution-failed = Agent execution failed: { $error }
api-error-agent-clone-spawn-failed = Failed to spawn clone: { $error }
api-error-agent-error = Agent error: { $error }
api-error-agent-not-found-with-id = Agent not found: { $id }
api-error-agent-invalid-sort = Invalid sort field '{ $field }'. Valid fields: { $valid }

# Message errors
api-error-message-too-large = Message too large (max 64KB)
api-error-message-delivery-failed = Message delivery failed: { $reason }
api-error-message-required = Message is required
api-error-message-missing-field = Missing 'message' field
api-error-message-streaming-failed = Failed to send streaming message

# Template errors
api-error-template-invalid-name = Invalid template name
api-error-template-not-found = Template '{ $name }' not found
api-error-template-parse-failed = Failed to parse template: { $error }
api-error-template-required = Either 'manifest_toml' or 'template' is required
api-error-template-invalid-manifest = Invalid template manifest
api-error-template-read-failed = Failed to read template

# Manifest errors
api-error-manifest-too-large = Manifest too large (max 1MB)
api-error-manifest-invalid-format = Invalid manifest format
api-error-manifest-signature-mismatch = Signed manifest content does not match manifest_toml
api-error-manifest-signature-failed = Manifest signature verification failed
api-error-manifest-invalid = Invalid manifest: { $error }

# Auth errors
api-error-auth-invalid-key = Invalid API key
api-error-auth-missing-header = Missing Authorization: Bearer <api_key> header
api-error-auth-missing = API key not configured for this provider

# Session errors
api-error-session-load-failed = Session load failed
api-error-session-not-found = Session not found
api-error-session-invalid-id = Invalid session ID
api-error-session-no-label = No session found with that label
api-error-session-cleanup-expired-failed = Failed to cleanup expired sessions: { $error }
api-error-session-cleanup-excess-failed = Failed to cleanup excess sessions: { $error }

# Workflow errors
api-error-workflow-missing-steps = Missing 'steps' array
api-error-workflow-step-needs-agent = Step '{ $step }' needs 'agent_id' or 'agent_name'
api-error-workflow-invalid-id = Invalid workflow ID
api-error-workflow-execution-failed = Workflow execution failed
api-error-workflow-not-found = Workflow not found

# Trigger errors
api-error-trigger-missing-agent-id = Missing 'agent_id'
api-error-trigger-invalid-agent-id = Invalid agent_id
api-error-trigger-invalid-pattern = Invalid trigger pattern
api-error-trigger-missing-pattern = Missing 'pattern'
api-error-trigger-registration-failed = Trigger registration failed (agent not found?)
api-error-trigger-invalid-id = Invalid trigger ID
api-error-trigger-not-found = Trigger not found

# Budget errors
api-error-budget-invalid-amount = Invalid budget amount
api-error-budget-update-failed = Budget update failed
api-error-budget-provide-at-least-one = Provide at least one of: max_cost_per_hour_usd, max_cost_per_day_usd, max_cost_per_month_usd, max_llm_tokens_per_hour

# Config errors
api-error-config-parse-failed = Failed to parse configuration: { $error }
api-error-config-write-failed = Failed to write configuration: { $error }
api-error-config-save-failed = Failed to save configuration: { $error }
api-error-config-remove-failed = Failed to remove configuration: { $error }
api-error-config-missing-toml = Missing toml_content field

# Profile errors
api-error-profile-not-found = Profile '{ $name }' not found

# Cron errors
api-error-cron-invalid-id = Invalid cron job ID
api-error-cron-not-found = Cron job not found
api-error-cron-create-failed = Failed to create cron job: { $error }
api-error-cron-invalid-expression = Invalid cron expression
api-error-cron-invalid-expression-detail = Invalid cron expression: requires 5 fields (minute hour day month weekday)
api-error-cron-missing-field = Missing 'cron' field

# Goal errors
api-error-goal-not-found = Goal not found
api-error-goal-not-found-with-id = Goal '{ $id }' not found
api-error-goal-missing-title = Missing or empty 'title' field
api-error-goal-title-too-long = Title too long (max 256 characters)
api-error-goal-description-too-long = Description too long (max 4096 characters)
api-error-goal-invalid-status = Invalid status. Must be one of: pending, in_progress, completed, cancelled
api-error-goal-progress-range = Progress must be in the range 0-100
api-error-goal-parent-not-found = Parent goal '{ $id }' not found
api-error-goal-self-parent = A goal cannot be its own parent
api-error-goal-circular-parent = Circular parent reference detected
api-error-goal-save-failed = Failed to save goal: { $error }
api-error-goal-update-failed = Failed to update goal: { $error }
api-error-goal-delete-failed = Failed to delete goal: { $error }
api-error-goal-load-failed = Failed to load goals: { $error }
api-error-goal-title-empty = Title cannot be empty
api-error-goal-status-invalid = Invalid status

# Memory errors
api-error-memory-not-enabled = Proactive memory is not enabled
api-error-memory-not-found = Memory not found
api-error-memory-operation-failed = Memory operation failed
api-error-memory-export-failed = Failed to export memory
api-error-memory-import-failed = Failed to import memory during clear
api-error-memory-key-not-found = Key not found
api-error-memory-missing-kv = Request body missing or invalid 'kv' object
api-error-memory-serialization-error = Serialization error
api-error-memory-missing-ids = Missing 'ids' array

# Network / A2A errors
api-error-network-not-enabled = Peer network is not enabled
api-error-network-peer-not-found = Peer not found
api-error-network-a2a-not-found = A2A agent '{ $url }' not found
api-error-network-connection-failed = Connection failed: { $error }
api-error-network-auth-failed = Authentication failed (HTTP { $status })
api-error-network-task-post-failed = Failed to post task: { $error }
api-error-network-missing-url = Missing 'url' query parameter

# Plugin errors
api-error-plugin-missing-name = Missing 'name'
api-error-plugin-missing-name-registry = Missing 'name' for registry install
api-error-plugin-missing-path = Missing 'path' for local install
api-error-plugin-missing-url = Missing 'url' for git install
api-error-plugin-invalid-source = Invalid source. Use one of: 'registry', 'local', 'git'

# Channel errors
api-error-channel-unknown = Unknown channel
api-error-channel-missing-agent-id = Missing required field: agent_id
api-error-channel-invalid-from = Invalid from_agent_id
api-error-channel-invalid-to = Invalid to_agent_id

# Provider errors
api-error-provider-missing-alias = Missing required field: alias
api-error-provider-missing-model-id = Missing required field: model_id
api-error-provider-missing-id = Missing required field: id
api-error-provider-missing-key = Missing or empty 'key' field
api-error-provider-alias-exists = Alias '{ $alias }' already exists
api-error-provider-alias-not-found = Alias '{ $alias }' not found
api-error-provider-model-not-found = Model '{ $id }' not found
api-error-provider-not-found = Provider '{ $name }' not found
api-error-provider-model-exists = Model '{ $id }' already exists in provider '{ $provider }'
api-error-provider-custom-model-not-found = Custom model '{ $id }' not found
api-error-provider-no-key-required = This provider does not require an API key
api-error-provider-key-not-configured = Provider API key is not configured
api-error-provider-secrets-write-failed = Failed to write secrets.env: { $error }
api-error-provider-secrets-update-failed = Failed to update secrets.env: { $error }
api-error-provider-invalid-url = Invalid URL format
api-error-provider-missing-url = Missing or empty 'url'
api-error-provider-missing-base-url = Missing or empty 'base_url' field
api-error-provider-unknown = Unknown provider '{ $name }'
api-error-provider-base-url-invalid = base_url must start with http:// or https://
api-error-provider-missing-model = Missing 'model' field
api-error-provider-token-save-failed = Failed to save token: { $error }
api-error-provider-unknown-poll = Unknown poll_id
api-error-provider-secret-write-failed = Failed to write secret: { $error }

# Skill errors
api-error-skill-missing-name = Missing or empty 'name' field
api-error-skill-invalid-name = Skill name may only contain alphanumeric characters, hyphens, and underscores
api-error-skill-not-found-source = Source code not found for this skill
api-error-skill-only-prompt = Only prompt-only skills can be created from the Web UI
api-error-skill-name-too-long = Name exceeds maximum length (256 characters)
api-error-skill-description-too-long = Description exceeds maximum length ({ $max } characters)
api-error-skill-dir-create-failed = Failed to create skill directory: { $error }
api-error-skill-toml-write-failed = Failed to write skill.toml: { $error }
api-error-skill-install-failed = Install failed: { $error }

# Hand errors
api-error-hand-not-found = Hand not found: { $id }
api-error-hand-definition-not-found = Hand definition not found
api-error-hand-instance-not-found = Instance not found

# MCP errors
api-error-mcp-missing-name = Missing 'name' field
api-error-mcp-missing-transport = Missing 'transport' field
api-error-mcp-invalid-config = Invalid MCP server configuration: { $error }
api-error-mcp-not-found = MCP server '{ $name }' not found
api-error-mcp-write-failed = Failed to write configuration: { $error }

# Integration/Extension errors
api-error-integration-not-found = Integration '{ $id }' not found
api-error-integration-missing-id = Missing 'id' field
api-error-extension-not-found = Extension '{ $id }' not found

# System errors
api-error-system-cli-not-found = CLI not found in PATH

# KV / Structured memory errors
api-error-kv-missing-fields = Missing 'fields' object
api-error-kv-missing-value = Missing 'value' field
api-error-kv-array-empty = Array cannot be empty
api-error-kv-missing-path = Missing 'path' field

# Approval errors
api-error-approval-invalid-id = Invalid approval ID
api-error-approval-not-found = Approval not found

# Webhook errors
api-error-webhook-not-enabled = Webhook triggers are not enabled
api-error-webhook-invalid-id = Invalid webhook ID
api-error-webhook-not-found = Webhook not found
api-error-webhook-missing-url = Missing 'url' field
api-error-webhook-missing-events = Missing 'events' array
api-error-webhook-invalid-events = Event types must be strings
api-error-webhook-event-types-required = At least one event type is required
api-error-webhook-url-unreachable = Webhook URL is unreachable: { $error }
api-error-webhook-event-publish-failed = Failed to publish event: { $error }
api-error-webhook-invalid-url = Invalid webhook URL format
api-error-webhook-agent-exec-failed = Webhook agent execution failed: { $error }
api-error-webhook-reach-failed = Failed to reach webhook URL: { $error }
api-error-webhook-unknown-event = Unknown event type '{ $event }'. Valid types: { $valid }

# Backup errors
api-error-backup-not-found = Backup not found
api-error-backup-file-not-found = Backup file not found
api-error-backup-invalid-filename = Invalid backup filename
api-error-backup-invalid-filename-zip = Invalid backup filename — must be a .zip file
api-error-backup-missing-manifest = Backup archive is missing manifest.json — not a valid LibreFang backup
api-error-backup-dir-create-failed = Failed to create backup directory: { $error }
api-error-backup-file-create-failed = Failed to create backup file: { $error }
api-error-backup-finalize-failed = Failed to finalize backup: { $error }
api-error-backup-open-failed = Failed to open backup: { $error }
api-error-backup-invalid-archive = Invalid backup archive: { $error }
api-error-backup-delete-failed = Failed to delete backup: { $error }

# Schedule errors
api-error-schedule-not-found = Schedule not found
api-error-schedule-missing-cron = Missing 'cron' field
api-error-schedule-missing-enabled = Missing 'enabled' field
api-error-schedule-invalid-cron = Invalid cron expression
api-error-schedule-invalid-cron-detail = Invalid cron expression: requires 5 fields (minute hour day month weekday)
api-error-schedule-save-failed = Failed to save schedule: { $error }
api-error-schedule-update-failed = Failed to update schedule: { $error }
api-error-schedule-delete-failed = Failed to delete schedule: { $error }
api-error-schedule-load-failed = Failed to load schedule: { $error }

# Job errors
api-error-job-invalid-id = Invalid job ID
api-error-job-not-found = Job not found
api-error-job-not-retryable = Task not found or not in a retryable state (must be completed or failed)
api-error-job-disappeared-cancel = Task disappeared after cancel
api-error-job-disappeared-complete = Task disappeared after completion

# Task errors
api-error-task-not-found = Task not found
api-error-task-disappeared = Task disappeared

# Pairing errors
api-error-pairing-not-enabled = Pairing is not enabled
api-error-pairing-invalid-token = Invalid or missing token

# Binding errors
api-error-binding-out-of-range = Binding index is out of range

# Command errors
api-error-command-not-found = Command '{ $name }' not found

# File/Upload errors
api-error-file-not-found = File not found
api-error-file-not-in-whitelist = File is not in whitelist
api-error-file-too-large = File too large (max { $max })
api-error-file-content-too-large = File content too large (max 32KB)
api-error-file-empty-body = Empty file body
api-error-file-save-failed = Failed to save file
api-error-file-missing-filename = Missing 'filename' field
api-error-file-missing-path = Missing 'path' field
api-error-file-path-too-deep = Path too deep (max 3 levels)
api-error-file-path-traversal = Path traversal denied
api-error-file-unsupported-type = Unsupported content type. Allowed: image/*, text/*, audio/*, application/pdf
api-error-file-upload-dir-failed = Failed to create upload directory
api-error-file-dir-not-found = Directory not found
api-error-file-workspace-error = Workspace path error

# Tool errors
api-error-tool-provide-allowlist = Provide 'tool_allowlist' and/or 'tool_blocklist'
api-error-tool-not-found = Tool not found: { $name }

# Validation errors
api-error-validation-content-empty = Content cannot be empty
api-error-validation-name-empty = new_name cannot be empty
api-error-validation-title-required = Title is required
api-error-validation-avatar-url-invalid = Avatar URL must be http/https or data URI
api-error-validation-color-invalid = Color must be a hex code starting with '#'

# General errors
api-error-not-found = Resource not found
api-error-internal = Internal server error
api-error-bad-request = Bad request: { $reason }
api-error-rate-limited = Rate limit exceeded. Try again later.
