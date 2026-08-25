import type { ModelItem } from "../api";

/** Build an unambiguous storage key for a model. */
export function modelKey(m: Pick<ModelItem, "provider" | "id">): string {
  return `${m.provider}\u001f${m.id}`;
}

/** Filter to only visible (non-hidden) models */
export function filterVisible(models: ModelItem[], hiddenKeys: Set<string>): ModelItem[] {
  return models.filter(m => !hiddenKeys.has(modelKey(m)));
}
