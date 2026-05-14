//! Built-in tool definitions and the "always-native" subset.
//!
//! `builtin_tool_definitions()` returns the full schema list every agent
//! starts with; `select_native_tools()` projects it down to the
//! `ALWAYS_NATIVE_TOOLS` shortlist used in lazy-load mode (#3044).

use super::media::SUPPORTED_AUDIO_EXTS_DOC;
use librefang_types::tool::ToolDefinition;

/// Tools that are always shipped as full JSON schemas in every LLM request,
/// regardless of lazy-loading settings.
///
/// Rationale (issue #3044): shipping all ~75 builtin tool schemas on every
/// turn burns ~6k tokens of request payload. Most conversations only use a
/// handful of tools — and the ones below are the ones agents reach for most
/// often, so it's worth paying their declaration cost upfront to avoid a
/// `tool_load` round-trip on the common path.
///
/// Everything else in [`builtin_tool_definitions`] is available via the
/// `tool_load(name)` meta-tool (declared as part of this list so the LLM can
/// always discover new tools) and `tool_search(query)`.
///
/// Order matters only for readability in logs — the final list is a Vec, so
/// the order is preserved into the request body.
pub const ALWAYS_NATIVE_TOOLS: &[&str] = &[
    // Meta: discovery + loading. Without these, the LLM cannot escape the
    // lazy-load regime on its own.
    "tool_load",
    "tool_search",
    // Memory: used on nearly every turn of a multi-turn conversation.
    "memory_store",
    "memory_recall",
    "memory_list",
    // Web: the most common "go find something" action.
    "web_search",
    "web_fetch",
    // Files: reading is near-universal; writing and listing round out the
    // core file-flow so agents don't round-trip to load each one.
    "file_read",
    // Agent-to-agent / messaging: common proactive output path.
    "agent_send",
    "agent_list",
    "channel_send",
    // Private channel to the owner — intentionally cheap so agents never
    // skip using it because of declaration cost.
    "notify_owner",
    // Artifact retrieval — must be always available so agents can recover
    // spilled content even in lazy-tool mode.
    "read_artifact",
    // Skill evolution helpers stay native because they're also in the
    // always-available set enforced by the kernel.
    "skill_read_file",
    "skill_evolve_create",
    "skill_evolve_update",
    "skill_evolve_patch",
    "skill_evolve_delete",
    "skill_evolve_rollback",
    "skill_evolve_write_file",
    "skill_evolve_remove_file",
];

/// Select the subset of `all` whose names appear in [`ALWAYS_NATIVE_TOOLS`].
/// Used by the agent loop to build the lazy-mode tools list.
pub fn select_native_tools(all: &[ToolDefinition]) -> Vec<ToolDefinition> {
    let want: std::collections::HashSet<&str> = ALWAYS_NATIVE_TOOLS.iter().copied().collect();
    all.iter()
        .filter(|t| want.contains(t.name.as_str()))
        .cloned()
        .collect()
}

