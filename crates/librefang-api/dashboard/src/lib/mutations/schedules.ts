import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  createSchedule,
  updateSchedule,
  deleteSchedule,
  runSchedule,
  createTrigger,
  updateTrigger,
  deleteTrigger,
} from "../http/client";
import type { TriggerPatch, CreateTriggerPayload } from "../../api";
import { cronKeys, scheduleKeys, triggerKeys, workflowKeys } from "../queries/keys";

// Schedules surface in two views: SchedulerPage (via useSchedules →
// scheduleKeys) and HandsPage's cron widget (via useCronJobs → cronKeys).
// Every write MUST invalidate both slices so acting from one page never
// leaves the other showing stale data.
function invalidateScheduleCaches(qc: ReturnType<typeof useQueryClient>) {
  return Promise.all([
    qc.invalidateQueries({ queryKey: scheduleKeys.all }),
    qc.invalidateQueries({ queryKey: cronKeys.all }),
    qc.invalidateQueries({ queryKey: workflowKeys.lists() }),
  ]);
}

export function useCreateSchedule() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: createSchedule,
    onSuccess: () => invalidateScheduleCaches(qc),
  });
}

export function useUpdateSchedule() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, data }: { id: string; data: Parameters<typeof updateSchedule>[1] }) =>
      updateSchedule(id, data),
    onSuccess: () => invalidateScheduleCaches(qc),
  });
}

export function useDeleteSchedule() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: deleteSchedule,
    onSuccess: () => invalidateScheduleCaches(qc),
  });
}

export function useRunSchedule() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: runSchedule,
    onSuccess: () => invalidateScheduleCaches(qc),
  });
}

export function useCreateTrigger() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (payload: CreateTriggerPayload) => createTrigger(payload),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: triggerKeys.all });
      qc.invalidateQueries({ queryKey: cronKeys.all });
    },
  });
}

export function useUpdateTrigger() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, data }: { id: string; data: TriggerPatch }) =>
      updateTrigger(id, data),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: triggerKeys.all });
      qc.invalidateQueries({ queryKey: cronKeys.all });
    },
  });
}

export function useDeleteTrigger() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: deleteTrigger,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: triggerKeys.all });
      qc.invalidateQueries({ queryKey: cronKeys.all });
    },
  });
}
