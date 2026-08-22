// Draft rules for the per-model `max_tokens` override edited on the providers page.
//
// Extracted for the same reason `agentModelPatch.ts` exists: the rule below decides both what gets
// saved and whether Save lights up, and keeping one copy is what stops those two from drifting.
//
// The field edits a **preference** — how long a reply to ask this model for. It is not the model's
// capacity, which is the headline figure on the provider card and which nothing here moves.
// Conflating the two is what this rule previously did.

export interface MaxTokensDraft {
  /** What to persist: a number sets the override, `null` clears it. */
  value: number | null;
  /** True when the text is not a usable positive whole number. */
  invalid: boolean;
  /** True when saving would change the stored state. */
  dirty: boolean;
}

/**
 * Resolve the draft state for the override field.
 *
 * `input` is the raw text; an empty field means "no preference here", which
 * clears the override and lets the resolution chain supply a value.
 *
 * `catalogMax` is deliberately **not** a parameter. The old rule treated a
 * typed value equal to the model's catalog capacity as "same as the default,
 * so clear it", on the theory that an absent override means the capacity gets
 * requested. It does not: an absent override falls through to the kernel's own
 * default, so that rule silently discarded a deliberate setting and left the
 * model somewhere the operator never chose. Capacity has no say in what a
 * preference resolves to, so it is not consulted.
 */
export function resolveMaxTokensDraft(
  input: string,
  storedOverride: number | undefined,
): MaxTokensDraft {
  const trimmed = input.trim();
  const parsed = trimmed === "" ? null : Number(trimmed);
  const invalid = parsed !== null && (!Number.isInteger(parsed) || parsed <= 0);
  return {
    value: parsed,
    invalid,
    dirty: parsed !== (storedOverride ?? null),
  };
}
