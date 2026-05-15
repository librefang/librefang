//! Context engine plugin management — install, remove, list, scaffold.
//!
//! Plugins live at `~/.librefang/plugins/<name>/` and contain:
//! - `plugin.toml`     — manifest (name, version, hooks, requirements)
//! - `hooks/`          — Python hook scripts (ingest.py, after_turn.py, etc.)
//! - `requirements.txt` — optional Python dependencies
//!
//! # Install sources
//! - **GitHub registry**: configurable `owner/repo` (default: `librefang/librefang-registry`)
//! - **Local path**: copy from a local directory
//! - **Git URL**: clone a git repo into the plugins directory

use librefang_types::config::{PluginManifest, PluginSystemRequirement};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

mod install;
mod registry;
mod scaffold;

pub use self::install::{
    install_plugin, install_requirements, list_registry_plugins, remove_plugin, RegistryPluginEntry,
};
pub use self::scaffold::scaffold_plugin;

use self::registry::fetch_verified_index;

/// Returns the list of hook script paths declared in `[hooks]` that have no
/// matching entry in `[integrity]`. An empty result means every declared hook
/// is covered (or there are no hooks at all).
///
/// This is the source of truth used by both:
/// * `install_from_registry` — hard error when missing entries are found, since
///   registry-distributed plugins must be tamper-evident.
/// * `lint_plugin` — warning surfaced to plugin authors so they catch the issue
///   locally before submitting to the registry (issue #4036).
pub fn manifest_missing_integrity_hooks(manifest: &PluginManifest) -> Vec<String> {
    [
        manifest.hooks.ingest.as_deref(),
        manifest.hooks.after_turn.as_deref(),
        manifest.hooks.bootstrap.as_deref(),
        manifest.hooks.assemble.as_deref(),
        manifest.hooks.compact.as_deref(),
        manifest.hooks.prepare_subagent.as_deref(),
        manifest.hooks.merge_subagent.as_deref(),
    ]
    .into_iter()
    .flatten()
    .filter(|hook| !manifest.integrity.contains_key(*hook))
    .map(|s| s.to_string())
    .collect()
}

/// Validate that a plugin name is a safe directory component (no path traversal).
pub fn validate_plugin_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Plugin name cannot be empty".to_string());
    }
    if name.len() > 128 {
        return Err(format!(
            "Invalid plugin name: exceeds maximum length of 128 characters (got {})",
            name.len()
        ));
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") || name == "." {
        return Err(format!(
            "Invalid plugin name '{name}': must be a simple identifier (no /, \\, or ..)"
        ));
    }
    // Only allow alphanumeric, hyphens, underscores
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!(
            "Invalid plugin name '{name}': only alphanumeric, hyphens, and underscores allowed"
        ));
    }
    Ok(())
}

/// Default plugin directory: `~/.librefang/plugins/`.
pub fn plugins_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| {
            warn!("HOME directory not set; using temporary directory for plugins");
            #[cfg(unix)]
            let fallback = PathBuf::from("/tmp/librefang");
            #[cfg(windows)]
            let fallback =
                PathBuf::from(std::env::var("TEMP").unwrap_or_else(|_| r"C:\Temp".to_string()))
                    .join("librefang");
            #[cfg(not(any(unix, windows)))]
            let fallback = PathBuf::from(".librefang");
            fallback
        })
        .join(".librefang")
        .join("plugins")
}

/// Ensure the plugins directory exists.
pub fn ensure_plugins_dir() -> std::io::Result<PathBuf> {
    let dir = plugins_dir();
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Describes a single backward-incompatibility between an old and new plugin manifest.
#[derive(Debug, Clone)]
pub struct ManifestCompatWarning {
    pub kind: ManifestCompatKind,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ManifestCompatKind {
    /// A hook that was present in the old manifest is absent in the new one.
    HookRemoved,
    /// The runtime changed (e.g. Python → Node) — may break existing state files.
    RuntimeChanged,
    /// The major version decreased (downgrade).
    MajorVersionDowngrade,
    /// The plugin name changed — unusual and likely a mistake.
    NameChanged,
}

/// Information about an installed plugin, returned by list/get operations.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PluginInfo {
    pub manifest: PluginManifest,
    /// Absolute path to the plugin directory.
    pub path: PathBuf,
    /// Whether all declared hook scripts exist on disk.
    pub hooks_valid: bool,
    /// Size of the plugin directory in bytes.
    pub size_bytes: u64,
    /// Whether the plugin is enabled (not disabled via marker file).
    pub enabled: bool,
    /// Declared capabilities from the `needs` array in plugin.toml.
    pub needs: Vec<String>,
}

/// Result of a plugin lint check.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PluginLintReport {
    pub plugin: String,
    pub ok: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

/// Source for plugin installation.
#[derive(Debug, Clone)]
pub enum PluginSource {
    /// Install from a GitHub registry (`owner/repo`).
    /// `None` defaults to `librefang/librefang-registry`.
    Registry {
        name: String,
        github_repo: Option<String>,
    },
    /// Install from a local directory (copy).
    Local { path: PathBuf },
    /// Install from a git URL (clone).
    Git { url: String, branch: Option<String> },
}

/// Load and validate a plugin manifest from a directory.
///
/// Also enforces `librefang_min_version` compatibility: returns an error when
/// the running daemon is older than what the plugin requires.
pub fn load_plugin_manifest(plugin_dir: &Path) -> Result<PluginManifest, String> {
    let manifest_path = plugin_dir.join("plugin.toml");
    if !manifest_path.exists() {
        return Err(format!(
            "plugin.toml not found at {}",
            manifest_path.display()
        ));
    }

    let content = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("Failed to read {}: {e}", manifest_path.display()))?;

    let manifest: PluginManifest =
        toml::from_str(&content).map_err(|e| format!("Invalid plugin.toml: {e}"))?;

    // Enforce minimum version requirement declared by the plugin.
    if let Some(ref min_ver) = manifest.librefang_min_version {
        const DAEMON_VERSION: &str = env!("CARGO_PKG_VERSION");
        if !version_satisfies(DAEMON_VERSION, min_ver) {
            return Err(format!(
                "Plugin '{}' requires LibreFang >= {min_ver} but running {DAEMON_VERSION}. \
                 Upgrade the daemon or use an older plugin version.",
                manifest.name
            ));
        }
    }

    // Verify integrity hashes for declared hook scripts.
    if !manifest.integrity.is_empty() {
        for (rel_path, expected_hex) in &manifest.integrity {
            let abs_path = plugin_dir.join(rel_path);
            match std::fs::read(&abs_path) {
                Ok(bytes) => {
                    let actual_hex = sha256_hex(&bytes);
                    if actual_hex != *expected_hex {
                        return Err(format!(
                            "Plugin '{}': integrity check failed for '{}' \
                             (expected {expected_hex}, got {actual_hex}). \
                             The hook file may have been tampered with.",
                            manifest.name, rel_path
                        ));
                    }
                }
                Err(e) => {
                    return Err(format!(
                        "Plugin '{}': cannot read '{}' for integrity check: {e}",
                        manifest.name, rel_path
                    ));
                }
            }
        }
        debug!(plugin = manifest.name, "All integrity hashes verified");
    }

    // Validate env_schema: warn for required vars that are not set in the daemon env.
    for (key, desc) in &manifest.hooks.env_schema {
        if let Some(required_key) = key.strip_prefix('!') {
            // Check if it's configured in the plugin's [env] section or daemon environment
            let in_plugin_env = manifest.env.contains_key(required_key);
            let in_daemon_env = std::env::var(required_key).is_ok();
            if !in_plugin_env && !in_daemon_env {
                warn!(
                    plugin = manifest.name,
                    var = required_key,
                    description = desc.as_str(),
                    "Required env var is not set (declared in [hooks.env_schema])"
                );
            }
        }
    }

    // Check plugin dependencies are satisfied.
    if !manifest.plugin_depends.is_empty() {
        let plugins_root = plugin_dir.parent().unwrap_or(plugin_dir);
        for dep in &manifest.plugin_depends {
            let dep_dir = plugins_root.join(dep);
            if !dep_dir.join("plugin.toml").exists() {
                return Err(format!(
                    "Plugin '{}' requires plugin '{dep}' but it is not installed. \
                     Install it first.",
                    manifest.name
                ));
            }
        }
    }

    Ok(manifest)
}

