Record the agent type a step agent or an LLM-routed specialist was spawned from, not just one created through `POST /api/agents`.
`source_template` (#8018) was stamped on the HTTP spawn path alone, so an agent instantiated from a template by `resolve_step_agent` or by assistant routing showed a blank origin — indistinguishable in the dashboard from an agent that was never created from a template at all.
Both paths now stamp it the same way the HTTP route does.
The ephemeral-worker path deliberately does not: it registers no agent and persists no manifest, so there is nothing for the provenance to live on, and that is now written down where the next reader will look. (#8109) (@houko)
