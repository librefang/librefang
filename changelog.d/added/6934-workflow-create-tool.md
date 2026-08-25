Agents can now define a workflow during a conversation with the new `workflow_create` tool, instead of workflow authoring being reachable only from the dashboard canvas, the CLI, or the HTTP API.
A workflow an agent writes is registered immediately and outlives the turn, so the next `workflow_run` — from that agent or any other — can reach it.
Names stay unique: the check and the registration are one atomic operation inside the engine, so two agents proposing the same name concurrently cannot both succeed and leave a workflow that name-based lookup resolves to at random (#7857) (@houko)
