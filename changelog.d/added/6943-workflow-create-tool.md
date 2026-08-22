Give every agent a `workflow_create` tool so multi-step workflows can be authored directly from a conversation, without touching the canvas, CLI, or HTTP API.
The tool validates the name and runs the same `Workflow::validate()` semantic checks as `POST /api/workflows`, then hot-reloads the result into the engine so `workflow_run` can use it immediately.
The dashboard chat view also gained a "Save as Workflow" button that extracts a workflow JSON block from an agent's reply and hands it to the canvas.
A new `workflow-creator` prompt-only skill teaches agents when and how to design workflows with the tool (#6934) (@DaBlitzStein)
