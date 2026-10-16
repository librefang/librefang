# Reasoning-effort resolution and per-provider wire mapping

Refs #7946, #8591.

`reasoning_effort` names how hard the model should reason on a turn.
Its value is an **open token string** drawn from the industry-convergent superset — `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max` — or any token a model's catalog entry declares.
`provider-default` is the **absence** of a value, not a level: leaving `reasoning_effort` unset sends nothing, and no code path turns that into a token.

The feature exists because `budget_tokens` cannot express two things operators need.

It cannot say *do not reason at all*.
A sub-1024 budget merely omits the reasoning opt-in, and on a model that reasons by default — every DeepSeek V4 id — omitting the opt-in reads as consent.
Prompt-level workarounds ("answer fast, don't overthink") do not substitute: the reasoning tokens are still generated, still counted against the output budget, and still billed.
Non-think has to be a wire-level toggle or it is not a toggle.

It also cannot reach rungs that providers name themselves — `xhigh`, `minimal` — because four values and a bucket mapping cannot express a vocabulary that grows with the provider ecosystem.

## Four rules that shape everything below

1. **The token is data, not a description.** The level the operator picks travels to the wire verbatim; drivers choose *where* to put it, not *which rung it means*.
2. **The catalog's per-model policy is authoritative.** `supported_efforts` (with `default_effort`, `off_allowed`, `mandatory`) is the model's declared vocabulary. A request outside it is **coerced to the nearest supported rung with a warning** — never dropped silently, never forwarded, never answered with a 400 by us. `none` on a model that forbids off (`off_allowed = false` or `mandatory = true`) becomes the lowest graded rung.
3. **An entry that declares nothing keeps the pre-#8591 behaviour.** Undeclared means "no published vocabulary", not "supports everything": the legacy tokens keep their family spellings (`max` folded where the schema demands it) because that is what every pre-#8591 config was written against. Budget-based families — Gemini 2.5, older Anthropic, Ollama, Qwen's budget control — live in that state by design.
4. **Enum and budget are never parallel.** Where the model declares an effort enum, the effort is the control and the `budget_tokens` bucket is not consulted; where the model declares no enum, `budget_tokens` is the control (Ollama: the `think` boolean). The two never ride the same request.

## Naming, and the deprecated alias

The user-facing key is `reasoning_effort` everywhere (TOML, REST bodies, WS frames, session JSON).
`reasoning_mode` — the name this feature shipped under in #7946 — is accepted as a **deprecated alias**:

- TOML: `[thinking] reasoning_mode` deserializes into the same field via `#[serde(alias = …)]`; serialization always writes `reasoning_effort`.
- REST `MessageRequest`: same alias on the field. Sending both keys in one body is a serde duplicate-field error (422) — the alias resolves to the same field.
- WS frame and `PATCH /api/sessions/{id}/reasoning`: parsed by hand, so both keys are accepted and `reasoning_effort` wins when a caller sends both.
- The Rust type keeps the name `ReasoningMode` from #7946; renaming it would churn every driver wire site for no wire-visible change.

## Where it is set

| Rung | Location | Key |
|---|---|---|
| 1 (highest) | per message | `reasoning_effort` on the body of `POST /api/agents/{id}/message`, `POST /api/agents/{id}/message/stream`, or the WebSocket chat frame |
| 2 | per session | `PATCH /api/sessions/{id}/reasoning` — body `{"reasoning_effort": "xhigh"}`; `null` clears the pin |
| 3 | per agent | `agent.toml` → `[thinking] reasoning_effort` |
| 4 | global | `config.toml` → `[thinking] reasoning_effort` |
| 5 (lowest) | compiled default | none — the `[thinking] budget_tokens` bucket is used instead (undeclared models only) |

