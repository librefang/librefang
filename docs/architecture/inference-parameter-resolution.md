# Inference-parameter resolution

Which setting wins when an agent, a model, and the system all have an opinion about `temperature`, and why the answer is different for `max_tokens` than it is for `context_window`.

Source of truth: `crates/librefang-types/src/inference_params.rs`.
The kernel calls it once per turn in `crates/librefang-kernel/src/kernel/agent_execution.rs`, after model routing has picked the final model.

## The case this exists for

An operator has one agent type, `writer`, and two instances of it: one creative, one formal for academic prose.
Both point at the same model.
The only difference is temperature.

That must work, and it did not.
Tuning the shared model's temperature overwrote both instances with the same value and discarded their individual settings without saying so.
The specific setting lost to the general one, which is backwards from what anyone expects.

## Two categories, opposite rules

### Preferences — how the agent should sound

`temperature`, `top_p`, `max_tokens`, `frequency_penalty`, `presence_penalty`.

Resolution: **agent manifest > per-model override > system default.**

Each is `Option` on `ModelConfig`, and `None` is a real state meaning "this agent has no opinion".
That state is what makes the ordering possible.
Before it existed every agent carried a concrete `4096` / `0.7` whether or not anyone chose those numbers, so letting the manifest win would have made per-model overrides unreachable for every agent in existence — the inverted priority was a workaround for the missing state, not a decision about precedence.

System defaults are `DEFAULT_MODEL_MAX_TOKENS` (4096) and `DEFAULT_MODEL_TEMPERATURE` (0.7).
The other three have no default: unset means the parameter is simply not sent.

### Endpoint facts — what the transport will accept

`reasoning_effort`, plus the `use_max_completion_tokens` / `force_max_tokens` / `no_system_role` transport flags.

Resolution: **model level only, and it wins.**

These are not preferences.
A gateway that rejects `reasoning_effort` rejects every turn that carries it — that was #7770, where a LiteLLM deployment took down every turn for an agent.
The model level has to be able to say "never send this here" and have that stick, so an agent cannot force one on, and an agent-level `extra_params["reasoning_effort"]` written by hand is *removed* when the model level does not set one.
Leaving it in place would let a stale manifest entry survive onto a model whose endpoint refuses the parameter, which is the same failure again.

### Limits — what the endpoint can do

`context_window`, `max_output_tokens`.

These describe capacity.
They are never merged into the request as sampling parameters, and they **never clamp**.

A `max_tokens` above a known limit produces a warning at save time — red in the editors, a `warnings` entry in the `PATCH /api/agents/{id}/config` response, a `WARN` in the log — and the value is stored and sent exactly as the operator typed it.

The reason is the failure mode of the alternative.
If the catalog's figure is the thing that is wrong, a silent truncation leaves the operator debugging a number they never chose, with nothing anywhere saying it was changed.
An explicit provider error names the problem in one line.
Being wrong loudly beats being wrong quietly.

## Known limits versus inferred ones

A limit only warns when something actually asserted it.

`ModelCatalogEntry::limits_known` is the discriminator, and it follows the same convention as the older `pricing_known`: absent means `true`, because registry entries predate the field and carry curated values.

| Source | `limits_known` | Warns? |
|---|---|---|
| `agent.toml: [model] context_window` / `max_output_tokens` | operator-set, always known | yes |
| Shipped registry / curated catalog entry | `true` | yes |
| Gateway that reported a capacity (LiteLLM `/model/info`, OpenRouter snapshot) | `true` | yes |
| `merge_discovered_models` placeholders (`131_072` / `16_384`) | `false` | **no** |
| `model_metadata.rs` L5 tail — substring table, `DEFAULT_GENERIC_CONTEXT`, `DEFAULT_ANTHROPIC_CONTEXT` | `false` | **no** |

A discovered model has no capacity to source: `DiscoveredModelInfo` has no such field, and the OpenAI-compatible `/v1/models` shape carries none either (#7780).
The daemon still needs a number for compaction and budget math, so a placeholder goes in — but nothing may present it to an operator as measured, and nothing may warn against it.
Warning someone for exceeding a ceiling that was invented from their model's *name* is noise, and noise is what trains people to stop reading warnings.

The same rule governs the editors' step ladders: a known cap trims the ladder so no unusable rung is offered, and an unknown cap trims nothing.

## Step ladders

`max_tokens` and `context_window` are chosen from presets rather than dragged on a slider.
The useful values are an order-of-magnitude sequence; the numbers in between mean nothing to any provider, and a slider gave no clue which values were sensible.
Both editors also offer `inherit` as a rung — "no opinion" is something you can point at rather than something you infer from a blank field — and a custom entry for the value that is not on the ladder.

- Context: 8K · 32K · 128K · 256K · 512K · 1M · 2M
- Output: 1K · 4K · 8K · 16K · 32K · 64K · 128K

The two ladders are deliberately different and the output one stops at 128K.
Output tokens are not context tokens: Gemini's 1M / 2M are how much it can *read*, and no model generates a million tokens of reply.
A ladder offering 1M output would assert that the value is valid, which is worse than the slider it replaced.

Defined in `CONTEXT_WINDOW_LADDER` / `MAX_OUTPUT_TOKENS_LADDER` (Rust, used by the TUI) and mirrored in `crates/librefang-api/dashboard/src/lib/modelParamLadders.ts` (used by the dashboard).
Change both together; `modelParamLadders.test.ts` pins the values so a one-sided edit fails.

## Surfaces

All four set the same seven knobs on the same tri-state contract.

| Surface | Where |
|---|---|
| API | `PATCH /api/agents/{id}/config` — omit a key to leave it unchanged, send `null` to inherit, send a value to pin |
| WebUI | `AgentManifestForm` (agent editor and agent-type editor) |
| TUI | Agent detail → `p`, in `tui/screens/model_params.rs` |
| CLI | `librefang agent set <id> <field> <value>`, with `inherit` as the clearing value |

## Migration

Moving the fields to `Option` does not change behaviour for an already-deployed agent.
Every persisted manifest carries explicit numbers — the old type had no way to omit them — so they deserialize as `Some(n)` and stay pinned.
The inherit state appears only on newly created agents and where an operator chooses it.

What *does* change, deliberately, is which value wins when both an agent and a per-model override are set: the agent's, now.
That only affects deployments that had set a per-model override, and it is the bug this work exists to fix.
