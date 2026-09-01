//! Channel configuration + status handlers.
//!
//! Every channel adapter runs as an out-of-process sidecar. The router
//! exposes 4 endpoints:
//!
//! - `GET /channels` — list configured + discoverable channels
//! - `POST /channels/reload` — manually trigger a channel hot-reload
//! - `GET /channels/registry` — read disk-persisted channel metadata
//! - `POST /channels/sidecar/{name}/configure` — write a sidecar entry
//!
//! The per-channel `/configure` (POST/DELETE), `/instances` (GET/POST),
//! `/instances/{index}` (PUT/DELETE), `/test` (POST), and `/{name}`
//! (GET) endpoints are gone — they all 404'd unconditionally after the
//! in-process channel registry emptied. Restore them alongside any
//! future in-process channel that re-introduces a `ChannelMeta`-style
//! schema.

/// Build routes for the Channel domain.
pub fn router() -> axum::Router<std::sync::Arc<super::AppState>> {
    axum::Router::new()
        .route("/channels", axum::routing::get(list_channels))
        .route("/channels/reload", axum::routing::post(reload_channels))
        // Single read-only QR endpoint that replaces the four removed
        // pre-migration ones (`/{wechat,whatsapp}/qr/{start,status}`).
        // The sidecar drives the QR lifecycle and emits `qr_ready` /
        // `qr_status` events; this handler just reads the cached
        // `ChannelStatus.qr` from `kernel.channel_adapters_ref()`.
        .route(
            "/channels/{name}/qr",
            axum::routing::get(get_channel_qr),
        )
        .route(
            "/channels/registry",
            axum::routing::get(list_channel_registry),
        )
        .route(
            "/channels/sidecar/{name}/configure",
            axum::routing::post(configure_sidecar_channel),
        )
        .route(
            "/channels/sidecar/{name}",
            axum::routing::delete(delete_sidecar_channel),
        )
}

use super::sidecar_describe::{describe_sidecar, SidecarSchema, SidecarSchemaField};
// The `super::skills` channel-config helpers
// (upsert_channel_config / remove_channel_config /
// append_channel_instance / update_channel_instance /
// remove_channel_instance / CHANNEL_AOT_CONFLICT_PREFIX /
// validate_env_var) that the deleted in-process channel REST
// endpoints depended on were retired alongside them in this same
// change — `routes/skills.rs` no longer carries any channel-config
// codepaths.
use super::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::types::ApiErrorResponse;

// All channel handlers below resolve the LibreFang home directory via
// `state.kernel.home_dir()` so they honour the kernel's authoritative
// `KernelConfig.home_dir` setting (which itself respects `LIBREFANG_HOME`
// and falls back to `~/.librefang`). The previously-local
// `librefang_home()` helper was removed because it bypassed kernel config
// overrides — see codex review fix #1 and its generalization in fix #7.

// ---------------------------------------------------------------------------
// Channel status endpoints — sidecar-only (every channel runs out-of-process)
// ---------------------------------------------------------------------------

// `FieldType` / `ChannelField` / `ChannelMeta` / `CHANNEL_REGISTRY` /
// `find_channel_meta` / `is_channel_configured` / `build_field_json` /
// `inject_callback_url` / `webhook_route_suffix` /
// `webhook_endpoint_url` / `channel_config_values` /
// `channel_instance_count` / `channel_instances_serialized` —
// the 4 types + 10 helper functions that powered the dashboard's
// per-in-process-channel UI are gone. The registry had been empty
// for several PRs (`const CHANNEL_REGISTRY: &[ChannelMeta] = &[]`)
// and every helper returned the same constant unconditionally.
// All callers — `list_channels` / `channels_snapshot` /
// `get_channel` / `configure_channel` / `remove_channel` /
// `list_channel_instances` / `create_channel_instance` /
// `update_channel_instance_handler` / `delete_channel_instance` /
// `test_channel` — were either deleted (the per-channel REST
// endpoints, which all 404-via-`find_channel_meta` anyway) or
// simplified to skip the empty-registry loop. Dashboard channel
// surface is now exclusively driven by `SIDECAR_CATALOG` +
// `[[sidecar_channels]]` via `sidecar_channel_rows` /
// `sidecar_discovery_rows`.

/// Synthesize dashboard channel rows for configured `[[sidecar_channels]]`.
///
/// telegram / ntfy (and any other sidecar adapter) were removed from
/// `CHANNEL_REGISTRY` when they migrated out-of-process (#5241 / #5224),
/// which silently dropped them from the dashboard channels page. They
/// are still channels — surface the configured ones here so the
/// operator view stays consistent regardless of whether an adapter
/// runs in-process or as a sidecar. These rows are config.toml-managed
/// (`[[sidecar_channels]]`, also under Config -> Sidecar Channels) and are
/// also editable in place: each one carries the `fields` its adapter's
/// cached `--describe` schema declares, merged with this instance's stored
/// values (see `configured_instance_fields`), plus the same
/// `schema_error` / `sdk_version` provenance a discovery row carries so the
/// dashboard can explain an empty `fields` list instead of rendering a
/// drawer with a dead Save button (#8063). The page renders them as
/// configured/online cards and conditionally hides an empty
/// `fields`/`setup_steps`.
///
/// # Liveness (#6606)
///
/// Each row carries the supervisor's real per-instance health, read from `ChannelStatus` via the adapter registered under the instance `name` in `kernel.channel_adapters_ref()`: `connected`, `started_at`, `last_message_at`, `messages_received`, `messages_sent`, `last_error`, plus a `supervised` flag that says whether an adapter is registered for that name at all.
///
/// The dashboard derives its status indicator from these facts; the mapping lives in `dashboard/src/lib/channelLiveness.ts` and is shared by the Channels page and the Comms page's Channels tab.
/// Three properties of the underlying data constrain that mapping and are documented here because they are not obvious from the field names:
///
/// - `started_at` is the timestamp of the **last successful child spawn** (`sidecar.rs` sets it next to `connected = true`), while `messages_received` / `messages_sent` accumulate on the adapter's `Arc<Mutex<ChannelStatus>>`, which outlives every supervised restart.
///   So the counters are since-adapter-creation, not since `started_at`, and must never be captioned as a 24h figure.
/// - `last_error` is **sticky**: the supervisor sets it on a sidecar `error` event, a failed spawn, and a circuit-break, and never clears it — not even on the `connected = true` that follows a successful respawn.
///   A connected channel that carries one is therefore "was unhealthy at least once", not "is broken now", and must read as degraded rather than dead.
///   (The circuit-break regression test in `sidecar.rs` asserts the persistence, so do not "fix" this by clearing on reconnect.)
/// - A configured sidecar whose `start_adapter` failed has its plain key removed from the adapter map again (`channel_bridge.rs`), and one added to `config.toml` without a following channel reload was never registered in the first place.
///   Both land on `supervised: false`, which is the honest reading available here: the API layer cannot tell "start failed" from "never attempted".
fn sidecar_channel_rows(
    sidecar: &[librefang_types::config::SidecarChannelConfig],
    msgs_24h_by_type: &std::collections::HashMap<String, u64>,
    with_msgs: bool,
    adapters: &dashmap::DashMap<String, Arc<dyn librefang_channels::types::ChannelAdapter>>,
    secrets_env_keys: &std::collections::HashSet<String>,
) -> Vec<serde_json::Value> {
    // Previously skipped sidecar entries whose `name` collided with an
    // in-process `CHANNEL_REGISTRY` row; that registry is empty now so
    // there's nothing to shadow — every sidecar gets a card.
    let mut instance_counts: std::collections::HashMap<&str, usize> =
        std::collections::HashMap::new();
    for sc in sidecar {
        *instance_counts.entry(sc.name.as_str()).or_insert(0) += 1;
    }
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut rows = Vec::new();
    // Both describe caches are read once for the whole loop rather than per
    // row. `std::sync::RwLock` gives no re-entrancy guarantee, so taking a
    // second read guard inside the loop while one is already held here can
    // deadlock against a waiting writer.
    let schema_guard = read_cache_recover(schema_cache(), "schema");
    let schema_err_guard = read_cache_recover(schema_error_cache(), "schema error");
    for sc in sidecar {
        let name = sc.name.as_str();
        // One card per distinct sidecar name.
        if !seen.insert(name) {
            continue;
        }
        let channel_type = sc.channel_type.as_deref().unwrap_or(name);
        let schema = schema_guard.get(channel_type);
        let mut row = serde_json::json!({
            "name": name,
            "display_name": name,
            "icon": "SC",
            "description": format!(
                "Out-of-process sidecar adapter ({} {})",
                sc.command,
                sc.args.join(" ")
            ),
            "category": "sidecar",
            "difficulty": "",
            "setup_time": "",
            "quick_setup": "",
            "setup_type": "sidecar",
            "configured": true,
            "instance_count": instance_counts.get(name).copied().unwrap_or(1),
            "has_token": true,
            // Merges the catalog's cached `--describe` schema with this
            // instance's current `env` values (and secrets.env presence) so
            // the gear icon's SidecarForm has something to edit — see
            // `configured_instance_fields`. Empty only when no schema is
            // cached for `channel_type` (SDK missing and no static
            // fallback), same as an unconfigured discovery row.
            "fields": configured_instance_fields(schema, channel_type, sc, secrets_env_keys),
            "setup_steps": [
                "Runs as an out-of-process sidecar adapter",
                "Configured via [[sidecar_channels]] in config.toml \
                 (Config \u{2192} Sidecar Channels)",
            ],
            "config_template": format!(
                "[[sidecar_channels]]\nname = \"{name}\"\nchannel_type = \"{channel_type}\""
            ),
            // Surfaced on its own (it previously only appeared inside
            // `config_template`) so the UI can label the per-type traffic
            // figure with the type it actually covers.
            "channel_type": channel_type,
            // Per-instance default-agent binding (multi-instance support).
            // `null` when this instance has no default agent configured.
            "agent": sc.agent,
        });
        // The two schema-provenance keys a discovery row has always carried.
        // A configured row omitted both, and the missing `schema_error` is the
        // whole of #8063: with no cached schema `fields` is empty, so the gear
        // icon opened a drawer with nothing to edit, no explanation of why, and
        // a live Save button whose request could only ever 503. The dashboard
        // can only distinguish "this adapter has no form" from "this adapter's
        // form could not be loaded, here is the fix" if the row says so.
        if let Some(version) = schema.and_then(|s| s.sdk_version.as_deref()) {
            row["sdk_version"] = serde_json::json!(version);
        }
        if let Some(reason) = schema_err_guard.get(channel_type) {
            row["schema_error"] = serde_json::json!(reason);
        }
        // Per-instance liveness from the sidecar supervisor.
        // Adapters are registered under the instance `name` (the qualified `name:account_id` alias points at the same adapter), which is the same key this loop iterates, so the lookup is per-bot.
        // `json!` of an `Option` yields `null` for `None`, so the three nullable fields keep a stable shape whether or not an adapter is registered — a consumer never has to distinguish "absent" from "unknown".
        let status = adapters.get(name).map(|a| a.value().status());
        row["supervised"] = serde_json::json!(status.is_some());
        row["connected"] = serde_json::json!(status.as_ref().is_some_and(|s| s.connected));
        row["started_at"] = serde_json::json!(status
            .as_ref()
            .and_then(|s| s.started_at)
            .map(|ts| ts.to_rfc3339()));
        row["last_message_at"] = serde_json::json!(status
            .as_ref()
            .and_then(|s| s.last_message_at)
            .map(|ts| ts.to_rfc3339()));
        row["messages_received"] =
            serde_json::json!(status.as_ref().map_or(0, |s| s.messages_received));
        row["messages_sent"] = serde_json::json!(status.as_ref().map_or(0, |s| s.messages_sent));
        row["last_error"] = serde_json::json!(status.as_ref().and_then(|s| s.last_error.clone()));
        if with_msgs {
            // Deliberately per-TYPE, and named so no consumer can mistake it for per-bot traffic.
            // `usage_events.channel` stores the channel type (see `UsageStore::channel_type_msgs_24h_bulk`), so every sidecar of the same type reads the same number.
            // The previous `msgs_24h` key carried a `.or_else(|| msgs_24h.get(name))` per-instance fallback that could never be reached, which made a shared aggregate look per-bot: on a host with six Telegram sidecars all six cards reported the Telegram total (#6606).
            // Per-bot traffic is `messages_received` / `messages_sent` above.
            let m = msgs_24h_by_type.get(channel_type).copied().unwrap_or(0);
            row["msgs_24h_channel_type"] = serde_json::json!(m);
        }
        rows.push(row);
    }
    rows
}

