// Draft rules for the per-model limit overrides edited on the providers page.
//
// Extracted for the same reason `agentModelPatch.ts` exists: the rule below
// decides both what gets saved and whether Save lights up, and keeping one copy
// is what stops those two from drifting.
//
// The fields edit a **preference** — how long a reply to ask this model for, how
// much context to send it. They are not the model's catalog figures, which the
// provider card reports and which nothing here moves. The one place capacity
// matters is that an absent override resolves *to* a figure the chain already
// produces: `context_window` to the model's catalog window, and `max_tokens` to
// the model's output ceiling (#8502) before the kernel default. A typed value
// equal to that figure is therefore a redundant override, and
// `resolveLimitDraft` clears it. It takes `catalogValue` for exactly that case
// and for no other.

export interface LimitDraft {
  /** What to persist: a number sets the override, `null` clears it. */
  value: number | null;
  /** True when the text is not a usable positive whole number. */
  invalid: boolean;
  /** True when saving would change the stored state. */
  dirty: boolean;
}

/**
 * Resolve the draft state for a limit-override field.
 *
 * `input` is the raw text; an empty field means "no preference here", which
 * clears the override and lets the resolution chain supply a value.
 *
 * `catalogValue` is the figure an *absent* override resolves to for this field,
 * or `undefined` when no figure answers. Both fields this editor drives fall
 * through to the catalog, so both pass it:
 *
 * - `context_window`: absent resolves to the catalog figure
 *   (`resolve_context_window` ranks agent manifest → `model_overrides.json` →
 *   `ModelCatalog`). A typed value equal to the catalog is a redundant override
 *   that *pins* the window: a later registry or discovery correction
 *   (131072 → 200000) would be silently shadowed. Passing `catalogValue` makes
 *   an equality clear the override, so the field keeps following the catalog
 *   unless the operator deliberately picks a different number.
 * - `max_tokens`: since #8502 an absent override does not fall straight to
 *   `DEFAULT_MODEL_MAX_TOKENS`; it first takes the model's output ceiling — the
 *   catalog entry's `max_output_tokens`, or the operator's per-model correction
 *   of it. A typed value equal to the catalog figure is therefore redundant for
 *   the same reason, so it clears too. Only when nothing declares a ceiling
 *   does the daemon default stand in, and then there is nothing to compare
 *   against: `catalogValue` is `undefined` and the typed value persists.
 *
 * Capacity is still not consulted for the dirty/save decision beyond that:
 * the display path (`seed` in `ProvidersPage.tsx`) keeps its own `catalogValue`.
 */
export function resolveLimitDraft(
  input: string,
  storedOverride: number | undefined,
  catalogValue?: number,
): LimitDraft {
  const trimmed = input.trim();
  const parsed = trimmed === "" ? null : Number(trimmed);
  const invalid = parsed !== null && (!Number.isInteger(parsed) || parsed <= 0);
  // Equal to the figure an absent override would resolve to, so there is
  // nothing to store: clearing it is not discarding a preference, it is
  // declining to pin a value the chain already produces.
  const value =
    parsed != null && catalogValue != null && parsed === catalogValue ? null : parsed;
  return {
    value,
    invalid,
    dirty: value !== (storedOverride ?? null),
  };
}
