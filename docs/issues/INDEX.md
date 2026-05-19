# LibreFang Audit — Issue Index

**119 tracking items after two rounds of thematic consolidation.** Two independent audits against `/Volumes/Lexar/Workspace/oss/librefang/librefang` @ `087a0481`, each running 10 parallel review agents on 2026-05-18.

Pipeline:
- **Pass 1**: 136 findings produced by per-domain agents.
- **Pass 2**: 86 additional findings from a second sweep.
- **Dedups**: 9 exact duplicates removed across both passes.
- **Round 1 consolidation**: 10 clusters folded 34 sub-findings into 10 representative issues (function-level and cross-file thematic merges).
- **Round 2 consolidation**: 23 clusters folded 65 sub-findings into 23 representative issues (same-domain hygiene roundups).

Filename convention: `{slug}.md`. Severity is captured in each file's `**Severity:**` line.

## Rollup

| Severity | Active |
|---|---|
| Critical | 7 |
| High | 37 |
| Medium | 54 |
| Low | 22 |
| **Total** | **120** |

## Critical

- [ssrf-attachment-urls](ssrf-attachment-urls.md) — SSRF via attachment URLs in `POST /api/agents/{id}/message`
- [skill-install-path-traversal](skill-install-path-traversal.md) — Path traversal in `POST /api/skills/install`
- [state-secret-default-random](state-secret-default-random.md) — `LIBREFANG_STATE_SECRET` defaults to per-process random key
- [api-error-generic-missing-fluent-key](api-error-generic-missing-fluent-key.md) — 41 endpoints return `"api-error-generic"` literal
- [list-sessions-decode-on-poll](list-sessions-decode-on-poll.md) — Dashboard 5s `list_sessions()` decodes full rmp blobs
- [audit-export-401](audit-export-401.md) — Audit export silently 401 since #3620
- [agents-mutation-routes-untested](agents-mutation-routes-untested.md) — ~30 `/api/agents/*` mutation routes have no semantic test

## High

### Auth & secrets
- [auth-callback-no-rate-limit](auth-callback-no-rate-limit.md) — `/api/auth/callback` not in auth rate-limit allowlist
- [dashboard-login-logs-phc-hash](dashboard-login-logs-phc-hash.md) — `dashboard_login` logs Argon2id PHC on legacy upgrade
- [github-copilot-oauth-unauthenticated](github-copilot-oauth-unauthenticated.md) — `/api/providers/github-copilot/oauth/*` unauthenticated
- [write-secret-env-toctou](write-secret-env-toctou.md) — `write_secret_env` TOCTOU before chmod 0600

### API attack surface
- [migrate-arbitrary-paths](migrate-arbitrary-paths.md) — `POST /api/migrate` accepts arbitrary paths
- [install-deps-rce-admin](install-deps-rce-admin.md) — `POST /api/hands/{id}/install-deps` RCE-for-Admin
- [webhook-create-no-ssrf-check](webhook-create-no-ssrf-check.md) — Webhook create/update lacks SSRF blocklist
- [require-auth-for-reads-false-leak](require-auth-for-reads-false-leak.md) — `require_auth_for_reads = Some(false)` leaks on non-loopback

### Error handling
- [agent-not-found-returns-500](agent-not-found-returns-500.md) — Session/model handlers return 500 on `AgentNotFound`
- [rusqlite-errors-leak](rusqlite-errors-leak.md) — Raw `rusqlite` errors propagate to clients
- [audit-export-malformed-json](audit-export-malformed-json.md) — `audit_export` can produce malformed JSON

### Performance
- [memory-recall-n+1-update](memory-recall-n+1-update.md) — N+1 UPDATE in memory recall
- [dashboard-snapshot-no-cache](dashboard-snapshot-no-cache.md) — `dashboard_snapshot_inner` re-enriches every 5s
- [tool-calls-deque-unbounded](tool-calls-deque-unbounded.md) — `record_tool_calls` deque grows unbounded when limit disabled
- [blocking-fs-on-executor](blocking-fs-on-executor.md) — Blocking `std::fs` on axum executor
- [budget-ranking-deep-clone](budget-ranking-deep-clone.md) — `agent_budget_ranking` deep-clones every `AgentEntry`

