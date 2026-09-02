The `workflow_*` tools are now injected into every agent's tool set, mirroring the `skill_evolve_*` gate.
No named profile (research, coding, messaging, automation, minimal) lists any of them, so a profile-scoped agent never saw one in the set handed to the model, and a model that named one anyway got "Unknown tool" back from the dispatcher.
`ALWAYS_NATIVE_TOOLS` did not help: it controls which schemas ship eagerly in lazy-load mode, not which tools an agent has.
The whole family goes in rather than `workflow_create` alone, because that tool's schema tells the model to call `workflow_list` before choosing a name and says the result runs via `workflow_run` / `workflow_start`.
A narrow `capabilities.tools` declaration is overridden as well, on the same reasoning as the evolve gate; per-agent `tool_blocklist`, a non-empty `tool_allowlist` and the kernel-wide `tool_policy` all still apply afterwards, so operators keep an opt-out. (#7407) (@DaBlitzStein)
