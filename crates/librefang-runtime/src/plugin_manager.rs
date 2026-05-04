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

use librefang_types::config::{PluginI18n, PluginManifest, PluginSystemRequirement};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// One embedded pubkey + its expiry (Unix seconds). Slot-0 (active) uses
/// `expires_at == None`; rotation-window slots MUST set an expiry, or
/// daemons in the field would accept the prior key indefinitely — and a
/// post-rotation leak of that prior private key would still be exploitable
/// against every still-running daemon binary that carries it. PR re-review
/// MEDIUM (round 3).
struct EmbeddedPubkey {
    /// Base64-encoded raw 32-byte Ed25519 public key. Field name is
    /// `pubkey_b64` (not `b64`) so the lockstep CI script's regex picks
    /// up *this* field unambiguously and not some unrelated future
    /// `b64: "..."` literal that drifts in (PR re-review LOW round 4).
    pubkey_b64: &'static str,
    /// `None` = active key, no expiry. `Some(t)` = prior key, valid only
    /// while `now() < t`.
    expires_at: Option<i64>,
}

/// Embedded raw 32-byte Ed25519 public keys for the official LibreFang
/// plugin registry. Slot 0 is the **current** production key (no expiry).
/// Subsequent slots carry **prior** keys during a rotation window, each
/// with a hard expiry timestamp.
///
/// Primary trust root for `librefang/librefang-registry` installs — the
/// earlier TOFU-via-HTTP approach was MITM-vulnerable on first install
/// (cafe wifi, hostile DNS, subdomain takeover) and silently pinned the
/// attacker key forever (PR review HIGH #5/#16). Compiling the key in
/// moves the trust path to HTTPS + the daemon release pipeline.
///
/// `EMBEDDED_REGISTRY_PUBKEYS[0].pubkey_b64` MUST match `REGISTRY_PUBLIC_KEY` in:
///   - `web/workers/registry-worker/wrangler.toml`
///   - `web/workers/marketplace-worker/wrangler.toml`
///   - `web/public/_worker.js`
///
/// `scripts/check-pubkey-lockstep.sh` (CI guard) extracts and compares
/// against slot 0 only.
///
/// Rotation procedure:
///   1. Generate new keypair, publish private to registry CI.
///   2. Land a daemon release that prepends the new key to slot 0 AND
///      moves the old slot-0 entry to slot 1 with `expires_at: Some(t)`
///      where `t` ≈ now + 4 weeks.
///   3. Roll registry / worker side to the new key.
///   4. After the deprecation window passes, drop the prior entry in a
///      follow-up daemon release. Daemons that didn't update by then will
///      hard-fail installs (the failure surfaces an actionable error
///      message, unlike the previous "accept forever" behaviour).
const EMBEDDED_REGISTRY_PUBKEYS: &[EmbeddedPubkey] = &[
    EmbeddedPubkey {
        pubkey_b64: "ClGa0Ucap8NdrKAy1rw9Tt6A9I8eg4zJ53+xIuKMuq0=",
        expires_at: None,
    },
];

/// Default URL for self-hosted registries that opt into HTTP pubkey
/// resolution (operators of `acme/private-registry` style forks who don't
/// want to rebuild the daemon to ship a key constant). Off the official
/// trust path — never consulted for the official registry, which uses
/// [`EMBEDDED_REGISTRY_PUBKEYS`].
///
/// Uses the `/api/registry/pubkey` form because the official custom
/// domain routes only `/api/*` to the worker; the `.well-known/...`
/// alias only resolves on the workers.dev hostname.
const OFFICIAL_PUBKEY_URL: &str = "https://stats.librefang.ai/api/registry/pubkey";

/// `owner/repo` of the official LibreFang plugin registry on GitHub.
///
/// When `fetch_verified_index` is called with this exact value (the default),
/// the daemon prefers the worker-signed `/api/registry/index.json` mirror at
/// [`OFFICIAL_INDEX_URL`] / [`OFFICIAL_INDEX_SIG_URL`] over GitHub raw — the
/// mirror serves a flat plugins array signed by the registry-worker on every
/// cron tick, giving real Ed25519 verification end-to-end. Self-hosted forks
/// (any other `owner/repo`) keep using the GitHub raw fallback unless the
/// `LIBREFANG_REGISTRY_INDEX_URL` env override is set.
const OFFICIAL_REGISTRY_REPO: &str = "librefang/librefang-registry";

/// Daemon-shaped flat plugins index, signed and served by `registry-worker`.
/// Format: `[{"name": "...", "version": "...", "description": "...",
/// "needs": ["dep1", ...]}, ...]`. The signature at
/// [`OFFICIAL_INDEX_SIG_URL`] covers these exact bytes.
const OFFICIAL_INDEX_URL: &str = "https://stats.librefang.ai/api/registry/index.json";

/// Base64-encoded Ed25519 signature over the bytes returned by
/// [`OFFICIAL_INDEX_URL`].
const OFFICIAL_INDEX_SIG_URL: &str =
    "https://stats.librefang.ai/api/registry/index.json.sig";

/// On-disk pin for the registry pubkey (TOFU cache).
///
/// First successful fetch is written here; later calls read this file
/// instead of going to the network. Rotation requires deleting this file
/// (operators or a daemon-side `librefang plugin rotate-pubkey` command).
fn registry_pubkey_cache_path() -> Result<PathBuf, String> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| "Cannot determine home directory for pubkey cache".to_string())?;
    Ok(PathBuf::from(home).join(".librefang").join("registry.pub"))
}

/// True iff `s` decodes to a valid 32-byte non-all-zero Ed25519 pubkey.
fn is_valid_registry_pubkey_b64(s: &str) -> bool {
    use base64::Engine as _;
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(s.trim()) else {
        return false;
    };
    bytes.len() == 32 && !bytes.iter().all(|&b| b == 0)
}

/// Read the TOFU pubkey cache file with O_NOFOLLOW + regular-file check.
///
/// Hardens against PR review MEDIUM #13: a compromised post-install hook
/// could otherwise plant `~/.librefang/registry.pub` as a symlink to
/// `/etc/passwd` (read returns garbage that fails b64 validation, harmless)
/// — but the subsequent `fs::write` would follow the symlink and corrupt
/// the target. Refusing to follow symlinks and validating regular-file
/// status before reading closes that surface.
fn read_pubkey_cache_safely(path: &std::path::Path) -> Option<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut opts = std::fs::OpenOptions::new();
        opts.read(true).custom_flags(libc::O_NOFOLLOW);
        let mut f = opts.open(path).ok()?;
        let meta = f.metadata().ok()?;
        if !meta.is_file() {
            warn!("Pubkey cache {} is not a regular file — ignoring", path.display());
            return None;
        }
        use std::io::Read as _;
        let mut buf = String::new();
        f.read_to_string(&mut buf).ok()?;
        Some(buf)
    }
    #[cfg(not(unix))]
    {
        // Windows symlink semantics differ enough that a generic O_NOFOLLOW
        // doesn't translate; still validate regular-file status and rely on
        // NTFS ACLs for the rest.
        let meta = std::fs::metadata(path).ok()?;
        if !meta.is_file() {
            warn!("Pubkey cache {} is not a regular file — ignoring", path.display());
            return None;
        }
        std::fs::read_to_string(path).ok()
    }
}

/// Write the TOFU cache with mode 0600 on Unix so other local users can't
/// read or replace it. Hardens against PR review MEDIUM #13.
fn write_pubkey_cache_safely(path: &std::path::Path, value: &str) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        use std::io::Write as _;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)?;
        f.write_all(value.as_bytes())?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, value)
    }
}

/// Resolve the registry pubkey using the layered chain:
///   1. `LIBREFANG_REGISTRY_PUBKEY` env var override (always wins — covers
///      self-hosted forks and operator-driven rotation).
///   2. [`EMBEDDED_REGISTRY_PUBKEYS`] compiled-in constant (the **primary
///      trust root** for the official `librefang/librefang-registry`
///      registry — no network call, no MITM surface).
///   3. `~/.librefang/registry.pub` TOFU cache + HTTP fetch from
///      `LIBREFANG_REGISTRY_PUBKEY_URL` — only consulted when the env var
///      override and the embedded key are both unavailable, i.e. a
///      self-hosted fork that didn't ship a custom binary. The HTTP path
///      is **opt-in via env var only** for that reason: the official
///      registry never reaches it.
///
/// Returns `Err` only when all sources are unavailable or invalid; callers
/// may then choose to hard-fail (index verification) or fall back to
/// weaker integrity checks (archive install with verified SHA-256).
async fn resolve_registry_pubkey(client: &reqwest::Client) -> Result<String, String> {
    if let Ok(env_key) = std::env::var("LIBREFANG_REGISTRY_PUBKEY") {
        let trimmed = env_key.trim().to_string();
        if !trimmed.is_empty() {
            if is_valid_registry_pubkey_b64(&trimmed) {
                return Ok(trimmed);
            }
            warn!("LIBREFANG_REGISTRY_PUBKEY is set but is not a valid 32-byte Ed25519 key");
        }
    }

    // Active embedded key (slot 0) is the primary trust anchor for the
    // official registry. The full slice — including any rotation-window
    // keys — is consulted at verification time via
    // `verify_registry_index_multi`, which also enforces per-key expiry.
    // Defense-in-depth invariant: slot 0 (the active key) MUST NOT have
    // an expiry — that's what "active" means. A maintainer who absent-
    // mindedly sets `expires_at: Some(_)` on slot 0 during a rotation
    // edit would silently break installs the moment the timestamp passed
    // (PR re-review LOW round 4). Catch it in debug builds before
    // shipping; the tests/ block also asserts this at compile + test time.
    debug_assert!(
        EMBEDDED_REGISTRY_PUBKEYS
            .first()
            .map(|k| k.expires_at.is_none())
            .unwrap_or(true),
        "EMBEDDED_REGISTRY_PUBKEYS[0] must have expires_at: None (active key)"
    );

    if let Some(active) = EMBEDDED_REGISTRY_PUBKEYS
        .first()
        .filter(|k| is_valid_registry_pubkey_b64(k.pubkey_b64))
    {
        return Ok(active.pubkey_b64.to_string());
    }
    // Invalid slot-0 key is a build-time mistake; warn but keep trying
    // so the daemon stays usable for self-hosted forks.
    warn!("EMBEDDED_REGISTRY_PUBKEYS slot 0 is malformed — falling through to TOFU/HTTP");

    let cache_path = registry_pubkey_cache_path()?;
    if let Some(cached) = read_pubkey_cache_safely(&cache_path) {
        let trimmed = cached.trim().to_string();
        if is_valid_registry_pubkey_b64(&trimmed) {
            debug!("Using TOFU-pinned registry pubkey from {}", cache_path.display());
            return Ok(trimmed);
        }
        warn!(
            "Cached registry pubkey at {} is malformed; ignoring",
            cache_path.display()
        );
    }

    let url = std::env::var("LIBREFANG_REGISTRY_PUBKEY_URL")
        .unwrap_or_else(|_| OFFICIAL_PUBKEY_URL.to_string());
    let resp = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("Failed to fetch registry pubkey from {url}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!(
            "Registry pubkey endpoint {url} returned HTTP {}",
            resp.status()
        ));
    }
    let body = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read registry pubkey response: {e}"))?;
    let trimmed = body.trim().to_string();
    if !is_valid_registry_pubkey_b64(&trimmed) {
        return Err(format!(
            "Registry pubkey from {url} is not a valid base64-encoded 32-byte Ed25519 key"
        ));
    }

    if let Some(parent) = cache_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match write_pubkey_cache_safely(&cache_path, &trimmed) {
        Ok(()) => info!(
            "Pinned registry pubkey to {} (TOFU); rotation requires deleting this file",
            cache_path.display()
        ),
        Err(e) => warn!(
            "Could not pin registry pubkey to {}: {} — pubkey will be re-fetched next install",
            cache_path.display(),
            e
        ),
    }

    Ok(trimmed)
}

/// Verify an Ed25519 signature over registry index JSON bytes.
///
/// The registry is expected to serve a companion file `index.json.sig`
/// containing the raw 64-byte Ed25519 signature, base64-encoded.
///
/// # Arguments
/// - `index_bytes`: the raw bytes of `index.json`
/// - `sig_b64`: base64-encoded 64-byte signature from `index.json.sig`
/// - `pubkey_b64`: base64-encoded 32-byte Ed25519 public key
///
/// Returns `Ok(())` if the signature is valid, `Err(reason)` otherwise.
fn verify_registry_index(
    index_bytes: &[u8],
    sig_b64: &str,
    pubkey_b64: &str,
) -> Result<(), String> {
    use base64::Engine as _;
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};

    let sig_bytes = base64::engine::general_purpose::STANDARD
        .decode(sig_b64.trim())
        .map_err(|e| format!("Invalid signature encoding: {e}"))?;

    let key_bytes = base64::engine::general_purpose::STANDARD
        .decode(pubkey_b64.trim())
        .map_err(|e| format!("Invalid public key encoding: {e}"))?;

    let sig_arr: [u8; 64] = sig_bytes
        .try_into()
        .map_err(|_| "Signature must be exactly 64 bytes".to_string())?;

    let key_arr: [u8; 32] = key_bytes
        .try_into()
        .map_err(|_| "Public key must be exactly 32 bytes".to_string())?;

    let signature = Signature::from_bytes(&sig_arr);
    let verifying_key =
        VerifyingKey::from_bytes(&key_arr).map_err(|e| format!("Invalid public key: {e}"))?;

    verifying_key
        .verify(index_bytes, &signature)
        .map_err(|e| format!("Signature verification failed: {e}"))
}

/// Verify the index signature against `resolved_pubkey` first, then fall
/// back to any **non-expired** prior keys in [`EMBEDDED_REGISTRY_PUBKEYS`].
/// Expired keys are skipped — closes round-3 MEDIUM (a leaked prior key
/// must not stay accepted forever).
///
/// Provides a bounded rotation grace window: when ops rotates the
/// worker-side key but a daemon in the field is still on the previous
/// release, the daemon keeps verifying installs against the prior key
/// until its `expires_at` passes, after which installs hard-fail with an
/// actionable error.
fn verify_registry_index_multi(
    index_bytes: &[u8],
    sig_b64: &str,
    resolved_pubkey: &str,
) -> Result<(), String> {
    let mut last_err = match verify_registry_index(index_bytes, sig_b64, resolved_pubkey) {
        Ok(()) => return Ok(()),
        Err(e) => e,
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // Trim once for the dedup compare. All current call sites pass a
    // pre-trimmed key, but a future code path that forgets would
    // otherwise verify the resolved key twice (wasted CPU, not unsafe).
    // PR re-review MEDIUM round 4.
    let resolved_trimmed = resolved_pubkey.trim();
    let mut expired_count = 0usize;
    for embedded in EMBEDDED_REGISTRY_PUBKEYS {
        if embedded.pubkey_b64 == resolved_trimmed {
            continue; // already tried as the resolved key
        }
        if let Some(exp) = embedded.expires_at {
            if now >= exp {
                debug!(
                    "Skipping embedded pubkey: expired at {} (now {})",
                    exp, now
                );
                expired_count += 1;
                continue;
            }
        }
        match verify_registry_index(index_bytes, sig_b64, embedded.pubkey_b64) {
            Ok(()) => {
                warn!(
                    "Registry index verified against a prior embedded pubkey \
                     (rotation grace window). Daemon binary still carries the \
                     prior key — update to a newer release before its expiry \
                     to keep installs working."
                );
                return Ok(());
            }
            Err(e) => last_err = e,
        }
    }
    // PR re-review MEDIUM round 4: when slot-0 verification fails AND
    // every prior key in the slice has aged out, surface "your daemon
    // binary is past its rotation window — upgrade" so the user has an
    // actionable next step instead of a bare verify-failed message.
    if expired_count > 0 {
        last_err = format!(
            "{last_err} ({expired_count} prior embedded pubkey(s) past expiry — \
             this daemon binary is past its rotation window; upgrade librefang \
             to restore plugin installs)"
        );
    }
    Err(last_err)
}

// `verify_archive_signature` was removed in PR #4600 — it always fetched
// `{listing_url}.sig` (a GitHub Contents API URL), which 404s in the
// official registry layout, causing the function to silently return
// Ok(()) on every invocation. Per-plugin trust now flows through the
// signed plugins-index membership check in `install_from_registry`
// instead. See PR review CRITICAL #3.

/// Return the path used to cache a registry index locally.
/// The filename is a sanitised form of the registry URL.
fn registry_cache_path(registry: &str) -> std::path::PathBuf {
    let cache_dir = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".librefang")
        .join("registry_cache");
    // Sanitise the URL into a safe filename (replace non-alphanumeric with '_').
    let safe_name: String = registry
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    cache_dir.join(format!("{safe_name}.json"))
}

