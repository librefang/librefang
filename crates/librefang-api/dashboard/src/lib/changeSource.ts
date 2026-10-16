import type { TFunction } from "i18next";
import { CHANGE_SOURCES, SKILL_CHANGE_SOURCES, type ChangeSource, type SkillChangeSource } from "../api";

const KNOWN_CHANGE_SOURCES = new Set<string>(CHANGE_SOURCES);
const KNOWN_SKILL_CHANGE_SOURCES = new Set<string>(SKILL_CHANGE_SOURCES);

function isChangeSource(source: string): source is ChangeSource {
  return KNOWN_CHANGE_SOURCES.has(source);
}

function isSkillChangeSource(source: string): source is SkillChangeSource {
  return KNOWN_SKILL_CHANGE_SOURCES.has(source);
}

/**
 * Operator-facing label for a stored version's `change_source` (#8394).
 *
 * The value is an internal token the server writes as free text, so it is looked up rather than shown: a raw `dashboard` spliced into a translated confirm sentence reads as neither language.
 * A value outside `CHANGE_SOURCES` — a producer the dashboard has not been taught, or a row from an older database — comes back verbatim, because provenance in a destructive-confirm dialog is worth more than a tidy blank.
 * Membership is checked before calling `t` so a dotted token is never resolved as a nested locale path.
 */
export function changeSourceLabel(t: TFunction, source: string): string {
  return isChangeSource(source) ? t(`agentTypes.change_source.${source}`) : source;
}

/**
 * Operator-facing label for a stored skill version's `change_source` (#8021).
 *
 * Same contract as [`changeSourceLabel`], against the skill version store's own vocabulary — which
 * is distinct from the template and manifest stores': the store writes `evolve`, `patch`,
 * `file-write`, … and a raw token spliced into the destructive restore sentence reads as neither
 * language.
 * A value outside `SKILL_CHANGE_SOURCES` — a producer the dashboard has not been taught, or a row
 * from an older database — comes back verbatim, because provenance in a destructive-confirm dialog
 * is worth more than a tidy blank.
 * Membership is checked before calling `t` so a dotted token is never resolved as a nested locale path.
 */
export function skillChangeSourceLabel(t: TFunction, source: string): string {
  return isSkillChangeSource(source)
    ? t(`skills.change_source.${source}`)
    : source;
}
