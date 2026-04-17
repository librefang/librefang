import {
  useMutation,
  useQueryClient,
  type UseMutationOptions,
} from "@tanstack/react-query";
import {
  shutdownServer,
  reloadConfig,
  createBackup,
  restoreBackup,
  deleteBackup,
  deleteTaskFromQueue,
  retryTask,
  cleanupSessions,
} from "../../api";
import { overviewKeys, runtimeKeys, sessionKeys } from "../queries/keys";

type ShutdownResult = { status: string };
type ReloadResult = { status: string; restart_required?: boolean; restart_reasons?: string[] };

export function useShutdownServer(
  options?: Partial<UseMutationOptions<ShutdownResult, Error, void>>,
) {
  return useMutation<ShutdownResult, Error, void>({
    ...options,
    mutationFn: shutdownServer,
  });
}

export function useReloadConfig(
  options?: Partial<UseMutationOptions<ReloadResult, Error, void>>,
) {
  const queryClient = useQueryClient();
  return useMutation<ReloadResult, Error, void>({
    ...options,
    mutationFn: reloadConfig,
    onSuccess: (data, vars, ctx, meta) => {
      queryClient.invalidateQueries({ queryKey: overviewKeys.snapshot() });
      options?.onSuccess?.(data, vars, ctx, meta);
    },
  });
}

export function useCreateBackup() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: createBackup,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: runtimeKeys.backups() });
    },
  });
}

export function useRestoreBackup() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: restoreBackup,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: runtimeKeys.backups() });
      queryClient.invalidateQueries({ queryKey: overviewKeys.snapshot() });
    },
  });
}

export function useDeleteBackup() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: deleteBackup,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: runtimeKeys.backups() });
    },
  });
}

export function useDeleteTask() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: deleteTaskFromQueue,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: runtimeKeys.tasks() });
    },
  });
}

export function useRetryTask() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: retryTask,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: runtimeKeys.tasks() });
    },
  });
}

export function useCleanupSessions() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: cleanupSessions,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: sessionKeys.all });
    },
  });
}