/// Return the default registry cache TTL in seconds (1 hour).
fn default_registry_cache_ttl_secs() -> u64 {
    3600
}

/// Try to load a cached registry index.
/// Returns `Some(bytes)` if the cache file exists and is newer than `ttl_secs`.
fn load_registry_cache(path: &std::path::Path, ttl_secs: u64) -> Option<Vec<u8>> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    let age = std::time::SystemTime::now()
        .duration_since(modified)
        .unwrap_or(std::time::Duration::MAX);
    if age.as_secs() > ttl_secs {
        return None; // stale
    }
    std::fs::read(path).ok()
}

/// Write bytes to the registry cache, creating parent dirs as needed.
fn save_registry_cache(path: &std::path::Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, bytes);
}

/// Pick the `(index_url, sig_url)` pair to fetch for `registry`, honoring
/// `LIBREFANG_REGISTRY_INDEX_URL` / `LIBREFANG_REGISTRY_INDEX_SIG_URL`
/// overrides. For the official registry the default is the worker-signed
/// mirror at [`OFFICIAL_INDEX_URL`] / [`OFFICIAL_INDEX_SIG_URL`] (only path
/// that yields a verifiable signature — the GitHub repo has no committed
/// `index.json`). Self-hosted forks fall back to GitHub raw, which is
/// unsigned by default; operators can opt back into signing by pointing the
/// env vars at their own signed mirror.
fn registry_index_urls(
    registry: &str,
    env_index: Option<String>,
    env_sig: Option<String>,
) -> (String, String) {
    let default_index = if registry == OFFICIAL_REGISTRY_REPO {
        OFFICIAL_INDEX_URL.to_string()
    } else {
        format!("https://raw.githubusercontent.com/{registry}/main/index.json")
    };
    let default_sig = if registry == OFFICIAL_REGISTRY_REPO {
        OFFICIAL_INDEX_SIG_URL.to_string()
    } else {
        format!("https://raw.githubusercontent.com/{registry}/main/index.json.sig")
    };
    (
        env_index.unwrap_or(default_index),
        env_sig.unwrap_or(default_sig),
    )
}

/// Fetch registry `index.json` and optionally verify its Ed25519 signature.
///
/// Signature verification is skipped when:
/// - `LIBREFANG_REGISTRY_VERIFY=0` env var is set
/// - No `index.json.sig` companion file exists at the registry
/// - The configured public key is the placeholder value (all-zero bytes)
///
/// A missing signature file produces a warning; a present but invalid
/// signature is always a hard error.
pub async fn fetch_verified_index(
    client: &reqwest::Client,
    registry: &str,
) -> Result<Vec<serde_json::Value>, String> {
    let cache_path = registry_cache_path(registry);
    let ttl = std::env::var("LIBREFANG_REGISTRY_CACHE_TTL_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(default_registry_cache_ttl_secs);

    // Try cache first (skip if LIBREFANG_REGISTRY_NO_CACHE=1).
    let skip_cache = std::env::var("LIBREFANG_REGISTRY_NO_CACHE").as_deref() == Ok("1");
    if !skip_cache {
        if let Some(cached) = load_registry_cache(&cache_path, ttl) {
            if let Ok(value) = serde_json::from_slice::<Vec<serde_json::Value>>(&cached) {
                debug!("Using cached registry index for {registry} (age < {ttl}s)");
                return Ok(value);
            }
        }
    }

    let (index_url, sig_url) = registry_index_urls(
        registry,
        std::env::var("LIBREFANG_REGISTRY_INDEX_URL").ok(),
        std::env::var("LIBREFANG_REGISTRY_INDEX_SIG_URL").ok(),
    );

    // Fetch index bytes.
    let index_resp = client
        .get(&index_url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch registry index: {e}"))?;

    if !index_resp.status().is_success() {
        return Err(format!(
            "Registry index returned HTTP {}",
            index_resp.status()
        ));
    }

    let index_bytes = index_resp
        .bytes()
        .await
        .map_err(|e| format!("Failed to read registry index body: {e}"))?;

    // Skip verification if explicitly disabled.
    if std::env::var("LIBREFANG_REGISTRY_VERIFY").as_deref() == Ok("0") {
        warn!("Registry signature verification disabled via LIBREFANG_REGISTRY_VERIFY=0");
    } else {
        // Resolve the public key via env > TOFU cache > worker fetch. If none
        // of the three paths produce a valid key we hard-fail: the registry
        // index drives every subsequent install, so trusting an unverified
        // index would mask a compromised or man-in-the-middle registry.
        let pubkey = resolve_registry_pubkey(client).await.map_err(|e| {
            format!(
                "Plugin registry public key unavailable — refusing to fetch registry index. {e} \
                 Configure LIBREFANG_REGISTRY_PUBKEY (base64), point \
                 LIBREFANG_REGISTRY_PUBKEY_URL at a reachable endpoint, or set \
                 LIBREFANG_REGISTRY_VERIFY=0 to disable verification (development use only)."
            )
        })?;

        // Hard-fail on missing or unreachable .sig — for ALL registries,
        // not just the official one. The previous "soft for forks" path
        // was bypassable: an attacker who could change the registry slug
        // to anything other than "librefang/librefang-registry" flipped
        // off the require_sig flag and then served an unsigned index that
        // EMBEDDED pubkey couldn't verify but the warning path silently
        // accepted. Closes PR re-review HIGH-NEW-B.
        //
        // Self-hosted forks that haven't deployed signing infrastructure
        // yet must explicitly opt out via LIBREFANG_REGISTRY_VERIFY=0
        // (gated above) — there is no implicit-by-slug downgrade path.
        match client.get(&sig_url).send().await {
            Ok(sig_resp) if sig_resp.status().is_success() => {
                let sig_text = sig_resp
                    .text()
                    .await
                    .map_err(|e| format!("Failed to read signature: {e}"))?;
                verify_registry_index_multi(&index_bytes, sig_text.trim(), &pubkey)?;
                info!(registry, "Registry index signature verified OK");
            }
            Ok(sig_resp) => {
                return Err(format!(
                    "Registry index signature unavailable (HTTP {} from {sig_url}) \
                     — refusing to trust unsigned index. Set \
                     LIBREFANG_REGISTRY_VERIFY=0 only if you are testing against an \
                     unsigned development mirror.",
                    sig_resp.status()
                ));
            }
            Err(e) => {
                return Err(format!(
                    "Registry index signature fetch failed ({sig_url}): {e}. \
                     Refusing to trust unsigned index — a network downgrade \
                     attack must not silently bypass verification."
                ));
            }
        }
    }

    // Persist to disk cache for future calls.
    save_registry_cache(&cache_path, &index_bytes);

    serde_json::from_slice::<Vec<serde_json::Value>>(&index_bytes)
        .map_err(|e| format!("Failed to parse registry index JSON: {e}"))
}

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

/// Install a plugin from a source.
pub async fn install_plugin(source: &PluginSource) -> Result<PluginInfo, String> {
    let plugins = ensure_plugins_dir().map_err(|e| format!("Cannot create plugins dir: {e}"))?;

    let info = match source {
        PluginSource::Local { path } => {
            // install_from_local walks/copies a directory tree synchronously;
            // run it on the blocking pool so we don't stall the async runtime.
            let path = path.clone();
            let plugins = plugins.clone();
            tokio::task::spawn_blocking(move || install_from_local(&path, &plugins))
                .await
                .map_err(|e| format!("install_from_local task panicked: {e}"))?
        }
        PluginSource::Registry { name, github_repo } => {
            let repo = github_repo
                .as_deref()
                .unwrap_or("librefang/librefang-registry");
            install_from_registry(name, repo, &plugins).await
        }
        PluginSource::Git { url, branch } => {
            install_from_git(url, branch.as_deref(), &plugins).await
        }
    }?;

    // Check that all declared plugin dependencies are already installed.
    let raw_toml = tokio::fs::read_to_string(info.path.join("plugin.toml"))
        .await
        .unwrap_or_default();
    let needs = extract_needs(&raw_toml);
    if let Err(e) = check_plugin_needs(&needs) {
        // Don't remove the partially-installed plugin — let the user decide.
        // Just warn so they know what to install next.
        warn!("{e}");
    }

    // Warn about missing system binaries declared in [[requires]].
    let missing_bins = check_system_requires(&info.manifest.requires);
    for (bin, hint) in &missing_bins {
        let hint_str = hint.as_deref().unwrap_or("(no install hint provided)");
        warn!(
            "Plugin '{}' requires system binary '{}' which was not found on PATH. {}",
            info.manifest.name, bin, hint_str
        );
    }

    Ok(info)
}

/// Install from a local directory by copying.
fn install_from_local(src: &Path, plugins_dir: &Path) -> Result<PluginInfo, String> {
    // Canonicalize the source path to resolve symlinks and relative components
    let canonical_src = src
        .canonicalize()
        .map_err(|e| format!("Failed to resolve local path '{}': {e}", src.display()))?;

    // Reject paths that still contain '..' after canonicalization (should not happen, but defense in depth)
    if canonical_src
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(format!(
            "Refusing to install from path with '..' components: {}",
            canonical_src.display()
        ));
    }

    warn!(
        path = %canonical_src.display(),
        "Installing plugin from local path"
    );

    // Validate source has a plugin.toml
    let manifest = load_plugin_manifest(&canonical_src)?;
    // Validate manifest name is safe for use as a directory name
    validate_plugin_name(&manifest.name)?;
    let target_dir = plugins_dir.join(&manifest.name);

    if target_dir.exists() {
        return Err(format!(
            "Plugin '{}' already installed at {}. Remove it first.",
            manifest.name,
            target_dir.display()
        ));
    }

    copy_dir_recursive(&canonical_src, &target_dir)
        .map_err(|e| format!("Failed to copy plugin: {e}"))?;

    info!(plugin = manifest.name, "Installed plugin from local path");
    get_plugin_info(&manifest.name)
}

/// Validate that a GitHub repo string looks like `owner/repo`.
fn validate_github_repo(repo: &str) -> Result<(), String> {
    let parts: Vec<&str> = repo.split('/').collect();
    if parts.len() != 2
        || parts[0].is_empty()
        || parts[1].is_empty()
        || repo.contains("..")
        || repo.contains(' ')
    {
        return Err(format!(
            "Invalid GitHub repo '{repo}': must be 'owner/repo'"
        ));
    }
    Ok(())
}

/// Install from a GitHub plugin registry (`owner/repo`).
async fn install_from_registry(
    name: &str,
    github_repo: &str,
    plugins_dir: &Path,
) -> Result<PluginInfo, String> {
    validate_plugin_name(name)?;
    validate_github_repo(github_repo)?;
    let target_dir = plugins_dir.join(name);
    if target_dir.exists() {
        return Err(format!(
            "Plugin '{name}' already installed. Remove it first."
        ));
    }

    let base_url = format!("https://api.github.com/repos/{github_repo}/contents/plugins");
    let listing_url = format!("{base_url}/{name}");

    let client = crate::http_client::client_builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    // List files in the plugin directory
    let resp = client
        .get(&listing_url)
        .header("Accept", "application/vnd.github.v3+json")
        .send()
        .await
        .map_err(|e| format!("Failed to fetch plugin '{name}' from registry: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!(
            "Plugin '{name}' not found in registry (HTTP {})",
            resp.status()
        ));
    }

    let files: Vec<GitHubContent> = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse registry response: {e}"))?;

    // Create target directory
    tokio::fs::create_dir_all(&target_dir)
        .await
        .map_err(|e| format!("Failed to create plugin dir: {e}"))?;

    // Download each file — cleanup on failure
    let download_result = async {
        for file in &files {
            download_github_entry(&client, file, &target_dir, 0).await?;
        }
        Ok::<(), String>(())
    }
    .await;

    if let Err(e) = download_result {
        // Clean up partial download
        let _ = tokio::fs::remove_dir_all(&target_dir).await;
        return Err(format!("Failed to download plugin '{name}': {e}"));
    }

    // Verify checksum if a checksum file exists. This catches in-flight
    // tampering of the manifest bytes between GitHub and the daemon, but
    // does NOT serve as a fallback for the index-membership check below
    // (PR re-review HIGH-NEW-C): the .sha256 file lives on the same
    // GitHub repo as the plugin, so an attacker who controls the listing
    // can forge a matching checksum trivially.
    if let Some(expected) = fetch_checksum(&client, &listing_url, name).await {
        let manifest_bytes = tokio::fs::read(target_dir.join("plugin.toml"))
            .await
            .unwrap_or_default();
        if let Err(e) = verify_checksum(&manifest_bytes, &expected) {
            let _ = tokio::fs::remove_dir_all(&target_dir).await;
            return Err(e);
        }
        info!(plugin = name, "Checksum verified OK");
    } else {
        debug!(
            plugin = name,
            "No checksum file alongside this plugin release — relying on \
             signed index membership instead."
        );
    }

    // Per-plugin Ed25519 archive signatures are NOT served by the official
    // registry — the older code that fetched `{listing_url}.sig` was always
    // a 404 (PR review CRITICAL #3) and silently passed every install.
    // Instead, gate the install on membership in the signed plugins-index:
    // an attacker who can serve a malicious GitHub Contents listing for
    // `<name>` cannot also forge an entry for `<name>` in the worker's
    // Ed25519-signed flat index (the worker won't sign content it didn't
    // pull from the registry repo's committed `plugins-index.json`).
    // Note that `checksum_verified` is now NOT a fallback for index-fetch
    // failure (PR re-review HIGH-NEW-C). The SHA-256 checksum file lives
    // on the same attacker-controlled GitHub repo as the plugin itself,
    // so an attacker who can serve a doctored manifest with a matching
    // checksum AND DoS stats.librefang.ai gets a free pass on the older
    // logic. Refuse to install on any index-fetch failure: a real
    // operational network issue should stop installs (which fail safe),
    // not silently downgrade to a weaker integrity check.
    if std::env::var("LIBREFANG_ARCHIVE_VERIFY").as_deref() == Ok("0") {
        debug!("Index-membership verification disabled via LIBREFANG_ARCHIVE_VERIFY=0");
    } else {
        // Single retry with backoff catches transient network blips
        // without papering over a sustained outage / active downgrade.
        let mut last_err: Option<String> = None;
        let mut entries: Option<Vec<serde_json::Value>> = None;
        for attempt in 0..2 {
            match fetch_verified_index(&client, github_repo).await {
                Ok(es) => {
                    entries = Some(es);
                    break;
                }
                Err(e) => {
                    last_err = Some(e);
                    if attempt == 0 {
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    }
                }
            }
        }

        let Some(index_entries) = entries else {
            let _ = tokio::fs::remove_dir_all(&target_dir).await;
            return Err(format!(
                "Cannot verify plugin '{name}' integrity: signed registry index \
                 fetch failed after retry. {} Refusing to install — install must \
                 fail safe when the trust root is unreachable, regardless of any \
                 SHA-256 checksum on the GitHub repo (which an attacker who can \
                 serve a doctored manifest can also forge).",
                last_err.unwrap_or_default()
            ));
        };

        let in_index = index_entries.iter().any(|e| {
            e.get("name").and_then(|v| v.as_str()) == Some(name)
        });
        if !in_index {
            let _ = tokio::fs::remove_dir_all(&target_dir).await;
            return Err(format!(
                "Plugin '{name}' is not present in the signed registry index. \
                 Refusing to install — the GitHub Contents listing alone is not \
                 a sufficient trust root. If this is a brand-new plugin, wait \
                 for the registry's CI to regenerate plugins-index.json and \
                 re-sign before installing."
            ));
        }
        info!(plugin = name, "Plugin presence in signed index confirmed");
    }

    // Bug #3804 — verify hook script integrity after install.
    //
    // The checksum above only covers plugin.toml (the manifest).  Hook scripts
    // that are referenced in the manifest but NOT listed in its [integrity]
    // section bypass all content verification — an attacker who controls the
    // download can serve a legitimate manifest with a valid checksum while
    // substituting malicious hook scripts.
    //
    // If the manifest declares hook scripts, every one of them MUST have a
    // corresponding entry in [integrity].  Missing entries are a hard error
    // for registry-installed plugins; authors who intentionally omit integrity
    // hashes (e.g. during development) can install via Local or Git sources.
    {
        let manifest_path = target_dir.join("plugin.toml");
        let manifest_opt = tokio::fs::read_to_string(&manifest_path)
            .await
            .ok()
            .and_then(|s| toml::from_str::<PluginManifest>(&s).ok());
        match manifest_opt {
            Some(manifest) => {
                let missing_integrity = manifest_missing_integrity_hooks(&manifest);
                if !missing_integrity.is_empty() {
                    // Hard error: registry plugins must declare integrity hashes for
                    // every hook script.  Without them, the hook content is unverified
                    // and could have been substituted after the manifest was signed.
                    let _ = tokio::fs::remove_dir_all(&target_dir).await;
                    return Err(format!(
                        "Plugin '{}' is missing [integrity] hashes for hook script(s): {}. \
                         Registry-installed plugins must provide SHA-256 checksums for every \
                         hook script declared in [hooks] so that tampered scripts are detected \
                         at load time. Add an [integrity] section to plugin.toml with \
                         \"hooks/<script>\" = \"<sha256hex>\" entries, or install via a local \
                         path (PluginSource::Local) to bypass this requirement.",
                        manifest.name,
                        missing_integrity.join(", ")
                    ));
                }
            }
            None => {
                // Manifest could not be re-read after install — treat as integrity failure.
                let _ = tokio::fs::remove_dir_all(&target_dir).await;
                return Err(format!(
                    "Plugin '{name}': failed to re-read plugin.toml after install \
                     — cannot verify hook script integrity"
                ));
            }
        }
    }

    info!(
        plugin = name,
        "Plugin installed successfully (manifest + hook script integrity verified)"
    );

    // Bust the registry cache so subsequent searches see an up-to-date index.
    let cache_path = registry_cache_path(github_repo);
    let _ = tokio::fs::remove_file(&cache_path).await;

    get_plugin_info(name)
}

