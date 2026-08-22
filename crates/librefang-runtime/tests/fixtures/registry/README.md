# Registry fixture snapshot (tests only)

Pinned snapshot of [librefang/librefang-registry](https://github.com/librefang/librefang-registry) at commit `89d0e4c8b3abd20c5604f1d99d667257f488ac7e` (2026-06-12), pruned to exactly what the test suites load:

- `providers/` — all provider TOMLs; `model_catalog` tests assert on named providers (cohere, xai, bedrock, zai, kimi2, alibaba-coding-plan, …) and on catalog-wide counts.
- `mcp/` — all catalog TOMLs; `extensions::catalog` asserts ≥ 20 entries and named entries (github, brave-search, exa-search).
- `hands/` — all hands, `HAND.toml` + `SKILL*.md` only (READMEs pruned); `load_hands_from_disk` asserts ≥ 15 and `kernel-router` routes to 14 of them by id.
- `agents/hello-world/agent.toml` — the template the default-agent dispatch path instantiates, under the directory name the registry serves today.
- `agent-types/hello-world/agent.toml` — byte-identical copy under the forward-compatible directory name, added so tests exercise `librefang_types::registry_paths::resolve_agent_types_dir` resolving the "both present, `agent-types/` wins" case even through the shared fixture-seeded home. See that module's own unit tests for the isolated single-directory and neither-present cases.
- `aliases.toml` — `model_catalog` alias-resolution tests.

Anything the tests don't read (channels, workflow templates, schema.toml, other agent templates, hand READMEs) is deliberately absent — don't re-add content without a test that needs it.

Consumed by `librefang_runtime::registry_sync::seed_registry_fixture_for_tests`, which copies it into a test home's `registry/` cache and fans content out exactly like a real sync — no network, deterministic under `LIBREFANG_REGISTRY_OFFLINE=1`.

To refresh: clone the registry, re-copy the directories above with the same pruning, and update the pinned commit here.
Tests that assert on specific entries define what the snapshot must contain.
