import type { TFunction } from "i18next";
import { CHANGE_SOURCES, type ChangeSource } from "../api";

const KNOWN_CHANGE_SOURCES = new Set<string>(CHANGE_SOURCES);

function isChangeSource(source: string): source is ChangeSource {
  return KNOWN_CHANGE_SOURCES.has(source);
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