/// Lightweight entry returned when browsing a registry.
///
/// Populated from each plugin's `plugin.toml` when available. Fields beyond
/// `name`/`registry` are optional so that registries that fail to serve a
/// manifest still degrade gracefully to a name-only listing.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct RegistryPluginEntry {
    pub name: String,
    pub registry: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Hook names declared by the plugin (e.g. `ingest`, `after_turn`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hooks: Vec<String>,
    /// Per-language overrides for `name` / `description`. Keyed by BCP-47
    /// tag (`zh`, `zh-TW`, …). API routes resolve `Accept-Language` against
    /// this and fall back to the English values above.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub i18n: HashMap<String, PluginI18n>,
}

/// Disk cache file for an enriched registry listing.
///
/// Stored separately from the `index.json` cache so that listings built from
/// the GitHub Contents API + per-plugin manifest fetches do not clobber a
/// signed index cache.
fn registry_listing_cache_path(registry: &str) -> std::path::PathBuf {
    let cache_dir = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".librefang")
        .join("registry_cache");
    let safe_name: String = registry
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    cache_dir.join(format!("{safe_name}__listing.json"))
}

/// Fetch and parse `plugins/<name>/plugin.toml` from a registry, extracting the
/// fields we care about for a browse-listing card. Network and parse errors
/// degrade to `None` so a single bad plugin does not sink the whole listing.
async fn fetch_registry_plugin_meta(
    client: &reqwest::Client,
    github_repo: &str,
    name: &str,
) -> RegistryPluginEntry {
    let mut entry = RegistryPluginEntry {
        name: name.to_string(),
        registry: github_repo.to_string(),
        ..Default::default()
    };

    let url =
        format!("https://raw.githubusercontent.com/{github_repo}/main/plugins/{name}/plugin.toml");
    let text = match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => resp.text().await.ok(),
        _ => None,
    };
    let Some(text) = text else { return entry };

    let Ok(value) = toml::from_str::<toml::Value>(&text) else {
        return entry;
    };
    if let Some(v) = value.get("version").and_then(|v| v.as_str()) {
        entry.version = Some(v.to_string());
    }
    if let Some(v) = value.get("description").and_then(|v| v.as_str()) {
        entry.description = Some(v.to_string());
    }
    if let Some(v) = value.get("author").and_then(|v| v.as_str()) {
        entry.author = Some(v.to_string());
    }
    if let Some(hooks) = value.get("hooks").and_then(|v| v.as_table()) {
        entry.hooks = hooks.keys().cloned().collect();
        entry.hooks.sort();
    }
    entry.i18n = parse_plugin_i18n_blocks(&value);
    entry
}

/// Pull `[i18n.<lang>]` tables off a parsed plugin TOML, keeping only the
/// `name` and `description` overrides. Empty entries (neither field set)
/// are dropped to keep the map tight.
///
/// Exposed as `pub(crate)` so it can be unit-tested without a network
/// round-trip; the production caller is `fetch_registry_plugin_meta`.
pub(crate) fn parse_plugin_i18n_blocks(value: &toml::Value) -> HashMap<String, PluginI18n> {
    let mut out: HashMap<String, PluginI18n> = HashMap::new();
    let Some(i18n) = value.get("i18n").and_then(|v| v.as_table()) else {
        return out;
    };
    for (lang, body) in i18n {
        let Some(tbl) = body.as_table() else { continue };
        let pi = PluginI18n {
            name: tbl
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            description: tbl
                .get("description")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        };
        if pi.name.is_some() || pi.description.is_some() {
            out.insert(lang.clone(), pi);
        }
    }
    out
}

/// List available plugins in a GitHub registry, enriched with manifest metadata.
///
/// Lists `plugins/` via the GitHub Contents API, then fetches each plugin's
/// `plugin.toml` concurrently to populate `version/description/author/hooks`.
/// Results are cached to disk with the same TTL as the signed index cache
/// to avoid hammering GitHub on every dashboard reload.
pub async fn list_registry_plugins(github_repo: &str) -> Result<Vec<RegistryPluginEntry>, String> {
    validate_github_repo(github_repo)?;

    let ttl = std::env::var("LIBREFANG_REGISTRY_CACHE_TTL_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(default_registry_cache_ttl_secs);
    let skip_cache = std::env::var("LIBREFANG_REGISTRY_NO_CACHE").as_deref() == Ok("1");
    let cache_path = registry_listing_cache_path(github_repo);

    if !skip_cache {
        if let Some(bytes) = load_registry_cache(&cache_path, ttl) {
            if let Ok(cached) = serde_json::from_slice::<Vec<RegistryPluginEntry>>(&bytes) {
                debug!(
                    "Using cached registry listing for {github_repo} ({} plugins)",
                    cached.len()
                );
                return Ok(cached);
            }
        }
    }

    let client = crate::http_client::client_builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let url = format!("https://api.github.com/repos/{github_repo}/contents/plugins");
    let resp = client
        .get(&url)
        .header("Accept", "application/vnd.github.v3+json")
        .send()
        .await
        .map_err(|e| format!("Failed to fetch registry '{github_repo}': {e}"))?;

    if !resp.status().is_success() {
        return Err(format!(
            "Registry '{github_repo}' not accessible (HTTP {})",
            resp.status()
        ));
    }

    let entries: Vec<GitHubContent> = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse registry listing: {e}"))?;

    let names: Vec<String> = entries
        .into_iter()
        .filter(|e| e.content_type == "dir")
        .map(|e| e.name)
        .collect();

    let futs = names
        .iter()
        .map(|n| fetch_registry_plugin_meta(&client, github_repo, n));
    let mut plugins: Vec<RegistryPluginEntry> = futures::future::join_all(futs).await;
    plugins.sort_by(|a, b| a.name.cmp(&b.name));

    if !skip_cache {
        if let Ok(bytes) = serde_json::to_vec(&plugins) {
            save_registry_cache(&cache_path, &bytes);
        }
    }

    Ok(plugins)
}

/// Install from a git URL by cloning.
async fn install_from_git(
    url: &str,
    branch: Option<&str>,
    plugins_dir: &Path,
) -> Result<PluginInfo, String> {
    // Validate URL to prevent argument injection (git interprets `-` prefixed args as flags)
    if url.starts_with('-') {
        return Err("Invalid git URL: must not start with '-'".to_string());
    }
    if !url.starts_with("https://")
        && !url.starts_with("http://")
        && !url.starts_with("git://")
        && !url.starts_with("ssh://")
        && !url.contains('@')
    {
        return Err(
            "Invalid git URL: must start with https://, http://, git://, or ssh://".to_string(),
        );
    }
    if let Some(b) = branch {
        if b.starts_with('-') {
            return Err("Invalid branch name: must not start with '-'".to_string());
        }
    }

    // Clone into a temp dir, validate, then move
    let temp_dir = tempfile::tempdir().map_err(|e| format!("Failed to create temp dir: {e}"))?;

    let mut cmd = tokio::process::Command::new("git");
    cmd.arg("clone").arg("--depth=1");
    if let Some(b) = branch {
        cmd.arg("--branch").arg(b);
    }
    // Use `--` to separate options from positional args
    cmd.arg("--").arg(url).arg(temp_dir.path());

    let output = cmd
        .output()
        .await
        .map_err(|e| format!("Failed to run git clone: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git clone failed: {stderr}"));
    }

    // Validate the cloned repo has a plugin.toml with a safe name.
    // load_plugin_manifest reads files synchronously; run on the blocking pool.
    let manifest_dir = temp_dir.path().to_path_buf();
    let manifest = tokio::task::spawn_blocking(move || load_plugin_manifest(&manifest_dir))
        .await
        .map_err(|e| format!("load_plugin_manifest task failed: {e}"))??;
    validate_plugin_name(&manifest.name)?;
    let target_dir = plugins_dir.join(&manifest.name);

    if target_dir.exists() {
        return Err(format!(
            "Plugin '{}' already installed. Remove it first.",
            manifest.name
        ));
    }

    // Move (rename) from temp to plugins dir.
    // copy_dir_recursive walks/copies a directory tree synchronously; run on the
    // blocking pool so we don't stall the async runtime.
    let copy_src = temp_dir.path().to_path_buf();
    let copy_dst = target_dir.clone();
    tokio::task::spawn_blocking(move || copy_dir_recursive(&copy_src, &copy_dst))
        .await
        .map_err(|e| format!("copy_dir_recursive task failed: {e}"))?
        .map_err(|e| format!("Failed to install plugin: {e}"))?;

    // Remove .git directory to save space
    let git_dir = target_dir.join(".git");
    if git_dir.exists() {
        let _ = tokio::fs::remove_dir_all(&git_dir).await;
    }

    info!(plugin = manifest.name, "Installed plugin from git");
    get_plugin_info(&manifest.name)
}

/// Remove an installed plugin.
pub fn remove_plugin(name: &str) -> Result<(), String> {
    validate_plugin_name(name)?;
    let plugin_dir = plugins_dir().join(name);
    if !plugin_dir.exists() {
        return Err(format!("Plugin '{name}' is not installed"));
    }

    // Validate it's actually a plugin directory (has plugin.toml)
    if !plugin_dir.join("plugin.toml").exists() {
        return Err(format!(
            "Directory {} does not appear to be a plugin (no plugin.toml)",
            plugin_dir.display()
        ));
    }

    std::fs::remove_dir_all(&plugin_dir)
        .map_err(|e| format!("Failed to remove plugin '{name}': {e}"))?;

    info!(plugin = name, "Removed plugin");
    Ok(())
}

/// Create a scaffold for a new plugin. `runtime` defaults to `"python"`;
/// pass `"v"` / `"node"` / `"go"` / `"deno"` / `"native"` to generate a
/// template for that language instead.
pub fn scaffold_plugin(
    name: &str,
    description: &str,
    runtime: Option<&str>,
) -> Result<PathBuf, String> {
    validate_plugin_name(name)?;
    let plugins = ensure_plugins_dir().map_err(|e| format!("Cannot create plugins dir: {e}"))?;
    let plugin_dir = plugins.join(name);

    if plugin_dir.exists() {
        return Err(format!("Plugin '{name}' already exists"));
    }

    let hooks_dir = plugin_dir.join("hooks");
    std::fs::create_dir_all(&hooks_dir)
        .map_err(|e| format!("Failed to create plugin directory: {e}"))?;

    // Normalize the runtime tag via PluginRuntime so aliases (py/js/golang/...)
    // resolve the same way the hook dispatcher will at runtime.
    let runtime_kind = crate::plugin_runtime::PluginRuntime::from_tag(runtime);
    let runtime_tag = runtime_kind.label();

    // Each runtime declares its own hook filenames + template body so the
    // manifest + files stay in sync.
    let files = hook_templates(runtime_kind.clone());
    let (ingest_file, ingest_body) = files.ingest;
    let (after_file, after_body) = files.after_turn;
    let (assemble_file, assemble_body) = files.assemble;
    let (compact_file, compact_body) = files.compact;
    let (bootstrap_file, bootstrap_body) = files.bootstrap;
    let (prepare_file, prepare_body) = files.prepare_subagent;
    let (merge_file, merge_body) = files.merge_subagent;

    // Write plugin.toml as a hand-crafted string so we can include comments
    // that guide users toward the new hook slots.
    let runtime_line = if matches!(runtime_kind, crate::plugin_runtime::PluginRuntime::Python) {
        String::new()
    } else {
        format!("runtime = \"{runtime_tag}\"\n")
    };
    let requirements_line = if matches!(runtime_kind, crate::plugin_runtime::PluginRuntime::Python)
    {
        "requirements = \"requirements.txt\"\n".to_string()
    } else {
        String::new()
    };
    let manifest_toml = format!(
        r#"name = "{name}"
version = "0.1.0"
description = "{description}"
# librefang_min_version = "2026.4.0"   # refuse to load on older daemons
{runtime_line}
# hook_timeout_secs = 30   # per-invocation timeout; bootstrap gets 2× this value
# max_retries       = 0    # retry hook on failure (0 = no retry)
# retry_delay_ms    = 500  # wait between retries
# on_hook_failure   = "warn"   # "warn" | "abort" | "skip"

[hooks]
# --- Active hooks ---
ingest    = "hooks/{ingest_file}"
after_turn = "hooks/{after_file}"

# ingest_filter = "remember"  # only run ingest when message contains this string

# --- Optional hooks (uncomment to activate; template files already written) ---
# bootstrap        = "hooks/{bootstrap_file}"   # runs once at startup (2× timeout)
# assemble         = "hooks/{assemble_file}"    # control what the LLM sees (powerful)
# compact          = "hooks/{compact_file}"     # custom context compression
# prepare_subagent = "hooks/{prepare_file}"     # called before sub-agent spawns
# merge_subagent   = "hooks/{merge_file}"       # called after sub-agent completes

# [env]
# MY_SERVICE_URL = "http://localhost:6333"
# MY_API_KEY     = "${{MY_API_KEY}}"   # expanded from daemon environment at runtime
{requirements_line}"#,
        name = name,
        description = description,
        ingest_file = ingest_file,
        after_file = after_file,
        bootstrap_file = bootstrap_file,
        assemble_file = assemble_file,
        compact_file = compact_file,
        prepare_file = prepare_file,
        merge_file = merge_file,
        runtime_line = runtime_line,
        requirements_line = requirements_line,
    );
    std::fs::write(plugin_dir.join("plugin.toml"), manifest_toml)
        .map_err(|e| format!("Failed to write plugin.toml: {e}"))?;

    let ingest_path = hooks_dir.join(ingest_file);
    let after_path = hooks_dir.join(after_file);
    let assemble_path = hooks_dir.join(assemble_file);
    let compact_path = hooks_dir.join(compact_file);
    let bootstrap_path = hooks_dir.join(bootstrap_file);
    let prepare_path = hooks_dir.join(prepare_file);
    let merge_path = hooks_dir.join(merge_file);
    std::fs::write(&ingest_path, ingest_body)
        .map_err(|e| format!("Failed to write {ingest_file}: {e}"))?;
    std::fs::write(&after_path, after_body)
        .map_err(|e| format!("Failed to write {after_file}: {e}"))?;
    std::fs::write(&assemble_path, assemble_body)
        .map_err(|e| format!("Failed to write {assemble_file}: {e}"))?;
    std::fs::write(&compact_path, compact_body)
        .map_err(|e| format!("Failed to write {compact_file}: {e}"))?;
    std::fs::write(&bootstrap_path, bootstrap_body)
        .map_err(|e| format!("Failed to write {bootstrap_file}: {e}"))?;
    // prepare_subagent and merge_subagent may share the same template body;
    // write them to distinct files so users can customise them independently.
    std::fs::write(&prepare_path, prepare_body)
        .map_err(|e| format!("Failed to write {prepare_file}: {e}"))?;
    std::fs::write(&merge_path, merge_body)
        .map_err(|e| format!("Failed to write {merge_file}: {e}"))?;

    // Native plugins exec the file directly, so the scaffolded shell wrapper
    // needs the executable bit. No-op on Windows (which uses extension-based
    // execution) and on other runtimes (interpreter handles execution).
    if runtime_kind.requires_executable_bit() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for path in [
                &ingest_path,
                &after_path,
                &assemble_path,
                &compact_path,
                &bootstrap_path,
                &prepare_path,
                &merge_path,
            ] {
                if let Ok(meta) = std::fs::metadata(path) {
                    let mut perms = meta.permissions();
                    perms.set_mode(0o755);
                    let _ = std::fs::set_permissions(path, perms);
                }
            }
        }
    }

    // Python plugins get requirements.txt; other runtimes manage deps
    // their own way (go.mod, package.json, v.mod, ...).
    if matches!(runtime_kind, crate::plugin_runtime::PluginRuntime::Python) {
        std::fs::write(
            plugin_dir.join("requirements.txt"),
            "# Python dependencies\n",
        )
        .map_err(|e| format!("Failed to write requirements.txt: {e}"))?;
    }

    info!(
        plugin = name,
        runtime = runtime_tag.as_ref(),
        "Scaffolded new plugin"
    );
    Ok(plugin_dir)
}

