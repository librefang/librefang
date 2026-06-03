# Config reload: which fields hot-reload, which need a restart

This is the single canonical, ops-facing answer to: *"I edited
`~/.librefang/config.toml` (or `PUT /api/config`) and called
`POST /api/config/reload` — will my change take effect, or do I need to
restart the daemon?"*

Every `KernelConfig` field is listed below with its reload
classification and a one-line meaning. The table is **transcribed from
`build_reload_plan` / `build_reload_plan_with_caps` in
`crates/librefang-kernel/src/config_reload.rs`** — that function is the
source of truth, and a drift-guard test
(`doc_reload_table_matches_classified_reload_fields` in the same file)
fails the build if a field is added to the planner but not to this doc
(or vice-versa). **When you change a field's classification in
`build_reload_plan`, update this table in the same PR.**

## How `POST /api/config/reload` works

1. The new config is parsed and validated (`validate_config_for_reload`).
2. `build_reload_plan(old, new)` diffs the running config against the new
   one and sorts every changed field into one of three buckets:
   - **RequiresRestart** — the value is captured once at boot (into a
     kernel field, the axum router, a background task, or a cached LLM
     driver) and no hot action rebuilds that consumer. A bare config
     swap would silently no-op, so the planner sets `restart_required`
     and the operator must restart the daemon for the change to land.
   - **HotReload** — the change emits a `HotAction` that re-initialises
     the affected subsystem in place (reconnect channels, resize
     semaphores, flush a cache, RCU a snapshot, …). No restart needed.
   - **Ignore / noop** — the value is read live from `config_ref()` /
     `self.config.load()` on every message or request. The ArcSwap
     config swap makes the edit effective on the next use with no extra
     action; the planner records it as informational only.
3. Hot actions are applied according to the configured `[reload] mode`
   (`off` / `restart` / `hot` / `hybrid`) — see `should_apply_hot`.

When `restart_required` is set, the dashboard / API response says so
explicitly. A field that is `Ignore`/`noop` is **not** a failure — it
just means "already effective on the next message, nothing to reapply".

### Gotcha: per-agent concurrency caps need a respawn, not a reload

`agent.toml: max_concurrent_invocations` is **not** a `KernelConfig`
field and is **not** covered by this table. Its live semaphore is
created lazily per agent and is **not** invalidated on manifest
hot-reload. To pick up a new cap you must kill the agent and let it
respawn (or restart the daemon) — an in-place activate/status flip
silently keeps the old cap. See
[`../architecture/trigger-dispatch-concurrency.md`](../architecture/trigger-dispatch-concurrency.md).

### Conditional: `log_level`

`log_level` is **HotReload** only when the embedding binary installed a
`LogLevelReloader` (the CLI daemon does; embedded callers such as the
desktop server do not). Without the reloader, the planner demotes a
`log_level` change to **RequiresRestart** so the dashboard reports an
honest "needs restart" instead of a false "applied". See
`ReloadCapabilities` in `config_reload.rs`.

## Field classification table

Legend: **R** = RequiresRestart · **H** = HotReload · **N** =
Ignore/noop (effective on next message/request via ArcSwap swap). A field
may carry more than one class (e.g. **R/H**) when its sub-fields are
classified differently — the row note spells out which is which.

### Server / network / bind

| Field | Class | Meaning |
|---|---|---|
| `api_listen` | R | API listen address (the bound socket is fixed at boot). |
| `network_enabled` | R | Master switch for the OFP network layer. |
| `network` | R | Network config (shared secret, listen addresses, …). |
| `cors_origin` | R | Allowed CORS origins (baked into the router at boot). |
| `trusted_hosts` | R | Hostnames allowed to drive the MCP OAuth redirect URI. |
| `trusted_proxies` | R | CIDRs of reverse proxies trusted to set forwarding headers. |
| `trust_forwarded_for` | R | Master switch for forwarding-header trust. |
| `trusted_manifest_signers` | R | Ed25519 public keys allowed to sign agent manifests. |
| `allowed_mount_roots` | R | Host directories under which workspace mounts may resolve. |
| `max_request_body_bytes` | R | Global request-body size cap (router safety net). |
| `max_upload_size_bytes` | R | Maximum upload size in bytes. |
| `rate_limit` | R | API and WebSocket rate-limiting config. |

### Auth / RBAC / dashboard

