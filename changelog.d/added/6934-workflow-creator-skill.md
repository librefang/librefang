Ship a `workflow-creator` skill so an agent reaching for `workflow_create` knows the shape of a good workflow rather than guessing at one.
The tool has been available since #7857, but its JSON schema can only describe fields — not when a workflow beats a one-off `agent_send`, why `{"type": "researcher"}` binding makes a workflow portable to an instance where nothing is pre-registered, or which of the creation-time validations an author is most likely to trip.
Installing it from the registry puts that in the system prompt.
Two step fields the tool had always accepted are now advertised as well: `required_skills`, which fails a step before it bills an LLM call, and the per-step `session_mode` (#7873) (#6934) (@houko)
