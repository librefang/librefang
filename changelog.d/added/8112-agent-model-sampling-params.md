The agent editor exposes the remaining sampling parameters as typed fields: context window, `top_p`, `frequency_penalty`, and `presence_penalty`.
`top_p` and the penalties are real `ModelConfig` fields now instead of untyped `extra_params` keys, so the manifest form, the TOML, and the request body all express one validated value.
OpenAI-compatible providers receive them by flattening `extra_body` and Ollama receives them nested under its native `options` object, rather than letting an operator set a knob that a typed-body driver will quietly drop.
`reasoning_effort` stays a per-model setting, not an agent manifest field, matching the endpoint-fact rule from #7770.
A template name on `POST /api/agents` now also resolves from the `agent-types/` store, preferring it on a collision with a live agent's manifest exactly as the template catalog does.
A template that exists but cannot be read is reported as a server-side failure rather than as a missing template, and that verdict no longer depends on the caller's language: the status is decided where the error is raised instead of by matching English substrings of an already-translated message.
Out-of-range sampling values are refused rather than written into the manifest TOML, so an operator learns the number was rejected instead of finding a value they never chose.
(#8112) (@DaBlitzStein)