/// Get definitions for all built-in tools.
pub fn builtin_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        // --- Filesystem tools ---
        ToolDefinition {
            name: "file_read".to_string(),
            description: "Read the contents of a file. Paths are relative to the agent workspace.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "The file path to read" }
                },
                "required": ["path"]
            }),
        },
        ToolDefinition {
            name: "file_write".to_string(),
            description: "Write content to a file. Paths are relative to the agent workspace.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "The file path to write to" },
                    "content": { "type": "string", "description": "The content to write" }
                },
                "required": ["path", "content"]
            }),
        },
        ToolDefinition {
            name: "file_list".to_string(),
            description: "List files in a directory. Paths are relative to the agent workspace.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "The directory path to list" }
                },
                "required": ["path"]
            }),
        },
        ToolDefinition {
            name: "apply_patch".to_string(),
            description: "Apply a multi-hunk diff patch to add, update, move, or delete files. Use this for targeted edits instead of full file overwrites.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "patch": {
                        "type": "string",
                        "description": "The patch in *** Begin Patch / *** End Patch format. Use *** Add File:, *** Update File:, *** Delete File: markers. Hunks use @@ headers with space (context), - (remove), + (add) prefixed lines."
                    }
                },
                "required": ["patch"]
            }),
        },
        // --- Web tools ---
        ToolDefinition {
            name: "web_fetch".to_string(),
            description: "Fetch a URL with SSRF protection. Supports GET/POST/PUT/PATCH/DELETE. For GET, HTML is converted to Markdown. For other methods, returns raw response body.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "The URL to fetch (http/https only)" },
                    "method": { "type": "string", "enum": ["GET","POST","PUT","PATCH","DELETE"], "description": "HTTP method (default: GET)" },
                    "headers": { "type": "object", "description": "Custom HTTP headers as key-value pairs" },
                    "body": { "type": "string", "description": "Request body for POST/PUT/PATCH" }
                },
                "required": ["url"]
            }),
        },
        ToolDefinition {
            name: "web_fetch_to_file".to_string(),
            description: "Fetch a URL and stream the response body straight into a workspace file. \
Same SSRF protection, DNS pinning, and redirect re-validation as web_fetch, but the body \
never enters the agent context — only a short summary (path, byte count, sha256, content-type, \
status) is returned. Use this when downloading documents, papers, or other artifacts for later \
use instead of web_fetch + file_write (which round-trips the entire body through the model)."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "The URL to fetch (http/https only)" },
                    "dest_path": { "type": "string", "description": "Workspace-relative or absolute path to write to. Absolute paths must stay inside the agent workspace or a read-write named workspace." },
                    "method": { "type": "string", "enum": ["GET","POST","PUT","PATCH","DELETE"], "description": "HTTP method (default: GET)" },
                    "headers": { "type": "object", "description": "Custom HTTP headers as key-value pairs" },
                    "body": { "type": "string", "description": "Request body for POST/PUT/PATCH" },
                    "max_bytes": { "type": "integer", "description": "Optional per-call cap; clamped down to the configured max_file_bytes" }
                },
                "required": ["url", "dest_path"]
            }),
        },
        ToolDefinition {
            name: "web_search".to_string(),
            description: "Search the web using multiple providers (Tavily, Brave, Perplexity, DuckDuckGo) with automatic fallback. Returns structured results with titles, URLs, and snippets.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "The search query" },
                    "max_results": { "type": "integer", "description": "Maximum number of results to return (default: 5, max: 20)" }
                },
                "required": ["query"]
            }),
        },
        // --- Shell tool ---
        ToolDefinition {
            name: "shell_exec".to_string(),
            description: "Execute a shell command and return its output.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The command to execute" },
                    "timeout_seconds": { "type": "integer", "description": "Timeout in seconds (default: 30)" }
                },
                "required": ["command"]
            }),
        },
        // --- Owner-side channel ---
        ToolDefinition {
            name: "notify_owner".to_string(),
            description: "Send a private notice to the agent's owner (operator DM) WITHOUT posting it to the source chat. Use this in groups when you have something to tell the owner that should not be visible to other participants. Returns an opaque ack — do NOT repeat the summary in your public reply.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "reason": {
                        "type": "string",
                        "description": "Short machine-readable category, e.g. 'confirmation_needed', 'stranger_request', 'escalation'."
                    },
                    "summary": {
                        "type": "string",
                        "description": "Human-readable message body addressed to the owner."
                    }
                },
                "required": ["reason", "summary"]
            }),
        },
        // --- Inter-agent tools ---
        ToolDefinition {
            name: "agent_send".to_string(),
            description: "Send a message to another agent and receive their response. Accepts UUID or agent name. Use agent_find first to discover agents.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "The target agent's UUID or name" },
                    "message": { "type": "string", "description": "The message to send to the agent" }
                },
                "required": ["agent_id", "message"]
            }),
        },
        ToolDefinition {
            name: "agent_spawn".to_string(),
            description: "Spawn a new agent from settings. Returns the new agent's ID and name.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Unique name for the new agent. Ensure it does not conflict with existing agents."
                    },
                    "system_prompt": {
                        "type": "string",
                        "description": "The system prompt for the new agent"
                    },
                    "tools": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Select from all available tools, including MCP tools. Use the full tool names only"
                    },
                    "network": {
                        "type": "boolean",
                        "description": "Whether to enable network access for the new agent (required to be true when web_fetch is in tools)"
                    },
                    "shell": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Preset necessary shell commands based on the agent's task (e.g., [\"uv *\", \"pnpm *\"]). "
                    }
                },
                "required": ["name", "system_prompt"]
            }),
        },
        ToolDefinition {
            name: "agent_list".to_string(),
            description: "List all currently running agents with their IDs, names, states, and models.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "agent_kill".to_string(),
            description: "Kill (terminate) another agent. Accepts UUID or agent name.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "The target agent's UUID or name" }
                },
                "required": ["agent_id"]
            }),
        },
        // --- Shared memory tools ---
        ToolDefinition {
            name: "memory_store".to_string(),
            description: "Store a value in shared memory accessible by all agents. Use for cross-agent coordination and data sharing.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "The storage key" },
                    "value": { "type": "string", "description": "The value to store (JSON-encode objects/arrays, or pass a plain string)" }
                },
                "required": ["key", "value"]
            }),
        },
        ToolDefinition {
            name: "memory_recall".to_string(),
            description: "Recall a value from shared memory by key.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "The storage key to recall" }
                },
                "required": ["key"]
            }),
        },
        ToolDefinition {
            name: "memory_list".to_string(),
            description: "List all keys stored in shared memory.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
            }),
        },
        // --- Memory wiki tools (issue #3329) — return KernelOpError::unavailable
        //     when [memory_wiki] enabled = false in config.toml. ---
        ToolDefinition {
            name: "wiki_get".to_string(),
            description:
                "Read a wiki page by topic from the durable knowledge vault. \
                 Returns the page as JSON: {topic, frontmatter, body}. The \
                 frontmatter carries provenance (which agents/sessions \
                 contributed and when)."
                    .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "topic": { "type": "string", "description": "Page topic — must match [a-zA-Z0-9_-]+ and not be `index` or `_*`" }
                },
                "required": ["topic"]
            }),
        },
        ToolDefinition {
            name: "wiki_search".to_string(),
            description:
                "Search wiki page bodies (case-insensitive substring). Topic \
                 hits outrank body hits. Returns an array of \
                 {topic, snippet, score} sorted by score descending."
                    .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query" },
                    "limit": { "type": "integer", "description": "Max hits (default 10)" }
                },
                "required": ["query"]
            }),
        },
        ToolDefinition {
            name: "wiki_write".to_string(),
            description:
                "Write or update a wiki page. Body may use [[topic]] \
                 placeholders for cross-references; the vault rewrites them \
                 per its render mode. Provenance is auto-filled from the \
                 calling agent. If the page was edited externally since the \
                 last write, the call fails unless `force = true`, in which \
                 case the external body is preserved and only provenance is \
                 appended."
                    .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "topic": { "type": "string", "description": "Page topic — must match [a-zA-Z0-9_-]+" },
                    "body":  { "type": "string", "description": "Markdown body. Use [[other-topic]] placeholders for cross-references." },
                    "force": { "type": "boolean", "description": "Overwrite even if the page was edited externally (default false)" }
                },
                "required": ["topic", "body"]
            }),
        },
        // --- Collaboration tools ---
        ToolDefinition {
            name: "agent_find".to_string(),
            description: "Discover agents by name, tag, tool, or description. Use to find specialists before delegating work.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query (matches agent name, tags, tools, description)" }
                },
                "required": ["query"]
            }),
        },
        ToolDefinition {
            name: "task_post".to_string(),
            description: "Post a task to the shared task queue for another agent to pick up.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "Short task title" },
                    "description": { "type": "string", "description": "Detailed task description" },
                    "assigned_to": { "type": "string", "description": "Agent name or ID to assign the task to (optional)" }
                },
                "required": ["title", "description"]
            }),
        },
        ToolDefinition {
            name: "task_claim".to_string(),
            description: "Claim the next available task from the task queue assigned to you or unassigned.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "task_complete".to_string(),
            description: "Mark a previously claimed task as completed with a result.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string", "description": "The task ID to complete" },
                    "result": { "type": "string", "description": "The result or outcome of the task" }
                },
                "required": ["task_id", "result"]
            }),
        },
        ToolDefinition {
            name: "task_list".to_string(),
            description: "List tasks in the shared queue, optionally filtered by status (pending, in_progress, completed).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "status": { "type": "string", "description": "Filter by status: pending, in_progress, completed (optional)" }
                }
            }),
        },
        ToolDefinition {
            name: "task_status".to_string(),
            description: "Look up a single task on the shared queue by ID and return its status, result, title, assignee, created_at, and completed_at. Native counterpart of the comms_task_status MCP bridge tool — no MCP load required when polling for a delegated task's outcome. Any agent that knows the task_id can read it — task visibility is shared across all agents in the workspace, mirroring task_list / comms_task_status semantics.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string", "description": "The task ID returned by task_post" }
                },
                "required": ["task_id"]
            }),
        },
        ToolDefinition {
            name: "event_publish".to_string(),
            description: "Publish a custom event that can trigger proactive agents. Use to broadcast signals to the agent fleet.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "event_type": { "type": "string", "description": "Type identifier for the event (e.g., 'code_review_requested')" },
                    "payload": { "type": "object", "description": "JSON payload data for the event" }
                },
                "required": ["event_type"]
            }),
        },
        // --- Skill file read tool ---
        ToolDefinition {
            name: "skill_read_file".to_string(),
            description: "Read a companion file from an installed skill. Use when a skill's prompt context references additional files by relative path (e.g. 'see references/syntax.md').".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "skill": { "type": "string", "description": "The skill name as listed in Available Skills" },
                    "path": { "type": "string", "description": "Path relative to the skill directory, e.g. 'references/query-syntax.md'" }
                },
                "required": ["skill", "path"]
            }),
        },
        // --- Scheduling tools ---
        ToolDefinition {
            name: "schedule_create".to_string(),
            description: "Schedule a recurring task using natural language or cron syntax. Examples: 'every 5 minutes', 'daily at 9am', 'weekdays at 6pm', '0 */5 * * *'.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "description": { "type": "string", "description": "What this schedule does (e.g., 'Check for new emails')" },
                    "schedule": { "type": "string", "description": "Natural language or cron expression (e.g., 'every 5 minutes', 'daily at 9am', '0 */5 * * *')" },
                    "tz": { "type": "string", "description": "IANA timezone for time-of-day schedules (e.g., 'Asia/Shanghai', 'US/Eastern'). Omit for UTC. Always set this for schedules like 'daily at 9am' so they run in the user's local time." },
                    "agent": { "type": "string", "description": "Agent name or ID to run this task (optional, defaults to self)" }
                },
                "required": ["description", "schedule"]
            }),
        },
        ToolDefinition {
            name: "schedule_list".to_string(),
            description: "List all scheduled tasks with their IDs, descriptions, schedules, and next run times.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "schedule_delete".to_string(),
            description: "Remove a scheduled task by its ID.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "The schedule ID to remove" }
                },
                "required": ["id"]
            }),
        },
        // --- Knowledge graph tools ---
        ToolDefinition {
            name: "knowledge_add_entity".to_string(),
            description: "Add an entity to the knowledge graph. Entities represent people, organizations, projects, concepts, locations, tools, etc.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Display name of the entity" },
                    "entity_type": { "type": "string", "description": "Type: person, organization, project, concept, event, location, document, tool, or a custom type" },
                    "properties": { "type": "object", "description": "Arbitrary key-value properties (optional)" }
                },
                "required": ["name", "entity_type"]
            }),
        },
        ToolDefinition {
            name: "knowledge_add_relation".to_string(),
            description: "Add a relation between two entities in the knowledge graph.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "source": { "type": "string", "description": "Source entity ID or name" },
                    "relation": { "type": "string", "description": "Relation type: works_at, knows_about, related_to, depends_on, owned_by, created_by, located_in, part_of, uses, produces, or a custom type" },
                    "target": { "type": "string", "description": "Target entity ID or name" },
                    "confidence": { "type": "number", "description": "Confidence score 0.0-1.0 (default: 1.0)" },
                    "properties": { "type": "object", "description": "Arbitrary key-value properties (optional)" }
                },
                "required": ["source", "relation", "target"]
            }),
        },
        ToolDefinition {
            name: "knowledge_query".to_string(),
            description: "Query the knowledge graph. Filter by source entity, relation type, and/or target entity. Returns matching entity-relation-entity triples.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "source": { "type": "string", "description": "Filter by source entity name or ID (optional)" },
                    "relation": { "type": "string", "description": "Filter by relation type (optional)" },
                    "target": { "type": "string", "description": "Filter by target entity name or ID (optional)" },
                    "max_depth": { "type": "integer", "description": "Maximum traversal depth (default: 1)" }
                }
            }),
        },
        // --- Image analysis tool ---
        ToolDefinition {
            name: "image_analyze".to_string(),
            description: "Analyze an image file — returns format, dimensions, file size, and a base64 preview. For vision-model analysis, include a prompt.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the image file" },
                    "prompt": { "type": "string", "description": "Optional prompt for vision analysis (e.g., 'Describe what you see')" }
                },
                "required": ["path"]
            }),
        },
        // --- Location tool ---
        ToolDefinition {
            name: "location_get".to_string(),
            description: "Get approximate geographic location based on IP address. Returns city, country, coordinates, and timezone.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        // --- Browser automation tools ---
        ToolDefinition {
            name: "browser_navigate".to_string(),
            description: "Navigate a browser to a URL. Returns the page title and readable content as markdown. Opens a persistent browser session.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "The URL to navigate to (http/https only)" }
                },
                "required": ["url"]
            }),
        },
        ToolDefinition {
            name: "browser_click".to_string(),
            description: "Click an element on the current browser page by CSS selector or visible text. Returns the resulting page state.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector (e.g., '#submit-btn', '.add-to-cart') or visible text to click" }
                },
                "required": ["selector"]
            }),
        },
        ToolDefinition {
            name: "browser_type".to_string(),
            description: "Type text into an input field on the current browser page.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector for the input field (e.g., 'input[name=\"email\"]', '#search-box')" },
                    "text": { "type": "string", "description": "The text to type into the field" }
                },
                "required": ["selector", "text"]
            }),
        },
        ToolDefinition {
            name: "browser_screenshot".to_string(),
            description: "Take a screenshot of the current browser page. Returns a base64-encoded PNG image.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "browser_read_page".to_string(),
            description: "Read the current browser page content as structured markdown. Use after clicking or navigating to see the updated page.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "browser_close".to_string(),
            description: "Close the browser session. The browser will also auto-close when the agent loop ends.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "browser_scroll".to_string(),
            description: "Scroll the browser page. Use this to see content below the fold or navigate long pages.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "direction": { "type": "string", "description": "Scroll direction: 'up', 'down', 'left', 'right' (default: 'down')" },
                    "amount": { "type": "integer", "description": "Pixels to scroll (default: 600)" }
                }
            }),
        },
        ToolDefinition {
            name: "browser_wait".to_string(),
            description: "Wait for a CSS selector to appear on the page. Useful for dynamic content that loads asynchronously.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector to wait for" },
                    "timeout_ms": { "type": "integer", "description": "Max wait time in milliseconds (default: 5000, max: 30000)" }
                },
                "required": ["selector"]
            }),
        },
        ToolDefinition {
            name: "browser_run_js".to_string(),
            description: "Run JavaScript on the current browser page and return the result. For advanced interactions that other browser tools cannot handle.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "expression": { "type": "string", "description": "JavaScript expression to run in the page context" }
                },
                "required": ["expression"]
            }),
        },
        ToolDefinition {
            name: "browser_back".to_string(),
            description: "Go back to the previous page in browser history.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        // --- Media understanding tools ---
        ToolDefinition {
            name: "media_describe".to_string(),
            description: "Describe an image using a vision-capable LLM. Auto-selects the best available provider (Anthropic, OpenAI, or Gemini). Returns a text description of the image content.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the image file (relative to workspace)" },
                    "prompt": { "type": "string", "description": "Optional prompt to guide the description (e.g., 'Extract all text from this image')" }
                },
                "required": ["path"]
            }),
        },
        ToolDefinition {
            name: "media_transcribe".to_string(),
            description: "Transcribe audio to text using speech-to-text. Auto-selects the best available provider (Groq Whisper or OpenAI Whisper). Returns the transcript.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": format!("Path to the audio file (relative to workspace). Supported: {SUPPORTED_AUDIO_EXTS_DOC}.") },
                    "language": { "type": "string", "description": "Optional ISO-639-1 language code (e.g., 'en', 'es', 'ja')" }
                },
                "required": ["path"]
            }),
        },
        // --- Image generation tool ---
        ToolDefinition {
            name: "image_generate".to_string(),
            description: "Generate images from a text prompt. Supports multiple providers: OpenAI (dall-e-3, gpt-image-1), Gemini (imagen-3.0), MiniMax (image-01). Auto-detects configured provider if not specified.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "Text description of the image to generate (max 4000 chars)" },
                    "model": { "type": "string", "description": "Model to use (e.g. 'dall-e-3', 'imagen-3.0-generate-002', 'image-01'). Uses provider default if not specified." },
                    "aspect_ratio": { "type": "string", "description": "Aspect ratio: '1:1' (default), '16:9', '9:16'" },
                    "width": { "type": "integer", "description": "Image width in pixels (provider-specific)" },
                    "height": { "type": "integer", "description": "Image height in pixels (provider-specific)" },
                    "quality": { "type": "string", "description": "Quality: 'hd', 'standard', etc." },
                    "count": { "type": "integer", "description": "Number of images (1-4, default: 1)" },
                    "provider": { "type": "string", "description": "Provider (openai, gemini, minimax). Auto-detects if not specified." }
                },
                "required": ["prompt"]
            }),
        },
        // --- Video/music generation tools ---
        ToolDefinition {
            name: "video_generate".to_string(),
            description: "Generate a video from a text prompt or reference image. Returns a task_id for polling. Use video_status to check progress.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "Text description of the video to generate (required unless image_url is provided)" },
                    "image_url": { "type": "string", "description": "Reference image URL for image-to-video generation" },
                    "model": { "type": "string", "description": "Model ID (default: auto-detect)" },
                    "duration": { "type": "integer", "description": "Duration in seconds (default: 6)" },
                    "resolution": { "type": "string", "description": "Resolution (720P, 768P, 1080P)" },
                    "provider": { "type": "string", "description": "Provider (openai, gemini, minimax). Auto-detects if not specified." }
                },
                "required": []
            }),
        },
        ToolDefinition {
            name: "video_status".to_string(),
            description: "Check the status of a video generation task. Returns status and download URL when complete.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string", "description": "Task ID from video_generate" },
                    "provider": { "type": "string", "description": "Provider that created the task" }
                },
                "required": ["task_id"]
            }),
        },
        ToolDefinition {
            name: "music_generate".to_string(),
            description: "Generate music from a prompt and/or lyrics. Saves audio to workspace output/ directory.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "Style/mood description (e.g. 'upbeat pop song')" },
                    "lyrics": { "type": "string", "description": "Song lyrics with optional [Verse], [Chorus] tags" },
                    "model": { "type": "string", "description": "Model ID (default: music-2.5)" },
                    "instrumental": { "type": "boolean", "description": "Generate instrumental only, no vocals" },
                    "provider": { "type": "string", "description": "Provider (default: auto-detect)" }
                }
            }),
        },
        // --- Cron scheduling tools ---
        ToolDefinition {
            name: "cron_create".to_string(),
            description: "Create a scheduled/cron job. Supports one-shot (at), recurring (every N seconds), and cron expressions. Max 50 jobs per agent.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Job name (max 128 chars, alphanumeric + spaces/hyphens/underscores)" },
                    "schedule": {
                        "type": "object",
                        "description": "Schedule: {\"kind\":\"at\",\"at\":\"2025-01-01T00:00:00Z\"} or {\"kind\":\"every\",\"every_secs\":300} or {\"kind\":\"cron\",\"expr\":\"0 */6 * * *\",\"tz\":\"America/New_York\"}. For cron schedules, always include \"tz\" (IANA timezone, e.g. \"Asia/Shanghai\", \"Europe/London\") so the schedule runs in the user's local time. Omitting tz defaults to UTC."
                    },
                    "action": {
                        "type": "object",
                        "description": "Action: {\"kind\":\"system_event\",\"text\":\"...\"} or {\"kind\":\"agent_turn\",\"message\":\"...\",\"timeout_secs\":300}"
                    },
                    "delivery": {
                        "type": "object",
                        "description": "Delivery target: {\"kind\":\"none\"} or {\"kind\":\"channel\",\"channel\":\"telegram\"} or {\"kind\":\"last_channel\"}"
                    },
                    "one_shot": { "type": "boolean", "description": "If true, auto-delete after execution. Default: false" },
                    "session_mode": { "type": "string", "enum": ["persistent", "new"], "description": "Session behaviour for AgentTurn actions. 'persistent' (default): all fires share one dedicated cron session, preserving history across runs. 'new': each fire gets a fresh isolated session with no memory of previous runs." }
                },
                "required": ["name", "schedule", "action"]
            }),
        },
        ToolDefinition {
            name: "cron_list".to_string(),
            description: "List all scheduled/cron jobs for the current agent.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "cron_cancel".to_string(),
            description: "Cancel a scheduled/cron job by its ID.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "job_id": { "type": "string", "description": "The UUID of the cron job to cancel" }
                },
                "required": ["job_id"]
            }),
        },
        // --- Channel send tool (proactive outbound messaging) ---
        ToolDefinition {
            name: "channel_send".to_string(),
            description: "Send a message or media to a user on a configured channel (email, telegram, slack, etc). For email: recipient is the email address; optionally set subject. For media: set image_url, file_url, or file_path to send an image or file instead of (or alongside) text. Use thread_id to reply in a specific thread/topic. When recipient is omitted during message handling, the tool automatically replies to the original sender.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "channel": { "type": "string", "description": "Channel adapter name (e.g., 'email', 'telegram', 'slack', 'discord')" },
                    "recipient": { "type": "string", "description": "Platform-specific recipient identifier (email address, user ID, etc.). Omit only when replying from an inbound message context where the original sender is available." },
                    "subject": { "type": "string", "description": "Optional subject line (used for email; ignored for other channels)" },
                    "message": { "type": "string", "description": "The message body to send (required for text, optional caption for media)" },
                    "image_url": { "type": "string", "description": "URL of an image to send (supported on Telegram, Discord, Slack)" },
                    "file_url": { "type": "string", "description": "URL of a file to send as attachment" },
                    "file_path": { "type": "string", "description": "Local file path to send as attachment (reads from disk; use instead of file_url for local files)" },
                    "filename": { "type": "string", "description": "Filename for file attachments (defaults to the basename of file_path, or 'file')" },
                    "thread_id": { "type": "string", "description": "Thread/topic ID to reply in (e.g., Telegram message_thread_id, Slack thread_ts)" },
                    "account_id": { "type": "string", "description": "Optional account_id of the specific configured bot to send through (e.g., 'admin-bot'). When omitted, uses the first configured adapter for this channel." },
                    "poll_question": { "type": "string", "description": "Question for a poll (starts a poll, mutually exclusive with image_url/file_url/file_path)" },
                    "poll_options": { "type": "array", "items": { "type": "string" }, "description": "Answer options for the poll (2-10 items, required with poll_question)" },
                    "poll_is_quiz": { "type": "boolean", "description": "Set to true for a quiz mode (one correct answer)" },
                    "poll_correct_option": { "type": "integer", "description": "Index of the correct answer (0-based, for quiz mode)" },
                    "poll_explanation": { "type": "string", "description": "Explanation shown after answering (quiz mode)" }
                },
                "required": ["channel"]
            }),
        },
        // --- Hand tools (curated autonomous capability packages) ---
        ToolDefinition {
            name: "hand_list".to_string(),
            description: "List available Hands (curated autonomous packages) and their activation status.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "hand_activate".to_string(),
            description: "Activate a Hand — spawns a specialized autonomous agent with curated tools and skills.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "hand_id": { "type": "string", "description": "The ID of the hand to activate (e.g. 'researcher', 'clip', 'browser')" },
                    "config": { "type": "object", "description": "Optional configuration overrides for the hand's settings" }
                },
                "required": ["hand_id"]
            }),
        },
        ToolDefinition {
            name: "hand_status".to_string(),
            description: "Check the status and metrics of an active Hand.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "hand_id": { "type": "string", "description": "The ID of the hand to check status for" }
                },
                "required": ["hand_id"]
            }),
        },
        ToolDefinition {
            name: "hand_deactivate".to_string(),
            description: "Deactivate a running Hand and stop its agent.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "instance_id": { "type": "string", "description": "The UUID of the hand instance to deactivate" }
                },
                "required": ["instance_id"]
            }),
        },
        // --- A2A outbound tools ---
        ToolDefinition {
            name: "a2a_discover".to_string(),
            description: "Discover an external A2A agent by fetching its agent card from a URL. Returns the agent's name, description, skills, and supported protocols.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Base URL of the remote LibreFang/A2A-compatible agent (e.g., 'https://agent.example.com')" }
                },
                "required": ["url"]
            }),
        },
        ToolDefinition {
            name: "a2a_send".to_string(),
            description: "Send a task/message to an external A2A agent and get the response. Use agent_name to send to a previously discovered agent, or agent_url for direct addressing.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "message": { "type": "string", "description": "The task/message to send to the remote agent" },
                    "agent_url": { "type": "string", "description": "Direct URL of the remote agent's A2A endpoint" },
                    "agent_name": { "type": "string", "description": "Name of a previously discovered A2A agent (looked up from kernel)" },
                    "session_id": { "type": "string", "description": "Optional session ID for multi-turn conversations" }
                },
                "required": ["message"]
            }),
        },
        // --- TTS/STT tools ---
        ToolDefinition {
            name: "text_to_speech".to_string(),
            description: "Convert text to speech audio. Supports multiple providers (OpenAI, Gemini, MiniMax). Saves audio to workspace output/ directory.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "The text to convert to speech (max 4096 chars)" },
                    "voice": { "type": "string", "description": "Voice name (provider-specific). OpenAI: 'alloy', 'echo', 'fable', 'onyx', 'nova', 'shimmer'. Default: 'alloy'" },
                    "format": { "type": "string", "description": "Output format: 'mp3', 'opus', 'aac', 'flac', 'wav' (default: 'mp3')" },
                    "output_format": { "type": "string", "enum": ["mp3", "ogg_opus"], "description": "Final output format. 'ogg_opus' converts to OGG Opus via ffmpeg (required for WhatsApp voice notes); falls back to provider format if ffmpeg is unavailable or conversion fails. Default: 'mp3'" },
                    "provider": { "type": "string", "description": "Provider: 'openai', 'gemini', 'minimax'. Auto-detected if omitted." },
                    "model": { "type": "string", "description": "Model ID (provider-specific). OpenAI: 'tts-1', 'tts-1-hd'. Default varies by provider." },
                    "speed": { "type": "number", "description": "Playback speed (0.25-4.0). OpenAI only. Default: 1.0" }
                },
                "required": ["text"]
            }),
        },
        ToolDefinition {
            name: "speech_to_text".to_string(),
            description: format!("Transcribe audio to text using speech-to-text. Auto-selects Groq Whisper or OpenAI Whisper. Supported formats: {SUPPORTED_AUDIO_EXTS_DOC}."),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the audio file (relative to workspace)" },
                    "language": { "type": "string", "description": "Optional ISO-639-1 language code (e.g., 'en', 'es', 'ja')" }
                },
                "required": ["path"]
            }),
        },
        // --- Docker sandbox tool ---
        ToolDefinition {
            name: "docker_exec".to_string(),
            description: "Execute a command inside a Docker container sandbox. Provides OS-level isolation with resource limits, network isolation, and capability dropping. Requires Docker to be installed and docker.enabled=true.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The command to execute inside the container" }
                },
                "required": ["command"]
            }),
        },
        // --- Persistent process tools ---
        ToolDefinition {
            name: "process_start".to_string(),
            description: "Start a long-running process (REPL, server, watcher). Returns a process_id for subsequent poll/write/kill operations. Max 5 processes per agent.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The executable to run (e.g. 'python', 'node', 'npm')" },
                    "args": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Command-line arguments (e.g. ['-i'] for interactive Python)"
                    }
                },
                "required": ["command"]
            }),
        },
        ToolDefinition {
            name: "process_poll".to_string(),
            description: "Read accumulated stdout/stderr from a running process. Non-blocking: returns whatever output has buffered since the last poll.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "process_id": { "type": "string", "description": "The process ID returned by process_start" }
                },
                "required": ["process_id"]
            }),
        },
        ToolDefinition {
            name: "process_write".to_string(),
            description: "Write data to a running process's stdin. A newline is appended automatically if not present.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "process_id": { "type": "string", "description": "The process ID returned by process_start" },
                    "data": { "type": "string", "description": "The data to write to stdin" }
                },
                "required": ["process_id", "data"]
            }),
        },
        ToolDefinition {
            name: "process_kill".to_string(),
            description: "Terminate a running process and clean up its resources.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "process_id": { "type": "string", "description": "The process ID returned by process_start" }
                },
                "required": ["process_id"]
            }),
        },
        ToolDefinition {
            name: "process_list".to_string(),
            description: "List all running processes for the current agent, including their IDs, commands, uptime, and alive status.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        // --- Goal tracking tool ---
        ToolDefinition {
            name: "goal_update".to_string(),
            description: "Update a goal's status and/or progress. Use this to autonomously track your progress toward assigned goals.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "goal_id": { "type": "string", "description": "The goal's UUID to update" },
                    "status": { "type": "string", "enum": ["pending", "in_progress", "completed", "cancelled"], "description": "New status for the goal (optional)" },
                    "progress": { "type": "integer", "description": "Progress percentage 0-100 (optional)" }
                },
                "required": ["goal_id"]
            }),
        },
        // --- Workflow tools ---
        ToolDefinition {
            name: "workflow_run".to_string(),
            description: "Run a registered workflow pipeline end-to-end. Workflows are multi-step agent pipelines (e.g., bug-triage, code-review, test-generation). Accepts a workflow UUID or name.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "workflow_id": { "type": "string", "description": "The workflow UUID or registered name (e.g., 'bug-triage', 'code-review')" },
                    "input": { "type": "object", "description": "Optional input parameters to pass to the workflow's first step (JSON object)" }
                },
                "required": ["workflow_id"]
            }),
        },
        ToolDefinition {
            name: "workflow_list".to_string(),
            description: "List all registered workflow definitions. Returns an array of {id, name, description, step_count} objects sorted by name. Use this to discover available workflows before calling workflow_run.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
        ToolDefinition {
            name: "workflow_status".to_string(),
            description: "Get the current status of a workflow run. Returns run state (pending/running/paused/completed/failed), timing, output, error, and step details. Use the run_id returned by workflow_run.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "run_id": { "type": "string", "description": "The workflow run UUID returned by workflow_run" }
                },
                "required": ["run_id"]
            }),
        },
        ToolDefinition {
            name: "workflow_start".to_string(),
            description: "Start a workflow asynchronously (fire-and-forget). Returns the run_id immediately without waiting for completion. Use workflow_status to poll progress. Differs from workflow_run which blocks until the workflow finishes.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "workflow_id": { "type": "string", "description": "The workflow UUID or registered name (e.g., 'bug-triage', 'code-review')" },
                    "input": { "type": "object", "description": "Optional input parameters to pass to the workflow's first step (JSON object)" }
                },
                "required": ["workflow_id"]
            }),
        },
        ToolDefinition {
            name: "workflow_cancel".to_string(),
            description: "Cancel a running or paused workflow. Returns the run_id and final state on success. Returns an error if the run is not found or is already in a terminal state (completed, failed, cancelled).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "run_id": { "type": "string", "description": "The workflow run UUID to cancel" }
                },
                "required": ["run_id"]
            }),
        },
        // --- System time tool ---
        ToolDefinition {
            name: "system_time".to_string(),
            description: "Get the current date, time, and timezone. Returns ISO 8601 timestamp, Unix epoch seconds, and timezone info.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
        // --- Canvas / A2UI tool ---
        ToolDefinition {
            name: "canvas_present".to_string(),
            description: "Present an interactive HTML canvas to the user. The HTML is sanitized (no scripts, no event handlers) and saved to the workspace. The dashboard will render it in a panel. Use for rich data visualizations, formatted reports, or interactive UI.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "html": { "type": "string", "description": "The HTML content to present. Must not contain <script> tags, event handlers, or javascript: URLs." },
                    "title": { "type": "string", "description": "Optional title for the canvas panel" }
                },
                "required": ["html"]
            }),
        },
        // --- Artifact retrieval tool ---
        ToolDefinition {
            name: "read_artifact".to_string(),
            description: "Retrieve content from the artifact store. Use this when a previous tool result was truncated with a message like '[tool_result: … | sha256:… | … bytes | preview:]'. Pass the handle exactly as shown (e.g. \"sha256:abc…\"), an optional byte offset (default 0), and an optional length (default 4096, max 65536).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "handle": {
                        "type": "string",
                        "description": "Artifact handle from the spill stub, e.g. \"sha256:abc123…\" (64 hex chars after the prefix)."
                    },
                    "offset": {
                        "type": "integer",
                        "description": "Byte offset to start reading from (default 0)."
                    },
                    "length": {
                        "type": "integer",
                        "description": "Number of bytes to read (default 4096, max 65536)."
                    }
                },
                "required": ["handle"]
            }),
        },
        // --- Skill evolution tools ---
        ToolDefinition {
            name: "skill_evolve_create".to_string(),
            description: "Create a new prompt-only skill from a successful task approach. Use after completing a complex task (5+ tool calls) that involved trial-and-error or a non-trivial workflow worth reusing. The skill becomes available to all agents.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Skill name: lowercase alphanumeric with hyphens (e.g., 'csv-analysis', 'api-debugging')" },
                    "description": { "type": "string", "description": "One-line description of what this skill teaches (max 1024 chars)" },
                    "prompt_context": { "type": "string", "description": "Markdown instructions that will be injected into the system prompt when this skill is active. Should capture the methodology, pitfalls, and best practices discovered." },
                    "tags": { "type": "array", "items": { "type": "string" }, "description": "Tags for discovery (e.g., ['data', 'csv', 'analysis'])" }
                },
                "required": ["name", "description", "prompt_context"]
            }),
        },
        ToolDefinition {
            name: "skill_evolve_update".to_string(),
            description: "Rewrite a skill's prompt_context entirely. Use when the skill needs a major overhaul based on new learnings. Creates a rollback snapshot automatically.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Name of the existing skill to update" },
                    "prompt_context": { "type": "string", "description": "New Markdown instructions (full replacement)" },
                    "changelog": { "type": "string", "description": "Brief description of what changed and why" }
                },
                "required": ["name", "prompt_context", "changelog"]
            }),
        },
        ToolDefinition {
            name: "skill_evolve_patch".to_string(),
            description: "Make a targeted find-and-replace edit to a skill's prompt_context. Use when only a section needs fixing. Supports fuzzy matching (tolerates whitespace/indent differences).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Name of the existing skill to patch" },
                    "old_string": { "type": "string", "description": "Text to find in the current prompt_context (fuzzy-matched)" },
                    "new_string": { "type": "string", "description": "Replacement text" },
                    "changelog": { "type": "string", "description": "Brief description of what changed and why" },
                    "replace_all": { "type": "boolean", "description": "Replace all occurrences (default: false)" }
                },
                "required": ["name", "old_string", "new_string", "changelog"]
            }),
        },
        ToolDefinition {
            name: "skill_evolve_delete".to_string(),
            description: "Delete an agent-evolved skill. Only works on locally-created skills (not marketplace installs).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Name of the skill to delete" }
                },
                "required": ["name"]
            }),
        },
        ToolDefinition {
            name: "skill_evolve_rollback".to_string(),
            description: "Roll back a skill to its previous version. Use when a recent update degraded the skill's effectiveness.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Name of the skill to roll back" }
                },
                "required": ["name"]
            }),
        },
        ToolDefinition {
            name: "skill_evolve_write_file".to_string(),
            description: "Add a supporting file to a skill (references, templates, scripts, or assets). Use to enrich a skill with additional context like API docs, code templates, or example configurations.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Name of the skill to add the file to" },
                    "path": { "type": "string", "description": "Relative path under the skill directory (e.g., 'references/api.md', 'templates/config.yaml'). Must be under references/, templates/, scripts/, or assets/" },
                    "content": { "type": "string", "description": "File content to write" }
                },
                "required": ["name", "path", "content"]
            }),
        },
        ToolDefinition {
            name: "skill_evolve_remove_file".to_string(),
            description: "Remove a supporting file from a skill.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Name of the skill" },
                    "path": { "type": "string", "description": "Relative path of file to remove (e.g., 'references/old-api.md')" }
                },
                "required": ["name", "path"]
            }),
        },
        // --- Meta-tools: lazy tool loading (issue #3044) ---
        ToolDefinition {
            name: "tool_load".to_string(),
            description: "Load the full JSON schema for a tool by name. Call this before using a tool that is listed in the catalog but not yet declared with a full schema. The loaded tool becomes callable on the next turn.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Tool name to load (e.g., 'file_write', 'browser_navigate')" }
                },
                "required": ["name"]
            }),
        },
        ToolDefinition {
            name: "tool_search".to_string(),
            description: "Find tools by keyword. Returns matching tool names and one-line hints from the full catalog.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Keyword(s) to match against tool names and descriptions (e.g., 'read file', 'screenshot')" },
                    "limit": { "type": "integer", "description": "Max results (default 10)", "minimum": 1, "maximum": 50 }
                },
                "required": ["query"]
            }),
        },
    ]
}
