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
import type { CronDeliveryTarget } from "../http/client";
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

/**
 * Replace a schedule's fan-out `delivery_targets` list.
 *
 * The backend treats `delivery_targets` as a full replace — passing an
 * empty array clears all configured fan-out destinations (legacy single
 * `delivery` is unaffected). Internally this is just `updateSchedule`
 * with only the `delivery_targets` field set, but exposing it as its own
 * hook keeps call sites simple and keeps the cache-invalidation contract
 * identical to other schedule writes.
 */
export function useSetScheduleDeliveryTargets() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, targets }: { id: string; targets: CronDeliveryTarget[] }) =>
      updateSchedule(id, { delivery_targets: targets }),
    onSuccess: () => invalidateScheduleCaches(qc),
  });
}

// Trigger writes must invalidate triggerKeys.all (not just the per-agent
// sub-key) so that SchedulerPage — which queries triggerKeys.lists() without
// an agentId — also refreshes.  triggerKeys.list(agentId) is a strictly
// longer key; react-query prefix matching never reaches the shorter lists()
// key from it.
function invalidateTriggerCaches(qc: ReturnType<typeof useQueryClient>) {
  return Promise.all([
    qc.invalidateQueries({ queryKey: triggerKeys.all }),
    qc.invalidateQueries({ queryKey: cronKeys.all }),
  ]);
}

export function useCreateTrigger() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (payload: CreateTriggerPayload) => createTrigger(payload),
    onSuccess: () => invalidateTriggerCaches(qc),
  });
}

export function useUpdateTrigger() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, data }: { id: string; data: TriggerPatch; agentId?: string }) =>
      updateTrigger(id, data),
    onSuccess: () => invalidateTriggerCaches(qc),
  });
}

export function useDeleteTrigger() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id }: { id: string; agentId?: string }) => deleteTrigger(id),
    onSuccess: (_data, { id }) => {
      qc.removeQueries({ queryKey: triggerKeys.detail(id) });
      return invalidateTriggerCaches(qc);
    },
  });
}
