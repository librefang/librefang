# Passkey (WebAuthn/FIDO2) login (#5981)

Passkeys let an operator sign in to the dashboard with a platform authenticator (Touch ID, Face ID, Windows Hello, Android biometrics) or a roaming security key instead of typing a password.
It is an **additive** login method: username/password login keeps working unchanged, and passkeys are **opt-in** per deployment.

## Enabling

Passkeys are off by default.
Turn them on in `~/.librefang/config.toml`:

```toml
passkey_enabled = true
passkey_rp_id = "librefang.example.com"
passkey_rp_origin = "https://librefang.example.com"
```

These three fields require a restart to take effect — the `Webauthn` instance is built once at boot, and `POST /api/config/reload` reports them as restart-required rather than silently no-op-ing (see `docs/operations/config-reload.md`).

### RP-ID and origin

`passkey_rp_id` is the WebAuthn Relying Party ID — the registrable domain the dashboard is served from, with no scheme and no port (`librefang.example.com`, or `localhost` for local development).
`passkey_rp_origin` is the full scheme+host+port the browser loads the dashboard from (`https://librefang.example.com`).
The RP-ID must be the effective domain of the origin; a passkey is cryptographically bound to its RP-ID and stops working if that value changes, so set both explicitly in production.

Convenience fallbacks for local development:

- Only `passkey_rp_id` set → origin defaults to `http://<rp_id>`.
- Only `passkey_rp_origin` set → RP-ID is derived from the origin host.
- Neither set → the engine fails to build, a `WARN` is logged, and the `/api/auth/passkey/*` endpoints answer `503`. Password login is unaffected.

WebAuthn requires a secure context: browsers permit `http://localhost` for development, but any non-localhost deployment must serve the dashboard over HTTPS or the authenticator prompt never appears.

## Flow

The two standard WebAuthn ceremonies are each two requests.
In-flight challenge state lives in a short-TTL (5 minute) in-memory map keyed by an opaque `ceremony_id` handed to the browser and echoed back on verify; it is never persisted (a half-finished ceremony is worthless after a restart).

### Registration (add a passkey)

Gated behind an authenticated Owner session — you add a passkey from Settings → Security.

1. `POST /api/auth/passkey/registration-options` → `{ ceremony_id, options }` (the `PublicKeyCredentialCreationOptions`).
2. Browser calls `navigator.credentials.create(options)` (via `@simplewebauthn/browser`).
3. `POST /api/auth/passkey/registration-verify` with `{ ceremony_id, credential, label? }` → the server verifies the attestation and stores the credential.

### Authentication (sign in)

Public — no session exists yet.

1. `POST /api/auth/passkey/authentication-options` → `{ ceremony_id, options }` (the `PublicKeyCredentialRequestOptions`).
2. Browser calls `navigator.credentials.get(options)`.
3. `POST /api/auth/passkey/authentication-verify` with `{ ceremony_id, credential }` → the server verifies the assertion, persists the updated sign-count, and mints a session token **identical to `dashboard_login`** (same `{ok, token, created_at, expires_at}` body and `Set-Cookie`), so middleware, RBAC, logout, and the frontend Bearer flow all work unchanged.

### Credential management

- `GET /api/auth/passkey/credentials` — list the account's registered passkeys (metadata only; the credential blob is never exposed).
- `DELETE /api/auth/passkey/credentials/{id}` — revoke one, scoped to the authenticated principal.

## TOTP interaction

A passkey is phishing-resistant possession and, with user verification, inherence — it already satisfies the second-factor bar on its own.
A successful passkey assertion therefore mints the session directly and does **not** trigger the password-path TOTP challenge.
Operators who want a typed second factor should keep using password login, where TOTP enforcement (when configured) still applies.

## Authorization

- The two `authentication-*` endpoints are in the middleware public-route allowlist (they run before any session exists) and are rate-limited as a login brute-force surface alongside `dashboard-login`.
- The `registration-*` endpoints and `DELETE …/credentials/{id}` are Owner-only (`is_owner_only_write`): registering or revoking a passkey changes who can log in, the same trust boundary as TOTP enrollment.
- The `GET …/credentials` list is a normal authenticated read.

## Identity binding

Credentials bind to the **same principal** the password login produces (the resolved `dashboard_user`).
Each stored row carries its `user_name` to stay forward-compatible with the multi-user `[[users]]` path.
The WebAuthn user handle is a stable UUIDv5 derived from the principal name, so the same operator always maps to the same handle across registrations.

## Storage

Registered credentials live in the `webauthn_credentials` table (SQLite migration v44) on the shared memory substrate pool, so they survive a daemon restart.
The whole serialized `webauthn-rs` `Passkey` is stored opaquely in the `cred` column so the updated `sign_count` can be persisted after each assertion — a sign-count regression is how `webauthn-rs` detects a cloned authenticator.

## Browser support

| Browser | Minimum version |
|---|---|
| iOS Safari | 16 |
| Chrome for Android | 108 |
| Chrome / Edge desktop | 108 |
| Safari desktop | 16 |
| Firefox desktop | 122 |

The dashboard hides the passkey UI entirely when `window.PublicKeyCredential` is absent.

## Dependencies

- Server: [`webauthn-rs`](https://github.com/kanidm/webauthn-rs) 0.5.x.
- Dashboard: [`@simplewebauthn/browser`](https://simplewebauthn.dev) 13.x (base64url ↔ ArrayBuffer handling around the native `navigator.credentials` API).