/// Build the editable `fields[]` for an already-configured sidecar
/// instance, merging the catalog's cached `--describe` schema (fetched by
/// `channel_type`) with this instance's current values so the gear icon's
/// SidecarForm has something to save back — before this, configured rows
/// always carried an empty `fields[]`, which reached the dashboard as an
/// unusable edit form (#7892 covered enabling the gear icon at all, not
/// this).
///
/// `has_value` / `value` for non-secret fields come straight from
/// `sc.env` (each `[[sidecar_channels]]` block owns its own env table, so
/// this is already correctly scoped per instance). Secret fields have no
/// stored value here by design (never echoed back as plaintext) — only
/// `has_value`, computed from `secrets_env_keys`, the caller's one-time
/// parse of `secrets.env`. A secondary instance (`sc.name != channel_type`)
/// checks its own `<PREFIX>__KEY` namespaced key rather than the bare
/// global one, mirroring the precedence `write_sidecar_configuration` writes
/// under and `librefang_channels::sidecar::build_spawn_env` reads back.
///
/// `schema` is the caller's already-held lookup of `channel_type` in
/// [`schema_cache`] — the lock is taken once for the whole row loop rather
/// than re-entered here, because `std::sync::RwLock` promises nothing about a
/// nested read guard.
///
/// Returns an empty vec when no schema is cached for `channel_type`. The row
/// then carries `schema_error` instead, which is what lets the dashboard
/// explain the empty form rather than rendering a dead one (#8063).
fn configured_instance_fields(
    schema: Option<&SidecarSchema>,
    channel_type: &str,
    sc: &librefang_types::config::SidecarChannelConfig,
    secrets_env_keys: &std::collections::HashSet<String>,
) -> Vec<serde_json::Value> {
    let Some(schema) = schema else {
        return Vec::new();
    };
    let namespace = if sc.name == channel_type {
        None
    } else {
        Some(librefang_channels::sidecar::instance_secret_prefix(
            &sc.name,
        ))
    };
    schema
        .fields
        .iter()
        .map(|f| {
            let (has_value, value) = if f.field_type == "secret" {
                let key = match &namespace {
                    Some(prefix) => format!("{prefix}__{}", f.key),
                    None => f.key.clone(),
                };
                (secrets_env_keys.contains(&key), None)
            } else {
                let stored = sc.env.get(&f.key).filter(|v| !v.is_empty()).cloned();
                (stored.is_some(), stored)
            };
            let mut field = serde_json::json!({
                "key": f.key,
                "label": f.label,
                "type": f.field_type,
                "required": f.required,
                "placeholder": f.placeholder,
                "advanced": f.advanced,
                "options": f.options,
                "has_value": has_value,
                "env_var": f.key,
            });
            if let Some(value) = value {
                field["value"] = serde_json::json!(value);
            }
            field
        })
        .collect()
}

/// Compile-time field descriptor used as a fallback when the Python sidecar
/// SDK is not installed and `--describe` cannot be executed at boot.
///
/// Field semantics mirror `SidecarSchemaField` but use `&'static str` so the
/// data can live in the binary. The `options` field is omitted because no
/// first-party adapter with `select`-type fields relies on static fallback —
/// adapters with select fields must have the SDK installed.
struct StaticSidecarField {
    key: &'static str,
    label: &'static str,
    /// Matches the `SidecarSchemaField.field_type` values used at runtime:
    /// `"text"`, `"secret"`, `"select"`, `"bool"`.
    field_type: &'static str,
    required: bool,
    placeholder: &'static str,
    advanced: bool,
}

/// One discoverable, first-party sidecar adapter shipped in the SDK.
///
/// `name` doubles as the catalog key — it must match the value the
/// operator will put in `[[sidecar_channels]].channel_type` (or
/// `name`, when `channel_type` is omitted), so a configured entry
/// suppresses the matching catalog row in `sidecar_discovery_rows`.
struct SidecarCatalogEntry {
    name: &'static str,
    display_name: &'static str,
    description: &'static str,
    /// Executable spawned by `populate_sidecar_schema_cache()` with `--describe`
    /// to retrieve the field schema. Also the value the operator would write
    /// to `[[sidecar_channels]].command` if configuring by hand.
    command: &'static str,
    /// Module / script arguments passed to `command`. `--describe` is appended
    /// by `describe_sidecar()` at probe time.
    args: &'static [&'static str],
    /// Last-resort fallback schema for the configure form. `describe_sidecar`
    /// injects the embedded SDK onto PYTHONPATH, so a `python3`-only host (no
    /// `pip install`) normally gets the adapter's live schema; this is used only
    /// when that probe fails outright (no usable `python3`, or the embedded
    /// extract errored). `None` ⇒ empty form in that rare case.
    static_fields: Option<&'static [StaticSidecarField]>,
}

/// First-party sidecar adapters shipped under
/// `sdk/python/librefang/sidecar/adapters/`. Listed here so they stay
/// discoverable on the dashboard channels page after migrating out of
/// `CHANNEL_REGISTRY` (#5241 / #5224) — without an entry, an operator
/// who has never configured them sees no card and no picker entry, so
/// the only way to learn telegram / ntfy exist is to read source code
/// or release notes. `webhook` is deliberately omitted: it still has an
/// in-process entry in `CHANNEL_REGISTRY` and we must not show two
/// "webhook" cards on the page.
/// Compile-time field descriptors for the Telegram adapter.
///
/// Telegram is the first adapter probed at daemon boot. On slow disks its
/// cold Python import can exceed the five-second describe timeout even though
/// the embedded SDK is healthy. Keep this fallback aligned with both the
/// Python and Rust Telegram schemas so the configure form remains usable.
const TELEGRAM_STATIC_FIELDS: &[StaticSidecarField] = &[
    StaticSidecarField {
        key: "TELEGRAM_BOT_TOKEN",
        label: "Bot Token",
        field_type: "secret",
        required: true,
        placeholder: "123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11",
        advanced: false,
    },
    StaticSidecarField {
        key: "ALLOWED_USERS",
        label: "Allowed User IDs",
        field_type: "list",
        required: false,
        placeholder: "123456789, 987654321 — leave empty to allow ALL users (insecure)",
        advanced: true,
    },
    StaticSidecarField {
        key: "TELEGRAM_CLEAR_DONE_REACTION",
        label: "Clear done reaction",
        field_type: "bool",
        required: false,
        placeholder: "",
        advanced: true,
    },
];

/// Compile-time field descriptors for the Feishu / Lark adapter.
///
/// Mirrors `FeishuAdapter.SCHEMA.fields` in
/// `sdk/python/librefang/sidecar/adapters/feishu.py`. These are used as
/// the fallback schema when `python3 -m librefang.sidecar.adapters.feishu
/// --describe` fails at daemon boot (e.g. on Windows without the Python
/// sidecar SDK installed), so the dashboard configure form always shows the
/// required input fields — `FEISHU_APP_ID` and `FEISHU_APP_SECRET` — rather
/// than an empty drawer. Keep in sync with the Python `SCHEMA` definition
/// when fields are added or removed.
const FEISHU_STATIC_FIELDS: &[StaticSidecarField] = &[
    StaticSidecarField {
        key: "FEISHU_APP_ID",
        label: "App ID",
        field_type: "text",
        required: true,
        placeholder: "cli_a...",
        advanced: false,
    },
    StaticSidecarField {
        key: "FEISHU_APP_SECRET",
        label: "App Secret",
        field_type: "secret",
        required: true,
        placeholder: "",
        advanced: false,
    },
    StaticSidecarField {
        key: "FEISHU_REGION",
        label: "Region (cn|intl)",
        field_type: "text",
        required: false,
        placeholder: "cn",
        advanced: true,
    },
    StaticSidecarField {
        key: "FEISHU_RECEIVE_MODE",
        label: "Receive mode (websocket|webhook)",
        field_type: "text",
        required: false,
        placeholder: "websocket",
        advanced: true,
    },
    StaticSidecarField {
        key: "FEISHU_WEBHOOK_PORT",
        label: "Webhook port (webhook mode only)",
        field_type: "text",
        required: false,
        placeholder: "8453",
        advanced: true,
    },
    StaticSidecarField {
        key: "FEISHU_VERIFICATION_TOKEN",
        label: "Verification token (webhook mode)",
        field_type: "secret",
        required: false,
        placeholder: "",
        advanced: true,
    },
    StaticSidecarField {
        key: "FEISHU_ENCRYPT_KEY",
        label: "Encrypt key (webhook mode)",
        field_type: "secret",
        required: false,
        placeholder: "",
        advanced: true,
    },
    StaticSidecarField {
        key: "FEISHU_ACCOUNT_ID",
        label: "Account ID (multi-bot routing)",
        field_type: "text",
        required: false,
        placeholder: "",
        advanced: true,
    },
];

const SIDECAR_CATALOG: &[SidecarCatalogEntry] = &[
    SidecarCatalogEntry {
        name: "telegram",
        display_name: "Telegram",
        description: "Telegram Bot API adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.telegram"],
        static_fields: Some(TELEGRAM_STATIC_FIELDS),
    },
    SidecarCatalogEntry {
        name: "ntfy",
        display_name: "ntfy",
        description: "ntfy.sh pub/sub notifications (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.ntfy"],
        static_fields: None,
    },
    SidecarCatalogEntry {
        name: "gotify",
        display_name: "Gotify",
        description: "Gotify push notifications (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.gotify"],
        static_fields: None,
    },
    SidecarCatalogEntry {
        name: "mastodon",
        display_name: "Mastodon",
        description: "Mastodon Streaming API (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.mastodon"],
        static_fields: None,
    },
    SidecarCatalogEntry {
        name: "bluesky",
        display_name: "Bluesky",
        description: "Bluesky / AT Protocol adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.bluesky"],
        static_fields: None,
    },
    SidecarCatalogEntry {
        name: "reddit",
        display_name: "Reddit",
        description: "Reddit OAuth2 API adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.reddit"],
        static_fields: None,
    },
    SidecarCatalogEntry {
        name: "twitch",
        display_name: "Twitch",
        description: "Twitch IRC gateway adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.twitch"],
        static_fields: None,
    },
    SidecarCatalogEntry {
        name: "rocketchat",
        display_name: "Rocket.Chat",
        description: "Rocket.Chat REST API adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.rocketchat"],
        static_fields: None,
    },
    SidecarCatalogEntry {
        name: "discord",
        display_name: "Discord",
        description: "Discord Gateway bot adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.discord"],
        static_fields: None,
    },
    SidecarCatalogEntry {
        name: "nextcloud",
        display_name: "Nextcloud Talk",
        description: "Nextcloud Talk OCS REST adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.nextcloud"],
        static_fields: None,
    },
    SidecarCatalogEntry {
        name: "slack",
        display_name: "Slack",
        description: "Slack Socket Mode bot adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.slack"],
        static_fields: None,
    },
    SidecarCatalogEntry {
        name: "webex",
        display_name: "Webex",
        description: "Cisco Webex bot adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.webex"],
        static_fields: None,
    },
    SidecarCatalogEntry {
        name: "line",
        display_name: "LINE",
        description: "LINE Messaging API adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.line"],
        static_fields: None,
    },
    SidecarCatalogEntry {
        name: "zulip",
        display_name: "Zulip",
        description: "Zulip REST + event-queue long-poll adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.zulip"],
        static_fields: None,
    },
    SidecarCatalogEntry {
        name: "mattermost",
        display_name: "Mattermost",
        description: "Mattermost WebSocket + REST adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.mattermost"],
        static_fields: None,
    },
    SidecarCatalogEntry {
        name: "signal",
        display_name: "Signal",
        description: "signal-cli REST API adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.signal"],
        static_fields: None,
    },
    SidecarCatalogEntry {
        name: "qq",
        display_name: "QQ Bot",
        description: "QQ Bot API v2 WebSocket + REST adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.qq"],
        static_fields: None,
    },
    SidecarCatalogEntry {
        name: "matrix",
        display_name: "Matrix",
        description: "Matrix Client-Server API adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.matrix"],
        static_fields: None,
    },
    SidecarCatalogEntry {
        name: "feishu",
        display_name: "Feishu / Lark",
        description: "Feishu/Lark Open Platform adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.feishu"],
        // Compile-time fallback — surfaces the configure form even when
        // the Python sidecar SDK is not installed (common on Windows).
        // Mirrors FeishuAdapter.SCHEMA.fields in feishu.py; keep in sync.
        static_fields: Some(FEISHU_STATIC_FIELDS),
    },
    SidecarCatalogEntry {
        name: "wecom",
        display_name: "WeCom",
        description: "WeCom (\u{4f01}\u{4e1a}\u{5fae}\u{4fe1}) intelligent-bot WebSocket adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.wecom"],
        static_fields: None,
    },
    SidecarCatalogEntry {
        name: "email",
        display_name: "Email (IMAP + SMTP)",
        description: "IMAP / SMTP email adapter (out-of-process sidecar, Python stdlib only)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.email"],
        static_fields: None,
    },
    SidecarCatalogEntry {
        name: "dingtalk",
        display_name: "DingTalk",
        description: "DingTalk (\u{9489}\u{9489}) Robot stream-mode adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.dingtalk"],
        static_fields: None,
    },
    SidecarCatalogEntry {
        name: "wechat",
        display_name: "WeChat",
        description: "WeChat personal-account adapter via the iLink (ClawBot) gateway (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.wechat"],
        static_fields: None,
    },
    SidecarCatalogEntry {
        name: "teams",
        display_name: "Microsoft Teams",
        description: "Teams Bot Framework v3 adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.teams"],
        static_fields: None,
    },
    SidecarCatalogEntry {
        name: "whatsapp",
        display_name: "WhatsApp",
        description: "WhatsApp adapter — Meta Cloud API + Web/QR (Baileys) gateway dual-mode (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.whatsapp"],
        static_fields: None,
    },
    SidecarCatalogEntry {
        name: "webhook",
        display_name: "Webhook",
        description: "Generic HMAC-signed HTTP webhook adapter (out-of-process sidecar, Python stdlib only)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.webhook"],
        static_fields: None,
    },
    SidecarCatalogEntry {
        name: "google_chat",
        display_name: "Google Chat",
        description: "Google Chat adapter — service-account JWT auth + REST API send, HTTP webhook receive (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.google_chat"],
        static_fields: None,
    },
];

