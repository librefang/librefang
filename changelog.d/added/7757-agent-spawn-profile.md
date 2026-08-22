`agent_spawn` gains a `profile` parameter that pins the spawned agent onto a named model profile.
This is the case model profiles were built for: the goal loop auto-spawns helpers, and without it a verifier whose only job is to answer "is this done?" inherits its parent's expensive model.
Passing `profile = "quick"` now writes that profile's provider and model onto the child's manifest instead of letting it inherit the default.
Naming a profile that does not exist fails with the list of profiles that do, rather than spawning an agent on the wrong model.
The parameter is honoured whether or not `[model_router] enabled` is set: that switch governs the automatic per-turn router, while naming a profile here is an explicit choice, so a cheap sub-agent does not require turning on routing for every agent (#7757) (@DaBlitzStein)