/// All hook file names and template bodies for a given runtime.
struct HookFiles {
    /// `(filename, template_body)` pairs for each hook.
    ingest: (&'static str, &'static str),
    after_turn: (&'static str, &'static str),
    assemble: (&'static str, &'static str),
    compact: (&'static str, &'static str),
    /// One-shot startup hook (connect to vector DB, warm cache, etc.)
    bootstrap: (&'static str, &'static str),
    /// Called before a sub-agent spawns.
    prepare_subagent: (&'static str, &'static str),
    /// Called after a sub-agent completes.
    merge_subagent: (&'static str, &'static str),
}

/// Return scaffolded hook filenames + body content for a given runtime.
///
/// Each hook gets a working template showing the stdin/stdout protocol.
/// Python, Node, Go, and Deno get full implementations with token-budget
/// logic; other runtimes get minimal no-op stubs with protocol comments.
fn hook_templates(runtime: crate::plugin_runtime::PluginRuntime) -> HookFiles {
    use crate::plugin_runtime::PluginRuntime as R;
    match runtime {
        R::Python => HookFiles {
            ingest: ("ingest.py", PY_INGEST),
            after_turn: ("after_turn.py", PY_AFTER_TURN),
            assemble: ("assemble.py", PY_ASSEMBLE),
            compact: ("compact.py", PY_COMPACT),
            bootstrap: ("bootstrap.py", PY_BOOTSTRAP),
            prepare_subagent: ("prepare_subagent.py", PY_PREPARE_SUBAGENT),
            merge_subagent: ("merge_subagent.py", PY_MERGE_SUBAGENT),
        },
        R::Node => HookFiles {
            ingest: ("ingest.js", NODE_INGEST),
            after_turn: ("after_turn.js", NODE_AFTER_TURN),
            assemble: ("assemble.js", NODE_ASSEMBLE),
            compact: ("compact.js", NODE_COMPACT),
            bootstrap: ("bootstrap.js", NODE_BOOTSTRAP),
            prepare_subagent: ("prepare_subagent.js", STUB_BOOTSTRAP_NODE),
            merge_subagent: ("merge_subagent.js", STUB_BOOTSTRAP_NODE),
        },
        R::Deno => HookFiles {
            ingest: ("ingest.ts", DENO_INGEST),
            after_turn: ("after_turn.ts", DENO_AFTER_TURN),
            assemble: ("assemble.ts", DENO_ASSEMBLE),
            compact: ("compact.ts", DENO_COMPACT),
            bootstrap: ("bootstrap.ts", DENO_BOOTSTRAP),
            prepare_subagent: ("prepare_subagent.ts", STUB_LIFECYCLE_DENO),
            merge_subagent: ("merge_subagent.ts", STUB_LIFECYCLE_DENO),
        },
        R::Go => HookFiles {
            ingest: ("ingest.go", GO_INGEST),
            after_turn: ("after_turn.go", GO_AFTER_TURN),
            assemble: ("assemble.go", GO_ASSEMBLE),
            compact: ("compact.go", GO_COMPACT),
            bootstrap: ("bootstrap.go", GO_BOOTSTRAP),
            prepare_subagent: ("prepare_subagent.go", STUB_LIFECYCLE_GO),
            merge_subagent: ("merge_subagent.go", STUB_LIFECYCLE_GO),
        },
        R::V => HookFiles {
            ingest: ("ingest.v", V_INGEST),
            after_turn: ("after_turn.v", V_AFTER_TURN),
            assemble: ("assemble.v", STUB_ASSEMBLE_V),
            compact: ("compact.v", STUB_COMPACT_V),
            bootstrap: ("bootstrap.v", STUB_LIFECYCLE_V),
            prepare_subagent: ("prepare_subagent.v", STUB_LIFECYCLE_V),
            merge_subagent: ("merge_subagent.v", STUB_LIFECYCLE_V),
        },
        R::Ruby => HookFiles {
            ingest: ("ingest.rb", RUBY_INGEST),
            after_turn: ("after_turn.rb", RUBY_AFTER_TURN),
            assemble: ("assemble.rb", STUB_ASSEMBLE_RUBY),
            compact: ("compact.rb", STUB_COMPACT_RUBY),
            bootstrap: ("bootstrap.rb", STUB_LIFECYCLE_RUBY),
            prepare_subagent: ("prepare_subagent.rb", STUB_LIFECYCLE_RUBY),
            merge_subagent: ("merge_subagent.rb", STUB_LIFECYCLE_RUBY),
        },
        R::Bash => HookFiles {
            ingest: ("ingest.sh", BASH_INGEST),
            after_turn: ("after_turn.sh", BASH_AFTER_TURN),
            assemble: ("assemble.sh", STUB_ASSEMBLE_BASH),
            compact: ("compact.sh", STUB_COMPACT_BASH),
            bootstrap: ("bootstrap.sh", STUB_LIFECYCLE_BASH),
            prepare_subagent: ("prepare_subagent.sh", STUB_LIFECYCLE_BASH),
            merge_subagent: ("merge_subagent.sh", STUB_LIFECYCLE_BASH),
        },
        R::Bun => HookFiles {
            ingest: ("ingest.ts", BUN_INGEST),
            after_turn: ("after_turn.ts", BUN_AFTER_TURN),
            assemble: ("assemble.ts", STUB_ASSEMBLE_BUN),
            compact: ("compact.ts", STUB_COMPACT_BUN),
            bootstrap: ("bootstrap.ts", STUB_LIFECYCLE_BUN),
            prepare_subagent: ("prepare_subagent.ts", STUB_LIFECYCLE_BUN),
            merge_subagent: ("merge_subagent.ts", STUB_LIFECYCLE_BUN),
        },
        R::Php => HookFiles {
            ingest: ("ingest.php", PHP_INGEST),
            after_turn: ("after_turn.php", PHP_AFTER_TURN),
            assemble: ("assemble.php", STUB_ASSEMBLE_PHP),
            compact: ("compact.php", STUB_COMPACT_PHP),
            bootstrap: ("bootstrap.php", STUB_LIFECYCLE_PHP),
            prepare_subagent: ("prepare_subagent.php", STUB_LIFECYCLE_PHP),
            merge_subagent: ("merge_subagent.php", STUB_LIFECYCLE_PHP),
        },
        R::Lua => HookFiles {
            ingest: ("ingest.lua", LUA_INGEST),
            after_turn: ("after_turn.lua", LUA_AFTER_TURN),
            assemble: ("assemble.lua", STUB_ASSEMBLE_LUA),
            compact: ("compact.lua", STUB_COMPACT_LUA),
            bootstrap: ("bootstrap.lua", STUB_LIFECYCLE_LUA),
            prepare_subagent: ("prepare_subagent.lua", STUB_LIFECYCLE_LUA),
            merge_subagent: ("merge_subagent.lua", STUB_LIFECYCLE_LUA),
        },
        R::Native => HookFiles {
            // Shell wrapper — users replace with a real pre-compiled binary.
            ingest: ("ingest", NATIVE_INGEST),
            after_turn: ("after_turn", NATIVE_AFTER_TURN),
            assemble: ("assemble", STUB_ASSEMBLE_NATIVE),
            compact: ("compact", STUB_COMPACT_NATIVE),
            bootstrap: ("bootstrap", STUB_LIFECYCLE_NATIVE),
            prepare_subagent: ("prepare_subagent", STUB_LIFECYCLE_NATIVE),
            merge_subagent: ("merge_subagent", STUB_LIFECYCLE_NATIVE),
        },
        R::Wasm => HookFiles {
            // Wasm hooks run inline via wasmtime — no template files needed.
            // Scaffold stubs so the directory structure is consistent.
            ingest: ("ingest.wasm", NATIVE_INGEST),
            after_turn: ("after_turn.wasm", NATIVE_AFTER_TURN),
            assemble: ("assemble.wasm", STUB_ASSEMBLE_NATIVE),
            compact: ("compact.wasm", STUB_COMPACT_NATIVE),
            bootstrap: ("bootstrap.wasm", STUB_LIFECYCLE_NATIVE),
            prepare_subagent: ("prepare_subagent.wasm", STUB_LIFECYCLE_NATIVE),
            merge_subagent: ("merge_subagent.wasm", STUB_LIFECYCLE_NATIVE),
        },
        // Custom launchers: fall back to the native (shell-wrapper) templates.
        // Users will replace these with scripts suitable for their launcher.
        R::Custom(_) => HookFiles {
            ingest: ("ingest", NATIVE_INGEST),
            after_turn: ("after_turn", NATIVE_AFTER_TURN),
            assemble: ("assemble", STUB_ASSEMBLE_NATIVE),
            compact: ("compact", STUB_COMPACT_NATIVE),
            bootstrap: ("bootstrap", STUB_LIFECYCLE_NATIVE),
            prepare_subagent: ("prepare_subagent", STUB_LIFECYCLE_NATIVE),
            merge_subagent: ("merge_subagent", STUB_LIFECYCLE_NATIVE),
        },
    }
}

// --- Python templates (the original, kept verbatim for backwards compat) ---

const PY_INGEST: &str = r#"#!/usr/bin/env python3
"""Context engine ingest hook.

Receives via stdin:
    {
      "type": "ingest",
      "agent_id": "...",
      "message": "user message text",
      "peer_id": "platform-user-id-or-null"
    }

Should print to stdout:
    {"type": "ingest_result", "memories": [{"content": "recalled fact"}]}

Tip: scope your recall to peer_id when present to prevent cross-user leaks.
"""
import json
import sys

def main():
    request = json.loads(sys.stdin.read())
    agent_id = request["agent_id"]
    message = request["message"]
    peer_id = request.get("peer_id")  # None when called directly via API

    # TODO: Implement your custom recall logic here.
    # Example: query a vector database, search a knowledge base, etc.
    memories = []

    print(json.dumps({"type": "ingest_result", "memories": memories}))

if __name__ == "__main__":
    main()
"#;

const PY_AFTER_TURN: &str = r#"#!/usr/bin/env python3
"""Context engine after_turn hook.

Receives via stdin:
    {
      "type": "after_turn",
      "agent_id": "...",
      "messages": [{"role": "user"|"assistant", "content": "...", "pinned": false}, ...]
    }

Note: message content is truncated to 500 chars per message for performance.

Should print to stdout:
    {"type": "ok"}
"""
import json
import sys

def main():
    request = json.loads(sys.stdin.read())
    agent_id = request["agent_id"]
    messages = request["messages"]

    # TODO: Implement your post-turn logic here.
    # Example: update indexes, persist state, log analytics, etc.

    print(json.dumps({"type": "ok"}))

if __name__ == "__main__":
    main()
"#;

const PY_ASSEMBLE: &str = r#"#!/usr/bin/env python3
"""Context engine assemble hook — controls what the LLM sees.

This is the most powerful hook. Called before every LLM request.

Receives via stdin:
    {
      "type": "assemble",
      "system_prompt": "...",
      "messages": [
        {"role": "user"|"assistant"|"tool", "content": <text or blocks>, "pinned": false},
        ...
      ],
      "context_window_tokens": 200000
    }

Messages use the full LibreFang message format — content can be a plain string
or a list of blocks (text, tool_use, tool_result, image, thinking).

Should print to stdout:
    {"type": "assemble_result", "messages": [...]}

Return a trimmed/reordered subset of messages that fits the token budget.
If you return an empty list or fail, LibreFang falls back to its default
overflow recovery (trim oldest, then compact).
"""
import json
import sys