/// Process-wide cache of sidecar `--describe` schemas, keyed by
/// `SidecarCatalogEntry::name`. Populated once at daemon boot by
/// [`populate_sidecar_schema_cache`]; consumed on every `GET /api/channels`
/// to emit `fields[]` for unconfigured discovery rows. A `RwLock` is used
/// so the in-test seeder ([`__test_seed_sidecar_schema_cache`]) can replace
/// entries deterministically between tests without rebuilding the daemon.
static SIDECAR_SCHEMA_CACHE: OnceLock<RwLock<HashMap<&'static str, SidecarSchema>>> =
    OnceLock::new();

fn schema_cache() -> &'static RwLock<HashMap<&'static str, SidecarSchema>> {
    SIDECAR_SCHEMA_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Process-wide cache of the *reason* a catalog adapter has no usable schema, keyed by `SidecarCatalogEntry::name`.
/// Populated alongside [`SIDECAR_SCHEMA_CACHE`] in [`populate_sidecar_schema_cache`] when `--describe` fails AND the entry has no `static_fields` fallback — i.e. exactly the case where the dashboard would otherwise render an empty configure form with no explanation.
/// The string is the already-actionable hint from `describe_sidecar` (e.g. the `pip install librefang-sdk` install hint), surfaced verbatim as the row's `schema_error` so the operator learns *why* the form is empty and how to fix it instead of staring at a blank drawer.
static SIDECAR_SCHEMA_ERROR_CACHE: OnceLock<RwLock<HashMap<&'static str, String>>> =
    OnceLock::new();

fn schema_error_cache() -> &'static RwLock<HashMap<&'static str, String>> {
    SIDECAR_SCHEMA_ERROR_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

fn read_cache_recover<'a, T>(
    cache: &'a RwLock<T>,
    cache_name: &'static str,
) -> RwLockReadGuard<'a, T> {
    cache.read().unwrap_or_else(|poisoned| {
        tracing::warn!(
            cache = cache_name,
            "Channel schema cache lock poisoned; recovering state"
        );
        cache.clear_poison();
        poisoned.into_inner()
    })
}

fn write_cache_recover<'a, T>(
    cache: &'a RwLock<T>,
    cache_name: &'static str,
) -> RwLockWriteGuard<'a, T> {
    cache.write().unwrap_or_else(|poisoned| {
        tracing::warn!(
            cache = cache_name,
            "Channel schema cache lock poisoned; recovering state"
        );
        cache.clear_poison();
        poisoned.into_inner()
    })
}

/// Spawn `<command> <args> --describe` for every catalog entry and cache
/// the resulting schemas. Called once at daemon boot from
/// `server::build_router`. `describe_sidecar` injects the binary-embedded
/// `librefang-sdk` onto the child's PYTHONPATH (see there), so on any host with
/// just `python3` on PATH the probe succeeds and the dashboard gets the
/// adapter's authoritative live schema — no `pip install` required.
/// `static_fields` is now only a last-resort fallback for the case where even
/// that probe fails (no `python3` at all, or the embedded extract errored): a
/// failure is logged at WARN, and when the entry carries `static_fields` those
/// compile-time fields seed the form instead of leaving an empty `fields[]`.
/// `home_dir` must be the kernel's `KernelConfig.home_dir`
/// (`KernelApi::home_dir()`); it locates the embedded-SDK extraction dir.
pub async fn populate_sidecar_schema_cache(home_dir: &std::path::Path) {
    for entry in SIDECAR_CATALOG {
        let args: Vec<String> = entry.args.iter().map(|s| s.to_string()).collect();
        match describe_sidecar(entry.command, &args, home_dir).await {
            Ok(schema) => {
                tracing::info!(
                    adapter = entry.name,
                    fields = schema.fields.len(),
                    sdk_version = schema.sdk_version.as_deref().unwrap_or("unreported"),
                    "sidecar schema cached"
                );
                write_cache_recover(schema_cache(), "schema").insert(entry.name, schema);
            }
            Err(e) => {
                if let Some(static_fields) = entry.static_fields {
                    // Use the compile-time fallback so the configure form is
                    // usable even without a working Python SDK installation.
                    let fallback = SidecarSchema {
                        name: entry.name.to_string(),
                        display_name: entry.display_name.to_string(),
                        description: entry.description.to_string(),
                        fields: static_fields
                            .iter()
                            .map(|f| SidecarSchemaField {
                                key: f.key.to_string(),
                                label: f.label.to_string(),
                                field_type: f.field_type.to_string(),
                                required: f.required,
                                placeholder: f.placeholder.to_string(),
                                advanced: f.advanced,
                                options: None,
                            })
                            .collect(),
                        // The fallback exists precisely because `--describe`
                        // failed, so no adapter reported a version here.
                        sdk_version: None,
                    };
                    tracing::warn!(
                        adapter = entry.name,
                        error = %e,
                        fields = fallback.fields.len(),
                        "sidecar --describe failed; using compile-time fallback schema"
                    );
                    write_cache_recover(schema_cache(), "schema").insert(entry.name, fallback);
                } else {
                    tracing::warn!(
                        adapter = entry.name,
                        error = %e,
                        "sidecar --describe failed; channel cards will have no form fields"
                    );
                    // Stash the failure reason so every row for this adapter — the discovery card and each configured `[[sidecar_channels]]` instance of the type (#8063) — can tell the operator *why* the form is empty (typically: Python sidecar SDK not installed).
                    write_cache_recover(schema_error_cache(), "schema error")
                        .insert(entry.name, e.to_string());
                }
            }
        }
    }
}

