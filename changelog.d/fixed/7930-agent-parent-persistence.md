Persist an agent's parent link, so the API stops reporting every agent as parentless after a daemon restart.
`AgentEntry.parent` recorded which agent spawned another but had no column in the `agents` table, so the reload path reconstructed it as `None` and `parent_agent_id` came back `null` for agents that demonstrably had a parent — a populated field asserting a lineage that was not theirs, rather than an absent one a client could ignore.
Spend attribution felt it first: the kernel bills a spawned worker's usage to `parent.unwrap_or(id)`, so a restarted worker silently began billing itself instead of its spawner.
Schema v54 adds `agents.parent_id` with an index over it, and derives `children` from those edges instead of storing a second copy of the same relationship that nothing kept in step.
A row written before the migration reports its lineage as unknown rather than quietly promoting itself to a root agent. (#7931) (@houko)