def estimate_tokens(text: str) -> int:
    """Rough token estimate: ~4 chars per token."""
    return max(1, len(text) // 4)

def message_text(msg: dict) -> str:
    content = msg.get("content", "")
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return " ".join(
            b.get("text", b.get("content", ""))
            for b in content
            if isinstance(b, dict)
        )
    return ""

def main():
    request = json.loads(sys.stdin.read())
    messages = request["messages"]
    context_window_tokens = request["context_window_tokens"]

    # Reserve tokens for system prompt and response headroom
    budget = context_window_tokens - 4000

    # Keep messages newest-first until we exceed the budget, then stop
    kept = []
    used = 0
    for msg in reversed(messages):
        tokens = estimate_tokens(message_text(msg))
        if used + tokens > budget:
            break
        kept.append(msg)
        used += tokens

    kept.reverse()
    print(json.dumps({"type": "assemble_result", "messages": kept}))

if __name__ == "__main__":
    main()
"#;

const PY_COMPACT: &str = r#"#!/usr/bin/env python3
"""Context engine compact hook — custom context compression.

Called when the context window is under pressure.

Receives via stdin:
    {
      "type": "compact",
      "agent_id": "...",
      "messages": [...],   # full message list (same format as assemble)
      "model": "llama-3.3-70b-versatile",
      "context_window_tokens": 200000
    }

Should print to stdout:
    {"type": "compact_result", "messages": [...]}

Return a compacted version of the message list. If you fail or return
an empty list, LibreFang falls back to its built-in LLM-based compaction.
"""
import json
import sys

def message_text(msg: dict) -> str:
    content = msg.get("content", "")
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return " ".join(
            b.get("text", b.get("content", ""))
            for b in content
            if isinstance(b, dict)
        )
    return ""

def main():
    request = json.loads(sys.stdin.read())
    messages = request["messages"]

    # Simple strategy: keep the first (system/context) message and the last 10
    pinned = [m for m in messages if m.get("pinned")]
    rest = [m for m in messages if not m.get("pinned")]

    summary_text = "... (older messages summarized) ..."
    summary_msg = {"role": "assistant", "content": summary_text, "pinned": False}

    if len(rest) > 10:
        compacted = pinned + [summary_msg] + rest[-10:]
    else:
        compacted = pinned + rest

    print(json.dumps({"type": "compact_result", "messages": compacted}))

if __name__ == "__main__":
    main()
"#;

// --- Python lifecycle hooks (bootstrap / prepare_subagent / merge_subagent) ---

const PY_BOOTSTRAP: &str = r#"#!/usr/bin/env python3
"""Context engine bootstrap hook — runs ONCE when the engine initialises.

Use this to connect to external services (vector databases, caches, HTTP APIs)
and warm any state your other hooks will read at runtime.

Receives via stdin:
    {
      "type": "bootstrap",
      "context_window_tokens": 200000,
      "stable_prefix_mode": false,
      "max_recall_results": 10
    }

Should print to stdout:
    {"type": "ok"}

Failures here are non-fatal — the engine continues without your bootstrap work,
but the missing connection may cause later hooks to fail silently.

Note: bootstrap gets DOUBLE the configured hook_timeout_secs.
"""
import json
import sys

def main():
    request = json.loads(sys.stdin.read())
    context_window_tokens = request.get("context_window_tokens", 200000)
    stable_prefix_mode = request.get("stable_prefix_mode", False)

    # TODO: Connect to your data store here.
    # Example: initialise a SQLite connection, ping a vector DB, etc.
    #
    # import sqlite3
    # db = sqlite3.connect(os.path.expanduser("~/.librefang/my-plugin.db"))
    # db.execute("CREATE TABLE IF NOT EXISTS memories (...)")
    # db.commit()
    # db.close()
    #
    # Any errors raised here are caught and logged as warnings.

    print(json.dumps({"type": "ok"}))

if __name__ == "__main__":
    main()
"#;

const PY_PREPARE_SUBAGENT: &str = r#"#!/usr/bin/env python3
"""Context engine prepare_subagent hook.

Called just before a sub-agent is spawned. Use this to isolate memory scope,
snapshot parent state, or set up any resources the child agent needs.

Receives via stdin:
    {
      "type": "prepare_subagent",
      "parent_id": "uuid-of-parent-agent",
      "child_id":  "uuid-of-child-agent"
    }

Should print to stdout:
    {"type": "ok"}

Non-fatal: failures are logged as warnings and the sub-agent still spawns.
"""
import json
import sys

def main():
    request = json.loads(sys.stdin.read())
    parent_id = request["parent_id"]
    child_id = request["child_id"]

    # TODO: Snapshot or fork per-agent state here.
    # Example: copy parent memories to child scope in your data store.

    print(json.dumps({"type": "ok"}))

if __name__ == "__main__":
    main()
"#;

const PY_MERGE_SUBAGENT: &str = r#"#!/usr/bin/env python3
"""Context engine merge_subagent hook.

Called after a sub-agent completes. Use this to merge the child agent's
findings or memories back into the parent context.

Receives via stdin:
    {
      "type": "merge_subagent",
      "parent_id": "uuid-of-parent-agent",
      "child_id":  "uuid-of-child-agent"
    }

Should print to stdout:
    {"type": "ok"}

Non-fatal: failures are logged as warnings; the parent agent continues normally.
"""
import json
import sys

def main():
    request = json.loads(sys.stdin.read())
    parent_id = request["parent_id"]
    child_id = request["child_id"]

    # TODO: Merge child agent state into the parent here.
    # Example: copy child memories back to parent scope in your data store.

    print(json.dumps({"type": "ok"}))

if __name__ == "__main__":
    main()
"#;

// --- Node templates (assemble + compact) ---

const NODE_ASSEMBLE: &str = r#"#!/usr/bin/env node
// Context engine assemble hook (Node.js).
// Controls what the LLM sees — called before every LLM request.
//
// Receives on stdin:
//   {
//     "type": "assemble",
//     "system_prompt": "...",
//     "messages": [{"role":"user"|"assistant", "content": ..., "pinned": false}, ...],
//     "context_window_tokens": 200000
//   }
// content can be a plain string or an array of blocks (tool_use, tool_result, image, thinking).
//
// Emits on stdout:
//   {"type": "assemble_result", "messages": [...]}
//
// Return an empty list or fail to trigger fallback to LibreFang's default trimming.

"use strict";

function estimateTokens(msg) {
  const text = typeof msg.content === "string"
    ? msg.content
    : (Array.isArray(msg.content)
        ? msg.content.map(b => b.text || b.content || "").join(" ")
        : "");
  return Math.max(1, Math.ceil(text.length / 4));
}

let buf = "";
process.stdin.on("data", chunk => { buf += chunk.toString("utf8"); });
process.stdin.on("end", () => {
  const req = JSON.parse(buf);
  const messages = req.messages;
  const budget = req.context_window_tokens - 4000; // headroom for system + response

  // Keep newest messages that fit within the token budget.
  let used = 0;
  const kept = [];
  for (let i = messages.length - 1; i >= 0; i--) {
    const tokens = estimateTokens(messages[i]);
    if (used + tokens > budget) break;
    kept.unshift(messages[i]);
    used += tokens;
  }

  process.stdout.write(JSON.stringify({ type: "assemble_result", messages: kept }) + "\n");
});
"#;

const NODE_COMPACT: &str = r#"#!/usr/bin/env node
// Context engine compact hook (Node.js).
// Custom context compression — called under context pressure.
//
// Receives on stdin:
//   {
//     "type": "compact",
//     "agent_id": "...",
//     "messages": [...],
//     "model": "...",
//     "context_window_tokens": 200000
//   }
//
// Emits on stdout:
//   {"type": "compact_result", "messages": [...]}
//
// Return an empty list or fail to trigger fallback to LLM-based compaction.

"use strict";

let buf = "";
process.stdin.on("data", chunk => { buf += chunk.toString("utf8"); });
process.stdin.on("end", () => {
  const req = JSON.parse(buf);
  const messages = req.messages;

  const pinned = messages.filter(m => m.pinned);
  const rest   = messages.filter(m => !m.pinned);

  // Keep last 10 non-pinned messages; summarise the rest with a placeholder.
  let compacted;
  if (rest.length > 10) {
    const summary = { role: "assistant", content: "... (older messages summarised) ...", pinned: false };
    compacted = [...pinned, summary, ...rest.slice(-10)];
  } else {
    compacted = [...pinned, ...rest];
  }

  process.stdout.write(JSON.stringify({ type: "compact_result", messages: compacted }) + "\n");
});
"#;

// --- Deno / TypeScript templates (assemble + compact) ---

const DENO_ASSEMBLE: &str = r#"// Context engine assemble hook (Deno / TypeScript).
// Controls what the LLM sees — called before every LLM request.
//
// Run via: deno run --allow-read assemble.ts

type ContentBlock = { type: string; text?: string; content?: string; [k: string]: unknown };
type Message = { role: string; content: string | ContentBlock[]; pinned: boolean };

function estimateTokens(msg: Message): number {
  const text = typeof msg.content === "string"
    ? msg.content
    : msg.content.map((b: ContentBlock) => b.text ?? b.content ?? "").join(" ");
  return Math.max(1, Math.ceil(text.length / 4));
}

const raw = new TextDecoder().decode(await Deno.readAll(Deno.stdin));
const req = JSON.parse(raw) as { type: string; messages: Message[]; context_window_tokens: number };
const budget = req.context_window_tokens - 4000;

let used = 0;
const kept: Message[] = [];
for (let i = req.messages.length - 1; i >= 0; i--) {
  const tokens = estimateTokens(req.messages[i]);
  if (used + tokens > budget) break;
  kept.unshift(req.messages[i]);
  used += tokens;
}

console.log(JSON.stringify({ type: "assemble_result", messages: kept }));
"#;

const DENO_COMPACT: &str = r#"// Context engine compact hook (Deno / TypeScript).
// Custom context compression — called under context pressure.
//
// Run via: deno run --allow-read compact.ts

type Message = { role: string; content: unknown; pinned: boolean };

const raw = new TextDecoder().decode(await Deno.readAll(Deno.stdin));
const req = JSON.parse(raw) as { type: string; messages: Message[] };
const messages = req.messages;

const pinned = messages.filter((m: Message) => m.pinned);
const rest   = messages.filter((m: Message) => !m.pinned);

const summary: Message = { role: "assistant", content: "... (older messages summarised) ...", pinned: false };
const compacted = rest.length > 10
  ? [...pinned, summary, ...rest.slice(-10)]
  : [...pinned, ...rest];

console.log(JSON.stringify({ type: "compact_result", messages: compacted }));
"#;

// --- Go templates (assemble + compact) ---

const GO_ASSEMBLE: &str = r#"// Context engine assemble hook (Go).
// Controls what the LLM sees — called before every LLM request.
//
// Run with: go run assemble.go
package main

import (
	"encoding/json"
	"io"
	"os"
)

type Message struct {
	Role    string `json:"role"`
	Content any    `json:"content"`
	Pinned  bool   `json:"pinned"`
}

type AssembleRequest struct {
	Type                string    `json:"type"`
	SystemPrompt        string    `json:"system_prompt"`
	Messages            []Message `json:"messages"`
	ContextWindowTokens int       `json:"context_window_tokens"`
}

type AssembleResult struct {
	Type     string    `json:"type"`
	Messages []Message `json:"messages"`
}

func estimateTokens(m Message) int {
	text := ""
	switch v := m.Content.(type) {
	case string:
		text = v
	}
	tokens := len(text) / 4
	if tokens < 1 {
		tokens = 1
	}
	return tokens
}

func main() {
	raw, err := io.ReadAll(os.Stdin)
	if err != nil {
		os.Exit(1)
	}
	var req AssembleRequest
	if err := json.Unmarshal(raw, &req); err != nil {
		os.Exit(1)
	}

	budget := req.ContextWindowTokens - 4000
	used := 0
	kept := []Message{}
	for i := len(req.Messages) - 1; i >= 0; i-- {
		tokens := estimateTokens(req.Messages[i])
		if used+tokens > budget {
			break
		}
		kept = append([]Message{req.Messages[i]}, kept...)
		used += tokens
	}

	out, _ := json.Marshal(AssembleResult{Type: "assemble_result", Messages: kept})
	os.Stdout.Write(out)
	os.Stdout.Write([]byte("\n"))
}
"#;

const GO_COMPACT: &str = r#"// Context engine compact hook (Go).
// Custom context compression — called under context pressure.
//
// Run with: go run compact.go
package main

import (
	"encoding/json"
	"io"
	"os"
)

type Message struct {
	Role    string `json:"role"`
	Content any    `json:"content"`
	Pinned  bool   `json:"pinned"`
}

type CompactRequest struct {
	Type                string    `json:"type"`
	AgentID             string    `json:"agent_id"`
	Messages            []Message `json:"messages"`
	Model               string    `json:"model"`
	ContextWindowTokens int       `json:"context_window_tokens"`
}

type CompactResult struct {
	Type     string    `json:"type"`
	Messages []Message `json:"messages"`
}

func main() {
	raw, err := io.ReadAll(os.Stdin)
	if err != nil {
		os.Exit(1)
	}
	var req CompactRequest
	if err := json.Unmarshal(raw, &req); err != nil {
		os.Exit(1)
	}

	var pinned, rest []Message
	for _, m := range req.Messages {
		if m.Pinned {
			pinned = append(pinned, m)
		} else {
			rest = append(rest, m)
		}
	}

	compacted := append(pinned, rest...)
	if len(rest) > 10 {
		summary := Message{
			Role:    "assistant",
			Content: "... (older messages summarised) ...",
			Pinned:  false,
		}
		compacted = append(pinned, summary)
		compacted = append(compacted, rest[len(rest)-10:]...)
	}

	out, _ := json.Marshal(CompactResult{Type: "compact_result", Messages: compacted})
	os.Stdout.Write(out)
	os.Stdout.Write([]byte("\n"))
}
"#;

// --- Node / Deno / Go bootstrap templates ---

const NODE_BOOTSTRAP: &str = r#"#!/usr/bin/env node
// Context engine bootstrap hook (Node.js).
// Runs ONCE at engine startup — connect to external services here.
// Receives: { type, context_window_tokens, stable_prefix_mode, max_recall_results }
// Returns:  { type: "ok" }
'use strict';
const { stdin } = process;
let raw = '';
stdin.setEncoding('utf8');
stdin.on('data', chunk => { raw += chunk; });
stdin.on('end', () => {
  // const req = JSON.parse(raw);
  // TODO: initialise your data store, warm caches, etc.
  process.stdout.write(JSON.stringify({ type: 'ok' }) + '\n');
});
"#;

const DENO_BOOTSTRAP: &str = r#"// Context engine bootstrap hook (Deno / TypeScript).
// Runs ONCE at engine startup — connect to external services here.
// Receives: { type, context_window_tokens, stable_prefix_mode, max_recall_results }
// Returns:  { type: "ok" }
const raw = new TextDecoder().decode(await Deno.readAll(Deno.stdin));
// const req = JSON.parse(raw);
// TODO: initialise your data store, warm caches, etc.
console.log(JSON.stringify({ type: 'ok' }));
"#;

const GO_BOOTSTRAP: &str = r#"// Context engine bootstrap hook (Go).
// Runs ONCE at engine startup — connect to external services here.
// go run bootstrap.go
package main

import (
	"encoding/json"
	"fmt"
	"os"
)

type BootstrapRequest struct {
	Type               string `json:"type"`
	ContextWindowTokens int   `json:"context_window_tokens"`
	StablePrefixMode   bool   `json:"stable_prefix_mode"`
	MaxRecallResults   int    `json:"max_recall_results"`
}

func main() {
	var req BootstrapRequest
	if err := json.NewDecoder(os.Stdin).Decode(&req); err != nil {
		fmt.Fprintln(os.Stderr, "bootstrap: invalid JSON on stdin:", err)
		os.Exit(1)
	}

	// TODO: connect to your database, warm caches, etc.

	fmt.Println(`{"type":"ok"}`)
}
"#;

// --- Minimal lifecycle stubs for other runtimes ---
// bootstrap / prepare_subagent / merge_subagent all use the same "ok" response.
// These stubs print {"type":"ok"} and exit — sufficient to acknowledge the hook.

const STUB_BOOTSTRAP_NODE: &str = r#"#!/usr/bin/env node
// Lifecycle hook stub (Node.js) — bootstrap / prepare_subagent / merge_subagent.
// Replace body with your logic; response must be {"type":"ok"}.
'use strict';
let raw = '';
process.stdin.setEncoding('utf8');
process.stdin.on('data', c => { raw += c; });
process.stdin.on('end', () => {
  // const req = JSON.parse(raw);
  process.stdout.write(JSON.stringify({ type: 'ok' }) + '\n');
});
"#;

const STUB_LIFECYCLE_DENO: &str = r#"// Lifecycle hook stub (Deno / TypeScript).
// bootstrap / prepare_subagent / merge_subagent — all return {"type":"ok"}.
await Deno.readAll(Deno.stdin); // consume stdin
console.log(JSON.stringify({ type: 'ok' }));
"#;

const STUB_LIFECYCLE_GO: &str = r#"// Lifecycle hook stub (Go).
// bootstrap / prepare_subagent / merge_subagent — all return {"type":"ok"}.
// go run <hook>.go
package main

import (
	"fmt"
	"io"
	"os"
)

func main() {
	io.ReadAll(os.Stdin) // consume stdin
	fmt.Println(`{"type":"ok"}`)
}
"#;

const STUB_LIFECYCLE_V: &str = r#"// Lifecycle hook stub (V).
// bootstrap / prepare_subagent / merge_subagent — all return {"type":"ok"}.
import os

fn main() {
    os.get_raw_stdin()  // consume stdin
    println('{"type":"ok"}')
}
"#;

const STUB_LIFECYCLE_RUBY: &str = r#"# Lifecycle hook stub (Ruby).
# bootstrap / prepare_subagent / merge_subagent — all return {"type":"ok"}.
require 'json'
$stdin.read  # consume stdin
puts JSON.generate({ type: 'ok' })
"#;

const STUB_LIFECYCLE_BASH: &str = r#"#!/usr/bin/env bash
# Lifecycle hook stub (Bash).
# bootstrap / prepare_subagent / merge_subagent — all return {"type":"ok"}.
cat /dev/stdin > /dev/null   # consume stdin
printf '{"type":"ok"}\n'
"#;

const STUB_LIFECYCLE_BUN: &str = r#"// Lifecycle hook stub (Bun / TypeScript).
// bootstrap / prepare_subagent / merge_subagent — all return {"type":"ok"}.
await Bun.stdin.text(); // consume stdin
console.log(JSON.stringify({ type: 'ok' }));
"#;

const STUB_LIFECYCLE_PHP: &str = r#"<?php
// Lifecycle hook stub (PHP).
// bootstrap / prepare_subagent / merge_subagent — all return {"type":"ok"}.
file_get_contents('php://stdin'); // consume stdin
echo json_encode(['type' => 'ok']) . "\n";
"#;

const STUB_LIFECYCLE_LUA: &str = r#"-- Lifecycle hook stub (Lua).
-- bootstrap / prepare_subagent / merge_subagent — all return {"type":"ok"}.
io.read("*a")  -- consume stdin
print('{"type":"ok"}')
"#;

const STUB_LIFECYCLE_NATIVE: &str = r#"#!/bin/sh
# Lifecycle hook stub (native/shell wrapper).
# bootstrap / prepare_subagent / merge_subagent — all return {"type":"ok"}.
cat > /dev/null  # consume stdin
printf '{"type":"ok"}\n'
"#;

// --- Minimal stubs for other runtimes (assemble + compact) ---
// These fall back gracefully — returning an empty messages list causes
// LibreFang to use its default overflow recovery / LLM compaction.

const STUB_ASSEMBLE_V: &str = r#"// Context engine assemble hook stub (V).
// See docs/agent/plugins for the full protocol.
// Returning empty messages triggers LibreFang's default context trimming.
module main
import os
import json

fn main() {
    _ := os.get_raw_stdin().bytestr()
    // TODO: implement assemble logic or delete this file to use default trimming.
    println(json.encode({ 'type': 'assemble_result', 'messages': [] }))
}
"#;

const STUB_COMPACT_V: &str = r#"// Context engine compact hook stub (V).
module main
import os
import json

fn main() {
    _ := os.get_raw_stdin().bytestr()
    // TODO: implement compact logic or delete this file to use LLM compaction.
    println(json.encode({ 'type': 'compact_result', 'messages': [] }))
}
"#;

const STUB_ASSEMBLE_RUBY: &str = r#"# Context engine assemble hook stub (Ruby).
# See docs/agent/plugins for the full protocol.
require "json"
_req = JSON.parse($stdin.read)
# TODO: implement assemble logic, or delete this file to use default trimming.
puts JSON.generate({ "type" => "assemble_result", "messages" => [] })
"#;

const STUB_COMPACT_RUBY: &str = r#"# Context engine compact hook stub (Ruby).
require "json"
_req = JSON.parse($stdin.read)
# TODO: implement compact logic, or delete this file to use LLM compaction.
puts JSON.generate({ "type" => "compact_result", "messages" => [] })
"#;

const STUB_ASSEMBLE_BASH: &str = r#"#!/usr/bin/env bash
# Context engine assemble hook stub (Bash).
# See docs/agent/plugins for the full protocol.
# For non-trivial logic, pipe stdin through `jq` or call a helper binary.
set -euo pipefail
_input=$(cat)
# TODO: implement assemble logic, or delete this file to use default trimming.
printf '{"type":"assemble_result","messages":[]}\n'
"#;

const STUB_COMPACT_BASH: &str = r#"#!/usr/bin/env bash
# Context engine compact hook stub (Bash).
set -euo pipefail
_input=$(cat)
# TODO: implement compact logic, or delete this file to use LLM compaction.
printf '{"type":"compact_result","messages":[]}\n'
"#;

const STUB_ASSEMBLE_BUN: &str = r#"// Context engine assemble hook stub (Bun / TypeScript).
// See docs/agent/plugins for the full protocol.
const _req = JSON.parse(await Bun.stdin.text());
// TODO: implement assemble logic, or delete this file to use default trimming.
console.log(JSON.stringify({ type: "assemble_result", messages: [] }));
"#;

const STUB_COMPACT_BUN: &str = r#"// Context engine compact hook stub (Bun / TypeScript).
const _req = JSON.parse(await Bun.stdin.text());
// TODO: implement compact logic, or delete this file to use LLM compaction.
console.log(JSON.stringify({ type: "compact_result", messages: [] }));
"#;

const STUB_ASSEMBLE_PHP: &str = r#"<?php
// Context engine assemble hook stub (PHP).
// See docs/agent/plugins for the full protocol.
$_req = json_decode(file_get_contents('php://stdin'), true);
// TODO: implement assemble logic, or delete this file to use default trimming.
echo json_encode(['type' => 'assemble_result', 'messages' => []]) . "\n";
"#;

const STUB_COMPACT_PHP: &str = r#"<?php
// Context engine compact hook stub (PHP).
$_req = json_decode(file_get_contents('php://stdin'), true);
// TODO: implement compact logic, or delete this file to use LLM compaction.
echo json_encode(['type' => 'compact_result', 'messages' => []]) . "\n";
"#;

const STUB_ASSEMBLE_LUA: &str = r#"-- Context engine assemble hook stub (Lua).
-- See docs/agent/plugins for the full protocol.
local json = require("json")  -- install lua-cjson or dkjson
local _req = json.decode(io.read("*a"))
-- TODO: implement assemble logic, or delete this file to use default trimming.
print(json.encode({ type = "assemble_result", messages = {} }))
"#;

const STUB_COMPACT_LUA: &str = r#"-- Context engine compact hook stub (Lua).
local json = require("json")
local _req = json.decode(io.read("*a"))
-- TODO: implement compact logic, or delete this file to use LLM compaction.
print(json.encode({ type = "compact_result", messages = {} }))
"#;

const STUB_ASSEMBLE_NATIVE: &str = r#"#!/bin/sh
# Context engine assemble hook stub (native shell wrapper).
# Replace this script with a pre-compiled binary that speaks the JSON protocol.
# Returning empty messages triggers LibreFang's default context trimming.
read -r _input
printf '{"type":"assemble_result","messages":[]}\n'
"#;

const STUB_COMPACT_NATIVE: &str = r#"#!/bin/sh
# Context engine compact hook stub (native shell wrapper).
# Replace with a pre-compiled binary that speaks the JSON protocol.
read -r _input
printf '{"type":"compact_result","messages":[]}\n'
"#;

// --- V language templates ---

const V_INGEST: &str = r#"// Context engine ingest hook (V).
//
// Receives on stdin:
//   {"type": "ingest", "agent_id": "...", "message": "user message text"}
// Emits on stdout:
//   {"type": "ingest_result", "memories": [{"content": "recalled fact"}]}
//
// Run with: `v run ingest.v` (or pre-compile: `v ingest.v`)
module main

import os
import json

struct IngestRequest {
	@type     string @[json: 'type']
	agent_id  string
	message   string
}

struct Memory {
	content string
}

struct IngestResult {
	@type    string   @[json: 'type']
	memories []Memory
}

fn main() {
	input := os.get_raw_stdin().bytestr()
	req := json.decode(IngestRequest, input) or {
		eprintln('ingest: invalid JSON on stdin: ${err}')
		exit(1)
	}
	_ := req.agent_id
	_ := req.message

	// TODO: Implement your custom recall logic here.
	result := IngestResult{
		@type: 'ingest_result'
		memories: []
	}
	println(json.encode(result))
}
"#;

const V_AFTER_TURN: &str = r#"// Context engine after_turn hook (V).
//
// Receives on stdin:
//   {"type": "after_turn", "agent_id": "...", "messages": [...]}
// Emits on stdout:
//   {"type": "ok"}
module main

import os
import json

struct AfterTurnRequest {
	@type    string @[json: 'type']
	agent_id string
}

struct Ok {
	@type string @[json: 'type']
}

fn main() {
	input := os.get_raw_stdin().bytestr()
	_ := json.decode(AfterTurnRequest, input) or {
		eprintln('after_turn: invalid JSON on stdin: ${err}')
		exit(1)
	}

	// TODO: persist state, update indexes, log analytics, ...

	println(json.encode(Ok{ @type: 'ok' }))
}
"#;

// --- Node templates ---

const NODE_INGEST: &str = r#"#!/usr/bin/env node
// Context engine ingest hook (Node.js).
//
// Receives on stdin:
//   {"type": "ingest", "agent_id": "...", "message": "user message text"}
// Emits on stdout:
//   {"type": "ingest_result", "memories": [{"content": "recalled fact"}]}

"use strict";

let buf = "";
process.stdin.on("data", (chunk) => { buf += chunk.toString("utf8"); });
process.stdin.on("end", () => {
  const req = JSON.parse(buf);
  const agentId = req.agent_id;
  const message = req.message;

  // TODO: Implement your custom recall logic here.
  const memories = [];

  process.stdout.write(JSON.stringify({ type: "ingest_result", memories }) + "\n");
});
"#;

const NODE_AFTER_TURN: &str = r#"#!/usr/bin/env node
// Context engine after_turn hook (Node.js).

"use strict";

let buf = "";
process.stdin.on("data", (chunk) => { buf += chunk.toString("utf8"); });
process.stdin.on("end", () => {
  const req = JSON.parse(buf);
  const _agentId = req.agent_id;
  const _messages = req.messages;

  // TODO: persist state, update indexes, log analytics, ...

  process.stdout.write(JSON.stringify({ type: "ok" }) + "\n");
});
"#;

// --- Deno / TypeScript templates ---

const DENO_INGEST: &str = r#"// Context engine ingest hook (Deno / TypeScript).
//
// Run via `deno run --allow-read ingest.ts`.

interface IngestRequest { type: "ingest"; agent_id: string; message: string; }
interface Memory { content: string; }
interface IngestResult { type: "ingest_result"; memories: Memory[]; }

const raw = new TextDecoder().decode(await Deno.readAll(Deno.stdin));
const req = JSON.parse(raw) as IngestRequest;
void req.agent_id; void req.message;

// TODO: Implement your custom recall logic here.
const result: IngestResult = { type: "ingest_result", memories: [] };
console.log(JSON.stringify(result));
"#;

const DENO_AFTER_TURN: &str = r#"// Context engine after_turn hook (Deno / TypeScript).

const raw = new TextDecoder().decode(await Deno.readAll(Deno.stdin));
void JSON.parse(raw);

// TODO: persist state, update indexes, log analytics, ...

console.log(JSON.stringify({ type: "ok" }));
"#;

// --- Go templates ---

const GO_INGEST: &str = r#"// Context engine ingest hook (Go).
//
// Run with: `go run ingest.go`
package main

import (
	"encoding/json"
	"io"
	"os"
)

type IngestRequest struct {
	Type    string `json:"type"`
	AgentID string `json:"agent_id"`
	Message string `json:"message"`
}

type Memory struct {
	Content string `json:"content"`
}

type IngestResult struct {
	Type     string   `json:"type"`
	Memories []Memory `json:"memories"`
}

func main() {
	raw, err := io.ReadAll(os.Stdin)
	if err != nil {
		os.Exit(1)
	}
	var req IngestRequest
	if err := json.Unmarshal(raw, &req); err != nil {
		os.Exit(1)
	}
	_ = req.AgentID
	_ = req.Message

	// TODO: Implement your custom recall logic here.
	out, _ := json.Marshal(IngestResult{Type: "ingest_result", Memories: []Memory{}})
	os.Stdout.Write(out)
	os.Stdout.Write([]byte("\n"))
}
"#;

const GO_AFTER_TURN: &str = r#"// Context engine after_turn hook (Go).
package main

import (
	"encoding/json"
	"io"
	"os"
)

func main() {
	raw, err := io.ReadAll(os.Stdin)
	if err != nil {
		os.Exit(1)
	}
	var req map[string]any
	_ = json.Unmarshal(raw, &req)

	// TODO: persist state, update indexes, log analytics, ...

	out, _ := json.Marshal(map[string]string{"type": "ok"})
	os.Stdout.Write(out)
	os.Stdout.Write([]byte("\n"))
}
"#;

// --- Native (bring-your-own-binary) templates ---

const NATIVE_INGEST: &str = r#"#!/bin/sh
# Native plugin ingest hook.
#
# Replace this shell wrapper with your own pre-compiled binary
# (V / Rust / Go / Zig / C++ — anything that speaks the JSON
# stdin/stdout protocol).
#
# Receives on stdin:
#   {"type": "ingest", "agent_id": "...", "message": "..."}
# Emits on stdout:
#   {"type": "ingest_result", "memories": [...]}
#
# chmod +x hooks/ingest to make this executable.

read -r _input
printf '{"type":"ingest_result","memories":[]}\n'
"#;

const NATIVE_AFTER_TURN: &str = r#"#!/bin/sh
# Native plugin after_turn hook — replace with your binary.
read -r _input
printf '{"type":"ok"}\n'
"#;

// --- Ruby templates ---

const RUBY_INGEST: &str = r#"# Context engine ingest hook (Ruby).
#
# Receives on stdin:
#   {"type": "ingest", "agent_id": "...", "message": "..."}
# Emits on stdout:
#   {"type": "ingest_result", "memories": [{"content": "..."}]}
require "json"

req = JSON.parse($stdin.read)
_agent_id = req["agent_id"]
_message  = req["message"]

# TODO: Implement your custom recall logic here.
memories = []

puts JSON.generate({ "type" => "ingest_result", "memories" => memories })
"#;

const RUBY_AFTER_TURN: &str = r#"# Context engine after_turn hook (Ruby).
require "json"

req = JSON.parse($stdin.read)
_agent_id = req["agent_id"]
_messages = req["messages"]

# TODO: Implement your post-turn logic here.

puts JSON.generate({ "type" => "ok" })
"#;

// --- Bash templates ---

const BASH_INGEST: &str = r#"#!/usr/bin/env bash
# Context engine ingest hook (Bash).
#
# Receives on stdin:
#   {"type":"ingest","agent_id":"...","message":"..."}
# Emits on stdout:
#   {"type":"ingest_result","memories":[]}
#
# For non-trivial logic, pipe stdin through `jq` or call out to a helper binary.
set -euo pipefail

_input=$(cat)
# TODO: parse "$_input" and build your recall result.
printf '{"type":"ingest_result","memories":[]}\n'
"#;

const BASH_AFTER_TURN: &str = r#"#!/usr/bin/env bash
# Context engine after_turn hook (Bash).
set -euo pipefail

_input=$(cat)
# TODO: persist state, update indexes, etc.
printf '{"type":"ok"}\n'
"#;

// --- Bun templates (TypeScript via Bun) ---

const BUN_INGEST: &str = r#"// Context engine ingest hook (Bun / TypeScript).
//
// Receives on stdin:
//   {"type": "ingest", "agent_id": "...", "message": "..."}
// Emits on stdout:
//   {"type": "ingest_result", "memories": [{"content": "..."}]}
//
// Run with: `bun run ingest.ts`

interface IngestRequest {
  type: "ingest";
  agent_id: string;
  message: string;
}

interface Memory { content: string }

const input = await Bun.stdin.text();
const req = JSON.parse(input) as IngestRequest;
void req.agent_id;
void req.message;

// TODO: Implement your custom recall logic here.
const memories: Memory[] = [];

console.log(JSON.stringify({ type: "ingest_result", memories }));
"#;

const BUN_AFTER_TURN: &str = r#"// Context engine after_turn hook (Bun / TypeScript).
const input = await Bun.stdin.text();
const _req = JSON.parse(input);

// TODO: Implement your post-turn logic here.

console.log(JSON.stringify({ type: "ok" }));
"#;

// --- PHP templates ---

const PHP_INGEST: &str = r#"<?php
// Context engine ingest hook (PHP).
//
// Receives on stdin:
//   {"type": "ingest", "agent_id": "...", "message": "..."}
// Emits on stdout:
//   {"type": "ingest_result", "memories": [{"content": "..."}]}

$raw = stream_get_contents(STDIN);
$req = json_decode($raw, true);
$_agentId = $req["agent_id"] ?? null;
$_message = $req["message"] ?? null;

// TODO: Implement your custom recall logic here.
$memories = [];

echo json_encode(["type" => "ingest_result", "memories" => $memories]), "\n";
"#;

const PHP_AFTER_TURN: &str = r#"<?php
// Context engine after_turn hook (PHP).
$raw = stream_get_contents(STDIN);
$_req = json_decode($raw, true);

// TODO: Implement your post-turn logic here.

echo json_encode(["type" => "ok"]), "\n";
"#;

// --- Lua templates ---

const LUA_INGEST: &str = r#"-- Context engine ingest hook (Lua).
--
-- Receives on stdin:
--   {"type": "ingest", "agent_id": "...", "message": "..."}
-- Emits on stdout:
--   {"type": "ingest_result", "memories": [{"content": "..."}]}
--
-- Requires a JSON library on LUA_PATH (`luarocks install dkjson`).
local json = require("dkjson")

local raw = io.read("*a")
local req = json.decode(raw)
local _agent_id = req.agent_id
local _message  = req.message

-- TODO: Implement your custom recall logic here.
local memories = {}

io.write(json.encode({ type = "ingest_result", memories = memories }), "\n")
"#;

const LUA_AFTER_TURN: &str = r#"-- Context engine after_turn hook (Lua).
local json = require("dkjson")

local raw = io.read("*a")
local _req = json.decode(raw)

-- TODO: Implement your post-turn logic here.

io.write(json.encode({ type = "ok" }), "\n")
"#;

/// Install Python requirements for a plugin.
pub async fn install_requirements(plugin_name: &str) -> Result<String, String> {
    validate_plugin_name(plugin_name)?;
    let plugin_dir = plugins_dir().join(plugin_name);
    let requirements = plugin_dir.join("requirements.txt");

    if !requirements.exists() {
        return Ok("No requirements.txt found — nothing to install".to_string());
    }

    // In virtualenv/conda environments, pip forbids --user installs.
    let in_venv = std::env::var("VIRTUAL_ENV").is_ok() || std::env::var("CONDA_PREFIX").is_ok();
    let mut args = vec!["-m", "pip", "install"];
    if !in_venv {
        args.push("--user");
    }
    args.push("-r");

    warn!(
        plugin = plugin_name,
        requirements = %requirements.display(),
        venv = in_venv,
        "Installing Python requirements"
    );

    let output = tokio::process::Command::new("python")
        .args(&args)
        .arg(&requirements)
        .output()
        .await
        .map_err(|e| format!("Failed to run python -m pip: {e}"))?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("python -m pip install failed: {stderr}"))
    }
}

