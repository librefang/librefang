# `MAX_MESSAGE_SIZE` uses byte length instead of character count → CJK users hit the cap too early

**Severity:** Medium
**Category:** Input validation
**Labels:** `i18n`, `validation`, `medium`

## Affected files
- `crates/librefang-api/src/routes/agents.rs:1673-1674, 2681-2682, 6283-6284`
- `crates/librefang-api/src/routes/skills.rs:3589-3590`
- `crates/librefang-api/src/routes/network.rs:2013-2018` (`comms_send`)

## Description

```rust
if req.message.len() > MAX_MESSAGE_SIZE { ... }
```

`.len()` is the UTF-8 byte count. 64KB of CJK ≈ 21K characters (3 bytes per glyph). Chinese / Japanese / Korean users are rejected at roughly one-third of the character budget that ASCII users get.

Inconsistency: `kernel/reviewer_sanitize.rs:50` and `kernel/mod.rs:304` use `chars().count()`.

From an LLM token-budget perspective, a byte cap is too strict for CJK and too loose for 1 MB of pure ASCII (it sidesteps tokenizer-aware limits). The same byte/char confusion appears in `web_content.rs:312` and the heuristics in `compactor.rs`.

## Recommendation

Pick one:

1. Document the cap as bytes ("64KB raw UTF-8") and add a complementary `chars().count()` check;
2. Switch to `req.message.chars().count() > MAX_CHARS`.

Error messages should show both byte count and character count to make diagnosis easier.
