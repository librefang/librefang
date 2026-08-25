Ephemeral workers can now be launched from outside a running agent turn.
`POST /api/agents/spawn-ephemeral` is the HTTP entry point to the spawn engine that #7875 shipped, and the Agent Types page gains a Quick Run control that calls it — pick the agent to run on behalf of, type the task, read the answer.
Everything the engine guarantees still holds because the route adds no policy of its own: the advertised tool set is the executable one, the spend and the `[resources]` quota land on the parent you chose, and the recursion bound is the shared `max_agent_call_depth`.
  Resolving an agent type by name now also searches the writable `agent-types/` store, which is where `POST /api/templates` and the `agent_type_create` tool have been writing all along.
  Without that, every type the dashboard can create was invisible to the one engine whose job is to run it, and Quick Run would have failed on the entire catalog the page renders.
  (#7903) (@houko)
