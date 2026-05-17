import {
  useMutation,
  useQueryClient,
  type UseMutationOptions,
} from "@tanstack/react-query";
import {
  shutdownServer,
  createBackup,
  restoreBackup,
  deleteBackup,
  deleteTaskFromQueue,
  retryTask,
  cleanupSessions,
} from "../../api";
import {
  agentKeys,
  channelKeys,
  configKeys,
  memoryKeys,
  overviewKeys,
  runtimeKeys,
  scheduleKeys,
  sessionKeys,
  skillKeys,
  triggerKeys,
  cronKeys,
  handKeys,
} from "../queries/keys";

type ShutdownResult = { status: string };

export function useShutdownServer(
  options?: Partial<UseMutationOptions<ShutdownResult, Error, void>>,
) {
  return useMutation<ShutdownResult, Error, void>({
    ...options,
    mutationFn: shutdownServer,
  });
}

export function useCreateBackup() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: createBackup,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: runtimeKeys.backups() });
    },
  });
}

// A backup restore overwrites the entire ~/.librefang data directory
// (agents, memory, sessions, config, channels, schedules, triggers,
// cron, skills, hands). Every cached domain is therefore potentially
// stale — this is the legitimate "cache reset" case for `.all` keys
// described in AGENTS.md, not the narrow per-id default. Without this,
// every page navigated after a restore shows pre-restore state until a
// manual refresh (#5140).
const RESTORE_DIRTIED_KEYS = [
  agentKeys.all,
  memoryKeys.all,
  sessionKeys.all,
  configKeys.all,
  channelKeys.all,
  scheduleKeys.all,
  triggerKeys.all,
  cronKeys.all,
  skillKeys.all,
  handKeys.all,
  runtimeKeys.all,
  overviewKeys.all,
] as const;

export function useRestoreBackup() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: restoreBackup,
    onSuccess: () => {
      for (const queryKey of RESTORE_DIRTIED_KEYS) {
        qc.invalidateQueries({ queryKey });
      }
    },
  });
}

export function useDeleteBackup() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: deleteBackup,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: runtimeKeys.backups() });
    },
  });
}

export function useDeleteTask() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: deleteTaskFromQueue,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: runtimeKeys.tasks() });
      qc.invalidateQueries({ queryKey: runtimeKeys.taskStatus() });
      qc.invalidateQueries({ queryKey: runtimeKeys.queueStatus() });
    },
  });
}

export function useRetryTask() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: retryTask,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: runtimeKeys.tasks() });
      qc.invalidateQueries({ queryKey: runtimeKeys.taskStatus() });
      qc.invalidateQueries({ queryKey: runtimeKeys.queueStatus() });
    },
  });
}

export function useCleanupSessions() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: cleanupSessions,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: sessionKeys.all });
    },
  });
}
