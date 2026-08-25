`GET /api/models` now asks a self-hosted OpenAI-compatible gateway what it serves, so the models on a LiteLLM, vLLM, LM Studio or llama.cpp endpoint are selectable instead of absent.
The ids on such a gateway are the operator's own, so no shipped catalog can contain them, and the handler previously refreshed a live catalog for OpenRouter and EveryAPI alone — everything else was served from the snapshot and a gateway had nothing in it.
The listing is read through the same 60-second probe cache the provider grid uses and a failure falls back to the shipped catalog, so a gateway that proxies `/chat/completions` without exposing `/models` is unaffected.
Being in the catalog is also what stops the runtime assuming an 8k context window for a model that handles far more, which is where the missing entry stopped being cosmetic.
(#7816) (@houko)
