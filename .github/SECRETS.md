# Repository Secrets

This document tracks every GitHub Actions secret the LibreFang workflows
consume, why each one exists, and how to rotate it. **Update this file
whenever a workflow starts or stops using a secret** — silent drift is
the failure mode that bites maintainers six months later when a release
breaks and nobody remembers what `FOO_TOKEN_2` was for.

> Repository → Settings → Secrets and variables → Actions

Secrets are organisation-wide unless noted. Forks do not inherit them by
design — the `pull_request` trigger explicitly runs without secrets, so
any workflow gated on a fork-PR build must degrade gracefully when the
secret is empty.

---

## Mobile distribution (release.yml `mobile_android` / `mobile_ios`)

Required to attach signed mobile artifacts to GitHub releases. When any
of these are absent the corresponding mobile job degrades to an unsigned
debug build and skips the release-attach step — desktop builds are
unaffected.

### Android

| Secret | Purpose | Format | Rotation |
|---|---|---|---|
| `ANDROID_KEYSTORE_B64` | Base64-encoded `release.jks` keystore. Lose this and Play Store will refuse all future updates — the package identity is bound to its signing key. **Back up offline.** | `base64 -w0 release.jks` | Forever (per Play Store policy) — only rotate if compromised, with explicit Play Store key-rotation flow |
| `ANDROID_KEYSTORE_PASSWORD` | Password for the `.jks` keystore | UTF-8 | When personnel changes |
| `ANDROID_KEY_ALIAS` | Alias of the release key inside the keystore | plain string | n/a |
| `ANDROID_KEY_PASSWORD` | Password for the alias (often same as keystore password but treated separately) | UTF-8 | When personnel changes |

**Generation reference**

```sh
keytool -genkey -v -keystore release.jks -alias librefang \
  -keyalg RSA -keysize 4096 -validity 10000
base64 -w0 release.jks > keystore.b64   # paste contents into ANDROID_KEYSTORE_B64
```

### iOS / Apple

| Secret | Purpose | Format | Rotation |
|---|---|---|---|
| `APPLE_TEAM_ID` | 10-char Apple developer team ID | plain string (e.g. `ABCDE12345`) | n/a |
| `APPLE_CERT_P12` | Distribution certificate (`.p12`) | `base64 -w0 dist.p12` | **Yearly** — Apple distribution certs expire after one year |
| `APPLE_CERT_PASSWORD` | Password set when exporting the `.p12` from Keychain Access | UTF-8 | When personnel changes |
| `APPLE_PROVISIONING_PROFILE_B64` | Distribution provisioning profile (`.mobileprovision`) | `base64 -w0 librefang.mobileprovision` | **Yearly** — bound to the cert above |

**Rotation runbook (yearly Apple cert refresh)**

1. Apple Developer portal → Certificates → renew the iOS Distribution cert.
2. Download `.cer`, double-click to import into Keychain Access.
3. Right-click the new cert → Export → save as `.p12` with a strong password.
4. Update `APPLE_CERT_P12` and `APPLE_CERT_PASSWORD` in repo secrets.
5. Profiles tab → regenerate the matching distribution provisioning
   profile against the new cert.
6. Update `APPLE_PROVISIONING_PROFILE_B64`.
7. Trigger `release.yml` via `workflow_dispatch` against a tagged commit
   to validate end-to-end before the next real release.

---

## Mobile store distribution (release.yml `mobile_android` / `mobile_ios`)

Required to push signed builds straight to **Play Internal Testing** and
**TestFlight** without a human in the loop. Independent of the signing
secrets above — when these are absent the build still attaches to the
GitHub release; only the store-promotion step is skipped.

### Google Play

| Secret | Purpose | Format | Rotation |
|---|---|---|---|
| `GOOGLE_PLAY_SERVICE_ACCOUNT_JSON` | Service-account JSON for the Google Play Developer API. Must hold the **Release Manager** role (or narrower: "Release to testing tracks") on `ai.librefang.app` | Raw JSON (paste contents directly) | When personnel changes or the SA key is rotated in GCP |

**Generation reference**

1. Play Console → Setup → API access → link a Google Cloud project.
2. In GCP, create a service account, generate a JSON key, download it.
3. Back in Play Console → API access → grant the SA the *Release Manager*
   role (or the narrower "Release to testing tracks" if you want the
   automation locked out of production).