### Architecture
- [kernel-depends-on-extensions](kernel-depends-on-extensions.md) — Kernel depends on extensions (contradicts CLAUDE.md)
- [openapi-paths-incomplete](openapi-paths-incomplete.md) — OpenAPI `paths(...)` missing 76+ handlers
- [config-reload-coverage](config-reload-coverage.md) — `build_reload_plan` covers 40/100 fields

### Test coverage
- [integration-tests-mock-router](integration-tests-mock-router.md) — 32/42 integration tests use mock router
- [trigger-concurrency-no-e2e](trigger-concurrency-no-e2e.md) — Trigger concurrency caps lack end-to-end test
- [prompt-determinism-test-gap](prompt-determinism-test-gap.md) — Only OpenAI has prompt-determinism test

### CI / hooks
- [rustfmt-loses-spaced-paths](rustfmt-loses-spaced-paths.md) — `pre-commit` rustfmt loses files with spaces

### Dashboard
- [rel-noopener-mixed](rel-noopener-mixed.md) — Mixed `rel` on `target="_blank"`
- [session-sse-withcredentials](session-sse-withcredentials.md) — SSE uses `withCredentials` with Bearer auth

### LLM driver & MCP
- [driver-blocking-fs-read](driver-blocking-fs-read.md) — Blocking `std::fs::read` in driver image paths
- [pooled-driver-no-invalidate](pooled-driver-no-invalidate.md) — `PooledDriver` doesn't invalidate rate-limited key

### Secrets (Pass 2)
- [oauth-refresh-error-body-token-leak](oauth-refresh-error-body-token-leak.md) — OAuth refresh endpoint logs full token-endpoint response body
- [oauth-tokens-derive-debug-serialize](oauth-tokens-derive-debug-serialize.md) — `OAuthTokens` derives `Debug`+`Serialize` over raw `String` tokens

### Sandbox & concurrency isolation (Pass 2)
- [shell-meta-double-quote-bypass](shell-meta-double-quote-bypass.md) — Shell-metacharacter denylist bypassed by `"$(...)"` / backticks in double quotes
- [channel-bridge-bypasses-lane-semaphore](channel-bridge-bypasses-lane-semaphore.md) — Channel bridge dispatch bypasses `Lane::Trigger`, spawn rate unbounded

### Data integrity (Pass 2)
- [sqlite-file-permissions](sqlite-file-permissions.md) — SQLite DB files created world-readable (no `chmod 0600`)
- [agent-cascade-delete-missing-tables](agent-cascade-delete-missing-tables.md) — Agent delete cascade misses 8 `agent_id`-keyed tables (paired_devices replay)

### DoS (Pass 2)
- [upload-route-bypasses-body-limit](upload-route-bypasses-body-limit.md) — Upload route mounted before `RequestBodyLimitLayer`, buffers full body to RAM
- [trigger-engine-no-per-agent-cap](trigger-engine-no-per-agent-cap.md) — `TriggerEngine::register` lacks per-agent cap

### Input validation (Pass 2)
- [comms-send-impersonation](comms-send-impersonation.md) — `/api/comms/send` trusts caller-supplied `from_agent_id` (impersonation)

## Medium

### Auth & secrets
- [set-provider-key-arbitrary-names](set-provider-key-arbitrary-names.md) — `set_provider_key` accepts arbitrary names
- [mcp-callback-empty-code](mcp-callback-empty-code.md) — MCP callback doesn't reject empty `code`
- [sessions-json-plaintext](sessions-json-plaintext.md) — `sessions.json` plaintext at rest
- [logout-no-secure-cookie](logout-no-secure-cookie.md) — `dashboard_logout` no `Secure` over plain HTTP
- [caller-fingerprint-anon-constant](caller-fingerprint-anon-constant.md) — `caller_fingerprint(&None)` is constant

### API attack surface
- [react-asset-spa-fallback-phish](react-asset-spa-fallback-phish.md) — `react_asset` SPA fallback amplifies phishing

