The agent editor exposes the remaining sampling parameters as typed fields: context window, `top_p`, `frequency_penalty`, and `presence_penalty`.
`top_p` and the penalties are real `ModelConfig` fields now instead of untyped `extra_params` keys, so the manifest form, the TOML, and the request body all express one validated value.
Only providers that flatten `extra_body` actually receive them — every OpenAI-compatible provider plus Ollama — so the three fields now carry a hint saying so, rather than letting an operator set a knob that a typed-body driver will quietly drop.
`reasoning_effort` stays a per-model setting, not an agent manifest field, matching the endpoint-fact rule from #7770.
A template name on `POST /api/agents` now also resolves from the `agent-types/` store, preferring it on a collision with a live agent's manifest exactly as the template catalog does.
A template that exists but cannot be read is reported as a server-side failure rather than as a missing template, and that verdict no longer depends on the caller's language: the status is decided where the error is raised instead of by matching English substrings of an already-translated message.
Out-of-range sampling values are clamped at save time instead of being written into the manifest TOML.
(#8112) (@DaBlitzStein)
