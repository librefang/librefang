# `safe_trim_messages` does not call the repair functions on `session_messages` after trimming

**Severity:** Medium
**Category:** Kernel orchestration logic
**Labels:** `bug`, `history`, `medium`

## Affected files
- `crates/librefang-runtime/src/agent_loop/message.rs:218-292`

## Description

When `session_messages.len() > max_history`:

- The function drains + re-inserts pinned messages (`:220-246`);
- It does **not** invoke `validate_and_repair` / `ensure_starts_with_user` on `session_messages`.

The LLM working copy `messages` does call both repair functions (`:287-292`), but the persisted blob is not repaired:

- The persisted blob may start with an assistant message;
- It may have a dangling `ToolUse`;
- The next daemon reload uses exactly this broken history;
- The "rescued pinned at position 0" re-insertion (line 244) — if the pinned message is an assistant turn with no subsequent user — already breaks the invariant.

## Recommendation

After trim + re-insertion, apply both repair functions to `session_messages`:

```rust
validate_and_repair(&mut session_messages)?;
ensure_starts_with_user(&mut session_messages);
```

Regression test: pin an assistant message, trim, reload, assert the loaded history starts with a user message.