### Concurrency
- [cost-reservation-not-atomic](cost-reservation-not-atomic.md) — `CostReservationLedger` not atomic
- [trigger-dispatch-two-snapshots](trigger-dispatch-two-snapshots.md) — Trigger dispatch reads manifest twice
- [session-mode-override-clamped](session-mode-override-clamped.md) — Per-trigger `New` clamped by manifest

### Error handling
- [spawn-connect-mcp-swallows-panic](spawn-connect-mcp-swallows-panic.md) — `tokio::spawn(connect_mcp_servers)` swallows panics

### Performance
- [file-read-tracker-leak](file-read-tracker-leak.md) — `file_read_tracker` never reclaims dead sessions

### Architecture
- [hot-reload-docs-scattered](hot-reload-docs-scattered.md) — Hot-reload semantics scattered across docs
- [install-integration-extension-result](install-integration-extension-result.md) — `install_integration` leaks `ExtensionResult`

### Test coverage
- [trigger-test-fixed-sleep](trigger-test-fixed-sleep.md) — Trigger test uses 150ms fixed sleep

### LLM driver & MCP
- [roundrobin-index-desync](roundrobin-index-desync.md) — RoundRobin index desync after hot-reload
- [drivercache-defaulthasher](drivercache-defaulthasher.md) — `DriverCache::cache_key` collision risk
- [parse-tool-args-brace-counter](parse-tool-args-brace-counter.md) — `parse_tool_args` mishandles JSON escapes

### Dashboard
- [notification-menu-no-keyboard](notification-menu-no-keyboard.md) — NotificationCenter no keyboard nav
- [i18n-escapeValue-false](i18n-escapeValue-false.md) — i18n `escapeValue: false` + `dangerouslySetInnerHTML`

### CI / hooks
- [guard-bash-safety-missing-lib](guard-bash-safety-missing-lib.md) — `guard-bash-safety.sh` silent on missing lib

### Auth & secrets (Pass 2)
- [x-forwarded-proto-trusted-proxies](x-forwarded-proto-trusted-proxies.md) — `X-Forwarded-Proto` honored without trusted-proxy check → `Secure` cookie downgrade
- [login-prefix-match](login-prefix-match.md) — `/api/auth/login` allowlist prefix without trailing slash
- [driverconfig-api-key-serialize](driverconfig-api-key-serialize.md) — `DriverConfig::api_key` Serialize-derives plaintext
- [wechat-bot-token-prefix-debug-log](wechat-bot-token-prefix-debug-log.md) — WeChat bot-token prefix (10 chars) logged at `debug!`

### Sandbox (Pass 2)
- [docker-network-cap-add-allowlist](docker-network-cap-add-allowlist.md) — Docker `--network` / `--cap-add` not allowlisted
- [plugin-archive-sha-per-hook](plugin-archive-sha-per-hook.md) — Plugin install: integrity SHA per-hook, not per-archive

### Concurrency (Pass 2)
- [cron-prune-lock-across-llm-await](cron-prune-lock-across-llm-await.md) — Cron prune guard held across LLM `try_summarize_trim().await`
- [trace-store-blocking-mutex-on-tokio](trace-store-blocking-mutex-on-tokio.md) — `TraceStore::insert` holds std mutex across SQLite I/O on tokio worker
- [workflow-path-drops-lane-permit](workflow-path-drops-lane-permit.md) — Workflow path drops `Lane::Trigger` permit before spawn

### Panic / error handling (Pass 2)
- [metering-token-overflow](metering-token-overflow.md) — Metering token-count overflow on adversarial LLM response (`cache_read + cache_creation`)
- [workspace-setup-write-all-swallow](workspace-setup-write-all-swallow.md) — Agent identity bootstrap silently swallows `write_all` failures

### Data integrity (Pass 2)
- [cleanup-orphan-sessions-format-sql](cleanup-orphan-sessions-format-sql.md) — `cleanup_orphan_sessions` uses `format!` instead of parameter binding
- [prompt-store-second-pool-no-fk](prompt-store-second-pool-no-fk.md) — `PromptStore::new_with_path` opens second pool without `foreign_keys=ON`
- [migration-ladder-partial-upgrade-hazard](migration-ladder-partial-upgrade-hazard.md) — Migration ladder commits per step → partial upgrade hazard
- [sessions-missing-index](sessions-missing-index.md) — `sessions.agent_id` lacks independent index; hot paths scan
- [json-text-silent-parse-fallback](json-text-silent-parse-fallback.md) — JSON-in-TEXT columns silently swallow parse failures

