// Patch-builder for the agent "model" inline edit form on AgentsPage.
//
// AgentModelDetail.max_tokens / .temperature are optional: the backend omits
// them when unset. startModelEdit seeds the draft with the same compiled
// defaults the kernel uses (4096 / 0.7), so the persisted side MUST apply the
// identical nullish defaults before comparing — otherwise a provider/model-only
// edit would see the seeded default as a change and silently PATCH 4096 / 0.7
// into an agent the user never touched. This module is the single source of
// truth for that comparison baseline, shared by saveModelEdit and modelDirty's
// regression test so the two cannot drift apart (the original #5917 defect).

// Compiled kernel defaults surfaced in the edit form when the backend omits
// the optional field. Keep in sync with the `?? 4096` / `?? 0.7` fallbacks in
// AgentsPage's startModelEdit and modelDirty derivation.
export const MODEL_MAX_TOKENS_DEFAULT = 4096;
export const MODEL_TEMPERATURE_DEFAULT = 0.7;

export interface PersistedModel {
  provider?: string;
  model?: string;
  max_tokens?: number;
  temperature?: number;
}

export interface ModelDraft {
  provider: string;
  model: string;
  max_tokens: string;
  temperature: string;
}

export interface ModelConfigPatch {
  provider?: string;
  model?: string;
  max_tokens?: number;
  temperature?: number;
}

export interface BuildModelConfigPatchResult {
  /** null when the draft fails validation (caller should not submit). */
  patch: ModelConfigPatch | null;
}

// Build the PATCH payload from the draft, including a field only when the user
// actually changed it from its persisted (nullish-defaulted) value. Returns
// `{ patch: null }` when the draft is invalid so the caller can bail without
// re-implementing the validation.
export function buildModelConfigPatch(
  draft: ModelDraft,
  persisted: PersistedModel | undefined,
): BuildModelConfigPatchResult {
  const trimmedProvider = draft.provider.trim();
  const trimmedModel = draft.model.trim();
  const parsedMaxTokens = parseInt(draft.max_tokens, 10);
  const parsedTemperature = parseFloat(draft.temperature);

  if (!trimmedProvider || !trimmedModel) return { patch: null };
  if (isNaN(parsedMaxTokens) || parsedMaxTokens <= 0) return { patch: null };
  if (isNaN(parsedTemperature) || parsedTemperature < 0 || parsedTemperature > 2) {
    return { patch: null };
  }

  const patch: ModelConfigPatch = {};

  const modelChanged = trimmedModel !== persisted?.model;
  const providerChanged = trimmedProvider !== persisted?.provider;
  if (modelChanged || providerChanged) {
    patch.model = trimmedModel;
    patch.provider = trimmedProvider;
  }

  // Same nullish-defaulted baseline as the modelDirty gate — see module doc.
  if (parsedMaxTokens !== (persisted?.max_tokens ?? MODEL_MAX_TOKENS_DEFAULT)) {
    patch.max_tokens = parsedMaxTokens;
  }
  if (parsedTemperature !== (persisted?.temperature ?? MODEL_TEMPERATURE_DEFAULT)) {
    patch.temperature = parsedTemperature;
  }

  return { patch };
}
