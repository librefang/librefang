# [High] `KernelConfig` has ~100 fields; `build_reload_plan` covers only 40 — new fields silently noop on `POST /api/config/reload`

**Severity:** High · **Domain:** Architecture · **Source:** `audit-06-architecture.md`

## Location
`crates/librefang-kernel/src/config_reload.rs:188` checks 40 distinct `old.<field>` against new.

Unchecked (incomplete list): `trusted_hosts`, `trusted_proxies`, `trust_forwarded_for`, `agent_max_iterations`, `max_history_messages`, `auto_dream`, `context_engine`, `audit`, `telemetry`, `rate_limit`, `tool_timeout_secs`, `terminal`, `prompt_caching`, `session`, `compaction`, `task_board`, `vertex_ai`, `azure_openai`, `oauth`, `pairing`, `auth_profiles` (~30 more).

## Problem
When a contributor adds a config field, it must also be added to `build_reload_plan` for `/api/config/reload` to honor it at runtime. There's no compile-time or test-time enforcement — silent noop on reload is the default failure mode.

## Fix
**Reflection test** that asserts every `KernelConfig` field is classified by `build_reload_plan`:

```rust
#[test]
fn every_config_field_is_reload_classified() {
    let fields: HashSet<&str> = KernelConfig::field_names();
    let covered = build_reload_plan_field_coverage();
    let missing: Vec<_> = fields.difference(&covered).collect();
    assert!(missing.is_empty(), "fields not classified in build_reload_plan: {missing:?}");
}
```

Either use `serde` introspection or a `field_names!()` macro on `KernelConfig`. Each field must be tagged as one of `RequiresRestart`, `HotReload`, or `Ignore`.

Backfill the ~60 missing fields with the right classification.

## Tests
- The reflection test above.
- For each `HotReload` field, an integration test that POSTs `/api/config/reload` with a changed value and asserts the runtime picks it up.
