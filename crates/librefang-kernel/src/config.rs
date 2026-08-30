//! Configuration loading from `~/.librefang/config.toml` with defaults.
//!
//! Supports config includes: the `include` field specifies additional TOML files
//! to load and deep-merge before the root config (root overrides includes).

use librefang_types::config::{
    default_config_version, run_migrations, KernelConfig, CONFIG_VERSION,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tracing::info;

/// Maximum include nesting depth.
const MAX_INCLUDE_DEPTH: u32 = 10;

fn atomic_write_config(path: &Path, content: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    // Preserve a configured symlink. Renaming over `path` itself would replace
    // the link rather than atomically updating the operator-owned target.
    let target = match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => std::fs::canonicalize(path)?,
        _ => path.to_path_buf(),
    };
    let original_permissions = match std::fs::metadata(&target) {
        Ok(metadata) => {
            // Atomic rename is governed by directory permissions and could
            // otherwise replace an operator-owned read-only config that the
            // previous direct-write path correctly refused to modify.
            std::fs::OpenOptions::new().write(true).open(&target)?;
            Some(metadata.permissions())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let file_name = target
        .file_name()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "missing filename"))?;
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut temp_name = file_name.to_os_string();
    temp_name.push(format!(".{}.{seq}.{timestamp}.tmp", std::process::id()));
    let temp_path = target.with_file_name(temp_name);

    let write_result = (|| -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)?;
        if let Some(permissions) = original_permissions {
            file.set_permissions(permissions)?;
        }
        file.write_all(content.as_bytes())?;
        file.sync_all()
    })();
    if let Err(error) = write_result {
        let _ = std::fs::remove_file(&temp_path);
        return Err(error);
    }

    if let Err(error) = std::fs::rename(&temp_path, &target) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(error);
    }

    #[cfg(unix)]
    {
        let parent = target
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        std::fs::File::open(parent)?.sync_all()?;
    }

    Ok(())
}

/// Load kernel configuration from a TOML file, with defaults.
///
/// Returns `Err` when the config file exists but cannot be parsed as valid TOML
/// or cannot be deserialized into `KernelConfig` — a content failure that means
/// the operator's settings have been silently discarded. The caller must not
/// substitute `KernelConfig::default()` on `Err`; doing so would hide the
/// real problem and produce a misleading downstream error (see issue #5186).
///
/// Returns `Ok(KernelConfig::default())` when the config file is absent or
/// unreadable due to an I/O error (file not found, permission denied), because
/// that is a deployment-time condition where defaults are a safe starting point.
///
/// If the config contains an `include` field, included files are loaded
/// and deep-merged first, then the root config overrides them.
pub fn load_config(path: Option<&Path>) -> Result<KernelConfig, String> {
    let config_path = path
        .map(|p| p.to_path_buf())
        .unwrap_or_else(default_config_path);

    if config_path.exists() {
        match std::fs::read_to_string(&config_path) {
            Ok(contents) => match toml::from_str::<toml::Value>(&contents) {
                Ok(mut root_value) => {
                    // Process includes before deserializing
                    let config_dir = config_path
                        .parent()
                        .unwrap_or_else(|| Path::new("."))
                        .to_path_buf();
                    let mut visited = HashSet::new();
                    if let Ok(canonical) = std::fs::canonicalize(&config_path) {
                        visited.insert(canonical);
                    } else {
                        visited.insert(config_path.clone());
                    }

                    if let Err(e) =
                        resolve_config_includes(&mut root_value, &config_dir, &mut visited, 0)
                    {
                        tracing::warn!(
                            error = %e,
                            "Config include resolution failed, using root config only"
                        );
                    }

                    // Remove the `include` field before deserializing to avoid confusion
                    if let toml::Value::Table(ref mut tbl) = root_value {
                        tbl.remove("include");
                    }

                    // --- Versioned config migration ---
                    // Keep a clone of the pre-migration value for best-effort fallback.
                    let original_value = root_value.clone();

                    let file_version = root_value
                        .as_table()
                        .and_then(|t| t.get("config_version"))
                        .and_then(|v| v.as_integer())
                        .map(|v| v as u32)
                        .unwrap_or_else(default_config_version);

                    let mut migrated = file_version >= CONFIG_VERSION;
                    if file_version < CONFIG_VERSION {
                        match run_migrations(&mut root_value, file_version) {
                            Ok(_) => {
                                info!(
                                    from = file_version,
                                    to = CONFIG_VERSION,
                                    "Config migrated successfully"
                                );
                                migrated = true;
                            }
                            Err(e) => {
                                tracing::error!(
                                    error = %e,
                                    from = file_version,
                                    to = CONFIG_VERSION,
                                    "Config migration failed, attempting best-effort load of original config"
                                );
                                // Fall back to original value
                                root_value = original_value.clone();
                            }
                        }
                    }

                    // Detect unknown top-level fields before deserialization.
                    let unknown_fields = KernelConfig::detect_unknown_fields(&root_value);
                    // Detect unknown fields in known nested sections (#3460):
                    // typos like `[memory] decay_ratee = 0.1` previously
                    // deserialised into the section's `Default`, silently
                    // dropping the operator's intent.
                    let unknown_nested = KernelConfig::detect_unknown_nested_fields(&root_value);

                    // Check if strict_config is set in the raw TOML (before
                    // deserializing the full struct) so we can decide whether
                    // to reject or warn on unknown fields.
                    let is_strict = root_value
                        .as_table()
                        .and_then(|t| t.get("strict_config"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);

                    let mut all_unknown: Vec<String> = unknown_fields
                        .iter()
                        .cloned()
                        .chain(unknown_nested.iter().cloned())
                        .collect();
                    all_unknown.sort();

                    if !all_unknown.is_empty() {
                        if is_strict {
                            tracing::error!(
                                path = %config_path.display(),
                                fields = %all_unknown.join(", "),
                                "strict_config is enabled and config contains unknown fields, using defaults"
                            );
                            return Ok(KernelConfig {
                                strict_config: true,
                                ..KernelConfig::default()
                            });
                        }
                        for field in &all_unknown {
                            tracing::warn!(field, "Unknown config field (ignored)");
                        }
                    }

                    // #5476: targeted warning for `[agents.<name>.<key>]`
                    // blocks placed in `config.toml`. These look plausible
                    // (the original #4870 issue body even published the
                    // syntax) but `KernelConfig` has no `agents` field, so
                    // the override silently no-ops. Point operators at
                    // `agent.toml`'s top-level `[<key>]`, which is the
                    // actual surface AgentManifest deserialises.
                    for (agent, key) in
                        KernelConfig::detect_misplaced_per_agent_overrides(&root_value)
                    {
                        tracing::warn!(
                            agent = %agent,
                            key = %key,
                            "[agents.{agent}.{key}] in config.toml is ignored; \
                             per-agent overrides live in {{workspace}}/agent.toml's \
                             top-level [{key}] block (or the [agents.{agent}] \
                             section of a HAND.toml), not in config.toml — see #5476"
                        );
                    }

                    // Targeted warning for pre-sidecar `[channels.<vendor>]` blocks.
                    // Every channel adapter migrated out-of-process (#5317–#5459); the in-process per-vendor fields were removed from `ChannelsConfig`, so an old block parses into nothing and the configured channel silently disappears on upgrade.
                    // The generic unknown-field pass already names `channels.<vendor>`, but does not tell the operator the channel moved to `[[sidecar_channels]]`.
                    for vendor in KernelConfig::detect_legacy_channel_blocks(&root_value) {
                        tracing::warn!(
                            channel = %vendor,
                            "[channels.{vendor}] in config.toml is ignored: in-process \
                             channels migrated to out-of-process sidecars. Re-add it under \
                             [[sidecar_channels]] (Dashboard → Channels → Add, or hand-edit \
                             config.toml) and install the sidecar SDK ('pip install \
                             librefang-sdk') — see docs/architecture/sidecar-channels.md"
                        );
                    }

                    match root_value.try_into::<KernelConfig>() {
                        Ok(config) => {
                            // `prompt_intelligence.hash_prompts = false` cannot be
                            // honoured: the SHA-256 `content_hash` is load-bearing
                            // (it drives prompt-version dedup and is stored NOT
                            // NULL), so it is always computed. Surface the ignored
                            // knob instead of letting it silently no-op.
                            if !config.prompt_intelligence.hash_prompts {
                                tracing::warn!(
                                    "[prompt_intelligence] hash_prompts = false is ignored: \
                                     the content_hash is required for prompt-version dedup and \
                                     is always computed. Remove the setting to silence this warning."
                                );
                            }
                            // `channels.file_upload_max_bytes` is dead config
                            // since the in-process channel adapters moved to
                            // sidecars (#5317–#5459); it is not threaded into the
                            // sidecar send path. Warn only when an operator has
                            // tuned it away from the default so they are not
                            // misled into thinking a tighter cap took effect.
                            if config.channels.file_upload_max_bytes
                                != librefang_types::config::ChannelsConfig::default()
                                    .file_upload_max_bytes
                            {
                                tracing::warn!(
                                    configured = config.channels.file_upload_max_bytes,
                                    "[channels] file_upload_max_bytes is currently NOT enforced: \
                                     the in-process adapters it configured were replaced by \
                                     out-of-process sidecars. Outbound uploads are bounded by the \
                                     sidecar's hardcoded cap; honouring this value requires a \
                                     sidecar-crate change."
                                );
                            }
                            // Write migrated config back to disk so future loads skip migration.
                            //
                            // Managed mode owns the file, so the write is skipped rather than attempted and warned about.
                            // Attempting it against a read-only ConfigMap mount fails on every boot, and because the failure is only a `warn!` the migration silently re-runs forever with nothing but a repeating log line to show for it.
                            // The in-memory config is already migrated either way, so skipping costs nothing at runtime — what the operator loses is the on-disk persistence, and they need to know that their manifest is now a schema version behind.
                            if migrated
                                && file_version < CONFIG_VERSION
                                && config_mode() == ConfigMode::Managed
                            {
                                tracing::warn!(
                                    path = %config_path.display(),
                                    from_version = file_version,
                                    to_version = CONFIG_VERSION,
                                    "Config was migrated in memory but NOT written back: the file is deployment-managed (LIBREFANG_CONFIG_MODE=managed). \
                                     This migration re-runs on every boot until the managed source is updated to the new schema."
                                );
                            } else if migrated && file_version < CONFIG_VERSION {
                                let toml_str = toml::to_string_pretty(&config);
                                match toml_str {
                                    Ok(s) => {
                                        if let Err(e) = atomic_write_config(&config_path, &s) {
                                            tracing::warn!(
                                                error = %e,
                                                path = %config_path.display(),
                                                "Failed to write migrated config back to disk"
                                            );
                                        } else {
                                            info!(
                                                path = %config_path.display(),
                                                "Wrote migrated config to disk"
                                            );
                                        }
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            error = %e,
                                            "Failed to serialize migrated config"
                                        );
                                    }
                                }
                            }
                            info!(path = %config_path.display(), "Loaded configuration");
                            return Ok(config);
                        }
                        Err(e) => {
                            let msg = format!(
                                "Config file cannot be deserialized (path={}): {e}",
                                config_path.display()
                            );
                            eprintln!("error: {msg}");
                            return Err(msg);
                        }
                    }
                }
                Err(e) => {
                    let msg = format!(
                        "Config file has invalid TOML and cannot be loaded (path={}): {e}",
                        config_path.display()
                    );
                    eprintln!("error: {msg}");
                    return Err(msg);
                }
            },
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    path = %config_path.display(),
                    "Failed to read config file, using defaults"
                );
            }
        }
    } else {
        info!(
            path = %config_path.display(),
            "Config file not found, using defaults"
        );
    }

    Ok(KernelConfig::default())
}

