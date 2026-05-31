//! Configuration validation logic: unknown field detection, structural validation, and safety boundary constraints.
//!
//! ## Allowlist source-of-truth (#4298)
//!
//! [`KernelConfig::known_top_level_fields`] and [`KernelConfig::detect_unknown_nested_fields`]
//! used to be hand-maintained string lists. Every time a new field landed
//! on `KernelConfig` (or any nested config struct), the lists silently
//! drifted and `strict_config = true` rejected the new field as unknown.
//!
//! Both lists are now **derived at runtime from the JSON Schema that
//! `schemars` emits for `KernelConfig`** (which sees every struct field,
//! independent of `#[serde(skip_serializing_if = …)]`). Adding a field to
//! any config struct automatically updates the allowlist on the next
//! process start. A small static supplement (`MANUAL_TOP_LEVEL_ALIASES`)
//! adds `#[serde(alias = …)]` aliases that JSON Schema does not surface,
//! such as `listen_addr` → `api_listen` and `approval_policy` → `approval`.

use super::types::*;
use schemars::schema::{RootSchema, Schema, SchemaObject};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::OnceLock;

/// Aliases honoured by `#[serde(alias = …)]` on `KernelConfig` fields.
/// JSON Schema does not surface serde aliases, so the allowlist must add
/// them by hand. Keep this list in sync with the alias attributes on the
/// `KernelConfig` struct in `types.rs`.
const MANUAL_TOP_LEVEL_ALIASES: &[&str] = &[
    "listen_addr",     // alias for api_listen
    "approval_policy", // alias for approval
];

/// Nested aliases honoured by `#[serde(alias = …)]` on fields of nested
/// config structs. Each entry is a `(dotted_path, alias)` pair where
/// `dotted_path` is the section that owns the aliased field (e.g.
/// `"terminal"` for `TerminalConfig`) and `alias` is the legacy name still
/// accepted on the wire.
///
/// schemars (0.8) drops `serde(alias)` declarations when generating the
/// JSON Schema, so strict-mode rejected the legacy name even though serde
/// would happily deserialise it (#5129). Keep this list in sync with the
/// `alias = "…"` attributes on nested struct fields in `types.rs`.
const MANUAL_NESTED_ALIASES: &[(&str, &str)] = &[
    // TerminalConfig.require_proxy_headers was renamed from
    // `trust_proxy_headers`; the old name stays accepted via serde(alias).
    ("terminal", "trust_proxy_headers"),
];

/// `[agents.<name>.<key>]` keys that a `config.toml` operator might write
/// expecting a per-agent override, but which `KernelConfig` silently
/// ignores because it has no `agents` field (#5476, #6). Each key here is
/// a section that *is* honoured when placed in the agent's own `agent.toml`
/// (or the `[agents.<name>]` table of a `HAND.toml`); the detector turns a
/// silent no-op into a targeted "move this to agent.toml" warning.
///
/// These are the **config.toml-facing** names — what the operator types
/// under `[agents.<name>.…]`. For most overrides that is also the
/// `AgentManifest` field name, but two diverge:
///   - `tool_exec` mirrors the global `[tool_exec]` section, while the
///     manifest field is the scalar `tool_exec_backend`; both spellings
///     are flagged so the operator is caught regardless of which one they
///     copied.
///   - `skill_workshop` has no global `KernelConfig` section at all — it is
///     an agent-only feature whose documented surface is `[skill_workshop]`
///     in `agent.toml`.
///
/// Do not hand-edit this list to track new `AgentManifest` fields: the
/// `per_agent_override_keys_cover_manifest_overrides` test in
/// `config/mod.rs` exhaustively destructures `AgentManifest` and fails the
/// build if a newly-added field that overrides a global `KernelConfig`
/// section is missing here (or if a stale key no longer maps to anything).
pub(super) const PER_AGENT_OVERRIDE_KEYS: &[&str] = &[
    "compaction",
    "exec_policy",
    "max_history_messages",
    "proactive_memory",
    "skill_workshop",
    "thinking",
    "tool_exec",
    "tool_exec_backend",
];

