# Reasoning-mode resolution and per-provider wire mapping

Refs #7946.

`reasoning_mode` names how hard the model should reason on a turn, in four rungs: `none`, `low`, `high`, `max`.
It exists because `budget_tokens` cannot express two things operators need.

It cannot say *do not reason at all*.
A sub-1024 budget merely omits the reasoning opt-in, and on a model that reasons by default — every DeepSeek V4 id — omitting the opt-in reads as consent.
Prompt-level workarounds ("answer fast, don't overthink") do not substitute: the reasoning tokens are still generated, still counted against the output budget, and still billed.
Non-think has to be a wire-level toggle or it is not a toggle.

It also cannot reach the top rung of providers that have one.
The bucket mapping in the OpenAI-compatible driver tops out at `high`, so `max` / `xhigh` was unreachable from any user-facing setting.

## Where it is set

| Rung | Location | Key |
|---|---|---|
| 1 (highest) | per call | `reasoning_mode` on the body of `POST /api/agents/{id}/message` and `POST /api/agents/{id}/message/stream` |
| 2 | per agent | `agent.toml` → `[thinking] reasoning_mode` |
| 3 | global | `config.toml` → `[thinking] reasoning_mode` |
| 4 (lowest) | compiled default | none — the `[thinking] budget_tokens` bucket is used instead |

