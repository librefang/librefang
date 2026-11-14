The per-turn prompt-cache metric no longer flatters itself: `TokenUsage::cache_hit_ratio` divides `cache_read_input_tokens` by the whole prompt instead of by the cache buckets (`cache_read + cache_creation`), so the uncached remainder counts.
  The old denominator made the ratio collapse to `1.0` on every provider that reports cache reads but never cache writes — OpenAI-compatible drivers hardcode `cache_creation_input_tokens: 0`, which turned a 30 % hit rate into "100 % cached" in the `prompt cache metrics for turn` log line.
  On the normalized Anthropic shape the same bug overstated the ratio (`0.875` instead of `0.70`).
  `None` still means "no ratio to report": caching wasn't active at all, the prompt size is unknown, or the provider's cache buckets sum past the prompt it was given — the last case is not clamped to `1.0`, because that would dress a broken invariant up as a perfect hit.
  The dashboard's auto-dream cache row now derives the same ratio from `input_tokens`, and refuses the same malformed payloads instead of rendering a percentage above 100.
(#8753, #8752) (@nevgenov)
