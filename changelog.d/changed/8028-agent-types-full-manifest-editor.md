The agent-types editor now writes the full agent manifest through the shared `AgentManifestForm`, backed by a new `PUT /api/templates/{name}/toml` endpoint that parses, validates, and persists raw TOML.
The endpoint enforces the same 1MB manifest cap as the agent spawn path, reports TOML keys the manifest schema does not recognize as `unknown_keys` in the save response (with a `WARN` log) instead of dropping them silently, and pins identity to the URL so a `name` edit in the body cannot move the document.
The dead `useUpdateAgentType` dashboard hook and `updateAgentType` client function are removed; the flat `PUT /api/templates/{name}` patch endpoint remains the contract for the agent-facing `agent_type_create` tool and the SDKs.
(#8028) (@DaBlitzStein)
