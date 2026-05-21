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
        .route(
            "/channels/registry",
            axum::routing::get(list_channel_registry),
        )
        .route(
            "/channels/sidecar/{name}/configure",
            axum::routing::post(configure_sidecar_channel),
        )
}

use super::sidecar_describe::{describe_sidecar, SidecarSchema};
// `super::skills` channel helpers (upsert_channel_config /
// remove_channel_config / append_channel_instance /
// update_channel_instance / remove_channel_instance /
// CHANNEL_AOT_CONFLICT_PREFIX / validate_env_var / write_secret_env /
// remove_secret_env) were used by the deleted in-process channel
// REST endpoints. None of the surviving handlers
// (`list_channels`, `channels_snapshot`, `reload_channels`,
// `list_channel_registry`, `configure_sidecar_channel`) call any of
// them; the helpers themselves are now orphaned in skills.rs and
// should be retired in a follow-up.
use super::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

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
/// (`[[sidecar_channels]]`, also under Config -> Sidecar Channels), so
/// they carry no editable `fields`; the page renders them as
/// configured/online cards (it conditionally hides empty
/// `fields`/`setup_steps`).
fn sidecar_channel_rows(
    sidecar: &[librefang_types::config::SidecarChannelConfig],
    msgs_24h: &std::collections::HashMap<String, u64>,
    with_msgs: bool,
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
    for sc in sidecar {
        let name = sc.name.as_str();
        // One card per distinct sidecar name.
        if !seen.insert(name) {
            continue;
        }
        let channel_type = sc.channel_type.as_deref().unwrap_or(name);
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
            "fields": Vec::<serde_json::Value>::new(),
            "setup_steps": [
                "Runs as an out-of-process sidecar adapter",
                "Configured via [[sidecar_channels]] in config.toml \
                 (Config \u{2192} Sidecar Channels)",
            ],
            "config_template": format!(
                "[[sidecar_channels]]\nname = \"{name}\"\nchannel_type = \"{channel_type}\""
            ),
        });
        if with_msgs {
            let m = msgs_24h
                .get(channel_type)
                .or_else(|| msgs_24h.get(name))
                .copied()
                .unwrap_or(0);
            row["msgs_24h"] = serde_json::json!(m);
        }
        rows.push(row);
    }
    rows
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
const SIDECAR_CATALOG: &[SidecarCatalogEntry] = &[
    SidecarCatalogEntry {
        name: "telegram",
        display_name: "Telegram",
        description: "Telegram Bot API adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.telegram"],
    },
    SidecarCatalogEntry {
        name: "ntfy",
        display_name: "ntfy",
        description: "ntfy.sh pub/sub notifications (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.ntfy"],
    },
    SidecarCatalogEntry {
        name: "gotify",
        display_name: "Gotify",
        description: "Gotify push notifications (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.gotify"],
    },
    SidecarCatalogEntry {
        name: "mastodon",
        display_name: "Mastodon",
        description: "Mastodon Streaming API (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.mastodon"],
    },
    SidecarCatalogEntry {
        name: "bluesky",
        display_name: "Bluesky",
        description: "Bluesky / AT Protocol adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.bluesky"],
    },
    SidecarCatalogEntry {
        name: "reddit",
        display_name: "Reddit",
        description: "Reddit OAuth2 API adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.reddit"],
    },
    SidecarCatalogEntry {
        name: "twitch",
        display_name: "Twitch",
        description: "Twitch IRC gateway adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.twitch"],
    },
    SidecarCatalogEntry {
        name: "rocketchat",
        display_name: "Rocket.Chat",
        description: "Rocket.Chat REST API adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.rocketchat"],
    },
    SidecarCatalogEntry {
        name: "discord",
        display_name: "Discord",
        description: "Discord Gateway bot adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.discord"],
    },
    SidecarCatalogEntry {
        name: "nextcloud",
        display_name: "Nextcloud Talk",
        description: "Nextcloud Talk OCS REST adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.nextcloud"],
    },
    SidecarCatalogEntry {
        name: "slack",
        display_name: "Slack",
        description: "Slack Socket Mode bot adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.slack"],
    },
    SidecarCatalogEntry {
        name: "webex",
        display_name: "Webex",
        description: "Cisco Webex bot adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.webex"],
    },
    SidecarCatalogEntry {
        name: "line",
        display_name: "LINE",
        description: "LINE Messaging API adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.line"],
    },
    SidecarCatalogEntry {
        name: "zulip",
        display_name: "Zulip",
        description: "Zulip REST + event-queue long-poll adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.zulip"],
    },
    SidecarCatalogEntry {
        name: "mattermost",
        display_name: "Mattermost",
        description: "Mattermost WebSocket + REST adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.mattermost"],
    },
    SidecarCatalogEntry {
        name: "signal",
        display_name: "Signal",
        description: "signal-cli REST API adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.signal"],
    },
    SidecarCatalogEntry {
        name: "qq",
        display_name: "QQ Bot",
        description: "QQ Bot API v2 WebSocket + REST adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.qq"],
    },
    SidecarCatalogEntry {
        name: "matrix",
        display_name: "Matrix",
        description: "Matrix Client-Server API adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.matrix"],
    },
    SidecarCatalogEntry {
        name: "feishu",
        display_name: "Feishu / Lark",
        description: "Feishu/Lark Open Platform adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.feishu"],
    },
    SidecarCatalogEntry {
        name: "wecom",
        display_name: "WeCom",
        description: "WeCom (\u{4f01}\u{4e1a}\u{5fae}\u{4fe1}) intelligent-bot WebSocket adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.wecom"],
    },
    SidecarCatalogEntry {
        name: "email",
        display_name: "Email (IMAP + SMTP)",
        description: "IMAP / SMTP email adapter (out-of-process sidecar, Python stdlib only)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.email"],
    },
    SidecarCatalogEntry {
        name: "dingtalk",
        display_name: "DingTalk",
        description: "DingTalk (\u{9489}\u{9489}) Robot stream-mode adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.dingtalk"],
    },
    SidecarCatalogEntry {
        name: "wechat",
        display_name: "WeChat",
        description: "WeChat personal-account adapter via the iLink (ClawBot) gateway (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.wechat"],
    },
    SidecarCatalogEntry {
        name: "teams",
        display_name: "Microsoft Teams",
        description: "Teams Bot Framework v3 adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.teams"],
    },
    SidecarCatalogEntry {
        name: "whatsapp",
        display_name: "WhatsApp",
        description: "WhatsApp adapter — Meta Cloud API + Web/QR (Baileys) gateway dual-mode (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.whatsapp"],
    },
    SidecarCatalogEntry {
        name: "webhook",
        display_name: "Webhook",
        description: "Generic HMAC-signed HTTP webhook adapter (out-of-process sidecar, Python stdlib only)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.webhook"],
    },
    SidecarCatalogEntry {
        name: "google_chat",
        display_name: "Google Chat",
        description: "Google Chat adapter — service-account JWT auth + REST API send, HTTP webhook receive (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.google_chat"],
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