/// Strict counterpart of [`load_config`] that returns `Err` on every failure
/// mode instead of silently falling back to [`KernelConfig::default`].
///
/// Used by [`crate::kernel::Kernel::reload_config`] (issue #4664) so a bad
/// on-disk config — TOML syntax error, broken `include = [...]`, migration
/// failure, deserialize-shape mismatch — never wipes the operator's live
/// in-memory state. The hot-reload watcher and `POST /api/config/reload`
/// handler both already map `Err(...)` to a warning + 400 respectively,
/// so the live config stays intact and the operator gets an actionable
/// error.
///
/// Differences from `load_config`:
///
/// - No write-back of the migrated TOML to disk. The reload path doesn't
///   own the file, and a partial migration that fails downstream would
///   leave the disk file in a half-migrated state. The next initial-boot
///   `load_config` call still does the write-back.
/// - Unknown fields under `strict_config = true` produce `Err` instead of
///   "return defaults with `strict_config: true`" — same intent, but
///   surfaced as an error so the reload path can refuse to apply it.
/// - Unknown fields under `strict_config = false` (or unset) still warn
///   and proceed, matching `load_config`'s tolerant behaviour.
pub fn try_load_config(path: &Path) -> Result<KernelConfig, String> {
    if !path.exists() {
        return Err(format!("Config file not found: {}", path.display()));
    }
    let contents =
        std::fs::read_to_string(path).map_err(|e| format!("Failed to read config file: {e}"))?;
    let mut root_value: toml::Value =
        toml::from_str(&contents).map_err(|e| format!("Config file has invalid TOML: {e}"))?;

    // Resolve `include = [...]` chains. Failures here include: missing
    // file, unparseable include, traversal / absolute-path attempts,
    // circular references, depth overflow.
    let config_dir = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let mut visited = HashSet::new();
    if let Ok(canonical) = std::fs::canonicalize(path) {
        visited.insert(canonical);
    } else {
        visited.insert(path.to_path_buf());
    }
    resolve_config_includes(&mut root_value, &config_dir, &mut visited, 0)
        .map_err(|e| format!("Config include resolution failed: {e}"))?;
    if let toml::Value::Table(ref mut tbl) = root_value {
        tbl.remove("include");
    }

    // Migrate older config versions in place. We do NOT write the result
    // back to disk from the reload path (see doc comment above).
    let file_version = root_value
        .as_table()
        .and_then(|t| t.get("config_version"))
        .and_then(|v| v.as_integer())
        .map(|v| v as u32)
        .unwrap_or_else(default_config_version);
    if file_version < CONFIG_VERSION {
        run_migrations(&mut root_value, file_version).map_err(|e| {
            format!("Config migration failed (from v{file_version} to v{CONFIG_VERSION}): {e}")
        })?;
    }

    // Strict mode: refuse to load when unknown / typo'd fields are present.
    let unknown_fields = KernelConfig::detect_unknown_fields(&root_value);
    let unknown_nested = KernelConfig::detect_unknown_nested_fields(&root_value);
    let is_strict = root_value
        .as_table()
        .and_then(|t| t.get("strict_config"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let mut all_unknown: Vec<String> = unknown_fields.into_iter().chain(unknown_nested).collect();
    all_unknown.sort();
    if !all_unknown.is_empty() {
        if is_strict {
            return Err(format!(
                "strict_config is enabled and config contains unknown fields: {}",
                all_unknown.join(", ")
            ));
        }
        for field in &all_unknown {
            tracing::warn!(field, "Unknown config field (ignored on reload)");
        }
    }

    // #5476: same misplaced per-agent override warning as `load_config`,
    // applied on hot-reload so an operator who tries to "fix" the
    // override by editing config.toml + `POST /api/config/reload` still
    // sees the actionable hint instead of silent no-op.
    for (agent, key) in KernelConfig::detect_misplaced_per_agent_overrides(&root_value) {
        tracing::warn!(
            agent = %agent,
            key = %key,
            "[agents.{agent}.{key}] in config.toml is ignored on reload; \
             per-agent overrides live in {{workspace}}/agent.toml's \
             top-level [{key}] block (or the [agents.{agent}] section of \
             a HAND.toml), not in config.toml — see #5476"
        );
    }

    // Same legacy `[channels.<vendor>]` migration warning as `load_config`, applied on hot-reload so an operator who pastes their old block back and `POST /api/config/reload`s still gets the actionable hint instead of a silent no-op.
    for vendor in KernelConfig::detect_legacy_channel_blocks(&root_value) {
        tracing::warn!(
            channel = %vendor,
            "[channels.{vendor}] in config.toml is ignored on reload: in-process \
             channels migrated to out-of-process sidecars. Re-add it under \
             [[sidecar_channels]] (Dashboard → Channels → Add, or hand-edit \
             config.toml) and install the sidecar SDK ('pip install \
             librefang-sdk') — see docs/architecture/sidecar-channels.md"
        );
    }

    root_value
        .try_into::<KernelConfig>()
        .map_err(|e| format!("Failed to deserialize config: {e}"))
}

/// Resolve config includes by deep-merging included files into the root value.
///
/// Included files are loaded first and the root config overrides them.
/// Security: rejects absolute paths, `..` components, and circular references.
fn resolve_config_includes(
    root_value: &mut toml::Value,
    config_dir: &Path,
    visited: &mut HashSet<PathBuf>,
    depth: u32,
) -> Result<(), String> {
    if depth > MAX_INCLUDE_DEPTH {
        return Err(format!(
            "Config include depth exceeded maximum of {MAX_INCLUDE_DEPTH}"
        ));
    }

    // Extract include list from the current value
    let includes = match root_value {
        toml::Value::Table(tbl) => {
            if let Some(toml::Value::Array(arr)) = tbl.get("include") {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect::<Vec<_>>()
            } else {
                return Ok(());
            }
        }
        _ => return Ok(()),
    };

    if includes.is_empty() {
        return Ok(());
    }

    // Merge each include (earlier includes are overridden by later ones,
    // and the root config overrides everything).
    let mut merged_base = toml::Value::Table(toml::map::Map::new());

    for include_path_str in &includes {
        // SECURITY: reject absolute paths
        let include_path = Path::new(include_path_str);
        if include_path.is_absolute() {
            return Err(format!(
                "Config include rejects absolute path: {include_path_str}"
            ));
        }
        // SECURITY: reject `..` components
        for component in include_path.components() {
            if let std::path::Component::ParentDir = component {
                return Err(format!(
                    "Config include rejects path traversal: {include_path_str}"
                ));
            }
        }

        let resolved = config_dir.join(include_path);
        // SECURITY: verify resolved path stays within config dir
        let canonical = std::fs::canonicalize(&resolved).map_err(|e| {
            format!(
                "Config include '{}' cannot be resolved: {e}",
                include_path_str
            )
        })?;
        let canonical_dir = std::fs::canonicalize(config_dir)
            .map_err(|e| format!("Config dir cannot be canonicalized: {e}"))?;
        if !canonical.starts_with(&canonical_dir) {
            return Err(format!(
                "Config include '{}' escapes config directory",
                include_path_str
            ));
        }

        // SECURITY: circular detection
        if !visited.insert(canonical.clone()) {
            return Err(format!(
                "Circular config include detected: {include_path_str}"
            ));
        }

        info!(include = %include_path_str, "Loading config include");

        let contents = std::fs::read_to_string(&canonical)
            .map_err(|e| format!("Failed to read config include '{}': {e}", include_path_str))?;
        let mut include_value: toml::Value = toml::from_str(&contents)
            .map_err(|e| format!("Failed to parse config include '{}': {e}", include_path_str))?;

        // Recursively resolve includes in the included file
        let include_dir = canonical.parent().unwrap_or(config_dir).to_path_buf();
        resolve_config_includes(&mut include_value, &include_dir, visited, depth + 1)?;
        // `visited` models the active recursion stack, not every file seen
        // during the whole traversal. A shared dependency in a diamond graph
        // is valid once the first branch has unwound.
        visited.remove(&canonical);

        // Remove include field from the included file
        if let toml::Value::Table(ref mut tbl) = include_value {
            tbl.remove("include");
        }

        // Deep merge: include overrides the base built so far
        deep_merge_toml(&mut merged_base, &include_value);
    }

    // Now deep merge: root overrides the merged includes
    // Save root's current values (minus include), then merge root on top
    let root_without_include = {
        let mut v = root_value.clone();
        if let toml::Value::Table(ref mut tbl) = v {
            tbl.remove("include");
        }
        v
    };
    deep_merge_toml(&mut merged_base, &root_without_include);
    *root_value = merged_base;

    Ok(())
}

/// Deep-merge two TOML values. `overlay` values override `base` values.
/// For tables, recursively merge. For everything else, overlay wins.
pub fn deep_merge_toml(base: &mut toml::Value, overlay: &toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(base_tbl), toml::Value::Table(overlay_tbl)) => {
            for (key, overlay_val) in overlay_tbl {
                if let Some(base_val) = base_tbl.get_mut(key) {
                    deep_merge_toml(base_val, overlay_val);
                } else {
                    base_tbl.insert(key.clone(), overlay_val.clone());
                }
            }
        }
        (base, overlay) => {
            *base = overlay.clone();
        }
    }
}

