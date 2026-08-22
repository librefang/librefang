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

/** Render a token count the way operators read them: `128K`, `1M`, or the raw number. */
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
