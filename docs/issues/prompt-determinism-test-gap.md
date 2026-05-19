# [High] Prompt-determinism boundary: only OpenAI has byte-identical-under-permutation test

**Severity:** High · **Domain:** Test coverage · **Source:** `audit-07-test-coverage.md`

## Location
- Present: `extra_body_merge_is_byte_identical_across_insertion_orders` at `crates/librefang-llm-drivers/src/drivers/openai.rs:3500`
- Missing equivalent in: `anthropic.rs`, `gemini.rs`, `bedrock.rs`, `vertex.rs`, `groq.rs`
- Missing for whole-prompt assembly: `prompt_builder.rs::build_system_prompt`
- Missing for workspace rendering: `ensure_named_workspaces`

## Problem
The #3298 deterministic-ordering invariant (HashMap iteration order silently invalidates provider prompt caches) is critical to LibreFang's economics. The kernel-side regression tests cover the MCP summary path (`kernel/tests.rs:5350-5535`), but every other prompt-assembly boundary relies on code review alone to catch regressions.

Any future PR that introduces a `HashMap` / `HashSet` in a prompt-shaping path silently doubles the daily LLM bill and the test suite catches nothing.

## Fix
Lift the OpenAI test pattern to every prompt-assembly boundary:

```rust
#[test]
fn anthropic_request_body_is_byte_identical_across_input_permutations() {
    let inputs = build_test_inputs();
    let canonical = anthropic::convert_request(&inputs);
    for permutation in inputs.permutations() {
        let body = anthropic::convert_request(&permutation);
        assert_eq!(canonical, body);
    }
}
```

One test per driver. One test per non-driver prompt-assembly site (`build_system_prompt`, `ensure_named_workspaces`, tool registry stringification).

## Tests
- The tests above.
- Property test (proptest) that random shuffles of input maps/sets never change the serialized output bytes.