/// Test-only seeder for the sidecar schema cache. Wipes any existing
/// entries and replaces them with the supplied pairs so integration tests
/// can assert deterministic `fields[]` payloads without depending on a
/// working Python SDK installation. `#[doc(hidden)]` because no production
/// caller should ever reach for this — the public path is
/// [`populate_sidecar_schema_cache`] at boot.
#[doc(hidden)]
pub fn __test_seed_sidecar_schema_cache(entries: &[(&'static str, SidecarSchema)]) {
    let mut guard = write_cache_recover(schema_cache(), "schema");
    guard.clear();
    for (k, v) in entries {
        guard.insert(*k, v.clone());
    }
}

/// Test-only seeder for the sidecar schema-error cache.
/// Mirrors [`__test_seed_sidecar_schema_cache`] so integration tests can assert the `schema_error` field on discovery rows without a failing live `--describe`.
/// `#[doc(hidden)]` for the same reason.
#[doc(hidden)]
pub fn __test_seed_sidecar_schema_error_cache(entries: &[(&'static str, String)]) {
    let mut guard = write_cache_recover(schema_error_cache(), "schema error");
    guard.clear();
    for (k, v) in entries {
        guard.insert(*k, v.clone());
    }
}

/// Synthesize **catalog** dashboard rows — one per `SIDECAR_CATALOG` entry,
/// always, regardless of how many `[[sidecar_channels]]` instances of that
/// type are already configured. These feed the Add-channel picker.
///
/// Before multi-instance support (#8xxx) a catalog entry was suppressed
/// once ANY `[[sidecar_channels]]` matched its type, on the theory that a
/// configured channel type "has done its job" and should yield entirely to
/// the configured rows from [`sidecar_channel_rows`]. That precluded ever
/// adding a *second* instance of an already-configured type (a second
/// Telegram bot, a second Slack workspace) from the dashboard — the type's
/// only picker entry was gone. The catalog row is a always a "start a new
/// instance of this type" affordance now; `configured` rows (with their own
/// edit/delete actions) are the only place an *existing* instance is
/// edited.
fn sidecar_discovery_rows() -> Vec<serde_json::Value> {
    let cache_guard = read_cache_recover(schema_cache(), "schema");
    let err_guard = read_cache_recover(schema_error_cache(), "schema error");
    let mut rows = Vec::new();
    for entry in SIDECAR_CATALOG {
        let fields: Vec<serde_json::Value> = cache_guard
            .get(entry.name)
            .map(|s| {
                s.fields
                    .iter()
                    .map(|f| {
                        serde_json::json!({
                            "key": f.key,
                            "label": f.label,
                            "type": f.field_type,
                            "required": f.required,
                            "placeholder": f.placeholder,
                            "advanced": f.advanced,
                            "options": f.options,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let mut row = serde_json::json!({
            "name": entry.name,
            "display_name": entry.display_name,
            "icon": "SC",
            "description": entry.description,
            "category": "sidecar",
            "difficulty": "",
            "setup_time": "",
            "quick_setup": "",
            "setup_type": "sidecar",
            "configured": false,
            "instance_count": 0,
            "has_token": false,
            "fields": fields,
            "setup_steps": [
                "Runs as an out-of-process sidecar adapter",
                "Fill the form to save credentials to ~/.librefang/secrets.env \
                 (secrets) and ~/.librefang/config.toml (non-secrets)",
            ],
        });
        // The SDK version the adapter reported on `--describe`, so an operator
        // can see which `librefang-sdk` is actually winning without shelling
        // into the box (#7140: a March SDK served a August daemon for months).
        // Omitted rather than nulled when the adapter did not report one — an
        // SDK too old to carry the field, or a failed describe.
        if let Some(version) = cache_guard
            .get(entry.name)
            .and_then(|s| s.sdk_version.as_deref())
        {
            row["sdk_version"] = serde_json::json!(version);
        }
        // When `--describe` failed at boot and there is no static fallback, `fields` is empty and the configure form would be a blank drawer.
        // Surface the cached failure reason (typically the `pip install librefang-sdk` install hint) so the dashboard can explain why instead of showing nothing.
        if let Some(reason) = err_guard.get(entry.name) {
            row["schema_error"] = serde_json::json!(reason);
        }
        rows.push(row);
    }
    rows
}

/// Request body for `POST /api/channels/sidecar/{name}/configure`.
///
/// `values` is a flat `key → string` map where each key matches a
/// `SidecarSchemaField.key` returned by the sidecar's `--describe`.
/// The endpoint splits the map by `field_type`: `secret` fields are
/// written line-by-line to `~/.librefang/secrets.env`, every other
/// field is written under `[sidecar_channels.env]` in
/// `~/.librefang/config.toml`. All current first-party sidecar field
/// types (text, secret, list, bool, select) are stringly representable,
/// so a flat `HashMap<String, String>` is sufficient — payload-typed
/// fields (numbers etc.) would need a richer shape.
#[derive(serde::Deserialize, utoipa::ToSchema)]
pub struct ConfigureSidecarBody {
    pub values: HashMap<String, String>,
    /// Multi-instance support: the unique `[[sidecar_channels]].name` to
    /// write. Defaults to the path `{name}` (the catalog type) when absent
    /// or blank, which is exactly today's one-instance-per-type behaviour —
    /// existing dashboard builds and API callers that never send this field
    /// keep working unchanged. Set it to a distinct value to configure a
    /// second (or third, …) instance of the same catalog type — e.g. two
    /// Telegram bots named `telegram` and `telegram-support`.
    #[serde(default)]
    pub instance_name: Option<String>,
    /// Per-instance default agent (`[[sidecar_channels]].agent`) — inbound
    /// messages on this instance with no more specific binding route here.
    /// `None` / empty clears the field; omitted entirely leaves an existing
    /// value untouched only insofar as the whole `agent` key is untouched by
    /// this form — unlike `values`, there is no partial-update semantics
    /// here because there's exactly one field, so send the field's current
    /// value back to keep it.
    #[serde(default)]
    pub agent: Option<String>,
}

/// Detect `[[sidecar_channels]]` entries in files referenced from the root
/// config's `include = [...]` directive.
///
/// Background: librefang merges every file in `include` into the runtime
/// config (`librefang_kernel::config::load_config`). The merge concatenates
/// arrays-of-tables — so if an included file declares `[[sidecar_channels]]`
/// and we write a fresh root-level `[[sidecar_channels]]` here, the live
/// config will contain BOTH entries. The freshly-written root entry will
/// silently shadow the included one on dashboard / configure paths
/// (the kernel reads them in include-first order, but the dashboard
/// configure flow expects to be editing the canonical entry).
///
/// Cheap heuristic: substring-match `[[sidecar_channels]]` in each included
/// file. False positives on a comment containing that exact string are
/// acceptable — the operator can either remove the comment or edit the
/// included file directly as the 409 message recommends. Returns the list
/// of include paths that contain at least one `[[sidecar_channels]]`
/// header. A missing root config has no includes; every other read or parse
/// failure is returned so the write-side safety check fails closed.
#[cfg(test)]
async fn included_files_with_sidecars(
    config_path: &std::path::Path,
) -> Result<Vec<std::path::PathBuf>, String> {
    let content = match tokio::fs::read_to_string(config_path).await {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(format!(
                "failed to read root config {}: {e}",
                config_path.display()
            ));
        }
    };
    // Extract owned paths before the first include-file await. DocumentMut
    // contains non-Send internals and must not be retained across a suspend
    // point in this axum handler.
    let include_paths = {
        let doc: toml_edit::DocumentMut = content
            .parse()
            .map_err(|e| format!("failed to parse root config {}: {e}", config_path.display()))?;
        // `include` may be a string array at the document root.
        let Some(include_arr) = doc.get("include").and_then(|i| i.as_array()) else {
            return Ok(Vec::new());
        };
        let parent = config_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));
        include_arr
            .iter()
            .filter_map(|entry| entry.as_str())
            .map(|raw| {
                if std::path::Path::new(raw).is_absolute() {
                    std::path::PathBuf::from(raw)
                } else {
                    parent.join(raw)
                }
            })
            .collect::<Vec<_>>()
    };
    let mut hits = Vec::new();
    for path in include_paths {
        let body = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| format!("failed to read included config {}: {e}", path.display()))?;
        if body.contains("[[sidecar_channels]]") {
            hits.push(path);
        }
    }
    Ok(hits)
}

/// Blocking counterpart used only inside the configure handler's `spawn_blocking` transaction.
/// Keeping the include reads in that transaction prevents the include list from changing between validation and the configuration write.
fn included_files_with_sidecars_blocking(
    config_path: &std::path::Path,
) -> Result<Vec<std::path::PathBuf>, String> {
    let content = match std::fs::read_to_string(config_path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(format!(
                "failed to read root config {}: {error}",
                config_path.display()
            ));
        }
    };
    let document: toml_edit::DocumentMut = content.parse().map_err(|error| {
        format!(
            "failed to parse root config {}: {error}",
            config_path.display()
        )
    })?;
    let Some(include_array) = document.get("include").and_then(|item| item.as_array()) else {
        return Ok(Vec::new());
    };
    let parent = config_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let mut hits = Vec::new();
    for raw in include_array.iter().filter_map(|entry| entry.as_str()) {
        let raw_path = std::path::Path::new(raw);
        let path = if raw_path.is_absolute() {
            raw_path.to_path_buf()
        } else {
            parent.join(raw_path)
        };
        let body = std::fs::read_to_string(&path).map_err(|error| {
            format!("failed to read included config {}: {error}", path.display())
        })?;
        if body.contains("[[sidecar_channels]]") {
            hits.push(path);
        }
    }
    Ok(hits)
}

#[derive(Debug)]
enum ConfigureSidecarWriteError {
    IncludedSidecars(Vec<std::path::PathBuf>),
    Write(String),
    /// A different `[[sidecar_channels]]` entry already owns `instance_name`
    /// under a different `channel_type`. Carries that entry's channel type
    /// so the 409 body can name the collision. Same-type reuse of a name is
    /// not a conflict — it is the update path (`upsert_sidecar_block`
    /// matches by name and edits in place).
    NameConflict(String),
    /// A `required` secret field carries no value: the payload omitted it (or
    /// sent whitespace) and this instance has no non-empty value stored for it
    /// either (a bare `KEY=` line in secrets.env is a key, not a value).
    /// Carries the field key so the 400 body can name it. Checked inside the
    /// write step rather than in the handler because deciding it needs the
    /// same `secrets.env` snapshot the write uses — reading it earlier, outside
    /// `config_write_lock`, would be a TOCTOU against a concurrent save.
    MissingRequiredSecret(String),
    /// `instance_name` normalizes to the same `<PREFIX>__` secret namespace as one or more already-configured instances, so saving it would overwrite their secrets.
    /// Carries the shared prefix and the colliding names.
    SecretPrefixConflict {
        prefix: String,
        names: Vec<String>,
    },
}

/// Names of the configured `[[sidecar_channels]]` entries that keep their secrets in a `<PREFIX>__` namespace — that is, every entry whose `name` differs from its `channel_type`.
/// The instance sharing the catalog's own name writes bare keys and so cannot collide through the prefix.
fn namespaced_instance_names(config_content: &str) -> Result<Vec<String>, String> {
    if config_content.trim().is_empty() {
        return Ok(Vec::new());
    }
    let document: toml_edit::DocumentMut = config_content
        .parse()
        .map_err(|error| format!("parse config.toml: {error}"))?;
    let Some(array) = document
        .get("sidecar_channels")
        .and_then(|item| item.as_array_of_tables())
    else {
        return Ok(Vec::new());
    };
    Ok(array
        .iter()
        .filter_map(|table| {
            let name = table.get("name").and_then(|item| item.as_str())?;
            let channel_type = table
                .get("channel_type")
                .and_then(|item| item.as_str())
                .unwrap_or(name);
            (name != channel_type).then(|| name.to_string())
        })
        .collect())
}

/// Look for an existing `[[sidecar_channels]]` entry named `instance_name`
/// whose `channel_type` (or, when unset, its own `name`) differs from
/// `channel_type`. Multiple instances are allowed to share a `name` only
/// when they are actually the same instance being edited — a name collision
/// across two different adapter types would otherwise silently reassign an
/// existing bot's block to a new command/schema on the next save.
fn find_conflicting_channel_type(
    config_content: &str,
    instance_name: &str,
    channel_type: &str,
) -> Result<Option<String>, String> {
    if config_content.trim().is_empty() {
        return Ok(None);
    }
    let document: toml_edit::DocumentMut = config_content
        .parse()
        .map_err(|error| format!("parse config.toml: {error}"))?;
    let Some(array) = document
        .get("sidecar_channels")
        .and_then(|item| item.as_array_of_tables())
    else {
        return Ok(None);
    };
    for table in array.iter() {
        let name = table
            .get("name")
            .and_then(|item| item.as_str())
            .unwrap_or("");
        if name != instance_name {
            continue;
        }
        let existing_type = table
            .get("channel_type")
            .and_then(|item| item.as_str())
            .unwrap_or(name);
        if existing_type != channel_type {
            return Ok(Some(existing_type.to_string()));
        }
        return Ok(None);
    }
    Ok(None)
}

fn read_file_snapshot(
    path: &std::path::Path,
) -> Result<Option<String>, ConfigureSidecarWriteError> {
    match std::fs::read_to_string(path) {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(ConfigureSidecarWriteError::Write(format!(
            "failed to snapshot {} before sidecar configuration: {error}",
            path.display()
        ))),
    }
}

#[cfg(unix)]
fn sync_rollback_parent(path: &std::path::Path) -> Result<(), String> {
    let parent = path.parent().ok_or("rollback path has no parent")?;
    std::fs::File::open(parent)
        .and_then(|dir| dir.sync_all())
        .map_err(|error| format!("sync parent directory {}: {error}", parent.display()))
}

#[cfg(not(unix))]
fn sync_rollback_parent(_path: &std::path::Path) -> Result<(), String> {
    Ok(())
}

fn restore_secret_snapshot(path: &std::path::Path, contents: Option<&str>) -> Result<(), String> {
    let Some(contents) = contents else {
        return match std::fs::remove_file(path) {
            Ok(()) => sync_rollback_parent(path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("remove {}: {error}", path.display())),
        };
    };

    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);

    let parent = path.parent().ok_or("secrets path has no parent")?;
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let staging = parent.join(format!(
        ".secrets.env.rollback.{}.{seq}",
        std::process::id()
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&staging)
        .map_err(|error| format!("create {}: {error}", staging.display()))?;
    let write_result = file
        .write_all(contents.as_bytes())
        .and_then(|()| file.sync_all());
    if let Err(error) = write_result {
        drop(file);
        let _ = std::fs::remove_file(&staging);
        return Err(format!("write {}: {error}", staging.display()));
    }
    drop(file);
    if let Err(error) = std::fs::rename(&staging, path) {
        let _ = std::fs::remove_file(&staging);
        return Err(format!(
            "rename {} to {}: {error}",
            staging.display(),
            path.display()
        ));
    }
    sync_rollback_parent(path)
}

fn rollback_sidecar_configuration(
    config_path: &std::path::Path,
    original_config: Option<&str>,
    secrets_path: &std::path::Path,
    original_secrets: Option<&str>,
) -> Vec<String> {
    let mut errors = Vec::new();
    if let Err(error) = super::sidecar_toml::restore_sidecar_file(config_path, original_config) {
        errors.push(format!("config.toml: {error}"));
    }
    if let Err(error) = restore_secret_snapshot(secrets_path, original_secrets) {
        errors.push(format!("secrets.env: {error}"));
    }
    errors
}

fn write_sidecar_configuration(
    config_path: &std::path::Path,
    secrets_path: &std::path::Path,
    instance_name: &str,
    entry: &SidecarCatalogEntry,
    schema: &SidecarSchema,
    values: &HashMap<String, String>,
    agent: Option<&str>,
) -> Result<Vec<String>, ConfigureSidecarWriteError> {
    let shadowing = included_files_with_sidecars_blocking(config_path)
        .map_err(ConfigureSidecarWriteError::Write)?;
    if !shadowing.is_empty() {
        return Err(ConfigureSidecarWriteError::IncludedSidecars(shadowing));
    }

    // Snapshot both files before the first mutation. A sidecar form spans
    // config.toml and secrets.env, while each helper can only atomically
    // replace one file. Keeping these snapshots under config_write_lock lets
    // us compensate if any later secret or config write fails without
    // overwriting another in-process writer.
    let original_config = read_file_snapshot(config_path)?;
    let original_secrets = read_file_snapshot(secrets_path)?;

    // Multi-instance support (several `[[sidecar_channels]]` of the same
    // catalog type, each with its own `name`): refuse a save that would
    // reassign an existing, differently-typed instance's block to this
    // request's adapter. Checked against the pre-write snapshot, before any
    // file is touched, so a rejected request leaves both files untouched —
    // same "fail before the first mutation" contract as the include check
    // above.
    if let Some(conflicting_type) = find_conflicting_channel_type(
        original_config.as_deref().unwrap_or_default(),
        instance_name,
        entry.name,
    )
    .map_err(ConfigureSidecarWriteError::Write)?
    {
        return Err(ConfigureSidecarWriteError::NameConflict(conflicting_type));
    }

    // `instance_secret_prefix` uppercases and maps every non-alphanumeric character to `_`, so it is many-to-one: `bot-1`, `bot.1` and `BOT+1` all land on `BOT_1`.
    // Two instances that collapse together share one `<PREFIX>__KEY` namespace in secrets.env, and the second save silently overwrites the first one's token — the opposite of the isolation this endpoint exists to provide.
    //
    // `warn_secret_prefix_collisions` reports that from the boot / reload loop, which was enough while a second instance meant hand-editing config.toml in front of the existing entries.
    // It is not enough for a two-field form: by the time the WARN appears the token is already gone. Refuse here, on the same "fail before the first mutation" contract as the checks above.
    //
    // Only namespaced instances can collide this way — the one sharing the catalog's own name writes bare keys — so a default-named save skips it.
    if instance_name != entry.name {
        let existing = namespaced_instance_names(original_config.as_deref().unwrap_or_default())
            .map_err(ConfigureSidecarWriteError::Write)?;
        let names = librefang_channels::sidecar::secret_prefix_conflict(&existing, instance_name);
        if !names.is_empty() {
            return Err(ConfigureSidecarWriteError::SecretPrefixConflict {
                prefix: librefang_channels::sidecar::instance_secret_prefix(instance_name),
                names,
            });
        }
    }

    // A second (third, …) named instance of the same catalog type must not
    // share the first instance's secret — `TELEGRAM_BOT_TOKEN` can only ever
    // hold one value. `librefang_channels::sidecar::build_spawn_env` already
    // resolves a `<PREFIX>__<KEY>` namespaced secret ahead of the bare
    // global key for exactly this reason (#6169); this save just has to
    // start writing into that namespace once there's more than one instance
    // of the type. The instance sharing the catalog's own name keeps writing
    // the bare key — zero behaviour change for every config that predates
    // multi-instance support.
    let secret_namespace = if instance_name == entry.name {
        None
    } else {
        Some(librefang_channels::sidecar::instance_secret_prefix(
            instance_name,
        ))
    };
    let secret_key = |field_key: &str| match &secret_namespace {
        Some(prefix) => format!("{prefix}__{field_key}"),
        None => field_key.to_string(),
    };

    let secrets_env_entries = librefang_channels::sidecar::parse_secrets_env_contents(
        original_secrets.as_deref().unwrap_or_default(),
    );
    // Every key present in the file, regardless of whether it holds a value —
    // what the shadow-warning check below wants ("was this key already in
    // secrets.env before this save?").
    let secrets_env_keys: std::collections::HashSet<String> = secrets_env_entries
        .iter()
        .map(|(key, _)| key.clone())
        .collect();
    // Keys that actually hold a value. This is the set the required-secret
    // check below reads, and it applies the same filter `read_secrets_env_keys`
    // applies when it computes the row's `has_value` — so a save is accepted on
    // exactly the secrets the drawer showed as set, and a hand-edited `KEY=`
    // line (a key with no value) cannot satisfy a required field.
    let secrets_env_values: std::collections::HashSet<String> = secrets_env_entries
        .into_iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(key, _)| key)
        .collect();

    // Required-secret validation, deliberately "has a value after this save"
    // rather than "is present in this payload" (#8063).
    //
    // The configure drawer never echoes a stored secret back as plaintext, so
    // editing one non-secret field on a configured instance submits a payload
    // with no token in it — and the write path below already treats that as
    // "leave the stored secret alone" (an absent key never reaches
    // `upsert_secret`). Rejecting it here anyway made every edit of a
    // configured Slack instance 400 with `required field SLACK_APP_TOKEN is
    // missing or empty` unless the operator re-pasted both workspace tokens
    // from scratch, which is the failure the issue reproduced.
    //
    // The guarantee the check exists for is unchanged: after a successful save
    // the instance has a value for every required secret. Accepted sources, in
    // the order `librefang_channels::sidecar::build_spawn_env` resolves them —
    // this request's payload, then this instance's `secrets.env` key
    // (namespaced `<INSTANCE>__<KEY>` for a secondary instance, bare for the
    // one named after its catalog type), then the daemon's own environment.
    //
    // The daemon-environment source is deliberately accepted for the bare-key
    // path only, and that is narrower than what the child actually sees: the
    // supervisor never calls `Command::env_clear` (see the precedence notes on
    // `build_spawn_env`), so an exported bare key is inherited by *every*
    // sidecar child, namespaced instances included. Widening it here would
    // undo the point of per-instance namespacing — a second instance would be
    // allowed to save with no secret of its own and silently run on the first
    // one's token — so a namespaced instance must name its own secret.
    //
    // Required non-secret fields stay strict in the handler's earlier pass:
    // their current values *are* echoed into the form, so the form always
    // resubmits them, and `write_form_managed` deletes a managed env key that
    // a save omits. Accepting an omission here would green-light a save that
    // then removes the very key it was told was satisfied.
    for field in &schema.fields {
        if !field.required || field.field_type != "secret" {
            continue;
        }
        let submitted = values
            .get(&field.key)
            .is_some_and(|value| !value.trim().is_empty());
        let stored = secrets_env_values.contains(&secret_key(&field.key));
        let inherited = secret_namespace.is_none()
            && std::env::var(&field.key).is_ok_and(|value| !value.trim().is_empty());
        if !submitted && !stored && !inherited {
            return Err(ConfigureSidecarWriteError::MissingRequiredSecret(
                field.key.clone(),
            ));
        }
    }

    // Namespaced per-instance secrets always win over the parent process env
    // (`build_spawn_env` never consults it for them), so the shadow warning
    // — "a shell-exported var will out-rank what this save just wrote" — is
    // only meaningful for the bare-key (single/default-instance) path.
    let mut shadowed_secrets: Vec<String> = schema
        .fields
        .iter()
        .filter(|field| field.field_type == "secret")
        .filter(|field| {
            values
                .get(&field.key)
                .is_some_and(|value| !value.trim().is_empty())
        })
        .filter(|field| {
            secret_namespace.is_none()
                && std::env::var(&field.key).is_ok()
                && !secrets_env_keys.contains(&field.key)
        })
        .map(|field| field.key.clone())
        .collect();
    shadowed_secrets.sort();

    let write_result = (|| -> Result<(), String> {
        let mut nonsecret_env = std::collections::BTreeMap::new();
        for field in &schema.fields {
            let Some(raw) = values.get(&field.key) else {
                continue;
            };
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                continue;
            }
            if field.field_type == "secret" {
                let key = secret_key(&field.key);
                super::secrets_env::upsert_secret(secrets_path, &key, trimmed)?;
            } else {
                nonsecret_env.insert(field.key.clone(), trimmed.to_string());
            }
        }

        let managed_env_keys: Vec<&str> = schema
            .fields
            .iter()
            .filter(|field| field.field_type != "secret")
            .map(|field| field.key.as_str())
            .collect();
        super::sidecar_toml::upsert_sidecar_block(
            config_path,
            instance_name,
            entry.name,
            entry.command,
            entry.args,
            &nonsecret_env,
            &managed_env_keys,
            agent,
        )
    })();
    if let Err(error) = write_result {
        let rollback_errors = rollback_sidecar_configuration(
            config_path,
            original_config.as_deref(),
            secrets_path,
            original_secrets.as_deref(),
        );
        let error = if rollback_errors.is_empty() {
            error
        } else {
            format!("{error}; rollback failed: {}", rollback_errors.join("; "))
        };
        return Err(ConfigureSidecarWriteError::Write(error));
    }

    Ok(shadowed_secrets)
}

/// `POST /api/channels/sidecar/{name}/configure` — save schema-driven
/// sidecar form values, splitting the payload across `secrets.env` and
/// `config.toml`, then trigger a hot-reload so the kernel picks up the
/// new `[[sidecar_channels]]` block without a restart. `name` is always the
/// `SIDECAR_CATALOG` key (`telegram`, `ntfy`, …) — it picks the adapter's
/// schema/command/args and never changes across a rename. The
/// `[[sidecar_channels]].name` actually written is `body.instance_name`
/// when present, falling back to `name` (today's one-instance-per-type
/// behaviour) so multiple named instances of the same catalog type
/// (e.g. two Telegram bots) can be configured side by side.
#[utoipa::path(
    post,
    path = "/api/channels/sidecar/{name}/configure",
    tag = "channels",
    request_body = ConfigureSidecarBody,
    params(
        ("name" = String, Path, description = "Sidecar catalog name (e.g. telegram, ntfy)")
    ),
    responses(
        (status = 200, description = "Saved; reload plan returned. Body fields: \
            `status` (\"saved\"), `hot_actions_applied` ([String]), `restart_required` (bool), \
            `shadowed_secrets` ([String]) — secret field keys whose value is already \
            present in the daemon's process environment (e.g. exported by the launching \
            shell). Those values will out-rank the freshly-written secrets.env entry \
            until the operator unsets them and restarts the daemon.", body = crate::types::JsonObject),
        (status = 400, description = "Missing required field or invalid value", body = crate::types::JsonObject),
        (status = 404, description = "Unknown catalog name", body = crate::types::JsonObject),
        (status = 409, description = "config.toml uses `include` and an existing `[[sidecar_channels]]` entry lives in an included file — would silently shadow; or `instance_name` already names a differently-typed instance.", body = crate::types::JsonObject),
        (status = 423, description = "Configuration is managed by the deployment; declare the sidecar in the manifest instead.", body = crate::types::JsonObject),
        (status = 503, description = "Schema not cached — SDK module may be missing", body = crate::types::JsonObject),
    )
)]
pub async fn configure_sidecar_channel(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<ConfigureSidecarBody>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, Json<serde_json::Value>)> {
    // 0. Managed mode (#6695) — refused in full, before the catalog lookup and before either file is opened.
    //    The scope matches `set_provider_key` rather than the narrow config-only guards: this handler writes `secrets.env` and `config.toml` inside one `spawn_blocking` call (`write_sidecar_configuration`), so a guard placed at the config write would already have persisted the secrets half and mutated the process environment before refusing.
    //    Refusing up front keeps the request atomic and states the contract plainly: in a managed deployment a sidecar channel is declared as `[[sidecar_channels]]` in the manifest, with its secrets supplied from the pod environment.
    if let Some(locked) = crate::routes::guard_config_write(state.kernel.config_path()) {
        return Err(locked);
    }

    // 1. Catalog lookup — only first-party adapters listed in
    //    SIDECAR_CATALOG can be configured through this endpoint.
    let entry = SIDECAR_CATALOG
        .iter()
        .find(|e| e.name == name)
        .ok_or_else(|| {
            ApiErrorResponse::not_found(format!("no sidecar adapter named `{name}`"))
                .into_json_tuple()
        })?;

    // 2. Pull the cached `--describe` schema. Without it we can't
    //    validate required fields or split secret-vs-nonsecret.
    let schema = read_cache_recover(schema_cache(), "schema")
        .get(entry.name)
        .cloned()
        .ok_or_else(|| {
            ApiErrorResponse::internal(format!(
                "schema for `{name}` not cached — SDK module may be missing or `--describe` failed at boot"
            ))
            .with_status(StatusCode::SERVICE_UNAVAILABLE)
            .into_json_tuple()
        })?;

    // 3. Validate required NON-SECRET fields: present in payload AND non-empty
    //    after trim. The form renders these with their current values, so a
    //    save always resubmits them, and `write_form_managed` removes a managed
    //    env key that a save omits — an omission here really is a request to
    //    end up with no value.
    //
    //    Required *secret* fields are validated in `write_sidecar_configuration`
    //    instead, against the `secrets.env` snapshot the write itself uses: a
    //    stored secret is never echoed into the form, so "absent from the
    //    payload" means "keep what is stored", not "clear it" (#8063).
    for f in &schema.fields {
        if f.required && f.field_type != "secret" {
            let v = body.values.get(&f.key).map(|s| s.trim()).unwrap_or("");
            if v.is_empty() {
                return Err(ApiErrorResponse::bad_request(format!(
                    "required field `{}` is missing or empty",
                    f.key
                ))
                .into_json_tuple());
            }
        }
    }

    // 3a. Resolve the instance name this save actually writes under.
    //     Blank is treated the same as absent — a stray empty string from a
    //     cleared form field must not become a `[[sidecar_channels]]` with
    //     name = "".
    let instance_name = body
        .instance_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(name.as_str())
        .to_string();
    let agent = body
        .agent
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    // 3b. Resolve `~/.librefang` paths from the kernel's configured
    //     `home_dir` rather than recomputing from `LIBREFANG_HOME` /
    //     `~/.librefang`: when the operator boots with a non-default
    //     `KernelConfig.home_dir`, the recomputed default would write
    //     to the wrong path while `reload_config()` and
    //     `reload_channels_from_disk()` read from the kernel's path.
    //     (Shell-shadow detection for secret fields now lives under
    //     the config_write_lock in step 4a below.)
    let home = state.kernel.home_dir().to_path_buf();
    let secrets_path = home.join("secrets.env");
    let config_path = state.kernel.config_path().to_path_buf();

    // 4. Split payload: secrets go to secrets.env, everything else goes into the [sidecar_channels.env] table.
    //
    //    Both the secrets.env upserts and the config.toml upsert below run inside `state.config_write_lock`.
    //    That mutex also gates `POST /api/config/set` and the legacy `configure_channel` handler (issue #3183), so two concurrent `POST /api/channels/sidecar/{a,b}/configure` calls — or one of those interleaved with `config_set` — cannot lost-update on `~/.librefang/config.toml` or on `~/.librefang/secrets.env`.
    //    The guard is dropped before `reload_config().await` so the hot-reload step does not gate other config-writing handlers.
    //
    //    Include-file detection, the `secrets.env` membership read, and all durable writes run as one serialized blocking task.
    //    Keeping include detection under the lock also prevents another config writer from changing the include list between the check and write.
    let shadowed_secrets = {
        let _config_guard = state.config_write_lock.lock().await;
        let write_instance_name = instance_name.clone();
        tokio::task::spawn_blocking(move || {
            write_sidecar_configuration(
                &config_path,
                &secrets_path,
                &write_instance_name,
                entry,
                &schema,
                &body.values,
                agent.as_deref(),
            )
        })
        .await
        .map_err(|e| {
            ApiErrorResponse::internal_scrub(format!("sidecar configure task failed: {e}"))
                .into_json_tuple()
        })?
        .map_err(|error| match error {
            ConfigureSidecarWriteError::IncludedSidecars(paths) => {
                let files = paths
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                ApiErrorResponse::conflict(format!(
                    "config.toml uses `include` directive and existing `[[sidecar_channels]]` entries live in {files}. Edit that file directly to avoid silently shadowing the included sidecars."
                ))
                .into_json_tuple()
            }
            ConfigureSidecarWriteError::NameConflict(conflicting_type) => {
                ApiErrorResponse::conflict(format!(
                    "instance name `{instance_name}` is already used by a configured `{conflicting_type}` channel. Pick a different instance name."
                ))
                .into_json_tuple()
            }
            ConfigureSidecarWriteError::MissingRequiredSecret(key) => {
                ApiErrorResponse::bad_request(format!(
                    "required field `{key}` is missing or empty"
                ))
                .into_json_tuple()
            }
            ConfigureSidecarWriteError::SecretPrefixConflict { prefix, names } => {
                ApiErrorResponse::conflict(format!(
                    "instance name `{instance_name}` normalizes to the same secret namespace `{prefix}__` as the configured instance(s) {}. Saving it would overwrite their `{prefix}__<KEY>` secrets in secrets.env. Pick a name that differs by more than punctuation or case.",
                    names
                        .iter()
                        .map(|n| format!("`{n}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
                .with_code("sidecar_secret_prefix_conflict")
                .into_json_tuple()
            }
            ConfigureSidecarWriteError::Write(error) => {
                ApiErrorResponse::internal_scrub(error).into_json_tuple()
            }
        })?
    };

    // 6. Trigger hot-reload. The kernel diffs the on-disk config
    //    against the live snapshot and returns the resulting plan;
    //    the dashboard surfaces `restart_required` so the operator
    //    knows whether further action is needed.
    let plan = state
        .kernel
        .reload_config()
        .await
        .map_err(|e| ApiErrorResponse::internal_scrub(e).into_json_tuple())?;

    // 7. When the plan emits `ReloadChannels`, the kernel has already
    //    cleared `mesh.channel_adapters` — but the supervisor map is
    //    only re-populated by re-entering `start_channel_bridge_with_config`
    //    via `channel_bridge::reload_channels_from_disk`. Without this
    //    follow-up the [[sidecar_channels]] entry we just wrote stays
    //    on disk only and no sidecar process is spawned until daemon
    //    restart — silently breaking the operator's expectation that
    //    `hot_actions_applied: [ReloadChannels]` means a new sidecar
    //    is live. Mirrors `routes/config.rs::config_reload` and
    //    `routes/channels.rs::configure_channel`.
    if plan
        .hot_actions
        .contains(&librefang_kernel::config_reload::HotAction::ReloadChannels)
    {
        if let Err(e) = crate::channel_bridge::reload_channels_from_disk(&state).await {
            tracing::error!("sidecar configure: bridge restart failed: {e}");
            return Err(ApiErrorResponse::internal(format!(
                "saved config.toml but bridge restart failed: {e}"
            ))
            .into_json_tuple());
        }
    }

    Ok(Json(serde_json::json!({
        "status": "saved",
        "hot_actions_applied": plan
            .hot_actions
            .iter()
            .map(|a| format!("{a:?}"))
            .collect::<Vec<_>>(),
        "restart_required": plan.restart_required,
        "shadowed_secrets": shadowed_secrets,
    })))
}

/// `DELETE /api/channels/sidecar/{name}` — remove a configured sidecar channel and stop its child process.
#[utoipa::path(
    delete,
    path = "/api/channels/sidecar/{name}",
    tag = "channels",
    params(
        ("name" = String, Path, description = "Configured sidecar channel name to remove")
    ),
    responses(
        (status = 200, description = "Removed; reload plan returned. Body fields: `status` (\"removed\"), `hot_actions_applied` ([String]), `restart_required` (bool).", body = crate::types::JsonObject),
        (status = 404, description = "No configured sidecar channel with that name", body = crate::types::JsonObject),
        (status = 423, description = "Configuration is managed by the deployment; remove the entry from the manifest instead.", body = crate::types::JsonObject)
    )
)]
pub async fn delete_sidecar_channel(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, Json<serde_json::Value>)> {
    // Managed mode (#6695) — refused before the rewrite, so the `[[sidecar_channels]]` block a manifest declared cannot be deleted out from under it.
    // The guard precedes the 404 branch on purpose: whether the entry exists is a fact about the managed file, and answering `404` first would tell a caller which manifest entries are present through a route that is not allowed to act on any of them.
    if let Some(locked) = crate::routes::guard_config_write(state.kernel.config_path()) {
        return Err(locked);
    }

    let config_path = state.kernel.config_path().to_path_buf();

    // Rewrite config.toml under the same lock that gates configure and POST /api/config/set.
    let removed = {
        let _config_guard = state.config_write_lock.lock().await;
        // `config_path` isn't read again after this block, so move it straight into the
        // blocking task instead of cloning; `name` is still needed below for the 404 message.
        let remove_name = name.clone();
        tokio::task::spawn_blocking(move || {
            super::sidecar_toml::remove_sidecar_block(&config_path, &remove_name)
        })
        .await
        .map_err(|e| {
            ApiErrorResponse::internal_scrub(format!("sidecar delete task failed: {e}"))
                .into_json_tuple()
        })?
        .map_err(|e| ApiErrorResponse::internal_scrub(e).into_json_tuple())?
    };
    if !removed {
        return Err(ApiErrorResponse::not_found(format!(
            "no configured sidecar channel named `{name}`"
        ))
        .into_json_tuple());
    }

    let plan = state
        .kernel
        .reload_config()
        .await
        .map_err(|e| ApiErrorResponse::internal_scrub(e).into_json_tuple())?;

    // Re-enter the bridge so the removed sidecar child is actually stopped, not just dropped from disk.
    if plan
        .hot_actions
        .contains(&librefang_kernel::config_reload::HotAction::ReloadChannels)
    {
        if let Err(e) = crate::channel_bridge::reload_channels_from_disk(&state).await {
            tracing::error!("sidecar delete: bridge restart failed: {e}");
            // Surface the actionable partial-failure signal (config WAS removed) but
            // not the raw error chain — the full `e` is already logged above.
            return Err(ApiErrorResponse::internal(
                "removed from config.toml but bridge restart failed",
            )
            .into_json_tuple());
        }
    }

    Ok(Json(serde_json::json!({
        "status": "removed",
        "hot_actions_applied": plan
            .hot_actions
            .iter()
            .map(|a| format!("{a:?}"))
            .collect::<Vec<_>>(),
        "restart_required": plan.restart_required,
    })))
}

#[cfg(test)]
mod included_sidecar_config_tests {
    use super::included_files_with_sidecars;

    #[tokio::test]
    async fn missing_root_config_has_no_included_sidecars() {
        let tmp = tempfile::tempdir().unwrap();
        let hits = included_files_with_sidecars(&tmp.path().join("config.toml"))
            .await
            .unwrap();
        assert!(hits.is_empty());
    }

    #[tokio::test]
    async fn unreadable_included_config_fails_closed() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("config.toml");
        tokio::fs::write(&root, "include = [\"missing.toml\"]\n")
            .await
            .unwrap();

        let error = included_files_with_sidecars(&root).await.unwrap_err();
        assert!(error.contains("failed to read included config"));
        assert!(error.contains("missing.toml"));
    }

    #[tokio::test]
    async fn invalid_root_config_fails_closed() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("config.toml");
        tokio::fs::write(&root, "include = [").await.unwrap();

        let error = included_files_with_sidecars(&root).await.unwrap_err();
        assert!(error.contains("failed to parse root config"));
    }

    #[tokio::test]
    async fn included_sidecar_blocks_are_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("config.toml");
        let included = tmp.path().join("channels.toml");
        tokio::fs::write(&root, "include = [\"channels.toml\"]\n")
            .await
            .unwrap();
        tokio::fs::write(&included, "[[sidecar_channels]]\nname = \"telegram\"\n")
            .await
            .unwrap();

        let hits = included_files_with_sidecars(&root).await.unwrap();
        assert_eq!(hits, vec![included]);
    }
}

