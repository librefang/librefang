// Step ladders for the two token-count fields in the agent / agent-type editors.
//
// These replaced free sliders. The useful values for a token count are an order-of-magnitude
// sequence, not a continuum: dragging a slider to land on exactly 131072 is a chore, the numbers
// in between mean nothing to any provider, and the slider gave no clue which values were sensible.
// A short list of rungs plus a custom field covers both the common case and the exception.
//
// Mirrors `CONTEXT_WINDOW_LADDER` / `MAX_OUTPUT_TOKENS_LADDER` in
// `crates/librefang-types/src/inference_params.rs`, which the TUI's editor uses.
// The two are small and stable; change both together.

/**
 * Context-window presets, smallest first.
 * How much the model can *read* — the figure Gemini quotes as 1M / 2M.
 */
export const CONTEXT_WINDOW_LADDER = [
  8_192, 32_768, 131_072, 262_144, 524_288, 1_048_576, 2_097_152,
] as const;

/**
 * Maximum-output-token presets, smallest first.
 *
 * Deliberately a different ladder, and it stops at 128K.
 * Output tokens are not context tokens: no model generates a million tokens of reply, so offering
 * 1M here would assert that the value is valid and invite a setting the provider will refuse.
 * A ladder that lies is worse than the slider it replaced.
 */
export const MAX_OUTPUT_TOKENS_LADDER = [
  1_024, 4_096, 8_192, 16_384, 32_768, 65_536, 131_072,
] as const;

/**
 * Temperature presets, smallest first.
 *
 * Sampling parameters get rungs for the same reason token counts do: the useful settings are a
 * handful of named behaviours — deterministic, focused, default, loose — and a 0.01-step slider
 * across them asks the operator to distinguish 0.68 from 0.71, which no model does.
 * `0` is a rung rather than the inherit state: "always take the likeliest token" is a decision, and
 * conflating it with "no opinion" is what a placeholder-empty number box did.
 */
export const TEMPERATURE_LADDER = [0, 0.2, 0.5, 0.7, 1, 1.5, 2] as const;

/**
 * Nucleus-sampling presets, smallest first.
 *
 * Stops at 1 because `top_p` is a probability mass, not a score — 1 already means "consider every
 * token", and the ladder must not offer a value the endpoint will reject.
 */
export const TOP_P_LADDER = [0.1, 0.5, 0.8, 0.9, 0.95, 1] as const;

/**
 * Frequency- and presence-penalty presets, smallest first.
 *
 * Symmetric around `0` because the sign is meaningful: negatives encourage repetition, which is a
 * real setting and not an error to be clamped away.
 */
export const PENALTY_LADDER = [-2, -1, -0.5, 0, 0.5, 1, 2] as const;

/**
 * Top-k presets, smallest first.
 *
 * A token count, so whole numbers only; `40` is llama.cpp's and Ollama's default and `64` is Gemini's.
 * `1` is greedy decoding, a real setting rather than the inherit state.
 */
export const TOP_K_LADDER = [1, 10, 20, 40, 64, 100] as const;

/**
 * Minimum-probability (min-p) presets, smallest first.
 *
 * A fraction of the top token's probability. `0` switches the filter off; `0.05` is llama.cpp's default and the value usually recommended alongside a higher temperature.
 */
export const MIN_P_LADDER = [0, 0.02, 0.05, 0.1, 0.2] as const;

/**
 * Repetition-penalty presets, smallest first.
 *
 * Multiplicative, so `1` is "off" rather than `0`; useful values sit just above it, and past `1.5` output degrades.
 */
export const REPEAT_PENALTY_LADDER = [1, 1.05, 1.1, 1.15, 1.2, 1.3, 1.5] as const;

/**
 * Render a rung the way operators read it: `128K`, `1M`, `0.7`, or the raw number.
 *
 * Shared by the token ladders and the sampling ladders. The `K`/`M` shortening only fires on exact
 * multiples of 1024, so a temperature or a penalty falls through to its plain decimal form.
 */
export function formatTokens(value: number): string {
  if (value >= 1024 * 1024 && value % (1024 * 1024) === 0) return `${value / (1024 * 1024)}M`;
  if (value >= 1024 && value % 1024 === 0) return `${value / 1024}K`;
  return String(value);
}

/**
 * Trim a ladder to a limit the model actually declared.
 *
 * `cap` must be `undefined` unless some source vouched for it — the catalog's `limits_known` flag
 * is what separates a measured limit from a discovery placeholder (#7780).
 * Capping against an invented ceiling would hide rungs the endpoint may well support.
 *
 * When the cap falls between two rungs it is appended so it stays selectable, which is the only
 * way to offer a model whose real maximum is, say, 20000.
 */
export function ladderUpTo(ladder: readonly number[], cap?: number): number[] {
  if (cap === undefined || cap <= 0) return [...ladder];
  const rungs = ladder.filter((r) => r <= cap);
  if (!rungs.includes(cap)) rungs.push(cap);
  return rungs.sort((a, b) => a - b);
}

/**
 * Whether `value` sits on the ladder.
 * Anything else was typed by hand and belongs in the custom field.
 */
export function isOnLadder(ladder: readonly number[], value: number | null): boolean {
  return value !== null && ladder.includes(value);
}