The per-agent knob lives in `agent.toml`, **not** `config.toml`.
`KernelConfig` has no `agents` field, so `[agents.<name>.thinking]` in `config.toml` parses and then reaches no manifest (CLAUDE.md #5476).

## Resolution order

Per-message > per-session > per-agent > global > compiled default, the same shape as `max_history_messages` and `session_mode`.

Mechanically, in the order the kernel performs it:

1. The manifest carries the agent's own `[thinking]` table, or `None`.
2. `messaging.rs` / `agent_execution.rs` backfill the global `[thinking]` table into the manifest **only when the manifest has none**, so a per-agent table beats the global one by suppressing the backfill entirely rather than by merging field-by-field.
3. The session snapshot's `reasoning_effort` (stored in the `sessions.reasoning_effort` column, migration v64) is stamped on via `manifest_helpers::apply_session_reasoning_effort`, after the backfill so it beats the agent/global level.
4. `manifest_helpers::apply_thinking_override` stamps the per-message override on top of whatever survived.
5. **Coercion runs last**, against the effective model: `lookup_reasoning_policy(&manifest.model.model)` then `coerce_reasoning_effort_to_model`. `ReasoningEffortPolicy::coerce` maps a token outside the declared set to the nearest supported rung (ties resolve downward — `xhigh` → `high`) and `none` to the floor when the model forbids off, logging a `warn!` whenever the token changed. An undeclared policy passes everything through.
6. Whatever is left in `manifest.thinking` is copied onto the `CompletionRequest`, together with the effective policy, and each driver re-applies the same coercion as a wire-side backstop for callers that build requests directly.

Tests: `test_reasoning_effort_global_reaches_an_agent_that_declares_nothing`, `test_reasoning_effort_per_agent_beats_global`, `test_reasoning_effort_per_call_beats_per_agent_beats_global`, `test_session_reasoning_effort_resolution_order`, `test_declared_reasoning_policy_coerces_the_manifest` in `crates/librefang-kernel/src/kernel/tests.rs`; `test_reasoning_policy_coerces_to_the_nearest_supported_effort` in `crates/librefang-types/src/config/mod.rs`.

### The two per-message keys

The message routes accept both the pre-existing boolean `thinking` and `reasoning_effort`.
`reasoning_effort` was added alongside the boolean rather than widening it, because a JSON `true` cannot grow a third value without breaking every client already sending it.
When both are present `reasoning_effort` wins — it is strictly more specific, and a client that has learned to send it is not also relying on the boolean to mean something else.

The two are not redundant. `thinking: false` clears the thinking config, which only *omits* the opt-in; `reasoning_effort: "none"` keeps a config carrying the mode, which is what lets the driver send the provider's explicit non-think toggle.
Collapsing the two would re-open the gap this feature closes — see `test_disable_and_mode_none_are_not_the_same_thing`.

`thinking: true` clears an inherited `reasoning_effort = "none"` — from the agent, the global config, or the session pin — rather than leaving it in place.
The boolean documents itself as "force thinking on even if the manifest has it off", and a non-think mode is exactly that off-state, so leaving it would make the boolean a silent no-op on any agent (or global config) that pins `none`.
A *graded* mode is left alone — the caller asked for reasoning, not for a particular amount of it.
See `test_enable_clears_an_inherited_non_think_mode` and `test_enable_leaves_a_graded_mode_alone`.

## The catalog policy

`ModelCatalogEntry` carries the per-model declaration, and `model_overrides.json` can extend or correct it field by field:

| Field | Meaning |
|---|---|
| `supported_efforts: Vec<String>` | The graded tokens the model accepts, verbatim. Empty = undeclared. `none` may be listed, but `off_allowed` is the authoritative bit for off. |
| `default_effort: Option<String>` | The model's own default rung, informational for selection surfaces and the fallback coercion target for a non-canonical token. **Not** injected when the caller left the effort unset — absence stays absence. |
| `off_allowed: bool` (default `true`) | Whether the model can be told not to reason. `false` removes `none` from the offer and coerces a requested `none` to the lowest graded rung. |
| `mandatory: bool` (default `false`) | Whether reasoning is mandatory (R1-like ids). Same effect on the offer and coercion as `off_allowed = false`; kept distinct because the wire reasons differ (no off toggle at all vs. an always-on model). |

`offered()` returns the tokens a UI may show: the graded set plus `none` when the model allows it — and nothing at all for an undeclared model, because an unknown model must not be offered a guessed rung.

## Per-provider wire mapping

Decided in one place, `reasoning_wire_fields` in `crates/librefang-llm-drivers/src/drivers/openai.rs`.
The family is chosen from `base_url`, not from the model id: `openrouter/deepseek/deepseek-v4-pro` and a direct `deepseek-v4-pro` are the same model behind two different dialects.

Every table below is read together with rule 2: the token shown is what reaches the wire *after* coercion, and an undeclared model takes the legacy rows.

### DeepSeek V4, direct (`ReasoningEchoPolicy::Echo`, non-OpenRouter `base_url`)

| Mode | Wire |
|---|---|
| `none` (allowed) | `"thinking": {"type": "disabled"}` |
| a graded token | `"thinking": {"type": "enabled"}`, `"reasoning_effort": <token verbatim>` |
| *unset* | neither field — see below |

The token is verbatim: `low` → `low`, `medium` → `medium`, `xhigh` → `xhigh`, `max` → `max`.
The pre-#8591 folds (`medium` / `xhigh` → `high`) applied only to the four-rung enum and are gone for any model with a declared policy; a model whose entry declares nothing keeps the legacy spellings, so old configs stay byte-identical.

With **no** explicit mode a V4 body is byte-for-byte what it was before #7946.
DeepSeek ignores `budget_tokens`, so deriving an effort from the budget would change the wire shape of every existing V4 request without changing what the model does.
Opting in is how the behaviour changes.

### OpenRouter-routed models (`base_url` contains `openrouter`)

Nested form only:

| Mode | Wire |
|---|---|
| `none` (allowed) | `"reasoning": {"effort": "none"}` |
| a graded token | `"reasoning": {"effort": <token verbatim>}` |
| *unset, undeclared model* | `"reasoning": {"effort": <budget bucket>}`, or nothing when the budget is under 1024 |
| *unset, declared model* | nothing — the enum is the control |

One legacy translation survives for an **undeclared** model: `max` → `xhigh`, because OpenRouter's own name for the top rung is `xhigh` and no model declared `max` before the vocabulary existed.
A model whose policy declares `max` gets `max` verbatim.

**OpenRouter answers HTTP 400 to a body carrying both `reasoning_effort` and `reasoning.effort`**, so the two are mutually exclusive per request and this branch never sets the top-level field.
The invariant is checked exhaustively over every (family × echo policy × mode × budget × rejected × policy) combination by `no_payload_ever_carries_both_reasoning_controls`, rather than case by case — a future branch that sets the wrong pair would otherwise surface only as a production 400 on one provider.

### Generic OpenAI-compatible endpoints (`ReasoningEchoPolicy::None`)

Top-level form only:

| Mode | Wire |
|---|---|
| `none` | field omitted |
| a graded token | `"reasoning_effort": <token verbatim>` — including `xhigh`, `medium`, `minimal` |
| *unset, undeclared model* | `"reasoning_effort": <budget bucket>`, or nothing under 1024 |
| *unset, declared model* | nothing — the enum is the control |

`none` is expressed by omission because OpenAI answers 400 to the literal `reasoning_effort: "none"`, and a gateway that reasons by *default* is exactly the case that has its own gate field and is handled above.
For an **undeclared** model `max` is still clamped to `high`, because the OpenAI schema has no rung above it and every pre-#8591 config pinning `max` was written against that clamp. A model whose policy declares `max` gets it verbatim.
If a declared token still draws a 400, the #7769 strip-and-retry handles it rather than a driver-side heuristic second-guessing the declaration.

### Providers that ignore the mode

| Family | Behaviour |
|---|---|
| Kimi / Moonshot (`ReasoningEchoPolicy::EmptyString`) | `"thinking": {"type": "disabled"}` regardless of mode. Thinking is disabled wire-side so multi-turn `tool_calls` work without round-tripping `reasoning_content` — a correctness requirement, not a preference. |
| DeepSeek R1 (`ReasoningEchoPolicy::Strip`) | Nothing sent, in **either** dialect — the `Strip` check short-circuits ahead of the OpenRouter branch, so an R1 route through OpenRouter is not handed a budget-derived `reasoning.effort` it never received before #7946. R1 reasons unconditionally and exposes no wire toggle, so the field would change the payload without changing the model. Pinned by `deepseek_r1_gets_no_reasoning_control_in_either_dialect`. |
| Gemini 2.5, Ollama, Qwen-with-budget drivers | Keep `budget_tokens` as the control (Ollama: the `think` boolean). Their wire control *is* a token budget rather than an effort enum, and remapping a configured budget into a rung would silently change what the operator is already paying for. `none` is the one mode they *do* honour, because it has to be: `apply_thinking_override` has to materialise a `ThinkingConfig` to carry the mode, and a bare `budget_tokens` read of that config would enable `think: true` (Ollama) for a turn that asked for the opposite — and bill it. Pinned by `build_request_think_field_false_for_reasoning_effort_none`. Gemini does not read `request.thinking` at all. |
| Anthropic driver, budgeted generation (Haiku 4.5 and older Claude models, and the non-Claude ids on Anthropic-compatible endpoints) | Same as the row above: `budget_tokens` goes on the wire as-is, no rung is derived from it, and `none` is honoured as off. Pinned by `reasoning_effort_none_does_not_enable_extended_thinking`. |

### The Anthropic driver's adaptive generation

Opus 4.6 and newer no longer have a token budget to send: `thinking.budget_tokens` was deprecated on the 4.6 pair and answers 400 from Opus 4.7 onwards, and the depth control that replaced it is `output_config.effort`.
The API's rungs are `low` / `medium` / `high` / `xhigh` / `max`, and **which of them a generation accepts is catalog data** — `xhigh` only from Opus 4.7 onwards.
So the driver emits the token verbatim when the model's policy declares it and coerces an out-of-set request to the nearest declared rung — the same rule the kernel and the OpenAI-compatible driver apply, so the two wires cannot disagree about what a request meant.
An entry that declares nothing keeps the pre-#8591 mapping — `low` / `high` / `max` only, and the open tokens are dropped with a warning — because sending `xhigh` to a generation that rejects it would be a 400 rather than a rung, and no model that predates the vocabulary should start receiving one.
`none` is still off, spelled `thinking: {"type": "disabled"}` on the ids that both reason unasked and accept that spelling and by omission everywhere else; when the model forbids off, the kernel has already coerced it to the lowest graded rung.

With no mode set, the driver sends **no** `output_config` at all rather than bucketing `budget_tokens` into one.
That keeps this table's original promise on the only wire where it can still be kept: `effort` defaults to `high` when the field is absent, and `reasoning_effort_for_budget` would map the compiled-in default budget of 10_000 to `medium`, so bucketing would make every agent that configured nothing reason *less* than an unconfigured request does — the same silent change of what the operator is paying for that the budget-bucket exemption existed to prevent.
A budget keeps the one job it can still do on this wire: below 1024 it means do not reason at all, the same convention `reasoning_effort_for_budget` applies everywhere else.
Pinned by `only_an_explicit_mode_pins_an_effort_rung`, `declared_vocabulary_coerces_the_adaptive_effort_rung`, and `thinking_is_turned_off_explicitly_only_where_that_is_both_needed_and_accepted` in `crates/librefang-llm-drivers/src/drivers/anthropic.rs`.

Which generation a model id belongs to is decided in one place, `crates/librefang-llm-drivers/src/drivers/anthropic_models.rs`.

## Interaction with the #7769 negative cache

An endpoint that has answered 400 to a reasoning control for a given model is recorded in `reasoning_effort_unsupported` and is not asked again — the answer is deterministic, so re-sending only buys a 400 and a retry every turn.
That suppression covers **both** spellings, and the strip-and-retry in `complete()` / `stream()` clears both, because an OpenRouter-routed body carries the nested object rather than the top-level field.
`is_unsupported_reasoning_effort_error` recognises both names too — matching only the literal `reasoning_effort` left the nested spelling with no strip-and-retry and no cache entry at all, so a gateway answering `unsupported parameter: reasoning` burned every retry and then failed the same way on every subsequent turn.
`reasoning_content` is excluded from that match: DeepSeek's demand that it be echoed back on `tool_calls` turns is a content requirement, and stripping a reasoning control would not answer it.

This is deliberately **not** the same failure as an out-of-set request: a parameter the endpoint does not know is a strip-and-retry + mute problem, while a rung the model does not carry is a coercion problem, and rule 2 handles it before the request is built.

One rough edge remains, deliberately: the `effort_rejected` short-circuit in `reasoning_wire_fields` returns *before* the `Echo` branch, so once a model is in the negative cache a V4 agent pinned to `reasoning_effort = "none"` stops receiving `thinking: {"type": "disabled"}` and silently reverts to reasoning-on.
Emitting the gate anyway would risk a permanent 400 on a gateway that does not understand `thinking` either — the retry loop clears only the two effort fields — so the conservative direction was kept.

## Bucket mapping (undeclared models only)

`reasoning_effort_for_budget`: `<1024` → none, `1024–4096` → `low`, `4097–16384` → `medium`, `>16384` → `high`.

The bucket is the compiled default rung for models that declare **no** enum; once a model declares one, the bucket is not consulted at all (rule 4), and an unset effort stays absent.
`medium` is a first-class token as well, but the bucket mapping itself is unchanged.

## Not exposed in the UI (this branch)

The dashboard agent-edit form and the TUI do not surface `reasoning_effort` yet; that work is tracked separately so this change stays a backend contract.
The contract they must honour when they do: call `ReasoningEffortPolicy::offered()` for the selected model and render exactly that list, with `none` absent when `off_allowed = false` or `mandatory = true`, and the session / per-message keys as the two writable surfaces above.