/// Serialize a channel's config to a JSON Value for pre-populating dashboard forms.
/// GET /api/channels — List all 40 channel adapters with status and field metadata.
///
/// Envelope is the canonical `PaginatedResponse{items,total,offset,limit}`
/// shape used by `/api/agents`, `/api/peers`, `/api/skills`, etc. (#3842).
/// The full channel registry is materialized in-memory, so this is a single
/// page — `offset=0`, `limit=None`. The bespoke `configured_count` sibling
/// is preserved for the dashboard's "X of Y configured" sub-line.
//
// Row shape is documented on `sidecar_channel_rows`: configured rows carry
// per-instance liveness (`connected`, `started_at`, `last_message_at`,
// `messages_received`, `messages_sent`, `last_error`, `supervised`) plus a
// per-channel-TYPE `msgs_24h_channel_type` figure. Deliberately a plain
// comment rather than a doc line — the doc comment above is the source of this
// operation's `summary` / `description` in the generated `openapi.json`, and
// duplicating the row shape there would only add spec churn.
#[utoipa::path(
    get,
    path = "/api/channels",
    tag = "channels",
    responses(
        (status = 200, description = "List configured channels", body = crate::types::JsonObject)
    )
)]
pub async fn list_channels(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // 24h activity per channel TYPE — one grouped SQL pass for the whole
    // page; falls back to an empty map if the query fails so the listing
    // itself still loads. Keyed by `usage_events.channel`, which holds the
    // type, so every sidecar instance of a type shares the number; rows
    // publish it under `msgs_24h_channel_type` and per-bot traffic comes
    // from the supervisor counters instead (#6606).
    // Configured channels come from `sidecar_channel_rows`; unconfigured
    // catalog adapters come from `sidecar_discovery_rows`. The
    // in-process CHANNEL_REGISTRY loop that used to feed both is gone.
    let msgs_24h_by_type = state
        .kernel
        .memory_substrate()
        .usage()
        .channel_type_msgs_24h_bulk()
        .unwrap_or_default();
    let kcfg = state.kernel.config_ref();
    let secrets_env_keys = read_secrets_env_keys(state.kernel.home_dir());
    let configured_rows = sidecar_channel_rows(
        &kcfg.sidecar_channels,
        &msgs_24h_by_type,
        true,
        state.kernel.channel_adapters_ref(),
        &secrets_env_keys,
    );
    let configured_count = configured_rows.len() as u32;
    let mut channels = configured_rows;
    channels.extend(sidecar_discovery_rows());

    let total = channels.len();
    // Canonical PaginatedResponse envelope (#3842) hand-built so the bespoke
    // `configured_count` sibling can ride alongside `items`/`total`/`offset`/
    // `limit` without a new struct.
    Json(serde_json::json!({
        "items": channels,
        "total": total,
        "offset": 0,
        "limit": serde_json::Value::Null,
        "configured_count": configured_count,
    }))
}

