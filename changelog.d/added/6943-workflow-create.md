Add the `workflow_create` tool so agents can create reusable workflows directly during a conversation — previously workflows could only be created via the dashboard canvas, CLI, or HTTP API. (#6943) (@DaBlitzStein)

The tool validates names (`[A-Za-z0-9_-]`, 1–64 chars) at both the runtime and kernel layers, runs the same `Workflow::validate()` semantic checks as the HTTP API, registers the workflow in the engine for immediate availability, and is always-native so every agent has it without configuration. (#6943) (@DaBlitzStein)

Add the `workflow-creator` skill — a promptonly skill teaching agents when and how to design multi-step workflows, with step structure, execution modes, error handling, and worked examples. (#6943) (@DaBlitzStein)

Dashboard: add a "Save as Workflow" button on agent chat messages that extracts a workflow definition from the message and opens it in the canvas, and show the workflow empty state instead of auto-redirecting to templates. (#6943) (@DaBlitzStein)