### DoS (Pass 2)
- [regex-cache-unbounded](regex-cache-unbounded.md) — Router regex compile cache unbounded
- [pii-filter-regex-no-size-cap](pii-filter-regex-no-size-cap.md) — `PiiFilter::new` compiles operator regex without `size_limit`
- [channel-rate-limiter-buckets-unbounded](channel-rate-limiter-buckets-unbounded.md) — Channel rate-limiter `buckets` DashMap entries never evicted
- [active-sessions-unbounded](active-sessions-unbounded.md) — `active_sessions` HashMap pruned only on WS upgrade

### Supply chain (Pass 2)
- [serde-yaml-unmaintained](serde-yaml-unmaintained.md) — `serde_yaml 0.9.34+deprecated` (RUSTSEC-2024-0320, archived)
- [imap-2-old-nom-base64](imap-2-old-nom-base64.md) — `imap 2.x` drags in `nom 5.1.3` + `base64 0.13.1`
- [whatsapp-gateway-set-var-bypass-lock](whatsapp-gateway-set-var-bypass-lock.md) — `whatsapp_gateway.rs` writes env var bypassing `secrets_env::ENV_WRITE_LOCK`

### Input validation (Pass 2)
- [message-byte-vs-char-cap](message-byte-vs-char-cap.md) — `MAX_MESSAGE_SIZE` is byte length → CJK users rejected at ~1/3 char budget
- [wire-message-other-variant-silent](wire-message-other-variant-silent.md) — `WireMessage*` `#[serde(other)]` drops unknown variants silently
- [session-mode-deserialize-fallback](session-mode-deserialize-fallback.md) — `SessionMode` deserialization fallback hides typo
- [agent-list-limit-none-unbounded](agent-list-limit-none-unbounded.md) — `AgentListQuery` `limit=None` returns full unpaginated list
- [prompt-version-system-prompt-no-cap](prompt-version-system-prompt-no-cap.md) — `PromptVersion` accepts unbounded `system_prompt`, client-supplied `is_active`
- [check-json-depth-unused](check-json-depth-unused.md) — `check_json_depth` defined but never called from any route

### Kernel orchestration (Pass 2)
- [warn-missed-fires-only-one-catchup](warn-missed-fires-only-one-catchup.md) — `warn_missed_fires` logs N but fires only 1 catch-up
- [trigger-new-session-non-deterministic](trigger-new-session-non-deterministic.md) — Trigger `New`-mode SessionId is random UUID, no log correlation
- [safe-trim-messages-session-copy-no-repair](safe-trim-messages-session-copy-no-repair.md) — `safe_trim_messages` skips repair on `session_messages`
- [cron-channel-name-not-reserved](cron-channel-name-not-reserved.md) — `cron`/`autonomous`/`webui` reserved names not validated at channel ingress
- [workflow-skip-per-agent-semaphore](workflow-skip-per-agent-semaphore.md) — Workflow path skips per-agent semaphore unconditionally

## Low

### Auth & secrets
- [oidc-callback-email-info-log](oidc-callback-email-info-log.md) — OIDC callback logs email at INFO

### API attack surface
- [registry-content-abs-path-leak](registry-content-abs-path-leak.md) — Registry content response leaks home dir

### Concurrency
- [agent-concurrency-get-then-insert](agent-concurrency-get-then-insert.md) — `agent_concurrency_for` get-then-insert
- [mqtt-task-not-tracked](mqtt-task-not-tracked.md) — process-manager readers detached + `held_locks` `RefCell` (MQTT half resolved by removing the adapter)

### Performance
- [jwks-cache-no-reload-evict](jwks-cache-no-reload-evict.md) — JWKS cache no eviction on config reload
- [extract-text-content-allocs](extract-text-content-allocs.md) — `extract_text_content` allocates per save

### Architecture
- [two-migrate-crates](two-migrate-crates.md) — Two unrelated `*-migrate` crates