/// Returns channels list for the dashboard snapshot endpoint.
pub(crate) async fn channels_snapshot(state: &Arc<AppState>) -> Vec<serde_json::Value> {
    // Same sidecar-only shape as `list_channels` above; just no
    // pagination envelope and the snapshot's caller doesn't care
    // about the per-channel-type 24h msg count. Per-instance liveness
    // rides along unconditionally — it is read from the in-memory
    // adapter map, so it costs the snapshot nothing. See
    // `list_channels` for the history of the in-process loop that this
    // used to mirror.
    let kcfg = state.kernel.config_ref();
    let secrets_env_keys = read_secrets_env_keys(state.kernel.home_dir());
    let mut channels = sidecar_channel_rows(
        &kcfg.sidecar_channels,
        &std::collections::HashMap::new(),
        false,
        state.kernel.channel_adapters_ref(),
        &secrets_env_keys,
    );
    channels.extend(sidecar_discovery_rows());
    channels
}

/// One-shot parse of `secrets.env` into the set of keys with a non-empty
/// value, for [`configured_instance_fields`]'s `has_value` check. Missing
/// file (no secrets configured yet) is not an error — same "nothing set"
/// outcome as an empty file.
fn read_secrets_env_keys(home_dir: &std::path::Path) -> std::collections::HashSet<String> {
    std::fs::read_to_string(home_dir.join("secrets.env"))
        .ok()
        .map(|content| {
            librefang_channels::sidecar::parse_secrets_env_contents(&content)
                .into_iter()
                .filter(|(_, value)| !value.is_empty())
                .map(|(key, _)| key)
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// In-process per-channel REST endpoints — DELETED
// ---------------------------------------------------------------------------
//
// `get_channel` (GET /api/channels/{name}), `configure_channel` (POST
// /api/channels/{name}/configure), `remove_channel` (DELETE same),
// `list_channel_instances` (GET /api/channels/{name}/instances),
// `create_channel_instance` (POST same), `update_channel_instance_handler`
// (PUT /api/channels/{name}/instances/{index}), `delete_channel_instance`
// (DELETE same), `test_channel` (POST /api/channels/{name}/test), plus
// helpers `build_instance_fields_json`, `resolve_secret_env_overrides`,
// `canonical_json`, `instance_signature`, `read_disk_channels`,
// `PreparedWrite` / `prepare_fields_write` / `apply_secret_writes`, and
// `send_channel_test_message` are gone.
//
// All nine endpoints already 404'd unconditionally after the in-process
// channel registry emptied (every handler started with
// `find_channel_meta(&name)?`-style early-return). Sidecar channels
// configure via `POST /api/channels/sidecar/{name}/configure`
// (`configure_sidecar_channel`, below) and surface via
// `list_channels` / `channels_snapshot` (above) which now read
// exclusively from `SIDECAR_CATALOG` + `[[sidecar_channels]]`.
#[utoipa::path(
    post,
    path = "/api/channels/reload",
    tag = "channels",
    responses(
        (status = 200, description = "Channels reloaded successfully", body = crate::types::JsonObject),
        (status = 500, description = "Reload failed", body = crate::types::JsonObject)
    )
)]
/// POST /api/channels/reload — Manually trigger a channel hot-reload from disk config.
pub async fn reload_channels(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match crate::channel_bridge::reload_channels_from_disk(&state).await {
        Ok(started) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ok",
                "started": started,
            })),
        ),
        Err(e) => ApiErrorResponse::internal(e).into_json_tuple(),
    }
}