/// Returns `true` when `running` >= `required` for the leading semver portion.
///
/// Strips any `-` pre-release suffix before comparing, then does a
/// lexicographic comparison on dot-separated numeric segments (left-padded so
/// component widths align). This is intentionally simple: LibreFang uses
/// `YYYY.M.D-betaN` versioning, so a real semver library is overkill.
fn version_satisfies(running: &str, required: &str) -> bool {
    fn semver_parts(v: &str) -> Vec<u64> {
        v.split('-')
            .next()
            .unwrap_or(v)
            .split('.')
            .filter_map(|p| p.parse().ok())
            .collect()
    }
    let run = semver_parts(running);
    let req = semver_parts(required);
    let len = run.len().max(req.len());
    for i in 0..len {
        let r = run.get(i).copied().unwrap_or(0);
        let q = req.get(i).copied().unwrap_or(0);
        match r.cmp(&q) {
            std::cmp::Ordering::Greater => return true,
            std::cmp::Ordering::Less => return false,
            std::cmp::Ordering::Equal => {}
        }
    }
    true // equal
}

/// Get detailed info about a single installed plugin.
pub fn get_plugin_info(plugin_name: &str) -> Result<PluginInfo, String> {
    validate_plugin_name(plugin_name)?;
    let plugin_dir = plugins_dir().join(plugin_name);
    if !plugin_dir.exists() {
        return Err(format!("Plugin '{plugin_name}' is not installed"));
    }

    let manifest = load_plugin_manifest(&plugin_dir)?;

    // Validate hook scripts exist
    let hooks_valid = check_hooks_exist(&plugin_dir, &manifest);

    // Calculate directory size
    let size_bytes = dir_size(&plugin_dir);

    // Enabled unless a .disabled marker file exists
    let enabled = !plugin_dir.join(".disabled").exists();

    // Extract declared capabilities from raw TOML needs array
    let needs = {
        let manifest_path = plugin_dir.join("plugin.toml");
        std::fs::read_to_string(&manifest_path)
            .ok()
            .map(|raw| extract_needs(&raw))
            .unwrap_or_default()
    };

    Ok(PluginInfo {
        manifest,
        path: plugin_dir,
        hooks_valid,
        size_bytes,
        enabled,
        needs,
    })
}

/// Re-read a plugin's `plugin.toml` from disk and validate it.
///
/// This is semantically equivalent to [`get_plugin_info`] but signals
/// intent: callers use this when they want to pick up manifest changes
/// (e.g. after editing `plugin.toml`).
///
/// **Hot-reload semantics:**
/// - Hook *script* changes take effect immediately — scripts are re-executed
///   fresh on each call, so edits to `.py` / `.js` / binary hooks are live.
/// - Manifest changes (adding or removing hook declarations) are reflected in
///   the returned [`PluginInfo`], but the running agent's context engine is
///   not restarted. A full agent restart is required for new hooks to become
///   active.
pub fn reload_plugin(name: &str) -> Result<PluginInfo, String> {
    validate_plugin_name(name)?;
    get_plugin_info(name)
}

/// Doctor entry for a single installed plugin.
///
/// Tells the user whether the plugin is structurally valid (hook scripts
/// exist) *and* whether the runtime it asks for is usable on this host.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PluginDoctorEntry {
    pub name: String,
    /// Canonical runtime tag (`python`, `v`, ...). Falls back to the
    /// dispatcher's default (`python`) for plugins that don't declare one.
    pub runtime: String,
    /// `true` when the declared runtime's launcher resolved on PATH
    /// (or for `native`, always `true`).
    pub runtime_available: bool,
    /// `true` when every hook script declared in `plugin.toml` exists.
    pub hooks_valid: bool,
    /// Install hint surfaced when `runtime_available` is `false`.
    pub install_hint: String,
}

/// Aggregate doctor report: per-runtime availability + per-plugin readiness.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DoctorReport {
    /// Availability of every supported runtime, in stable order.
    pub runtimes: Vec<crate::plugin_runtime::RuntimeStatus>,
    /// One entry per installed plugin.
    pub plugins: Vec<PluginDoctorEntry>,
}

/// Probe the environment and return a diagnostic report.
///
/// Spawns one subprocess per runtime (`{launcher} --version`) — caller
/// should wrap in `tokio::task::spawn_blocking` if used from async.
pub fn run_doctor() -> DoctorReport {
    use crate::plugin_runtime::{check_runtime_status, PluginRuntime};

    let runtimes: Vec<_> = PluginRuntime::all()
        .iter()
        .map(|r| check_runtime_status(r.clone()))
        .collect();

    // Index by runtime tag so per-plugin entries can look up availability
    // without re-probing subprocesses.
    let availability: std::collections::HashMap<&str, (bool, &str)> = runtimes
        .iter()
        .map(|s| (s.runtime.as_str(), (s.available, s.install_hint.as_str())))
        .collect();

    let plugins = list_plugins()
        .into_iter()
        .map(|info| {
            let runtime_kind = PluginRuntime::from_tag(info.manifest.hooks.runtime.as_deref());
            let tag = runtime_kind.label();
            let (available, hint) = availability
                .get(tag.as_ref())
                .copied()
                .unwrap_or((false, ""));
            PluginDoctorEntry {
                name: info.manifest.name,
                runtime: tag.to_string(),
                runtime_available: available,
                hooks_valid: info.hooks_valid,
                install_hint: hint.to_string(),
            }
        })
        .collect();

    DoctorReport { runtimes, plugins }
}

