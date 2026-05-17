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
import { runtimeKeys, sessionKeys } from "../queries/keys";

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

// A backup restore overwrites the entire ~/.librefang data directory:
// workflows/, data/ (the SQLite substrate backing approvals, usage,
// budgets, mcp, plugins, totp, peers, network, audit, a2a, media,
// users, permission policies, authz), data/custom_models.json, and
// config.toml (which carries provider config). Every cached domain in
// the dashboard is therefore potentially stale. Enumerating each
// domain key here repeatedly drifted from what backup.rs actually
// archives (#5182), so we treat this as a daemon-restart level cache
// reset and nuke the entire query cache in one call — this is the
// legitimate "cache reset" case for blanket invalidation described in
// AGENTS.md, not the narrow per-id default. Without this, every page
// navigated after a restore shows pre-restore state until a manual
// refresh (#5140).
export function useRestoreBackup() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: restoreBackup,
    onSuccess: () => {
      qc.invalidateQueries();
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