// ---------------------------------------------------------------------------
// Single read-only QR projection — replaces the four
// pre-migration WhatsApp/WeChat endpoints with one endpoint that
// reads `ChannelStatus.qr` (populated by the supervisor from
// `qr_ready` / `qr_status` sidecar events; see `librefang-channels`
// `sidecar.rs` and `types.rs::QrState`).
// ---------------------------------------------------------------------------

/// GET /api/channels/{name}/qr — Return the latest QR-login state
/// published by the sidecar.
///
/// The sidecar drives the QR start/poll cycle itself and emits
/// `qr_ready` / `qr_status` events; this handler just reads the
/// cached `ChannelStatus.qr` and returns it to the dashboard.
///
/// Status codes:
/// - `200` — sidecar has published at least one QR event; payload is
///   the current `QrState` (which may be in any lifecycle phase).
/// - `204` — sidecar is running but has not published a QR session
///   yet (e.g. WeChat sidecar authenticated from a cached
///   `WECHAT_BOT_TOKEN`, no QR needed). The dashboard treats this as
///   "no scan required" and closes the dialog.
/// - `404` — no sidecar is currently registered under that name.
///   With the in-process registry retired, a "known channel name"
///   check would just duplicate "is there a running adapter?", so we
///   collapse the two cases — easier to read in a dashboard error
///   panel ("Sidecar not running") than two indistinguishable 404s.
#[utoipa::path(
    get,
    path = "/api/channels/{name}/qr",
    tag = "channels",
    params(
        ("name" = String, Path, description = "Channel adapter name (e.g. wechat, whatsapp)")
    ),
    responses(
        (status = 200, description = "QR-login state", body = crate::types::JsonObject),
        (status = 204, description = "Sidecar running, no QR session yet"),
        (status = 404, description = "Sidecar not running", body = crate::types::JsonObject)
    )
)]
pub async fn get_channel_qr(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let adapter = state.kernel.channel_adapters_ref().get(&name);
    let Some(adapter) = adapter else {
        return ApiErrorResponse::not_found(format!(
            "Sidecar for '{name}' is not running — start it from the dashboard first"
        ))
        .into_response();
    };
    let status = adapter.value().status();
    match status.qr {
        Some(qr) => (StatusCode::OK, Json(qr)).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    }
}

// ---------------------------------------------------------------------------
// Channel registry metadata — loaded from ~/.librefang/channels/*.toml
// ---------------------------------------------------------------------------

/// Return channel metadata from the registry (synced from librefang-registry).
///
/// `GET /api/channels/registry`
#[utoipa::path(
    get,
    path = "/api/channels/registry",
    tag = "channels",
    responses(
        (status = 200, description = "Channel metadata from registry", body = Vec<serde_json::Value>)
    )
)]
pub async fn list_channel_registry(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let channels_dir = state.kernel.home_dir().join("channels");
    let metadata = librefang_kernel::channel_registry::load_channel_metadata(&channels_dir);
    Json(metadata)
}

// `test_channel_status_tests` + `instance_helper_tests` modules
// removed entirely. The former tested the `test_channel` HTTP
// handler (deleted with the in-process-channel scaffolding); the
// latter tested `instance_signature` + `resolve_secret_env_overrides`
// (both deleted with their only callers, the per-instance REST
// handlers).

#[cfg(test)]
mod static_schema_tests {
    use super::{FEISHU_STATIC_FIELDS, SIDECAR_CATALOG, TELEGRAM_STATIC_FIELDS};

    #[test]
    fn telegram_catalog_entry_has_static_fields() {
        let entry = SIDECAR_CATALOG
            .iter()
            .find(|e| e.name == "telegram")
            .expect("telegram must be in SIDECAR_CATALOG");
        assert_eq!(
            entry
                .static_fields
                .expect("telegram catalog entry must have static_fields set")
                .len(),
            3,
            "static fields must match TelegramAdapter.SCHEMA"
        );
    }

    #[test]
    fn telegram_static_fields_match_schema_contract() {
        let fields: Vec<(&str, &str, bool, bool)> = TELEGRAM_STATIC_FIELDS
            .iter()
            .map(|f| (f.key, f.field_type, f.required, f.advanced))
            .collect();
        assert_eq!(
            fields,
            vec![
                ("TELEGRAM_BOT_TOKEN", "secret", true, false),
                ("ALLOWED_USERS", "list", false, true),
                ("TELEGRAM_CLEAR_DONE_REACTION", "bool", false, true),
            ]
        );
        let allowed_users = TELEGRAM_STATIC_FIELDS
            .iter()
            .find(|field| field.key == "ALLOWED_USERS")
            .expect("Telegram fallback must declare ALLOWED_USERS");
        assert!(
            allowed_users.placeholder.contains("empty")
                && allowed_users.placeholder.contains("ALL users")
                && allowed_users.placeholder.contains("insecure"),
            "fallback schema must disclose blank permit-all behavior"
        );
    }

    /// The Feishu catalog entry must declare FEISHU_APP_ID + FEISHU_APP_SECRET
    /// as required non-advanced fields and the remaining six as optional
    /// advanced fields so the dashboard configure form shows the required
    /// inputs by default and hides the rest under "Show advanced".
    #[test]
    fn feishu_catalog_entry_has_static_fields() {
        let entry = SIDECAR_CATALOG
            .iter()
            .find(|e| e.name == "feishu")
            .expect("feishu must be in SIDECAR_CATALOG");
        let fields = entry
            .static_fields
            .expect("feishu catalog entry must have static_fields set");
        assert_eq!(
            fields.len(),
            8,
            "expected 8 static fields matching FeishuAdapter.SCHEMA.fields"
        );
    }

    #[test]
    fn feishu_static_fields_required_set_is_correct() {
        let required: Vec<&str> = FEISHU_STATIC_FIELDS
            .iter()
            .filter(|f| f.required)
            .map(|f| f.key)
            .collect();
        assert_eq!(
            required,
            vec!["FEISHU_APP_ID", "FEISHU_APP_SECRET"],
            "only FEISHU_APP_ID and FEISHU_APP_SECRET are required"
        );
    }

    #[test]
    fn feishu_static_fields_advanced_set_is_correct() {
        let advanced: Vec<&str> = FEISHU_STATIC_FIELDS
            .iter()
            .filter(|f| f.advanced)
            .map(|f| f.key)
            .collect();
        assert_eq!(
            advanced,
            vec![
                "FEISHU_REGION",
                "FEISHU_RECEIVE_MODE",
                "FEISHU_WEBHOOK_PORT",
                "FEISHU_VERIFICATION_TOKEN",
                "FEISHU_ENCRYPT_KEY",
                "FEISHU_ACCOUNT_ID",
            ],
            "optional advanced fields must match FeishuAdapter.SCHEMA"
        );
    }

    #[test]
    fn feishu_static_fields_secret_type_set_is_correct() {
        let secrets: Vec<&str> = FEISHU_STATIC_FIELDS
            .iter()
            .filter(|f| f.field_type == "secret")
            .map(|f| f.key)
            .collect();
        assert_eq!(
            secrets,
            vec![
                "FEISHU_APP_SECRET",
                "FEISHU_VERIFICATION_TOKEN",
                "FEISHU_ENCRYPT_KEY",
            ],
            "secret-typed fields must match FeishuAdapter.SCHEMA"
        );
    }
}

#[cfg(test)]
mod schema_error_discovery_tests {
    use super::{
        __test_seed_sidecar_schema_cache, __test_seed_sidecar_schema_error_cache,
        sidecar_discovery_rows, SidecarSchema, SidecarSchemaField,
    };

    // Both assertions live in ONE test: the schema / error caches are process-wide, and the seeders clear-then-set, so running the two halves as separate (parallel) tests would race on the shared maps.
    #[test]
    fn discovery_row_surfaces_schema_error_only_when_schema_missing() {
        const HINT: &str = "librefang-sdk is not installed (test hint)";

        // --- describe failed, no static fallback: row carries the reason ---
        __test_seed_sidecar_schema_cache(&[]);
        __test_seed_sidecar_schema_error_cache(&[("wechat", HINT.to_string())]);
        let rows = sidecar_discovery_rows();
        let wechat = rows
            .iter()
            .find(|r| r["name"] == "wechat")
            .expect("wechat discovery row must be present");
        assert_eq!(
            wechat["fields"].as_array().map(|a| a.len()),
            Some(0),
            "no cached schema → empty fields"
        );
        assert_eq!(
            wechat["schema_error"], HINT,
            "the cached failure reason must ride along as schema_error"
        );
        assert!(
            wechat.get("sdk_version").is_none(),
            "a failed describe reported no SDK version, so the key must be absent rather than null"
        );

        // --- schema cached: no schema_error, fields populated ---
        let schema = SidecarSchema {
            name: "wechat".to_string(),
            display_name: "WeChat".to_string(),
            description: "test".to_string(),
            sdk_version: Some("2026.8.19".to_string()),
            fields: vec![SidecarSchemaField {
                key: "WECHAT_BOT_TOKEN".to_string(),
                label: "Bot token".to_string(),
                field_type: "secret".to_string(),
                required: true,
                placeholder: String::new(),
                advanced: false,
                options: None,
            }],
        };
        __test_seed_sidecar_schema_cache(&[("wechat", schema)]);
        __test_seed_sidecar_schema_error_cache(&[]);
        let rows = sidecar_discovery_rows();
        let wechat = rows
            .iter()
            .find(|r| r["name"] == "wechat")
            .expect("wechat discovery row must be present");
        assert_eq!(
            wechat["fields"].as_array().map(|a| a.len()),
            Some(1),
            "cached schema → fields populated"
        );
        assert!(
            wechat.get("schema_error").is_none(),
            "a usable schema must not carry a schema_error"
        );
        assert_eq!(
            wechat["sdk_version"], "2026.8.19",
            "the adapter's reported SDK version must reach the discovery row"
        );

        // Reset shared caches so we don't leak state into other tests.
        __test_seed_sidecar_schema_cache(&[]);
        __test_seed_sidecar_schema_error_cache(&[]);
    }
}

#[cfg(test)]
mod schema_cache_poison_tests {
    use super::{read_cache_recover, write_cache_recover};
    use std::collections::HashMap;
    use std::sync::RwLock;

    #[test]
    fn cache_helpers_recover_reads_and_writes_after_held_lock_panics() {
        let read_recovery = RwLock::new(HashMap::from([("telegram", "schema")]));
        let _ = std::panic::catch_unwind(|| {
            let _guard = read_recovery.write().unwrap();
            panic!("poison cache before recovered read");
        });
        assert!(read_recovery.is_poisoned());

        let guard = read_cache_recover(&read_recovery, "test read cache");
        assert_eq!(guard.get("telegram"), Some(&"schema"));
        drop(guard);
        assert!(!read_recovery.is_poisoned());
        assert!(read_recovery.read().is_ok());

        let write_recovery = RwLock::new(HashMap::from([("wechat", "error")]));
        let _ = std::panic::catch_unwind(|| {
            let _guard = write_recovery.write().unwrap();
            panic!("poison cache before recovered write");
        });
        assert!(write_recovery.is_poisoned());

        write_cache_recover(&write_recovery, "test write cache").insert("feishu", "fallback");
        assert!(!write_recovery.is_poisoned());
        let guard = write_recovery.read().unwrap();
        assert_eq!(guard.get("wechat"), Some(&"error"));
        assert_eq!(guard.get("feishu"), Some(&"fallback"));
    }
}