/// List all installed plugins.
pub fn list_plugins() -> Vec<PluginInfo> {
    let dir = plugins_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };

    entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            if !entry.file_type().ok()?.is_dir() {
                return None;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            match get_plugin_info(&name) {
                Ok(info) => Some(info),
                Err(e) => {
                    warn!(plugin = name, error = %e, "Skipping invalid plugin");
                    None
                }
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Compute a hex-encoded SHA-256 digest of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    // NOTE: Rust's `DefaultHasher` is NOT cryptographic. We use a simple
    // hand-rolled SHA-256 here so we don't pull in a new crate. If the project
    // adds `sha2` in future, swap this implementation out.
    //
    // This is a pure-Rust SHA-256 implementation (RFC 6234).
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    // Pre-processing: padding
    let bit_len = (bytes.len() as u64).wrapping_mul(8);
    let mut msg = bytes.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0x00);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    // Process each 512-bit (64-byte) block
    for block in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] =
            [h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]];
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    format!(
        "{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}",
        h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]
    )
}

/// Compute the SHA-256 hex digest of a byte slice (delegates to [`sha256_hex`]).
fn sha256_hex_of_bytes(data: &[u8]) -> String {
    sha256_hex(data)
}

/// Verify downloaded plugin bytes against an expected SHA-256 checksum.
///
/// Returns `Ok(())` on match, `Err(message)` on mismatch or parse failure.
fn verify_checksum(data: &[u8], expected: &str) -> Result<(), String> {
    let actual = sha256_hex_of_bytes(data);
    if actual.eq_ignore_ascii_case(expected.trim()) {
        Ok(())
    } else {
        Err(format!(
            "Plugin checksum mismatch!\n  Expected: {expected}\n  Actual:   {actual}\n\
             The downloaded file may be corrupted or tampered with. Aborting install."
        ))
    }
}

