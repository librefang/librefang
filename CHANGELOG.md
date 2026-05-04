# Changelog

All notable changes to LibreFang will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project uses [Calendar Versioning](https://calver.org/) (YYYY.M.DD).

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


## [Unreleased]

### Fixed

- **Centralised `KernelOpError → ApiErrorResponse` mapping** for the #3541 typed-errors series. The 7 stacked PRs (#4608–#4619) shipped per-route ad-hoc matches that drifted: `approvals.rs` mapped *every* `KernelOpError` to 404 (so an `Unavailable` "approval system disabled" surfaced as 404 instead of 503); `prompts.rs` mapped *every* error to 500 (so `NotFound { kind: "prompt_version" }` collapsed to 500 instead of 404); `task_queue.rs::map_kernel_op_err` mapped `Unavailable` to 500 instead of the documented 503; `workflows.rs::create_cron_job` mapped `Unavailable` to 500 and `Other` to 400. New `impl From<KernelOpError> for ApiErrorResponse` in `librefang-api/src/error.rs` is now the single source of truth (`NotFound→404, Invalid→400, Unavailable→503, Serialize/Other→500`); all four routes delegate to it. Adds machine-readable `code: not_found / invalid_input / service_unavailable / serialize_failed / internal_error`. (#3541 follow-up) (@houko)
- **Replace the `agents.rs` `format!("{e}").contains("Agent not found")` substring grep** with a structural match on the `KernelError::LibreFang(LibreFangError::AgentNotFound | QuotaExceeded)` variants in the `send_message` handler — eliminating the typed-grep that the #3541 series claimed to retire but missed at this hot-path call site. The session-mismatch branch still flows through `LibreFangError::Internal(_)` at the kernel side and remains a substring check scoped to that variant; eliminating that last grep needs a kernel emit-site refactor to a typed `SessionAgentMismatch` variant, tracked as a separate follow-up. (#3541 follow-up) (@houko)
- **`PromptStore` kernel impl: classify input-validation failures as `Invalid { field, reason }`** instead of `Other(format!("Invalid X ID: …"))`. Affects `agent_id`, `experiment_id`, `variant_id`, and `version_id` parse failures across `get_running_experiment`, `record_experiment_request`, `get_prompt_version`, `delete_prompt_version`, `set_active_prompt_version`, `get_experiment`, `update_experiment_status`, and `get_experiment_metrics`. Combined with the new central mapping above, malformed IDs now surface as 400 instead of 500. The `"Prompt store not initialized"` `ok_or` arms migrated to `KernelOpError::unavailable("Prompt store")` (503) so callers can distinguish "feature wired off" from generic 500. Closes the explicit follow-up debt acknowledged in the #3541 6/N CHANGELOG entry. (#3541 follow-up) (@houko)
- **`idempotency_test` filter to exclude the auto-spawned `assistant`** when asserting `agent_registry().list().len()`. The 3 failing tests (`spawn_with_idempotency_key_replays_response`, `spawn_with_reused_key_different_body_is_409`, `spawn_without_idempotency_key_is_unchanged`) were main-red since they merged in #4565 because `LibreFangKernel::boot_with_config` auto-creates a default `assistant` agent on a fresh registry — the assertions assumed an empty registry. New `test_spawned_agents` helper filters the bootstrap agent so each test counts only the agents it explicitly created. Test-only change; no production behaviour shift. (@houko)
- **`session_repair::tests::prop::validate_and_repair_no_orphans_no_dup_results` proptest invariant 3 refined** to match the actual repair-pipeline contract. The original "no duplicate ToolResult tool_use_ids in the output" was structurally inconsistent with the explicit `reorder_preserves_per_turn_synthetic_when_tool_id_collides_across_turns` regression test, which deliberately requires both ToolResults to survive when a ToolUse id is reused across multiple assistant turns (Moonshot/Kimi per-completion-counter pattern, e.g. `memory_store:6` reset per call). The proptest now mirrors `deduplicate_tool_results`'s `collision_ids` logic: ids that appear in >1 assistant turn are positional duplicates by design and skip the uniqueness check; everything else is still required to be unique. Test-only change; no production behaviour shift. Fixes the fourth main-red CI failure that has been blocking the #3541 stack from going green. (@houko)

### Added

- `cargo xtask check-changed` — local mirror of the `changes` job in `.github/workflows/ci.yml`. Computes which CI lanes a branch's diff would trigger (rust / docs / ci / install / workspace_cargo / xtask_src), the `full_run` and `full_test` decisions, and the affected crate set (with the schema-mirror rule that pulls `librefang-api` in for any `librefang-types` change). `--json` for tooling, `--run check,clippy,test` actually invokes scoped cargo commands. (#3296) (@houko)
- Config-driven session mode for agent triggers (`session_mode = "new" | "persistent"`) — per-agent default with per-trigger override
- **Real-client-IP resolution for proxied deployments** via two new top-level config fields, `trusted_proxies` and `trust_forwarded_for`. When both are set and the TCP peer matches the allowlist, the GCRA + auth-login rate limiters and the WebSocket per-IP connection cap key on the IP from forwarding headers (`CF-Connecting-IP` → `X-Real-IP` → `Forwarded` (RFC 7239) → rightmost-untrusted hop in `X-Forwarded-For`) instead of the proxy's own address. Without both flags set, behaviour is unchanged: TCP peer only, headers ignored. Forged forwarding headers from peers outside the allowlist are still ignored, so a rotating `X-Forwarded-For` from the open internet can never bypass per-IP limits. Closes the long-standing TODO referenced in `rate_limiter::resolve_client_ip` (now retired).

### Changed

- **`KernelHandle` role traits gain a typed `KernelOpError`** (#3541 1/N): `EventBus`, `KnowledgeGraph`, and `CronControl` migrated from `Result<_, String>` to `Result<_, librefang_kernel_handle::KernelOpError>`. The new enum has structured variants (`Unavailable { capability }`, `NotFound { kind, id }`, `Invalid { field, reason }`, `Serialize`, `Other(String)`) so callers can match on the cause instead of substring-grepping the formatted message. The catch-all `Other(String)` and `From<String>`/`From<&str>` impls keep the migration window cheap — un-classified kernel sites can opt in incrementally. The api workflow handler now maps `Unavailable` to HTTP 503 directly off the variant. Existing `Arc<dyn KernelHandle>` callers keep compiling unchanged via `KernelHandle`'s blanket impl. (#3541 1/N) (@houko)
- **`TaskQueue` role trait migrated to `KernelOpError`** (#3541 2/N): all 8 methods (`task_post`, `task_claim`, `task_complete`, `task_list`, `task_delete`, `task_retry`, `task_get`, `task_update_status`) now return `Result<_, KernelOpError>`. The kernel impl emits `NotFound { kind: "agent", id }` when the agent UUID-or-name lookup fails — the substring-grep that the historical `String` error required is gone. `crates/librefang-api/src/routes/task_queue.rs` introduces a small `map_kernel_op_err` helper that maps `NotFound`→404, `Invalid`→400, `Unavailable`→503, fallback→500, replacing the unconditional `ApiErrorResponse::internal` at all 8 call sites. (#3541 2/N) (@houko)
- **`MemoryAccess` role trait migrated to `KernelOpError`** (#3541 3/N): `memory_store`, `memory_recall`, `memory_list` flipped from `Result<_, String>` to `Result<_, KernelOpError>`. Test stubs and call sites in `librefang-runtime/src/tool_runner.rs`, `librefang-runtime-wasm/src/host_functions.rs`, and the kernel-handle test fixtures all moved over. Callers in tool_runner / host_functions bridge to their surrounding `Result<_, String>` shells with `.map_err(|e| e.to_string())?` until those wrappers themselves migrate. 3 of 14 role traits typed; `Other(String)` catch-all population keeps shrinking. (#3541 3/N) (@houko)
- **`HandsControl` / `WorkflowRunner` / `GoalControl` migrated to `KernelOpError`** (#3541 4/N): bundles three small role traits — 8 methods total (`hand_list`, `hand_install`, `hand_activate`, `hand_status`, `hand_deactivate`, `run_workflow`, `goal_list_active`, `goal_update`). Default impls now return `KernelOpError::Unavailable { capability: "Hands system" / "Workflow engine" / "Goal system" }` instead of opaque "X not available" strings — callers can branch on the variant directly. `LibreFangKernel::hand_deactivate` emits `Invalid { field: "instance_id" }` for malformed UUIDs; `goal_update` emits `NotFound { kind: "goal", id }` when the goal isn't in the store. 6 of 14 role traits typed. (#3541 4/N) (@houko)
- **`ChannelSender` / `ApprovalGate` / `PromptStore` / `AgentControl` migrated to `KernelOpError`** (#3541 5/N — final): closes the migration. `ChannelSender` (7 methods) covers the channel send + roster surface; `ApprovalGate` (4 methods) covers the approval lifecycle; `PromptStore` (13 methods) covers prompt versions + experiments; `AgentControl` (6 methods — `spawn_agent`, `spawn_agent_checked`, `send_to_agent`, `send_to_agent_as`, `kill_agent`, `run_forked_agent_oneshot`) covers agent lifecycle on the hot path. Default impls return `KernelOpError::Unavailable { capability: "Channel … send" / "Approval system" / "Prompt store" / "run_forked_agent_oneshot" }` so callers branch on the variant. All 12 fallible role traits now use the typed enum (the remaining `A2ARegistry` returns `Option<…>` and `ToolPolicy` has no fallible methods). Substring-grepping at the runtime↔kernel boundary is gone. (#3541 5/N) (@houko)

- **macOS now skips the Keychain by default for the vault master key.** macOS Keychain ACLs are bound to the per-binary code signature, so every fresh `cargo build` invalidates the ACL and triggered an "allow" prompt on every daemon restart. The vault now uses the AES-256-GCM-wrapped file fallback at `~/Library/Application Support/librefang/.keyring` (mode 0600) by default on macOS — equivalent at-rest security in our threat model. Linux and Windows behaviour is unchanged. Override with `[vault] use_os_keyring = true` in `config.toml`, or force-disable on any platform with `LIBREFANG_VAULT_NO_KEYRING=1`. **Existing macOS users**: the daemon does one final Keychain read on first restart after upgrade, mirrors the master key into the file store, and never touches the Keychain again. To clean up the now-unused entry, run `security delete-generic-password -s librefang-vault -a master-key`.
- Default `api_listen` flipped from `0.0.0.0:4545` to `127.0.0.1:4545` (loopback-only). New installs are local-only by default; set `api_listen = "0.0.0.0:4545"` to expose on LAN/remote. Affects `librefang init`, the dashboard's init endpoint, and `librefang.toml.example`. `librefang start` with an explicit `--config <path>` that doesn't exist now prints a clear `librefang init` hint instead of failing obscurely. (#2766)
- **iOS minimum supported version raised from 14.0 to 16.0.** Required by the Tauri 2 mobile toolchain that the new mobile CI builds against. Devices on iOS 14 or 15 (iPhone 6s, original iPhone SE, iPad Air 2 and similar) will no longer be able to install the LibreFang mobile app once mobile bundles ship. Affects only the iOS app — the desktop and Android builds are unchanged. (#3970)

### Security

- **Channel webhook HMAC verification is now mandatory** for Messenger, LINE, Teams, Viber, and DingTalk. Previously, missing signature headers were silently bypassed; they now return `400 Bad Request`, and signature mismatches return `401 Unauthorized`. **Action required if you operate any of these channels:**
  - **Messenger** — set `MESSENGER_APP_SECRET` to your Facebook App Secret (the new `app_secret_env` field in `[channels.messenger]` defaults to this). If unset, signatures are skipped with a startup warning and the endpoint stays unauthenticated — production should always set it.
  - **Teams** — set `TEAMS_SECURITY_TOKEN` to the base64 outgoing-webhook security token from the Teams portal (the new `security_token_env` field in `[channels.teams]`). Same fallback semantics as Messenger.
  - **LINE / Viber / DingTalk** — no new env vars, but probes that don't carry the platform's signature header (curl, monitoring health checks pointed at the webhook path) will now return 4xx instead of 200.
- **Outbound `[channels.webhook] callback_url` is SSRF-guarded.** Adapters refuse to start if the URL resolves to a private (`10/8`, `172.16/12`, `192.168/16`), CGN (`100.64/10`), loopback (`127/8`, `::1`), link-local, multicast, or cloud-metadata range. Catches IPv6 short forms like `[::]`, IPv4-mapped (`[::ffff:127.0.0.1]`), NAT64, and trailing-dot FQDNs. **Action required**: local dev setups using `callback_url = "http://127.0.0.1/..."` must switch to a public tunnel (ngrok, cloudflared) or omit `callback_url`. (#3942)
- **BREAKING**: `require_auth_for_reads` now defaults to *enabled* whenever any form of authentication is configured (`api_key`, `user_api_keys`, or dashboard credentials). Previously the flag had to be set explicitly, leaving read endpoints open even on instances with an `api_key`. Operators who deliberately want open reads on an authenticated instance (e.g. behind a trusted reverse proxy) must now set `require_auth_for_reads = false` in `config.toml`. A boot-time INFO log records when the flag is auto-enabled. (#2448)

### Maintenance

- Wire `cargo xtask integration-test` into CI as a `live-integration-smoke` job — spawns a real `target/debug/librefang start` daemon on every PR touching Rust or CI files, hits `/api/health`, `/api/agents`, `/api/budget`, `/api/network/status`, and SIGTERMs. Catches the failure modes the in-process integration tests miss (route not registered in `server.rs`, daemon failing to bind, config fields not deserializing). Runs with `--skip-llm` to keep the gate hermetic; the live-LLM branch is reserved for the release/nightly workflow that has provider keys. (#3405) (@houko)

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