/// Cached allowlists derived once from the schemars-emitted JSON Schema
/// for `KernelConfig`. Built on first use and reused for the rest of the
/// process.
static ALLOWLISTS: OnceLock<DerivedAllowlists> = OnceLock::new();

struct DerivedAllowlists {
    /// Top-level field names (struct fields plus serde aliases),
    /// sorted, deduplicated, and `'static`-leaked so the public API can
    /// keep returning `&'static [&'static str]`.
    top_level: Vec<&'static str>,
    /// `field_path` → set of accepted child field names, for every
    /// nested object schema reachable from `KernelConfig`. Keys are
    /// dotted paths like `"memory"`, `"queue.concurrency"`. Values are
    /// `'static`-leaked strings.
    nested: BTreeMap<String, BTreeSet<&'static str>>,
}

fn allowlists() -> &'static DerivedAllowlists {
    ALLOWLISTS.get_or_init(build_allowlists)
}

fn build_allowlists() -> DerivedAllowlists {
    let root: RootSchema = schemars::schema_for!(KernelConfig);
    let definitions = root.definitions.clone();

    // --- Top level -------------------------------------------------
    let mut top: BTreeSet<String> = root
        .schema
        .object
        .as_ref()
        .map(|obj| obj.properties.keys().cloned().collect())
        .unwrap_or_default();
    for alias in MANUAL_TOP_LEVEL_ALIASES {
        top.insert((*alias).to_string());
    }
    let top_level: Vec<&'static str> = top
        .into_iter()
        .map(|s| Box::leak(s.into_boxed_str()) as &'static str)
        .collect();

    // --- Nested paths ----------------------------------------------
    // Walk every property of the root schema. For each property that
    // resolves to an object schema (directly or via $ref), record its
    // child field names under the dotted path, and recurse.
    let mut nested: BTreeMap<String, BTreeSet<&'static str>> = BTreeMap::new();
    if let Some(root_obj) = root.schema.object.as_ref() {
        for (name, sub) in &root_obj.properties {
            walk_nested(
                name.clone(),
                sub,
                &definitions,
                &mut nested,
                &mut HashSet::new(),
            );
        }
    }
    // Add `#[serde(alias)]` declarations that schemars dropped (#5129).
    // Only insert into paths the schema actually surfaced — that way a
    // stale entry in `MANUAL_NESTED_ALIASES` (e.g. a section that was
    // later removed) doesn't silently widen the allowlist.
    for (path, alias) in MANUAL_NESTED_ALIASES {
        if let Some(entry) = nested.get_mut(*path) {
            let leaked: &'static str = Box::leak((*alias).to_string().into_boxed_str());
            entry.insert(leaked);
        }
    }

    DerivedAllowlists { top_level, nested }
}

/// Resolve a `Schema` to its underlying `SchemaObject`, following a
/// single level of `$ref` into `definitions` and unwrapping the common
/// `Option<T>` shape that schemars emits as `{ "anyOf": [T, null] }` or
/// `{ "type": ["object","null"] }`.
fn resolve<'a>(
    schema: &'a Schema,
    defs: &'a schemars::Map<String, Schema>,
) -> Option<&'a SchemaObject> {
    let obj = match schema {
        Schema::Object(o) => o,
        Schema::Bool(_) => return None,
    };
    if let Some(reference) = obj.reference.as_ref() {
        let def_name = reference.strip_prefix("#/definitions/")?;
        let def = defs.get(def_name)?;
        return resolve(def, defs);
    }
    // Option<T> → anyOf: [T, null] — drill into the non-null branch.
    if let Some(sub) = obj.subschemas.as_ref() {
        for branch in sub
            .any_of
            .iter()
            .chain(sub.one_of.iter())
            .chain(sub.all_of.iter())
            .flatten()
        {
            if let Some(resolved) = resolve(branch, defs) {
                if resolved.object.is_some() {
                    return Some(resolved);
                }
            }
        }
    }
    Some(obj)
}