/// Fetch the SHA-256 checksum for a plugin release asset from the registry.
///
/// Looks for a `checksums.txt` (or `{plugin_name}.sha256`) file alongside
/// the plugin archive. Returns `None` if no checksum file is available
/// (older registry entries without checksums are allowed through with a warning).
async fn fetch_checksum(
    client: &reqwest::Client,
    archive_url: &str,
    plugin_name: &str,
) -> Option<String> {
    // Try {archive_url}.sha256 first, then checksums.txt in the same directory.
    let candidates = [format!("{archive_url}.sha256"), {
        let base = archive_url
            .rsplit_once('/')
            .map(|(b, _)| b)
            .unwrap_or(archive_url);
        format!("{base}/checksums.txt")
    }];

    for url in &candidates {
        if let Ok(resp) = client.get(url).send().await {
            if resp.status().is_success() {
                if let Ok(text) = resp.text().await {
                    // checksums.txt format: "<sha256>  <filename>" per line
                    for line in text.lines() {
                        let parts: Vec<&str> = line.splitn(2, ' ').collect();
                        if !parts.is_empty() {
                            let hash = parts[0].trim();
                            if hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()) {
                                // If it's a checksums.txt, check the filename matches
                                if parts.len() == 1 || parts[1].trim().contains(plugin_name) {
                                    return Some(hash.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// Enable a previously disabled plugin by removing the `.disabled` marker file.
///
/// Returns an error if the plugin does not exist or was not disabled.
pub fn enable_plugin(name: &str) -> Result<(), String> {
    validate_plugin_name(name)?;
    let plugin_dir = plugins_dir().join(name);
    if !plugin_dir.exists() {
        return Err(format!("Plugin '{name}' is not installed"));
    }
    let marker = plugin_dir.join(".disabled");
    if !marker.exists() {
        return Err(format!("Plugin '{name}' is already enabled"));
    }
    std::fs::remove_file(&marker).map_err(|e| format!("Failed to enable plugin '{name}': {e}"))?;
    info!(plugin = name, "Plugin enabled");
    Ok(())
}

/// Disable a plugin by creating a `.disabled` marker file.
///
/// The running context engine will not pick up the change until it is
/// restarted; this marks the intent so the next start skips the plugin.
pub fn disable_plugin(name: &str) -> Result<(), String> {
    validate_plugin_name(name)?;
    let plugin_dir = plugins_dir().join(name);
    if !plugin_dir.exists() {
        return Err(format!("Plugin '{name}' is not installed"));
    }
    let marker = plugin_dir.join(".disabled");
    if marker.exists() {
        return Err(format!("Plugin '{name}' is already disabled"));
    }
    std::fs::write(&marker, "").map_err(|e| format!("Failed to disable plugin '{name}': {e}"))?;
    info!(plugin = name, "Plugin disabled");
    Ok(())
}

/// Compare two plugin manifests and return a list of backward-incompatibility warnings.
///
/// An empty return value means the upgrade is safe.
fn check_manifest_compat(old: &PluginManifest, new: &PluginManifest) -> Vec<ManifestCompatWarning> {
    let mut warnings = Vec::new();

    // Name change
    if old.name != new.name {
        warnings.push(ManifestCompatWarning {
            kind: ManifestCompatKind::NameChanged,
            message: format!("plugin name changed from '{}' to '{}'", old.name, new.name),
        });
    }

    // Runtime change
    if old.hooks.runtime != new.hooks.runtime {
        warnings.push(ManifestCompatWarning {
            kind: ManifestCompatKind::RuntimeChanged,
            message: format!(
                "hook runtime changed from {:?} to {:?}",
                old.hooks.runtime, new.hooks.runtime
            ),
        });
    }

    // Removed hooks — check each of the 7 known hook script fields
    let hook_pairs = [
        (
            "bootstrap",
            old.hooks.bootstrap.as_ref(),
            new.hooks.bootstrap.as_ref(),
        ),
        (
            "ingest",
            old.hooks.ingest.as_ref(),
            new.hooks.ingest.as_ref(),
        ),
        (
            "assemble",
            old.hooks.assemble.as_ref(),
            new.hooks.assemble.as_ref(),
        ),
        (
            "compact",
            old.hooks.compact.as_ref(),
            new.hooks.compact.as_ref(),
        ),
        (
            "after_turn",
            old.hooks.after_turn.as_ref(),
            new.hooks.after_turn.as_ref(),
        ),
        (
            "prepare_subagent",
            old.hooks.prepare_subagent.as_ref(),
            new.hooks.prepare_subagent.as_ref(),
        ),
        (
            "merge_subagent",
            old.hooks.merge_subagent.as_ref(),
            new.hooks.merge_subagent.as_ref(),
        ),
    ];
    for (hook_name, old_script, new_script) in &hook_pairs {
        if old_script.is_some() && new_script.is_none() {
            warnings.push(ManifestCompatWarning {
                kind: ManifestCompatKind::HookRemoved,
                message: format!(
                    "hook '{}' was present in old manifest but removed in new",
                    hook_name
                ),
            });
        }
    }

    // Major version downgrade — parse "major.minor.patch" tuples
    if let (Some(old_ver), Some(new_ver)) = (
        parse_semver_triple(&old.version),
        parse_semver_triple(&new.version),
    ) {
        if new_ver.0 < old_ver.0 {
            warnings.push(ManifestCompatWarning {
                kind: ManifestCompatKind::MajorVersionDowngrade,
                message: format!(
                    "major version downgrade from {} to {}",
                    old.version, new.version
                ),
            });
        }
    }

    warnings
}

/// Parse "major.minor.patch" into a (u32, u32, u32) tuple.
/// Returns None if the string doesn't match the pattern.
fn parse_semver_triple(s: &str) -> Option<(u32, u32, u32)> {
    let parts: Vec<&str> = s.split('.').collect();
    let major = parts.first()?.parse().ok()?;
    let minor = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(0);
    let patch = parts.get(2).and_then(|p| p.parse().ok()).unwrap_or(0);
    Some((major, minor, patch))
}

/// Upgrade a plugin in-place: remove the old version, reinstall from source.
///
/// The `.disabled` state is preserved across the upgrade.
pub async fn upgrade_plugin(name: &str, source: &PluginSource) -> Result<PluginInfo, String> {
    validate_plugin_name(name)?;
    let plugin_dir = plugins_dir().join(name);
    if !plugin_dir.exists() {
        return Err(format!(
            "Plugin '{name}' is not installed. Use install instead."
        ));
    }

    // Capture old manifest before removing so we can compare with the new one.
    let old_manifest = load_plugin_manifest(&plugin_dir).ok();

    // Preserve the enabled/disabled state
    let was_disabled = plugin_dir.join(".disabled").exists();

    // Remove old version
    tokio::fs::remove_dir_all(&plugin_dir)
        .await
        .map_err(|e| format!("Failed to remove old version of '{name}': {e}"))?;

    // Reinstall
    let info = install_plugin(source).await?;

    // Check for breaking changes between old and new manifest.
    if let Some(ref old) = old_manifest {
        let compat_warnings = check_manifest_compat(old, &info.manifest);
        if !compat_warnings.is_empty() {
            for w in &compat_warnings {
                warn!(plugin = %name, kind = ?w.kind, "{}", w.message);
            }
        }
    }

    // Restore disabled state if it was set
    if was_disabled {
        let marker = plugins_dir().join(name).join(".disabled");
        let _ = tokio::fs::write(&marker, "").await;
    }

    info!(plugin = name, "Plugin upgraded");
    Ok(info)
}

/// Compute SHA-256 integrity hashes for all declared hook scripts and write
/// them into `plugin.toml` under the `[integrity]` section.
///
/// Returns a map of `relative_path → sha256_hex` for every hook that was hashed.
/// After this call the plugin can be loaded with integrity verification enabled.
pub fn sign_plugin(name: &str) -> Result<std::collections::HashMap<String, String>, String> {
    validate_plugin_name(name)?;
    let plugin_dir = plugins_dir().join(name);
    if !plugin_dir.exists() {
        return Err(format!("Plugin '{name}' is not installed"));
    }

    let mut manifest = load_plugin_manifest_raw(&plugin_dir)?;

    // Collect all declared hook script paths
    let hooks = &manifest.hooks;
    let mut hook_paths: Vec<String> = Vec::new();
    for p in [
        hooks.ingest.as_deref(),
        hooks.after_turn.as_deref(),
        hooks.assemble.as_deref(),
        hooks.compact.as_deref(),
        hooks.bootstrap.as_deref(),
        hooks.prepare_subagent.as_deref(),
        hooks.merge_subagent.as_deref(),
    ]
    .iter()
    .flatten()
    {
        hook_paths.push(p.to_string());
    }

    if hook_paths.is_empty() {
        return Err(format!("Plugin '{name}' has no hook scripts declared"));
    }

    let mut hashes = std::collections::HashMap::new();
    for rel_path in &hook_paths {
        let abs_path = plugin_dir.join(rel_path);
        let bytes = std::fs::read(&abs_path)
            .map_err(|e| format!("Cannot read '{}' for signing: {e}", abs_path.display()))?;
        hashes.insert(rel_path.clone(), sha256_hex(&bytes));
    }

    // Update manifest integrity map
    manifest.integrity = hashes.clone();

    // Rewrite plugin.toml with updated integrity section.
    // We do a targeted TOML patch: read the original, remove any existing
    // [integrity] table, then append a fresh one.
    let manifest_path = plugin_dir.join("plugin.toml");
    let original = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("Cannot read plugin.toml: {e}"))?;

    // Strip existing [integrity] block (from "[integrity]" to next bare "[" section)
    let stripped = strip_toml_section(&original, "integrity");

    // Append new [integrity] block.  Iterate via sorted keys so the on-disk
    // order is deterministic across processes and OS file iteration quirks.
    let mut new_content = stripped.trim_end().to_string();
    new_content.push_str("\n\n[integrity]\n");
    let mut entries: Vec<(&String, &String)> = hashes.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    for (path, hash) in entries {
        new_content.push_str(&format!("\"{}\" = \"{}\"\n", path, hash));
    }

    std::fs::write(&manifest_path, &new_content)
        .map_err(|e| format!("Failed to write plugin.toml: {e}"))?;

    info!(
        plugin = name,
        hooks = hook_paths.len(),
        "Plugin signed — integrity hashes written"
    );
    Ok(hashes)
}

/// Collect every hook script path declared in `[hooks]` of the given manifest.
///
/// Returns a flat `Vec` of relative paths (e.g. `"hooks/ingest.py"`) in the
/// canonical declaration order: ingest, after_turn, bootstrap, assemble,
/// compact, prepare_subagent, merge_subagent.  Hooks that aren't declared
/// produce no entry.
fn declared_hook_paths(manifest: &PluginManifest) -> Vec<String> {
    [
        manifest.hooks.ingest.as_deref(),
        manifest.hooks.after_turn.as_deref(),
        manifest.hooks.bootstrap.as_deref(),
        manifest.hooks.assemble.as_deref(),
        manifest.hooks.compact.as_deref(),
        manifest.hooks.prepare_subagent.as_deref(),
        manifest.hooks.merge_subagent.as_deref(),
    ]
    .into_iter()
    .flatten()
    .map(|s| s.to_string())
    .collect()
}

/// Validate that a plugin directory is ready to be published to a registry.
///
/// Returns `Ok(())` when every script declared under `[hooks]` has a matching
/// entry under `[integrity]`.  Returns `Err(message)` listing the offending
/// hook scripts otherwise.
///
/// This is the publish-time backstop introduced for issue #4036: the official
/// `context-decay` plugin shipped without `[integrity]` because its publish
/// pipeline never enforced the rule.  Registry CI / `pack_plugin_for_publish`
/// call this so an unsigned manifest cannot reach end users in the first
/// place — `load_plugin_manifest` already enforces it on the install side.
pub fn validate_publish_ready(plugin_dir: &Path) -> Result<(), String> {
    let manifest_path = plugin_dir.join("plugin.toml");
    if !manifest_path.exists() {
        return Err(format!(
            "plugin.toml not found at {}",
            manifest_path.display()
        ));
    }

    let raw = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("Failed to read {}: {e}", manifest_path.display()))?;
    let manifest: PluginManifest =
        toml::from_str(&raw).map_err(|e| format!("Invalid plugin.toml: {e}"))?;

    // Reuse the install-side / lint-side source of truth so all three
    // call sites (install_from_registry, lint_plugin, validate_publish_ready)
    // agree on what counts as missing.
    let mut missing = manifest_missing_integrity_hooks(&manifest);
    missing.sort();

    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Plugin '{}' is missing [integrity] hashes for hook script(s): {}. \
             Registry-published plugins must include SHA-256 checksums for every \
             hook script declared in [hooks]. Re-run the publish packer (which \
             auto-computes hashes via pack_plugin_for_publish) before uploading.",
            manifest.name,
            missing.join(", ")
        ))
    }
}

/// Auto-compute SHA-256 hashes for every hook script declared in a plugin
/// directory and write them into `plugin.toml`'s `[integrity]` section.
///
/// This is the publish-pipeline entry point that fixes issue #4036: registry
/// authors call it (via CI / `librefang-registry` automation) before uploading
/// an artifact.  It guarantees the resulting `plugin.toml` will satisfy
/// [`load_plugin_manifest`]'s integrity check at install time.
///
/// Behaviour:
/// - Reads `plugin_dir/plugin.toml`,
/// - For every hook script declared in `[hooks]`, computes the SHA-256 of the
///   on-disk file and inserts it into `[integrity]`,
/// - Rewrites `plugin.toml` with a fresh `[integrity]` block (any pre-existing
///   `[integrity]` block is replaced verbatim — stale entries are dropped),
/// - Calls [`validate_publish_ready`] so a missing hook script (declared but
///   not on disk) becomes a hard error before the artifact is shipped.
///
/// Returns the `relative_path → sha256_hex` map that was written.
///
/// # Errors
/// - `plugin.toml` missing or unparseable
/// - A declared hook script does not exist on disk (typo / packager bug)
/// - The rewritten `plugin.toml` cannot be persisted (filesystem error)
pub fn pack_plugin_for_publish(
    plugin_dir: &Path,
) -> Result<std::collections::HashMap<String, String>, String> {
    let manifest_path = plugin_dir.join("plugin.toml");
    if !manifest_path.exists() {
        return Err(format!(
            "plugin.toml not found at {}",
            manifest_path.display()
        ));
    }

    let original = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("Failed to read {}: {e}", manifest_path.display()))?;
    let manifest: PluginManifest =
        toml::from_str(&original).map_err(|e| format!("Invalid plugin.toml: {e}"))?;

    let hook_paths = declared_hook_paths(&manifest);
    if hook_paths.is_empty() {
        // Nothing to sign — but still validate so a partial / malformed
        // [integrity] block doesn't slip through.
        validate_publish_ready(plugin_dir)?;
        return Ok(std::collections::HashMap::new());
    }

    // Hash every declared hook script.  A missing file is a packaging bug
    // (the manifest references a script that isn't being shipped), so fail
    // loudly rather than emitting a hash for an empty / nonexistent file.
    let mut hashes: std::collections::HashMap<String, String> =
        std::collections::HashMap::with_capacity(hook_paths.len());
    for rel_path in &hook_paths {
        let abs_path = plugin_dir.join(rel_path);
        let bytes = std::fs::read(&abs_path).map_err(|e| {
            format!(
                "Plugin '{}': cannot read hook '{}' for SHA-256 computation: {e}. \
                 Did you forget to include it in the artifact?",
                manifest.name,
                abs_path.display()
            )
        })?;
        hashes.insert(rel_path.clone(), sha256_hex(&bytes));
    }

    // Rewrite plugin.toml: strip any existing [integrity] block, then append
    // a fresh one with deterministic key ordering so byte-identical inputs
    // produce byte-identical artifacts (important for archive checksums and
    // reproducible-build verifiers).
    let stripped = strip_toml_section(&original, "integrity");
    let mut new_content = stripped.trim_end().to_string();
    new_content.push_str("\n\n[integrity]\n");
    let mut entries: Vec<(&String, &String)> = hashes.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    for (path, hash) in entries {
        new_content.push_str(&format!("\"{}\" = \"{}\"\n", path, hash));
    }

    std::fs::write(&manifest_path, &new_content)
        .map_err(|e| format!("Failed to write {}: {e}", manifest_path.display()))?;

    // Defense in depth: re-read the rewritten file and confirm every declared
    // hook is now covered.  Catches any corner case where the writer dropped
    // an entry (e.g. shell-quote oddities in a hook path).
    validate_publish_ready(plugin_dir)?;

    info!(
        plugin = manifest.name,
        hooks = hook_paths.len(),
        path = %plugin_dir.display(),
        "Plugin packed for publish — [integrity] hashes auto-injected"
    );
    Ok(hashes)
}

/// A parsed dependency specifier: `name` with an optional version constraint.
///
/// Syntax: `"plugin_name"` or `"plugin_name>=1.2.0"` etc.
/// Supported operators: `>=`, `>`, `<=`, `<`, `=`.
#[derive(Debug, Clone)]
struct DepSpec {
    name: String,
    op: Option<VersionOp>,
    version: Option<(u32, u32, u32)>, // (major, minor, patch)
}

#[derive(Debug, Clone, PartialEq)]
enum VersionOp {
    Gte,
    Gt,
    Lte,
    Lt,
    Eq,
}

impl DepSpec {
    /// Parse a dependency specifier string.
    fn parse(s: &str) -> Self {
        // Try each operator in order (longer ones first to avoid prefix clash)
        let ops: &[(&str, VersionOp)] = &[
            (">=", VersionOp::Gte),
            (">", VersionOp::Gt),
            ("<=", VersionOp::Lte),
            ("<", VersionOp::Lt),
            ("=", VersionOp::Eq),
        ];
        for (sym, op) in ops {
            if let Some(idx) = s.find(sym) {
                let name = s[..idx].trim().to_string();
                let ver_str = s[idx + sym.len()..].trim();
                let version = Self::parse_version(ver_str);
                return Self {
                    name,
                    op: Some(op.clone()),
                    version,
                };
            }
        }
        // No operator — plain name
        Self {
            name: s.trim().to_string(),
            op: None,
            version: None,
        }
    }

    fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() < 2 {
            return None;
        }
        // Strip pre-release / build-metadata suffixes (e.g. "0-alpha", "1+build.1")
        // before parsing so that semver strings like "1.2.0-alpha" are accepted.
        let numeric_prefix = |p: &str| -> Option<u32> {
            p.split(|c: char| !c.is_ascii_digit())
                .next()
                .filter(|n| !n.is_empty())
                .and_then(|n| n.parse().ok())
        };
        let major = numeric_prefix(parts[0])?;
        let minor = numeric_prefix(parts[1])?;
        let patch = parts.get(2).and_then(|p| numeric_prefix(p)).unwrap_or(0);
        Some((major, minor, patch))
    }

    /// Check whether an installed version satisfies this constraint.
    /// `installed` is a `"major.minor.patch"` string.
    fn satisfied_by(&self, installed: &str) -> bool {
        let (op, req) = match (self.op.as_ref(), self.version) {
            (Some(op), Some(v)) => (op, v),
            _ => return true, // no constraint → always satisfied
        };
        let inst = match Self::parse_version(installed) {
            Some(v) => v,
            None => return false,
        };
        match op {
            VersionOp::Gte => inst >= req,
            VersionOp::Gt => inst > req,
            VersionOp::Lte => inst <= req,
            VersionOp::Lt => inst < req,
            VersionOp::Eq => inst == req,
        }
    }
}

/// Extract the `needs` capability array from raw plugin.toml content.
///
/// Returns only the string values from `needs = ["network", "filesystem", ...]`.
/// Non-string values and missing keys are silently ignored.
fn extract_needs(raw_toml: &str) -> Vec<String> {
    toml::from_str::<toml::Value>(raw_toml)
        .ok()
        .and_then(|v| v.get("needs").and_then(|n| n.as_array()).cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect()
}

/// Return `true` if `name` resolves to an executable on `PATH`.
///
/// Walks each directory in `PATH` and checks whether `name` (or `name.exe`
/// on Windows) exists as a file in that directory.  No shell quoting or
/// tilde-expansion is performed — the binary name should be a plain
/// filename without path separators.
fn binary_on_path(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            if dir.join(name).exists() {
                return true;
            }
            // Windows: also check with .exe extension
            #[cfg(target_os = "windows")]
            if dir.join(format!("{name}.exe")).exists() {
                return true;
            }
        }
    }
    false
}

/// Check whether each `[[requires]]` binary is available on PATH.
///
/// Returns a list of `(binary, install_hint)` pairs for each missing binary.
/// An empty list means all required binaries are present.
fn check_system_requires(requires: &[PluginSystemRequirement]) -> Vec<(String, Option<String>)> {
    requires
        .iter()
        .filter(|req| !req.binary.is_empty() && !binary_on_path(&req.binary))
        .map(|req| (req.binary.clone(), req.install_hint.clone()))
        .collect()
}

/// Check whether all plugins listed in `needs` are already installed and
/// satisfy any declared version constraints.
///
/// Returns `Ok(())` if all dependencies are present and their versions satisfy
/// any constraints, or an error describing the first failure.
pub fn check_plugin_needs(needs: &[String]) -> Result<(), String> {
    if needs.is_empty() {
        return Ok(());
    }
    let installed: std::collections::HashMap<String, String> = list_plugins()
        .into_iter()
        .map(|p| (p.manifest.name.clone(), p.manifest.version.clone()))
        .collect();

    for entry in needs {
        let spec = DepSpec::parse(entry);
        match installed.get(&spec.name) {
            None => {
                return Err(format!(
                    "required dependency '{}' is not installed",
                    spec.name
                ));
            }
            Some(ver) => {
                if !spec.satisfied_by(ver) {
                    return Err(format!(
                        "dependency '{}' requires version constraint '{}' but {} is installed",
                        spec.name, entry, ver
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Resolve installation order for a plugin and all its transitive dependencies.
///
/// Performs a topological sort using DFS. Returns an ordered list of plugin
/// names to install (dependencies first). Detects circular dependencies.
///
/// Only resolves plugins available in the registry index (`registry_plugins`).
/// Unknown dependencies are returned as-is and the caller decides whether
/// to error.
pub fn resolve_install_order(
    root: &str,
    registry_plugins: &[serde_json::Value],
) -> Result<Vec<String>, String> {
    let mut order: Vec<String> = Vec::new();
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut in_stack: std::collections::HashSet<String> = std::collections::HashSet::new();

    fn dfs(
        name: &str,
        registry: &[serde_json::Value],
        order: &mut Vec<String>,
        visited: &mut std::collections::HashSet<String>,
        in_stack: &mut std::collections::HashSet<String>,
    ) -> Result<(), String> {
        if visited.contains(name) {
            return Ok(());
        }
        if in_stack.contains(name) {
            return Err(format!(
                "Circular dependency detected: '{name}' depends on itself"
            ));
        }
        in_stack.insert(name.to_string());

        // Find the plugin in the registry index
        let needs: Vec<String> = registry
            .iter()
            .find(|p| p.get("name").and_then(|v| v.as_str()) == Some(name))
            .and_then(|p| p.get("needs"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        for dep in &needs {
            let dep_name = DepSpec::parse(dep).name;
            dfs(&dep_name, registry, order, visited, in_stack)?;
        }

        in_stack.remove(name);
        visited.insert(name.to_string());
        order.push(name.to_string());
        Ok(())
    }

    dfs(
        root,
        registry_plugins,
        &mut order,
        &mut visited,
        &mut in_stack,
    )?;
    Ok(order)
}

/// Load a plugin manifest from disk without running integrity/dependency checks.
///
/// Used internally for operations that need to read and then re-write the
/// manifest (e.g. `sign_plugin`).
fn load_plugin_manifest_raw(plugin_dir: &Path) -> Result<PluginManifest, String> {
    let manifest_path = plugin_dir.join("plugin.toml");
    let content = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("Failed to read {}: {e}", manifest_path.display()))?;
    toml::from_str(&content).map_err(|e| format!("Invalid plugin.toml: {e}"))
}

/// Remove a TOML section (and its contents) from `src`.
///
/// Strips everything from `[section_name]` up to (but not including) the next
/// bare `[` header, or to the end of the file. Case-sensitive.
fn strip_toml_section(src: &str, section_name: &str) -> String {
    let header = format!("[{section_name}]");
    let mut result = String::with_capacity(src.len());
    let mut skip = false;
    for line in src.lines() {
        let trimmed = line.trim();
        if trimmed == header {
            skip = true;
            continue;
        }
        // Any new bare [section] ends the skip (but not [[array]] tables)
        if skip && trimmed.starts_with('[') && !trimmed.starts_with("[[") && trimmed != header {
            skip = false;
        }
        if !skip {
            result.push_str(line);
            result.push('\n');
        }
    }
    result
}

/// Lint a plugin: validate its manifest, hook files, and structure.
///
/// Returns a [`PluginLintReport`] with any errors and warnings found.
/// This is a best-effort static analysis — it does not execute any hook scripts.
pub fn lint_plugin(name: &str) -> Result<PluginLintReport, String> {
    validate_plugin_name(name)?;
    let plugin_dir = plugins_dir().join(name);
    if !plugin_dir.exists() {
        return Err(format!("Plugin '{name}' is not installed"));
    }

    let mut errors: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    // 1. Load and parse manifest (this also runs version and integrity checks)
    let manifest = match load_plugin_manifest(&plugin_dir) {
        Ok(m) => m,
        Err(e) => {
            return Ok(PluginLintReport {
                plugin: name.to_string(),
                ok: false,
                errors: vec![e],
                warnings,
            });
        }
    };

    // 2. Check that all declared hook scripts exist and have correct extension
    let hooks = &manifest.hooks;
    let check_hook = |rel: &str, errors: &mut Vec<String>, warnings: &mut Vec<String>| {
        let abs = plugin_dir.join(rel);
        if !abs.exists() {
            errors.push(format!("Hook script not found: '{rel}'"));
            return;
        }
        // Warn if runtime tag and extension mismatch (best effort)
        if let Some(rt) = hooks.runtime.as_deref() {
            let ext = abs.extension().and_then(|e| e.to_str()).unwrap_or("");
            let expected = match rt {
                "python" | "py" => "py",
                "node" | "nodejs" => "js",
                "deno" => "ts",
                "go" | "golang" => "go",
                "ruby" | "rb" => "rb",
                "bash" | "sh" => "sh",
                "bun" => "ts",
                "php" => "php",
                "lua" => "lua",
                _ => "",
            };
            if !expected.is_empty() && ext != expected {
                warnings.push(format!(
                    "Hook '{rel}' has extension '.{ext}' but runtime is '{rt}' (expected '.{expected}')"
                ));
            }
        }
        // Check executable bit for native runtime
        #[cfg(unix)]
        if hooks.runtime.as_deref() == Some("native") {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = std::fs::metadata(&abs) {
                if meta.permissions().mode() & 0o111 == 0 {
                    errors.push(format!(
                        "Hook '{rel}' is not executable (chmod +x required for native runtime)"
                    ));
                }
            }
        }
    };

    if let Some(ref p) = hooks.ingest {
        check_hook(p, &mut errors, &mut warnings);
    }
    if let Some(ref p) = hooks.after_turn {
        check_hook(p, &mut errors, &mut warnings);
    }
    if let Some(ref p) = hooks.assemble {
        check_hook(p, &mut errors, &mut warnings);
    }
    if let Some(ref p) = hooks.compact {
        check_hook(p, &mut errors, &mut warnings);
    }
    if let Some(ref p) = hooks.bootstrap {
        check_hook(p, &mut errors, &mut warnings);
    }
    if let Some(ref p) = hooks.prepare_subagent {
        check_hook(p, &mut errors, &mut warnings);
    }
    if let Some(ref p) = hooks.merge_subagent {
        check_hook(p, &mut errors, &mut warnings);
    }

    // 3. Warn on missing optional but recommended fields
    if manifest.description.is_none() {
        warnings.push("Missing 'description' field in plugin.toml".to_string());
    }
    if manifest.author.is_none() {
        warnings.push("Missing 'author' field in plugin.toml".to_string());
    }
    if manifest.version.is_empty() {
        warnings.push("'version' field is empty in plugin.toml".to_string());
    }

    // 4. Warn if no hooks are declared at all
    if hooks.ingest.is_none()
        && hooks.after_turn.is_none()
        && hooks.assemble.is_none()
        && hooks.compact.is_none()
        && hooks.bootstrap.is_none()
    {
        warnings.push("No hooks declared in [hooks] section — plugin is a no-op".to_string());
    }

    // 5. Warn if plugin_depends references unknown plugins
    let plugins_root = plugin_dir.parent().unwrap_or(&plugin_dir);
    for dep in &manifest.plugin_depends {
        if !plugins_root.join(dep).join("plugin.toml").exists() {
            warnings.push(format!("Declared dependency '{dep}' is not installed"));
        }
    }

    // 6. If plugin is disabled, add informational warning
    if plugin_dir.join(".disabled").exists() {
        warnings.push("Plugin is currently disabled (.disabled marker present)".to_string());
    }

    // 7. Validate needs array for unknown capabilities
    let manifest_path = plugin_dir.join("plugin.toml");
    if let Ok(raw) = std::fs::read_to_string(&manifest_path) {
        let needs = extract_needs(&raw);
        const KNOWN_CAPABILITIES: &[&str] = &["network", "filesystem", "env", "subprocess", "gpu"];
        for cap in &needs {
            if !KNOWN_CAPABILITIES.contains(&cap.as_str()) {
                warnings.push(format!(
                    "Unknown capability '{}' in needs array (known: {})",
                    cap,
                    KNOWN_CAPABILITIES.join(", ")
                ));
            }
        }
    }

    // 8. Warn when declared hooks lack [integrity] entries — registry-installed
    //    plugins are rejected at install time without these hashes (issue #4036).
    //    Surface it locally so plugin authors fix it before submitting to the
    //    registry rather than after users hit the install-time hard error.
    let missing_integrity = manifest_missing_integrity_hooks(&manifest);
    if !missing_integrity.is_empty() {
        warnings.push(format!(
            "Missing [integrity] hashes for hook script(s): {}. \
             Registry-installed plugins are rejected without SHA-256 checksums for every hook. \
             Add `\"hooks/<script>\" = \"<sha256hex>\"` entries under `[integrity]` in plugin.toml \
             before publishing.",
            missing_integrity.join(", ")
        ));
    }

    // 9. Warn about missing system binaries declared in [[requires]]
    let missing_bins = check_system_requires(&manifest.requires);
    for (bin, hint) in &missing_bins {
        let hint_str = hint.as_deref().unwrap_or("(no install hint provided)");
        warnings.push(format!(
            "Required binary '{}' not found on PATH — {}",
            bin, hint_str
        ));
    }

    let ok = errors.is_empty();
    Ok(PluginLintReport {
        plugin: name.to_string(),
        ok,
        errors,
        warnings,
    })
}

fn check_hooks_exist(plugin_dir: &Path, manifest: &PluginManifest) -> bool {
    // Canonicalize plugin_dir first so the starts_with check works even when
    // the input path contains symlinks (e.g. /tmp → /private/tmp on macOS).
    let canonical_dir = match plugin_dir.canonicalize() {
        Ok(d) => d,
        Err(_) => return false,
    };
    let check = |rel_path: &str| -> bool {
        let joined = canonical_dir.join(rel_path);
        // Canonicalize to resolve any `..` and verify the resolved path
        // stays inside the plugin directory. If canonicalize fails (file
        // doesn't exist), the hook is missing.
        match joined.canonicalize() {
            Ok(abs) => abs.starts_with(&canonical_dir),
            Err(_) => false,
        }
    };

    let mut valid = true;
    if let Some(ref p) = manifest.hooks.ingest {
        if !check(p) {
            valid = false;
        }
    }
    if let Some(ref p) = manifest.hooks.after_turn {
        if !check(p) {
            valid = false;
        }
    }
    valid
}

/// Calculate total size of a directory recursively.
fn dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let meta = entry.metadata();
            if let Ok(m) = meta {
                if m.is_file() {
                    total += m.len();
                } else if m.is_dir() {
                    total += dir_size(&entry.path());
                }
            }
        }
    }
    total
}

/// Recursively copy a directory. Symlinks are skipped for security.
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        // Skip symlinks to prevent following links outside the plugin directory
        if ft.is_symlink() {
            continue;
        }
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if ft.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

/// Install runtime dependencies for a plugin based on its declared runtime and
/// any package manifest files found in its directory.
///
/// Returns a list of log lines describing what was (or was not) done.
///
/// # Errors
/// Returns an error if the plugin does not exist or if the dependency install
/// command exits with a non-zero status code.
pub async fn install_plugin_deps(name: &str) -> Result<Vec<String>, String> {
    validate_plugin_name(name)?;
    let plugin_dir = plugins_dir().join(name);
    if !plugin_dir.exists() {
        return Err(format!("Plugin '{name}' is not installed"));
    }

    // load_plugin_manifest_raw reads plugin.toml synchronously; run on the
    // blocking pool so we don't stall the async runtime.
    let manifest_dir = plugin_dir.clone();
    let manifest = tokio::task::spawn_blocking(move || load_plugin_manifest_raw(&manifest_dir))
        .await
        .map_err(|e| format!("load_plugin_manifest_raw task failed: {e}"))??;
    let runtime = manifest
        .hooks
        .runtime
        .as_deref()
        .unwrap_or("python")
        .to_string();

    let mut log: Vec<String> = Vec::new();

    // Determine the install command based on runtime and package manifest presence.
    // Returns `(executable, args, package_manifest_filename)`.
    let cmd_info: Option<(&'static str, Vec<&'static str>, &'static str)> = match runtime.as_str() {
        "python" | "py" => {
            if plugin_dir.join("requirements.txt").exists() {
                Some((
                    "pip",
                    vec!["install", "-r", "requirements.txt"],
                    "requirements.txt",
                ))
            } else {
                None
            }
        }
        "node" | "nodejs" => {
            if plugin_dir.join("package.json").exists() {
                Some(("npm", vec!["install"], "package.json"))
            } else {
                None
            }
        }
        "bun" => {
            if plugin_dir.join("package.json").exists() {
                Some(("bun", vec!["install"], "package.json"))
            } else {
                None
            }
        }
        "go" | "golang" => {
            if plugin_dir.join("go.mod").exists() {
                Some(("go", vec!["mod", "download"], "go.mod"))
            } else {
                None
            }
        }
        "ruby" | "rb" => {
            if plugin_dir.join("Gemfile").exists() {
                Some(("bundle", vec!["install"], "Gemfile"))
            } else {
                None
            }
        }
        "php" => {
            if plugin_dir.join("composer.json").exists() {
                Some(("composer", vec!["install"], "composer.json"))
            } else {
                None
            }
        }
        _ => None,
    };

    match cmd_info {
        None => {
            log.push(format!(
                "No package manifest found for runtime '{}' — nothing to install",
                runtime
            ));
        }
        Some((cmd, args, manifest_file)) => {
            log.push(format!(
                "Running: {} {} (manifest: {})",
                cmd,
                args.join(" "),
                manifest_file
            ));
            let output = tokio::process::Command::new(cmd)
                .args(&args)
                .current_dir(&plugin_dir)
                .output()
                .await
                .map_err(|e| format!("Failed to launch '{cmd}': {e}"))?;

            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            if !stdout.trim().is_empty() {
                log.push(stdout);
            }
            if !stderr.trim().is_empty() {
                log.push(stderr);
            }

            if !output.status.success() {
                return Err(format!(
                    "Dependency install failed for plugin '{name}' (exit {})",
                    output.status
                ));
            }
            log.push("Dependencies installed successfully.".to_string());
        }
    }

    Ok(log)
}

/// Install a plugin and all its declared dependencies from the registry.
///
/// Resolves the dependency graph, then installs each plugin in topological
/// order (dependencies first). Already-installed plugins are skipped.
/// Returns the list of plugin names that were newly installed.
pub async fn install_plugin_with_deps(
    name: &str,
    github_repo: Option<&str>,
) -> Result<Vec<String>, String> {
    validate_plugin_name(name)?;

    // Fetch the registry index to resolve the dependency graph.
    // Routed through librefang-http so the registry fetch honors [proxy] and
    // the workspace TLS roots (#3577).
    let repo = github_repo.unwrap_or("librefang/librefang-registry");
    let client = librefang_http::proxied_client_builder()
        .user_agent("librefang-plugin-installer/1.0")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;
    let registry_plugins = fetch_verified_index(&client, repo).await?;

    let order = resolve_install_order(name, &registry_plugins)?;

    let installed_names: std::collections::HashSet<String> = list_plugins()
        .into_iter()
        .map(|p| p.manifest.name.clone())
        .collect();

    let mut newly_installed = Vec::new();
    for dep_name in &order {
        if installed_names.contains(dep_name) {
            info!(
                plugin = dep_name.as_str(),
                "Dependency already installed, skipping"
            );
            continue;
        }
        let source = PluginSource::Registry {
            name: dep_name.clone(),
            github_repo: github_repo.map(String::from),
        };
        install_plugin(&source).await?;
        newly_installed.push(dep_name.clone());
    }
    Ok(newly_installed)
}

/// Open (or create) the persistent hook trace store at the default location.
///
/// The database is stored at `~/.librefang/hook_traces.db` and retains the
/// last 10,000 hook execution records across daemon restarts.
pub fn open_trace_store() -> Result<crate::trace_store::TraceStore, String> {
    let path = plugins_dir()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(plugins_dir)
        .join("hook_traces.db");
    crate::trace_store::TraceStore::open(&path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
