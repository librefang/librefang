# ADR-001: OpenFang Baseline — What We're Forking and Why

**Status**: Accepted
**Date**: 2026-03-14
**Authors**: Daniel Alberttis

## Version History

| Version | Date | Author | Changes |
|---------|------|--------|---------|
| 0.1 | 2026-03-14 | Daniel Alberttis | Initial baseline record |
| 0.2 | 2026-03-14 | Daniel Alberttis | Added: 40-adapter channel breakdown, webchat 10-panel detail, trigger system, workflow engine, 30 bundled templates, markdown stream chunker, Ed25519 spawn security |
| 0.3 | 2026-03-14 | Daniel Alberttis | Corrected memory limitation #1: O(n) unindexed scan, not absence of cosine similarity |
| 0.4 | 2026-03-14 | Daniel Alberttis | Scope restricted to upstream OpenFang v0.4.0 state only — no fork plans, no external dependencies |
| 0.5 | 2026-03-14 | Daniel Alberttis | Added §8–§14: Hands, OFP Wire Protocol, A2A, Audit Log, Session Management, Storage Layout, API Surface — sourced from direct source reads |
| 0.6 | 2026-03-14 | Daniel Alberttis | Fixed §7 (MCP is bidirectional — also a server); added §15–§19: built-in tool inventory, agent manifest/lifecycle types, capability system, security subsystem, OpenAI compat layer; §15→§20 |
| 0.7 | 2026-03-14 | Daniel Alberttis | Added §21–§24: testing infrastructure (1,767+ tests, CI/CD pipeline details), build system details (workspace resolver, rust-version, C dependency note), performance characteristics (cold start, runtime limits), migration compatibility (OpenClaw live, LangChain/AutoGPT planned) |
| 0.8 | 2026-03-14 | Daniel Alberttis | Corrected §14 route count (160 paths / 185+ HTTP methods); removed stale route entries (tasks, extensions, metering); added §25 Configuration System (45-field KernelConfig), §26 Daemon Lifecycle (PID file, stale detection, DaemonInfo) |
| 0.9 | 2026-03-14 | Daniel Alberttis | Added §27 Error Handling Strategy (OpenFangError 21 variants, HTTP status mapping), §28 Resource Enforcement (ResourceQuota defaults, MeteringEngine, ContextBudget), §29 Extension Architecture (WASM sandbox, HookRegistry, Skills runtimes, MCP templates), §30 Crate Dependency Diagram |
| 1.0 | 2026-03-14 | Daniel Alberttis | Source-verified corrections: OpenFangError 21 variants (not 22); KernelConfig 45 fields (not 44, workflows_dir present); 60 built-in tools (not ~50); HandDefinition has no version field; bundled_agents.rs is in openfang-cli (not openfang-kernel) |

---

## What This Project Is

