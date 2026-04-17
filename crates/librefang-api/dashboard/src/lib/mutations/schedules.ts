import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  createSchedule,
  updateSchedule,
  deleteSchedule,
  runSchedule,
  updateTrigger,
  deleteTrigger,
} from "../http/client";
import { scheduleKeys, triggerKeys } from "../queries/keys";

export function useCreateSchedule() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: createSchedule,
    onSuccess: () => qc.invalidateQueries({ queryKey: scheduleKeys.all }),
  });
}

export function useUpdateSchedule() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, data }: { id: string; data: Parameters<typeof updateSchedule>[1] }) =>
      updateSchedule(id, data),
    onSuccess: () => qc.invalidateQueries({ queryKey: scheduleKeys.all }),
  });
}

export function useDeleteSchedule() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: deleteSchedule,
    onSuccess: () => qc.invalidateQueries({ queryKey: scheduleKeys.all }),
  });
}

export function useRunSchedule() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: runSchedule,
    onSuccess: () => qc.invalidateQueries({ queryKey: scheduleKeys.all }),
  });
}

export function useUpdateTrigger() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, data }: { id: string; data: { enabled: boolean } }) =>
      updateTrigger(id, data),
    onSuccess: () => qc.invalidateQueries({ queryKey: triggerKeys.all }),
  });
}

export function useDeleteTrigger() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: deleteTrigger,
    onSuccess: () => qc.invalidateQueries({ queryKey: triggerKeys.all }),
  });
}