// ---------------------------------------------------------------------------
// GitHub API types
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct GitHubContent {
    name: String,
    #[serde(rename = "type")]
    content_type: String,
    download_url: Option<String>,
    url: Option<String>,
}

/// Recursively download a GitHub directory entry.
///
/// `depth` limits recursion to prevent unbounded traversal (max 10 levels).
async fn download_github_entry(
    client: &reqwest::Client,
    entry: &GitHubContent,
    target_dir: &Path,
    depth: usize,
) -> Result<(), String> {
    if depth > 10 {
        return Err("GitHub directory recursion depth exceeded (max 10 levels)".to_string());
    }

    // Validate entry.name to prevent path traversal attacks
    if entry.name.contains('/')
        || entry.name.contains('\\')
        || entry.name.contains("..")
        || entry.name.contains('\0')
    {
        return Err(format!(
            "Refusing to download entry with unsafe name: '{}'",
            entry.name
        ));
    }

    let target_path = target_dir.join(&entry.name);

    match entry.content_type.as_str() {
        "file" => {
            let download_url = entry
                .download_url
                .as_ref()
                .ok_or_else(|| format!("No download URL for {}", entry.name))?;

            let resp = client
                .get(download_url)
                .send()
                .await
                .map_err(|e| format!("Failed to download {}: {e}", entry.name))?;

            // Check Content-Length before downloading to reject oversized files early
            const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024; // 10 MiB per file
            if let Some(len) = resp.content_length() {
                if len > MAX_FILE_SIZE {
                    return Err(format!(
                        "File '{}' too large ({len} bytes, max {MAX_FILE_SIZE})",
                        entry.name
                    ));
                }
            }

            let content = resp
                .bytes()
                .await
                .map_err(|e| format!("Failed to read {}: {e}", entry.name))?;

            if content.len() as u64 > MAX_FILE_SIZE {
                return Err(format!(
                    "File '{}' too large ({} bytes, max {MAX_FILE_SIZE})",
                    entry.name,
                    content.len()
                ));
            }

            tokio::fs::write(&target_path, &content)
                .await
                .map_err(|e| format!("Failed to write {}: {e}", target_path.display()))?;

            debug!(
                file = entry.name,
                bytes = content.len(),
                "Downloaded plugin file"
            );
        }
        "dir" => {
            tokio::fs::create_dir_all(&target_path)
                .await
                .map_err(|e| format!("Failed to create dir: {e}"))?;

            // Recursively list and download subdirectory
            let sub_url = entry
                .url
                .as_ref()
                .ok_or_else(|| format!("No API URL for dir {}", entry.name))?;

            let resp = client
                .get(sub_url)
                .header("Accept", "application/vnd.github.v3+json")
                .send()
                .await
                .map_err(|e| format!("Failed to list dir {}: {e}", entry.name))?;

            let sub_entries: Vec<GitHubContent> = resp
                .json()
                .await
                .map_err(|e| format!("Failed to parse dir listing: {e}"))?;

            for sub_entry in &sub_entries {
                Box::pin(download_github_entry(
                    client,
                    sub_entry,
                    &target_path,
                    depth + 1,
                ))
                .await?;
            }
        }
        other => {
            debug!(
                name = entry.name,
                r#type = other,
                "Skipping unknown entry type"
            );
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Check that all declared hook scripts exist on disk and are within the plugin directory.
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
mod tests {
    use super::*;

    #[test]
    fn test_plugins_dir() {
        let dir = plugins_dir();
        assert!(dir.ends_with("plugins"));
        assert!(dir.to_string_lossy().contains(".librefang"));
    }

    #[test]
    fn test_list_plugins_no_panic() {
        // Should not panic even if plugins dir doesn't exist
        let _ = list_plugins();
    }

    #[test]
    fn test_get_plugin_not_installed() {
        let result = get_plugin_info("nonexistent-test-plugin-xyz");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not installed"));
    }

    #[test]
    fn test_remove_not_installed() {
        let result = remove_plugin("nonexistent-test-plugin-xyz");
        assert!(result.is_err());
    }

    #[test]
    fn test_scaffold_and_remove() {
        let tmp = tempfile::tempdir().unwrap();
        // Override HOME to use temp dir
        let plugin_dir = tmp.path().join("test-scaffold-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        // Test manifest parsing from scaffold content
        let manifest_content = r#"name = "test-scaffold"
version = "0.1.0"
description = "Test scaffold"
author = ""

[hooks]
ingest = "hooks/ingest.py"
after_turn = "hooks/after_turn.py"
"#;
        let manifest: PluginManifest = toml::from_str(manifest_content).unwrap();
        assert_eq!(manifest.name, "test-scaffold");
        assert_eq!(manifest.version, "0.1.0");
        assert_eq!(manifest.hooks.ingest.as_deref(), Some("hooks/ingest.py"));
        assert_eq!(
            manifest.hooks.after_turn.as_deref(),
            Some("hooks/after_turn.py")
        );
    }

    #[test]
    fn test_copy_dir_recursive() {
        let tmp_src = tempfile::tempdir().unwrap();
        let tmp_dst = tempfile::tempdir().unwrap();

        // Create source structure
        std::fs::create_dir_all(tmp_src.path().join("hooks")).unwrap();
        std::fs::write(tmp_src.path().join("plugin.toml"), "name = \"test\"").unwrap();
        std::fs::write(tmp_src.path().join("hooks/ingest.py"), "# hook").unwrap();

        let dst = tmp_dst.path().join("copied");
        copy_dir_recursive(tmp_src.path(), &dst).unwrap();

        assert!(dst.join("plugin.toml").exists());
        assert!(dst.join("hooks/ingest.py").exists());
    }

    #[test]
    fn test_dir_size() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.txt"), "hello").unwrap();
        std::fs::write(tmp.path().join("b.txt"), "world!").unwrap();
        let size = dir_size(tmp.path());
        assert_eq!(size, 11); // 5 + 6
    }

    #[test]
    fn test_check_hooks_exist() {
        let tmp = tempfile::tempdir().unwrap();
        let plugin_dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(plugin_dir.join("hooks")).unwrap();
        std::fs::write(plugin_dir.join("hooks/ingest.py"), "").unwrap();

        let manifest = PluginManifest {
            name: "test".to_string(),
            version: "0.1.0".to_string(),
            hooks: librefang_types::config::ContextEngineHooks {
                ingest: Some("hooks/ingest.py".to_string()),
                after_turn: Some("hooks/after_turn.py".to_string()), // missing
                ..Default::default()
            },
            ..Default::default()
        };

        assert!(!check_hooks_exist(&plugin_dir, &manifest));

        // Now create the missing file
        std::fs::write(plugin_dir.join("hooks/after_turn.py"), "").unwrap();
        assert!(check_hooks_exist(&plugin_dir, &manifest));

        // Path traversal: hook pointing outside plugin dir should fail
        let manifest_escape = PluginManifest {
            name: "test".to_string(),
            version: "0.1.0".to_string(),
            hooks: librefang_types::config::ContextEngineHooks {
                ingest: Some("../../etc/passwd".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(!check_hooks_exist(&plugin_dir, &manifest_escape));
    }

    /// Live listing smoke test — ensures the enriched listing populates
    /// `description`/`version`/`hooks` from at least one plugin's `plugin.toml`.
    /// Ignored by default — requires network access to GitHub.
    #[tokio::test]
    #[ignore]
    async fn test_list_registry_plugins_enriched() {
        // Skip disk cache so a cached name-only listing from a previous run
        // cannot mask a regression.
        // SAFETY: this test is marked #[ignore] and only runs explicitly (not
        // in parallel); no other test thread races on this env var.
        unsafe { std::env::set_var("LIBREFANG_REGISTRY_NO_CACHE", "1") };
        let entries = list_registry_plugins("librefang/librefang-registry")
            .await
            .expect("registry listing should succeed");
        assert!(!entries.is_empty(), "expected at least one plugin");
        assert!(
            entries.iter().any(|e| e.description.is_some()),
            "expected at least one plugin with a description"
        );
        assert!(
            entries.iter().any(|e| e.version.is_some()),
            "expected at least one plugin with a version"
        );
        assert!(
            entries.iter().any(|e| !e.hooks.is_empty()),
            "expected at least one plugin declaring hooks"
        );
    }

    /// Integration test: install from GitHub registry, run hook, then remove.
    /// Ignored by default — requires network access.
    #[tokio::test]
    #[ignore]
    async fn test_registry_install_run_remove() {
        // 1. Install echo-memory from registry
        let source = PluginSource::Registry {
            name: "echo-memory".to_string(),
            github_repo: None,
        };
        let info = install_plugin(&source)
            .await
            .expect("registry install failed");
        assert_eq!(info.manifest.name, "echo-memory");
        assert_eq!(info.manifest.version, "0.1.0");
        assert!(info.hooks_valid);

        // 2. List should include it
        let plugins = list_plugins();
        assert!(plugins.iter().any(|p| p.manifest.name == "echo-memory"));

        // 3. Run ingest hook
        let ingest_path = info.path.join("hooks/ingest.py");
        assert!(ingest_path.exists());

        let mut child = tokio::process::Command::new("python3")
            .arg(&ingest_path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("python3 should be available");

        {
            use tokio::io::AsyncWriteExt;
            let stdin = child.stdin.as_mut().unwrap();
            stdin
                .write_all(br#"{"type":"ingest","agent_id":"test-001","message":"Hello world"}"#)
                .await
                .unwrap();
        }
        child.stdin.take(); // close stdin
        let out = child.wait_with_output().await.unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("ingest_result"), "got: {stdout}");
        assert!(stdout.contains("echo-memory"), "got: {stdout}");

        // 4. Remove
        remove_plugin("echo-memory").expect("remove failed");
        assert!(get_plugin_info("echo-memory").is_err());
    }

    /// Sanity: a manifest with no `[i18n.*]` tables yields an empty map,
    /// not a serialization error or panic.
    #[test]
    fn parse_plugin_i18n_no_block() {
        let toml_str = r#"
name = "test-plugin"
version = "0.1.0"
description = "English description"
"#;
        let value: toml::Value = toml::from_str(toml_str).unwrap();
        let i18n = parse_plugin_i18n_blocks(&value);
        assert!(i18n.is_empty());
    }

    /// Multiple `[i18n.<lang>]` blocks with both fields populate cleanly.
    #[test]
    fn parse_plugin_i18n_multi_lang() {
        let toml_str = r#"
name = "auto-summarizer"
version = "0.1.0"
description = "English description"

[i18n.zh]
name = "自动摘要"
description = "持续维护会话摘要。"

[i18n.zh-TW]
name = "自動摘要"
description = "持續維護會話摘要。"

[i18n.fr]
name = "Auto-résumé"
description = "Maintient un résumé continu."
"#;
        let value: toml::Value = toml::from_str(toml_str).unwrap();
        let i18n = parse_plugin_i18n_blocks(&value);
        assert_eq!(i18n.len(), 3);
        assert_eq!(i18n["zh"].name.as_deref(), Some("自动摘要"));
        assert_eq!(i18n["zh-TW"].name.as_deref(), Some("自動摘要"));
        assert_eq!(
            i18n["fr"].description.as_deref(),
            Some("Maintient un résumé continu.")
        );
    }

    /// A block that only sets `name` (no description) survives, with
    /// description left as `None` so callers know to fall back.
    #[test]
    fn parse_plugin_i18n_partial_entry() {
        let toml_str = r#"
[i18n.de]
name = "Beispiel"
"#;
        let value: toml::Value = toml::from_str(toml_str).unwrap();
        let i18n = parse_plugin_i18n_blocks(&value);
        assert_eq!(i18n.len(), 1);
        assert_eq!(i18n["de"].name.as_deref(), Some("Beispiel"));
        assert!(i18n["de"].description.is_none());
    }

    /// A `[i18n.<lang>]` block that sets neither field is dropped — keeping
    /// it would just take memory for no observable effect at the API
    /// boundary.
    #[test]
    fn parse_plugin_i18n_empty_entry_dropped() {
        let toml_str = r#"
[i18n.ja]
"#;
        let value: toml::Value = toml::from_str(toml_str).unwrap();
        let i18n = parse_plugin_i18n_blocks(&value);
        assert!(i18n.is_empty(), "empty i18n.ja entry should not be kept");
    }

    /// Non-string `name` / `description` values (e.g. someone wrote a
    /// number by mistake) are silently ignored rather than panicking.
    #[test]
    fn parse_plugin_i18n_non_string_values_ignored() {
        let toml_str = r#"
[i18n.es]
name = 42
description = "Spanish description"
"#;
        let value: toml::Value = toml::from_str(toml_str).unwrap();
        let i18n = parse_plugin_i18n_blocks(&value);
        assert_eq!(i18n.len(), 1);
        assert!(i18n["es"].name.is_none(), "non-string name dropped");
        assert_eq!(
            i18n["es"].description.as_deref(),
            Some("Spanish description")
        );
    }

    // ── #3805 — registry pubkey resolver (env > TOFU cache > worker fetch) ──

    /// Round-trip: a 32-byte non-zero key encodes/decodes through the
    /// validator. This is the shape the resolver, the worker keygen script,
    /// and ed25519_dalek all agree on.
    #[test]
    fn valid_registry_pubkey_b64_accepts_real_key() {
        use base64::Engine as _;
        let real_key = [0xABu8; 32];
        let b64 = base64::engine::general_purpose::STANDARD.encode(real_key);
        assert!(
            is_valid_registry_pubkey_b64(&b64),
            "non-zero 32-byte key must validate"
        );
    }

    /// The validator must reject the historical all-zero placeholder, garbage
    /// base64, and wrong-length keys — the three failure modes that fall back
    /// to the next link of the resolver chain.
    #[test]
    fn valid_registry_pubkey_b64_rejects_invalid_inputs() {
        use base64::Engine as _;

        // All-zero 32-byte key (legacy placeholder) — rejected.
        let zero = base64::engine::general_purpose::STANDARD.encode([0u8; 32]);
        assert!(
            !is_valid_registry_pubkey_b64(&zero),
            "all-zero placeholder must be rejected"
        );

        // Wrong length (16 bytes) — rejected.
        let short = base64::engine::general_purpose::STANDARD.encode([0xAAu8; 16]);
        assert!(
            !is_valid_registry_pubkey_b64(&short),
            "16-byte key must be rejected"
        );

        // Wrong length (64 bytes) — rejected.
        let long = base64::engine::general_purpose::STANDARD.encode([0xAAu8; 64]);
        assert!(
            !is_valid_registry_pubkey_b64(&long),
            "64-byte key must be rejected"
        );

        // Non-base64 garbage — rejected.
        assert!(
            !is_valid_registry_pubkey_b64("not-base64!!!"),
            "garbage input must be rejected"
        );

        // Empty input — rejected.
        assert!(!is_valid_registry_pubkey_b64(""), "empty must be rejected");

        // Whitespace tolerated around a valid key.
        let real_key = [0xABu8; 32];
        let b64_padded = format!(
            "  {}  ",
            base64::engine::general_purpose::STANDARD.encode(real_key),
        );
        assert!(
            is_valid_registry_pubkey_b64(&b64_padded),
            "validator must trim whitespace"
        );
    }

    /// The TOFU cache path is derived from $HOME — verify the directory layout
    /// matches the conventional `~/.librefang/` location used elsewhere in the
    /// runtime (config, plugins, agents).
    #[test]
    fn registry_pubkey_cache_path_lives_under_dotlibrefang() {
        // Save and restore HOME to avoid leaking into other tests.
        let original = std::env::var("HOME").ok();
        // SAFETY: tests in this module are single-threaded for env mutation.
        unsafe {
            std::env::set_var("HOME", "/tmp/librefang-test-home-pubkey");
        }
        let path = registry_pubkey_cache_path().expect("path resolution");
        // SAFETY: restoring the prior value of HOME.
        unsafe {
            match original {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
        }
        assert_eq!(
            path,
            std::path::PathBuf::from("/tmp/librefang-test-home-pubkey/.librefang/registry.pub"),
        );
    }

    /// Slot 0 of the embedded keys MUST be the active key — no expiry.
    /// A maintainer who absent-mindedly sets `expires_at: Some(...)` on
    /// slot 0 during a rotation edit would silently break installs the
    /// moment that timestamp passed (PR re-review LOW round 4). Compile-
    /// time + test-time guard so the regression is caught before ship.
    #[test]
    fn embedded_pubkeys_slot0_has_no_expiry() {
        let slot0 = EMBEDDED_REGISTRY_PUBKEYS
            .first()
            .expect("EMBEDDED_REGISTRY_PUBKEYS must have at least one entry");
        assert!(
            slot0.expires_at.is_none(),
            "EMBEDDED_REGISTRY_PUBKEYS[0] must have expires_at: None — slot 0 \
             is the active key and must not be marked for rotation"
        );
        assert!(
            is_valid_registry_pubkey_b64(slot0.pubkey_b64),
            "EMBEDDED_REGISTRY_PUBKEYS[0].pubkey_b64 is not a valid 32-byte Ed25519 key"
        );
    }

    /// Official registry defaults to the worker-signed mirror — the GitHub
    /// repo has no committed `index.json`, so any other choice would lose
    /// the only end-to-end Ed25519-verifiable path.
    #[test]
    fn registry_index_urls_official_defaults_to_worker_mirror() {
        let (idx, sig) = registry_index_urls("librefang/librefang-registry", None, None);
        assert_eq!(idx, "https://stats.librefang.ai/api/registry/index.json");
        assert_eq!(sig, "https://stats.librefang.ai/api/registry/index.json.sig");
    }

    /// Self-hosted forks fall back to GitHub raw — keeps the existing path
    /// for forks that don't yet run a signed mirror, while still allowing
    /// them to opt in via the env vars.
    #[test]
    fn registry_index_urls_fork_falls_back_to_github_raw() {
        let (idx, sig) = registry_index_urls("acme/private-registry", None, None);
        assert_eq!(
            idx,
            "https://raw.githubusercontent.com/acme/private-registry/main/index.json"
        );
        assert_eq!(
            sig,
            "https://raw.githubusercontent.com/acme/private-registry/main/index.json.sig"
        );
    }

    /// Env overrides win regardless of which registry is in use — operators
    /// of air-gapped / on-prem deployments must be able to redirect both
    /// the official and the fork path at their own infrastructure.
    #[test]
    fn registry_index_urls_env_overrides_win_for_both_paths() {
        let (idx, sig) = registry_index_urls(
            "librefang/librefang-registry",
            Some("https://internal.example/index.json".into()),
            Some("https://internal.example/index.json.sig".into()),
        );
        assert_eq!(idx, "https://internal.example/index.json");
        assert_eq!(sig, "https://internal.example/index.json.sig");

        let (idx, sig) = registry_index_urls(
            "acme/private-registry",
            Some("https://internal.example/index.json".into()),
            Some("https://internal.example/index.json.sig".into()),
        );
        assert_eq!(idx, "https://internal.example/index.json");
        assert_eq!(sig, "https://internal.example/index.json.sig");
    }

    // ── Bug #3804 — hook script integrity check logic ────────────────────────

    /// Helper: build a minimal PluginManifest with the given hook paths and
    /// integrity entries so we can exercise the detection logic without
    /// spinning up an HTTP server.
    fn make_manifest_with_hooks(
        hooks: &[(&str, &str)],     // (field_name, script_path)
        integrity: &[(&str, &str)], // (script_path, sha256hex)
    ) -> PluginManifest {
        let mut m = PluginManifest {
            name: "test-plugin".to_string(),
            version: "0.1.0".to_string(),
            ..Default::default()
        };
        for &(field, path) in hooks {
            match field {
                "ingest" => m.hooks.ingest = Some(path.to_string()),
                "after_turn" => m.hooks.after_turn = Some(path.to_string()),
                "bootstrap" => m.hooks.bootstrap = Some(path.to_string()),
                "assemble" => m.hooks.assemble = Some(path.to_string()),
                "compact" => m.hooks.compact = Some(path.to_string()),
                "prepare_subagent" => m.hooks.prepare_subagent = Some(path.to_string()),
                "merge_subagent" => m.hooks.merge_subagent = Some(path.to_string()),
                _ => {}
            }
        }
        for &(path, hash) in integrity {
            m.integrity.insert(path.to_string(), hash.to_string());
        }
        m
    }

    /// Extracts the list of hook script paths that are declared in a manifest
    /// but missing from its integrity map.  Delegates to the production
    /// `manifest_missing_integrity_hooks` so the install-time check, the
    /// `lint_plugin` warning, and these regression tests can never drift.
    fn missing_integrity_hooks(manifest: &PluginManifest) -> Vec<String> {
        super::manifest_missing_integrity_hooks(manifest)
    }

    /// A plugin with no hooks declared requires no integrity entries.
    #[test]
    fn hook_integrity_no_hooks_no_requirement() {
        let m = make_manifest_with_hooks(&[], &[]);
        assert!(
            missing_integrity_hooks(&m).is_empty(),
            "no hooks → no integrity entries required"
        );
    }

    /// Every declared hook must appear in [integrity]; any missing entry is flagged.
    #[test]
    fn hook_integrity_missing_entries_detected() {
        let m = make_manifest_with_hooks(
            &[
                ("ingest", "hooks/ingest.py"),
                ("after_turn", "hooks/after_turn.py"),
            ],
            &[
                // after_turn is covered, but ingest is not
                ("hooks/after_turn.py", "abc123"),
            ],
        );
        let missing = missing_integrity_hooks(&m);
        assert_eq!(missing, vec!["hooks/ingest.py"]);
    }

    /// When all declared hooks have integrity entries, no missing entries are reported.
    #[test]
    fn hook_integrity_all_covered_passes() {
        let m = make_manifest_with_hooks(
            &[
                ("ingest", "hooks/ingest.py"),
                ("after_turn", "hooks/after_turn.py"),
            ],
            &[
                ("hooks/ingest.py", "deadbeef"),
                ("hooks/after_turn.py", "cafebabe"),
            ],
        );
        assert!(
            missing_integrity_hooks(&m).is_empty(),
            "all hooks covered → no missing integrity entries"
        );
    }

    /// All seven hook fields are checked, not just ingest/after_turn.
    #[test]
    fn hook_integrity_all_hook_fields_checked() {
        let all_hooks = [
            ("ingest", "hooks/ingest.py"),
            ("after_turn", "hooks/after_turn.py"),
            ("bootstrap", "hooks/bootstrap.py"),
            ("assemble", "hooks/assemble.py"),
            ("compact", "hooks/compact.py"),
            ("prepare_subagent", "hooks/prepare_subagent.py"),
            ("merge_subagent", "hooks/merge_subagent.py"),
        ];
        // Provide integrity for all but compact and merge_subagent.
        let integrity_provided = [
            ("hooks/ingest.py", "h1"),
            ("hooks/after_turn.py", "h2"),
            ("hooks/bootstrap.py", "h3"),
            ("hooks/assemble.py", "h4"),
            ("hooks/prepare_subagent.py", "h6"),
        ];
        let m = make_manifest_with_hooks(&all_hooks, &integrity_provided);
        let mut missing = missing_integrity_hooks(&m);
        missing.sort();
        assert_eq!(
            missing,
            vec!["hooks/compact.py", "hooks/merge_subagent.py"],
            "compact and merge_subagent must be flagged"
        );
    }

    // ── Bug #4036 — registry publish pipeline must auto-inject integrity ──

    /// Helper: write a minimal plugin layout into `dir` with the requested
    /// hook scripts and the requested raw `plugin.toml` body.  Returns the
    /// directory path so the caller can keep ownership of the tempdir.
    fn write_fake_plugin(
        dir: &Path,
        manifest_toml: &str,
        scripts: &[(&str, &[u8])],
    ) -> std::path::PathBuf {
        std::fs::write(dir.join("plugin.toml"), manifest_toml).unwrap();
        for (rel, body) in scripts {
            let abs = dir.join(rel);
            if let Some(parent) = abs.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&abs, body).unwrap();
        }
        dir.to_path_buf()
    }

    /// pack_plugin_for_publish must write [integrity] entries with the
    /// correct SHA-256 of every declared hook.  Mirrors the real
    /// `context-decay` regression: declares two hooks, no [integrity].
    #[test]
    fn pack_plugin_for_publish_auto_injects_hashes_for_context_decay_shape() {
        let tmp = tempfile::tempdir().unwrap();
        let plugin_dir = tmp.path().join("context-decay");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        let manifest = r#"name = "context-decay"
version = "0.1.0"
description = "Decay older context entries"
author = "Test"

[hooks]
ingest = "hooks/ingest.py"
after_turn = "hooks/after_turn.py"
"#;
        let ingest_body = b"# ingest hook\nprint('ingest')\n";
        let after_turn_body = b"# after_turn hook\nprint('after')\n";
        let plugin_dir = write_fake_plugin(
            &plugin_dir,
            manifest,
            &[
                ("hooks/ingest.py", ingest_body),
                ("hooks/after_turn.py", after_turn_body),
            ],
        );

        // Sanity: the unsigned manifest must fail validation up-front,
        // matching the user-visible error in the bug report.
        let pre = validate_publish_ready(&plugin_dir).expect_err("unsigned must fail");
        assert!(
            pre.contains("hooks/ingest.py") && pre.contains("hooks/after_turn.py"),
            "validate_publish_ready must list every missing hook, got: {pre}"
        );

        // Run the publish packer.
        let written = pack_plugin_for_publish(&plugin_dir).expect("pack must succeed");
        assert_eq!(written.len(), 2, "both hooks must be hashed");
        assert_eq!(written["hooks/ingest.py"], sha256_hex(ingest_body));
        assert_eq!(written["hooks/after_turn.py"], sha256_hex(after_turn_body));

        // Re-read the manifest and confirm the [integrity] block is present
        // and matches the expected hashes.
        let rewritten = std::fs::read_to_string(plugin_dir.join("plugin.toml")).unwrap();
        let parsed: PluginManifest = toml::from_str(&rewritten).expect("rewritten manifest valid");
        assert_eq!(
            parsed.integrity.get("hooks/ingest.py").map(String::as_str),
            Some(sha256_hex(ingest_body).as_str())
        );
        assert_eq!(
            parsed
                .integrity
                .get("hooks/after_turn.py")
                .map(String::as_str),
            Some(sha256_hex(after_turn_body).as_str())
        );

        // The packed plugin must now satisfy the publish-readiness check.
        validate_publish_ready(&plugin_dir).expect("packed plugin must validate");

        // And the install-time loader must accept it without complaint.
        let loaded = load_plugin_manifest(&plugin_dir).expect("install-time load must accept");
        assert_eq!(loaded.name, "context-decay");
        assert_eq!(loaded.integrity.len(), 2);
    }

    /// pack_plugin_for_publish must replace a stale [integrity] block — not
    /// duplicate it — when authors re-pack after editing a hook script.
    #[test]
    fn pack_plugin_for_publish_replaces_stale_integrity_block() {
        let tmp = tempfile::tempdir().unwrap();
        let plugin_dir = tmp.path().join("stale-test");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        let manifest = r#"name = "stale-test"
version = "0.1.0"
description = "Replace stale integrity"
author = "Test"

[hooks]
ingest = "hooks/ingest.py"

[integrity]
"hooks/ingest.py" = "deadbeef_stale_hash_must_be_replaced"
"hooks/removed.py" = "0000_orphan_hash_must_be_dropped"
"#;
        let ingest_body = b"# fresh content\n";
        write_fake_plugin(&plugin_dir, manifest, &[("hooks/ingest.py", ingest_body)]);

        let written = pack_plugin_for_publish(&plugin_dir).expect("pack must succeed");
        assert_eq!(written.len(), 1);

        let rewritten = std::fs::read_to_string(plugin_dir.join("plugin.toml")).unwrap();
        // Only one [integrity] header — no duplicates from the stale block.
        assert_eq!(
            rewritten.matches("[integrity]").count(),
            1,
            "stale [integrity] block must be replaced, not appended:\n{rewritten}"
        );
        // Stale entry for a hook that no longer exists must be gone.
        assert!(
            !rewritten.contains("hooks/removed.py"),
            "orphan integrity entry must be dropped:\n{rewritten}"
        );

        let parsed: PluginManifest = toml::from_str(&rewritten).unwrap();
        assert_eq!(
            parsed.integrity.get("hooks/ingest.py").map(String::as_str),
            Some(sha256_hex(ingest_body).as_str())
        );
        assert!(!parsed.integrity.contains_key("hooks/removed.py"));
    }

    /// pack_plugin_for_publish must fail loudly when a manifest references
    /// a hook script that isn't on disk — that's a packaging bug and
    /// emitting the SHA-256 of empty bytes would silently mask it.
    #[test]
    fn pack_plugin_for_publish_rejects_missing_hook_file() {
        let tmp = tempfile::tempdir().unwrap();
        let plugin_dir = tmp.path().join("missing-hook");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        let manifest = r#"name = "missing-hook"
version = "0.1.0"
description = "Hook file is not shipped"
author = "Test"

[hooks]
ingest = "hooks/ingest.py"
"#;
        // Note: deliberately do NOT write hooks/ingest.py.
        std::fs::write(plugin_dir.join("plugin.toml"), manifest).unwrap();

        let err = pack_plugin_for_publish(&plugin_dir).expect_err("missing hook must fail");
        assert!(
            err.contains("hooks/ingest.py"),
            "error must name the missing hook, got: {err}"
        );
    }

    /// A plugin that declares no hooks at all is publish-ready by definition
    /// — pack returns an empty map, validate accepts.
    #[test]
    fn pack_plugin_for_publish_accepts_no_hooks_plugin() {
        let tmp = tempfile::tempdir().unwrap();
        let plugin_dir = tmp.path().join("metadata-only");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        let manifest = r#"name = "metadata-only"
version = "0.1.0"
description = "No hooks, just metadata"
author = "Test"
"#;
        std::fs::write(plugin_dir.join("plugin.toml"), manifest).unwrap();

        validate_publish_ready(&plugin_dir).expect("no-hooks plugin is publish-ready");
        let written = pack_plugin_for_publish(&plugin_dir).expect("pack must succeed");
        assert!(written.is_empty(), "no hooks → no hashes written");
    }

    /// validate_publish_ready must accept a partially-signed manifest only
    /// when EVERY declared hook is covered.  A plugin that ships
    /// `[integrity]` for some but not all hooks is still rejected — this is
    /// the defense-in-depth backstop for issue #4036.
    #[test]
    fn validate_publish_ready_rejects_partial_integrity() {
        let tmp = tempfile::tempdir().unwrap();
        let plugin_dir = tmp.path().join("partial");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        let manifest = r#"name = "partial"
version = "0.1.0"
description = "Half-signed"
author = "Test"

[hooks]
ingest = "hooks/ingest.py"
after_turn = "hooks/after_turn.py"

[integrity]
"hooks/ingest.py" = "abc"
"#;
        std::fs::write(plugin_dir.join("plugin.toml"), manifest).unwrap();

        let err = validate_publish_ready(&plugin_dir).expect_err("partial must fail");
        assert!(
            err.contains("hooks/after_turn.py"),
            "after_turn must be flagged as missing, got: {err}"
        );
        assert!(
            !err.contains("hooks/ingest.py"),
            "ingest is signed and must NOT be flagged, got: {err}"
        );
    }

    /// pack_plugin_for_publish must produce byte-identical output across
    /// repeated invocations on identical inputs — a property the registry
    /// archive checksum and any reproducible-build verifier depend on.
    #[test]
    fn pack_plugin_for_publish_is_deterministic() {
        fn pack_once(seed: &[u8]) -> String {
            let tmp = tempfile::tempdir().unwrap();
            let plugin_dir = tmp.path().join("det");
            std::fs::create_dir_all(&plugin_dir).unwrap();
            let manifest = r#"name = "det"
version = "0.1.0"
description = "Determinism test"
author = "Test"

[hooks]
ingest = "hooks/ingest.py"
after_turn = "hooks/after_turn.py"
bootstrap = "hooks/bootstrap.py"
"#;
            write_fake_plugin(
                &plugin_dir,
                manifest,
                &[
                    ("hooks/ingest.py", seed),
                    ("hooks/after_turn.py", seed),
                    ("hooks/bootstrap.py", seed),
                ],
            );
            pack_plugin_for_publish(&plugin_dir).unwrap();
            std::fs::read_to_string(plugin_dir.join("plugin.toml")).unwrap()
        }

        let a = pack_once(b"identical seed");
        let b = pack_once(b"identical seed");
        assert_eq!(
            a, b,
            "pack output must be byte-identical for identical inputs"
        );
    }
}