**openfang-ai is a fork of OpenFang v0.4.0 with one primary architectural change: the SQLite vector backend in `SemanticStore` has been replaced with [ruvector](https://github.com/ruvnet/ruvector)'s `rvf-runtime` — a persistent HNSW vector store using the RVF (RuVector Format) binary format.**

Everything above the vector storage layer is OpenFang: the scheduling model, channels, tools, API, CLI, agent manifests, skills, hands, OFP wire protocol, and all 40+ operational stores. Only `SemanticStore` changed internals.

The ruvector ecosystem (`rvf-runtime`, `rvf-adapters/sona`, `rvf-federation`, `ruvllm`) defines the Phase 2+ roadmap — self-learning consolidation (ADR-009), intelligent LLM routing (ADR-010), and shared store hardening (ADR-008). These are features OpenFang consumes from ruvector as a library; we do not maintain the ruvector crates.

**What is NOT this project's domain:**
- The `openfang.sh` install script domain (external infra — not code)
- The ruvector crates themselves (`rvf-runtime`, `rvf-index`, `rvf-crypto` are vendored and pinned)
- The RuVix Cognition Kernel (upstream ruvector — documented as compatibility context in ADR-011)

---

## Context

openfang-ai begins as a fork of OpenFang v0.4.0 (`github.com/RightNow-AI/openfang`). This ADR documents what OpenFang is at the point of forking: its capabilities, architecture, crate structure, and the specific limitations that motivate the changes described in ADR-002 onward. It is a permanent baseline record — a snapshot of the starting point before any modifications are made.

---

## Decision

### 1. What OpenFang Is

OpenFang is an open source AI agent operating system written in Rust. It runs AI agents that can hold conversations, use tools, execute skills, connect to messaging channels, coordinate with other agents, and manage their own memory. It is designed to run as a daemon process, exposing an HTTP/WebSocket API and a CLI.

OpenFang v0.4.0 is a production-capable agent runtime with a mature feature set. It is not a prototype.

### 2. Crate Structure (13 crates, v0.4.0)

| Crate | Description |
|-------|-------------|
| `openfang-types` | Core types and traits: agent configs, memory types, capability definitions |
| `openfang-memory` | Memory substrate: SQLite-backed structured + semantic store, session management, consolidation |
| `openfang-runtime` | Agent execution loop: tool runner, embedding driver, MCP tool dispatch |
| `openfang-kernel` | Core kernel: boot, config, MCP client, channel orchestration, RBAC, cron, metering, background tasks |
| `openfang-api` | HTTP/WebSocket API server: REST routes for agents, sessions, memory, channels, skills, tasks |
| `openfang-channels` | Channel bridge: 40 adapters across 4 waves — core (15), high-value (8), enterprise (9), niche (8) |
| `openfang-skills` | Skill system: registry, loader, OpenClaw marketplace compatibility, hot-reload |
| `openfang-wire` | OFP (OpenFang Protocol): agent-to-agent peer networking |
| `openfang-hands` | Hands: curated autonomous capability packages (autonomous action bundles) |
| `openfang-extensions` | Extension system: one-click MCP server setup, credential vault, OAuth2 PKCE |
| `openfang-migrate` | Migration engine: import from other agent frameworks |
| `openfang-desktop` | Tauri 2.0 native desktop application |
| `openfang-cli` | CLI binary: `openfang` command |

### 3. Memory Layer in Detail

The memory layer (`openfang-memory`) is the primary target of the fork. Its upstream v0.4.0 implementation:

**Persistence**: SQLite via `rusqlite = { version = "0.31", features = ["bundled"] }` — the database is bundled (compiles libsqlite3 from source), creating a C dependency.

**Source files and their roles:**

| File | Role |
|------|------|
| `substrate.rs` | `MemorySubstrate` struct — composes all memory stores, initialised during kernel boot |
| `structured.rs` | `StructuredStore` — SQLite KV store for agent state, preferences, counters |
| `semantic.rs` | `SemanticStore` — SQLite BLOB embedding storage + cosine re-ranking (O(n) linear scan) |
| `session.rs` | Session management — turn-by-turn conversation history, LLM compaction |
| `knowledge.rs` | `KnowledgeStore` — higher-level knowledge retrieval |
| `consolidation.rs` | `ConsolidationEngine` — periodic confidence decay (`decay_rate: f32`, `consolidation_interval_hours: u64`) |
| `migration.rs` | Schema migrations |
| `usage.rs` | Memory usage tracking |
| `lib.rs` | Crate exports |

**How memory recall works** (`openfang-runtime/src/agent_loop.rs:147-191`):
1. If an `EmbeddingDriver` is configured, embed the user's message and call `memory.recall_with_embedding_async(query_vec, MemoryFilter { agent_id, ... })`
2. Fetch `limit × 10` candidates from SQLite ordered by recency, then re-rank by cosine similarity in memory
3. If no embedding driver is configured, fall back to `memory.recall()` (LIKE-based text search)
4. Retrieved memories are injected into the system prompt before the LLM call

**How memories are stored** (`agent_loop.rs:459-501`):
1. After the LLM responds, format `"User asked: {msg}\nI responded: {response}"` as the interaction text
2. If embedding driver available: `memory.remember_with_embedding_async(text, MemorySource::Conversation, "episodic")`
3. Otherwise: `memory.remember(text, MemorySource::Conversation, "episodic")`

**`MemoryConfig`** (`openfang-types/src/config.rs:1470-1491`):
- `embedding_provider`: OpenAI | Ollama — external API call required for embeddings
- `consolidation_interval_hours: u64` — default 24h
- `decay_rate: f32` — default 0.1

**Limitations of the upstream memory layer:**
1. **Unindexed O(n) vector scan** — cosine similarity re-ranking fetches a broad candidate set then sorts in memory. No ANN index. Recall latency grows linearly with memory count; degrades badly at thousands of memories.
2. **No self-learning** — memories accumulate but the system never extracts patterns, trains on them, or adapts its retrieval behaviour. The `ConsolidationEngine` only applies confidence decay; it does not learn.
3. **C dependency** — `rusqlite` bundles libsqlite3. Complicates cross-compilation, increases binary size, breaks pure-Rust.
4. **No scope isolation** — episodic conversation memories share the search space with semantic and procedural memories. No filtering by memory type at recall time.
5. **Limited source attribution** — memories tagged `MemorySource::Conversation` only. No `Document`, `Observation`, `Inference`, `UserProvided`, `System` variants.
6. **No soft-delete** — `delete_memory` hard-removes the row; no audit trail.
7. **No access tracking** — `access_count` and `last_accessed_at` not recorded. Ranking signal is recency-of-write only.
8. **External embedding API required** — every recall requires a network call to OpenAI or Ollama if embeddings are used. No local embedding option.

### 4. Agent Runtime in Detail

The runtime (`openfang-runtime`) is capable and well-designed. Key characteristics:

- `run_agent_loop` / `run_agent_loop_streaming` — the main agent execution paths
- Full MCP client: connects to external MCP servers over stdio or SSE; dispatches tool calls to them
- Tool runner: executes built-in tools + MCP tools + skill tools
- `EmbeddingDriver` abstraction: OpenAI and Ollama providers auto-detected from config
- Session compaction: LLM-generated rolling summaries for long conversations
- **Markdown-aware stream chunking** (`stream_chunker.rs`): buffers streaming output and flushes at natural break points (paragraph > newline > sentence). Never splits inside fenced code blocks.
- **Taint tracking** (`openfang-types/src/taint.rs`): lattice-based information flow model. `TaintLabel` variants: `ExternalNetwork`, `UserInput`, `Pii`, `Secret`, `UntrustedAgent`. Applied at shell and network sinks to block prompt injection and credential exfiltration.
- **ExecPolicy** (`openfang-types/src/config.rs`): per-agent shell execution security. Three modes: `Deny`, `Allowlist` (default), `Full`. Configurable safe_bins and command allowlist. Timeout and max-output-bytes enforced on every shell call.
- **Canvas/A2UI**: `canvas_present` tool renders structured HTML output to an agent's canvas viewport — enables agent-to-agent UI surfaces.

### 5. Kernel in Detail

The kernel (`openfang-kernel`) manages the full daemon lifecycle:

- Boot: loads config, initialises `MemorySubstrate`, connects MCP servers, registers channels
- Background tasks: consolidation loop, config hot-reload (30s poll), heartbeat, metering
- RBAC: capability-based permissions per agent (`MemoryRead`, `MemoryWrite`, `ToolUse`, etc.)
- Cron: scheduled job execution
- Approval gate: human-in-the-loop confirmation for sensitive operations
- WhatsApp gateway management
- **Trigger system** (`openfang_kernel::triggers`): `TriggerPattern`-based event routing with `TriggerId` — reactive agent activation without polling
- **Workflow engine** (`openfang_kernel::workflow`): multi-step pipelines with `Workflow`, `WorkflowStep`, `StepAgent`, `StepMode`, `ErrorMode`; exposed via API
- **30 bundled agent templates** (`bundled_agents.rs` in `openfang-cli`): embedded at compile time via `include_str!`. Includes `analyst`, `architect`, `assistant`, and 27 more. Installed to `~/.openfang/agents/` on first run. Available immediately after install.

### 6. Channel Support

OpenFang v0.4.0 ships **40 channel adapters** (`openfang-channels/src/lib.rs`), organized by wave:

**Wave 1 — Core:**

| Channel | Notes |
|---------|-------|
| WhatsApp | Via Baileys-based gateway (`whatsapp_gateway.rs` in kernel) |
| Telegram | Native |
| Discord | Native |
| Slack | Socket Mode WebSocket (app token) + Web API (bot token) |
| Signal | Native |
| Matrix | Native |
| Email | Native |
| Teams | Native |
| Mattermost | WebSocket API v4 + REST API v4 |
| IRC | Native |
| Google Chat | REST API with service account JWT auth + webhook listener |
| RocketChat | Native |
| Twitch | Native |
| XMPP | Native |
| Zulip | Native |

**Wave 2 — High-value:**

| Channel | Notes |
|---------|-------|
| Bluesky | Native |
| Feishu | Native |
| LINE | Messaging API v2, HMAC-SHA256 webhook signature verification |
| Mastodon | Native |
| Messenger | Native |
| Reddit | Native |
| Revolt | Native |
| Viber | Native |

**Wave 3 — Enterprise & community:**
Flock, Guilded, Keybase, Nextcloud, Nostr, Pumble, Threema, Twist, Webex

**Wave 4 — Niche & differentiating:**
DingTalk, Discourse, Gitter, Gotify, LinkedIn, Mumble, ntfy, Webhook (generic)

All adapters convert platform-native messages into a unified `ChannelMessage` event consumed by the kernel.

**WebChat (embedded)**: Single-binary dashboard assembled at compile time from `static/` source files. All vendor libraries (Alpine.js, marked.js, highlight.js, Chart.js) bundled locally — no CDN dependency.

- Alpine.js SPA with hash-based routing, **10 panels**: overview, chat, agents, memory browser, workflows, audit log, and more
- Dark/light theme toggle, responsive collapsible sidebar
- Markdown rendering + syntax highlighting, WebSocket streaming with HTTP fallback
- ETag-based caching (`openfang-{VERSION}`)

### 7. MCP Architecture

OpenFang is both an MCP **client** and an MCP **server**:

- **MCP client** (primary role): connects to external MCP servers and exposes their tools to agents. Configured in `openfang.toml`; supports stdio (subprocess) and SSE (HTTP) transports.
- **MCP server**: exposes OpenFang's own built-in tools to external MCP clients (Claude Desktop, VS Code, etc.) via `openfang-runtime/src/mcp_server.rs`. Protocol version `2024-11-05`. Implements `initialize`, `tools/list`, and `tools/call` methods. Stateless handler wired to a stdio transport by the CLI.

The `openfang-extensions` crate wraps MCP server lifecycle: one-click install, credential vault, OAuth2 PKCE for services that require it.

**Agent spawn security**: `SpawnRequest` optionally accepts a `signed_manifest` (Ed25519-signed JSON envelope). When present, the signature is verified before the agent is started. Agents can also be spawned from named templates in `~/.openfang/agents/{template}/agent.toml`.

### 8. Hands

Hands (`openfang-hands`) are OpenFang's marketplace-distributed autonomous agent configurations.

**Definition** (from `openfang-hands/src/lib.rs`):
> "A Hand is a pre-built, domain-complete agent configuration that users activate from a marketplace. Unlike regular agents (you chat with them), Hands work for you (you check in on them)."

Hands bundle: a pre-configured agent identity, tool/skill selections, scheduling rules, and runtime settings into a single installable unit. A user installs a Hand and it begins operating autonomously within its configured scope.

**Key types:**

| Type | Role |
|------|------|
| `HandDefinition` | The marketplace listing: id, name, description, category, tools, skills, requires, settings, agent config, dashboard schema |
| `HandInstance` | A running instance: which agent id it owns, current lifecycle state, settings values |
| `HandRegistry` | In-process registry of installed Hands — backed by `hand_state.json` on disk |
| `HandRequirement` | A declared dependency (skill name, tool name, or MCP server) that must be satisfied before activation |
| `HandSetting` | Typed configuration key with default, validation, and description |
| `HandMetric` | Named numeric counter or gauge exposed through the Hand's dashboard |
| `HandDashboard` | Aggregated view: metrics, recent activity, agent status, setting summary |
| `HandAgentConfig` | The agent.toml content that a Hand installs when activated |

**Lifecycle states**: `Available → Installing → Ready → Active → Paused → Deactivating → Deactivated`

**Bundled hands** (`bundled.rs`): Hand definitions embedded at compile time via `include_str!` — same pattern as bundled agent templates.

**HTTP routes** (14 routes in `openfang-api`):
- `GET /api/hands` — list available Hands
- `GET /api/hands/:id` — Hand definition detail
- `POST /api/hands/:id/install` — install a Hand (resolves requirements)
- `GET /api/hands/:id/check-deps` — check whether all requirements are satisfied
- `POST /api/hands/:id/activate` — start the Hand's agent
- `POST /api/hands/:id/pause` — suspend without deactivating
- `POST /api/hands/:id/resume` — resume from paused
- `POST /api/hands/:id/deactivate` — stop and clean up
- `GET /api/hands/:id/status` — current lifecycle state + health
- `GET /api/hands/:id/settings` — current setting values
- `PUT /api/hands/:id/settings` — update settings
- `GET /api/hands/:id/stats` — metrics and counters
- `GET /api/hands/:id/dashboard` — full dashboard view
- `GET /api/hands/:id/browser` — browser-accessible dashboard (HTML)

---

### 9. OFP Wire Protocol

The `openfang-wire` crate implements OFP (OpenFang Protocol) — cross-machine agent networking.

**What it is** (from `openfang-wire/src/lib.rs`):
> "Cross-machine agent discovery, authentication, and communication over TCP connections using a JSON-RPC framed protocol."

**Transport**: TCP, length-prefixed JSON-RPC frames.

**Authentication** (`peer.rs`): HMAC-SHA256 on the handshake message. The peer's identity is validated before any messages are exchanged.

**Replay protection**: `NonceTracker` — DashMap of seen nonces with insertion timestamps. 5-minute replay window. Nonces outside the window are rejected. Stale nonces (> 5 min) are evicted on each check.

**Key types:**

| Type | Role |
|------|------|
| `PeerNode` | A remote OpenFang instance: address, peer id, public key, last-seen timestamp |
| `PeerRegistry` | In-process registry of known peers; supports discovery queries |
| `WireMessage` | Envelope: message type, payload, sender id, nonce, HMAC |
| `WireRequest` | Outbound RPC request to a remote agent |
| `WireResponse` | Response to a `WireRequest`, including error variants |
| `PeerHandle` | Trait for routing remote messages through the kernel — implemented by the kernel handle |

**Agent-to-agent capabilities via OFP**: spawn tasks on remote agents, query remote agent state, relay tool calls across machines.

---

### 10. A2A (Agent-to-Agent Internal)

Distinct from OFP (which is TCP-based cross-machine networking), A2A is the HTTP-based in-process agent discovery and tasking layer.

**Routes** (`openfang-api/src/routes.rs`):
- `GET /api/a2a/agents` — list all running agents visible to this node
- `POST /api/a2a/discover` — trigger discovery of agents (local + OFP peers)
- `POST /api/a2a/send` — send a message to another agent by id
- `GET /api/a2a/tasks/{id}/status` — poll a cross-agent task by task id

**Comms layer** (lower-level event bus under A2A):
- `GET /api/comms/topology` — current agent communication graph
- `GET /api/comms/events` — SSE stream of inter-agent events
- `POST /api/comms/send` — raw comms message (lower level than A2A send)
- `POST /api/comms/task` — delegate a task via the comms bus

**Relationship**: OFP (`openfang-wire`) handles TCP peer connections between OpenFang instances on different machines. A2A routes handle agent coordination within a single instance and across OFP-connected peers at the HTTP API layer.

---

### 11. Audit Log

`openfang-runtime/src/audit.rs` — tamper-evident audit trail for all significant agent actions.

**Implementation**: Merkle hash chain. Each entry's hash is `SHA-256(entry_fields || previous_hash)`. The chain cannot be silently modified without invalidating all subsequent hashes.

**Persistence**: SQLite `audit_entries` table (schema V8 in the migration sequence). Written on every auditable action.

**`AuditAction` enum** (all variants, from `audit.rs`):
- `AgentSpawn` — agent started
- `AgentKill` — agent stopped or crashed
- `AgentMessage` — inbound user message
- `ToolInvoke` — tool invoked by agent
- `CapabilityCheck` — RBAC capability check performed
- `MemoryAccess` — any memory read or write operation
- `FileAccess` — filesystem access by agent
- `NetworkAccess` — outbound network call by agent
- `ShellExec` — shell command executed by agent
- `AuthAttempt` — authentication attempt (API key, peer handshake)
- `WireConnect` — OFP peer connection established
- `ConfigChange` — runtime configuration modified

**`AuditEntry` struct**: `id: Uuid`, `action: AuditAction`, `agent_id: Option<String>`, `timestamp: DateTime<Utc>`, `hash: String` (SHA-256 hex), `prev_hash: String`.

**WebChat exposure**: The audit log panel (one of the 10 WebChat panels) reads from the audit trail and displays it in the browser dashboard.

---

### 12. Session Management

`openfang-memory/src/session.rs` — turn-by-turn conversation history for each agent.

**`Session` struct**:
```
id: SessionId          — UUID
agent_id: AgentId      — which agent owns this session
messages: Vec<Message> — full turn history
context_window_tokens: u64 — running token count
label: Option<String>  — user-assigned label for the session
```

**`SessionStore`**: SQLite-backed via `Arc<Mutex<Connection>>`. One session per row; messages stored as JSON.

**Multi-session**: Each agent can have multiple sessions. The API allows creating, switching, and listing sessions per agent.

**LLM compaction**: When `context_window_tokens` approaches the configured limit, the runtime calls the LLM to produce a rolling summary. The summary replaces the oldest turns in the message list. The full history remains on disk; only the in-context window is compacted.

**Session labels**: Users can label sessions (e.g., `"tax-research-2026"`) for retrieval. Used by the WebChat session browser.

**HTTP routes**: `POST /api/agents/:id/sessions`, `GET /api/agents/:id/sessions`, `GET /api/agents/:id/sessions/:sid`, `DELETE /api/agents/:id/sessions/:sid`, plus session-switch and session-compact endpoints.

---

### 13. Storage Layout

All OpenFang runtime state lives under `~/.openfang/` by default. Override with `OPENFANG_HOME` environment variable.

```
~/.openfang/
├── config.toml / openfang.toml  ← main configuration file
├── .env                          ← secrets (API keys, tokens) — NOT committed
├── openfang.db                   ← SQLite: sessions, audit_entries, structured KV, semantic embeddings
├── cron_jobs.json                ← persisted cron job definitions
├── custom_models.json            ← user-defined LLM model aliases
├── hand_state.json               ← HandRegistry persistence (installed Hands + lifecycle state)
├── skills/                       ← installed skills (each in its own subdirectory)
│   └── {skill-name}/
│       ├── skill.toml
│       └── ... (skill files)
├── workflows/                    ← persisted workflow definitions
│   └── {workflow-id}.json
├── whatsapp-gateway/             ← WhatsApp Baileys gateway state and session data
│   └── ...
└── agents/                       ← named agent templates (for Ed25519-signed spawns)
    └── {template-name}/
        └── agent.toml
```

**SQLite database** (`openfang.db`): Holds sessions (`session_entries`), audit log (`audit_entries`), structured KV (`kv_store`), and semantic memory with embeddings (`memories`). Single file — no separate database process.

**Config precedence**: `OPENFANG_HOME` env → `--config` CLI flag → `~/.openfang/config.toml` → built-in defaults.

---

### 14. API Surface

`openfang-api` exposes **~160 distinct route paths / ~185 HTTP method+path combinations** across two axum router chunks (the API server splits into two `Router` chains to stay within axum's type-nesting limit).

| Domain | Route prefix | Count | Notes |
|--------|-------------|-------|-------|
| Agents | `/api/agents` | ~25 | CRUD, spawn, kill, mode, model, tools, skills, MCP servers, identity, config, clone, files, deliveries, upload, history, compaction, streaming chat |
| Sessions | `/api/agents/{id}/sessions`, `/api/sessions` | ~8 | Multi-session per agent, switch, label, compact, delete |
| Memory (KV) | `/api/memory/agents/{id}/kv` | 4 | Structured KV get/set/delete/list — **the only memory HTTP surface** |
| Memory (semantic) | — | 0 | **No HTTP routes.** Semantic recall is internal to `agent_loop.rs`. |
| Channels | `/api/channels` | 6 | List, configure, remove, test, reload, WhatsApp QR flow |
| Skills | `/api/skills`, `/api/marketplace`, `/api/clawhub` | ~10 | List, install, uninstall, create; ClawHub browse/search/detail/install |
| Hands | `/api/hands` | ~14 | Install, activate, check-deps, install-deps, settings, pause/resume, deactivate, stats, browser (see §8) |
| Workflows | `/api/workflows` | 5 | CRUD + run + run history |
| Triggers | `/api/triggers` | 2 | CRUD |
| Schedules / Cron | `/api/schedules`, `/api/cron/jobs` | ~8 | Two overlapping schedule abstractions — schedules (one-time/recurring) and cron jobs (crontab-syntax) |
| OFP/Comms | `/api/peers`, `/api/comms` | 6 | Peer list, topology, event log, event stream (SSE), raw send, task send |
| A2A | `/api/a2a`, `/.well-known/agent.json`, `/a2a/` | ~10 | Inbound task receive + outbound agent discovery/send/status (see §10) |
| Integrations | `/api/integrations` | 7 | List available/active, add, remove, reconnect, health, reload |
| Audit | `/api/audit` | 2 | Recent entries, chain verify |
| Usage / Budget | `/api/usage`, `/api/budget` | ~8 | Token usage by model/day/summary; budget status + per-agent limits |
| Models / Providers | `/api/models`, `/api/providers` | ~12 | List models, custom aliases, provider key/URL/test, GitHub Copilot OAuth |
| Approvals | `/api/approvals` | 3 | List, approve, reject |
| Config | `/api/config` | 4 | Get, schema, set, reload |
| Pairing | `/api/pairing` | 5 | Request, complete, list devices, remove device, notify |
| MCP HTTP | `/mcp` | 1 | Exposes MCP protocol over HTTP (see §7) |
| Auth | `/api/auth` | 3 | Login, logout, check |
| System | `/api/health`, `/api/status`, `/api/version`, `/api/security`, `/api/shutdown`, `/api/logs/stream` | 6 | Health (plain + detail), version, security dashboard, graceful shutdown, SSE log tail |
| Bindings | `/api/bindings` | 2 | List/add, remove by index |
| Commands | `/api/commands` | 1 | Dynamic slash command menu for webchat |
| Migrate | `/api/migrate` | 3 | Detect, scan, run |
| WebChat / Static | `/`, `/logo.png`, `/favicon.ico` | 3 | Embedded SPA + assets |
| WebSocket | `/api/agents/{id}/ws` | 1 | Real-time streaming chat + events |
| OpenAI compat | `/v1/chat/completions`, `/v1/models` | 2 | Drop-in endpoint — `model` resolves to OpenFang agent; SSE streaming + non-streaming |
| Webhooks | `/hooks/wake`, `/hooks/agent` | 2 | External event injection |

**GCRA rate limiter** (`openfang-api/src/rate_limiter.rs`): cost-aware leaky-bucket per IP, 500 tokens/min. Per-route token costs: health check = 1, agent spawn = 50, agent message = 30.

**Key architectural fact**: Semantic memory has **no HTTP API surface**. The only memory routes are 4 structured KV operations. All semantic recall happens inside `agent_loop.rs` and is never exposed externally. This is intentional — semantic memory is an internal agent capability, not a user-queryable store.

---

### 15. Built-in Tool Inventory

`openfang-runtime/src/tool_runner.rs` — 60 built-in tools dispatched by name. All enforce capability grants before execution. An approval gate checks `requires_approval(tool_name)` before every tool runs.

| Group | Tools |
|-------|-------|
| Filesystem | `file_read`, `file_write`, `file_list`, `apply_patch` |
| Web | `web_fetch` (SSRF-protected, taint-checked), `web_search` (multi-provider, DDG) |
| Shell | `shell_exec` (metacharacter check → ExecPolicy → taint heuristic) |
| Inter-agent | `agent_send`, `agent_spawn`, `agent_list`, `agent_kill`, `agent_find` |
| Memory | `memory_store`, `memory_recall` |
| Collaboration | `task_post`, `task_claim`, `task_complete`, `task_list`, `event_publish` |
| Scheduling | `schedule_create`, `schedule_list`, `schedule_delete` |
| Cron | `cron_create`, `cron_list`, `cron_cancel` |
| Knowledge graph | `knowledge_add_entity`, `knowledge_add_relation`, `knowledge_query` |
| Media | `image_analyze`, `image_generate`, `media_describe`, `media_transcribe` |
| TTS / STT | `text_to_speech`, `speech_to_text` |
| Browser (Chrome/Chromium) | `browser_navigate`, `browser_click`, `browser_type`, `browser_screenshot`, `browser_read_page`, `browser_scroll`, `browser_wait`, `browser_back`, `browser_run_js`, `browser_close` |
| Sandbox | `docker_exec` |
| Persistent processes | `process_start`, `process_poll`, `process_write`, `process_kill`, `process_list` |
| Hands | `hand_list`, `hand_activate`, `hand_status`, `hand_deactivate` |
| A2A outbound | `a2a_discover`, `a2a_send` |
| Channel send | `channel_send` (proactive outbound messaging to any configured channel adapter) |
| Location / time | `location_get`, `system_time` |
| Canvas / A2UI | `canvas_present` (structured HTML output to agent's canvas viewport) |

**Inter-agent call depth**: `MAX_AGENT_CALL_DEPTH = 5` enforced via Tokio task-local counter — prevents infinite A→B→C→... recursion.

**Tool name normalization**: `tool_compat.rs::normalize_tool_name` resolves LLM-hallucinated aliases (e.g., `"fs-write"` → `"file_write"`) to canonical OpenFang names before capability enforcement.

---

### 16. Agent Manifest and Lifecycle Types

`openfang-types/src/agent.rs` — all types describing an agent's identity, permissions, and scheduling.

**`AgentManifest`** (key fields):

| Field | Type | Description |
|-------|------|-------------|
| `name` | `String` | Human-readable name |
| `version` | `String` | Semantic version |
| `module` | `String` | `builtin:chat`, WASM path, or Python path |
| `schedule` | `ScheduleMode` | When the agent runs |
| `model` | `ModelConfig` | LLM provider, model id, max_tokens, temperature, system_prompt |
| `fallback_models` | `Vec<FallbackModel>` | Tried in order if primary model fails |
| `routing` | `Option<ModelRoutingConfig>` | Auto-select cheap/mid/expensive model by token count |
| `resources` | `ResourceQuota` | Memory, CPU, tool calls/min, network/hr, cost limits |
| `priority` | `Priority` | Scheduling priority |
| `capabilities` | `ManifestCapabilities` | Parsed into `Capability` enum by kernel |
| `profile` | `Option<ToolProfile>` | Named tool preset — expands to tool list + derived capabilities |
| `tool_allowlist` / `tool_blocklist` | `Vec<String>` | Per-agent tool overrides (applied after profile) |
| `exec_policy` | `Option<ExecPolicy>` | Per-agent shell execution policy (overrides global) |
| `autonomous` | `Option<AutonomousConfig>` | Guardrails for 24/7 continuous agents |
| `mcp_servers` | `Vec<String>` | Allowlist of MCP servers this agent may use (empty = all) |
| `generate_identity_files` | `bool` | Default: `true` — creates SOUL.md / USER.md in agent workspace on spawn |

**`AgentState`** (lifecycle states):

| State | Meaning |
|-------|---------|
| `Created` | Spawned but not yet started |
| `Running` | Actively processing events |
| `Suspended` | Paused, not consuming events |
| `Terminated` | Stopped permanently |
| `Crashed` | Failed, awaiting recovery |

**`AgentMode`** (runtime permission mode):

| Mode | Tools available |
|------|----------------|
| `Observe` | None |
| `Assist` | Read-only: `file_read`, `file_list`, `memory_recall`, `web_fetch`, `web_search`, `agent_list` |
| `Full` (default) | All granted tools |

**`ScheduleMode`**: `Reactive` (default, event-driven), `Periodic { cron }`, `Proactive { conditions }`, `Continuous { check_interval_secs }` (default 60s).

**`ToolProfile`** (named presets):

| Profile | Tool set |
|---------|---------|
| `Minimal` | `file_read`, `file_list` |
| `Coding` | Filesystem + `shell_exec` + `web_fetch` |
| `Research` | Web + filesystem |
| `Messaging` | Agent comms + memory |
| `Automation` | Filesystem + shell + web + agents + memory |
| `Full` (default) | `*` |

**`ResourceQuota`** defaults: 256 MB memory, 30 s CPU, 60 tool calls/min, 100 MB network/hr. Cost limits (per-hour / per-day / per-month) default 0.0 (unlimited).

**`ModelRoutingConfig`** defaults: `simple_threshold = 100` tokens → `claude-haiku-4-5-20251001`; `complex_threshold = 500` tokens → `claude-sonnet-4-20250514`.

**`AutonomousConfig`**: `quiet_hours` (cron expression), `max_iterations: 50`, `max_restarts: 10`, `heartbeat_interval_secs: 30`, optional `heartbeat_channel`.

---

### 17. Capability System

`openfang-types/src/capability.rs` — capability-based security. Capabilities are **immutable after agent creation** and enforced at the kernel level by `CapabilityManager` (DashMap of `AgentId → Vec<Capability>`).

**`Capability` enum** (all variants):

| Group | Variants |
|-------|---------|
| Filesystem | `FileRead(glob)`, `FileWrite(glob)` |
| Network | `NetConnect(host:port pattern)`, `NetListen(port: u16)` |
| Tools | `ToolInvoke(tool_id)`, `ToolAll` (dangerous — explicit grant required) |
| LLM | `LlmQuery(model pattern)`, `LlmMaxTokens(u64)` |
| Agent | `AgentSpawn`, `AgentMessage(target pattern)`, `AgentKill(target pattern)` |
| Memory | `MemoryRead(scope pattern)`, `MemoryWrite(scope pattern)` |
| Shell | `ShellExec(command pattern)`, `EnvRead(var pattern)` |
| OFP | `OfpDiscover`, `OfpConnect(peer pattern)`, `OfpAdvertise` |
| Economic | `EconSpend(max_usd: f64)`, `EconEarn`, `EconTransfer(target pattern)` |

Pattern matching uses glob syntax (`"*.openai.com:443"`, `"self.*"`, `"*"`). `CapabilityManager::check()` iterates grants; first match wins. `revoke_all()` is called on agent termination.

---

### 18. Security Subsystem

**Taint tracking** (`openfang-types/src/taint.rs`): lattice-based information flow model to prevent confused-deputy attacks. `TaintLabel` values: `ExternalNetwork`, `UserInput`, `Pii`, `Secret`, `UntrustedAgent`.

Two enforcement points in `tool_runner.rs`:
- **`shell_exec`**: Layer 1 — shell metacharacter injection (backticks, `$(`, `${`, semicolons) **always** blocked, regardless of mode. Layer 2 — heuristic patterns (`curl`, `wget`, `| sh`, `| bash`, `base64 -d`, `eval`) blocked unless `ExecPolicy.mode = Full`.
- **`web_fetch` / `browser_navigate`**: blocks URLs containing `api_key=`, `token=`, `secret=`, `password=` in query parameters to prevent credential exfiltration via LLM tool call.

**`ExecPolicy`** (`openfang-types/src/config.rs`):

| Mode | Behaviour |
|------|-----------|
| `Deny` | Block all `shell_exec` calls |
| `Allowlist` (default) | Only `safe_bins` or `allowed_commands` permitted |
| `Full` | Allow all commands (dev / hand agents only) |

Default `safe_bins`: `sleep`, `true`, `false`, `cat`, `sort`, `uniq`, `cut`, `tr`, `head`, `tail`, `wc`, `date`, `echo`, `printf`, `basename`, `dirname`, `pwd`, `env`. Timeout: 30 s. Max output: 100 KB. No-output idle timeout: 30 s.

**Approval gate** (kernel): `requires_approval(tool_name)` checked before every tool execution. Blocks until human responds; timeout = denied.

**GCRA rate limiter** (`openfang-api/src/rate_limiter.rs`): cost-aware leaky-bucket per IP. 500 tokens/min budget. Per-route costs: health = 1, agent spawn = 50, agent message = 30.

**Session auth** (`openfang-api/src/session_auth.rs`): HMAC-SHA256 signed stateless tokens for the web dashboard. No server-side session store required.

---

### 19. OpenAI Compatibility Layer

`openfang-api/src/openai_compat.rs` — drop-in OpenAI-compatible endpoint.

**Route**: `POST /v1/chat/completions`

**`model` field resolution**: agent name → UUID → `openfang:<name>` prefix → active agent list.

**Request mapping**: `ChatCompletionRequest` → OpenFang `Message` sequence. Content may be:
- Plain string (text only)
- Array of content parts: `{ "type": "text" }` or `{ "type": "image_url", "image_url": { "url": "..." } }`

Parameters: `max_tokens`, `temperature`, `stream`.

**Response modes**: `stream: false` → blocking JSON body; `stream: true` → Server-Sent Events (SSE), token-by-token.

**Use case**: Any OpenAI SDK client can point `base_url` at the OpenFang instance and use agents as model endpoints with no client-side code changes.

---

### 20. What OpenFang Does Well

The agent runtime, channel system, MCP client, skill system, workflow engine, trigger system, and extension framework are mature and well-designed. The fork preserves all of this unchanged.

What the fork targets is narrow: **the memory layer only**.

---

### 21. Testing Infrastructure

OpenFang v0.4.0 ships with a substantial test suite integrated across all crates.

**Test counts** (from `grep -r '#\[test\]' --include='*.rs'`):
- **1,767+** `#[test]` functions across source files
- Tests are inline in source modules (Rust idiomatic `#[cfg(test)]` blocks), not in a separate test-only tree

**CI Pipeline** (`.github/workflows/ci.yml`):

| Job | What it runs |
|-----|-------------|
| `check` | `cargo check --workspace` — fast compile-only gate |
| `test` | `cargo test --workspace` on ubuntu/macos/windows matrix |
| `clippy` | `cargo clippy -- -D warnings` (warnings are errors) |
| `fmt` | `cargo fmt --check` |
| `audit` | `cargo audit` — dependency vulnerability scan (RustSec advisory DB) |
| `secrets` | `trufflehog filesystem` with `--only-verified --fail` |
| `install-smoke` | Validates the shell (`install.sh`) and PowerShell (`install.ps1`) installers |

**Platform matrix**: ubuntu-latest, macos-latest, windows-latest (all three for the `test` job).

**`RUSTFLAGS="-D warnings"`** is set globally — any rustc warning fails CI.

The test suite covers unit tests per crate, integration tests where present, and the install-smoke job validates the binary distribution mechanism end-to-end.

---

### 22. Build System Details

**Cargo workspace** (`Cargo.toml` root):

```toml
[workspace]
resolver = "2"
members = ["crates/*", "xtask"]

[workspace.package]
version    = "0.4.0"
edition    = "2021"
rust-version = "1.75"
license    = "Apache-2.0 OR MIT"
```

- **resolver = "2"**: feature unification uses Cargo's v2 resolver — required for correct feature flag isolation across the 13-crate workspace
- **rust-version = "1.75"**: minimum supported Rust version (MSRV); enforced in CI
- **13 crates** plus `xtask` (build helper)
- **License**: Apache-2.0 OR MIT dual license on all crates

**The one C dependency**: `rusqlite` with the `bundled` feature links and compiles libsqlite3 from source during `cargo build`. This is the sole C dependency in the workspace. It:
- Adds ~15s to a clean build
- Requires a C compiler toolchain (`cc`, `ar`) on the build host
- Complicates cross-compilation (especially to musl targets and WASM)
- Is the primary reason the fork removes `rusqlite` — replacing it with `redb` (pure Rust, no C) eliminates this dependency entirely

**Desktop build** (`openfang-desktop`): uses Tauri, which requires additional system packages on Linux: `libwebkit2gtk-4.1-dev`, `libappindicator3-dev`, `librsvg2-dev`. These are not needed for the server binary.

**xtask**: build helper crate for code generation and release tasks. Not published to crates.io.

---

### 23. Performance Characteristics

Runtime limits enforced in `openfang-kernel` (constants visible in source):

| Constant | Value | Meaning |
|----------|-------|---------|
| `MAX_AGENT_CALL_DEPTH` | 5 | Maximum agent-to-agent delegation depth before abort |
| `TOOL_TIMEOUT_SECS` | 120 | Per-tool execution timeout |
| `MAX_CONTINUATIONS` | 5 | Maximum auto-continue loops per agent turn |
| `MAX_HISTORY_MESSAGES` | 20 | Context window cap for session history sent to LLM |

**Cold start** (from README): `< 200ms` from process launch to first agent ready. This measures SQLite open + schema migration check + agent state reload from `openfang.db`.

**Async runtime**: all agent processing runs on Tokio. The kernel uses `Arc<Mutex<>>` for the SQLite connection, which serializes all database writes. Under concurrent agent load, this mutex is the primary throughput bottleneck — multiple agents sharing one write lock on one SQLite file.

**Memory recall latency**: the `SemanticStore.recall_with_embedding` path fetches `(limit × 10).max(100)` rows from SQLite then re-ranks in Rust memory via scalar cosine. For `limit = 5` this is up to 100 SQL rows loaded per recall. Latency grows linearly with the number of rows in the `memories` table.

---

### 24. Migration Compatibility

OpenFang includes `openfang-migrate` — a crate for importing agent configurations from other frameworks into OpenFang format.

**Current implementation state** (`openfang-migrate/src/lib.rs`):

| Framework | Status | What migrates |
|-----------|--------|---------------|
| OpenClaw | **Implemented** | Agents, memory, sessions, skills, channel configs |
| LangChain | Planned — returns `UnsupportedSource` error | — |
| AutoGPT | Planned — returns `UnsupportedSource` error | — |

**Migration options**:
```rust
pub struct MigrateOptions {
    pub source:     MigrateSource,   // OpenClaw | LangChain | AutoGPT
    pub source_dir: PathBuf,         // directory containing source config files
    pub target_dir: PathBuf,         // openfang data directory to write into
    pub dry_run:    bool,            // if true: validate and report, do not write
}
```

`dry_run = true` walks the source directory, validates all files, and reports what would be migrated without writing anything. This is safe to run against a live OpenFang instance.

The migration target format is the SQLite schema (V8 as of v0.4.0). When the memory layer is replaced in the fork, `openfang-migrate` will need an update: imported memories will need to be inserted into the new backend (HNSW + redb) rather than the `memories` SQLite table.

**Export/Import API status**: The `Memory` trait defines `export(format) -> Vec<u8>` and `import(data, format) -> ImportReport` with `ExportFormat::Json` and `ExportFormat::MessagePack` variants. Both are **Phase 1 stubs** — the implementations return empty Vec and a report with zero counts and an "Import not yet implemented" error string. The trait surface exists as a contract but is not functional in v0.4.0.

---

### 25. Configuration System

All configuration lives in `KernelConfig` (`openfang-types/src/config.rs`). The struct has **45 top-level fields**; each is a separate sub-config type. The full set from `KernelConfig::default()`:

| Field | Type | Default / Notes |
|-------|------|-----------------|
| `data_dir` | `PathBuf` | `{openfang_home}/data` |
| `home_dir` | `PathBuf` | `openfang_home_dir()` — platform-specific |
| `log_level` | `String` | `"info"` |
| `api_listen` | `String` | `"127.0.0.1:50051"` |
| `network_enabled` | `bool` | `false` — OFP peer network off by default |
| `default_model` | `DefaultModelConfig` | Provider + model name for new agents |
| `memory` | `MemoryConfig` | SQLite path, decay rate — **the field this fork replaces** |
| `network` | `NetworkConfig` | OFP listen port, discovery, encryption |
| `channels` | `ChannelsConfig` | Per-adapter configs (40+ adapters) |
| `api_key` | `String` | Empty — auth disabled by default |
| `mode` | `KernelMode` | Daemon or interactive |
| `language` | `String` | `"en"` |
| `users` | `Vec<UserConfig>` | RBAC user list |
| `mcp_servers` | `Vec<McpServerConfig>` | External MCP servers to connect at boot |
| `a2a` | `Option<A2aConfig>` | A2A advertised endpoint URL |
| `usage_footer` | `UsageFooterMode` | Token count display in responses |
| `web` | `WebConfig` | WebChat UI customization |
| `fallback_providers` | `Vec<ProviderConfig>` | Ordered fallback chain if primary fails |
| `browser` | `BrowserConfig` | Playwright browser sandbox config |
| `extensions` | `ExtensionsConfig` | WASM extension sandbox settings |
| `vault` | `VaultConfig` | Credential vault encryption key/path |
| `workspaces_dir` | `Option<PathBuf>` | Multi-workspace root (None = single workspace) |
| `media` | `MediaConfig` | Image/video/audio handling |
| `links` | `LinkConfig` | URL preview and link handling |
| `reload` | `ReloadConfig` | Hot-reload watch paths and debounce interval |
| `webhook_triggers` | `Option<WebhookConfig>` | Inbound webhook routing rules |
| `approval` | `ApprovalPolicy` | Which tool calls require human approval |
| `max_cron_jobs` | `usize` | Max concurrent scheduled jobs |
| `include` | `Vec<PathBuf>` | Additional config files to merge |
| `exec_policy` | `ExecPolicy` | Which shell commands agents may run |
| `bindings` | `Vec<BindingConfig>` | Route URL → agent bindings |
| `broadcast` | `BroadcastConfig` | Cross-agent broadcast settings |
| `auto_reply` | `AutoReplyConfig` | Trigger-based auto-reply rules |
| `canvas` | `CanvasConfig` | Canvas/whiteboard feature settings |
| `tts` | `TtsConfig` | Text-to-speech provider and voice |
| `docker` | `DockerSandboxConfig` | Docker sandbox isolation settings |
| `pairing` | `PairingConfig` | Mobile device pairing PIN settings |
| `auth_profiles` | `HashMap<String, AuthProfile>` | Named auth credential sets |
| `thinking` | `Option<ThinkingConfig>` | Extended thinking mode (Claude 3.7+) |
| `budget` | `BudgetConfig` | Global spend limits + per-agent overrides |
| `provider_urls` | `HashMap<String, String>` | Override base URLs per provider |
| `provider_api_keys` | `HashMap<String, String>` | API keys per provider |
| `oauth` | `OAuthConfig` | OAuth2 callback URLs and client credentials |
| `auth` | `AuthConfig` | Dashboard login password hash |
| `workflows_dir` | `Option<PathBuf>` | Directory for workflow definition files |

**Config loading precedence** (highest wins): environment variable overrides → CLI `--config` flag → `openfang.toml` in working directory → `{openfang_home}/config.toml` → `KernelConfig::default()`.

**Hot-reload**: the `/api/config/reload` endpoint re-reads `openfang.toml` at runtime. The `reload.watch` field can also trigger automatic reload on file change (debounced). Not all fields are safe to hot-reload — `api_listen` and `data_dir` changes require a restart.

---

### 26. Daemon Lifecycle

When started as a daemon, OpenFang writes a **`DaemonInfo`** file to track the running instance:

```
{data_dir}/daemon.json
```

**`DaemonInfo` struct**:
```rust
struct DaemonInfo {
    pid:         u32,     // OS process ID
    listen_addr: String,  // e.g. "127.0.0.1:50051"
    started_at:  String,  // RFC3339 timestamp
    version:     String,  // CARGO_PKG_VERSION
    platform:    String,  // std::env::consts::OS
}
```

**Startup sequence** (from `crates/openfang-api/src/server.rs`):
1. Check if `daemon.json` exists
2. If it exists, read the PID and `listen_addr`
3. Call `is_process_alive(pid)` — OS-level liveness check
4. If alive: call `is_daemon_responding(listen_addr)` — HTTP health probe
5. If **both** return true: abort with `"Another daemon (PID {}) is already running"` error
6. If either returns false (stale PID file — process died or OS reused the PID): log `"Removing stale daemon info file"` and delete it
7. Write new `daemon.json` for current process
8. Call `restrict_permissions(daemon_info_path)` — tightens file mode to owner-only read (the comment notes this as a security measure: "contains PID and port")

**Graceful shutdown**: `POST /api/shutdown` triggers a controlled shutdown. The PID file is removed as part of the shutdown sequence.

**Why this matters for the fork**: `daemon.json` is written to `data_dir` alongside `openfang.db`. The fork uses the same `data_dir` convention — the new memory backend should read its path from the same `[memory]` section of `KernelConfig` rather than hard-coding paths.

---

### 27. Error Handling Strategy

**`OpenFangError`** is defined in `openfang-types/src/error.rs` and is the single error type used across all crates. `type OpenFangResult<T> = Result<T, OpenFangError>`.

**21 variants:**

| Variant | When raised |
|---------|-------------|
| `AgentNotFound(String)` | Kernel: unknown agent ID |
| `AgentAlreadyExists(String)` | Kernel: duplicate spawn |
| `CapabilityDenied(String)` | Runtime: capability check failed |
| `QuotaExceeded(String)` | Scheduler / MeteringEngine |
| `InvalidState { current, operation }` | Kernel: wrong agent state for op |
| `SessionNotFound(String)` | Memory: session lookup miss |
| `Memory(String)` | SQLite or memory substrate failure |
| `ToolExecution { tool_id, reason }` | Runtime: tool call returned error |
| `LlmDriver(String)` | LLM provider error (rate limit, timeout, etc.) |
| `Config(String)` | Config parse or validation failure |
| `ManifestParse(String)` | Invalid agent manifest blob |
| `Sandbox(String)` | WASM sandbox execution error |
| `Network(String)` | OFP wire or HTTP client error |
| `Serialization(String)` | msgpack / JSON (de)serialization failure |
| `MaxIterationsExceeded(u32)` | `run_agent_loop` hit iteration cap |
| `ShuttingDown` | Kernel: request during shutdown |
| `Io(#[from] std::io::Error)` | Filesystem I/O |
| `Internal(String)` | Unexpected invariant violation |
| `AuthDenied(String)` | API auth check failed |
| `MeteringError(String)` | Cost tracking failure |
| `InvalidInput(String)` | User/caller input validation |

**Propagation path** (runtime → kernel → API):
```
run_agent_loop() → OpenFangResult<AgentLoopResult>
       ↓
kernel::send_message_with_handle() → KernelResult<AgentLoopResult>
  • pre-execution: scheduler.check_quota() — returns QuotaExceeded if over token limit
  • pre-execution: metering.check_quota() — returns QuotaExceeded if over cost limit
  • post-execution: audit_log.record() on error
       ↓
API route handler → HTTP status mapping:
  • "quota" / "Quota" in error string → 429 Too Many Requests
  • "Agent not found"                 → 404 Not Found
  • everything else                   → 500 Internal Server Error
  • Response body: {"error": "<error message>"}
```

**Error recovery**: there is no automatic retry at the kernel layer. Retries are the caller's responsibility. The `fallback_providers` config field provides ordered LLM provider fallback — if the primary provider returns `LlmDriver`, the kernel tries the next provider in the list before surfacing an error.

---

### 28. Resource Enforcement

**`ResourceQuota`** (in `AgentManifest`, `openfang-types/src/agent.rs`) — per-agent limits with defaults:

| Field | Default | Notes |
|-------|---------|-------|
| `max_memory_bytes` | 256 MB | WASM linear memory cap |
| `max_cpu_time_ms` | 30,000 ms | Per WASM invocation |
| `max_tool_calls_per_minute` | 60 | Sliding window, tracked by scheduler |
| `max_llm_tokens_per_hour` | 0 (unlimited) | Hourly rolling window |
| `max_network_bytes_per_hour` | 100 MB | OFP + HTTP combined |
| `max_cost_per_hour_usd` | 0.0 (unlimited) | Hourly cost gate |
| `max_cost_per_day_usd` | 0.0 (unlimited) | Daily cost gate |
| `max_cost_per_month_usd` | 0.0 (unlimited) | Monthly cost gate |

**Enforcement layers** (checked in order per agent message):

1. **Scheduler quota** (`openfang-kernel/src/scheduler.rs`) — checked pre-execution:
   - Hourly rolling window for `max_llm_tokens_per_hour`; window resets when an hour has elapsed
   - Returns `OpenFangError::QuotaExceeded` if over; check is skipped if quota field is 0

2. **Metering engine** (`openfang-kernel/src/metering.rs`) — checked pre-execution:
   - Queries `usage_events` table for hourly / daily / monthly cost sums
   - `estimate_cost(model, input_tokens, output_tokens)` uses per-model rates to compute USD cost
   - Returns `OpenFangError::QuotaExceeded` with the current vs limit values in the message

3. **Context budget** (`openfang-runtime/src/context_budget.rs`) — enforced inside `run_agent_loop`:
   - Per-result cap: 30% of context window tokens × chars-per-token
   - Total tool result headroom: 75% of context window
   - When total tool result bytes exceed headroom, oldest tool_result blocks are compacted in-place
   - This prevents context overflow silently — no error is raised; content is truncated with a marker

4. **WASM fuel** — enforced inside the sandbox at instruction granularity (see §29)

**When a limit is exceeded**: `QuotaExceeded` propagates up as HTTP 429. The agent is not killed — the specific message is rejected. The agent remains alive and can receive new messages once the rolling window resets.

---

### 29. Extension Architecture

OpenFang has four distinct extension mechanisms, each with different trust levels and sandboxing:

#### Skills (5 Runtimes)

`openfang-skills` manages skills loaded from `{skills_dir}/` and bundled skills compiled in at build time. The registry **freezes after boot** — no new skills can be loaded at runtime (prevents dynamic loading exploits).

| Runtime | How it executes | Trust level |
|---------|----------------|-------------|
| `PromptOnly` | Injects `prompt_context` text into system prompt — no code executes | Lowest risk |
| `Builtin` | Compiled-in Rust code, runs in-process | Highest trust |
| `Wasm` | WASM sandbox (see below) | Sandboxed |
| `Python` | Subprocess with controlled env | OS-level isolation |
| `Node` | Subprocess (OpenClaw compatibility) | OS-level isolation |

Skills declare `SkillRequirements`: which built-in tools they need and which capability grants are required. The skill scanner reads ClawHub-sourced skills for injection patterns (341 malicious skills flagged in dataset); critical threats are blocked at load time.

#### WASM Sandbox (`openfang-runtime/src/sandbox.rs`)

Powered by **Wasmtime**. WASM modules run on blocking threads (not the Tokio executor).

**Default limits** (`SandboxConfig`):
- Fuel limit: **1,000,000 instructions** (deterministic CPU budget; `OutOfFuel` trap → `SandboxError::FuelExhausted`)
- Memory: **16 MB** linear memory
- Timeout: **30 seconds** wall-clock via watchdog epoch thread

**Guest ABI** (what a WASM module must export):
- `memory` — linear memory export
- `alloc(size: i32) -> i32` — allocation function
- `execute(input_ptr: i32, input_len: i32) -> i64` — main entry point; returns `(result_ptr << 32) | result_len`

**Host ABI** (what the sandbox provides to the WASM guest):
- `host_call(request_ptr, request_len) -> i64` — capability-checked RPC (JSON request/response); dispatches `fs_read/write`, `net_fetch`, `shell_exec`, `env_read`, `kv_get/set`, `agent_send/spawn`
- `host_log(level, msg_ptr, msg_len)` — logging, no capability check

#### Hook Registry (`openfang-runtime/src/hooks.rs`)

Four hook events, registered via `HookRegistry`:

| Event | Blocking? | Use |
|-------|-----------|-----|
| `BeforeToolCall` | **Yes** — first `Err(reason)` stops the call | Approval gates, audit pre-checks |
| `AfterToolCall` | No | Observability, logging |
| `BeforePromptBuild` | No | Prompt inspection |
| `AgentLoopEnd` | No | Post-turn cleanup |

Multiple handlers per event are supported (fired in registration order). `BeforeToolCall` errors propagate as `ToolExecution` errors; other event errors are logged and discarded.

#### Integrations / MCP Templates (`openfang-extensions`)

**25 bundled `IntegrationTemplate`s** compiled in at build time, each describing an MCP server (Stdio or SSE transport). Users install from these templates; installed state persists in `~/.openfang/integrations.toml`.

- **Credential vault**: AES-256-GCM encryption with Argon2 key derivation; OS keyring support
- **OAuth2 PKCE**: 6 providers (Google, GitHub, Microsoft, Slack and others); callback routes in `openfang-api`
- **Health monitoring**: exponential backoff + auto-reconnect for installed MCP servers

---

### 30. Crate Dependency Diagram

```
openfang-types          (foundation — no internal deps)
│
├── openfang-memory     (rusqlite + tokio)
│
├── openfang-skills     (serde_yaml, walkdir, zip)
│
├── openfang-wire       (OFP TCP protocol)
│
├── openfang-channels   (40+ adapter impls: lettre, imap, tungstenite, ...)
│
├── openfang-hands      (dashmap — autonomous capability packages)
│
├── openfang-extensions (aes-gcm, argon2, axum OAuth flows)
│
├── openfang-migrate    (walkdir, serde_yaml, json5)
│
└── openfang-runtime    (wasmtime, tungstenite)
    ├── openfang-memory
    └── openfang-skills

openfang-kernel
    ├── openfang-types
    ├── openfang-memory
    ├── openfang-runtime
    ├── openfang-skills
    ├── openfang-wire
    ├── openfang-channels
    ├── openfang-hands
    └── openfang-extensions

openfang-api            (axum, tower, governor GCRA)
    ├── openfang-kernel
    ├── openfang-runtime
    ├── openfang-memory
    ├── openfang-channels
    ├── openfang-wire
    ├── openfang-skills
    ├── openfang-hands
    ├── openfang-extensions
    └── openfang-migrate

openfang-desktop        (Tauri 2.0 — wraps API as native app)
    ├── openfang-kernel
    └── openfang-api

openfang-cli            (clap + ratatui TUI)
    ├── openfang-kernel
    ├── openfang-api
    ├── openfang-runtime
    ├── openfang-skills
    ├── openfang-extensions
    └── openfang-migrate

xtask                   (build helper — not published)
    └── (no internal deps)
```

**Dependency depth**: 4 levels max (`openfang-types` → `openfang-memory` → `openfang-runtime` → `openfang-kernel` → `openfang-api`).

**The memory layer sits at level 2** — it is a direct dep of `openfang-runtime` and `openfang-kernel`. Replacing it requires no changes to `openfang-api`, `openfang-channels`, `openfang-wire`, `openfang-hands`, `openfang-extensions`, `openfang-migrate`, `openfang-desktop`, or `openfang-cli`. Only `openfang-runtime` (agent loop call sites) and `openfang-kernel` (boot + agent lifecycle) need updating.

---

## Consequences

### Positive

- Permanent baseline record. Any developer joining can read ADR-001 to understand exactly what was present before modifications began.
- Makes the fork scope explicit and bounded: memory layer only, not a ground-up rewrite.
- Documents OpenFang's strengths — prevents unnecessary rewriting of things that already work.

### Negative

- None. This is a record-keeping document, not a decision with technical trade-offs.

---

## References

- [OpenFang upstream — github.com/RightNow-AI/openfang](https://github.com/RightNow-AI/openfang)
- [OpenFang v0.4.0 release](https://github.com/RightNow-AI/openfang/releases)
- `ADR-002-memory-engine-integration.md` — the planned memory engine replacement
- `docs/memory-map.md` — comprehensive reference of every memory-related component: all 13 SQLite tables (schema V8), store layer composition, Memory trait vs concrete MemorySubstrate methods, all call sites in agent_loop.rs and kernel.rs, and bifurcation options A/B/C
