import type { ModelItem } from "../api";

/** Build the storage key for a model: "provider:id" */
export function modelKey(m: Pick<ModelItem, "provider" | "id">): string {
  return `${m.provider}:${m.id}`;
}

/** Filter to only visible (non-hidden) models */
export function filterVisible(models: ModelItem[], hiddenKeys: Set<string>): ModelItem[] {
  return models.filter(m => !hiddenKeys.has(modelKey(m)));
}
