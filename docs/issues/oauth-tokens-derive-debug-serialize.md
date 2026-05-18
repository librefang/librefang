# `OAuthTokens` wraps cleartext tokens in `#[derive(Debug, Serialize)]`

**Severity:** High
**Category:** Secrets & credential handling
**Labels:** `security`, `secrets`, `type-safety`, `high`

## Affected files
- `crates/librefang-types/src/oauth.rs:41-57`

## Description

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    ...
}
```

`access_token_zeroizing()` / `refresh_token_zeroizing()` helpers already exist (same file, `:67-77`), but only protect the path when a caller remembers to use them. Every one of the following routes leaks cleartext:

- `tracing::debug!(?tokens, ...)` or `error!(?tokens, ...)`;
- `format!("{:?}", tokens)` and panic messages that include the type;
- `serde_json::to_string` flowing into snapshots / logs / dashboard payloads;
- cross-process trace dumps.

Protection lives entirely in caller discipline; there is no type-level enforcement. By comparison, `PooledCredential::Debug` in `credential_pool.rs:57-72` is hand-written and properly redacted — that is the correct pattern.

## Recommendation

1. Hand-write `Debug` to emit `<redacted len=N hint=****XXXX>` (mirroring `PooledCredential::Debug`).
2. Switch token fields to `secrecy::SecretString` or `Zeroizing<String>` so any incorrect serialization path **fails to compile**.
3. Split internal vs. wire types: `OAuthTokensWire` (response serialization only) vs. internal `OAuthTokens` (does not implement `Serialize`).
