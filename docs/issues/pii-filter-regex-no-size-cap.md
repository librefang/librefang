# `PiiFilter::new` compiles operator-supplied regexes without a size or timeout cap

**Severity:** Medium
**Category:** DoS / resource exhaustion
**Labels:** `dos`, `regex`, `medium`

## Affected files
- `crates/librefang-runtime/src/pii_filter.rs:81-100`
- Similar: `crates/librefang-kernel/src/orchestration.rs:239` (`regex_lite`)
- Similar: `crates/librefang-runtime/src/context_engine/scriptable/mod.rs:415`

## Description

`Regex::new(pat)` is invoked directly on arbitrary operator-supplied patterns. The `regex` crate is RE2 — matching is linear — but **compilation** has no size cap:

- Alternation classes like `(a|a|...|a){50}` push the parser into O(pattern_size²) or worse compile loops;
- There is no per-match timeout.

`regex_lite` is even looser than `regex` and is more exposed.

## Recommendation

Use `RegexBuilder` with bounds at every operator/agent-controlled compilation site:

```rust
let re = RegexBuilder::new(pat)
    .size_limit(1 << 20)     // 1 MiB NFA
    .dfa_size_limit(1 << 20) // 1 MiB DFA
    .build()?;
```

Patterns that exceed the cap should be rejected at config-load time with an error.
