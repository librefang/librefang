// Draft rules for the per-model limit overrides edited on the providers page.
//
// Extracted for the same reason `agentModelPatch.ts` exists: the rule below
// decides both what gets saved and whether Save lights up, and keeping one copy
// is what stops those two from drifting.
//
// The fields edit a **preference** — how long a reply to ask this model for, how
// much context to send it. They are not the model's catalog figures, which the
// provider card reports and which nothing here moves. Conflating the two is what
// the rule below previously did.

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
 * `catalogValue` is deliberately **not** a parameter. The old rule treated a
 * typed value equal to the model's catalog figure as "same as the default, so
 * clear it", on the theory that an absent override means that figure gets
 * requested. It does not: the resolution chain in `inference_params.rs` falls
 * through to the kernel's own default — `DEFAULT_MODEL_MAX_TOKENS` (4096) for
 * `max_tokens` — so that rule silently discarded a deliberate setting and left
 * the model somewhere the operator never chose.
 *
 * The same argument holds for both fields this editor drives. `context_window`
 * and `max_tokens` are overrides the operator set on purpose; whether the number
 * they typed happens to match a catalog entry says nothing about whether they
 * meant it. Capacity has no say in what a preference resolves to, so it is not
 * consulted — it stays a parameter of the *display* path (`effective`), which is
 * a different question and stays where it is.
 */
export function resolveLimitDraft(input: string, storedOverride: number | undefined): LimitDraft {
  const trimmed = input.trim();
  const parsed = trimmed === "" ? null : Number(trimmed);
  const invalid = parsed !== null && (!Number.isInteger(parsed) || parsed <= 0);
  return {
    value: parsed,
    invalid,
    dirty: parsed !== (storedOverride ?? null),
  };
}
