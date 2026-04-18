import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  abortAutoDream,
  setAutoDreamEnabled,
  triggerAutoDream,
} from "../http/client";
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

/**
 * Abort an in-flight manually-triggered dream. Scheduled dreams cannot be
 * aborted — the endpoint returns `{aborted: false}` with a reason in that
 * case. Invalidate the status query so the progress card transitions from
 * "running" to "aborted" in one refetch.
 */
export function useAbortAutoDream() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (agentId: string) => abortAutoDream(agentId),
    onSuccess: () => qc.invalidateQueries({ queryKey: autoDreamKeys.all }),
  });
}

/**
 * Toggle an agent's `auto_dream_enabled` opt-in flag. In-memory update —
 * the scheduler picks it up on the next tick. Invalidate so the settings
 * card reflects the new toggle state.
 */
export function useSetAutoDreamEnabled() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ agentId, enabled }: { agentId: string; enabled: boolean }) =>
      setAutoDreamEnabled(agentId, enabled),
    onSuccess: () => qc.invalidateQueries({ queryKey: autoDreamKeys.all }),
  });
}