#[cfg(test)]
mod sidecar_configuration_write_tests {
    use super::{
        write_sidecar_configuration, ConfigureSidecarWriteError, SidecarCatalogEntry,
        SidecarSchema, SidecarSchemaField,
    };
    use std::collections::HashMap;

    static TEST_ENTRY: SidecarCatalogEntry = SidecarCatalogEntry {
        name: "test-sidecar",
        display_name: "Test Sidecar",
        description: "test",
        command: "test-command",
        args: &["--serve"],
        static_fields: None,
    };

    fn schema() -> SidecarSchema {
        SidecarSchema {
            name: TEST_ENTRY.name.to_string(),
            display_name: TEST_ENTRY.display_name.to_string(),
            description: TEST_ENTRY.description.to_string(),
            sdk_version: Some("2026.8.19".to_string()),
            fields: vec![
                SidecarSchemaField {
                    key: "TEST_TOKEN".to_string(),
                    label: "Token".to_string(),
                    field_type: "secret".to_string(),
                    required: true,
                    placeholder: String::new(),
                    advanced: false,
                    options: None,
                },
                SidecarSchemaField {
                    key: "ROOM".to_string(),
                    label: "Room".to_string(),
                    field_type: "text".to_string(),
                    required: false,
                    placeholder: String::new(),
                    advanced: false,
                    options: None,
                },
            ],
        }
    }

    #[test]
    fn writes_secret_and_nonsecret_configuration() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let secrets_path = dir.path().join("secrets.env");
        let values = HashMap::from([
            ("TEST_TOKEN".to_string(), "secret-value".to_string()),
            ("ROOM".to_string(), "alerts".to_string()),
        ]);

        write_sidecar_configuration(
            &config_path,
            &secrets_path,
            TEST_ENTRY.name,
            &TEST_ENTRY,
            &schema(),
            &values,
            None,
        )
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(&secrets_path).unwrap(),
            "TEST_TOKEN=secret-value\n"
        );
        let config = std::fs::read_to_string(&config_path).unwrap();
        assert!(config.contains("name = \"test-sidecar\""));
        assert!(config.contains("ROOM = \"alerts\""));
        assert!(!config.contains("TEST_TOKEN"));
    }

    /// An omitted required secret is satisfied by a *stored value*, not by a
    /// bare key sitting in secrets.env.
    ///
    /// `parse_secrets_env_contents` yields `("TEST_TOKEN", "")` for a
    /// hand-edited `TEST_TOKEN=` line, and `read_secrets_env_keys` — the GET
    /// path that computes the row's `has_value` — filters exactly that out. If
    /// the required-secret check keyed on key presence instead, the drawer would
    /// show the field as empty and required while the save it rejects came back
    /// 200, and the adapter would then spawn with an empty token.
    #[test]
    fn empty_stored_secret_does_not_satisfy_a_required_field() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let secrets_path = dir.path().join("secrets.env");
        std::fs::write(&secrets_path, "TEST_TOKEN=\nKEEP_ME=unchanged\n").unwrap();
        let values = HashMap::from([("ROOM".to_string(), "alerts".to_string())]);

        let result = write_sidecar_configuration(
            &config_path,
            &secrets_path,
            TEST_ENTRY.name,
            &TEST_ENTRY,
            &schema(),
            &values,
            None,
        );

        assert!(
            matches!(
                &result,
                Err(ConfigureSidecarWriteError::MissingRequiredSecret(key)) if key == "TEST_TOKEN"
            ),
            "{result:?}"
        );
        // Rejected before the first mutation, same contract as every other
        // pre-write refusal in this function.
        assert!(!config_path.exists());
        assert_eq!(
            std::fs::read_to_string(&secrets_path).unwrap(),
            "TEST_TOKEN=\nKEEP_ME=unchanged\n"
        );
    }

    /// The other half: a required secret that really is stored needs no
    /// re-paste, which is the #8063 relaxation itself.
    #[test]
    fn stored_secret_satisfies_an_omitted_required_field() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let secrets_path = dir.path().join("secrets.env");
        std::fs::write(&secrets_path, "TEST_TOKEN=already-stored\n").unwrap();
        let values = HashMap::from([("ROOM".to_string(), "alerts".to_string())]);

        write_sidecar_configuration(
            &config_path,
            &secrets_path,
            TEST_ENTRY.name,
            &TEST_ENTRY,
            &schema(),
            &values,
            None,
        )
        .expect("an omitted-but-stored required secret must not block the save");

        assert_eq!(
            std::fs::read_to_string(&secrets_path).unwrap(),
            "TEST_TOKEN=already-stored\n",
            "the omitted secret must be left exactly as it was"
        );
        assert!(std::fs::read_to_string(&config_path)
            .unwrap()
            .contains("ROOM = \"alerts\""));
    }

    #[test]
    fn included_sidecar_conflict_prevents_all_writes() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let included_path = dir.path().join("channels.toml");
        let secrets_path = dir.path().join("secrets.env");
        std::fs::write(&config_path, "include = [\"channels.toml\"]\n").unwrap();
        std::fs::write(
            &included_path,
            "[[sidecar_channels]]\nname = \"existing\"\n",
        )
        .unwrap();
        let values = HashMap::from([("TEST_TOKEN".to_string(), "secret-value".to_string())]);

        let result = write_sidecar_configuration(
            &config_path,
            &secrets_path,
            TEST_ENTRY.name,
            &TEST_ENTRY,
            &schema(),
            &values,
            None,
        );

        assert!(matches!(
            result,
            Err(ConfigureSidecarWriteError::IncludedSidecars(paths))
                if paths == vec![included_path]
        ));
        assert!(!secrets_path.exists());
        assert_eq!(
            std::fs::read_to_string(config_path).unwrap(),
            "include = [\"channels.toml\"]\n"
        );
    }

    #[test]
    fn config_write_failure_restores_existing_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let secrets_path = dir.path().join("secrets.env");
        let original_config = "sidecar_channels = \"not-an-array\"\n";
        let original_secrets = "TEST_TOKEN=old-value\nKEEP_ME=unchanged\n";
        std::fs::write(&config_path, original_config).unwrap();
        std::fs::write(&secrets_path, original_secrets).unwrap();
        let values = HashMap::from([("TEST_TOKEN".to_string(), "new-value".to_string())]);

        let result = write_sidecar_configuration(
            &config_path,
            &secrets_path,
            TEST_ENTRY.name,
            &TEST_ENTRY,
            &schema(),
            &values,
            None,
        );

        assert!(matches!(result, Err(ConfigureSidecarWriteError::Write(_))));
        assert_eq!(
            std::fs::read_to_string(config_path).unwrap(),
            original_config
        );
        assert_eq!(
            std::fs::read_to_string(secrets_path).unwrap(),
            original_secrets
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(dir.path().join("secrets.env"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn config_write_failure_removes_new_secrets_file() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let secrets_path = dir.path().join("secrets.env");
        let original_config = "sidecar_channels = \"not-an-array\"\n";
        std::fs::write(&config_path, original_config).unwrap();
        let values = HashMap::from([("TEST_TOKEN".to_string(), "new-value".to_string())]);

        let result = write_sidecar_configuration(
            &config_path,
            &secrets_path,
            TEST_ENTRY.name,
            &TEST_ENTRY,
            &schema(),
            &values,
            None,
        );

        assert!(matches!(result, Err(ConfigureSidecarWriteError::Write(_))));
        assert_eq!(
            std::fs::read_to_string(config_path).unwrap(),
            original_config
        );
        assert!(!secrets_path.exists());
    }

    static TEST_ENTRY_2: SidecarCatalogEntry = SidecarCatalogEntry {
        name: "other-sidecar",
        display_name: "Other Sidecar",
        description: "test",
        command: "other-command",
        args: &["--serve"],
        static_fields: None,
    };

    /// Multi-instance support: two `[[sidecar_channels]]` of the same
    /// catalog type, distinguished only by `name`, must coexist with
    /// independent env values and independent `agent` bindings.
    #[test]
    fn writes_two_named_instances_of_the_same_catalog_type() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let secrets_path = dir.path().join("secrets.env");

        write_sidecar_configuration(
            &config_path,
            &secrets_path,
            "test-sidecar-a",
            &TEST_ENTRY,
            &schema(),
            &HashMap::from([
                ("TEST_TOKEN".to_string(), "token-a".to_string()),
                ("ROOM".to_string(), "room-a".to_string()),
            ]),
            Some("agent-a"),
        )
        .unwrap();
        write_sidecar_configuration(
            &config_path,
            &secrets_path,
            "test-sidecar-b",
            &TEST_ENTRY,
            &schema(),
            &HashMap::from([
                ("TEST_TOKEN".to_string(), "token-b".to_string()),
                ("ROOM".to_string(), "room-b".to_string()),
            ]),
            Some("agent-b"),
        )
        .unwrap();

        let config = std::fs::read_to_string(&config_path).unwrap();
        assert!(config.contains("name = \"test-sidecar-a\""));
        assert!(config.contains("name = \"test-sidecar-b\""));
        assert_eq!(
            config.matches("channel_type = \"test-sidecar\"").count(),
            2,
            "both instances share the catalog channel_type: {config}"
        );
        assert!(config.contains("ROOM = \"room-a\""));
        assert!(config.contains("ROOM = \"room-b\""));
        assert!(config.contains("agent = \"agent-a\""));
        assert!(config.contains("agent = \"agent-b\""));

        let secrets = std::fs::read_to_string(&secrets_path).unwrap();
        assert!(secrets.contains("TEST_TOKEN=token-b"));
    }

    /// Re-saving the same instance with `agent: None` must clear the
    /// field rather than leaving the previous value stuck.
    #[test]
    fn clears_agent_when_omitted_on_a_resave() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let secrets_path = dir.path().join("secrets.env");
        let values = HashMap::from([("TEST_TOKEN".to_string(), "secret-value".to_string())]);

        write_sidecar_configuration(
            &config_path,
            &secrets_path,
            TEST_ENTRY.name,
            &TEST_ENTRY,
            &schema(),
            &values,
            Some("some-agent"),
        )
        .unwrap();
        assert!(std::fs::read_to_string(&config_path)
            .unwrap()
            .contains("agent = \"some-agent\""));

        write_sidecar_configuration(
            &config_path,
            &secrets_path,
            TEST_ENTRY.name,
            &TEST_ENTRY,
            &schema(),
            &values,
            None,
        )
        .unwrap();
        let config = std::fs::read_to_string(&config_path).unwrap();
        assert!(!config.contains("agent ="), "agent cleared: {config}");
    }

    /// A second catalog type may not steal an instance name already owned
    /// by a different type — that would silently reassign the existing
    /// bot's block to a new command/schema on the very next save.
    #[test]
    fn rejects_instance_name_already_used_by_a_different_channel_type() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let secrets_path = dir.path().join("secrets.env");
        let values = HashMap::from([("TEST_TOKEN".to_string(), "secret-value".to_string())]);

        write_sidecar_configuration(
            &config_path,
            &secrets_path,
            "shared-name",
            &TEST_ENTRY,
            &schema(),
            &values,
            None,
        )
        .unwrap();
        let before = std::fs::read_to_string(&config_path).unwrap();

        let result = write_sidecar_configuration(
            &config_path,
            &secrets_path,
            "shared-name",
            &TEST_ENTRY_2,
            &schema(),
            &values,
            None,
        );

        assert!(matches!(
            result,
            Err(ConfigureSidecarWriteError::NameConflict(conflicting_type))
                if conflicting_type == "test-sidecar"
        ));
        assert_eq!(
            std::fs::read_to_string(&config_path).unwrap(),
            before,
            "rejected save must not touch config.toml"
        );
    }

    /// Same instance name AND same channel_type is the ordinary update
    /// path, not a conflict.
    #[test]
    fn same_instance_name_and_type_is_an_update_not_a_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let secrets_path = dir.path().join("secrets.env");

        write_sidecar_configuration(
            &config_path,
            &secrets_path,
            "test-sidecar",
            &TEST_ENTRY,
            &schema(),
            &HashMap::from([("TEST_TOKEN".to_string(), "v1".to_string())]),
            None,
        )
        .unwrap();
        write_sidecar_configuration(
            &config_path,
            &secrets_path,
            "test-sidecar",
            &TEST_ENTRY,
            &schema(),
            &HashMap::from([
                ("TEST_TOKEN".to_string(), "v2".to_string()),
                ("ROOM".to_string(), "updated".to_string()),
            ]),
            None,
        )
        .unwrap();

        let config = std::fs::read_to_string(&config_path).unwrap();
        assert_eq!(
            config.matches("name = \"test-sidecar\"").count(),
            1,
            "update in place, not a second block: {config}"
        );
        assert!(config.contains("ROOM = \"updated\""));
    }
}
