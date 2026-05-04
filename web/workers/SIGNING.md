# Plugin signing — operator runbook

This document describes how to provision and rotate the Ed25519 keypair the
two registry workers (`registry-worker` and `marketplace-worker`) use to sign
plugin manifests and bundle metadata, and how the LibreFang daemon picks up
the resulting public key.

For the trust-model design rationale (why Ed25519, why TOFU, how it layers
with SHA-256), see `docs/architecture/plugin-signing.md`.

## What the workers sign

| Worker | Endpoint exposed to daemon | Bytes that are signed |
|---|---|---|
| `registry-worker` | `GET /api/registry/index.json.sig` | The exact bytes returned by `GET /api/registry/index.json` (the cron-refreshed canonical index). |
| `marketplace-worker` | `GET /v1/download/<slug>/<version>/signature` returns `{ signed, sig }` | `signed` is the canonical string `<slug>@<version>\|<bundle_url>\|<bundle_sha256>` — the daemon reconstructs this string locally and verifies. |

Both workers also serve the public key:

- `registry-worker`: `GET /.well-known/registry-pubkey`
- `marketplace-worker`: `GET /v1/pubkey`

Both endpoints return the **raw 32-byte Ed25519 public key, base64-encoded**
(`ed25519_dalek::VerifyingKey::from_bytes` accepts this directly).

## Provisioning

1. **Generate a keypair locally** (do NOT commit the output):

   ```bash
   node web/workers/keygen.mjs
   ```

   This prints two values:
   - `REGISTRY_PUBLIC_KEY` — raw 32-byte pubkey, base64. Non-secret. Paste
     into `[vars]` in both `wrangler.toml` files (or push via
     `wrangler vars put`).
   - `REGISTRY_PRIVATE_KEY` — PKCS#8 DER, base64. **SECRET.** Store in your
     password manager (1Password, Vault, etc.) before deploying.

2. **Update `wrangler.toml` with the public key** for both workers:

   ```toml
   [vars]
   REGISTRY_PUBLIC_KEY = "<paste-raw-pubkey-base64>"
   ```

   Commit this. The daemon TOFUs against this value the first time it sees
   it, then pins.

3. **Deploy the private key as a secret** to both workers:

   ```bash
   cd web/workers/registry-worker
   echo "<paste-private-key-base64>" | wrangler secret put REGISTRY_PRIVATE_KEY

   cd ../marketplace-worker
   echo "<paste-private-key-base64>" | wrangler secret put REGISTRY_PRIVATE_KEY
   ```

4. **Deploy each worker**:

   ```bash
   cd web/workers/registry-worker && wrangler deploy
   cd ../marketplace-worker         && wrangler deploy
   ```

5. **Verify the endpoints** are live:

   ```bash
   # Public key — must return the same base64 you set in step 2.
   curl -s https://librefang-registry.<account>.workers.dev/.well-known/registry-pubkey
   curl -s https://librefang-marketplace.<account>.workers.dev/v1/pubkey

   # Trigger a registry refresh (cron also runs at 02:00 UTC daily).
   # On the next refresh, the signature endpoint will start returning data.
   curl -s "https://librefang-registry.<account>.workers.dev/api/registry?refresh=1" >/dev/null
   curl -s    https://librefang-registry.<account>.workers.dev/api/registry/index.json.sig | head -c 100
   ```

## Behavior when no key is configured

The signing path is opt-in and degrades gracefully — the workers stay
backward-compatible:

- `REGISTRY_PRIVATE_KEY` unset → `signWithRegistryKey()` returns `null`,
  the cron refresh skips storing a signature, and new
  `package_versions.bundle_sig` rows are written as `NULL`.
- `REGISTRY_PUBLIC_KEY` unset → the `/v1/pubkey` and
  `/.well-known/registry-pubkey` endpoints return HTTP 503.
- Existing `/api/registry`, `/api/registry/raw`, `/v1/packages`,
  `/v1/download/...` paths are unaffected.

The daemon, when it cannot retrieve a public key, falls back to SHA-256-only
verification (see `docs/architecture/plugin-signing.md`).

## Rotation

Rotate when a private key is suspected leaked, when an operator with access
leaves, or annually as hygiene.

1. Generate a fresh keypair via `node web/workers/keygen.mjs`.
2. Update `REGISTRY_PUBLIC_KEY` in both `wrangler.toml` files **and**
   re-deploy `REGISTRY_PRIVATE_KEY` as a secret to both workers.
3. Deploy both workers.
4. Trigger a registry refresh so the new index signature is generated:
   `curl https://librefang-registry.<account>.workers.dev/api/registry?refresh=1`.
5. **Bump the daemon's pinned key**: instruct daemon operators to delete
   `~/.librefang/registry.pub` (the TOFU cache); the next install will
   re-fetch and pin the new key. For mass deployments, ship a daemon
   release with the new key embedded in `OFFICIAL_REGISTRY_PUBKEY_B64` (or
   whatever resolver default replaces it — see daemon resolver design).

The old keypair should be archived (not destroyed) for audit and for
verifying historical signatures.

## Schema migration

`marketplace-worker` adds a new column `package_versions.bundle_sig`. For
new D1 databases the `CREATE TABLE` in `schema.sql` covers it. For existing
databases:

```sql
ALTER TABLE package_versions ADD COLUMN bundle_sig TEXT;
```

D1 ignores duplicate `ALTER` statements, so this is safe to run on every
deploy via a migration step.