The per-agent knob lives in `agent.toml`, **not** `config.toml`.
`KernelConfig` has no `agents` field, so `[agents.<name>.thinking]` in `config.toml` parses and then reaches no manifest (CLAUDE.md #5476).

## Resolution order

Per-call > per-agent > global > compiled default, the same shape as `max_history_messages` and `session_mode`.

Mechanically, in the order the kernel performs it:

1. The manifest carries the agent's own `[thinking]` table, or `None`.
2. `messaging.rs` / `agent_execution.rs` backfill the global `[thinking]` table into the manifest **only when the manifest has none**, so a per-agent table beats the global one by suppressing the backfill entirely rather than by merging field-by-field.
3. `manifest_helpers::apply_thinking_override` stamps the per-call override on top of whatever survived.
4. Whatever is left in `manifest.thinking` is copied onto the `CompletionRequest` and is the effective configuration for the turn.

Tests: `test_reasoning_mode_global_reaches_an_agent_that_declares_nothing`, `test_reasoning_mode_per_agent_beats_global`, `test_reasoning_mode_per_call_beats_per_agent_beats_global` in `crates/librefang-kernel/src/kernel/tests.rs`.

### The two per-call keys

`POST /api/agents/{id}/message` accepts both the pre-existing boolean `thinking` and the new `reasoning_mode`.
`reasoning_mode` was added alongside the boolean rather than widening it, because a JSON `true` cannot grow a third value without breaking every client already sending it.
When both are present `reasoning_mode` wins — it is strictly more specific, and a client that has learned to send it is not also relying on the boolean to mean something else.

The two are not redundant. `thinking: false` clears the thinking config, which only *omits* the opt-in; `reasoning_mode: "none"` keeps a config carrying the mode, which is what lets the driver send the provider's explicit non-think toggle.
Collapsing the two would re-open the gap this feature closes — see `test_disable_and_mode_none_are_not_the_same_thing`.

## Per-provider wire mapping

Decided in one place, `reasoning_wire_fields` in `crates/librefang-llm-drivers/src/drivers/openai.rs`.
The family is chosen from `base_url`, not from the model id: `openrouter/deepseek/deepseek-v4-pro` and a direct `deepseek-v4-pro` are the same model behind two different dialects.

### DeepSeek V4, direct (`ReasoningEchoPolicy::Echo`, non-OpenRouter `base_url`)

| Mode | Wire |
|---|---|
| `none` | `"thinking": {"type": "disabled"}` |
| `low` | `"thinking": {"type": "enabled"}`, `"reasoning_effort": "low"` |
| `high` | `"thinking": {"type": "enabled"}`, `"reasoning_effort": "high"` |
| `max` | `"thinking": {"type": "enabled"}`, `"reasoning_effort": "max"` |
| *unset* | neither field — see below |

V4's own docs accept these; the default is enabled at `high`, and the compatibility spellings `medium` / `xhigh` fold into `high`, so nothing needs to emit those.
The comment in `openai.rs` that used to claim "this API family rejects requests with unexpected reasoning fields" predated V4 and no longer described it.

With **no** explicit mode a V4 body is byte-for-byte what it was before #7946.
DeepSeek ignores `budget_tokens`, so deriving an effort from the budget would change the wire shape of every existing V4 request without changing what the model does.
Opting in is how the behaviour changes.

### OpenRouter-routed models (`base_url` contains `openrouter`)

Nested form only:

| Mode | Wire |
|---|---|
| `none` | `"reasoning": {"effort": "none"}` |
| `low` | `"reasoning": {"effort": "low"}` |
| `high` | `"reasoning": {"effort": "high"}` |
| `max` | `"reasoning": {"effort": "xhigh"}` |
| *unset* | `"reasoning": {"effort": <budget bucket>}`, or nothing when the budget is under 1024 |

**OpenRouter answers HTTP 400 to a body carrying both `reasoning_effort` and `reasoning.effort`**, so the two are mutually exclusive per request and this branch never sets the top-level field.
The invariant is checked exhaustively over every (family × echo policy × mode × budget × rejected) combination by `no_payload_ever_carries_both_reasoning_controls`, rather than case by case — a future branch that sets the wrong pair would otherwise surface only as a production 400 on one provider.

### Generic OpenAI-compatible endpoints (`ReasoningEchoPolicy::None`)

Top-level form only:

| Mode | Wire |
|---|---|
| `none` | field omitted |
| `low` | `"reasoning_effort": "low"` |
| `high` | `"reasoning_effort": "high"` |
| `max` | `"reasoning_effort": "high"` |
| *unset* | `"reasoning_effort": <budget bucket>` (`low` / `medium` / `high`), or nothing under 1024 |

`none` is expressed by omission because OpenAI answers 400 to the literal `reasoning_effort: "none"`, and a gateway that reasons by *default* is exactly the case that has its own gate field and is handled above.
`max` is clamped to `high` because the OpenAI schema has no rung above it; inventing one buys a 400 plus a strip-and-retry.

### Providers that ignore the mode

| Family | Behaviour |
|---|---|
| Kimi / Moonshot (`ReasoningEchoPolicy::EmptyString`) | `"thinking": {"type": "disabled"}` regardless of mode. Thinking is disabled wire-side so multi-turn `tool_calls` work without round-tripping `reasoning_content` — a correctness requirement, not a preference. |
| DeepSeek R1 (`ReasoningEchoPolicy::Strip`) | Nothing sent. R1 reasons unconditionally and exposes no wire toggle. |
| Anthropic, Gemini, Ollama drivers | Continue to read `budget_tokens`. Their wire control *is* a token budget rather than an effort enum, and remapping an existing Claude user's configured budget into a four-rung bucket would silently change what they are already paying for. |

## Interaction with the #7769 negative cache

An endpoint that has answered 400 to a reasoning control for a given model is recorded in `reasoning_effort_unsupported` and is not asked again — the answer is deterministic, so re-sending only buys a 400 and a retry every turn.
That suppression now covers **both** spellings, and the strip-and-retry in `complete()` / `stream()` clears both, because an OpenRouter-routed body carries the nested object rather than the top-level field.

## Bucket mapping (unchanged fallback)

`reasoning_effort_for_budget`: `<1024` → none, `1024–4096` → `low`, `4097–16384` → `medium`, `>16384` → `high`.

`medium` is reachable only here, never as a settable mode.
A fifth rung whose only distinct meaning is "DeepSeek folds it into `high`" is a knob that does nothing on the provider that motivated the feature.

## Not exposed in the UI

The dashboard agent-edit form and the TUI do not surface `reasoning_mode`.
The issue lists UI exposure as optional; it is left out so the config-plus-wire change stays reviewable on its own.