/// Spawn `<command> <args> --describe` for every catalog entry and cache
/// the resulting schemas. Called once at daemon boot from
/// `server::build_router`. Failures (SDK not installed, describe crashed)
/// are logged at WARN and the row falls back to an empty `fields[]` — the
/// operator then sees the description + setup-steps text but no form.
/// This keeps daemon boot resilient in dev environments that have not
/// run `pip install -e sdk/python`.
pub async fn populate_sidecar_schema_cache() {
    for entry in SIDECAR_CATALOG {
        let args: Vec<String> = entry.args.iter().map(|s| s.to_string()).collect();
        match describe_sidecar(entry.command, &args).await {
            Ok(schema) => {
                tracing::info!(
                    adapter = entry.name,
                    fields = schema.fields.len(),
                    "sidecar schema cached"
                );
                schema_cache().write().unwrap().insert(entry.name, schema);
            }
            Err(e) => {
                tracing::warn!(
                    adapter = entry.name,
                    error = %e,
                    "sidecar --describe failed; discovery card will have no form fields"
                );
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
    let mut guard = schema_cache().write().unwrap();
    guard.clear();
    for (k, v) in entries {
        guard.insert(*k, v.clone());
    }
}

/// Synthesize **unconfigured** dashboard rows for catalog sidecar
/// adapters (`telegram`, `ntfy`) so they remain discoverable in the
/// Add picker after the out-of-process migration. A catalog entry is
/// suppressed when ANY `[[sidecar_channels]]` already has a matching
/// `channel_type` (or, when `channel_type` is unset, a matching `name`)
/// — i.e. once the operator has set up "telegram" under whatever local
/// alias, the discovery card has done its job and should yield to the
/// configured rows emitted by [`sidecar_channel_rows`].
fn sidecar_discovery_rows(
    sidecar: &[librefang_types::config::SidecarChannelConfig],
) -> Vec<serde_json::Value> {
    // The historical in-process `CHANNEL_REGISTRY` shadow check is
    // gone (registry is deleted; every channel runs as a sidecar).
    // Only suppress catalog rows whose channel name is already
    // covered by a configured `[[sidecar_channels]]` entry.
    let mut covered: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for sc in sidecar {
        let kind = sc.channel_type.as_deref().unwrap_or(sc.name.as_str());
        covered.insert(kind);
        covered.insert(sc.name.as_str());
    }

    let cache_guard = schema_cache().read().unwrap();
    let mut rows = Vec::new();
    for entry in SIDECAR_CATALOG {
        if covered.contains(entry.name) {
            continue;
        }
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

        rows.push(serde_json::json!({
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
        }));
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
/// header. Empty list = safe to write to root.
fn included_files_with_sidecars(config_path: &std::path::Path) -> Vec<std::path::PathBuf> {
    let content = match std::fs::read_to_string(config_path) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let doc: toml_edit::DocumentMut = match content.parse() {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    // `include` may be a string array at the document root.
    let include_arr = match doc.get("include").and_then(|i| i.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };
    let parent = config_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let mut hits = Vec::new();
    for entry in include_arr.iter() {
        let raw = match entry.as_str() {
            Some(s) => s,
            None => continue,
        };
        let path = if std::path::Path::new(raw).is_absolute() {
            std::path::PathBuf::from(raw)
        } else {
            parent.join(raw)
        };
        if let Ok(body) = std::fs::read_to_string(&path) {
            if body.contains("[[sidecar_channels]]") {
                hits.push(path);
            }
        }
    }
    hits
}

/// `POST /api/channels/sidecar/{name}/configure` — save schema-driven
/// sidecar form values, splitting the payload across `secrets.env` and
/// `config.toml`, then trigger a hot-reload so the kernel picks up the
/// new `[[sidecar_channels]]` block without a restart. `name` is the
/// `SIDECAR_CATALOG` key (`telegram`, `ntfy`, …).
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
        (status = 409, description = "config.toml uses `include` and an existing `[[sidecar_channels]]` entry lives in an included file — would silently shadow.", body = crate::types::JsonObject),
        (status = 503, description = "Schema not cached — SDK module may be missing", body = crate::types::JsonObject),
    )
)]
pub async fn configure_sidecar_channel(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<ConfigureSidecarBody>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, Json<serde_json::Value>)> {
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
    let schema = schema_cache()
        .read()
        .unwrap()
        .get(entry.name)
        .cloned()
        .ok_or_else(|| {
            ApiErrorResponse::internal(format!(
                "schema for `{name}` not cached — SDK module may be missing or `--describe` failed at boot"
            ))
            .with_status(StatusCode::SERVICE_UNAVAILABLE)
            .into_json_tuple()
        })?;

    // 3. Validate required fields: present in payload AND non-empty after trim.
    for f in &schema.fields {
        if f.required {
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
    let config_path = home.join("config.toml");

    // 3c. Refuse to save when an `include`d file already owns the
    //     `[[sidecar_channels]]` array. Writing a root-level entry on
    //     top of that would silently shadow the included one after the
    //     kernel merges them — the operator's intent (edit *that*
    //     entry) and our behaviour (append a fresh root entry) would
    //     diverge without warning. The dashboard / docs steer the
    //     operator to the file that owns the existing block.
    let shadowing = included_files_with_sidecars(&config_path);
    if !shadowing.is_empty() {
        let files = shadowing
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(ApiErrorResponse::conflict(format!(
            "config.toml uses `include` directive and existing `[[sidecar_channels]]` entries live in {files}. Edit that file directly to avoid silently shadowing the included sidecars."
        ))
        .into_json_tuple());
    }

    // 4. Split payload: secrets go to secrets.env, everything else
    //    accumulates into the [sidecar_channels.env] table.
    //
    //    Both the secrets.env upserts and the config.toml upsert below
    //    run inside `state.config_write_lock`. That mutex also gates
    //    `POST /api/config/set` and the legacy `configure_channel`
    //    handler (issue #3183), so two concurrent
    //    `POST /api/channels/sidecar/{a,b}/configure` calls — or one of
    //    those interleaved with `config_set` — cannot lost-update on
    //    `~/.librefang/config.toml` or on `~/.librefang/secrets.env`.
    //    The guard is dropped before `reload_config().await` so the
    //    hot-reload step does not gate other config-writing handlers.
    //
    //    The `secrets.env` membership read (for shell-shadow detection)
    //    also lives inside the guard so two concurrent saves on
    //    different keys cannot each see the pre-write file state and
    //    falsely report shadows on keys the other handler is about to
    //    write — a cosmetic-only TOCTOU but trivially closed by reading
    //    under the same lock that gates the write.
    let mut nonsecret_env: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();
    let shadowed_secrets: Vec<String>;
    {
        let _config_guard = state.config_write_lock.lock().await;

        // 4a. Detect shell-environment shadowing of `secret` fields,
        //     under the lock. The dotenv loader's priority is system env
        //     > vault > .env > secrets.env (see
        //     `librefang_extensions::dotenv`). If the operator exported
        //     `TELEGRAM_BOT_TOKEN` before launching the daemon,
        //     `std::env::var` returns that exported value and the
        //     sidecar child inherits it — not whatever we write to
        //     `secrets.env`. The save still succeeds mechanically, but
        //     the new value never takes effect. Warn before the operator
        //     chases this for an hour.
        //
        //     `std::env::var` also returns true for keys we loaded from
        //     `secrets.env` into the process env at boot, so subtract
        //     those out by reading the on-disk `secrets.env` once: a
        //     key already in `secrets.env` means the env presence is
        //     our own boot-time write, not a shell shadow.
        // KEY-only extraction: this set is used purely for membership
        // checks against the schema's secret field names (i.e. "is
        // TELEGRAM_BOT_TOKEN listed in secrets.env?"). Quotes never
        // appear inside dotenv KEYS, so the parser here intentionally
        // mirrors `librefang_channels::sidecar::parse_secrets_env`'s
        // key-extraction path but skips the value-side quote-stripping
        // that `parse_secrets_env` performs. If a future change starts
        // comparing VALUES here, switch to invoking the channels-crate
        // helper directly so quote/whitespace handling stays consistent
        // with how the sidecar actually inherits env vars at spawn time
        // (codex review fix #9).
        let secrets_env_keys: std::collections::HashSet<String> =
            std::fs::read_to_string(&secrets_path)
                .ok()
                .map(|s| {
                    s.lines()
                        .filter_map(|line| {
                            let line = line.trim();
                            if line.is_empty() || line.starts_with('#') {
                                return None;
                            }
                            let eq = line.find('=')?;
                            let k = line[..eq].trim();
                            if k.is_empty() {
                                None
                            } else {
                                Some(k.to_string())
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
        let mut shadowed: Vec<String> = schema
            .fields
            .iter()
            .filter(|f| f.field_type == "secret")
            .filter(|f| {
                body.values
                    .get(&f.key)
                    .map(|s| !s.trim().is_empty())
                    .unwrap_or(false)
            })
            .filter(|f| std::env::var(&f.key).is_ok() && !secrets_env_keys.contains(&f.key))
            .map(|f| f.key.clone())
            .collect();
        shadowed.sort();
        shadowed_secrets = shadowed;

        for f in &schema.fields {
            let Some(raw) = body.values.get(&f.key) else {
                continue;
            };
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                continue;
            }
            if f.field_type == "secret" {
                super::secrets_env::upsert_secret(&secrets_path, &f.key, trimmed).map_err(|e| {
                    ApiErrorResponse::internal(format!("failed to write secret: {e}"))
                        .into_json_tuple()
                })?;
            } else {
                nonsecret_env.insert(f.key.clone(), trimmed.to_string());
            }
        }

        // 5. Upsert the [[sidecar_channels]] block keyed by adapter name.
        //    Idempotent: a second POST with the same name replaces the
        //    block in-place, preserving formatting of every other section.
        //    `managed_env_keys` is the form's set of NON-SECRET schema
        //    fields — i.e. the keys the configure form is the source of
        //    truth for. Every OTHER env key already in the block (operator
        //    hand-edits such as `PYTHONPATH`, `HTTP_PROXY`, locale vars,
        //    or even a hand-edited `TELEGRAM_BOT_TOKEN` inline) is
        //    preserved untouched. Secret schema fields never appear in
        //    config.toml at all — they live in `secrets.env` — so they
        //    are intentionally excluded from this set.
        let managed_env_keys: Vec<&str> = schema
            .fields
            .iter()
            .filter(|f| f.field_type != "secret")
            .map(|f| f.key.as_str())
            .collect();
        super::sidecar_toml::upsert_sidecar_block(
            &config_path,
            entry.name,
            entry.name, // channel_type defaults to the catalog name
            entry.command,
            entry.args,
            &nonsecret_env,
            &managed_env_keys,
        )
        .map_err(|e| {
            ApiErrorResponse::internal(format!("failed to write config.toml: {e}"))
                .into_json_tuple()
        })?;
    }

    // 6. Trigger hot-reload. The kernel diffs the on-disk config
    //    against the live snapshot and returns the resulting plan;
    //    the dashboard surfaces `restart_required` so the operator
    //    knows whether further action is needed.
    let plan = state.kernel.reload_config().await.map_err(|e| {
        ApiErrorResponse::internal(format!("config reload failed: {e}")).into_json_tuple()
    })?;

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

/// Serialize a channel's config to a JSON Value for pre-populating dashboard forms.
/// GET /api/channels — List all 40 channel adapters with status and field metadata.
///
/// Envelope is the canonical `PaginatedResponse{items,total,offset,limit}`
/// shape used by `/api/agents`, `/api/peers`, `/api/skills`, etc. (#3842).
/// The full channel registry is materialized in-memory, so this is a single
/// page — `offset=0`, `limit=None`. The bespoke `configured_count` sibling
/// is preserved for the dashboard's "X of Y configured" sub-line.
#[utoipa::path(
    get,
    path = "/api/channels",
    tag = "channels",
    responses(
        (status = 200, description = "List configured channels", body = crate::types::JsonObject)
    )
)]
pub async fn list_channels(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // 24h activity per channel — backs the design's "slack · 142 msgs/24h"
    // sub-line. One grouped SQL pass for the whole page; falls back to an
    // empty map if the query fails so the listing itself still loads.
    // Configured channels come from `sidecar_channel_rows`; unconfigured
    // catalog adapters come from `sidecar_discovery_rows`. The
    // in-process CHANNEL_REGISTRY loop that used to feed both is gone.
    let msgs_24h = state
        .kernel
        .memory_substrate()
        .usage()
        .channels_msgs_24h_bulk()
        .unwrap_or_default();
    let kcfg = state.kernel.config_ref();
    let configured_rows = sidecar_channel_rows(&kcfg.sidecar_channels, &msgs_24h, true);
    let configured_count = configured_rows.len() as u32;
    let mut channels = configured_rows;
    channels.extend(sidecar_discovery_rows(&kcfg.sidecar_channels));

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
    // about per-channel msg counts. See `list_channels` for the
    // history of the in-process loop that this used to mirror.
    let kcfg = state.kernel.config_ref();
    let mut channels = sidecar_channel_rows(
        &kcfg.sidecar_channels,
        &std::collections::HashMap::new(),
        false,
    );
    channels.extend(sidecar_discovery_rows(&kcfg.sidecar_channels));
    channels
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
// ---------------------------------------------------------------------------
// WhatsApp QR endpoints removed — the WhatsApp adapter migrated
// to a sidecar (librefang.sidecar.adapters.whatsapp). The Baileys
// gateway (when in use) owns the QR login flow end-to-end now.
// ---------------------------------------------------------------------------

// WeChat QR endpoints removed — the WeChat adapter migrated to
// a sidecar (librefang.sidecar.adapters.wechat). The sidecar
// now runs the QR login flow itself and logs the QR string at
// INFO; the dashboard surfaces it by tailing sidecar logs.


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
    Json(serde_json::to_value(&metadata).unwrap_or_default())
}

// `test_channel_status_tests` + `instance_helper_tests` modules
// removed entirely. The former tested the `test_channel` HTTP
// handler (deleted with the in-process-channel scaffolding); the
// latter tested `instance_signature` + `resolve_secret_env_overrides`
// (both deleted with their only callers, the per-instance REST
// handlers).
