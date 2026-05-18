# `[SILENT]` internal cron marker uses substring matching → user paste can trigger silent message loss

**Severity:** Low
**Category:** Kernel orchestration logic
**Labels:** `bug`, `low`

## Affected files
- `crates/librefang-kernel/src/kernel/agent_execution.rs:889-892, 1102`

## Description

```rust
if message.contains("[SILENT]") { ... }
```

uses a substring match against free-form user text. Real users pasting content that happens to contain a literal `[SILENT]` (log snippets, translation tables, internal-doc copy-paste) all trigger:

- `:890` skips the LLM-side payload mutation;
- `:1102` skips persistence;
- and that user turn **vanishes silently**.

## Recommendation

Introduce a structured marker instead of a string check:

```rust
// add a field to SenderContext
pub struct SenderContext {
    ...
    pub silent: bool,
}
```

As a less-invasive alternative: restrict the check to `is_internal_cron && message.starts_with("[SILENT]")` (prefix rather than arbitrary position), and prepend a control-character prefix that user paste can never produce.
