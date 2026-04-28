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

## Desktop signing (release.yml `desktop`)

| Secret | Purpose |
|---|---|
| `MAC_CERT_BASE64` | macOS Developer ID Application cert (.p12, base64) for signing the Tauri desktop bundles |
| `MAC_CERT_PASSWORD` | Password for the .p12 above |
| `MAC_NOTARIZE_APPLE_ID` | Apple ID used for `notarytool submit` |
| `MAC_NOTARIZE_PASSWORD` | App-specific password for that Apple ID |
| `MAC_NOTARIZE_TEAM_ID` | Apple team ID for notarisation |
| `TAURI_SIGNING_PRIVATE_KEY` | Tauri updater signing key (PEM) — DO NOT confuse with the Apple Developer cert; this signs auto-update manifests |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Passphrase for the updater key |

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