4. Paste the JSON contents into `GOOGLE_PLAY_SERVICE_ACCOUNT_JSON`. No
   base64 step required — the secret store handles multi-line values.

### Apple TestFlight (App Store Connect API)

| Secret | Purpose | Format | Rotation |
|---|---|---|---|
| `APPLE_API_KEY_ID` | App Store Connect API key identifier (10-char string) | plain string | When the key is revoked |
| `APPLE_API_KEY_ISSUER_ID` | Issuer ID of the Apple developer team (UUID) | plain string | n/a |
| `APPLE_API_KEY_P8` | PKCS8-formatted private key contents (`-----BEGIN PRIVATE KEY----- … -----END PRIVATE KEY-----`) | raw `.p8` text incl. BEGIN/END | Rotate yearly or on personnel change |

**Generation reference**

1. App Store Connect → Users and Access → Integrations → App Store
   Connect API → Generate a key with **App Manager** role.
2. Download the `.p8` (one-time download — keep an offline backup).
3. Copy the issuer ID from the same page.
4. Paste the raw `.p8` contents (including the BEGIN / END lines) into
   `APPLE_API_KEY_P8`. GitHub Actions accepts multi-line secret values;
   no base64 step is required.

API-key auth is preferred over Apple-ID + app-specific password: the key
is revocable per-key without 2FA prompts and does not inherit the human
owner's account scope.

---

## Desktop signing (release.yml `desktop`)

| Secret | Purpose |
|---|---|
| `MAC_CERT_BASE64` | macOS Developer ID Application cert (.p12, base64) for signing the Tauri desktop bundles |
| `MAC_CERT_PASSWORD` | Password for the .p12 above |
| `MAC_NOTARIZE_APPLE_ID` | Apple ID used for `notarytool submit` |
| `MAC_NOTARIZE_PASSWORD` | App-specific password for that Apple ID |
| `MAC_NOTARIZE_TEAM_ID` | Apple team ID for notarisation |
| `TAURI_SIGNING_PRIVATE_KEY` | Tauri updater signing key (minisign secret-key file contents) — DO NOT confuse with the Apple Developer cert; this signs auto-update manifests. BossFang fork uses a fresh minisign keypair (public half lives in `crates/librefang-desktop/tauri.desktop.conf.json`), not upstream's. |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Passphrase for the updater key. Empty string is acceptable if the keypair was generated without a passphrase (`minisign -GW`). Leave the secret unset to mean empty — `secrets.X` evaluates to `""` for missing names. |

---

## Package registries

| Secret | Purpose |
|---|---|
| `NPM_TOKEN` | Publishes `@librefang/*` packages to npm |
| `PYPI_API_TOKEN` | Legacy fallback only — primary path is OIDC trusted-publishing |
| `CARGO_REGISTRY_TOKEN` | Publishes `librefang-sdk` (and friends) to crates.io |

PyPI uses GitHub OIDC trusted publishing where possible — keep the API
token only as a break-glass option.

---

## Internal infrastructure

| Secret | Purpose |
|---|---|
| `HOMEBREW_TAP_TOKEN` | PAT with `contents:write` on `librefang/homebrew-tap` for `sync_homebrew` / `sync_homebrew_cask` |
| `RAILWAY_TOKEN` / `RENDER_API_KEY` / `FLY_API_TOKEN` | One-click deploy preview environments triggered by `release.yml` |

---

## Operational rules

- **Never echo a secret.** GitHub Actions masks known secret values, but
  one `set -x` upstream of a `cat keystore.jks` will leak the binary —
  always pipe through `base64 --decode > "$RUNNER_TEMP/..."` directly.
- **Wipe runner copies.** Every workflow that materialises a secret to
  disk (`$RUNNER_TEMP/cert.p12`, `$RUNNER_TEMP/release.jks`, etc.) must
  end with an `if: always()` cleanup step so a build cancellation does
  not leave the artifact on a self-hosted runner.
- **Forks shouldn't fail.** All mobile and desktop signing steps are
  guarded by an "is this secret present?" check that downgrades to an
  unsigned build instead of failing the job. This keeps the smoke build
  meaningful for external contributors.
- **Rotate on personnel change.** When a maintainer with secret access
  leaves, rotate `*_PASSWORD` and any PAT-backed secrets within a week.

When in doubt, prefer adding a new secret over reusing an existing one
with overloaded scope — clarity at rotation time is worth the small
extra cost in the secret store.
