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
 *   - `offered_efforts` is the backend's own answer (`ReasoningEffortPolicy::
 *     offered()`, served by #8616 and later). When the payload carries it, it
 *     is used verbatim — the list already has the declaration cleaned, `none`
 *     removed from the declared rungs, and `none` put back only when the model
 *     allows it. Nothing is re-derived from the other fields.
 *   - Without it (current main and older daemons), `supported_efforts` is the
 *     model's declaration; `none` is never part of it. The offered list is
 *     built the way the backend builds it: the declared graded rungs, with
 *     `none` prepended when the model may be switched off.
 *   - `off_allowed: false` (or `mandatory: true` — xAI, Kimi K3, Gemini 3 and
 *     friends) means the model cannot be switched off: `none` is not offered.
 *   - `default_effort` is the level the provider uses when no explicit value
 *     is sent — what the "inherit/default" option resolves to. Carried on the
 *     source so a surface can name it; no surface does yet.
 *
 * A level outside the offered list is never armed, pinned or selectable. A
 * value that still arrives from elsewhere is shown as current state, disabled,
 * never suggested: the session route stores a validated token as-is and a
 * driver may coerce it at turn time, so offering it would misstate what runs.
 *
 * The canonical seven are only a fallback for "nobody said" (older daemon,
 * provider without metadata, and no `offered_efforts` field), because an empty
 * menu would read as "reasoning does not exist" rather than as missing data —
 * and the off-capability filter still applies to them.
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
  /** The model's declared levels (`none` excluded). Absent = nobody said. */
  supported_efforts?: string[];
  /**
   * The backend's prepared offer (`ReasoningEffortPolicy::offered()`): the
   * declared rungs, plus `none` when the model allows it. Present on #8616 and
   * later; when present it wins over anything re-derived here.
   */
  offered_efforts?: string[];
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
 * Trim blanks and duplicates out of a level list, preserving order.
 *
 * An empty result means "nobody said" for `supported_efforts`; for
 * `offered_efforts` it is authoritative and stays empty.
 */
function cleanLevels(levels: readonly unknown[]): string[] {
  const cleaned: string[] = [];
  for (const level of levels) {
    if (typeof level !== "string") continue;
    const trimmed = level.trim();
    if (!trimmed || cleaned.includes(trimmed)) continue;
    cleaned.push(trimmed);
  }
  return cleaned;
}

/**
 * The levels a model offers, in the order a menu should show them.
 *
 * `offered_efforts` from the payload wins whenever the field is present — it
 * is the backend's own answer and the only source that stays right when the
 * offer rules change. Otherwise the list mirrors `offered()`: the declared
 * graded rungs (never `none`, even if the declaration lists it), with `none`
 * prepended when `off_allowed !== false && mandatory !== true`.
 *
 * An undeclared model keeps the canonical fallback — but only while the
 * payload has no `offered_efforts`. Once the backend serves that field, its
 * answer (possibly empty) is authoritative, and main compatibility is no
 * longer a reason to guess.
 */
export function resolveReasoningEfforts(
  model?: ReasoningEffortSource | null,
): string[] {
  if (Array.isArray(model?.offered_efforts)) {
    return cleanLevels(model.offered_efforts);
  }

  const declared = model?.supported_efforts;
  const cleaned = Array.isArray(declared) ? cleanLevels(declared) : [];
  // A list that cleans to nothing is "nobody said" (canonical fallback); a
  // declaration that is non-empty but contains only `none` is a real, empty
  // set of graded rungs, and must not widen to the canonical ones.
  const rungs =
    cleaned.length > 0
      ? cleaned.filter((level) => level !== "none")
      : CANONICAL_REASONING_EFFORTS.filter((level) => level !== "none");

  return offDisallowed(model) ? rungs : ["none", ...rungs];
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
  const entry = findReasoningModel(models, model, provider);
  if (entry) return resolveReasoningEfforts(entry);
  // A catalog miss is not "undeclared": the model may simply be absent from
  // the filtered list (custom model, `available: true`, or the query has not
  // resolved). Once the payload serves `offered_efforts` on any model, the
  // backend can answer for every model it knows, so a miss must offer
  // nothing rather than guess the canonical seven (#8617 review).
  const capabilityAware = models.some((candidate) =>
    Array.isArray(candidate.offered_efforts),
  );
  return capabilityAware ? [] : resolveReasoningEfforts(null);
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