/// Env var that relocates `config.toml` away from `LIBREFANG_HOME`.
///
/// Relocation and locking are deliberately separate concerns — see [`ConfigMode`].
pub const CONFIG_PATH_ENV: &str = "LIBREFANG_CONFIG_PATH";

/// Env var that selects [`ConfigMode`]. Only the exact value `managed` locks.
pub const CONFIG_MODE_ENV: &str = "LIBREFANG_CONFIG_MODE";

/// Who owns `config.toml`.
///
/// The mode is read from the process environment, never from the config file itself, so a write through the API can never unlock the very file it is being refused access to (#6695).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigMode {
    /// Default. The file is application state: the API may persist to it.
    Mutable,
    /// The file is owned by the deployment (a Kubernetes ConfigMap, a bind mount, a config management system).
    /// Every API surface that would persist to it refuses with `423 Locked`.
    Managed,
}

impl ConfigMode {
    /// Whether configuration may be persisted through the API in this mode.
    pub fn is_writable(self) -> bool {
        matches!(self, ConfigMode::Mutable)
    }

    /// The stable wire string used by the API and the dashboard.
    pub fn as_str(self) -> &'static str {
        match self {
            ConfigMode::Mutable => "mutable",
            ConfigMode::Managed => "managed",
        }
    }
}

/// Resolve the configuration ownership mode from the environment.
///
/// Anything other than a case-insensitive `managed` — including unset, empty, and a typo — resolves to [`ConfigMode::Mutable`].
/// Defaulting a typo to the locked mode would brick the dashboard for a deployment that never asked for it; defaulting it to mutable preserves today's behaviour, which is the compatibility guarantee the RFC requires.
pub fn config_mode() -> ConfigMode {
    match std::env::var(CONFIG_MODE_ENV) {
        Ok(v) if v.trim().eq_ignore_ascii_case("managed") => ConfigMode::Managed,
        _ => ConfigMode::Mutable,
    }
}

/// Get the default config file path.
///
/// Priority: `LIBREFANG_CONFIG_PATH` > `LIBREFANG_HOME`/config.toml > `~/.librefang/config.toml`.
///
/// `LIBREFANG_CONFIG_PATH` relocates the file and nothing more.
/// It is independent of [`config_mode`] on purpose: a Compose deployment may reasonably bind-mount the config directory somewhere outside `LIBREFANG_HOME` while still editing it from the dashboard, and inferring the lock from the path would hand that operator a read-only UI they never asked for.
pub fn default_config_path() -> PathBuf {
    config_path_override().unwrap_or_else(|| librefang_home().join("config.toml"))
}

