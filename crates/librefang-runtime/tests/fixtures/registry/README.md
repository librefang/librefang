# Registry fixture snapshot (tests only)

Pinned snapshot of [librefang/librefang-registry](https://github.com/librefang/librefang-registry) at commit `89d0e4c8b3abd20c5604f1d99d667257f488ac7e` (2026-06-12), pruned to what the test suites load: `providers/`, `mcp/`, `hands/` (HAND.toml / SKILL*.md), `agents/`, `channels/`, `workflows/`, `aliases.toml`, `schema.toml`.

Consumed by `librefang_runtime::registry_sync::seed_registry_fixture_for_tests`, which copies it into a test home's `registry/` cache and fans content out exactly like a real sync — no network, deterministic under `LIBREFANG_REGISTRY_OFFLINE=1`.

To refresh: clone the registry, re-copy the directories above, prune `hands/**/README.md`, and update the pinned commit here. Tests that assert on specific catalog entries (e.g. `catalog_get("github")`, `test_zai_models`) define what the snapshot must contain.
