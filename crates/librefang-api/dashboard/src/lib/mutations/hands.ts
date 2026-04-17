import { useMutation, useQueryClient, type QueryClient } from "@tanstack/react-query";
import {
  activateHand,
  deactivateHand,
  pauseHand,
  resumeHand,
  uninstallHand,
  setHandSecret,
  updateHandSettings,
  sendHandMessage,
} from "../http/client";
import { agentKeys, handKeys, overviewKeys } from "../queries/keys";

// Schedule toggle/delete hooks that used to live here have been consolidated
// into mutations/schedules.ts (useUpdateSchedule / useDeleteSchedule) so both
// HandsPage and SchedulerPage share one invalidation policy that refreshes
// scheduleKeys AND cronKeys together.

// Hands surface in the agent space (DashboardSnapshot.agents with is_hand: true)
// so lifecycle mutations must invalidate agent + overview caches too.
function invalidateHandAndAgentCaches(qc: QueryClient) {
  qc.invalidateQueries({ queryKey: handKeys.all });
  qc.invalidateQueries({ queryKey: agentKeys.all });
  qc.invalidateQueries({ queryKey: overviewKeys.snapshot() });
}

export function useActivateHand() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => activateHand(id),
    onSuccess: () => invalidateHandAndAgentCaches(qc),
  });
}

export function useDeactivateHand() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => deactivateHand(id),
    onSuccess: () => invalidateHandAndAgentCaches(qc),
  });
}

export function usePauseHand() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => pauseHand(id),
    onSuccess: () => invalidateHandAndAgentCaches(qc),
  });
}

export function useResumeHand() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => resumeHand(id),
    onSuccess: () => invalidateHandAndAgentCaches(qc),
  });
}

export function useUninstallHand() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => uninstallHand(id),
    onSuccess: () => invalidateHandAndAgentCaches(qc),
  });
}

export function useSetHandSecret() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      handId,
      key,
      value,
    }: {
      handId: string;
      key: string;
      value: string;
    }) => setHandSecret(handId, key, value),
    onSuccess: () => qc.invalidateQueries({ queryKey: handKeys.all }),
  });
}

export function useUpdateHandSettings() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      handId,
      config,
    }: {
      handId: string;
      config: Record<string, unknown>;
    }) => updateHandSettings(handId, config),
    onSuccess: () => qc.invalidateQueries({ queryKey: handKeys.all }),
  });
}

export function useSendHandMessage() {
  return useMutation({
    mutationFn: ({
      instanceId,
      message,
    }: {
      instanceId: string;
      message: string;
    }) => sendHandMessage(instanceId, message),
  });
}