/// The `LIBREFANG_CONFIG_PATH` override, or `None` when it is unset, empty, or whitespace.
///
/// The single place that reads the variable, so relocation cannot be honoured by one resolver and ignored by another.
fn config_path_override() -> Option<PathBuf> {
    let raw = std::env::var(CONFIG_PATH_ENV).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

/// Resolve the `config.toml` that backs an already-loaded [`KernelConfig`].
///
/// `LIBREFANG_CONFIG_PATH` wins exactly as it does in [`default_config_path`]; otherwise the file sits in the config's own `home_dir`.
/// This is the resolver for [`crate::LibreFangKernel::boot_with_config`], where an embedder handed over a `KernelConfig` without saying where it came from — `boot` records the path it actually loaded from instead.
///
/// It deliberately reads `home_dir` from the config rather than from `LIBREFANG_HOME`: an embedder that builds a `KernelConfig` in memory and points `home_dir` at a scratch directory means that directory, and resolving to the real user's `~/.librefang/config.toml` would have such a process write over a config it never read.
pub fn config_path_for(config: &KernelConfig) -> PathBuf {
    config_path_override().unwrap_or_else(|| config.home_dir.join("config.toml"))
}

/// Every file that contributes to the effective configuration, the primary file first (#6695).
///
/// Deliberately tolerant where [`resolve_config_includes`] is strict: this feeds a status endpoint, not a loader, so a broken `include` chain yields a shorter list rather than an error.
/// The traversal applies the same two security rules — absolute paths and `..` components are skipped, never followed — and dedupes across the whole walk so a diamond graph lists a shared file once and a cycle terminates instead of hanging.
///
/// Paths are canonicalised where possible, so the primary file appears exactly once even when the caller passed a path through a symlink.
pub fn config_source_files(path: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut seen: std::collections::BTreeSet<PathBuf> = std::collections::BTreeSet::new();
    collect_config_sources(path, 0, &mut seen, &mut out);
    out
}

fn collect_config_sources(
    path: &Path,
    depth: u32,
    seen: &mut std::collections::BTreeSet<PathBuf>,
    out: &mut Vec<PathBuf>,
) {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if !seen.insert(canonical.clone()) {
        return;
    }
    if !canonical.is_file() {
        return;
    }
    out.push(canonical.clone());
    if depth >= MAX_INCLUDE_DEPTH {
        return;
    }

    let Ok(text) = std::fs::read_to_string(&canonical) else {
        return;
    };
    let Ok(toml::Value::Table(table)) = toml::from_str::<toml::Value>(&text) else {
        return;
    };
    let Some(toml::Value::Array(includes)) = table.get("include") else {
        return;
    };
    let dir = canonical
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    for value in includes {
        let Some(relative) = value.as_str() else {
            continue;
        };
        let relative = Path::new(relative);
        if relative.is_absolute()
            || relative
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            continue;
        }
        collect_config_sources(&dir.join(relative), depth + 1, seen, out);
    }
}

/// Provenance of the effective configuration, for the authenticated status endpoint.
///
/// Deliberately carries no secret material: the checksum is over file bytes, which the caller already has to be authenticated to influence, and the paths are deployment facts an operator needs in order to know where to make a change.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ConfigProvenance {
    /// `"mutable"` or `"managed"`.
    pub mode: &'static str,
    /// Absolute path the effective configuration was loaded from.
    pub source: String,
    /// Whether the API will accept a write.
    /// Equivalent to `mode == "mutable"`, surfaced separately so the dashboard branches on a boolean rather than string-matching a mode name.
    pub writable: bool,
    /// `sha256:<hex>` covering every file that contributes to the effective configuration, or `None` when the primary file does not exist.
    ///
    /// With no `include`, this is the digest of the primary file's raw bytes, unchanged from before #6695 so an existing `checksum/config` rollout annotation keeps matching.
    ///
    /// With includes, hashing the primary file alone would report a rollout that never happened — an edit to an included file changes the effective configuration and leaves the primary byte-identical.
    /// The composite is therefore the digest of `sha256sum` output over the source files in include order, relative to the primary file's directory, which an operator can reproduce with
    /// `(cd /etc/librefang && sha256sum config.toml extra.toml) | sha256sum`.
    pub checksum: Option<String>,
    /// Paths of the included files that contribute, relative to the primary file's directory, in include order.
    ///
    /// Empty for the ordinary single-file deployment. Non-empty means `checksum` is the composite form described above.
    pub includes: Vec<String>,
    /// RFC 3339 timestamp of the most recent modification across every source file, or `None` when unavailable.
    ///
    /// Across every source rather than the primary alone for the same reason as the checksum: an operator watching this field to confirm an edit landed must not be told nothing happened.
    pub modified_at: Option<String>,
}

/// Build the provenance record for the config file currently in effect.
pub fn config_provenance(path: Option<&Path>) -> ConfigProvenance {
    use sha2::{Digest, Sha256};

    let config_path = path
        .map(|p| p.to_path_buf())
        .unwrap_or_else(default_config_path);
    let mode = config_mode();

    let sources = config_source_files(&config_path);
    let base_dir = sources
        .first()
        .and_then(|p| p.parent())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let checksum = match sources.len() {
        // No includes: the historical digest, byte for byte.
        0 | 1 => std::fs::read(&config_path)
            .ok()
            .map(|bytes| format!("sha256:{:x}", Sha256::digest(&bytes))),
        _ => {
            let mut manifest = String::new();
            for source in &sources {
                let Ok(bytes) = std::fs::read(source) else {
                    continue;
                };
                let name = source
                    .strip_prefix(&base_dir)
                    .unwrap_or(source.as_path())
                    .display();
                manifest.push_str(&format!("{:x}  {name}\n", Sha256::digest(&bytes)));
            }
            Some(format!("sha256:{:x}", Sha256::digest(manifest.as_bytes())))
        }
    };

    let includes = sources
        .iter()
        .skip(1)
        .map(|p| {
            p.strip_prefix(&base_dir)
                .unwrap_or(p.as_path())
                .display()
                .to_string()
        })
        .collect();

    let modified_at = sources
        .iter()
        .filter_map(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
        .max()
        .or_else(|| {
            std::fs::metadata(&config_path)
                .and_then(|m| m.modified())
                .ok()
        })
        .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339());

    ConfigProvenance {
        mode: mode.as_str(),
        source: config_path.display().to_string(),
        writable: mode.is_writable(),
        checksum,
        includes,
        modified_at,
    }
}

