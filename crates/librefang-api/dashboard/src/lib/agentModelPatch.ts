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
  const trimmedMaxTokens = draft.max_tokens.trim();
  const trimmedTemperature = draft.temperature.trim();

  if (!trimmedProvider || !trimmedModel) return { patch: null };
  if (!/^\d+$/.test(trimmedMaxTokens)) return { patch: null };
  if (!/^(?:\d+(?:\.\d*)?|\.\d+)$/.test(trimmedTemperature)) return { patch: null };

  const parsedMaxTokens = Number(trimmedMaxTokens);
  const parsedTemperature = Number(trimmedTemperature);
  if (!Number.isSafeInteger(parsedMaxTokens) || parsedMaxTokens <= 0) return { patch: null };
  if (!Number.isFinite(parsedTemperature) || parsedTemperature < 0 || parsedTemperature > 2) {
    return { patch: null };
  }

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

  // Same nullish-defaulted baseline as the modelDirty gate — see module doc.
  if (parsedMaxTokens !== (persisted?.max_tokens ?? MODEL_MAX_TOKENS_DEFAULT)) {
    patch.max_tokens = parsedMaxTokens;
  }
  if (parsedTemperature !== (persisted?.temperature ?? MODEL_TEMPERATURE_DEFAULT)) {
    patch.temperature = parsedTemperature;
  }

  return { patch };
}
