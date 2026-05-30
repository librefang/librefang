# Changelog

All notable changes to LibreFang will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project uses [Calendar Versioning](https://calver.org/) (YYYY.M.DD).

## [2026.5.28] - 2026-05-28

_46 PRs from 5 contributors since v2026.5.25-beta.13._

### Breaking Changes

- Rust sidecar adapter SDK + AI-codegen-era rationale rewrite (#5821) (@houko)

### Added

- Per-agent channel allowlist (#5738) (@DaBlitzStein)
- Implement describe_image() and wire ImageFile description through channel adapters (#5815) (@houko)
- Rust sidecar adapter SDK + AI-codegen-era rationale rewrite (#5821) (@houko)

### Fixed

- Isolate attachment pre-inject per chat session — close cross-chat image leak (#5334) (@f-liva)
- Make migrate path containment existence-independent (fixes #5716) (#5719) (@houko)
- Repair discussion-to-issue backfill — gh api --jq doesn't take --arg (#5754) (@houko)
- Tool_runner taint — unified SECRET_KEYS, substring match, header trim, single-pass normalization (#5762) (@leszek3737)
- Tool_runner shell_safety — command injection hardening, quote-aware tokenizer (#5765) (@leszek3737)
- Tool_runner definitions — ALWAYS_NATIVE complete, OnceLock caches, schema fixes, tool_name constants (#5771) (@leszek3737)
- Tool_runner error — Upstream message preserved, MissingParameter String, ResourceNotFound 404 (#5772) (@leszek3737)
- Tool_runner cron — sender_id override, TOCTOU reduction, HashSet lookup, empty job_id rejected (#5773) (@leszek3737)
- Tool_runner dispatch — mutex split, fallback ACL, ACP args, spill wiring, snapshot ordering (#5774) (@leszek3737)
- Tool_runner spill — config-based threshold, validation, fast-path (#5776) (@leszek3737)
- Tool_runner wiki — limit cap, input validation, safe usize, caller_agent_id required (#5777) (@leszek3737)
- Tool_runner meta — case-insensitive lookup, Cow optimization, deterministic sort (#5779) (@leszek3737)
- Tool_runner task — typed deserialization, contextual errors, empty validation, status default (#5780) (@leszek3737)
- Tool_runner notify — length limit, control char sanitization, PII removal (#5782) (@leszek3737)
- Tool_runner hand — deterministic sort, empty id reject, config whitelist, output sanitization (#5784) (@leszek3737)
- Tool_runner goal — progress type fix, range validation (#5785) (@leszek3737)
- Tool_runner event — event_type validation, caller identity, reserved prefix guard (#5786) (@leszek3737)
- Tool_runner a2a — session_id taint, SSRF diagnostics, zero-alloc agent check (#5787) (@leszek3737)
- Tool_runner artifact — spawn_blocking, explicit errors, usize safe, zero-length reject (#5788) (@leszek3737)
- Tool_runner channel — poll u8 safe, file size limit, email regex, mirror dedup, thread_id routing (#5789) (@leszek3737)
- Skip bridge-side formatting for sidecar adapters (fixes #5795) (#5796) (@DaBlitzStein)
- Return forward-slash relative path from registry/content on Windows (#5801) (@houko)
- Make step timeout errors actionable with remediation guidance (#5806) (@houko)
- Eliminate Instant subtraction that panics on Windows CI (fixes #5726) (#5808) (@houko)
- Seed Feishu/Lark configure form when Python SDK is absent (#5809) (@houko)
- Unbreak coverage build — thread session_id into two SessionWriter test stubs (#5816) (@houko)
- Wiki.rs lifetime + shell.rs test arity after #5774/#5777 (#5818) (@houko)
- Unbreak main — agent channels in ApiDoc + fmt + secrets baseline (#5820) (@houko)
- Install gh CLI for release flow (#5826) (@houko)
- Run `gh auth setup-git` to unblock git push from container (#5827) (@houko)
- Override host-absolute credential helper path inside container (#5829) (@houko)

### Changed

- Migrate tool_runner tools to ToolError (#3576) (#5737) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Maintenance

- Bump the cargo-minor-patch group with 4 updates (#5748) (@app/dependabot)
- Bump wasmtime from 44.0.1 to 45.0.0 (#5749) (@app/dependabot)
- Bump sysinfo from 0.38.4 to 0.39.2 (#5750) (@app/dependabot)
- Bump which from 7.0.3 to 8.0.2 (#5751) (@app/dependabot)
- Bump tikv-jemallocator from 0.6.1 to 0.7.0 (#5752) (@app/dependabot)
- Bump the actions-minor-patch group with 4 updates (#5790) (@app/dependabot)
- Bump actions/setup-python from 5 to 6 (#5791) (@app/dependabot)
- Bump the web-minor-patch group in /web with 3 updates (#5810) (@app/dependabot)
- Bump the dashboard-minor-patch group in /crates/librefang-api/dashboard with 7 updates (#5811) (@app/dependabot)
- Bump globals from 15.15.0 to 17.6.0 in /crates/librefang-api/dashboard (#5812) (@app/dependabot)
- Docker fallback for `just release` when cargo is missing (#5825) (@houko)

</details>


## [2026.5.25] - 2026-05-25

_308 PRs from 7 contributors since v2026.5.17-beta.12._

### Breaking Changes

- Migrate ntfy from in-process adapter to sidecar (P7) (#5224) (@houko)
- Remove in-process telegram adapter (now sidecar-only) (#5241) (@houko)
- Migrate gotify from in-process adapter to sidecar (#5263) (@houko)
- Migrate mastodon from in-process adapter to sidecar (#5264) (@houko)
- Remove 6 low-value channel adapters (#5265) (@houko)
- Drop 12 unmaintained adapters (#5267) (@houko)
- Migrate bluesky from in-process adapter to sidecar (#5277) (@houko)
- Migrate reddit from in-process adapter to sidecar (#5281) (@houko)
- Migrate twitch from in-process adapter to sidecar (#5297) (@houko)
- Migrate rocketchat from in-process adapter to sidecar (#5298) (@houko)
- Migrate discord from in-process adapter to sidecar (#5299) (@houko)
- Migrate nextcloud from in-process adapter to sidecar (#5301) (@houko)
- Migrate slack from in-process adapter to sidecar (#5302) (@houko)
- Migrate webex from in-process adapter to sidecar (#5309) (@houko)
- Migrate zulip from in-process adapter to sidecar (#5310) (@houko)
- Migrate line from in-process adapter to sidecar (#5312) (@houko)
- Migrate mattermost from in-process adapter to sidecar (#5315) (@houko)
- Migrate signal from in-process adapter to sidecar (#5317) (@houko)
- Migrate qq from in-process adapter to sidecar (#5325) (@houko)
- Migrate matrix from in-process adapter to sidecar (#5368) (@houko)
- Migrate feishu from in-process adapter to sidecar (#5380) (@houko)
- Migrate wecom from in-process adapter to sidecar (WebSocket-only) (#5392) (@houko)
- Migrate email from in-process adapter to sidecar (#5408) (@houko)
- Migrate dingtalk from in-process adapter to sidecar (Stream mode only) (#5417) (@houko)
- Migrate wechat from in-process adapter to sidecar (#5421) (@houko)
- Migrate teams from in-process adapter to sidecar (#5433) (@houko)
- Migrate whatsapp from in-process adapter to sidecar (dual-mode) (#5445) (@houko)
- Migrate webhook from in-process adapter to sidecar (#5455) (@houko)
- Migrate google_chat from in-process adapter to sidecar (#5459) (@houko)
- Delete dead per-channel REST endpoints + their helpers (#5463) (@houko)

### Highlights

- **Channel adapter sidecar migration** — all 27 messaging integrations (Slack, Discord, Telegram, WhatsApp, Signal, Teams, and more) are now isolated sidecar processes instead of in-process adapters; 18 unmaintained adapters were removed. Sidecar adapters can be configured directly from the dashboard.
- **Human-in-the-loop (HITL) approval step** — agents can now pause and request operator approval mid-run; approvals route back to the originating chat with inline keyboard buttons on supported adapters, and the same tool only prompts once per session.
- **Credential pools** — configure multiple API keys per LLM provider for automatic round-robin rotation and instant failover on rate limits.
- **Schedule tab & budget visibility** — the dashboard now has an editable Schedule tab for managing triggers, cron jobs, and continuous mode; a new per-provider budget caps surface shows spend and limits per provider.
- **Security hardening** — session tokens are now hashed at rest, SSRF validation added to URL inputs, path-traversal guards tightened across asset and file routes, SQL bindings replace string concatenation in session cleanup, and request bodies are size-capped against pre-allocation DoS.

### Added

- Credential pools — multi-key rotation per provider with… (#5063) (@Chukwuebuka-2003)
- Add per-agent memory isolation via agent_id parameter (#5071) (@leszek3737)
- Propagate W3C traceparent to outbound LLM HTTP requests (#5190) (@neo-wanderer)
- Implement HITL operator-step — notify dispatch, timeout watchdog, HTTP actions→resume (#5133, #5134, #5135) (#5191) (@houko)
- Caller-controlled conversation_key for agent_send (#5212) (@houko)
- Forced /compact with async spawn, ack+event, summary banner (#5213) (@houko)
- Sidecar channel parity — protocol, supervision, config (P0–P3) (#5219) (@houko)
- Python sidecar channel adapter framework (P4) (#5220) (@houko)
- Hard-block new in-process channel adapters (P5) (#5221) (@houko)
- Migrate ntfy from in-process adapter to sidecar (P7) (#5224) (@houko)
- Compute wasMentioned from group_trigger_patterns when mentionedJids is empty (#5230) (@f-liva)
- Telegram full sidecar parity (formatter + full inbound/outbound), stdlib-only (#5232) (@houko)
- Remove in-process telegram adapter (now sidecar-only) (#5241) (@houko)
- Configure sidecar adapters (telegram/ntfy) from dashboard (#5252) (@houko)
- Editable Schedule tab — triggers, cron, continuous mode (#4924) (#5256) (@houko)
- HITL operator-step dashboard surfaces (#4977) (#5257) (@houko)
- Credential pools for multi-key per-provider rotation (#4965) (#5260) (@houko)
- Migrate gotify from in-process adapter to sidecar (#5263) (@houko)
- Migrate mastodon from in-process adapter to sidecar (#5264) (@houko)
- Migrate bluesky from in-process adapter to sidecar (#5277) (@houko)
- Migrate reddit from in-process adapter to sidecar (#5281) (@houko)
- Migrate twitch from in-process adapter to sidecar (#5297) (@houko)
- Migrate rocketchat from in-process adapter to sidecar (#5298) (@houko)
- Migrate discord from in-process adapter to sidecar (#5299) (@houko)
- Migrate nextcloud from in-process adapter to sidecar (#5301) (@houko)
- Migrate slack from in-process adapter to sidecar (#5302) (@houko)
- Migrate webex from in-process adapter to sidecar (#5309) (@houko)
- Migrate zulip from in-process adapter to sidecar (#5310) (@houko)
- Migrate line from in-process adapter to sidecar (#5312) (@houko)
- Migrate mattermost from in-process adapter to sidecar (#5315) (@houko)
- Migrate signal from in-process adapter to sidecar (#5317) (@houko)
- Migrate qq from in-process adapter to sidecar (#5325) (@houko)
- Migrate matrix from in-process adapter to sidecar (#5368) (@houko)
- Migrate feishu from in-process adapter to sidecar (#5380) (@houko)
- Migrate wecom from in-process adapter to sidecar (WebSocket-only) (#5392) (@houko)
- Migrate email from in-process adapter to sidecar (#5408) (@houko)
- Migrate dingtalk from in-process adapter to sidecar (Stream mode only) (#5417) (@houko)
- Migrate wechat from in-process adapter to sidecar (#5421) (@houko)
- Migrate teams from in-process adapter to sidecar (#5433) (@houko)
- Migrate whatsapp from in-process adapter to sidecar (dual-mode) (#5445) (@houko)
- Migrate webhook from in-process adapter to sidecar (#5455) (@houko)
- Migrate google_chat from in-process adapter to sidecar (#5459) (@houko)
- Restore ChannelsPage as a sidecar-only page (#5470) (@houko)
- Embed librefang-sdk + reconnect WeChat QR flow (#5472) (@houko)
- Approval notifications use inline keyboard on interactive-capable adapters (#5483) (@houko)
- Route approval popup to originating chat (follow-up to #5483) (#5484) (@houko)
- Thread chat_id through approval flow for group-chat support (#5489) (@houko)
- Cache per-session approvals so the same tool only prompts once (#5663) (@houko)
- Per-agent [proactive_memory] extraction_model override (#5475) (#5690) (@houko)
- Bootstrap ESLint with jsx-no-target-blank guard (fixes #5561) (#5701) (@houko)
- Propagate kernel-attested caller context to MCP servers (fixes #5699) (#5704) (@houko)
- Expose per-provider budget caps surface (#5705) (@houko)

### Fixed

- Force HOME so spawned CLI can find its credentials (#4997) (@f-liva)
- Distinguish JoinError cancellation from panic in streaming bridge (#5058) (#5064) (@leszek3737)
- Spill oversized MCP/tool results to artifact store before truncation (#5149) (@neo-wanderer)
- Deny unknown fields in request DTOs to catch body typos (#5131) (#5151) (@houko)
- Validate expression at insert and auto-disable on repeated fallback (#5160) (@houko)
- Unwedge cooldown on wall-clock backstep (#5162) (@houko)
- Respect per-agent fallback_models override — None inherits global, Some([]) opts out (#5167) (@DaBlitzStein)
- Serde/config polish (#5145) (#5172) (@houko)
- AuxClient inherits agent fallback chain when [llm.auxiliary] unset (#5169) (#5173) (@houko)
- Cap rate-limited autonomous loop re-fires (#5168) (#5174) (@houko)
- Time/clock/scheduling robustness (#5136) (#5175) (@houko)
- Surface swallowed errors on persistence/IO paths (#5137) (#5176) (@houko)
- Enforce prompt-cache key determinism (#5143) (#5177) (@houko)
- Security defense-in-depth — symlink/archive/header/IP edge cases (#5141) (#5178) (@houko)
- Enforce per-user memory/wiki ACL at tool dispatch (#5139) (#5179) (@houko)
- Concurrency hazard follow-ups — kill_agent run/abort lifecycle (#5142) (#5180) (@houko)
- Memory substrate data integrity (#5138) (#5181) (@houko)
- Data-layer invalidation + a11y + dead code (#5140) (#5182) (@houko)
- Task lifecycle / resource-leak follow-ups (#5144) (#5184) (@houko)
- Reject same-task re-entrant agent_msg_lock acquisition (#5125, #5126) (#5187) (@houko)
- Show full agent name on hover in chat sidebar (#5188) (@neo-wanderer)
- Regenerate OpenAPI/SDK/schema baselines for #5151 DTO changes (#5165) (#5189) (@houko)
- Prevent history_fold mid-string truncation on verbose-JSON models (#5206) (@houko)
- Re-enable send button on typing:stop (#5207) (@houko)
- /context reports real model context window (#5208) (@houko)
- Surface config deserialize errors and fail closed on hard parse failure (#5209) (@houko)
- Honor token-trigger in inner compaction gate (#5210) (@houko)
- Canonical session pointer recovery on restart (#5198, #5199) (#5211) (@houko)
- Cover ChainExhausted in PooledDriver match (unblock main) (#5215) (@houko)
- Restore rustfmt-clean main after #5209 (#5214) (#5216) (@houko)
- Expose background section + drop stale /api/cron/list allowlist row (#5217) (@houko)
- Sidecar protocol/SDK follow-ups from #5219/#5220 review (#5223) (@houko)
- Move first-party channel adapters out of examples into librefang-sdk (#5228) (@houko)
- Unwrap ephemeral/viewOnce/edited wrappers before reading contextInfo (closes #48) (#5229) (@f-liva)
- Surface producer crash via ProducerCrashed, not SystemExit (#5231) (@houko)
- Handle inbound poll_answer in telegram adapter (sidecar parity) (#5242) (@houko)
- Close kill_agent/dispatch race + break HITL self-cycle (#5244 follow-ups) (#5244) (@houko)
- Unblock main — pass force=false in compact gate test (#5210/#5213 collision) (#5245) (@houko)
- Sidecar channels visible AND read-only on the dashboard (no 404 actions) (#5249) (@houko)
- Surface telegram/ntfy discovery rows on the channels page (#5250) (@houko)
- Auto-pin agentId-only sessions + bind dropdown active to live connection (#5199) (#5253) (@houko)
- Cron picker click no longer closes schedule form (#5247) (#5254) (@houko)
- Agent wizard tools/skills selectable + MCP servers dropdown (#5246) (#5255) (@houko)
- Follow-ups from third sidecar-configure review (#5261) (@houko)
- Block cross-chat memory bleed via chat-scoped recall (#5227) (#5262) (@houko)
- Patch Baileys executeInitQueries to non-blocking allSettled (#5268) (@f-liva)
- Align opentelemetry stack on 0.32 to fix main build break (#5279) (@houko)
- Include kernel Bearer token on all REST forwards (#5285) (@f-liva)
- Thread sender context through streaming message handler (#5288) (@f-liva)
- Skip file-upload OCR for image/* mime types (closes #5290) (#5291) (@DaBlitzStein)
- Add default_agent to SidecarChannelConfig — restore inbound routing pin (closes #5294) (#5295) (@DaBlitzStein)
- Restore main — fmt drift, MCP caller_agent_id semantics, openapi baseline (#5300) (@houko)
- Honour Retry-After across sidecar polling adapters (#5303) (@houko)
- Emit poll bursts in chronological order across sidecar adapters (#5305) (@houko)
- Restore main — fmt drift + stale config schema baseline (#5307) (@houko)
- Detect chat-template `[User]` line-leader as cascade leak (#5308) (@f-liva)
- Update openclaw test fixtures after mattermost sidecar (closes #5316) (#5318) (@houko)
- Wrap config sub-tabs + hide number-input spinner buttons (closes #5293) (#5319) (@houko)
- Pin response_format = Json on history_fold + web_augment aux calls (closes #5287) (#5320) (@houko)
- Observability + regression coverage on sidecar reconnect loop (closes #5111) (#5321) (@houko)
- Define api-error-generic across all 6 locales (audit: api-error-generic-missing-fluent-key) (#5322) (@houko)
- Use canonical getStoredApiKey for export download (audit: audit-export-401) (#5324) (@houko)
- Purge pending_approvals on agent cascade-delete + schema-walking guard (audit: agent-cascade-delete-missing-tables) (#5328) (@houko)
- Refuse to boot without LIBREFANG_STATE_SECRET when external_auth.enabled (closes #5336) (#5337) (@houko)
- Validate skill name + hand against path traversal (closes #5338) (#5339) (@houko)
- Wrap upload_routes in route-local RequestBodyLimitLayer (closes #5342) (#5343) (@houko)
- Cap triggers per agent at MAX_TRIGGERS_PER_AGENT = 50 (closes #5345) (#5346) (@houko)
- Verify caller owns from_agent_id before comms_send (closes #5349) (#5350) (@houko)
- SSRF-validate URLs at create + update (closes #5352) (#5353) (@houko)
- Gate require_auth_for_reads=false bypass behind external_auth_proxy (closes #5356) (#5357) (@houko)
- Add /api/auth/callback to rate-limit allowlist (closes #5358) (#5359) (@houko)
- Expect() on serde_json::to_writer in stream_json (closes #5360) (#5361) (@houko)
- Write Argon2id upgrade-hint to 0600 file instead of log (closes #5364) (#5365) (@houko)
- Kernel_err_to_status helper for 404/409 mapping (closes #5366) (#5367) (@houko)
- Require auth on GitHub Copilot OAuth endpoints (closes #5369) (#5370) (@houko)
- Atomic-rename write for secrets.env eliminates 0644 TOCTOU (closes #5371) (#5372) (@houko)
- Scrub raw rusqlite errors before responding (#5378) (@houko)
- Split /api/auth/login allowlist into exact + slash-prefix (#5382) (@houko)
- Always emit Secure on logout cookie clear (#5384) (@houko)
- Anchor [SILENT] cron marker to message prefix (#5386) (@houko)
- Clamp listing endpoints — no more limit=None → full collection (#5388) (@houko)
- Rel=noopener noreferrer + safeUrl on MCP catalog get_url (#5390) (@houko)
- Hand-write Debug to redact OAuthTokens secrets (#5395) (@houko)
- Warn on serde(other) Unknown variants with raw tag (#5397) (@houko)
- Bind cleanup_orphan_sessions IN-clause instead of string-concat (#5401) (@houko)
- Hotfix dangling refs from #5368 + #5380 sidecar migrations (FeishuConfig + pulldown-cmark) (#5402) (@houko)
- Drop dangling channels.feishu access in openclaw roundtrip test (#5404) (@houko)
- Bound regex cache at 4096 entries with FIFO eviction (#5406) (@houko)
- Per-process random anonymous fingerprint (#5410) (@houko)
- Wire check_json_depth into global request middleware (#5412) (@houko)
- Use SHA-256 (128-bit truncated) for DriverCache::cache_key (#5414) (@houko)
- Persist trimmed active_sessions after periodic GC (#5419) (@houko)
- Tighten SQLite database files to 0o600 + data dir to 0o700 (#5422) (@houko)
- Recover sessionWebhook via ChannelUser.librefang_user (#5423) (@houko)
- Sanitize Custom channel names that collide with kernel-internal cron/autonomous/webui (#5425) (@houko)
- Byte + char dual cap on chat-message size (#5427) (@houko)
- Saturating_add inner cache-token sum (#5430) (@houko)
- Recover passive-reply msg_id via ChannelUser.librefang_user (#5431) (@houko)
- Apply foreign_keys=ON + full PRAGMA set to PromptStore second pool (#5434) (@houko)
- Scan raw string for command substitution — close double-quote bypass (#5436) (@houko)
- Recover per-message reply correlation via ChannelUser.librefang_user across 6 sidecars (#5439) (@houko)
- Size-bounded PII regex compilation (#5444) (@houko)
- Repair persisted session after trim+pinned-rescue (#5447) (@houko)
- Recover context_token via librefang_user across sidecar restart (#5448) (@houko)
- Recover req_id via librefang_user across sidecar restart (#5449) (@houko)
- Include traceback + cmd_type when on_command bare-except logs (#5450) (@houko)
- WARN on env-vs-keyring master-key divergence (#5453) (@houko)
- Recover main build from sidecar fallout (missing default + orphans + test drift) (#5456) (@houko)
- Repair test build after #5455 (write_service_account_env removed) (#5460) (@houko)
- Cross-audit follow-ups (Retry-After x4, dedupe x2, LINE reply API) (#5462) (@houko)
- Rewrite ModuleNotFoundError into actionable install hint (#5465) (@houko)
- Preserve specific cause in last_error after circuit-breaker trip (#5468) (@houko)
- Redact WhatsApp JIDs atomically (no partial-redact via phone regex) (#5469) (@f-liva)
- Recover reply context via XRPC on cache miss (closes #5452) (#5471) (@houko)
- Demote /api/metrics 401 from WARN to DEBUG (#5482) (@houko)
- Repair 3 pre-existing main CI breakers inherited by all open PRs (#5486) (@houko)
- Ack duplicate `/approve <id>` instead of error-shaped not-found (#5487) (@houko)
- Wake idle agent after approval resolve so the chat gets the result (#5488) (@houko)
- Suppress redundant /approve|/reject ack on inline-keyboard tap (#5490) (@houko)
- Route agent reply through channel after wake — fixes "tap Approve → silence" (#5491) (@houko)
- Cargo fmt + regenerate sdk/ to repair main CI (Quality, OpenAPI Drift) (#5494) (@houko)
- Log only email domain at INFO in OIDC auth_callback (#5504) (@houko)
- Sanitize reserved channel names at every SenderContext ingress (#5506) (@houko)
- Return path relative to home_dir, not absolute (#5509) (@houko)
- Keyboard nav for NotificationCenter (WAI-ARIA Menu Button) (#5510) (@houko)
- Record comms_send in hash-chained audit log (#5512) (@houko)
- Reject empty code in OAuth callback before token exchange (#5515) (@houko)
- SSRF-validate attachment URLs + DNS-rebind pin (#5517) (@houko)
- Cap bulk-handler Vec::with_capacity to prevent DoS pre-allocation (#5520) (@houko)
- Bound buckets map with hard cap + periodic sweep (#5522) (@houko)
- Never log raw IdP token-endpoint response bodies (#5526) (@houko)
- Detect partial-upgrade drift between migrations table and user_version pragma (#5528) (@houko)
- Never silently default or fabricate from corrupt JSON-in-TEXT columns (#5532) (@houko)
- Reclaim per-session bucket on session delete (#5534) (@houko)
- Bound RoundRobin cursor with cycle-aware iteration (#5536) (@houko)
- Restore main — rustfmt drift + 2 PR-only test failures (#5538) (@houko)
- Release prune lock across try_summarize_trim().await; CAS on messages_generation (#5541) (@houko)
- Validate provider name shape before deriving env var (#5542) (@houko)
- Bijective SHA-256 agent_id suffix to stop container-name collisions (#5545) (@houko)
- Hold ledger mutex across check + add (#5548) (@houko)
- Validate tool args at boundary before forwarding to MCP server (#5550) (@houko)
- Acquire per-agent semaphore in workflow send_message closure (#5554) (@houko)
- Cap system_prompt size and lock down create-handler invariants (#5558) (@houko)
- Allow zero spaces in attribution regex (#5560) (@houko)
- Swap RefCell for parking_lot::Mutex to remove async borrow-panic footgun (#5563) (@houko)
- Reject `..` per-segment in react_asset, not by substring (#5565) (@houko)
- Switch useSessionStream to authenticated WebSocket (#5567) (@houko)
- Make agent_concurrency_for entry construction atomic (#5569) (@houko)
- Hash session tokens at rest in sessions.json — backup-snapshot replay resistance (#5571) (@houko)
- #[serde(skip_serializing)] api_key + proxy_url (#5573) (@houko)
- Escape translator HTML, route via <Trans> (#5576) (@houko)
- Canonicalize + containment-check source/target_dir (#5577) (@houko)
- Warn at boot when declared provider API-key env vars are unset or empty (#5579) (@houko)
- Gate X-Forwarded-Proto on trusted_proxies for session cookie Secure flag (#5581) (@houko)
- Allowlist --network and --cap-add to prevent sandbox collapse (#5583) (@houko)
- Install-deps program allowlist + flag denylist + Owner-only role (#5588) (@houko)
- Warn on manifest swap when session_mode or max_concurrent_invocations changes (#5590) (@houko)
- Remove partial identity files on write failure (#5592) (@houko)
- Evict JWKS + discovery caches on external_auth hot-reload (#5594) (@houko)
- Fail-closed when guard-bash-safety lib is missing (#5596) (@houko)
- Rephrase strip_images placeholder so LLM does not deny image reception (#5597) (@DaBlitzStein)
- Wrap connect_mcp_servers spawns in spawn_supervised (#5599) (@houko)
- Hold Lane::Trigger permit across run_workflow spawn (#5602) (@houko)
- Derive deterministic SessionId for New-mode fires (#5604) (@houko)
- Align missed-fire log with single-catchup behaviour (#5606) (@houko)
- Classify refresh failures, single-flight refresh, drop unwrap (#5609) (@houko)
- Allow known framework source dirs, not just the librefang home (#5614) (@houko)
- Backfill missing #[utoipa::path] handlers + regenerate openapi.json (#5620) (@houko)
- API-surface hygiene — SPA route allowlist, registry id validation, auth/providers gating (#5638) (@houko)
- Non-IdP external_auth edits are a no-op, not a restart (#5646) (@houko)
- Propagate sender peer_id through remember_interaction_b… (#5647) (@Chukwuebuka-2003)
- Persist /sync since_token across restarts (#5651) (@neo-wanderer)
- External_auth IdP change is hot-reload, not restart (restore main) (#5652) (@houko)
- Clear clippy Quality lane (needless borrow, doc indentation, manual char comparison, await-holding-lock) (#5654) (@houko)
- Downgrade boot integrity-check failure to WARN (#5659) (@houko)
- Migrate legacy shared-namespace row on fallback hit (#5660) (@houko)
- Bound graceful shutdown so daemon.lock release isn't blocked by a hung phase (#5662) (@houko)
- Plug data leaks, restore lost state, harden parsing (#5674) (@leszek3737)
- Harden pre-commit + add detect-secrets CI workflow (#5681) (@houko)
- CommsKeys hierarchy + TerminalTabs storage helper + Modal autoFocus (#5682) (@houko)
- Soft-cap in-memory entries between trims at 1.5x max_in_memory_entries (#5683) (@houko)
- Harden build.rs git/date invocation; document pnpm audit ignores (#5684) (@houko)
- WARN when [agents.<name>.proactive_memory] appears in config.toml (real path is agent.toml) (#5687) (@houko)
- Filter /commands dispatch by account_id (multi-bot isolation) (#5688) (@houko)
- Widen exclusions, regenerate baseline, ignore generated_at drift (#5691) (@houko)
- Update audit_retention_test for #5683 soft-cap drain (#5693) (@houko)
- Strip line_number drift from detect-secrets baseline diff (#5695) (@houko)
- Log bot_token fingerprint instead of full token (fixes #5543) (#5700) (@houko)
- Replace removed `all-channels` feature with `telemetry` (#5702) (@houko)
- Add provider_budget_routes_test to detect-secrets baseline (#5707) (@houko)
- Regenerate SDKs for /api/budget/providers to repair main CI drift (#5709) (@houko)
- Include sdk/python/librefang in flake source filter (#5714) (@houko)

### Changed

- Unify error contracts — RFC + ToolError + first migration (#3576) (#5258) (@houko)
- Extract shared helpers + WS client + test fakes (#5335) (@houko)
- Return librefang-types IntegrationError from install_integration (stop leaking ExtensionResult) (#5622) (@houko)
- Return types-owned outcome from install_integration (stop leaking InstallResult) (#5644) (@houko)
- Widen ApiErrorResponse::internal_scrub sweep across routes (#5661) (@houko)

### Performance

- Use count_sessions() on status + snapshot (audit: list-sessions-decode-on-poll) (#5326) (@houko)
- Use list_arcs() in agent_budget_ranking (closes #5347) (#5348) (@houko)
- Evict stale tool-call timestamps on push (closes #5362) (#5363) (@houko)
- Rotate to next key on first RateLimit (closes #5373) (#5374) (@houko)
- Tx-wrap recall access bump + batched IN hydrate (closes #5375) (#5376) (@houko)
- Composite sessions(agent_id, updated_at) + audit_entries(agent_id, timestamp) indexes (#5399) (@houko)
- Stream extract_text_content into a single String to avoid per-save Vec<String> allocation (#5501) (@houko)
- Offload SQLite insert+prune via spawn_blocking, counter-gate prune (#5524) (@houko)
- Block_in_place for ImageFile reads (4 sites) (#5530) (@houko)
- Memoize dashboard_snapshot_inner with 900ms TTL cache (#5552) (@houko)
- Unblock axum executor on create_backup + persist_budget (spawn_blocking) (#5556) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Documentation

- Sidecar-first channel documentation (P6) (#5225) (@houko)
- Import audit backlog (120 tracking items) (#5240) (@houko)
- Fix stale telegram.rs reference in custom-channel example (#5248) (@houko)
- Fill in [[sidecar_channels]] samples for all 27 adapters (#5464) (@houko)
- Canonical config-reload field table derived from build_reload_plan (#5642) (@houko)

### Maintenance

- Restore rustfmt-clean main (Quality CI gate) (#5222) (@houko)
- Add Dockerfile.rust-dev with Tauri Linux GTK deps (#5233) (@houko)
- Cross-impl protocol conformance corpus + versioned spec (v1) (#5237) (@houko)
- Remove 6 low-value channel adapters (#5265) (@houko)
- Drop per-merge auto-update trigger from auto-update-branches (#5266) (@houko)
- Drop 12 unmaintained adapters (#5267) (@houko)
- Bump the cargo-minor-patch group with 8 updates (#5269) (@app/dependabot)
- Bump opentelemetry-otlp from 0.31.1 to 0.32.0 (#5270) (@app/dependabot)
- Bump russh-keys from 0.45.0 to 0.49.2 (#5271) (@app/dependabot)
- Bump shlex from 1.3.0 to 2.0.1 (#5272) (@app/dependabot)
- Bump tracing-opentelemetry from 0.32.1 to 0.33.0 (#5273) (@app/dependabot)
- Cargo fmt — fix rustfmt drift on main after channel-removal merges (#5274) (@houko)
- Bump Apple-Actions/upload-testflight-build from 5.1.0 to 5.2.1 in the actions-minor-patch group (#5304) (@app/dependabot)
- Pin silent_response markers against prompt-builder output (#5344) (@f-liva)
- Drop pulldown-cmark workspace dep, orphaned by matrix sidecar #5368 (#5407) (@houko)
- Pin SessionMode strict-variant deserialization (audit-disputed) (#5416) (@houko)
- Bump the web-minor-patch group in /web with 9 updates (#5438) (@app/dependabot)
- Bump the dashboard-minor-patch group in /crates/librefang-api/dashboard with 12 updates (#5440) (@app/dependabot)
- 3 nits from post-merge audit (#5454) (@houko)
- Remove dead in-process channel scaffolding (#5461) (@houko)
- Delete dead per-channel REST endpoints + their helpers (#5463) (@houko)
- Rephrase docstring "stub" mentions to stop bot false positives (#5467) (@houko)
- Prune unused dependencies across the workspace (#5473) (@houko)
- Clean up sidecar migration tails (#5479) (@houko)
- Bump the docs-minor-patch group in /docs with 10 updates (#5493) (@app/dependabot)
- Skip Cloudflare Pages deploy on Dependabot PRs (#5495) (@houko)
- Run Coverage workflow on push:main only, not per-PR (#5496) (@houko)
- Make the per-PR test lane Linux-only (#5498) (@houko)
- Cover LIBREFANG_VAULT_KEY 32-ASCII-vs-32-bytes pitfall (#5611) (@houko)
- Replace fixed 150ms sleeps with condition-based polling (#5613) (@houko)
- Parallel semaphore-contention coverage for trigger concurrency caps (#5616) (@houko)
- Assert every KernelConfig field is reload-classified + backfill (#5619) (@houko)
- Replace unmaintained serde_yaml with serde_yaml_ng (RUSTSEC-2024-0320) (#5626) (@houko)
- Full-router semantic tests for lifecycle routes (suspend/resume/mode) (#5628) (@houko)
- Convert tools integration tests from mock to full router (#5630) (@houko)
- Convert load_test from mock to full router (exercise real middleware) (#5632) (@houko)
- Full-router semantic tests for files (path-traversal) + capabilities routes (#5634) (@houko)
- Convert agent_identity_registry tests from mock to full router (#5636) (@houko)
- Full-router semantic tests for clone/reload/push + bulk routes (#5640) (@houko)
- Delete 65 audit docs whose GitHub issue is closed (#5670) (@houko)
- Rename librefang-migrate → librefang-import + reconcile stale CLAUDE.md + justfile policy (#5668) (#5685) (@houko)

### Reverted

- Roll back v2026.5.25-beta.13 / beta.14 version bumps to 2026.5.17-beta.12 (#5717) (@houko)

### Other

- [Medium] Per-trigger `session_mode_override = New` is throttled by the manifest's `Persistent` clamp (#5624) (@houko)

</details>


## [Unreleased]

### Breaking Changes

- **Subprocess plugin sandbox is now secure-by-default** (#2) (@houko).
  Hook subprocesses no longer get network or filesystem access unless the plugin opts in.
  `allow_network` and `allow_filesystem` in a plugin's `[hooks]` table now default to `false` (previously `true`); a plugin that needs outbound network or filesystem writes must declare `allow_network = true` / `allow_filesystem = true` in its `plugin.toml`.
  Existing plugins that relied on the old open-by-default behaviour will stop reaching the network / writing files after upgrade until they add these declarations.
  The `seccomp-sandbox` and `landlock-sandbox` features (Linux syscall + LSM filtering) are now enabled in the default feature set; they are no-ops on macOS / Windows.

### Changed

- **refactor(subprocess): extract a shared `librefang-subprocess` transport; migrate the context-engine sidecar onto it** (@houko) — first step of unifying the three hand-rolled "persistent JSON-over-stdio subprocess" bridges (channels `SidecarAdapter`, `plugin_runtime::HookProcessPool`, `context_engine::SidecarContextEngine`), each of which re-implemented spawn + a background reply reader + id-matching + stderr draining + lifecycle, and had drifted apart (inconsistent reply-line caps, a missing write timeout in one, no exit signal in another). New low-level crate `crates/librefang-subprocess` owns that mechanism: `SubprocessTransport::spawn(TransportConfig)` + `request(json) -> Result<Value, TransportError>`, with a bounded reply-line read (default 16 MiB), a timeout that bounds the write as well as the reply wait, stderr → log, a `subprocess_transport_exited` metric, and `kill_on_drop` reaping. It depends on no `librefang-*` crate so both `librefang-channels` and `librefang-runtime` can sit above it. `SidecarContextEngine` is migrated to it, shedding ~220 lines of duplicated transport for a thin policy wrapper; behaviour is unchanged (5 engine tests still pass through the new transport). The extraction also fixed a latent semantic bug carried over from the inline code: a dead child now surfaces as `TransportError::Dead`, distinct from a `{"error": …}` reply (`TransportError::Remote`), instead of conflating the two. `read_capped_line` and `write_line_timeout` are exposed as low-level primitives for the consumers whose protocol isn't id-matched request/reply. `plugin_runtime::HookProcessPool` is migrated onto them: its persistent `do_call` now reads through the bounded-line primitive and writes through the timeout-bounded one, gaining the reply-line cap and the per-call timeout it previously lacked (it accepted `timeout_secs` but never enforced it — 20 plugin-runtime tests still pass). The channels `SidecarAdapter` — event-stream rather than request/reply — reuses the bounded-line primitive for its inbound reader (previously read with an unbounded `lines()`), keeping its own event protocol and supervision/respawn loop; its 472 unit + 27 bridge-integration + 4 conformance-corpus tests pass. All three bridges now share the crate's line handling, so a transport-layer fix lands once: consistent reply-line caps, a write timeout on every consumer, and no unbounded inbound reads.
- **context engine: the sidecar now auto-respawns instead of degrading until restart** (@houko) — builds on the shared transport above. `librefang-subprocess` gains `SupervisedTransport`, a lazy, self-respawning wrapper around `SubprocessTransport`: it spawns the child on first use and re-spawns it on the first call after a crash, rate-limited by a cooldown (default 5s) so a persistently-broken command can't spawn-storm. `SidecarContextEngine` uses it, so a crashed context sidecar degrades only the in-flight call to the built-in engine and recovers on a later turn — removing the documented v1 "dead until the daemon restarts" limitation. Construction is now infallible (lazy), so the engine no longer carries an `Option<transport>`. Covered by `supervised_respawns_after_child_exits` and `supervised_cooldown_blocks_respawn_storm`.
- **memory: out-of-process extractor via the shared transport** (@houko) — a fourth consumer of `librefang-subprocess`, and the second "policy in a subprocess" extraction point. The `MemoryExtractor` trait already existed (the store holds `Arc<dyn MemoryExtractor>`), so this adds `SidecarMemoryExtractor`, which implements it over `SupervisedTransport`. Unlike the context engine's `compact` (which must reuse the daemon's configured driver for cost/streaming/cache and so stays in Rust), memory extraction is a background, non-streaming, fire-and-forget task, so a sidecar is free to do it with its own LLM key, a cheap local model, embeddings, or heuristics. The sidecar returns *simple* memory items (`{content, category?, level?, metadata?}`) and the daemon assigns each a UUID + `created_at` and stamps `source = "sidecar"`; the SQLite store and the dedup decision (`decide_action`'s heuristic) stay in Rust. Selected via `[proactive_memory.extractor_sidecar]` (`command` / `args` / `request_timeout_secs`), which takes precedence over `extraction_model`; a down sidecar degrades to "nothing memorized this turn" and auto-respawns. A commandless `[extractor_sidecar]` table is treated as a misconfiguration — it is logged and ignored rather than spawning `""` on every turn, so extraction falls back to the built-in path; and because the sidecar bypasses the LLM extractor wholesale (including any per-agent `extraction_model` override), configuring both now emits a WARN naming which one wins. Ships a dependency-free Python reference (`docs/examples/memory_extractor_sidecar.py`). Covered by `extracts_via_sidecar`, `missing_sidecar_memorizes_nothing`, `empty_sidecar_command_is_ignored_and_falls_through_to_llm`, and `configured_sidecar_command_takes_precedence_over_llm`.
- **BREAKING: rename crate `librefang-migrate` → `librefang-import`** (@houko) — the framework-import tool shared the word "migration" with `librefang-memory/src/migration.rs` (SQLite schema migrations), so grepping for "migration" returned the wrong file and forced every reader to disambiguate. The crate is renamed to `librefang-import`, which is what it actually does: import agents / memory / sessions / skills / channel configs from other frameworks (OpenClaw, OpenFang, LangChain, AutoGPT). Directory moved `crates/librefang-migrate/` → `crates/librefang-import/`; package name in `Cargo.toml` flipped; the three consumers (`librefang-api`, `librefang-cli`, `xtask`) updated to depend on `librefang-import` and reference `librefang_import::` in source. Public API (`MigrateSource`, `MigrateOptions`, `run_migration`, `report::MigrationReport`, `openclaw::*`, `openfang::*`) is preserved verbatim — only the crate name and import path change. The user-facing CLI command (`librefang migrate ...`) is unchanged. Workspace `members = [...]` and the doc references in `CLAUDE.md` / `AGENTS.md` / `CONTRIBUTING.md` / `README.md` + the 7 i18n READMEs (zh / es / fr / ko / de / ja / pl) also updated. **Operator action required**: out-of-tree consumers that declared `librefang-migrate = { path = "...", ... }` in their `Cargo.toml` must update both the dep name and any `use librefang_migrate::...` paths to `librefang_import`. Closes #5668.
- **docs(claude-md): reconcile `crates/librefang-extensions/CLAUDE.md` and `crates/librefang-channels/CLAUDE.md` with current code** (@houko) — both files had drifted away from the live source; replaced with concise pointers to the top-level `CLAUDE.md` crate index so the stale prose can't mislead readers further. Closes #5668.
- **build(justfile): make `justfile` the canonical developer entry point; delegate to `cargo xtask`** (@houko) — the `justfile` and `xtask/` overlapped on `setup` and several other recipes, with docs alternating between `just setup` and `cargo xtask setup` interchangeably. The rule is now documented at the top of `justfile`: `just` is the developer-facing surface, `xtask` is the underlying logic; recipes that duplicated logic now call `cargo xtask <subcmd>`. Closes #5668.
- **docs(architecture): strengthen the sidecar-channels rationale against AI-codegen-era critique** (@houko) — the "Why" paragraph in `docs/architecture/sidecar-channels.md` had been leading with the contributor-ergonomics argument ("writable in ~40 lines of Python against a documented protocol"), which is a real benefit but no longer load-bearing on its own now that LLM-assisted Rust authoring has narrowed the practical gap between writing Python and writing Rust. Rewrote the top-of-doc rationale to lead with the three properties that survive when the language gap closes — crash isolation (an in-process panic ends the daemon, a sidecar crash is a `waitpid` event the supervisor restarts), supply-chain confinement (each of ~28 platform SDKs is its own sealed dependency tree, not part of the kernel binary's `cargo audit` surface), and the iteration loop (subprocess restart in seconds vs. `cargo build` + daemon restart that drops every active agent session) — and demoted the contributor-ergonomics benefit to a secondary effect. Added a new "Why subprocess, not in-process Rust?" section that addresses the AI-codegen counterargument head-on, plus an inline note that the wire protocol (see `sidecar-protocol.md`) is language-agnostic — a Rust sidecar SDK against the existing `conformance/sidecar/corpus/` is unblocked future work — so Python is the current first-party SDK out of migration friction, not architectural necessity. Also corrected the stale "~46 in-process Rust adapters that predate the policy are grandfathered and frozen" claim — `crates/librefang-channels/src/channels-allowlist.txt` now permits only the `sidecar` trampoline itself, the in-process set has been fully drained. No code changes; pure documentation.

- **kernel(config-reload): non-IdP `external_auth` edits are a no-op, not a restart** — follow-up to #5652. #5652 cleared the main-CI red by gating #5619's backfill restart on `!external_auth_idp_changed(...)`, which left a non-IdP `external_auth` change (`session_ttl_secs`, `allowed_domains`, `redirect_url`, scopes, audience, `require_email_verified`) still classified as restart-required. But the OAuth layer reads every one of those live from the ArcSwap config on each request (`oauth.rs`: `config_ref()` / `config_snapshot()`), so the bare config swap already applies them on the next request — a restart was never needed, only over-conservative classification inherited from #5619's "RESTART when the live-read path can't be verified" default. The planner now records a no-op (`"external_auth config changed (effective on next request via config swap)"`) for those edits instead of telling the operator to restart; IdP-identity changes still queue `ReloadExternalAuth` (#5594) to evict the JWKS/discovery caches. Removed the now-dead `restart_if_changed(external_auth)` branch and moved the `classified_reload_fields()` entry into the hot-reload group. `test_external_auth_unrelated_field_does_not_evict_caches` extended to assert `!restart_required` + the no-op entry. `cargo test -p librefang-kernel --lib config_reload::` 40/40 (with the strengthened assertions), `cargo clippy -p librefang-kernel --lib -- -D warnings` + `cargo fmt --check` clean. Also refreshed the canonical ops table in `docs/operations/config-reload.md` — the `external_auth` row now reads **H/N** (was **R/H**), and the non-IdP prose reflects the live-read ArcSwap path instead of "no hot reapply path is wired, planner conservatively flags a restart". (@houko)

### Added

- **context engine: out-of-process `engine = "sidecar"`** — run the per-turn context **policy** (recall, window assembly, after-turn bookkeeping) in a subprocess of any language, keeping the **mechanism** it needs in Rust. The `ContextEngine` trait was already the right seam (`ingest` / `assemble` / `after_turn` / `bootstrap`); `SidecarContextEngine` implements it by delegating those async, non-LLM hooks to a child process over a newline-delimited JSON request/reply protocol, and wraps a built-in engine for the rest. LLM-bearing `compact` (its `Arc<dyn LlmDriver>` can't cross a process boundary) and the cheap synchronous hooks (`truncate_tool_result`, `should_compress`, `update_model`, metrics) stay on the inner engine. Robustness is the headline: the context engine is on the per-turn critical path, so **every bridged call falls back to the built-in engine on any failure** — spawn failure, write error, request timeout, malformed reply, or a crashed process — and a flaky sidecar never breaks a turn. Configured via `[context_engine] engine = "sidecar"` + a `[context_engine.sidecar]` block (`command`, `args`, `request_timeout_secs`); the command is trusted operator config so its environment is inherited. Ships a dependency-free Python reference (`docs/examples/context_engine_sidecar.py`) and a protocol/design doc (`docs/architecture/sidecar-context-engine.md`). Tests cover an end-to-end `assemble` round-trip through a real subprocess and spawn-failure fallback. (@houko)
- **skills: WASM skill runtime now executes in the existing `WasmSandbox`** — `SkillRuntime::Wasm` was a dead stub that returned `RuntimeNotAvailable("WASM skill runtime not yet implemented")`, even though the runtime already shipped a hardened `WasmSandbox` (capability gating, fuel/memory/wall-clock metering, denial-of-wallet host-call reservations from #3532 / #3864 / #3866). Wired the two together so a `[runtime] type = "wasm"` skill actually runs. Routing lives in `librefang-runtime` (`tool_runner/wasm_skill.rs`), not the skills loader: the sandbox and its `host_call` ABI need a `KernelHandle`, and `librefang-skills` must not depend on `librefang-runtime` (circular). The dispatcher branches on `runtime_type == Wasm` and calls `execute_wasm_skill`, which resolves the module path through the same `validate_script_path` containment guard the subprocess runtimes use (now `pub`), reads the `.wasm`/`.wat` bytes, maps the skill's declared `[requirements] capabilities` strings to `Capability` grants (fail-closed: an unrecognised string is logged and dropped, never granted), applies `requirements.timeout_secs`, and feeds the guest the same `{"tool", "input"[, "config"]}` envelope the Python/Node/Shell runtimes use so guest tool-dispatch is runtime-agnostic. Tests cover capability parsing (arg / no-arg / numeric variants and fail-closed garbage), an end-to-end echo module run through the sandbox, and path-traversal rejection. (@houko)
- **skills: `librefang-skill` Rust SDK for authoring WASM skills** — new crate at `sdk/rust/librefang-skill` (its own workspace root, like the sidecar SDKs) that hides the raw sandbox guest ABI behind one `skill!(handler)` macro plus typed host-call wrappers, so a skill author writes a `fn(Request) -> Result<Value, String>` and nothing else. The macro emits the `alloc` / `execute` exports; `memory` is exported by the `wasm32-unknown-unknown` cdylib automatically. `Request` carries the `{tool, input, config}` envelope; a handler `Err` or malformed envelope surfaces to the agent as `{"error": ...}`. Host functions are exposed under `host::` (`time_now`, `fs_read`/`fs_write`/`fs_list`, `env_read`, `kv_get`/`kv_set`, `net_fetch`, `shell_exec`, `agent_send`, `agent_spawn`) with the capability and fuel cost of each documented; `host_call` / `log` are the escape hatches. The pointer marshaling is gated to `wasm32` while the envelope/pack/dispatch logic is target-agnostic and host-unit-tested (7 tests). Verified end-to-end: a real `cargo build --target wasm32-unknown-unknown` of an SDK-based skill emits a module importing exactly `librefang.host_call` / `librefang.host_log` (no WASI) and exporting `memory` / `alloc` / `execute` — the precise surface `WasmSandbox` instantiates. Ships with an `examples/echo.rs` and a README covering the `Cargo.toml` (`crate-type = ["cdylib"]`, `panic = "abort"`), build command, and matching `skill.toml`. (@houko)
- **skills: WASM authoring is now first-class end to end (docs, CLI, example)** — supporting pieces so the WASM runtime is actually usable, not just present. (1) **Docs** (`docs/src/app/agent/skills/page.mdx` + zh): the WASM section described a `wasm32-wasi` + `_start` + stdin model that never matched the sandbox and would fail to instantiate (the sandbox provides only the `librefang` host imports, no WASI); rewritten to the real `librefang-skill` SDK flow — `wasm32-unknown-unknown`, the `skill!` macro, the `{tool, input, config}` envelope, the host-call capability/fuel table, and accurate sandbox limits. (2) **`librefang skill create`**: the scaffold hard-coded `entry = "src/main.py"` for every runtime and emitted a `// TODO` stub for anything non-Python; it now generates a correct per-runtime scaffold — for `wasm` a `cdylib` `Cargo.toml` (`librefang-skill` dep, `panic = "abort"`), a `src/lib.rs` with the `skill!` handler, the right `entry`, and the build/copy steps (the Node entry-path bug is fixed in passing). (3) **`librefang skill test`**: WASM skills now run in the real sandbox with no kernel (`execute_wasm_skill` is now `pub` and re-exported from `tool_runner`) — pure-compute tools run locally, capability-bearing host calls report an error rather than the previous "execution skipped". (4) **`publish.rs`**: a `.wasm` entry is now validated to start with the `\0asm` magic, catching an unbuilt placeholder or wrong path before publish; the manifest convention is `entry = "skill.wasm"` at the skill root (the packager excludes `target/`). (5) **Example**: new `examples/custom-skill-wasm/` (the WASM twin of `custom-skill-python`) — builds to a valid `wasm32-unknown-unknown` module verified to carry the `\0asm` magic. (6) **CI**: a path-gated `WASM Skill SDK` job fmt/clippy/tests the SDK and compiles the example for `wasm32-unknown-unknown`, so the SDK (an independent workspace the kernel lanes never build) can't silently drift from the sandbox guest ABI. (@houko)
- **memory: per-agent `[proactive_memory] extraction_model` override** — #5475. `ProactiveMemoryOverrides` (per-agent override block in `agent.toml`) previously exposed only the three boolean knobs (`enabled`, `auto_memorize`, `auto_retrieve`) that #4870/#4892 added; the LLM extraction model was global-only, so multi-provider deployments had to pick one extractor that may not be reachable from every agent's provider keys. Added `extraction_model: Option<String>` to `ProactiveMemoryOverrides` with the same `provider/model` / `provider:model` / bare-name surface as the global field, plus a `resolve_extraction_model(global)` resolver. Plumbed through a new `CatalogQuery::proactive_memory_extraction_model_for(agent_id)` role-trait method (default returns `None`, real impl in `LibreFangKernel` looks up the agent registry then resolves agent-override → kernel-global → `None`) that `LlmMemoryExtractor::extract_memories_with_agent_id` consults at request-build time to swap the wire-level model name. Driver itself is reused from the boot-time extraction driver — full per-agent driver switching (cross-provider) is a follow-up, documented inline on the new field. Integration test (`crates/librefang-runtime/tests/proactive_memory_extraction_model_override.rs`) builds a stub `KernelHandle` + recording `LlmDriver` to assert: agent override wins over the boot-time model, missing override falls back, and the `provider/` / `provider:` prefix is stripped before the API request. `Copy` removed from `ProactiveMemoryOverrides` (now owns a `String`). Docs updated in `docs/src/app/configuration/page.mdx` under `[proactive_memory]` with a new "Per-agent overrides" subsection. (@houko)

- **sdk(rust): first-party Rust sidecar adapter SDK at `sdk/rust/librefang-sidecar/`** (@houko) — pairs with `sdk/python/librefang/sidecar/`, both implementing the post-#5219 sidecar wire protocol. Same surface on both sides: `Content::*` builders for every `ChannelContent` variant; event builders (`events::ready`, `events::message` + `MessageBuilder`, `events::error`, `events::typing`, `events::qr_ready`, `events::qr_status`); typed inbound `Command` enum with `parse_command(&str)` parsing every variant (`send`, `ready_ack`, `shutdown`, `heartbeat`, `typing`, `reaction`, `interactive`, `stream_start` / `_delta` / `_end`, plus a `Command::Unknown` forward-compat envelope); `SidecarAdapter` async trait with `on_send` (required) and `on_command` / `produce` / `on_shutdown` defaults; `run_stdio(adapter)` driver that owns the stdin reader, stdout writer, `ready` re-announce handshake (bounded by `ready_max_attempts` so a pre-#5219 daemon without `ready_ack` doesn't get spammed), graceful shutdown, and routing; `run_stdio_main(schema_fn, build_fn)` one-stop `main` helper that serves the daemon's `--describe` discovery contract before constructing the adapter (builder returns `Result<A, DynError>` so adapters whose `new()` validates required env vars can fail with a structured error instead of panicking before discovery can respond); `with_backoff` helper for platform reconnect (independent of daemon-managed process lifecycle); `Schema` / `Field` for `--describe` self-description payloads. Wire-equivalent with the Python SDK and the Rust supervisor's `SidecarEvent` / `SidecarCommand` — pinned by the shared `conformance/sidecar/corpus/` via `sdk/rust/librefang-sidecar/tests/conformance.rs` (13 tests: producer-side asserts every event builder reproduces its corpus frame, consumer-side asserts every command corpus parses into the expected typed `Command`; coverage guards refuse a corpus entry with no assertion on either side, mirroring the Python suite's `EVENT_PRODUCER_SKIP` discipline). Lives as its own independent cargo workspace at `sdk/rust/librefang-sidecar/` so it doesn't pull `librefang-channels` / `librefang-kernel` into adapter authors' dep trees and doesn't contend with the kernel's shared `target/`. `cargo test` → 16 unit + 13 conformance + 3 doc-tests pass; `cargo clippy --all-targets -- -D warnings` clean; `cargo fmt --check` clean (Linux container build via `Dockerfile.rust-dev`). Includes `examples/echo.rs` — minimal `SidecarAdapter` that echoes every `send` back as a synthetic inbound `message` (no platform integration, suitable as a smoke-test against the supervisor and as a template for new adapters). `docs/architecture/sidecar-protocol.md` lifted from "two independent implementations" to "three" (Rust supervisor + Python SDK + Rust SDK), with the conformance-pair table updated. `docs/architecture/sidecar-channels.md` "polyglot by design" paragraph dropped its "unblocked future work" hedge and now lists the two SDKs side-by-side with their respective trade-offs.
- **docs(architecture): canonical references for the Rust sidecar SDK and the Rust Telegram adapter** (@houko) — the Rust sidecar SDK (#5821) and the Rust Telegram adapter (#5831) both shipped with crate-level READMEs but no entry under `docs/architecture/`, so a reader following the project's documentation convention (the same place `sidecar-channels.md` and `sidecar-protocol.md` live) had no landing page. Added `docs/architecture/rust-sidecar-sdk.md` covering the SDK's trait + type surface (`SidecarAdapter`, `Command`, `Content`, `MessageBuilder`, `Schema`), the `run_stdio_main(schema_fn, build_fn)` lazy-build pattern that lets `--describe` discovery succeed before required env vars are set, panic isolation, `with_backoff`, the conformance-corpus pinning, and the common pitfalls (spawned-future-outliving-produce, mutex-across-await, stdout-must-be-reserved, recoverable-vs-fatal-errors-in-produce). Added `docs/architecture/rust-telegram-sidecar.md` covering the adapter's five-layer architecture (`api/` → `format/` → `translator.rs` → `dispatcher.rs` → `adapter.rs`), the inbound/outbound dataflow, the Markdown → HTML → sanitised → UTF-16-chunked text-rendering pipeline (including the Private-Use-Area sentinel scheme that prevents inline-code placeholder collision and the tag-aware chunker that preserves `<a href="…">` attributes across chunk boundaries), the security model (`BotClient::redact`, allowlist-on-every-event-kind, MediaGroup-recursion-cap, strict FileData byte decode), the 429 retry shape with `MAX_RETRY_AFTER_SECS = 300` cap, and the three deliberate Python-parity divergences (`parse_command` UTF-16 vs whitespace split, MediaGroup > 10 chunked vs `ValueError`, `channel`-chat-type not treated as group). Updated `docs/architecture/sidecar-channels.md` and `docs/architecture/sidecar-protocol.md` to link to both new pages so the existing SDK-bullet readers can find them. Updated `sdk/rust/librefang-sidecar/README.md` and `sdk/rust/librefang-sidecar-telegram/README.md` to point at their respective canonical reference pages.
- **channels(rust): first-party Rust Telegram sidecar adapter at `sdk/rust/librefang-sidecar-telegram/`** (@houko) — feature-parity port of `sdk/python/librefang/sidecar/adapters/telegram.py` written against the Rust `librefang-sidecar` SDK that merged in #5821. Capabilities declared: `typing`, `reaction`, `interactive`, `thread`, `streaming`. Bot API surface covered: `getUpdates` long-poll with exponential backoff + cancel-safe yields, `sendMessage` (Markdown → Telegram-HTML via a sanitiser that allowlist-checks tags / href schemes and balances unclosed tags + 4096-UTF-16-unit chunker), `editMessageText` (with plain-text fallback on "can't parse entities"), `deleteMessage`, `sendChatAction(typing)`, `setMessageReaction` (same emoji-translation map as Python — ⏳ → 👀, ⚙️ → ⚡, ✅ → 🎉 or cleared depending on `TELEGRAM_CLEAR_DONE_REACTION`, ❌ → 👎), `sendPhoto` / `sendDocument` / `sendVoice` / `sendAudio` / `sendVideo` / `sendAnimation` / `sendSticker` / `sendLocation` / `sendMediaGroup` (2–10 photos/videos) / `sendPoll` (quiz / regular with explanation) — all via Bot API URL pass-through. Inline file bytes (`Content::FileData`) detect Ogg/Opus magic and route to `sendVoice` else `sendDocument` via multi-part upload. Inbound translation: `text`, leading `bot_command` entity becomes `Content::Command{name,args}` (with `@botname` suffix stripped), photo → `Content::Image` (resolved via `getFile`), document → `Content::File` (with voice-extension auto-routing), audio / voice / animation / video / video_note → respective rich-content variants, location → `Content::Location`, sticker → `Content::Sticker{file_id}`, contact → text label, `callback_query` → `Content::ButtonCallback` with the originating `message_id` / `chat_id` in metadata, `poll_answer` → `Content::PollAnswer{poll_id, option_ids}`. Reply context: `[Replying to <sender>: "<truncated>"]` prefixed onto text or image caption. Streaming: `StreamStart` sends a placeholder, `StreamDelta` accumulates and edits the same message debounced at 1 second, `StreamEnd` flushes. Access control via `ALLOWED_USERS` CSV of numeric user IDs or `@usernames` (case-insensitive, leading `@` optional; empty list ⇒ open). Schema served via `--describe`: `TELEGRAM_BOT_TOKEN` (secret, required), `ALLOWED_USERS` (list, advanced), `TELEGRAM_CLEAR_DONE_REACTION` (bool, advanced). HTTP via `reqwest` with `rustls-tls` (no system OpenSSL). Independent cargo workspace; no transitive into the kernel binary. `cargo test` → 19 unit tests pass (UTF-16 chunking, Markdown → HTML, sanitiser tag-allowlist + href safety + unclosed-tag balancing); `cargo clippy --all-targets -- -D warnings` clean; `cargo fmt --check` clean. Configure via a `[[sidecar_channels]]` block: `command = "/abs/path/to/target/release/librefang-sidecar-telegram"`, `[sidecar_channels.secrets] TELEGRAM_BOT_TOKEN = "<botfather-token>"`.
- **cli: restore `librefang channel` subcommand as a sidecar-only group** — #5463 deleted the entire `Commands::Channel(ChannelCommands)` group on the same "everything was broken anyway" theory that took out `ChannelsPage.tsx`. Same wrong call. Restored as four sidecar-aware subcommands driving the surviving daemon endpoints: (@houko)
  - `librefang channel list` → `GET /api/channels` → table (NAME, KIND, CONFIGURED, TOKEN, 24H MSGS). (@houko)
  - `librefang channel reload` → `POST /api/channels/reload` → hot-reload `[[sidecar_channels]]` from disk without a daemon restart. (@houko)
  - `librefang channel setup [<NAME>]` → schema-driven prompt against `GET /api/channels` (consumes the per-row `fields[]` describe schema); non-secret fields pre-fill from `f.value`, secret fields with `has_value=true` render `(set — leave blank to keep)` so an empty submission preserves the stored secret. POSTs to `POST /api/channels/sidecar/{name}/configure` and shows `restart_required` / `shadowed_secrets_warning` from the response. Without `<NAME>` it shows an interactive picker over unconfigured rows. (@houko)
  - `librefang channel rm <NAME>` → strips the matching `[[sidecar_channels]]` entry from `~/.librefang/config.toml` via `toml_edit`, then best-effort hot-reload (warns + continues if no daemon is running, so the change still applies on next start). (@houko)
  Five `clap`-parse `assert!(matches!(...))` tests added covering `list`, `setup`, `setup <name>`, `reload`, `rm <name>`. The pre-#5463 `test` / `enable` / `disable` arms are **not** restored — sidecars surface their own health via stdout logs (no in-band `/test` endpoint), and the presence of the `[[sidecar_channels]]` block is the only on/off signal (`rm` is the replacement). (@houko)

### Fixed

- **runtime(web_fetch): strip credential headers across redirect-origin boundaries** — the manual per-hop redirect loop (`send_with_pinned_redirects`, introduced to close the DNS-rebinding TOCTOU window by re-pinning every hop) re-attached the caller's headers verbatim on every hop, including cross-origin 3xx targets. reqwest's built-in redirect machinery removes `Authorization` / `Cookie` / `Proxy-Authorization` when a redirect leaves the origin, but the hand-written loop bypassed that, so an attacker-controlled public host could `302 Location: https://attacker.example/` and harvest the caller's bearer token or session cookie. The loop now mirrors reqwest's `remove_sensitive_headers`: a `crosses_origin` check (scheme / host / port) latches a `credentials_stripped` flag the first time any hop leaves the original origin, after which the sensitive headers (`authorization`, `cookie`, `cookie2`, `proxy-authorization`, `www-authenticate`, case-insensitive) are never re-attached for the rest of the chain — same-origin hops keep them, non-sensitive caller headers always pass through. The fix lands in the shared helper, so both `web_fetch` and `web_fetch_to_file` are covered. New tests: `test_cross_host_redirect_strips_credentials`, `test_same_host_redirect_preserves_credentials`, `test_redirect_to_metadata_ip_blocked_on_later_hop`, plus unit coverage for the two helpers. (@houko)

- **llm: retry transport-layer errors and make the retry count configurable (#10)** — the HTTP-API drivers' retry loop only covered server-side throttling (429 / 529 / 503); transport failures from `reqwest::send()` — connection refused, TLS record-layer alerts, read timeouts — returned immediately via `?` and never entered the loop, so a single network hiccup on the only configured provider failed the whole turn instead of being retried (the outer `FallbackChain` was the only safety net, useless in single-provider setups). Transport errors now go through the same attempt/backoff decision as a 429, via a new `backoff::transport_error_is_retryable` that classifies a `reqwest::Error` by its structured predicates (`is_timeout` / `is_connect` / `is_request`) with a substring fallback to the shared transient classifier; applied across anthropic, openai, gemini, bedrock, and vertex_ai (complete + stream). The retry cap, previously hard-coded to 3 in six hand-copied loops, is now configurable through `DriverConfig.max_retries` (serde default 3 — behaviour unchanged; 0 disables in-driver retries) and a per-provider `provider_max_retries` map in `config.toml` mirroring `provider_request_timeout_secs`, readable/writable via the API config surface and classified restart-required in the config-reload plan. (@houko)

- **Plugin seccomp allowlist was missing glibc / language-runtime start-up syscalls** (`rseq`, `set_robust_list`, `rt_sigtimedwait`, `statx`, `sched_getaffinity`, …) (#2) (@houko).
  With `seccomp-sandbox` off by default the gap was never exercised; enabling it by default exposed that every hook subprocess was `SIGSYS`-killed before reading stdin (observed as "Broken pipe"). The allowlist now covers the universal start-up set on both x86_64 and aarch64.
- **Plugin `unshare` namespace probe checked only `unshare --help`, not whether the kernel actually grants the namespace** (#2) (@houko).
  In unprivileged containers / hardened CI the binary exists but `CLONE_NEWNET` / `CLONE_NEWNS` returns EPERM, so wrapping a deny-network / deny-filesystem hook with `unshare` killed the child instead of failing open to the env-var / Landlock isolation. The probe now runs `unshare --<ns> -- true` and only wraps when the namespace can be created — important now that deny-by-default is the default posture.
- **security(approval): `trusted_senders` no longer waives approval for high-risk tools** — the trusted-sender escape hatch was all-or-nothing: any `user_id` in `trusted_senders` made `requires_approval_with_context` return `false` for *every* tool — including `shell_exec`, `file_write`, `agent_spawn`, and the rest — and made `is_tool_denied_with_context` bypass channel deny rules for all of them too, so a single trusted id silently disabled the entire approval gate. The trust bypass now applies only to tools below `High` risk (`ApprovalManager::classify_risk`): for high-risk and Critical tools the bypass is skipped, so a gate the operator already configured (via `require_approval` or a channel deny rule) stays in force even for a trusted sender. `classify_risk` is extended to classify the control-plane tools (`agent_spawn`, `agent_kill`, `config_set`, `kernel_reload`) as `Critical` alongside `shell_exec`, matching their blast radius; `RiskLevel` derives `Ord` so the gate can compare severities. Tests: `test_context_trusted_sender_bypasses_low_risk_only` (trusted + Critical/High still gated, trusted + low-risk still exempt, untrusted unaffected) and `test_context_trusted_sender_channel_deny_high_risk_still_enforced` (channel deny on a high-risk tool survives trust, deny on a low-risk tool is still bypassed). (@houko)
- **test(runtime): fix a Windows-only failure in `tool_runner::tests::shell::test_capability_enforcement_allowed`** — the test resolves a path under a nonexistent intermediate directory and asserts the error reads as file-not-found rather than permission-denied, but its accepted-phrase OR-chain only covered the Unix wordings ("No such file", "does not exist", …). On Windows a missing intermediate directory surfaces as `ERROR_PATH_NOT_FOUND` (os error 3) — "The system cannot find the path specified" — which matched none of them, so the `Test / Windows` shard went red on `main` independent of any feature change. Added `cannot find` to the OR-chain, which covers both the path form (os error 3) and the file form `ERROR_FILE_NOT_FOUND` (os error 2, "cannot find the file specified"). Test-only; no production behaviour change. (@houko)

- **memory: audit-sweep on the proactive-memory subsystem — 5 CRITICAL + 7 HIGH findings (#5839)** — closes a split-brain in `list()` / `get()` (now read from the semantic store directly; the best-effort KV `memory:*` mirror is deleted entirely so future divergence is structurally impossible); drops the silent raw-transcript fallback in `ProactiveMemory::add` when extraction yields no structured signal; honors per-row `metadata["confidence"]` on insert (the LLM extractor prompt now requests it explicitly, so `extraction_threshold` is finally live); reworks decay as `rate / boost` (cap MAX_BOOST=4) so popular memories decay slower but strictly monotonically (the previous formula clamped the post-boost product back to 1.0, freezing every `access_count >= 2` memory at confidence 1.0 forever); stamps `deleted_at` on every `forget*` path so the retention sweep can finally hard-delete user-/API-initiated soft-deletes (previously only TTL decay stamped it, so manual deletes leaked their embedding BLOB forever); gates 12 memory write endpoints (`POST /api/memory`, `PUT /api/memory/items/{id}`, `DELETE`, bulk-delete, reset, clear-level, consolidate, cleanup, export, import, decay, relations) through the namespace guard with proper `AuthDenied → 403` plumbing. Also tightens `duplicate_threshold` default 0.5 → 0.85 (mem0's near-duplicate cut-off) and the `decide_action` UPDATE thresholds 0.5/0.6 → 0.7/0.8 so a topically-related but semantically-different memory no longer silently UPDATEs over an existing row; bounds `format_context` at the new configurable `ProactiveMemoryConfig::format_context_max_chars` (default 8000 chars / ~2000 tokens) with a "[+N omitted]" footer; detaches the every-10-calls auto-consolidate to `tokio::spawn` (inside a `tracing::Instrument` span tagged `task = "auto_consolidate"`) so the agent's hot path no longer waits on an O(n²) merge plus SQLite tx; threads the new `duplicate_threshold` into the periodic `ConsolidationEngine` via a new `set_duplicate_threshold` setter on `MemorySubstrate`, wired from both kernel boot and the `UpdateProactiveMemory` hot-reload op so the global sweep and the per-agent on-demand consolidate agree; tightens LLM-extraction validation (4-char minimum content, allowlist-fuzzy-matched category with case + plural tolerance, MAX_MEMORIES_PER_EXTRACTION = 20 cap). Drive-by: collapsed a clippy::manual_option_zip in `kernel/background_lifecycle.rs:88` (failed workspace clippy gate). **Operator-facing behaviour changes worth noting on upgrade:** (1) Audit-log shape: requests authenticated by the root `api_key` now carry a synthetic Owner-equivalent `AuthenticatedApiUser{name: "root", user_id: ROOT_API_KEY_USER_ID}` instead of the previous "trusted but anonymous" `None`, so every audit row written through `record_with_context(..., api_user.user_id, ...)` (`budget.rs`, `audit.rs`, memory-route denials, etc.) stamps a `user_id` for root-key callers where it previously stamped null. Queries grouped by `user_id` will see a new "root" bucket appear after upgrade. The synthetic id is a constant UUID (`00000000-0000-0000-0000-72006f0074a0`) outside the `LIBREFANG_USER_NAMESPACE` v5 hash space, so a real `[users] name = "root"` entry in `config.toml` cannot collide with it. (2) `POST /api/memory` with content the extractor cannot parse no longer creates a row — pre-fix it silently captured the whole concatenated message transcript as a session memory with no category, which was the dominant source of `category=null` rows on the dashboard. Operators that legitimately need raw-content capture should use `ProactiveMemoryStore::add_with_level` (the trait-impl path explicitly stores raw content) instead of the `add` trait method. (@houko)

- **ci(discussion-to-issue): repair the daily backfill — `gh api --jq` does not accept `jq`'s `--arg`** — #5753. The follow-up commit on #3938 switched the backfill category filter to `gh api "repos/${REPO}/discussions" --paginate --jq --arg cat "$CAT" '.[] | select(...)'` to avoid shell-interpolating `${CAT}` into the jq source. `gh api --jq` only takes a single bare filter string — it does NOT proxy `--arg` through to jq — so `gh` parsed the call as `--jq=--arg` followed by three extra positional arguments and rejected it before any HTTP request, with `accepts 1 arg(s), received 4`. Every daily run since #3938 merged on 2026-04-28 has been red (27 consecutive failures). Fix is to drop `gh api`'s `--jq` flag and pipe the raw JSON to standalone `jq`, which does support `--arg` — preserving the original commit's shell-injection hardening intent (no `${CAT}` interpolated into the jq source) while restoring a working invocation. (@houko)

### Changed

- **ci: run the Coverage workflow on `push: main` only, not on every PR** — `coverage.yml` ran the full instrumented `cargo llvm-cov nextest --workspace` (~20-30 min) on every non-docs PR to produce a report-only LCOV artifact that gates no merge — and a *full* workspace run, where the per-PR `test-unit` / `test-ubuntu` lanes are selective, so for a typical small PR it was the single most expensive job in the pipeline. Dropped the `pull_request` trigger; coverage now runs on `push: main` (keeping the `paths-ignore` skip for non-runtime diffs) plus `workflow_dispatch` for ad-hoc runs. The main-branch series is all a report-only trend metric needs. (@houko)
- **ci: PR test lane is Linux-only — macOS / Windows run on push to main** — `test-macos` and `test-windows` in `ci.yml` previously ran on every rust PR (selective, affected crates), spending a macOS runner (10× Linux cost) and a Windows runner (2×) on each one. Both are now gated to `push: main`, where the full sharded suite already runs — a platform regression turns main red and is caught before it spreads, while PRs pay only the Linux lanes (`test-unit`, `test-ubuntu`, `live-integration-smoke`). The now-dead per-shard selective-skip gate in `test-windows` and the selective `else` branches in both jobs are removed. `main` carries no required-status-check ruleset, so the skipped jobs cannot block a PR. (@houko)
- **sidecar migration tails: openapi spec + 4 SDKs + dashboard generated.ts regenerated** — #5463 deleted the per-channel REST handlers but the openapi-drift bot never auto-regenerated, leaving `openapi.json` + the auto-generated SDKs documenting 8 dead paths (`/api/channels/{name}/configure` POST + DELETE, `/instances` + `/instances/{index}` GET / POST / PUT / DELETE, `/test`, `/whatsapp/qr/{start,status}`, `/wechat/qr/{start,status}`). Hand-removed those entries from `openapi.json`, added the missing `/api/channels/registry` declaration (live in the router since #5463 but never declared in the utoipa bundle), regenerated via `python3 scripts/codegen-sdks.py` so `sdk/python/librefang/librefang_client.py`, `sdk/go/librefang.go`, `sdk/javascript/index.js`, `sdk/rust/src/lib.rs` all lose the 10 dead methods (`testChannel` / `configureChannel` / `listChannelInstances` / `createChannelInstance` / `updateChannelInstanceHandler` / `deleteChannelInstance` / `wechatQrStart` / `wechatQrStatus` / `whatsappQrStart` / `whatsappQrStatus`) and gain `listChannelRegistry`. `crates/librefang-api/dashboard/openapi/generated.ts` regenerated from the same `openapi.json` via `npx openapi-typescript`. (@houko)
- **sidecar migration tails: 22 docs (en + zh) rewritten to match post-sidecar reality** — every `[channels.<name>]` config sample replaced with the equivalent `[[sidecar_channels]]` entry across `configuration/page.mdx`, `configuration/channels/page.mdx`, `integrations/channels/{core,enterprise,integrations}/page.mdx`, `getting-started/page.mdx`, `getting-started/examples/page.mdx`, `integrations/cli/commands/page.mdx`, `integrations/cli/examples/page.mdx`, `operations/faq/page.mdx`, and `integrations/channels/page.mdx` (en + zh mirrors). The Channel Overrides section in `configuration/channels/page.mdx` + `configuration/page.mdx` + `integrations/channels/page.mdx` gets a "legacy — removed" banner with a migration mapping table for every knob (`model` / `system_prompt` / `dm_policy` / `group_policy` / `rate_limit_*` / `output_format` / `disable_commands` / `allowed_commands` / `blocked_commands` / `threading` / `usage_footer` / `typing_mode` / `prefix_agent_name`); the deleted `[channels.<name>.overrides]` shape is kept for migration reference only. The CLI command reference page rebuilt to document the four restored `librefang channel` subcommands above. The API endpoint table in `integrations/api/page.mdx` trimmed to the four live channel endpoints (`/api/channels`, `/api/channels/registry`, `/api/channels/sidecar/{name}/configure`, `/api/channels/reload`); the 7 dead `/api/channels/{name}/…` + `/api/channels/{wechat,whatsapp}/qr/…` rows removed. (@houko)
- **sidecar migration tails: test probe + 4 stale-macro comments cleaned** — `boot_fails_on_stale_channel_output_format_key` in `config_routes_integration.rs` rotated from `[channels.google_chat]` + `webhook_port = "eighty-eighty"` (google_chat migrated to sidecar, the block is now an unknown section and the test was passing for the wrong reason) to `[[sidecar_channels]]` + `restart_initial_backoff_ms = "eighty-eighty"` so the wrong-type-coerce probe actually exercises a typed field on a still-living config section. Stale `for_each_channel_field!` / `check_channel!` / `find_channel_info!` macro references in `channel_bridge.rs:2393`, `messaging.rs:383`, and `channel_sender.rs:1` / `:17` / `:367` / `:380` rewritten to drop the dead-macro tour and just describe what the code actually does. (@houko)

- **dashboard: restore `pages/ChannelsPage.tsx` as a sidecar-only page** — #5463 deleted the page wholesale on the assumption that "every interactive path was broken anyway"; that was the wrong call. The page is back, slimmed from 1488 → 761 lines, and routes every interaction through the surviving endpoint contract: `useChannels()` reads `GET /api/channels`, `useReloadChannels()` triggers `POST /api/channels/reload`, and the schema-driven `SidecarForm` drawer (which #5463 had already added) saves through `useSaveSidecarConfig()` → `POST /api/channels/sidecar/{name}/configure`. `SidecarForm` is also wired up properly this time: non-secret fields pre-populate from `ChannelField.value` so re-configuring an existing channel shows the current values instead of a blank form, and secret fields with `has_value: true` render a `"•••• (set — leave blank to keep)"` placeholder so the operator can re-save without retyping the secret (server treats absent keys as "leave alone"). The `config_template` TOML snippet (emitted by the backend on each row) is surfaced inline under a `<details>` summary inside `SidecarForm` for ops who want to hand-edit `config.toml` instead of using the form. The `/channels` route is restored in `router.tsx`, the runtime-section nav entry + `Network` lucide import in `App.tsx`, the `n: { to: "/channels" }` vim shortcut in `useKeyboardShortcuts.ts`, the `channels` navigate-to entry in `CommandPalette.tsx`, and both `nav.channels` + a pruned 27-key `channels` namespace (down from 48 pre-#5463 — every key that drove a deleted endpoint or in-process-only feature is gone) in `locales/{en,zh}.json` — i18n-parity verified. `ChannelsPage.test.tsx` is rewritten to 386 lines / 11 cases covering the surviving flows: loading skeleton, empty-state CTA, configured-list render, search filter, picker drawer with unconfigured channels, picker → sidecar configure drawer swap, schema-driven Save dispatch → `useSaveSidecarConfig.mutate`, Reload header → `useReloadChannels.mutate`, **non-secret field pre-population from `f.value`**, **secret field "currently set" placeholder when `f.has_value`**, and **`config_template` `<details>` snippet inside the drawer**. The 13 instance / test / qr test cases from #5463-pre that drove deleted endpoints stay deleted. **No dead code is restored**: the 6 `Promise.reject` stubs (`testChannel` / `configureChannel` / `listChannelInstances` / `createChannelInstance` / `updateChannelInstance` / `deleteChannelInstance`), the `_channelEndpointGone` helper, the 4 dead QR helpers, the `ChannelInstance` / `ChannelInstancesResponse` / `QrStartResponse` / `QrStatusResponse` types, the 5 wrapper mutation hooks (`useConfigureChannel` / `useCreateChannelInstance` / `useUpdateChannelInstance` / `useDeleteChannelInstance` / `useTestChannel`), the `useChannelInstances` query + the `instances(name)` factory entry on `channelKeys`, and the typed-http-client re-exports of all of the above stay gone. The `ChannelField` + `config_template` are kept on `ChannelItem` because `SidecarForm` consumes them; the 7 in-process-era optional fields that #5463-pre's `ChannelItem` carried (`instance_count` / `difficulty` / `setup_time` / `quick_setup` / `setup_type` / `setup_steps` / `webhook_endpoint`) are dropped from the type — they were either never emitted post-migration (`webhook_endpoint`) or always degenerate values (`""` / `"sidecar"` / `0`) the page only rendered via dead conditional branches. The 1488-line page collapses to 761 by dropping the in-process-era `ChannelForm` / `InstancesDialog` / `QrLoginDialog` blocks wholesale, trimming `DetailsModal` to (status badge + `has_token` row + `Required Fields` checklist + read-only "manage via config.toml" note), removing the unreachable `!configured && config_template` branch on `DetailsModal` (the `config_template` surface lives inside `SidecarForm` instead), and dropping the qr / legacy branches in `handleCardConfigure` / `handlePick`. The four small `ChannelsPage`-referencing doc comments in `ProvidersPage.tsx`, `ProvidersPage.test.tsx`, `UsersPage.test.tsx`, and `DrawerPanel.test.tsx` are restored with corrected text — the cross-reference is valid again now that the page exists, and the references that pointed at deleted components (`ChannelsPage::ConfigDialog`) or fictional history ("tabs being retired") are rewritten. Dashboard `pnpm typecheck` clean; `npx vitest run src/pages/ChannelsPage.test.tsx` reports **11/11 passing**. (@houko)

### Removed

- **BREAKING: delete dead `/api/channels/{name}/*` REST endpoints + their CLI / TUI surface** — per-channel-instance HTTP endpoints that all 404'd unconditionally after the in-process channel registry emptied: `GET /api/channels/{name}` (get_channel), `POST /api/channels/{name}/configure` (configure_channel), `DELETE /api/channels/{name}/configure` (remove_channel), `GET /api/channels/{name}/instances` (list_channel_instances), `POST same` (create_channel_instance), `PUT /api/channels/{name}/instances/{index}` (update_channel_instance_handler), `DELETE same` (delete_channel_instance), `POST /api/channels/{name}/test` (test_channel). Every handler started with `find_channel_meta(&name)?` which returned `None` since `CHANNEL_REGISTRY` is empty, producing a fall-through 404. The 9 handlers + 5 helper functions (`build_instance_fields_json`, `resolve_secret_env_overrides`, `canonical_json`, `instance_signature`, `read_disk_channels`, `PreparedWrite` / `prepare_fields_write` / `apply_secret_writes`, `send_channel_test_message`) + 2 type definitions (`ChannelMeta`, `ChannelField`) + 1 enum (`FieldType`) + 1 empty const (`CHANNEL_REGISTRY`) + 1 lookup (`find_channel_meta`) + 5 dispatchers (`is_channel_configured`, `webhook_route_suffix`, `webhook_endpoint_url`, `inject_callback_url`, `build_field_json`, `channel_config_values`, `channel_instance_count`, `channel_instances_serialized`) are gone. `list_channels` and `channels_snapshot` simplified to skip the empty-registry loop (they now serve sidecar rows exclusively via `sidecar_channel_rows` + `sidecar_discovery_rows`). The 9 supporting helpers in `routes/skills.rs` that powered the deleted handlers also go: `upsert_channel_config`, `remove_channel_config`, `build_channel_toml_table`, `append_channel_instance`, `update_channel_instance`, `remove_channel_instance`, `CHANNEL_AOT_CONFLICT_PREFIX`, `validate_env_var` (+ `DENIED_ENV_VARS` / `ENV_VALUE_MAX_LEN` constants), plus 16 unit tests covering them. The `test_channel_status_tests` + `instance_helper_tests` modules in `routes/channels.rs` are deleted entirely. **The CLI `librefang channel {list,setup,test,enable,disable}` subcommand group is also removed** — every wizard arm targeted an in-process adapter that had since migrated to a sidecar, and the wizard's fall-through arm already errored out for every supported channel; scripts that called these will now fail with `error: unrecognized subcommand 'channel'`. **The TUI `Channels` tab is also gone** — its `F8` / `Alt-8` shortcuts are retired and fall through to the default key handler rather than being silently swallowed; the screen module (`tui/screens/channels.rs`, 720 lines) + the `ChannelListLoaded` / `ChannelTestResult` events + the `spawn_fetch_channels` / `spawn_test_channel` helpers + the `handle_channel_action` dispatcher were all retired with it. **The dashboard SPA `ChannelsPage.tsx` (~1.5k lines) + its 524-line vitest file are deleted wholesale**, along with the `testChannel` / `configureChannel` / `listChannelInstances` / `createChannelInstance` / `updateChannelInstance` / `deleteChannelInstance` helpers in `api.ts`, the corresponding mutation/query hooks in `lib/mutations/channels.ts` + `lib/queries/channels.ts`, the `instances(name)` factory entry in `channelKeys`, the typed-http-client re-exports in `lib/http/client.ts`, the `ChannelField` / `ChannelInstance` / `ChannelInstancesResponse` / `QrStartResponse` / `QrStatusResponse` types, and the four `wechatQrStart` / `wechatQrStatus` / `whatsappQrStart` / `whatsappQrStatus` helpers (the matching daemon QR routes had already been removed when WhatsApp / WeChat migrated to sidecars). The `/channels` route + its `lazyWithReload` entry are stripped from `router.tsx`, the route-type union in `App.tsx`, the runtime-section nav entry (and now-unused `Network` lucide import), the `n: { to: "/channels" }` vim-style shortcut in `useKeyboardShortcuts.ts`, the `channels` command-palette navigate-to entry (the registry-browse entry that opens `librefang.ai/channels` in a new tab stays — it points at the public catalog, not the deleted dashboard route), and both `nav.channels` + the entire 48-key `channels` namespace from `locales/en.json` and `locales/zh.json` (i18n-parity script reports parity at 3445 keys). Stale `ChannelsPage`-references in `ProvidersPage.tsx`, `ProvidersPage.test.tsx`, `UsersPage.test.tsx`, and `DrawerPanel.test.tsx` doc comments are rewritten to drop the dead cross-reference. Dashboard typecheck (`pnpm typecheck`) clean; `pnpm test` 659/659 outside two pre-existing failure clusters in `ProvidersPage.test.tsx` (introduced by #5260's `useCredentialPools` without a matching mock update) and `ModelsPage.test.tsx` (zustand `persist` middleware hitting an unmocked jsdom `storage` shim) — both predate this PR and are tracked separately. The 4 surviving HTTP endpoints — `GET /api/channels` (list), `POST /api/channels/reload`, `GET /api/channels/registry` (now also declared in the utoipa bundle so it appears in the generated OpenAPI spec / SDKs), `POST /api/channels/sidecar/{name}/configure` — cover the post-migration dashboard contract. **Operator action required**: any custom integration that hit the deleted endpoints needs to switch to `POST /api/channels/sidecar/{name}/configure` for channel configuration; channel deletion happens by removing the corresponding `[[sidecar_channels]]` entry from `config.toml` then `POST /api/channels/reload`. Anyone scripted against `librefang channel …` should move to editing `[[sidecar_channels]]` directly + the sidecar configure REST endpoint. Net: **-6446 / +233 lines** across 27 files (Rust side: `routes/channels.rs` -1684, `routes/skills.rs` -934, `cli/main.rs` -302, `cli/tui/screens/channels.rs` -720, `cli/tui/mod.rs` -84, `cli/tui/event.rs` -95, `cli/tui/screens/mod.rs` +3, `openapi.rs` -5; dashboard side: `pages/ChannelsPage.tsx` -1488, `pages/ChannelsPage.test.tsx` -524, `api.ts` -156, `lib/mutations/channels.ts` -95, `lib/queries/channels.ts` -22, `lib/queries/keys.ts` -6, `lib/http/client.ts` -8, `App.tsx` -3, `router.tsx` -8, `components/ui/CommandPalette.tsx` -1, `lib/useKeyboardShortcuts.ts` -1, `locales/en.json` -51, `locales/zh.json` -51, and small comment fixups in 4 unrelated pages). Workspace `cargo check --workspace --lib` + `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo test -p librefang-api --lib` 653/653 (was 679 — 26 tests removed alongside the deleted helpers); `cargo test -p librefang-cli --lib` clean. (@houko)

### Changed

- **chore(channels): remove dead in-process channel scaffolding** — every channel runs as a sidecar now (#5459 closed the migration with google_chat); the kernel-side scaffolding that gated in-process adapter dispatch has zero remaining consumers and is gone. Concretely: (1) **`for_each_channel_field!` macro** + its `#[macro_export]` + 3 invocation sites in `channel_sender.rs::resolve_channel_owner` / `messaging.rs::resolve_agent_home_channel` (rewritten to scan `cfg.sidecar_channels` directly) + the `for_each_channel_field_macro_uses_dictionary_order` test (witness pool empty), (2) `channel_bridge.rs::channel_overrides` body — the `find_channel_info!` macro never expanded; the function now returns `None` unconditionally (per-channel overrides only live on sidecar adapters via `agent_channel_overrides`), (3) `channel_bridge.rs::start_channel_bridge_with_config` — the `check_channel!` macro + `has_any` flag + 18 stale "X migrated to sidecar" comments collapsed; the function now early-returns when `cfg.sidecar_channels.is_empty()`, (4) `routes/channels.rs::instance_helper_tests` 4-test suite that broke at runtime after #5455 emptied `CHANNEL_REGISTRY` (their `find_channel_meta("webhook")` panicked because webhook had migrated; suite retired with the witness pool), (5) `OneOrMany<T>` type + JSON-schema + serde impls + my own `OoMTestRow` regression tests from `librefang-types/src/config/serde_helpers.rs` — no production caller left after every `OneOrMany<XConfig>` channel field went away, (6) `ChannelsConfig` body comment-wall (18 redundant "X migrated to sidecar" doc comments — the type now only carries the 3 `file_download_*` / `file_upload_max_bytes` global file-transfer fields, summarised in one doc paragraph), (7) Cargo feature aliases gone from 5 manifests: `librefang-channels::all-channels` / `librefang-api::core-channels` + `all-channels` + `all-channels-no-email` + `mini` / `librefang-cli::all-channels` + `mini` + `android` / `librefang-desktop::all-channels` + `mini` + `mobile-no-email`. `librefang-cli::default` drops `librefang-api/core-channels` (now just `["telemetry"]`); `librefang-api::default` drops `core-channels` (now just `["telemetry"]`). `.github/workflows/mobile-smoke.yml` drops the `-f mobile-no-email` flag from the Android tauri build (rustls-platform-verifier's Android workaround is no longer needed — the IMAP/SMTP code path it gated runs out-of-process). `librefang-channels/src/lib.rs` top-of-file docstring rewritten to drop the "40+ pluggable messaging integrations" claim + the 18-line `// X migrated to sidecar` migration comment wall. Net: **-628 lines** across 13 files. No behaviour change — every removed symbol was either an unused macro / dead code path or a feature alias that had collapsed to `[]`. Workspace `cargo check --workspace --lib --tests` + `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo test -p librefang-types` 817/817, `-p librefang-api` 679/679 (4 previously-broken instance_helper_tests now properly retired), `-p librefang-kernel` 1079/1079. (@houko)

### Removed

- **BREAKING: removed 6 low-value channel adapters** — `viber`, `messenger`, `nostr`, `discourse`, `mqtt`, `linkedin`. Full cascade: `src/<name>.rs` deletions; `lib.rs` mods; `Cargo.toml` features in both `librefang-channels` and `librefang-api` (plus the `k256` / `rumqttc` optional deps that nostr / mqtt pulled in); the channels-allowlist entries (so `cargo xtask channel-policy` permanently blocks reintroduction); `<Name>Config` structs + `Default` impls; `channels.<name>` fields in `ChannelsConfig` + its `Default`; the validation-hook env-var checks; `channel_bridge.rs` imports, spawn blocks, `find_channel_info!` / `check_channel!` macro arms, and default-empty test assertions; `routes/channels.rs` `ChannelMeta` entries plus the 4 match arms (`is_some` / serialize / `len` / `ser`); the `webhook_route_suffix` allowlist entries; `routes/config.rs` `ch!()` calls; kernel `channel_sender` `for_each_channel_field!` macro entries and expected-name list; CLI TUI `ChannelDef` entries; docs `[channels.X]` blocks in `configuration/page.mdx` / `configuration/channels/page.mdx` (en + zh) and the corresponding `integrations/channels/{social,integrations}/page.mdx` tutorial sections. **Operator action required**: an existing `[channels.viber]` / `[channels.messenger]` / `[channels.nostr]` / `[channels.discourse]` / `[channels.mqtt]` / `[channels.linkedin]` block is no longer recognised — remove it from `config.toml`. (@houko)
- **BREAKING: drop 12 unmaintained in-process channel adapters** — `gitter`, `keybase`, `flock`, `pumble`, `revolt`, `guilded`, `mumble`, `xmpp`, `irc`, `threema`, `twist`, `voice` are removed wholesale: adapter modules under `crates/librefang-channels/src/`, the matching `[channels.<name>]` config structs (`IrcConfig`, `XmppConfig`, `GitterConfig`, `KeybaseConfig`, `FlockConfig`, `PumbleConfig`, `RevoltConfig`, `GuildedConfig`, `MumbleConfig`, `ThreemaConfig`, `TwistConfig`, `VoiceConfig`), the per-channel `cargo` features (`channel-<name>`, incl. removal from `all-channels` / `all-channels-no-email` / `mini`), the `channel_bridge` import/check/boot blocks, the kernel `for_each_channel_field!` macro entries, the API channel registry (`ChannelMeta` / `is_channel_configured` / `channel_config_values` / `channel_instance_count` / `channel_instances_serialized`), the CLI TUI `ChannelDef` rows, the `[channels.<x>]` configuration docs (en + zh), and the dashboard i18n `fld_irc` entry. The 12 basenames are removed from `crates/librefang-channels/src/channels-allowlist.txt`, so the sidecar-first ratchet (`cargo xtask channel-policy`) now permanently rejects any attempt to reintroduce these adapters in-process. **Operator action required**: anyone setting `features = ["channel-<name>"]` (any of the 12) in `Cargo.toml` or carrying a `[channels.<name>]` block will fail to build / fail to deserialize on upgrade — pin a pre-removal release if you still need one of these in-process, or ship a sidecar adapter (see `docs/architecture/sidecar-channels.md` and the `librefang.sidecar` SDK examples under `sdk/python/`). The OpenClaw migrator (`librefang-migrate::openclaw`) now emits a warning instead of writing `[channels.irc]` when it encounters legacy IRC blocks. `voice` is removed because the standalone WebSocket STT/TTS channel was orthogonal to (and overlapped) the in-band audio transcription path that already lives in `librefang-runtime-media`; `xmpp` / `irc` users have migrated to Matrix and Discord respectively; the remaining nine adapters had effectively zero traction. (@vip)

### Changed

- **Config samples now cover all 27 sidecar channel adapters.** `librefang.toml.example` and `crates/librefang-cli/templates/init_default_config.toml` previously sampled only 4 sidecars (telegram / discord / slack / wechat) — operators running `librefang init` or copy-pasting from the example file had no in-tree guidance for the other 23 (bluesky / dingtalk / email / feishu / google_chat / gotify / line / mastodon / matrix / mattermost / nextcloud / ntfy / qq / reddit / rocketchat / signal / teams / twitch / webex / webhook / wecom / whatsapp / zulip), even though they all shipped through the #5224 → #5459 migration project. Both files now carry a commented `[[sidecar_channels]]` block per adapter, generated from each adapter's own `SCHEMA` declaration via `python3 -m librefang.sidecar.adapters.<name> --describe` so the sample can never silently drift from the env-var contract the adapter enforces at startup. Each block lists the required env vars verbatim from the SCHEMA + up to 2 commonly-tuned optionals as inline hints; the full inventory is one `--describe` away. The 4 pre-existing blocks were rewritten into the same generated format. Sanitisations applied during generation so the sample stays operator-useful: secret-type values render as `"..."` (the SCHEMA `placeholder` for secret fields is free-text dashboard prose — `"from Settings → Development → Your apps"`, `"(production should always set this)"` — that doesn't belong in a TOML literal) BUT the original hint is preserved as a trailing `# <hint>` comment so semantic guidance like `TWITCH_OAUTH_TOKEN = "..."  # oauth:abc123… (the prefix is auto-added)` isn't lost; text placeholders with `"` get backslash-escaped; descriptions split on `". "` (period + space) so "Rocket.Chat REST API" doesn't truncate to "Rocket"; the redundant "(out-of-process sidecar)" suffix is stripped from per-block headers since the section header already documents this for all 27 entries. Per-block migration warnings (`# Migrated from in-process to sidecar in #<NNN>. Old [channels.<name>] blocks are no longer recognised.`) ship for all 27 adapters so an operator upgrading a pre-migration config gets an explicit signal at the matching block rather than a parser error in isolation. Two adapters whose SCHEMA marks every env-var optional but which still require operator input to start (whatsapp — Cloud API vs Baileys gateway; wechat — pre-supplied token vs QR-login) carry a one-line "Requires EITHER / OR" hint above the env table. ntfy carries a one-line privacy note that `NTFY_SERVER_URL` defaults to the public ntfy.sh server unless overridden. The generic "Sidecar channel adapters (out-of-process, any language)" section in `librefang.toml.example` that documents the protocol-level meta-fields (`restart`, `restart_initial_backoff_ms`, `message_buffer`, `overflow`, …) now sits ABOVE the 27 per-adapter blocks so the operator reads the protocol context first, then the specific instances. The generator script ships at `scripts/gen_sidecar_samples.py` for future re-runs when a new sidecar lands or a SCHEMA rotates — invoke as `cd sdk/python && python3 ../../scripts/gen_sidecar_samples.py > /tmp/blocks.txt`, paste between the marker headers in both files, then `cargo xtask schema-check gen` to refresh `xtask/baselines/config.sha256`. Also drops the stale `[channels.whatsapp]` in-process snippet (`phone_number_id_env` / `access_token_env` fields no longer exist post-#5445). Verification: `tomllib.load()` on both files clean; `cargo xtask schema-check gen && check` baseline regenerated and matches. (@houko)
- **BREAKING: Google Chat migrated from in-process Rust adapter to sidecar-only** — the in-process `librefang-channels::google_chat` adapter (`GoogleChatAdapter`, 818 lines: service-account JWT auth via RS256 (rsa + sha2 crates), `https://oauth2.googleapis.com/token` exchange with 5-minute refresh buffer + double-checked-locking token cache, `https://chat.googleapis.com/v1/{space}/messages` outbound (text only, 4096-char chunking), axum-mounted `/channels/google_chat/webhook` route on the shared API server, `MESSAGE`-only inbound filter, `space.name` allowlist via `GoogleChatConfig.space_ids`, `space.type != "DM"` group detection, `text.starts_with('/')` → `ChannelContent::Command` routing, multi-bot `account_id` metadata injection, `ALLOWED_TOKEN_URI_PREFIXES` SSRF allowlist on the `token_uri` field of the service-account JSON) is deleted along with the `[channels.google_chat]` config schema (`GoogleChatConfig` + `Default` impl), the `channel-google-chat` cargo feature in both `librefang-channels` and `librefang-api` (which collapses `all-channels` / `all-channels-no-email` / `mini` / `core-channels` to empty arrays — `webhook` migrated in #5455 was the only remaining sibling), the `rsa` optional dep in `librefang-channels/Cargo.toml` (no other in-process consumer left), the dashboard `ChannelMeta` descriptor + 4 match arms (`is_some` / serialize / `len` / `ser`) + the `webhook_route_suffix` `google_chat` entry, the kernel `channel_sender` `for_each_channel_field!` entry + `EXPECTED` name-list (now both empty — no in-process channels left), the config-validation env-var hook (`service_account_env` + `service_account_key_path` presence check), the `channel_bridge` `GoogleChatAdapter` import + builder block + `check_channel!` invocation + `find_channel_info!` match arm, and the route-handler 412/200 test witness pair (`missing_required_env_returns_412` + `credentials_present_no_target_returns_200`) — retired because the in-process witness pool of channels with a `required: true` secret env var is now empty (rotation history: matrix → whatsapp → google_chat). The `routes/channels.rs::instance_helper_tests` 4-test suite is also retired (witness pool empty — `webhook` is also a sidecar after #5455). `google_chat` is removed from `crates/librefang-channels/src/channels-allowlist.txt`, so `cargo xtask channel-policy` permanently rejects any attempt to reintroduce an in-process Google Chat adapter. `librefang-migrate`'s OpenClaw importer (both the typed `migrate_channels_from_openclaw` path and the loose-JSON `migrate_channels_from_json` path) records the legacy `[channels.google_chat]` block as a SkippedItem (same shape as Matrix / Feishu / Teams / WhatsApp / Webhook removals); the four channel-table helpers (`map_dm_policy`, `map_group_policy`, `build_channel_table`, `allow_from_to_toml_array`) that the in-process import paths used are all deleted with the Google Chat code path that was their last consumer. The CLI `librefang init <channel>` wizard match collapses to a fall-through unknown-channel hint (`maybe_write_channel_config` / `notify_daemon_restart` also removed). Behaviour is preserved by the new reference sidecar `librefang.sidecar.adapters.google_chat` (ships in `librefang-sdk`; source at `sdk/python/librefang/sidecar/adapters/google_chat.py`, stdlib-only — `urllib.request` for REST + `BaseHTTPRequestHandler` over `HTTPServer` for inbound, no third-party deps): same service-account JWT auth path with an in-module PKCS#8 PEM parser + RSA modular-exponentiation signer + PKCS#1 v1.5 SHA-256 padding (`_parse_pkcs8_rsa_private_key`, `_pkcs1_sign_sha256`, `_sign_rs256_jwt` — covered by `test_jwt_signing_round_trip_against_test_pem`), same `ALLOWED_TOKEN_URI_PREFIXES` SSRF allowlist on `token_uri`, same `https://oauth2.googleapis.com/token` exchange with 5-minute refresh buffer cached in a `_TokenCache` (`threading.Lock`-backed, mirrors the Rust `Arc<RwLock<...>>`), same pre-supplied `access_token` fallback path (cached as `DEFAULT_TOKEN_LIFETIME_SECS`, no auto-refresh), same `https://chat.googleapis.com/v1/{space}/messages` outbound with 4096-char UTF-8-safe chunking, same `MESSAGE`-only / space-allowlist / DM-vs-group / `/cmd`-routing inbound semantics, same multi-bot `account_id` metadata injection (#5003). The in-process adapter mounted onto LibreFang's shared axum server at `/channels/google_chat/webhook`; the sidecar runs its own webhook server (configurable `GOOGLE_CHAT_WEBHOOK_PORT`, default `8090`) so the public URL operators register in the Google Cloud Console Bot configuration changes from `https://<host>/channels/google_chat/webhook` to `https://<host>:<GOOGLE_CHAT_WEBHOOK_PORT>/webhook`. **Three improvements over the Rust adapter**: (1) **401 clears the cached token** — the Rust adapter cached the OAuth2 access token until its TTL expired and surfaced a generic `Google Chat API error 401` on stale-token failures, forcing the operator to wait out the cache. The sidecar's `_send_text` clears `_token_cache` on 401 so the next send re-runs the JWT auth path; (2) **`WEBHOOK_MAX_BODY_BYTES = 1 MiB` cap on the inbound webhook body** — Rust inherited axum's default `DefaultBodyLimit` (2 MiB); the sidecar's `HTTPServer` enforces 1 MiB at the handler before allocating the body buffer, rejecting a malicious `Content-Length: 10G` with 413 before any read; (3) **start-time PEM validation** — the Rust adapter deferred RSA private-key parsing until the first `_get_access_token` call (lazy), so an invalid PEM surfaced as a runtime error on the first outbound. The sidecar parses + caches the `(n, d)` tuple in `__init__`, raising `RuntimeError` at boot so misconfigured deployments fail-fast. **Operator action required**: an existing `[channels.google_chat]` block is no longer recognised — re-declare as `[[sidecar_channels]]` running `python3 -m librefang.sidecar.adapters.google_chat` with `GOOGLE_CHAT_SERVICE_ACCOUNT_JSON` (the **full JSON blob**, not a path — in `~/.librefang/secrets.env`), `GOOGLE_CHAT_WEBHOOK_PORT` (default `8090`, in `[sidecar_channels.env]`). Optional knobs: `GOOGLE_CHAT_SPACE_IDS` (CSV, e.g. `spaces/AAAA,spaces/BBBB`; empty = all spaces), `GOOGLE_CHAT_ACCOUNT_ID` (multi-bot routing — surfaces as `google_chat:<id>` in `channel_defaults`), `GOOGLE_CHAT_API_BASE` (testing override). The Google Cloud Console Bot configuration messaging endpoint must be repointed to the sidecar URL. `ChannelType::Custom("google_chat")` is preserved via `channel_type = "google_chat"` on the sidecar entry so existing routing / `channel_role_mapping` keys that reference `google_chat` continue to resolve. After this PR there are **zero in-process channels** in the workspace — every channel runs as a sidecar. Verification: `cd sdk/python && pytest tests/test_google_chat_adapter.py` — **32 passed** (env enforcement (missing JSON / bad JSON / no auth path), pre-supplied access_token construction, JWT construction parses PEM into `(n, d)` tuple, CSV space-id parsing, account_id propagation, bad webhook port raises, JWT `token_uri` SSRF allowlist (rejects `attacker.example`), inbound parsing (plain text / slash command / DM `is_group=false` / threaded / non-MESSAGE / empty text / space allowlist filter / empty filter = all), UTF-8-safe `_split_message` (short / empty / byte-boundary / multibyte), outbound `_send_text` (endpoint / auth header / chunking / 401 clears cache), `on_send` dispatch (real `Send` dataclass: `channel_id` happy-path / `cmd.user.platform_id` fallback / empty channel drops / non-`spaces/` channel drops / empty text drops), JWT round-trip against a 2048-bit test key (header / claims / signature byte length), schema sanity (required `secret`-type service-account field, `advanced=true` on account_id)). (@houko)
- **BREAKING: Webhook migrated from in-process Rust adapter to sidecar-only** — the in-process `librefang-channels::webhook` adapter (`WebhookAdapter`, 772 lines: HMAC-SHA256 signature verification on `X-Webhook-Signature: sha256=<hex>` with constant-time compare, optional `X-Webhook-Timestamp` replay-window check (±5 minutes), JSON inbound parsing for `{sender_id, sender_name, message, thread_id, is_group, metadata}`, slash-command routing on messages starting with `/`, outbound `POST {callback_url}` signed the same way with 65535-char chunking + 100ms inter-chunk delay, **SSRF guard** via `http_client::validate_url_for_fetch` (rejects private/loopback/link-local/multicast/cloud-metadata callback URLs at adapter construction), `deliver_only` mode that tags inbound with `__deliver_only__` + `__deliver_target__` metadata for the kernel's `bridge.rs:2845-2851` LLM-short-circuit routing) is deleted along with the `[channels.webhook]` config schema (`WebhookConfig`), the `channel-webhook` cargo feature in both `librefang-channels` and `librefang-api` (incl. its membership in `core-channels` / `all-channels` / `all-channels-no-email` / `mini` — `core-channels` collapses to `channel-google-chat` alone now), the dashboard `ChannelMeta` descriptor + 4 match arms (`is_some` / serialize / `len` / `ser`) + the `"webhook"` arm of `webhook_route_suffix`, the kernel `channel_sender` `for_each_channel_field!` entry + `EXPECTED` name-list, the config-validation hook (env-var presence + `deliver_only` needs `deliver`), the `channel_bridge` `WebhookAdapter` import + builder block + `check_channel!` invocation + `find_channel_info!` match arm, the route-handler 412/200 test witness (rotated `webhook` → `google_chat`), and the demo-only Python adapter that previously lived at `sdk/python/librefang/sidecar/adapters/webhook.py` (132 lines using a hand-rolled JSON-RPC protocol — replaced by the standard `SidecarAdapter`-framework port). `webhook` is removed from `crates/librefang-channels/src/channels-allowlist.txt`, so `cargo xtask channel-policy` permanently rejects any attempt to reintroduce an in-process webhook adapter. The `librefang-types` `mod.rs` test rotations move the OneOrMany + `deny_unknown_fields` (#5130) witnesses from WebhookConfig to GoogleChatConfig (the LAST remaining in-process channel) and McpServerConfigEntry respectively; the `librefang-api` `config_routes_integration` boot-fail test rotates `[channels.webhook] listen_port = "eighty-eighty"` → `[channels.google_chat] webhook_port = "eighty-eighty"` for the wrong-type-coerce probe. Behaviour is preserved by the new reference sidecar `librefang.sidecar.adapters.webhook` (ships in `librefang-sdk`; source at `sdk/python/librefang/sidecar/adapters/webhook.py`, stdlib-only — `BaseHTTPRequestHandler` over `ThreadingTCPServer` for inbound, `urllib.request` for outbound, no third-party deps): same HMAC-SHA256 verification with constant-time compare, same ±5-minute timestamp skew window for replay protection, same sig-only fallback (with a per-request WARN log) when the timestamp header is absent — backwards-compatible with clients that never sent it; PRESENT-but-malformed timestamp returns 400 to distinguish "client bug" from "attacker probing the bypass" (matches webhook.rs:295-310). Auth failures collapse to a single `Forbidden` response so an attacker can't probe which check failed. Same JSON inbound parse (`message` / `sender_id` / `sender_name` / `thread_id` / `is_group` / `metadata`, fallbacks `"webhook-user"` / `"Webhook User"` for missing identity fields, empty `message` drops with 200 OK so caller doesn't retry). Same slash-command routing (`/cmd args` → `Command`). Same outbound shape (`{sender_id: "librefang", sender_name: "LibreFang", recipient_id, recipient_name, message, timestamp}`), 65535-char chunking, 100 ms inter-chunk delay. Same SSRF guard — pure-Python port of `http_client::validate_url_for_fetch` covering IPv4 (`0/8`, `10/8`, `127/8`, `100.64/10`, `169.254/16`, `172.16/12`, `192.168/16`, `192.0.0/24`, multicast `224-239`, reserved `240-255`), IPv6 (loopback, link-local, site-local, multicast, unique-local, IPv4-mapped via `::ffff:` and NAT64 via `64:ff9b::`), and reserved hostnames (`localhost`, `localhost.`, `kubernetes.default.svc.cluster.local`). Same `deliver_only` metadata propagation (`__deliver_only__` + `__deliver_target__`) — kernel bridge routing is unchanged. Same multi-bot `account_id` injection (#5003). **Operator action required**: an existing `[channels.webhook]` block is no longer recognised — re-declare as `[[sidecar_channels]]` running `python3 -m librefang.sidecar.adapters.webhook` with `WEBHOOK_LISTEN_PORT` (in `[sidecar_channels.env]`) and `WEBHOOK_SECRET` (in `~/.librefang/secrets.env`). Optional knobs (all on the `[sidecar_channels.env]` block): `WEBHOOK_LISTEN_PATH` (default `/webhook`), `WEBHOOK_CALLBACK_URL` (optional outbound delivery, SSRF-guarded at startup AND on every send), `WEBHOOK_DELIVER_ONLY = "1"` + `WEBHOOK_DELIVER = "telegram"` for pass-through, `WEBHOOK_ACCOUNT_ID`. **Four improvements over the Rust adapter**: (1) **inbound dedupe on `platform_message_id`** — Rust assigned a fresh `wh-<timestamp_ms>` ID on each emit and never deduped, so a misbehaving upstream that delivered twice would double-emit. Sidecar threads either the inbound's own `metadata.message_id` (when present) or a synthesised `wh-<ms>-<body_hash[:8]>` ID through a bounded `SeenSet` (10000 cap / 5000 evict); the 8-char body-hash suffix prevents collisions between simultaneous deliveries at the same millisecond, which the Rust millisecond-only ID flattened together; (2) **429 `Retry-After` honoured** on outbound POSTs — Rust raised on first non-2xx (webhook.rs:476-480). Sidecar parses `Retry-After` (default 30 s, floor 1 s, cap 60 s), sleeps, retries, then logs-and-continues on the second 429 so a single throttled chunk doesn't drop the rest of a multi-chunk reply; (3) **explicit 30 s timeout** on every outbound POST — Rust relied on `reqwest`'s default; (4) **per-send SSRF re-check** — the Rust adapter validated the `callback_url` once at adapter construction; the sidecar re-checks before every POST so a config-reload that swapped the URL to a private host doesn't leak the signing secret to localhost. The `deliver_only` validation is also tighter: Rust warn-and-continued when `deliver_only=true` but `deliver` was unset (silent inbound drop at runtime); the sidecar fail-closes at startup with `SystemExit(2)`. Verification: `cd sdk/python && pytest tests/test_webhook_adapter.py` — **74 passed** (env handling incl. all SSRF-rejection paths (loopback IPv4 / RFC 1918 / link-local / CGN / multicast / IPv6 loopback + link-local + IPv4-mapped / reserved hostnames / non-http scheme), signature verify (valid / wrong key / wrong body / missing / empty / wrong prefix / short-length-mismatch), `parse_webhook_body` happy path + missing-message drop + default-sender fallback + non-string-field defaults + non-dict, `_verify_request` (valid sig with/without timestamp / missing sig 403 / empty sig 403 / malformed-timestamp 400 / stale-timestamp 403 / future-timestamp 403 / wrong-sig 403 / ±300 s skew boundary), `_handle_webhook_body` end-to-end (happy / slash-command with-and-without args / msg-ID dedupe / account_id injection / deliver_only metadata / is_group propagation / invalid sig 403 / malformed JSON 400 / empty message 200 / 10-minute-old replay 403), outbound (basic / chunking / no-callback log+drop / signature round-trip verifies / 429 retry / non-2xx raises / empty text drops), `on_send` dispatch (text / user.platform_id fallback / empty recipient drops / unsupported placeholder), schema + capabilities). (@houko)
- **BREAKING: WhatsApp migrated from in-process Rust adapter to sidecar-only (dual-mode preserved)** — the in-process `librefang-channels::whatsapp` adapter (`WhatsAppAdapter`, 918 lines: Cloud API outbound to `graph.facebook.com/v17.0/<phone_id>/messages` for text / audio / image / document / location, OpenSSL-backed Bearer auth, multipart media upload for raw voice bytes (`api_upload_media`), Web/QR gateway outbound proxy (`gateway_send_message` / `gateway_send_voice` to `{gateway_url}/message/{send,send-voice}`), DM / group policy filter (`should_handle_message` with `DmPolicy::{Respond,AllowedOnly,Ignore}` × `GroupPolicy::{All,MentionOnly,CommandsOnly,Ignore}`), `is_bot_mentioned` substring match against `bot_phone` (with / without `@` + `+` strip) and `bot_name`, sender allowlist by exact phone match, multi-bot `account_id` metadata) is deleted along with the `[channels.whatsapp]` config schema (`WhatsAppConfig` + `deny_unknown_fields`), the `channel-whatsapp` cargo feature in both `librefang-channels` and `librefang-api` (incl. its membership in `all-channels` / `all-channels-no-email` / `mini`), the dashboard `ChannelMeta` descriptor + 4 match arms (`is_some` / serialize / `len` / `ser`), the dashboard's custom `POST /channels/whatsapp/qr/start` + `GET /channels/whatsapp/qr/status` route pair (~210 lines incl. the `gateway_http_post` / `gateway_http_get` raw-TCP helper functions — the Baileys gateway when in use now exposes its own QR endpoint and the dashboard proxies it directly), the kernel `whatsapp_gateway.rs` module (`include_str!` of `packages/whatsapp-gateway/{index.js,package.json,scripts/postinstall.js}`, gateway-dir extraction, `npm install` orchestration, `node index.js` child-process supervisor with 5s / 10s / 20s restart backoff, `whatsapp_gateway_pid` field on `LibreFangKernel`, `whatsapp_pid()` accessor, shutdown SIGTERM / taskkill cleanup, and the `background_lifecycle` spawn block that auto-started it whenever `[channels.whatsapp]` was non-empty), the kernel `channel_sender` `for_each_channel_field!` entry + `EXPECTED` name-list, the config-validation env-var hook, the `channel_bridge` `WhatsAppAdapter` import + builder block + `check_channel!` invocation + `find_channel_info!` match arm, the CLI-TUI `ChannelDef`, the CLI `librefang channel setup whatsapp` wizard arm + status-table row, and the route-handler 412/200 test witness (rotated `whatsapp` → `google_chat`, which still ships in-process with a `required: true` secret env var). `whatsapp` is removed from `crates/librefang-channels/src/channels-allowlist.txt`, so `cargo xtask channel-policy` permanently rejects any attempt to reintroduce an in-process WhatsApp adapter. `librefang-migrate`'s OpenClaw importer (both the YAML + the JSON5 paths) now records the legacy `[channels.whatsapp]` block as a SkippedItem (same shape as IRC / Mattermost / Signal / Matrix / Feishu / Email / WeCom / WeChat / DingTalk / Teams removals); the `[channels.whatsapp]` round-trip channel-items count drops from 2 → 1. The `librefang-migrate` `openfang.rs` drift tests rotate `[channels.whatsapp]` → `[[mcp_servers]]` (the remaining `deny_unknown_fields` witness after WhatsAppConfig went away) and the `config_routes_integration` boot-fail test rotates to `[channels.webhook]` for the wrong-type-coerce probe. Behaviour is preserved by the new reference sidecar `librefang.sidecar.adapters.whatsapp` (ships in `librefang-sdk`; source at `sdk/python/librefang/sidecar/adapters/whatsapp.py`, stdlib-only — `urllib.request` for REST + `BaseHTTPRequestHandler` over `ThreadingTCPServer` for inbound, no third-party deps): same Cloud API outbound for text / audio (URL link) / image (link + caption) / document (link + filename) / location, same gateway outbound proxy with graceful-degradation per-content-type to text in Web mode (voice URL → `(Voice message: <url>)`, image without caption → `(Image — not supported in Web mode)`, file → `(File: <name> — not supported in Web mode)`), same 4096-char chunking, same DM × group policy filter logic with TOML-string-compatible policy names (`respond` / `allowed_only` / `ignore` × `all` / `mention_only` / `commands_only` / `ignore`), same bot-mention detection (phone with / without `@` + `+` strip, name substring, all case-insensitive), same allowlist semantics, same multi-bot `account_id` metadata injection (#5003). The shared `/channels/whatsapp/qr/*` routes are gone — the Baileys gateway (when still in use for Web/QR mode) is now operated as a separate `[[sidecar_channels]]` entry (or external service) and the kernel no longer embeds / auto-spawns the Node.js process. **Four improvements over the Rust adapter**: (1) **real Cloud API inbound webhook** — `WhatsAppAdapter::start()` at whatsapp.rs:454-483 was a `TODO` stub that logged "webhook ready" and never actually parsed incoming activities; operators wanting Cloud API inbound had to wire their own webhook → `/api/agents/{id}/message` forwarder. The sidecar implements the real handler: `GET {path}` returns `hub.challenge` for Meta's subscription confirmation when `hub.mode == "subscribe"` and `hub.verify_token` matches `WHATSAPP_VERIFY_TOKEN`; `POST {path}` verifies `X-Hub-Signature-256` against `HMAC-SHA256(WHATSAPP_APP_SECRET, raw_body)` (constant-time compare), then parses `entry[].changes[].value.messages[]` and emits text events through the standard sidecar protocol; (2) **inbound dedupe on `message.id`** — Meta retries on non-200, bounded `SeenSet` (10000 / evict 5000) keeps redeliveries from double-emitting; (3) **429 `Retry-After` honoured** on every outbound POST — Rust warned-and-failed on the first non-2xx (whatsapp.rs:373-377); (4) **explicit 30 s timeouts** on every REST call — Rust relied on `reqwest`'s defaults. **Operator action required**: an existing `[channels.whatsapp]` block is no longer recognised — re-declare as `[[sidecar_channels]]` running `python3 -m librefang.sidecar.adapters.whatsapp`. For **Cloud API mode**: `WHATSAPP_PHONE_NUMBER_ID` (in `[sidecar_channels.env]`) + `WHATSAPP_ACCESS_TOKEN` / `WHATSAPP_VERIFY_TOKEN` / `WHATSAPP_APP_SECRET` (in `~/.librefang/secrets.env`), optional `WHATSAPP_WEBHOOK_PORT` (default `8460`) / `WHATSAPP_WEBHOOK_PATH` (default `/webhook`). For **Web/QR mode**: `WHATSAPP_GATEWAY_URL = "http://localhost:3009"` (in `[sidecar_channels.env]`) and the Baileys gateway (`npx @librefang/whatsapp-gateway`) must be run separately — the kernel no longer auto-spawns it. Optional knobs in both modes: `WHATSAPP_ALLOWED_USERS` (csv), `WHATSAPP_ACCOUNT_ID`, `WHATSAPP_BOT_PHONE` / `WHATSAPP_BOT_NAME` (for `mention_only` group policy), `WHATSAPP_DM_POLICY` / `WHATSAPP_GROUP_POLICY`. `ChannelType::WhatsApp` is preserved via `channel_type = "whatsapp"` on the sidecar entry so existing routing / `channel_role_mapping` keys continue to resolve. Verification: `cd sdk/python && pytest tests/test_whatsapp_adapter.py` — **76 passed** (env handling + dual-mode construction (Cloud / gateway / missing-creds-fails-closed), CSV / path-normalize / lowercase-policy parsing, `X-Hub-Signature-256` verify (valid / wrong key / wrong body / missing / empty / wrong prefix / non-hex / empty digest), `is_bot_mentioned` (phone / `@`-prefix / name case-insensitive / no-match / empty-text), `should_handle_message` (DM × `respond` / `allowed_only` reject+accept / `allowed_only` empty-allowlist / `ignore`; group × `all` / `mention_only` reject+accept / `commands_only` reject+accept-with-leading-spaces / `ignore` / unknown policy fails-closed), `parse_cloud_api_message` (text / non-text dropped / empty text / missing field / phone fallback / multiple / account_id injection / non-dict / missing entry), `_handle_get_verify` (subscribe match / wrong token / wrong mode all-reject), `_handle_post_webhook` (signature disabled / valid / invalid 401 / malformed 400 / dedupe / DM policy applied), Cloud API outbound (basic text / chunking / 429 retry / non-2xx raises / empty drops / audio-link / image with-and-without caption / file / location), gateway outbound (text basic / chunks / non-2xx raises), `on_send` dispatch (cloud text / image / voice / file / location; gateway text / voice degrades / image uses caption / no caption placeholder; empty recipient drops; user.platform_id fallback), schema + capabilities). (@houko)
- **BREAKING: Microsoft Teams migrated from in-process Rust adapter to sidecar-only** — the in-process `librefang-channels::teams` adapter (`TeamsAdapter`, 948 lines: Bot Framework v3 REST + axum-mounted `/channels/teams/webhook` route on the shared API server, OAuth2 client-credentials flow with 5-minute refresh buffer, `Authorization: HMAC <base64>` HMAC-SHA256 verification on every inbound, Azure AD tenant allowlist via `channelData.tenant.id`, self-skip by `from.id == app_id`, `/cmd args` slash-command routing, group detection via `conversation.isGroup`) is deleted along with the `[channels.teams]` config schema (`TeamsConfig`), the `channel-teams` cargo feature in both `librefang-channels` and `librefang-api` (incl. its membership in `all-channels` / `all-channels-no-email` / `mini`), the dashboard `ChannelMeta` descriptor + 4 match arms (`is_some` / serialize / `len` / `ser`), the `webhook_route_suffix` `teams` entry, the kernel `channel_sender` `for_each_channel_field!` entry + `EXPECTED` name-list, the config-validation env-var hook, the `channel_bridge` `TeamsAdapter` import + builder block + `check_channel!` invocation + `find_channel_info!` match arm, and the route-handler 412/200 test witness (`Path("teams")` → `Path("whatsapp")`). `teams` is removed from `crates/librefang-channels/src/channels-allowlist.txt`, so `cargo xtask channel-policy` permanently rejects any attempt to reintroduce an in-process teams adapter. `librefang-migrate`'s OpenClaw importer (both YAML + JSON5 paths) now records the legacy `[channels.teams]` block as a SkippedItem (same shape as IRC / Mattermost / Signal / Matrix / Feishu / Email / WeCom / WeChat / DingTalk removals); the `[channels.teams]` round-trip channel-items count drops from 3 → 2. Behaviour is preserved by the new reference sidecar `librefang.sidecar.adapters.teams` (ships in `librefang-sdk`; source at `sdk/python/librefang/sidecar/adapters/teams.py`, stdlib-only — `urllib.request` for REST + `BaseHTTPRequestHandler` over `ThreadingTCPServer` for inbound, no third-party deps): same Bot Framework v3 inbound activity parsing, same HMAC-SHA256 verification of `Authorization: HMAC <base64>` over the raw request body using the base64-decoded `TEAMS_SECURITY_TOKEN` (empty/non-base64 token → WARN + disabled), same OAuth2 client-credentials token cache with 5-minute refresh buffer (`POST {oauth_url}` with `grant_type=client_credentials` + `scope=https://api.botframework.com/.default`), same outbound `POST {service_url}/v3/conversations/{id}/activities` with `{type: "message", text: <chunk>}`, same 4096-char chunking, same `from.id == app_id` self-skip, same `channelData.tenant.id` allowlist (empty = all), same `/cmd args` Command routing, same `conversation.isGroup` group detection, same multi-bot `account_id` metadata injection (#5003), same typing indicator via `{type: "typing"}` (declared `capabilities = ["typing"]` so the daemon routes `TypingCmd`). The in-process adapter mounted onto LibreFang's shared axum server at `/channels/teams/webhook`; the sidecar runs its own webhook server (configurable `TEAMS_WEBHOOK_PORT` / `TEAMS_WEBHOOK_PATH`, defaults `8459` / `/webhook`) so the public URL operators register in the Azure Bot Channel configuration changes from `https://<host>/channels/teams/webhook` to `https://<host>:<TEAMS_WEBHOOK_PORT><TEAMS_WEBHOOK_PATH>`. **Four improvements over the Rust adapter**: (1) **per-conversation `service_url` reuse** — the Rust adapter stored the inbound `serviceUrl` in `metadata.serviceUrl` but never used it on outbound, so for tenant- / region-routed deployments where Microsoft assigns different service URLs per conversation, every reply silently landed on `DEFAULT_SERVICE_URL`. The sidecar caches the most recent `serviceUrl` per `conversation_id` and uses it on outbound; (2) **inbound dedupe on Activity ID** — Rust emitted every activity unconditionally and Bot Framework retries on non-2xx / timeout could double-emit. Bounded `SeenSet` (10000 cap / 5000 evict); (3) **429 `Retry-After` honoured** on every outbound POST — Rust warned-and-dropped (teams.rs:254-258); (4) **explicit 30 s timeout on every REST call** — Rust relied on `reqwest`'s default. **Operator action required**: an existing `[channels.teams]` block is no longer recognised — re-declare as `[[sidecar_channels]]` running `python3 -m librefang.sidecar.adapters.teams` with `TEAMS_APP_ID` + `TEAMS_WEBHOOK_PORT` (in `[sidecar_channels.env]`) and `TEAMS_APP_PASSWORD` + `TEAMS_SECURITY_TOKEN` (in `~/.librefang/secrets.env`). Optional knobs: `TEAMS_WEBHOOK_PATH` (default `/webhook`), `TEAMS_ALLOWED_TENANTS` (csv), `TEAMS_ACCOUNT_ID`. The Azure Bot Channel messaging endpoint must be repointed to the sidecar URL. `ChannelType::Teams` is preserved via `channel_type = "teams"` on the sidecar entry so existing routing / `channel_role_mapping` keys continue to resolve. Verification: `cd sdk/python && pytest tests/test_teams_adapter.py` — **64 passed** (env enforcement, HMAC-SHA256 signature verify (valid / wrong key / wrong body / missing / empty / wrong prefix / non-base64 / empty base64), `parse_teams_activity` (basic / self-skip / non-message / missing from / empty text / tenant accept+reject / tenant missing with allowlist / group / `/cmd` routing with-args + no-args / account_id injection / non-dict), `_handle_webhook_body` end-to-end (emit / bad sig 401 / missing auth 400 / verification-disabled accept / malformed JSON 400 / Activity-ID dedupe / per-conversation service_url cache / fallback to default), `_send_text` (basic / cached service_url / chunking / empty drop / empty conversation drop / 429 retry / 5xx warn+continue), OAuth token cache (caches across calls / non-2xx raises / missing access_token raises / default TTL on missing expires_in), `on_send` (basic / user.platform_id fallback / empty conversation drop / unsupported placeholder / empty text drop), typing (basic / swallows errors / empty conv skipped / on_command routes TypingCmd / on_command empty channel drops), schema + capabilities). (@houko)
- **BREAKING: DingTalk migrated from in-process Rust adapter to sidecar-only, Stream mode only — Webhook mode is removed** — the in-process `librefang-channels::dingtalk` adapter (`DingTalkAdapter`, 1276 lines: dual `DingTalkReceiveMode` (`Stream` default + `Webhook` legacy); **Stream mode**: dynamic gateway registration via `POST https://api.dingtalk.com/v1.0/gateway/connections/open` returning per-connection `{endpoint, ticket}`, WebSocket to `wss://endpoint?ticket=<url-encoded-ticket>` with strict `{code, headers, message, data}` ACK schema for every CALLBACK frame (no ACK → DingTalk redelivers), SYSTEM ping/pong heartbeat on application-level `headers.topic == "ping"` frames, per-message `sessionWebhook` URL extraction for replies; **Webhook mode**: HTTP POST callback server with HMAC-SHA256 signature verification computed as `HMAC_SHA256(secret, timestamp + "\n" + secret + body_bytes)` and ±5 minute replay window, outbound via `POST https://oapi.dingtalk.com/robot/send?access_token=&timestamp=&sign=` with `HMAC_SHA256(secret, timestamp + "\n" + secret)` (body excluded, legacy quirk)) is deleted along with the `[channels.dingtalk]` config schema (`DingTalkConfig` + `DingTalkReceiveMode` enum), the `channel-dingtalk` cargo feature in both `librefang-channels` and `librefang-api` (incl. its membership in `all-channels` / `all-channels-no-email`), the dashboard `ChannelMeta` descriptor + 4 match arms (`is_some` / serialize / `len` / `ser`) + the `webhook_route_suffix` `dingtalk` entry, the CLI-TUI `ChannelDef`, the kernel `channel_sender` `for_each_channel_field!` entry + `EXPECTED` name-list, the config-validation env-var hook, and the `channel_bridge` adapter initialization (both Stream + Webhook arms) + `check_channel!` invocation + `find_channel_info!` match arm. `dingtalk` is removed from `crates/librefang-channels/src/channels-allowlist.txt`, so `cargo xtask channel-policy` permanently rejects any attempt to reintroduce an in-process dingtalk adapter. Behaviour is preserved for **Stream mode** by the new reference sidecar `librefang.sidecar.adapters.dingtalk` (ships in `librefang-sdk`; source at `sdk/python/librefang/sidecar/adapters/dingtalk.py`, stdlib-only — RFC 6455 WS client via the shared `librefang.sidecar.ws.WebSocketClient`, no third-party deps): same `POST /v1.0/gateway/connections/open` registration with `{clientId, clientSecret, subscriptions: [{type: "CALLBACK", topic: "/v1.0/im/bot/messages/get"}], ua: "librefang"}`, same `wss://endpoint?ticket=<url-encoded>` connection (base64 ticket chars `+`/`=`/`/` percent-encoded via `urllib.parse.quote(safe='')`), same SYSTEM-ping pong with echoed `data` field + `messageId`, same CALLBACK frame parsing (frame `data` is a JSON-encoded string requiring nested `json.loads`), same strict `{code: 200, headers: {contentType, messageId}, message: "OK", data: "{\"response\": null}"}` ACK schema after every CALLBACK regardless of parse outcome, same `msgtype: "text"` filter (other types silently dropped), same `senderStaffId`/`senderId` fallback chain, same conversationType `"1"` (DM) / `"2"` (group) mapping, same `isInAtList` + `atUsers` non-empty mention detection, same slash-command parsing (`/cmd args` → `Command`), same per-message `sessionWebhook` URL extraction for replies, same 20000-char chunking via shared `split_message`, same 200 ms inter-chunk delay, same 3 → 60 s exponential reconnect backoff, same multi-bot `account_id` metadata injection (#5003). **Webhook mode is NOT ported** — both DingTalk modes were stdlib-compatible (HMAC-SHA256 in stdlib `hmac`), but Stream mode is the DingTalk-documented modern default (requires no public IP / port), simpler to operate, and a strict superset of Webhook for restricted-egress deployments. Operators on Webhook mode must re-create the robot in the DingTalk Open Platform with stream subscription enabled and migrate to the sidecar's stream credentials (`DINGTALK_APP_KEY` + `DINGTALK_APP_SECRET` instead of `DINGTALK_ACCESS_TOKEN` + `DINGTALK_SECRET`). **Four improvements over the Rust adapter**: (1) **inbound dedupe on `messageId`** — Rust emitted every CALLBACK unconditionally; on reconnect + platform redelivery the bot could re-emit. Sidecar threads `messageId` through `librefang.sidecar.common.SeenSet` (capacity 10000, evict 5000); (2) **heartbeat-and-send coexist on one socket via stdlib `queue.Queue`** — Rust used `tokio::mpsc` with a separate read/write split; sidecar drains a queue between `wait_readable` ticks. `on_send` is non-blocking; the WS thread drains the queue between heartbeat ticks and message reads, so a slow `sessionWebhook` POST never wedges inbound; (3) **429 `Retry-After` honoured on every outbound POST** — Rust had no 429 handling, so a throttled `sessionWebhook` reply burned the chunking delay or dropped the chunk. Sidecar parses `Retry-After` (default 30 s, floor 1 s, cap 60 s), sleeps, retries once, then logs-and-continues on the second 429 (same shape as #5303 across other sidecars); (4) **explicit 15 s `urlopen` timeout on every HTTP call** — Rust used `reqwest`'s `.timeout(Duration::from_secs(15))` only on gateway registration; the outbound `self.client.post` relied on the client default. Sidecar passes `timeout=15.0` on every call so a misbehaving `sessionWebhook` host can't hang the send loop. **Operator action required**: an existing `[channels.dingtalk]` block is no longer recognised — re-declare as `[[sidecar_channels]]` running `python3 -m librefang.sidecar.adapters.dingtalk` with `DINGTALK_APP_KEY` (in `[sidecar_channels.env]`) and `DINGTALK_APP_SECRET` (in `~/.librefang/secrets.env`); optional knobs are `DINGTALK_ALLOWED_USERS` (CSV staffId allowlist), `DINGTALK_ACCOUNT_ID`. See `sdk/python/librefang/sidecar/adapters/dingtalk.py` header for the exact config. `ChannelType::Custom("dingtalk")` is preserved via `channel_type = "dingtalk"` on the sidecar entry so existing routing / `channel_role_mapping` keys that reference `dingtalk` continue to resolve. Verification: `cd sdk/python && pytest tests/test_dingtalk_adapter.py` — **61 passed** (env enforcement, frame helpers (`_is_system_ping`, `_build_pong_frame`, `_build_callback_ack`), `parse_dingtalk_event` for text / group / slash command / non-text msgtype reject / data not-string / data unparseable / sender fallback / mention detection (`isInAtList` + `atUsers`) / allowlist accept+reject / account_id injection / message_id fallback chain / zero-expired-time omission, `_enqueue_text` (chunking / empty / no-session-webhook), `_mark_seen` (fresh + dedupe + empty), `on_send` (cached sessionWebhook lookup + eviction / user.session_webhook fallback / missing webhook drops / unsupported content placeholder / empty text drop), `_register_gateway` (happy + missing endpoint/ticket + non-200), and end-to-end `_run_session` via an in-memory WS fake (emit after parse / always ACK / pong / msgId dedupe / session_webhook caching / unknown frame types / send-queue drain). (@houko)
- **BREAKING: WeChat (personal account via iLink) migrated from in-process Rust adapter to sidecar-only** — the in-process `librefang-channels::wechat` adapter (`WeChatAdapter`, 1122 lines: REST + long-poll over iLink with persistent `bot_token`, QR-code login flow (`GET /ilink/bot/get_bot_qrcode` + status poll), `POST /ilink/bot/getupdates` long-poll loop with 35 s server-held connections, `POST /ilink/bot/sendmessage` outbound with per-user `context_token` for reply association, `POST /ilink/bot/getconfig` typing-ticket refresh, 5 inbound item types (text / image / voice / file / video), bot-origin self-skip via `@im.bot` suffix, per-user reply-context cache, sender allowlist with exact-match) is deleted along with the `[channels.wechat]` config schema (`WeChatConfig`), the `channel-wechat` cargo feature (from both `librefang-channels` and `librefang-api`, incl. its membership in `all-channels` / `all-channels-no-email`), the dashboard `ChannelMeta` + 4 match arms (`is_some` / serialize / `len` / `ser`), the two custom QR-flow routes (`POST /channels/wechat/qr/start` + `GET /channels/wechat/qr/status`, ~150 lines of QR-state handler), the kernel `channel_sender` `for_each_channel_field!` entry + `EXPECTED` name-list, the `channel_bridge` `WeChatAdapter` import + builder loop, the CLI init-template `[channels.wechat]` block, and the round-trip skill-config test witness (`[channels.wechat]` → `[channels.whatsapp]`). `wechat` is removed from `crates/librefang-channels/src/channels-allowlist.txt`, so `cargo xtask channel-policy` permanently rejects any attempt to reintroduce an in-process wechat adapter. Behaviour is preserved by the new reference sidecar `librefang.sidecar.adapters.wechat` (ships in `librefang-sdk`; source at `sdk/python/librefang/sidecar/adapters/wechat.py`, stdlib-only — `urllib.request` for REST, no third-party deps): same `ilinkai.weixin.qq.com` endpoint, same QR-code login flow (driven by the sidecar itself — QR string logged at INFO for operators to scan from the WeChat app; the dashboard reads it back from sidecar logs), same long-poll cadence with the server-supplied `longpolling_timeout_ms` hint, same `context_token` per-user reply-association cache, same 4096-char chunking, same 5-item-type inbound parsing (text / image / voice / file / video), same allowlist semantics (exact user_id match), same `@im.bot`-suffix self-skip, same multi-bot `account_id` metadata injection (#5003), same persistent `WECHAT_BOT_TOKEN` env-var override to skip the QR flow on restart, same outbound-media degradation (image / file / voice / video send a "[Unsupported content type]" placeholder — the in-process adapter never wired media upload either). **Improvements over the Rust adapter**: (1) **inbound dedupe on `msg_id` / `svr_msg_id`** — Rust emitted every parsed message unconditionally; a long-poll retry could re-deliver. Sidecar threads the IDs through a bounded `SeenSet` (10000 capacity / 5000 evict); (2) **429 `Retry-After` honoured on every REST path** — Rust had no 429 handling at all, so a throttled `getupdates` or `sendmessage` either burned the backoff budget or dropped the chunk. Sidecar parses `Retry-After` (default 30 s, floor 1 s, capped at `WECHAT_MAX_BACKOFF_SECS`), sleeps, retries once, then logs-and-continues on the second 429; (3) **explicit 30 s timeouts on every REST call** — Rust pre-configured `reqwest`'s 90 s default; the sidecar tightens it so a wedged iLink endpoint doesn't pin the worker thread; (4) **shutdown event interrupts backoff** — `threading.Event.wait(backoff)` lets a `Shutdown` command exit the executor thread promptly. **Operator action required**: an existing `[channels.wechat]` block is no longer recognised — re-declare as `[[sidecar_channels]]` running `python3 -m librefang.sidecar.adapters.wechat`; persisted bot tokens move from a `WECHAT_BOT_TOKEN` env-var (still the same name, just consumed by the sidecar process now) into `~/.librefang/secrets.env`. Optional knobs: `WECHAT_ALLOWED_USERS` (csv), `WECHAT_ACCOUNT_ID`, `WECHAT_INITIAL_BACKOFF_SECS`, `WECHAT_MAX_BACKOFF_SECS`. The two dashboard endpoints (`/channels/wechat/qr/start` + `/qr/status`) are removed; the sidecar now logs the QR code itself at INFO. `ChannelType::WeChat` is preserved via `channel_type = "wechat"` on the sidecar entry so existing routing / `channel_role_mapping` keys continue to resolve. Verification: `cd sdk/python && pytest tests/test_wechat_adapter.py` — **51 passed** (env handling, `generate_wechat_uin` shape + uniqueness, `parse_wechat_msg` for 5 item types incl. bot-origin self-skip / empty-text / unsupported / cdn_url fallback / display-name fallback / account_id injection, `_send_text` (basic / chunking / empty drop / no-token raise / 429-retry / HTTP-error raise / body shape), `on_send` dispatch (Text / user.platform_id fallback / unsupported placeholder / empty-user drop / empty-text drop / cached context_token reuse), `_dispatch_messages` integration (emit + reply-context stash / dedupe / allowlist accept+reject / bot-origin skip / account_id injection), QR login (happy path / expired / missing qrcode / non-200 retry), schema + capabilities). (@houko)
- **BREAKING: Email (IMAP + SMTP) migrated from in-process Rust adapter to sidecar-only** — the in-process `librefang-channels::email` adapter (`EmailAdapter`, 1604 lines: `imap` crate poll loop with custom `rustls-connector` TLS context for the per-instance CA-pinning + accept-invalid-certs knobs (#4877), `mailparse` MIME extraction, `lettre` async SMTP over `tokio` with implicit-TLS / STARTTLS pivoting on port, SASL `AUTHENTICATE PLAIN` fallback for Lark/Larksuite, sender allowlist with `@domain` matching (#3463), `[agent] Subject` routing, quarantine-on-poison-pill (`+FLAGS \Seen Librefang-Quarantine`, #3481), per-sender reply-context cache for `In-Reply-To` threading) is deleted along with the `[channels.email]` config schema (`EmailConfig` including the 4 split-credentials fields + `tls_root_ca_path` + `tls_accept_invalid_certs`), the `channel-email` cargo feature (and the five optional deps it gated: `lettre` / `imap` / `rustls-connector` / `rustls-pemfile` / `mailparse`), the `all-channels-no-email` Android-target carve-out (no longer needed — the rustls-connector / rustls-platform-verifier Android incompatibility went away with the IMAP stack), the dashboard `ChannelMeta` descriptor + 4 match arms (`is_some` / serialize / `len` / `ser`), the CLI-TUI `ChannelDef`, the CLI wizard's `email` arm, the kernel `channel_sender` `for_each_channel_field!` entry + `EXPECTED` name-list, the config-validation env-var hook, and the `channel_bridge` `EmailCredentials` + `resolve_email_credentials` helper (+ the 7-test split-credentials fallback unit-test block that exercised it). `email` is removed from `crates/librefang-channels/src/channels-allowlist.txt`, so `cargo xtask channel-policy` permanently rejects any attempt to reintroduce an in-process email adapter. Behaviour is preserved by the new reference sidecar `librefang.sidecar.adapters.email` (ships in `librefang-sdk`; source at `sdk/python/librefang/sidecar/adapters/email.py`, **stdlib-only** — `imaplib.IMAP4_SSL` + `smtplib.SMTP` / `SMTP_SSL` + the `email` package + `ssl`, no third-party deps): same IMAP-poll cadence (default 30 s) with `UID SEARCH UNSEEN UNKEYWORD Librefang-Quarantine` (fallback `UNSEEN` on rejection), same 50-UID-per-cycle fetch cap, same SASL PLAIN fallback (`\0user\0pass`) when LOGIN fails, same MIME walker preferring `text/plain` over the first subpart, same `[agent] Subject` extraction (surfaced via `metadata.target_agent`), same exact-address / `@domain` allowlist (substring rejected, #3463), same per-sender `(subject, message_id)` reply context cache feeding `In-Reply-To` + `References` on outbound, same quarantine-on-poison-pill, same `Subject: ...\n\nbody` convention on outbound text, same SMTP port-routing (465 → `SMTP_SSL`, otherwise STARTTLS via `EHLO`), same multi-bot `account_id` metadata injection (#5003), same `EMAIL_TLS_ROOT_CA_PATH` / `EMAIL_TLS_ACCEPT_INVALID_CERTS` knobs (always-WARN on every connect when validation is off, #4877). **Improvements over the Rust adapter**: (1) **inbound dedupe on Message-ID** — Rust marked Seen after emit; a flag-update failure left the message UNSEEN and the next poll re-emitted it. Sidecar runs a bounded `SeenSet` on Message-ID so a flag-update hiccup doesn't double-emit; (2) **explicit timeouts on every IMAP + SMTP connection** (`EMAIL_NET_TIMEOUT_SECS`, default 60 s); (3) **shutdown event interrupts backoff** — `threading.Event.wait(backoff)` lets `Shutdown` exit the executor thread promptly. **Operator action required**: an existing `[channels.email]` block is no longer recognised — re-declare as `[[sidecar_channels]]` running `python3 -m librefang.sidecar.adapters.email` with `EMAIL_IMAP_HOST` / `EMAIL_SMTP_HOST` / `EMAIL_USERNAME` (in `[sidecar_channels.env]`) and `EMAIL_PASSWORD` (in `~/.librefang/secrets.env`). Per-protocol overrides land on `EMAIL_IMAP_USERNAME` / `EMAIL_IMAP_PASSWORD` / `EMAIL_SMTP_USERNAME` / `EMAIL_SMTP_PASSWORD`; advanced knobs are `EMAIL_IMAP_PORT` (993) / `EMAIL_SMTP_PORT` (587) / `EMAIL_POLL_INTERVAL_SECS` (30) / `EMAIL_FOLDERS` (INBOX, csv) / `EMAIL_ALLOWED_SENDERS` (csv) / `EMAIL_ACCOUNT_ID` / `EMAIL_TLS_ROOT_CA_PATH` / `EMAIL_TLS_ACCEPT_INVALID_CERTS` / `EMAIL_NET_TIMEOUT_SECS`. See `sdk/python/librefang/sidecar/adapters/email.py` header for the exact config. `ChannelType::Email` is preserved via `channel_type = "email"` on the sidecar entry so existing routing / `channel_role_mapping` keys continue to resolve. The Android-specific `all-channels-no-email` feature on `librefang-api` / `librefang-cli` / `librefang-desktop` now collapses to `all-channels` because the IMAP/SMTP code is no longer in the Rust crate graph. Verification: `cd sdk/python && pytest tests/test_email_adapter.py` — **63 passed** (env handling, port/CSV/bool parsing, `extract_email_addr`, `sender_matches_allowlist` (exact / `@domain` / case-insensitive / #3463 no-substring), `extract_agent_from_subject` / `strip_agent_tag`, `parse_email_message` (plaintext / multipart / malformed / Message-ID / text/plain preference / HTML-fallback), `build_outbound_subject`, `_ReplyCtxCache`, `_parse_uid_search` + `_parse_fetch_response`, `_imap_login` LOGIN-then-PLAIN fallback, `_poll_once` (happy-path / Seen flag-set / disallowed-sender quarantine / unparseable quarantine / Message-ID dedupe / fallback-search / account_id injection / reply-context storage), `on_send` (basic / In-Reply-To / explicit Subject prefix / fallback / invalid recipient / unsupported content / port-465 SMTP_SSL), schema + capabilities). (@houko)
- **BREAKING: Feishu / Lark migrated from in-process Rust adapter to sidecar-only** — the in-process `librefang-channels::feishu` adapter (`FeishuAdapter`, 2926 lines: unified Feishu CN + Lark intl, dual receive mode with `axum`-mounted webhook router or `tokio-tungstenite` WebSocket gateway, tenant access token cache with 7200 s expiry + 300 s refresh buffer, AES-256-CBC + PKCS#7 decryption for encrypted webhook payloads via the `aes` + `cbc` crates, `parse_feishu_event` for v2 `im.message.receive_v1` + `parse_feishu_event_v1` legacy + `parse_card_action` for approval button clicks, `@_user_N` mention placeholder expansion, sliding-window event dedup, processing-state `Typing` reaction add/remove via `POST /reactions` / `DELETE /reactions/{id}`, REST `POST /open-apis/im/v1/messages` text + interactive-card outbound, `build_approval_card` builder) is deleted along with the `[channels.feishu]` config schema (`FeishuConfig`, `FeishuRegion`, `FeishuReceiveMode`), the `channel-feishu` cargo feature in both `librefang-channels` and `librefang-api` (incl. its membership in `all-channels` and the `aes` / `cbc` optional deps it gated), the dashboard `ChannelMeta` descriptor + 4 match arms (`is_some` / serialize / `len` / `ser`) + `webhook_route_suffix` entry, the CLI-TUI `ChannelDef`, the kernel `channel_sender` `for_each_channel_field!` entry + `EXPECTED` name-list, the config-validation env-var hook, and the `channel_bridge` `feishu` builder + import. `feishu` is removed from `crates/librefang-channels/src/channels-allowlist.txt`, so `cargo xtask channel-policy` now permanently rejects any attempt to reintroduce an in-process feishu adapter. `librefang-migrate`'s OpenClaw importer (both YAML + JSON5 paths) now records the legacy `[channels.feishu]` block as a skipped sidecar channel (same shape as IRC / Mattermost / Signal / Matrix removals); `test_policy_migration`'s in-process `dmPolicy → dm_policy` witness rotates feishu → google_chat to keep mapping coverage alive. Behaviour is preserved by the new reference sidecar `librefang.sidecar.adapters.feishu` (ships in `librefang-sdk`; source at `sdk/python/librefang/sidecar/adapters/feishu.py`, stdlib-only — `urllib.request` for REST, hand-rolled RFC 6455 WS client over `socket`+`ssl` like discord/slack/webex/mattermost/qq/matrix, **pure-Python AES-256-CBC + PKCS#7 decrypt** for encrypted webhook payloads so we don't pull `cryptography` into the sidecar's stdlib-only dependency contract — verified against NIST SP 800-38A F.2.5 test vectors): same two-step WS endpoint discovery (`POST /callback/ws/endpoint` → `wss://` URL + `ClientConfig.PingInterval`), same default `websocket` receive mode + opt-in `webhook` mode (HTTP server on `FEISHU_WEBHOOK_PORT`), same Feishu (CN) ↔ Lark (intl) auto-routing via `FEISHU_REGION`, same tenant access token caching + 5-minute refresh buffer (feishu.rs:1021-1075 parity), same `im.message.receive_v1` v2 + legacy v1 inbound parsing, same `card.action.trigger` routing to a `Command` content (`approve` / `reject` with `[request_id]` args), same `@_user_N` placeholder expansion (replaces with `@<display_name>`, `@_all` → `@all`), same `sender_type in ("app", "bot")` self-skip (closes the #2435 echo loop), same `root_id` → `thread_id` round-trip, same `account_id` metadata injection for multi-bot routing (#5003), same processing-state `Typing` reaction add/remove (fail-open), same `MAX_MESSAGE_LEN = 4096` chunking, same interactive card outbound via `msg_type = "interactive"`. **Improvements over the Rust adapter**: (1) **pure-Python AES-256-CBC decrypt** — Rust used the `aes` + `cbc` crates with `SHA256(encrypt_key)` as the key; the sidecar re-implements the same primitive in stdlib (`hashlib.sha256` + a hand-coded AES-256 round / S-box / mix-columns) so encrypted webhook payloads round-trip without third-party deps; (2) **explicit timeouts on every HTTP call** — Rust relied on `reqwest`'s default (none); a wedged Feishu endpoint hung the producer task forever. Sidecar passes `timeout=30s` on every REST call + WS handshake; (3) **event dedup is locked at construction** — Rust's `seen_events` was a `Mutex<HashMap>` populated lazily; the sidecar's `_EventDedup` is initialised in `__init__` so concurrent first-event arrivals never race-create separate maps. **Operator action required**: an existing `[channels.feishu]` block is no longer recognised — re-declare as `[[sidecar_channels]]` running `python3 -m librefang.sidecar.adapters.feishu` with `FEISHU_APP_ID` (in `[sidecar_channels.env]`) and `FEISHU_APP_SECRET` (in `~/.librefang/secrets.env`); optional knobs are `FEISHU_REGION` (`cn` / `intl`), `FEISHU_RECEIVE_MODE` (`websocket` / `webhook`), `FEISHU_WEBHOOK_PORT`, `FEISHU_VERIFICATION_TOKEN`, `FEISHU_ENCRYPT_KEY`, `FEISHU_ACCOUNT_ID`. See `sdk/python/librefang/sidecar/adapters/feishu.py` header for the exact config. `ChannelType::Custom("feishu")` and `ChannelType::Custom("lark")` are preserved via `channel_type = "feishu"` on the sidecar entry so existing routing / `channel_role_mapping` keys continue to resolve. Verification: `cd sdk/python && pytest tests/test_feishu_adapter.py` — **91 passed** (covers env handling, region/mode parsing, NIST AES-256 test vector + full PKCS#7 round-trip, payload-decrypt failure modes, event dedup + sliding window purge, `build_approval_card` for 4 risk levels, v2 + v1 + card-action parsers incl. mention expansion / self-skip / group-vs-DM / root_id thread / slash-command routing, token cache + refresh + API-error surfacing, `_validate`, `_send_text` chunking + error propagation, `_send_card` for interactive cards, `on_send` dispatch for Text / Interactive / fallback / channel-id fallback, `_get_ws_endpoint`, `_handle_ws_text` + `_handle_ws_binary` (protobuf-wrapped JSON extraction), `_dispatch_event` dedup + `account_id` injection, webhook HTTP handler for challenge / token verification / encrypted-payload decrypt / 404 / 400, schema + capabilities). (@houko)
- **BREAKING: WeCom migrated from in-process Rust adapter to sidecar-only, WebSocket mode only — Callback mode is removed** — the in-process `librefang-channels::wecom` adapter (`WeComAdapter`, 2497 lines: WebSocket long-connection to `wss://openws.work.weixin.qq.com` with `aibot_subscribe` / `aibot_msg_callback` / `aibot_respond_msg` / `aibot_send_msg` / `ping` / `pong` frame routing + `cmd`/`action` and `body`/`data` legacy-key tolerance + per-user `req_id` cache for passive replies + 30 s heartbeat + 1 → 30 s exponential reconnect backoff + **callback mode**: HTTP webhook server with HMAC-SHA1 signature verification over `sort(token, timestamp, nonce, encrypt)`, AES-CBC-256 decryption of inbound payloads (32-byte base64 key, 16-byte IV from key prefix, 16-byte random prefix + 4-byte big-endian length + receiveid suffix + PKCS#7 32-byte block alignment), one-shot `response_url` cache with 5-minute TTL + composite `user_id|chat_id` key for groups, and webhook-key fallback extracted from the first inbound `response_url`) is deleted along with the `[channels.wecom]` config schema (`WeComConfig` + `WeComMode` enum), the `channel-wecom` cargo feature in both `librefang-channels` and `librefang-api` (incl. its membership in `all-channels` / `all-channels-no-email`; the optional `aes` / `cbc` deps stay because `channel-feishu` still gates them), the dashboard `ChannelMeta` descriptor + 4 match arms (`is_some` / serialize / `len` / `ser`), the kernel `channel_sender` `for_each_channel_field!` entry + `EXPECTED` name-list, the kernel-side wecom-specific formatter dispatch (`format_for_wecom` / `markdown_to_wecom_plain` plus the 7 internal helpers they used: `strip_atx_heading` / `strip_blockquote_prefix` / `strip_task_list_prefix` / `is_fenced_code_marker` / `is_setext_heading_underline` / `is_table_divider` / `strip_inline_markdown`), the `default_channel_initial_backoff_secs` shared constant that had no other caller, the `inject_callback_url` arm and the `webhook_route_suffix` `wecom` entry (wecom was the only `callback_url`-bearing in-process channel), and the `test_one_or_many_array_of_wecom_tables` config-parse test. `wecom` is removed from `crates/librefang-channels/src/channels-allowlist.txt`, so `cargo xtask channel-policy` permanently rejects any attempt to reintroduce an in-process wecom adapter. Behaviour is preserved for **WebSocket mode** by the new reference sidecar `librefang.sidecar.adapters.wecom` (ships in `librefang-sdk`; source at `sdk/python/librefang/sidecar/adapters/wecom.py`, stdlib-only — RFC 6455 WS client via the shared `librefang.sidecar.ws.WebSocketClient`, no third-party deps): same `wss://openws.work.weixin.qq.com` endpoint, same `aibot_subscribe` handshake carrying `bot_id` + `secret`, same `cmd`/`action` and `body`/`data` legacy-key tolerance, same `userid`/`user_id` and `chattype`/`chat_type` fallback chain, same `is_subscribe_success` detection (both explicit `cmd: "aibot_subscribe"` ack AND server-style ack with `errcode: 0` + `headers.req_id` starting with `"aibot_subscribe"`), same `aibot_msg_callback` parsing with text-only msgtype filter (non-text msgtypes still silently dropped, mirroring `wecom.rs:103-106`), same per-user `req_id` cache so the first outbound after an inbound uses `aibot_respond_msg` (one-shot, evicted on send) and subsequent outbounds fall back to `aibot_send_msg`, same `msgtype: "markdown"` body shape (WeCom's intelligent-bot `aibot_respond_msg` rejects `msgtype: "text"`), same 4096-char chunking via the shared `split_message`, same 30 s `cmd: "ping"` heartbeat, same 1 → 30 s exponential reconnect backoff, same multi-bot `account_id` metadata injection (#5003). **Callback mode is NOT ported** — Python's standard library has no AES-CBC primitive, and the sidecar SDK is stdlib-only by policy across all 19 reference adapters. Operators who relied on callback mode must either switch the bot to WebSocket mode in the WeCom admin console (it requires no public endpoint, so the WS path is a strict superset of what callback could do in restricted-egress environments), or ship their own callback-mode sidecar that brings its own AES dependency. **Three improvements over the Rust adapter**: (1) **inbound dedupe on `req_id`** — the Rust emit at `wecom.rs:770` was unconditional, so a WS reconnect that races with the platform's redelivery would emit the same message twice. The sidecar threads `req_id` through `librefang.sidecar.common.SeenSet` (capacity 10000, evict 5000), matching the dedupe envelope every recent sidecar (qq, mattermost, signal, line, matrix) settled on; (2) **heartbeat-and-send coexist on one socket via a stdlib `queue.Queue`** — the Rust adapter used a bounded `tokio::mpsc` (`wecom.rs:580`); the sidecar uses an unbounded `queue.Queue` polled at the same read tick as inbound. `on_send` is non-blocking; the WS thread drains the queue between heartbeat ticks and message reads, so a slow `aibot_send_msg` server-side never wedges inbound; (3) **send result is observable in logs** — the Rust adapter only logged `frame sent over WebSocket successfully` (`wecom.rs:631`) before the server ACK arrived; the sidecar logs the same plus the server's `errcode` / `errmsg` (when non-zero) on the ACK frame, so operators can correlate a `send succeeded` log line with the actual platform-side outcome instead of having to enable DEBUG. **Operator action required**: an existing `[channels.wecom]` block is no longer recognised — re-declare as `[[sidecar_channels]]` running `python3 -m librefang.sidecar.adapters.wecom` with `WECOM_BOT_ID` (in `[sidecar_channels.env]`) and `WECOM_BOT_SECRET` (in `~/.librefang/secrets.env`); optional knobs are `WECOM_ALLOWED_USERS`, `WECOM_ACCOUNT_ID`. See `sdk/python/librefang/sidecar/adapters/wecom.py` header for the exact config. `ChannelType::Custom("wecom")` is preserved via `channel_type = "wecom"` on the sidecar entry so existing routing / `channel_role_mapping` keys that reference `wecom` continue to resolve. Verification: `cd sdk/python && pytest tests/test_wecom_adapter.py` — **62 passed** (env enforcement, frame-helper key fallback (`cmd`/`action`, `body`/`data`, `headers.req_id`/`body.req_id`), `_is_subscribe_success` (explicit + server-style ack + nonzero errcode), `parse_wecom_event` for text / legacy `action`/`data` / group / non-text msgtype reject / event-cmd reject / missing-req_id / missing-user / empty-content / allowlist accept+reject / account_id injection + omission / `response_url` metadata surfacing, frame builders (subscribe / respond_msg / send_msg / ping), `_enqueue_text` routing (no-req_id → send_msg / req_id → respond_msg+evict / chunking / first-chunk-respond-rest-send / empty-text noop), `_mark_seen` (fresh + dedupe + empty), `on_send` (basic / user.platform_id fallback / unsupported content placeholder / no-user-id drop / empty-text drop), and end-to-end `_run_session` via a scripted in-memory WS fake (subscribe-first emission / message-after-ack / req_id dedupe across redelivery / req_id caching / subscribe failure returns / non-msg-callback frames ignored / send-queue drained). (@houko)
- **BREAKING: Matrix migrated from in-process Rust adapter to sidecar-only** — the in-process `librefang-channels::matrix` adapter (`MatrixAdapter`, 3356 lines: long-poll `GET /sync` + `PUT /rooms/{}/send/{}/{}` outbound + `POST /_matrix/media/v3/upload` + reaction lifecycle + streaming-edit `m.replace` with 429 retry + E2EE warn-once per room + `mxc://` → MSC3916 authenticated download URL + `pulldown-cmark` Markdown→HTML rendering for `formatted_body`) is deleted along with the `[channels.matrix]` config schema (`MatrixConfig`), the `channel-matrix` cargo feature (and the optional `pulldown-cmark` dep it gated), the dashboard `ChannelMeta` descriptor + 4 match arms, the CLI-TUI `ChannelDef`, the kernel `channel_sender` macro entry, the config-validation env-var hook, and the legacy `crates/librefang-api/tests/channels_routes_test.rs` integration test (which used `MatrixConfig` as its only in-process witness and required a separate rewrite that's deferred). `matrix` is removed from `crates/librefang-channels/src/channels-allowlist.txt`, so `cargo xtask channel-policy` permanently rejects any attempt to reintroduce an in-process matrix adapter. `librefang-migrate`'s OpenClaw importer (both YAML + JSON5 paths) now records the legacy `[channels.matrix]` block as a skipped sidecar channel; `test_policy_migration`'s in-process witness rotates discord → slack → mattermost → signal → matrix → **feishu** to keep `dmPolicy: "disabled"` → `dm_policy = "ignore"` coverage alive. Behaviour is preserved by the new reference sidecar `librefang.sidecar.adapters.matrix` (ships in `librefang-sdk`; source at `sdk/python/librefang/sidecar/adapters/matrix.py`, stdlib-only — `urllib.request` for HTTPS, hand-rolled CommonMark subset renderer for `formatted_body`, no third-party deps): same long-poll `/sync` with `since` cursor + 30 s server timeout, same `m.room.message` event filter with 5-msgtype dispatch (`m.text`/`m.notice`/`m.emote` → text or Command on `/` prefix; `m.image`/`m.file`/`m.audio`/`m.video` → media), same room allowlist + self-skip on `sender == user_id`, same E2EE warn-once per room, same `mxc://` → MSC3916 authenticated download URL conversion, same `parse_thread_relation` → `thread_id` on inbound, same multi-bot `account_id` metadata injection (#5003), same `MAX_MESSAGE_LEN = 4096` chunking, same 5 outbound surfaces (text + 11 ChannelContent variants in `on_send`, typing via `TypingCmd`, reaction with lifecycle redact + insert via `Reaction`, thread wrap via `cmd.thread_id`, streaming edit via `StreamStart`/`StreamDelta`/`StreamEnd`), same `m.replace` edit with shared `txn_id` across both attempts under 429, same 1–60 s exponential reconnect backoff on `/sync` failure, same default 50 MiB upload cap (`MATRIX_MAX_UPLOAD_BYTES` override). **Three improvements over the Rust adapter**: (1) **inbound dedupe on `event_id`** — Rust emitted every event_id from a sync batch unconditionally; on retry / `since` reset the bot could re-emit. Bounded `SeenSet` with `SEEN_MESSAGES_MAX=10000` / `EVICT=5000`; (2) **429 `Retry-After` honoured at every PUT, not just edit** — Rust's `api_edit_event_with_retry` honoured Retry-After but `api_send_event` and `api_redact` did not. The sidecar's `_put_event` honours it everywhere (1 retry then raise on second 429); (3) **explicit 60 s timeout on `/sync`, 30 s on every other REST call** — Rust relied on `reqwest`'s default (none); a hung homeserver would hang the producer thread forever. Markdown→HTML rendering is a stdlib subset (headings, bold, italic, inline code, fenced code blocks, links with `javascript:` / `data:` scheme rejection, lists, blockquotes, horizontal rules, GFM tables, strikethrough, `<think>` strip, paragraph wrapping). Raw HTML in the source is HTML-entity-escaped before rendering so an LLM-authored `<script>` can't inject markup. **Operator action required**: an existing `[channels.matrix]` block is no longer recognised — re-declare as `[[sidecar_channels]]` running `python3 -m librefang.sidecar.adapters.matrix` with `MATRIX_HOMESERVER_URL` + `MATRIX_USER_ID` (in `[sidecar_channels.env]`) and `MATRIX_ACCESS_TOKEN` (in `~/.librefang/secrets.env`); optional knobs `MATRIX_ALLOWED_ROOMS`, `MATRIX_ACCOUNT_ID`, `MATRIX_MAX_UPLOAD_BYTES`. See `sdk/python/librefang/sidecar/adapters/matrix.py` header for the exact config. `ChannelType::Matrix` is preserved via `channel_type = "matrix"` on the sidecar entry so existing routing / `channel_role_mapping` keys that reference `matrix` continue to resolve. Verification: `cd sdk/python && pytest tests/test_matrix_adapter.py` — **81 passed** (covers env enforcement, `mxc_to_http` (4 cases), `markdown_to_matrix_html` (15 cases incl. `javascript:` / `data:` rejection + HTML-escape + `<think>` strip + GFM tables), `text_body_with_html` + `build_edit_body` truncation, `parse_thread_relation` (present / absent / replace / malformed), `parse_inbound_msg_content` for 5 msgtypes + edge cases (empty body / unknown / missing-url / slash-command / Voice via MSC3245 / Audio plain / Video / File `filename` over `body`), `_process_sync_body` (emit, self-skip, room allowlist, dedupe across two batches, E2EE warn-once, non-m.room.message skip, thread surfacing, account_id injection), reaction-lifecycle cache (insert/replace/lookup/remove/capacity eviction), `_put_event` (happy / 429-then-200 / persistent-429-raises / non-2xx-raises), `_upload_media` (returns mxc / size-cap rejects / failure raises), `_validate` (200 / 401), `_format_with_button_hints`, `on_send` (text / chunks-long / thread-wraps-relation / empty-room drops / falls-back-to-user.platform_id), and the SCHEMA + capabilities contract. (@houko)
- **BREAKING: QQ migrated from in-process Rust adapter to sidecar-only** — the in-process `librefang-channels::qq` adapter (`QqAdapter`, 758 lines: `POST https://bots.qq.com/app/getAppAccessToken` token mint + `GET /gateway` discovery + `tokio-tungstenite` WebSocket to QQ Bot API v2's gateway with HELLO(op=10) → IDENTIFY(op=2) → READY handshake + heartbeat(op=1) loop + DISPATCH(op=0) routing across `MESSAGE_CREATE` / `AT_MESSAGE_CREATE` / `DIRECT_MESSAGE_CREATE` / `GROUP_AT_MESSAGE_CREATE` / `C2C_MESSAGE_CREATE` + REST `POST {api_base}{endpoint}` outbound with markdown stripping) is deleted along with the `[channels.qq]` config schema (`QqConfig`), the `channel-qq` cargo feature in both `librefang-channels` and `librefang-api` (incl. its membership in `all-channels` / `all-channels-no-email`), the dashboard `ChannelMeta` descriptor + 4 match arms (`is_some` / serialize / `len` / `ser`), the kernel `channel_sender` `for_each_channel_field!` entry + `EXPECTED` name-list. `qq` is removed from `crates/librefang-channels/src/channels-allowlist.txt`, so `cargo xtask channel-policy` now permanently rejects any attempt to reintroduce an in-process qq adapter. Behaviour is preserved by the new reference sidecar `librefang.sidecar.adapters.qq` (ships in `librefang-sdk`; source at `sdk/python/librefang/sidecar/adapters/qq.py`, stdlib-only — `urllib.request` for REST, hand-rolled RFC 6455 WS client over `socket`+`ssl` like the discord/slack/webex/mattermost sidecars, no third-party deps): same token mint via `POST bots.qq.com/app/getAppAccessToken`, same `GET /gateway` discovery, same HELLO/IDENTIFY/READY/HEARTBEAT/DISPATCH/RECONNECT/INVALID_SESSION opcode handling, same intents bitmask default (`GUILDS | GUILD_MEMBERS | DIRECT_MESSAGE | GROUP_AND_C2C | PUBLIC_GUILD_MESSAGES`), same 4 dispatch event types with the same reply-endpoint mapping, same leading-`/` bot-mention strip, same allowlist + slash-command routing, same multi-bot `account_id` metadata injection (#5003), same outbound markdown stripping pipeline (think tags, code blocks, inline code, bold, italic, headings, table separators, links, blockquotes, horizontal rules, three-or-more newlines), same 2000-char chunking, same 1–60s exponential reconnect backoff. **Four improvements over the Rust adapter**: (1) **reply context actually round-trips** — the Rust `parse_dispatch_event` (`qq.rs:182-246`) computed `reply_endpoint` and `msg_id` but the dispatch loop bound them to `_endpoint` / `_msg_id` (`qq.rs:399`) and dropped them on the floor; `send` (`qq.rs:497-498`) then expected `user.platform_id` to be encoded as `"<endpoint>|<msg_id>"` and silently no-op'd when the delimiter wasn't there. The Rust adapter therefore failed every real outbound — only the synthetic wiremock tests at `qq.rs:686-712` exercised the working shape. The sidecar surfaces the reply endpoint as `channel_id` and the QQ `msg_id` as `thread_id` on the inbound event so the bridge round-trips them through to `on_send`, which posts to `{api_base}{channel_id}` with the correct passive-reply `msg_id`; (2) **inbound dedupe on `msg.id`** — the Rust dispatch loop (`qq.rs:399-410`) emitted every parsed event unconditionally; a WS reconnect that races with the server's last-delivery cursor would re-deliver. Bounded local set on `id` with `SEEN_MESSAGES_MAX=10000` / `EVICT=5000` (same policy as reddit / rocketchat / nextcloud / webex / line / mattermost / signal); (3) **429 `Retry-After` honoured on every REST path** — Rust had no 429 handling, so a throttled `getAppAccessToken` / `/gateway` / outbound `POST` either burned the reconnect budget or dropped the chunk. Sidecar parses `Retry-After` (default 30 s fallback, floor 1 s, cap `MAX_BACKOFF_SECS`), sleeps, retries once, then logs-and-continues on the second 429 (same shape as #5303); (4) **explicit 15s `urlopen` timeouts on every REST call** — `urllib.request.urlopen` has no default timeout; Rust pre-configured `reqwest`'s 30s default. Sidecar passes `timeout=SEND_TIMEOUT_SECS` (15 s) on every call so a misbehaving REST endpoint trips an explicit error instead of hanging the worker thread. **Operator action required**: an existing `[channels.qq]` block is no longer recognised — re-declare as `[[sidecar_channels]]` running `python3 -m librefang.sidecar.adapters.qq` with `QQ_APP_ID` (in `[sidecar_channels.env]`) and `QQ_APP_SECRET` (in `~/.librefang/secrets.env`); optional knobs are `QQ_ALLOWED_USERS`, `QQ_ACCOUNT_ID`, `QQ_INTENTS`. See `sdk/python/librefang/sidecar/adapters/qq.py` header for the exact config. `ChannelType::Custom("qq")` is preserved via `channel_type = "qq"` on the sidecar entry so existing routing / `channel_role_mapping` keys continue to resolve. Verification: `cd sdk/python && pytest tests/test_qq_adapter.py` (77 new tests) covers env-var enforcement (app_id/secret required, intents decimal+hex+garbage, ws-url override), `strip_markdown` (bold/italic/code/heading/link/quote/table-sep/HR/think-tags/triple-newlines collapse), `_parse_retry_after` (5 cases), `parse_qq_event` for all 4 event types + edge cases (empty/whitespace content, unknown type, non-dict data, allowlist accept/reject, bot-mention `/` strip vs slash-command, account_id injection, username fallback, missing msg_id), `_mark_seen` capacity eviction, `_fetch_token` / `_fetch_gateway` (happy path + 429-retry + non-200 + missing field), `_post_message` (basic shape, chunking, empty endpoint, omits msg_id when None, 429-retry-once, persistent-429 fail-open, 5xx fail-open keeps chunking), `on_send` (text, markdown stripping at on_send boundary, unsupported content → placeholder, empty endpoint, falls back to user.platform_id), and the WS gateway flow via a mock `_WebSocketClient` (HELLO → IDENTIFY token+intents+shard, DISPATCH emission, msg.id dedupe across two dispatches, RECONNECT op returns, INVALID_SESSION sleeps 3s and returns, heartbeat fires after interval). (@houko)
- **BREAKING: Signal migrated from in-process Rust adapter to sidecar-only** — the in-process `librefang-channels::signal` adapter (`SignalAdapter`, 975 lines: polling loop against `signal-cli-rest-api` with a configurable URL + phone number, optional Bearer `SIGNAL_API_KEY`, SSRF guard rejecting loopback / RFC-1918 / link-local / CGNAT / IPv6 ULA addresses unless `allow_local = true`, `POST /v2/send` outbound with optional base64 attachments, slash-command routing) is deleted along with the `[channels.signal]` config schema (`SignalConfig`), the `channel-signal` cargo feature in both `librefang-channels` and `librefang-api` (incl. its membership in `all-channels` / `all-channels-no-email` / `mini`), the dashboard `ChannelMeta` descriptor + 4 match arms (`is_some` / serialize / `len` / `ser`), the CLI-TUI `ChannelDef`, the kernel `channel_sender` `for_each_channel_field!` entry + `EXPECTED` name-list, and the `default_signal_poll_interval_secs` helper. `signal` is removed from `crates/librefang-channels/src/channels-allowlist.txt`, so `cargo xtask channel-policy` now permanently rejects any attempt to reintroduce an in-process signal adapter. `librefang-migrate`'s OpenClaw importer records the legacy `[channels.signal]` block (and the JSON-block variant) as a skipped sidecar channel (same shape as the IRC / Mattermost removals) instead of emitting TOML the kernel would refuse to deserialize. Behaviour is preserved by the new reference sidecar `librefang.sidecar.adapters.signal` (ships in `librefang-sdk`; source at `sdk/python/librefang/sidecar/adapters/signal.py`, stdlib-only — `urllib.request` + `socket`/`ipaddress` for the SSRF guard, no third-party deps): same SSRF safety contract (default-deny on private/loopback unless `SIGNAL_ALLOW_LOCAL=1`), same `GET /v1/receive/{phone}` polling + `POST /v2/send` outbound, same self / allowlist / empty-text filters, same `slash-command` routing, same `account_id` metadata injection (#5003). **Improvements over the Rust adapter**: (1) inbound dedupe on `envelope.timestamp` with `SEEN_MESSAGES_MAX=10000` / `EVICT=5000` (Rust emit at signal.rs:398-415 was unconditional, so a retry redelivered duplicates); (2) 429 `Retry-After` honoured on both poll and send paths (Rust had no 429 handling); (3) explicit 15s `urlopen` timeouts on every REST call; (4) 1–60s exponential backoff on transport / non-2xx errors (Rust just `continue`-d on every error, spinning at `poll_interval` against a wedged daemon). The Rust adapter's inline base64 attachment support (`Image` / `Voice` / `Video` / `Audio` / `Animation` / `File` / `FileData` / `MediaGroup`) is not yet wired through the sidecar — non-text content currently degrades to a `(Unsupported content type)` placeholder; a follow-up will restore the base64 round-trip. **Operator action required**: an existing `[channels.signal]` block is no longer recognised — re-declare as `[[sidecar_channels]]` running `python3 -m librefang.sidecar.adapters.signal`. See `sdk/python/librefang/sidecar/adapters/signal.py` header for the exact config. `ChannelType::Signal` is preserved via `channel_type = "signal"` on the sidecar entry so existing routing / `channel_role_mapping` keys continue to resolve. (@houko)
- **BREAKING: Mattermost migrated from in-process Rust adapter to sidecar-only** — the in-process `librefang-channels::mattermost` adapter (`MattermostAdapter`, 954 lines: WebSocket gateway to `wss://<host>/api/v4/websocket` with an `authentication_challenge` JSON frame after the upgrade, `posted` event parsing, REST `POST /api/v4/posts` outbound, REST `POST /api/v4/users/me/typing` typing indicators, auth via Bearer personal/bot access token) is deleted along with the `[channels.mattermost]` config schema (`MattermostConfig`), the `channel-mattermost` cargo feature in both `librefang-channels` and `librefang-api` (incl. its membership in `all-channels` / `all-channels-no-email` / `mini`), the dashboard `ChannelMeta` descriptor + 4 match arms (`is_some` / serialize / `len` / `ser`), the CLI-TUI `ChannelDef`, the kernel `channel_sender` `for_each_channel_field!` entry + `EXPECTED` name-list, and the config-validation env-var hook. `mattermost` is removed from `crates/librefang-channels/src/channels-allowlist.txt`, so `cargo xtask channel-policy` now permanently rejects any attempt to reintroduce an in-process mattermost adapter. `librefang-migrate`'s OpenClaw importer now records the legacy `[channels.mattermost]` block as a skipped sidecar channel (same shape as the IRC removal) instead of emitting TOML the kernel would refuse to deserialize. Behaviour is preserved by the new reference sidecar `librefang.sidecar.adapters.mattermost` (ships in `librefang-sdk`; source at `sdk/python/librefang/sidecar/adapters/mattermost.py`, stdlib-only — `urllib.request` for REST, hand-rolled RFC 6455 WS client over `socket`+`ssl` like the webex/discord/slack sidecars, no third-party deps): same `GET /api/v4/users/me` startup credential probe + bot-id self-skip, same WebSocket auth challenge handshake, same `posted` event filter with double-decoded `data.post` JSON parse (mattermost.rs:197 parity), same source-type → `is_group` mapping (`channel_type == "D"` ⇒ DM), same slash-command routing, same channel-allowlist filter, same `account_id` metadata injection (#5003), same `MAX_MESSAGE_LEN=16383` chunking, same `(Unsupported content type)` fallback, same 1–60s exponential reconnect backoff. **Improvements over the Rust adapter**: (1) inbound `post.root_id` is round-tripped as `thread_id` and `on_send` re-posts `root_id`, so the bot's reply actually threads under the originating post (Rust `send` at mattermost.rs:446-462 dropped `root_id`); (2) 429 `Retry-After` honoured on every REST path (Rust had no 429 handling); (3) bounded inbound dedupe on `post.id` with `SEEN_MESSAGES_MAX=10000` / `SEEN_MESSAGES_EVICT=5000` (Rust emit at mattermost.rs:425 was unconditional, so a WS reconnect double-delivered); (4) explicit 15s `urlopen` timeouts on every REST call. **Operator action required**: an existing `[channels.mattermost]` block is no longer recognised — re-declare as `[[sidecar_channels]]` running `python3 -m librefang.sidecar.adapters.mattermost`. See `sdk/python/librefang/sidecar/adapters/mattermost.py` header for the exact config. `ChannelType::Custom("mattermost")` is preserved via `channel_type = "mattermost"` on the sidecar entry so existing routing / `channel_role_mapping` keys continue to resolve. (@houko)
- **BREAKING: LINE migrated from in-process Rust adapter to sidecar-only** — the in-process `librefang-channels::line` adapter (`LineAdapter`, 881 lines: `BaseHTTPRequestHandler`-style axum webhook route mounted at `/channels/line/webhook` on the shared API server for `X-Line-Signature`-verified inbound delivery + `POST /v2/bot/message/push` for outbound, auth via Bearer channel-access-token) is deleted along with the `[channels.line]` config schema (`LineConfig`), the `channel-line` cargo feature in both `librefang-channels` and `librefang-api` (incl. its membership in `all-channels` / `all-channels-no-email`), the dashboard `ChannelMeta` descriptor + 4 match arms (`is_some` / serialize / `len` / `ser`), the CLI-TUI `ChannelDef`, the kernel `channel_sender` `for_each_channel_field!` entry + `EXPECTED` name-list, the config-validation env-var hook, and the `webhook_route_suffix` allowlist entry that previously routed LINE's inbound POSTs through the shared API server. `line` is removed from `crates/librefang-channels/src/channels-allowlist.txt`, so `cargo xtask channel-policy` now permanently rejects any attempt to reintroduce an in-process line adapter. Behaviour is preserved by the new reference sidecar `librefang.sidecar.adapters.line` (ships in `librefang-sdk`; source at `sdk/python/librefang/sidecar/adapters/line.py`, stdlib-only — `urllib.request` + `http.server.ThreadingHTTPServer`, no third-party deps; on the `librefang.sidecar` SDK): same `GET /v2/bot/info` startup credential probe, same `X-Line-Signature` HMAC-SHA256 verification over the **raw wire bytes** (not bytes round-tripped through `serde_json::Value`, which would reorder keys and never match — `line.rs:229-250` parity, including the regression for the bug a `serde_json::from_slice` re-serialise path would have introduced), same `message`-event-only / `text`-message-only filter (other event types — follow, unfollow, postback, beacon — and other message types — sticker, image, video — dropped, `line.rs:256-273`), same source-type → `reply_to` mapping (`user` → `userId`, `group` → `groupId`, `room` → `roomId`; group/room → `is_group=true`, `line.rs:280-290`), same slash-command routing on `/cmd args` → `Command` (text otherwise), same metadata preservation (`user_id` / `reply_to` / `reply_token` / `source_type` — every key the Rust adapter wrote at `line.rs:310-329` ships unchanged so downstream consumers continue to resolve), same multi-bot `account_id` metadata injection (#5003 parity), same `MAX_MESSAGE_LEN = 5000` character chunking (`LINE_MSG_LIMIT` parity with the Rust constant at `line.rs:39`), same image-branch wire shape (`originalContentUrl` + `previewImageUrl` both set to the caller-supplied URL, caption sent as a follow-up text push, `line.rs:464-490`). **Three improvements on top of the Rust adapter**: (1) **429 `Retry-After` honoured on outbound** — the Rust `api_push_message` (`line.rs:148-184`) had no 429 handling at all, so a server-side rate-limit caused `send()` to return `Err` and dropped the outbound chunk; the sidecar parses `Retry-After` (with a `RETRY_AFTER_DEFAULT_SECS = 30.0` fallback, floor 1 s, cap `MAX_BACKOFF_SECS`), sleeps, and retries once before logging-and-continuing on the second 429 (same shape as `fix(channels): honour Retry-After across sidecar polling adapters` #5303); (2) **inbound dedupe on `message.id`** — LINE redelivers webhook events when the operator's endpoint fails (non-2xx or timeout); the Rust handler at `line.rs:413-427` emitted every event unconditionally, so a transient downstream failure caused duplicate agent invocations. The sidecar dedupes locally on `message.id` with a bounded `SEEN_MESSAGES_MAX = 10 000` / `SEEN_MESSAGES_EVICT = 5 000` cap (same policy as reddit / rocketchat / nextcloud / webex); (3) **explicit HTTP timeouts on every `urlopen`** — `urllib.request.urlopen` has no default timeout, so a hung LINE API would hang the worker thread forever; every call now passes `timeout=SEND_TIMEOUT_SECS` (15 s) so a misbehaving endpoint trips an explicit error. **Operator action required (substantive)**: the sidecar runs **its own HTTP webhook server** (default port `9090`, override via `LINE_WEBHOOK_PORT`; default path `/webhook`, override via `LINE_WEBHOOK_PATH`) — it is **no longer mounted on the LibreFang API port**, so the webhook URL you have registered at the LINE Developers Console must be updated to point at the sidecar host (typical pattern: an HTTPS reverse proxy in front of the sidecar's listening port). An existing `[channels.line]` block is no longer recognised — re-declare as a `[[sidecar_channels]]` running `python3 -m librefang.sidecar.adapters.line` with `LINE_CHANNEL_SECRET` and `LINE_CHANNEL_ACCESS_TOKEN` (both in `~/.librefang/secrets.env`) plus any optional knobs (in `[sidecar_channels.env]`): `LINE_WEBHOOK_PORT`, `LINE_WEBHOOK_PATH`, `LINE_ACCOUNT_ID`, `LINE_BIND_HOST` (defaults to `0.0.0.0`). `ChannelType::Custom("line")` (the channel-type token the Rust adapter advertised at `line.rs:353-355`) is preserved across this migration via `channel_type = "line"` on the sidecar entry, so existing routing and `channel_role_mapping` keys that reference `line` continue to resolve. Verification: `cd sdk/python && pytest tests/test_line_adapter.py` (68 new tests) covers env-var enforcement (whitespace-only secret/token still exits 2, port/path/account-id parsing, bind-host override, api_base override), `_split_message` chunking (under-limit, newline-cut, hard-cut, 5000 cap parity), `_parse_retry_after` (missing-uses-default, integer seconds, garbage-falls-back, 1 s floor, `MAX_BACKOFF_SECS` cap), `verify_line_signature` (round-trip happy path, wrong secret rejects, mutated body rejects, empty/whitespace/non-base64 signature rejects — regression for #3439, and the wire-bytes-vs-JSON-roundtrip regression which would otherwise have rejected every legitimate LINE webhook), `parse_line_event` (text user message, group `groupId` → reply_to mapping, room `roomId` → reply_to mapping, slash command with args, slash command no args, non-message event returns None, non-text message returns None, empty text returns None, missing source returns None, missing reply_token omitted from metadata, account_id metadata injection when present and omission when absent), `_mark_seen` (fresh vs repeat, empty id no-op, eviction at cap with parametrised small caps), `_validate_token` (200 happy path with auth-header + timeout assertions, 429-then-200 with `Retry-After` honoured, non-200 raises with status in the message, missing-displayName falls back to "LINE Bot"), `_push_text` + `_post_push` (single-chunk shape, multi-chunk one-call-per-chunk preservation, 429-then-200 with explicit Retry-After honoured, persistent 429 fail-open so the rest of a multi-chunk reply still ships), `_push_image` (image + caption two-call shape, no-caption single call, empty-URL skip), `_handle_webhook_body` (valid signature emits, invalid/missing signature returns 401, bad JSON returns 400, non-object body returns 400, empty events returns 200 for the LINE Developers Console URL-verification ping, dedupes repeated message ids, follow event leaves dedupe set empty so the next real text message is not silently dropped, account_id metadata injection on emitted events), `on_send` wiring (text, image, unsupported content → placeholder, empty platform_id drops silently, falls back to `user.platform_id` when `channel_id` is absent), and the `--describe` SCHEMA round-trip. (@houko)
- **BREAKING: Zulip migrated from in-process Rust adapter to sidecar-only** — the in-process `librefang-channels::zulip` adapter (`ZulipAdapter`, 713 lines: HTTP Basic auth on `<bot_email>:<api_key>` + `POST /api/v1/register` event-queue mint + long-poll `GET /api/v1/events?queue_id=<q>&last_event_id=<n>&dont_block=false` + `POST /api/v1/messages` form-encoded publish) is deleted along with the `[channels.zulip]` config schema (`ZulipConfig`), the `channel-zulip` cargo feature in both `librefang-channels` and `librefang-api` (incl. its membership in `all-channels` / `all-channels-no-email`), the dashboard `ChannelMeta` descriptor + 4 match arms (`is_some` / serialize / `len` / `ser`), the CLI-TUI `ChannelDef`, the kernel `channel_sender` `for_each_channel_field!` entry + `EXPECTED` name-list, the config-validation env-var hook, and the `librefang-types::config::tests::test_zulip_config_defaults` / `…_serde` unit tests. `zulip` is removed from `crates/librefang-channels/src/channels-allowlist.txt`, so `cargo xtask channel-policy` now permanently rejects any attempt to reintroduce an in-process zulip adapter. Behaviour is preserved by the new reference sidecar `librefang.sidecar.adapters.zulip` (ships in `librefang-sdk`; source at `sdk/python/librefang/sidecar/adapters/zulip.py`, stdlib-only, on the `librefang.sidecar` SDK): same `GET /api/v1/users/me` startup credential probe to discover the bot's stable integer `user_id` and `full_name`, same HTTP Basic auth on every REST call (`<bot_email>:<api_key>`), same event-queue register with `event_types=["message"]` + optional `narrow=[["stream", "<name>"], …]` when `ZULIP_STREAMS` is set, same long-poll `GET /api/v1/events` with `dont_block=false` and a 70 s HTTP timeout (matching the Rust `POLL_TIMEOUT_SECS + 10` budget at zulip.rs:244), same queue-expiry recovery on 400 + `code == "BAD_EVENT_QUEUE_ID"` re-register (mirrors zulip.rs:262-308), same client-side stream filter on `message.display_recipient` as defence-in-depth against the server-side narrow being best-effort, same slash-command routing on `/cmd args` → `Command` (text otherwise), same DM detection via `message.type == "private"` → `is_group = false` with platform_id falling back to sender email, same multi-bot `account_id` metadata injection, same 10 000-char message chunking (`ZULIP_MSG_LIMIT` parity with the Rust `MAX_MESSAGE_LEN`), same outbound DM heuristic (`@` in `cmd.user.platform_id` ⇒ `type=direct`), same exponential reconnect backoff 1 s → 60 s. **Four improvements on top of the Rust adapter**: (1) **outbound topic round-trip via `thread_id`** — the Rust `send` at `crates/librefang-channels/src/zulip.rs` line 463 hard-coded `topic = "LibreFang"` for every stream reply, losing the inbound topic context so the bot's response always landed in a "LibreFang" topic regardless of which topic triggered it (a separate `send_in_thread` path at line 471 did pass `thread_id` through, but the kernel only reached it when the trigger explicitly carried a thread id; the common case dropped the topic). The sidecar surfaces the inbound `message.subject` as `thread_id` on inbound and `on_send` routes every stream send through that topic so the reply lands in the originating topic. Mirrors reddit / rocketchat / nextcloud / webex; (2) **429 `Retry-After` honoured on every REST path** — the Rust adapter had no 429 handling, only the generic 1 s → 60 s exponential backoff at zulip.rs:228-313; a server-side rate-limit either burned the poll budget or caused the send to return an Err. The sidecar's `_http` exposes response headers and `_parse_retry_after` floors at 1 s + caps at `MAX_BACKOFF_SECS` with a `RETRY_AFTER_DEFAULT_SECS = 30.0` fallback; `_validate`, `_register_queue`, `_poll_once`, and `_post_message` all detect 429, sleep the indicated interval, then retry (poll raises so the producer's outer backoff applies; send raises only on a second 429 inside the same call). Same pattern as the merged `fix(channels): honour Retry-After across sidecar polling adapters` #5303; (3) **bounded `message.id` dedupe** — Zulip's `last_event_id` cursor narrows the *event* range server-side, but on queue re-register (`BAD_EVENT_QUEUE_ID`) the bot can re-see a message it already emitted because the new queue starts fresh. The Rust emit at zulip.rs:434 was unconditional. The sidecar dedupes locally on `message.id` with a bounded `SEEN_MESSAGES_MAX = 10 000` / `SEEN_MESSAGES_EVICT = 5 000` cap (same policy as reddit / rocketchat / nextcloud / webex); (4) **self-skip by stable integer `sender_id`** — the Rust adapter compared `sender_email == bot_email` (zulip.rs:357). Email is the bot's outward identifier and rarely rotates, but on realms that change bot ownership the email moves while the integer `user_id` stays — the email-only check breaks. The sidecar prefers `sender_id == own_user_id` (the integer `/users/me` returns) and falls back to `sender_email == own_email` when `sender_id` is absent (parallels the rocketchat #5298 / nextcloud #5301 fix). New env-var knobs (read from `[sidecar_channels.env]`): `ZULIP_SERVER_URL` (replaces `server_url`), `ZULIP_BOT_EMAIL` (replaces `bot_email`), optional `ZULIP_STREAMS` (comma-separated stream names, empty = all subscribed), optional `ZULIP_ACCOUNT_ID` for multi-bot routing. **Operator action required**: an existing `[channels.zulip]` block is no longer recognised — re-declare as a `[[sidecar_channels]]` running `python3 -m librefang.sidecar.adapters.zulip` with `ZULIP_SERVER_URL` + `ZULIP_BOT_EMAIL` (in `[sidecar_channels.env]`) and `ZULIP_API_KEY` (in `~/.librefang/secrets.env`) — see the module's header for the exact config. `ChannelType::Custom("zulip")` (the channel-type token the Rust adapter advertised at zulip.rs:197) is preserved across this migration via `channel_type = "zulip"` on the sidecar entry, so existing routing and `channel_role_mapping` keys that reference `zulip` continue to resolve. Verification: `cd sdk/python && pytest tests/test_zulip_adapter.py` (74 new tests) covers env-var enforcement (server URL trailing-slash strip + scheme validation, whitespace-only api-key still exits 2), comma-separated stream parse, account-id optional, `_split_message` chunking (under-limit, newline-cut, hard-cut, 10000 cap parity), `_split_csv`, `_parse_retry_after` (missing-uses-default, integer/decimal seconds, garbage-falls-back, 1 s floor, `MAX_BACKOFF_SECS` cap), `_auth_headers` Basic-auth shape with optional form Content-Type, `parse_zulip_event` (basic stream message, basic DM falls back to sender email + no thread, slash-command form with/without args, self-skip by stable sender_id even when email rotates, fallback to email when sender_id absent, non-self with different id is NOT skipped, self-skip disabled when both keys missing, stream filter accept/reject/empty-all, account_id injection, non-message event types skipped, empty content skipped, missing sender_full_name → "unknown", string sender_id coerced, malformed event / message dict → None), `_mark_seen` (first-time / repeat suppress / empty-id always fresh / capacity eviction at cap), `_validate` (happy path + 401 raise + missing user_id raise + 429 retry-after), `_register_queue` (basic body shape with event_types JSON literal, with-streams includes `narrow` JSON, 4xx raise, 429 retry with explicit Retry-After, 429 without header falls back to default), `_poll_once` (emit, id-repeat dedupe across two polls, `BAD_EVENT_QUEUE_ID` returns `reregister` signal without raising, other 400 codes raise, 429 sleeps then raises, watermark advances to max event.id in batch, long-poll timeout 70 s passed, non-message events still advance watermark), `_post_message` (stream form shape with topic, direct shape with URL-encoded @, multi-chunk for long bodies, 429 retry-once with explicit Retry-After, double-429 raises, 5xx raises, missing destination rejection), `on_send` wiring (uses `cmd.thread_id` as stream topic — the P1 improvement, falls back to `DEFAULT_STREAM_TOPIC = "LibreFang"` when absent, DM via `@` in platform_id, falls back to `channel_id` when `user` is None, non-text content → placeholder), SCHEMA advertises required fields, `suppress_error_responses = False` (chat-room precedent), `capabilities = ["thread"]`. (@vip)
- **BREAKING: Webex migrated from in-process Rust adapter to sidecar-only** — the in-process `librefang-channels::webex` adapter (`WebexAdapter`, 645 lines: Cisco Mercury WebSocket gateway at `wss://mercury-connection-a.wbx2.com/v1/apps/wx2/registrations` for activity events + `GET /messages/<id>` REST follow-up for the message body + `POST /messages` publish, auth via bot Bearer token) is deleted along with the `[channels.webex]` config schema (`WebexConfig`), the `channel-webex` cargo feature in both `librefang-channels` and `librefang-api` (incl. its membership in `all-channels` / `all-channels-no-email`), the dashboard `ChannelMeta` descriptor + 4 match arms (`is_some` / serialize / `len` / `ser`), the CLI-TUI `ChannelDef`, the kernel `channel_sender` `for_each_channel_field!` entry + `EXPECTED` name-list, and the config-validation env-var hook. `webex` is removed from `crates/librefang-channels/src/channels-allowlist.txt`, so `cargo xtask channel-policy` now permanently rejects any attempt to reintroduce an in-process webex adapter. Behaviour is preserved by the new reference sidecar `librefang.sidecar.adapters.webex` (ships in `librefang-sdk`; source at `sdk/python/librefang/sidecar/adapters/webex.py`, stdlib-only, on the `librefang.sidecar` SDK): same `GET /people/me` startup credential probe to discover the bot's own id + display name, same hard-coded Mercury WSS endpoint with `Authorization: Bearer <token>` on the upgrade request (no device-registration handshake — Cisco's gateway accepts the bare connect), same `data.activity` envelope parsing with verb=="post" filter and actor-id self-skip, same `GET /messages/<id>` REST follow-up to retrieve the full message body, same room-filter behaviour (empty allowlist = all rooms the bot is in), same slash-command routing on `/cmd args` → `Command` (text otherwise), same `roomType == "group"` → `is_group` mapping, same multi-bot `account_id` metadata injection, same 7439-char message chunking (`WEBEX_MSG_LIMIT` parity with the Rust `MAX_MESSAGE_LEN`), same exponential reconnect backoff (1s → 60s). The WebSocket client is the same hand-rolled RFC 6455 reader as the discord / slack / nextcloud sidecars — `select`-gated frame waits, masked-pong replies to server pings, close-frame handling. The Rust adapter also carried a never-wired `register_webhook` helper (`webex.rs:137-168`, marked `#[allow(dead_code)]`) for an HTTP-webhook delivery alternative the channel-bridge never enabled; the sidecar drops it without replacement, since the canonical webhook-delivery path is now the generic `[[sidecar_channels]]` running `librefang.sidecar.adapters.webhook`. Inbound `personDisplayName` (when the `/messages/<id>` body carries it) now drives `user_name` instead of the Rust adapter's unconditional `personEmail` (`webex.rs:431`), so bot logs and dashboard UI surface "Alice" rather than "alice@example.com" — `personEmail` is still preserved in metadata for routing / audit and used as the fallback when `personDisplayName` is absent. **Four improvements on top of the Rust adapter**: (1) **`parentId` outbound threading wired** — the Rust `api_send_message` (`crates/librefang-channels/src/webex.rs` lines 171-201 on the migrating tree) built a body of just `{"roomId", "text"}`, so Webex's `parentId` field (which threads a reply under a parent message in a Space) was never sent; the inbound side dropped the message id entirely (`thread_id: None` at line 438 of the same file), so even when we knew the parent we had nothing to round-trip. The sidecar surfaces the inbound `id` (or the inbound `parentId` when the user themselves was already inside a thread, so the bot threads alongside rather than starting a nested child) as `thread_id`, and `on_send` posts `parentId` populated so threaded replies actually thread — mirrors reddit / rocketchat / nextcloud / mastodon / bluesky; (2) **429 `Retry-After` honoured on both fetch and send** — Webex documents 429 with `Retry-After`, but the Rust adapter had no 429 handling at either `GET /messages/<id>` (`webex.rs:380-398`) or `POST /messages` (`webex.rs:171-201`); a server-side rate-limit either lost the inbound fetch or caused `send()` to return an `Err` and drop the outbound. The sidecar parses `Retry-After` (with a `RETRY_AFTER_DEFAULT_SECS = 30.0` fallback, floor 1 s, cap `MAX_BACKOFF_SECS`), sleeps, and retries once before logging-and-continuing on the second 429 (same fail-open shape as the discord / slack 429-retry pattern, matching `fix(channels): honour Retry-After across sidecar polling adapters` #5303); (3) **Mercury activity-id dedupe** — Mercury can re-deliver an `activity.object.id` on reconnect (the Rust adapter had no dedupe, see the unconditional emit at `webex.rs:459` — the only filters were verb / self / empty-id / allowed-rooms); operators with a flaky network saw the bot react twice to the same message after a transient drop. The sidecar dedupes locally on `activity.object.id` with a bounded `SEEN_MESSAGES_MAX = 10 000` / `SEEN_MESSAGES_EVICT = 5 000` cap (same policy as reddit / rocketchat / nextcloud); (4) **explicit HTTP timeouts on every `urlopen`** — `urllib.request.urlopen` has no default timeout, so a hung Webex API would hang the producer thread forever; every `_http` call now passes `timeout=SEND_TIMEOUT_SECS` (15 s) so a misbehaving REST endpoint trips an explicit error and loops the reconnect backoff instead of hanging. New env-var knobs (read from `[sidecar_channels.env]`): `WEBEX_ALLOWED_ROOMS` (comma-separated room IDs, empty = allow all), optional `WEBEX_ACCOUNT_ID` for multi-bot routing. **Operator action required**: an existing `[channels.webex]` block is no longer recognised — re-declare as a `[[sidecar_channels]]` running `python3 -m librefang.sidecar.adapters.webex` with `WEBEX_BOT_TOKEN` (in `~/.librefang/secrets.env`) and any of the optional knobs above (in `[sidecar_channels.env]`). `ChannelType::Custom("webex")` (the channel-type token the Rust adapter advertised at `webex.rs:258`) is preserved across this migration via `channel_type = "webex"` on the sidecar entry, so existing routing and `channel_role_mapping` keys that reference `webex` continue to resolve. Verification: `cd sdk/python && pytest tests/test_webex_adapter.py` (78 new tests) covers env-var enforcement (whitespace-only token still exits 2, allowed-rooms CSV with whitespace, account-id passthrough, api_base / ws_url overrides), `_split_message` chunking (under-limit, newline-cut, hard-cut, 7439 cap parity), `_split_csv`, `_parse_retry_after` (missing-uses-default, integer/decimal seconds, garbage-falls-back, 1 s floor, `MAX_BACKOFF_SECS` cap), `parse_webex_message` (basic text, non-post verb skip, self-actor skip with own_bot_id-None bypass, missing object id, empty text, room-filter accept/reject/empty-all, command form with/without args, DM roomType=direct → not group, account_id injection, thread-reply uses inbound `parentId` and top-level uses own id, roomType=missing defaults to group, missing personEmail/personId fallbacks, full-msg roomId fallback to activity.target.id, malformed activity / msg), `_mark_seen` dedupe (first-time / repeat / empty-id / capacity eviction at cap), `_validate_bot_token` (happy path + 4 fail cases + 429 retry-after on the auth probe), `_fetch_message` (happy path with URL-quoting of special chars, non-2xx returns None, 429 retries with explicit Retry-After then default, double-429 returns None), `_post_message` (basic shape, `parentId` round-trip, chunks preserve `parentId`, 429 retry with explicit + default, double-429 fail-open continues with remaining chunks, 5xx fail-open, explicit timeout passed on every `urlopen`), `_handle_envelope` end-to-end (full flow with REST follow-up, self-skip without REST call, non-post verb skip without REST call, room-filter skip without REST call, dedupes repeated activity ids so only one REST fetch happens per id, account_id injection, fetch failure drops without crash, malformed payloads), `on_send` wiring (uses `channel_id`, falls back to `user.platform_id`, round-trips `thread_id` as `parentId`, non-text content placeholder, drops on empty room id), and SCHEMA / capabilities. (@vip)
- **BREAKING: Nextcloud Talk migrated from in-process Rust adapter to sidecar-only** — the in-process `librefang-channels::nextcloud` adapter (`NextcloudAdapter`, 640 lines: 3 s per-room polling of `GET /ocs/v2.php/apps/spreed/api/v1/chat/<token>?lookIntoFuture=1` with `lastKnownMessageId=<watermark>` cursor + form-`POST chat/<token>` publish, auth via Bearer app-password plus the mandatory `OCS-APIRequest: true` header) is deleted along with the `[channels.nextcloud]` config schema (`NextcloudConfig`), the `channel-nextcloud` cargo feature in both `librefang-channels` and `librefang-api` (incl. its membership in `all-channels` / `all-channels-no-email`), the dashboard `ChannelMeta` descriptor + 4 match arms (`is_some` / serialize / `len` / `ser`), the CLI-TUI `ChannelDef`, the kernel `channel_sender` `for_each_channel_field!` entry + `EXPECTED` name-list, and the config-validation env-var hook. `nextcloud` is removed from `crates/librefang-channels/src/channels-allowlist.txt`, so `cargo xtask channel-policy` now permanently rejects any attempt to reintroduce an in-process nextcloud adapter. Behaviour is preserved by the new reference sidecar `librefang.sidecar.adapters.nextcloud` (ships in `librefang-sdk`; source at `sdk/python/librefang/sidecar/adapters/nextcloud.py`, stdlib-only, on the `librefang.sidecar` SDK): same `GET /ocs/v2.php/cloud/user` startup credential probe, same per-room `chat/<token>?lookIntoFuture=1` polling at the same 3 s default interval, same empty-allowlist → `apps/spreed/api/v4/room` auto-discovery of joined rooms, same Bearer + `OCS-APIRequest: true` headers, same slash-command routing on `/cmd args` → `Command` (text otherwise), same multi-bot `account_id` metadata injection, same 32000-char message chunking, same per-room transport-error isolation, same 304-as-no-op handling of Talk's long-poll-expired response. **Three improvements on top of the Rust adapter**: (1) **outbound threading is now actually wired** — the Rust adapter's `api_send_message` (`crates/librefang-channels/src/nextcloud.rs` lines 130-160 on main) called `POST /chat/<token>` with a body of just `{"message": ...}`, so Talk's `replyTo` form parameter (which links a reply to a parent message id) was never sent and chunked / threaded replies always landed at the room root regardless of inbound context; the sidecar surfaces the inbound `id` (or the inbound `parentMessage.id` when the user themselves was already inside a thread, so the bot threads alongside rather than starting a child) as `thread_id`, and `on_send` posts `replyTo` populated so the reply threads correctly — mirrors reddit / bluesky / mastodon / rocketchat; (2) **self-skip on `(actorType, actorId)` rather than `actorId` alone** — the Rust adapter compared `msg["actorId"] == own_user` (nextcloud.rs:338 on main) without inspecting `actorType`, so a Talk guest / `federated_users` actor whose id happens to equal the bot's user id would silently spoof self-skip and the bot would ignore the guest's messages; the sidecar requires `actorType == "users"` AND `actorId == own_user_id`, eliminating the ambiguity (parallels the rocketchat #5298 fix); (3) **dedupe set on `id`** — the Rust adapter advanced `last_known_ids` (nextcloud.rs:347-354 on main) but only relied on the server-side `lastKnownMessageId` cursor for deduplication; under retry / re-poll boundaries Talk can resend the same id (e.g. when the previous fetch's response was lost but the newest-id update wasn't persisted client-side), re-emitting messages. The sidecar keeps the watermark for the API query but additionally dedupes locally on `id` with a bounded `SEEN_MESSAGES_MAX=10000` / `SEEN_MESSAGES_EVICT=5000` cap (same policy as reddit / rocketchat). Additionally, the sidecar marks `suppress_error_responses = true` (Talk rooms are typically multi-participant, same rationale as mastodon / bluesky / reddit / rocketchat). New env-var knobs: `NEXTCLOUD_SERVER_URL` (replaces `server_url`), optional `NEXTCLOUD_ROOMS` (comma-separated room-token list, empty = auto-discover joined rooms via the spreed v4 room endpoint), optional `NEXTCLOUD_ACCOUNT_ID` for multi-bot routing, optional `NEXTCLOUD_POLL_INTERVAL_SECS` (default 3, floor 1). **Operator action required**: an existing `[channels.nextcloud]` block is no longer recognised — re-declare as a `[[sidecar_channels]]` running `python3 -m librefang.sidecar.adapters.nextcloud` with env var `NEXTCLOUD_SERVER_URL` (in `[sidecar_channels.env]`) and `NEXTCLOUD_TOKEN` (in `~/.librefang/secrets.env`) — see the module's header for the exact config. Verification: `cd sdk/python && pytest tests/test_nextcloud_adapter.py` (58 new tests) covers env-var enforcement, server URL normalization (trailing-slash strip, scheme validation), poll-interval clamping, `_split_message` chunking (under-limit, newline-cut, hard-cut, 32000 cap parity), `_verify_credentials` (OCS + Bearer header shape, 401, missing-id fallback), `apps/spreed/api/v4/room` discovery, `_parse_message` (basic text, thread-reply uses inbound `parentMessage.id`, string-vs-int parent id, self-skip on `(actorType,actorId)`, guest-with-matching-id is NOT self, self-skip disabled when own_user_id empty, system-message skip, empty-body skip, command form, no-args command, `referenceId` in metadata, malformed input, non-integer id graceful handling), `_poll_once` (emit + watermark advance, dedupe across id repeats, self-skip still marks seen, `account_id` injection, 401 raises, 304 no-op, per-room transport-error isolation, 500 logged-and-skipped, URL + auth-header shape with `lookIntoFuture=1` / `limit=100` / `lastKnownMessageId=<wm>` / `format=json`), dedupe-set capacity eviction at cap and idempotent mark + empty-id ignore, `_post_message` (basic form-encoded shape with `message`, `replyTo` on thread, multi-chunk preserves `replyTo`, missing-room rejection, non-2xx surfaced), `on_send` wiring (uses `cmd.user.platform_id` as room token, threads via `thread_id`, falls back to `cmd.channel_id`, non-text content → placeholder). (@vip)
- **REGRESSION (acknowledged, matches the telegram / discord precedent): live Slack workspace-role RBAC is unavailable in the sidecar.** The Rust `SlackAdapter` implemented `ChannelRoleQuery::lookup_role` by calling `users.info` on every message and collapsing `is_primary_owner` / `is_owner` / `is_admin` / `is_restricted` / `is_ultra_restricted` into one of `owner` / `admin` / `guest` / `member`, which the kernel then translated through `[channel_role_mapping.slack]` into a LibreFang `UserRole`. `ChannelRoleQuery` is a Rust trait the sidecar process cannot implement, so post-migration `role_query.is_none()` for Slack, the kernel's `resolve_role_for_sender` falls through to the default-deny branch, and `[channel_role_mapping.slack]` (static config) is never consulted. Operators who relied on automatic workspace-role-to-LibreFang-role mapping see every Slack user fall back to `Viewer` unless explicitly added under `[users]`. Same situation telegram has been in since #5241 and discord since #5299; flagged here so operators aren't surprised by the silent demotion. (Workaround: enumerate authorised operators under `[users]` with `channel_bindings = { slack = ["<slack_user_id>"] }` and an explicit `role`.) The `parse_users_info` precedence parser is preserved in `sdk/python/librefang/sidecar/adapters/slack.py` so a future sidecar-protocol query/response pair can reuse it without re-deriving the logic. (@houko)
- **BREAKING: Slack migrated from in-process Rust adapter to sidecar-only** — the in-process `librefang-channels::slack` adapter (`SlackAdapter`, 1 890 lines: Socket Mode WebSocket via `apps.connections.open` + Web API via `chat.postMessage` / `reactions.add` / `users.info`) is deleted along with the `[channels.slack]` config schema (`SlackConfig`), the `channel-slack` cargo feature (incl. its membership in `all-channels` / `all-channels-no-email` / `core-channels` / `mini`), the dashboard `ChannelMeta` descriptor + 5 match arms (`is_some` / serialize / `len` / `ser` / `is_channel_configured`), the CLI `librefang channel setup slack` wizard arm + `channel list` row, the kernel `channel_sender` `for_each_channel_field!` entry + `EXPECTED` name-list, the config-validation env-var hook, and the `routes/channels.rs` live-test `slack` branch that POSTed to `https://slack.com/api/chat.postMessage`. `slack` is removed from `crates/librefang-channels/src/channels-allowlist.txt`, so `cargo xtask channel-policy` now permanently rejects any attempt to reintroduce an in-process slack adapter. The canonical `deny_unknown_fields` rustdoc anchor (#5130) moves to `WhatsAppConfig`. Behaviour is preserved by the new reference sidecar `librefang.sidecar.adapters.slack` (ships in `librefang-sdk`; source at `sdk/python/librefang/sidecar/adapters/slack.py`, stdlib-only, on the `librefang.sidecar` SDK): same `POST /api/auth.test` startup probe to discover `bot_user_id`, same `POST /api/apps.connections.open` to mint a Socket Mode WSS URL, same envelope-id ACK loop for `events_api` / `interactive`, same `message` + `app_mention` event handling with `message_changed` subtype extraction and all-other-subtype skip, same self-skip on `bot_id` presence OR `user == bot_user_id`, same `allowed_channels` filter with DM exemption (channels starting with `D`), same slash-command routing on `/cmd args` → `Command`, same `thread_ts` capture as `thread_id`, same DM detection via channel-id prefix, same `block_actions` interactive payload → `ButtonCallback` content with `action_id` / `trigger_id` / `block_action` metadata, same `chat.postMessage` send with optional `thread_ts` + `unfurl_links` + Block Kit blocks, same 3 000-char chunking (`SLACK_MSG_LIMIT` parity), same `eyes` reaction on receive flipped to `white_check_mark` on send-complete (opt-out via `SLACK_REACTIONS=false`), same `force_flat_replies` knob to post replies as top-level channel messages instead of threads, same `sender_user_id` metadata key (`SENDER_USER_ID_KEY` parity), same account-id injection for multi-bot routing. The Block Kit `_build_block_kit` builder mirrors the Rust adapter's section + actions block layout (one section for the text, one actions block per row of buttons, `primary` / `danger` style validation, `url` button passthrough, malformed-row skip). The WebSocket client is the same hand-rolled RFC 6455 reader as the discord sidecar (#5299) — `select`-gated frame waits, masked-pong replies to server pings, close-frame handling. **One improvement on top of the Rust adapter**: **pending-reaction map is bounded** at `MAX_PENDING_REACTIONS = 2 000` entries with oldest-eviction; the Rust adapter used an unbounded `RwLock<HashMap>` so a flood of inbound messages followed by a hang in the agent loop would grow the map without bound (a small but real memory-leak surface that the eviction now closes). **Two regressions to call out alongside the parity claim** (matching the discord precedent #5299): (a) live Slack workspace-role RBAC is gone (see the dedicated regression entry above — `ChannelRoleQuery::lookup_role` was Rust-trait-bound and cannot cross the sidecar boundary; `[channel_role_mapping.slack]` is no longer consulted because `role_query` is now `None` for Slack); (b) the per-`[channels.slack] proxy = "..."` override (#4795) is no longer wired through — the sidecar honours standard `HTTP_PROXY` / `HTTPS_PROXY` / `ALL_PROXY` env vars via Python stdlib but the per-channel override key has no `SLACK_PROXY_URL` env var yet (filed as a follow-up; operators with a per-channel proxy today should fall back to the process-wide env vars). New env-var knobs (read from `[sidecar_channels.env]`): `SLACK_ALLOWED_CHANNELS` (comma-separated channel IDs, empty = allow all), `SLACK_UNFURL_LINKS` (tri-state — unset = use Slack default, `true` / `false` to force), `SLACK_FORCE_FLAT_REPLIES` (default false), `SLACK_REACTIONS` (default true), optional `SLACK_ACCOUNT_ID` for multi-bot routing. **Operator action required**: an existing `[channels.slack]` block is no longer recognised — re-declare as a `[[sidecar_channels]]` running `python3 -m librefang.sidecar.adapters.slack` with `SLACK_APP_TOKEN` and `SLACK_BOT_TOKEN` (in `~/.librefang/secrets.env`) and any of the optional knobs above (in `[sidecar_channels.env]`). The OpenClaw migrator (`librefang-migrate::openclaw`) now emits a `SkippedItem` with a sidecar-redirect message instead of writing `[channels.slack]` to the migrated config (mirrors how telegram + discord were handled). `ChannelType::Slack` enum variant stays — it is used by the router / bridge for routing logic and is preserved across this migration the same way `ChannelType::Telegram` and `ChannelType::Discord` were preserved. Verification: `cd sdk/python && pytest` (488 tests, 72 new for slack) covers env handling (xapp + xoxb required, tri-state unfurl_links, force-flat-replies + reactions defaults, allowed-channels splitting), `_split_message` chunking, `_split_csv` / `_bool_env`, `parse_users_info` precedence (owner > admin > guest > member; `user_not_found` returns silent `None`), `parse_slack_event` (basic text, app_mention sets was_mentioned, self-skip via bot_id + user_id, message_changed subtype extraction, drops other subtypes, slash-command routing, empty-text drop, allowed-channels filter with DM exemption, thread_ts capture, account_id injection), `parse_slack_block_action` (basic shape with message_text / action_id / trigger_id / block_action metadata, drops non-block_actions type, drops self-user, drops empty action value, respects allowed_channels), `_validate_bot_token` (auth.test happy path, rejection on `ok: false`, missing user_id surface), `_fetch_socket_mode_url` (apps.connections.open shape using app-level token, rejection on `ok: false`, non-wss URL rejection), `_post_message` (channel + text + thread_ts + unfurl_links + Block Kit blocks shape, 3000-char chunking, fail-open on `ok: false`, fail-open on 5xx), `_build_block_kit` (section-first, primary/danger style validation, url passthrough, malformed-row skip), reactions (`already_reacted` / `no_reaction` benign-silence, disabled-noop, pending-reactions bounded cap, eyes → white_check_mark flip on finalize), `_handle_envelope` state machine (events_api ACK + emit + eyes reaction, interactive ACK + ButtonCallback emit, hello no-op, disconnect raises, skipped events still ACK but no emit), `on_send` routing (text uses channel_id, thread_ts wiring, force_flat_replies drops thread, Interactive uses Block Kit, unsupported content placeholder, user.platform_id fallback, drops on empty channel_id). Also `cargo test -p librefang-channels -p librefang-types -p librefang-migrate -p librefang-kernel -p librefang-api --features 'librefang-api/all-channels'` runs clean (lib + integration) and `cargo clippy --workspace --all-targets --features 'librefang-api/all-channels' -- -D warnings` is zero-warning. (@vip)
- **BREAKING: Rocket.Chat migrated from in-process Rust adapter to sidecar-only** — the in-process `librefang-channels::rocketchat` adapter (`RocketChatAdapter`, 585 lines: 2 s per-room polling of `GET /api/v1/channels.history` with RFC3339 `oldest=<watermark>` cursor + `chat.sendMessage` publish, auth via `X-Auth-Token` / `X-User-Id` personal-access-token headers) is deleted along with the `[channels.rocketchat]` config schema (`RocketChatConfig`), the `channel-rocketchat` cargo feature in both `librefang-channels` and `librefang-api` (incl. its membership in `all-channels` / `all-channels-no-email`), the dashboard `ChannelMeta` descriptor + 4 match arms (`is_some` / serialize / `len` / `ser`), the CLI-TUI `ChannelDef`, the kernel `channel_sender` `for_each_channel_field!` entry + `EXPECTED` name-list, and the config-validation env-var hook. `rocketchat` is removed from `crates/librefang-channels/src/channels-allowlist.txt`, so `cargo xtask channel-policy` now permanently rejects any attempt to reintroduce an in-process rocketchat adapter. Behaviour is preserved by the new reference sidecar `librefang.sidecar.adapters.rocketchat` (ships in `librefang-sdk`; source at `sdk/python/librefang/sidecar/adapters/rocketchat.py`, stdlib-only, on the `librefang.sidecar` SDK): same `GET /api/v1/me` startup credential probe, same per-room `channels.history` polling at the same 2 s default interval, same empty-allowlist → `channels.list.joined` auto-discovery, same `X-Auth-Token` / `X-User-Id` auth headers, same slash-command routing on `/cmd args` → `Command` (text otherwise), same multi-bot `account_id` metadata injection, same 4096-char message chunking, same per-room transport-error isolation. **Three improvements on top of the Rust adapter**: (1) **outbound threading is now actually wired** — the Rust adapter captured the inbound `tmid` on receive but `send()` always called `chat.sendMessage` without forwarding it, so threaded replies broke and the bot's responses landed at the room root regardless of context; the sidecar surfaces the inbound `_id` (or the inbound `tmid` when the user themselves was already inside a thread, so the bot threads alongside rather than starting a child) as `thread_id`, and `on_send` calls `POST /api/v1/chat.postMessage` with `tmid` populated so the reply threads correctly — mirrors reddit / bluesky / mastodon (see `crates/librefang-channels/src/rocketchat.rs` lines 297, 304-340 in main for the captured-but-unused `tmid` field, and `sdk/python/librefang/sidecar/adapters/rocketchat.py` `_parse_message` / `_post_message` for the round-trip); (2) **dedupe set on `_id`** — the Rust adapter advanced its per-room `last_timestamps` cursor on RFC3339 string comparison and re-fetched `oldest=<watermark>`, which with `count=50` and same-`ts` repeats either re-emitted duplicates or silently dropped messages that shared a timestamp boundary (see `crates/librefang-channels/src/rocketchat.rs` lines 280-302 in main); the sidecar keeps the watermark for the API query but additionally dedupes on `msg._id` with a bounded `SEEN_MESSAGES_MAX=10000` / `SEEN_MESSAGES_EVICT=5000` cap (same policy as reddit); (3) **self-skip by stable user id** — the Rust adapter compared `u.username == own_username` (`crates/librefang-channels/src/rocketchat.rs` line 285 in main), which silently breaks when the bot's display name rotates; the sidecar compares `u._id == ROCKETCHAT_USER_ID` (the stable internal id the operator already configured) and falls back to username only when the inbound shape omits `u._id`. Additionally, the sidecar marks `suppress_error_responses = true` (Rocket.Chat messages are public to a room, same rationale as mastodon / bluesky / reddit). New env-var knobs: `ROCKETCHAT_SERVER_URL` (replaces `server_url`), `ROCKETCHAT_USER_ID` (replaces `user_id`), optional `ROCKETCHAT_CHANNELS` (comma-separated room id list, empty = auto-discover joined channels), optional `ROCKETCHAT_ACCOUNT_ID` for multi-bot routing, optional `ROCKETCHAT_POLL_INTERVAL_SECS` (default 2, floor 1). **Operator action required**: an existing `[channels.rocketchat]` block is no longer recognised — re-declare as a `[[sidecar_channels]]` running `python3 -m librefang.sidecar.adapters.rocketchat` with env vars `ROCKETCHAT_SERVER_URL`, `ROCKETCHAT_USER_ID` (in `[sidecar_channels.env]`) and `ROCKETCHAT_TOKEN` (in `~/.librefang/secrets.env`) — see the module's header for the exact config. Verification: `cd sdk/python && pytest tests/test_rocketchat_adapter.py` (54 new tests) covers env-var enforcement, server URL normalization (trailing-slash strip, scheme validation), poll-interval clamping, `_split_message` chunking (under-limit, newline-cut, hard-cut, 4096 cap parity), `_verify_credentials` (auth header shape, 401, missing-username fallback), `channels.list.joined` discovery, `_parse_message` (basic text, thread-reply uses inbound `tmid`, self-skip by user id, username-fallback when `u._id` missing, empty-body skip, command form, no-args command, malformed input), `_poll_once` (emit + watermark advance, dedupe across same-`ts` repeats, self-skip still marks seen, `account_id` injection, 401 raises, per-room transport-error isolation, 500 logged-and-skipped, URL + auth-header shape), dedupe-set capacity eviction at cap and idempotent mark + empty-id ignore, `_post_message` (basic shape with `roomId` + `text`, `tmid` on thread, multi-chunk preserves `tmid`, missing-room rejection, non-2xx surfaced, soft-error `success=false` logged), `on_send` wiring (uses `cmd.user.platform_id` as room, threads via `thread_id`, falls back to `cmd.channel_id`, non-text content → placeholder). (@vip)
- **BREAKING: Twitch migrated from in-process Rust adapter to sidecar-only** — the in-process `librefang-channels::twitch` adapter (`TwitchAdapter`, 535 lines: plaintext TCP to `irc.chat.twitch.tv:6667`, raw IRC handshake `PASS oauth:<token>` / `NICK` / `JOIN #channel`, hand-rolled `parse_privmsg`, per-send fresh-TCP-connect dance) is deleted along with the `[channels.twitch]` config schema (`TwitchConfig`), the `channel-twitch` cargo feature (incl. its membership in `all-channels` / `all-channels-no-email`), the dashboard `ChannelMeta` descriptor + 4 match arms (`is_some` / serialize / `len` / `ser`), the CLI-TUI `ChannelDef`, the kernel `channel_sender` `for_each_channel_field!` entry + `EXPECTED` name-list, and the config-validation env-var hook. `twitch` is removed from `crates/librefang-channels/src/channels-allowlist.txt`, so `cargo xtask channel-policy` now permanently rejects any attempt to reintroduce an in-process twitch adapter. Behaviour is preserved by the new reference sidecar `librefang.sidecar.adapters.twitch` (ships in `librefang-sdk`; source at `sdk/python/librefang/sidecar/adapters/twitch.py`, stdlib-only — `socket` + `ssl` + `threading`, no third-party deps): same OAuth `PASS` / `NICK` handshake (with the `oauth:` prefix auto-added when absent), same `JOIN #<channel>` for each configured channel, same PRIVMSG → ChannelMessage path with self-skip on case-insensitive nick match, same `/cmd` / `!cmd` routing to `Content::Command`, same PING → PONG keepalive, same `MAX_MESSAGE_LEN = 500` chunking, same exponential reconnect backoff (1s → 60s), same `account_id` multi-bot routing via `TWITCH_ACCOUNT_ID`. **Three improvements on top of the Rust adapter**: (1) **TLS by default** — the sidecar connects to `irc.chat.twitch.tv:6697` and wraps the socket with `ssl.create_default_context()`; the Rust adapter used plaintext `6667` (hard-coded as `TWITCH_IRC_PORT` in `crates/librefang-channels/src/twitch.rs:24`) and sent the OAuth token in cleartext on every connect, a credential-leak-on-wire that operators get fixed automatically on upgrade. Plaintext is reachable only via `TWITCH_PLAINTEXT=1` for local mock listeners (tests use this); (2) **per-message reply threading via IRCv3 tags** — the sidecar issues `CAP REQ :twitch.tv/tags twitch.tv/commands` after auth, parses the `@…` tag block on every PRIVMSG, surfaces `@id` as `thread_id` so the daemon round-trips it back via `cmd.thread_id`, and attaches `@reply-parent-msg-id=<id>` on outbound PRIVMSG so Twitch renders the bot's response threaded under the source message. The Rust adapter never requested any IRCv3 capability and discarded any tag block, so chunked replies arrived as a flat sequence of unthreaded messages (matches the bluesky #5277 improvement); (3) **ban-avoidance token bucket on outbound** — Twitch's anti-spam logic drops the bot from chat above 20 msgs / 30 s for a non-mod account (100 / 30 s for a mod). The Rust adapter shipped zero throttling — every PRIVMSG hit the wire immediately, so a chatty agent in a busy channel would be silently dropped. The sidecar gates every outbound chunk through an in-process token bucket (defaults `20 / 30 s`, override via `TWITCH_RATE_LIMIT_MSGS` / `TWITCH_RATE_LIMIT_SECS`). New env-var knobs: `TWITCH_NICK`, `TWITCH_CHANNELS` (comma-separated, no `#`), optional `TWITCH_ACCOUNT_ID` for multi-bot routing, optional `TWITCH_RATE_LIMIT_MSGS` / `TWITCH_RATE_LIMIT_SECS` to tune the bucket, optional `TWITCH_PLAINTEXT` / `TWITCH_HOST` / `TWITCH_PORT` test escape hatches. **Operator action required**: an existing `[channels.twitch]` block is no longer recognised — re-declare as a `[[sidecar_channels]]` running `python3 -m librefang.sidecar.adapters.twitch` with env vars `TWITCH_NICK`, `TWITCH_CHANNELS` (config table) and `TWITCH_OAUTH_TOKEN` (`~/.librefang/secrets.env`) — see the module's header for the exact config. Verification: `cd sdk/python && python -m pytest tests/test_twitch_adapter.py` (68 new tests) covers env-var enforcement, channel name normalization (`#`/whitespace/case), `_split_message` chunking, IRC tag-block parsing with IRCv3 escapes, IRC line parsing (PRIVMSG with/without tags, PING, CAP ACK, ERROR-only), token-bucket starts-full-and-drains/blocks-when-empty/capacity-floor, slash- and bang-command routing, self-skip case-insensitive, dedupe by `@id` tag with capped eviction, account_id metadata injection, reply-parent metadata round-trip, PRIVMSG output shape (plain + threaded + chunked + channel-normalised), `_pass_string` auto-prefix, PING→PONG response, end-to-end `_connect()` against a local TCP listener asserting `CAP REQ` precedes `PASS` precedes `NICK` precedes ordered `JOIN`s (improvement #2), supervisor backoff on connect failure, on_send channel fallback (`channel_id` then `user.platform_id`) and unsupported-content placeholder, shutdown idempotence, `--describe` schema. (@vip)
- **REGRESSION (acknowledged, matches the telegram precedent #5241): live Discord-guild-role RBAC is unavailable in the sidecar.** The Rust `DiscordAdapter` implemented `ChannelRoleQuery::lookup_role` (Discord channel ID → guild ID → guild member roles → translate via `[channel_role_mapping.discord]`), and the kernel's `resolve_role_for_sender` invoked it on every message so a user's live Discord guild roles could promote them above the default-deny `Viewer`. `ChannelRoleQuery` is a Rust trait the sidecar process cannot implement, so post-migration `role_query.is_none()` for Discord, the kernel falls through to the default-deny branch, and `[channel_role_mapping.discord]` (static config) is never consulted. Operators who relied on automatic guild-role-to-LibreFang-role mapping see every Discord user fall back to `Viewer` unless explicitly added under `[users]`. Same situation telegram has been in since #5241; flagged here so operators aren't surprised by the silent demotion. (Workaround: enumerate authorised operators under `[users]` with `channel_bindings = { discord = ["<discord_user_id>"] }` and an explicit `role`.) Re-introducing live role lookup for sidecar adapters is a separate roadmap item — it needs a sidecar-protocol query/response pair the kernel can drive over stdio. (@houko)
- **BREAKING: Discord migrated from in-process Rust adapter to sidecar-only** — the in-process `librefang-channels::discord` adapter (`DiscordAdapter`, 1 747 lines: Discord Gateway WebSocket v10 + REST API v10) is deleted along with the `[channels.discord]` config schema (`DiscordConfig`), the `channel-discord` cargo feature (incl. its membership in `all-channels` / `all-channels-no-email` / `core-channels` / `mini`), the dashboard `ChannelMeta` descriptor + 5 match arms (`is_some` / serialize / `len` / `ser` / `is_channel_configured`), the CLI `librefang channel setup discord` wizard arm + `channel list` row, the kernel `channel_sender` `for_each_channel_field!` entry + `EXPECTED` name-list, the config-validation env-var hook, and the `routes/channels.rs` live-test `discord` branch that POSTed to `https://discord.com/api/v10/channels/{id}/messages`. `discord` is removed from `crates/librefang-channels/src/channels-allowlist.txt`, so `cargo xtask channel-policy` now permanently rejects any attempt to reintroduce an in-process discord adapter. Behaviour is preserved by the new reference sidecar `librefang.sidecar.adapters.discord` (ships in `librefang-sdk`; source at `sdk/python/librefang/sidecar/adapters/discord.py`, stdlib-only, on the `librefang.sidecar` SDK): same `GET /gateway/bot` URL discovery + WSS connect with `?v=10&encoding=json`, same opcode handling (HELLO/IDENTIFY/RESUME/HEARTBEAT/HEARTBEAT_ACK/RECONNECT/INVALID_SESSION/DISPATCH), same READY-driven `(bot_user_id, session_id, resume_gateway_url)` capture, same MESSAGE_CREATE / MESSAGE_UPDATE → `message` event mapping with self-skip via `bot_user_id`, `ignore_bots` filter, `allowed_users` / `allowed_guilds` whitelists, attachment-takes-priority-over-slash-command content extraction (Image/Video/Voice/File by MIME prefix, with audio/file warn-and-drop on companion text matching the Rust adapter), discriminator-aware display name (`username` for new-style or `username#discriminator` for legacy users), mention detection via `mentions[]` array + `<@bot_id>` / `<@!bot_id>` content tags + case-insensitive `mention_patterns`, `is_group = guild_id.is_some()`, `was_mentioned` metadata flag, `POST /channels/{id}/messages` with 2 000-UTF-16-unit chunking, `POST /channels/{id}/typing` for typing indicators, account-id injection into message metadata for multi-bot routing. The WebSocket client is a hand-rolled RFC 6455 reader on `socket` + `ssl` (no third-party WS lib) — `select`-gated frame waits keep mid-frame reads from racing with heartbeat ticks, server pings get a masked pong reply, and known-fatal close codes (4004 auth, 4013 invalid intents, 4014 disallowed intents) raise rather than reconnect so the supervisor's circuit-breaker stops a hard config error instead of looping. **Two improvements on top of the Rust adapter**: (1) **periodic client-side heartbeats**. The Rust adapter captured `heartbeat_interval` from HELLO but never spawned a heartbeat task — connections silently dropped after ~45 s with `code=4000` and re-IDENTIFY'd, losing the session every minute. The sidecar runs proper periodic heartbeats (with the RFC-mandated random jitter on the first beat) so sessions actually survive long-running idle periods, which then makes RESUME after a transient disconnect work for the first time; (2) **429 retry-with-`Retry-After`**. The Rust adapter's `api_send_message` warned on 429 and returned `Ok(())` (fail-open silent message loss); the sidecar honours `Retry-After` and retries once before logging-and-continuing on the second 429 (same fail-open behaviour for the unrecoverable case, but the recoverable case now actually delivers). **Two regressions to call out alongside the parity claim**: (a) live Discord-guild-role RBAC is gone (see the dedicated regression entry above — `ChannelRoleQuery::lookup_role` was Rust-trait-bound and cannot cross the sidecar boundary; `[channel_role_mapping.discord]` is no longer consulted because `role_query` is now `None` for Discord); (b) the per-`[channels.discord] proxy = "..."` override (#4795) is no longer wired through — the sidecar honours standard `HTTP_PROXY` / `HTTPS_PROXY` / `ALL_PROXY` env vars via Python stdlib (`urllib.request.ProxyHandler` default) but the per-channel override key has no `DISCORD_PROXY_URL` env var yet (filed as a follow-up; operators with a per-channel proxy today should fall back to the process-wide env vars). New env-var knobs (read from `[sidecar_channels.env]`): `DISCORD_ALLOWED_GUILDS` (comma-separated guild IDs, empty = allow all), `DISCORD_ALLOWED_USERS` (comma-separated user IDs), `DISCORD_INTENTS` (default 37376 = GUILD_MESSAGES | DIRECT_MESSAGES | MESSAGE_CONTENT), `DISCORD_IGNORE_BOTS` (default `true`), `DISCORD_MENTION_PATTERNS` (comma-separated case-insensitive substrings), optional `DISCORD_ACCOUNT_ID` for multi-bot routing. **Operator action required**: an existing `[channels.discord]` block is no longer recognised — re-declare as a `[[sidecar_channels]]` running `python3 -m librefang.sidecar.adapters.discord` with env var `DISCORD_BOT_TOKEN` (in `~/.librefang/secrets.env`) and any of the optional knobs above (in `[sidecar_channels.env]`). The OpenClaw migrator (`librefang-migrate::openclaw`) now emits a `SkippedItem` with a sidecar-redirect message instead of writing `[channels.discord]` to the migrated config (mirrors how telegram migration handled the same case). `ChannelType::Discord` enum variant stays — it is used by the router / bridge for routing logic and is preserved across this migration the same way `ChannelType::Telegram` was preserved in #5241. Verification: `cd sdk/python && pytest` (293 tests, 68 new for discord) covers env handling, `_split_to_utf16_chunks` (ASCII / emoji surrogate-pair / exact boundary), `_split_csv` / `_parse_retry_after`, `parse_attachment` (image with caption / video / audio drops companion text / file fallback / missing-URL fallback / empty list), `parse_message_create` (self-skip via bot_user_id, `ignore_bots` filter with self-skip still firing when `ignore_bots=false`, `allowed_users` / `allowed_guilds` filters, slash command with/without args, attachment-takes-priority-over-command, mention via array / content tag / custom pattern, discriminator legacy format, DM not-group, account_id injection), `_handle_payload` state machine (READY captures session, INVALID_SESSION non-resumable clears state vs resumable preserves it, RECONNECT raises, MESSAGE_CREATE / MESSAGE_UPDATE emit, server-initiated heartbeat responds with `last_seq`, fatal close-code 4014 translates to `_FatalGatewayError`), `_fetch_gateway_url` (appends query, surfaces 429 / missing-URL as errors), `_send_message` (POST shape with `Bot` auth, UTF-16 chunking, 429-then-200 retry-once, 429-then-429 fail-open, 5xx fail-open), `on_send` routing (uses `cmd.channel_id`, falls back to `cmd.user.platform_id`, non-text placeholder, drops on empty channel_id), and end-to-end `_run_session` (sends IDENTIFY when no session vs RESUME when session known, scripted HELLO+READY+MESSAGE_CREATE emits exactly one message event with the correct content shape). Also `cargo test -p librefang-channels -p librefang-types -p librefang-migrate -p librefang-kernel -p librefang-api --features 'librefang-api/all-channels'` runs clean (lib + integration) and `cargo clippy --workspace --all-targets --features 'librefang-api/all-channels' -- -D warnings` is zero-warning. (@vip)
- **BREAKING: Reddit migrated from in-process Rust adapter to sidecar-only** — the in-process `librefang-channels::reddit` adapter (`RedditAdapter`, 903 lines: OAuth2 password-grant token cache + per-subreddit 5 s polling of `GET /r/{sub}/comments?limit=25&sort=new` + `POST /api/comment` reply) is deleted along with the `[channels.reddit]` config schema (`RedditConfig`), the `channel-reddit` cargo feature (incl. its membership in `all-channels` / `all-channels-no-email`), the dashboard `ChannelMeta` descriptor + 4 match arms (`is_some` / serialize / `len` / `ser`), the CLI-TUI `ChannelDef`, the kernel `channel_sender` `for_each_channel_field!` entry + `EXPECTED` name-list, and the config-validation env-var hook. `reddit` is removed from `crates/librefang-channels/src/channels-allowlist.txt`, so `cargo xtask channel-policy` now permanently rejects any attempt to reintroduce an in-process reddit adapter. Behaviour is preserved by the new reference sidecar `librefang.sidecar.adapters.reddit` (ships in `librefang-sdk`; source at `sdk/python/librefang/sidecar/adapters/reddit.py`, stdlib-only, on the `librefang.sidecar` SDK): same OAuth2 password-grant token mint with 5 min refresh buffer, same per-subreddit polling at 5 s of `/r/{sub}/comments?limit=25&sort=new`, same `kind == "t1"` filter (posts skipped), same own-/`[deleted]`/`[removed]`-author skip, same `/cmd args` → Command routing, same `POST /api/comment` reply with `api_type=json` and chunks joined by `\n\n---\n\n` (Reddit allows one reply per parent), same dedupe-set cap at 10 000 IDs with oldest-half eviction, same Reddit-required unique `User-Agent` header. **Two improvements on top of the Rust adapter**: (1) **outbound reply target is now correctly wired** — the Rust adapter set `thread_id = subreddit` on inbound and tried to pass `user.platform_id` as the parent fullname to `POST /api/comment`, but `parse_reddit_comment` wrote the author username to `platform_id` (not the fullname Reddit's API needs), so the Rust send-path only ever worked because its unit tests mocked `platform_id = "t1_<id>"` directly — a real bridge call would have 400'd with `thing_id must be a fullname`. The sidecar surfaces the fullname (`t1_<comment_id>`) as `thread_id`, so the daemon round-trips it to `on_send` as `cmd.thread_id` and `_post_comment` uses it directly. This also aligns Reddit's per-comment threading with the Bluesky / Mastodon sidecars (each mention → its own agent session); (2) `suppress_error_responses = true` — Reddit comments are public (same rationale as Mastodon / Bluesky), so internal errors must not echo back as a reply. New env-var knobs: `REDDIT_SUBREDDITS` (comma-separated list of subreddits to monitor, e.g. `rust,programming`), optional `REDDIT_ACCOUNT_ID` for multi-bot routing, optional `REDDIT_USER_AGENT` to override the default `librefang:sidecar (by /u/librefang-bot)` UA per Reddit's API guidelines. **Operator action required**: an existing `[channels.reddit]` block is no longer recognised — re-declare as a `[[sidecar_channels]]` running `python3 -m librefang.sidecar.adapters.reddit` with env vars `REDDIT_CLIENT_ID`, `REDDIT_USERNAME`, `REDDIT_SUBREDDITS` (config table) and `REDDIT_CLIENT_SECRET`, `REDDIT_PASSWORD` (`~/.librefang/secrets.env`) — see the module's header for the exact config. Verification: `cd sdk/python && pytest tests/test_reddit_adapter.py` (40 new tests, 212 total) covers env-var enforcement, subreddit normalization (`r/` prefix + trailing-slash stripping), `_split_message` chunking (under-limit, newline-cut, hard-cut), `_parse_reddit_comment` (basic, self-skip case-insensitive, `[deleted]`/`[removed]` skip, empty-body skip, `kind=t3` post skip, `/cmd args` routing, optional permalink omission, malformed input), token fetch (basic-auth header, password-grant form body, 401/missing-field errors, 300 s refresh buffer math), `_verify_credentials` (own_username discovery, 401 rejection), `_post_comment` (basic shape, separator-joined chunks, missing-fullname rejection, 5xx surfaced, 401 → refresh → retry), `_poll_once` (parsed-emit, dedupe on seen IDs, 401 clears token, account_id injection into metadata, per-subreddit transport-error isolation), `_mark_seen` eviction at cap with deterministic list ordering, `on_send` (thread_id → thing_id round-trip, non-text content fallback to placeholder). (@vip)
- **BREAKING: Bluesky migrated from in-process Rust adapter to sidecar-only** — the in-process `librefang-channels::bluesky` adapter (`BlueskyAdapter`, 580 lines: AT-Protocol `createSession` + `listNotifications` 5 s polling + `createRecord` publish) is deleted along with the `[channels.bluesky]` config schema (`BlueskyConfig`), the `channel-bluesky` cargo feature (incl. its membership in `all-channels` / `all-channels-no-email`), the dashboard `ChannelMeta` descriptor + 5 match arms (`is_some` / serialize / `len` / `ser` / configured-detail), the CLI-TUI `ChannelDef`, the kernel `channel_sender` `for_each_channel_field!` entry + `EXPECTED` name-list, and the config-validation env-var hook. `bluesky` is removed from `crates/librefang-channels/src/channels-allowlist.txt`, so `cargo xtask channel-policy` now permanently rejects any attempt to reintroduce an in-process bluesky adapter. Behaviour is preserved by the new reference sidecar `librefang.sidecar.adapters.bluesky` (ships in `librefang-sdk`; source at `sdk/python/librefang/sidecar/adapters/bluesky.py`, stdlib-only, on the `librefang.sidecar` SDK) and additionally extended: same `com.atproto.server.createSession` auth with JWT refresh before 90 min expiry, same `app.bsky.notification.listNotifications?limit=25` 5 s polling filtered to `reason in {mention, reply}` with own-DID skip, same slash-command routing on `/cmd args`, same `display_name` (fallback handle) sender, same `app.bsky.feed.post` lexicon publish via `com.atproto.repo.createRecord`, same 300-char chunking with hard-cut fallback, same 401-on-publish → refresh → retry-once, same `updateSeen` `seenAt` watermark to suppress duplicate emissions. **Two improvements on top of the Rust adapter**: (1) **outbound threading is now actually wired** — the Rust `send()` always passed `reply: None`, so chunked replies showed up as a flat sequence of unthreaded posts; the sidecar parses `record.reply` on inbound and caches `{root, parent}` keyed by notification URI in an in-memory LRU (capacity 200), then `_post_status` looks up the cache on outbound and attaches `reply` so the bot's response lands as a proper thread under the originating mention/reply (every chunk in the chain reuses the same reply ref); (2) `suppress_error_responses = true` (Bluesky posts are public — same rationale as Mastodon). New env-var knobs: `BLUESKY_SERVICE_URL` (default `https://bsky.social`, for custom PDS), optional `BLUESKY_ACCOUNT_ID` for multi-bot routing. **Operator action required**: an existing `[channels.bluesky]` block is no longer recognised — re-declare as a `[[sidecar_channels]]` running `python3 -m librefang.sidecar.adapters.bluesky` with env vars `BLUESKY_IDENTIFIER` and `BLUESKY_APP_PASSWORD` (see the module's header for the exact config). Verification: `cd sdk/python && pytest` (165 tests, 36 new for bluesky) covers URL/scheme normalization, required-env enforcement, `_LruCache` put/get/eviction/LRU-touch, `_compute_reply_ref` for direct mention vs nested reply (root preserved), notification shape including thread_id surfacing and reply_ref caching, self-DID skip, slash-command routing, session create/refresh with create-fallback, `_post_status` bearer-auth + record shape, P1 threading on cache hit, cold-cache unthreaded fallback, chunked posts share the same reply ref, 5xx surfaced, 401 refresh+retry, polling 401 clears session, `seenAt` query param when set. (@houko)
- **BREAKING: Mastodon migrated from in-process Rust adapter to sidecar-only** — the in-process `librefang-channels::mastodon` adapter (`MastodonAdapter`, 850 lines: SSE user-stream subscribe + REST `/api/v1/statuses` publish) is deleted along with the `[channels.mastodon]` config schema (`MastodonConfig`), the `channel-mastodon` cargo feature (incl. its membership in `all-channels` / `all-channels-no-email`), the dashboard channel descriptor + 4 match arms, the CLI-TUI `ChannelDef`, the kernel `channel_sender` registry entry, and the config-validation hook. `mastodon` is removed from `crates/librefang-channels/src/channels-allowlist.txt`, so `cargo xtask channel-policy` now permanently rejects any attempt to reintroduce an in-process mastodon adapter. Behaviour is preserved by the new reference sidecar `librefang.sidecar.adapters.mastodon` (ships in `librefang-sdk`; source at `sdk/python/librefang/sidecar/adapters/mastodon.py`, stdlib-only, on the `librefang.sidecar` SDK): same SSE `event: notification` parsing filtered to `type == "mention"`, HTML stripper for `status.content` (`<br>`/`</p>`/`</div>`/`</li>` insert newlines; entities decoded via stdlib `html.unescape`), `/cmd args` → Command, sender from `display_name` (fallback `username`), `verify_credentials` at startup to discover the bot's own account id (skips self-mention echoes), thread chaining (`in_reply_to_id`) on chunked replies, REST polling fallback when SSE fails, exponential-backoff reconnect (1s → 60s), `suppress_error_responses = true` (Mastodon posts are public). New env-var knobs: `MASTODON_VISIBILITY` (public/unlisted/private/direct, default `unlisted`), `MASTODON_MAX_MESSAGE_LEN` (default 500, raise for instances configured for longer toots), optional `MASTODON_ACCOUNT_ID` for multi-bot routing. **Operator action required**: an existing `[channels.mastodon]` block is no longer recognised — re-declare as a `[[sidecar_channels]]` running `python3 -m librefang.sidecar.adapters.mastodon` with env vars `MASTODON_INSTANCE_URL` and `MASTODON_ACCESS_TOKEN` (see the module's header for the exact config). Verification: `cd sdk/python && pytest` (129 tests, 32 new for mastodon) covers URL/scheme normalization, required-env enforcement, visibility validation, HTML stripper edge cases (mention anchor, block-close newlines, entity decoding), notification shape including thread_id surfacing, self-mention skip, slash-command routing, REST publish with form-encoded body, chunked thread chaining, HTTP error surfacing, account_id ready-event. (@houko)
- **BREAKING: Gotify migrated from in-process Rust adapter to sidecar-only** — the in-process `librefang-channels::gotify` adapter (`GotifyAdapter`, 649 lines: WebSocket `/stream` subscribe + REST `/message` publish) is deleted along with the `[channels.gotify]` config schema (`GotifyConfig`), the `channel-gotify` cargo feature (incl. its membership in `all-channels` / `all-channels-no-email`), the dashboard channel descriptor + 5 match arms, the CLI-TUI `ChannelDef`, the kernel `channel_sender` registry entry, and the config-validation hook. `gotify` is removed from `crates/librefang-channels/src/channels-allowlist.txt`, so `cargo xtask channel-policy` now permanently rejects any attempt to reintroduce an in-process gotify adapter. Behaviour is preserved by the new reference sidecar `librefang.sidecar.adapters.gotify` (ships in `librefang-sdk`; source at `sdk/python/librefang/sidecar/adapters/gotify.py`, stdlib-only, on the `librefang.sidecar` SDK): same WebSocket subscribe with token-in-query, JSON frame parsing (`id`/`message`/`title`/`priority`/`appid`), `/`-prefixed-text → `Command`, sender derived from `title` (fallback `app-{appid}`), REST publish with `priority: 5` and chunked title `(i/N)`, optional `GOTIFY_ACCOUNT_ID` for multi-bot routing, exponential-backoff reconnect (1s → 60s). The WebSocket client is a hand-rolled RFC 6455 reader on `socket` + `ssl` (no third-party WS lib) — responds to server pings with masked pongs and echoes close frames before disconnecting. **Operator action required**: an existing `[channels.gotify]` block is no longer recognised — re-declare as a `[[sidecar_channels]]` running `python3 -m librefang.sidecar.adapters.gotify` with env vars `GOTIFY_SERVER_URL`, `GOTIFY_APP_TOKEN`, `GOTIFY_CLIENT_TOKEN` (see the module's header for the exact config). The separate gotify *push-notification provider* (`push_provider = "gotify"`, used by device pairing) is unaffected — it is a different feature and was deliberately left intact. Verification: `cd sdk/python && pytest` (97 tests, 22 new for gotify) covers WS frame parsing on a loopback server, schema validation, env-var enforcement, command vs text routing, sender fallback, chunked publish with numbered titles, HTTP error surfacing. (@houko)
- **memory_store/memory_recall/memory_list now scope to the calling agent by default instead of a shared namespace** (#5070). Previously all agents wrote to a hardcoded sentinel UUID (`00000000-...-0001`), meaning one agent could read or overwrite another's keys. Per-agent isolation is now active when `caller_agent_id` is supplied by the tool dispatch layer (the default for agent-initiated calls). Legacy data in the shared namespace remains readable via a recall-only fallback with a deprecation warning; it is intentionally left invisible to `memory_list` and not migrated in place during this interim compatibility window. WASM host functions use the same recall-only fallback in `crates/librefang-runtime/src/host_functions.rs`; remove it after the documented cleanup release. (@leszek3737)

### Fixed

- **Cascade leak: chat-template tokenizer bleed via `[User]` line-leader** — `is_cascade_leak` now detects a single envelope-standalone marker (`[User]`/`[Stranger]`/`[Forwarded]`) as a line-leader when preceded by a silent sentinel (`NO_REPLY`, `[no reply needed]`, etc.). Incident 2026-05-20: model emitted `NO_REPLY\n\n[User]\nEhi puoi dirmi…` — the sentinel led, the chat template continued into the next-turn header, and the hallucinated question was dispatched verbatim. The new `has_envelope_marker_as_line_leader` gates on `is_silent_response(before)` so explanatory replies quoting the markers are not silenced. (@f-liva)
- **`channels`: channel-side `/commands` filter by `account_id` so multi-bot deployments stop collapsing to the first-registered agent, and `/agent` no longer leaks user-defaults across bots** (closes #5672). Every command match arm in `handle_command` (`/btw`, `/new`, `/reboot`, `/compact`, `/model`, `/stop`, `/usage`, `/think`) plus the `model:` interactive-button callback in `dispatch_message` called the context-less `AgentRouter::resolve(channel_type, platform_id, user_key)` to pick the target agent. `resolve` has no `account_id` parameter — it probes only the plain `channel_key` (`"telegram"`) for the per-channel default — but the per-bot defaults registered by `bridge.rs` are stored under the qualified `"telegram:<account_id>"` key (see #2183 / #4861). With no plain-`"telegram"` entry to match, the lookup fell through to the system default, which the first-registered telegram bot also set (`channel_bridge.rs` "if !system_default_set" branch), so `/model` (and every other command) in `bot-b` / `bot-c` always reported `bot-a`'s agent. The non-command message dispatcher at `dispatch_message` already routed correctly because it built a `BindingContext` and called `resolve_with_context`; the regression was strictly in the command arms. Layer B: `/agent <name>` wrote the user's selection via `router.set_user_default(sender.platform_id, agent_id)`, keyed by `platform_id` only (no channel scope). The same lookup is then probed at step 2 of every resolver before the per-channel default, so a `/agent agent-C` issued in `bot-a` silently overrode every other bot's `default_agent` for that user — and stayed leaked until daemon restart (`user_defaults` is in-memory `DashMap`, never persisted). Fix is a single architectural change applied in two places: (1) `AgentRouter.user_defaults` is now keyed by `(Option<channel_account_key>, user_key)` where `channel_account_key = Some("<channel>:<account_id>")` for per-bot scope or `None` for legacy channel-agnostic global scope; `resolve_with_context` probes the per-channel-account entry first and falls back to the global. `set_user_default(user_key, agent)` keeps its legacy global semantics (relied on by `tests/bridge_integration_test.rs` and `benches/dispatch.rs`); a new `set_user_default_for_channel(channel_account_key, user_key, agent)` is the production-path helper. (2) `handle_command` gains an `account_id: Option<&str>` parameter, the two dispatch-site callers in `dispatch_message` thread `message.metadata["account_id"]` through, and the internal `resolve_for_command` closure builds a `BindingContext` per call and routes through `resolve_with_context`; `/agent` writes via `set_user_default_for_channel` when an `account_id` is known and falls back to the legacy global write for CLI / single-bot channels. `resolve_or_fallback`'s auto-bind on first-message (`bridge.rs` line 3017) is migrated the same way — otherwise the fallback path would silently re-introduce the cross-bot leak. Four new regression tests pin the fix: `router::tests::user_default_does_not_leak_across_bots`, `router::tests::channel_scoped_user_default_overrides_global`, `router::tests::legacy_set_user_default_is_channel_agnostic` (existing tests + benches continue to compile), `bridge::tests::command_resolution_respects_account_id` (records the `set_model(agent_id, _)` call sequence via a recording mock and asserts `[agent_b, agent_c]` for `/model` issued in `bot-b` then `bot-c` — pre-fix this would be `[agent_a, agent_a]`), `bridge::tests::agent_command_does_not_leak_across_bots` (asserts the post-`/agent` `resolve_with_context` for `bot-b` returns `bot-b`'s default, not `bot-a`'s override). `cargo test -p librefang-channels --lib` 465/465 green (including the 6 new + the pre-existing `test_multi_bot_account_id_routing` from #2183), `cargo clippy -p librefang-channels --all-targets -- -D warnings` clean, `cargo check --workspace --lib` clean. Same architectural class as #4868 / #4905 (which scoped `/new`'s session reset to the calling channel but left the agent resolution itself context-less) and #2183 / #4861 (which added the account-qualified `channel_defaults` lookup but never migrated the command-side callers to use `resolve_with_context`). (@houko)
- **`channels/matrix-sidecar`: persist `/sync` `since_token` across restarts so respawned adapters don't replay the recent timeline as fresh inbound** — `MatrixAdapter.since_token` (`sdk/python/librefang/sidecar/adapters/matrix.py:766`) was an instance attribute with no disk backing. Every supervisor respawn (`SidecarAdapter` restart loop in `crates/librefang-channels/src/sidecar.rs`) re-instantiated the adapter with `since_token = None`, so the first `/sync` returned the most-recent ~10 timeline events per joined room (the limit baked into `_build_sync_url`'s filter), each one emitted as a fresh inbound `message` event because the in-memory `_seen` dedupe set is also blown away on respawn. The daemon then routed every one to its bound or default agent, and the bot re-responded to every old DM. Symptom on a single-bot setup: every `docker compose restart` produced N duplicate replies, N = sum of recent events across all joined rooms. Fix persists the cursor to `$LIBREFANG_HOME/sidecar-state/matrix-<safe-user>-since.txt` (atomic `tmp + os.replace`) after every successful `/sync` and reloads it in `__init__`; the file is keyed by `user_id` with `[^\w\-_.@]` swapped for `_` so multi-bot setups (different `MATRIX_ACCOUNT_ID` values pointing at different `MATRIX_USER_ID`s) don't collide. Graceful degradation: load-time `OSError` logs `WARN` and falls back to `since_token = None`, persist-time `OSError` logs `WARN` and lets the next `/sync` retry; `$LIBREFANG_HOME` unset or empty disables persistence entirely (test confirms `_persist_since_token()` is a no-op rather than a crash). The in-memory `_seen` set is unchanged — it remains the second-level dedupe for the `/sync delayed-success-then-retry` case the module header already documents. Seven new tests in `sdk/python/tests/test_matrix_adapter.py`: state-path derivation from `$LIBREFANG_HOME`, user_id sanitisation (`:` → `_`), cross-instance persistence (`Adapter#1.process_sync_body → Adapter#2.__init__ reads cursor`), missing-file cold start, unreadable-file fallback to `None`, atomic-replace (no leftover `.tmp`), no-home-defensive no-crash. All 80 matrix-adapter tests green (`cd sdk/python && python3 -m pytest tests/test_matrix_adapter.py -k "since or process_sync"` → 16/16; the broader suite's pre-existing `test_on_send_text_path` asyncio failure is unrelated — missing `pytest-asyncio` in this dev env). No protocol change, no Rust-side touch. (@neo-wanderer)
- **`api-error-generic` Fluent key defined across all 6 locales — 41+ HTTP 500 handlers stop returning the literal key as the response body** (audit `docs/issues/api-error-generic-missing-fluent-key.md`, Severity: Critical). 41 call sites in `crates/librefang-api/src/routes/{agents,tools_sessions}.rs` (and elsewhere — `rg -c 'api-error-generic'` across `crates/librefang-api/src/routes/`) build their 500 response body as `t_args("api-error-generic", &[("error", &e.to_string())])`, but the key was never defined in any of `crates/librefang-types/locales/{en,ja,zh-CN,de,fr,es}/errors.ftl`. The fallback at `i18n.rs:163-164` returns `key.to_string()` on a missing-key lookup, so the `{$error}` interpolation never ran — every one of those 5xx responses surfaced the literal `"api-error-generic"` to the client with the actual `e.to_string()` silently dropped. This swallowed the diagnostic context behind every 500 in `agents` + `tools_sessions` routes (including the bug classes `agent-not-found-returns-500`, `rusqlite-errors-leak`, and the `agent_clone` 500-on-`AgentAlreadyExists` case). Stopgap fix per the audit doc: define the key in every locale. English uses bare `{ $error }` (the underlying error string is already in English for now); the five other locales prefix with a localized `Error / エラー / 错误 / Fehler / Erreur / Error: ` so the localized prefix differs across bundles (the per-locale assertion in `all_languages_have_same_keys` requires non-English bundles emit a distinct rendered string). The long-term remediation is to route every 500 through a typed `MemoryRouteError`-style helper (per `routes/memory.rs:198-215`), audited separately. Two regression guards land alongside: `api-error-generic` is appended to the key list in `all_languages_have_same_keys` so a new locale or a stale `errors.ftl` cannot silently regress this, and the new dedicated `api_error_generic_interpolates_underlying_error` test exercises every supported locale with a sample `$error` and asserts (a) the rendered body is not the literal key and (b) the underlying error string is contained verbatim. All 22 `i18n::tests` green; `cargo test -p librefang-types --lib i18n`. (@houko)
- **`history_fold` + web-augment search-query generation: aux-LLM requests pin `response_format = Some(ResponseFormat::Json)`** (closes #5287). Two aux-LLM call sites in `librefang-runtime` were building `CompletionRequest` with `response_format: None` even though their system prompts explicitly asked for a JSON response — strict-output providers (DeepSeek `json_object` mode, OpenAI, Mistral, Gemini) require the request flag set in lockstep with the prompt, and without it they emit free-form prose that the downstream JSON parser silently rejects. (1) `crates/librefang-runtime/src/history_fold.rs:603` (the `summarise_batch` aux call): system prompt asks for "summaries of tool outputs as a JSON array, echoing the supplied ids verbatim", `parse_labeled_summaries` then failed with `JSON parse failed: invalid number at line 1 column 3`, and the fold degraded to "applying raw response as bulk summary" — losing per-`tool_use_id` granularity (`tool_use_id` / `tool_name` / `is_error` / `status`) every few turns on long sessions. Most visible on DeepSeek-only setups (`deepseek-v4-flash` + `deepseek-v4-pro`) where the WARN log spammed. (2) `crates/librefang-runtime/src/agent_loop/web_augment.rs:98` (the `generate_search_queries` aux call): system prompt (`SEARCH_QUERY_GEN_PROMPT`) says `"Respond ONLY with a JSON object: {"queries": ["query1", "query2"]}"`, parser uses `text.find('{')? ... rfind('}')?` so a prose response with no braces returns `None`, `generate_search_queries` returns `None`, and `web_search_augment` falls back to running the raw user message verbatim as a search query. The failure was completely silent — no log, just degraded retrieval relevance. The same precedent already exists in sibling aux paths — `crates/librefang-runtime/src/proactive_memory.rs:449,608` pin `Some(ResponseFormat::Json)` for the same class of structured-output aux call. Providers that don't honour the flag (Anthropic on non-tool-call shapes) ignore it without error; existing parser tolerances (`parse_labeled_summaries`'s `strip_code_fence` + single-object lift; web_augment's outermost-brace extraction) are preserved. Regression guards: `fold_request_pins_response_format_json_for_aux_llm` (history_fold) and `search_query_request_pins_response_format_json` (web_augment) each record the aux request's `response_format` via a `ResponseFormatRecordingDriver` mock and assert `Some(ResponseFormat::Json)` — wired alongside the existing `MaxTokensRecordingDriver` so future aux-budget edits don't accidentally drop the flag at either site. Other `response_format: None` occurrences in the runtime crate were audited and confirmed correct: `compactor.rs:715,847` are free-text summary prompts (no JSON expected); `aux_client.rs:397,535` and `routing.rs:199` are all inside `#[cfg(test)] mod tests`. **Cross-crate audit follow-up**: the same defect class also lived in `crates/librefang-kernel/src/skill_workshop/llm_review.rs:126` — `SYSTEM_PROMPT` requires `{"accept": bool, "reason": "..."}` JSON verbatim, but `response_format: None` let DeepSeek / Qwen / older Mistral append a prose preamble and silently stall every workshop candidate as `ReviewDecision::Indeterminate` behind a parse error nobody triages (default `approval_policy = "pending"` makes the queue grow without surfacing). Fixed inline with a third regression guard, `pins_response_format_json`, alongside the existing two; `librefang-kernel::skill_workshop::llm_review` tests pass (11/11). Also: `web_augment.rs` fallback path (LLM query generation returns no parseable queries → fall back to verbatim user message as the single search query) now emits a `debug!` log so operators can spot a degraded provider — the fix prevents the failure mode for well-behaved providers, but a misbehaving Ollama / older Anthropic that still ignores the flag was previously fully silent. (@houko)
- **`remove_agent` cascade now purges `pending_approvals`; schema-walking regression guard catches future agent-keyed tables added without a matching DELETE** (audit `docs/issues/agent-cascade-delete-missing-tables.md`, Severity: High). `execute_structured_agent_deletes` in `crates/librefang-memory/src/structured.rs:706` enumerated 14 tables but missed `pending_approvals` (v26, #3611). Authored approvals are scoped by `agent_id` so a deleted agent left stale approval rows behind that could fail-open on restart recovery. Fix adds `DELETE FROM pending_approvals WHERE agent_id = ?1` to the cascade. The other three tables the audit doc listed turn out **not** to be agent-scoped at the `librefang-memory` layer and are explicitly excluded with rationale in the new test's doc-comment so future maintainers don't reintroduce the audit's misreading: (a) `paired_devices` has no `agent_id` column — devices are operator-scoped, not per-agent, and continue to authenticate against the operator's API key rather than any particular agent (the audit doc's "bearer-token replay against deleted agent" framing maps to paired-device lifecycle, not agent lifecycle, and is out of scope here); (b) `idempotency_keys` has no `agent_id` — keys are scoped by request `Idempotency-Key` value; (c) `workflow_runs` has `workflow_id` but no `workflows.agent_id` mapping exists in `librefang-memory` (definitions live in the kernel + YAML on disk), so per-agent scoping must be done at the kernel layer if ever needed and is not addressable from this crate. New regression test `agent_cascade_purges_every_agent_keyed_table` walks `sqlite_master` for every user table, runs `PRAGMA table_info` on each to discover the agent-keyed subset (any `agent_id` or `source_agent` column), seeds rows for a target + a control agent, runs the cascade transaction (the same `execute_session_agent_deletes + execute_structured_agent_deletes` pair that fires in `substrate.rs:1446-1447`), and asserts the target's rows are gone. A future migration that adds a new agent-keyed table without a matching DELETE in the cascade will fail this test with the offending table name printed. All 246 `librefang-memory` lib tests green; clippy clean. (@houko)
- **Dashboard: config page sub-tabs wrap to multiple rows + global hide of browser-default `[type=number]` spinner buttons** (closes #5293). The reporter raised two distinct complaints on `/config/general`: trailing sub-tabs clipping off-screen at default zoom, and "arrow up and down for scrolling appears in below ... useless and no need for them" — the latter being the browser-default spinner buttons on `<input type="number">` fields, which Linux/KDE renders more prominently than macOS. (1) **Sub-tab wrap** — the `general` and `infra` categories ship 14 and 15 section sub-tabs respectively, but Row 2's container at `ConfigPage.tsx:989` rendered them as a non-wrapping single-line flex strip with no `overflow-x-auto` (unlike Row 1.5's category tabs, which already had it). On a default-zoom 753-px-wide viewport the trailing sub-tabs (`provider_request_timeout_secs`, `vertex_ai`, `azure_openai`, `proxy` on general; `notification`, `background` on infra) were horizontally clipped off the page edge with no scroll affordance or visible overflow. Adding `flex-wrap` + `gap-y-1` lets the strip break to a second / third row when needed; the active-tab underline stays anchored on each tab via `border-b-2 -mb-px` (not on the container), so wrapping doesn't draw stray separator lines between rows. Row 1.5 (7 category tabs) is unchanged — it already fits in a single line on every supported viewport. (2) **Spinner-buttons hidden globally** — every `<input type="number">` on the dashboard (config page, agent manifest form, agent schedule panel, slider input, schema form, …) is for typed numeric entry (port numbers, timeouts, token caps, intervals) — the stepper offers no value and the reporter's "arrow up and down" complaint is specifically about its visual noise. `src/index.css` now applies `appearance: textfield` + `::-webkit-{inner,outer}-spin-button { -webkit-appearance: none }` to every `input[type=number]` on the page; Up/Down arrow keys still step the value so keyboard affordance is preserved. Verified via Playwright at the reporter's 753 px and 805 px viewport widths: `flex-wrap` renders all 14 general sub-tabs across 3 rows at 753 px and 2 rows at 805 px (last tab at x=689, well inside 753); computed `getComputedStyle(input, '::-webkit-inner-spin-button').appearance === 'textfield'` confirms the spinner rule is applied. (@houko)
- **`/api/status` + `/api/dashboard/snapshot` session-count uses indexed `COUNT(*)` instead of decoding every session blob** (audit `docs/issues/list-sessions-decode-on-poll.md`, Severity: Critical, Performance). Two call sites in `crates/librefang-api/src/routes/config.rs` (lines 235 and 3009 — the `/api/status` health probe and the dashboard 5 s snapshot poll respectively) computed `session_count` via `substrate.list_sessions().map(|s| s.len())`. `list_sessions()` returns `Vec<serde_json::Value>` — each row's full rmp-encoded message history is decoded just to compute `.len()`, for what is morphologically a `SELECT COUNT(*)`. The dashboard polls `/api/dashboard/snapshot` every 5 s with `refetchIntervalInBackground: false`, so every foreground operator hits it continuously. At 100 sessions × 200 KB each the daemon decoded ~20 MB of message bodies per poll (≈ 4 MB/s on a 5 s cadence, doubled if both `/api/status` and `/api/dashboard/snapshot` are polled concurrently) — needless rmp deserializer allocation pressure on the hot path. `MemorySubstrate::count_sessions()` (an indexed `SELECT COUNT(*)` over `sessions`) already exists in `crates/librefang-memory/src/substrate.rs:391` and is what the route should have been calling. Both sites now use it; regression test in `crates/librefang-api/tests/config_status_session_count_test.rs` pins that the wire `session_count` actually responds to substrate state (catches a future `.unwrap_or(0)` silent-pin regression on either route). (@houko)
- **Dashboard `AuditPage` Export downloads no longer 401 for every user** (audit `docs/issues/audit-export-401.md`, Severity: Critical). `AuditPage.tsx:110` 's `downloadExport` helper read the bearer token via the bare `safeStorageGet("librefang-api-key")` accessor, which **only checks `localStorage`**. The current credential layer (`api.ts:1038-1047 getStoredApiKey()`) prefers `sessionStorage` and #3620 explicitly wipes `localStorage` on save (`api.ts:3595-3596`) so the token has been in `sessionStorage` for every signed-in user since that release. Result: every "Export audit log" click read an empty token, sent `Authorization: Bearer ` (one trailing space, no value), the server replied 401, and the `ApiError` surfaced to the user with no actionable hint. Broken for every user since #3620. Fix: drop the inline `safeStorageGet` and use the canonical `getStoredApiKey` accessor from `api.ts` (the same one `authHeader()` and every other token consumer goes through). The other `safeStorageGet` call sites in `pages/` were audited and are correctly scoped (`TerminalPage.tsx`'s `terminal.fontSize` + hint keys, `ChatPage.tsx`'s `librefang.chat.show_hand_agents`) — none of them target `librefang-api-key`. (@houko)
- **CI: repair three pre-existing `[Unreleased]`/main breakers inherited by every open PR.** Three checks were red on `main` itself (and therefore on every PR that merged it), unrelated to the PR contents: (1) `CHANGELOG Attribution (full [Unreleased])` failed because the `cli: restore librefang channel` entry carried its `(@houko)` only on the trailing continuation line, while `--all-unreleased` requires `(@user)` on every bullet incl. the four nested sub-command bullets — each now suffixed. (2) Dashboard `build` (vitest) failed 9 `ProvidersPage.test.tsx` cases with `No "useCredentialPools" export is defined on the "../lib/queries/providers" mock` — `ProvidersPage.tsx` calls `useCredentialPools()` (CredentialPoolsSection, #5459 era) but the test's `vi.mock` never stubbed it; added a default `vi.fn(() => ({ data: undefined, isLoading: false, error: null }))` so the section renders its hidden/empty state. (3) `Workspace coverage (llvm-cov)` failed 4 `librefang-migrate::openclaw` tests (`test_full_migration`, `test_json5_full_migration`, `test_json5_channel_extraction`, `test_policy_migration`) which still used `google_chat` as an in-process "witness" channel; #5459 migrated it to a sidecar, so `migrate_channels_from_json` now returns `None` and every channel surfaces as a `report.skipped` `ItemKind::Channel` rather than an import — assertions updated to the all-sidecar contract already pinned by `test_google_chat_channel_records_skipped_after_sidecar_migration`. (@houko)
- **CI: raise the `Test / Windows` shard timeout 45 → 60 min (match macOS) so a cold cache stops cancelling `main`.** The Windows test job (`ci.yml` `test-windows`, 2 shards) was the dominant red on `main` — nearly every recent push showed `The job has exceeded the maximum execution time of 45m0s` on both shards. The tests themselves pass; the job simply doesn't fit in 45 min when the `rust-cache` is cold. Each shard pays a ~22 min full workspace compile (any `Cargo.lock` change busts the `test-windows-${hashFiles(Cargo.lock)}` key, and recent crate churn — #5852 subprocess transport extraction et al. — busts it often) on top of ~22 min of test execution, landing a warm run at ~45 min (run 26617513861 shard 1 finished at 44m56s) and overrunning on any cold compile. Windows runner I/O + cargo build is ~2× Linux, so adding shards would multiply the 2×-cost compile rather than help; raising the per-shard ceiling to 60 min (the value macOS already uses for the same full-workspace run) absorbs a cold compile and costs nothing on green runs, which end when nextest finishes rather than at the ceiling. Test failure mode only — no production change. (@houko)
- **`bluesky` sidecar: reply context survives sidecar restart via on-demand XRPC re-fetch** (closes #5452). `parse_notification` stashed the AT Protocol `{root: {uri, cid}, parent: {uri, cid}}` reply struct in an in-process `_thread_cache` LRU (capacity 200) keyed by notification URI, then `_post_status` looked it up on the outbound. After ANY sidecar restart between the inbound mention and the bot's reply (operator restart, supervisor crash-recovery, container redeploy, etc.) the cache was empty, the lookup returned `None`, and the reply posted as a top-level skeet — visible to every follower's feed instead of staying in the thread. The single-string `cmd.user.librefang_user` carrier added in #5439 only round-tripped the URI; AT Protocol needs 4 fields (`root.uri`, `root.cid`, `parent.uri`, `parent.cid`) which don't fit. Fix: `_post_status` on cache-miss now issues one `app.bsky.feed.getPosts?uris=<uri>` XRPC call to re-fetch the post's cid AND its existing `record.reply` chain, reconstructs the `{root, parent}` struct using the exact same root-resolution rules `_compute_reply_ref` uses on the inbound path (post-with-reply → keep its `record.reply.root`; top-level post → `root == parent`), re-populates `_thread_cache` so subsequent chunks in the same `_post_status` call don't re-fetch, and uses the recovered ref. Recovery failure (post deleted, instance unreachable, auth still bad after caller's refresh) emits a WARN naming the URI and falls through to a top-level skeet — same degradation as pre-fix but with operator visibility instead of silent mis-threading. `_recover_reply_ref` rejects non-`at://`-prefixed values up-front so a misrouted `librefang_user` from another channel (dingtalk sessionWebhook URL, telegram `@username`) never reaches bsky's XRPC. Cost: one extra XRPC round-trip per cache-miss reply (post-restart, post-eviction). Verification: `cd sdk/python && pytest tests/test_bluesky_adapter.py` — **49 passed** (was 44; +5 regression guards: recovery happy-path, deep-thread root preservation, multi-chunk re-uses recovered ref without re-fetching, recovery-failure degrades-to-unthreaded WARN, cross-channel garbage rejection). Pre-existing cold-cache test (`test_post_status_cold_cache_falls_back_to_unthreaded`) renamed to `test_post_status_cold_cache_recovery_failure_falls_back_to_unthreaded` and updated to assert the getPosts re-fetch fires + 404s + then degrades — the renamed assertion shape pins the recovery path is actually attempted, not bypassed. **Not addressed in this PR** (deliberately scoped): the structural option-3 fix from the issue (widen `SidecarSendParams` with a `metadata: HashMap<String, Value>` bytewise-round-tripped field that would close this class for bluesky, wechat, wecom, and any future sidecar needing > 1 field of correlation state) is a separate protocol change spanning Rust + Python SDK; this PR is option-1 (narrowest blast radius, bluesky-only). (@houko)
- **`librefang-runtime::pii_filter` partial-redacts WhatsApp JIDs** — the `phone` built-in pattern (`(?:\+\d{1,3}[\s\-]?)?\(?\d{2,4}\)?[\s.\-]?\d{3,4}[\s.\-]?\d{3,4}`) greedily consumed the leading digits of a WhatsApp JID (`<digits>@lid`, `<digits>@s.whatsapp.net`, `<digits>@c.us`, `<digits>@g.us`, `<digits>@broadcast`, `<digits>@newsletter`), leaving the trailing digits and the `@<domain>` suffix in the output (e.g. `393511083257@lid` → `[REDACTED]257@lid`). This is both an information leak (tail digits + suffix) and a correctness bug for downstream consumers that parse the redacted text expecting either a full JID or a full `[REDACTED]` placeholder. `regex_lite` has no lookahead so a negative `(?!@(?:lid|…))` anchor on `phone` isn't available; the fix instead adds a `whatsapp_jid` pattern (`\b\d{5,20}@(?:lid|s\.whatsapp\.net|c\.us|g\.us|broadcast|newsletter)\b`) as the FIRST entry of `BUILTIN_PATTERNS` so the full JID is consumed as a single unit before `phone` ever sees it. In `Pseudonymize` mode the JID gets a stable `[Whatsapp_jid-A]` mapping (same `get_or_create_pseudonym` machinery as `[Email-A]` / `[User-A]`). Six new tests in `pii_filter::tests` pin the contract: full LID redact, full phone-JID redact, full group-JID redact, atomic-not-partial regression (no digits or `@<domain>` survive in the output), pseudonym stability across calls, and a negative-control pair asserting the new pattern does not regress the `email` / `phone` paths. `cargo test -p librefang-runtime pii_filter --lib` — 22/22 passed. (@f-liva)
- **Cross-sidecar audit follow-ups: `Retry-After` on 429 for `slack` / `feishu` / `gotify` / `google_chat`; inbound dedupe for `gotify` / `google_chat`; LINE reply-API path now used inside the 55 s freshness window.** Cross-cutting consistency audit across the 27 freshly-migrated sidecar adapters (everything from `ntfy` #5224 through `google_chat` #5459) caught four adapters that landed without 429 handling — Slack's `_post_message` / `_add_reaction` / `_remove_reaction` and Feishu's `_http_json` / Gotify's `_publish` / Google Chat's `_send_text` all routed 429 through the same `status >= 300` arm as 5xx, dropping the chunk and ignoring the `Retry-After` window so the next outbound burst extended the server-side rate-limit. The Slack 3-tuple `_http` helper (`_resp_hdrs` was deliberately stripped) is now a retry-aware wrapper that re-issues the call once with `parse_retry_after(default_secs=RETRY_AFTER_DEFAULT_SECS)` from the shared `librefang.sidecar.common`; `_post_message` / `_add_reaction` / `_remove_reaction` inherit the fix without touching the call sites, so the existing 3-tuple unpack contract is preserved. Feishu's `_http_json` (4-tuple, already exposed headers) gained the same `retry_429=True` once-shot before falling through to its `code != 0` arm. Gotify's `_publish` and Google Chat's `_send_text` factored their POST body into a `_publish_chunk` / `_send_chunk` helper so the 429 retry shares one code path with the original raise-on-non-2xx semantics. Also caught two adapters with missing inbound dedupe — Gotify's WebSocket can replay buffered frames on reconnect and Google Chat's webhook is at-least-once-delivery from Pub/Sub. Both now thread inbound messages through `librefang.sidecar.common.SeenSet` (10 000 / evict 5 000, identical policy to nextcloud / reddit / rocketchat / webex) keyed on `gotify-<id>` and the Google Chat `message.name`. Feishu was a false-positive in the audit — it already has Rust-parity `_EventDedup` (mirrors `feishu.rs:122-125`) used at `_dispatch_event` line 1485. LINE picked up an additional capability fix: the inbound `reply_token` was parsed and stashed in `metadata.reply_token` but `metadata` doesn't round-trip back to the sidecar's `on_send`, so every reply degraded to the push API (quota-charged, rate-limited) even within the LINE-server's ~60 s reply window where the free reply API was available. The token is now carried through `librefang_user` (the field the daemon's bridge round-trips bytewise) with a `linereply:<token>:<event_ts_ms>` shape; a `LINE_REPLY_TOKEN_TTL_SECS = 55.0` window plus a `linereply:` prefix guard (librefang_user is shared across channels — dingtalk stores a sessionWebhook URL, telegram stores an @username — and a misrouted value must not be fed to LINE's reply endpoint) decides between reply and push at send time, with automatic push-fallback if LINE rejects the reply call (the most common case being a token already burned between dispatch and the agent's wakeup). Image+caption sends stay on push regardless (the reply token is one-shot-locked on first acceptance and a follow-up caption would error). Verification: 17 new pytest cases (3 × slack-429 + 3 × feishu-429 + 3 × gotify-429-and-dedupe + 3 × google-chat-429-and-dedupe + 5 × line-reply) across `sdk/python/tests/test_{slack,feishu,gotify,google_chat,line}_adapter.py`; full `pytest tests/` — 1845 passed (was 1828). `cargo check --workspace --lib` + `cargo clippy --workspace --all-targets -- -D warnings` clean. **Audited but not changed (false-positives or out-of-scope)**: `feishu` already had Rust-parity dedup via `_EventDedup`; `runtime.py:246` bare-except on `on_command` is OPEN PR #5450 territory (not duplicated here to avoid conflict); `runtime.py:219` producer bare-except is the same class but would conflict with #5450 in the same file — deferred to land after #5450 merges; `mastodon` SSE+poll dual-flow dedupe is theoretical (since_id watermark already covers normal operation); long-lived SSE timeouts are by design. (@houko)
- **`librefang-api` test build repaired after #5455 (webhook → sidecar).** #5455 folded `write_service_account_env` into the generic `write_secret_env` (identical newline-rejection contract) but left `routes/skills.rs`'s unit test calling the removed name, so the `librefang-api` test target failed to compile (`E0425`) and `main` was red again immediately after #5456. The test now calls `write_secret_env` (renamed `write_secret_env_value_with_newline_is_rejected`); same assertions. (@houko)
- **CI on `main` restored — build break + `librefang-migrate` test drift from the wecom/teams/wechat/feishu/whatsapp sidecar merges.** `main` (HEAD `ff3f673`) did not compile and its migrate tests were red, so every open PR's Rust CI was failing on inherited breakage. Compile fixes: (1) `librefang-types` referenced `default_local_probe_interval_secs` from `local_probe_interval_secs`'s `#[serde(default)]` and the `Default` impl but the function was never defined (`E0425`) — added it returning 60 s per the field doc; (2) removed the orphaned `default_channel_max_backoff_secs` / `default_channel_initial_backoff_2s` (no `serde(default=)` consumers, `dead_code` under `-D warnings`); (3) `librefang-migrate::openclaw` — renamed the now-unused `ch` YAML-parse binding to `_ch` (every channel arm is a sidecar skip; the parse only keeps `LegacyYamlChannelConfig` referenced) and removed the orphaned `allow_from_to_toml_array`; (4) `librefang-api::openapi` — dropped the utoipa `paths(...)` refs to the removed `whatsapp_qr_*` / `wechat_qr_*` routes (`E0433`); (5) `librefang-cli` — removed the orphaned `maybe_write_channel_config` / `notify_daemon_restart` helpers (their in-process channel-onboarding callers were dropped). Test-fixture drift (only `google_chat` / `webhook` remain in-process channels): `test_roundtrip_migrate_output_into_real_structs`, `test_json5_channel_extraction`, and `test_full_migration` now use `google_chat` as the in-process witness instead of WhatsApp (asserting WhatsApp as a skipped sidecar); `test_json5_full_migration` / `test_secrets_migration` lower the secret-count floors `7 → 5` and flip the stale `FEISHU_APP_SECRET` extraction assertion to *absent* (the feishu sidecar-skip no longer extracts its secret — note Mattermost still does on skip, an intentional per-channel asymmetry flagged in-test); and the `openfang` `deny_unknown_fields` drift test drops the removed flat `command` field from its `[[mcp_servers]]` fixture so the intended `nickname` typo is what gets rejected. Verified locally: `cargo check --workspace --all-targets` clean, `cargo test -p librefang-migrate` 56 passed, `cargo test -p librefang-memory` 246 passed. (@houko)
- **Telegram sidecar reconnect loop: cap aligned with siblings, recovery now logged, regression coverage added** (closes #5111). #5111 was filed against `v2026.5.12-beta.11`, when telegram was the in-process Rust `librefang-channels::telegram` adapter — that adapter exited its polling task on a DNS resolution / transient network failure and the bridge stayed dead until the daemon was restarted, exactly as the issue describes. The #5241 sidecar migration replaced it with the Python `sdk/python/librefang/sidecar/adapters/telegram.py`, which already wraps `_poll_once` in a `while True / except Exception → backoff` loop and so silently fixes the reported "bridge stays dead" symptom. **This PR is not the bug fix** (that was #5241); it is the observability + cap-alignment + regression coverage that should have shipped alongside #5241 so a future refactor can't re-introduce the original failure mode invisibly: (1) backoff cap moved from a hardcoded `120.0` to a new `MAX_BACKOFF_SECS = 60.0` module constant, matching the convention every sibling polling sidecar (`bluesky`, `discord`, `line`, `mastodon`, `mattermost`, `nextcloud`, `ntfy`, `reddit`, `rocketchat`, `twitch`) already settled on — behavioural change, persistent-outage retries now cap at one-minute intervals instead of two-minute ones; (2) the WARN line on each backoff now reports `retries=<consecutive-failure-count>` alongside `error` and `delay`, so operators can read "how long have we been degraded" off a single log line; (3) on the first successful poll after at least one retry the loop emits an INFO `telegram poll recovered retries=N last_backoff=…` — closes the issue's "restored DNS — bridge does NOT recover" symptom not by changing recovery (it already worked) but by making the recovery visible in the operator's log timeline; (4) `TimeoutError` (LONGPOLL_SERVER_SECS server-side block expiring with no updates — normal protocol behaviour) explicitly resets both `backoff` and `retries_in_a_row` and `continue`s without consuming the sleep budget, so an idle channel never accidentally drifts toward MAX. **Deliberately NOT done** per a deviation from the issue's suggestion of "ERROR only after N consecutive failures": every sibling polling sidecar reserves `log.error` for fatal startup-config issues (`{discord,line,mattermost}_required env vars missing`) — none of them escalate during steady-state backoff. The producer-crash path in `librefang.sidecar.runtime` already emits `log.error("producer crashed", …)` if an exception ESCAPES `produce()`; the backoff loop is precisely the layer that prevents that escape. Adding ERROR escalation here in telegram alone would diverge from the family, and the new WARN-with-retry-counter + INFO-on-recovery already provide the "how long degraded / when restored" signals an operator needs. Three new pytest cases assert the contract end-to-end: `test_produce_recovers_after_startup_network_failure` (URLError on first poll → warn + sleep + retry → success + INFO recovered), `test_produce_backoff_is_capped_at_max` (consecutive failures produce delays = `[1, 2, 4, 8, 16, 32, 60, 60, …]`, max never exceeds `MAX_BACKOFF_SECS`, the cap is actually reached), `test_produce_treats_longpoll_timeout_as_normal` (TimeoutError alternating with success → loop re-enters without sleeping, no backoff growth). All three monkeypatch `tg.asyncio.sleep` against a saved real-sleep closure to avoid the infinite-recursion footgun (`tg.asyncio` and the test file's `asyncio` import point at the same module object). (@houko)
- **CI on `main` restored (post-mattermost-sidecar) — `librefang-migrate::openclaw::tests` test-fixture drift after #5315** (closes #5316). The mattermost sidecar migration converted `mattermost` from in-process to sidecar in the production code paths (both YAML `parse_legacy_channels` and JSON5 `migrate_channels_from_json` now push a `SkippedItem` with a sidecar reason instead of emitting `[channels.mattermost]`), but four `openclaw::tests` cases were not updated in lock-step and used `mattermost` as their only in-process channel witness — so on main the `migrate_channels_from_json` return became `None`, no `ItemKind::Channel` ever landed in `report.imported`, and the JSON5 full-migration imported-count dropped from 7 to 6: (1) `create_legacy_yaml_workspace` only emitted `messaging/{telegram,discord,slack,mattermost}.yaml` — all four now sidecar-skipped — so `test_full_migration` 's `report.imported.iter().any(|i| i.kind == ItemKind::Channel)` asserted against an empty channel-imports vector and panicked at `openclaw.rs:4526`; fixture now also writes `messaging/whatsapp.yaml` (in-process), `test_scan_workspace` 's `channels.len() == 4` updated to `5` with a `whatsapp` membership assert. (2) `test_json5_channel_extraction` 's inline JSON5 (telegram/discord/slack/mattermost-only) made `channels.is_some()` false at `openclaw.rs:4054`; fixture now includes `whatsapp: { dmPolicy: "open", allowFrom: ["phone1"] }`, the `!ch_table.contains_key` / `report.skipped` checks add `mattermost`, the in-process-witness assertion flips from `mattermost` to `whatsapp`, the imported-count stays at 1, the 5-secrets assertion is preserved (mattermost token still flows into `MATTERMOST_TOKEN` via the sidecar-skipped path at `openclaw.rs:1920`), and a `MATTERMOST_TOKEN=mm-token` secrets.env assertion is added so the secret-extraction-on-sidecar-skip behaviour is explicitly covered. (3) `test_json5_full_migration` 's `channel_items.len() == 7` failed; assertion updated to `5` after the rebase (signal also migrated to sidecar in #5317, so the in-process count is whatsapp, matrix, feishu, google_chat, msteams = 5) and the count-comment rewritten to enumerate the 8 skips (telegram, discord, slack, signal, irc, mattermost, imessage, bluebubbles). (4) `test_policy_migration` used `mattermost` for the `dmPolicy: "disabled"` → `dm_policy = "ignore"` happy-path mapping; replaced with `matrix` (still in-process, also accepts `dm_policy`), `mattermost` + `signal` added to the sidecar-skip loop alongside discord/slack, and the comment chain is updated to record the witness-rotation history (discord → slack → mattermost → signal → matrix). All 36 `openclaw::tests` + 51 `librefang-migrate` lib tests + 6 `tests/idempotency.rs` integration tests + `cargo clippy -p librefang-migrate --all-targets -- -D warnings` are green locally. (@houko)
- **Cross-chat attachment leak: image/PDF/text attachments persisted into the wrong session** — `SessionWriter::inject_attachment_blocks` took only `(agent_id, blocks)` and wrote into the agent registry's `entry.session_id`, which is a single persistent slot per agent — NOT the per-chat session derived for the current turn. For a chat agent with `session_mode = "persistent"` whose group chat stays warm, any subsequent DM-inbound image landed in the *group* session instead of the DM. Real production incident 2026-05-20: at 08:43:13Z the owner sent a private Amazon-order screenshot ("metro laser, 27,98€") + caption "segna spesa in Shopping" via WhatsApp DM (chat 121043); at 09:44:04Z (1h later) the agent replied in a *public* WhatsApp group ("Non perdiamoci 💻", chat 120957) with those private order details verbatim. A msgpack dump of the group session showed the `[Image (image/jpeg) previously processed]` placeholder at the same second as the DM inbound — i.e. the DM image had been persisted into the group session's history. Root cause was on a code path PR #5288 (streaming sender-context fix) deliberately did not touch: the *attachment pre-inject* step that runs BEFORE the streaming/non-streaming handler proper. The trait now requires an explicit `session_id` parameter and the API call site (`inject_attachments_into_session` in `routes/agents.rs`) derives the same session id `send_message_*` would, from the per-request `SenderContext` and `session_id_override`. Both the streaming and the non-streaming attachment paths now route image / PDF / text content blocks into the *same* session as the text part of the same request. Regression coverage: `crates/librefang-kernel/tests/attachment_session_isolation_test.rs` pins the trait contract — passing `(agent_id, X, blocks)` writes into session `X`, never into `entry.session_id`; any future "fall back to entry.session_id" reintroduction trips the assertion. (@f-liva)
- **Slack sidecar: reply threading + `:eyes:`→`:white_check_mark:` reaction targeting.** `parse_slack_event` set `thread_id = thread_ts` only, so a top-level message carried `thread_id = None`: (a) the bot's reply posted at the channel root instead of threading under the triggering message (the `force_flat_replies` knob exists precisely to opt *out* of threading, so threaded-by-default is the intended behaviour), and (b) `on_send`'s reaction finalization received `None` (always, and doubly so under `SLACK_FORCE_FLAT_REPLIES`) and fell back to "first pending reaction in the channel", so concurrent messages flipped the `:eyes:` on the wrong message and left the real request stuck. `thread_id` now falls back to the message's own `ts` (mirroring rocketchat / nextcloud's `thread_id = parent or own_id`), and `on_send` finalizes against the inbound thread id rather than the force-flattened posting `thread_ts`, so the reaction lands on the exact triggering message. In-thread replies remain best-effort (the `Send` protocol carries no inbound `message_id`). Tests: `test_parse_event_top_level_thread_id_falls_back_to_ts`, `test_on_send_force_flat_finalizes_correct_message`. (@houko)
- **`channel_send` mirror (#4824) restored for sidecar channels** — `resolve_channel_owner` in `crates/librefang-kernel/src/kernel/handles/channel_sender.rs` only scanned the in-process `cfg.channels` (via `for_each_channel_field!`), so once a channel moved to a sidecar (`[[sidecar_channels]]`) it returned `None` and the agent's outbound `channel_send` was no longer mirrored back into the channel-owning agent's session. This silently affected every migrated sidecar channel (telegram #5241, discord #5299, nextcloud, rocketchat, reddit, bluesky, mastodon, and now slack). The resolver now also consults `cfg.sidecar_channels[*].default_agent` (the same field that seeds inbound routing via `AgentRouter.channel_defaults`), keyed by `channel_type` falling back to `name`, so the mirror works uniformly across in-process and sidecar channels. Unit tests: `sidecar_default_agent_matches_by_channel_type_then_name`, `sidecar_default_agent_skips_entries_without_agent_and_is_first_match`. (@houko)
- **CI on `main` restored** — three regressions had main red since #3576: (1) `cargo fmt` drift across `librefang-cli`, `librefang-kernel`, and `librefang-api` workflow operator tests; (2) `test_mcp_http_rehydrates_caller_context_from_agent_header` panicking on the substring assertion because #3576 routed the no-`X-LibreFang-Agent-Id` path through `ToolError::Internal("caller agent id missing — dispatcher did not attribute …")` — but the MCP HTTP route legitimately allows that None path for external clients, so the user-recoverable mapping is `ToolError::MissingParameter("agent_id")` (lifts to `LibreFangError::InvalidInput` → HTTP 400, not `Internal` → 500). The operator-facing per-tool diagnostic is preserved via a `tracing::warn!` next to the constructor. The `cron.rs` unit test was updated in lock-step; the `error-contracts.md` migration note was corrected; (3) `xtask/baselines/openapi.sha256` was stale after a recent `openapi.json` regen — re-baselined via `cargo xtask schema-check gen`. No source-of-truth `openapi.json` / `sdk/` bytes changed; only the schema digest. (@houko)
- **Sidecar polling adapters honour `Retry-After` on 429** (follow-up to #5301, then expanded to cover sibling adapters discovered to share the same gap) — the freshly-migrated `librefang.sidecar.adapters.nextcloud`, `bluesky`, `mastodon`, `rocketchat`, and `ntfy` all shipped with the same defect: the generic exponential-backoff loop (1 s → 60 s, or 1 s → 120 s for ntfy SSE reconnect) ignored `Retry-After`, so when the upstream returned 429 (Nextcloud OCS bruteforce throttle, Bluesky / Mastodon / Rocket.Chat REST rate limit, or ntfy per-topic publish quota) the producer thread / publish loop kept probing inside the server-side block window and extended the throttling. Each adapter now (a) threads response headers through its HTTP helper(s) (`_http` / `_post_json` / `_get_json` / inlined `urlopen` paths) with lowercase-normalised keys, (b) adds a `_retry_after_secs(headers)` static helper that parses seconds-form `Retry-After` with floor 1 s, cap `MAX_BACKOFF_SECS`, and falls back to a per-adapter `RETRY_AFTER_DEFAULT_SECS = 30.0` when the header is absent or unparseable, and (c) detects 429 at every reachable call site — `_verify_credentials`, channel / room discovery, polling, and outbound posting — sleeping the indicated interval and raising so the outer backoff pauses before its next pass (discovery returns empty since the next iteration retries on its own; ntfy SSE subscribe + publish do the same sleep-then-raise as the polling adapters). Verification: 36 new pytest cases across `sdk/python/tests/test_{nextcloud,bluesky,mastodon,rocketchat,ntfy}_adapter.py` — 7 for nextcloud (existing), plus 7 for bluesky, 8 for mastodon (also corrected one pre-existing test that relied on `_FakeUrlopen` returning a 5xx instead of raising `HTTPError`, the real `urlopen` behaviour), 8 for rocketchat, and 6 for ntfy — assert each code path honours the header, falls back when absent, and the existing 471 tests stay green (507 total across the sidecar test suite). Also retroactively documents the silent endpoint bug-fix that landed with #5301: the Rust adapter polled `/ocs/v2.php/apps/spreed/api/v4/room/<token>/chat` (`crates/librefang-channels/src/nextcloud.rs` lines 273-276 on `89dbd0b5^`), an endpoint the Talk OCS API does not expose for incoming chat — its own `api_send_message` at line 136 already used `/api/v1/chat/<token>`, which is the documented chat endpoint, and which the sidecar uses for both poll and post. Inbound polling on the Rust adapter was likely silently broken (404 / empty body) for any operator using it; the sidecar transparently fixed this on migration. `discord`, `reddit`, and `telegram` were audited and already honoured `Retry-After` (`_parse_retry_after` / `_retry_after_secs` / `_extract_retry_after` helpers respectively); `gotify`, `twitch`, and `webhook` are not applicable (push-only / IRC / inbound-only). (@houko)

## [2026.5.17] - 2026-05-17

_76 PRs from 5 contributors since v2026.5.12-beta.11._

### Highlights

- **Workflow operator nodes** — Wait, Gate, Transform, Branch, and human-in-the-loop pause/resume steps bring full orchestration control to multi-step workflows, with inline image display and rich invocation support
- **Per-agent compaction & prompt-cache tuning** — agents can now configure context compaction thresholds and Anthropic prompt-cache breakpoint strategy directly in `agent.toml`, reducing token costs on long sessions
- **On-demand tool/skill loading and declarative triggers** — tools and skills load only when needed, and `[[triggers]]` can now be declared directly in `agent.toml`, cutting startup overhead and simplifying agent configuration
- **Async task tracker and training exporters** — a kernel-level async task registry with W&B, Tinker, and Atropos trajectory exporters enables continuous learning pipelines from agent runs
- **Audio transcription and voice routing fixes** — inbound channel audio auto-transcribes when enabled, outbound OGG/Opus correctly routes via `sendVoice`, and per-channel proxy configuration is now supported

### Added

- Show skill descriptions in agent Skills tab (#5013) (@houko)
- Display generated images inline in workflow run view (#5015) (@houko)
- File_read deduplication — stub repeated reads of unchanged files (#5016) (@houko)
- Per-channel proxy configuration (#4795) (#5019) (@houko)
- Per-agent compaction settings in agent.toml (#4976) (#5020) (@houko)
- Prompt-cache breakpoint strategy for Anthropic (#5021) (@houko)
- Dual-layer compression — gateway safety net before agent loop (#4972) (#5022) (@houko)
- Reference existing registry agents in workflow steps (#5023) (@houko)
- Async task tracker — kernel registry + event injection + wake-idle (#4983) (#5033) (@houko)
- New crate + W&B + Tinker + Atropos exporters (#3331) (#5034) (@houko)
- Non-agent operator nodes — Wait, Gate, Transform, Branch (#4980) (#5035) (@houko)
- Skill/tool finder in agent creation dialog (#5049) (#5066) (@houko)
- ProviderExhaustionStore substrate + AuxClient consumer (#4807) (#5067) (@houko)
- Declarative [[triggers]] in agent.toml (#5014) (#5068) (@houko)
- On-demand tool/skill loading (#5073) (@houko)
- Rich workflow invocation (#4982) (#5075) (@houko)
- Document ElevenLabs and validate voice_id at driver boundary (#5078) (@houko)
- Operator step mode — human-in-the-loop pause + resume (#4977 step 1/N) (#5108) (@houko)

### Fixed

- Keep ANTHROPIC_API_KEY in subprocess env (#4967) (@f-liva)
- Surface CLI stderr on stdin write failure (#4974) (@f-liva)
- Add schedule field to PATCH partial update path (#4986) (@DaBlitzStein)
- Allow deleting connection arrows between steps (#4978) (#4993) (@houko)
- Scope ApprovalRequested delivery to requesting agent's adapters/recipients (#4985) (#4994) (@houko)
- Allow media read tools to access kernel staging dir (#4981) (#4995) (@houko)
- Accept absolute workspace paths under workspaces_root (#4991) (#4996) (@houko)
- Route audio/ogg outbound via sendVoice (#4959) (#4998) (@houko)
- Auto-transcribe inbound channel audio when [media].audio_transcription = true (#4975) (#4999) (@houko)
- Node delete via context menu writes history and cascades edges (#5007) (@houko)
- Keep ANTHROPIC_* env vars when spawning CLI (#5008) (@houko)
- Override account_id() in non-Telegram multi-bot adapters (#5009) (@houko)
- Magic-byte sniff outbound audio/ogg to catch mislabeled payloads (#5010) (@houko)
- Route approvals to bound chats when default_agent is None (#5002) (#5011) (@houko)
- Downgrade OGG Vorbis to sendDocument; only Opus is valid for sendVoice (#5012) (@houko)
- Unblock Windows test lane (7 assertions / platform divergences) (#5024) (@houko)
- Stabilise diagnose_stdin macOS test (#5024 follow-up) (#5026) (@houko)
- Resolve ioreg / reg.exe by absolute path (#5025) (#5031) (@houko)
- Schedule field PATCH + actual_provider wiring + warn_ws_proxy_bypass gating (supersedes #4986) (#5036) (@houko)
- Unblock main — docs TS 6 + lettre RUSTSEC-2026-0141 (#5056) (@houko)
- Guard pr-status-labels filter against undefined check_run entries (#5057) (@houko)
- Unify init() key resolution with resolve_master_key() (#5074) (@houko)
- Add input_schema: None to Workflow literals after #5075 (#5105) (@houko)
- Add input_schema: None to workflow_with_single_op_step test helper (#5107) (@houko)
- Apply per-agent tool_allowlist/blocklist on tools/list (#5101) (#5109) (@houko)
- Invalidate budget/usage on send and snapshot-prefix on session override (#5147) (@houko)
- Raise persisted-session message cap from 200 to 2000 (#5148) (@houko)
- Preserve other config sections during default-model write (#5150) (@houko)
- Deny unknown fields in request DTOs to catch body typos (#5131) (#5151) (@houko)
- Reuse reqwest::Client across fan-out fires; skip engine on empty targets (#5152) (@houko)
- Preserve nested serde aliases + deny unknown fields on repeated tables (#5129, #5130) (#5154) (@houko)
- Clamp negative age in stale-run recovery to survive NTP backstep (#5155) (@houko)
- Replace SSRF substring stub with parsed-URL allowlist (#5156) (@houko)
- Require non-empty sub claim on IdTokenClaims (#5128) (#5157) (@houko)
- Refuse to run hook when concurrency semaphore is closed (#5158) (@houko)
- Block Azure IMDS alternative 192.0.0.192 in MCP SSRF helper (#5159) (@houko)
- Reject peer: key prefix and colon-bearing peer_id at substrate boundary (#5161) (@houko)
- Propagate DB error from agent deletion (#5117) (#5163) (@houko)
- Bind named params at run time (#5170) (@houko)
- Give the root route an explicit notFoundComponent (#5171) (@houko)
- Cap sysinfo at 0.38 to honor 1.94.1 MSRV (#5183) (@houko)

### Changed

- #3710 god-crate split — 5 standalone crates + oauth/wasm collapse (#5053) (@houko)
- Typed SandboxError replaces anyhow (#3576) (#5077) (@houko)
- Drop pass-through KernelError wrapper (#3576 wedge) (#5110) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Documentation

- Add Auto-Evolution Mode page (companion to registry#94) (#5029) (@houko)
- Trajectory format RFC (#3330) (#5032) (@houko)
- Clarify extraction_model provider/model format (#5059) (#5062) (@leszek3737)
- Correct historical attribution in README (#3710 follow-up) (#5100) (@houko)
- Sync DEFAULT_MAX_HISTORY_MESSAGES default (60, not 40) (#5153) (@houko)

### Maintenance

- Bump the actions-minor-patch group with 4 updates (#4988) (@app/dependabot)
- Bump apple-actions/import-codesign-certs from b2e261033a9e248f91a9b57201e8d1e12b15a24e to 5142e029c445c10ffc7149d172e540235a065466 (#4989) (@app/dependabot)
- Bump actions/setup-python from 5 to 6 (#4990) (@app/dependabot)
- Install rustc on cli_npm/cli_pypi to fix sysinfo MSRV (#4992) (@houko)
- Bump the dashboard-minor-patch group in /crates/librefang-api/dashboard with 9 updates (#5027) (@app/dependabot)
- Bump the web-minor-patch group in /web with 7 updates (#5028) (@app/dependabot)
- Bump typescript from 5.9.3 to 6.0.3 in /docs (#5052) (@app/dependabot)
- Update IGNORE path after #5053 god-crate split (#5102) (@houko)
- Rustfmt mcp_tools_list_allowlist_test.rs (fix main CI) (#5146) (@houko)

</details>


## [2026.5.12] - 2026-05-12

_95 PRs from 5 contributors since v2026.5.8-beta.10._

### Highlights

- **Workflow Engine** — agents can now start, cancel, and monitor multi-step workflows natively via new tools (`workflow_start`, `workflow_cancel`, `workflow_list`, `workflow_status`), with run history persisted to SQLite, configurable retry backoff, timeouts, and event triggers that fire workflows directly
- **Multi-Instance Dashboard Management** — manage multiple LibreFang instances from a single dashboard UI
- **Redesigned Memory Page** — the Memory dashboard is rebuilt around a per-agent rail with tabs, and Auto-Dream settings move there from Settings; proactive memory extraction now supports provider-qualified model IDs and per-agent overrides
- **Messaging & Channel Improvements** — full P1 parity for reactions, threads, streaming, redaction, edits, and media; channel messages now mirror into inbound-routing sessions; cron/autonomous fires are labeled with `[Scheduled trigger]` in history
- **Security & Fetch Hardening** — new SSRF-safe `fetch_url_bytes` helper with redirect re-validation, `web_fetch_to_file` for downloading URLs directly to disk, streaming abort on prompt-leak detection, and at-rest token hashing for workflow credentials

### Added

- Defer rate-limit failures + claim verifier (#4754) (@f-liva)
- Buffer text-only group messages skipped at gating (#4755) (@f-liva)
- Configurable burst ratio with NaN guard and tests (#4830) (@DaBlitzStein)
- P1 parity (reactions, threads, streaming, redaction, edit) + media (#4831) (@neo-wanderer)
- Persist workflow runs to SQLite (#4838) (@DaBlitzStein)
- Render per-parameter form fields for workflow runs (#4839) (@DaBlitzStein)
- Separate IMAP and SMTP credentials in EmailConfig (#4841) (@DaBlitzStein)
- Add bounded SSRF-safe fetch_url_bytes helper (#4846) (@houko)
- Catalog-driven ReasoningEchoPolicy with substring fallback (#4842) (#4863) (@houko)
- Multi-instance management from the dashboard (#4837) (#4865) (@houko)
- Tls_root_ca_path + tls_accept_invalid_certs for self-hosted IMAP (#4877) (#4889) (@houko)
- [proactive_memory] extraction_model honours provider-qualified ids (#4871, #4870) (#4892) (@houko)
- Add workflow_list and workflow_status native tools (#4902) (@houko)
- Add run cancel, total timeout, retry backoff (#4844) (#4906) (@houko)
- Allow event triggers to fire workflows directly (#4844) (#4909) (@houko)
- Add workflow_start and workflow_cancel native tools (#4844) (#4910) (@houko)
- At-rest token hashing, typed errors, pause/resume HTTP endpoints, async POST /run (#4911) (@houko)
- Accept .oga audio extension in media_transcribe tool (#4919) (@f-liva)
- Make token burst ratio configurable per agent (#4921) (@DaBlitzStein)
- Add mcp_disabled field to AgentManifest (#4930) (@houko)
- Mirror channel_send into inbound-routing session (#4932) (@houko)
- Web_fetch_to_file — download URLs straight to disk (#4964) (@houko)

### Fixed

- Cache response_url per user to enable per-message replies (#4751) (@f-liva)
- Mark cron/autonomous fires with [Scheduled trigger] prefix (#4752) (@f-liva)
- Resilience pass — heartbeat, dedup, crash-safety, sweep race (#4759) (@f-liva)
- Allow same-eTLD+1 metadata endpoints at discovery (#4665, follow-up to #4779) (#4789) (@neo-wanderer)
- Channel=current uses main HEAD, not the tag's frozen commit (#4813) (@houko)
- Switch ollama provider to native Ollama API (#4810) (#4814) (@houko)
- Release --channel current works without `gh repo set-default` (#4816) (@houko)
- Channel=current dispatches against main, takes tag via input (#4817) (@houko)
- Unbreak main clippy on parse_github_owner_repo (#4819) (@houko)
- Use chrono for config-backup timestamp; drop deprecated libc::time_t (#4820) (@houko)
- Xcconfig shim for iOS signing; use apple-actions for cert (#4821) (@houko)
- Unit-fast lane should not error on binary-only crates (#4822) (@houko)
- Unblock iOS exportArchive + idempotent crates.io publish (#4827) (@houko)
- Pre-dispatch provider budget gate on all 3 dispatch paths (#4828) (@DaBlitzStein)
- Classify workflow retry backoff by error type (#4829) (@DaBlitzStein)
- Pin scheme on Rule 2 eTLD+1 acceptance (supersedes #4789) (#4848) (@houko)
- Persist workflow runs to SQLite (supersedes #4838) (#4849) (@houko)
- Case-insensitive retry classifier + honour Retry-After (supersedes #4829) (#4850) (@houko)
- Snapshot sock at sendOrEdit entry (supersedes #4759) (#4851) (@houko)
- Pre-dispatch provider budget gate + integration tests (supersedes #4828) (#4852) (@houko)
- Parse-time validation for default_burst_ratio + dup doc fix (supersedes #4830) (#4853) (@houko)
- Seed workflow param defaults + clarify {{var}} contract (supersedes #4839) (#4854) (@houko)
- Test fallback resolver for split email creds + regen schema golden (supersedes #4841) (#4855) (@houko)
- Round-trip reasoning_content for deepseek-v4-flash tool_calls (#4842) (#4856) (@houko)
- Drain pipes during wait to avoid >pipe-buffer deadlock (#4857) (@neo-wanderer)
- Re-validate redirect targets in fetch_url_bytes (security) (#4858) (@houko)
- Persist Paused state immediately at pause-transition site (#4859) (@houko)
- Channel-default key mismatch — resolver used Debug format (#4861) (@neo-wanderer)
- Redirect dashboard login to / instead of /dashboard (#4860) (#4862) (@houko)
- Persist PUT /api/budget to config.toml + hot-reload + dashboard read (#4797) (#4864) (@houko)
- Actionable error when stdio MCP runtime is missing (#4836) (#4867) (@houko)
- Keep iPad portrait on the desktop layout (#4873) (#4880) (@houko)
- Deliver ApprovalRequested events to channel adapters (#4875) (#4881) (@houko)
- Typed 429 retry + idempotent txn_id + edit size cap (#4831 follow-up) (#4882) (@houko)
- Backfill approval_audit.second_factor_used on upgrade (#4874) (#4883) (@houko)
- Real session summaries via aux LLM + per-agent proactive_memory override (#4869, #4870) (#4885) (@houko)
- Honour suppression for CLI/local providers + un-suppress on URL reconfigure (#4803) (#4886) (@houko)
- Raise DEFAULT_MAX_HISTORY_MESSAGES from 40 to 60 (#4891) (@houko)
- Stop the dashboard 401 spam on initial mount (#4893) (@houko)
- Make embedding & extraction model fields suggest options instead of being raw text inputs (#4894) (@houko)
- Switch embedding/extraction model fields to real <select> dropdowns (#4897) (@houko)
- Recognise known embedding models when provider is Auto-detect (#4900) (@houko)
- Batch history_fold LLM call + persist rewrites to session (#4866) (#4901) (@houko)
- Scope `/new` to the calling channel + purge JSONL on delete (#4868) (#4905) (@houko)
- Eradicate cascade scaffolding leak in agent replies (#4907) (@f-liva)
- Persist workflow definitions to disk on register/remove (#4920) (@DaBlitzStein)
- Unblock main coverage — /api/health/detail auth + workflow timeout overlay (#4928) (@houko)
- Abort streaming on incremental prompt-leak detection (#4931) (@houko)
- Sweep stale ACP UDS orphan tempfiles on bind (#4933) (@houko)
- Detect audio MIME via magic bytes / filename (#4934) (@houko)
- Allow shell_exec read commands against RO workspaces (#4935) (@houko)
- Memory store alias + peer-scoped /btw read fix + kv-write logs (#4936) (@houko)
- Per-session model override (#4898) (#4937) (@houko)
- Close gaps from #4907-#4910/#4920 audit (#4938) (@houko)
- Unblock Security audit — Next.js patch + tanstack/history GHSA (#4944) (@houko)
- Align status fields, fix OFP-disabled empty-state (#4945) (@houko)
- Add missing model_override in Session literal (#4955) (@houko)
- Exclude cache-read hits from burst limit; sort agent-detail skills (#4957) (@houko)
- Propagate DB error from agent deletion instead of false 200 OK (#5117) (@houko)

### Changed

- Move Auto-Dream runtime panel from Settings to Memory page (#4890) (@houko)
- Fold Auto-Dream into per-agent memory card (#4896) (@houko)
- Redesign /dashboard/memory around an agent rail + tabs (#4904) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Documentation

- Clarify manifest allowlist vs MCP server registry split (#4845) (@houko)
- Correct skill_workshop default to OFF in agent guide (#4872) (@neo-wanderer)
- Require fixing review nits in-PR instead of punting to follow-ups (#4879) (@houko)

### Maintenance

- Clarify, clean up, and loosen the AI agent rules (#4815) (@houko)
- Regenerate SDKs + rustfmt Rust output (#4887) (#4888) (@houko)
- End-to-end inbound POST → cache → send round-trip (#4929) (@houko)
- Bump the cargo-minor-patch group with 14 updates (#4946) (@app/dependabot)
- Bump opentelemetry from 0.31.0 to 0.32.0 (#4947) (@app/dependabot)
- Bump r2d2_sqlite from 0.33.0 to 0.34.0 (#4950) (@app/dependabot)
- Bump pulldown-cmark from 0.10.3 to 0.13.3 (#4951) (@app/dependabot)
- Bump sysinfo from 0.38.4 to 0.39.1 (#4952) (@app/dependabot)

### Reverted

- Pin opentelemetry to 0.31 (#4947 broke main) (#4953) (@houko)

</details>


## [2026.5.8] - 2026-05-08

_68 PRs from 5 contributors since v2026.5.6-beta.9._

### Highlights
- **New Dashboard & UI Refinements** — Adds a dedicated dashboard, resolves 159+ UI bugs and accessibility gaps, and fixes summarize-and-trim compaction for persistent agent sessions.
- **Durable Knowledge Vault** — Introduces an isolated v1 knowledge vault with lazy initialization to fix silent setup successes and load secrets at boot for cross-restart persistence.
- **Native Editor Integration** — Implements an Agent Client Protocol adapter and SSH/Daytona tool-exec backends for seamless editor-to-agent workflow connections.
- **Passive Skill Capture & DM Improvements** — Launches a post-turn capture pipeline for automated skill development and exposes sender identity in direct message prompts.
- **Performance Optimizations** — Batches per-agent KV lookups via useQueries to enhance dashboard and agent response speeds.

### Added

- Tool-exec backend trait + SSH and Daytona impls (#3332) (#4677) (@houko)
- Scaffold durable knowledge vault — isolated mode v1 (#3329) (#4712) (@houko)
- Closes #3328 — passive after-turn capture pipeline (#4741) (@houko)
- Agent Client Protocol (ACP) adapter for native editor integration (#4742) (@houko)
- Expose sender identity in DM prompts, not just groups (#4666) (#4776) (@houko)
- Add dashboard (#4780) (@houko)
- User-editable per-model capability overrides (#4745) (#4781) (@houko)

### Fixed

- Terminal page reconnect loop on container hosts (#4675) (#4681) (@houko)
- Expose every KernelConfig section in single-page UI (#4682) (@houko)
- Summarize-and-trim compaction mode for Persistent sessions (#3693) (#4683) (@houko)
- Close DrawerPanel on parent-driven isOpen=false (#4687) (#4691) (@houko)
- Expand leading ~ in stdio transport args (#4680) (#4692) (@houko)
- Hub install/uninstall surface stale state across all 4 hubs (#4689) (#4696) (@houko)
- Regenerate schema baselines as part of release/lts bump (#4697) (@houko)
- PID fallback and clearer error when restart hits 401 (#4693) (#4698) (@houko)
- Deterministic two-phase driver for find_by_name_is_atomic_under_concurrent_register_and_remove (#4704) (#4705) (@houko)
- Reload_config must reject invalid TOML, not silently swap to defaults (#4664) (#4711) (@houko)
- Resolve 35 UI bugs and review follow-ups across 10 pages (#4718) (@leszek3737)
- Resolve 80+ bugs, a11y gaps, and i18n misses across 18 page components (#4719) (@leszek3737)
- Toast refresh errors in AnalyticsPage (#4718 review L1) (#4724) (@houko)
- Drain in-flight workflow runs on graceful shutdown (#3335) (#4725) (@houko)
- DrawerPanel parent-close must check slot ownership (#4714) (#4727) (@houko)
- Resolve 44 confirmed UI bugs across 13 dashboard components (#4731) (@leszek3737)
- A11y improvements and UI bugfixes (#4733) (@leszek3737)
- State-correctness and a11y bugs in UI primitives (#4734) (@leszek3737)
- A11y polish and UX fixes across UI components (#4735) (@leszek3737)
- Scope PushDrawer focus traps to their actual viewport (#4734 followup) (#4737) (@houko)
- Close SSRF gaps in cron webhook delivery (#4732) (#4739) (@houko)
- Load secrets.env at boot so dashboard-saved keys survive restart (#4701) (#4740) (@houko)
- Unblock Dashboard / Mobile / Docker on main (#4744) (@houko)
- Correlate daemon logs with agent.id / session.id across run_agent_loop and supervised tasks (#4761) (@neo-wanderer)
- Pipe prompt to CLI stdin instead of argv to avoid E2BIG (#4764) (@f-liva)
- Block CLI progress placeholders + add stream_to_channel toggle (#4765) (@f-liva)
- Default opt-in + bell/tab navigation (#3328 follow-up) (#4775) (@houko)
- Align tool_runner test assertions with new pre-ACP path guard (#4777) (@houko)
- Allow unused_mut on chromium_candidates() for android/ios builds (#4778) (@houko)
- Allow same-eTLD+1 token endpoint for cross-domain OAuth proxies (#4779) (@houko)
- Kill SIGPIPE 141 noise in PreToolUse hooks (#4782) (@houko)
- Bump corepack so pnpm 10.x signature check passes (#4784) (@houko)
- Escape literal {name} in providers route assert message (#4786) (@houko)
- Bump dashboard builder node to 20.20.2-alpine for vite 8 / rolldown engines (#4787) (@houko)
- Drop install_integration fixture after boot to dodge sync_registry orphan cleanup (#4791) (@houko)
- Lazy-init vault.enc on first set() — fix install_integration silent-success (#4793) (@houko)
- Add deterministic catalog seed for mock kernel — fix capability_override flake (#4796) (@houko)
- Expose ModelCatalog::from_entries outside cfg(test) — unbreak main (#4798) (@houko)
- Channels bridge: fail closed on non-2xx in `download_file_to_blocks` / `download_image_to_blocks`. Previously a 4xx/5xx response body (e.g. Synapse's 45-byte `M_NOT_FOUND` JSON envelope on the frozen `/_matrix/media/v3/download` endpoint) was streamed to disk as `<uuid>.<ext>` and surfaced to the agent as a corrupt file.
- Matrix adapter: switch inbound media downloads to MSC3916 authenticated `/_matrix/client/v1/media/download/{server}/{mediaId}`, which Synapse 1.100+ requires (default Synapse no longer serves the legacy unauthenticated path). The bot's access token is attached via a new `ChannelAdapter::fetch_headers_for(url)` hook, gated by a homeserver-host match so the credential cannot leak to model-controlled URLs.
- Matrix adapter: flush the placeholder edit on the first non-empty delta instead of waiting for the 1500ms / 256-char debounce. Previously the kernel's `\n\n🔧 toolname\n\n` progress markers were ~35 chars each, so tool-only sequences (rapid tool calls with no LLM prose between them) never crossed the size budget and never re-fired the time check, leaving the user staring at `…` until the agent loop ended. Brings parity with telegram's "first delta becomes the message body" UX.
- Channels bridge: surface the kernel `ToolUseStart` phase as a `LifecycleReaction` to the channel adapter (closes the architectural gap where `librefang-api/src/channel_bridge.rs` filtered every `PhaseChange` event except `context_warning` to `_ => {}`). The streaming dispatch's tee task now sniffs the `\n\n🔧 toolname\n\n` text marker that the api bridge already emits for that phase and fires `send_lifecycle_reaction(... AgentPhase::ToolUse)` so adapters that render reactions (Matrix's redact-previous chain, Slack's reactji) flip the trigger-message reaction to ⚙️ for the duration of the call. The inline text marker is preserved — reactions are an additional surface, not a replacement. Refactor: drain task moved from `tokio::spawn` to a `tokio::join!` sibling so it shares the dispatch task's borrow of `&dyn ChannelAdapter` (avoids the `'static` constraint that would otherwise force an `Arc<dyn ChannelAdapter>` plumbing change).
- Channels bridge: bump `send_lifecycle_reaction` failure logging from `debug!` to `warn!`. The previous level hid per-room rate-limit drops on Matrix (`M_LIMIT_EXCEEDED`) where the trailing `✅ Done` reaction was being silently swallowed at default verbosity, making the lifecycle-reaction feature look broken even when it was working. WARN surfaces the actionable diagnosis: "your homeserver is rate-limiting the bot".
- Matrix adapter: tighten streaming edit cadence from 1500ms / 256-char debounce to 700ms / 96-char so progressive deltas remain visible after the first-delta flush. Previous values produced a "placeholder + first + final" cadence on typical 2-3s LLM responses (~150 chars/sec), so the response felt like it arrived in one shot once the placeholder was replaced. New values yield ~4-5 visible edits over the same window — closer to Telegram's 1000ms feel — while still staying inside Synapse's `rc_message: 5/s, burst 60` budget that the operator tuning lifted in this session.
- Matrix adapter: replace the 429-retry string-match (`format!("{e}").contains("429")`) with a typed `MatrixApiError::RateLimited { retry_after_ms }`, and reuse a single `txn_id` across both attempts inside `api_edit_event_with_retry`. The string-match was fragile (any error whose message coincidentally contained "429" would mistrigger); the typed enum is internal-only and erases back into `Box<dyn Error + Send + Sync>` via `MatrixApiError::into_boxed` so public call sites are unchanged. The txn_id reuse closes an idempotency hole: Matrix dedupes on `(sender, txn_id)`, so a 429 that masks a quietly-successful first PUT would have landed a duplicate `m.replace` event in the room — now the second attempt either hits the same server-side dedup slot or wins fresh. `Retry-After` (delta-seconds form) is honored and clamped to `[100ms, 5s]` so a missing / zero / overlong hint doesn't either spam the homeserver or stall streaming. (#4831 follow-up) (@houko)
- Matrix adapter: defensively truncate `api_edit_event` inputs to `MAX_MESSAGE_LEN` via `librefang_types::truncate_str` (UTF-8 safe). An edit can only target one event_id so we cannot split into multiple events here — callers that need every byte preserved (streaming overflow) already split BEFORE calling. The cap stops the `send(EditInteractive)` / `send(DeleteMessage)` paths, which today feed `text + button-hint suffix` straight through, from producing an oversized `m.room.message` that Synapse would reject with a hard-to-debug 413 / `M_TOO_LARGE`. (#4831 follow-up) (@houko)
- Channels bridge: restore the `send_lifecycle_reaction` rustdoc summary line ("Send a lifecycle reaction (best-effort, non-blocking for supported adapters).") that was accidentally re-attached to `extract_tool_marker_name` during #4831, leaving `send_lifecycle_reaction` summary-less and `extract_tool_marker_name` claiming to "Send a lifecycle reaction…". rustdoc summary indexing now matches the function's actual job. Doc-only — no behaviour change. (#4831 follow-up) (@houko)
- Channels bridge: re-converge `download_image_to_blocks` on the shared `http_client::fetch_url_bytes` helper instead of carrying its own SSRF guard + content-length pre-check + chunk-accumulator loop. PR #4831 forked the helper inline because it needed to attach MSC3916 auth headers and the helper didn't yet support them. Now `fetch_url_bytes` / `fetch_url_bytes_unchecked` accept `extra_headers: &[(String, String)]`, so the image path collapses from ~105 LOC back to a single `match`. Telegram's three private-URL multipart-fallback call sites pass `&[]` and behave identically. Adds `fetch_url_bytes_unchecked_attaches_extra_headers` so a future regression that silently drops the headers (e.g. Matrix's Bearer token) fails loud. (#4831 follow-up) (@houko)
- Channels: `[channels].file_upload_max_bytes` makes the Matrix and Telegram outbound media upload cap operator-configurable. New `ChannelsConfig.file_upload_max_bytes: u64` field (default 50 MiB to match the previous hardcoded constants; deliberately separate from `file_download_max_bytes` since inbound `server → agent → disk` and outbound `bot → server upload` are different layers, and binding them would let an operator override the inbound knob and silently constrain outbound replies). `MatrixAdapter` and `TelegramAdapter` gain `with_max_upload_bytes(usize)` builders, plumbed in by `start_channel_bridge_with_config` so a single config knob applies to every bot instance. Pinned by `test_with_max_upload_bytes_overrides_default_cap` — a 1 KiB override rejects a 2 KiB upload and the rejection message names the override, so a regression where the builder is silently dropped fails loud rather than re-introducing the hardcoded 50 MiB. (#4831 follow-up) (@houko)

### Changed

- Replace Arc<Mutex<Connection>> with r2d2 connection pool (#3378 part 2) (#4685) (@houko)
- Align ProvidersPage with ChannelsPage add-via-picker pattern (#4708) (@houko)
- Split kernel/mod.rs into per-cluster files (#3744 phases 1-3) (#4713) (@houko)
- Harden shell, extract modal, fix React perf and error handling (#4717) (@leszek3737)
- KernelApi trait + Arc<dyn KernelApi> AppState (#3566) (#4726) (@houko)
- Decompose LibreFangKernel god struct into 13 subsystems (#3565) (#4756) (@houko)
- Migrate inherent forwards to *SubsystemApi traits (#3565 follow-up) (#4766) (@houko)
- Manifest-first control plane — types spine + cached vault facade (#4783) (@houko)
- Install-path vault facade + hook regex narrowing (#4788) (@houko)

### Performance

- Batch per-agent KV lookups via useQueries (#4722) (#4738) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Documentation

- Document DrawerPanel ownership check in file-level sync model (#4727 followup) (#4729) (@houko)

### Maintenance

- Include PR number, failed jobs, and step names (#4694) (@houko)
- Refresh openapi.sha256 to match merged v2026.5.6-beta.9 openapi.json (#4695) (@houko)
- Auto-stage refreshed openapi.sha256 when openapi.json is committed (#4700) (@houko)
- Bump the web-minor-patch group in /web with 6 updates (#4720) (@app/dependabot)
- Bump the dashboard-minor-patch group in /crates/librefang-api/dashboard with 6 updates (#4721) (@app/dependabot)
- Fix PR Status Labels 403 by splitting pull_request_review trigger (#4746) (@houko)
- Pin pnpm via package.json so cache: pnpm save step works (#4758) (@houko)
- Ignore graphify-out/ (#4762) (@neo-wanderer)
- Bump the docs-minor-patch group in /docs with 6 updates (#4769) (@app/dependabot)
- Bump postcss-focus-visible from 10.0.1 to 11.0.0 in /docs (#4770) (@app/dependabot)
- Bump @sindresorhus/slugify from 2.2.1 to 3.0.0 in /docs (#4771) (@app/dependabot)
- Bump marked from 16.2.1 to 18.0.3 in /docs (#4772) (@app/dependabot)

</details>


## [2026.5.6] - 2026-05-06

_310 PRs from 3 contributors since v2026.5.2-beta8._

### Added

- Add schema drift check with sha256 baselines (#4367) (@houko)
- Surface external tip-anchor status in /api/audit/verify (#4388) (@houko)
- Announce health-status flips via aria-live (#4405) (@houko)
- Add message_coalesce_window_ms knob (#4145) (#4441) (@houko)
- Allow obsidian:// and obsidian-advanced-uri:// in markdown links (#4456) (@neo-wanderer)
- Trace session_mode resolution to expose channel/cron overrides (#3692) (#4489) (@houko)
- Expose existing budget/LLM metrics on /api/health/detail (#3776) (#4494) (@houko)
- Surface agent_id in HTTP access log via response extensions (#3511) (#4504) (@houko)
- Vault startup sentinel + rotate-key + audit on crypto failure (#3651) (#4514) (@houko)
- Trusted_proxies + trust_forwarded_for for real-client-IP resolution (#4534) (@neo-wanderer)
- Render historical thinking blocks on session reload (#4542) (@neo-wanderer)
- Surface caller IDs as x-librefang-* headers (#4548) (@neo-wanderer)
- Add metrics for queue lanes, MCP reconnect, LLM 429, tool calls (#3495) (#4560) (@houko)
- Idempotency-Key on /api/agents + /api/a2a/send (#3637 1/N) (#4565) (@houko)
- Expand agent_id access-log coverage to hot-path routes (#3511) (#4567) (@houko)
- Native task_status(task_id) tool (#4549) (#4570) (@houko)
- Maintainer-namespaced prompts in .claude/prompts/ (#3308) (#4583) (@houko)
- LIBREFANG_LOCAL_CHECK_MODE throttle escape (#3301) (#4585) (@houko)
- Ed25519 signing across workers + daemon TOFU resolver (#4600) (@houko)
- Standardize list pagination + error envelope (#3639) (#4629) (@houko)
- Persist canonical agent UUID across respawns (#4614) (#4630) (@houko)
- Access log emits structured agent_id / session_id (#3511) (#4633) (@houko)
- Wire progress.rs into long-running commands (#3306) (#4642) (@houko)
- Emit x-librefang-* trace headers from Anthropic/Gemini/ChatGPT (#4637 1/N) (#4644) (@houko)
- Idempotency-Key on hand/plugin/webhook (#3637 2/N) (#4645) (@houko)
- CI + runtime supply-chain audit for marketplace artifacts (#3333) (#4649) (@houko)
- Tool-result artifact spill + read_artifact tool (#3347 1/N) (#4651) (@houko)
- Emit x-librefang-* trace headers from Bedrock/Vertex/Copilot (#4637 2/N) (#4653) (@houko)
- Trace identifiers via env vars on CLI-style drivers (#4637 3/N) (#4658) (@houko)
- Close out tool-result context budget umbrella (#3347) (#4660) (@houko)
- Incognito chat mode (#4073) (#4662) (@houko)
- Collapse chat tool calls into a per-message popup (#4672) (@houko)

### Fixed

- Propagate stream send errors as backpressure (#4300) (@houko)
- Drop config_reload_lock before LLM call (#3564) (#4302) (@houko)
- Meet WCAG AA contrast in CommandPalette hints (#4303) (@houko)
- Translate ShortcutsHelp modal strings (#4304) (@houko)
- Drop needless ref binding in restrict_to match (#4305) (@houko)
- Query peer registry live so /api/peers reflects current peers (#4306) (@houko)
- Route ChatPage and ProvidersPage through queries/mutations layer (#4307) (@houko)
- Typed failover_reason replaces substring matcher (#4309) (@houko)
- Register 12 missing endpoints in openapi.json (#4310) (@houko)
- Typed placeholders for free-form JSON responses (refs #3396) (#4314) (@houko)
- Satisfy clippy doc_lazy_continuation and needless_borrows in session tests (#4328) (@houko)
- Drain client request before responding in redirect test (#4344) (@houko)
- Standardize /api/peers on PaginatedResponse envelope (#4355) (@houko)
- Return mutated GoalItem from PUT /api/goals/{id} (#4356) (@houko)
- Goals list returns PaginatedResponse (#3842) (#4358) (@houko)
- Return updated ResourceQuota from PUT /api/budget/agents/{id} (#4360) (@houko)
- Standardize /api/usage on PaginatedResponse envelope (#4362) (@houko)
- List returns PaginatedResponse (#3842) (#4363) (@houko)
- Return updated PromptExperiment from start/pause/complete (#4364) (@houko)
- Activate version returns PromptVersion entity (#3832) (#4365) (@houko)
- Standardize /api/audit/* on PaginatedResponse envelope (#4368) (@houko)
- Skills/hands lists return PaginatedResponse (#3842) (#4371) (@houko)
- Channels list returns PaginatedResponse (#3842) (#4372) (@houko)
- Update returns Workflow entity (#3832) (#4373) (@houko)
- Canonicalize sessions list envelopes (#3842) (#4374) (@houko)
- Pause and resume return live HandInstance (#3832) (#4375) (@houko)
- List endpoints return PaginatedResponse (#3842) (#4376) (@houko)
- List returns PaginatedResponse (#3842) (#4377) (@houko)
- Return live tools config from PUT /api/agents/{id}/tools (#3832) (#4378) (@houko)
- Standardize /api/comms/events on PaginatedResponse envelope (#3842) (#4379) (@houko)
- Install returns full HandDefinition entity (#3832) (#4380) (@houko)
- Canonicalize /api/network/trusted-peers list envelope (#4381) (@houko)
- Return canonical memory config from PATCH /api/memory/config (#4382) (@houko)
- Canonical PaginatedResponse envelope for /api/schedules (#4383) (@houko)
- Return persisted ModelOverrides from PUT overrides (#3832) (#4384) (@houko)
- Restore typed PythonError variant (#3711) (#4389) (@houko)
- Close spawn-before-publish race in AgentRegistry (#4393) (@houko)
- Make Sessions Play button actually open the session in chat (#4292) (#4428) (@houko)
- Warn in lint when hook integrity hashes are missing (#4036) (#4431) (@houko)
- Lock Conversation tab to per-agent sessions endpoint (#4294) (#4432) (@houko)
- Stop loading stale messages on session switch (#4295) (#4433) (@houko)
- Emit `active` on /api/sessions rows (#4290) (#4437) (@houko)
- Preserve URL hand-agent + sessionId across bootstrap race (#4296) (#4438) (@houko)
- Derive strict-mode allowlist from KernelConfig schema (#4440) (@houko)
- Align /api/agents/{id}/sessions `active` with running-loop semantics (#4442) (@houko)
- Give ChannelsConfig a non-zero file_download_max default (#4476) (@houko)
- Allowlist channel download dir for file_read/file_list (#4478) (@houko)
- Honor file_download_dir across all upload sites (#4479) (@houko)
- Extract PDF/text content for downloaded attachments (#4480) (@houko)
- Honor named-workspace prefixes in media/image tools (#4481) (@houko)
- Wire init wizard Smart Router into config (#4466) (#4482) (@houko)
- Align with PaginatedResponse + return-entity envelope changes (#4483) (@houko)
- Auto-inject [integrity] hashes at registry publish (#4036) (#4484) (@houko)
- Bound contains-style tool_call heuristics to short responses (#4028) (#4485) (@houko)
- Thread parent_session_id through fork LoopOptions to fix TOCTOU race (#4291) (#4487) (@houko)
- Enrich PDFs sent with octet-stream MIME (refs #4448) (#4492) (@neo-wanderer)
- Return 412/502 for channel test failures instead of 200 (#3507) (#4497) (@houko)
- Harden TOTP/recovery code inputs against shoulder-surf (#3551) (#4498) (@houko)
- Surface cron persist failures with 500 instead of silent revert (#3515) (#4499) (@houko)
- Make DELETE handlers idempotent and fix webhook_wake auth status (#3509) (#4501) (@houko)
- Time out slash-command WS listener and surface dropped commands (#3550) (#4503) (@houko)
- Close en/zh locale parity gap (#3557) (#4509) (@houko)
- Pin Docker bases, add HEALTHCHECK, validate entrypoint env (#3556) (#4510) (@houko)
- Switch sessions_fts to content-linked + add triggers + backfill (#3548) (#4515) (@houko)
- Post-merge regressions for #3571 #3603 #3692 #3776 (#4517) (@houko)
- Clear baseline main-red blocking 24h merged PR queue (#4520) (@houko)
- Post-merge clippy regressions from 2026-05-03 batch (#4521) (@houko)
- Exempt PWA static files (manifest, sw, icons) from auth allowlist (#4529) (@neo-wanderer)
- Canonicalize last 3 list envelopes — close out #3842 (#4538) (@houko)
- Async wrappers for kernel substrate calls (#3378 part 1) (#4544) (@houko)
- Persist token_endpoint to bare namespace so refresh works (#4547) (@neo-wanderer)
- Skip ref override for fork PRs in openapi-drift checkout (#4557) (@houko)
- Preserve source() chain on LibreFangError typed variants (#3745) (#4562) (@houko)
- Split canonical name from localized display_name (404 on Chinese labels) (#4563) (@houko)
- Standardize error responses on ApiErrorResponse (#3505) (#4566) (@houko)
- Warn on context-window approach + expose session size (#3693) (#4572) (@houko)
- Annotate top-N endpoints with utoipa schemas (#3396) (#4578) (@houko)
- A11y on historical thinking drawer toggle (#4542 follow-up) (#4597) (@houko)
- Drop {status,budget} envelope on updateUserBudget return type (#4598) (@houko)
- Invalidate full plugin domain so Marketplace 'Installed' badge updates (#4617) (@houko)
- Defend AuditPage against missing entries on empty audit log (#4618) (@houko)
- Drop standalone Canvas entry from observability nav (#4620) (@houko)
- Restore # pragma: no-attribution on legacy [Unreleased] entries (#4643) (@houko)
- Progress.rs early-exit hygiene + failure-finish glyph (#3306 follow-up) (#4647) (@houko)
- Align remaining route assertions with nested error envelope (#3639) (#4655) (@houko)
- TUI mcp_catalog().read() compile break + 2 missed init-upgrade early exits (#4656) (@houko)
- Review follow-ups for #4640/#4649/#4651/#4655 (#4657) (@houko)
- DELETE /api/agents/{id} idempotent on nonexistent (refs #4614) (#4663) (@houko)
- Align 5 missed assertions with dual-shape error envelope (#4670) (@houko)
- Isolate Live Integration Smoke from default dashboard credentials (#4671) (@houko)
- Kill wall-clock flake in registry concurrent-register-and-remove test (#4673) (@houko)
- Bump test_sidecar_adapter_spawn_echo timeout for Windows cold-start (#4676) (#4679) (@houko)

### Changed

- Switch prometheus_handle to OnceLock (#3747) (#4339) (@houko)
- Drop duplicate PUT /agents/{id}/update, fold into PATCH (#4348) (@houko)
- Preserve typed HandError across kernel boundary (1-of-21 slice of #3711) (#4351) (@houko)
- Preserve typed SandboxError across kernel boundary (2-of-21 slice of #3711) (#4354) (@houko)
- Preserve typed HandError at 7 remaining collapse sites (extends #4351) (#4359) (@houko)
- Remove rotting issue-number refs from PaginatedResponse comments (#4370) (@houko)
- Drop KernelError dep in classify_streaming_error (#3744) (#4386) (@houko)
- Drop KernelResult dep in stream bridge fns (#3744) (#4390) (@houko)
- Drop ApprovalManager static call from dashboard_login (#3744) (#4391) (@houko)
- Drop ApprovalManager static calls in TOTP setup (#3744) (#4394) (@houko)
- Wrap inbox_status behind kernel method (#3744) (#4395) (@houko)
- Wrap probe_and_update_local_provider in kernel method (#3744) (#4397) (@houko)
- Drop kernel dep for librefang_home() lookup (#3744) (#4401) (@houko)
- Wrap auto_dream module behind kernel methods (#3744) (#4403) (@houko)
- Drop ApprovalManager static is_recovery_code_format calls (#3744) (#4404) (@houko)
- Drop ApprovalManager static calls in TOTP verify (#3744) (#4406) (@houko)
- Wrap session trajectory export behind kernel method (#3744) (#4407) (@houko)
- Drop WorkflowEngine import via Workflow::to_template (#3744) (#4410) (@houko)
- Drop KernelError dep in stream bridge tests (#3744) (#4412) (@houko)
- Drop KernelError test imports (#3744 14-of-many) (#4414) (@houko)
- Re-export UserRole through middleware boundary (#4416) (@houko)
- Route trajectory imports through crate-local facade (#3744) (#4417) (@houko)
- Re-export KernelOAuthProvider via crate::mcp_oauth (#3744) (#4418) (@houko)
- Wrap workflow_to_template behind LibreFangKernel method (#3744) (#4419) (@houko)
- Drop librefang_kernel::config::librefang_home calls (#3744) (#4420) (@houko)
- Drop direct router::invalidate_hand_route_cache imports (#3744) (#4421) (@houko)
- Route config_reload validate through Kernel method (#3744) (#4423) (@houko)
- Route UserRole through middleware re-export (#3744) (#4424) (@houko)
- Re-export kernel trigger types via librefang-api::triggers (#3744) (#4425) (@houko)
- Extract pairing handlers from system.rs (#3749 1/8) (#4452) (@houko)
- Extract tool-profile + agent-template handlers from system.rs (#3749 2/8) (#4454) (@houko)
- Extract tools + sessions handlers from system.rs (#3749 3/8) (#4455) (@houko)
- Extract hooks + commands handlers from system.rs (#3749 4/N) (#4458) (@houko)
- Extract backup/restore handlers from system.rs (#3749 5/N) (#4459) (@houko)
- Extract audit handlers from system.rs (#4461) (@houko)
- Extract webhooks subdomain from system.rs (#3749) (#4464) (@houko)
- Extract task-queue handlers from system.rs (#3749 9/N) (#4468) (@houko)
- Extract registry handlers from system.rs (#3749 10/N) (#4473) (@houko)
- Add Path<AgentId> extractor and remove parsing boilerplate (#3603) (#4493) (@houko)
- Remove unused retry abstraction (#3600) (#4495) (@houko)
- Extract approvals + TOTP handlers from system.rs (#3749) — supersedes #4460 (#4513) (@houko)
- Extract hooks + commands handlers from system.rs (#3749 4/N) — supersedes #4458 (#4518) (@houko)
- Extract registry handlers from system.rs (#3749 10/N) — supersedes #4473 (#4519) (@houko)
- Split god trait into 14 role traits (#3746) (#4536) (@houko)
- Extract last 5 sub-routers from system.rs (#3749 11/N) (#4539) (@houko)
- Re-export kernel workflow types via librefang-api::workflow (#3744) (#4543) (@houko)
- Drop Option<Arc<KernelHandle>> from internal call sites (#3652) (#4559) (@houko)
- Mutation envelope cleanup — budget + prompts/goals HTTP semantics (#3832) (#4561) (@houko)
- Rename prompts::routes to router for module-naming consistency (#3748) (#4574) (@houko)
- Type CanvasPage nodes — drop `as any`/`as CanvasNodeData` hatches (#3390) (#4577) (@houko)
- Progress + table facade; scripts/commit.sh (#3306 1/N) (#4582) (@houko)
- Explicit discriminator + sentinel lint (#3302 1/N) (#4587) (@houko)
- API → Kernel for 15 runtime types (#3596 1/N) (#4590) (@houko)
- Re-export kernel approval/error via librefang_api (#3744 N/M) (#4592) (@houko)
- Migrate remaining printf tables to Table builder (#3306 2/N) (#4632) (@houko)
- Clean up AppState double-Arc + boot-static field wrappers (#3747) (#4635) (@houko)
- KernelOpError is now a LibreFangError alias (#3541 8/N final) (#4636) (@houko)
- Reduce librefang-api → librefang_kernel internal imports (#3744) (#4650) (@houko)
- Full KernelHandle widening — close LibreFangKernel leaks (#3744 N/N) (#4661) (@houko)

### Performance

- Use save_session_async in async paths (#3379) (#4301) (@houko)
- Bound debouncer + WeCom WS channels (#3580) (#4415) (@houko)
- Cache unlocked vault to avoid per-call Argon2id KDF (#3598) (#4491) (@houko)
- Persist message_count column to skip blob deserialization in list_sessions (#3607) (#4496) (@houko)
- Make LlmError::TimedOut.partial_text Arc-shared (#3552) (#4500) (@houko)
- Suppress polling refetch in background tabs (#3393) (#4502) (@houko)
- Switch send_channel_file_data to bytes::Bytes (#3553) (#4505) (@houko)
- Event-drive agents WS instead of per-client 5s polling (#3513) (#4508) (@houko)
- ArcSwap budget_config + tokio::fs for agent_context (#3579) (#4564) (@houko)
- Arc<AgentEntry> registry; migrate dashboard hot paths (#3569) (#4569) (@houko)
- Parking_lot Mutex<VecDeque<Arc<Event>>> for history (#3385) (#4571) (@houko)
- Split chunks + lazy-load KaTeX (#3381) (#4576) (@houko)
- Swap model_catalog RwLock for ArcSwap (#3384) (#4599) (@houko)
- ArcSwap + tokio::fs for hot-path locks and sync I/O (#3579) (#4654) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Documentation

- Retire manual curl checklist, point to integration tests (refs #3721) (#4398) (@houko)
- Wire French README + skill-development.zh into language switchers (#3399) (#4506) (@houko)
- Refresh CLAUDE.md cron + session_mode note (#3657) (#4507) (@houko)
- Seed README.md for the 8 Tier-1 crates (#3398) (#4537) (@houko)
- Rewrite root AGENTS.md in Telegraph style (#3309) (#4579) (@houko)
- AI-agent collaboration boundaries + CI wait policy (#3299) (#4594) (@houko)

### Maintenance

- Default round-trip coverage for AgentManifest, ChannelsConfig, BroadcastConfig (#4308) (@houko)
- Cover UserBudgetPage (refs #3853) (#4311) (@houko)
- Cover TOTP settings section (Refs #3853) (#4312) (@houko)
- Add ApprovalsPage RTL coverage for #3853 (#4313) (@houko)
- Cover global and per-agent budget routes (Refs #3571) (#4315) (@houko)
- Integration tests for /api/channels routes (#3571) (#4316) (@houko)
- Integration tests for /api/agents routes (Refs #3571) (#4317) (@houko)
- Cover skills-domain HTTP routes (Refs #3571) (#4318) (@houko)
- Integration tests for memory routes (partial #3571) (#4319) (@houko)
- Integration tests for providers/models routes (Refs #3571) (#4320) (@houko)
- Integration tests for approvals routes (Refs #3571) (#4321) (@houko)
- /audit routes integration coverage (audit slice of #3571) (#4322) (@houko)
- Integration tests for plugins routes (#3571) (#4323) (@houko)
- Add /api/hands HTTP route integration tests (#3571 hands slice) (#4324) (@houko)
- Integration tests for /api/a2a/* routes (#4325) (@houko)
- Auto-close umbrella issues when their last referencing PR merges (#4326) (@houko)
- Add integration tests for /api/goals/* routes (#4327) (@houko)
- Add integration tests for workflows routes (#3571) (#4329) (@houko)
- Integration coverage for config routes (#4330) (@houko)
- Cover peers/network/comms route slice (#3571) (#4331) (@houko)
- Inject APPLE_DEVELOPMENT_TEAM into iOS init + build steps (#4332) (@houko)
- Integration coverage for inbox routes (#3571 partial) (#4333) (@houko)
- Integration coverage for /api/authz/{effective,check} (#4334) (@houko)
- Cover mcp_auth status/start/callback edge paths (#4335) (@houko)
- Cover /api/auto-dream/* routes with integration tests (#4336) (@houko)
- /v1/* OpenAI-compat integration tests (partial #3571) (#4337) (@houko)
- Cover oauth route validation paths (oauth slice of #3571) (#4338) (@houko)
- Integration tests for profiles/templates routes (#3571) (#4340) (@houko)
- Cover tools and sessions GET routes in system router (#4341) (@houko)
- Integration tests for hooks/commands routes (#4342) (@houko)
- Cover pairing notify/devices + backup/restore routes (#4343) (@houko)
- Cover /api/terminal/* REST validation + auth gates (#4345) (@houko)
- Integration coverage for prompts routes (#4346) (@houko)
- Add /media/* integration coverage (media slice of #3571) (#4347) (@houko)
- Integration tests for /channels/* webhook router (#4349) (@houko)
- Unit tests for templates module helpers (refs #3582) (#4350) (@houko)
- Add launcher daemon-detection tests (refs #3582) (#4352) (@houko)
- Add desktop_install unit tests (#3582) (#4353) (@houko)
- Drop gh-pr-merge guard so the AI can land merges directly (#4357) (@houko)
- Slim pre-commit to fmt + secrets, move clippy to pre-push (#3303) (#4369) (@houko)
- Unit-test init_wizard config emission helpers (#4387) (@houko)
- Cover state_badge classifier branches and fall-through (#4392) (@houko)
- Cover AnalyticsPage load/empty/budget interactions (#3853) (#4409) (@houko)
- Cover tui::widgets pure helpers (#4411) (@houko)
- Cover chat screen pure helpers and input history (#4413) (@houko)
- Add report-only code coverage measurement (#3819) (#4443) (@houko)
- Drop #3842 pagination envelope fallbacks (#4444) (@houko)
- Wire APPLE_PROFILE_NAME for manual iOS signing (#4446) (@houko)
- Cover gotify send() path with wiremock (1-of-N) (#4447) (@houko)
- Cover LogsPage load/error/filter/export paths (#4449) (@houko)
- Cover PluginsPage load/empty/install/scaffold paths (#4451) (@houko)
- Cover RuntimePage (#3853) (#4453) (@houko)
- Cover ModelsPage load/filter/add/delete paths (#4462) (@houko)
- Cover MemoryPage stats/list/mutations (#3853) (#4463) (@houko)
- Cover GoalsPage tree, create, status, and delete flows (#3853) (#4465) (@houko)
- Cover ChannelsPage flows (#4467) (@houko)
- Cover HandsPage flows (#4469) (@houko)
- Cover SchedulerPage rendering and mutation wiring (#3853) (#4470) (@houko)
- Cover WorkflowsPage tab/run/delete/template flows (#4471) (@houko)
- Cover ProvidersPage list, tabs, search, and test action (#4472) (@houko)
- Cover UserPolicyPage RBAC matrix editor (#4474) (@houko)
- Cover MobilePairingPage flows (#3853) (#4475) (@houko)
- Harden lifecycle load tests with timeout-based polling (#3817) (#4486) (@houko)
- Smoke-matrix coverage for ~80% untested routes (#3571) (#4488) (@houko)
- Cover launcher / init_wizard / desktop_install (#3582) (#4490) (@houko)
- Gate dependabot auto-merge on CI success + 24h age (#3555) (#4511) (@houko)
- Scope -D warnings to first-party via workspace lints (#3554) (#4512) (@houko)
- Add idempotency + forward-compat fixtures (#3407) (#4516) (@houko)
- Surface failing tests via step summary + always-on artifact (#4525) (@houko)
- Clear baseline main CI red (fmt + openapi + clippy) (#4526) (@houko)
- Align kill/delete + channel-creds assertions with #3509 / #3507 (#4527) (@houko)
- Wire schema-check into CI + cover agent.toml (#3300) (#4528) (@houko)
- Scoped clippy + codegen fingerprint cache in pre-push (#4531) (@houko)
- Strip pre-push to a protected-branch guard, defer to CI (#4532) (@houko)
- Scope test matrix away from xtask/workflow-only changes (#4533) (@houko)
- Skip workflow on tooling/docs-only PRs (#4535) (@houko)
- Auto-commit regenerated openapi.json + sdk on internal PRs (#4540) (@houko)
- Cover UsersPage render branches and action wiring (#3853) (#4541) (@houko)
- Cover slack send() path with wiremock (#3820 2-of-N) (#4545) (@houko)
- Cover McpServersPage RTL flows (#3853 19/N) (#4546) (@houko)
- Cover teams send() path with wiremock (#3820 4-of-N) (#4550) (@houko)
- Cover discord/keybase/mastodon/nextcloud/ntfy/pumble/reddit send() with wiremock (#3820 8-of-N) (#4551) (@houko)
- Cover dingtalk/messenger/mattermost/bluesky + viber send() with wiremock (#3820 6-of-N) (#4552) (@houko)
- Cover line send() path with wiremock (#3820 5-of-N) (#4553) (@houko)
- Auto-regenerate schema baselines too (#4554) (@houko)
- Only update PRs with failing CI (#4556) (@houko)
- Proptest invariants for approval rules + trim_history (#3409) (#4568) (@houko)
- Validate (@user) attribution on Unreleased CHANGELOG entries (#3400) (#4573) (@houko)
- Wiremock'd transport for Slack / Discord / Matrix (#3406) (#4575) (@houko)
- Script articles/ scaffold from CHANGELOG (#3397) (#4580) (@houko)
- Adopt cargo-deny for supply-chain audit (#3305) (#4581) (@houko)
- Unify prerelease format to vYYYY.M.D-beta.N (#3310) (#4584) (@houko)
- Nextest 4-way sharding + xtask build-timings tracker (#3311) (#4586) (@houko)
- Scaffold split per-target workflows (#3304 1/N) (#4588) (@houko)
- Supply-chain audit for skills / hands / extensions (#3333) (#4589) (@houko)
- Wiremock send() coverage for Telegram (#3820) (#4591) (@houko)
- Dead-route audit catches missing server.rs registrations (#3721 1/N) (#4593) (@houko)
- Wire xtask integration-test as live-integration-smoke job (#3405) (#4601) (@houko)
- Integration tests for runtime / llm-drivers / extensions / runtime-mcp / hands (#3696) (#4628) (@houko)
- Enforce 100% (@author) attribution (#3307) (#4631) (@houko)
- Install libdbus-1-dev to unblock daemon build (#4638) (@houko)
- Assert CWD has Cargo.toml in fs_read deny test (#4639) (@houko)
- Split test job into unit-fast + integration lanes (#3696) (#4640) (@houko)
- Bump the actions-minor-patch group with 2 updates (#4667) (@app/dependabot)
- Bump actions/checkout from 4 to 6 (#4668) (@app/dependabot)
- Bump sigstore/cosign-installer from 3.10.1 to 4.1.1 (#4669) (@app/dependabot)

### Other

- Mirror ci.yml lane detection locally (#3296) (#4603) (@houko)

</details>


## [2026.5.2] - 2026-05-02

_338 PRs from 7 contributors since v2026.4.28-beta7._

### Highlights

- **iOS & Android mobile app** — native mobile clients launch with responsive UI, bottom-tab navigation, QR-code daemon pairing, and automated TestFlight/Play Store upload
- **FangHub marketplace** — browse, install, and track download/star counts for skills and MCP servers directly from the dashboard, with a redesigned 4-step install wizard
- **Ed25519 peer identity & encrypted OFP connections** — peers now authenticate with persistent Ed25519 keys, TOFU pin storage, and X25519 ephemeral session encryption
- **Redesigned dashboard** — new design-system tokens applied across Overview, Agents, Approvals, Skills, Workflows, and Canvas pages; per-agent stats panel and auto session titles added
- **Broad security hardening** — dozens of fixes covering SSRF, shell injection, auth bypass, TOTP replay, atomic file writes, rate limiting, and sandbox escapes across the daemon and API layer

### Added

- Include session_id in agent-loop-failure warn log (#3260) (@neo-wanderer)
- POST /api/tasks to enqueue from external callers (#3261) (@neo-wanderer)
- Scaffold iOS/Android mobile support (#3342) (#3886) (@houko)
- Mobile-first responsive pass (#3343) (#3898) (@houko)
- Daemon connection wizard with QR pairing (#3344) (#3916) (@houko)
- Add Polish language (pl) (#3937) (@leszek3737)
- TestFlight + Play upload automation, version mapping, release SOP (#4004) (@houko)
- Group roster, alias triggering, and reply precheck wiring (#4035) (@DaBlitzStein)
- Include session_id in operator alert notifications (#4057) (@neo-wanderer)
- Group roster stores wired into kernel/bridge (takeover #4035) (#4079) (@houko)
- Land design-system tokens + redesigned Overview (#4111) (@houko)
- Design-tokens overhaul + master-detail Agents + auto session titles (#4131) (@houko)
- Pin agent_send results and rescue them from history trim (#4138) (@DaBlitzStein)
- Federated hub view for Skills page (#4144) (@houko)
- Add v2 handshake Ed25519 keys and trusted peers store (#4146) (@Chukwuebuka-2003)
- Mobile bottom-tab nav + adapt Overview/Agents/Chat/Approvals (#4150) (@houko)
- Bundle dashboard into mobile release builds (#4151) (@houko)
- FangHub marketplace + worker refactor (#4164) (@houko)
- Show marketplace downloads/stars on registry pages (#4178) (@houko)
- Polish marketplace stats UI on registry cards and detail pages (#4185) (@houko)
- Add usable Ed25519 peer identity primitive (refs #3873, 1/5) (#4245) (@houko)
- Align Agents page with design canvas + per-agent /stats (#4246) (@houko)
- Bind OFP handshake to per-peer Ed25519 identity (refs #3873, 2/5) (#4253) (@houko)
- Persist OFP identity, wire start_with_identity (refs #3873, 3/5) (#4259) (@houko)
- Persist OFP TOFU pins across restarts (refs #3873, 4/5) (#4263) (@houko)
- Expose OFP identity fingerprint, refresh docs (closes #3873, 5/5) (#4267) (@houko)
- X25519 ephemeral KEX for OFP session keys (closes #4269) (#4273) (@houko)
- Redesign Approvals page per design bundle (#4274) (@houko)
- Unblank Skills/Schedule/Logs tabs (#4275) (@houko)
- Redesign MCP marketplace cards + 4-step install wizard (#4278) (@houko)
- Hide unconfigured catalog behind Add picker (#4279) (@houko)
- Horizontal-flow layout logic to match new node visuals (#4280) (@houko)

### Fixed

- Add page-level render tests and CI integration (#3408) (#3425) (@Chukwuebuka-2003)
- Use listing API instead of search API in welcome workflow (#3881) (@houko)
- Add root Dockerfile for Render auto-deploy (#3882) (@houko)
- Add kill_on_drop(true) to prevent orphan subprocess accumulation (#3883) (@houko)
- Replace let _ = error discards with tracing::warn logging (#3884) (@houko)
- Scope memory consolidation queries to agent_id to prevent cross-tenant leak (#3885) (@houko)
- Reject empty webhook secrets and newlines in secret env writes (#3887) (@houko)
- Remove unconditional auth bypass for loopback requests in middleware (#3888) (@houko)
- Enforce memory limit and fix path traversal in capability check (#3889) (@houko)
- Persist agent manifest in PUT manifest handler (#3891) (@houko)
- Use atomic temp+rename pattern for vault file writes (#3893) (@houko)
- Prevent shell injection in skill dependency command execution (#3894) (@houko)
- Merge upload routes before auth/rate-limit layers to prevent bypass (#3895) (@houko)
- Remove ?token= query auth and enforce body limit on webhook routes (#3897) (@houko)
- Eprintln→tracing, Dockerfile non-root, deduplicate operationId, preserve env secret values (#3900) (@houko)
- Reject all-zero Ed25519 registry key and verify hook script integrity (#3901) (@houko)
- Capability glob separators, host_log bounds, block_in_place for host_call (#3902) (@houko)
- Strengthen webhook signature validation for Feishu, DingTalk, and generic adapters (#3903) (@houko)
- Resolve Rust SDK example compile errors and Android CLI build failure (#3904) (@houko)
- Warn missed fires on restart, skip suspended agents, document UTC scheduling (#3906) (@houko)
- Harden pre_check_script env/cwd/output; warn on shell_exec readonly bypass (#3907) (@houko)
- Enforce body limits, auth on task transcripts, pending state for discovered agents (#3909) (@houko)
- KV namespace isolation, result_len cap, per-invocation engine epoch isolation (#3910) (@houko)
- Add timeouts, OAuth CSRF state binding, dotenv escaping, visible proxy fallback (#3911) (@houko)
- Parse Retry-After header, remove fake output_tokens, stop streaming on receiver drop (#3912) (@houko)
- Bind AES-GCM ciphertext to vault path via AAD; fix(triggers): persist cooldown timestamps (#3913) (@houko)
- Add --ignore-scripts to npm publish steps (#3914) (@houko)
- Verify SHA256 of downloaded binary assets before npm publish (#3915) (@houko)
- Validate id path components, skip existing files, version check, atomic writes (#3917) (@houko)
- Non-root container user; MCP SSE protocol + Content-Type validation (#3919) (@houko)
- Nonce check after HMAC, 64KB message cap, recipient node_id in handshake HMAC (#3920) (@houko)
- DELETE handlers return 204, scope agents by user_id, v1 routes in OpenAPI (#3922) (@houko)
- Cron suspended-agent skip, env-clear scripts, ordered triggers; WASM block_in_place + host_log cap (#3923) (@houko)
- Aria-label for agent dots, dialog roles on hand-written modals, message windowing (#3924) (@houko)
- Canonicalize before capability check, readonly workspaces, glob separators (#3925) (@houko)
- Kill stdio child on drop, cap SSE body, pipe stderr, restrict env expansion (#3926) (@houko)
- Channel body limit, remove ?token= from REST routes, implement PUT agents, fix operationIds (#3927) (@houko)
- Skip env file substitution, fix README, update CLAUDE.md anchors, replace eprintln (#3928) (@houko)
- Per-task trigger depth, observable event bus drops, DST-aware cron log (#3929) (@houko)
- Tab ARIA roles, submit guards, WS stale URL, aria-live, WS auth error handling (#3930) (@houko)
- Signal SSRF guard, ClawHub SHA256 validation, expand license deny-list (#3931) (@houko)
- Inline tauri::generate_handler! to fix E0282 on main (#3933) (@houko)
- Target agent dispatch, workflow crash recovery, persistent A2A task store (#3935) (@houko)
- Enable input sanitizer for Command messages, add per-peer OFP rate limit (#3936) (@houko)
- Harden workflow shell injection, add dependabot npm/pip coverage (#3938) (@houko)
- Auth-gate logs/stream SSE, set 0600 on sessions file, enforce WS origin, tighten CSP (#3939) (@houko)
- SSRF guard for OAuth discovery, validate token_endpoint domain, per-flow PKCE state, auth-gate callback (#3940) (@houko)
- WASM env blocklist, auth-gate approvals/session, restrict config/set paths, apply_patch readonly check (#3941) (@houko)
- Mandatory webhook HMAC verification + SSRF guard (#3942) (@houko)
- Atomic TOTP/recovery-code operations, require email_verified in OIDC, persist lockout counter (#3943) (@houko)
- Cap SKILL.md size, auth-gate uploads, enforce OIDC nonce, atomic init write, random keyring fallback (#3944) (@houko)
- Noopener on OAuth window, htmlFor on form labels, invalidate budget after media gen, optimize streaming updates, tree-shake lucide icons (#3945) (@houko)
- Graceful prometheus init, surface JoinError, wire timeout_secs, graceful task shutdown, persist cron on each run (#3946) (@houko)
- 5min staleTime for models, webhook HMAC error-path tests, Dockerfile non-root USER (#3948) (@houko)
- Remove email/google-chat from default channel features, fix RSA timing attack dep, switch provider maps to BTreeMap (#3949) (@houko)
- Per-IP rate limit on auth endpoints (10 attempts / 15 min) (#3950) (@houko)
- Prevent TOTP replay, remove ?token= from WS, warn on unauthenticated network exposure (#3952) (@houko)
- Replace set_var in async, cap OpenAI retry backoff, disable A2A redirects, harden desktop CSP (#3953) (@houko)
- Atomic persist with fsync for cron/config/webhook/agent-flag (#3954) (@houko)
- Recover from poisoned locks, log Anthropic errors, log shutdown persist failures (#3955) (@houko)
- Block agent self-send, pre-call budget gate, log EventBus drops, stable system prompt, propagate Telegram chunk errors (#3956) (@houko)
- Cap AuditLog, evict GCRA entries, single-query budget, reduce clones (#3957) (@houko)
- CanvasPage React Query migration, raise agent limit, SSE keep-alive, paginate sessions/approvals, complete AgentItem type (#3958) (@houko)
- Async TUI HTTP, tokio::fs plugin_manager, SkillsPage guard, track watcher handles, inbox spin loop (#3959) (@houko)
- 5 concurrency bugs — lane permit, session-scoped injection, trigger depth, orphaned task abort, panic logging (#3960) (@houko)
- TUI auth header, block TOTP overwrite, proper memory error codes, remove build.rs git config, log skill install errors (#3961) (@houko)
- SQLite FK enforcement, per-step migration transactions, save_session atomicity, schema version guard, daemon file lock (#3962) (@houko)
- SessionStorage WS token, SSRF OAuth endpoints, random vault key, skill timeout (#3963) (@houko)
- 5 runtime behavior bugs (#3597 #3611 #3625 #3628 #3672) (#3965) (@houko)
- TUI SSE cancellation, crossterm Resize+Paste, atomic clawhub install, hot-path clone reduction (#3966) (@houko)
- Standardize error format, spawn_blocking for journal I/O, document ignored load tests (#3967) (@houko)
- Skip Cloudflare deploy step for fork PRs in deploy-web workflow (#3968) (@houko)
- Close 5 concurrency bugs (#3736 #3737 #3738 #3742 #3717) (#3969) (@houko)
- #3425 follow-up — restore deps, fix tests, real lint (#3998) (@houko)
- Restore host-separator-aware glob matching (regressed by #3925) (#4005) (@houko)
- Un-break upstream/main from two bad merges (#4007) (@neo-wanderer)
- Close two truncated test helpers blocking pre-commit fmt (#4010) (@houko)
- Release_reservation() for non-LLM paths; reserve 0 under unlimited quota (#4011) (@houko)
- Extend RwLock/Mutex poison recovery beyond commands.rs (#4012) (@houko)
- Cap on-boot load at max_tasks instead of slurping retention window (#4013) (@houko)
- Atomic running_tasks swap to close abort-handle race (#4014) (@houko)
- Don't leak internal error messages on 5xx from memory routes (#4015) (@houko)
- Create mobile WebviewWindow so iOS/Android stop launching black (#4017) (@houko)
- Serialize triggers/workflow persist writes to close in-process tmp-file race (#4018) (@houko)
- Close SSRF bypass via IPv4-mapped IPv6 / NAT64 / trailing-dot host (#4019) (@houko)
- Close two real bypasses of #3950 auth rate limit (#4020) (@houko)
- Repair main — conflict markers, duplicate fn, unclosed delimiter, stale schema golden (#4021) (@houko)
- Auth-gate every /api/approvals read, not just the session subtree (#4022) (@houko)
- Use atomic vault_redeem_recovery_code in channel-bridge approve path (#4023) (@houko)
- Drop stale chat label; suppress inbox spin on un-removable empty file (#4024) (@houko)
- Keep journal mutex held across disk write to restore WAL invariant (#4025) (@houko)
- Use word-boundary check in env-var blocklist to stop false positives (#4026) (@houko)
- Repair tool pairs before saving on failure paths (#4029) (#4032) (@DaBlitzStein)
- Normalize workflow_id to id in createWorkflow response (#4038) (@DaBlitzStein)
- Atomic machine-id write, no-regen on length mismatch, race-safe O_EXCL (#4040) (@houko)
- Reject OIDC callback when id_token validation fails (no userinfo fallback) (#4041) (@houko)
- Atomic .env save closes #3944 truncation + perms TOCTOU (#4042) (@houko)
- Wire TOTP replay check to channel-bridge + totp_revoke (#4043) (@houko)
- Atomic create with mode(0o600) for sessions file (#4044) (@houko)
- Keep draining stderr after log cap to prevent child pipe stall (#4045) (@houko)
- Close shell-injection in deploy-web/docs missed by #3938 (#4046) (@houko)
- Init wizard saves API key only after successful validation (#4047) (@houko)
- Gate Dependabot auto-merge on patch/minor update-type only (#4048) (@houko)
- Persistent OIDC nonce single-use enforcement (#4049) (@houko)
- Preserve in-memory entries whose SQLite write failed during trim (#4050) (@houko)
- Stream MCP response body with running cap (no 16 MiB pre-rejection allocation) (#4051) (@houko)
- Bound rmcp client close() with a 10s timeout (cap shutdown stall) (#4052) (@houko)
- Host_log uses lossy UTF-8 decode so multi-byte boundary doesn't drop 4 KiB (#4053) (@houko)
- Refuse symlink-leaf writes in host_fs_write (close grant escape) (#4054) (@houko)
- Segment-aware glob also splits on Windows backslash (#4055) (@houko)
- Propagate PUBLISH_EVENT_DEPTH scope across trigger_dispatch spawn (#4056) (@houko)
- Unbreak docker build (#3948 added duplicate user creation) (#4058) (@houko)
- Drop noopener on OAuth window so dashboard tab isn't navigated away (#4059) (@houko)
- Stop CanvasPage clobbering unsaved edits every 30s (#4060) (@houko)
- Fetch workflows after template instantiate (don't read stale closure) (#4062) (@houko)
- Recover ChatPage WS from retries-exhausted state on tab visible / online (#4063) (@houko)
- Main CI green — clippy doc/collapsible-if + openapi regen (#4064) (@houko)
- Treat /private/tmp as /tmp for capability checks on macOS (#4065) (@houko)
- Remove one-shot job on record_skipped (stop garbage accumulation) (#4066) (@houko)
- Evaluate triggers in deterministic id order (#4067) (@houko)
- Wire webhook handler through verify_request (no more dead code) (#4068) (@houko)
- List full in-memory window so pagination total matches reality (#4069) (@houko)
- Re-announce same-string aria-live so screen readers don't dedupe (#4070) (@houko)
- Repair TUI daemon_client() refs and missing api_key arg in chat_runner (#4071) (@neo-wanderer)
- Register 'pl' in registry-route + search-dialog locale lists (#4072) (@houko)
- Repair main — sanitizer field, dingtalk test args, rustfmt diff (#4074) (@houko)
- Drop dead sha2::Digest import in machine_fingerprint (#4075) (@houko)
- Preserve TUI api_key auth + repair main build (#4076) (@houko)
- Stop polling protected endpoints before login (#4077) (@houko)
- Repair daemon-token shadowing in spawn_save_provider_key (#4078) (@houko)
- Drop entry on DB write failure to preserve chain integrity (#4080) (@houko)
- Rename misleading trait method + wire roster_upsert that #4079 left dead (#4081) (@houko)
- Repair upsert_sender_into_roster signature (close #4081 E0277) (#4082) (@houko)
- Cargo fmt --all to clear accumulated main drift (#4083) (@houko)
- Repair stale AppState initializers (close E0061+E0063 across 8 files) (#4084) (@houko)
- Strip [ ] brackets from IPv6 host_str before IpAddr parse (#4085) (@houko)
- Chmod 0600, AAD schema binding, dotenv newline escape (#4089) (@houko)
- 3 data-integrity bugs (#4091) (@houko)
- WS auth via Sec-WebSocket-Protocol + status-class log levels (#4092) (@houko)
- Re-validate redirect targets against SSRF allowlist (close #3782) (#4093) (@houko)
- Repair model lookup + capability detection for HF-imported models (close #4034) (#4094) (@houko)
- Repair SearXNG config deserialization (close #4016) (#4095) (@houko)
- Block http MITM-RCE on webview + guard build.rs git mutation (#4098) (@houko)
- Reject userinfo URLs and bound shell_exec runtime (#4099) (@houko)
- Close 3 inbound-safety holes (LINE/Teams/email) (#4100) (@houko)
- Stop swallowing vault write errors in 3 security paths (#4101) (@houko)
- Approval audit, disconnect cancel, MCP tool order (#4103) (@houko)
- DNS-rebind, chunk loss, journal stall, lag drops (#4104) (@houko)
- Cap outbound JSON bodies + gate sends on trusted URLs (#4105) (@houko)
- Bound Python/Node/Shell subprocess + validate inputs (#4106) (@houko)
- Five single-spot stability and correctness fixes (#4107) (@houko)
- Tighten host_call/result size caps + per-store epoch interrupt (#4108) (@houko)
- Bind OAuth state to caller, tighten sessions/TOTP perms (#4109) (@houko)
- Close 3 followup safety gaps (#4110) (@houko)
- Repair three silent data-corruption paths (#4112) (@houko)
- Close 5 API endpoint authz gaps (#4113) (@houko)
- Cron concurrency, trigger depth, persist tmp, lock GC (#4114) (@houko)
- Error handling + persistence + hot-reload (#4115) (@houko)
- Atomic OpenClaw migration via staging dir + version check (#4116) (@houko)
- Checkpoint kill-pid race + remove dishonest wasm-hooks feature (#4117) (@houko)
- Batch4 OIDC/MCP/vault/WASM hardening + close stale issues (#4119) (@houko)
- Atomicity + reliability batch (#4120) (@houko)
- Trigger lane timeout + workflow pause atomicity (#4121) (@houko)
- Harden task lifecycle (panics, locks, races) (#4122) (@houko)
- Dashboard + CLI quality batch (7 small fixes) (#4123) (@houko)
- Batch 6 driver/runtime correctness fixes (#4124) (@houko)
- Preserve merge state, surface vector errors, atomic cascade (#4125) (@houko)
- Tighten audit, sandbox, and spawn deniability holes (#4126) (@houko)
- Batch of 6 fixes (#4127) (@houko)
- Close 6 endpoint reliability holes (#4128) (@houko)
- Claude_code break-on-disconnect + stream retry backoff (#4130) (@houko)
- Cap looks_like_tool_call heuristic to short responses (#4132) (@DaBlitzStein)
- Exempt agent_send results from aggressive 2K context compaction (#4136) (@DaBlitzStein)
- Accept Sec-WebSocket-Protocol bearer token for non-loopback WS auth (#4139) (@neo-wanderer)
- Unbreak mobile-smoke + release mobile builds (#4140) (@houko)
- Overview margins, dark default, per-session metering (#4141) (@houko)
- Switch stamps.last() to next_back() to satisfy clippy (#4143) (@houko)
- Unbreak mobile builds + connection screen (#4149) (@houko)
- Finish #3630 lag-counter migration (#4152) (@houko)
- Restore public access to live demo (fly.io) (#4157) (@houko)
- Strengthen keyring-fallback wrap-key derivation (#4159) (@houko)
- TOTP recovery code entropy + TOCTOU hardening (#4161) (@houko)
- Unbreak workspace build (#4179) (@houko)
- Allow marketplace.librefang.ai in connect-src (#4182) (@houko)
- Close two forbid-main-worktree bypass holes (#4193) (@houko)
- Apply_patch read-only enforcement + A2A SSRF hardening (#3662, #3563) (#4197) (@houko)
- Shlex tokenization — kill the commit-message false-positive class (#4199) (@houko)
- Refuse non-loopback bind without auth (#3572) (#4203) (@houko)
- Clippy::manual_contains in config writable-key check (#4204) (@houko)
- Align Arc<Event> receiver and Arc<Vec<Message>> in tests (#4207) (@houko)
- Surface TOTP DB write errors and resync openapi.json (#4209) (@houko)
- Bump npm deps to clear audit advisories (#4227) (#4228) (@houko)
- Pin MCP OAuth token_endpoint to issuer host (#3713) (#4229) (@houko)
- Make append_canonical atomic to prevent cross-session message loss (#4233) (@houko)
- Clear clippy::let_unit_value in TOTP test (fixes #4232) (#4234) (@houko)
- Allow worktree-cleanup commands from main worktree (#4235) (@houko)
- Log send errors instead of silently swallowing them (#4237) (@houko)
- Handle RwLock poisoning gracefully in TUI model picker (#4238) (@houko)
- Add wildcard arms for non_exhaustive enums (#4241) (@houko)
- Route HTTP clients through librefang-http (#4242) (@houko)
- Unwrap audit entries on agents Logs tab (#4243) (@houko)
- Honor CompletionRequest.timeout_secs in gemini driver (#4249) (@houko)
- Align AgentItem TS type with Rust AgentEntry wire form (#4250) (@houko)
- Replace blocking std::fs in async plugin_manager fns (#4251) (@houko)
- Set explicit SSE keep-alive interval (closes #3690) (#4252) (@houko)
- Skip macOS Keychain by default to avoid prompt fatigue (#4255) (@houko)
- Honor Retry-After header on 429/503 (#4257) (@houko)
- Track real KernelConfig fields in strict-mode allowlist (#4258) (@neo-wanderer)
- Paginate /api/sessions/search to bound result sets (#4260) (@houko)
- Surface backpressure on full inject_message channel (#4261) (@houko)
- Route plugin-installer through librefang-http (refs #3577) (#4262) (@houko)
- Stop foreground tee from duplicating every log line (#4265) (@neo-wanderer)
- Structured McpOAuthError replaces stringly errors (#4266) (@houko)
- Wire detail-panel tabs to per-agent endpoints (#4268) (@houko)
- Render Conversation markdown + project Memory rows (#4272) (@houko)
- Typed /events schema + skills_disabled / type tidy (#4277) (@houko)
- PageHeader CJK wrap + strip MCP tool prefix (#4281) (@houko)
- Normalize MCP server name when stripping tool prefix (#4287) (@houko)
- Pin test vault key + align resolve precedence (#4297) (@houko)

### Changed

- Trim CLAUDE.md comment-style violations from #4093 review (#4096) (@houko)
- Typed allowlist + enumeration test against route drift (#4162) (@houko)
- Harden warmup, drop trait silent-fail default, pin first-burst log (#4163) (@houko)
- Consolidate fmtNum, harden marketplace stats a11y/CLS (#4189) (@houko)
- Redesign workflows page list & templates (#4271) (@houko)
- Apply design language to workflow node visual (#4276) (@houko)

### Performance

- Batch hot-path allocations on every LLM turn (#4090) (@houko)
- Async config-reload poll, lucide chunk split, GCRA sweep test (#4118) (@houko)
- Cut Vec/Arc clones, regex compiles, and N+1 SUMs (#4129) (@houko)
- Cache hot-path config + add LLM driver tracing spans + thread request_id (#3722, #3683, #3775) (#4202) (@houko)
- Optimize session repair pipeline — skip unchanged turns, consolidate overflow passes (#3568) (#4226) (@leszek3737)
- Hoist tool list out of agent loop hot path (#4264) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Documentation

- Update README with new crate and feature counts new Hands, channels and LLM driver's number (#3437) (@AIHunter83)
- Record OFP plaintext-on-the-wire decision (#4003) (@houko)
- Update README with new crates counts new Hands, channels replacing closed PR [#3437] (#4027) (@AIHunter83)
- Align parser test + SECURITY note with userinfo fix (#4156) (@houko)
- Is_ssrf_blocked_url — reorder doc as numbered pipeline (#4160) (@houko)
- Correct stale crate, driver, and channel counts in README (#4239) (@houko)
- Merge dual [Unreleased] sections in CHANGELOG (#4240) (@houko)
- Link follow-up issue for per-message HMAC coupling (#4270) (@houko)

### Maintenance

- Pin all GitHub Actions to commit SHAs and migrate PyPI to OIDC (#3905) (@houko)
- Integration tests for session_mode_override resolution and trigger concurrency caps (#3951) (@houko)
- IOS + Android release jobs and PR build smoke (#3970) (@houko)
- Bump @xyflow/react from 12.10.1 to 12.10.2 in /crates/librefang-api/dashboard (#3971) (@app/dependabot)
- Bump lucide-react from 0.577.0 to 1.11.0 in /crates/librefang-api/dashboard (#3972) (@app/dependabot)
- Bump clap from 4.6.0 to 4.6.1 (#3973) (@app/dependabot)
- Bump @tanstack/react-query from 5.90.21 to 5.100.5 in /crates/librefang-api/dashboard (#3976) (@app/dependabot)
- Bump jsdom from 29.0.2 to 29.1.0 in /crates/librefang-api/dashboard (#3980) (@app/dependabot)
- Bump zip from 8.5.1 to 8.6.0 (#3984) (@app/dependabot)
- Bump reqwest from 0.13.2 to 0.13.3 (#3985) (@app/dependabot)
- Bump actions/setup-python from 5.6.0 to 6.2.0 (#3986) (@app/dependabot)
- Bump actions/upload-artifact from 4.6.2 to 7.0.1 (#3987) (@app/dependabot)
- Ignore @librefang/cli-* placeholder bumps in dependabot (#3988) (@houko)
- Bump @xterm/addon-search from 0.15.0 to 0.16.0 in /crates/librefang-api/dashboard (#3990) (@app/dependabot)
- Bump @tailwindcss/vite from 4.2.1 to 4.2.4 in /crates/librefang-api/dashboard (#3991) (@app/dependabot)
- Bump recharts from 3.8.0 to 3.8.1 in /crates/librefang-api/dashboard (#3992) (@app/dependabot)
- Bump react-i18next from 16.5.8 to 16.6.5 in /crates/librefang-api/dashboard (#3993) (@app/dependabot)
- Only run nix build on push-to-main, drop per-PR trigger (#3994) (@houko)
- Bump rand from 0.10.0 to 0.10.1 (#3995) (@app/dependabot)
- Only run docker build on push-to-main, drop per-PR trigger (#3996) (@houko)
- Bump vitest to 4.1.5 (#4000) (@houko)
- Regenerate kernel_config_schema golden fixture (#4002) (@houko)
- Add unit tests for spawn_agent, session_mode, cron_crea… (#4009) (@Chukwuebuka-2003)
- Close stale issues (#4030, #3807, #3700) + lock prompt-cache test (#4086) (@houko)
- Lock auth gate on /api/logs/stream + close stale a2a/logs issues (#4087) (@houko)
- Harden release supply chain (sha256, --ignore-scripts, OIDC) (#4088) (@houko)
- Unify retention + soft-delete consistency (5 fixes) (#4102) (@houko)
- Auto-update-branches uses PAT so merges trigger CI (#4142) (@houko)
- Add KernelHandle contract coverage #3818 (#4148) (@leszek3737)
- Centralize test infrastructure with librefang-testing (#4153) (@leszek3737)
- Add wiremock-based retry integration tests for OpenAI, Anthropic, Gemini (#4154) (@leszek3737)
- Expand dependabot to npm/pnpm/python trees (#4158) (@houko)
- Bump dependabot/fetch-metadata from 2.3.0 to 3.1.0 (#4165) (@app/dependabot)
- Bump android-actions/setup-android from 3.2.2 to 4.0.1 (#4166) (@app/dependabot)
- Bump actions/cache from 4.2.2 to 5.0.5 (#4167) (@app/dependabot)
- Bump metrics-exporter-prometheus from 0.18.1 to 0.18.3 (#4168) (@app/dependabot)
- Bump tauri from 2.10.3 to 2.11.0 (#4169) (@app/dependabot)
- Bump rustls from 0.23.39 to 0.23.40 (#4170) (@app/dependabot)
- Bump i18next from 25.8.18 to 26.0.8 in /crates/librefang-api/dashboard (#4171) (@app/dependabot)
- Bump wasmtime from 44.0.0 to 44.0.1 (#4172) (@app/dependabot)
- Bump vite from 7.3.1 to 8.0.10 in /crates/librefang-api/dashboard (#4173) (@app/dependabot)
- Bump metrics from 0.24.3 to 0.24.5 (#4174) (@app/dependabot)
- Bump @playwright/test from 1.58.2 to 1.59.1 in /crates/librefang-api/dashboard (#4175) (@app/dependabot)
- Bump lucide-react from 1.11.0 to 1.14.0 in /crates/librefang-api/dashboard (#4176) (@app/dependabot)
- Bump jsdom from 29.1.0 to 29.1.1 in /crates/librefang-api/dashboard (#4177) (@app/dependabot)
- Rebase open PRs on main update + alert when main goes red (#4180) (@houko)
- Forbid main-worktree edits + ban local cargo build/test (#4187) (@houko)
- Consolidate git-side hooks into scripts/hooks/ (#4190) (@houko)
- Kick off pnpm build alongside just dev (#4191) (@houko)
- Validate release tag, harden contributor-role permissions, sign artifacts (#3545, #3547, #3546) (#4195) (@houko)
- Mark public error/state enums as #[non_exhaustive] (#3660, #3542) (#4196) (@houko)
- Slim default features and consolidate duplicate deps (#3655, #3688, #3679, #3667) (#4198) (@houko)
- Allow PR auto-merge invocations from AI sessions (#4201) (@houko)
- Drop pr-auto-assign workflow in favor of native CODEOWNERS (#4208) (@houko)
- Bump the web-minor-patch group in /web with 7 updates (#4210) (@app/dependabot)
- Bump the dashboard-minor-patch group in /crates/librefang-api/dashboard with 4 updates (#4211) (@app/dependabot)
- Bump react-i18next from 16.6.5 to 17.0.6 in /crates/librefang-api/dashboard (#4215) (@app/dependabot)
- Bump pnpm/action-setup from 6.0.3 to 6.0.4 in the actions-minor-patch group (#4216) (@app/dependabot)
- Bump actions/setup-java from 4.8.0 to 5.2.0 (#4219) (@app/dependabot)
- Bump the docs-minor-patch group in /docs with 12 updates (#4220) (@app/dependabot)
- Bump shiki from 2.5.0 to 4.0.2 in /docs (#4224) (@app/dependabot)
- Bump clap_complete from 4.6.0 to 4.6.3 in the cargo-minor-patch group (#4225) (@app/dependabot)
- HTTP integration coverage for TOTP & MCP OAuth flows (#4230) (@houko)
- Replace fixed sleeps in bridge integration tests with condition polling (#4236) (@houko)
- KernelConfig default-vs-empty-TOML roundtrip regression for #3404 (#4244) (@houko)
- Add daily reconciliation workflow to close stale-resolved issues (#4256) (@houko)

### Other

- Add zh + en entries for #4279 strings (#4288) (@houko)

</details>


### Security

- **Cross-peer memory leak via non-injective `peer:{pid}:{key}` framing + LLM-controlled `peer:`-prefixed keys** (#5119 + #5120). Two paired confused-deputy holes in the shared-memory substrate are closed at the `KernelHandle::MemoryAccess` boundary. (#5119) `peer_scoped_key` rendered a peer-scoped row as `peer:{peer_id}:{key}` and `memory_list(Some(pid))` recovered the peer's keys via `strip_prefix("peer:{pid}:")`. The pair is only injective when `pid` is non-empty and contains no `:` — a Slack-style `peer_id = "T1:U2"`, an IRC-style `"user:42"`, or an empty `""` (which renders `peer::{key}`, ambiguous with a `None`-scope key literally named `:{key}`) collides with a different `(peer_id, key)` tuple and lets one peer enumerate or shadow another peer's keys. (#5120) `memory_store(key, value, peer_id=None)` accepted any LLM-supplied string for `key`, so an agent running without `peer_id` context could write at `key = "peer:victim:user_name"` in the shared namespace; a subsequent `memory_list(Some("victim"))` recovered the planted row as if `victim` wrote it — a trivial impersonation path that bypasses per-peer isolation entirely. Both vectors are now rejected at the kernel-handle boundary with `KernelOpError::InvalidInput`: `peer_scoped_key` (now `Result`-returning) refuses colon-bearing **and empty** `peer_id` plus `peer:`-prefixed keys; `MemoryAccess::memory_store` / `memory_recall` / `memory_list` enforce the same checks before touching the substrate. On the **read side**, `memory_list(Some(pid))` no longer blindly `strip_prefix`es: each recovered inner key is re-rendered through the now-strict `peer_scoped_key(inner, Some(pid))` and only surfaced when it round-trips byte-for-byte to the stored key, so a pre-fix nested / double-scoped plant (`peer:victim:peer:other:secret`) or any structurally-impossible row is dropped, and a colon-bearing list query is rejected outright before any recovery runs (closing the #5119 cross-peer-strip path for the tool layer). The WAS... (line truncated to 2000 chars) (@houko)

- **MCP transport SSRF guard — replace substring stub with parsed-URL allowlist** (#5124). `McpConnection::check_ssrf` (the gate on the SSE, Streamable-HTTP, and HTTP-compat connect paths in `crates/librefang-runtime-mcp/src/lib.rs`) was a lowercase substring match that rejected only `169.254.169.254` and `metadata.google` — every other internal address bypassed it, including loopback (`127.0.0.1`, `localhost`, `[::1]`), RFC1918 (`10/8`, `172.16/12`, `192.168/16`), CGNAT (`100.64.0.0/10`), AWS IMDS (`metadata.aws.internal`), the NAT64 well-known prefix smuggling IMDS (`[64:ff9b::a9fe:a9fe]`), IPv4-mapped IPv6 smuggling loopback (`[::ffff:7f00:1]`), DNS-rebinding hostnames (`169-254-169-254.nip.io`), `user:pw@host` userinfo URLs, and non-`http(s)` schemes like `file://`. A correct policy already existed in the same crate (`mcp_oauth::is_ssrf_blocked_url`); `check_ssrf` is now a thin wrapper around it, so the connect path and the OAuth discovery / token-exchange path share one policy and cannot diverge again. The shared helper also gained `100.64.0.0/10` (CGNAT, incl. Alibaba Cloud IMDS `100.100.100.200`), `metadata.aws.internal`, `instance-data`, `ip6-localhost`, `ip6-loopback`, and an explicit `0.0.0.0` block — aligning the blocklist with `librefang_runtime::web_fetch::check_ssrf`. Tests: the existing `test_ssrf_check` is extended to cover the new categories (loopback IPv4/IPv6/hostname, RFC1918, CGNAT, NAT64-IMDS, IPv4-mapped loopback, userinfo, `file://`, allowed public host). Closes #5124. (@houko)

### Performance

- **Reuse `reqwest::Client` across cron fan-out fires + skip engine on empty targets (#5127)** — `cron_fan_out_targets()` used to call `CronDeliveryEngine::new(sender)` on every AgentTurn / Workflow fire from `cron_tick.rs`, and the engine's constructor in turn built a fresh `reqwest::Client` (TLS context, DNS cache, HTTP/2 connection pool) via `Client::builder().build()`. On busy cron loads (`* * * * *` jobs, `0 */2 * * *` swarms) this churned connection pools per fire even with zero webhook targets configured, FD count climbed under sustained pressure, and TLS handshake CPU rose for no benefit since `reqwest::Client` is documented to be cloned and reused. Two changes: (1) a process-wide `OnceLock<reqwest::Client>` in `kernel::cron_bridge` lazily builds the client on first fan-out and hands a cheap `Arc`-backed `.clone()` to every subsequent `CronDeliveryEngine::with_http_client(sender, http)` invocation, so the TLS / DNS / pool state is reused across all jobs for the lifetime of the daemon; (2) the two call sites in `cron_tick.rs` (`:534` AgentTurn fire, `:638` Workflow fire) now gate the `cron_fan_out_targets` call on `!delivery_targets.is_empty()`, so a job with zero fan-out targets never allocates the bridge `Arc`, never touches the `OnceLock`, and never enters the engine — saving the function-call overhead and the `Arc::clone` of the kernel handle on the common no-webhook path. The DI shape at the engine boundary stays — `CronDeliveryEngine::with_http_client` already existed for tests; the production code now uses it too instead of `new()`. While threading the shared client through, the builder was also routed via `librefang_runtime::http_client::proxied_client_builder()` instead of bare `reqwest::Client::builder()` so the fan-out path picks up the daemon's `[proxy]` config (HTTPS_PROXY / HTTP_PROXY / NO_PROXY), the bundled `webpki-roots` TLS fallback (required on minimal Docker / Termux / musl images that lack a system CA bundle), and the project-wide `libref... (line truncated to 2000 chars) (@houko)

### Fixed

- **Workflow stale-run recovery survives backwards NTP step (#5114)** — `WorkflowEngine::recover_stale_running_runs` previously computed run age as `Utc::now().signed_duration_since(run.started_at)`. Wall-clock arithmetic across daemon restarts is unsound: a backwards NTP correction makes `age` negative so `age < stale_secs` is always true and no row is reaped (silently masking real stale rows); a forward step at boot makes every Running row look ancient and force-fails them all as `Interrupted by daemon restart`. The boot sweep now detects a negative `age`, emits a structured `warn!` with `now`, `started_at`, `run_id`, and the negative `age_secs`, and skips the row (treats it as fresh) without changing state. The proper long-term fix is a monotonic / heartbeat-based reap signal that does not depend on wall-clock; that's out of scope here. Tests: 2 new kernel-unit cases in `librefang_kernel::workflow::tests` — `recover_stale_skips_run_with_started_at_in_the_future` (future `started_at` → row stays Running, empty recovered list, no error/completed_at mutation) and `recover_stale_still_reaps_normally_aged_running_run` (1h-old `started_at` under a 60s cutoff still demotes to Failed, pinning the happy path so the new branch can't silently short-circuit it). (@houko)
- **Cron schedule wedge — validate expression at insert and auto-disable after repeated fallbacks (#5113)** — semantically impossible 5-field cron expressions like `"0 0 30 2 *"` (Feb 30 — never matches) used to pass the librefang-types `validate_cron_expr` field/character check and reach the kernel scheduler, where `compute_next_run_after` silently fell back to `after + Duration::hours(1)` on every `due_jobs()` tick. The job stayed enabled and re-fired hourly for the daemon's lifetime, burning LLM tokens / budget on a schedule that could never produce a real next fire. `CronScheduler::add_job` now probes any `CronSchedule::Cron { .. }` once at insert via the new `compute_next_run_after_opt` helper and rejects with `LibreFangError::InvalidInput` when no future fire exists, before the bad job ever lands in the scheduler. As defense-in-depth for jobs that became unfireable AFTER insert (older persisted jobs, future code paths that mutate schedule without re-validating), a new `JobMeta.consecutive_fallbacks` counter mirrors the existing `MAX_CONSECUTIVE_ERRORS = 5` shape from `record_failure`: each `due_jobs()` tick that gets `None` from `compute_next_run_after_opt` increments the counter, and on reaching 5 the job is auto-disabled with `auto_disabled = true` and `last_status = "auto-disabled: cron schedule produces no future fire time"` so `reassign_agent_jobs` can still re-enable on agent respawn the same way it does for repeated-failure auto-disables today. The counter resets on `record_success`, explicit `set_enabled(true)`, `update_job` enable-toggle, `reassign_agent_jobs`. Tests: 3 new `librefang-kernel::cron::tests` cases (`test_add_job_rejects_cron_with_no_future_fire` for the Feb 30 case, `test_add_job_rejects_malformed_cron_expression` for the 4-field malformed case pinning the librefang-types layer from the scheduler side, `test_due_jobs_auto_disables_after_repeated_fallbacks` driving 5 ticks against an injected wedged schedule and asserting `enabled = false` + `auto_disabled = true` + counter = 5 + subsequent ticks don't return it). Closes #5113. (@houko)

- **Trigger cooldown wedges when `last_fired_at > now`** (#5115). The cooldown check in `librefang_kernel::triggers::TriggerEngine::evaluate_with_resolver` computed `elapsed = (now - last).to_std().unwrap_or(Duration::ZERO)`; on a future-dated `last_fired_at` (wall-clock backstep after NTP correction, manual clock adjustment, VM snapshot restore, or imported state with an ahead-of-now timestamp) `to_std()` errors, the fallback collapses elapsed to 0, and `0 < cooldown` then silently suppresses every gated trigger fire until the wall clock catches up. Replaced with a typed match on the `to_std()` result: the `Err` arm emits a structured `warn!` with `trigger_id`, `agent_id`, `now`, and `last_fired_at`, and treats the cooldown as elapsed-exceeded (`Duration::MAX`) so the trigger fires once. The successful fire then overwrites `last_fired` with `now`, self-healing the registry entry so subsequent evaluations resume the normal cooldown path. Regression test (`test_cooldown_unwedges_on_future_last_fired_at`) seeds the registry with a `+1h` future timestamp, asserts the first evaluation fires, that `last_fired` is rewritten to a non-future value, and that the immediate second evaluation is suppressed by the normal cooldown again — pinning both the unwedge and the self-heal. Closes #5115. (@houko)

- **API request DTOs reject unknown fields so body typos surface as 400 instead of silent feature loss (#5131)** — Every `*Request` / `*Body` shape that an axum `Json<T>` extractor materialises in `crates/librefang-api/src/` gains `#[serde(deny_unknown_fields)]`. Before the fix, a payload like `{"name":"x","url":"…","evnts":["foo"]}` (note the typo'd `evnts`) deserialised to `CreateWebhookRequest` with the unknown key silently dropped and `events` defaulting to `[]`; the server returned 201 Created and the webhook never fired anything. After the fix, serde rejects the payload at the deserialization boundary and axum surfaces 400 Bad Request — the operator sees the typo immediately. DTOs locked down: `webhook_store::{CreateWebhookRequest, UpdateWebhookRequest}`; `types::{SpawnRequest, MessageRequest, AttachmentRef, InjectMessageRequest, SkillInstallRequest, SkillUninstallRequest, SetModeRequest, MigrateRequest, MigrateScanRequest, ClawHubInstallRequest, BulkCreateRequest, BulkAgentIdsRequest, ExtensionInstallRequest, ExtensionUninstallRequest, PushMessageRequest}`; `routes::approvals::{CreateApprovalRequest, ApproveRequestBody, ModifyRequestBody, BatchResolveRequest, ApproveAllForSessionRequest, TotpSetupBody, TotpConfirmBody, TotpRevokeBody}`; `routes::agents::{SetAgentToolsRequest, UpdateIdentityRequest, PatchAgentConfigRequest, CloneAgentRequest, SetAgentFileRequest}`; `routes::users::{UserUpsert, BulkImportRequest}`; `routes::memory::{MemoryAddBody, MemoryUpdateBody}`; `routes::skills::PatchMcpTaintRequest`; `routes::terminal::{CreateWindowRequest, RenameWindowRequest}`; `routes::auto_dream::SetEnabledRequest`; `routes::pairing::PairingCompleteRequest`; `server::ChangePasswordRequest`. Deferred (and why): OpenAI-compat `ChatCompletionRequest` (clients legitimately send `top_p`, `frequency_penalty`, `n`, … — OpenAI's own spec is permissive); OAuth `CallbackBody` / `IntrospectRequest` / `RefreshRequest` (RFC 6749 §3.1 / RFC 7662 explicitly permit extra parameters); request DTOs that live in `librefang-types` (`MediaImageRequest`, `MediaTtsRequest`, `MediaVideoRequest`, `MediaMusicRequest`, `PromptVersion`, `PromptExperiment`, webhook `WakePayload` / `AgentHookPayload`) — they are shared types that are also deserialised from internal stores, so locking them down belongs in their owning crate, not this PR. **Potential client breakage**: callers that previously got away with sending extra fields (typos, optimistic forward-compat keys, debug fields) will now get 400 Bad Request on the listed endpoints. This is the intended behaviour — the silent-drop semantics were the bug — but operators with custom integrations should audit their request bodies. Integration test `crates/librefang-api/tests/api_deny_unknown_fields_test.rs` drives the canonical reproduction from the issue (`POST /api/webhooks` with `evnts` typo) and asserts 400 + zero side-effects, plus a companion test that the same handler still accepts a correctly-spelled body. Closes #5131. (@houko)

- **`POST /api/providers/{name}/default` no longer wipes operator-authored sections of `config.toml`** (#5116). `persist_default_model` previously read the existing config with `std::fs::read_to_string(&path).unwrap_or_default()`, so any transient read failure (`EACCES`, `EIO`, …) collapsed the input to an empty string. The rewrite then serialized a fresh TOML tree containing only `[default_model]` through `atomic_write`, atomically replacing the on-disk file and silently destroying every other section the operator had authored (`[email]`, `[telegram]`, `[proxy]`, `[skill_workshop]`, `[queue]`, …). The fix discriminates `ErrorKind::NotFound` (which legitimately means "first write" — the daemon may create `config.toml` here) from every other read error and returns the latter as `Err`, so the route reports a failed `persisted=false` to the caller and leaves the on-disk file untouched rather than truncating it. The downstream `toml::from_str` / `atomic_write` path is unchanged so the existing crash-safety (temp-write + rename) still applies to a successful merge. Integration test: `set_default_provider_preserves_other_config_sections` in `crates/librefang-api/tests/providers_routes_test.rs` pre-seeds `config.toml` with `[default_model]` + sibling `[email]` and `[proxy]` sections, drives the route, and asserts all three survive in the post-write file; `set_default_provider_when_config_toml_absent_creates_it_with_default_model` pins the `NotFound` branch to confirm the fresh-daemon case still writes a usable config. (@houko)

- **`memory::save_session` silently truncated persisted history to 200 messages** (#5121). `MemorySubstrate::save_session` capped the SQLite write at a hard-coded `MAX_PERSISTED_MESSAGES = 200`, but `clamp_max_history` (`librefang-runtime::agent_loop::history`) only enforces a floor — operators configuring `max_history_messages > 200` (long-context Anthropic / Gemini agents) kept N messages in RAM and persisted only 200, silently losing messages `200..N` on every daemon restart with no log to surface the loss. The original cap (#2929) is kept as defense-in-depth against worst-case blob size and cold-reload RAM, but raised to 2000 — comfortably above any realistic in-memory clamp — and now emits a structured `warn!` log with `agent_id`, `session_id`, `requested_count`, and `cap` when truncation actually fires, so operators are no longer blind to the (now-rare) cases where the cap engages. The stale "in-memory limit is much lower" comment is rewritten to accurately describe the relationship between the runtime trim cap and the persistence ceiling. Two regression tests pin the new contract: `test_save_session_preserves_history_above_legacy_cap` persists 300 messages and asserts the oldest survives reload (the old 200 cap would have dropped it), and `test_save_session_truncates_above_defense_in_depth_cap` confirms the cap still fires above 2000 with the correct most-recent window preserved. (@houko)

- **Docs sync for `DEFAULT_MAX_HISTORY_MESSAGES` (60, not 40)** — `CLAUDE.md` and `docs/architecture/message-history-trimming.md` still cited the compiled-in default as 40, but `crates/librefang-runtime/src/agent_loop/history.rs:38` has been 60 since #4891. Pure docs sync; no code or behaviour change. (@houko)

- **Dashboard mutation cache invalidation — budget/usage on send, snapshot prefix on session model override (#5122, #5123)** — two `crates/librefang-api/dashboard/src/lib/mutations/` invalidation bugs that left the UI showing stale data after a successful mutation. (1) `useSendAgentMessage` promised in its JSDoc that the topbar Budget chip and Analytics page would refresh after a completed turn but only invalidated `agentKeys.session / sessions / stats`; neither `budgetKeys` nor `usageKeys` was touched, so spend updates were invisible until the next poll. Imported both factories and added `invalidateQueries({ queryKey: budgetKeys.all })` + `invalidateQueries({ queryKey: usageKeys.all })` to `onSuccess`. (2) `useSetSessionModelOverride` called `agentKeys.session(variables.agentId)` with the `sessionId` argument omitted, which resolves to the 4-element key `["agents","session",agentId,null]` and only invalidates the "no override" snapshot slot — any cached `(agent, sessionId)` snapshot keyed by an explicit `sessionId` stayed stale. Switched to the 3-element prefix `agentKeys.sessionSnapshots(variables.agentId)` so every snapshot for the agent is invalidated regardless of how its sessionId slot was filled. Tests: 2 new cases in `agents.test.tsx` (`useSendAgentMessage` with explicit `session_id` asserts the 4-element `agentKeys.session(agentId, sessionId)` plus `budgetKeys.all` / `usageKeys.all`; without `session_id` asserts the null-slot fallback plus the same budget/usage pair), 2 new cases in `misc-mutations.test.tsx` (`useSetSessionModelOverride` with `agentId` asserts exactly 4 invalidate calls including `agentKeys.sessionSnapshots(agentId)` and explicitly NOT the 4-element `agentKeys.session(agentId)` form; without `agentId` asserts only the two session-keyed invalidates fire). Closes #5122. Closes #5123. (@houko)

- **External hook concurrency cap no longer silently bypassed on `SemaphoreClosed` (#5118)** — `ExternalHookSystem::run_hook` previously acquired its `HOOK_CONCURRENCY` permit with `.acquire().await.ok()`, so a closed semaphore returned `None` and the hook ran anyway, defeating the documented system-wide cap on external hook `fork()` rate. The static `LazyLock<Semaphore>` is never closed in practice, but the silent bypass is exactly the kind of "shouldn't happen so it's fine" assumption that turns into a fork-bomb when something upstream changes. Acquire is now factored into a small `acquire_hook_permit` helper that returns `Option<SemaphorePermit<'_>>`: on `Ok(permit)` the hook runs as before; on `Err(SemaphoreClosed)` the helper logs `error!("HOOK_CONCURRENCY semaphore closed; refusing to run hook", hook_name=…, event=…)` and returns `None`, and `run_hook` early-returns without spawning the process. Refusing the hook makes the cap a hard guarantee instead of a best-effort one. Tests: 2 new `#[tokio::test]` cases in `librefang_kernel::hooks::tests` — `acquire_hook_permit_returns_some_on_open_semaphore` (sanity check that a fresh local `Semaphore::new(1)` still yields a permit) and `acquire_hook_permit_returns_none_when_semaphore_closed` (the regression itself: a closed local `Semaphore` must refuse to run a hook). Refs #5118. (@houko)

- **Strict-mode config preserves nested `serde(alias)` declarations and denies unknown fields inside repeated tables (#5129 + #5130)** — closes two silent failure modes in `librefang-types` config validation. (1) schemars (0.8) drops `#[serde(alias = …)]` annotations when generating the JSON Schema, so the strict-mode allowlist derived in `validation.rs` was missing nested aliases such as `terminal.trust_proxy_headers` (alias for `require_proxy_headers`); under `strict_config = true` a legacy config carrying the old spelling was rejected as "unknown field" and boot fell back to `KernelConfig::default()`, silently dropping the operator's intent. Added a `MANUAL_NESTED_ALIASES` constant (mirroring the existing `MANUAL_TOP_LEVEL_ALIASES`) and spliced its entries into the schema-derived nested allowlist, gated on the parent path actually existing in the schema so stale entries cannot widen the allowlist by accident. (2) `KernelConfig::detect_unknown_nested_fields` only walks single-table paths and cannot descend into elements of repeated tables (`[[channels.telegram]]`, `[[mcp_servers]]`, …), so typos inside those entries deserialised into the element's `Default` and the operator's intent never reached the runtime. Added `#[serde(deny_unknown_fields)]` to `TelegramConfig`, `DiscordConfig`, `SlackConfig`, `WhatsAppConfig`, `MattermostConfig`, and `McpServerConfigEntry` so serde itself rejects unknown keys on every element regardless of repeated-vs-single TOML shape. **Operator-visible breakage**: configs with stray / mistyped keys inside any of these six sections now fail to deserialize with a `unknown field …` error; `kernel::config::load_config` logs the error at `warn` and falls back to a full `KernelConfig::default()` for the whole file (api_listen, channels, providers — everything — reverts to defaults until the typo is fixed). The offending field name is in the warn log so the fix is local. The reload path (`try_load_config`) returns `Err(...)` and refuses to apply the bad config, preserving the live in-memory state. Tests: 4 new unit tests in `crates/librefang-types/src/config/mod.rs` (`strict_config_accepts_nested_serde_alias_5129`, `strict_config_rejects_typo_in_repeated_channel_table_5130`, `strict_config_rejects_typo_in_repeated_mcp_servers_table_5130`, `well_formed_repeated_channel_table_still_parses_5130`). The remaining channel structs (Signal/Matrix/Email/Teams/IRC/Twitch/Rocket.Chat/Zulip/XMPP/LINE/Viber/Messenger/Reddit/Mastodon/Bluesky/Feishu/Revolt/Nextcloud/Guilded/Keybase/Threema/Nostr/Webex/Pumble/Flock/Twist/Mumble/DingTalk/QQ/Discourse/Gitter/ntfy/Gotify/LinkedIn/Webhook/Google Chat) were left tolerant in this PR to keep the operator-visible breakage scoped to the issue's stated targets; locking them down can land as a follow-up once we confirm no in-the-wild configs carry stray fields. (@houko)
- **Workflow operator-node validation + dry-run gaps (#4980 review on #5035)** — closes two silent run-time failure modes the original PR series shipped. (1) `Workflow::validate()` now rejects any workflow that combines `depends_on` with an operator-node `StepMode` (Wait / Gate / Approval / Transform / Branch). The DAG executor (`execute_run_dag`) does not match on `StepMode` and would otherwise route operator nodes through `agent_resolver`, surfacing `format_missing_agent_error` at run time instead of the operator's wait / gate / transform / branch behaviour. Wiring the operators into the DAG executor was rejected in favour of the validate gate because Branch's forward-jump semantics interact non-trivially with DAG layer ordering (`Loop` already covers backward jumps, and a forward jump across parallel layers has no obvious meaning). (2) `WorkflowEngine::dry_run` now emits a `_operator:<kind>` row with `agent_found = true` for each operator-node step instead of falling through to `agent_resolver` and reporting them as broken-agent rows; Transform additionally re-runs `validate_transform_template` and surfaces parse errors as `skipped` with a typed reason matching the run-time executor's shape. Also folded into the same change: nit cleanup from the review — the synthetic `StepResult.prompt` slot now carries a unified JSON-object trace shape (`{"op": "<kind>", ...}`) across Wait / Gate / Transform / Branch so a future dashboard renderer can dispatch on `op` alone, Branch's match-path trace now carries the truncated decision input (operators debugging a "wrong arm fired" report could previously see only the arm index), `prompt_template` on Wait / Gate / Approval / Branch is rejected at validate time when it is anything other than the default (the executor silently ignored it, and `Transform` legitimately uses its own `code` field instead), and the stale `record_operator_noop_step_result` docstring is narrowed to Approval (the only remaining caller after steps 2–4 wired their own inline `StepResult`s). Dashboard editor support for operator nodes — distinct styling, inline config fields — is the remaining piece tracked against #4980 and deferred to a separate PR. Tests: 2 new kernel-unit cases (`workflow_validate_rejects_operator_node_combined_with_dag_depends_on` and `workflow_validate_rejects_non_default_prompt_template_on_operator_nodes`) plus 2 acceptance cases (`workflow_validate_accepts_default_prompt_template_on_operator_nodes`, `workflow_validate_accepts_non_default_prompt_template_on_transform`); 3 new integration cases in `workflow_operator_nodes_test.rs` (`validate_rejects_dag_workflow_with_operator_node_step`, `dry_run_reports_operator_nodes_as_found_with_synthetic_agent_names`, `dry_run_marks_unparseable_transform_template_as_skipped`); the existing `branch_step_arm_hit_routes_to_target_and_skips_intermediate_steps` updated to assert the new JSON trace shape. Refs #4980, #5035. (@houko)

### Added

- **Rich workflow invocation — engine-side per-key `{{var}}` substitution from JSON input** (#4982 follow-up, PR #5075). Closes the BLOCKING gap from #4982 review: the runtime's `_artifact` resolver was already landing the resolved handle string in the workflow input JSON, but `WorkflowEngine::execute_run_sequential` / `execute_run_dag` initialised an empty `variables` map and never extracted per-key vars from the input object, so `{{cover}}` / `{{topic}}` in step prompts stayed literal — defeating the whole "agent passes `{cover: {_artifact: …}}` → step receives the handle as `{{cover}}`" promise. New `seed_input_vars_from_json` helper extracts top-level keys at the dispatch boundary (sequential / DAG / dry-run paths) and feeds them into the substitution map; string values pass through verbatim, numbers / bools `to_string()`, nested objects / arrays serialise as compact JSON. Existing entries (resume-snapshot variables, prior-step `output_var` writes) win over the seed via `entry().or_insert()`. `{{input}}` (the whole-blob form) keeps rendering the original blob for backward compat. Surface adds: `kernel-handle` types `WorkflowSummary`, `WorkflowInputParam`, `WorkflowDescription`, `StepOutputSummary`, `WorkflowRunSummary` gain `#[non_exhaustive]` so the staged #4982 follow-ups (param-type strictness, dashboard hints) stay non-breaking — paired with `new()` constructors that downstream crates use in place of struct literals; `regex::Regex::new` at `workflow_runner.rs::describe_workflow` now caches via `std::sync::OnceLock` instead of recompiling per call, and `expect`s rather than silently returning "workflow not found" on a static-pattern bug; `Invalid '_artifact'` error message now interpolates the offending handle string unconditionally so the agent sees it on every failure path, not only the wrong-length one. Tests: 5 new unit in `librefang_kernel::workflow::tests` (`seed_input_vars_from_json` covers all JSON value kinds, the non-object no-op cases, and the preserve-existing-entries semantics; `execute_run_substitutes_per_key_vars_from_object_input` and `execute_run_dag_substitutes_per_key_vars_from_object_input` pin the sequential + DAG dispatch boundaries by capturing the prompt the agent loop would receive). 3 new integration in `crates/librefang-api/tests/workflows_routes_integration.rs` (POST + GET round-trip of `input_schema` rows, PUT replacement semantics, malformed-row skip-WARN policy). 1 new in `crates/librefang-kernel/tests/workflow_integration_test.rs` (`workflow_engine_substitutes_input_schema_vars_into_step_prompt` — end-to-end claim that a workflow with `[[input_schema]]` declaring `topic: string` + `cover: file`, run with `{"topic":"Rust","cover":"sha256:…"}`, dispatches a step prompt with both placeholders filled). (@houko)

- **Rich workflow invocation from agents — parameter discovery, file/image input refs, structured run results** (#4982). Workflows are already launchable via `workflow_run` / `workflow_start`, and the kernel async task tracker (#4983, PR #5033) covers Gap 1 (non-blocking + auto-notify). This PR fills the remaining two gaps and enriches the result shape. (a) Parameter discovery — `Workflow` gains an optional `input_schema: Vec<WorkflowInputParam>` field (each entry: `name`, `param_type` ∈ `string|number|boolean|file|image|agent_id`, `required` defaults true, `description?`). New `workflow_describe(workflow_id)` tool returns `{id, name, description, step_names, input_schema}`; when no explicit schema is authored, the kernel auto-detects parameters by scanning `{{var}}` placeholders across step `prompt_template`s (reserving `{{input}}` for previous-step output as it does today). `workflow_list` adds `has_input_schema` so the LLM knows when calling `workflow_describe` is worthwhile. (b) File/image input — `workflow_run` / `workflow_start` input now accepts `{"_artifact": "sha256:<64-hex>"}` shapes anywhere in the JSON object; the runtime resolves them to bare handle strings before the workflow engine substitutes them into step prompts, so a downstream step can `read_artifact` the bytes. Handle format is validated upfront via `artifact_store::ArtifactHandle::parse`; a malformed `_artifact` reference fails the tool call with a clear error rather than silently rendering `[object Object]` into a step prompt. (c) Structured results — `workflow_run` / `workflow_status` now return `step_outputs: [{step_name, output}, ...]` alongside the final output string, plus `output_json` when the final-step output parses as JSON; the agent can navigate stage results without re-fetching. Surface: kernel-handle gains `WorkflowRunner::describe_workflow`, `WorkflowInputParam`, `WorkflowDescription`, `StepOutputSummary`; `WorkflowSummary` adds `has_input_schema`; `WorkflowRunSummary` adds `step_outputs`. HTTP `POST/PUT /api/workflows` accept `input_schema` arrays; `GET` renders them. Tests: 14 unit tests in `librefang_runtime::tool_runner::rich_workflow_invocation_tests` (artifact-ref resolution at top-level / nested-in-array / invalid handle / multi-key object guard; `prepare_workflow_input` JSON-shape contract; `build_workflow_run_result` shape with and without parsed output_json / step_outputs; `workflow_describe` tool definition + descriptors), 3 in `librefang_kernel::workflow::tests` (TOML + JSON round-trip of `WorkflowInputParam`, default-required-true behaviour), 3 in `crates/librefang-kernel/tests/workflow_integration_test.rs` (explicit `input_schema` surfaces verbatim via `describe_workflow`; auto-detect fallback skips reserved `{{input}}` and sorts deterministically; `workflow_list.has_input_schema` is true for explicit + auto, false when no placeholders). Closes #4982. (@houko)

- **Async task tracker — review fixes for #5033** (#4983). Dedupe `register_async_task` against existing `(run_id)` / `(target_agent, prompt_hash)` so `workflow_start` can't silently orphan a handle on duplicate registration. Wake-idle now acquires `Lane::Trigger` and the per-agent `max_concurrent_invocations` semaphore before `send_message_full`, so operator-set fan-out caps apply uniformly with trigger dispatch. Boot-time recovery sweep walks the registry for tasks tied to runs demoted by `recover_stale_running_runs` and synthesizes `TaskStatus::Failed("workflow run interrupted by daemon restart")` events. Timeout text format pinned by a string-equality unit test in `kernel/handles/workflow_runner.rs`; renderer-drift test in `async_task_tracker_runtime_test.rs` asserts kernel/runtime renderers produce identical bytes. `KernelApi::injection_senders_ref` and `pending_async_task_count` marked `#[doc(hidden)]` to keep test introspection off the public docs surface. (@houko)
- **Async task tracker — runtime consumer + `[async_tasks]` manifest config (step 3/3)** (#4983). Third and final PR of the kernel-level async task tracker proposed in the issue. Builds on step 2 (#5045) to add the runtime-side wake-idle path, the per-agent `[async_tasks]` config block, and end-to-end integration tests through `TestServer`. New `AsyncTasksConfig` struct in `crates/librefang-types/src/agent.rs` carries `default_timeout_secs: Option<u64>` (None = no kernel-imposed default, matching the step-1 "timeout ownership is agent-side" decision) and `notify_on_timeout: bool` (default `true`); wired onto `AgentManifest.async_tasks` with `#[serde(default)]` so existing `agent.toml` files keep parsing. The corresponding entry in `AgentManifest::Default` is added — `CLAUDE.md` "Common Gotchas" specifically calls out missing `Default` impl entries as a silent build-failure trap (#4870). Step-2 kernel registry methods (`register_async_task`, `complete_async_task`, `pending_async_task_count`) plus a test-only `injection_senders_ref` accessor are surfaced on the `KernelApi` trait so integration tests can drive the tracker through the same trait object the dashboard and route handlers use, instead of needing the concrete `LibreFangKernel`. `complete_async_task` gains a wake-idle code path: when the originating session has no live injection receiver attached (because the agent loop is idle between turns), the kernel upgrades the stored `Weak<LibreFangKernel>`, renders the `TaskCompletionEvent` as the same `[System] [ASYNC_RESULT]` line that the runtime's mid-turn handler produces, and spawns a fresh `tokio::task` that drives a new turn via `send_message_full(agent_id, &rendered_text, …, Some(session_id))` pinned to the originating session. The wake-up is detached so the workflow that called `complete_async_task` returns immediately — agents wake on their own time without backpressuring the executor. The renderer logic is duplicated between `librefang-runtime::agent_loop::format_task_completion_text` (mid-turn) and `librefang-kernel::kernel::task_registry::format_task_completion_text` (wake-idle) because the runtime crate cannot re-export back into the kernel (the runtime depends on `librefang-kernel-handle`, not on the kernel directly); both sites produce byte-identical output by convention so session history reads consistently regardless of delivery path. The async-spawn block in `kernel/handles/workflow_runner.rs::start_workflow_async_tracked` now honours `[async_tasks]` settings: it caches the caller agent's `AsyncTasksConfig` at registration time, wraps `execute_run` in `tokio::time::timeout(Duration::from_secs(default_timeout_secs))` when set, and emits `TaskStatus::Failed("workflow run timed out after Ns (agent-side default_timeout_secs)")` on elapsed timeout. `notify_on_timeout = false` suppresses ONLY the timeout-specific event; success and non-timeout failures still surface as today — operationally meaningful for batch agents whose sessions are never read by a human. 7 new `#[tokio::test(flavor = "multi_thread")]` in `crates/librefang-api/tests/async_task_tracker_runtime_test.rs`: `[async_tasks]` block parses out of `agent.toml` (with explicit + missing-block defaults), kernel-handle `register_async_task` + `complete_async_task` round-trip through the `Arc<dyn KernelApi>` exposed on `AppState`, mid-turn delivery to a live receiver, wake-idle spawn when no receiver is attached (the `set_self_handle()` dance that the `boot()` helper performs is documented inline because the wake-idle path silently no-ops without it), `start_workflow_async_tracked` fail-fast on unknown workflow id (the lookup-before-register ordering is operator-visible — pinned by the test so it stays that way), `notify_on_timeout = false` round-trips through agent spawn and lands in the registry verbatim (#4870-style "config field landed in the active state" check), and double-completion via `AppState.kernel.complete_async_task` is a no-op on the second call (only one signal arrives on the channel). 1 additional `#[tokio::test(flavor = "multi_thread")]` in the existing kernel-side test file (`wake_idle_path_returns_true_when_self_handle_is_set`) explicitly exercises the spawn-with-self_handle code path the integration test depends on. Verified with `cargo check --workspace --lib`, `cargo clippy -p librefang-kernel --lib -- -D warnings`, `cargo test -p librefang-kernel --test async_task_tracker_test` (8/8 pass, +1 since step 2), `cargo test -p librefang-api --test async_task_tracker_runtime_test` (7/7 pass, new), `cargo test -p librefang-kernel --test workflow_integration_test` (4/4 pass), `cargo test -p librefang-runtime --test tool_runner_workflow_write` (12/12 pass), `cargo test -p librefang-kernel --test kernel_handle_contract_broader` (6/6 pass — confirms the new `KernelApi` methods on `LibreFangKernel` did not break the broader contract). Refs #4983. Refs #5033 (step 1). Refs #5045 (step 2). (@houko)

- **Async task tracker — kernel registry + event injection (step 2/3)** (#4983). Second of three PRs landing the kernel-level async task tracker proposed in the issue. Builds on the types-only step 1 (#5033) to add the kernel-side substrate: a `HashMap<TaskId, PendingTask>` async-task registry under `EventSubsystem`, two inherent `LibreFangKernel` methods (`register_async_task(agent_id, session_id, kind) -> TaskHandle` and `complete_async_task(task_id, status) -> Result<bool, KernelError>`), and a delivery path that wraps the terminal `TaskCompletionEvent` in a new `AgentLoopSignal::TaskCompleted` variant and pushes it through the existing per-`(agent, session)` mid-turn injection channel (#956) — reusing one mechanism instead of building a parallel one. Migrates the kernel's existing `WorkflowRunId` to a `pub use librefang_types::task::WorkflowRunId` re-export so workflow runs and async-task handles share one canonical newtype. `start_workflow_async` now forwards to a new `start_workflow_async_tracked(workflow_id, input, caller_agent_id, caller_session_id)` `KernelHandle` method that registers a `TaskKind::Workflow` entry against the originating session when both pieces of caller context are supplied; on terminal completion of the spawned `execute_run`, the kernel emits a `TaskStatus::Completed(...)` or `TaskStatus::Failed(...)` and injects it. The `workflow_start` tool in `librefang-runtime/src/tool_runner.rs` is updated to forward its `caller_agent_id` and `session_id` to the new tracked variant so any `workflow_start` invocation from an agent loop is auto-tracked without touching the tool surface. Cron / trigger callers that don't carry an `(agent, session)` keep their previous fire-and-forget behaviour unchanged because the tracker registration is gated behind both ids parsing successfully. Step-1 defaults honoured verbatim: delete-on-delivery (registry entry removed the moment `TaskCompletionEvent` is built, before the injection attempt completes; session history is the durable record); agent-side timeout (no global default, no kernel-side GC sweep); `TaskStatus::Failed(String)` (free-form failure message). `complete_async_task` is idempotent — a second call for the same `TaskId` (e.g. retry-after-error in a future supervisor) hits an empty registry slot and returns `Ok(false)` without emitting a duplicate signal. `BTreeMap` was the initial choice for the registry but switched to `HashMap` after the step-1 types deliberately did not derive `Ord` on `TaskId`; the registry is accessed by exact-key lookup only and never iterated to build an LLM-bound prompt, so the #3298 deterministic-ordering rule does not apply. New file `crates/librefang-kernel/src/kernel/task_registry.rs` (the registry methods); new field `async_tasks: parking_lot::Mutex<HashMap<TaskId, PendingTask>>` on `EventSubsystem`; new variant `AgentLoopSignal::TaskCompleted { event: TaskCompletionEvent }` in `librefang-types/src/tool.rs`; new agent-loop helper `format_task_completion_text` that renders the event as a `[System] [ASYNC_RESULT]` line consumed by the existing mid-turn injection path (step 3 will refine this into a typed turn-start trigger with `[async_tasks]` config). Tests: 7 `#[tokio::test(flavor = "multi_thread")]` in `crates/librefang-kernel/tests/async_task_tracker_test.rs` — registry insert + lookup, workflow-kind completion delivers `TaskCompleted` signal with the right `run_id` and `Completed(value)` payload, delegation-kind delivers with the right target `AgentId` + `prompt_hash` + `Failed(msg)` payload, idle path (no live receiver) still removes the registry entry, unknown `TaskId` returns `Ok(false)` without panic, double-completion is a no-op on the second call (only one signal lands on the channel), and a sanity check that `librefang_kernel::workflow::WorkflowRunId` and `librefang_types::task::WorkflowRunId` are the same nominal type after the re-export migration. Verified with `cargo check --workspace --lib`, `cargo clippy -p librefang-kernel --lib -- -D warnings`, `cargo clippy -p librefang-runtime --lib -- -D warnings`, `cargo test -p librefang-kernel --test async_task_tracker_test` (7/7 pass), `cargo test -p librefang-kernel --test workflow_integration_test` (4/4 pass — `WorkflowRunId` re-export migration is transparent), `cargo test -p librefang-runtime --test tool_runner_workflow_write` (12/12 pass — the stub override migrated to `start_workflow_async_tracked`). Also fixes a pre-existing `dead_code` build failure on `librefang_channels::http_client::warn_ws_proxy_bypass` that blocked `cargo check --workspace --lib` on the wt-4983 worktree (the function is only reachable behind `channel-slack` / `channel-discord` / `channel-mattermost` features; gated the `pub(crate) fn` on the same `any(feature = ...)` clause that gates its callers, plus `test` so the existing smoke test still compiles). Refs #4983. Refs #5033. (@houko)

- **Async task tracker types — typed handle for non-blocking workflow / delegation results (step 1/3)** (#4983). First of three PRs landing the kernel-level async task tracker proposed in the issue. Today, an agent that calls `workflow_run` blocks its conversation loop for the entire duration of the workflow; if the workflow takes minutes, the agent is unresponsive to every other inbound message and a tool-layer timeout surfaces as a dead-end with no `run_id`. The `workflow_start` (async) variant returns a `run_id` immediately but has no mechanism to deliver the eventual result back into the agent's session — by the next user turn the agent has moved on. Production fallout was a Telegram-assistant agent (`ltdata`) being bricked for the duration of a multi-minute workflow run, with the user seeing nothing and an orchestrator agent burning ~$2/day polling the same gap shut. This PR is **types-only** at the bottom of the crate DAG so the kernel and runtime work that lands in steps 2 and 3 has a stable interface to import — no behaviour change, no kernel wiring, no agent-loop changes. New module `crates/librefang-types/src/task.rs` adds: `TaskId(pub Uuid)` (newtype with `new()` / `Default` / `Display` / `FromStr`) — kernel-assigned identifier for a registered async task; `WorkflowRunId(pub Uuid)` (same shape) — colocated here because `librefang-types` cannot import the kernel and step 2 will migrate the kernel's own `WorkflowRunId` to this canonical definition; `TaskKind` — `#[serde(tag = "kind", rename_all = "snake_case")]` enum with `Workflow { run_id }` and `Delegation { agent_id, prompt_hash }` variants, externally tagged so additive variants in steps 2/3 (`ExternalWebhook`, `LongRunningTool`, …) do not break wire compatibility with already-registered handles; `TaskHandle { id, kind, started_at }` — the typed handle returned synchronously to an agent that spawns an async task, holdable across turns; `TaskStatus` — `#[serde(tag = "status", content = "value")]` enum with `Pending` / `Running` / `Completed(serde_json::Value)` / `Failed(String)` / `Cancelled` arms; `TaskCompletionEvent { handle, status, completed_at }` — the wire payload the kernel will inject into the originating agent's session in step 2. Three design decisions are defaulted in module-level rustdoc and called out for re-discussion in steps 2/3: **(1) Cleanup semantics** — a registered task is removed from the kernel registry the moment its `TaskCompletionEvent` is delivered into the originating session; no retention window, no replay, session history is the durable record. **(2) Timeout ownership** — timeouts are agent-side. The spawning agent passes a deadline when it registers the task; the kernel does not impose a global default, keeping the "how long is too long for THIS operation?" policy with the caller that actually knows the answer. **(3) Error shape** — `TaskStatus::Failed(String)` is conservative on purpose. A richer typed-error variant can land later as an additive enum arm without breaking on-disk or wire formats (serde will continue to deserialise the existing `String` form, and new variants deserialise into their own arms). Module re-exports surfaced from `crates/librefang-types/src/lib.rs`: `TaskCompletionEvent`, `TaskHandle`, `TaskId`, `TaskKind`, `TaskStatus`, `WorkflowRunId`. Tests in `crates/librefang-types/src/task.rs` (6 unit tests, all wire-format round-trips against serde_json): `task_status_serde_roundtrip` (Pending / Running / Completed-with-JSON-payload / Failed-with-message / Cancelled all survive the round-trip), `task_kind_serde_roundtrip` (Workflow + Delegation variants), `task_completion_event_full_roundtrip` (the full struct including nested handle + status), `task_status_failed_preserves_message` (pins the error-shape contract — a free-form failure message is faithfully preserved across the wire), `task_completion_event_delegation_roundtrip` (delegation kind + Cancelled status combination), `task_id_display_and_parse_roundtrip` (Display / FromStr symmetry). **Step 2** (separate PR) will add the kernel pending-task registry and the event-injection path on `EventBus`; **step 3** (separate PR) will teach the agent loop to recognise the injected `TaskCompletionEvent` and surface it as a new turn. Verified with `cargo check -p librefang-types --lib`, `cargo clippy -p librefang-types --lib --all-targets -- -D warnings`, `cargo test -p librefang-types --lib task::` (6/6 pass). Refs #4983. (@houko)


- **Workflow `Branch` operator executor (#4980 step 4/N)** — wires the previously-stubbed `StepMode::Branch` to exact-match dispatch on the previous step's output. The dispatcher parses the previous output as JSON when possible (so numeric and structural match values compare by JSON deep-equality), iterates the arms in declaration order, and forward-jumps to the first matching `arm.then` step by name. No arm matches → run halts with `WorkflowRunState::Failed` and a reason naming the unmatched output (truncated at 200 chars so a multi-MB predecessor output cannot blow up the error string). Target step missing → halt with a reason naming the missing target. Target step at or before the current index → halt with a "backward jumps not allowed" reason — `Loop` already exists for that semantic, so a Branch that targets backwards is almost always an operator typo and silently allowing it would let an unbounded loop hide inside a Branch. The decided arm is recorded in the synthetic `StepResult.prompt` as `branch -> '<target>' (arm <idx>)` so the dashboard run history surfaces *which* arm fired without re-resolving the comparison. Skipped arms are skipped — no `_operator:branch` `StepResult` is emitted for the arms that did not match. Design decision (deferred from step 1, locked in step 4): exact equality on V1 matching the proposal in step 1's PR body. Range / regex / in-set matchers can land as additive `BranchArm` fields later (`match_range`, `match_regex` with exactly-one-of validation) without breaking the wire format. Forward jumps only — backward jumps would let an unbounded loop hide inside Branch when `Loop` already covers that. Tests: 3 new integration tests in `workflow_operator_nodes_test.rs` (`branch_step_arm_hit_routes_to_target_and_skips_intermediate_steps` driving two separate workflows with different seed literals and target terminals; each run's step trail is asserted to skip the two intermediate Transform steps that sit between the Branch and the named target; `branch_step_no_arm_match_halts_workflow_with_recorded_reason` covering the explicit-halt path with the full step trail asserted; `branch_step_no_match_solo_halts_workflow` pinning the single-step Branch path). Tests now total 14 in the operator-node integration test file (was 11 → +3 new Branch tests, -1 old stub test retired). Refs #4980, #5035 (step 1), #5044 (step 2), #5046 (step 3). (@houko)

- **Workflow `Transform` operator executor with Tera (#4980 step 3/N)** — wires the previously-stubbed `StepMode::Transform` to a Tera-rendered template against the previous step's output. The template context exposes `prev` (raw string), `prev_json` (only when the predecessor output parses as JSON, so a template that references `prev_json` against a non-JSON predecessor surfaces a clear "variable not found" Tera error rather than silently rendering empty), and `vars` (a `BTreeMap<String, String>` of `output_var`-bound workflow variables — `BTreeMap` for deterministic iteration order per #3298 so prompt caches stay valid). Tera was picked over `mlua` / `rhai` / a hand-rolled DSL because it ships sandboxed by default (no I/O, no shell escape, bounded recursion), is MIT-licensed and well-maintained, and adds the smallest delta to the dependency tree. `shell_exec` was explicitly NOT considered. Render failures (missing variable, syntax error reached at run time) halt the run with the Tera error wrapped as `transform render failed: <Tera message>` — Tera errors include line / column information, so the operator can pin the bad placeholder without re-running the workflow. New `Workflow::validate()` method on `librefang_kernel::workflow::Workflow` parses every `StepMode::Transform` template at manifest-load time and surfaces syntax errors as `Vec<(step_name, reason)>`, so an unterminated `{{ prev` or `{% if %}` without `{% endif %}` blows up before any run starts rather than at run time. The rendered template replaces `current_input` so downstream `{{input}}` consumers see the formatted output. Adds `tera = "1"` as a new workspace dependency (default-features disabled — the renderer is sandboxed; the `builtins` feature that adds filesystem-touching functions stays off). Tests: 3 new integration tests in `workflow_operator_nodes_test.rs` (happy template render, missing-variable halt with recorded reason, syntax error caught at load time via `validate`); 7 new kernel unit tests covering raw-prev render, `prev_json` indexing, `vars.<name>` exposure, missing-variable error wrapper, the `validate_transform_template` parse helper happy and failure paths, and `Workflow::validate` surfacing transform-step errors with step names attached. Refs #4980, #5035 (step 1), #5044 (step 2). (@houko)

- **Workflow `Gate` operator executor (#4980 step 2/N)** — wires the previously-stubbed `StepMode::Gate` to a declarative comparator AST evaluated against the previous step's output, so workflows can express "branch on score > 0.8" / "status == approved" / "tags contains beta" without an LLM call. The shape locked in from step 1's deferred-design slot: `Gate { condition: GateCondition }` where `GateCondition` is `{ field: Option<String> (RFC 6901 JSON Pointer), op: GateOp, value: serde_json::Value }` and `GateOp` is the boring `Eq | Ne | Gt | Lt | Gte | Lte | Contains` set. Typed AST chosen over a string DSL because a string would force a one-shot wire-format commitment incompatible with a future richer expression language; this shape is additive — new operators (regex, in-set, range) land as new `GateOp` variants without touching anything else. The executor `evaluate_gate_condition` resolves the JSON Pointer into the previous step's output (falling back to raw-string compare when the output isn't JSON), runs the operator (numeric path for ordering ops when both sides parse as f64, lexicographic otherwise; deep JSON equality for `Eq`/`Ne`; substring for `Contains`), and either routes execution onwards (pass) or halts the run with `WorkflowRunState::Failed` and a recorded reason naming the gate / field / op (fail). The blocking step still appears in `run.step_results` so the dashboard run history surfaces *which* gate stopped the workflow; the step's `output` field carries the failure reason for inline display. A malformed condition (missing `op`, unknown operator, wrong types) surfaces as a serde deserialisation error at manifest-load time — the gate cannot default to "passing" silently. The HTTP `POST /api/workflows` route's flat-string `"gate"` parser now reads `condition` as a typed object through `serde_json::from_value::<GateCondition>` and fails closed (`Eq` against `Value::Null`, which fails any real input) on malformed payloads rather than the previous silent "default to empty string" behaviour. Approval stays a no-op-with-warn — explicitly blocked on #4983 (async-task tracker) which is being driven in parallel; the warn message and an in-source `TODO(#4983)` marker call out the cross-issue dependency so the stub state is self-documenting. Integration tests in `crates/librefang-api/tests/workflow_operator_nodes_test.rs`: 4 new `#[tokio::test]` cases (`gate_step_passes_and_routes_onwards`, `gate_step_fails_and_halts_workflow_with_recorded_reason`, `gate_step_malformed_condition_fails_deserialization_at_load_time`, `gate_step_completed_when_field_omitted_compares_whole_input`) plus the old `gate_step_is_noop_with_warn_and_completes` retired. Kernel unit tests: 6 new in `librefang_kernel::workflow::tests` covering the comparator AST, the resolver fallback paths, and the deserialisation-failure-on-missing-`op` contract. Refs #4980, #5035 (step 1). (@houko)

- **Workflow operator-node step modes (#4980 step 1/N) — `Wait` executor + types-only landing for `Gate` / `Approval` / `Transform` / `Branch`**. Every workflow step previously required an agent dispatch, which meant a real LLM call for trivial control-flow operations like "wait 5 minutes", "branch on score > 0.8", or "render the output as Markdown" — wasting tokens and forcing the dashboard's visual editor to expose every node as an agent-shaped box. This PR adds five new variants to `librefang_kernel::workflow::StepMode` so workflow definitions can express zero-LLM-token operations: `Wait { duration_secs: u64 }`, `Gate { condition: String }`, `Approval { recipients: Vec<String>, timeout_secs: Option<u64> }`, `Transform { code: String }`, and `Branch { arms: Vec<BranchArm> }` where `BranchArm` is a new public struct `{ match_value: serde_json::Value, then: String }`. The match value is a typed `serde_json::Value` rather than a stringified expression so downstream tooling (dashboard, dry-run linter, future workflow analyzer) can inspect the branch tree without re-parsing. All five variants serde-round-trip cleanly — verified by 7 new `librefang-kernel::workflow::tests` cases (`test_step_mode_wait_serialization`, `test_step_mode_gate_serialization`, `test_step_mode_approval_serialization`, `test_step_mode_approval_timeout_optional` pinning the absent-`timeout_secs` case from the issue's TOML example, `test_step_mode_transform_serialization`, `test_step_mode_branch_serialization`). Only `Wait` is fully wired in this PR: its executor in the sequential workflow loop calls `tokio::time::sleep(duration_secs)` raced against the run's `cancel_notify` so a long Wait (e.g. `Wait { 86400 }`) still honours `WorkflowEngine::cancel_run` at sub-step granularity instead of ignoring it for a full day; records a synthetic `StepResult` with `agent_name = "_operator:wait"`, `agent_id = ""`, `input_tokens = 0`, `output_tokens = 0`, and `duration_ms` reflecting the actual sleep; preserves `current_input` verbatim so downstream `{{input}}` and `output_var` substitutions still work. The other four variants land as no-op executors that emit a structured `warn!` log (`<variant> executor not yet implemented — refs #4980`) and return success, with a shared `record_operator_noop_step_result` helper so all four arms read identically and tag the step result with `_operator:gate` / `_operator:approval` / `_operator:transform` / `_operator:branch` for log-side discoverability. The no-op-with-warn shape keeps the wire format usable from day one — workflows that include these variants serialise, persist to SQLite, and round-trip through pause/resume without error — while leaving the design questions on each variant's body open: Gate.condition string-form vs declarative shape (`{ field: "score", op: "gt", value: 0.8 }`), Approval operator-identity model (per-channel UUIDs vs free-form strings vs the #4977 `Recipient` type), Transform.code expression-language vs registered-functions (Tera/Handlebars/Rhai/WASM extension), and Branch jump semantics + default-arm shape. The HTTP `POST /api/workflows` route's `parse_step_mode` learns five new flat-string forms (`"wait"`, `"gate"`, `"approval"`, `"transform"`, `"branch"`) that pull their config from sibling fields on the step JSON object, mirroring the legacy `"conditional"` / `"loop"` shape so the dashboard and the TOML examples in the issue body can write `mode = "wait"` with sibling `duration_secs = 5`. Integration tests in `crates/librefang-api/tests/workflow_operator_nodes_test.rs`: 6 `#[tokio::test(flavor = "multi_thread")]` cases driving the engine directly with a `panicking_agent_resolver` + `panicking_send_message` pair that pins the contract "operator-node executors must never dispatch to an agent" — `wait_step_completes_after_duration_and_skips_agent_dispatch` (elapsed ≥ 950ms lower-bound, output preserved, step result fields), `wait_step_zero_duration_completes_immediately`, and one no-op-with-warn smoke per Gate / Approval / Transform / Branch asserting Completed state and the matching `_operator:<kind>` tag. The DAG executor branch is intentionally untouched in this PR — it does not match on `StepMode`, so operator nodes in DAG workflows would attempt an agent dispatch; widening the DAG executor lands as a follow-up alongside the four no-op-with-warn executor bodies (the remaining four variants are each scoped as their own follow-up PR so the deferred design questions can be debated independently rather than bundled). Refs #4980. (@houko)


- **Declarative `[[triggers]]` in `agent.toml`** (#5014). Event triggers can now be declared in the manifest alongside the existing API/CLI creation paths, so trigger definitions live with the agent in version control, migrate with the workspace, and are recreated reproducibly after a fresh install or reset. New `ManifestTrigger` shape (`pattern` / `prompt_template` / `max_fires` / `cooldown_secs` / `session_mode` / `target_agent` / `workflow_id` / `enabled`) on `AgentManifest`. On agent spawn, hot-reload (`POST /api/agents/{id}/reload`), and daemon boot, the kernel reconciles the manifest list against the existing `trigger_jobs.json` store: missing entries are created, matching `(pattern, prompt_template)` keys have their mutable fields updated in place (TOML wins), and runtime-only triggers created via `POST /api/triggers` or `librefang trigger create` are governed by a new per-agent `reconcile_orphans` field (`"keep"` default — never delete an API-created trigger silently; `"warn"` — log; `"delete"` — reap). The reconcile is idempotent: re-applying the same manifest is a no-op, no persist thrash. `target_agent` is declared by **name** in the TOML and resolved to an `AgentId` at reconcile time via the registry's `find_by_name`; unresolved names log a warning and register the trigger without a target rather than failing the whole reconcile. Invalid pattern entries are skipped with a per-entry warning so one malformed `[[triggers]]` block never aborts the rest. Tests: 6 unit in `librefang_types::agent::tests` (defaults, full-shape parse, per-field defaults, `OrphanPolicy` serde, TOML round-trip), 11 unit in `librefang_kernel::triggers::tests::reconcile_*` (create / update / idempotency / each `OrphanPolicy` arm / name resolution / unresolvable target / invalid pattern skip / `"task_posted"` string normalisation / `enabled=false` persists), 5 integration in `crates/librefang-api/tests/declarative_triggers_test.rs` (register-on-spawn, idempotency across reloads, Keep preserves API-created, Delete reaps API-created, in-place update with stable id). Closes #5014. (@houko)



- **`librefang-rl-export`: security & reliability hardening on PR #5034 review** (#3331). Five blocking review items addressed inside the rl-export crate; no kernel-wide surface change. (1) **Rename** `TrajectoryExport` → `RlTrajectoryExport` so the public type stops colliding with the kernel's `TrajectoryExporter` (session audit trail vs RL rollout egress — two entirely different concepts). (2) **`*_env` indirection on every secret-bearing field**: `ExportTarget::WandB.api_key: String` → `api_key_env: String` (env-var name), same for `Tinker.api_key_env`; resolution happens at upload time via `resolve_env_secret()`, fail-closed with `InvalidConfig` on missing / empty env var. Matches the workspace `client_secret_env` / `api_key_env` convention and keeps secrets out of `config.toml`. (3) **SSRF egress allowlist**: new `crate::ssrf` module duplicates the policy of `librefang_runtime_mcp::mcp_oauth::is_ssrf_blocked_url` (loopback / RFC-1918 / link-local / IMDS / userinfo-smuggling / non-http schemes / IPv4-mapped-and-NAT64 IPv6 all rejected) with two modes — `Public` for W&B / Tinker, `LoopbackOrPrivate` for Atropos — gated in the public `export()` entry point before any I/O. Atropos's implicit `http://localhost:8000` default is removed: `base_url` is now `String` (required), so operators make the loopback decision explicitly and the variant can never accidentally hit a public host. W&B's `entity: Option<String>` is now `String` (required) — the prior `"default"` fallback was a guess at an undocumented W&B "personal entity" convention that would silently land runs under a wrong-named bucket. (4) **Redaction of `toolset_metadata`** before egress: new `crate::redact` module mirrors the kernel's `RedactionPolicy` regex set (`api_key`-shaped strings, JWTs, long base64 blobs) and is applied to the metadata blob in both the W&B `create-run` body and the Tinker `create_session` body. The two regex sets are intentionally duplicated rather than imported because pulling `librefang-kernel` into a leaf egress crate inverts the dep layer (the kernel must not depend on rl-export) and drags ~50 transitive crates for three patterns; the two sets must change together — flagged in the module rustdoc. (5) **Retry with exponential backoff** on transient failures: new `crate::retry` module wraps every upload call (W&B create-run + upload-file, Tinker create-session + telemetry, Atropos register-env + scored_data) in 3 attempts with 200ms / 400ms backoff. `is_transient` matches the workspace-standard set — network drops, 5xx, 429 — and leaves `AuthError`, `InvalidConfig`, 4xx (non-429), `MalformedResponse`, and `TrainerNotReady` permanent. Linked review nits resolved in the same commit (in scope per the "fix what you found" rule): `ExportError` now `#[non_exhaustive]`; dedicated `TrainerNotReady { status_label }` variant replaces the synthetic-503-from-200-sentinel hack so the condition is pattern-matchable without parsing the body and no longer collides with a real 503; `ExportTarget::Atropos` gains optional `max_token_length` / `group_size` / `weight` tuning knobs so operators don't have to fork the crate; Tinker `tags` sorted before send for byte-identical wire output (refs #3298 prompt-cache determinism); parameter shadowing `export: export` renamed to `payload`; "Step 1 of 3" framing dropped from the crate-level rustdoc. Tests: +6 (2 new security E2E — `wandb::tests::toolset_metadata_is_redacted_before_upload` asserts a `sk-live-…` literal in tool-result metadata is replaced with `<REDACTED:CREDENTIAL>` in the mock-received body; `tests::export_rejects_tinker_base_url_at_imds` asserts a Tinker base_url at `169.254.169.254` surfaces as `InvalidConfig`, not a successful upload; `tests::export_rejects_atropos_public_base_url` asserts Atropos at a public host is also rejected) plus 11 unit tests across the new `ssrf` / `redact` / `retry` / env-secret helpers; previously-failing tests rewritten for the new signature. Verification: `cargo check --workspace --lib`, `cargo clippy -p librefang-rl-export --all-targets -- -D warnings`, `cargo test -p librefang-rl-export` (46 passed, 0 failed). Refs PR #5034 review. (@houko)

- **New `librefang-rl-export` crate — long-horizon RL rollout trajectory exporter, W&B integration first** (#3331). Step 1 of 3 on the issue. Adds `crates/librefang-rl-export/` with the public surface (`ExportTarget`, `TrajectoryExport`, `ExportReceipt`, `export()`, `ExportError`) plus a private `wandb` module implementing the Weights & Biases REST flow: `POST {base}/api/runs` to create the run, then `POST {base}/files/<entity>/<project>/<run_id>` to upload the opaque trajectory bytes as a single file artefact under that run. Authentication uses W&B's documented HTTP Basic convention with the literal user `api` and the API key as the password (`Authorization: Basic base64("api:<key>")`); 401 / 403 collapse into `ExportError::AuthError` so the operator is prompted to refresh credentials rather than seeing the raw rejected token echoed back from some upstream error bodies, other non-2xx responses surface as `ExportError::UpstreamRejected { status, body }` with the body truncated to 4 KiB so a pathological upstream cannot bloat the error. All outbound HTTP flows through `librefang_http::proxied_client()` — the workspace shared client carrying the operator's `[proxy]` config, TLS fallback roots, and the canonical `User-Agent: librefang/<version>` — so this crate adds no bespoke reqwest plumbing per the `librefang-extensions` AGENTS.md "no bespoke `reqwest::Client`" rule. The exporter is intentionally **wire-format-agnostic**: `TrajectoryExport.trajectory_bytes: Vec<u8>` is opaque and forwarded to the upstream verbatim, so this crate does **not** depend on #3330's wire-format RFC and can land and be integration-tested today; once #3330 locks the on-the-wire serialization the exporter API is unchanged. `ExportTarget` is `#[non_exhaustive]` so the follow-up PRs (#3331 step 2 — Tinker; #3331 step 3 — Atropos) add additive variants without breaking callers. Tests: 6 `#[tokio::test]` cases in `crates/librefang-rl-export/src/wandb.rs` against `wiremock::MockServer` (`export_happy_path_creates_run_then_uploads_bytes` pins the two-call sequence, the Basic-auth header shape, and the `target_run_url` / `bytes_uploaded` receipt fields; `export_falls_back_to_default_entity_when_unset` pins that the upload path uses the `default` placeholder so W&B resolves the personal entity server-side; `export_maps_401_to_auth_error` and `export_maps_other_4xx_to_upstream_rejected_with_body` pin the status-classification split; `empty_api_key_is_rejected_before_any_http` pins the `InvalidConfig` short-circuit by pointing at an invalid base URL that must never be contacted; `basic_auth_uses_api_user_placeholder` pins the `api:<key>` Basic-auth convention so a future refactor cannot silently switch to a bare-key shape). Refs #3331. (@houko)

- **`librefang-rl-export`: Atropos exporter** (#3331 step 3 of 3). Additive `ExportTarget::Atropos { project, base_url }` variant on the existing `#[non_exhaustive]` enum + new private `atropos` module — non-breaking against the W&B (step 1) and Tinker (step 2) PRs. Atropos (<https://github.com/NousResearch/atropos>) is NousResearch's LLM RL environments framework — a FastAPI microservice mediating between rollout producers and a trainer process, running locally (default `http://localhost:8000`) with no auth (`atroposlib/api/server.py` adds `CORSMiddleware` but no auth middleware). The exporter maps the rollout onto Atropos's producer-side `register-env` / `scored_data` pair: `POST {base}/register-env` to register this rollout under a `desired_name` and recover the server-assigned `env_id` + `wandb_name` (request body matches the `RegisterEnv` Pydantic model: `max_token_length`, `desired_name`, `weight`, `group_size`, `min_batch_allocation`), then `POST {base}/scored_data` to submit the opaque trajectory bytes verbatim as a `ScoredData` JSON payload (`Content-Type: application/json`). `TrajectoryExport.trajectory_bytes` MUST already be valid `ScoredData` JSON for Atropos (`tokens`/`masks`/`scores`/...); the exporter forwards the bytes verbatim and lets Atropos validate — invalid payloads surface as `UpstreamRejected{status: 422, body}` with Atropos's Pydantic error body. Default `base_url` is `http://localhost:8000` matching the Atropos `run-api` default; `ExportTarget::Atropos.base_url: Option<String>` lets operators override (tests use this for `wiremock::MockServer`). **Trainer-not-ready handling**: Atropos's `register-env` is gated by `app.state.started` — if the trainer process hasn't called `/register` (a trainer-only endpoint NOT in this exporter's surface), the server returns HTTP 200 with the sentinel body `{"status": "wait for trainer to start"}` and *no* `env_id`. The exporter detects the missing `env_id` and converts that overloaded 200-as-busy into a synthetic `UpstreamRejected { status: 503, body }` so callers see a retry-after-trainer-up signal rather than `MalformedResponse`. `ExportReceipt.target_run_url` returns `{base}/latest_example#env={wandb_name}` — Atropos has no browser-loadable run-URL concept (it's a local microservice) but `/latest_example` is its documented debug-inspection endpoint, so an operator can `curl {base}/latest_example` to verify the upload landed. Error-classification mirrors the W&B / Tinker exporters exactly (401/403 → `AuthError` for reverse-proxy-fronted deployments; other non-2xx → `UpstreamRejected{status,body}` truncated to 4 KiB); all HTTP flows through `librefang_http::proxied_client()`. Tests: 6 `#[tokio::test]` in `crates/librefang-rl-export/src/atropos.rs` against `wiremock::MockServer` mirroring the W&B / Tinker test shape (`export_happy_path_registers_env_then_submits_scored_data` pins the two-call sequence + `RegisterEnv` body shape via `body_partial_json` + `ScoredData` payload round-trip + receipt URL with the server-assigned `wandb_name`; `export_translates_trainer_not_ready_to_upstream_rejected_503` pins the sentinel 200-as-busy → synthetic 503 conversion so this contract cannot silently break; `export_maps_401_to_auth_error_for_proxy_fronted_deployments` pins the auth-collapse for reverse-proxied deployments; `export_maps_422_validation_failure_to_upstream_rejected_with_body` pins Atropos's Pydantic 422 path with the upstream body forwarded; `empty_project_is_rejected_before_any_http` and `empty_trajectory_bytes_is_rejected_before_any_http` pin the `InvalidConfig` short-circuit against a base URL that must never be contacted). Closes #3331. (@houko)

- **`librefang-rl-export`: Tinker exporter** (#3331 step 2 of 3). Additive `ExportTarget::Tinker { api_key, project, base_url }` variant on the existing `#[non_exhaustive]` enum + new private `tinker` module — non-breaking against the W&B-only PR. Tinker's public REST surface (<https://thinkingmachines.ai/tinker/>, SDK at <https://github.com/thinking-machines-lab/tinker>) is built around training calls (`/api/v1/forward`, `/api/v1/forward_backward`, `/api/v1/optim_step`) and session-scoped telemetry; there is no Tinker-side "opaque trajectory upload" endpoint today. The exporter maps the rollout onto the closest stable two-call pair Tinker actually accepts: `POST {base}/api/v1/create_session` to register a session and recover its server-assigned `session_id`, then `POST {base}/api/v1/telemetry` to submit a single `GenericEvent` whose `event_data` carries the base64-encoded opaque trajectory bytes + rollout window timestamps + caller-side run id under that session. Default base URL `https://tinker.thinkingmachines.dev/services/tinker-prod` matches the Tinker Python SDK's `TINKER_BASE_URL` fallback; `ExportTarget::Tinker.base_url: Option<String>` lets operators on a self-hosted control plane override (tests point this at `wiremock::MockServer`). Authentication is `X-API-Key: <api_key>` per Tinker's `ApiKeyAuthProvider`; the SDK requires keys start with `tml-` but this crate forwards the key verbatim and lets the upstream enforce the prefix so JWT-style credentials surfaced by `TINKER_CREDENTIAL_CMD` still flow through. `ExportReceipt.target_run_url` returns the literal `{base}/api/v1/get_session/{session_id}` URL pattern that the Tinker SDK's `service.get_session(session_id)` convention exposes, so an operator can click through to the session. Error-classification mirrors the W&B exporter exactly (401/403 → `AuthError`; other non-2xx → `UpstreamRejected{status,body}` with body truncated to `MAX_ERROR_BODY_BYTES = 4096`); all HTTP flows through `librefang_http::proxied_client()` so the operator's `[proxy]` + TLS fallback apply uniformly. Tests: 6 `#[tokio::test]` in `crates/librefang-rl-export/src/tinker.rs` against `wiremock::MockServer` mirroring the W&B test shape (`export_happy_path_creates_session_then_submits_telemetry` pins the two-call sequence + `X-API-Key` header on both calls + receipt URL using the server-assigned session id; `export_forwards_trajectory_bytes_as_base64_event_data` pins the base64 wire shape via `body_partial_json` so a future refactor cannot silently switch encoding; `export_maps_401_to_auth_error` and `export_maps_other_4xx_to_upstream_rejected_with_body` pin the status-classification split; `empty_api_key_is_rejected_before_any_http` and `empty_project_is_rejected_before_any_http` pin the `InvalidConfig` short-circuit against a base URL that must never be contacted). Assumption flagged for maintainer review in the module-level rustdoc + the PR body: if Tinker ships a dedicated trajectory-upload endpoint in a future release, this module should switch to it; the `create_session + telemetry` pair is the closest stable target against the current SDK source. Refs #3331. (@houko)

- **Configurable prompt-cache breakpoint strategy for Anthropic and compatible providers** (#4970). The driver already placed cache breakpoints at system + tools-last + the last 3 messages (`system_and_3`, the strategy used by Hermes Agent for the ~75% input-token savings reported on Anthropic), but the placement was hard-coded — operators couldn't dial it back for thrashy workloads or disable it independently of the global `prompt_caching` master switch. New `[prompt_cache]` config section with `strategy = "disabled" | "system_only" | "system_and_<N>"` and `cache_ttl_hint_secs` (default `300`). `PromptCacheStrategy::SystemAndN(N)` is parametric — `N` is a *hint*; Anthropic's 4-breakpoint hard cap is enforced by the driver in most-stable-first order (system → tools-last → newest message backward), so `system_and_8` still emits at most 4 markers and never over-spends the budget. `prompt_caching = false` (master switch) wins over any per-request strategy, preserving the global kill switch for operators who don't want cache hints on any provider. Surface: a parsed string round-trips through `PromptCacheStrategy::FromStr` + custom serde with `deny_unknown_fields` on the section, so a typo like `strategy = "sytem_and_3"` fails at config load with an error pointing at the bad value instead of silently falling back to a default. Wire-through: kernel forwards `prompt_cache.strategy` as a string via existing per-agent manifest metadata; agent loop parses back into the enum and sets `CompletionRequest.prompt_cache_strategy`; only the Anthropic driver currently honours the field — OpenAI/DeepSeek cache automatically above their own length thresholds (no per-request annotation needed) and Gemini's `cached_content` API is deferred to a follow-up issue since it requires server-side context registration rather than per-request annotation. Tests: 11 in `librefang_types::config::types::prompt_cache_tests` (parse happy paths incl. `system_and_0`, `system_and_255`; rejection of negative tail, non-numeric tail, u8 overflow, typos, empty; display round-trip; default = `system_and_3`; serde via string; serde error mentions bad value; TOML round-trip; `deny_unknown_fields`; helpers), 6 new in `librefang_llm_drivers::drivers::anthropic::tests` (`strategy_disabled_emits_no_markers`, `strategy_system_only_marks_only_system`, `strategy_system_and_zero_marks_tools_but_no_messages`, `strategy_system_and_n_clips_to_4_breakpoint_cap` exercising the 4-cap with `system_and_8`, `strategy_none_falls_back_to_system_and_3` for backward compatibility, `master_switch_off_suppresses_strategy`, `strategy_system_and_3_snapshot_json_shape` literal-string compare on the wire body). Closes #4970. (@houko)

- **New `web_fetch_to_file` tool — fetch a URL straight into a workspace file without round-tripping the body through the model** (#4964). Information-gathering agents (research, ingestion, scraping) previously had two bad options when `web_fetch` returned a body too large to want in context: regenerate it through `file_write` (burning tokens proportional to the body) or lose it. The escape hatch — `shell_exec curl ...` — was blocked under `Allowlist` mode by `contains_shell_metacharacters` (`?`/`&`/`*` in URLs trip the metachar check at `tool_runner.rs:1355-1369`), and a downgrade to `Full` mode also lifts every other shell restriction on that agent — not the trade-off researchers asked for. New built-in `web_fetch_to_file(url, dest_path)` (`crates/librefang-runtime/src/web_fetch_to_file.rs`) streams the response body directly to a workspace-relative path; the agent receives only a short summary line (`Wrote N bytes to ... (sha256:..., content-type: ..., status: ...)`). Same SSRF protection, DNS pinning, and redirect re-validation as `web_fetch` (reuses `WebFetchEngine::pinned_client` and `check_ssrf`, now exposed `pub(crate)`); same taint scans (`check_taint_net_fetch` / outbound text / outbound header) in the dispatch arm; same workspace-jail and read-only named-workspace pre-flight checks as `file_write`. `WebFetchConfig.max_file_bytes` (default 50 MiB) caps download size; per-call `max_bytes` is clamped down to this hard ceiling, never up. Stream-reads via `Response::chunk()` so a server that omits or lies about `Content-Length` cannot push past the cap. Lives in a new module rather than growing `tool_runner.rs`/`agent_loop.rs` (per `crates/librefang-runtime/CLAUDE.md`; both files are slated for #3710 split). Tests: 9 `#[tokio::test]` in `crates/librefang-runtime/tests/web_fetch_to_file_test.rs` (happy path with sha256 + content-type match, `..` rejection, absolute-path-outside-workspace rejection, configured `max_file_bytes` overflow, per-call clamp, SSRF loopback block, HTTP 4xx pass-through, missing required params), plus 5 `clamp_max_bytes` unit tests next to the impl. (@houko)

- **Email channel: `tls_root_ca_path` and `tls_accept_invalid_certs` for self-hosted IMAP behind self-signed / expired certs** (#4877). The IMAP poll path used `RustlsConnector::new_with_native_certs()` with no operator-controlled escape hatch, so a self-hosted IMAP server behind a private CA (or with an expired self-signed cert) failed every poll with `TLS handshake failed: IO error: invalid peer certificate: certificate expired` and the only workaround was renewing the cert. Two new fields on `EmailConfig` cover the two real-world cases: (1) `tls_root_ca_path: Option<String>` — additionally trust certificates from a PEM file on top of the system root store; hostname / expiry / signature / chain validation **stay ON**. This is the recommended path for self-hosted IMAP behind a private CA. Multiple certs in one PEM are supported; missing file, empty file, and garbage non-PEM input each return a distinct error mentioning the path, so operators can locate the typo without reading source. (2) `tls_accept_invalid_certs: bool` (default `false`) — last-resort dev escape hatch that disables ALL certificate validation (hostname, expiry, signature, chain). Implemented as a custom `rustls::client::danger::ServerCertVerifier` that accepts unconditionally; advertises every standard `SignatureScheme` so peers don't filter the (no-op) verification. A WARN log fires on **every** IMAP connect attempt while this is enabled (every poll cycle and every flag-update RPC, ~30s default cadence), so the MITM-vulnerability stays visible in operator logs rather than being noticed once at startup and forgotten. Both knobs flow through new `EmailAdapter::with_tls_root_ca_path` / `with_tls_accept_invalid_certs` builder methods (no breaking change to `EmailAdapter::new`'s 11-arg signature). The TLS connector construction is factored into a single `build_imap_tls_connector` helper used by both `fetch_unseen_emails` and `mark_uids_outcome`, so the two TLS-using sites can never drift. Adds `rustls-pemfile = "2"` as a workspace dep for parsing the user-supplied PEM. Tests in `crates/librefang-channels/src/email.rs`: 7 new — default-validating connector, accept-invalid-certs opt-in, missing CA path (error mentions the path), empty CA file (error mentions "no PEM certificates"), garbage non-PEM file (same path), valid PEM CA loaded from a re-encoded native-store cert (skipped with `eprintln!` rather than failing on minimal CI images without `ca-certificates`), and an `EmailAdapter::with_tls_*` builder smoke test. Closes #4877. (@houko)

- **`[proactive_memory] extraction_model` honours provider-qualified ids** (#4871). Before this change, `crates/librefang-kernel/src/kernel/boot.rs:1325-1328` always routed proactive-memory extraction through `kernel.llm.default_driver`, regardless of what `extraction_model` named — the model string was passed through `strip_provider_prefix` for the *default* provider, then sent to the default driver. So on a deployment with `default_model.provider = "ollama"` and `extraction_model = "anthropic/haiku"`, every extraction call dispatched the Anthropic model name through the Ollama driver and 404'd upstream. Operators were forced to route extraction through whatever provider they happened to have as default, even when that was wildly suboptimal for the extraction workload (e.g., expensive default model burned on every turn's extraction while a cheap haiku/openrouter alternative was just one config edit away). The fix: `extraction_model` now accepts three forms — `provider:model` (consistent with `[llm.auxiliary]` chain spec), `provider/model` (consistent with `aliases.toml` and `default_model` shape), or a bare model name (legacy behaviour, routes to `default_driver`). The provider prefix is only honoured when the LHS is a **registered provider** per the live model catalog (`ModelCatalog::get_provider().is_some()`) — this avoids misparsing OpenRouter-style model ids like `google/gemini-2.5-flash` where `google` isn't a separate provider, so the whole string belongs to the configured default provider verbatim. Colon parsing is attempted first; quirky ollama tag suffixes like `qwen3:4b` fall through to bare because `qwen3` isn't a registered provider. Nested slashes (`openrouter/anthropic/claude-3-5-haiku`) split on the FIRST `/` so the model id keeps the inner slash. When the resolved provider differs from the kernel's default, a fresh driver is built via `drivers::create_driver` with API key, base URL, proxy, request timeout, and MCP bridge config all resolved through the same paths the boot path uses for the primary driver — driver-build failure logs a WARN naming the spec + provider + cause and falls back to NO LLM extractor (proactive memory then uses substring extraction; explicit visible degradation beats silently 404'ing the operator's named provider on every turn). Bare model names continue to route through `default_driver` unchanged — fully backward-compatible. Note: this PR also **closes #4870** — the per-agent `[proactive_memory]` override shipped in #4885 but the original PR's body said "Closes #4870" inline rather than in a recognised GitHub keyword line, so the issue stayed open after merge. Tests: 8 in `librefang-kernel::kernel::boot::extraction_model_tests` (bare model, colon-form known provider, slash-form known provider, slash-form unknown LHS → bare, nested slash form, colon-form unknown LHS → bare, empty sides → bare, colon precedence over slash when both present). Closes #4871. Closes #4870. (@houko)

- **Per-agent `[proactive_memory]` override in `agent.toml`** (#4870). `[proactive_memory]` in `config.toml` sets a single, kernel-wide policy that forces the same `auto_memorize` / `auto_retrieve` flags on every spawned agent. On hosts that mix one chatty user-facing agent with cron-driven sub-agents (data collectors, ETL, brief composers), enabling `auto_memorize` globally costs an extraction LLM call per sub-agent turn for content that has no recall value — the reporter's `lifeos-daily-brief` deployment was burning ~200 extraction calls/day on agents whose tool-output extraction was pure noise. New `librefang_types::memory::ProactiveMemoryOverrides` struct with three optional fields (`enabled`, `auto_memorize`, `auto_retrieve`); each `Option<bool>` either inherits the global config (`None`) or supersedes it for this agent (`Some(b)`). Wired into `AgentManifest.proactive_memory` (`#[serde(default)]`, all-`None` default → inherit). The runtime gates at the call site: new `gated_proactive_memory_for_retrieve` / `gated_proactive_memory_for_memorize` helpers in `agent_loop.rs` consult `manifest.proactive_memory.resolve_*` against `store.config()` and pass `None` into `RecallSetupContext` / `FinalizeEndTurnContext` when the per-agent override disables the side. Boot caveat documented on the struct: the global `proactive_memory.enabled = false` short-circuits store construction, so per-agent `enabled = Some(true)` cannot resurrect a non-existent store — the supported shape is **per-agent opt-out** when the global is on (which matches the issue's actual use case). Resolution precedence: `enabled = Some(false)` wins over both per-field overrides; otherwise per-field `Some(b)` wins; otherwise fall back to the global config. Tests: 5 in `librefang-types::memory::tests` (default-inherits, per-field disable, master switch, global-off inheritance, serde round-trip preserving `skip_serializing_if = "Option::is_none"`). (@houko)

- **Per-channel proxy override on Telegram / Discord / Slack / Mattermost adapters** (#4795). Each adapter's `[[channels.<name>]]` block now accepts an optional `proxy = "http://…"` (or `https://`, `socks5://`, `socks5h://`, with optional `user:pass@`) that routes the adapter's `reqwest::Client` through the named proxy, overriding the process-level `HTTP_PROXY` / `HTTPS_PROXY` / `ALL_PROXY` env vars. Unset = today's behaviour (env-var fallback still applies). Centralized in a new `librefang_channels::http_client::new_proxied_client` helper plus a `with_proxy(Option<&str>)` builder method on each affected adapter; invalid URLs are rejected at adapter init with a `ChannelProxyError::InvalidUrl` carrying the offending value and reqwest's reason, and the channel-bridge logs the redacted URL and skips that one adapter rather than booting with the wrong proxy. Required enabling reqwest's `socks` feature in the workspace `Cargo.toml`. Scope: REST client only — gateway / Socket Mode / Mattermost WebSocket connections still use the platform's default network path. Auth in the proxy URL works automatically via reqwest. Tests: 8 in `librefang_channels::http_client::tests` (None / http / https / socks5 / socks5h / userinfo / garbage URL / scheme-list display), 4 each in `telegram::tests`, 2 each in `discord::tests` / `slack::tests` / `mattermost::tests` (adapter-init smoke), plus a serde round-trip across all four configs in `librefang_types::config::tests`. (@houko)

- **Skill workshop — passive after-turn capture of teaching signals** (#3328) (default-OFF; opt in per agent via `[skill_workshop] enabled = true` in `agent.toml`, matching the original #3328 acceptance criteria). New `librefang-kernel::skill_workshop` subsystem. Once enabled, an `AgentLoopEnd` hook (registered alongside `auto_dream` in `set_self_handle`) runs three regex scanners against the latest user message + recent tool history after every non-fork turn and produces draft `CandidateSkill` TOML files under `~/.librefang/skills/pending/<agent_uuid>/<candidate_uuid>.toml`. Three signals: `ExplicitInstruction` ("from now on always …", "every time …"; conversational subjects "I" / "we" / "you" filtered, trigger must sit at sentence-start), `UserCorrection` ("no, do it like …", "actually …"), `RepeatedToolPattern` (same tool sequence ≥ 3 turns, length-1 patterns require ≥ 4). Per-agent config in `agent.toml` `[skill_workshop]`: `enabled` (default false), `auto_capture` (default true), `approval_policy` ("pending" default / "auto"), `review_mode` ("heuristic" default / "threshold_llm" / "none"; `"both"` is a serde alias for `threshold_llm`), `max_pending` (default 20). Once enabled, the conservative knob set is heuristic-only review and pending policy — microseconds of regex per turn plus a few KB written when a candidate lands, no LLM call, no auto-promote. Operators that want LLM refinement opt in via `review_mode = "threshold_llm"`, which routes through a dedicated `AuxTask::SkillWorkshopReview` slot and the cheap-tier provider chain (haiku → gpt-4o-mini → openrouter-haiku); when no cheap-tier credentials are configured the workshop returns `Indeterminate` rather than billing the operator's primary provider, blocking a financial-DoS regression. Approval routes through `evolution::create_skill` (same path as marketplace skills) so the `SkillVerifier::scan_prompt_content` security gate runs at both `save_candidate` and `approve_candidate` — `prompt_context`, `description`, and both 800-char provenance excerpts are scanned at save; Critical hits abort with `SecurityBlocked` before any temp file is written. UUID validation guards every public storage entry point that addresses files by id, so a non-UUID id never reaches `Path::join`. CLI: `librefang skill pending list / show / approve / reject`; HTTP: `GET/POST /api/skills/pending[…]` (auth-gated, `WorkshopError::InvalidId` → 400, not-found → 404, security/conflict → 409); dashboard: `PendingSkillsSection` on the Skills page with Approve / Reject buttons (the section renders nothing while the queue is empty so it doesn't waste page space). Architecture doc at `docs/architecture/skill-workshop.md`. Tests: 56 in `librefang-kernel::skill_workshop` (heuristic / llm_review / storage / candidate / mod) + 13 integration in `librefang-api::skill_workshop_pending_routes_test` (status + side-effect read-back per the project's mandatory pattern, including UUID-validation 400 cases). (@houko)

### Fixed

- **macOS vault no longer locks at launchd startup; `ioreg` / `reg.exe` are resolved by absolute path instead of bare command name** (#5025). `collect_os_machine_id_material` at `crates/librefang-extensions/src/vault.rs` shelled out with `Command::new("ioreg")` on macOS and `Command::new("reg")` on Windows. The installer-generated launchd plist gives the daemon a minimal `PATH = /usr/local/bin:/usr/bin:/bin` that excludes `/usr/sbin` — where `ioreg` lives — so the spawn ENOENT'd, the `if let Ok(...)` branch silently skipped, and the v3 keyring derivation produced a different wrap key than the one that wrote the keyring. Vault decrypted from an interactive shell (`/etc/zprofile` runs `path_helper` which adds `/usr/sbin`) but appeared locked under launchd; all vault-only secrets then dropped from the daemon's env by `librefang-dotenv`, breaking LLM providers and `env = ["VAULT_KEY"]` MCP wiring. Same anti-pattern applied to Windows `reg.exe` under a service-account context with a stripped PATH. The fix: new `resolve_command(candidates: &[&str])` helper picks the first absolute path that exists on disk, falling through to bare names only as a last resort; macOS now tries `/usr/sbin/ioreg`, `/sbin/ioreg`, `ioreg` in that order, Windows tries `C:\Windows\System32\reg.exe` then `reg`. Both call sites switched from `if let Ok(output) = ...` to an explicit `match` that emits a structured `warn!` log on `Err` (binary path + spawn error) AND on `Ok(_)` that yielded no UUID — eliminating the silent-failure mode that took multiple hours of `env -i` bisecting to diagnose. The warn log explicitly names #5025 so future operators searching daemon logs find the original issue immediately. Tests: 5 unit tests on `resolve_command` (`resolve_command_picks_first_existing_absolute` against `/bin/sh`, `_skips_missing_absolutes`, `_returns_last_when_all_absolutes_missing_and_no_bare`, `_skips_missing_windows_absolutes` against `C:\Windows\System32\cmd.exe`, `_accepts_bare_name_without_filesystem_check`). Closes #5025. (@houko)

- **Approval notifications reach chats bound to the requesting agent even when the adapter has `default_agent = None`** (#5002). Follow-up to #4985 / #4994. The post-#4994 listener gates delivery on `router.channel_default(<channel_key>)`; for adapters configured with `default_agent = None` (operator routes purely via `AgentBinding`, e.g. one Telegram bot serving multiple agents with different per-chat ownership), that lookup returns `None` and the listener silently dropped every approval — the original #4985 silent-drop branch listed this as an operator-visible narrowing tracked for a follow-up. The fix adds `AgentRouter::bound_recipients_for_agent(agent_id, channel_str, account_id)` which walks the binding list and returns every `peer_id` whose `AgentBinding.agent` resolves (via `agent_name_cache`) to the requesting agent on this adapter's `(channel_type, account_id)`. The listener now falls back to that binding-derived recipient set when `channel_default` does not cover the requesting agent. Fan-out semantics: all bindings that match — picking one arbitrarily would be wrong (one operator-configured chat would lose the prompt) and re-narrowing to "primary" requires a config concept that doesn't exist; multi-chat fan-out is the cheapest correct default and only sends to chats the operator deliberately bound, so it does NOT re-open the cross-agent broadcast leak #4985 closed. Bindings without a `peer_id` (channel-only catch-all rules) are skipped — they name no chat to deliver to. Bindings whose `match_rule.account_id` is set must match the adapter's `account_id()` exactly; bindings whose `account_id` is unset apply to every adapter on that channel type (consistent with the inbound resolver's semantics). When `channel_default` returns `None` AND no binding covers the requesting agent on the adapter, the listener now logs a structured `warn!` (`adapter`, `account_id`, `channel`, `request_id`, `requesting_agent`) so operators see "approval dropped: no recipients for agent X on channel Y" instead of the previous silent drop — that visibility is the user-facing half of the regression and the warn-log promotion is intentional (operators previously had no way to tell whether an approval was misrouted or just slow). Trait-extension question (raised in the issue body) resolved in the negative: the binding store lives on `AgentRouter`, which `BridgeManager` already holds, so adding a `bound_recipients_for_agent` method on `ChannelAdapter` would have been redundant plumbing — querying the router directly keeps every adapter implementation untouched. Tests in `crates/librefang-channels/tests/bridge_integration_test.rs`: `test_approval_listener_falls_back_to_agent_binding_when_default_unset` (direct #5002 repro — `default_agent = None`, one `AgentBinding` chat-z → agent X, approval for X lands in chat-z), `test_approval_listener_binding_fallback_does_not_leak_cross_agent` (same setup, approval for a DIFFERENT agent Y must NOT be delivered — pins that the fallback path does not re-introduce the #4985 leak shape), `test_approval_listener_fans_out_to_all_bound_chats` (agent X bound to chat-z1 AND chat-z2 → both receive the notification, asserts exactly-2 with a 100ms over-slack to catch double-send regressions), `test_approval_listener_skips_binding_with_no_peer_id` (channel-only catch-all binding with no `peer_id` is correctly NOT a delivery target — pre-fix this would have crashed `adapter.send()` with an empty `platform_id`), `test_approval_listener_binding_respects_account_id_scope` (binding scoped to `account_id = "bot-a"` fires on bot-a but not on bot-b — mirrors the #4985 multi-bot leak shape at the binding layer). Closes #5002. (@houko)

- **Prompt-cache hits no longer trip the per-minute burst limit; Anthropic driver normalizes `TokenUsage` to the workspace OpenAI-shape convention** (#4943, #4958). Two coupled issues. (1) `AgentScheduler`'s sliding-window burst guard at `crates/librefang-kernel/src/scheduler.rs:284-290` summed `usage.total()` (= `input_tokens + output_tokens`) into the per-minute window, so an agent with a large stable prompt — e.g. ~50k tokens of MCP tool definitions hitting the prompt cache every turn — would trip `Token burst limit would be exceeded: 411909 + 32128 reserved in last minute (max 240000/min)` despite the model doing almost no new work (provider charges ~0.1x of input rate for cache reads). New `TokenUsage::burst_tokens()` on `librefang-types::message::TokenUsage` returns `input_tokens.saturating_sub(cache_read_input_tokens) + output_tokens`. `record_usage` and `settle_reservation` switch the sliding-window push to `burst_tokens()`; the hourly absolute quota (`tracker.total_tokens` against `max_llm_tokens_per_hour`) continues to use `usage.total()` because that knob is operator-facing "raw tokens per hour" by historical contract — the asymmetry is deliberate (hourly = budget, burst = rate control). (2) For that formula to work on every provider, the workspace needs a single convention for what `input_tokens` represents. `librefang-kernel-metering::estimate_cost_from_rates` had been built on the OpenAI shape (`input_tokens` is the TOTAL prompt count, `cache_read` is a subset), and the cost tests confirm this in the comments (`test_estimate_cost_cache_read_discount`: "1M total input tokens, 500k are cache-read"). But the Anthropic driver was passing through raw API values, which use the opposite convention (`input_tokens` excludes cache, cache buckets reported separately — https://docs.anthropic.com/en/api/messages#response-usage). The result was silent: for every cache-using Anthropic turn `estimate_cost_from_rates`'s `saturating_sub` collapsed `regular_input` to 0, under-billing by the genuine new-input portion — and `budget` gates / dashboard rollups under-counted in the same direction. Anthropic driver now normalizes at the boundary (`convert_response` and the streaming `message_start` handler) by adding `cache_read_input_tokens + cache_creation_input_tokens` into `TokenUsage.input_tokens` so the wire shape downstream consumers see is identical across providers. The pre-existing `estimate_cost_from_rates` is now correct on Anthropic without further change; the new `burst_tokens` is correct on both. `TokenUsage` struct docstring documents the single convention. Tests: 4 in `scheduler::tests` — `test_burst_limit_excludes_cache_read_tokens` (cache-read-heavy turn passes the burst check that previously failed, plus a follow-up cache-creation-heavy turn that still hits the cap because cache writes are inside `input_tokens` and do go through the model), `test_burst_tokens_pure_cache_hit_is_zero_new_work`, `test_burst_tokens_saturates_when_cache_read_exceeds_input` (no panic / no wrap on malformed payloads), `test_burst_tokens_counts_cache_creation` (OpenAI and Anthropic-post-normalization shapes both produce the same `1100` / `1150` expected burst). Closes #4943. Closes #4958. (@houko)

- **Agent detail page sorts skills alphabetically** (#4940 — partial). `renderSkillsTab`, the row-level skills preview (first-3 chips on agent cards), and the detail sidebar `Skills` section in `crates/librefang-api/dashboard/src/pages/AgentsPage.tsx` previously rendered skills in the backend's allowlist order — meaningless to humans scanning a long list. All three sites now `.slice().sort()` before rendering so the same agent's skills appear in the same order in every view. Plain codepoint sort (not `localeCompare`) because skill names are slug-shape ASCII IDs and `localeCompare` would flip ordering under non-en locales (tr-TR dotless-i, etc). The issue's MCP and Channels claims don't apply — those tabs don't exist on the agent detail page (Conversation / Memory / Skills / Schedule / Logs only); commented on the issue to clarify. (@houko)

- **Approval requests now reach channel adapters instead of being kernel-only** (#4875). `BridgeManager::start_approval_listener` (`crates/librefang-channels/src/bridge.rs`) was defined as `pub async fn`, documented as the path that subscribes to `EventPayload::ApprovalRequested` and forwards each request to running channel adapters, but no code in the workspace ever called it — a repo-wide search for `start_approval_listener` returned exactly one hit (the definition). The lone bridge-startup path `start_channel_bridge_with_config` (`crates/librefang-api/src/channel_bridge.rs`) registered adapters and spawned their inbound tasks but never invoked the listener, so an agent attached to a Telegram channel that triggered a tool in `approval.require_approval` produced a pending request visible via `GET /api/approvals?status=pending` and in the dashboard, but **nothing** arrived in the originating chat — no text prompt, no `/approve <id>` hint, no daemon-log entry for delivery. The pre-existing listener body was also a stub: it formatted the notification text but then only called `info!(...)` per adapter (with a `let _ = &msg;` to silence the unused-variable warning) — even if it had been wired up, no `ChannelAdapter::send()` would have fired. Three changes land the actual delivery path: (1) `start_channel_bridge_with_config` now calls `manager.start_approval_listener().await` after the adapter-registration loop, before returning the manager — only when at least one adapter started successfully, so an all-failed bridge does not leak a listener task; lifetime is tied to `BridgeManager::shutdown_tx`, so hot-reload cancels the listener together with the rest of the bridge. (2) The listener body now actually delivers: for each running adapter, it iterates `adapter.notification_recipients()` and calls `adapter.send(user, ChannelContent::Text(msg))`; the `adapters: Vec<Arc<dyn ChannelAdapter>>` parameter is dropped from the signature in favour of `self.adapters.clone()` since the manager already owns the adapter list (the old parameter only existed because the function was never called and the type system had nothing to enforce). Delivery is best-effort per-recipient — `send()` errors log a warning with `adapter`, `request_id`, `recipient` fields and continue to the next user, so a transient failure on one platform does not block the broadcast to the rest. (3) A new `ChannelAdapter::notification_recipients() -> Vec<ChannelUser>` trait method (default empty Vec) exposes each adapter's operator inbox. `TelegramAdapter` overrides it to project `allowed_users` into `ChannelUser`s, filtering out bare `@username` entries because Telegram `sendMessage` requires a numeric `chat_id` and there is no API call that resolves `@username → chat_id` without a prior message from that user (the bot has no way to DM a stranger by handle). Other adapters keep the default empty-Vec impl, which means they silently skip the broadcast rather than `panic!`-ing or fanning out to wrong recipients — group-only adapters that have no concept of an operator DM (Mastodon, Reddit) are correctly handled by the default; configuring per-adapter recipients on Discord / Slack / Signal / WhatsApp / WeChat is a follow-up override that the new trait method already supports without further plumbing. Inline-keyboard delivery for adapters that support it (Telegram inline keyboards, Slack Block Kit, Feishu cards) is also a follow-up — the current payload is plain text with the truncated 8-char approval ID and `/approve <id>` / `/reject <id>` instructions, which is enough to unblock the user-visible "nothing arrives in the chat" symptom. Test coverage in `crates/librefang-channels/tests/bridge_integration_test.rs`: `test_approval_listener_delivers_to_configured_recipients` builds a `BridgeManager` with a mock adapter that overrides `notification_recipients()`, wires a real `tokio::broadcast` event bus through a new `EventBusHandle`, emits an `ApprovalRequested` event, and asserts the adapter received exactly one `send()` carrying the approval id prefix, tool name, and `/approve` / `/reject` hints to the correct recipient; `test_approval_listener_skips_adapter_without_recipients` pins the default-empty-Vec contract so future adapters that forget to override stay silent instead of crashing the listener task. Closes #4875. (@houko)

- **Approval notifications no longer leak across agents and unrelated chats** (#4985). Privacy regression introduced by #4881 in `v2026.5.12-beta.11`: the new approval listener in `crates/librefang-channels/src/bridge.rs` (around the `for adapter in &adapters` loop) iterated **every** running channel adapter and **every** recipient declared by `ChannelAdapter::notification_recipients()`, with no reference to `approval.agent_id`. In multi-bot Telegram setups this meant an approval triggered by `agent-A` (its own dedicated 1:1 bot) was also delivered to the bot bound to unrelated `agent-B` and to every group chat that bot was a member of — exposing tool names, agent IDs, and the human-readable action description to chats that had nothing to do with the requesting agent. The fix scopes delivery via the `AgentRouter` already owned by `BridgeManager`: for each adapter the listener now builds the same channel key the bridge boot stores in `channel_defaults` (`<channel_type>` for single-bot adapters, `<channel_type>:<account_id>` for multi-bot adapters, account-qualified key tried first to match `router::resolve_with_context`'s precedence), looks up `router.channel_default(&key)`, and only calls `adapter.send()` when the bound agent equals the requesting agent's parsed UUID. Adapters with no router binding (`channel_default` returns `None`) are suppressed rather than fanned out — pre-fix code would have broadcast to them, post-fix the listener treats "no bound agent" as "cannot scope safely, drop". A malformed `approval.agent_id` (not a valid UUID) is also dropped with a WARN log rather than reverting to the pre-fix broadcast. Two trait-level additions support the scoping: new `ChannelAdapter::account_id() -> Option<&str>` (default `None`) exposes the multi-bot account identifier so the listener can build the same `telegram:<account_id>` key the router uses; `TelegramAdapter` overrides to return its configured `account_id`. Other adapters keep the default `None`, which means single-bot configurations fall through to the bare `<channel_type>` key as before — fully backward-compatible for the common case. Tests in `crates/librefang-channels/tests/bridge_integration_test.rs`: `test_approval_listener_scopes_delivery_to_requesting_agent_adapter` (two adapters bound to different agents via `telegram:bot-a` / `telegram:bot-b` keys, approval for agent A reaches only adapter A's recipient, adapter B's recipient receives nothing — the direct #4985 regression guard); `test_approval_listener_skips_unbound_adapter` (adapter with no `channel_default` entry is silently skipped instead of leaked to); `test_approval_listener_drops_malformed_agent_id` (non-UUID `agent_id` does not revert to broadcast). The two existing tests from #4881 were updated to register a router binding so the scoping check allows their happy path. `/approve` and `/reject` command dispatch is **not** chat-scoped in this PR — that's a separate concern noted in the issue body alongside #4905; tracked as out-of-scope follow-up. Two operator-visible narrowings land with this fix and are tracked as follow-ups: (a) adapters configured purely for `AgentBinding`-based routing with no `default_agent` (and so no `channel_defaults` entry) no longer receive approval notifications — the listener now treats "no bound agent on the channel key" as "cannot scope safely, drop" rather than fanning out. Binding-aware lookup is the proper fix and lands separately as a follow-up issue against this PR. (b) only `TelegramAdapter` currently overrides `account_id()`, so multi-bot deployments on Slack / Discord / Matrix / Mattermost / WeChat / Signal still resolve under the bare `<channel_type>` key and continue to share a single channel-default binding across bots; the per-adapter `account_id()` override is a small follow-up per adapter and is tracked separately. PR #4994 follow-up tightens two regressions found in review: (i) the qualified-key lookup no longer falls back to the bare key for adapters with `account_id().is_some()` — in mixed configs (one single-bot adapter + one multi-bot adapter on the same channel type) the bare-key fallback would have leaked an approval into the multi-bot adapter when its requesting agent matched the single-bot adapter's default; (ii) a malformed `approval.agent_id` (non-UUID) is now logged at ERROR rather than WARN so a misconfigured `require_approval` caller silently swallowing every approval surfaces in operator logs. No migration needed; the fix is purely defensive narrowing. Closes #4985. (@houko)

- **`save_session_summary` now produces real summaries via the auxiliary LLM** (#4869). On `reset_session` / `/new`, the kernel writes a summary of the about-to-be-deleted session to `kv_store`. Three independent defects compounded into a near-useless on-reset write: (1) the implementation looked at only the **last 10 messages**, so any non-trivial session was summarised from its closing pleasantries ("thanks", "sure", "you too"); (2) the filter accepted only `MessageContent::Text` user messages, so a session that ended on a tool-result turn produced **no summary at all** — the function early-returned on `topics.is_empty()` before writing anything (this is the silent vector for "some `/new`-deleted sessions on a heavy agent have no `session_*` kv_store entry"); (3) the storage key was `session_{date}_{slug}` where the slug came from the first user message's first 6 words, so two sessions on the same day whose first user turn slugged identically — easy with one-word openings like "Thanks", "Sure", "OK", "Yes" — silently overwrote each other. The reporter's 186-message vault-operator meal-planning session ("96 user + 90 assistant turns doing 6+ hours of work") got summarised as `session_2026-05-10_thanks → Key exchanges: 1. Thanks / 2. Sure / 3. You too`. New `AuxTask::SessionSummary` variant routes through `[llm.auxiliary]` like the workshop reviewer (default chain: openrouter-haiku → anthropic-haiku → openai gpt-4o-mini, same shape as `Compression` / `Fold` / `SkillWorkshopReview`). `save_session_summary` now clones the session messages, spawns a `tokio` task, calls `AuxClient::resolve(SessionSummary)`, and feeds the **entire** rendered transcript (text + `tool_use` + `tool_result` + `thinking` blocks, capped at 48k chars with the head dropped first so recent context survives) to the cheap-tier driver with a "5–10 bullets covering goal / work / files / decisions / final state" instruction. The 30s wall-clock timeout means a slow path doesn't keep the spawned task alive; failures log WARN and fall back to a trivial digest (turn counts, tool activity, first/last user goal) instead of producing nothing. **Same WARN-and-degrade behaviour when no aux chain resolves** — matches the `SkillWorkshopReview` precedent (no billing of the operator's primary provider on a side task); operators see a visible degraded-mode signal instead of silent quality loss. Storage key changed from `session_{date}_{slug}` to `session_{session_id}` — collision-free across same-day sessions because session IDs are unique by construction; also writes `{workspace}/memory/session-{session_id}.md` for human browsing when the workspace exists. The output is capped at 16 KiB (truncated on a UTF-8 boundary) so a misbehaving aux model can't write runaway content to disk or DB. The spawned task pre-clones everything it needs (messages, agent name, workspace path, substrate `Arc`) so it survives the immediate session deletion in `reset_session`. When no tokio runtime is available (non-async caller), the trivial digest is written synchronously — preserving on-reset behaviour without panicking. Tests: 4 in `librefang-kernel::kernel::session_ops::session_summary_tests` covering the previously silent failures (`trivial_summary_survives_tool_result_only_tail` pins the fix for defect 2 — a session ending mid-tool-loop now produces output; `trivial_summary_reports_turn_counts` pins the digest shape; `render_transcript_includes_tool_calls` confirms tool activity reaches the prompt — defect 2's root cause; `render_transcript_truncates_head_preserves_tail` pins the 48k-char cap behaviour). Closes #4869. (@houko)

- **`[budget]` config edits via dashboard now persist and take effect without restart** (#4797 / #4864). Three stacked regressions made `[budget]` look broken from every angle: (1) `GET /api/budget` returns the kernel-side `BudgetStatus` shape (`hourly_limit` / `daily_limit` / `monthly_limit` / `*_spend` / `*_pct`) but `dashboard/src/api.ts::BudgetStatus` and `AnalyticsPage` read `max_hourly_usd` / `max_daily_usd` / `max_monthly_usd` — a typed-shape mismatch that always rendered `-` for the operator's configured caps; the TypeScript interface and the AnalyticsPage cap row now match the wire shape, and dashboard reads include the `*_spend` / `*_pct` rollups computed against the live `usage_events` table. (2) `PUT /api/budget` previously called `kernel.update_budget_config` which only flipped the in-memory `BudgetConfig` ArcSwap (the route comment explicitly said "not persisted to config.toml") — a daemon restart silently dropped the operator's edit. The handler now merges body fields onto the live snapshot, validates each cap at the boundary (NaN / infinity / negative values / non-numeric types → 400 with the offending field named; `null` is treated as "no change"; canonical name wins over alias when both appear), rewrites the `[budget]` table in `config.toml` via `toml_edit` (preserving comments and unrelated sections like `[default_model]` / `[mcp_servers]` / `[[users]]`), backs up `config.toml.prev`, atomic-writes the new content, and calls `reload_config()` so the new caps reach the metering subsystem on the next LLM call. Failed persists are also audited with the attempted diff so forensics can see what an operator tried to set even when the kernel rejected it. (3) Editing `[budget]` directly in `config.toml` only took effect on restart because `MeteringSubsystem.budget_config` is initialised once at boot from `KernelConfig.budget` and the reload-plan diff never emitted a matching `HotAction` — `POST /api/config/reload` updated `self.config` but left the metering ArcSwap pointed at the stale snapshot. New `HotAction::UpdateBudget` variant + `apply_hot_actions_inner` arm that RCUs the new budget into `MeteringSubsystem`. The `Token quota exceeded` follow-on symptom in the bug report is the budget gate firing correctly once a non-zero cap is in effect; with the persistence + reload paths fixed, operators can now actually see and adjust the limits that drive that gate. Test coverage: 11 new integration tests in `budget_routes_test.rs` (`budget_put_persists_to_config_toml` reads the file directly to prove disk write happened, `budget_put_accepts_get_shape_aliases` pins read-modify-write contract, `budget_put_canonical_name_wins_over_alias` pins precedence, `budget_put_rejection_does_not_persist` pins byte-identical config.toml after 400, `budget_put_treats_null_as_no_change` pins null semantics, plus six rejection cases for negative/NaN/non-numeric/alias/token/threshold inputs) + `test_budget_hot_reload_emits_update_action` regression in `config_reload::tests` pinning `[budget]` diff → `HotAction::UpdateBudget`. Closes #4797. (@houko)
- **`provider = "ollama"` speaks the native Ollama protocol instead of the OpenAI-compat shim** (#4810). The registry shipped `ollama` with `api_format = OpenAI` and `base_url = "http://127.0.0.1:11434/v1"`, so every turn POSTed to `/v1/chat/completions`. Real Ollama supports that endpoint, but Ollama-protocol-only servers (AMD's Lemonade, certain llama.cpp wrappers, gpt4all variants) implement only `/api/chat` and 404'd on every request — see the original report's `lemond[2918]: 2026-05-08 23:29:51.528 [Error] (Server) Error 404: POST /v1/chat/completions`. Adds a new native `ApiFormat::Ollama` variant + `drivers::ollama::OllamaDriver` that POSTs to `{base_url}/api/chat` with the native body shape (`messages`, `tools`, `format`, `options.{temperature,num_predict}`, first-class `think: bool`) and parses NDJSON streaming with incremental `content` / `thinking` deltas plus a final `done: true` envelope carrying `prompt_eval_count` / `eval_count`. Tool calls land via `message.tool_calls[].function.{name, arguments(object)}` with synthesised `ollama-call-<uuid>` IDs since the native protocol doesn't return one; the round-trip on tool results uses `role: "tool"` + `tool_name` (Ollama's correlation key) rather than the OpenAI `tool_call_id`. Multi-modal images attach via `message.images: ["<base64>"]` instead of OpenAI's `image_url` envelope. The `think` field is **always sent** (driven by `request.thinking.is_some()`), preserving the legacy OpenAI-shim contract exactly — reasoning models like qwen3 / deepseek-r1 / gpt-oss default `think: true` upstream, so omitting the field would have silently flipped chain-of-thought on for users who never enabled the dashboard's deep-thinking toggle. `OllamaDriver::sanitize_base_url` silently strips a trailing `/v1` from existing user configs (with an INFO log) so the upgrade is non-breaking — pre-#4810 setups pinning `http://host:11434/v1` keep working — but the strip is gated on `/v1` being the *entire* path component, so reverse-proxy mounts like `http://api.corp.com/openai/v1` or `http://api.corp.com/api/v1` are left verbatim (stripping those would either compose a still-wrong `…/openai/api/chat` or mask a misconfiguration the user needs to see). Switching to the native protocol also lets us delete the `OpenAIDriver::is_ollama_like` substring detector and the `extra_body.think` injection hack — `think` is now a first-class request field on the only driver that consumes it. The `[provider_urls]` `set_provider_url` route and the `/api/providers/.../test` connectivity probe both branch on `ApiFormat::Ollama` so paste flows like `http://192.168.1.10:11434` no longer get a spurious `/v1` appended and the probe hits `/api/tags` instead of `/v1/models`. Streaming `tool_calls` chunks that fail to deserialise (protocol drift, malformed local-model output) emit a debug log and keep the prior snapshot rather than silently clearing it; truncated NDJSON (no final `done: true`) returns the partial response with zero token usage so callers can detect "incomplete" without a hard error. New integration suite `tests/ollama_driver.rs` covers request shape, native think, tool-call parsing & ID synthesis, NDJSON streaming aggregation, first-class thinking deltas, streaming tool-call event pairs, error mapping (404→ModelNotFound, 401→AuthenticationFailed, 502→Api), multi-modal image serialisation, `role:"tool"` round-trip, the legacy `/v1` migration, reverse-proxy `/v1` paths preserved verbatim, stringified tool-call argument coercion, malformed `tool_calls` chunks keeping prior snapshot, and truncated streams returning partial output with zero usage. Closes #4810. (@houko)
- **MCP OAuth: accept `token_endpoint` on the same registrable domain as the authorization server** (#4665). The strict #3713 host pin refused legitimate cross-domain OAuth-proxy patterns where a vendor's MCP service delegates token exchange to its main OAuth domain — Slack's `mcp.slack.com` advertises a `token_endpoint` on `slack.com/api/oauth.v2.user.access`, and the pin left users with no workaround. `token_endpoint_host_matches` in `routes/mcp_auth.rs` now accepts either an exact host match (preserves the #3713 pin) or hosts on the same eTLD+1 computed via the Public Suffix List (`psl` crate, compile-time-baked data). Multi-label public suffixes (`*.co.uk`, …) and PSL private domains (`*.github.io`, `*.s3.amazonaws.com`, …) are handled correctly so cross-tenant lookalikes don't false-match. IP literals (v4 + v6, including bracketed IPv6) only ever pass via exact match — `psl::domain_str("10.0.0.1")` returns `Some("0.1")` under the unknown-TLD default rule, so an explicit `is_ip_literal` short-circuit guards the eTLD+1 path. Threat trade-off documented inline on the helper: loosening admits an attacker who controls *any* sibling subdomain on the issuer's registrable domain to redirect the token exchange there *if they also* tamper with HTTPS-validated discovery metadata; accepted because the strict pin left no escape hatch and sibling-subdomain takeover within an org's own domain implies the org itself is compromised. Eight unit tests pin every acceptance and rejection path (cross-org, sibling subdomain accept, multi-label PSL, IPv4/IPv6 literals, IPv4 with shared trailing labels, mixed IP-vs-domain, PSL private-domain false-match guard). (@houko)
- Stop reporting fake 99.9% uptime when daemon hasn't been running that long (@leszek3737)
- Preserve `progress` field through goal status change cycles instead of overwriting it (@neo-wanderer)
- Fix `tally` crash when rendering grouped audit breakdown with empty buckets (@leszek3737)
- Enforce base64 image size cap to prevent oversized payloads from stalling the agent loop (@leszek3737)
- Migrate 18 dashboard pages to i18n with proper translation keys and locale formatters (@leszek3737)
- **Dashboard-saved provider keys survive `librefang` daemon restart** (#4701). `POST /api/providers/{name}/key` (`routes/providers.rs::set_provider_key`) writes the key to `<home>/secrets.env` and `set_var`s the live process so the in-memory driver picks it up — the running daemon works, but the next restart booted a process that had never seen the key. Reason: the user-mode systemd unit produced by `librefang service install` (`librefang-cli/src/main.rs::service_install_linux`) and the packaged `deploy/librefang.service` both reference `<home>/env` (or `/etc/librefang/env`) for `EnvironmentFile=`, not `secrets.env`, and nothing in the boot path re-read `secrets.env`. Fix is two layers. (1) New `librefang-api::secrets_env` module exposes `load_into_process_blocking(home)` (sync, called from `cmd_start` *before* the tokio runtime / kernel boot — the only window where `std::env::set_var` is sound) and `load_into_process_async(home)` (spawn-blocking-guarded variant for the existing `channel_bridge::reload_channels_from_disk` hot-reload path, which previously inlined the same parser and now delegates here so the two paths cannot drift). The CLI `cmd_start` now calls the sync loader between `daemon_config_context()` and the runtime build, so any restart route — `systemctl restart`, `librefang restart`, plain `librefang start` — picks up the dashboard-saved key. (2) Belt-and-braces: both unit templates now declare `EnvironmentFile=-<home>/secrets.env` alongside the existing `…/env` so a future systemd-only restart path (skipping the in-process loader) still gets the key, and so newly installed users do not need to know the file exists. Existing installs pick up the loader on the first restart after upgrading; the unit edit only matters for fresh installs and for restarts that bypass the LibreFang CLI. Acceptance test in `secrets_env::tests::load_into_process_blocking_populates_std_env` writes a UUID-tagged `secrets.env` into a `tempdir`, calls the loader, and asserts the resulting `std::env` state matches the file. Closes #4701. (@houko)
- **Workflow runs no longer disappear on graceful daemon shutdown** (#3335). `LibreFangKernel::shutdown` now invokes a new `WorkflowEngine::drain_on_shutdown` once, after `supervisor.shutdown()`, which transitions every `Running` / `Pending` run to `Paused` with a fresh `resume_token` and reason `"Interrupted by daemon shutdown"` and then flushes `workflow_runs.json` via the existing tmp+rename writer. Pre-fix, `persist_runs` deliberately skipped Running and Pending (no durable rollforward boundary), so a `librefang stop` with three in-flight runs left only the unrelated Completed row on disk and the dashboard came back up empty. Post-fix the operator can see the in-flight workload after restart and resume it via the existing `resume_run` API; the stale-running-runs sweep at next boot remains the safety net for crash paths where the drain never executed. Crash shutdown (SIGKILL / OOM / power loss) is **not** changed by this PR — that is what `recover_stale_running_runs` already handles. (@houko)
- **CI Test job red on main for ~30 runs: align 5 missed assertions with dual-shape error envelope (#3639 / #4655 follow-up)**. `Test / macOS|Windows|Ubuntu (shard 2/3/4)` had been failing on the same five integration tests since the #3639 envelope migration: `plugins_routes_integration::install_plugin_rejects_missing_source` / `install_plugin_registry_source_requires_name` / `plugin_registry_search_rejects_invalid_registry_param` and `totp_flow_test::confirm_rejects_replayed_code` / `setup_when_already_confirmed_requires_current_code`. The cause is a shape mismatch — `/api/plugins/install` is `Idempotency-Key`-wrapped (#3637) so its inner handler still emits the flat `{error: <string>, code, type}` envelope, `/api/plugins/registry/search` returns the bare `ApiErrorResponse::bad_request()` shape `{error: <string>}`, while sibling routes use the standardized nested `{error: {message, ...}}` shape (#3639). PR #4655's first alignment pass committed the nested-only assertion at these five sites, so they read `body["error"]["message"]` and saw `Value::Null` when production returned flat. Switched to the dual-shape pattern `body["error"].as_str().or_else(|| body["error"]["message"].as_str())` consistent with the rest of `totp_flow_test.rs:340`, so the assertions pass on whichever envelope each route emits and survive future inner refactors. The `OpenAPI Drift` job (also red on `ee8ee554`) self-healed on the next push via the `IS_INTERNAL_PR` auto-commit branch in `.github/workflows/ci.yml` — local `cargo xtask codegen --openapi` + `python3 scripts/codegen-sdks.py` + `cargo xtask schema-check gen` are now no-ops on `origin/main`. (@houko)
- **Daemon log lines now carry `agent.id` / `session.id` for correlation across `run_agent_loop` and supervised background tasks.** Three changes: (1) `run_agent_loop`'s `#[instrument]` span sets `session.id` alongside `agent.id` AND is pinned to `level = "warn"` so the daemon's baseline filter `librefang_runtime=warn` (installed by `init_tracing_stderr` to suppress runtime INFO noise) does not drop the span before it is created — INFO-level spans are filtered at the registry layer regardless of whether downstream events are visible, so every WARN/ERROR event inside the loop was firing in a bare context; the level bump is invisible to operators because `#[instrument]` does not auto-emit on enter/exit; (2) `librefang_kernel::supervised_spawn` now wraps the spawned future with `.instrument(Span::current())` — `tokio::spawn` does NOT propagate the current tracing span, so every supervised background task (channel bridges, cron tickers, inbox pumps, persist loops; ~48 call sites in the kernel) was starting in a bare span context; (3) `run_agent_loop_streaming` picks up the same `level = "warn"` + `session.id` field for parity. Symptom before: `docker logs jarvis | grep "Shell exec"` showed no agent context. After: `WARN run_agent_loop{agent.name=… agent.id="…" session.id="…"}: shell exec full mode …`. New regression tests: `instrument_span_fields::warn_inside_agent_span_includes_agent_and_session_ids`, `info_span_is_dropped_under_warn_target_filter` (proves the original bug), and `warn_span_survives_warn_target_filter_and_carries_fields` (pins the fix) in runtime; `with_trace_id_compact_format_carries_agent_and_session_ids_from_span` in cli; `supervised_spawn::tests::supervised_task_inherits_caller_span` in kernel. (@vigneshjagadeesh)
- **`DELETE /api/agents/{id}` idempotent on nonexistent agent** — #4630 introduced a `?confirm=true` data-loss gate that fired UNCONDITIONALLY before the registry lookup, so even a DELETE for an already-absent agent returned 409 `delete_confirmation_required` instead of the documented idempotent 200. The handler now reorders: registry check first → 200 idempotent on absent → 409 confirm-required only when the agent actually exists → 409 hand-owned guard preserved. Restores the `test_delete_nonexistent_agent_is_idempotent` invariant in `librefang-testing`. Refs #4614 / #4630. (@houko)
- **Review follow-ups for #4640 / #4649 / #4651 / #4655** (batch 1). (1) CI `Test / Unit (lib+bin)` job: added `--no-tests=pass` to the full-run `cargo nextest` invocation so workspace crates with zero lib/bin tests (pure type-def crates) no longer exit 4 and fail the job. (2) `librefang-skills` supply-chain audit: `.pth` extension check is now case-insensitive (`eq_ignore_ascii_case`) catching `evil.PTH` / `evil.Pth` on macOS/Windows; `collect_recursive` switched from `path.is_dir()` (follows symlinks) to `entry.file_type().is_dir()` (does not follow), and any symlink in the bundle now produces a `symlink-escape` `Violation` that blocks the install. (3) `librefang-runtime` artifact store: `spill_threshold_bytes` and `max_artifact_bytes` wired from `ToolResultsConfig` through `ToolExecContext` into `tool_web_fetch_legacy` (previously hardcoded `16_384`); new `max_artifact_bytes` field added to `ToolResultsConfig` (default 64 MiB) and enforced in `artifact_store::write` so oversized payloads fall back to truncation; Windows rename TOCTOU fixed by using unique temp names (`{hash}.{pid}.{nanos}.tmp`) and treating a post-rename `dest.exists()` as an idempotent success. (4) `librefang-api` tests: two `body["error"]["message"].as_str()` flat-only assertions in `memory_routes_integration.rs` (bulk-delete and put-blank-content error paths) converted to the dual-shape pattern consistent with the rest of the file. (@houko)

- **`librefang-cli` TUI compile break + two missed early-exit progress sites** (#4654 / #4647 follow-up). PR #4654 changed `LibreFangKernel::mcp_catalog()` from `RwLock<McpCatalog>` to `ArcSwap<McpCatalog>` but missed `crates/librefang-cli/src/tui/event.rs:2596` which still called `.read().unwrap_or_else(...)` — `librefang-cli` failed to compile in any build that touched the TUI module (the `cargo check --workspace --lib` run in #4654 missed this because the binary entry isn't a `--lib` target). Migrated to the new `mcp_catalog_load()` snapshot. Also picks up the two `cmd_init_upgrade` early-exit paths (`Failed to create backups dir`, `Failed to backup config`) that PR #4647 missed — they were upstream of the four paths fixed there and still dropped the spinner without `finish()`. (@houko)
- **CLI `progress.rs` early-exit hygiene + failure-finish glyph** (#3306 follow-up). Three `cmd_init_upgrade` error paths called `std::process::exit(1)` while the progress bar was still active, leaving the TTY cursor hidden / spinner half-drawn — they now `finish()` first. New `ProgressReporter::finish_with_failure(msg)` method (with default impl delegating to `finish` for back-compat) is wired through `Spinner` / `ProgressBar` / `LogReporter`; `cmd_skill_install`, `cmd_skill_publish`, and `cmd_migrate` Err arms now use it so failure messages render with a distinct glyph instead of looking identical to success. `auto()` selection logic refactored into a pure `pick_reporter_kind(is_stderr_tty, total)` helper with explicit unit-test coverage. (#3306 follow-up) (@houko)
- **Centralised `KernelOpError → ApiErrorResponse` mapping** for the #3541 typed-errors series. The 7 stacked PRs (#4608–#4619) shipped per-route ad-hoc matches that drifted: `approvals.rs` mapped *every* `KernelOpError` to 404 (so an `Unavailable` "approval system disabled" surfaced as 404 instead of 503); `prompts.rs` mapped *every* error to 500 (so `NotFound { kind: "prompt_version" }` collapsed to 500 instead of 404); `task_queue.rs::map_kernel_op_err` mapped `Unavailable` to 500 instead of the documented 503; `workflows.rs::create_cron_job` mapped `Unavailable` to 500 and `Other` to 400. New `impl From<KernelOpError> for ApiErrorResponse` in `librefang-api/src/error.rs` is now the single source of truth (`NotFound→404, Invalid→400, Unavailable→503, Serialize/Other→500`); all four routes delegate to it. Adds machine-readable `code: not_found / invalid_input / service_unavailable / serialize_failed / internal_error`. (#3541 follow-up) (@houko)
- **Replace the `agents.rs` `format!("{e}").contains("Agent not found")` substring grep** with a structural match on the `KernelError::LibreFang(LibreFangError::AgentNotFound | QuotaExceeded)` variants in the `send_message` handler — eliminating the typed-grep that the #3541 series claimed to retire but missed at this hot-path call site. The session-mismatch branch still flows through `LibreFangError::Internal(_)` at the kernel side and remains a substring check scoped to that variant; eliminating that last grep needs a kernel emit-site refactor to a typed `SessionAgentMismatch` variant, tracked as a separate follow-up. (#3541 follow-up) (@houko)
- **`PromptStore` kernel impl: classify input-validation failures as `Invalid { field, reason }`** instead of `Other(format!("Invalid X ID: …"))`. Affects `agent_id`, `experiment_id`, `variant_id`, and `version_id` parse failures across `get_running_experiment`, `record_experiment_request`, `get_prompt_version`, `delete_prompt_version`, `set_active_prompt_version`, `get_experiment`, `update_experiment_status`, and `get_experiment_metrics`. Combined with the new central mapping above, malformed IDs now surface as 400 instead of 500. The `"Prompt store not initialized"` `ok_or` arms migrated to `KernelOpError::unavailable("Prompt store")` (503) so callers can distinguish "feature wired off" from generic 500. Closes the explicit follow-up debt acknowledged in the #3541 6/N CHANGELOG entry. (#3541 follow-up) (@houko)
- **`idempotency_test` filter to exclude the auto-spawned `assistant`** when asserting `agent_registry().list().len()`. The 3 failing tests (`spawn_with_idempotency_key_replays_response`, `spawn_with_reused_key_different_body_is_409`, `spawn_without_idempotency_key_is_unchanged`) were main-red since they merged in #4565 because `LibreFangKernel::boot_with_config` auto-creates a default `assistant` agent on a fresh registry — the assertions assumed an empty registry. New `test_spawned_agents` helper filters the bootstrap agent so each test counts only the agents it explicitly created. Test-only change; no production behaviour shift. (@houko)
- **`session_repair::tests::prop::validate_and_repair_no_orphans_no_dup_results` proptest invariant 3 refined** to match the actual repair-pipeline contract. The original "no duplicate ToolResult tool_use_ids in the output" was structurally inconsistent with the explicit `reorder_preserves_per_turn_synthetic_when_tool_id_collides_across_turns` regression test, which deliberately requires both ToolResults to survive when a ToolUse id is reused across multiple assistant turns (Moonshot/Kimi per-completion-counter pattern, e.g. `memory_store:6` reset per call). The proptest now mirrors `deduplicate_tool_results`'s `collision_ids` logic: ids that appear in >1 assistant turn are positional duplicates by design and skip the uniqueness check; everything else is still required to be unique. Test-only change; no production behaviour shift. Fixes the fourth main-red CI failure that has been blocking the #3541 stack from going green. (@houko)
- Matrix adapter: inbound `m.file` / `m.image` / `m.audio` / `m.video` events were silently surfaced as `ChannelContent::Text(filename)` — agents never saw the attachment bytes. Fixed by branching on `content.msgtype` and resolving `mxc://` to an HTTPS download URL the bridge layer can stage. (@neo-wanderer)

### Added

- **`librefang-memory-wiki` crate — durable markdown knowledge vault with provenance and Obsidian-friendly export** (#3329, v1 isolated mode). Pairs with the existing SQLite/vector substrate: where memory answers "find me the K nearest snippets", the wiki answers "give me a navigable knowledge base I can also open in Obsidian and edit by hand". Every page lives at `<vault>/<topic>.md` with YAML frontmatter (`topic`, `created`, `updated`, `content_sha256`, append-only `provenance: [{agent, session, channel, turn, at}]`); cross-references use `[[topic]]` placeholders that the vault rewrites per its `render_mode` (`native` → `[topic](topic.md)`, `obsidian` → `[[topic]]`). The compiler maintains a deterministic `index.md` plus `_meta/backlinks.json`, and refuses to silently overwrite a page whose on-disk **mtime or sha256** has drifted since the last write — the caller gets `WikiError::HandEditConflict` and must pass `force = true` to merge, in which case the human edit is preserved verbatim and only provenance is appended (the vault never drops a hand-edited line). Three new builtin tools: `wiki_get(topic)`, `wiki_search(query, limit?)`, and `wiki_write(topic, body, force?)` — provenance is constructed kernel-side from the calling agent so the LLM cannot spoof it. **Off by default**: a new `[memory_wiki]` config block (`enabled = false`, `mode = "isolated"|"bridge"|"unsafe_local"`, `vault_path`, `render_mode`, `ingest_filter`) gates construction; with the default config the wiki tools return `KernelOpError::unavailable("wiki")` and zero filesystem state is created. Reserved modes (`bridge` / `unsafe_local`) and the `memory_search corpus = all|kv|wiki` extension surface a typed not-yet-implemented error and are tracked as #3329 follow-ups; v1 ingests via explicit `wiki_write` rather than subscribing to memory events. New `WikiAccess` role trait on `KernelHandle` follows the #3746 split — default impls return `unavailable` so existing kernel stubs and mocks compile unchanged. Integration coverage in `crates/librefang-memory-wiki/tests/wiki_acceptance.rs` mirrors the seven-bullet acceptance list in the issue. Runbook to enable: set `enabled = true`, choose `render_mode`, pick a writable `vault_path`, and call `wiki_write` from any agent — the page lands at `<vault>/<topic>.md` with provenance frontmatter and shows up in `index.md` and `_meta/backlinks.json` on the next write. (@houko)
- **Tool-result context budget — cumulative cap + history fold + artifact GC + primary-fetch spill** (#3347). Closes #3347 by landing the four remaining mechanisms (mechanism 1 artifact spill landed via #4651, but only on the legacy plain-HTTP path). (1) `[runtime.tool_results] max_bytes_per_turn` (default 50 KB) is now active: when a single assistant turn's accumulated tool-result bytes would exceed the cap, the next result escalates to artifact spill (or tail truncation if spill fails); resets between turns. (2) `[runtime.tool_results] history_fold_after_turns` (default 8) is now active: tool-result messages older than N assistant turns have each `ContentBlock::ToolResult.content` rewritten in place to a compact `[history-fold] <summary>` stub (via the aux-LLM channel), preserving `tool_use_id` / `tool_name` / `is_error` / `status` so every assistant `tool_use` block keeps its matching answer — provider APIs (Anthropic Messages, OpenAI Responses, Gemini function_call) reject mismatched ids with `400 invalid_request_error`, so the earlier draft that replaced messages with free-form text would have broken the next provider call. Falls back to `[history-fold] [summarisation unavailable]` (per-block) when no aux-LLM is configured or the call fails, so stale payload is always removed. Pinned messages are never folded. New `[runtime.tool_results] fold_min_batch_size` (default 4) caps aux-LLM cost on long sessions: skips the fold pass until at least N newly-stale messages have accumulated, instead of paying one round-trip per turn just to fold a single just-stale message. (3) `[runtime.tool_results] artifact_max_age_days` (default 30; `0` disables) drives a startup-time artifact-store GC that walks `~/.librefang/data/artifacts/` once per daemon boot and evicts `<hash>.bin` files (and orphan `<hash>.<pid>.<nanos>.tmp` writers) older than the threshold; clock-skew futures are clamped to age zero. (4) Artifact spill now also wraps the primary `WebToolsContext::fetch` path (Tavily / Brave / Jina / SSRF-protected GET) and `web_search` (multi-provider + DDG fallback) — #4651 only wired the legacy plain-HTTP fallback, so large readability-converted payloads on the main paths were still inlined; both paths now share a single `spill_or_passthrough` helper that falls through to the original body on write failure. (5) Layer 2 (per-result) and Layer 3 (per-turn cumulative) spill route through `crate::artifact_store::maybe_spill` — the same content-addressed `~/.librefang/data/artifacts/<sha256>.bin` directory the web tools and `read_artifact` already use. The earlier draft sent Layer 2/3 to a parallel `/tmp/librefang-results/<id>.txt` directory; that path was outside both `read_artifact` (which only accepts `sha256:<hex>` handles) and the artifact-store GC's `.bin`/`.tmp` allowlist, so spilled data was unreachable from the LLM and the temp directory grew unbounded on macOS / Windows. Stub format unified to `[tool_result: <name> | sha256:... | N bytes | preview:]` so the LLM sees one shape regardless of which layer triggered the spill, and can fetch the original via `read_artifact(handle, offset, length)`. Closes #3347. (@houko)
- Incognito chat mode (`incognito: true` on message body / `--incognito` on CLI). Session-message persistence (every `save_session_async` call site in the agent loop) and proactive-memory writes via the `memory_store` tool are dropped silently — the LLM still sees a synthetic `"ok"` tool result so it does not retry. Memory reads remain full-access throughout the turn. Audit-log entries for tool calls are preserved (operator visibility unchanged). Closes #4073. (@houko)
- Wire `progress::auto()` into `cmd_skill_install`, `cmd_skill_publish`, `cmd_migrate`, and `cmd_init_upgrade`; TTY gets animated bar/spinner, non-TTY falls back to plain log lines. (#3306) (@houko)
- Surface caller agent / session / step IDs as `x-librefang-{agent,session,step}-id` headers on outbound OpenAI-compatible requests, so observability sidecars in front of the upstream provider can correlate request log records without parsing the JSON body. New `session_id` and `step_id` fields on `CompletionRequest` (sibling to the existing `agent_id`); both `Option<String>`, omitted from the wire when `None` or empty. Header values are validated via `reqwest::header::HeaderValue::from_str` and silently skipped (with a `warn!`) on parse failure so a malformed trace ID never fails the LLM call. Other drivers (Anthropic, Gemini, Bedrock, Vertex, ChatGPT, Copilot, Claude Code, Codex, Gemini CLI, Qwen Code) accept the new fields but do not yet emit headers; per-driver header emission is a follow-up that will reuse the same opt-out flag. The `x-` prefix is intentionally retained over RFC 6648's "prefer unprefixed" guidance — see `build_custom_header_map` doc-comment for the rationale (industry de-facto convention, internal precedent in `claude_code.rs`'s `X-LibreFang-Agent-Id`, RFC 6648 is non-normative BCP guidance for new protocols). (#4548) (@neo-wanderer)
- **Operator opt-out for trace headers** via new `[telemetry] emit_caller_trace_headers = true` config field (default `true`). Set to `false` in `config.toml` to suppress all three `x-librefang-*` headers wire-side regardless of whether `CompletionRequest`'s caller-id fields are populated. Targets operators with strict zero-egress policies (regulated tenants, EU healthcare, audit-sensitive deployments) who want no LibreFang-internal identifiers — even opaque UUIDs — crossing the upstream-provider boundary. The flag is plumbed through `DriverConfig.emit_caller_trace_headers` to `OpenAIDriver::with_emit_caller_trace_headers(...)` at driver-creation time. (#4548) (@neo-wanderer)
- **Wire-shape change for `extra_headers` on the OpenAI-compatible driver**: the driver now applies `extra_headers` via `RequestBuilder::headers(map)` (replace-on-same-name) instead of a per-entry `req_builder.header(...)` loop (append-on-same-name). Operators relying on the old append-and-keep-both behaviour for a header that ALSO appeared as a default elsewhere on the request builder (e.g. `Authorization`) will see one value on the wire instead of two — almost certainly the more useful behaviour, but worth flagging in release notes. Distinct-name entries are unaffected (still appended, still preserved). (#4548) (@neo-wanderer)
- **`agent_id` / `session_id` structured fields on HTTP access log** — the `request_logging` middleware now reads `AgentIdField` and `SessionIdField` markers from `Response::extensions` after `next.run().await` and emits `agent_id=<uuid>` and `session_id=<uuid>` on every access-log line (all four severity levels). Handlers that already parse these identifiers call the existing `with_agent_id` helper or the new `with_session_id` / composed `with_session_id(sid, with_agent_id(aid, body))` form. Three representative handlers updated as samples: `get_agent_session`, `send_message`, and `attach_session_stream`. Without this, tracing all requests for a specific agent or session required `RUST_LOG=debug` and substring-matching raw URI paths whose `{id}` segments are collapsed by the metrics normaliser. Closes #3511. (@houko)
- **`AppState.bridge_manager` migrated from `tokio::sync::Mutex<Option<BridgeManager>>` to `arc_swap::ArcSwap<Option<BridgeManager>>`** (#3747). Hot-reload reads are now lock-free atomic loads; the stop/swap path uses `ArcSwap::swap` + `Arc::try_unwrap` to obtain owned access for `BridgeManager::stop()`. `arc-swap` is already a workspace dependency (used by `librefang-kernel`); the `librefang-api` and `librefang-testing` crates now declare it explicitly. The `prometheus_handle` field was already absent from `AppState` (parked in a module-level `OnceLock` in `crate::telemetry`); the `peer_registry` field was also already absent (all routes call `state.kernel.peer_registry_ref()` directly). No behaviour change. (@houko)
- `cargo xtask check-changed` — local mirror of the `changes` job in `.github/workflows/ci.yml`. Computes which CI lanes a branch's diff would trigger (rust / docs / ci / install / workspace_cargo / xtask_src), the `full_run` and `full_test` decisions, and the affected crate set (with the schema-mirror rule that pulls `librefang-api` in for any `librefang-types` change). `--json` for tooling, `--run check,clippy,test` actually invokes scoped cargo commands. (#3296) (@houko)
- **Tool-result artifact spill + `read_artifact` tool** (#3347 1/N). When a tool returns a payload larger than `spill_threshold_bytes` (default 16 KB), the runtime writes the raw bytes to `~/.librefang/data/artifacts/<sha256>.bin` atomically and replaces the agent's copy with a compact stub: tool name, handle, total size, and a 1 KB preview. Agents use the new built-in `read_artifact(handle, offset?, length?)` tool to retrieve content in up to 64 KB chunks. The `[tool_results]` config table exposes three knobs: `spill_threshold_bytes` (active), `max_bytes_per_turn` (deferred — cumulative budget, depends on aux-LLM channel #3314, tracked in #3347 2/N), and `history_fold_after_turns` (deferred — history fold, tracked in #3347 3/N). Spill writes are idempotent (same hash → no rewrite) and the fallback to byte-cap truncation is preserved on write failure. (#3347 1/N) (@houko)
- `?offset=&limit=` pagination on `GET /api/peers` and `GET /api/skills`. Both endpoints now accept the canonical `PaginationQuery` and return the existing `PaginatedResponse{items,total,offset,limit}` envelope (skills also keeps the `categories` field). Server caps `limit` at 100; requests asking for more are silently clamped. Backward-compatible — clients that omit both query params still receive the unbounded list (full collection). Reusable `crate::types::PaginationQuery` + `paginate()` helper introduced in `librefang-api/src/types.rs` for future endpoints to adopt. (#3639 1/N) (@houko)
- **Idempotency-Key on three additional state-creating POST endpoints** (#3637 2/N): `POST /api/hands/{hand_id}/activate`, `POST /api/plugins/install`, and `POST /api/webhooks` now honour the opt-in `Idempotency-Key` header using the same substrate introduced in #4565 (`idempotency_keys` SQLite table, migration v34, 24 h TTL). Same key + same body replays the cached 2xx; same key + different body returns 409 Conflict; non-2xx responses are not cached so transient failures remain retriable. The inbound-channel dedup mechanism `(channel, chat, update_id)` requires adding `librefang-memory` as a new dependency of `librefang-channels`, which is an architectural boundary change; that slice is deferred to a follow-up PR rather than half-finished here. (#3637 2/N) (@houko)
- Config-driven session mode for agent triggers (`session_mode = "new" | "persistent"`) — per-agent default with per-trigger override # pragma: no-attribution
- **Real-client-IP resolution for proxied deployments** via two new top-level config fields, `trusted_proxies` and `trust_forwarded_for`. When both are set and the TCP peer matches the allowlist, the GCRA + auth-login rate limiters and the WebSocket per-IP connection cap key on the IP from forwarding headers (`CF-Connecting-IP` → `X-Real-IP` → `Forwarded` (RFC 7239) → rightmost-untrusted hop in `X-Forwarded-For`) instead of the proxy's own address. Without both flags set, behaviour is unchanged: TCP peer only, headers ignored. Forged forwarding headers from peers outside the allowlist are still ignored, so a rotating `X-Forwarded-For` from the open internet can never bypass per-IP limits. Closes the long-standing TODO referenced in `rate_limiter::resolve_client_ip` (now retired). # pragma: no-attribution
- Fan out `x-librefang-{agent,session,step}-id` trace headers to Anthropic, Gemini, and ChatGPT (Responses API) drivers. Logic extracted into a shared `drivers/trace_headers.rs` module; each driver gains `with_emit_caller_trace_headers(bool)` builder (default `true`) wired through `DriverConfig.emit_caller_trace_headers` — same opt-out that shipped with OpenAI in #4548. Bedrock, Vertex, Copilot, and CLI-style drivers are follow-ups. (#4637 1/N) (@houko)
- Trace headers extended to Bedrock / Vertex AI / Copilot drivers (`x-librefang-{agent,session,step}-id`). Bedrock placement verified for SigV4 compatibility: this driver uses Bearer token auth (`AWS_BEARER_TOKEN_BEDROCK`), not SigV4 canonical signing, so trace headers are appended alongside `Authorization: Bearer` with no signing-scope concern. Vertex AI uses the same `build_trace_header_map` helper as Gemini with Google Cloud OAuth2 Bearer auth. Copilot forwards the flag to its inner `OpenAIDriver` via `make_inner_driver`. All three gain `with_emit_caller_trace_headers(bool)` builders wired through `DriverConfig.emit_caller_trace_headers`. CLI-style drivers (Claude Code, Codex, Gemini CLI, Qwen Code) use config-payload identifiers, not wire headers — deferred to a 3/N follow-up. (#4637 2/N) (@houko)
- Trace identifiers extended to CLI-style drivers (Claude Code, Codex, Gemini CLI, Qwen Code) via env vars `LIBREFANG_AGENT_ID`, `LIBREFANG_SESSION_ID`, and `LIBREFANG_STEP_ID` set on the spawned subprocess. These env vars do not reach the upstream provider's wire (the CLI manages its own auth and LLM transport) but let operators correlate OS process-tree entries with LibreFang agent sessions via sidecars or `ps`/`/proc` inspection. Claude Code preserves its existing `X-LibreFang-Agent-Id` header in the `--mcp-config` JSON payload for back-compat; the new env vars are additive. All four drivers gain `with_emit_caller_trace_headers(bool)` builders (default `true`) wired through `DriverConfig.emit_caller_trace_headers`. Closes #4637 (all 10 drivers covered: 1 OpenAI-wire via #4548, 3 HTTP-wire via #4644+#4653, 6 CLI via this PR). (#4637 3/N) (@houko)
- Matrix adapter: lifecycle reactions (`send_reaction`) with redact-on-replace state, thread replies (`send_in_thread` with `m.thread` relation + back-compat `is_falling_back`), streaming output (`send_streaming` with debounced `m.replace` edits, supports_streaming = true), inbound + outbound media (`m.image` / `m.file` / `m.audio` / `m.video` / voice marker), `DeleteMessage` via `m.redact`, `EditInteractive` via `m.replace`, `Location`, `Sticker` (text fallback), `MediaGroup` (sequential events). E2EE rooms emit a one-shot WARN per room. Match on `ChannelContent` is now exhaustive across all 18 variants. (@neo-wanderer)
- Matrix adapter: render `m.text` `body` together with `format: "org.matrix.custom.html"` + `formatted_body` (CommonMark → HTML via `pulldown-cmark`, GFM tables / strikethrough / task-lists enabled). Element / SchildiChat / Cinny now render bold, headings, lists, links, fenced code blocks, and tables that previously appeared as raw markdown. Applied to `api_send_message`, `api_edit_event` (both outer fallback and `m.new_content`), thread replies, and streaming placeholders / overflow tails. Raw `Event::Html` / `Event::InlineHtml` from the input are demoted to text so model-authored output cannot inject `<script>` / `<iframe>` / `<img onerror>` into rooms. (@neo-wanderer)

### Performance

- Swap kernel `model_catalog` from `RwLock<ModelCatalog>` to `ArcSwap<ModelCatalog>` so the hot `send_message_full` path reads the catalog atomically instead of taking 5+ read locks per request. Writers (key validation, provider probes, catalog sync) use the RCU pattern via a new `LibreFangKernel::model_catalog_update(|cat| …)` helper. `ModelCatalog` gains `#[derive(Clone)]` (cheap by ref-count of Vec/HashMap members; only happens on the rare write paths). Removes the lock contention between concurrent agent loops on multi-tenant deployments without changing behaviour. (#3384) (@houko)
- **`mcp_catalog` migrated from `RwLock` to `ArcSwap`** (matching the `model_catalog` pattern from #4599). All five catalog read sites in `routes/skills.rs` (`list_mcp_catalog`, `get_mcp_catalog_entry`, two extensions list/detail handlers, and the install-flow template lookup) switch from `mcp_catalog().read().unwrap_or_else(…)` to a lock-free `mcp_catalog_load()` snapshot load; hot-reload and `POST /api/mcp/reload` writers use `mcp_catalog_reload()` which builds a fresh `McpCatalog` and stores it atomically. `McpCatalog` gains `#[derive(Clone)]` (only `HashMap<String, McpCatalogEntry>` + `PathBuf` — cheap to clone, clone only happens on the infrequent reload path). The existing `budget_config` was already on `ArcSwap` (migrated in a prior PR). Sync `std::fs::read_to_string` inside `reload_agent_from_disk` (a sync fn called from async axum handlers) is now wrapped with `tokio::task::block_in_place` so the tokio worker thread is not parked during I/O. The remaining sync context-md read on the streaming entry path (`send_message_streaming_with_sender_and_opts`) is deferred — the async `load_context_md_async` is already used on all other call sites. Closes #3579. (@houko)

### Changed

- **Drop `LibreFangKernel` inherent forwards in favor of focused `*SubsystemApi` traits** (#3565 follow-up #4766). The 13 focused subsystem traits introduced in #4756 (`AgentSubsystemApi`, `EventSubsystemApi`, `GovernanceSubsystemApi`, `LlmSubsystemApi`, `McpSubsystemApi`, `MediaSubsystemApi`, `MemorySubsystemApi`, `MeshSubsystemApi`, `MeteringSubsystemApi`, `ProcessSubsystemApi`, `SecuritySubsystemApi`, `SkillsSubsystemApi`, `WorkflowSubsystemApi`) are now the canonical surface for subsystem access — re-exported at the crate root so external consumers can `use librefang_kernel::FooSubsystemApi` without reaching into the `kernel::subsystems` module. ~50 thin forwarding methods on `LibreFangKernel` (`audit`, `metering_ref`, `agent_registry`, `agent_identities`, `memory_substrate`, `proactive_memory_store`, `auth_manager`, `pairing_ref`, `approvals`, `hook_registry`, `event_bus_ref`, `injection_senders_ref`, `processes`, `process_registry`, `model_catalog_ref`, `model_catalog_load`, `clear_driver_cache`, `embedding`, `default_model_override_ref`, `mcp_catalog`, `mcp_catalog_load`, `mcp_health`, `mcp_connections_ref`, `mcp_auth_states_ref`, `oauth_provider_ref`, `mcp_tools_ref`, `effective_mcp_servers_ref`, `web_tools`, `browser`, `media`, `tts`, `media_drivers`, `a2a_tasks`, `a2a_agents`, `delivery`, `channel_adapters_ref`, `bindings_ref`, `broadcast_ref`, `peer_registry_ref`, `peer_node_ref`, `cron`, `workflow_engine`, `templates`, `trigger_engine`, `command_queue_ref`, `scheduler_ref`, `supervisor_ref`, `traces`, `skill_registry_ref`, `hands`, `budget_config`) deleted from `accessors.rs`. The `KernelApi` god-trait surface stays byte-identical — its method names route to the focused traits via fully-qualified `<Self as crate::FooSubsystemApi>::method(self)` syntax, so every `Arc<dyn KernelApi>` caller in `librefang-api` is unaffected. `Arc<LibreFangKernel>` / `&LibreFangKernel` callers (kernel internals, CLI TUI, desktop app, integration tests, ACP adapter) gain a one-line `use librefang_kernel::FooSubsystemApi` import per file. Inherents that genuinely cannot move to a trait stay put: `update_budget_config` / `model_catalog_update` (`impl Fn`/`impl FnMut` arguments), `mcp_catalog_reload` / `install_peer_registry_for_test` (direct field writes), `aux_client` (`ArcSwap::load_full` returns an owned `Arc`). Lays the groundwork for #3566 to carve `KernelApi` itself into focused trait objects. (@houko)

- **`librefang-api` narrows the concrete `LibreFangKernel` coupling** (#3744 N/N). Two new role traits added to `librefang-kernel-handle`: `ApiAuth` (5 methods — `auth_api_key`, `dashboard_raw_config`, `auth_home_dir`, `auth_device_api_keys`, `auth_config_users`) and `SessionWriter` (1 method — `inject_attachment_blocks`). Both are implemented on `LibreFangKernel` and included in the `KernelHandle` supertrait. Server-layer auth helpers (`dashboard_session_token`, `valid_api_tokens`, `has_dashboard_credentials`, `configured_user_api_keys`, `paired_device_user_keys`, `any_auth_configured`, `check_bind_auth_safety`) narrowed from `&LibreFangKernel` to `&dyn ApiAuth`. `inject_attachments_into_session` in `routes/agents.rs` narrowed from `&LibreFangKernel` to `&dyn SessionWriter` with the injection logic moved into the kernel impl. All test stubs in `librefang-runtime` (`ApprovalKernel`, `ForceHumanCapturingKernel`, `NamedWsKernel`, `SpawnCheckKernel` in `tool_runner.rs`; `CapturingKernel` in `tool_runner_forwarding.rs`, `tool_runner_agent_event.rs`, `tool_runner_forwarding_task_cron.rs`) implement the new `ApiAuth` and `SessionWriter` traits. The `AppState.kernel` field, `channel_bridge.rs` adapters, `routes/mod.rs`, and `routes/providers.rs` retain `Arc<LibreFangKernel>` — these sites call 100+ kernel-internal methods (config, model catalog, probe results) that cannot be feasibly abstracted without exceeding the 30-method cap; they are covered by the allowlist in `scripts/check-api-kernel-imports.sh`. The comment-strip regex in that script is tightened to catch trailing-comment forms. Closes #3744. (@houko)

- Deprecate flat error fields in favor of nested `error.code|message|request_id`; flat shape kept for one minor. `ApiErrorResponse` now serializes both the new nested `error` envelope and the legacy top-level `code` / `type` / `request_id` fields on every JSON 4xx/5xx response, and the dashboard parser prefers the nested shape with a fallback to the flat one. (#3639 deferred) (@houko)
- **`KernelHandle` role traits gain a typed `KernelOpError`** (#3541 1/N): `EventBus`, `KnowledgeGraph`, and `CronControl` migrated from `Result<_, String>` to `Result<_, librefang_kernel_handle::KernelOpError>`. The new enum has structured variants (`Unavailable { capability }`, `NotFound { kind, id }`, `Invalid { field, reason }`, `Serialize`, `Other(String)`) so callers can match on the cause instead of substring-grepping the formatted message. The catch-all `Other(String)` and `From<String>`/`From<&str>` impls keep the migration window cheap — un-classified kernel sites can opt in incrementally. The api workflow handler now maps `Unavailable` to HTTP 503 directly off the variant. Existing `Arc<dyn KernelHandle>` callers keep compiling unchanged via `KernelHandle`'s blanket impl. (#3541 1/N) (@houko)
- **`TaskQueue` role trait migrated to `KernelOpError`** (#3541 2/N): all 8 methods (`task_post`, `task_claim`, `task_complete`, `task_list`, `task_delete`, `task_retry`, `task_get`, `task_update_status`) now return `Result<_, KernelOpError>`. The kernel impl emits `NotFound { kind: "agent", id }` when the agent UUID-or-name lookup fails — the substring-grep that the historical `String` error required is gone. `crates/librefang-api/src/routes/task_queue.rs` introduces a small `map_kernel_op_err` helper that maps `NotFound`→404, `Invalid`→400, `Unavailable`→503, fallback→500, replacing the unconditional `ApiErrorResponse::internal` at all 8 call sites. (#3541 2/N) (@houko)
- **`MemoryAccess` role trait migrated to `KernelOpError`** (#3541 3/N): `memory_store`, `memory_recall`, `memory_list` flipped from `Result<_, String>` to `Result<_, KernelOpError>`. Test stubs and call sites in `librefang-runtime/src/tool_runner.rs`, `librefang-runtime-wasm/src/host_functions.rs`, and the kernel-handle test fixtures all moved over. Callers in tool_runner / host_functions bridge to their surrounding `Result<_, String>` shells with `.map_err(|e| e.to_string())?` until those wrappers themselves migrate. 3 of 14 role traits typed; `Other(String)` catch-all population keeps shrinking. (#3541 3/N) (@houko)
- **`HandsControl` / `WorkflowRunner` / `GoalControl` migrated to `KernelOpError`** (#3541 4/N): bundles three small role traits — 8 methods total (`hand_list`, `hand_install`, `hand_activate`, `hand_status`, `hand_deactivate`, `run_workflow`, `goal_list_active`, `goal_update`). Default impls now return `KernelOpError::Unavailable { capability: "Hands system" / "Workflow engine" / "Goal system" }` instead of opaque "X not available" strings — callers can branch on the variant directly. `LibreFangKernel::hand_deactivate` emits `Invalid { field: "instance_id" }` for malformed UUIDs; `goal_update` emits `NotFound { kind: "goal", id }` when the goal isn't in the store. 6 of 14 role traits typed. (#3541 4/N) (@houko)
- **`ChannelSender` / `ApprovalGate` / `PromptStore` / `AgentControl` migrated to `KernelOpError`** (#3541 5/N — final): closes the migration. `ChannelSender` (7 methods) covers the channel send + roster surface; `ApprovalGate` (4 methods) covers the approval lifecycle; `PromptStore` (13 methods) covers prompt versions + experiments; `AgentControl` (6 methods — `spawn_agent`, `spawn_agent_checked`, `send_to_agent`, `send_to_agent_as`, `kill_agent`, `run_forked_agent_oneshot`) covers agent lifecycle on the hot path. Default impls return `KernelOpError::Unavailable { capability: "Channel … send" / "Approval system" / "Prompt store" / "run_forked_agent_oneshot" }` so callers branch on the variant. All 12 fallible role traits now use the typed enum (the remaining `A2ARegistry` returns `Option<…>` and `ToolPolicy` has no fallible methods). Substring-grepping at the runtime↔kernel boundary is gone. (#3541 5/N) (@houko)
- `librefang-api` no longer declares `librefang-runtime` as a direct dependency; the API → Kernel → Runtime layering is now compiler-enforced. `cargo check` rejects any new `use librefang_runtime::*` in the api crate. PR 1/N (#4590) seeded the kernel re-exports and migrated 15/34 import sites; this 2/N completes the migration: 16 kernel re-exports total (`a2a`, `agent_loop`, `audit`, `browser`, `catalog_sync`, `channel_registry`, `compactor`, `copilot_oauth`, `drivers`, `http_client`, `kernel_handle`, `llm_driver`, `llm_errors`, `mcp`, `mcp_oauth`, `mcp_server`, `media`, `model_catalog`, `pdf_text`, `plugin_manager`, `plugin_runtime`, `provider_health`, `registry_sync`, `silent_response`, `str_utils`, `tool_runner`); every src + tests file under `crates/librefang-api/` flipped to the `librefang_kernel::*` path; `librefang-runtime` line removed from `crates/librefang-api/Cargo.toml`. `scripts/check-api-runtime-decoupling.sh` flipped from informational to enforcing — fails CI if the dep or any direct import comes back. (#3596 2/N) (@houko)
- **`KernelOpError` is now an alias for `LibreFangError`** (#3541 8/N — final): closes the gap between the kernel-handle trait surface and the workspace's canonical structured-error enum (`librefang_types::error::LibreFangError`). The 5 categorical variants from 1/N–7/N collapse onto richer business variants: `Unavailable { capability }` → new `LibreFangError::Unavailable(String)`; `NotFound { kind: "agent", id }` → `AgentNotFound(_)`; `NotFound { kind: "session", id }` → `SessionNotFound(_)`; `Invalid { field, reason }` → `InvalidInput(_)`; `Serialize(_)` → `Serialization { source, message }`; `Other(_)` → `Internal(_)`. The new `KernelResult<T> = Result<T, KernelOpError>` alias makes new role-trait method signatures self-documenting. `LibreFangError` is `#[non_exhaustive]` (already was), so adding new variants in the future doesn't break callers. The api `map_kernel_op_err` helper in `routes/task_queue.rs` and the `routes/workflows.rs` matcher both pick up extra business categories for free (`AuthDenied`/`CapabilityDenied` → 403, `ManifestParse`/`InvalidState` → 400, `ShuttingDown`/`Unavailable` → 503). #3541 fully closed; the substring-grep anti-pattern at the runtime↔kernel seam is gone. (#3541 8/N) (@houko)
- `librefang-api` → `librefang_kernel` internal import surface reduced from 47 to 11 `use` statements (branch) and further to 15 script-counted refs (7 of which are intentional `pub use` facade re-exports in `approval`, `error`, `mcp_oauth`, `middleware`, `trajectory`, `triggers`, `workflow`). Inline qualified paths in `channel_bridge.rs`, `config.rs`, `server.rs`, and `pairing.rs` consolidated into `use` imports at the top of each file; `librefang_kernel::auth::UserRole` in `pairing.rs` now routes through `crate::middleware::UserRole`. Remaining hard boundary: `LibreFangKernel` concrete type in `AppState`, `server.rs`, `channel_bridge.rs`, and `routes/agents.rs::inject_attachments_into_session` — these require invasive `KernelHandle` widening deferred to a follow-up. Refs #3744. (@houko)

- **`AppState.bridge_manager` migrated from `tokio::sync::Mutex<Option<BridgeManager>>` to `arc_swap::ArcSwap<Option<BridgeManager>>`** (#3747). Hot-reload reads are now lock-free atomic loads; the stop/swap path uses `ArcSwap::swap` + `Arc::try_unwrap` to obtain owned access for `BridgeManager::stop()`. `arc-swap` is already a workspace dependency (used by `librefang-kernel`); the `librefang-api` and `librefang-testing` crates now declare it explicitly. The `prometheus_handle` field was already absent from `AppState` (parked in a module-level `OnceLock` in `crate::telemetry`); the `peer_registry` field was also already absent (all routes call `state.kernel.peer_registry_ref()` directly). No behaviour change. (@houko)
- **macOS now skips the Keychain by default for the vault master key.** macOS Keychain ACLs are bound to the per-binary code signature, so every fresh `cargo build` invalidates the ACL and triggered an "allow" prompt on every daemon restart. The vault now uses the AES-256-GCM-wrapped file fallback at `~/Library/Application Support/librefang/.keyring` (mode 0600) by default on macOS — equivalent at-rest security in our threat model. Linux and Windows behaviour is unchanged. Override with `[vault] use_os_keyring = true` in `config.toml`, or force-disable on any platform with `LIBREFANG_VAULT_NO_KEYRING=1`. **Existing macOS users**: the daemon does one final Keychain read on first restart after upgrade, mirrors the master key into the file store, and never touches the Keychain again. To clean up the now-unused entry, run `security delete-generic-password -s librefang-vault -a master-key`. # pragma: no-attribution
- Default `api_listen` flipped from `0.0.0.0:4545` to `127.0.0.1:4545` (loopback-only). New installs are local-only by default; set `api_listen = "0.0.0.0:4545"` to expose on LAN/remote. Affects `librefang init`, the dashboard's init endpoint, and `librefang.toml.example`. `librefang start` with an explicit `--config <path>` that doesn't exist now prints a clear `librefang init` hint instead of failing obscurely. (#2766) # pragma: no-attribution
- **iOS minimum supported version raised from 14.0 to 16.0.** Required by the Tauri 2 mobile toolchain that the new mobile CI builds against. Devices on iOS 14 or 15 (iPhone 6s, original iPhone SE, iPad Air 2 and similar) will no longer be able to install the LibreFang mobile app once mobile bundles ship. Affects only the iOS app — the desktop and Android builds are unchanged. (#3970) # pragma: no-attribution

### Security

- **`jsonwebtoken` crypto-provider feature now explicitly enabled (#5128).** While landing the OIDC `sub`-required tests below, the new integration suite uncovered a pre-existing latent bug: the workspace was pulling `jsonwebtoken = "10"` with default features, which enables `pem` parsing but installs no `CryptoProvider`. `jsonwebtoken` 10.x panics at `decode::<_>` time when neither `rust_crypto` nor `aws_lc_rs` is enabled — so any real OIDC token validation on the daemon (`/api/auth/introspect`, the OAuth callback's ID-token path, and the `oidc_auth_middleware`) would have panicked the first time it processed a signed JWT. The workspace `jsonwebtoken` dep now opts in to `aws_lc_rs` to match the rustls provider that `librefang-cli` installs at startup. (@houko)

- **OIDC `sub` claim is now strictly required on the external-auth path (#5128).** Three defects collided into a session-mixing vulnerability: (1) `IdTokenClaims.sub` carried `#[serde(default)]`, so a JWT missing the `sub` claim deserialised cleanly with `sub = ""`; (2) `validate_jwt_cached` only enforced `aud` + `exp`, never `sub`; (3) the OAuth callback then called `TOKEN_STORE.store(&claims.sub, …)` keyed on the empty string, so every token-less login collided on the same slot and a fresh sign-in could silently inherit the previously-stored user's refresh token. Fix lands the defence in three independent gates so the regression cannot recur: the `#[serde(default)]` attribute is gone (a missing claim now fails deserialisation), `validation.set_required_spec_claims(&["sub","exp","aud"])` is configured before `decode::<IdTokenClaims>` (catches the missing-claim case at the JWT layer even for callers that bypass the struct shape), and `validate_jwt_cached` rejects `claims.sub.is_empty()` after decoding (catches the explicit `"sub": ""` case which is structurally valid at the first two layers). The OAuth callback adds a final `claims.sub.is_empty()` guard before `TOKEN_STORE.store`, protecting the userinfo-fallback branch (where `sub` comes from `info["sub"].or(info["id"]).unwrap_or("")` and could otherwise still land empty). The same `validate_jwt_cached` change protects `/api/auth/userinfo`, `/api/auth/introspect`, and the `oidc_auth_middleware` — every call site funnels through the one function. Tests: 3 new `#[tokio::test]` cases in `crates/librefang-api/tests/oauth_sub_required_test.rs` that generate an RSA-2048 keypair in-process, serve the JWKS from a local axum listener on `127.0.0.1:0`, and drive `/api/auth/introspect` with JWTs that exercise each gate: `introspect_rejects_jwt_with_missing_sub_claim`, `introspect_rejects_jwt_with_empty_sub_claim`, and `introspect_accepts_jwt_with_well_formed_sub_claim` as the no-regression control. (@houko)

- **CI + runtime supply-chain audit for marketplace skills/hands/templates** — `.pth` import-hijack files, `base64`+`exec`/`eval` payloads, jailbreak/exfil prompt phrases, and `sys.path`/`importlib` abuse are now caught at two layers: (1) the `supply-chain-audit` CI workflow (`.github/workflows/supply-chain-audit.yml`) gates every PR touching skill/hand/extension trees; (2) `librefang_skills::supply_chain::scan()` runs at marketplace install time and refuses the install (`SkillError::SecurityBlocked`) if any critical finding is detected, cleaning up the partially-extracted directory. Set `LIBREFANG_SKIP_SUPPLY_CHAIN_AUDIT=1` to bypass for dev/testing (emits a WARN). Closes #3333. (@houko)

- **Channel webhook HMAC verification is now mandatory** for Messenger, LINE, Teams, Viber, and DingTalk. Previously, missing signature headers were silently bypassed; they now return `400 Bad Request`, and signature mismatches return `401 Unauthorized`. **Action required if you operate any of these channels:** # pragma: no-attribution
  - **Messenger** — set `MESSENGER_APP_SECRET` to your Facebook App Secret (the new `app_secret_env` field in `[channels.messenger]` defaults to this). If unset, signatures are skipped with a startup warning and the endpoint stays unauthenticated — production should always set it. # pragma: no-attribution
  - **Teams** — set `TEAMS_SECURITY_TOKEN` to the base64 outgoing-webhook security token from the Teams portal (the new `security_token_env` field in `[channels.teams]`). Same fallback semantics as Messenger. # pragma: no-attribution
  - **LINE / Viber / DingTalk** — no new env vars, but probes that don't carry the platform's signature header (curl, monitoring health checks pointed at the webhook path) will now return 4xx instead of 200. # pragma: no-attribution
- **Outbound `[channels.webhook] callback_url` is SSRF-guarded.** Adapters refuse to start if the URL resolves to a private (`10/8`, `172.16/12`, `192.168/16`), CGN (`100.64/10`), loopback (`127/8`, `::1`), link-local, multicast, or cloud-metadata range. Catches IPv6 short forms like `[::]`, IPv4-mapped (`[::ffff:127.0.0.1]`), NAT64, and trailing-dot FQDNs. **Action required**: local dev setups using `callback_url = "http://127.0.0.1/..."` must switch to a public tunnel (ngrok, cloudflared) or omit `callback_url`. (#3942) # pragma: no-attribution
- **BREAKING**: `require_auth_for_reads` now defaults to *enabled* whenever any form of authentication is configured (`api_key`, `user_api_keys`, or dashboard credentials). Previously the flag had to be set explicitly, leaving read endpoints open even on instances with an `api_key`. Operators who deliberately want open reads on an authenticated instance (e.g. behind a trusted reverse proxy) must now set `require_auth_for_reads = false` in `config.toml`. A boot-time INFO log records when the flag is auto-enabled. (#2448) # pragma: no-attribution

### Quality

- CI Test job split into **Unit** (`lib+bin`, ~2 min, single Ubuntu runner) and **Integration** (`--tests`, sharded across 4 Ubuntu shards + macOS + Windows). Unit failures now surface in ~2 min without waiting for the full integration matrix. Local fast iteration: `cargo nextest run --workspace --lib --bins`. Full validation: `cargo nextest run --workspace --no-fail-fast`. Closes #3696. (@houko)

### Maintenance

- Wire `cargo xtask integration-test` into CI as a `live-integration-smoke` job — spawns a real `target/debug/librefang start` daemon on every PR touching Rust or CI files, hits `/api/health`, `/api/agents`, `/api/budget`, `/api/network/status`, and SIGTERMs. Catches the failure modes the in-process integration tests miss (route not registered in `server.rs`, daemon failing to bind, config fields not deserializing). Runs with `--skip-llm` to keep the gate hermetic; the live-LLM branch is reserved for the release/nightly workflow that has provider keys. (#3405) (@houko)
- **`LibreFangKernel` god-struct decomposed into 13 typed subsystem handles** (#3565 / #4756). Pre-fix the kernel held ~70 flat fields owning every subsystem the runtime touches; post-fix it holds 33 fields — 13 subsystem handles (`agents`, `events`, `memory`, `workflows`, `llm`, `security`, `skills`, `mcp`, `media`, `mesh`, `governance`, `metering`, `processes`) plus the residual cross-cutting state (boot dirs, config, wasm sandbox, context engine, log reloader, `shutdown_tx`, `prompt_metadata_cache`, `auto_reply_engine`). Inner field names are preserved verbatim so the migration across ~600 internal call-sites is a literal rename (`self.X` → `self.<sub>.X`); the three inner-name collisions are resolved as `metering.engine` (was `metering`), `workflow.engine` (was `workflows`), and `memory.substrate` (was `memory`). Each subsystem additionally exposes a focused `*SubsystemApi` trait (e.g. `MeteringSubsystemApi`, `ProcessSubsystemApi`) and `LibreFangKernel` forwards them all in `subsystem_forwards.rs`, so new callers and test mocks can bind `&dyn FooSubsystemApi` without dragging in the whole `KernelApi` surface — existing `Arc<dyn KernelApi>` flows are unchanged. Boundary tests next to `ProcessSubsystemApi` and `MeteringSubsystemApi` assert object-safety, `Send + Sync`, and routing through `&dyn`, including a `StubProcesses` mock proving the trait shape is implementable without `LibreFangKernel`; the remaining 11 traits will gain the same coverage as consumers materialise. Drop ordering is not load-bearing — `LibreFangKernel::shutdown` is broadcast-based via `shutdown_tx` and explicitly drains `agents.supervisor` → `workflows.engine` → memory in order, documented at the top of `kernel::subsystems`. Method-body migration into per-subsystem `impl` blocks and the carving of the monolithic `KernelApi` trait are left as the next refactor. (@houko)

### Documentation

- Per-crate `AGENTS.md` for the six core crates (`librefang-{kernel,runtime,types,llm-driver,extensions,channels}`). Telegraph-style: scope, module map, lock strategy, taboos, common gotchas. Each one ships with a sibling `CLAUDE.md` symlink so AI tooling that walks up looking for `CLAUDE.md` (older Claude Code builds, Codex CLI variants) finds the same rules. New CI gate `agents-claude-pair` verifies the symlink remains in place via `scripts/check-agents-claude-pair.sh`. The dashboard's existing `AGENTS.md` also gains the symlink. (#3297) (@houko)

## [2026.4.28] - 2026-04-28

_67 PRs from 4 contributors since v2026.4.27-beta6._

### Highlights

- **Auxiliary LLM client** — a dedicated cheap-tier model now handles background side tasks, reducing cost on main-agent calls
- **BytePlus, Microsoft (GitHub Models), and Z.ai providers** — three new LLM provider families added, each with their own dedicated API key env vars
- **Thread ownership** — prevents multiple agents from sending duplicate replies to the same thread; paired with a pause/resume foundation for resumable multi-step workflows
- **Redesigned Users surface and dashboard UI** — compact card grid layout, push-style adaptive drawer, unified animations, and richer markdown help drawers across all pages; empty states now land on the marketplace tab automatically
- **Auto-fill channel replies and approval notifications** — channel replies now auto-populate the recipient from the sender, and approval notifications include the agent name for clarity

### Added

- Add env_passthrough allowlist to skill manifest (#3219) (@neo-wanderer)
- Include agent name in approval notifications (#3247) (@neo-wanderer)
- Auto-Highlights + collapse boilerplate + contributor roll-up (#3257) (@houko)
- Add per_call_cost billing for video/music modalities (#3270) (@houko)
- Add byteplus + byteplus_coding providers (#3271) (@houko)
- Split _coding provider env vars onto dedicated names (#3279) (@houko)
- Add microsoft provider entry with own env var (#3281) (@houko)
- Split zai api_key_env from zhipu (#3285) (@houko)
- Stream plugin / python stderr per-line to tracing (#3256) (#3287) (@houko)
- Backfill providers missing from TUI first-run setup (#3291) (@houko)
- Aux LLM client for cheap-tier side tasks (#3314) (#3321) (@houko)
- Add file-backed cross-process rate-limit guard (#3322) (@houko)
- Auto-fill channel_send recipient from sender_id for replies (#3323) (@leszek3737)
- Internationalize Users surface (en + zh) (#3324) (@houko)
- Redesign as compact card grid (#3336) (@houko)
- Polish UI/UX across users surface (#3341) (@houko)
- Push-style drawer that adapts main content width (#3356) (@houko)
- BeforePromptBuild hook can contribute prompt sections (#3358) (@houko)
- Unify all custom animations on motion (#3365) (@houko)
- Land on marketplace tab when no servers configured (#3411) (@houko)
- Land on marketplace tab when no workflows (#3412) (@houko)
- Land on marketplace tab when nothing installed (#3413) (@houko)
- Thread ownership prevents multi-agent duplicate replies (#3414) (@houko)
- Pause/resume foundation for resumable workflows (#3418) (@houko)
- Honest card cursor + detail drawers for plugins / MCP / FangHub skills (#3422) (@houko)
- I18n keys + surface plugin / MCP catalog [i18n.<lang>] blocks via Accept-Language (#3424) (@houko)
- Regroup metrics, surface unused per-agent data, collapse endpoints (#3427) (@houko)
- Click anywhere on a channel card to open the drawer (#3434) (@houko)
- Rich markdown help drawer + page coverage + UserBudget redesign (#3435) (@houko)

### Fixed

- Unbreak main — namespace traversal substring + openapi.json bump (#3258) (@houko)
- Add dbus to buildInputs to fix failing build (#3263) (@FrantaNautilus)
- Install libdbus-1 so image builds and starts (closes #3259) (#3265) (@houko)
- Keyring is target-conditional so musl/android cross builds compile (#3267) (@houko)
- Copy deploy/ into builder so include_str! observability assets resolve (closes #3259) (#3268) (@houko)
- Show declared tools in editor and persist to **disk** (#3269) (@leszek3737)
- Recognize BYTEPLUS_API_KEY in provider key checks (#3274) (@houko)
- Silence three sources of routine WARN log spam (#3275) (@houko)
- Skip OTLP exporter when no collector is reachable (#3276) (@houko)
- Point at recovery commands when boot integrity check fails (#3277) (@houko)
- Align model_catalog/routing tests with current registry (#3280) (@houko)
- Refresh provider list after Test button so latency shows (#3288) (@houko)
- Wire missing applyDatePreset for quick-pick buttons (#3289) (@houko)
- Align useDeleteWorkflow test with removeQueries semantics (#3290) (@houko)
- Use correct path + auth for Anthropic-protocol providers (#3292) (@houko)
- Add missing librefang-llm-drivers dep to unbreak main (#3294) (@houko)
- Stop bypassing needs-changes via comment inference / push (#3312) (@houko)
- Treat Anthropic 401/403 as reachable, not auth-failed (#3316) (@houko)
- Decouple model-id assertions from registry catalog state (#3317) (@houko)
- Enforce deterministic ordering for LLM-bound registries (#3325) (@houko)
- Install libdbus-1-dev for glibc Linux CLI builds (#3357) (@houko)
- Drop layout/AnimatePresence from StaggerList to unblock clicks (#3415) (@houko)
- Regenerate kernel config schema golden after thread-ownership field (#3417) (@houko)
- Drawer not opening on hands page (DrawerPanel mount race) (#3421) (@houko)
- Add /api/auto-dream/status to dashboard read allowlist (#3426) (@houko)
- Scale Top Endpoints status bar with call volume (#3428) (@houko)
- Exempt loopback + cheaper cost for dashboard polls (#3430) (@houko)

### Changed

- Tidy env_passthrough nits from #3219 review (#3273) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Documentation

- Align display name with registry rename (#3284) (@houko)
- Align Z.ai env + add Microsoft (GitHub Models) section (#3286) (@houko)
- Expand every page-header help drawer to a real explanation (#3433) (@houko)

### Maintenance

- Add Nix build workflow to catch flake breakage on PR (#3264) (@houko)
- Add Docker build + boot smoke test on PR (#3266) (@houko)
- Regenerate Cargo.lock for librefang-llm-drivers dep (#3318) (@houko)
- Shorten MCP nav label to 'MCP' (#3410) (@houko)
- Remove Settings from left sidebar nav (#3423) (@houko)
- Expand .dockerignore for security + smaller build context (#3431) (@houko)
- Minimal rustup profile + sync mise rust to MSRV (#3432) (@houko)

</details>


## [2026.4.27] - 2026-04-27

### Added

- TUI setup wizard now offers `microsoft`, `zai`, `zai_coding`, `volcengine`, `volcengine_coding`, `byteplus`, `byteplus_coding` alongside the existing first-run options. The wizard's PROVIDERS list had drifted from `PROVIDER_REGISTRY` and silently hid these from new installs; a unit test now pins these entries against future regressions. (@houko)
- Treat CLI logins as first-class default providers (#3061) (@houko)
- Grafana Tempo + business-level span instrumentation (#3064) (@houko)
- /new creates a new session instead of resetting the current one (#3071) (@neo-wanderer)
- Support image-generation models (registry modality field) (#3074) (@houko)
- Wire chat attachment uploads in ChatPage (#3075) (@houko)
- Add Novita AI as OpenAI-compatible provider (#3076) (@houko)
- Agent name prefix on outbound + Signal plain-text default (#3077) (@houko)
- SSE attach endpoint for multi-client session co-watching (#3078) (@houko)
- Add SearXNG self-hosted search provider (#3079) (@houko)
- Add AWS Bedrock provider with Bearer token auth (#3080) (@houko)
- AuditCheck framework + first 3 CLAUDE.md gotcha checks (#3082) (@houko)
- Add LlmFamily enum + LlmDriver::family() (#3083) (@houko)
- SSE attach hook for multi-client session co-watching (#3087) (@houko)
- Add ToolApprovalClass + tool_classifier (no behavior change yet) (#3092) (@houko)
- Session lifecycle event bus (additive, no subscribers yet) (#3093) (@houko)
- Support PDF and text/code file attachments end-to-end (#3094) (@houko)
- Trajectory export endpoint with privacy redaction (#3097) (@houko)
- Extend detect_embedding_provider with vLLM + LM Studio fallback (#3099) (@houko)
- Cron multi-destination delivery with failure isolation (#3102) (@houko)
- UI for cron multi-destination delivery targets (#3103) (@houko)
- Cache /config + reject pageno=0 + annotate truncation (#3108) (@houko)
- Re-read agent context.md per turn (#3115) (@houko)
- Central slash command registry (PR-1/3) (#3122) (@houko)
- Slash command registry — CLI/TUI surface (PR-2/3) (#3123) (@houko)
- Configurable max history messages (per-agent + global override) (#3125) (@neo-wanderer)
- System_and_3 prompt cache stamping for Anthropic (M1) (#3126) (@houko)
- ParallelSafety projection for batch tool dispatch (PR-1/6) (#3127) (@houko)
- Plan_batch + path-overlap planner for tool dispatch (PR-2/6) (#3129) (@houko)
- Model metadata lookup pipeline (PR-1/3, layers 1+2+5) (#3133) (@houko)
- Model metadata L3 cache + L4 Ollama probe (PR-2/3) (#3134) (@houko)
- Model metadata L4 Anthropic + OpenAI-compat probes (PR-2.5/3) (#3140) (@houko)
- KernelConfig.parallel_tools section (PR-3/6) (#3144) (@houko)
- Cron pre_script + silent_marker schema (PR-1/3) (#3145) (@houko)
- Cache_hit_ratio metric + trajectory field (M2/2) (#3149) (@houko)
- Agent detail drawer + filter pill i18n (#3159) (@houko)
- Right-side drawer pattern for inspect-detail surfaces (#3166) (@houko)
- Convert hand detail panel to drawer variant (#3168) (@houko)
- Roll out drawer/panel pattern across all page modals (#3175) (@houko)
- Add Jaeger as second trace backend alongside Tempo (#3176) (@houko)
- Granular MCP taint policy + dashboard tree editor (closes #3050) (#3193) (@houko)
- Jaeger trace backend + Loki/Alloy logs + CLI wiring (#3194) (@houko)
- Per-(agent, session) liveness tracking and session-scoped stop (#3195) (@houko)
- RBAC M2 — audit user/channel attribution + stable UserId (#3054) (#3196) (@houko)
- Hot-reload log_level via dashboard without daemon restart (#3200) (@houko)
- RBAC M4 — channel-native role mapping (Telegram/Discord/Slack) (#3054) (#3202) (@houko)
- RBAC M5 — audit query/export + per-user budget API (#3054) (#3203) (@houko)
- RBAC M3 — per-user tool policy + memory namespace ACL (#3054) (#3205) (@houko)
- RBAC M6 — dashboard (users, identity linking, simulator, CSV import + stubs) (#3054) (#3209) (@houko)
- Per-agent + global lane caps for trigger dispatch (#3210) (@neo-wanderer)
- Auto-download voice messages mirroring file path (#3212) (@neo-wanderer)
- Wip (#3213) (@houko)
- Hand agent runtime overrides with restart persistence (#3216) (@leszek3737)
- Deliver HealthCheckFailed to notification.alert_channels (#3218) (@neo-wanderer)
- Per-user budget write/clear endpoints + dashboard editor (#3224) (@houko)
- Activate AuditPage now that M5 audit endpoints shipped (#3225) (@houko)
- Per-action retention policy with chain-anchor trim (#3227) (@houko)
- RBAC effective-permissions snapshot — wire simulator (#3054) (#3228) (@houko)
- RBAC M3 — per-user policy GET/PUT + dashboard editor (#3229) (@houko)
- RBAC — single-decision authz/check endpoint (#3054) (#3231) (@houko)
- User-list summary flags + custom channel rule editor (#3229 follow-up) (#3232) (@houko)
- Owner-only API key rotation with live session kill (#3233) (@houko)
- External mount points in agent.toml (#3230) (#3234) (@houko)
- Channel field as dynamic dropdown with custom fallback (#3248) (@houko)
- URL-synced filters, JSON export, row detail modal (#3252) (@houko)
- Move filters into right-docked drawer (#3254) (@houko)
- BeforePromptBuild hook can contribute labeled DynamicSection injected into the system prompt, with 8KiB per-section / 32KiB total caps (closes #3326) (#3358) (@houko)

### Fixed

- Reconnect WhatsApp gateway after transient disconnects (#21) (@houko)
- Render connection screen via custom URI scheme (closes #3052) (#3056) (@houko)
- Create log dir + open log before stdout redirect (#3057) (@houko)
- Surface CLI logins as their own providers, not API-provider fallbacks (#3059) (@houko)
- Pre-create logs dir in entrypoint (defense for #3058) (#3060) (@houko)
- Bundle compose stack in-binary, add OTLP collector (#3062) (@houko)
- Create HTTP trace spans at INFO so OTel exporter sees them (#3063) (@houko)
- Move env_filter to fmt layer so OTel sees INFO spans (#3065) (@houko)
- Drop ingester/compactor from Tempo config (#3067) (@houko)
- Boot-time TOML drift detection now reaches hand agents (#3068) (@neo-wanderer)
- Reprobe local providers every 60s + refresh on test (#3069) (@houko)
- Add missing files to src to fix librefang-cli build (#3073) (@FrantaNautilus)
- Honor session_mode=new with per-fire isolated sessions (#3081) (@houko)
- Copilot streaming empty tool calls + Claude assistant strip (#3084) (@houko)
- Gemini array-items default + first-message-must-be-user (#3085) (@houko)
- Safe UTF-8 boundary in three remaining truncation sites (#3086) (@houko)
- PowerShell sandbox bypass + agent-config persistence + WS race + Revolt self-host (#3088) (@houko)
- Cron preservation across hand reactivation + telegram startup timeout + token estimation includes ToolUse (#3090) (@houko)
- Capture text from intermediate tool_use iterations (#3091) (@houko)
- Percent-decode WS auth token to preserve base64 characters (#3095) (@houko)
- Skip heartbeat timeout for agents in their idle grace window (#3096) (@houko)
- Handle BrokenPipe gracefully in doctor --json (#3100) (@houko)
- UTF-8-safe error truncation + 502/504 retry + response classify tests (#3104) (@houko)
- Cap accumulated_text + document streaming non-redelivery contract (#3106) (@houko)
- Cron dedupe + next_run + token_length annotation (#3109) (@houko)
- Sticky has_processed_message replaces time-based grace (#3111) (@houko)
- Use 127.0.0.1 instead of localhost for local LLM URLs (#3112) (@houko)
- Pass agents_dir to hand route candidate scan to silence WARN flood (#3113) (@houko)
- Close non-loopback auth bypass when api_key is empty (#3114) (@houko)
- Downgrade pure-normalization to debug, keep WARN for real repair (#3117) (@houko)
- Use "default" provider/model in custom-agent template (#3121) (@houko)
- Forward api_key as Bearer in local provider probe (#3128) (@houko)
- Degrade Memory page gracefully when proactive memory is disabled (#3131) (@houko)
- Allow named workspaces in read-side path resolution (#3137) (@neo-wanderer)
- Unbreak cron_delivery tests + move guards to input validation (#3139) (@houko)
- Unbreak local provider config in GUI (#3141) (@houko)
- Re-render hand [[settings]] tail after boot-time TOML drift (#3142) (@neo-wanderer)
- Relax probe timeout for remote local-provider URLs (#3146) (@houko)
- Preserve tool annotations for parallel safety classification (PR-6/6) (#3147) (@houko)
- Include SearXNG in web_search_available check (#3152) (@houko)
- Drop redundant runtime SSRF check in deliver_webhook (#3155) (@houko)
- Add .desktop entry and install icon (#3157) (@FrantaNautilus)
- Seed [[settings]] defaults into hand instance config on activation (#3160) (@houko)
- Skip empty Blocks when stamping prompt cache markers (review fix for #3126) (#3161) (@houko)
- Expose vLLM + LM Studio in embedding provider dropdown (refs #3138) (#3162) (@houko)
- Re-render Reference Knowledge + Your Team tails after TOML drift (#3164) (@houko)
- Provide .desktop entry and icon for librefang-desktop (#3156) (#3165) (@houko)
- Regenerate config_schema golden after parallel_tools addition (#3167) (@houko)
- Stop drawer scroll chaining into the page (#3169) (@houko)
- Observability auto-start opt-in + home_dir isolation + RAII cleanup (#3170) (@houko)
- Surface provider model list above the fold (#3179) (@houko)
- Wire OS keyring (libsecret/Keychain/Credential Manager) (#3180) (@houko)
- Wrap with wrapGAppsHook3 so tray icon resolves on NixOS (#3197) (@houko)
- Probe OpenAI fallback for ollama-slot servers, hide non-discovered local models (#3204) (@houko)
- Correct max_level_hint test assertions (#3206) (@houko)
- Correct max_level_hint test assertions (#3207) (@houko)
- Set sender_user_id metadata so RBAC works in groups (#3215) (@neo-wanderer)
- Serialize channel config writes via toml_edit + lock (#3183) (#3223) (@houko)
- Attribute loopback callers to user_api_keys when token provided (#3236) (@houko)
- Invalidate effective-permissions on policy/budget mutations (#3228 follow-up) (#3237) (@houko)
- Prefix sender_chat ids so they can't collide with user namespace (#3215 follow-up) (#3238) (@houko)
- RBAC M3 follow-up — memory ACL fail-closed for anonymous callers (#3239) (@houko)
- Include prev_hash so verifiers can replay the chain (#3203 follow-up) (#3240) (@houko)
- RBAC M4 follow-up — role_cache reload + Telegram DM owner-escalation (#3241) (@houko)
- Mark scope as user_policy_only to match implementation (#3231 follow-up) (#3242) (@houko)
- Attribute admin actions to caller + log old->new diffs (#21 follow-up) (#3245) (@houko)
- Harden CSV import + flag identity-link risk (#3209 follow-up) (#3246) (@houko)
- RBAC M3 follow-up — namespace traversal + case-insensitive deny + memory audit emit (#3205) (#3249) (@houko)
- Autonomous-loop tool calls bypass user gate (closes #3243) (#3251) (@houko)
- Channel dropdown uses /api/channels for full 44-adapter list (#3253) (@houko)
- Enforce deterministic ordering for LLM-bound MCP server / skill registries to stabilize provider prompt cache (closes #3298) (#3325) (@houko)

### Changed

- Derive JSON Schema from KernelConfig via schemars (#3055) (@houko)
- Extract SessionStore trait alongside SQLite substrate (#3089) (@houko)
- Make bridge helpers crate-private (#3181) (@houko)
- Remove unused public helpers (#3182) (@houko)
- Tighten visibility of internal request structs (#3184) (@houko)
- Merge duplicate type definitions across crates (#3185) (@houko)
- Rename Action enums to disambiguate from domain types (#3188) (@houko)
- **BREAKING**: Split coding-provider API keys onto dedicated env vars — `byteplus_coding` now reads `BYTEPLUS_CODING_API_KEY` (was `BYTEPLUS_API_KEY`), `volcengine_coding` reads `VOLCENGINE_CODING_API_KEY` (was `VOLCENGINE_API_KEY`), `zai_coding` reads `ZAI_CODING_API_KEY` (was `ZHIPU_API_KEY`), `zhipu_coding` reads `ZHIPU_CODING_API_KEY` (was `ZHIPU_API_KEY`). Per-token siblings (`byteplus`, `volcengine`, `zai`, `zhipu`) keep their original env vars. Set the new env var if you use any `_coding` provider. (#3279) (@houko)
- **BREAKING**: Register `microsoft` (GitHub Models / Azure AI Inference) as an explicit driver-registry entry with its own `GITHUB_MODELS_TOKEN` env var, distinct from `github-copilot`'s `GITHUB_TOKEN`. Same PAT works for both, but the env vars are now separate so configuring one product no longer auto-activates the other in the model picker. Set `GITHUB_MODELS_TOKEN` if you use the `microsoft` provider. (#3281) (@houko)
- **BREAKING**: Split `zai` from sharing `ZHIPU_API_KEY` with `zhipu` — `zai` (api.z.ai) now reads `ZAI_API_KEY` while `zhipu` (open.bigmodel.cn) keeps `ZHIPU_API_KEY`. Same Zhipu credential value works for both, but the env vars are now separate so configuring one no longer auto-activates the other. Set `ZAI_API_KEY` if you use the `zai` provider. (#3285) (@houko)

### Documentation

- Add tool_timeouts configuration documentation (#3098) (@leszek3737)
- Backfill reference for cron / config / providers / channels / api / observability (#3189) (@houko)
- Clarify worktree continuation drives to PR (#3190) (@houko)
- Align left nav with file tree (#3199) (@houko)
- Backfill source-vs-doc gaps (providers / channels — config / API / CLI to follow) (#3201) (@houko)
- Drop HTML comment that broke Deploy Docs on main (#3208) (@houko)
- Align Chinese translations with English source (#3220) (@houko)

### Maintenance

- Rename normalize_schema_recursive + warn on items fallback (#3105) (@houko)
- Document apply_agent_prefix idempotency caveats (#3107) (@houko)
- Timing-side-channel mitigation in percent_decode (#3110) (@houko)
- Align localhost test expectations with #3112 default change (#3118) (@houko)
- Ignore local .plans/ working notes directory (#3130) (@houko)
- Sync librefang-types tracing dep into Cargo.lock (#3132) (@houko)
- Unbreak main — cargo fmt for model_metadata.rs (#3150) (@houko)
- Unbreak main — fix clippy manual_pattern_char_comparison (#3153) (@houko)
- Hand-level skills propagation regression for #3135 (#3163) (@houko)
- Pull librefang-api into selective lane on librefang-types changes (#3171) (@houko)
- Drop LEGACY_TEAM_TAIL_MARKER fallback (#3177) (@houko)
- Install libdbus-1-dev for OpenAPI Drift job (#3186) (@houko)
- Remove unused dependencies across workspace (#3187) (@houko)
- Pin push_notification routing for health_check_failed (#3222) (@houko)
- Unbreak typecheck on sessions-stream test (#3235) (@houko)
- Unbreak typecheck on UserBudgetPage + duplicate type export (#3244) (@houko)

### Other

- Unbreak main — use local user_api_keys snapshot (#3250) (@houko)


## [2026.4.24] - 2026-04-24

### Added

- Per-tool timeout overrides via [tool_timeouts] (#2990) (@houko)
- Attach to remote CDP endpoint instead of spawning Chromium (#2991) (@houko)
- Attach to remote CDP endpoint instead of spawning Chromium (#2993) (@houko)
- Configurable cron session size limit (#2994) (@houko)
- REST API for task_queue + max_retries TTL enforcement (#2997) (@houko)
- Generic OpenAI-compat driver for user-defined image providers (#2998) (@houko)
- Per-tool / per-path taint policy with TaintRuleId skip API (#2999) (@houko)
- Per-tab session_id on WebSocket + URL-driven ChatPage (incremental on #2989) (#3001) (@neo-wanderer)
- Vacuum sqlite after session prune at startup (#3002) (@houko)
- Add TransformToolResult hook for plugin tool-result rewriting (#3003) (@houko)
- Add per-provider request_timeout_secs config (#3004) (@houko)
- Preserve @mention context and show reaction processing state (#3005) (@houko)
- Write compaction summaries in the user's conversation language (#3007) (@houko)
- Add media attachment delivery support (#3008) (@houko)
- Add reactions_enabled toggle for processing state indicators (#3009) (@houko)
- Add wakeAgent gate for cron script pre-check (#3010) (@houko)
- Add deliver_only mode for zero-LLM push notifications (#3011) (@houko)
- Add send_voice and dm/group message policies (#3012) (@houko)
- Per-agent ChannelOverrides in AgentManifest (#3020) (@DaBlitzStein)
- Tee foreground daemon logs to timestamped daily files (#3022) (@houko)
- Add POST /api/tools/{name}/invoke for direct tool execution (#3025) (@houko)
- Auto-generate Python/JS/Go/Rust SDKs from openapi.json (#3046) (@houko)
- Lazy tool loading via tool_load/tool_search (closes #3044) (#3047) (@houko)

### Fixed

- Resolve 2937, build of both librefang-cli and librefang-desktop on NixOS (#2974) (@FrantaNautilus)
- Infer Ollama model capabilities from families metadata (#2987) (@houko)
- Include stdio server arg paths in MCP roots capability (#2988) (@houko)
- Per-request session_id override on message send (#2989) (@houko)
- Inject bot aliases into reply_precheck classifier prompt (#2992) (@houko)
- Tolerate trailing reasoning tokens in tool call arguments (#2995) (@houko)
- Detect vision/embedding capabilities for Ollama local models (#2996) (@houko)
- Fix connection screen IPC on Windows + add uninstall button (#3000) (@houko)
- Restore audit polling to 30s, drop expensive verify refetchInterval (#3006) (@houko)
- Add missing task_get and task_update_status to stub KernelHandle impls (#3013) (@houko)
- Guard max_tokens against zero to prevent HTTP 400 (#3014) (@houko)
- Retry LLM stream on transient errors and add SSL/TLS error patterns (#3015) (@houko)
- Detect macOS Chrome .app bundle for browser hand (#3021) (@houko)
- Gate foreground tee behind #[cfg(unix)]; fix clippy warnings (#3024) (@houko)
- Cascade parent /stop into agent_send subagents (#3044 follow-up) (#3048) (@houko)
- Add plaintext fallback when editMessageText HTML is rejected (#3051) (@DaBlitzStein)

### Changed

- Add QueryOverrides support, use withOverrides consistently (#2981) (@leszek3737)

### Performance

- Optimize React components (#2979) (@leszek3737)
- Narrow mutation cache invalidation and fix missing invalidations (#2980) (@leszek3737)

### Maintenance

- Remove deprecated providers ai21, aider, chutes, venice (#3023) (@houko)
- Bump actions/cache from 4 to 5 (#3026) (@app/dependabot)
- Bump rustls from 0.23.37 to 0.23.39 (#3027) (@app/dependabot)
- Bump webpki-roots from 1.0.6 to 1.0.7 (#3028) (@app/dependabot)
- Bump tokio from 1.50.0 to 1.52.1 (#3029) (@app/dependabot)
- Bump cbc from 0.1.2 to 0.2.0 (#3030) (@app/dependabot)
- Bump aes from 0.8.4 to 0.9.0 (#3031) (@app/dependabot)
- Bump tauri-plugin-dialog from 2.6.0 to 2.7.0 (#3032) (@app/dependabot)
- Bump semver from 1.0.27 to 1.0.28 (#3033) (@app/dependabot)
- Bump rmcp from 1.3.0 to 1.5.0 (#3034) (@app/dependabot)
- Bump tauri-plugin-single-instance from 2.4.0 to 2.4.1 (#3035) (@app/dependabot)
- Bump wasmtime from 43.0.1 to 44.0.0 (#3036) (@app/dependabot)
- Bump open from 5.3.3 to 5.3.4 (#3037) (@app/dependabot)
- Bump rustix from 0.38.44 to 1.1.4 (#3038) (@app/dependabot)
- Bump lettre from 0.11.20 to 0.11.21 (#3039) (@app/dependabot)
- Bump uuid from 1.23.0 to 1.23.1 (#3040) (@app/dependabot)
- Bump rustls-connector from 0.22.0 to 0.23.0 (#3041) (@app/dependabot)
- Bump axum from 0.8.8 to 0.8.9 (#3042) (@app/dependabot)
- Bump seccompiler from 0.4.0 to 0.5.0 (#3043) (@app/dependabot)


## [2026.4.23] - 2026-04-23

### Added

- Auto-reset stuck in_progress tasks after TTL (closes #2923) (#2953) (@houko)
- Named shared workspaces + identity file isolation (#2958) (@houko)
- Add notify_owner tool + owner_notice output boundary (#2965) (@houko)
- Moonshot/Kimi file upload support via /v1/files (#2966) (@houko)
- Download channel files to disk for agent access (#2972) (@houko)
- Session_key dispatch log + boot self-test for channel scoping (#2973) (@houko)

### Fixed

- Drop ellipsis-terminated preambles without tool_use as silent (#2617) (@f-liva)
- Suppress NO_REPLY sentinel in streaming bridge, cron, and auto-reply (#2743) (@DaBlitzStein)
- Make split_message HTML-tag-aware for Telegram (#2760) (@DaBlitzStein)
- Auto-inject sender peer_id into cron jobs + delegation trust prompt (#2869) (@DaBlitzStein)
- Route trigger-fired responses to agent's home channel (closes #2872) (#2952) (@houko)
- Render real chat message timestamps on resume (closes #2934) (#2954) (@houko)
- Apply assignee_match:self filter to task_posted triggers (closes #2924) (#2955) (@houko)
- Inject bot identity into reply_precheck classifier (#2960) (@houko)
- Sanitize bot_name in classify_reply_intent prompt; add unit tests (#2961) (@houko)
- Tolerate tool_call_id collisions across turns in session_repair (#2962) (@houko)
- Inject RELAY prompt only on explicit owner intent (#2967) (@houko)
- Add missing timestamp field in session_repair Message structs (#2968) (@houko)
- Fix all missing timestamp fields and incomplete test stubs (#2969) (@houko)
- Read peer_id from job_json in cron_create (#2970) (@houko)
- Recover Signal session when upsert delivers null payload (#2971) (@houko)


## [2026.4.22] - 2026-04-22

_No notable changes._

## [2026.4.21] - 2026-04-21

### Added

- Complete trigger feature — persistence, CRUD API, CLI subcommands, dashboard UI (#2827) (#2830) (@houko)
- Add account_id to channel_send for explicit multi-bot routing (#2845) (@houko)
- Add per-agent auto_evolve flag to skip background skill review (#2846) (@houko)
- Implement MCP Roots capability (#2847) (@houko)

### Fixed

- Correct query invalidation and missing data flow across mutations (#2770) (@leszek3737)
- Harden workflow save and draft state (#2781) (@leszek3737)
- Align mutation flows across config channels goals and hands (#2782) (@leszek3737)
- Unify dashboard query hooks and flow guards (#2783) (@leszek3737)
- Exempt Unix/Slack-style timestamps from PII phone check (#2795) (@neo-wanderer)
- Change wizard default ollama model to gemma3:4b (#2811) (@houko)
- Strip empty assistant messages unconditionally (#2812) (@houko)
- Auto-delete At-schedule jobs after execution (#2808) (#2814) (@houko)
- Reimplement apply_seccomp_allowlist with libc::SYS_* constants (#2817) (@houko)
- Allow dashboard static assets through auth gate (#2824) (@leszek3737)
- Force wildcard bind for api_listen in Docker (#2825) (@leszek3737)
- Resolve channel_bridge test deadlock that blocked CI for 6h (#2829) (@houko)
- ChatPage — type safety, cache correctness, cleanup (#2832) (@leszek3737)
- Correct event sequence in show_progress=false test (#2834) (@houko)
- Exempt dashboard and static paths from GCRA rate limiter (#2835) (@houko)
- Use main as default branch for ~/.librefang git repo (#2837) (@houko)
- Task_claim() now matches assigned_to by name as well as UUID (#2844) (@houko)
- Dashboard refresh no longer drops history — unify webui session with canonical (#2848) (@houko)
- Type-safety and RC-safe fixes (#2849) (@leszek3737)
- Unbreak --all-features build + stop warning on local LLM providers (#2850) (@houko)
- Per-job session_mode override to fix context accumulation (#2647) (#2851) (@houko)
- Proactive extraction loses JSON mode through fork path + log noise cleanup (#2852) (@houko)

### Changed

- RC cleanup for ModelsPage (#2833) (@leszek3737)
- Relocate config backups under ~/.librefang/backups/ (#2838) (@houko)
- Move stray state/log files out of ~/.librefang root (#2840) (@houko)

### Documentation

- Add unofficial wiki link and DeepWiki badge to READMEs (#2821) (@leszek3737)

### Maintenance

- Run Windows and macOS tests on affected crates for every Rust PR (#2819) (@houko)
- Follow-up cleanup from #2783 review (#2820) (@houko)
- Ignore rust_out build artifact (#2836) (@houko)


## [2026.4.20] - 2026-04-20

### Added

- Canonical silent-response primitive, end the NO_REPLY literal leak (#2470) (@f-liva)
- Gate /dashboard/* behind auth + tailwind v4 renames (#2785) (@houko)
- Add stop button to interrupt in-flight agent streams (#2787) (@neo-wanderer)
- Add native Cohere driver (#2791) (@houko)
- Show tool execution progress in channel replies (#2792) (@houko)
- Finish channel-progress — universal coverage, Telegram fix, show_progress, i18n, prettify, dashboard parity (#2793) (@houko)
- Redesign `librefang status` for layered visibility (#2799) (@houko)
- Unify create/edit modals + inline rename (#2800) (@houko)

### Fixed

- Make extract_categories config drive LLM prompt categories (#2761) (@neo-wanderer)
- Sync terminal health and active window state (#2777) (@leszek3737)
- Clear history consistently and refresh model state (#2780) (@leszek3737)
- Align shared query flows for MCP, skills, and workflows (#2784) (@leszek3737)
- Route comms_task through kernel wrapper; surface task system events (#2789) (@neo-wanderer)
- Rewrite /install to /install.sh for CLI clients (#2794) (@houko)
- Stop writing PATH into the wrong rc file (#2796) (@houko)
- Auto-activate PATH after installation (#2797) (@houko)
- Bypass auth for loopback connections (#2802) (@houko)
- Drop stray </div> from #2800 modal refactor (#2803) (@houko)
- Surface reload error to dashboard instead of opaque 'saved but reload failed' (#2805) (@houko)
- Validate config BEFORE writing TOML so failed saves don't corrupt the file (#2806) (@houko)

### Documentation

- Clarify session_mode scope — cron/channels/forks ignore it (#2790) (@neo-wanderer)

### Maintenance

- Split PR/main pipelines; compute affected crates precisely (#2801) (@houko)
- Merge release-* workflows into one (keep notify) (#2804) (@houko)


## [2026.4.19] - 2026-04-19

### Added

- Add auto-dream per-agent background memory consolidation (#2750) (@houko)
- Trigger on AgentLoopEnd hook, scheduler becomes backstop (#2755) (@houko)
- Derivative LLM calls reuse parent's prompt cache (#2767) (@houko)

### Fixed

- Show Provider before Model in Config default_model section (#2749) (@houko)
- Add peer_id to cron jobs for peer-scoped memory access (#2759) (@DaBlitzStein)
- Match ImageFile in vision dispatch gates (#2762) (@DaBlitzStein)
- Default api_listen to 127.0.0.1:4545 for local-only startup (closes #2766) (#2769) (@houko)
- Clear stale TOTP banners, refetch status on reset, localize error messages (#2771) (@leszek3737)
- Fix 12 UI bugs across scheduler, sessions, memory, models, plugins, providers, runtime, workflows (#2772) (@leszek3737)
- Gate Duration import with cfg(unix) for Windows CI (#2773) (@houko)
- Harden canvas workflow recovery and related UI state (#2774) (@leszek3737)
- Derive 'connected' from health state + fix catalog card overflow (closes #2738) (#2775) (@houko)
- Align workflow mutation invalidation (#2778) (@leszek3737)

### Documentation

- Fix stale documentation references (#2720) (@leszek3737)

### Maintenance

- Replace cloudflare/wrangler-action with direct npx wrangler calls (#2740) (@houko)


## [2026.4.18] - 2026-04-18

### Added

- Forked agent pattern: kernel exposes `run_forked_agent_streaming(agent_id, prompt, allowed_tools)` for derivative LLM calls that share the parent turn's system + tools + message prefix (Anthropic prompt cache alignment) without persisting the derivative's messages into the canonical session. Anthropic driver's `cache_control` extended from system-only to cover both the last tool block (system + tools prefix) AND the last content block of the last message (full conversation prefix), giving forks near-full cache coverage. Dashboard settings page now surfaces cache-hit rate and per-dream cost so the forkedAgent savings are visible. Proactive-memory `LlmMemoryExtractor` migrated to the forkedAgent pattern: a new trait method `extract_memories_with_agent_id` routes the extraction LLM call through `KernelHandle::run_forked_agent_oneshot` (a new trait method that drives a single-turn fork and returns the final text), sharing the parent agent's `(system + tools + messages)` cache key. The extraction-specific system prompt is embedded into the fork's user message rather than replacing the agent's system prompt, so cache alignment holds. Fall back to a standalone `driver.complete()` with `prompt_caching = true` when no kernel handle is installed (tests / rule-based extractor / fork failure) so system-prompt caching still applies. Kernel wires the extractor's weak handle inside `set_self_handle` — first call only, matching the auto-dream hook idempotency pattern. Migrates auto-dream off its previous `SenderContext { channel: "auto_dream" }` side-channel pattern — dreams now fork from the canonical session and the kernel-side `channel == AUTO_DREAM_CHANNEL` tool filter is replaced by runtime `LoopOptions::allowed_tools` enforcement at tool execute time (request schema stays byte-identical to parent for cache alignment, model's `tool_use` for disallowed tools returns synthetic error). Agent loop adds `LoopOptions { is_fork, allowed_tools }` threaded through; fork turns skip `save_session_async` and add `"is_fork": true` to `AgentLoopEnd` hook context data so subscribers can filter fork events. Auto-dream's own hook filters fork turns to avoid dream-triggers-dream recursion. (@houko)
- Auto-dream: per-agent background memory consolidation with four-layer gating (global / per-agent opt-in / time / session count / file lock). Triggered event-driven from the `AgentLoopEnd` hook (fires the moment an agent finishes a turn) with a sparse daily backstop scheduler for opted-in agents that never turn. Includes web dashboard toggle card, TUI Dashboard strip, `[auto_dream]` config section, `DreamConsolidation` audit events with token and cost capture, runtime tool allowlist enforcement, and `GET/POST/PUT /api/auto-dream/status|trigger|abort|enabled` endpoints. (#2750) (@houko)

### Maintenance

- Drop bogus npm cache config on setup-node (#2736) (@houko)


## [2026.4.15] - 2026-04-15

### Added

- Add LIBREFANG_DASHBOARD_EMBEDDED_ONLY env var to pin dashboard to embedded assets (#2520) (@neo-wanderer)
- Add TOTP scope selector in Settings (#2526) (@houko)
- Add section tab switcher to config category pages (#2532) (@houko)
- Add voice input button to ChatPage (#2533) (@houko)
- Swap tab bar and page header positions in config pages (#2534) (@houko)
- Polish config page layout and UX (#2535) (@houko)
- Step-by-step provider creation wizard (#2544) (@houko)

### Fixed

- Scope telegram sessions per chat_id to prevent context leakage (#2349) (#2522) (@DaBlitzStein)
- Honour silent flag in KernelBridgeAdapter sender methods (#2521) (#2523) (@DaBlitzStein)
- Use is_some_and instead of map_or in webchat asset_path check (#2525) (@houko)
- Move TOTP scope to ConfigPage via schema (#2527) (@houko)
- Restore ready-for-review when blockers are cleared (#2528) (@houko)
- Fall back to npm when pnpm is unavailable in dev command (#2529) (@houko)
- Check review state before clearing needs-changes on push (#2530) (@houko)
- Remove needless borrow in serde_json::to_value call (#2531) (@houko)
- Show disabled mic button when STT not configured (#2536) (@houko)
- Fix stale state bugs in provider config modal (#2537) (@houko)
- Move field description to label column (#2538) (@houko)
- Show field description below input/toggle (#2539) (@houko)
- Save API key on provider creation and show remove button for all providers (#2540) (@houko)
- Improve provider auto-detection accuracy and UX (#2542) (@houko)
- Remove orphaned doc comment causing clippy failure on main (#2543) (@houko)


## [2026.4.14] - 2026-04-14

### Added

- Pass image blocks to CLI via @path references (#2331) (@f-liva)
- MCP OAuth discovery for Streamable HTTP transport (#2346) (@neo-wanderer)
- Add require_auth_for_reads to lock down dashboard reads (#2398) (@houko)
- Per-call deep-thinking toggle and reasoning display (#2423) (@houko)
- Add audit.anchor_path to redirect the tip-anchor file (#2442) (@houko)
- Enrich registry cards with manifest metadata (#2452) (@houko)
- Channel scoping enforcement, proactive LID, heartbeat watchdog, jittered backoff (#2462) (@f-liva)
- PR review state and issue response tracking labels (#2471) (@houko)
- Multi-page configuration editor under Configuration nav group (#2473) (@houko)
- Group addressee detection — stop responding when not actually spoken to (#2480) (@f-liva)
- Per-provider cost/token limits (#2316) (#2482) (@houko)
- Add qwen3.6-plus from coding plan (#2494) (@joshuachong)
- Add echo tracker to drop our own messages reflected back (#2498) (@f-liva)

### Fixed

- Transcode .oga to .ogg before Whisper transcription (#2386) (@f-liva)
- Relax brittle alibaba-coding-plan model count assertion (#2388) (@houko)
- Block SSRF via IPv4-mapped IPv6 addresses (#2396) (@houko)
- Reject path traversal in agent template name param (#2397) (@houko)
- Require trusted_manifest_signers for signed manifests (#2407) (@houko)
- Make NonceTracker check_and_record atomic and bounded (#2408) (@houko)
- Block SSRF via NAT64 well-known prefix (64:ff9b::/96) (#2409) (@houko)
- Stop leaking sandbox watchdog threads (#2410) (@houko)
- Extend IPv4-mapped IPv6 SSRF guard to remaining call sites (#2411) (@houko)
- Clippy regressions from refactor splits (#2404, #2406) (#2412) (@houko)
- GCRA rate limiter never honoured per-key token exhaustion (#2413) (@houko)
- Strip parent env before host_shell_exec spawns child (#2417) (@houko)
- Tighten upload MIME allowlist to match SECURITY.md (#2419) (@houko)
- Split_message panic on multi-byte UTF-8 at boundary (#2285) (#2420) (@houko)
- Add default connect/read timeouts to shared HTTP client (#2340) (#2421) (@houko)
- Lock Owner-only writes away from Admin-role API keys (#2422) (@houko)
- Copy button silently failing in non-secure contexts (#2424) (@houko)
- At schedules in the past no longer fire forever (#2337) (#2425) (@houko)
- Task_claim accepts agent name in addition to UUID (#2330) (#2427) (@houko)
- Emit stub tool_results when batch is interrupted (#2381) (#2428) (@houko)
- Actually extract WWW-Authenticate from rmcp AuthRequired (#2429) (@houko)
- Hot-reload of agent.toml updates ResourceQuota immediately (#2317) (#2430) (@houko)
- Add external tip anchor to audit log to detect full rewrites (#2431) (@houko)
- Default delivery to LastChannel instead of None (#2338) (#2432) (@houko)
- Session_repair phase 3 preserves tool-call boundaries (#2353) (#2433) (@houko)
- Claude_code fails fast when agent has tools (#2314) (#2434) (@houko)
- Wire audit log through with_db_anchored by default (#2436) (@houko)
- Use full viewport width for page content (#2439) (@houko)
- Enforce capability inheritance at spawn_agent_inner (#2440) (@houko)
- Terminal WebSocket rejected local-dev daemons with no api_key (#2441) (@houko)
- Break Feishu bot self-echo loop (#2435) (#2443) (@houko)
- Extend taint-sink checks to agent_send and web_fetch body/headers (#2444) (@houko)
- Terminal WebSocket froze after ~10 keystrokes from per-message cap (#2445) (@houko)
- Cap chat message bubble width for readability (#2446) (@houko)
- Taint-scan MCP tool-call arguments before send (#2447) (@houko)
- Derive require_auth_for_reads from api_key when unset (#2448) (@houko)
- Make overview stats cards responsive at md breakpoint (#2449) (@houko)
- Tighten recent agents grid and widen running hand chips (#2450) (@houko)
- Repair mobile layout breakage across pages (#2451) (@houko)
- Tighten card grid breakpoints across pages (#2453) (@houko)
- Revert issue auto-label body scan, keep keyword expansion (#2457) (@houko)
- Match camelCase/snake_case keywords in issue auto-label (#2461) (@houko)
- Scope canonical context injection per session to stop cross-chat leak (#2464) (@f-liva)
- Stop killing unrelated process groups in tree-kill path (#2472) (@houko)
- Bridge LibreFang tools to claude_code driver via MCP config (#2314) (#2478) (@houko)
- Scope canonical context injection per session to stop cross-chat leak (#2464) (#2490) (@houko)
- Wire MCP bridge end-to-end for claude_code (#2314) (#2495) (@houko)
- Use direct libc::kill syscall to prevent Ubuntu CI SIGTERM (#2497) (@houko)

### Changed

- Extract http_client into librefang-http shared crate (#2389) (@houko)
- Extract metering into librefang-kernel-metering subcrate (#2395) (@houko)
- Extract oauth flows into librefang-runtime-oauth subcrate (#2400) (@houko)
- Extract mcp into librefang-runtime-mcp subcrate (#2403) (@houko)
- Extract drivers and llm_driver trait into subcrates (#2404) (@houko)
- Extract wasm sandbox and kernel-handle trait into subcrates (#2405) (@houko)
- Extract hand/template router into librefang-kernel-router subcrate (#2406) (@houko)
- Remove bare SignedManifest::verify() and inline it as private (#2437) (@houko)
- Rename librefang-runtime-drivers to librefang-llm-drivers (#2467) (@houko)
- Extract pure helpers and tests out of kernel.rs (#2469) (@houko)

### Documentation

- Describe prompt-injection scanner as a heuristic (#2399) (@houko)
- Audit chain is tamper-evident only against partial edits (#2415) (@houko)
- Narrow the secret-zeroization claim to its actual scope (#2416) (@houko)
- Describe taint tracking as a two-sink pattern match (#2426) (@houko)
- Document additive penalty assumption in fallback recover (#2465) (@f-liva)

### Maintenance

- Stabilize load_endpoint_latency against shared-runner jitter (#2418) (@houko)
- Remove stray empty .codex marker file (#2454) (@houko)
- Broaden issue auto-label coverage and add backfill (#2455) (@houko)
- Refresh dashboard screenshot and drop unused images (#2456) (@houko)
- Address houko follow-ups on oga transcode (#2459) (@f-liva)
- Tidy repo metadata and remove stale api-docs (#2466) (@houko)
- PR conflict/CI-failure detection and issue status labels (#2481) (@houko)
- Sync Cargo.lock with librefang-api toml_edit dep (#2500) (@houko)
- Sync Cargo.lock after librefang-llm-driver dep addition (#2501) (@houko)


## [2026.4.13] - 2026-04-13

### Added

- Allow editing hand agent model settings from agents page (#2335) (@leszek3737)
- Add config-driven session_mode for agent triggers (#2341) (@neo-wanderer)
- Telegram rich media, polls, interactive commands, and channel_send tool (#2356) (@leszek3737)

### Fixed

- Decryption retry, streaming tag leak, session isolation (#2217) (@f-liva)
- Inherit kernel default_model instead of hardcoded Anthropic (#2299) (@houko)
- Per-agent loading state so streaming one agent doesn't block others (#2324) (@houko)
- Write MCP server config as TOML table, not stringified JSON (#2327) (@houko)
- Load secrets.env autonomously at boot time (#2359) (@f-liva)
- Prevent zombie processes on shutdown (#2360) (@f-liva)
- Refuse direct DELETE on hand-spawned agents + clarify revert warning (#2361) (@houko)
- Normalize MIME type parameters before allowlist check (#2362) (@f-liva)
- Resolve LID JIDs to phone numbers for owner detection (#2363) (@f-liva)
- Harden poll_options parsing and poll context cleanup (#2364) (@houko)
- Deterministic prompt context ordering and raise truncation cap (#2365) (@houko)
- Stop Qwen driver from leaking raw JSON into chat (#2366) (@f-liva)
- Let FallbackDriver recover from transient unhealthiness (#2367) (@f-liva)
- Clear stale per-agent overrides on provider switch (#2371) (@neo-wanderer)
- Scrub NO_REPLY sentinel in every reply path (#2373) (@f-liva)
- Restore /message/send-audio endpoint accidentally removed in #2217 (#2376) (@f-liva)
- Support "date" metric format and drop ureq from cli (#2382) (@houko)

### Performance

- Shrink dev debug info to line-tables-only (#2378) (@houko)

### Maintenance

- Split Docker image and deploy status (#2323) (@houko)
- Fix max_tokens assertions after pure-text short-circuit (#2325) (@houko)
- Strengthen telegram sanitizer coverage (#2334) (@leszek3737)
- Fix rustfmt on upsert_mcp_server test assert (#2358) (@houko)
- Replace cat with sleep in process_manager tests to fix flake (#2375) (@houko)
- Skip security and install-smoke on unrelated PRs (#2377) (@houko)
- Apply cargo fmt to runtime drivers (#2380) (@houko)


## [2026.4.11] - 2026-04-11

### Added

- Add WebSocket terminal with PTY backend and xterm frontend  (Phase 1) (#2229) (@leszek3737)
- Claude Code CLI profile rotation for rate-limit resilience (#2249) (@f-liva)
- Add MCP Servers management page (#2278) (@houko)
- Raise MSRV to 1.94.1 and keep stable toolchain (#2302) (@houko)
- Uninstall hand (#2312) (@houko)

### Fixed

- Change Docker setup to fix permissions for LIBREFANG_HOME (#2240) (@Cruel)
- Also ignore secrets.env (dashboard-managed env file) (#2248) (@DaBlitzStein)
- Localize agent template copy for zh users (#2257) (@houko)
- Restore approval context and dashboard auth flows (#2272) (@houko)
- Exclude Hand sub-agents from channel routing fallback (#2276) (@houko)
- Accept claude-code (hyphen) in CLI profile rotation guard (#2284) (@f-liva)
- Replace --verbose with --include-partial-messages for qwen driver (#2290) (@f-liva)
- Add missing cli_profile_dirs to DefaultModelConfig literals (#2296) (@houko)
- Delegate first-boot config to librefang init (#2297) (@houko)
- Scan workspaces/ dir to persist locally-installed hands across boot (#2298) (@houko)
- Hide delete button for built-in providers, flag custom (#2300) (@houko)
- Mark manifest mut in parse_manifest (#2306) (@houko)
- Stop middleware path normalization from swallowing GET / (#2307) (@houko)
- Preserve pending Telegram updates across daemon restart (#2309) (@houko)
- Stop agent loop on pure-text max_tokens overflow (#2310) (@houko)
- Make Hands Settings tab actually editable (#2311) (@houko)
- Wire ConPTY resize on Windows (#2313) (@houko)

### Changed

- Harden and optimize Telegram adapter (#2223) (@leszek3737)

### Maintenance

- Cover full-path context hook launchers (#2255) (@houko)
- Cover wechat and wecom multi-account config parsing (#2258) (@houko)

### Other

- Feat(ws) harden terminal websocket follow-ups after #2229 (#2304) (@houko)


## [2026.4.10] - 2026-04-10

### Added

- Per-channel session isolation via deterministic UUID v5 (#2097) (@f-liva)
- Save channel images as files instead of inline base64 (#2098) (@f-liva)
- TOTP second-factor for critical tool approvals (#2131) (@houko)
- Proper resource composition for hand agents (#2133) (@houko)
- Add extra_params support for openai compatible model (#2181) (@houko)
- Add config export/backup endpoint and UI button (#2186) (@houko)
- Prefill TOML editor from template selection (#2187) (@houko)
- Add per-channel auto-routing with configurable strategies (#2189) (@houko)
- Allow hooks to access vault secrets via allowed_secrets (#2216) (@houko)
- Add [config] section support to plugin.toml (#2218) (@houko)
- Add [[requires]] system binary checks to plugin.toml (#2219) (@houko)

### Fixed

- Detect "[no reply needed]" as silent response (#2093) (@f-liva)
- Harden agent loop tool flow and trim handling (#2135) (@leszek3737)
- Timezone-aware schedule creation (#2138) (@f-liva)
- Replace librefang.dev with librefang.ai (#2147) (@houko)
- Glob-match declared tools and auto-promote shell_exec exec_policy (#2148) (@houko)
- Persist mcp server updates in patch agent (#2151) (@TechWizard9999)
- Use codex exec for codex cli driver (#2153) (@TechWizard9999)
- Improve Claude Code detection for keychain auth and non-login shells (#2166) (@x86txt)
- Show active agent count instead of total in overview card (#2170) (@DaBlitzStein)
- Handle SkillHub search response format with proper headers (#2171) (@DaBlitzStein)
- Suppress CMD window flash on Windows (#2159) (#2176) (@houko)
- Resolve hand.toml agent scan conflict (#2136) (#2177) (@houko)
- Parameter errors trigger self-correction not user report (#2144) (#2178) (@houko)
- Resolve pre-existing clippy and test compile failures (#2180) (@houko)
- Multi-bot Telegram routing uses account_id, not first-match on allowed_users (#2183) (@houko)
- Resolve build errors and clippy warnings (#2184) (@houko)
- Skip auto-init when piped via curl, prompt user to run manually (#2190) (@houko)
- Clean up post-install messaging for piped installs (#2192) (@houko)
- Replace as_deref() with as_ref() for ChannelOverrides in bridge.rs (#2193) (@houko)
- Add missing extra_body field to make_completion_request (#2197) (@houko)
- Remove dead completion_timeout_override and build_completion_request (#2198) (@houko)
- Derive Default for PluginManifest (#2205) (@houko)
- Add INFO logs for all ingest hook success paths (#2213) (@houko)
- Reduce agent count display lag on state changes (#2215) (@houko)
- Decryption retry, streaming tag leak, session isolation (#2217) (@f-liva)
- Filter tool_use/tool_result blocks from chat rendering (#2220) (@f-liva)
- Resolve default provider in agent detail endpoint (#2221) (@DaBlitzStein)
- Resolve default provider before creating driver (#2222) (@DaBlitzStein)
- Add error handling to channel config dialog (#2224) (@DaBlitzStein)
- Default to unconfigured tab when no channels are set up (#2225) (@DaBlitzStein)
- Propagate ClawHub/Skillhub errors instead of returning 200 OK with empty items (#2231) (@DaBlitzStein)
- Fix compile errors and rustfmt from Custom variant merge (#2234) (@houko)
- Show embedding status ok when fts_only mode is active (#2236) (@houko)
- Rustfmt formatting in snapshot handler (#2237) (@houko)
- Rustfmt formatting in config routes (#2238) (@houko)
- Merge extra_body into JSON Value to avoid duplicate keys (#2239) (@shilkazx)
- Scope RwLockReadGuard before await in dashboard_snapshot (#2241) (@houko)
- Increase dark theme surface opacity for readable dropdowns (#2242) (@houko)
- Always load marketplace skills even without search keyword (#2243) (@houko)

### Changed

- Typed enums, O(1) indexes, and typed persistence v4 (#2161) (@leszek3737)

### Maintenance

- Apply rustfmt formatting across bridge, router, kernel, system (#2195) (@houko)
- Remove extra blank line in agent_loop.rs (#2203) (@houko)
- Remove mempalace-indexer from contrib — moved to registry (#2247) (@houko)


## [2026.4.7] - 2026-04-07

### Fixed

- Resume agent loops after approval without blocking (#2101) (@leszek3737)
- Skip Discord notification when release workflows are cancelled (#2129) (@houko)
- Embed dashboard in release binaries (#2132) (@houko)

### Maintenance

- Add desktop build/dev recipes to justfile (#2134) (@houko)


## [2026.4.6] - 2026-04-06

### Added

- Hot-reload skills dir and per-agent manifest (#2069) (@houko)
- Unify full-section empty/error states (#2088) (@houko)
- Focus trap + aria-modal + more n-shortcut coverage (#2092) (@houko)
- Add send-audio endpoint for voice notes and audio files (#2099) (@f-liva)
- Language-agnostic hook runtime (V / Go / Deno / Node / native) (#2100) (@houko)

### Fixed

- Allow tool retry on failure instead of early loop termination (#2065) (@neo-wanderer)
- Sync openclaw/openfang with current KernelConfig schema (#2066) (@houko)
- Stop stale messages_before index from breaking auto_memorize & append_canonical (#2068) (@houko)
- Agent_send/kill fall through to name lookup for stale UUIDs (#2070) (@houko)
- Reject missing required tool params instead of silent empty (#2071) (@houko)
- Surface silent session-cleanup failures and panic on empty chunks (#2072) (@houko)
- Return 404 for missing agents and reject malformed target_agent_id (#2073) (@houko)
- Log when webhook/dingtalk bridge drops incoming messages (#2074) (@houko)
- Surface agent tick panics instead of silent join drop (#2075) (@houko)
- Emit skills/workspace/tool_blocklist during OpenClaw import (#2076) (@houko)
- Providers.rs persistence failures + expect() panic (#2077) (@houko)
- Surface silent DB errors and wrap merge updates in tx (#2078) (@houko)
- Surface episodic memory persist failures in agent_loop (#2079) (@houko)
- Sanitize user-controlled identity fields in prompt builder (#2080) (@houko)
- Reload path must clamp bounds and clamp max_cron_jobs=0 (#2081) (@houko)
- Close SSRF via redirect + URL-encoding bypass in taint (#2082) (@houko)
- Route media tools through workspace sandbox (#2083) (@houko)
- Guard sandbox ptr arithmetic with checked_add (#2084) (@houko)
- ChatPage session-cache save effect + tool call keys (#2085) (@houko)
- Cascade agent-scoped tables on remove_agent (#2086) (@houko)
- Authorize cron_cancel + cap knowledge_query depth (#2087) (@houko)
- Use PAT for release creation so dashboard-build fires (#2094) (@houko)
- Suppress error messages in groups, show rate-limit in DMs only (#2095) (@f-liva)
- Auto-close unclosed HTML tags, plain-text fallback, and reply-to photo support (#2096) (@f-liva)
- Drop Ubuntu RUST_TEST_THREADS to 1 (#2117) (@houko)
- Unify agent manifest path on workspaces/agents/ (#2118) (@houko)

### Changed

- Align URL hierarchy with sidebar nav groups (#2119) (@houko)

### Maintenance

- Fix test_image_analyze_missing_file after sandbox wiring (#2103) (@houko)
- Ignore plugin scaffold templates (#2120) (@houko)

### Reverted

- V2026.4.6 stable release (was meant to be beta15) (#2126) (@houko)


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