/// Get the LibreFang home directory.
///
/// Priority: `LIBREFANG_HOME` env var > `~/.librefang`.
pub fn librefang_home() -> PathBuf {
    if let Ok(home) = std::env::var("LIBREFANG_HOME") {
        return PathBuf::from(home);
    }
    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".librefang")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_load_config_defaults() {
        let config = load_config(None).unwrap();
        assert_eq!(config.log_level, "info");
    }

    #[test]
    fn test_load_config_missing_file() {
        let config = load_config(Some(Path::new("/nonexistent/config.toml"))).unwrap();
        assert_eq!(config.log_level, "info");
    }

    #[test]
    fn atomic_config_write_never_exposes_partial_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let seed = "config_version = 2\n".to_string();
        let payload_a = format!("marker = \"{}\"\n", "a".repeat(16 * 1024));
        let payload_b = format!("marker = \"{}\"\n", "b".repeat(16 * 1024));
        std::fs::write(&path, &seed).unwrap();

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let writer = |payload: String| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                for _ in 0..30 {
                    atomic_write_config(&path, &payload).unwrap();
                }
            })
        };
        let writer_a = writer(payload_a.clone());
        let writer_b = writer(payload_b.clone());
        barrier.wait();

        for _ in 0..200 {
            let observed = std::fs::read_to_string(&path).unwrap();
            assert!(
                observed == seed || observed == payload_a || observed == payload_b,
                "observed partial config: {} bytes",
                observed.len()
            );
        }
        writer_a.join().unwrap();
        writer_b.join().unwrap();

        let final_content = std::fs::read_to_string(path).unwrap();
        assert!(final_content == payload_a || final_content == payload_b);
    }

    #[cfg(unix)]
    #[test]
    fn atomic_config_write_preserves_symlink_and_target_permissions() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("operator-config.toml");
        let link = dir.path().join("config.toml");
        std::fs::write(&target, "old = true\n").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
        symlink(&target, &link).unwrap();

        atomic_write_config(&link, "new = true\n").unwrap();

        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new = true\n");
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[cfg(unix)]
    #[test]
    fn atomic_config_write_refuses_a_read_only_target() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "old = true\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o400)).unwrap();

        let error = atomic_write_config(&path, "new = true\n").unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(std::fs::read_to_string(path).unwrap(), "old = true\n");
    }

    #[test]
    fn test_deep_merge_simple() {
        let mut base: toml::Value = toml::from_str(
            r#"
            log_level = "debug"
            api_listen = "0.0.0.0:4545"
        "#,
        )
        .unwrap();
        let overlay: toml::Value = toml::from_str(
            r#"
            log_level = "info"
            network_enabled = true
        "#,
        )
        .unwrap();
        deep_merge_toml(&mut base, &overlay);
        assert_eq!(base["log_level"].as_str(), Some("info"));
        assert_eq!(base["api_listen"].as_str(), Some("0.0.0.0:4545"));
        assert_eq!(base["network_enabled"].as_bool(), Some(true));
    }

    #[test]
    fn test_deep_merge_nested_tables() {
        let mut base: toml::Value = toml::from_str(
            r#"
            [memory]
            decay_rate = 0.1
            consolidation_threshold = 10000
        "#,
        )
        .unwrap();
        let overlay: toml::Value = toml::from_str(
            r#"
            [memory]
            decay_rate = 0.5
        "#,
        )
        .unwrap();
        deep_merge_toml(&mut base, &overlay);
        let mem = base["memory"].as_table().unwrap();
        assert_eq!(mem["decay_rate"].as_float(), Some(0.5));
        assert_eq!(mem["consolidation_threshold"].as_integer(), Some(10000));
    }

    /// A deployment with no `include` must keep the exact digest the `checksum/config` rollout
    /// annotation was generated from, or #7902's overlay and its kind e2e assertion both break.
    #[test]
    fn provenance_checksum_of_a_single_file_is_the_raw_byte_digest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, b"log_level = \"info\"\n").unwrap();

        let provenance = config_provenance(Some(&path));

        use sha2::{Digest, Sha256};
        let expected = format!("sha256:{:x}", Sha256::digest(std::fs::read(&path).unwrap()));
        assert_eq!(provenance.checksum.as_deref(), Some(expected.as_str()));
        assert!(provenance.includes.is_empty(), "{:?}", provenance.includes);
    }

    /// The defect this fixes: an edit to an included file changes the effective configuration,
    /// so a checksum that ignores it tells an operator a rollout landed when it did not.
    #[test]
    fn provenance_checksum_covers_included_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");
        let extra = dir.path().join("extra.toml");
        std::fs::write(&root, b"include = [\"extra.toml\"]\nlog_level = \"info\"\n").unwrap();
        std::fs::write(&extra, b"api_listen = \"0.0.0.0:4545\"\n").unwrap();

        let before = config_provenance(Some(&root));
        assert_eq!(before.includes, vec!["extra.toml".to_string()]);

        let root_bytes_digest = {
            use sha2::{Digest, Sha256};
            format!("sha256:{:x}", Sha256::digest(std::fs::read(&root).unwrap()))
        };
        assert_ne!(
            before.checksum.as_deref(),
            Some(root_bytes_digest.as_str()),
            "with includes the checksum must be the composite, not the primary file alone"
        );

        std::fs::write(&extra, b"api_listen = \"0.0.0.0:9999\"\n").unwrap();
        let after = config_provenance(Some(&root));

        assert_eq!(
            std::fs::read(&root).unwrap(),
            b"include = [\"extra.toml\"]\nlog_level = \"info\"\n",
            "the primary file is deliberately untouched — that is the whole point of the case"
        );
        assert_ne!(
            before.checksum, after.checksum,
            "editing an included file must move the checksum"
        );
    }

    /// The published reproduction command has to actually reproduce it, or an operator
    /// comparing a ConfigMap annotation against the API gets a mismatch with no explanation.
    #[test]
    fn composite_checksum_matches_the_documented_sha256sum_pipeline() {
        use sha2::{Digest, Sha256};

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");
        let extra = dir.path().join("extra.toml");
        std::fs::write(&root, b"include = [\"extra.toml\"]\n").unwrap();
        std::fs::write(&extra, b"log_level = \"debug\"\n").unwrap();

        // `sha256sum config.toml extra.toml | sha256sum`, spelled out.
        let mut manifest = String::new();
        for name in ["config.toml", "extra.toml"] {
            let bytes = std::fs::read(dir.path().join(name)).unwrap();
            manifest.push_str(&format!("{:x}  {name}\n", Sha256::digest(&bytes)));
        }
        let expected = format!("sha256:{:x}", Sha256::digest(manifest.as_bytes()));

        assert_eq!(
            config_provenance(Some(&root)).checksum.as_deref(),
            Some(expected.as_str())
        );
    }

    /// Provenance feeds a status endpoint, so a broken chain must degrade rather than error.
    #[test]
    fn source_walk_skips_unsafe_and_missing_includes_instead_of_failing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");
        std::fs::write(
            &root,
            b"include = [\"/etc/passwd\", \"../escape.toml\", \"missing.toml\"]\n",
        )
        .unwrap();

        let sources = config_source_files(&root);
        assert_eq!(
            sources.len(),
            1,
            "only the primary file contributes; got {sources:?}"
        );
        assert!(config_provenance(Some(&root)).includes.is_empty());
    }

    /// A cycle makes the loader error; the provenance walk must terminate and list each file once.
    #[test]
    fn source_walk_terminates_on_a_circular_include() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("config.toml");
        let b = dir.path().join("b.toml");
        std::fs::write(&a, b"include = [\"b.toml\"]\n").unwrap();
        std::fs::write(&b, b"include = [\"config.toml\"]\n").unwrap();

        let sources = config_source_files(&a);
        assert_eq!(sources.len(), 2, "{sources:?}");
    }

    #[test]
    fn test_basic_include() {
        let dir = tempfile::tempdir().unwrap();
        let base_path = dir.path().join("base.toml");
        let root_path = dir.path().join("config.toml");

        // Base config
        let mut f = std::fs::File::create(&base_path).unwrap();
        writeln!(f, "log_level = \"debug\"").unwrap();
        writeln!(f, "api_listen = \"0.0.0.0:9999\"").unwrap();
        drop(f);

        // Root config (includes base, overrides log_level)
        let mut f = std::fs::File::create(&root_path).unwrap();
        writeln!(f, "include = [\"base.toml\"]").unwrap();
        writeln!(f, "log_level = \"warn\"").unwrap();
        drop(f);

        let config = load_config(Some(&root_path)).unwrap();
        assert_eq!(config.log_level, "warn"); // root overrides
        assert_eq!(config.api_listen, "0.0.0.0:9999"); // from base
    }

    #[test]
    fn test_nested_include() {
        let dir = tempfile::tempdir().unwrap();
        let grandchild = dir.path().join("grandchild.toml");
        let child = dir.path().join("child.toml");
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&grandchild).unwrap();
        writeln!(f, "log_level = \"trace\"").unwrap();
        drop(f);

        let mut f = std::fs::File::create(&child).unwrap();
        writeln!(f, "include = [\"grandchild.toml\"]").unwrap();
        writeln!(f, "log_level = \"debug\"").unwrap();
        drop(f);

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "include = [\"child.toml\"]").unwrap();
        writeln!(f, "log_level = \"info\"").unwrap();
        drop(f);

        let config = load_config(Some(&root)).unwrap();
        assert_eq!(config.log_level, "info"); // root wins
    }

    #[test]
    fn test_diamond_include_is_not_treated_as_a_cycle() {
        let dir = tempfile::tempdir().unwrap();
        let shared = dir.path().join("shared.toml");
        let left = dir.path().join("left.toml");
        let right = dir.path().join("right.toml");
        let root = dir.path().join("config.toml");

        std::fs::write(&shared, "network_enabled = true\n").unwrap();
        std::fs::write(
            &left,
            "include = [\"shared.toml\"]\nlog_level = \"debug\"\n",
        )
        .unwrap();
        std::fs::write(
            &right,
            "include = [\"shared.toml\"]\napi_listen = \"127.0.0.1:5555\"\n",
        )
        .unwrap();
        std::fs::write(
            &root,
            format!(
                "config_version = {CONFIG_VERSION}\ninclude = [\"left.toml\", \"right.toml\"]\n"
            ),
        )
        .unwrap();

        let config = try_load_config(&root).expect("diamond includes must be valid");
        assert!(config.network_enabled, "shared config must be retained");
        assert_eq!(config.log_level, "debug");
        assert_eq!(config.api_listen, "127.0.0.1:5555");
    }

    #[test]
    fn test_circular_include_detected() {
        let dir = tempfile::tempdir().unwrap();
        let a_path = dir.path().join("a.toml");
        let b_path = dir.path().join("b.toml");

        let mut f = std::fs::File::create(&a_path).unwrap();
        writeln!(f, "config_version = {CONFIG_VERSION}").unwrap();
        writeln!(f, "include = [\"b.toml\"]").unwrap();
        writeln!(f, "log_level = \"info\"").unwrap();
        drop(f);

        let mut f = std::fs::File::create(&b_path).unwrap();
        writeln!(f, "include = [\"a.toml\"]").unwrap();
        drop(f);

        // Include errors are tolerated — the root file still loads with its own fields.
        let config = load_config(Some(&a_path)).unwrap();
        assert!(!config.log_level.is_empty());

        let error = try_load_config(&a_path).expect_err("real cycles must remain invalid");
        assert!(
            error.contains("Circular config include detected"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn test_path_traversal_blocked() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "include = [\"../etc/passwd\"]").unwrap();
        drop(f);

        // Include-chain security errors are tolerated; root-only config still loads.
        let config = load_config(Some(&root)).unwrap();
        assert_eq!(config.log_level, "info"); // defaults (root was just an include directive)
    }

    #[test]
    fn test_max_depth_exceeded() {
        let dir = tempfile::tempdir().unwrap();

        // Create a chain of 12 files (exceeds MAX_INCLUDE_DEPTH=10)
        for i in (0..12).rev() {
            let name = format!("level{i}.toml");
            let path = dir.path().join(&name);
            let mut f = std::fs::File::create(&path).unwrap();
            if i < 11 {
                let next = format!("level{}.toml", i + 1);
                writeln!(f, "include = [\"{next}\"]").unwrap();
            }
            writeln!(f, "log_level = \"level{i}\"").unwrap();
            drop(f);
        }

        let root = dir.path().join("level0.toml");
        // Include depth overflow is tolerated; the root file still loads.
        let config = load_config(Some(&root)).unwrap();
        assert!(!config.log_level.is_empty());
    }

    #[test]
    fn test_absolute_path_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "include = [\"/etc/shadow\"]").unwrap();
        drop(f);

        // Include-chain security errors are tolerated; root-only config still loads.
        let config = load_config(Some(&root)).unwrap();
        assert_eq!(config.log_level, "info"); // defaults
    }

    #[test]
    fn test_no_includes_works() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "log_level = \"trace\"").unwrap();
        drop(f);

        let config = load_config(Some(&root)).unwrap();
        assert_eq!(config.log_level, "trace");
    }

    // --- Tolerant / strict config mode tests ---

    #[test]
    fn test_tolerant_mode_loads_with_unknown_fields() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "log_level = \"debug\"").unwrap();
        writeln!(f, "unknown_field_xyz = 42").unwrap();
        writeln!(f, "another_typo = true").unwrap();
        drop(f);

        // Tolerant mode (default): should still load successfully
        let config = load_config(Some(&root)).unwrap();
        assert_eq!(config.log_level, "debug");
        assert!(!config.strict_config);
    }

    #[test]
    fn test_strict_mode_rejects_unknown_fields() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "strict_config = true").unwrap();
        writeln!(f, "log_level = \"debug\"").unwrap();
        writeln!(f, "bogus_field = \"oops\"").unwrap();
        drop(f);

        // Strict mode: should reject and return defaults (with strict_config=true)
        let config = load_config(Some(&root)).unwrap();
        // Falls back to defaults because strict mode rejected unknown fields
        assert_eq!(config.log_level, "info"); // default, not "debug"
        assert!(config.strict_config);
    }

    #[test]
    fn test_strict_mode_accepts_clean_config() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "strict_config = true").unwrap();
        writeln!(f, "log_level = \"warn\"").unwrap();
        drop(f);

        // Strict mode with no unknown fields: should load normally
        let config = load_config(Some(&root)).unwrap();
        assert_eq!(config.log_level, "warn");
        assert!(config.strict_config);
    }

    #[test]
    fn test_tolerant_mode_with_explicit_false() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "strict_config = false").unwrap();
        writeln!(f, "log_level = \"error\"").unwrap();
        writeln!(f, "not_a_real_field = 123").unwrap();
        drop(f);

        // Explicitly tolerant: should load despite unknown field
        let config = load_config(Some(&root)).unwrap();
        assert_eq!(config.log_level, "error");
        assert!(!config.strict_config);
    }

    #[test]
    fn test_load_config_migrates_v1_api_section() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        // v1 config with [api] section (no config_version field)
        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "[api]").unwrap();
        writeln!(f, "api_key = \"my-secret\"").unwrap();
        writeln!(f, "api_listen = \"0.0.0.0:9999\"").unwrap();
        drop(f);

        let config = load_config(Some(&root)).unwrap();
        assert_eq!(config.api_key, "my-secret");
        assert_eq!(config.api_listen, "0.0.0.0:9999");
        assert_eq!(config.config_version, CONFIG_VERSION);

        // Verify the migrated file was written back
        let contents = std::fs::read_to_string(&root).unwrap();
        assert!(
            contents.contains("config_version"),
            "migrated file should contain config_version"
        );
        assert!(
            !contents.contains("[api]"),
            "migrated file should not contain [api] section"
        );
    }

    #[test]
    fn test_load_config_v2_skips_migration() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "config_version = 2").unwrap();
        writeln!(f, "log_level = \"debug\"").unwrap();
        drop(f);

        let config = load_config(Some(&root)).unwrap();
        assert_eq!(config.log_level, "debug");
        assert_eq!(config.config_version, 2);
    }

    #[test]
    fn test_load_config_default_has_current_version() {
        let config = KernelConfig::default();
        assert_eq!(config.config_version, CONFIG_VERSION);
    }

    #[test]
    fn test_load_config_with_users_block() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "[[users]]").unwrap();
        writeln!(f, "name = \"Alice\"").unwrap();
        writeln!(f, "role = \"owner\"").unwrap();
        drop(f);

        let config = load_config(Some(&root)).unwrap();
        assert_eq!(config.users.len(), 1);
        assert_eq!(config.users[0].name, "Alice");
        assert_eq!(config.users[0].role, "owner");
        assert!(config.users[0].channel_bindings.is_empty());
        assert!(config.users[0].api_key_hash.is_none());
    }

    #[test]
    fn test_load_config_with_users_and_channel_bindings() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "[[users]]").unwrap();
        writeln!(f, "name = \"Alice\"").unwrap();
        writeln!(f, "role = \"owner\"").unwrap();
        writeln!(f, "[users.channel_bindings]").unwrap();
        writeln!(f, "telegram = \"123456\"").unwrap();
        drop(f);

        let config = load_config(Some(&root)).unwrap();
        assert_eq!(config.users.len(), 1);
        assert_eq!(config.users[0].name, "Alice");
        assert_eq!(
            config.users[0].channel_bindings.get("telegram").unwrap(),
            "123456"
        );
    }

    #[test]
    fn test_load_config_users_migration_roundtrip() {
        // When no config_version is present, migration runs and writes back.
        // Verify that the round-trip preserves users data.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "log_level = \"debug\"").unwrap();
        writeln!(f, "[[users]]").unwrap();
        writeln!(f, "name = \"Alice\"").unwrap();
        writeln!(f, "role = \"owner\"").unwrap();
        drop(f);

        // First load: triggers migration (v1 → v2), writes back
        let config1 = load_config(Some(&root)).unwrap();
        assert_eq!(config1.log_level, "debug");
        assert_eq!(config1.users.len(), 1);
        assert_eq!(config1.users[0].name, "Alice");
        assert_eq!(config1.config_version, CONFIG_VERSION);

        // Second load: reads migrated file, no migration needed
        let config2 = load_config(Some(&root)).unwrap();
        assert_eq!(config2.log_level, "debug");
        assert_eq!(config2.users.len(), 1);
        assert_eq!(config2.users[0].name, "Alice");
        assert_eq!(config2.config_version, CONFIG_VERSION);
    }

    #[test]
    fn test_migrated_config_no_users_array_key() {
        // When the migration writes back a config WITHOUT [[users]] entries,
        // it must NOT emit `users = []` (an inline empty array). If it does,
        // a subsequent manual `[[users]]` addition by the user would produce
        // duplicate `users` keys in the TOML file, breaking the next parse.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "log_level = \"debug\"").unwrap();
        // No [[users]] section — migration should not add users = []
        drop(f);

        let _config = load_config(Some(&root)).unwrap();
        let contents = std::fs::read_to_string(&root).unwrap();

        // The migrated file should not have a `users = []` line that would
        // conflict with a user later adding `[[users]]` sections.
        assert!(
            !contents.contains("users = []"),
            "Migrated config must not contain `users = []`; got:\n{contents}"
        );
    }

    #[test]
    fn test_users_array_key_conflicts_with_array_of_tables() {
        // Verify that if a config has BOTH `users = []` and `[[users]]`,
        // the TOML parser rejects it (duplicate key). The kernel must surface
        // the error rather than silently substituting defaults.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "log_level = \"warn\"").unwrap();
        writeln!(f, "users = []").unwrap(); // would conflict with [[users]]
        writeln!(f, "[[users]]").unwrap();
        writeln!(f, "name = \"Alice\"").unwrap();
        writeln!(f, "role = \"owner\"").unwrap();
        drop(f);

        // TOML parse error (duplicate key) → Err, not silent defaults.
        let result = load_config(Some(&root));
        assert!(
            result.is_err(),
            "duplicate TOML key must surface as Err, not silent defaults"
        );
        let msg = result.unwrap_err();
        assert!(
            msg.contains("invalid TOML") || msg.contains("duplicate"),
            "error must be actionable; got: {msg}"
        );
    }

    /// Regression for #3460: nested typos like `[memory] decay_ratee` were
    /// previously silently swallowed by `#[serde(default)]` because only
    /// top-level field names were checked. The loader must now warn about
    /// typos in known sections (warn-only by default, reject in strict mode).
    #[test]
    fn test_nested_typo_detected_in_warn_mode() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "log_level = \"debug\"").unwrap();
        writeln!(f, "[memory]").unwrap();
        writeln!(f, "decay_ratee = 0.42").unwrap(); // typo: extra 'e'
        writeln!(f, "[queue.concurrency]").unwrap();
        writeln!(f, "trigger_laneeee = 99").unwrap(); // typo
        drop(f);

        // Tolerant: still loads with defaults for the typo'd fields.
        let config = load_config(Some(&root)).unwrap();
        assert_eq!(config.log_level, "debug");
        // Defaults preserved (the typos didn't take effect).
        assert!(
            (config.memory.decay_rate
                - librefang_types::config::KernelConfig::default()
                    .memory
                    .decay_rate)
                .abs()
                < f64::EPSILON
        );
    }

    #[test]
    fn test_nested_typo_rejected_in_strict_mode() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "strict_config = true").unwrap();
        writeln!(f, "log_level = \"warn\"").unwrap();
        writeln!(f, "[budget]").unwrap();
        writeln!(f, "max_hourly_usdd = 5.0").unwrap(); // typo
        drop(f);

        let config = load_config(Some(&root)).unwrap();
        // Falls back to defaults because strict mode rejected the typo.
        assert_eq!(config.log_level, "info");
        assert!(config.strict_config);
    }

    #[test]
    fn test_load_config_users_missing_name_fails_closed() {
        // A [[users]] block without a required `name` field fails deserialization.
        // The kernel must surface the error rather than silently substituting
        // defaults (which would discard the operator's full user list).
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "log_level = \"warn\"").unwrap();
        writeln!(f, "[[users]]").unwrap();
        writeln!(f, "role = \"owner\"").unwrap(); // no name — invalid
        drop(f);

        let result = load_config(Some(&root));
        assert!(
            result.is_err(),
            "deserialization failure must surface as Err, not silent defaults"
        );
        let msg = result.unwrap_err();
        assert!(
            msg.contains("deserialize") || msg.contains("missing field"),
            "error must be actionable; got: {msg}"
        );
    }

    // -----------------------------------------------------------------------
    // try_load_config — strict variant used by reload (#4664)
    // -----------------------------------------------------------------------

    #[test]
    fn test_try_load_config_happy_path_returns_kernel_config() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "log_level = \"warn\"").unwrap();
        drop(f);

        let cfg = try_load_config(&root).expect("strict load must succeed on a clean file");
        assert_eq!(cfg.log_level, "warn");
    }

    #[test]
    fn test_try_load_config_missing_file_is_error_not_default() {
        // Tolerant `load_config` returns defaults for a missing file because
        // initial boot wants to come up. The strict variant must `Err` so the
        // reload path does not silently rewrite the live state.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("nonexistent.toml");

        let err = try_load_config(&root).expect_err("missing file must surface as Err");
        assert!(
            err.contains("not found"),
            "missing-file error must be operator-actionable; got: {err}"
        );
    }

    #[test]
    fn test_try_load_config_invalid_toml_is_error() {
        // The exact failure shape from #4664: duplicate `[web.searxng]` key.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "[web.searxng]").unwrap();
        writeln!(f, "url = \"http://first\"").unwrap();
        writeln!(f, "[web.searxng]").unwrap();
        writeln!(f, "url = \"http://second\"").unwrap();
        drop(f);

        let err = try_load_config(&root).expect_err("duplicate key must surface as Err");
        assert!(
            err.contains("invalid TOML"),
            "TOML syntax error must be tagged so the reload caller can wrap it; got: {err}"
        );
    }

    #[test]
    fn test_try_load_config_broken_include_chain_is_error() {
        // Root is well-formed but points at an unparseable include — tolerant
        // `load_config` warns and silently proceeds with root-only, which on a
        // reload path would still effectively zero the operator's settings
        // that lived in the include. Strict variant must refuse.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");
        let bad_include = dir.path().join("bad.toml");

        let mut f = std::fs::File::create(&bad_include).unwrap();
        // Same duplicate-key shape as the bug report, just inside the include.
        writeln!(f, "[memory]").unwrap();
        writeln!(f, "decay_rate = 0.1").unwrap();
        writeln!(f, "[memory]").unwrap();
        writeln!(f, "decay_rate = 0.2").unwrap();
        drop(f);

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "include = [\"bad.toml\"]").unwrap();
        writeln!(f, "log_level = \"debug\"").unwrap();
        drop(f);

        let err = try_load_config(&root).expect_err("broken include must surface as Err");
        assert!(
            err.contains("include"),
            "include-failure error must be tagged so the operator knows where to look; got: {err}"
        );
    }

    #[test]
    fn test_try_load_config_deserialize_shape_mismatch_is_error() {
        // TOML parses cleanly but a field has the wrong shape — `default_model`
        // is a struct, not a scalar. Both `load_config` and `try_load_config`
        // now return `Err` on a hard deserialize failure (see #5186).
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "default_model = \"not-a-table\"").unwrap();
        drop(f);

        let err = try_load_config(&root).expect_err("wrong-shape field must surface as Err");
        assert!(
            err.contains("deserialize"),
            "deserialize-failure error must be tagged; got: {err}"
        );
    }

    #[test]
    fn test_try_load_config_strict_mode_rejects_unknown_field() {
        // Mirrors the tolerant `load_config` behaviour test, but the strict
        // variant returns `Err` instead of "defaults with strict_config=true"
        // so the reload path can refuse to swap rather than silently zeroing.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "strict_config = true").unwrap();
        writeln!(f, "log_level = \"warn\"").unwrap();
        writeln!(f, "bogus_field = \"oops\"").unwrap();
        drop(f);

        let err =
            try_load_config(&root).expect_err("strict_config + unknown field must surface as Err");
        assert!(
            err.contains("unknown field"),
            "unknown-field error must be tagged; got: {err}"
        );
    }

    #[test]
    fn test_try_load_config_tolerant_mode_warns_on_unknown_field_but_still_loads() {
        // strict_config defaults to false. Unknown fields warn but don't block,
        // matching `load_config`'s tolerant semantics — otherwise a typo
        // anywhere would brick the reload.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "log_level = \"warn\"").unwrap();
        writeln!(f, "bogus_field = \"oops\"").unwrap();
        drop(f);

        let cfg =
            try_load_config(&root).expect("tolerant unknown fields must not fail strict load");
        assert_eq!(cfg.log_level, "warn");
    }

    // -----------------------------------------------------------------------
    // #5186 — hard deserialize failures fail closed; unknown fields stay tolerant
    // -----------------------------------------------------------------------

    /// A config with a wrong-type field (not a structurally unknown field) must
    /// cause `load_config` to return `Err` containing the field path, not
    /// silently substitute `KernelConfig::default()`.
    #[test]
    fn test_load_config_hard_deserialize_failure_returns_err_with_field_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        // `default_model` expects a table, not a plain string.
        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "log_level = \"warn\"").unwrap();
        writeln!(f, "default_model = \"not-a-table\"").unwrap();
        drop(f);

        let result = load_config(Some(&root));
        assert!(
            result.is_err(),
            "wrong-type field must surface as Err, not silent defaults"
        );
        let msg = result.unwrap_err();
        assert!(
            msg.contains("deserialize")
                || msg.contains("default_model")
                || msg.contains("invalid type"),
            "error must name the offending field or describe the mismatch; got: {msg}"
        );
    }

    /// A config with TOML syntax that the parser outright rejects must cause
    /// `load_config` to return `Err`, not silently substitute defaults.
    #[test]
    fn test_load_config_invalid_toml_returns_err() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        // Duplicate section key — valid TOML rejects this at parse time.
        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "[memory]").unwrap();
        writeln!(f, "decay_rate = 0.1").unwrap();
        writeln!(f, "[memory]").unwrap();
        writeln!(f, "decay_rate = 0.2").unwrap();
        drop(f);

        let result = load_config(Some(&root));
        assert!(
            result.is_err(),
            "invalid TOML must surface as Err, not silent defaults"
        );
        let msg = result.unwrap_err();
        assert!(
            msg.contains("invalid TOML") || msg.contains("duplicate"),
            "error must be actionable; got: {msg}"
        );
    }

    /// Unknown/extra fields in the config must NOT cause `load_config` to fail —
    /// the forward-compat (unknown-field tolerance) path introduced in #5130
    /// must remain intact even after the #5186 fail-closed change.
    #[test]
    fn test_load_config_unknown_field_forward_compat_still_loads() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        // A field that did not exist in the schema — simulates a stale key
        // from a prior release that has since been removed or renamed.
        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "log_level = \"debug\"").unwrap();
        writeln!(f, "api_key = \"secret\"").unwrap();
        writeln!(f, "output_format_legacy = \"markdown\"").unwrap(); // stale field
        drop(f);

        // Must succeed: unknown fields are tolerated and warned, not fatal.
        let config = load_config(Some(&root))
            .expect("unknown-field forward-compat path must still load successfully");
        assert_eq!(config.log_level, "debug");
        assert_eq!(config.api_key, "secret");
    }

    /// Regression for #5476: a `[agents.<name>.proactive_memory]` block
    /// in `config.toml` is silently ignored by the kernel (the actual
    /// surface is `{workspace}/agent.toml`'s top-level
    /// `[proactive_memory]`). Operators following the original #4870
    /// issue body's published syntax used to get a silent no-op with
    /// no log entry pointing at the correct location. Load must now
    /// (a) still succeed and (b) emit a targeted WARN naming the
    /// agent and the correct path.
    #[test]
    fn test_load_config_misplaced_per_agent_proactive_memory_warns() {
        use std::io;
        use std::sync::{Arc, Mutex};
        use tracing_subscriber::fmt::MakeWriter;
        use tracing_subscriber::layer::SubscriberExt;

        #[derive(Clone)]
        struct VecWriter(Arc<Mutex<Vec<u8>>>);
        impl io::Write for VecWriter {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        impl<'a> MakeWriter<'a> for VecWriter {
            type Writer = VecWriter;
            fn make_writer(&'a self) -> Self::Writer {
                self.clone()
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "log_level = \"debug\"").unwrap();
        writeln!(f, "[proactive_memory]").unwrap();
        writeln!(f, "enabled = true").unwrap();
        writeln!(f, "auto_memorize = false").unwrap();
        writeln!(f).unwrap();
        // The misplaced override — published verbatim in the #4870
        // issue description but silently no-ops in beta.12 (#5476).
        writeln!(f, "[agents.lifeos-daily-brief.proactive_memory]").unwrap();
        writeln!(f, "auto_memorize = true").unwrap();
        drop(f);

        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let writer = VecWriter(buf.clone());
        let layer = tracing_subscriber::fmt::layer()
            .with_writer(writer)
            .with_ansi(false)
            .with_target(false);
        let subscriber = tracing_subscriber::registry().with(layer);
        let _g = tracing::subscriber::set_default(subscriber);

        // Must still load successfully — the misplaced block is a soft
        // warning, not a hard error.
        let config = load_config(Some(&root)).expect("misplaced block must not fail load");
        assert_eq!(config.log_level, "debug");

        let captured = String::from_utf8(buf.lock().unwrap().clone()).expect("utf8");
        assert!(
            captured.contains("lifeos-daily-brief"),
            "warning must name the offending agent; captured: {captured:?}"
        );
        assert!(
            captured.contains("proactive_memory"),
            "warning must name the offending override key; captured: {captured:?}"
        );
        assert!(
            captured.contains("agent.toml"),
            "warning must point at the correct surface (agent.toml); captured: {captured:?}"
        );
        assert!(
            captured.contains("#5476"),
            "warning must reference the tracking issue; captured: {captured:?}"
        );
    }
}
