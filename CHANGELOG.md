# Changelog

All notable changes to LibreFang will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project uses [Calendar Versioning](https://calver.org/) (YYYY.M.DD).

## [2026.4.5] - 2026-04-05

### Added

- Add inline tool use display to chat UI (#2031) (@neo-wanderer)
- Support username and @username in allowed_users filter (#2036) (@leszek3737)
- Add alibaba coding plan as provider (#2040) (@joshuachong)
- Add hidden models — hide/unhide models from selectors (#2045) (@leszek3737)
- HITL notification engine, batch ops, modify-and-retry, audit log (#2046) (@houko)
- Add media generation page (#2051) (@houko)
- Redesign Hands page with running strip and richer cards (#2052) (@houko)
- Redesign Hands detail modal with hero, action bar, metrics strip (#2053) (@houko)
- Polish Hands list — grid skeleton, empty states, degraded (#2054) (@houko)
- Per-channel command policy for public-facing bots (#2063) (@houko)

### Fixed

- Stop embedding dashboard artifacts in release commits (#2039) (@houko)
- Remove tracked static/react/ build artifacts from git (#2041) (@houko)
- Trigger dashboard build on release publish (#2043) (@houko)
- Strip provider prefix from agent fallback_models (#2047) (@houko)
- Ensure static/react dir exists for include_dir! (#2048) (@houko)
- Defer WebSocket close until connection is established (#2050) (@houko)
- Hands detail modal tab bar height, underline, and schedules label (#2055) (@houko)
- Remove count pills from Hands detail tabs to guarantee equal height (#2056) (@houko)
- Auto-wire self handle in streaming path for inter-agent tools (#2061) (@houko)
- Scope per-turn recall by peer_id to stop cross-user leaks (#2062) (@houko)

### Documentation

- Update dashboard build references after static/react removal (#2042) (@houko)
- Clarify routing lives in agent manifest, not config.toml (#2060) (@houko)

### Maintenance

- Fix 20 pre-existing TypeScript errors (#2049) (@houko)


## [2026.4.4] - 2026-04-04

### Added

- Interactive model switcher dropdown in connection bar (#1995) (@neo-wanderer)
- Custom model management, workflow scheduling, and HandsPage fixes (#2028) (@houko)
- Wire up channel test/reload and session labels (#2030) (@houko)
- Serve dashboard from runtime directory with auto-sync (#2032) (@houko)

### Fixed

- Prevent duplicate TOML keys during config upgrade (#2025) (@houko)
- Unify scheduling system, improve dashboard and hand UX (#2026) (@houko)
- Sync Cargo.lock for flate2/tar dependencies (#2034) (@houko)


## [2026.4.3] - 2026-04-03

### Fixed

- Use plain reqwest client in integration tests (#2000) (@houko)
- Add elevenlabs support to API key test endpoints (#2005) (@Chukwuebuka-2003)
- Add retry logic to release asset upload steps (#2007) (@houko)


## [2026.4.2] - 2026-04-02

### Added

- Press 'r' in just dev to git pull and rebuild (#1949) (@houko)
- Inline session switcher in chat (#1953) (@houko)
- Dev hotkeys and auto-pull (#1955) (@houko)

### Fixed

- Expose cleanup_orphan_sessions on MemorySubstrate (#1943) (@houko)
- Skip non-GET requests in service worker cache (#1944) (@houko)
- Route hand agent workspace to hands/ instead of agents/ (#1945) (@houko)
- Preserve depends_on when instantiating templates (#1946) (@houko)
- Add proxy timeout and WebSocket support for dev server (#1947) (@houko)
- Respect usage_footer config in chat message footer (#1948) (@houko)
- Git pull from origin/main in dev hotkey (#1950) (@houko)
- Validate provider keys and model availability on boot (#1951) (@houko)
- Use fetch+rebase for dev 'r' hotkey (#1952) (@houko)
- Remove unused binary_clone variable (#1954) (@houko)
- Match usage_footer values to backend snake_case (#1956) (@houko)
- Serialize usage_footer with serde instead of Debug format (#1957) (@houko)
- Point skillhub API to skillhub.tencent.com (#1958) (@houko)
- Skillhub install via COS direct download (#1959) (@houko)
- Remove hardcoded default models and add model availability probe (#1960) (@houko)
- Install FangHub skills from local registry instead of GitHub (#1961) (@houko)
- Infer provider from model name in fallback resolution (#1962) (@houko)
- FangHub install and search use local registry (#1963) (@houko)
- Mark unreachable local providers as unavailable (#1964) (@houko)
- Assistant agent model not updated when config changes (#1965) (@houko)
- Test provider should check CLI availability before requiring API key (#1966) (@houko)
- Local provider status driven by probe, not detect_auth (#1967) (@houko)
- Filter hand agents from analytics and telemetry (#1968) (@houko)
- Rename plugin source to plugin marketplace in Chinese locale (#1969) (@houko)
- Remove install button from plugins page header (#1970) (@houko)
- Startup health check respects explicit api_key_env config (#1973) (@houko)

### Changed

- Remove bundled system and add per-hand skill install (#1942) (@houko)


## [2026.4.1] - 2026-04-01

### Added

- Add ssrf_allowed_hosts allowlist for web_fetch (#1899) (@houko)
- Add embedding provider auto-detection (#1901) (@houko)
- Translate built-in agent names in dashboard (#1913) (@houko)

### Fixed

- Sync streaming fixes (#1897) (@houko)
- Sync config defaults (#1898) (@houko)
- Trigger ReloadSkills on skills config TOML changes (#1900) (@houko)
- Prevent users=[] conflict with [[users]] array-of-tables (#1904) (@houko)
- Fix file_write failed bug when create directory with non-exists … (#1905) (@shilkazx)
- Google_tts size check and is_ssml false-positive test coverage (#1906) (@houko)
- Prevent NO_REPLY token from leaking in group chats (#1908) (@f-liva)
- Resolve symlinked workspace roots on macOS (#1910) (@houko)

### Maintenance

- Fetch full tag history so diff link is populated (#1907) (@houko)


## [2026.3.31] - 2026-03-31

### Fixed

- Replace _redirects with _worker.js for SPA routing (#1824) (@houko)
- Add auto-init step to Windows installer (#1825) (@houko)
- Auto-init on first run for start/chat commands (#1826) (@houko)
- Resolve all open issues (#1827 #1828 #1829 #1830 #1832) (#1834) (@houko)
- Add missing message_timeout_secs in test DefaultModelConfig (#1835) (@houko)
- Add missing message_timeout_secs in DefaultModelConfig initializers (#1836) (@houko)
- Remove needless borrow for clippy (Rust 1.94) (#1838) (@houko)

### Documentation

- Fix development guide with just usage and dashboard debugging (#1831) (@houko)
- Add Windows exe manual install guide (#1833) (@houko)

### Maintenance

- Fix workflow trigger issues and add concurrency controls (#1822) (@houko)
- Remove redundant web-lint workflow (#1823) (@houko)


## [2026.3.30] - 2026-03-30

### Added

- Add configurable IMAP email reader (#1322) (@devatsecure)
- Add message debounce with shutdown flush (#1684) (@Chukwuebuka-2003)
- Convert markdown to WhatsApp formatting (#1733) (@f-liva)
- Add WeCom callback mode UI (#1773) (@houko)
- Add AGENTS.md for AI assistant context (#1779) (@houko)
- Add password change support (#1780) (@houko)
- Add registry_mirror for faster marketplace access in China (#1783) (@houko)
- Add wildcard pattern support for tool capabilities (#1801) (@houko)
- Add voice channel adapter with WebSocket server (#1802) (@houko)
- Add DingTalk stream mode support (#1804) (@houko)
- Auto-init config and copy example on first just dev (#1808) (@houko)
- Add Streamable HTTP transport, custom headers, and browser.enabled config (#1809) (@houko)

### Fixed

- Auth bootstrap for protected sessions (#1687) (@TechWizard9999)
- Allow Windows absolute paths in secrets.env and config.toml writes (#1770) (@SenZhangAI)
- Load full workflow detail after template instantiation (#1772) (@SenZhangAI)
- Add event_id dedup to feishu adapter (#1776) (@houko)
- Skip disabled agents during background startup (#1777) (@houko)
- Stop hiding hand agents from chat sidebar (#1778) (@houko)
- Align probe result fields with dashboard (#1781) (@houko)
- Handle all HTTP error codes in provider test (#1782) (@houko)
- Refresh provider catalog in-place after registry write (#1784) (@houko)
- Add versioned migration flow with best-effort fallback (#1785) (@houko)
- Improve NO_REPLY detection, raise history limit, preserve user messages (#1787) (@f-liva)
- Don't cancel in-progress runs on main branch (#1788) (@houko)
- Use per-SHA concurrency group on main to prevent SIGTERM (#1794) (@houko)
- Install npm in runtime image (#1799) (@j5bart)
- Route Telegram messages to correct agent (#1803) (@houko)
- Throttle Ubuntu test to prevent OOM SIGTERM (#1805) (@houko)
- Limit nextest to 1 concurrent test binary on Ubuntu (#1807) (@houko)
- Respect default_agent in channel message routing (#1810) (@houko)
- Propagate group context and @mention detection (#1811) (@houko)
- Complete group chat support (P1-P3) (#1812) (@houko)
- Use mutable default for non-exhaustive config struct (#1814) (@houko)
- Add missing PromptContext fields from WhatsApp group PR (#1816) (@houko)
- Re-apply provider URLs after runtime catalog sync (#1818) (@leszek3737)
- Remove duplicate is_group/was_mentioned in PromptContext (#1820) (@houko)

### Other

- Update dashboard image in markdown (#1746) (@Jengro777)


## [2026.3.28] - 2026-03-28

### Added

- TUI guide for free provider setup on first run (#1731) (@houko)
- Add set-as-default button to provider UI (#1753) (@houko)

### Fixed

- Use English for shared contacts label (#1732) (@f-liva)
- Use live default model for provider auth checks (#1748) (@TechWizard9999)
- Hot-reload Wecom channel config without restart (#1754) (@houko)
- Use effective default provider instead of hardcoded OpenRouter (#1755) (@houko)
- Add parse_mode and sanitization to streaming initial message (#1759) (@f-liva)
- Avoid blocking_write panic in daemon on Termux/Android (#1765) (@houko)

### Maintenance

- Batch upgrade dependencies (#1752) (@houko)


## [2026.3.26] - 2026-03-26

### Added

- Persist workflow run state to survive daemon restarts (#1657) (@houko)
- Add nvidia/nim aliases for nvidia-nim provider (#1660) (@houko)
- Sync and serve channel metadata from registry (#1661) (@houko)
- Integrate goal system into agent loop and prompt builder (#1663) (@houko)
- Migrate MCP stdio transport to rmcp SDK, fix env leak (#1667) (@houko)
- Implement all missing hot-reload actions (#1679) (@houko)
- Pluggable VectorStore backend with HTTP implementation (#1691) (@houko)
- Multimodal memory schema foundation for image indexing (#1692) (@houko)
- Add 5 operator-facing config fields (tool_timeout, upload_size, concurrency, call_depth, body_size) (#1709) (@houko)
- Add /api/registry/schema endpoint for dashboard form generation (#1715) (@houko)
- Add upgrade mode to librefang init (#1723) (@houko)
- Replace WeCom app with intelligent bot WebSocket adapter (#1729) (@houko)

### Fixed

- Replace unsafe pointer mutation in budget config updates (#1637) (@houko)
- Make metering quota check and usage record atomic (#1638) (@houko)
- Add TTL-based expiration for A2A task store (#1639) (@houko)
- Track background tasks for graceful shutdown (#1640) (@houko)
- Use atomic DashMap entry API for agent registry name index (#1641) (@houko)
- Replace production panics with error handling (#1642) (@houko)
- Support multiple Hand instances with instance-scoped agent IDs (#1643) (@houko)
- Auto-patch node-gyp on Termux/Android for better-sqlite3 native build (#1649) (@houko)
- Use centralized http_client to avoid rustls-platform-verifier panic on Termux (#1650) (@houko)
- Centralize registry sync to prevent parallel git clone races (#1651) (@houko)
- Pin DNS resolution to prevent SSRF rebinding attacks (#1653) (@houko)
- Add 8 missing fields to strict config validation (#1654) (@houko)
- Log warnings for malformed LLM tool call arguments (#1655) (@houko)
- Add per-trigger cooldown to prevent event storms (#1656) (@houko)
- Resolve WhatsApp gateway config path from $HOME instead of hardcoded /data/ (#1658) (@houko)
- Enforce workspace sandbox and tool capability checks (#1665) (@houko)
- Dashboard auth dialog never shown when api_key is configured (#1666) (@houko)
- Add dropped event monitoring to event bus (#1668) (@houko)
- Docker symlink, memory merge, workflow conditions, config test (#1670) (@houko)
- Enforce tool call and cost quotas in scheduler (#1671) (@houko)
- Apply cache token discount and update model prices (#1672) (@houko)
- Implement OAuth refresh token flow (#1673) (@houko)
- Replace XOR obfuscation with Argon2 key wrapping (#1674) (@houko)
- Make config hot-reload atomic with epoch counter (#1676) (@houko)
- Remove dead client field from WebFetchEngine (#1678) (@houko)
- Restore backward-compatible agent IDs for single-instance hands (#1680) (@houko)
- Re-land SSRF DNS pinning to prevent TOCTOU rebinding attacks (#1681) (@houko)
- Budget enforcement, complete API error migration, cache invalidation (#1683) (@houko)
- Clippy warnings and rustfmt from recent merges (#1685) (@houko)
- Update hand tests for legacy agent ID format (#1686) (@houko)
- Sync workflow templates from registry on boot (#1688) (@houko)
- Remove workflows from registry sync (kernel handles this separately) (#1689) (@houko)
- Webchat responses silently dropped due to stream timeout and missing routing context (#1690) (@houko)
- Resolve compilation errors from merged PR conflicts (#1712) (@houko)
- Suppress clippy::manual_clamp in clamp_bounds (#1716) (@houko)
- Remove dangling doc comment in ws.rs (#1717) (@houko)
- Wrap load_templates_from_dir with block_in_place (#1719) (@houko)
- Repair test failures from goal system merge (#1720) (@houko)
- Recognize all available auth statuses for custom providers in WebUI (#1721) (@houko)
- Correct test expectations for metering and workflow collect (#1722) (@houko)
- Accept "Failed to resolve" error in Windows capability test (#1725) (@houko)
- Auto-detect default LLM provider, fix WeChat QR flashing (#1727) (@houko)

### Changed

- Standardize API error response format (#1646) (@houko)
- Deduplicate LLM driver request building and fix streaming (#1669) (@houko)
- Deduplicate constants and auto-generate user-agent version (#1693) (@houko)
- Remove pub const provider URLs, inline in driver registry (#1695) (@houko)
- Extract registry cache TTL into configurable RegistryConfig (#1698) (@houko)
- Extract API rate limiting constants into RateLimitConfig (#1701) (@houko)
- Extract compaction constants into CompactionConfig (#1704) (@houko)
- Extract trigger system constants into TriggersConfig (#1705) (@houko)
- Extract channel timeout and polling constants into per-channel config (#1707) (@houko)
- Move workflow template sync from kernel boot to registry_sync (#1713) (@houko)

### Performance

- Cache available_tools computation per agent (#1644) (@houko)

### Maintenance

- Extract build_agent_manifest_toml from tool_agent_spawn and test (#1648) (@aimlyo)
- Remove bundled integration templates from source tree (#1659) (@houko)
- Fix formatting issues caught by CI (#1714) (@houko)


## [2026.3.25] - 2026-03-25

### Added

- TUI multi-select provider menu in deploy script (#1618) (@houko)
- Add publish links to SDK release job summary (#1623) (@houko)
- Limit-the-degrees-of-freedom-of-agent_spawn (#1624) (@aimlyo)

### Fixed

- Read from /dev/tty in deploy script for curl-pipe compatibility (#1616) (@houko)
- TUI arrow key navigation crashes due to set -e (#1620) (@houko)
- Add -- to grep patterns in release workflows (#1622) (@houko)
- Use isolated test dir for model_catalog tests (#1627) (@houko)
- Resolve DMG asset name mismatch in Homebrew Cask sync (#1628) (@houko)
- Embed contributor avatars as base64 in SVG (#1630) (@houko)
- Always tag Docker image as :latest (#1631) (@houko)

### Maintenance

- Stop marking beta/rc as GitHub prerelease (#1626) (@houko)


## [2026.3.24] - 2026-03-24

### Added

- Implement depends_on DAG execution for workflow steps (#1440) (@houko)
- Add workflow template API endpoints (#1442) (@houko)
- Wire thinking model configuration into agent loop (#1443) (@houko)
- Mobile responsive + PWA + login + skill output persistence (#1445) (@houko)
- Implement session context injection with multiple sources (#1448) (@houko)
- Save existing workflow as reusable template (#1449) (@houko)
- Add Shell/Bash skill runtime (#1450) (@houko)
- Add push messaging API for agents to send to channels (#1451) (@houko)
- Add /btw ephemeral side question command (#1452) (@houko)
- Add structured output (JSON/JSON Schema) for agents (#1453) (@houko)
- Add session export/import for context hibernation (#1454) (@houko)
- Configurable heartbeat timeout and pruning per agent (#1455) (@houko)
- Cross-session wake via target_agent on triggers (#1456) (@houko)
- Add interactive message payloads for Telegram and Slack (#1457) (@houko)
- Add PII privacy controls with pseudonymization and redaction (#1458) (@houko)
- Tool-level authorization with per-sender and channel-specific policies (#1459) (@houko)
- Subagent context inheritance in workflow steps (#1460) (@houko)
- Lazy-load LLM driver cache for improved runtime performance (#1461) (@houko)
- Add Amazon Bedrock embedding driver with SigV4 signing (#1462) (@houko)
- FTS5 full-text session search with API endpoint (#1463) (@houko)
- Message injection between tool calls (mid-turn interrupt) (#1464) (@houko)
- Render LaTeX in chat (#1467) (@TechWizard9999)
- Automatic memory chunking for long documents (#1468) (@houko)
- Input sanitizer for prompt injection detection (#1469) (@houko)
- Add Android (aarch64) cross-compilation for Termux users (#1470) (@houko)
- Time-based memory decay for hierarchical memory management (#1471) (@houko)
- File-based input inbox for async external commands (#1472) (@houko)
- Interactive approval dialog in dashboard chat and channel events (#1474) (@houko)
- Telegram thread-based agent routing (#1475) (@houko)
- Pause/resume, busy guard, AgentManifest composition (#1482) (@houko)
- Add librefang-testing crate with mock infrastructure (#1483) (@houko)
- Show GitHub compare link before version confirmation (#1488) (@houko)
- Integrate Skillhub marketplace as second skill source (#1504) (@houko)
- Add WeChat personal account adapter via iLink protocol (#1506) (@houko)
- Comprehensive build automation CLI with 31 subcommands (#1511) (@houko)
- Enhance Hand system with i18n, pause/resume, and dashboard overhaul (#1515) (@houko)
- Enable by default, add Grafana, auto-start with Docker (#1520) (@houko)
- Multi-agent hand architecture (#1521) (@houko)
- Add regex group trigger patterns (#1529) (@TechWizard9999)
- Generic media generation drivers (image, TTS, video, music) (#1532) (@houko)
- Extend Prometheus metrics and add Grafana dashboards (#1533) (@houko)
- Add LTS version support (#1535) (@houko)

### Fixed

- Handle paginated /api/agents response (#1233) (@f-liva)
- Preserve caption on Telegram voice messages (#1249) (@f-liva)
- Detect and retry when LLM skips tool execution for action requests (#1413) (@houko)
- Stop agent loop on tool execution failure (#948) (#1415) (@houko)
- Complete ChatGPT Responses driver streaming/tool/reasoning mapping (#1405) (#1421) (@houko)
- Use 2-digit year in Tauri version for WiX MSI compatibility (#1439) (@houko)
- Harden workflow permissions and catalog path validation (#1444) (@SenZhangAI)
- Stabilize nodeTypes to fix workflow builder editing (#1447) (@houko)
- Harden reconnect and request handling (#1465) (@TechWizard9999)
- CI shell injection, clippy warnings, init config, and review findings (#1473) (@houko)
- Validate tool_use.input as dict in Anthropic and OpenAI drivers (#1476) (@houko)
- Replace plaintext password with Argon2id hashing (#1477) (@houko)
- Replace git-based registry sync with HTTP tarball download (#1479) (@houko)
- Hand registry race condition, state persistence, and optional requirements (#1481) (@houko)
- Resolve clippy errors blocking all PRs (#1486) (@houko)
- Consolidate confirmations into single final prompt (#1491) (@houko)
- Align chat websocket contract (#1498) (@poruru-code)
- Exempt non-autonomous agents from timeout check (#1499) (@houko)
- Stamp last_active before LLM call (#1500) (@houko)
- Reset last_active on agent restore (#1501) (@houko)
- Resolve clippy and compilation errors from merged PRs (#1502) (@houko)
- Use tokio::test for callback query tests (#1503) (@houko)
- Resolve compilation and clippy errors from recent merges (#1507) (@houko)
- Update tool fallback assertions for capability enforcement (#1508) (@houko)
- Follow up merged PR regressions (#1514) (@houko)
- Use endpoint discovery API for Feishu WebSocket connection (#1518) (@houko)
- Gitignore, channel logging, and xtask Windows CI (#1519) (@houko)
- Preserve coordinator role and role-bound trigger migration (#1523) (@houko)
- Restore --release flag in Dockerfile build (#1524) (@houko)
- Eliminate username enumeration timing side-channel (#1525) (@houko)
- Replace deterministic session token with random generation (#1526) (@houko)
- Prevent path traversal in skill script execution (#1527) (@houko)
- Make init_prometheus idempotent for parallel test safety (#1528) (@houko)
- Multi-agent parsing compat + registry sync version update (#1530) (@houko)
- Gate unix-only test behind #[cfg(unix)] (#1534) (@houko)
- Release tool compares against latest tag including prereleases (#1547) (@houko)
- Release tool retries commit after formatter hook (#1548) (@houko)
- Release tool compares against latest tag including prereleases (#1547) (#1550) (@houko)
- Remove unused find_latest_stable_tag in release.rs (#1551) (@houko)

### Changed

- Add facade getters and migrate API routes (#1478) (@houko)
- Modularize route registration into per-domain routers (#1484) (@houko)
- Split monolithic config.rs (5566 LOC) into modular sub-modules (#1485) (@houko)
- Registry as catalog, pre-install core content only (#1537) (@houko)
- Unified workspaces layout + hand/agent isolation + routing fixes (#1542) (@houko)

### Maintenance

- Cover claude code skip permissions args (#1364) (@TechWizard9999)
- Fix 16 Dependabot security alerts (#1438) (@SenZhangAI)
- Translate all Chinese comments to English (#1509) (@houko)

### Other

- Feature/opentel (#1516) (@Chukwuebuka-2003)
- Feature/fix gitignore (#1517) (@houko)


## [2026.3.23] - 2026-03-23

### Added

- Add pipeline runner agents + IMAP email reader script (#1307) (@devatsecure)
- Add ChatGPT device auth flow (#1332) (@poruru-code)
- Add Qwen International and US provider endpoints (#1370) (@houko)
- Add custom log directory config (#1379) (@houko)
- Enrich ClassifiedError with provider/model context (#1380) (@houko)
- Add rustfmt.toml for consistent code formatting (#1381) (@houko)
- Display version and git hash in startup logs (#1382) (@houko)
- Add unfurl_links config option for Slack channel (#1383) (@houko)
- Add DeepInfra as LLM provider (#1384) (@houko)
- Add configurable embedding dimensions (#1386) (@houko)
- Add config validation with tolerant mode (#1387) (@houko)
- Add Azure OpenAI provider support (#1388) (@houko)
- Add force_flat_replies config for Slack channels (#1390) (@houko)
- Add fts_only mode for memory indexing without embedding (#1391) (@houko)
- Add global workspace directory for cross-session persistence (#1392) (@houko)
- Add mention_patterns config for Discord channels (#1394) (@houko)
- Add WorkflowTemplate types and in-memory registry (#1395) (@houko)
- Add configurable session reset prompt (#1396) (@houko)
- Add per-agent plugin scoping with allowed_plugins (#1399) (@houko)
- Add /reboot slash command for graceful context reset (#1401) (@houko)
- Support arbitrary config keys in skill entries (#1402) (@houko)
- Add Homebrew Cask CI sync and improve Formula generation (#1404) (@houko)
- Comprehensive React dashboard UI/UX overhaul (#1419) (@houko)
- Add refresh param to bypass worker cache for migration (#1426) (@houko)
- Add Japanese dashboard localization (#1427) (@poruru-code)
- Add a new Librefang promotional SVG banner and update the corre… (#1429) (@houko)
- Just api starts dashboard dev server alongside API (#1434) (@houko)
- Implement depends_on DAG execution for workflow steps (#1440) (@houko)
- Add workflow template API endpoints (#1442) (@houko)
- Wire thinking model configuration into agent loop (#1443) (@houko)
- Mobile responsive + PWA + login + skill output persistence (#1445) (@houko)
- Implement session context injection with multiple sources (#1448) (@houko)
- Save existing workflow as reusable template (#1449) (@houko)
- Add Shell/Bash skill runtime (#1450) (@houko)
- Add push messaging API for agents to send to channels (#1451) (@houko)
- Add /btw ephemeral side question command (#1452) (@houko)
- Add structured output (JSON/JSON Schema) for agents (#1453) (@houko)
- Add session export/import for context hibernation (#1454) (@houko)
- Configurable heartbeat timeout and pruning per agent (#1455) (@houko)
- Cross-session wake via target_agent on triggers (#1456) (@houko)
- Add interactive message payloads for Telegram and Slack (#1457) (@houko)
- Add PII privacy controls with pseudonymization and redaction (#1458) (@houko)
- Tool-level authorization with per-sender and channel-specific policies (#1459) (@houko)
- Subagent context inheritance in workflow steps (#1460) (@houko)
- Lazy-load LLM driver cache for improved runtime performance (#1461) (@houko)
- Add Amazon Bedrock embedding driver with SigV4 signing (#1462) (@houko)
- FTS5 full-text session search with API endpoint (#1463) (@houko)
- Message injection between tool calls (mid-turn interrupt) (#1464) (@houko)
- Render LaTeX in chat (#1467) (@TechWizard9999)
- Automatic memory chunking for long documents (#1468) (@houko)
- Input sanitizer for prompt injection detection (#1469) (@houko)
- Add Android (aarch64) cross-compilation for Termux users (#1470) (@houko)
- Time-based memory decay for hierarchical memory management (#1471) (@houko)
- File-based input inbox for async external commands (#1472) (@houko)
- Interactive approval dialog in dashboard chat and channel events (#1474) (@houko)
- Telegram thread-based agent routing (#1475) (@houko)
- Pause/resume, busy guard, AgentManifest composition (#1482) (@houko)
- Add librefang-testing crate with mock infrastructure (#1483) (@houko)
- Show GitHub compare link before version confirmation (#1488) (@houko)
- Integrate Skillhub marketplace as second skill source (#1504) (@houko)
- Add WeChat personal account adapter via iLink protocol (#1506) (@houko)
- Comprehensive build automation CLI with 31 subcommands (#1511) (@houko)
- Enhance Hand system with i18n, pause/resume, and dashboard overhaul (#1515) (@houko)
- Enable by default, add Grafana, auto-start with Docker (#1520) (@houko)
- Multi-agent hand architecture (#1521) (@houko)
- Add regex group trigger patterns (#1529) (@TechWizard9999)
- Generic media generation drivers (image, TTS, video, music) (#1532) (@houko)
- Extend Prometheus metrics and add Grafana dashboards (#1533) (@houko)
- Add LTS version support (#1535) (@houko)

### Fixed

- Handle paginated /api/agents response (#1233) (@f-liva)
- Preserve caption on Telegram voice messages (#1249) (@f-liva)
- Correct language toggle logic in navigation sidebar (#1349) (@danilopopeye)
- Escape < in MDX comparison table to fix build (#1350) (@houko)
- Escape < in MDX troubleshooting page (#1351) (@houko)
- Resolve compilation errors breaking CI clippy check (#1353) (@houko)
- Clean stale registry dir before clone to prevent CI race condition (#1356) (@houko)
- Handle re-release in release.sh when no files changed (#1360) (@houko)
- Register aliases for custom models (#1366) (@TechWizard9999)
- Knowledge_query JOIN matches entities by name or ID (#1369) (@houko)
- Browser hand connection failure on Windows (#1371) (@houko)
- Infinite retry guard, dead branch cleanup, body size limit (#1372) (@houko)
- Workflow editor save handles nested mode/error_mode from frontend (#1373) (@houko)
- Scope knowledge JOIN by agent_id and add entities.name index (#1374) (@houko)
- Replace fragile cmd.len() < 50 heuristic in LoopGuard poll detection (#1378) (@houko)
- Fix sidebar navigation, broken links, and i18n issues (#1385) (@houko)
- Comprehensive website polish and bug fixes (#1389) (@houko)
- Accept [hand] wrapper in HAND.toml format (#1393) (@houko)
- Fix OG image, brand naming, PWA manifest, and missing i18n keys (#1397) (@houko)
- Improve Qwen Code CLI path detection (#1398) (@houko)
- Respect provider field when routing custom models (#1400) (@houko)
- Remove empty sections overrides and fix mobile nav indicators (#1406) (@houko)
- Correct Docker compose port binding for admin interface (#944) (#1407) (@houko)
- Allow hyphens in MCP server names (#947) (#1408) (@houko)
- Resolve GitHub stats zeros and optimize KV operations (#1409) (@houko)
- Load .env files in desktop app (#1410) (@houko)
- Prevent streaming interrupts during multi-tool sequences (#1411) (@houko)
- Resolve skill file paths for installed skill execution (#1412) (@houko)
- Detect and retry when LLM skips tool execution for action requests (#1413) (@houko)
- Cache workspace and skill metadata to reduce per-message overhead (#1414) (@houko)
- Stop agent loop on tool execution failure (#948) (#1415) (@houko)
- Replace processed images with text placeholders in session history (#911) (#1416) (@houko)
- Complete ChatGPT Responses driver streaming/tool/reasoning mapping (#1405) (#1421) (@houko)
- Migrate old KV keys to history blob and handle sparse chart data (#1422) (@houko)
- Complete dashboard i18n coverage for goals and analytics (#1423) (@poruru-code)
- Correct provider counts, model numbers, and free tier status (#1424) (@houko)
- Update Hands count to 14 and add deploy/registry links (#1428) (@houko)
- Release.sh grep compatibility on macOS (#1431) (@houko)
- Correct Cloudflare Pages _redirects SPA fallback format (#1432) (@houko)
- Release.sh — macOS grep compat + full diff link (#1433) (@houko)
- Generate anchor IDs for h3 headings and preserve TOML-style names (#1435) (@houko)
- Use 2-digit year in Tauri version for WiX MSI compatibility (#1439) (@houko)
- Harden workflow permissions and catalog path validation (#1444) (@SenZhangAI)
- Stabilize nodeTypes to fix workflow builder editing (#1447) (@houko)
- Harden reconnect and request handling (#1465) (@TechWizard9999)
- CI shell injection, clippy warnings, init config, and review findings (#1473) (@houko)
- Validate tool_use.input as dict in Anthropic and OpenAI drivers (#1476) (@houko)
- Replace plaintext password with Argon2id hashing (#1477) (@houko)
- Replace git-based registry sync with HTTP tarball download (#1479) (@houko)
- Hand registry race condition, state persistence, and optional requirements (#1481) (@houko)
- Resolve clippy errors blocking all PRs (#1486) (@houko)
- Consolidate confirmations into single final prompt (#1491) (@houko)
- Align chat websocket contract (#1498) (@poruru-code)
- Exempt non-autonomous agents from timeout check (#1499) (@houko)
- Stamp last_active before LLM call (#1500) (@houko)
- Reset last_active on agent restore (#1501) (@houko)
- Resolve clippy and compilation errors from merged PRs (#1502) (@houko)
- Use tokio::test for callback query tests (#1503) (@houko)
- Resolve compilation and clippy errors from recent merges (#1507) (@houko)
- Update tool fallback assertions for capability enforcement (#1508) (@houko)
- Follow up merged PR regressions (#1514) (@houko)
- Use endpoint discovery API for Feishu WebSocket connection (#1518) (@houko)
- Gitignore, channel logging, and xtask Windows CI (#1519) (@houko)
- Preserve coordinator role and role-bound trigger migration (#1523) (@houko)
- Restore --release flag in Dockerfile build (#1524) (@houko)
- Eliminate username enumeration timing side-channel (#1525) (@houko)
- Replace deterministic session token with random generation (#1526) (@houko)
- Prevent path traversal in skill script execution (#1527) (@houko)
- Make init_prometheus idempotent for parallel test safety (#1528) (@houko)
- Multi-agent parsing compat + registry sync version update (#1530) (@houko)
- Gate unix-only test behind #[cfg(unix)] (#1534) (@houko)
- Release tool compares against latest tag including prereleases (#1547) (@houko)
- Release tool retries commit after formatter hook (#1548) (@houko)

### Changed

- Switch to CalVer (YYYY.M.DDHH) (#1375) (@houko)
- Add facade getters and migrate API routes (#1478) (@houko)
- Modularize route registration into per-domain routers (#1484) (@houko)
- Split monolithic config.rs (5566 LOC) into modular sub-modules (#1485) (@houko)
- Registry as catalog, pre-install core content only (#1537) (@houko)
- Unified workspaces layout + hand/agent isolation + routing fixes (#1542) (@houko)

### Documentation

- Comprehensive review — fix errors, update numbers, add missing sections (#1368) (@houko)

### Maintenance

- Lock api status version regression (#1363) (@TechWizard9999)
- Cover claude code skip permissions args (#1364) (@TechWizard9999)
- Cover hand reactivation runtime profile (#1365) (@TechWizard9999)
- Cover local model default override routing (#1367) (@TechWizard9999)
- Auto-update PR branches on main push (#1417) (@houko)
- Add GitHub Stats Worker to deploy workflow (#1420) (@houko)
- Remove deploy worker job-level if conditions that fail on squash merges (#1425) (@houko)
- Fix 16 Dependabot security alerts (#1438) (@SenZhangAI)
- Translate all Chinese comments to English (#1509) (@houko)

### Other

- Feature/opentel (#1516) (@Chukwuebuka-2003)
- Feature/fix gitignore (#1517) (@houko)


## [2026.3.22] - 2026-03-22

### Added

- Add pipeline runner agents + IMAP email reader script (#1307) (@devatsecure)
- Add ChatGPT device auth flow (#1332) (@poruru-code)
- Add Qwen International and US provider endpoints (#1370) (@houko)
- Add custom log directory config (#1379) (@houko)
- Enrich ClassifiedError with provider/model context (#1380) (@houko)
- Add rustfmt.toml for consistent code formatting (#1381) (@houko)
- Display version and git hash in startup logs (#1382) (@houko)
- Add unfurl_links config option for Slack channel (#1383) (@houko)
- Add DeepInfra as LLM provider (#1384) (@houko)
- Add configurable embedding dimensions (#1386) (@houko)
- Add config validation with tolerant mode (#1387) (@houko)
- Add Azure OpenAI provider support (#1388) (@houko)
- Add force_flat_replies config for Slack channels (#1390) (@houko)
- Add fts_only mode for memory indexing without embedding (#1391) (@houko)
- Add global workspace directory for cross-session persistence (#1392) (@houko)
- Add mention_patterns config for Discord channels (#1394) (@houko)
- Add WorkflowTemplate types and in-memory registry (#1395) (@houko)
- Add configurable session reset prompt (#1396) (@houko)
- Add per-agent plugin scoping with allowed_plugins (#1399) (@houko)
- Add /reboot slash command for graceful context reset (#1401) (@houko)
- Support arbitrary config keys in skill entries (#1402) (@houko)
- Add Homebrew Cask CI sync and improve Formula generation (#1404) (@houko)
- Comprehensive React dashboard UI/UX overhaul (#1419) (@houko)
- Add refresh param to bypass worker cache for migration (#1426) (@houko)
- Add Japanese dashboard localization (#1427) (@poruru-code)
- Add a new Librefang promotional SVG banner and update the corre… (#1429) (@houko)
- Just api starts dashboard dev server alongside API (#1434) (@houko)

### Fixed

- Register aliases for custom models (#1366) (@TechWizard9999)
- Knowledge_query JOIN matches entities by name or ID (#1369) (@houko)
- Browser hand connection failure on Windows (#1371) (@houko)
- Infinite retry guard, dead branch cleanup, body size limit (#1372) (@houko)
- Workflow editor save handles nested mode/error_mode from frontend (#1373) (@houko)
- Scope knowledge JOIN by agent_id and add entities.name index (#1374) (@houko)
- Replace fragile cmd.len() < 50 heuristic in LoopGuard poll detection (#1378) (@houko)
- Fix sidebar navigation, broken links, and i18n issues (#1385) (@houko)
- Comprehensive website polish and bug fixes (#1389) (@houko)
- Accept [hand] wrapper in HAND.toml format (#1393) (@houko)
- Fix OG image, brand naming, PWA manifest, and missing i18n keys (#1397) (@houko)
- Improve Qwen Code CLI path detection (#1398) (@houko)
- Respect provider field when routing custom models (#1400) (@houko)
- Remove empty sections overrides and fix mobile nav indicators (#1406) (@houko)
- Correct Docker compose port binding for admin interface (#944) (#1407) (@houko)
- Allow hyphens in MCP server names (#947) (#1408) (@houko)
- Resolve GitHub stats zeros and optimize KV operations (#1409) (@houko)
- Load .env files in desktop app (#1410) (@houko)
- Prevent streaming interrupts during multi-tool sequences (#1411) (@houko)
- Resolve skill file paths for installed skill execution (#1412) (@houko)
- Cache workspace and skill metadata to reduce per-message overhead (#1414) (@houko)
- Replace processed images with text placeholders in session history (#911) (#1416) (@houko)
- Migrate old KV keys to history blob and handle sparse chart data (#1422) (@houko)
- Complete dashboard i18n coverage for goals and analytics (#1423) (@poruru-code)
- Correct provider counts, model numbers, and free tier status (#1424) (@houko)
- Update Hands count to 14 and add deploy/registry links (#1428) (@houko)
- Release.sh grep compatibility on macOS (#1431) (@houko)
- Correct Cloudflare Pages _redirects SPA fallback format (#1432) (@houko)
- Release.sh — macOS grep compat + full diff link (#1433) (@houko)
- Generate anchor IDs for h3 headings and preserve TOML-style names (#1435) (@houko)

### Changed

- Switch to CalVer (YYYY.M.DDHH) (#1375) (@houko)

### Documentation

- Comprehensive review — fix errors, update numbers, add missing sections (#1368) (@houko)

### Maintenance

- Lock api status version regression (#1363) (@TechWizard9999)
- Cover hand reactivation runtime profile (#1365) (@TechWizard9999)
- Cover local model default override routing (#1367) (@TechWizard9999)
- Auto-update PR branches on main push (#1417) (@houko)
- Add GitHub Stats Worker to deploy workflow (#1420) (@houko)
- Remove deploy worker job-level if conditions that fail on squash merges (#1425) (@houko)

## [2026.3.21] - 2026-03-21

### Added

- Add pipeline runner agents + IMAP email reader script (#1307) (@devatsecure)
- Add ChatGPT device auth flow (#1332) (@poruru-code)
- Add Qwen International and US provider endpoints (#1370) (@houko)
- Add custom log directory config (#1379) (@houko)
- Enrich ClassifiedError with provider/model context (#1380) (@houko)
- Add rustfmt.toml for consistent code formatting (#1381) (@houko)
- Display version and git hash in startup logs (#1382) (@houko)
- Add unfurl_links config option for Slack channel (#1383) (@houko)
- Add DeepInfra as LLM provider (#1384) (@houko)
- Add configurable embedding dimensions (#1386) (@houko)
- Add config validation with tolerant mode (#1387) (@houko)
- Add Azure OpenAI provider support (#1388) (@houko)
- Add force_flat_replies config for Slack channels (#1390) (@houko)
- Add fts_only mode for memory indexing without embedding (#1391) (@houko)
- Add global workspace directory for cross-session persistence (#1392) (@houko)
- Add mention_patterns config for Discord channels (#1394) (@houko)
- Add WorkflowTemplate types and in-memory registry (#1395) (@houko)
- Add configurable session reset prompt (#1396) (@houko)
- Add per-agent plugin scoping with allowed_plugins (#1399) (@houko)
- Add /reboot slash command for graceful context reset (#1401) (@houko)
- Support arbitrary config keys in skill entries (#1402) (@houko)
- Add Homebrew Cask CI sync and improve Formula generation (#1404) (@houko)
- Comprehensive React dashboard UI/UX overhaul (#1419) (@houko)
- Add refresh param to bypass worker cache for migration (#1426) (@houko)
- Add Japanese dashboard localization (#1427) (@poruru-code)
- Add a new Librefang promotional SVG banner and update the corre… (#1429) (@houko)

### Fixed

- Register aliases for custom models (#1366) (@TechWizard9999)
- Knowledge_query JOIN matches entities by name or ID (#1369) (@houko)
- Browser hand connection failure on Windows (#1371) (@houko)
- Infinite retry guard, dead branch cleanup, body size limit (#1372) (@houko)
- Workflow editor save handles nested mode/error_mode from frontend (#1373) (@houko)
- Scope knowledge JOIN by agent_id and add entities.name index (#1374) (@houko)
- Replace fragile cmd.len() < 50 heuristic in LoopGuard poll detection (#1378) (@houko)
- Fix sidebar navigation, broken links, and i18n issues (#1385) (@houko)
- Comprehensive website polish and bug fixes (#1389) (@houko)
- Accept [hand] wrapper in HAND.toml format (#1393) (@houko)
- Fix OG image, brand naming, PWA manifest, and missing i18n keys (#1397) (@houko)
- Improve Qwen Code CLI path detection (#1398) (@houko)
- Respect provider field when routing custom models (#1400) (@houko)
- Remove empty sections overrides and fix mobile nav indicators (#1406) (@houko)
- Correct Docker compose port binding for admin interface (#944) (#1407) (@houko)
- Allow hyphens in MCP server names (#947) (#1408) (@houko)
- Resolve GitHub stats zeros and optimize KV operations (#1409) (@houko)
- Load .env files in desktop app (#1410) (@houko)
- Prevent streaming interrupts during multi-tool sequences (#1411) (@houko)
- Resolve skill file paths for installed skill execution (#1412) (@houko)
- Cache workspace and skill metadata to reduce per-message overhead (#1414) (@houko)
- Replace processed images with text placeholders in session history (#911) (#1416) (@houko)
- Migrate old KV keys to history blob and handle sparse chart data (#1422) (@houko)
- Complete dashboard i18n coverage for goals and analytics (#1423) (@poruru-code)
- Correct provider counts, model numbers, and free tier status (#1424) (@houko)
- Update Hands count to 14 and add deploy/registry links (#1428) (@houko)

### Changed

- Switch to CalVer (YYYY.M.DDHH) (#1375) (@houko)

### Documentation

- Comprehensive review — fix errors, update numbers, add missing sections (#1368) (@houko)

### Maintenance

- Lock api status version regression (#1363) (@TechWizard9999)
- Cover hand reactivation runtime profile (#1365) (@TechWizard9999)
- Cover local model default override routing (#1367) (@TechWizard9999)
- Auto-update PR branches on main push (#1417) (@houko)
- Add GitHub Stats Worker to deploy workflow (#1420) (@houko)
- Remove deploy worker job-level if conditions that fail on squash merges (#1425) (@houko)

## [0.7.0] - 2026-03-21

### Added

- Configurable CORS, channel rate limits, audit pruning, and media gates (#1331) (@houko)
- Docs (#1334) (@houko)
- LLM intent routing, registry single source of truth, streaming fixes (#1336) (@houko)
- Add migrate --from openfang for OpenFang users (#1344) (@houko)
- Unify CLI detection + add Gemini CLI, Codex CLI, Aider providers (#1347) (@houko)

### Fixed

- Move CLI npm/PyPI publish to Shell workflow and fix Fly.io config path (#1327) (@houko)
- Strip provider prefix for internal LLM calls (#1330) (@houko)
- Sync upstream improvements (#1338) (@houko)
- Harden OpenClaw migration inputs (#1342) (@houko)
- Complete openfang migration across init wizard, API, and dashboard (#1345) (@houko)
- Detect Qwen Code CLI in test_connection and setup wizard (#1346) (@houko)
- Correct language toggle logic in navigation sidebar (#1349) (@danilopopeye)
- Escape < in MDX comparison table to fix build (#1350) (@houko)
- Escape < in MDX troubleshooting page (#1351) (@houko)
- Resolve compilation errors breaking CI clippy check (#1353) (@houko)
- Clean stale registry dir before clone to prevent CI race condition (#1356) (@houko)
- Handle re-release in release.sh when no files changed (#1360) (@houko)

### Changed

- Consolidate docs as Next.js deployment directory (#1335) (@houko)

### Documentation

- Add comparison page and clean up remaining artifacts (#1337) (@houko)

### Other

- Feature/fix docs (#1339) (@houko)


## [0.6.8] - 2026-03-20

### Added

- Add owner routing for external DM responses (#1266) (@f-liva)
- Distribute CLI binary via npm and PyPI (#1323) (@houko)

### Fixed

- Use GitHub API to create Go SDK tag (#1321) (@houko)

### Maintenance

- Remove wasteful workflows and fix bugs (#1320) (@houko)

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
