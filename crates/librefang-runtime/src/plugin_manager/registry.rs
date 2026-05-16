//! Registry trust chain: embedded pubkey rotation slots, signed-index
//! verification (Ed25519), and the on-disk index/pubkey cache layer.
//!
//! Split out of plugin_manager so the verify path lives next to the
//! constants it consumes (rotation slots, env-override URLs) instead of
//! being smeared across the parent module alongside install / publish
//! / lint logic.

use super::*;

/// One embedded pubkey + its expiry (Unix seconds). Slot-0 (active) uses
/// `expires_at == None`; rotation-window slots MUST set an expiry, or
/// daemons in the field would accept the prior key indefinitely — and a
/// post-rotation leak of that prior private key would still be exploitable
/// against every still-running daemon binary that carries it. PR re-review
/// MEDIUM (round 3).
pub(super) struct EmbeddedPubkey {
    /// Base64-encoded raw 32-byte Ed25519 public key. Field name is
    /// `pubkey_b64` (not `b64`) so the lockstep CI script's regex picks
    /// up *this* field unambiguously and not some unrelated future
    /// `b64: "..."` literal that drifts in (PR re-review LOW round 4).
    pub(super) pubkey_b64: &'static str,
    /// `None` = active key, no expiry. `Some(t)` = prior key, valid only
    /// while `now() < t`.
    pub(super) expires_at: Option<i64>,
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
pub(super) const EMBEDDED_REGISTRY_PUBKEYS: &[EmbeddedPubkey] = &[EmbeddedPubkey {
    pubkey_b64: "ClGa0Ucap8NdrKAy1rw9Tt6A9I8eg4zJ53+xIuKMuq0=",
    expires_at: None,
}];

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
const OFFICIAL_INDEX_SIG_URL: &str = "https://stats.librefang.ai/api/registry/index.json.sig";

/// On-disk pin for the registry pubkey (TOFU cache).
///
/// First successful fetch is written here; later calls read this file
/// instead of going to the network. Rotation requires deleting this file
/// (operators or a daemon-side `librefang plugin rotate-pubkey` command).
pub(super) fn registry_pubkey_cache_path() -> Result<PathBuf, String> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| "Cannot determine home directory for pubkey cache".to_string())?;
    Ok(PathBuf::from(home).join(".librefang").join("registry.pub"))
}

/// True iff `s` decodes to a valid 32-byte non-all-zero Ed25519 pubkey.
pub(super) fn is_valid_registry_pubkey_b64(s: &str) -> bool {
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
            warn!(
                "Pubkey cache {} is not a regular file — ignoring",
                path.display()
            );
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
            warn!(
                "Pubkey cache {} is not a regular file — ignoring",
                path.display()
            );
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
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt;
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
            debug!(
                "Using TOFU-pinned registry pubkey from {}",
                cache_path.display()
            );
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
                debug!("Skipping embedded pubkey: expired at {} (now {})", exp, now);
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
pub(super) fn registry_cache_path(registry: &str) -> std::path::PathBuf {
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
pub(super) fn default_registry_cache_ttl_secs() -> u64 {
    3600
}

/// Try to load a cached registry index.
/// Returns `Some(bytes)` if the cache file exists and is newer than `ttl_secs`.
pub(super) fn load_registry_cache(path: &std::path::Path, ttl_secs: u64) -> Option<Vec<u8>> {
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
pub(super) fn save_registry_cache(path: &std::path::Path, bytes: &[u8]) {
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
pub(super) fn registry_index_urls(
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
pub(super) async fn fetch_verified_index(
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
