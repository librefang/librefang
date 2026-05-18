# `check_json_depth` is defined but no handler calls it

**Severity:** Medium
**Category:** Input validation
**Labels:** `validation`, `dos`, `medium`

## Affected files
- Definition: `crates/librefang-api/src/validation.rs:143-180`
- Zero call sites: `rg "check_json_depth\(" crates/librefang-api/src/routes/` returns nothing
- 11+ `Json<serde_json::Value>` routes:
  - `routes/skills.rs:2940, 3181, 4140, 4302, 4742, 5106, 5153, 5273`
  - `routes/network.rs:380, 875, 1372`
  - `routes/workflows.rs:540-2926`

## Description

`serde_json` does not limit deserialization depth on its own. Deeply nested `[[[[…]]]]` flows through the axum body → `Value` path and recurses through downstream consumers (Cypher conversion in memory routes, plugin config, etc.). `lib.rs:8`'s `#![recursion_limit = "256"]` applies only to macros — not to runtime JSON.

The guard is written, but **wired into nothing**.

## Recommendation

Pick one:

1. Write a tower layer that calls `check_json_depth(&body, 32)` on every `application/json` request before the handler sees it;
2. At minimum, explicitly call it from hot-path handlers (`skills`, `workflows`, `webhooks`).

Integration test: POST a 100-level nested array; assert a 400 response.
