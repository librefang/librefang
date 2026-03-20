# Changelog

All notable changes to LibreFang will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.6.7] - 2026-03-20

### Added

- Add GitHub Discussions link to dashboard sidebar (#1302) (@TechWizard9999)

### Fixed

- Include user-installed HAND manifests in hand routing (#1205) (@TechWizard9999)
- Pass raw JSON payloads to context hook scripts (#1207) (@TechWizard9999)
- Pass GITHUB_TOKEN to contributor/star-history scripts (#1300) (@houko)
- Self-heal fish config PATH entries (#1303) (@TechWizard9999)
- Fix 3 release workflow failures from v0.6.6 (#1309) (@houko)

## [0.6.6] - 2026-03-20

### Fixed

- Use 'file' instead of 'dockerfile' in docker/build-push-action@v7 (#1298) (@houko)

## [0.6.5] - 2026-03-20

### Added

- Auto-initialize vault during librefang init (#1206) (@houko)
- Add token consumption metadata and reduce default hands (#1215) (@houko)
- Add image pipeline and subprocess management (#1223) (@f-liva)
- Add Qwen Code CLI as LLM provider (#1224) (@f-liva)
- Align init defaults with OpenRouter Stepfun (#1262) (@houko)
- Replace all icons with new LibreFang branding (#1263) (@f-liva)
- Fix shell (#1270) (@houko)

### Fixed

- Decrypt encrypted webhook payloads (#1208) (@TechWizard9999)
- Bootstrap context engine during startup (#1209) (@TechWizard9999)
- Support target ids in channel test (#1210) (@TechWizard9999)
- Make shell installer POSIX-compatible for Linux (#1226) (@houko)
- Web deployment issues (#1236) (@houko)
- Web deployment issues (#1237) (@houko)
- Create web/public/assets directory (#1238) (@houko)
- Web deployment and CI fixes (#1239) (@houko)
- Web deployment and CI fixes (#1243) (@houko)
- Repair install smoke script and drop update-star-history workflow (#1255) (@houko)
- Use web/public installer source and harden curl|sh install flow (#1259) (@houko)
- Address code-scanning path-injection findings (follow-up) (#1260) (@houko)
- Initialize rustls crypto provider for TLS connections (#1294) (@houko)

### Documentation

- Update star history (#1197) (@houko)
- Update star history (#1198) (@houko)
- Update star history (#1199) (@houko)
- Update star history (#1200) (@houko)
- Update star history (#1201) (@houko)
- Update star history (#1202) (@houko)
- Update star history (#1203) (@houko)
- Update star history (#1213) (@houko)
- Update contributors (#1214) (@houko)
- Update star history (#1225) (@houko)
- Update star history (#1228) (@houko)
- Add SDK usage examples to all README files (#1229) (@houko)
- Update star history (#1231) (@houko)
- Update star history (#1232) (@houko)
- Update star history (#1235) (@houko)
- Update contributors (#1240) (@app/github-actions)
- Update star history (#1242) (@app/github-actions)

### Maintenance

- Tidy repo structure (#1211) (@houko)
- Use .nvmrc for web Node.js version and fix Dockerfile path (#1234) (@houko)
- Bump actions/setup-node from 4 to 6 (#1280) (@app/dependabot)
- Bump actions/download-artifact from 4 to 8 (#1281) (@app/dependabot)
- Bump pnpm/action-setup from 4 to 5 (#1282) (@app/dependabot)
- Bump actions/labeler from 5 to 6 (#1283) (@app/dependabot)
- Bump zip from 8.2.0 to 8.3.0 (#1284) (@app/dependabot)
- Bump jsonwebtoken from 9.3.1 to 10.3.0 (#1285) (@app/dependabot)
- Bump tracing-subscriber from 0.3.22 to 0.3.23 (#1286) (@app/dependabot)
- Bump rusqlite from 0.38.0 to 0.39.0 (#1287) (@app/dependabot)
- Bump rumqttc from 0.24.0 to 0.25.1 (#1288) (@app/dependabot)
- Bump tokio-tungstenite from 0.28.0 to 0.29.0 (#1289) (@app/dependabot)
- Bump criterion from 0.5.1 to 0.8.2 (#1290) (@app/dependabot)
- Bump rand from 0.8.5 to 0.9.2 (#1291) (@app/dependabot)
- Bump toml_edit from 0.25.4+spec-1.1.0 to 0.25.5+spec-1.1.0 (#1292) (@app/dependabot)

### Other

- Clean skills (#1212) (@houko)
- Fix/webui chat input line break failed (#1245) (@aimlyo)

## [v0.6.4-20260320] - 2026-03-20

### Added

- Add image pipeline and subprocess management (#1223) (@f-liva)
- Add Qwen Code CLI as LLM provider (#1224) (@f-liva)
- Align init defaults with OpenRouter Stepfun (#1262) (@houko)
- Replace all icons with new LibreFang branding (#1263) (@f-liva)

### Fixed

- Web deployment issues (#1236) (@houko)
- Web deployment issues (#1237) (@houko)
- Create web/public/assets directory (#1238) (@houko)
- Web deployment and CI fixes (#1239) (@houko)
- Web deployment and CI fixes (#1243) (@houko)
- Repair install smoke script and drop update-star-history workflow (#1255) (@houko)
- Use web/public installer source and harden curl|sh install flow (#1259) (@houko)
- Address code-scanning path-injection findings (follow-up) (#1260) (@houko)

### Documentation

- Update star history (#1228) (@houko)
- Add SDK usage examples to all README files (#1229) (@houko)

### Maintenance

- Use .nvmrc for web Node.js version and fix Dockerfile path (#1234) (@houko)

### Other

- Fix/webui chat input line break failed (#1245) (@aimlyo)

## [0.6.4] - 2026-03-20

### Added

- Add image pipeline and subprocess management (#1223) (@f-liva)
- Add Qwen Code CLI as LLM provider (#1224) (@f-liva)
- Align init defaults with OpenRouter Stepfun (#1262) (@houko)
- Replace all icons with new LibreFang branding (#1263) (@f-liva)

### Fixed

- Web deployment issues (#1236) (@houko)
- Web deployment issues (#1237) (@houko)
- Create web/public/assets directory (#1238) (@houko)
- Web deployment and CI fixes (#1239) (@houko)
- Web deployment and CI fixes (#1243) (@houko)
- Repair install smoke script and drop update-star-history workflow (#1255) (@houko)
- Use web/public installer source and harden curl|sh install flow (#1259) (@houko)
- Address code-scanning path-injection findings (follow-up) (#1260) (@houko)

### Documentation

- Add SDK usage examples to all README files (#1229) (@houko)
- Update contributors (#1240) (@app/github-actions)
- Update contributors and star history (#1244) (@app/github-actions)
- Update contributors and star history (#1246) (@app/github-actions)
- Update contributors and star history (#1247) (@app/github-actions)
- Update contributors and star history (#1248) (@app/github-actions)
- Update contributors and star history (#1250) (@app/github-actions)
- Update contributors and star history (#1251) (@app/github-actions)
- Update contributors and star history (#1253) (@app/github-actions)
- Update contributors and star history (#1256) (@app/github-actions)
- Update contributors and star history (#1257) (@app/github-actions)
- Update contributors and star history (#1258) (@app/github-actions)
- Update contributors and star history (#1261) (@app/github-actions)
- Update contributors and star history (#1264) (@app/github-actions)
- Update contributors and star history (#1265) (@app/github-actions)
- Update contributors and star history (#1267) (@app/github-actions)

### Maintenance

- Use .nvmrc for web Node.js version and fix Dockerfile path (#1234) (@houko)

### Other

- Fix/webui chat input line break failed (#1245) (@aimlyo)

## [0.6.3] - 2026-03-19

### Added

- Auto-initialize vault during librefang init (#1206) (@houko)
- Add token consumption metadata and reduce default hands (#1215) (@houko)

### Fixed

- Decrypt encrypted webhook payloads (#1208) (@TechWizard9999)
- Bootstrap context engine during startup (#1209) (@TechWizard9999)
- Support target ids in channel test (#1210) (@TechWizard9999)
- Make shell installer POSIX-compatible for Linux (#1226) (@houko)

### Documentation

- Update contributors (#1214) (@houko)

### Maintenance

- Tidy repo structure (#1211) (@houko)

### Other

- Clean skills (#1212) (@houko)

## [0.6.2] - 2026-03-19

### Fixed

- Prevent provider appearing in multiple tier groups (#1190) (@SenZhangAI)
- Resolve 17 compilation errors breaking CI (#1193) (@houko)

## [0.6.1] - 2026-03-18

### Added

- Graceful degradation when no LLM provider configured (#1185) (@SenZhangAI)

### Fixed

- Remove markdown fence wrapper from dev.to articles (#1167) (@houko)
- Resolve secret scanning alert for MongoDB example URI (#1168) (@houko)
- Handle paginated response in agents list and chat resolver (#1169) (@houko)
- Resolve agent names to UUIDs in message and kill commands (#1170) (@houko)
- Return 409 Conflict when spawning duplicate agent (#1171) (@houko)
- Parse model aliases from API response correctly (#1172) (@houko)
- Include last_active in agent detail endpoint (#1173) (@houko)
- Parse wrapped API responses in CLI table views (#1175) (@houko)
- Resolve agent names in trigger, cron, and webhook commands (#1176) (@houko)
- Complete dashboard i18n translation coverage (#1177) (@houko)
- Webhook CLI commands use wrong API endpoints (#1178) (@houko)
- A2A agent card uses service config instead of random agent (#1179) (@houko)
- Budget PUT accepts GET response field names for read-modify-write (#1182) (@houko)
- Models set sends wrong field name to config/set API (#1183) (@houko)
- Cron create returns proper JSON instead of stringified blob (#1184) (@houko)
- CLI cron list reads nested schedule/action fields (#1186) (@houko)
- Triggers list returns wrapped object for consistency (#1187) (@houko)
- Include system_prompt in GET /api/agents/:id response (#1188) (@houko)

### Maintenance

- Fix rustfmt in a2a_agent_card handler (#1181) (@houko)

## [0.6.0] - 2026-03-18

### Added

- Add filtering, pagination and sorting to agent list endpoint (#399) (@houko)
- Add HTTP proxy support for all outbound connections (#415) (@houko)
- Auto-register local workflow definitions at daemon startup (#418) (@houko)
- Add multimedia support for Telegram and Discord channels (#422) (@houko)
- Add Telegram streaming output with progressive message updates (#423) (@houko)
- Add NVIDIA NIM as dedicated LLM provider (#428) (@houko)
- Add MQTT pub/sub channel adapter for IoT integration (#430) (@houko)
- Add workflow trigger support to cron jobs (#431) (@houko)
- Add hierarchical Goals system with REST API and dashboard UI (#434) (@houko)
- Bundle Python and Node.js runtimes in Docker image (#334) (#436) (@houko)
- Add Vertex AI driver with OAuth2 authentication (#448) (@houko)
- Add GET /api/providers/:name endpoint (#1090) (@houko)
- Add GET /api/workflows/:id endpoint (#1091) (@houko)
- Add GET /api/channels/:name endpoint (#1092) (@houko)
- Add GET /api/cron/jobs/:id endpoint (#1093) (@houko)
- Add GET /api/mcp_servers/:name endpoint (#1094) (@houko)
- Add PUT/DELETE /api/workflows/:id endpoints (#1095) (@houko)
- Add DELETE /api/agents/:id/files/:filename endpoint (#1097) (@houko)
- Add Workflow variant to CronAction for cron-triggered workflows (#1102) (@houko)
- Propagate sender identity from channels to agent context (#1105) (@houko)
- Auto-register local workflow definitions at daemon startup (fixes #382) (#1107) (@houko)
- Implement mem0-style proactive memory system (#1111) (@houko)
- Web search key rotation, data-driven hand routing, and health-aware LLM fallback (#1127) (@houko)
- Improve context engine accuracy and resilience (#1146) (@houko)
- Add context engine plugin management system (#1152) (@houko)
- Support multiple custom plugin registries (#1154) (@houko)

### Fixed

- Extract thread_ts from Slack events for thread replies (#1099) (@houko)
- Add mime_type to ChannelContent::Image for correct vision handling (#1100) (@houko)
- Use SHA-256 for Nostr pubkey derivation instead of DefaultHasher (#1101) (@houko)
- Prevent silent message dropping in Telegram dispatch (#1103) (@houko)
- Handle thought chunks in Gemini streaming for thinking models (#1104) (@houko)
- Don't break streaming bridge on intermediate ContentComplete (#1126) (@houko)
- Fall back to bundled Mozilla CA roots when system certs unavailable (#1142) (@houko)
- Upstream parity — 10 bug fixes from release comparison (#1143) (@houko)
- Resolve clippy warnings, test failures, and add agent list validation (#1162) (@houko)

### Documentation

- Slim down README (#1124) (@houko)

### Maintenance

- Skip version bump PRs in changelog generation (#1123) (@houko)
- Bump setup-python v5→v6 and create-pull-request v7→v8 (#1161) (@houko)

## [0.5.7] - 2026-03-18

### Added

- Add include_skills and include_tools flags to agent clone API (#366) (@houko)
- Add event webhooks API for system events (#394) (@houko)
- Add task queue management API (#395) (@houko)
- Add agent monitoring and metrics API (#396) (@houko)
- Add API input validation middleware (#398) (@houko)
- Add webhooks management API (#400) (@houko)
- Add 6 new bundled hand templates (#413) (@houko)
- Add multi-agent orchestration foundation (#323) (#437) (@houko)
- Add Feishu interactive card approval for agent permission requests (#439) (@houko)
- Add multi-token fallback with transparent quota rotation (#441) (@houko)
- Add JWT/service account auth to Google Chat adapter (#443) (@houko)
- Auto-post release article to GitHub Discussions (#582) (@houko)
- Add GET /api/integrations/:id endpoint (#1088) (@houko)
- Add GET /api/approvals/:id endpoint (#1089) (@houko)
- Add GET /api/sessions/:id endpoint (#1096) (@houko)
- Telegram streaming output support with progressive typing effect (fixes #317) (#1109) (@houko)

### Fixed

- YAML syntax errors + auto-post release to GitHub Discussions (#567) (@houko)
- Inherit parent env in MCP subprocess instead of clearing (#1098) (@houko)
- Send empty object instead of null for parameterless tool calls (fixes #918) (#1108) (@houko)
- Add missing TokenUsage fields in token rotation test (#1114) (@houko)

## [0.5.6] - 2026-03-17

### Added

- Add multi-language support for CLI and API error messages (#449) (@houko)
- Add truncation and metadata for Telegram reply-to-message (#560) (@SenZhangAI)

### Fixed

- SDK publish fixes + Bluesky notification + auto Dev.to article (#562) (@houko)
- YAML syntax error in Bluesky notification workflow (#563) (@houko)
- Add missing discovered_model_info field in ProbeResult test (#565) (@houko)

## [0.5.5] - 2026-03-17

### Added

- Add Telegram reply-to-message context (#553) (@SenZhangAI)
- Enrich Ollama model discovery with metadata (#554) (@SenZhangAI)
- Add GET /api/peers/{id} endpoint (#557) (@SenZhangAI)

### Fixed

- Improve Telegram markdown formatting for headings, lists, code blocks and blockquotes (#405) (@houko)
- Normalize OpenRouter model IDs to prevent 400 errors (#408) (@houko)
- Improve python3 detection and Chromium sandbox handling for Linux (#410) (@houko)
- Prevent Mastodon adapter from re-delivering old notifications and posting errors publicly (#411) (@houko)
- Replace unsafe pointer mutation with OnceLock for peer_registry/peer_node (#414) (@houko)
- Raise main_lane default concurrency from 1 to 3 (#552) (@SenZhangAI)
- Update static linking check to match static-pie binaries (#558) (@houko)

### Performance

- Optimize channel hot-path with reduced allocations and Criterion benchmarks (#451) (@houko)

### Documentation

- Update contributors (#555) (@houko)

### Maintenance

- Auto-cancel old release runs when tag is re-pushed (#547) (@houko)

## [0.5.4] - 2026-03-17

### Added

- Add bulk operations API for agents (#397) (@houko)
- Add Z.AI and Kimi 2 model support (#409) (@houko)
- Add static Linux binary builds with musl target (#438) (@houko)
- Add multi-provider OAuth/OIDC authentication support (#454) (@houko)
- Add session retention policy with automatic cleanup (#516) (@houko)
- Add configurable message queue with concurrency settings (#517) (@houko)
- Add multi-language SDKs (JavaScript, Python, Go, Rust) (#531) (@houko)
- Auto-generate OpenAPI spec with utoipa (#534) (@houko)

### Fixed

- Complete vertex ai config wiring (#498) (@houko)
- Trim message history at safe turn boundaries (#521) (@houko)
- Add logging for X-API-Version header insertion failures (#524) (@houko)
- Fix SDK publishing (PyPI, npm, crates.io, GHCR) (#537) (@houko)
- Make release creation idempotent (#539) (@houko)
- Force-push tag in release.sh to handle re-releases (#540) (@houko)
- Use file instead of ldd to verify static linking (#541) (@houko)
- Allow re-release to overwrite existing assets (#542) (@houko)
- Allow desktop re-release to overwrite existing assets (#543) (@houko)
- Make SDK publishing idempotent for re-releases (#544) (@houko)
- Re-fetch PREV_TAG after deleting old tag in release.sh (#545) (@houko)

### Changed

- Split monolithic routes.rs into domain-specific modules (#452) (@houko)

### Maintenance

- Move binary size check from PR to release-only (#528) (@houko)
- Split release workflow into independent parallel pipelines (#533) (@houko)

## [0.5.3] - 2026-03-17

### Added

- Add bulk operations API for agents (#397) (@houko)
- Add Z.AI and Kimi 2 model support (#409) (@houko)
- Add static Linux binary builds with musl target (#438) (@houko)
- Add multi-provider OAuth/OIDC authentication support (#454) (@houko)
- Add session retention policy with automatic cleanup (#516) (@houko)
- Add configurable message queue with concurrency settings (#517) (@houko)
- Add multi-language SDKs (JavaScript, Python, Go, Rust) (#531) (@houko)
- Auto-generate OpenAPI spec with utoipa (#534) (@houko)

### Fixed

- Complete vertex ai config wiring (#498) (@houko)
- Trim message history at safe turn boundaries (#521) (@houko)
- Add logging for X-API-Version header insertion failures (#524) (@houko)

### Changed

- Split monolithic routes.rs into domain-specific modules (#452) (@houko)

### Maintenance

- Move binary size check from PR to release-only (#528) (@houko)
- Split release workflow into independent parallel pipelines (#533) (@houko)

## [0.5.2] - 2026-03-16

### Fixed

- Auto-update contributors list from GitHub API (#512) (@houko)
- Use local SVG for contributors with circular avatars (#513) (@houko)
- WeCom secret env pattern + add pre-commit fmt hook (#518) (@houko)

### Maintenance

- Auto-merge release PRs after CI passes (#511) (@houko)

## [0.5.1] - 2026-03-16

### Fixed

- Improve API version negotiation and local provider detection (#507) (@houko)
- Inject vault secrets into process env at startup (#509) (@houko)

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

## [0.4.3-beta3] - 2026-03-14

### Fixed

- Render card empty due to nested anchor tags (#292) (@houko)
- Use WEBSITE_REPO_TOKEN for star history workflow (#281) (@houko)
- Auto-merge star history PR after creation (#280) (@houko)
- Use PR instead of direct push for star history workflow (#279) (@houko)
- Move Fly.io-specific badges from header to deploy form (#278) (@houko)

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

## [0.4.0] - 2026-03-14

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
