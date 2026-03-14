# --- API error messages (English) ---

# Agent errors
api-error-agent-not-found = Agent not found
api-error-agent-spawn-failed = Agent spawn failed
api-error-agent-invalid-id = Invalid agent ID
api-error-agent-already-exists = Agent already exists

# Message errors
api-error-message-too-large = Message too large (max 64KB)
api-error-message-delivery-failed = Message delivery failed: { $reason }

# Template errors
api-error-template-invalid-name = Invalid template name
api-error-template-not-found = Template '{ $name }' not found
api-error-template-parse-failed = Failed to parse template: { $error }
api-error-template-required = Either 'manifest_toml' or 'template' is required

# Manifest errors
api-error-manifest-too-large = Manifest too large (max 1MB)
api-error-manifest-invalid-format = Invalid manifest format
api-error-manifest-signature-mismatch = Signed manifest content does not match manifest_toml
api-error-manifest-signature-failed = Manifest signature verification failed

# Auth errors
api-error-auth-invalid-key = Invalid API key
api-error-auth-missing-header = Missing Authorization: Bearer <api_key> header

# Session errors
api-error-session-load-failed = Session load failed
api-error-session-not-found = Session not found

# Workflow errors
api-error-workflow-missing-steps = Missing 'steps' array
api-error-workflow-step-needs-agent = Step '{ $step }' needs 'agent_id' or 'agent_name'
api-error-workflow-invalid-id = Invalid workflow ID
api-error-workflow-execution-failed = Workflow execution failed

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

# Config errors
api-error-config-parse-failed = Failed to parse configuration: { $error }
api-error-config-write-failed = Failed to write configuration: { $error }

# Profile errors
api-error-profile-not-found = Profile '{ $name }' not found

# Cron errors
api-error-cron-invalid-id = Invalid cron job ID
api-error-cron-not-found = Cron job not found
api-error-cron-create-failed = Failed to create cron job: { $error }

# General errors
api-error-not-found = Resource not found
api-error-internal = Internal server error
api-error-bad-request = Bad request: { $reason }
api-error-rate-limited = Rate limit exceeded. Try again later.