| Field | Class | Meaning |
|---|---|---|
| `api_key` | N | API bearer key (effective immediately via config swap). |
| `dashboard_user` | H | Dashboard login username (config swap suffices). |
| `dashboard_pass` | H | Dashboard login password. |
| `dashboard_pass_hash` | H | Argon2id hash of the dashboard password. |
| `users` | H | RBAC user list — rebuilds the `AuthManager`. |
| `require_auth_for_reads` | R | Whether the dashboard-reads allowlist requires auth. |
| `external_auth_proxy` | R | Acknowledges an external auth proxy is in front. |
| `channel_role_mapping` | R | Maps platform-native channel roles to LibreFang roles. |
| `external_auth` | H/N | OAuth2/OIDC provider config. **IdP-identity** changes (`enabled`, `issuer_url`, per-provider `id`/`issuer_url`/`jwks_uri` — see `external_auth_idp_changed`) are **H**: they emit `ReloadExternalAuth` to flush the OIDC discovery + JWKS caches, no restart. **Non-IdP** sub-fields (`session_ttl_secs`, `allowed_domains`, `redirect_url`, scopes, audience, `require_email_verified`) are **N**: the OAuth layer reads them live from the ArcSwap config on every request (`oauth.rs`: `config_ref()` / `config_snapshot()`), so a bare config swap makes them effective on the next request — no restart, no cache eviction. |
| `oauth` | R | OAuth client-ID overrides for PKCE flows. |
| `auth_profiles` | R | Per-provider auth profiles for key rotation. |
| `pairing` | N | Device pairing config (read live per request). |

### Model providers / LLM

| Field | Class | Meaning |
|---|---|---|
| `default_model` | H | Default LLM provider/model (new agents pick it up). |
| `fallback_providers` | H | Fallback provider chain tried in order. |
| `credential_pools` | H | Multi-key rotation pools per provider — rebuilt. |
| `provider_urls` | H | Provider base-URL overrides (flushes driver cache). |
| `provider_regions` | H | Provider region selection (same hot action as `provider_urls`). |
| `provider_api_keys` | H | Provider API-key env-var overrides (flushes driver cache). |
| `provider_proxy_urls` | R | Per-provider proxy URL overrides (captured by cached drivers). |
| `provider_request_timeout_secs` | R | Per-provider HTTP request timeout overrides. |
| `provider_max_retries` | R | Per-provider in-driver retry-count overrides (captured by cached drivers at creation). |
| `vertex_ai` | R | Vertex AI provider config. |
| `azure_openai` | R | Azure OpenAI provider config. |
| `llm` | R | `[llm]` section (auxiliary side-task chain config). |
| `qwen_code_path` | N | Override path to the Qwen Code CLI binary. |
| `local_probe_interval_secs` | R | Interval between local-provider reachability probes. |
| `thinking` | N | Extended-thinking config (read live per message). |
| `default_routing` | N | Kernel-wide Smart Model Router defaults. |

### Prompt / caching / context

| Field | Class | Meaning |
|---|---|---|
| `stable_prefix_mode` | N | Avoid volatile prompt-prefix additions (next message). |
| `prompt_caching` | N | Master switch for provider prompt caching. |
| `prompt_cache` | N | Prompt-cache breakpoint strategy. |
| `compaction` | N | LLM-based history summarization config. |
| `gateway_compression` | N | Gateway-level safety-net compression. |
| `context_engine` | R | Pluggable context-engine config. |
| `tool_results` | N | Tool-result context budget + artifact spill config. |
| `max_history_messages` | N | Global message-history trim cap (see arch doc). |
| `agent_max_iterations` | N | Operator override for the agent-loop iteration cap. |
| `max_agent_call_depth` | N | Maximum inter-agent call depth. |

### Tools / execution / approval

| Field | Class | Meaning |
|---|---|---|
| `tool_policy` | H | Global tool deny/allow rules, groups, depth limits. |
| `approval` | H | Execution approval policy. |
| `tool_timeout_secs` | N | Default per-tool execution timeout. |
| `tool_timeouts` | N | Per-tool timeout overrides. |
| `tool_invoke` | N | `POST /api/tools/{name}/invoke` allowlist. |
| `exec_policy` | R | Shell/exec security policy. |
| `tool_exec` | R | Pluggable tool-execution backend selection. |
| `parallel_tools` | R | Parallel-tool dispatcher config. |
| `docker` | R | Docker container sandbox config. |
| `terminal` | R | Terminal / CLI access control (tmux wiring is boot-captured). |

### Channels / triggers / cron / queue

| Field | Class | Meaning |
|---|---|---|
| `channels` | H | In-process channel bridge config — reloads bridges. |
| `sidecar_channels` | H | Sidecar (external-process) channel adapters (same hot action). |
| `webhook_triggers` | H | Webhook trigger (external event injection) config. |
| `max_cron_jobs` | H | Cron scheduler max total jobs across agents. |
| `queue` | H | Message-queue config — resizes the lane semaphores. |
| `triggers` | N | Event-trigger system config (cooldowns, depth limits). |
| `auto_reply` | R | Auto-reply background engine config. |
| `broadcast` | R | Broadcast routing config. |
| `cron_session_max_tokens` | N | Cron session token-prune threshold. |
| `cron_session_max_messages` | N | Cron session message-count prune threshold. |
| `cron_session_warn_fraction` | N | Budget fraction at which a cron-session growth warning fires. |
| `cron_session_warn_total_tokens` | N | Fallback context-window ceiling for the warn. |
| `cron_session_compaction_mode` | N | Cron-session compaction strategy (prune / summarize_trim). |
| `cron_session_compaction_keep_recent` | N | Recent messages preserved verbatim after summarization. |

