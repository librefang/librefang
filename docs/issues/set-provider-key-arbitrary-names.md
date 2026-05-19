# [Medium] `set_provider_key` accepts arbitrary provider names → plants arbitrary env vars

**Severity:** Medium · **Domain:** Auth & secrets · **Source:** `audit-01-auth-secrets.md`

## Location
`crates/librefang-api/src/routes/providers.rs:1037-1073`

## Problem
Derivation: `format!("{}_API_KEY", name.to_uppercase().replace('-', "_"))`. No length cap, no charset cap. An authenticated Admin can:
- Plant `STRIPE_API_KEY` / `SENDGRID_API_KEY` into the live `std::env` and `secrets.env`, silently re-targeting any third-party crate that reads them.
- Submit `name = "a".repeat(1_000_000)` to write a 1 MB env var.

## Fix
- Restrict `name` to `[a-z0-9-]{1,64}` (matches catalog naming).
- Require resulting `env_var` to be either in the known catalog or match `^[A-Z][A-Z0-9_]{0,63}_API_KEY$`.
- Apply same restriction to `delete_provider_key`.

## Tests
- `name = "stripe"` → 400 "unknown provider".
- `name = "a".repeat(1000)` → 400 "too long".
- Legit `name = "openai"` → 200.
