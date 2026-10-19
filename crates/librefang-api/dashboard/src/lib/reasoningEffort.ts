/**
 * Reasoning effort levels — the shared vocabulary between the composer menu,
 * the agent editor and the wire.
 *
 * The wire spelling is the ecosystem's: `reasoning_effort` on the message
 * frame and on the session PATCH; the levels themselves are `"none" |
 * "minimal" | "low" | "medium" | "high" | "xhigh" | "max"`. `reasoning_mode`
 * is the deprecated alias of the field name — the dashboard reads it from old
 * manifests but never emits it.
 *
 * Capability is the rule, and it comes from the model's catalog metadata:
 *
 *   - `supported_efforts` is the model's exact set; a level outside it is
 *     never offered — not armed, not pinned, not selectable in the editor. A
 *     value that still arrives from elsewhere is coerced server-side with a
 *     warning, so the UI shows it as current state, disabled, and never
 *     suggests it.
 *   - `off_allowed: false` (or `mandatory: true` — xAI, Kimi K3, Gemini 3 and
 *     friends) means the model cannot be switched off: `none` is dropped from
 *     the offered set, declared or canonical.
 *   - `default_effort` is the level the provider uses when no explicit value
 *     is sent — what the "inherit/default" option resolves to. Carried on the
 *     source so a surface can name it; no surface does yet.
 *
 * The canonical seven are only a fallback for "nobody said" (older daemon,
 * provider without metadata), because an empty menu would read as "reasoning
 * does not exist" rather than as missing data — and the off-capability filter
 * still applies to them.
 *
 * Noted for when the catalog learns to mark it: a model whose reasoning is
 * shaped by a token budget rather than a level enum should get a budget input
 * instead of a level list. Nothing marks that today.
 */

/** The fallback set, and the levels every agreed surface understands. */
export const CANONICAL_REASONING_EFFORTS = [
  "none",
  "minimal",
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
] as const;

/**
 * Technical names, deliberately not translated: the level a model documents is
 * the level the API takes, and an operator debugging a request wants to see
 * the string they sent. `xHigh` is the one casing the providers themselves use.
 */
const LEVEL_LABELS: Record<string, string> = {
  none: "None",
  minimal: "Minimal",
  low: "Low",
  medium: "Medium",
  high: "High",
  xhigh: "xHigh",
  max: "Max",
};

/** Display label for a level: the technical name, or the raw value capitalized. */
export function reasoningEffortLabel(level: string): string {
  const trimmed = level.trim();
  const known = LEVEL_LABELS[trimmed.toLowerCase()];
  if (known) return known;
  if (!trimmed) return trimmed;
  return trimmed.charAt(0).toUpperCase() + trimmed.slice(1);
}

/** The subset of a catalog entry this module reads. */
export interface ReasoningEffortSource {
  provider?: string;
  id?: string;
  /** The model's exact set of accepted levels. Absent = the canonical set. */
  supported_efforts?: string[];
  /**
   * The level the provider uses when no explicit value is sent (what
   * "inherit/default" resolves to). Presentational today.
   */
  default_effort?: string | null;
  /** `false` = the model cannot be switched off, so `none` is not offered. */
  off_allowed?: boolean;
  /** `true` = reasoning is mandatory; same effect as `off_allowed: false`. */
  mandatory?: boolean;
}

/** True when the model cannot be switched off, so `none` must not be offered. */
const offDisallowed = (model?: ReasoningEffortSource | null): boolean =>
  model?.off_allowed === false || model?.mandatory === true;

/**
 * The levels a model accepts.
 *
 * Absent, empty, or cleaned-to-empty all mean "nobody said": the canonical set
 * applies. A declared list wins as given, minus blanks and duplicates, in the
 * declared order. Either way, `none` is dropped when the model cannot be
 * switched off — the capability flags outrank a list that claims otherwise.
 */
export function resolveReasoningEfforts(
  model?: ReasoningEffortSource | null,
): string[] {
  const declared = model?.supported_efforts;
  let levels: string[];
  if (Array.isArray(declared)) {
    const cleaned: string[] = [];
    for (const level of declared) {
      if (typeof level !== "string") continue;
      const trimmed = level.trim();
      if (!trimmed || cleaned.includes(trimmed)) continue;
      cleaned.push(trimmed);
    }
    levels = cleaned.length > 0 ? cleaned : [...CANONICAL_REASONING_EFFORTS];
  } else {
    levels = [...CANONICAL_REASONING_EFFORTS];
  }
  return offDisallowed(model)
    ? levels.filter((level) => level.toLowerCase() !== "none")
    : levels;
}

/**
 * The levels for the model an agent runs on, resolved from the catalog.
 *
 * Matches on provider and id together first — the same id can exist under two
 * providers with different level sets — then on id alone, for the callers that
 * know the model name but not where it is served from. Both may be spelled
 * `provider/model` in manifests, so that form matches either field.
 */
export function modelReasoningEfforts(
  models: readonly ReasoningEffortSource[],
  model: string,
  provider: string,
): string[] {
  return resolveReasoningEfforts(findReasoningModel(models, model, provider));
}

/**
 * The catalog entry the agent's model resolves to, if any.
 *
 * Exported so a caller that needs more than the level list (`default_effort`
 * for an "inherit resolves to" hint, say) reads the same match the list came
 * from rather than re-implementing it.
 */
export function findReasoningModel(
  models: readonly ReasoningEffortSource[],
  model: string,
  provider: string,
): ReasoningEffortSource | undefined {
  const name = model?.trim() ?? "";
  const vendor = provider?.trim() ?? "";
  if (!name) return undefined;

  const bareName = name.includes("/") ? name.slice(name.lastIndexOf("/") + 1) : name;
  return (
    models.find((m) => m.id === name && (!vendor || m.provider === vendor)) ??
    models.find((m) => m.id === bareName && (!vendor || m.provider === vendor)) ??
    models.find((m) => `${m.provider}/${m.id}` === name) ??
    models.find((m) => m.id === bareName)
  );
}
