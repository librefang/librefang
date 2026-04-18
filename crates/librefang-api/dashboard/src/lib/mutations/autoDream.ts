import { useMutation, useQueryClient } from "@tanstack/react-query";
import { triggerAutoDream } from "../http/client";
import { autoDreamKeys } from "../queries/keys";

/**
 * Manually trigger a consolidation for a specific agent. The outcome
 * arrives immediately — the dream runs detached on the kernel. Invalidating
 * the status query refetches timestamps so the UI reflects the new
 * `last_consolidated_at` once the backend finishes writing.
 */
export function useTriggerAutoDream() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (agentId: string) => triggerAutoDream(agentId),
    onSuccess: () => qc.invalidateQueries({ queryKey: autoDreamKeys.all }),
  });
}
