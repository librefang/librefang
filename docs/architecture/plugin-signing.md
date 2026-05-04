# Plugin signing — trust model

LibreFang plugins are third-party code that runs inside the daemon process.
Anything an attacker can drop into `~/.librefang/plugins/` is one
`PluginManifest`-scoped RCE away from the user. The signing pipeline is the
defense-in-depth layer that protects the **distribution channel** —
download-time tampering, compromised registry frontends, MITM on the install
path.

For the operator-side runbook (keygen, deploy, rotation), see
`web/workers/SIGNING.md`.

## Layered defenses

The install path validates plugins through three independent gates, in
order. Stronger checks short-circuit weaker ones; weaker checks **never**
short-circuit stronger ones, but a stronger check failing does not mean a
weaker check is sufficient — both must pass when both apply.

```
              install_from_registry
                       │
                       ▼
       ┌──────────────────────────────────┐
       │ 1. Transport (HTTPS + GitHub raw │   trust anchor: TLS PKI +
       │    or Cloudflare Worker URL)     │   GitHub repo permissions
       └──────────────────────────────────┘
                       │
                       ▼
       ┌──────────────────────────────────┐
       │ 2. SHA-256 checksum (optional;   │   trust anchor: registry
       │    `<archive>.sha256` or         │   provides a value the
       │    `checksums.txt`)              │   downloaded bytes must hash to
       └──────────────────────────────────┘
                       │
                       ▼
       ┌──────────────────────────────────┐
       │ 3. Ed25519 archive signature     │   trust anchor: registry holds
       │    (`<archive>.sig`)             │   a private key whose public
       │    verified with resolver pubkey │   half is pinned by the daemon
       └──────────────────────────────────┘
```

### When does a missing layer hard-fail?

| SHA-256 | Ed25519 | Outcome |
|---|---|---|
| ✓ verified | ✓ verified | install — both layers passed |
| ✓ verified | ✗ pubkey unavailable | **install with warning** — SHA-256 is enough integrity to proceed |
| ✓ verified | ✗ signature mismatch | **hard-fail** — active tampering signal |
| ✗ absent | ✓ verified | install — Ed25519 covers the bytes |
| ✗ absent | ✗ pubkey unavailable | **hard-fail** — no integrity check would remain |

This matches the [#3805] design: previously, the all-zero placeholder pubkey
caused a hard-fail at every install attempt, even when SHA-256 was already
verified. With no real pubkey deployed yet, that meant zero plugins could
install through the registry path. The resolver below restores fail-closed
behavior **only** when no integrity check at all is possible.

## The pubkey resolver chain

`resolve_registry_pubkey()` walks three sources in order. The first valid
key wins; subsequent calls in the same process re-walk the chain, so an
operator can override mid-session via env vars without restarting the
daemon.

### 1. `LIBREFANG_REGISTRY_PUBKEY` env var

Highest priority. Useful for self-hosted registries, CI smoke tests, and
operators who want the key embedded in their config-management layer
rather than fetched at runtime. Must be a base64-encoded raw 32-byte
Ed25519 public key — the same shape `ed25519_dalek::VerifyingKey::from_bytes`
consumes.

### 2. TOFU-pinned cache (`~/.librefang/registry.pub`)

Trust on first use. The first successful network fetch (step 3) writes the
key here; every later install reads from this file directly and skips the
network call. Rotation is explicit: delete the file and the next install
refreshes it.

The cache is plain text (base64 of 32 raw bytes, plus optional trailing
newline). It is not signed by any higher authority — this matches how SSH
known_hosts works. The protections are:

- The file is in `$HOME` (user-owned), not world-writable.
- A mismatch between cached and freshly-fetched key surfaces during
  rotation, since rotation requires deleting the cache.
- An attacker who can write to `$HOME` can already run code as the user;
  poisoning this file is downstream of that more fundamental compromise.

### 3. HTTP fetch (default `https://librefang.ai/.well-known/registry-pubkey`)

Backed by the `registry-worker` Cloudflare Worker
(`web/workers/registry-worker/index.js`), which serves the value of its
`REGISTRY_PUBLIC_KEY` env var. The default URL is overridable via
`LIBREFANG_REGISTRY_PUBKEY_URL` for self-hosted registries.

Network failures (timeout, non-2xx, malformed body) propagate as `Err` from
the resolver. The caller decides whether to hard-fail (index verification)
or fall through to SHA-256-only (archive install).

## What gets signed

| Artefact | Signed by | Canonical bytes |
|---|---|---|
| Registry index | `registry-worker` cron, after each refresh | The exact bytes returned by `GET https://stats.librefang.ai/api/registry/index.json` — a flat JSON array of `{name, version?, description?, needs?}` entries, sorted by name, rebuilt from the `librefang/librefang-registry` GitHub repo's plugin TOMLs. The dashboard's dict-shaped `GET /api/registry` is a separate KV row and is **not** what the daemon parses. |
| Marketplace bundle metadata | `marketplace-worker` on `POST /v1/packages/<slug>/versions` | `<slug>@<version>\|<bundle_url>\|<bundle_sha256>` (UTF-8). The daemon reconstructs this string locally before verifying. |

The bundle bytes themselves are NOT signed — only the metadata. The
SHA-256 of the bundle bytes is in the metadata, so a tampered bundle
fails the SHA-256 check. This split lets the bundle live on any CDN
(GitHub Releases, R2, S3) without requiring the worker to pipe gigabytes
through itself.

## Why Ed25519 (and not RSA / ECDSA)

- 32-byte public keys, 64-byte signatures — easy to ship as base64 in env
  vars, HTTP headers, and TOFU files.
- Deterministic signatures (no nonce reuse foot-guns).
- Native to Web Crypto in Cloudflare Workers (since 2023) and to
  `ed25519_dalek` in Rust.
- No curve-vs-key-size choices to misconfigure.

## What this is NOT

- **Not a replacement for code review.** A signed plugin from a trusted
  registry can still be malicious. The signature only attests "this came
  from the holder of the private key", not "this code is safe".
- **Not transparency log.** There is no Merkle tree, no Sigstore-style
  rekor. Rotation history, signing audit trails, and revocation are
  manual processes documented in `web/workers/SIGNING.md`.
- **Not key escrow.** The private key lives only as a Cloudflare Worker
  secret. Loss of that secret means all future signatures need a fresh
  keypair and a coordinated daemon-side rotation.

## References

- `crates/librefang-runtime/src/plugin_manager.rs` — `resolve_registry_pubkey`,
  `verify_registry_index`, `verify_archive_signature`,
  `install_from_registry`, `fetch_verified_index`
- `web/workers/registry-worker/index.js` — `signWithRegistryKey`,
  `handleSignedIndex`, `handleSignedIndexSig`, `handlePubkey`
- `web/workers/marketplace-worker/index.js` — `signWithRegistryKey`,
  `handleVersionSignature`, `handlePubkey`
- `web/workers/keygen.mjs` — Ed25519 keypair generation
- `web/workers/SIGNING.md` — operator runbook
