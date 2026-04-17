import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  activateHand,
  deactivateHand,
  pauseHand,
  resumeHand,
  uninstallHand,
  setHandSecret,
  updateHandSettings,
  sendHandMessage,
  updateSchedule,
  deleteSchedule,
} from "../http/client";
import { handKeys, cronKeys } from "../queries/keys";

export function useActivateHand() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => activateHand(id),
    onSuccess: () => qc.invalidateQueries({ queryKey: handKeys.all }),
  });
}

export function useDeactivateHand() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => deactivateHand(id),
    onSuccess: () => qc.invalidateQueries({ queryKey: handKeys.all }),
  });
}

export function usePauseHand() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => pauseHand(id),
    onSuccess: () => qc.invalidateQueries({ queryKey: handKeys.all }),
  });
}

export function useResumeHand() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => resumeHand(id),
    onSuccess: () => qc.invalidateQueries({ queryKey: handKeys.all }),
  });
}

export function useUninstallHand() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => uninstallHand(id),
    onSuccess: () => qc.invalidateQueries({ queryKey: handKeys.all }),
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

export function useHandScheduleToggle() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      scheduleId,
      enabled,
    }: {
      scheduleId: string;
      enabled: boolean;
    }) => updateSchedule(scheduleId, { enabled }),
    onSuccess: () => qc.invalidateQueries({ queryKey: cronKeys.all }),
  });
}

export function useHandScheduleDelete() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (scheduleId: string) => deleteSchedule(scheduleId),
    onSuccess: () => qc.invalidateQueries({ queryKey: cronKeys.all }),
  });
}
