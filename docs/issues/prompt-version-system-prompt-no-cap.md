# `PromptVersion` accepts an arbitrary-size `system_prompt` from the client and allows directly setting `is_active`

**Severity:** Medium
**Category:** Input validation
**Labels:** `validation`, `cost-amplification`, `medium`

## Affected files
- `crates/librefang-api/src/routes/prompts.rs:84-114` (`create_prompt_version`)
- `crates/librefang-types/src/agent.rs:1837-1858` (`PromptVersion` type)

## Description

`version.system_prompt` has no length limit — only the 1 MB `RequestBodyLimitLayer` acts as a backstop. Consequences:

- The write is hashed and stored;
- Once `is_active = true`, **every** LLM call carries this prompt → token-cost amplification attack;
- Both `is_active` and `version` are marked `#[serde(default)]`, so a client can POST `{"system_prompt": "...", "is_active": true}` — the server does not even check that an active version already exists;
- The same issue applies to `PromptExperiment` (`prompts.rs:206`).

## Recommendation

1. Add constants:

```rust
const MAX_SYSTEM_PROMPT_BYTES: usize = 32 * 1024;
const MAX_SYSTEM_PROMPT_CHARS: usize = 16 * 1024;
```

2. The create handler **ignores** the client's `is_active` (it can only be set via the `/activate` endpoint);
3. **Ignore** the client's `version`; the server numbers versions monotonically.