### Test coverage
- [dashboard-e2e-thin](dashboard-e2e-thin.md) — Dashboard E2E 56-line single file

### LLM driver & MCP
- [mcp-args-no-schema-check](mcp-args-no-schema-check.md) — MCP tool args not validated against schema

### Dashboard
- [data-layer-rule-clean](data-layer-rule-clean.md) — Data layer rule upheld (baseline)

### CI / hooks
- [commit-msg-attribution-regex](commit-msg-attribution-regex.md) — `commit-msg` regex misses zero-space variant

### Auth & secrets (Pass 2)
- [react-asset-path-traversal](react-asset-path-traversal.md) — `webchat::react_asset` path-traversal check is substring-only
- [vault-key-env-overrides-keyring](vault-key-env-overrides-keyring.md) — `LIBREFANG_VAULT_KEY` env silently overrides OS keyring
- [provider-api-keys-no-boot-validation](provider-api-keys-no-boot-validation.md) — `provider_api_keys` env vars not validated at boot

### Sandbox (Pass 2)
- [docker-container-name-collisions](docker-container-name-collisions.md) — Docker container-name sanitization collides agent IDs

### Concurrency (Pass 2)
- [publish-event-depth-and-held-locks](publish-event-depth-and-held-locks.md) — Trigger spawn lacks `held_agent_locks` re-entrance contract test

### Panic / error handling (Pass 2)
- [openai-compat-token-add-overflow](openai-compat-token-add-overflow.md) — `openai_compat` `total_tokens` addition not `saturating`

### DoS (Pass 2)
- [bulk-with-capacity-no-validate](bulk-with-capacity-no-validate.md) — Bulk handlers `Vec::with_capacity(req.ids.len())` without `validate_bulk_size`
- [audit-log-cap-only-on-trim-interval](audit-log-cap-only-on-trim-interval.md) — `AuditLog::record` caps only on `trim_interval` (default 1h)

### Supply chain (Pass 2)
- [phf-generator-old-rand](phf-generator-old-rand.md) — `phf_generator 0.8` pins `rand 0.7.3` (ecosystem fragmentation)

### Input validation (Pass 2)
- [comms-send-no-audit-log](comms-send-no-audit-log.md) — `comms_send` cross-agent send not in hash-chained audit log

### Kernel orchestration (Pass 2)
- [silent-marker-substring-match](silent-marker-substring-match.md) — `[SILENT]` marker uses substring match; user paste can trigger drop

---

## Triage order

### Pass 1 priority (unchanged)
1. **api-error-generic-missing-fluent-key** (one-liner per locale, restores diagnostics for 41 endpoints)
2. **ssrf-attachment-urls** + **skill-install-path-traversal** (concrete exploit paths)
3. **state-secret-default-random** (silently breaks multi-replica)
4. **list-sessions-decode-on-poll** + **audit-export-401** (single-line fixes, immediate user impact)
5. **write-secret-env-toctou** + **dashboard-login-logs-phc-hash** (secret hygiene)
6. **openapi-paths-incomplete** + **config-reload-coverage** (reflection tests block whole classes of regressions)

### Pass 2 additions — priority

7. **oauth-refresh-error-body-token-leak** + **oauth-tokens-derive-debug-serialize** — OAuth token plaintext in logs / `Debug`+`Serialize` (broad credential leak)
8. **sqlite-file-permissions** — SQLite DB file permissions (one-line fix, large blast radius on shared hosts)
9. **agent-cascade-delete-missing-tables** — agent cascade-delete misses `paired_devices` (bearer-token replay against deleted agents)
10. **comms-send-impersonation** — `/api/comms/send` agent impersonation (privilege boundary)
11. **shell-meta-double-quote-bypass** — shell-metacharacter double-quote bypass (allowlist mode regression)
12. **channel-bridge-bypasses-lane-semaphore** — channel bridge bypass of `Lane::Trigger` (DoS amplifier)
13. **upload-route-bypasses-body-limit** — upload route body-limit bypass (trivial RAM exhaustion for any authed user)
14. **trigger-engine-no-per-agent-cap** — trigger registration cap missing (DoS at manifest layer)