fn walk_nested(
    path: String,
    schema: &Schema,
    defs: &schemars::Map<String, Schema>,
    out: &mut BTreeMap<String, BTreeSet<&'static str>>,
    visiting: &mut HashSet<String>,
) {
    // Prevent unbounded recursion through self-referential definitions.
    if !visiting.insert(path.clone()) {
        return;
    }
    let Some(resolved) = resolve(schema, defs) else {
        visiting.remove(&path);
        return;
    };
    let Some(obj) = resolved.object.as_ref() else {
        visiting.remove(&path);
        return;
    };
    // Skip map-shaped objects (`additional_properties` set, no concrete
    // `properties`): those are user-supplied keys, not struct fields.
    if obj.properties.is_empty() {
        visiting.remove(&path);
        return;
    }
    let entry = out.entry(path.clone()).or_default();
    for child in obj.properties.keys() {
        let leaked: &'static str = Box::leak(child.clone().into_boxed_str());
        entry.insert(leaked);
    }
    for (child_name, child_schema) in &obj.properties {
        let child_path = format!("{path}.{child_name}");
        walk_nested(child_path, child_schema, defs, out, visiting);
    }
    visiting.remove(&path);
}

impl KernelConfig {
    /// Top-level field names accepted by `KernelConfig` (including serde
    /// aliases). Derived from the schemars-emitted JSON Schema; see the
    /// module-level docs for the drift-prevention contract.
    pub fn known_top_level_fields() -> &'static [&'static str] {
        &allowlists().top_level
    }

    /// Detect unknown top-level keys in a raw TOML value.
    ///
    /// Returns a list of field names that appear at the top level of the
    /// config file but are not recognised by `KernelConfig`.
    pub fn detect_unknown_fields(raw: &toml::Value) -> Vec<String> {
        let known: HashSet<&str> = Self::known_top_level_fields().iter().copied().collect();
        let mut unknown = Vec::new();
        if let toml::Value::Table(tbl) = raw {
            for key in tbl.keys() {
                if !known.contains(key.as_str()) {
                    unknown.push(key.clone());
                }
            }
        }
        unknown.sort();
        unknown
    }

    /// Detect unknown keys in nested config sections (#3460, #4298).
    ///
    /// Top-level [`detect_unknown_fields`] only catches typos at the
    /// root of `config.toml`. Sub-sections like `[memory]`,
    /// `[queue.concurrency]` or `[budget]` are silently accepted with
    /// `#[serde(default)]`, so a typo such as `decay_ratee = 0.1`
    /// deserialises into the section's `Default` and the operator's
    /// intent never reaches the runtime.
    ///
    /// The per-section allowlist is **derived from the schemars-emitted
    /// JSON Schema**, so adding a field to any nested config struct
    /// automatically updates this check on the next process start.
    /// Each entry returns a dotted path (e.g. `"memory.decay_ratee"`)
    /// so the warning is actionable. Returned in deterministic sorted
    /// order.
    pub fn detect_unknown_nested_fields(raw: &toml::Value) -> Vec<String> {
        let nested = &allowlists().nested;
        let mut unknown = Vec::new();
        for (path, known) in nested {
            // Walk the dotted path through the toml tree.
            let mut node: Option<&toml::Value> = Some(raw);
            for segment in path.split('.') {
                node = node.and_then(|v| v.as_table()).and_then(|t| t.get(segment));
            }
            let Some(toml::Value::Table(tbl)) = node else {
                continue;
            };
            for key in tbl.keys() {
                if !known.contains(key.as_str()) {
                    unknown.push(format!("{path}.{key}"));
                }
            }
        }
        unknown.sort();
        unknown
    }

    /// Detect `[agents.<name>.<override_key>]` blocks placed in
    /// `config.toml` (#5476, #6).
    ///
    /// `KernelConfig` has no `agents` field — per-agent overrides
    /// (`proactive_memory`, `skill_workshop`, `compaction`, `thinking`,
    /// `exec_policy`, `max_history_messages`, and the tool-exec backend;
    /// the full set lives in [`PER_AGENT_OVERRIDE_KEYS`]) live in each
    /// agent's own `agent.toml` (or the `[agents.<name>]` section of a
    /// `HAND.toml`), not in `config.toml`. The original #4870
    /// issue body proposed the `config.toml` syntax in error, and the
    /// kernel silently accepted-and-ignored the block (the unknown
    /// top-level `agents` key was warned about generically, but the
    /// warning did not point at the correct surface). Operators
    /// following the published syntax got a silent no-op.
    ///
    /// This helper walks the raw TOML for `[agents.<name>.<key>]`
    /// sub-tables and returns one `(agent_name, override_key)` pair
    /// per occurrence, sorted deterministically, so the caller can
    /// emit a targeted warning that names the correct location. Only
    /// the keys that are *actually* `agent.toml`-only overrides are
    /// flagged — generic typos under `[agents]` fall through to the
    /// existing unknown-top-level warning.
    pub fn detect_misplaced_per_agent_overrides(raw: &toml::Value) -> Vec<(String, String)> {
        let Some(agents_tbl) = raw
            .as_table()
            .and_then(|t| t.get("agents"))
            .and_then(|v| v.as_table())
        else {
            return Vec::new();
        };

        let mut found = Vec::new();
        for (agent_name, agent_value) in agents_tbl {
            let Some(agent_tbl) = agent_value.as_table() else {
                continue;
            };
            for key in PER_AGENT_OVERRIDE_KEYS {
                if agent_tbl.contains_key(*key) {
                    found.push((agent_name.clone(), (*key).to_string()));
                }
            }
        }
        found.sort();
        found
    }

    /// Validate the configuration, returning a list of warnings.
    ///
    /// Checks for common misconfigurations such as missing API keys for
    /// configured channels, invalid port numbers, unreachable paths,
    /// and unrecognised log levels.
    pub fn validate(&self) -> Vec<String> {
        let mut warnings = Vec::new();

        // whatsapp migrated to a sidecar (librefang.sidecar.adapters.whatsapp);
        // env-var presence is now validated inside the sidecar process.
        // matrix migrated to a sidecar (librefang.sidecar.adapters.matrix);
        // see SIDECAR_CATALOG in librefang-api/src/routes/channels.rs.
        // email migrated to a sidecar (librefang.sidecar.adapters.email);
        // env-var presence is now validated inside the sidecar process.
        // teams migrated to a sidecar (librefang.sidecar.adapters.teams);
        // env-var presence is now validated inside the sidecar process.
        // mattermost migrated to a sidecar (librefang.sidecar.adapters.mattermost);
        // env-var presence is now validated inside the sidecar process.
        // google_chat migrated to a sidecar (librefang.sidecar.adapters.google_chat);
        // env-var presence is now validated inside the sidecar process.
        // Wave 3 channels
        // line migrated to a sidecar (librefang.sidecar.adapters.line);
        // env-var presence is now validated inside the sidecar process.
        // feishu migrated to a sidecar (librefang.sidecar.adapters.feishu);
        // env-var presence is now validated inside the sidecar process.
        // Wave 4 channels
        // webex migrated to a sidecar (librefang.sidecar.adapters.webex);
        // env-var presence is now validated inside the sidecar process.
        // Wave 5 channels
        // dingtalk migrated to a sidecar (librefang.sidecar.adapters.dingtalk);
        // env-var presence is now validated inside the sidecar process.
        // webhook migrated to a sidecar (librefang.sidecar.adapters.webhook);
        // env-var presence + deliver_only-needs-target are now validated
        // inside the sidecar process at startup (fail-closed `SystemExit(2)`
        // when WEBHOOK_DELIVER_ONLY=1 but WEBHOOK_DELIVER is empty).

        // Web search provider validation
        match self.web.search_provider {
            SearchProvider::Brave => {
                if std::env::var(&self.web.brave.api_key_env)
                    .unwrap_or_default()
                    .is_empty()
                {
                    warnings.push(format!(
                        "Brave search selected but {} is not set",
                        self.web.brave.api_key_env
                    ));
                }
            }
            SearchProvider::Tavily => {
                if std::env::var(&self.web.tavily.api_key_env)
                    .unwrap_or_default()
                    .is_empty()
                {
                    warnings.push(format!(
                        "Tavily search selected but {} is not set",
                        self.web.tavily.api_key_env
                    ));
                }
            }
            SearchProvider::Perplexity => {
                if std::env::var(&self.web.perplexity.api_key_env)
                    .unwrap_or_default()
                    .is_empty()
                {
                    warnings.push(format!(
                        "Perplexity search selected but {} is not set",
                        self.web.perplexity.api_key_env
                    ));
                }
            }
            SearchProvider::Jina => {
                if std::env::var(&self.web.jina.api_key_env)
                    .unwrap_or_default()
                    .is_empty()
                {
                    warnings.push(format!(
                        "Jina search selected but {} is not set",
                        self.web.jina.api_key_env
                    ));
                }
            }
            SearchProvider::Searxng => {
                if self.web.searxng.url.is_empty() {
                    warnings.push(
                        "Searxng search selected but searxng.url is not configured".to_string(),
                    );
                }
            }
            SearchProvider::DuckDuckGo | SearchProvider::Auto => {}
        }

        // --- Structural validation ---

        // Validate api_listen has a parseable port
        if let Some(colon_pos) = self.api_listen.rfind(':') {
            let port_str = &self.api_listen[colon_pos + 1..];
            match port_str.parse::<u16>() {
                Ok(0) => {
                    warnings
                        .push("api_listen port is 0 (OS will assign a random port)".to_string());
                }
                Err(_) => {
                    warnings.push(format!("api_listen port '{}' is not a valid u16", port_str));
                }
                Ok(_) => {}
            }
        } else {
            warnings.push(format!(
                "api_listen '{}' does not contain a port (expected host:port)",
                self.api_listen
            ));
        }

        // Validate log_level is a recognised value
        match self.log_level.to_lowercase().as_str() {
            "trace" | "debug" | "info" | "warn" | "error" | "off" => {}
            other => {
                warnings.push(format!(
                    "log_level '{}' is not a recognised level (expected trace/debug/info/warn/error/off)",
                    other
                ));
            }
        }

        // Validate home_dir exists (or can be created)
        if !self.home_dir.as_os_str().is_empty() && !self.home_dir.exists() {
            warnings.push(format!(
                "home_dir '{}' does not exist (will be created on first use)",
                self.home_dir.display()
            ));
        }

        // Validate data_dir parent is writable (basic path sanity)
        if !self.data_dir.as_os_str().is_empty() && !self.data_dir.exists() {
            if let Some(parent) = self.data_dir.parent() {
                if !parent.as_os_str().is_empty() && !parent.exists() {
                    warnings.push(format!(
                        "data_dir parent '{}' does not exist",
                        parent.display()
                    ));
                }
            }
        }

        // Validate max_cron_jobs is within a reasonable range
        if self.max_cron_jobs > 10_000 {
            warnings.push(format!(
                "max_cron_jobs {} exceeds reasonable limit (10000)",
                self.max_cron_jobs
            ));
        }

        // Validate network config: shared_secret must be set if network is enabled
        if self.network_enabled && self.network.shared_secret.is_empty() {
            warnings.push("network_enabled is true but network.shared_secret is empty".to_string());
        }

        // --- Terminal access control validation ---

        if self.terminal.enabled {
            // Validate each allowed_origins entry is a valid http(s) URL
            for origin in &self.terminal.allowed_origins {
                if origin == "*" {
                    // Wildcard is valid syntax but requires allow_remote
                    if !self.terminal.allow_remote {
                        warnings.push(
                            "terminal.allowed_origins contains \"*\" (wildcard) but terminal.allow_remote is false — \
                             wildcard is incoherent without allow_remote, set allow_remote = true or remove \"*\""
                                .to_string(),
                        );
                    }
                    continue;
                }
                let looks_like_url = (origin.starts_with("http://")
                    || origin.starts_with("https://"))
                    && origin.contains("://");
                if !looks_like_url {
                    warnings.push(format!(
                        "terminal.allowed_origins entry '{}' is not a valid URL (must use http:// or https:// scheme)",
                        origin
                    ));
                }
            }

            // Warn if allow_remote is true without any authentication
            if self.terminal.allow_remote {
                // We can't check auth_configured here (requires runtime state),
                // but warn about the risk
                warnings.push(
                    "terminal.allow_remote is true — the terminal WebSocket will accept connections from \
                     non-local origins; ensure authentication is configured (api_key, dashboard credentials, or users)"
                        .to_string(),
                );
            }

            // Warn if require_proxy_headers is set but api_listen is loopback-only
            if self.terminal.require_proxy_headers {
                let listen = &self.api_listen;
                if listen.starts_with("127.0.0.1:")
                    || listen.starts_with("localhost:")
                    || listen.starts_with("[::1]:")
                {
                    warnings.push(
                        "terminal.require_proxy_headers is true but api_listen is loopback-only — \
                         proxy headers have no effect when only local connections can reach the server"
                            .to_string(),
                    );
                }
            }
        }

        // RBAC M3 review follow-up: per-user `memory_access` flags only
        // matter alongside the namespace list they actually depend on.
        // `MemoryNamespaceGuard` gates each flag like this:
        //
        //   pii_access     → needs READ access (redaction only runs on
        //                    items the user can read).
        //   export_allowed → needs READ access (`check_export` calls
        //                    `check_read` after the flag check).
        //   delete_allowed → needs WRITE access (`check_delete` calls
        //                    `check_write`).
        //
        // The earlier version of this pass grouped `delete_allowed` under
        // `readable_namespaces` — wrong; a user with read but no write
        // access who set `delete_allowed = true` would NOT have been
        // warned even though delete silently fails. Split into two
        // independent passes that mirror the runtime gates.
        for user in &self.users {
            let Some(ref acl) = user.memory_access else {
                continue;
            };

            // Pass 1 — read-dependent flags vs readable_namespaces.
            if acl.readable_namespaces.is_empty() {
                let read_dependent: Vec<&'static str> = [
                    ("pii_access", acl.pii_access),
                    ("export_allowed", acl.export_allowed),
                ]
                .into_iter()
                .filter_map(|(name, on)| on.then_some(name))
                .collect();
                if !read_dependent.is_empty() {
                    warnings.push(format!(
                        "[users.{}.memory_access] sets {:?} = true but \
                         `readable_namespaces` is empty — these flags are no-ops without \
                         read access. Likely a typo: did you mean to add \
                         `readable_namespaces = [\"...\"]`?",
                        user.name, read_dependent,
                    ));
                }
            }

            // Pass 2 — write-dependent flags vs writable_namespaces.
            if acl.delete_allowed && acl.writable_namespaces.is_empty() {
                warnings.push(format!(
                    "[users.{}.memory_access] sets `delete_allowed` = true but \
                     `writable_namespaces` is empty — delete is gated on write \
                     access (not read). Likely a typo: did you mean to add \
                     `writable_namespaces = [\"...\"]`?",
                    user.name,
                ));
            }
        }

        // #5138: `cron_session_max_messages` above the substrate's hard
        // persistence ceiling can never actually keep that many messages
        // across daemon restarts — `save_session` truncates the tail
        // beyond MAX_PERSISTED_SESSION_MESSAGES regardless of the cron cap.
        // Surface the discrepancy at config load instead of letting the
        // operator silently lose context.
        if let Some(n) = self.cron_session_max_messages {
            if n > super::MAX_PERSISTED_SESSION_MESSAGES {
                warnings.push(format!(
                    "cron_session_max_messages = {n} exceeds the substrate \
                     persistence ceiling of {} messages per session; history \
                     beyond {} is silently truncated on save and will not \
                     survive a daemon restart (#5138)",
                    super::MAX_PERSISTED_SESSION_MESSAGES,
                    super::MAX_PERSISTED_SESSION_MESSAGES,
                ));
            }
        }

        warnings
    }

    /// Clamp configuration values to safe production bounds.
    ///
    /// Called after loading config to prevent zero timeouts, unbounded buffers,
    /// or other misconfigurations that cause silent failures at runtime.
    #[allow(clippy::manual_clamp)]
    pub fn clamp_bounds(&mut self) {
        // Browser timeout: min 5s, max 300s
        if self.browser.timeout_secs == 0 {
            self.browser.timeout_secs = 30;
        } else if self.browser.timeout_secs > 300 {
            self.browser.timeout_secs = 300;
        }

        // Browser max sessions: min 1, max 100
        if self.browser.max_sessions == 0 {
            self.browser.max_sessions = 3;
        } else if self.browser.max_sessions > 100 {
            self.browser.max_sessions = 100;
        }

        // Web fetch max_response_bytes: min 1KB, max 50MB
        if self.web.fetch.max_response_bytes == 0 {
            self.web.fetch.max_response_bytes = 5_000_000;
        } else if self.web.fetch.max_response_bytes > 50_000_000 {
            self.web.fetch.max_response_bytes = 50_000_000;
        }

        // Web fetch timeout: min 5s, max 120s
        if self.web.fetch.timeout_secs == 0 {
            self.web.fetch.timeout_secs = 30;
        } else if self.web.fetch.timeout_secs > 120 {
            self.web.fetch.timeout_secs = 120;
        }

        // Web search timeout: min 5s, max 120s
        if self.web.timeout_secs == 0 {
            self.web.timeout_secs = 15;
        } else if self.web.timeout_secs > 120 {
            self.web.timeout_secs = 120;
        }

        // Queue concurrency: min 1 per lane (0 would deadlock)
        if self.queue.concurrency.main_lane == 0 {
            self.queue.concurrency.main_lane = 1;
        }
        if self.queue.concurrency.cron_lane == 0 {
            self.queue.concurrency.cron_lane = 1;
        }
        if self.queue.concurrency.subagent_lane == 0 {
            self.queue.concurrency.subagent_lane = 1;
        }
        if self.queue.concurrency.trigger_lane == 0 {
            self.queue.concurrency.trigger_lane = 1;
        }
        if self.queue.concurrency.default_per_agent == 0 {
            self.queue.concurrency.default_per_agent = 1;
        }
        // Trigger-fire timeout: 0 means "infinite hold on Lane::Trigger" (#3446)
        if self.queue.concurrency.trigger_fire_timeout_secs == 0 {
            self.queue.concurrency.trigger_fire_timeout_secs = 300;
        }

        // Triggers: max_per_event must be >= 1 (0 would prevent any trigger from firing)
        if self.triggers.max_per_event == 0 {
            self.triggers.max_per_event = 1;
        }
        // Triggers: max_depth must be >= 1
        if self.triggers.max_depth == 0 {
            self.triggers.max_depth = 1;
        }
        // Triggers: max_workflow_secs min 10s, max 86400s (24h)
        if self.triggers.max_workflow_secs < 10 {
            self.triggers.max_workflow_secs = 10;
        } else if self.triggers.max_workflow_secs > 86400 {
            self.triggers.max_workflow_secs = 86400;
        }

        // max_cron_jobs: min 1 (0 silently disables all cron job creation —
        // CronScheduler's limit check is `len >= max`, so 0 rejects every
        // create). Max 10_000 matches the validation warning threshold.
        // Clamp upward to the same default used by serde (500).
        if self.max_cron_jobs == 0 {
            self.max_cron_jobs = 500;
        } else if self.max_cron_jobs > 10_000 {
            self.max_cron_jobs = 10_000;
        }

        // RBAC M5: per-user `alert_threshold` is documented as "clamped to
        // 0..=1" but the field type is bare `f64` and TOML will accept any
        // value. Without this clamp, `alert_threshold = 5.0` makes
        // `alert_breach` permanently false (no alert ever fires) and
        // `-1.0` makes it permanently true (alerts on zero spend). Clamp
        // both ends so the documented contract holds.
        for user in &mut self.users {
            if let Some(ref mut budget) = user.budget {
                if !budget.alert_threshold.is_finite() {
                    budget.alert_threshold = 0.8;
                } else if budget.alert_threshold < 0.0 {
                    budget.alert_threshold = 0.0;
                } else if budget.alert_threshold > 1.0 {
                    budget.alert_threshold = 1.0;
                }
            }
        }
    }
}
