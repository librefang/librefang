# WeChat push log emits the bot-token's first 10 characters + user ID

**Severity:** Medium
**Category:** Secrets & credential handling
**Labels:** `security`, `secrets`, `logging`, `medium`

## Affected files
- `crates/librefang-channels/src/wechat.rs:327-329`

## Description

```rust
debug!("WeChat ilink_send_text: …, bot_token_prefix={}...",
       &token.as_str().chars().take(10).collect::<String>())
```

WeChat tokens commonly carry a type prefix (e.g. `wxidp_<rand>`); leaking 10 characters meaningfully shrinks a confirmation-oracle attack surface. The same log line carries `to_user_id`, enabling correlation between leaked prefixes and captured ciphertext for account-targeted attacks.

`credential_pool.rs:111-124`'s `CredentialSnapshot::from_credential` already demonstrates the correct shape (`****<last4>`).

## Recommendation

Change the field to:

```rust
bot_token_fingerprint = format!("{:.8}", sha256(token))
```

Diagnostic value is unchanged; prefix leakage drops to zero.
