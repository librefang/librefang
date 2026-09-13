import { useTranslation } from "react-i18next";
import { StepLadderInput } from "./StepLadderInput";
import {
  CONTEXT_WINDOW_LADDER,
  MAX_OUTPUT_TOKENS_LADDER,
  PENALTY_LADDER,
  TEMPERATURE_LADDER,
  TOP_P_LADDER,
} from "../../lib/modelParamLadders";

/**
 * The model parameters that more than one editor can set.
 *
 * Each is configurable from at least two places — the agent manifest, the
 * model's own settings, and the per-provider override — and each of those used
 * to render it differently: a rung ladder in one, a 1024-step slider from 1 Ki
 * to 2 Mi in another, a bare number box in the third. Same parameter, same
 * units, three controls and three vocabularies.
 */
export type ModelParamName =
  | "context_window"
  | "max_output_tokens"
  | "max_tokens"
  | "temperature"
  | "top_p"
  | "frequency_penalty"
  | "presence_penalty";

const LADDERS: Record<ModelParamName, readonly number[]> = {
  context_window: CONTEXT_WINDOW_LADDER,
  // An output cap and a per-request `max_tokens` are the same quantity seen
  // from two sides, so they share rungs.
  max_output_tokens: MAX_OUTPUT_TOKENS_LADDER,
  max_tokens: MAX_OUTPUT_TOKENS_LADDER,
  temperature: TEMPERATURE_LADDER,
  top_p: TOP_P_LADDER,
  // The two penalties take the same range and the same sign convention.
  frequency_penalty: PENALTY_LADDER,
  presence_penalty: PENALTY_LADDER,
};

const LABEL_KEYS: Record<ModelParamName, string> = {
  context_window: "model_param.context_window",
  max_output_tokens: "model_param.max_output_tokens",
  max_tokens: "model_param.max_tokens",
  temperature: "model_param.temperature",
  top_p: "model_param.top_p",
  frequency_penalty: "model_param.frequency_penalty",
  presence_penalty: "model_param.presence_penalty",
};

const PLACEHOLDER_KEYS: Record<ModelParamName, string> = {
  context_window: "model_param.context_window_placeholder",
  max_output_tokens: "model_param.max_output_tokens_placeholder",
  max_tokens: "model_param.max_tokens_placeholder",
  temperature: "model_param.temperature_placeholder",
  top_p: "model_param.top_p_placeholder",
  frequency_penalty: "model_param.penalty_placeholder",
  presence_penalty: "model_param.penalty_placeholder",
};

/**
 * The values each parameter can actually hold.
 *
 * Kept next to the rungs because the two answer the same question from opposite ends: the ladder
 * offers the sensible settings, this decides whether a hand-typed one is storable at all.
 * A token count is a positive whole number; a sampling parameter is a decimal inside a range the
 * provider will accept, and `0` is a legitimate temperature rather than an unset field.
 */
export const MODEL_PARAM_RANGES: Record<
  ModelParamName,
  { min: number; max?: number; integer: boolean }
> = {
  context_window: { min: 1, integer: true },
  max_output_tokens: { min: 1, integer: true },
  max_tokens: { min: 1, integer: true },
  temperature: { min: 0, max: 2, integer: false },
  top_p: { min: 0, max: 1, integer: false },
  frequency_penalty: { min: -2, max: 2, integer: false },
  presence_penalty: { min: -2, max: 2, integer: false },
};

/**
 * Every parameter this module governs, in a fixed order.
 *
 * Exported so a caller that has to iterate them — the agent patch-builder walks all seven to decide
 * which changed — reads the set from here instead of restating it. A second list is a second thing
 * to forget to extend.
 */
export const MODEL_PARAM_NAMES = Object.keys(MODEL_PARAM_RANGES) as ModelParamName[];

/** Granularity of the custom field. A token count is whole; a sampling value is not. */
const STEPS: Record<ModelParamName, number> = {
  context_window: 1,
  max_output_tokens: 1,
  max_tokens: 1,
  temperature: 0.01,
  top_p: 0.01,
  frequency_penalty: 0.01,
  presence_penalty: 0.01,
};

/**
 * Whether `raw` is a value `param` can hold, for editors that must refuse one before storing it.
 *
 * `min`/`max` on a number input are checked by the browser on form submit, and these drawers never
 * submit a form — so a `0` typed into `max_tokens` reached the provider as `"max_tokens": 0` for
 * every agent on that model. The rule lives here so each editor cannot invent its own.
 * An empty string is not a value: it is the inherit rung, which every caller handles before asking.
 */
export function isValidParamValue(param: ModelParamName, raw: string): boolean {
  const parsed = Number(raw.trim());
  if (raw.trim() === "" || !Number.isFinite(parsed)) return false;
  const range = MODEL_PARAM_RANGES[param];
  if (range.integer && !Number.isInteger(parsed)) return false;
  if (parsed < range.min) return false;
  return range.max === undefined || parsed <= range.max;
}

interface ModelParamFieldProps {
  param: ModelParamName;
  /** Form value in tokens. `""` means "no opinion here, inherit". */
  value: string;
  onChange: (next: string) => void;
  /**
   * A ceiling some source vouched for, used to trim rungs the endpoint cannot
   * honour. Leave undefined for a limit that was never measured — an unknown
   * limit is not a ceiling (#7780).
   */
  cap?: number;
  /** Advisory shown under the control, e.g. an over-limit warning. */
  warning?: string;
  /** Explanatory line under the control, for editors that need the context. */
  hint?: string;
  /**
   * Overrides the shared label. Use only where the surrounding page gives the
   * parameter a different meaning — not to rename it for decoration.
   */
  label?: string;
}

/**
 * One parameter, one control, wherever it is configured.
 *
 * This is the object every editor renders for these parameters: it owns the
 * rungs, the wording for "inherit" and "custom", and the placeholder, so the
 * agent editor, the model settings and the per-provider override cannot drift
 * into three different experiences for the same field again.
 *
 * `""` is the inherit state everywhere, which is also what each of the three
 * call sites means by its own idiom — an unset manifest field, an unticked
 * override toggle, a cleared box.
 */
export function ModelParamField({
  param,
  value,
  onChange,
  cap,
  warning,
  hint,
  label,
}: ModelParamFieldProps) {
  const { t } = useTranslation();
  return (
    <div>
      <StepLadderInput
        label={label ?? t(LABEL_KEYS[param])}
        value={value}
        onChange={onChange}
        ladder={LADDERS[param]}
        cap={cap}
        inheritLabel={t("model_param.inherit")}
        customLabel={t("model_param.custom")}
        customPlaceholder={t(PLACEHOLDER_KEYS[param])}
        warning={warning}
        min={MODEL_PARAM_RANGES[param].min}
        max={MODEL_PARAM_RANGES[param].max}
        step={STEPS[param]}
      />
      {hint && <p className="mt-1 text-[10px] text-text-dim/70 leading-snug">{hint}</p>}
    </div>
  );
}
