// The model's declared capacities, and the advisory shown when a draft exceeds one.
//
// Both editors that set `max_tokens` / `context_window` need the same three answers: which capacities the catalog actually vouched for, which limit `max_tokens` is measured against, and what to say when the typed value passes it.
// They were written once in the create form and not at all in the agent detail drawer, so the same parameter warned in one place and stayed silent in the other.

import { formatTokens } from "./modelParamLadders";

/** The subset of a catalog entry these helpers read. */
export interface ModelLimitSource {
  provider: string;
  id: string;
  context_window?: number;
  max_output_tokens?: number;
  /**
   * Whether the two capacities above were actually sourced.
   * `false` marks them as discovery placeholders rather than measurements (#7780); absent means an older daemon that had no such field, which is treated as sourced.
   */
  limits_known?: boolean;
}

export interface ModelLimits {
  contextWindow?: number;
  maxOutputTokens?: number;
}

/**
 * The limits for one model, and only when the catalog vouched for them.
 *
 * An unmeasured capacity is left `undefined` rather than passed on as a ceiling: capping a ladder or warning against an invented number trains operators to ignore both.
 * `0` is the catalog's "unknown" sentinel and is dropped for the same reason (#7774).
 */
export function selectModelLimits(
  models: readonly ModelLimitSource[],
  model: string,
  provider: string,
): ModelLimits {
  const entry = models.find((m) => m.id === model && m.provider === provider);
  if (!entry || entry.limits_known === false) return {};
  return {
    contextWindow: entry.context_window && entry.context_window > 0 ? entry.context_window : undefined,
    maxOutputTokens:
      entry.max_output_tokens && entry.max_output_tokens > 0 ? entry.max_output_tokens : undefined,
  };
}

/**
 * What `max_tokens` is measured against.
 *
 * An operator-set output cap describes this endpoint and outranks the catalog's figure for it — the operator is the one who knows their self-hosted checkpoint serves less than the catalog claims.
 */
export function resolveMaxTokensLimit(
  maxOutputTokensDraft: string,
  catalogMaxOutputTokens?: number,
): number | undefined {
  const trimmed = maxOutputTokensDraft.trim();
  if (trimmed === "") return catalogMaxOutputTokens;
  const parsed = Number(trimmed);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : catalogMaxOutputTokens;
}

/**
 * Advisory, not a validation error: the field is not marked invalid and the value is saved as typed.
 * If the catalog figure is the thing that is wrong, an explicit provider error beats a silent truncation.
 */
export function overLimitWarning(
  raw: string,
  limit: number | undefined,
  t: (key: string, opts?: Record<string, unknown>) => string,
): string | undefined {
  const parsed = Number(raw.trim());
  if (raw.trim() === "" || !Number.isFinite(parsed) || limit === undefined) return undefined;
  return parsed > limit
    ? t("agents.form.over_limit_warning", { limit: formatTokens(limit) })
    : undefined;
}
