# Changelog

All notable changes to LibreFang will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project uses [Calendar Versioning](https://calver.org/) (YYYY.M.DD).

## [Unreleased]

### Added

- **cli: restore `librefang channel` subcommand as a sidecar-only group** — #5463 deleted the entire `Commands::Channel(ChannelCommands)` group on the same "everything was broken anyway" theory that took out `ChannelsPage.tsx`. Same wrong call. Restored as four sidecar-aware subcommands driving the surviving daemon endpoints: (@houko)
  - `librefang channel list` → `GET /api/channels` → table (NAME, KIND, CONFIGURED, TOKEN, 24H MSGS). (@houko)
  - `librefang channel reload` → `POST /api/channels/reload` → hot-reload `[[sidecar_channels]]` from disk without a daemon restart. (@houko)
  - `librefang channel setup [<NAME>]` → schema-driven prompt against `GET /api/channels` (consumes the per-row `fields[]` describe schema); non-secret fields pre-fill from `f.value`, secret fields with `has_value=true` render `(set — leave blank to keep)` so an empty submission preserves the stored secret. POSTs to `POST /api/channels/sidecar/{name}/configure` and shows `restart_required` / `shadowed_secrets_warning` from the response. Without `<NAME>` it shows an interactive picker over unconfigured rows. (@houko)
  - `librefang channel rm <NAME>` → strips the matching `[[sidecar_channels]]` entry from `~/.librefang/config.toml` via `toml_edit`, then best-effort hot-reload (warns + continues if no daemon is running, so the change still applies on next start). (@houko)
  Five `clap`-parse `assert!(matches!(...))` tests added covering `list`, `setup`, `setup <name>`, `reload`, `rm <name>`. The pre-#5463 `test` / `enable` / `disable` arms are **not** restored — sidecars surface their own health via stdout logs (no in-band `/test` endpoint), and the presence of the `[[sidecar_channels]]` block is the only on/off signal (`rm` is the replacement). (@houko)

### Changed

- **ci: run the Coverage workflow on `push: main` only, not on every PR** — `coverage.yml` ran the full instrumented `cargo llvm-cov nextest --workspace` (~20-30 min) on every non-docs PR to produce a report-only LCOV artifact that gates no merge — and a *full* workspace run, where the per-PR `test-unit` / `test-ubuntu` lanes are selective, so for a typical small PR it was the single most expensive job in the pipeline. Dropped the `pull_request` trigger; coverage now runs on `push: main` (keeping the `paths-ignore` skip for non-runtime diffs) plus `workflow_dispatch` for ad-hoc runs. The main-branch series is all a report-only trend metric needs. (@houko)
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

### Fixed

- **CI: repair three pre-existing `[Unreleased]`/main breakers inherited by every open PR.** Three checks were red on `main` itself (and therefore on every PR that merged it), unrelated to the PR contents: (1) `CHANGELOG Attribution (full [Unreleased])` failed because the `cli: restore librefang channel` entry carried its `(@houko)` only on the trailing continuation line, while `--all-unreleased` requires `(@user)` on every bullet incl. the four nested sub-command bullets — each now suffixed. (2) Dashboard `build` (vitest) failed 9 `ProvidersPage.test.tsx` cases with `No "useCredentialPools" export is defined on the "../lib/queries/providers" mock` — `ProvidersPage.tsx` calls `useCredentialPools()` (CredentialPoolsSection, #5459 era) but the test's `vi.mock` never stubbed it; added a default `vi.fn(() => ({ data: undefined, isLoading: false, error: null }))` so the section renders its hidden/empty state. (3) `Workspace coverage (llvm-cov)` failed 4 `librefang-migrate::openclaw` tests (`test_full_migration`, `test_json5_full_migration`, `test_json5_channel_extraction`, `test_policy_migration`) which still used `google_chat` as an in-process "witness" channel; #5459 migrated it to a sidecar, so `migrate_channels_from_json` now returns `None` and every channel surfaces as a `report.skipped` `ItemKind::Channel` rather than an import — assertions updated to the all-sidecar contract already pinned by `test_google_chat_channel_records_skipped_after_sidecar_migration`. (@houko)
- **`bluesky` sidecar: reply context survives sidecar restart via on-demand XRPC re-fetch** (closes #5452). `parse_notification` stashed the AT Protocol `{root: {uri, cid}, parent: {uri, cid}}` reply struct in an in-process `_thread_cache` LRU (capacity 200) keyed by notification URI, then `_post_status` looked it up on the outbound. After ANY sidecar restart between the inbound mention and the bot's reply (operator restart, supervisor crash-recovery, container redeploy, etc.) the cache was empty, the lookup returned `None`, and the reply posted as a top-level skeet — visible to every follower's feed instead of staying in the thread. The single-string `cmd.user.librefang_user` carrier added in #5439 only round-tripped the URI; AT Protocol needs 4 fields (`root.uri`, `root.cid`, `parent.uri`, `parent.cid`) which don't fit. Fix: `_post_status` on cache-miss now issues one `app.bsky.feed.getPosts?uris=<uri>` XRPC call to re-fetch the post's cid AND its existing `record.reply` chain, reconstructs the `{root, parent}` struct using the exact same root-resolution rules `_compute_reply_ref` uses on the inbound path (post-with-reply → keep its `record.reply.root`; top-level post → `root == parent`), re-populates `_thread_cache` so subsequent chunks in the same `_post_status` call don't re-fetch, and uses the recovered ref. Recovery failure (post deleted, instance unreachable, auth still bad after caller's refresh) emits a WARN naming the URI and falls through to a top-level skeet — same degradation as pre-fix but with operator visibility instead of silent mis-threading. `_recover_reply_ref` rejects non-`at://`-prefixed values up-front so a misrouted `librefang_user` from another channel (dingtalk sessionWebhook URL, telegram `@username`) never reaches bsky's XRPC. Cost: one extra XRPC round-trip per cache-miss reply (post-restart, post-eviction). Verification: `cd sdk/python && pytest tests/test_bluesky_adapter.py` — **49 passed** (was 44; +5 regression guards: recovery happy-path, deep-thread root preservation, multi-chunk re-uses recovered ref without re-fetching, recovery-failure degrades-to-unthreaded WARN, cross-channel garbage rejection). Pre-existing cold-cache test (`test_post_status_cold_cache_falls_back_to_unthreaded`) renamed to `test_post_status_cold_cache_recovery_failure_falls_back_to_unthreaded` and updated to assert the getPosts re-fetch fires + 404s + then degrades — the renamed assertion shape pins the recovery path is actually attempted, not bypassed. **Not addressed in this PR** (deliberately scoped): the structural option-3 fix from the issue (widen `SidecarSendParams` with a `metadata: HashMap<String, Value>` bytewise-round-tripped field that would close this class for bluesky, wechat, wecom, and any future sidecar needing > 1 field of correlation state) is a separate protocol change spanning Rust + Python SDK; this PR is option-1 (narrowest blast radius, bluesky-only). (@houko)
- **Cross-sidecar audit follow-ups: `Retry-After` on 429 for `slack` / `feishu` / `gotify` / `google_chat`; inbound dedupe for `gotify` / `google_chat`; LINE reply-API path now used inside the 55 s freshness window.** Cross-cutting consistency audit across the 27 freshly-migrated sidecar adapters (everything from `ntfy` #5224 through `google_chat` #5459) caught four adapters that landed without 429 handling — Slack's `_post_message` / `_add_reaction` / `_remove_reaction` and Feishu's `_http_json` / Gotify's `_publish` / Google Chat's `_send_text` all routed 429 through the same `status >= 300` arm as 5xx, dropping the chunk and ignoring the `Retry-After` window so the next outbound burst extended the server-side rate-limit. The Slack 3-tuple `_http` helper (`_resp_hdrs` was deliberately stripped) is now a retry-aware wrapper that re-issues the call once with `parse_retry_after(default_secs=RETRY_AFTER_DEFAULT_SECS)` from the shared `librefang.sidecar.common`; `_post_message` / `_add_reaction` / `_remove_reaction` inherit the fix without touching the call sites, so the existing 3-tuple unpack contract is preserved. Feishu's `_http_json` (4-tuple, already exposed headers) gained the same `retry_429=True` once-shot before falling through to its `code != 0` arm. Gotify's `_publish` and Google Chat's `_send_text` factored their POST body into a `_publish_chunk` / `_send_chunk` helper so the 429 retry shares one code path with the original raise-on-non-2xx semantics. Also caught two adapters with missing inbound dedupe — Gotify's WebSocket can replay buffered frames on reconnect and Google Chat's webhook is at-least-once-delivery from Pub/Sub. Both now thread inbound messages through `librefang.sidecar.common.SeenSet` (10 000 / evict 5 000, identical policy to nextcloud / reddit / rocketchat / webex) keyed on `gotify-<id>` and the Google Chat `message.name`. Feishu was a false-positive in the audit — it already has Rust-parity `_EventDedup` (mirrors `feishu.rs:122-125`) used at `_dispatch_event` line 1485. LINE picked up an additional capability fix: the inbound `reply_token` was parsed and stashed in `metadata.reply_token` but `metadata` doesn't round-trip back to the sidecar's `on_send`, so every reply degraded to the push API (quota-charged, rate-limited) even within the LINE-server's ~60 s reply window where the free reply API was available. The token is now carried through `librefang_user` (the field the daemon's bridge round-trips bytewise) with a `linereply:<token>:<event_ts_ms>` shape; a `LINE_REPLY_TOKEN_TTL_SECS = 55.0` window plus a `linereply:` prefix guard (librefang_user is shared across channels — dingtalk stores a sessionWebhook URL, telegram stores an @username — and a misrouted value must not be fed to LINE's reply endpoint) decides between reply and push at send time, with automatic push-fallback if LINE rejects the reply call (the most common case being a token already burned between dispatch and the agent's wakeup). Image+caption sends stay on push regardless (the reply token is one-shot-locked on first acceptance and a follow-up caption would error). Verification: 17 new pytest cases (3 × slack-429 + 3 × feishu-429 + 3 × gotify-429-and-dedupe + 3 × google-chat-429-and-dedupe + 5 × line-reply) across `sdk/python/tests/test_{slack,feishu,gotify,google_chat,line}_adapter.py`; full `pytest tests/` — 1845 passed (was 1828). `cargo check --workspace --lib` + `cargo clippy --workspace --all-targets -- -D warnings` clean. **Audited but not changed (false-positives or out-of-scope)**: `feishu` already had Rust-parity dedup via `_EventDedup`; `runtime.py:246` bare-except on `on_command` is OPEN PR #5450 territory (not duplicated here to avoid conflict); `runtime.py:219` producer bare-except is the same class but would conflict with #5450 in the same file — deferred to land after #5450 merges; `mastodon` SSE+poll dual-flow dedupe is theoretical (since_id watermark already covers normal operation); long-lived SSE timeouts are by design. (@houko)
- **`librefang-api` test build repaired after #5455 (webhook → sidecar).** #5455 folded `write_service_account_env` into the generic `write_secret_env` (identical newline-rejection contract) but left `routes/skills.rs`'s unit test calling the removed name, so the `librefang-api` test target failed to compile (`E0425`) and `main` was red again immediately after #5456. The test now calls `write_secret_env` (renamed `write_secret_env_value_with_newline_is_rejected`); same assertions. (@houko)
- **CI on `main` restored — build break + `librefang-migrate` test drift from the wecom/teams/wechat/feishu/whatsapp sidecar merges.** `main` (HEAD `ff3f673`) did not compile and its migrate tests were red, so every open PR's Rust CI was failing on inherited breakage. Compile fixes: (1) `librefang-types` referenced `default_local_probe_interval_secs` from `local_probe_interval_secs`'s `#[serde(default)]` and the `Default` impl but the function was never defined (`E0425`) — added it returning 60 s per the field doc; (2) removed the orphaned `default_channel_max_backoff_secs` / `default_channel_initial_backoff_2s` (no `serde(default=)` consumers, `dead_code` under `-D warnings`); (3) `librefang-migrate::openclaw` — renamed the now-unused `ch` YAML-parse binding to `_ch` (every channel arm is a sidecar skip; the parse only keeps `LegacyYamlChannelConfig` referenced) and removed the orphaned `allow_from_to_toml_array`; (4) `librefang-api::openapi` — dropped the utoipa `paths(...)` refs to the removed `whatsapp_qr_*` / `wechat_qr_*` routes (`E0433`); (5) `librefang-cli` — removed the orphaned `maybe_write_channel_config` / `notify_daemon_restart` helpers (their in-process channel-onboarding callers were dropped). Test-fixture drift (only `google_chat` / `webhook` remain in-process channels): `test_roundtrip_migrate_output_into_real_structs`, `test_json5_channel_extraction`, and `test_full_migration` now use `google_chat` as the in-process witness instead of WhatsApp (asserting WhatsApp as a skipped sidecar); `test_json5_full_migration` / `test_secrets_migration` lower the secret-count floors `7 → 5` and flip the stale `FEISHU_APP_SECRET` extraction assertion to *absent* (the feishu sidecar-skip no longer extracts its secret — note Mattermost still does on skip, an intentional per-channel asymmetry flagged in-test); and the `openfang` `deny_unknown_fields` drift test drops the removed flat `command` field from its `[[mcp_servers]]` fixture so the intended `nickname` typo is what gets rejected. Verified locally: `cargo check --workspace --all-targets` clean, `cargo test -p librefang-migrate` 56 passed, `cargo test -p librefang-memory` 246 passed. (@houko)
- **Telegram sidecar reconnect loop: cap aligned with siblings, recovery now logged, regression coverage added** (closes #5111). #5111 was filed against `v2026.5.12-beta.11`, when telegram was the in-process Rust `librefang-channels::telegram` adapter — that adapter exited its polling task on a DNS resolution / transient network failure and the bridge stayed dead until the daemon was restarted, exactly as the issue describes. The #5241 sidecar migration replaced it with the Python `sdk/python/librefang/sidecar/adapters/telegram.py`, which already wraps `_poll_once` in a `while True / except Exception → backoff` loop and so silently fixes the reported "bridge stays dead" symptom. **This PR is not the bug fix** (that was #5241); it is the observability + cap-alignment + regression coverage that should have shipped alongside #5241 so a future refactor can't re-introduce the original failure mode invisibly: (1) backoff cap moved from a hardcoded `120.0` to a new `MAX_BACKOFF_SECS = 60.0` module constant, matching the convention every sibling polling sidecar (`bluesky`, `discord`, `line`, `mastodon`, `mattermost`, `nextcloud`, `ntfy`, `reddit`, `rocketchat`, `twitch`) already settled on — behavioural change, persistent-outage retries now cap at one-minute intervals instead of two-minute ones; (2) the WARN line on each backoff now reports `retries=<consecutive-failure-count>` alongside `error` and `delay`, so operators can read "how long have we been degraded" off a single log line; (3) on the first successful poll after at least one retry the loop emits an INFO `telegram poll recovered retries=N last_backoff=…` — closes the issue's "restored DNS — bridge does NOT recover" symptom not by changing recovery (it already worked) but by making the recovery visible in the operator's log timeline; (4) `TimeoutError` (LONGPOLL_SERVER_SECS server-side block expiring with no updates — normal protocol behaviour) explicitly resets both `backoff` and `retries_in_a_row` and `continue`s without consuming the sleep budget, so an idle channel never accidentally drifts toward MAX. **Deliberately NOT done** per a deviation from the issue's suggestion of "ERROR only after N consecutive failures": every sibling polling sidecar reserves `log.error` for fatal startup-config issues (`{discord,line,mattermost}_required env vars missing`) — none of them escalate during steady-state backoff. The producer-crash path in `librefang.sidecar.runtime` already emits `log.error("producer crashed", …)` if an exception ESCAPES `produce()`; the backoff loop is precisely the layer that prevents that escape. Adding ERROR escalation here in telegram alone would diverge from the family, and the new WARN-with-retry-counter + INFO-on-recovery already provide the "how long degraded / when restored" signals an operator needs. Three new pytest cases assert the contract end-to-end: `test_produce_recovers_after_startup_network_failure` (URLError on first poll → warn + sleep + retry → success + INFO recovered), `test_produce_backoff_is_capped_at_max` (consecutive failures produce delays = `[1, 2, 4, 8, 16, 32, 60, 60, …]`, max never exceeds `MAX_BACKOFF_SECS`, the cap is actually reached), `test_produce_treats_longpoll_timeout_as_normal` (TimeoutError alternating with success → loop re-enters without sleeping, no backoff growth). All three monkeypatch `tg.asyncio.sleep` against a saved real-sleep closure to avoid the infinite-recursion footgun (`tg.asyncio` and the test file's `asyncio` import point at the same module object). (@houko)
- **CI on `main` restored (post-mattermost-sidecar) — `librefang-migrate::openclaw::tests` test-fixture drift after #5315** (closes #5316). The mattermost sidecar migration converted `mattermost` from in-process to sidecar in the production code paths (both YAML `parse_legacy_channels` and JSON5 `migrate_channels_from_json` now push a `SkippedItem` with a sidecar reason instead of emitting `[channels.mattermost]`), but four `openclaw::tests` cases were not updated in lock-step and used `mattermost` as their only in-process channel witness — so on main the `migrate_channels_from_json` return became `None`, no `ItemKind::Channel` ever landed in `report.imported`, and the JSON5 full-migration imported-count dropped from 7 to 6: (1) `create_legacy_yaml_workspace` only emitted `messaging/{telegram,discord,slack,mattermost}.yaml` — all four now sidecar-skipped — so `test_full_migration` 's `report.imported.iter().any(|i| i.kind == ItemKind::Channel)` asserted against an empty channel-imports vector and panicked at `openclaw.rs:4526`; fixture now also writes `messaging/whatsapp.yaml` (in-process), `test_scan_workspace` 's `channels.len() == 4` updated to `5` with a `whatsapp` membership assert. (2) `test_json5_channel_extraction` 's inline JSON5 (telegram/discord/slack/mattermost-only) made `channels.is_some()` false at `openclaw.rs:4054`; fixture now includes `whatsapp: { dmPolicy: "open", allowFrom: ["phone1"] }`, the `!ch_table.contains_key` / `report.skipped` checks add `mattermost`, the in-process-witness assertion flips from `mattermost` to `whatsapp`, the imported-count stays at 1, the 5-secrets assertion is preserved (mattermost token still flows into `MATTERMOST_TOKEN` via the sidecar-skipped path at `openclaw.rs:1920`), and a `MATTERMOST_TOKEN=mm-token` secrets.env assertion is added so the secret-extraction-on-sidecar-skip behaviour is explicitly covered. (3) `test_json5_full_migration` 's `channel_items.len() == 7` failed; assertion updated to `5` after the rebase (signal also migrated to sidecar in #5317, so the in-process count is whatsapp, matrix, feishu, google_chat, msteams = 5) and the count-comment rewritten to enumerate the 8 skips (telegram, discord, slack, signal, irc, mattermost, imessage, bluebubbles). (4) `test_policy_migration` used `mattermost` for the `dmPolicy: "disabled"` → `dm_policy = "ignore"` happy-path mapping; replaced with `matrix` (still in-process, also accepts `dm_policy`), `mattermost` + `signal` added to the sidecar-skip loop alongside discord/slack, and the comment chain is updated to record the witness-rotation history (discord → slack → mattermost → signal → matrix). All 36 `openclaw::tests` + 51 `librefang-migrate` lib tests + 6 `tests/idempotency.rs` integration tests + `cargo clippy -p librefang-migrate --all-targets -- -D warnings` are green locally. (@houko)
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

