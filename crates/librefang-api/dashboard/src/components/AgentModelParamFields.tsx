import { useTranslation } from "react-i18next";
import { ModelParamField } from "./ui/ModelParamField";
import type { ModelNumericField } from "../lib/agentModelPatch";
import {
  overLimitWarning,
  resolveMaxTokensLimit,
  type ModelLimits,
} from "../lib/modelLimits";

interface AgentModelParamFieldsProps {
  /** The numeric half of the model draft. `""` is the inherit state everywhere, not zero. */
  draft: Record<ModelNumericField, string>;
  /**
   * One handler keyed by field rather than seven closures.
   *
   * Seven hand-written setters differing only in a key is the shape that eventually writes `presence_penalty` into `frequency_penalty`, and nothing renders the drawer in a test that would notice.
   * Passing the field through makes that mistake unspellable.
   */
  onChange: (field: ModelNumericField, next: string) => void;
  /**
   * Hand agents reach a different write path with a smaller surface.
   *
   * `PATCH /api/agents/{id}/hand-runtime-config` deserializes the full `PatchAgentConfigRequest` but maps only `max_tokens` and `temperature` into `HandAgentRuntimeOverride` (`crates/librefang-api/src/routes/agents/config.rs:1401-1419`, and the struct itself in `crates/librefang-hands/src/lib.rs:1158-1173`).
   * The other five are known keys, so nothing rejects them — the request returns 200 and the values are dropped on the floor.
   * Do not render them here again without a field on that struct to receive them.
   */
  isHand: boolean;
  /** Declared capacities for the selected model, when the catalog vouched for them. */
  limits?: ModelLimits;
}

/**
 * The seven model parameters an agent can pin, as the shared step ladders.
 *
 * Extracted from the detail drawer so the wiring is reachable from a test: the page itself has no render harness (~20 hooks), which is why `SystemPromptSection` is exported from it too.
 */
export function AgentModelParamFields({
  draft,
  onChange,
  isHand,
  limits,
}: AgentModelParamFieldsProps) {
  const { t } = useTranslation();
  const maxTokensLimit = resolveMaxTokensLimit(draft.max_output_tokens, limits?.maxOutputTokens);

  return (
    <div className="space-y-3 py-1">
      <ModelParamField
        param="temperature"
        value={draft.temperature}
        onChange={(next) => onChange("temperature", next)}
      />
      {!isHand && (
        <>
          <ModelParamField
            param="top_p"
            value={draft.top_p}
            onChange={(next) => onChange("top_p", next)}
          />
          <ModelParamField
            param="frequency_penalty"
            value={draft.frequency_penalty}
            onChange={(next) => onChange("frequency_penalty", next)}
          />
          <ModelParamField
            param="presence_penalty"
            value={draft.presence_penalty}
            onChange={(next) => onChange("presence_penalty", next)}
          />
        </>
      )}
      {/*
        Below the sampling rows and separated, because these are endpoint limits rather than preferences: what the model may read and emit, not how it should sound.
      */}
      <div className="pt-2 border-t border-border-subtle/40 space-y-3">
        <ModelParamField
          param="max_tokens"
          value={draft.max_tokens}
          onChange={(next) => onChange("max_tokens", next)}
          cap={limits?.maxOutputTokens}
          warning={overLimitWarning(draft.max_tokens, maxTokensLimit, t)}
        />
        {!isHand && (
          <>
            <ModelParamField
              param="context_window"
              value={draft.context_window}
              onChange={(next) => onChange("context_window", next)}
              warning={overLimitWarning(draft.context_window, limits?.contextWindow, t)}
            />
            <ModelParamField
              param="max_output_tokens"
              value={draft.max_output_tokens}
              onChange={(next) => onChange("max_output_tokens", next)}
            />
          </>
        )}
      </div>
    </div>
  );
}
