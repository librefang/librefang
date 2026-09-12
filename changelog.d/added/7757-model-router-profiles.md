Add complexity-based model routing so an agent can pick a model per turn instead of being pinned to one in its manifest.
The common deployment reality is a cheap model that handles most turns and an expensive one that should only see the hard ones; until now that choice was static per agent, so operators either overpaid on every trivial turn or underserved the hard ones.
A `ModelProfile` binds task tags to a provider/model pair, a cost tier and a complexity ceiling; the kernel scores each turn from its text and picks the best profile the agent is permitted to use.
Builtin profiles ship as an asset and are overridden per-name from `~/.librefang/model_profiles.toml`, which is re-read when its mtime moves so an edit needs no restart.
Off by default: set `[model_router] enabled = true` in `config.toml` and `mode = "flexible"` in an agent's `[model]` block.
Per-agent constraints live in `[model.router_override]` — `allowed_profiles` limits the choice, `cost_budget` caps the tier, `default_profile` is the fallback, and `fixed = true` opts the agent out entirely.
A fallback profile is re-checked against those same constraints, so a `default_profile` can never spend past an agent's cost budget.
Configurable from all four surfaces: `GET`/`PUT /api/agents/{id}/model_routing` and `GET /api/model-router/profiles` on the API, a Routing tab on the dashboard's agent detail, an `r` editor on the TUI agent detail screen, and `librefang agent routing` / `routing-set` / `routing-profiles` on the CLI (#7757) (@DaBlitzStein)
