// Patch-builder for the agent "model" inline edit form on AgentsPage.
//
// `max_tokens` / `temperature` are tri-state, and the empty string is the third state: it means
// "this agent has no opinion", so the per-model override supplies the value and, failing that, the
// system default.
// The form seeds an empty field from a `null` on the wire and sends `null` back to clear one.
//
// This used to seed the draft with the compiled kernel defaults (4096 / 0.7) and compare against
// the same baseline, so a provider-only edit would not silently PATCH those numbers into an agent
// the user never touched (#5917).
// That was a workaround for a type with no inherit state: every agent carried a concrete number, so
// "unset" had to be simulated by matching against the default.
// With the field genuinely nullable the workaround is gone — an untouched field stays empty, and
// an emptied field is a deliberate "hand this back to the model's setting" that reaches the
// backend as `null` instead of being silently indistinguishable from no edit at all.

export interface PersistedModel {
  provider?: string;
  model?: string;
  /** `null` / absent means the agent inherits rather than pinning a number. */
  max_tokens?: number | null;
  temperature?: number | null;
}

export interface ModelDraft {
  provider: string;
  model: string;
  /** `""` is the inherit state, not zero. */
  max_tokens: string;
  temperature: string;
}

export interface ModelConfigPatch {
  provider?: string;
  model?: string;
  /** `null` clears the agent's own value. */
  max_tokens?: number | null;
  temperature?: number | null;
}

export interface BuildModelConfigPatchResult {
  /** null when the draft fails validation (caller should not submit). */
  patch: ModelConfigPatch | null;
}

/**
 * Parse a tri-state numeric draft field.
 *
 * Returns `null` for the inherit state, a number for a pinned value, and
 * `undefined` when the text is not a number this field accepts — which the
 * caller treats as an invalid draft.
 */
function parseTriState(
  raw: string,
  parse: (s: string) => number,
  valid: (n: number) => boolean,
): number | null | undefined {
  const trimmed = raw.trim();
  if (trimmed === "") return null;
  if (Number.isNaN(Number(trimmed))) return undefined;
  const parsed = parse(trimmed);
  return Number.isNaN(parsed) || !valid(parsed) ? undefined : parsed;
}

// Build the PATCH payload from the draft, including a field only when the user
// actually changed it. Returns `{ patch: null }` when the draft is invalid so
// the caller can bail without re-implementing the validation.
export function buildModelConfigPatch(
  draft: ModelDraft,
  persisted: PersistedModel | undefined,
): BuildModelConfigPatchResult {
  const trimmedProvider = draft.provider.trim();
  const trimmedModel = draft.model.trim();
  if (!trimmedProvider || !trimmedModel) return { patch: null };

  const maxTokens = parseTriState(draft.max_tokens, (s) => parseInt(s, 10), (n) => n > 0);
  const temperature = parseTriState(draft.temperature, parseFloat, (n) => n >= 0 && n <= 2);
  if (maxTokens === undefined || temperature === undefined) return { patch: null };

  const patch: ModelConfigPatch = {};

  const persistedModel = persisted?.model?.trim() ?? "";
  const persistedProvider = persisted?.provider?.trim() ?? "";
  const modelChanged = trimmedModel !== persistedModel;
  const providerChanged = trimmedProvider !== persistedProvider;
  if (providerChanged) {
    // PATCH /config applies provider changes only while processing `model`,
    // so a provider edit must carry the current model as its trigger.
    patch.model = trimmedModel;
    patch.provider = trimmedProvider;
  } else if (modelChanged) {
    patch.model = trimmedModel;
  }

  // `?? null` rather than `|| null`: a persisted explicit `0` is a real value,
  // not an absent one.
  if (maxTokens !== (persisted?.max_tokens ?? null) && (!providerChanged || maxTokens !== null)) {
    patch.max_tokens = maxTokens;
  }
  if (temperature !== (persisted?.temperature ?? null) && (!providerChanged || temperature !== null)) {
    patch.temperature = temperature;
  }

  return { patch };
}
