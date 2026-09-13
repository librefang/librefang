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

import {
  isValidParamValue,
  MODEL_PARAM_NAMES,
  type ModelParamName,
} from "../components/ui/ModelParamField";

/**
 * The numeric half of the draft is exactly the shared parameter set, not a second list of its own.
 *
 * What a field may hold lives in `MODEL_PARAM_RANGES` next to the control that renders it, and
 * `isValidParamValue` is the one function that answers it — the same answer the create form and the
 * model settings already get. A table here would be a second opinion about the same seven fields,
 * free to drift from the `min`/`max`/`step` the operator's own input box enforces.
 *
 * The shared bounds are what `patch_agent_config` validates on `PatchAgentConfigRequest`
 * (`routes/agents/config.rs`): a range table covers the four float fields, and the three integer
 * ones are rejected only at zero. Sending a value outside them is a 400, so catching it here is the
 * difference between a disabled Save and a failed request.
 */
export type ModelNumericField = ModelParamName;

export const MODEL_NUMERIC_FIELDS = MODEL_PARAM_NAMES;

export interface PersistedModel {
  provider?: string;
  model?: string;
  /** `null` / absent means the agent inherits rather than pinning a number. */
  max_tokens?: number | null;
  temperature?: number | null;
  top_p?: number | null;
  frequency_penalty?: number | null;
  presence_penalty?: number | null;
  context_window?: number | null;
  max_output_tokens?: number | null;
}

export interface ModelDraft {
  provider: string;
  model: string;
  /** `""` is the inherit state, not zero. */
  max_tokens: string;
  temperature: string;
  top_p: string;
  frequency_penalty: string;
  presence_penalty: string;
  context_window: string;
  max_output_tokens: string;
}

export interface ModelConfigPatch {
  provider?: string;
  model?: string;
  /** `null` clears the agent's own value. */
  max_tokens?: number | null;
  temperature?: number | null;
  top_p?: number | null;
  frequency_penalty?: number | null;
  presence_penalty?: number | null;
  context_window?: number | null;
  max_output_tokens?: number | null;
}

/** Every numeric field in its inherit state — the shape a fresh draft starts in. */
export function emptyModelNumerics(): Pick<ModelDraft, ModelNumericField> {
  return Object.fromEntries(MODEL_NUMERIC_FIELDS.map((f) => [f, ""])) as Pick<
    ModelDraft,
    ModelNumericField
  >;
}

/**
 * Seed the numeric half of a draft from what the daemon returned.
 *
 * A `null` on the wire becomes `""`, not the compiled default: an untouched
 * field has to look untouched, or opening the drawer and saving would pin every
 * inherited value as a deliberate choice (#5917).
 */
export function seedModelNumerics(
  persisted: PersistedModel | undefined,
): Pick<ModelDraft, ModelNumericField> {
  return Object.fromEntries(
    MODEL_NUMERIC_FIELDS.map((f) => [f, persisted?.[f] == null ? "" : String(persisted[f])]),
  ) as Pick<ModelDraft, ModelNumericField>;
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
function parseTriState(param: ModelNumericField, raw: string): number | null | undefined {
  const trimmed = raw.trim();
  if (trimmed === "") return null;
  // `isValidParamValue` covers finiteness, the range, and whole-numberness for the token counts, so
  // `Number` is the only parse needed and cannot come back NaN after it. The previous `parseInt`
  // read a prefix rather than the value: `1e5` became 1 and `4096.7` became 4096, both stored
  // silently, and both accepted here while the shared validator rejected them.
  if (!isValidParamValue(param, trimmed)) return undefined;
  return Number(trimmed);
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

  const parsed = {} as Record<ModelNumericField, number | null>;
  for (const field of MODEL_NUMERIC_FIELDS) {
    const value = parseTriState(field, draft[field]);
    // One invalid field invalidates the whole draft: a partial PATCH would
    // save some of what the operator typed and silently drop the rest.
    if (value === undefined) return { patch: null };
    parsed[field] = value;
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

  for (const field of MODEL_NUMERIC_FIELDS) {
    const next = parsed[field];
    // `?? null` rather than `|| null`: a persisted explicit `0` is a real
    // value, not an absent one.
    const current = persisted?.[field] ?? null;
    if (next === current) continue;
    // A `null` here goes out even when the provider changed in the same edit.
    // This used to be skipped on the theory that switching provider resets
    // these server-side; it does not. `set_agent_model`
    // (`librefang-kernel/src/kernel/agent_state.rs`) clears `api_key_env` and
    // `base_url` and nothing else, and `patch_agent_config` writes model and
    // provider before the sampling fields, so a `null` alongside a provider
    // change lands rather than being overwritten. Skipping it could only ever
    // drop a clear the operator made by hand: an untouched pinned field seeds
    // to its own value and leaves by the equality check above.
    patch[field] = next;
  }

  return { patch };
}
