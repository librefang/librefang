# Changelog

All notable changes to LibreFang will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.5.2] - 2026-03-16

### Fixed

- Auto-update contributors list from GitHub API (#512) (@houko)
- Use local SVG for contributors with circular avatars (#513) (@houko)
- WeCom secret env pattern + add pre-commit fmt hook (#518) (@houko)

### Maintenance

- Auto-merge release PRs after CI passes (#511) (@houko)

### Other

- V0.5.1-20260316 (#510) (@houko)

## [0.5.1] - 2026-03-16

### Fixed

- Improve API version negotiation and local provider detection (#507) (@houko)
- Inject vault secrets into process env at startup (#509) (@houko)

### Other

- V0.5.0-20260316 (#506) (@houko)

## [0.5.0] - 2026-03-16

### Added

- Add GET /api/commands/:name endpoint (#369) (@houko)
- Add recipe-assistant agent template (#393) (@houko)
- Add Nix flake support (#412) (@houko)
- Add Qwen Code CLI as LLM provider (#417) (@houko)
- Add LLM provider prompt caching support (#381) (#424) (@houko)
- Add decision trace layer for tool selection reasoning (#426) (@houko)
- Add stable_prefix_mode for cache-friendly prompts (#427) (@houko)
- Replace native-tls with rustls for IMAP channel (#432) (@houko)
- Add API endpoint versioning support (#450) (@houko)
- Generate versioned homebrew formula on release (#503) (@houko)

### Fixed

- Use default_model from config in Web UI agent creation (#402) (@houko)
- Apply log_level from config.toml to tracing subscriber (#404) (@houko)
- Correctly read nested tokens.id_token for Codex CLI OAuth (#406) (@houko)
- Use deterministic UUIDs for hand agents to persist across restarts (#407) (@houko)
- Update nix flake for nixpkgs darwin SDK migration (#491) (@houko)
- Update nix flake for darwin SDK and crane warnings (#493) (@houko)
- Add git to devShell and preserve user PATH (#494) (@houko)
- Remove duplicate `/api/versions` route causing panic on startup (#501) (@houko)
- Use Render API for heartbeat + release script improvements (#504) (@houko)
- Allow re-release by replacing existing changelog entry (#505) (@houko)

### Documentation

- Improve CLI --help descriptions for all subcommands (#453) (@houko)

### Other

- V0.4.7-20260315 (#486) (@houko)
- V0.5.0-20260316 (#499) (@houko)


## [0.4.7] - 2026-03-15

### Added

- Add backup and restore functionality for kernel state (#444) (@houko)
- Add thread_id and attachments to CommsSendRequest (#469) (@TJUEZ)

### Fixed

- Resolve WhatsApp Web gateway E2EE, agent UUID, and auto-connect failures (#440) (@houko)
- Wire thread_id and attachments in comms_send handler (#479) (@houko)
- Include tauri.conf.json in release script git add (#482) (@houko)
- Strip date suffix from Tauri version for Windows MSI builds (#485) (@houko)

### Documentation

- Translate getting-started.md to French (#442) (@houko)
- Translate skill-development.md to Chinese (#447) (@houko)

### Other

- V0.4.6-20260315 (#478) (@houko)

## [0.4.6] - 2026-03-15

### Fixed

- Enable all channel features by default and fix changelog dedup (#473) (@houko)

## [0.4.5] - 2026-03-15

### Added

- Add academic-researcher agent template (#391) (@houko)
- Add code-review-checklist prompt skill (#377) (@houko)
- Add API endpoints for managing extensions (#372) (@houko)
- Add memory/knowledge graph export and import API (#371) (@houko)
- Add POST/PUT/DELETE endpoints for MCP server config management (#370) (@houko)
- Add POST/DELETE endpoints for model aliases (#364) (@houko)
- Add GET /api/profiles/:name endpoint (#363) (@houko)
- Add GET /api/tools/:name endpoint (#360) (@houko)
- Add GET /api/schedules/:id endpoint (#291) (@houko)
- Add GET /api/a2a/agents/:id endpoint (#290) (@houko)
- Add PUT /api/cron/jobs/:id endpoint for updating cron jobs (#289) (@houko)
- Horizontal scroll for long commands on deploy page (#276) (@houko)
- Add tooltip for truncated commands on deploy page (#275) (@houko)
- Add copy buttons to install commands on deploy hub (#258) (@houko)
- Add macOS, Linux, Windows install options to deploy hub (#257) (@houko)
- Deploy hub with multi-platform support (#251) (@houko)
- Add GCP free-tier deployment with Terraform (#249) (@houko)
- Support multi-bot routing per platform (#240) (@houko)

### Fixed

- Default to all-channels (#466) (@houko)
- Remove default-features=false to enable channel features (#465) (@houko)
- Respect PORT env var for Railway/Render compatibility (#455) (@houko)
- Stop syncing agent.toml versions with project release version (#375) (@houko)
- Skip pre-release tags when finding previous version for changelog (#374) (@houko)
- Catalog sync fails to parse remote files missing provider field (#362) (@houko)
- Add reconnect logic to Matrix channel adapter (#361) (@houko)
- Remove VOLUME directive from Dockerfile (#294) (@houko)
- Render card empty due to nested anchor tags (#292) (@houko)
- Log warnings instead of silently ignoring errors in API endpoints (#288) (@houko)
- Add URL validation to A2A discover endpoint to prevent SSRF (#287) (@houko)
- Validate environment variable names in channel config API (#286) (@houko)
- Use agent_id path parameter in KV memory endpoints (#285) (@houko)
- Use WEBSITE_REPO_TOKEN for star history workflow (#281) (@houko)
- Auto-merge star history PR after creation (#280) (@houko)
- Use PR instead of direct push for star history workflow (#279) (@houko)
- Move Fly.io-specific badges from header to deploy form (#278) (@houko)
- Revert wrangler-action to v3 (v4 does not exist) (#274) (@houko)
- Add explicit Tauri version for MSI compatibility (#272) (@houko)
- Prevent long commands from stretching deploy cards (#269) (@houko)
- Remove unnecessary card width constraint on deploy page (#268) (@houko)
- Consistent card widths on deploy page (#266) (@houko)
- Allow multi-segment prerelease in semver validation (#263) (@houko)
- Use docker run command on deploy hub (#262) (@houko)
- Use prebuilt GHCR image in docker-compose.yml (#261) (@houko)
- Docker deploy card links to correct README section (#260) (@houko)
- Add catalog directory to Dockerfile (#256) (@houko)
- Correct Railway URL and use prebuilt image for Render (#255) (@houko)
- Deploy page home button links to deploy.librefang.ai (#254) (@houko)
- Replace emoji with SVG icons and add home button (#253) (@houko)
- Prevent release notes from being lost due to race condition (#252) (@houko)
- Remove disk config for Render free tier (#247) (@houko)

### Documentation

- Update star history (#463) (@houko)
- Update star history (#462) (@houko)
- Update star history (#461) (@houko)
- Update star history (#460) (@houko)
- Update star history (#459) (@houko)
- Update star history (#458) (@houko)
- Update star history (#457) (@houko)
- Update star history (#435) (@houko)
- Update star history (#401) (@houko)
- Update star history (#378) (@houko)
- Update star history (#376) (@houko)
- Update star history (#297) (@houko)
- Update star history (#293) (@houko)
- Update star history (#284) (@houko)
- Update star history (#283) (@houko)
- Update star history (#282) (@houko)
- Use docker run across all README translations (#267) (@houko)
- Separate Fly.io and Render deploy descriptions (#248) (@houko)

### Maintenance

- Keep machines running to avoid cold starts (#445) (@houko)
- Auto-deploy to Fly.io on release (#429) (@houko)
- Keep at least 1 machine running to avoid cold starts (#416) (@houko)
- Add unit tests for channel rate limiter (#340) (@houko)
- Add workflow_dispatch to deploy-worker (#273) (@houko)
- Fix wrangler-action, force Node.js 24 (#271) (@houko)
- Upgrade wrangler-action to v4 for Node.js 24 (#270) (@houko)
- Add 'release' to allowed PR title types. (#246) (@houko)
- Update star history workflow schedule to run hourly. (#245) (@houko)

### Other

- V0.4.4-20260315 (#456) (@houko)
- V0.4.3-beta4-20260314 (#365) (@houko)
- V0.4.3-beta3-20260314 (#296) (@houko)
- V0.4.3-beta2-20260314 (#277) (@houko)
- V0.4.3-beta-20260314 (#264) (@houko)
- V0.4.2-20260314 (#244) (@houko)

## [0.4.4] - 2026-03-15

### Added

- Add academic-researcher agent template (#391) (@houko)
- Add code-review-checklist prompt skill (#377) (@houko)
- Add API endpoints for managing extensions (#372) (@houko)
- Add memory/knowledge graph export and import API (#371) (@houko)
- Add POST/PUT/DELETE endpoints for MCP server config management (#370) (@houko)
- Add POST/DELETE endpoints for model aliases (#364) (@houko)
- Add GET /api/profiles/:name endpoint (#363) (@houko)
- Add GET /api/tools/:name endpoint (#360) (@houko)
- Add GET /api/schedules/:id endpoint (#291) (@houko)
- Add GET /api/a2a/agents/:id endpoint (#290) (@houko)
- Add PUT /api/cron/jobs/:id endpoint for updating cron jobs (#289) (@houko)
- Horizontal scroll for long commands on deploy page (#276) (@houko)
- Add tooltip for truncated commands on deploy page (#275) (@houko)
- Add copy buttons to install commands on deploy hub (#258) (@houko)
- Add macOS, Linux, Windows install options to deploy hub (#257) (@houko)
- Deploy hub with multi-platform support (#251) (@houko)
- Add GCP free-tier deployment with Terraform (#249) (@houko)
- Support multi-bot routing per platform (#240) (@houko)

### Fixed

- Respect PORT env var for Railway/Render compatibility (#455) (@houko)
- Stop syncing agent.toml versions with project release version (#375) (@houko)
- Skip pre-release tags when finding previous version for changelog (#374) (@houko)
- Catalog sync fails to parse remote files missing provider field (#362) (@houko)
- Add reconnect logic to Matrix channel adapter (#361) (@houko)
- Remove VOLUME directive from Dockerfile (#294) (@houko)
- Render card empty due to nested anchor tags (#292) (@houko)
- Log warnings instead of silently ignoring errors in API endpoints (#288) (@houko)
- Add URL validation to A2A discover endpoint to prevent SSRF (#287) (@houko)
- Validate environment variable names in channel config API (#286) (@houko)
- Use agent_id path parameter in KV memory endpoints (#285) (@houko)
- Use WEBSITE_REPO_TOKEN for star history workflow (#281) (@houko)
- Auto-merge star history PR after creation (#280) (@houko)
- Use PR instead of direct push for star history workflow (#279) (@houko)
- Move Fly.io-specific badges from header to deploy form (#278) (@houko)
- Revert wrangler-action to v3 (v4 does not exist) (#274) (@houko)
- Add explicit Tauri version for MSI compatibility (#272) (@houko)
- Prevent long commands from stretching deploy cards (#269) (@houko)
- Remove unnecessary card width constraint on deploy page (#268) (@houko)
- Consistent card widths on deploy page (#266) (@houko)
- Allow multi-segment prerelease in semver validation (#263) (@houko)
- Use docker run command on deploy hub (#262) (@houko)
- Use prebuilt GHCR image in docker-compose.yml (#261) (@houko)
- Docker deploy card links to correct README section (#260) (@houko)
- Add catalog directory to Dockerfile (#256) (@houko)
- Correct Railway URL and use prebuilt image for Render (#255) (@houko)
- Deploy page home button links to deploy.librefang.ai (#254) (@houko)
- Replace emoji with SVG icons and add home button (#253) (@houko)
- Prevent release notes from being lost due to race condition (#252) (@houko)
- Remove disk config for Render free tier (#247) (@houko)

### Documentation

- Update star history (#435) (@houko)
- Update star history (#401) (@houko)
- Update star history (#378) (@houko)
- Update star history (#376) (@houko)
- Update star history (#297) (@houko)
- Update star history (#293) (@houko)
- Update star history (#284) (@houko)
- Update star history (#283) (@houko)
- Update star history (#282) (@houko)
- Use docker run across all README translations (#267) (@houko)
- Separate Fly.io and Render deploy descriptions (#248) (@houko)

### Maintenance

- Keep machines running to avoid cold starts (#445) (@houko)
- Auto-deploy to Fly.io on release (#429) (@houko)
- Keep at least 1 machine running to avoid cold starts (#416) (@houko)
- Add unit tests for channel rate limiter (#340) (@houko)
- Add workflow_dispatch to deploy-worker (#273) (@houko)
- Fix wrangler-action, force Node.js 24 (#271) (@houko)
- Upgrade wrangler-action to v4 for Node.js 24 (#270) (@houko)
- Add 'release' to allowed PR title types. (#246) (@houko)
- Update star history workflow schedule to run hourly. (#245) (@houko)

### Other

- V0.4.3-beta4-20260314 (#365) (@houko)
- V0.4.3-beta3-20260314 (#296) (@houko)
- V0.4.3-beta2-20260314 (#277) (@houko)
- V0.4.3-beta-20260314 (#264) (@houko)
- V0.4.2-20260314 (#244) (@houko)

## [0.4.3-beta3] - 2026-03-14

### Fixed

- Render card empty due to nested anchor tags (#292) (@houko)
- Use WEBSITE_REPO_TOKEN for star history workflow (#281) (@houko)
- Auto-merge star history PR after creation (#280) (@houko)
- Use PR instead of direct push for star history workflow (#279) (@houko)
- Move Fly.io-specific badges from header to deploy form (#278) (@houko)

### Documentation

- Update star history (#293) (@houko)
- Update star history (#284) (@houko)
- Update star history (#283) (@houko)
- Update star history (#282) (@houko)

### Other

- V0.4.3-beta2-20260314 (#277) (@houko)

## [0.4.3-beta2] - 2026-03-14

### Added

- Horizontal scroll for long commands on deploy page (#276) (@houko)
- Add tooltip for truncated commands on deploy page (#275) (@houko)
- Support multi-bot routing per platform (#240) (@houko)

### Fixed

- Revert wrangler-action to v3 (v4 does not exist) (#274) (@houko)
- Add explicit Tauri version for MSI compatibility (#272) (@houko)
- Prevent long commands from stretching deploy cards (#269) (@houko)
- Remove unnecessary card width constraint on deploy page (#268) (@houko)
- Consistent card widths on deploy page (#266) (@houko)
- Use prebuilt GHCR image in docker-compose.yml (#261) (@houko)

### Documentation

- Use docker run across all README translations (#267) (@houko)

### Maintenance

- Add workflow_dispatch to deploy-worker (#273) (@houko)
- Fix wrangler-action, force Node.js 24 (#271) (@houko)
- Upgrade wrangler-action to v4 for Node.js 24 (#270) (@houko)

### Other

- V0.4.3-beta-20260314 (#264) (@houko)

## [0.4.3-beta] - 2026-03-14

### Added

- Add copy buttons to install commands on deploy hub (#258) (@houko)
- Add macOS, Linux, Windows install options to deploy hub (#257) (@houko)
- Deploy hub with multi-platform support (#251) (@houko)
- Add GCP free-tier deployment with Terraform (#249) (@houko)

### Fixed

- Allow multi-segment prerelease in semver validation (#263) (@houko)
- Use docker run command on deploy hub (#262) (@houko)
- Docker deploy card links to correct README section (#260) (@houko)
- Add catalog directory to Dockerfile (#256) (@houko)
- Correct Railway URL and use prebuilt image for Render (#255) (@houko)
- Deploy page home button links to deploy.librefang.ai (#254) (@houko)
- Replace emoji with SVG icons and add home button (#253) (@houko)
- Prevent release notes from being lost due to race condition (#252) (@houko)
- Remove disk config for Render free tier (#247) (@houko)

### Documentation

- Separate Fly.io and Render deploy descriptions (#248) (@houko)

### Maintenance

- Add 'release' to allowed PR title types. (#246) (@houko)
- Update star history workflow schedule to run hourly. (#245) (@houko)

### Other

- V0.4.2-20260314 (#244) (@houko)

## [0.4.2] - 2026-03-14

### Added

- Add CLI deploy command and FAQ to deploy page (#238) (@houko)
- Auto-sync model catalog on daemon startup (#237) (@houko)
- Add channel sidecar protocol for external adapters (#228) (@houko)
- Integrate model-catalog sync with dashboard UI (#227) (@houko)
- Add cargo feature flags for channel adapters (#223) (@houko)
- Improve community organization and version governance (#212) (@houko)

### Fixed

- Revert file versions to 0.4.1-20260314 and fix release.sh (#243) (@houko)
- Release script uses PR instead of direct push (#242) (@houko)
- Daemon env vars, MCP probe, and SSE parsing (#211) (@houko)

### Changed

- Replace hardcoded model catalog with include_str TOML (#235) (@houko)
- Replace provider match with static registry (#224) (@houko)

### Documentation

- Add integration test writing guide to CONTRIBUTING.md (#232) (@houko)
- Add channel adapter contribution example (#231) (@houko)

### Maintenance

- Bump version to v0.4.2-20260314 (#241) (@houko)
- Trigger deploy worker auto-deploy (#239) (@houko)
- Add pre-commit hooks and i18n contribution guide (#233) (@houko)
- Add justfile for unified dev commands (#230) (@houko)
- Upgrade GitHub Actions for Node.js 24 compatibility (#229) (@houko)

## [0.4.2] - 2026-03-14

### Added

- Add CLI deploy command and FAQ to deploy page (#238) (@houko)
- Auto-sync model catalog on daemon startup (#237) (@houko)
- Add channel sidecar protocol for external adapters (#228) (@houko)
- Integrate model-catalog sync with dashboard UI (#227) (@houko)
- Add cargo feature flags for channel adapters (#223) (@houko)
- Improve community organization and version governance (#212) (@houko)

### Fixed

- Daemon env vars, MCP probe, and SSE parsing (#211) (@houko)

### Changed

- Replace hardcoded model catalog with include_str TOML (#235) (@houko)
- Replace provider match with static registry (#224) (@houko)

### Documentation

- Add integration test writing guide to CONTRIBUTING.md (#232) (@houko)
- Add channel adapter contribution example (#231) (@houko)

### Maintenance

- Trigger deploy worker auto-deploy (#239) (@houko)
- Add pre-commit hooks and i18n contribution guide (#233) (@houko)
- Add justfile for unified dev commands (#230) (@houko)
- Upgrade GitHub Actions for Node.js 24 compatibility (#229) (@houko)

## [0.4.0] - 2026-03-14

### Added

#### Authentication & Drivers
- **ChatGPT Session Auth**: New browser-based OAuth flow for ChatGPT Plus/Ultra subscribers.
  - PKCE S256 code challenge for secure token exchange.
  - Automatic model discovery (Codex endpoints).
  - `librefang auth chatgpt` subcommand to easily link accounts.
  - Persistent session caching with 7-day TTL.
- **MiniMax Dual-Platform Support**: Added separate `minimax-cn` provider for China-specific endpoints (using `MINIMAX_CN_API_KEY`).
- **QQ Bot Adapter**: Native support for QQ Bot messaging channel.

#### Web Dashboard & i18n
- **Internationalization (i18n)**: Full support for multiple languages in the dashboard.
  - Added `zh-CN` (Simplified Chinese) locale.
  - Unified translation helper `t()` across all JS modules.
- **UI Overhaul**:
  - New sidebar layout with integrated theme/language switchers.
  - Replaced emoji icons with high-quality inline SVG icons (globe, search, chart, etc.).
  - Improved ClawHub category wrapping for better responsiveness on small screens.

#### Core Platform
- **Version Alignment**: Synced all 31 built-in agents and sub-packages to version 0.4.0.
- **Config Hot-Reloading**: Enhanced reliability for runtime configuration updates without daemon restarts.

## [0.1.0] - 2026-02-24

### Added

#### Core Platform
- 15-crate Rust workspace: types, memory, runtime, kernel, api, channels, wire, cli, migrate, skills, hands, extensions, desktop, xtask
- Agent lifecycle management: spawn, list, kill, clone, mode switching (Full/Assist/Observe)
- SQLite-backed memory substrate with structured KV, semantic recall, vector embeddings
- 41 built-in tools (filesystem, web, shell, browser, scheduling, collaboration, image analysis, inter-agent, TTS, media)
- WASM sandbox with dual metering (fuel + epoch interruption with watchdog thread)
- Workflow engine with pipelines, fan-out parallelism, conditional steps, loops, and variable expansion
- Visual workflow builder with drag-and-drop node graph, 7 node types, and TOML export
- Trigger system with event pattern matching, content filters, and fire limits
- Event bus with publish/subscribe and correlation IDs
- 7 Hands packages for autonomous agent actions

#### LLM Support
- 3 native LLM drivers: Anthropic, Google Gemini, OpenAI-compatible
- 27 providers: Anthropic, Gemini, OpenAI, Groq, OpenRouter, DeepSeek, Together, Mistral, Fireworks, Cohere, Perplexity, xAI, AI21, Cerebras, SambaNova, Hugging Face, Replicate, Ollama, vLLM, LM Studio, and more
- Model catalog with 130+ built-in models, 23 aliases, tier classification
- Intelligent model routing with task complexity scoring
- Fallback driver for automatic failover between providers
- Cost estimation and metering engine with per-model pricing
- Streaming support (SSE) across all drivers

#### Token Management & Context
- Token-aware session compaction (chars/4 heuristic, triggers at 70% context capacity)
- In-loop emergency trimming at 70%/90% thresholds with summary injection
- Tool profile filtering (cuts default 41 tools to 4-10 for chat agents, saving 15-20K tokens)
- Context budget allocation for system prompt, tools, history, and response
- MAX_TOOL_RESULT_CHARS reduced from 50K to 15K to prevent tool result bloat
- Default token quota raised from 100K to 1M per hour

#### Security
- Capability-based access control with privilege escalation prevention
- Path traversal protection in all file tools
- SSRF protection blocking private IPs and cloud metadata endpoints
- Ed25519 signed agent manifests
- Merkle hash chain audit trail with tamper detection
- Information flow taint tracking
- HMAC-SHA256 mutual authentication for peer wire protocol
- API key authentication with Bearer token
- GCRA rate limiter with cost-aware token buckets
- Security headers middleware (CSP, X-Frame-Options, HSTS)
- Secret zeroization on all API key fields
- Subprocess environment isolation
- Health endpoint redaction (public minimal, auth full)
- Loop guard with SHA256-based detection and circuit breaker thresholds
- Session repair (validates and fixes orphaned tool results, empty messages)

#### Channels
- 40 channel adapters: Telegram, Discord, Slack, WhatsApp, Signal, Matrix, Email, Teams, Mattermost, Google Chat, Webex, Feishu/Lark, LINE, Viber, Facebook Messenger, Mastodon, Bluesky, Reddit, LinkedIn, Twitch, IRC, XMPP, and 18 more
- Unified bridge with agent routing, command handling, message splitting
- Per-channel user filtering and RBAC enforcement
- Graceful shutdown, exponential backoff, secret zeroization on all adapters

#### API
- 100+ REST/WS/SSE API endpoints (axum 0.8)
- WebSocket real-time streaming with per-agent connections
- OpenAI-compatible `/v1/chat/completions` API (streaming SSE + non-streaming)
- OpenAI-compatible `/v1/models` endpoint
- WebChat embedded UI with Alpine.js
- Google A2A protocol support (agent card, task send/get/cancel)
- Prometheus text-format `/api/metrics` endpoint for monitoring
- Multi-session management: list, create, switch, label sessions per agent
- Usage analytics: summary, by-model, daily breakdown
- Config hot-reload via polling (30-second interval, no restart required)

#### Web UI
- Chat message search with Ctrl+F, real-time filtering, text highlighting
- Voice input with hold-to-record mic button (WebM/Opus codec)
- TTS audio playback inline in tool cards
- Browser screenshot rendering in chat (inline images)
- Canvas rendering with iframe sandbox and CSP support
- Session switcher dropdown in chat header
- 6-step first-run setup wizard with provider API key help (12 providers)
- Skill marketplace with 4 tabs (Installed, ClawHub, MCP Servers, Quick Start)
- Copy-to-clipboard on messages, message timestamps
- Visual workflow builder with drag-and-drop canvas

#### Client SDKs
- JavaScript SDK (`@librefang/sdk`): full REST API client with streaming, TypeScript declarations
- Python client SDK (`librefang_client`): zero-dependency stdlib client with SSE streaming
- Python agent SDK (`librefang_sdk`): decorator-based framework for writing Python agents
- Usage examples for both languages (basic + streaming)

#### CLI
- 14+ subcommands: init, start, agent, workflow, trigger, migrate, skill, channel, config, chat, status, doctor, dashboard, mcp
- Daemon auto-detection via PID file
- Shell completion generation (bash, zsh, fish, PowerShell)
- MCP server mode for IDE integration

#### Skills Ecosystem
- 60 bundled skills across 14 categories
- Skill registry with TOML manifests
- 4 runtimes: Python, Node.js, WASM, PromptOnly
- FangHub marketplace with search/install
- ClawHub client for OpenClaw skill compatibility
- SKILL.md parser with auto-conversion
- SHA256 checksum verification
- Prompt injection scanning on skill content

#### Desktop App
- Tauri 2.0 native desktop app
- System tray with status and quick actions
- Single-instance enforcement
- Hide-to-tray on close
- Updated CSP for media, frame, and blob sources

#### Session Management
- LLM-based session compaction with token-aware triggers
- Multi-session per agent with named labels
- Session switching via API and UI
- Cross-channel canonical sessions
- Extended chat commands: `/new`, `/compact`, `/model`, `/stop`, `/usage`, `/think`

#### Image Support
- `ContentBlock::Image` with base64 inline data
- Media type validation (png, jpeg, gif, webp only)
- 5MB size limit enforcement
- Mapped to all 3 native LLM drivers

#### Usage Tracking
- Per-response cost estimation with model-aware pricing
- Usage footer in WebSocket responses and WebChat UI
- Usage events persisted to SQLite
- Quota enforcement with hourly windows

#### Interoperability
- OpenClaw migration engine (YAML/JSON5 to TOML)
- MCP client (JSON-RPC 2.0 over stdio/SSE, tool namespacing)
- MCP server (exposes LibreFang tools via MCP protocol)
- A2A protocol client and server
- Tool name compatibility mappings (21 OpenClaw tool names)

#### Infrastructure
- Multi-stage Dockerfile (debian:bookworm-slim runtime)
- docker-compose.yml with volume persistence
- GitHub Actions CI (check, test, clippy, format)
- GitHub Actions release (multi-platform, GHCR push, SHA256 checksums)
- Cross-platform install script (curl/irm one-liner)
- systemd service file for Linux deployment

#### Multi-User
- RBAC with Owner/Admin/User/Viewer roles
- Channel identity resolution
- Per-user authorization checks
- Device pairing and approval system

#### Production Readiness
- 1731+ tests across 15 crates, 0 failures
- Cross-platform support (Linux, macOS, Windows)
- Graceful shutdown with signal handling (SIGINT/SIGTERM on Unix, Ctrl+C on Windows)
- Daemon PID file with stale process detection
- Release profile with LTO, single codegen unit, symbol stripping
- Prometheus metrics for monitoring
- Config hot-reload without restart

[0.1.0]: https://github.com/librefang/librefang/releases/tag/v0.1.0
