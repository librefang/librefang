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
import { overviewKeys, runtimeKeys, sessionKeys } from "../queries/keys";

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

export function useRestoreBackup() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: restoreBackup,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: runtimeKeys.backups() });
      qc.invalidateQueries({ queryKey: overviewKeys.snapshot() });
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