### Extensions / MCP / A2A / skills

| Field | Class | Meaning |
|---|---|---|
| `extensions` | H | Extensions & integrations config — reloads extensions. |
| `mcp_servers` | H | MCP server list — reconnects MCP clients. |
| `taint_rules` | H | Named taint rule sets pushed into the shared swap (see field note: already-connected servers pick them up on next scan, not via reconnect). |
| `a2a` | H | Agent-to-Agent protocol config. |
| `skills` | H | Skills config (bundled + user-installed) — reloads registry. |
| `plugins` | R | Plugin registry config. |
| `registry` | R | Registry sync config (cache TTL, …). |
| `hands` | N | Hands marketplace SSRF allowlist (`registry_allowed_hosts`) — read live by the install handler per request. |
| `bindings` | R | Agent bindings for multi-account routing. |

### Memory / sessions

| Field | Class | Meaning |
|---|---|---|
| `memory` | R | Memory substrate config (restarts SQLite connections). |
| `memory_wiki` | R | Memory wiki vault (constructed once at boot). |
| `proactive_memory` | H | mem0-style proactive memory config — updated in place. Also pushes `duplicate_threshold` into the background `ConsolidationEngine` so the kernel-wide sweep and the per-agent on-demand consolidate stay in lockstep (audit findings #5839 H5). |
| `auto_dream` | R | Background memory-consolidation config. |
| `rl_export` | R | RL rollout trajectory export config (upstream target + enable toggle). Classified restart so a target / credential redirect takes effect through the documented restart path, not mid-session. |
| `session` | R | Session retention policy. |

### Budget / metering / privacy

| Field | Class | Meaning |
|---|---|---|
| `budget` | H | Global spending budget — RCUs the metering snapshot. |
| `privacy` | N | PII privacy controls for LLM context filtering. |
| `sanitize` | N | Channel-input sanitization / prompt-injection detection. |

### Web / browser / media / canvas / TTS

| Field | Class | Meaning |
|---|---|---|
| `web` | H | Web tools config (search + fetch) — rebuilds web context. |
| `browser` | H | Browser automation config. |
| `media` | N | Media-understanding config. |
| `links` | N | Link-understanding config. |
| `canvas` | R | Canvas (A2UI) config. |
| `tts` | N | Text-to-speech config. |

### Notifications / inbox / observability

| Field | Class | Meaning |
|---|---|---|
| `notification` | N | Notification-engine config for alerts and task state. |
| `usage_footer` | H | Usage footer mode (what to show after each response). |
| `inbox` | R | File-based input inbox config. |
| `audit` | R | Audit log config. |
| `telemetry` | R | OpenTelemetry + Prometheus config. |
| `health_check` | R | Health-check config. |
| `heartbeat` | R | Heartbeat-monitor global defaults. |
| `prompt_intelligence` | R | Prompt-intelligence (versioning + A/B) config. |
| `task_board` | R | Shared task-board safety knobs. |
| `background` | R | Background autonomous-loop executor knobs. |

### Proxy / runtime / paths / misc

| Field | Class | Meaning |
|---|---|---|
| `proxy` | H | HTTP proxy for outbound connections — re-exports env + flushes driver cache. |
| `log_level` | H* | Tracing filter (`*` = HotReload only when a log reloader is installed; otherwise R — see "Conditional" above). |
| `language` | N | CLI/message locale (effective on next message). |
| `mode` | N | Kernel operating mode (stable / default / dev). |
| `home_dir` | R | LibreFang home directory. |
| `data_dir` | R | Database data directory. |
| `log_dir` | R | Custom log directory. |
| `workspaces_dir` | R | Root directory for agent workspaces. |
| `vault` | R | Credential vault config (key derivation). |
| `config_version` | R | Config schema version for migration. |
| `include` | R | Config include files (deep-merged at load). |
| `reload` | R | The `[reload]` config-hot-reload settings block itself. |
| `strict_config` | R | Refuse to start on unknown config fields (tolerant-mode toggle). |
| `update_channel` | R | CLI update channel (stable / beta / rc). |
| `max_concurrent_bg_llm` | R | Max concurrent background LLM calls across agents. |
| `workflow_stale_timeout_minutes` | R | Stale-workflow recovery threshold on boot. |
| `workflow_default_total_timeout_secs` | R | Default wall-clock timeout for a workflow run. |

## See also

- Source of truth: `crates/librefang-kernel/src/config_reload.rs`
  (`build_reload_plan`, `HotAction`, `classified_reload_fields`).
- Message-history trim cap details:
  [`../architecture/message-history-trimming.md`](../architecture/message-history-trimming.md).
- Per-agent concurrency caps (respawn gotcha):
  [`../architecture/trigger-dispatch-concurrency.md`](../architecture/trigger-dispatch-concurrency.md).
