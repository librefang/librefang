# Token-estimation accuracy fixtures

These fixtures back the offline benchmark in
`crates/librefang-runtime/tests/token_estimation_accuracy.rs`, which measures
`compactor::estimate_token_count` against real provider `input_tokens`.

## Files

- `corpus.json` — message samples bucketed by content shape
  (`english_prose`, `cjk`, `mixed_cjk_latin`, `tool_json`).
  Committed; safe to extend.
- `tokens_truth.example.json` — the shape of the ground-truth file, with
  placeholder `input_tokens: 0` values.
- `tokens_truth*.json` — real captured ground truth, one file per
  provider/tokenizer. The harness reports each separately. Committed baselines:
  `tokens_truth.json` (Zhipu GLM `glm-4-flash`) and `tokens_truth_gptoss.json`
  (`openai/gpt-oss-120b`, o200k tokenizer family). Counts are per-tokenizer, so
  add a new file (e.g. `tokens_truth_anthropic.json`) rather than overwriting
  when measuring another provider.

The corpus stores plain content, not serialized `Message` objects, so it stays
readable and decoupled from message-struct serde changes. The benchmark and the
capture tool build identical `Message`s from it, so the bytes the estimator
sees match the bytes sent to the provider.

## Regenerating ground truth (live providers — run once, by a human)

Capturing real `input_tokens` requires live API calls and is therefore a
human-run step, not something CI or an assistant performs. Each sample is sent
once with `max_tokens = 1` and prompt caching disabled, so `input_tokens` is the
full, uncached prompt count; only one token is generated, keeping cost
negligible.

```bash
# OpenAI-compatible (OpenAI / Groq / Moonshot / …):
OPENAI_API_KEY=<key> cargo run -p librefang-llm-drivers \
  --example capture_token_truth -- \
  --provider openai --model gpt-4o-mini \
  --base-url https://api.openai.com/v1 \
  --out crates/librefang-runtime/tests/fixtures/token_estimation/tokens_truth.json

# Any other OpenAI-compatible backend (e.g. Zhipu / GLM): drive it via
# `--provider openai`, point `--base-url` at its endpoint, and pass `--label`
# so the recorded provenance is the real provider, not "openai". The API key
# still goes in OPENAI_API_KEY.
OPENAI_API_KEY=<zhipu-key> cargo run -p librefang-llm-drivers \
  --example capture_token_truth -- \
  --provider openai --label zhipu --model glm-4-flash \
  --base-url https://open.bigmodel.cn/api/paas/v4 \
  --out crates/librefang-runtime/tests/fixtures/token_estimation/tokens_truth.json

# Anthropic:
ANTHROPIC_API_KEY=<key> cargo run -p librefang-llm-drivers \
  --example capture_token_truth -- \
  --provider anthropic --model claude-haiku-4-5-20251001 \
  --out crates/librefang-runtime/tests/fixtures/token_estimation/tokens_truth.json
```

Then run the benchmark and read the per-bucket error table:

```bash
cargo test -p librefang-runtime --test token_estimation_accuracy -- --nocapture
```

Once a baseline is committed, gate regressions by setting a ceiling:

```bash
LIBREFANG_TOKEN_EST_MAX_MAE_PCT=20 \
  cargo test -p librefang-runtime --test token_estimation_accuracy
```

## Baseline findings (16 samples, 4 per bucket)

Two committed baselines with different tokenizers — `tokens_truth.json` (Zhipu
GLM `glm-4-flash`) and `tokens_truth_gptoss.json` (`openai/gpt-oss-120b` via
OpenRouter, which uses the o200k tokenizer family and is a close stand-in for
GPT-4o-class OpenAI models). Mean signed error of `estimate_token_count` vs real
`input_tokens` (positive = overestimate):

| bucket | GLM (efficient CJK) | gpt-oss (OpenAI o200k) |
| --- | --- | --- |
| cjk | +126% | +18% |
| mixed_cjk_latin | +76% | -4% |
| english_prose | +29% | -15% |
| tool_json | **-14%** | **-46%** |
| **ALL (signed)** | +54% | -12% |

Reading the two columns together is the point — it separates tokenizer-specific
error from cross-provider error:

- The CJK error is **strongly tokenizer-specific**: +126% on GLM but only +18%
  on the OpenAI-style tokenizer. GLM has a large Chinese vocabulary and
  tokenizes Han text very efficiently (often well under one token per
  character), so the heuristic's 1.5-tokens-per-CJK-char weight overshoots badly
  *for GLM* while being roughly reasonable for o200k. Do **not** change the CJK
  weight on the strength of this — it would help GLM and hurt OpenAI.
- The `tool_json` *under*-estimate is the **cross-provider signal**: both
  tokenizers undercount JSON-heavy tool steps (-14% GLM, -46% o200k), and o200k
  is worse. JSON structure and escaping in tool calls were originally weighted at
  the flat 0.25/char, which undercounts. Because the sign agrees across
  tokenizers, this is the safe, language-independent tuning target — addressed
  below.

## Tuning: JSON-aware structural weight

Acting on the cross-provider `tool_json` signal above, `estimate_token_count`
now counts JSON structural punctuation (`{}[]":,` and the escape `\`) at a
heavier per-char weight (`JSON_STRUCT_TOKEN_WEIGHT = 0.5`) on the tool paths
only (tool-call inputs, tool results, and tool-definition schemas). Prose is
untouched. The weight is calibrated against *both* committed baselines — the
value that improves or holds each without regressing any other bucket:

| tool_json | before (signed / MAE tok) | after (signed / MAE tok) |
| --- | --- | --- |
| GLM | -14% / 21.0 | **-3% / 20.2** |
| gpt-oss (o200k) | -46% / 103.8 | **-39% / 88.5** |

`cjk`, `english_prose`, and `mixed_cjk_latin` are byte-identical before/after
(they never reach the JSON path). GLM tokenizes JSON structure more efficiently
than o200k, so a single global weight is a deliberate compromise: 0.5 removes
GLM's systematic bias and roughly halves the gap on o200k. The residual o200k
undercount is the known limit of a tokenizer-independent heuristic, not a missed
tuning — closing it fully would over-count GLM.

## Methodology notes

- Per-request fixed overhead (role framing, BOS markers, and for tool steps the
  provider's own JSON serialization of tool calls) is real and provider-specific.
  Keep samples non-trivial so this overhead does not dominate relative error,
  and read absolute `MAE tokens` alongside `MAE%`.
- Ground truth is per-provider: different tokenizers give different counts for
  the same bytes. Record `provider` and `model` in each entry and do not mix
  providers within one `tokens_truth.json` without noting it.
- Capture with caching disabled so `input_tokens` reflects the whole prompt; a
  cached run would report a smaller new-input count.
