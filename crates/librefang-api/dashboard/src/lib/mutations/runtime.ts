import {
  useMutation,
  useQueryClient,
  type QueryClient,
  type UseMutationOptions,
} from "@tanstack/react-query";
import {
  shutdownServer,
  createBackup,
  restoreBackup,
  deleteBackup,
  deleteTaskFromQueue,
  retryTask,
  createTask,
  updateTaskStatus,
  cleanupSessions,
  type CreateTaskPayload,
  type CreateTaskResult,
} from "../../api";
import { overviewKeys, runtimeKeys, sessionKeys } from "../queries/keys";

type ShutdownResult = { status: string };
type MutationOptions<TData, TVariables> = Partial<
  Omit<UseMutationOptions<TData, Error, TVariables>, "mutationFn">
>;

function invalidateTaskQueries(qc: QueryClient) {
  qc.invalidateQueries({ queryKey: runtimeKeys.tasks() });
  qc.invalidateQueries({ queryKey: runtimeKeys.taskStatus() });
  qc.invalidateQueries({ queryKey: runtimeKeys.queueStatus() });
}

export function useShutdownServer(
  options?: MutationOptions<ShutdownResult, void>,
) {
  const qc = useQueryClient();
  return useMutation<ShutdownResult, Error, void>({
    ...options,
    mutationFn: shutdownServer,
    onSuccess: async (...args) => {
      await Promise.all([
        qc.cancelQueries({ queryKey: runtimeKeys.all }),
        qc.cancelQueries({ queryKey: overviewKeys.snapshot() }),
      ]);
      qc.removeQueries({ queryKey: runtimeKeys.all });
      qc.removeQueries({ queryKey: overviewKeys.snapshot() });
      await options?.onSuccess?.(...args);
    },
  });
}

export function useCreateBackup(
  options?: MutationOptions<Awaited<ReturnType<typeof createBackup>>, void>,
) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: createBackup,
    ...options,
    onSuccess: (...args) => {
      qc.invalidateQueries({ queryKey: runtimeKeys.backups() });
      options?.onSuccess?.(...args);
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
export type RestoreBackupVars = {
  filename: string;
  keepConfig?: boolean;
  components?: string[];
};

export function useRestoreBackup(
  options?: MutationOptions<Awaited<ReturnType<typeof restoreBackup>>, RestoreBackupVars>,
) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (vars: RestoreBackupVars) =>
      restoreBackup(vars.filename, { keepConfig: vars.keepConfig, components: vars.components }),
    ...options,
    onSuccess: (...args) => {
      qc.invalidateQueries();
      options?.onSuccess?.(...args);
    },
  });
}

export function useDeleteBackup(
  options?: MutationOptions<Awaited<ReturnType<typeof deleteBackup>>, string>,
) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: deleteBackup,
    ...options,
    onSuccess: (...args) => {
      qc.invalidateQueries({ queryKey: runtimeKeys.backups() });
      options?.onSuccess?.(...args);
    },
  });
}

export function useDeleteTask(
  options?: MutationOptions<Awaited<ReturnType<typeof deleteTaskFromQueue>>, string>,
) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: deleteTaskFromQueue,
    ...options,
    onSuccess: (...args) => {
      invalidateTaskQueries(qc);
      options?.onSuccess?.(...args);
    },
  });
}

export function useRetryTask(
  options?: MutationOptions<Awaited<ReturnType<typeof retryTask>>, string>,
) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: retryTask,
    ...options,
    onSuccess: (...args) => {
      invalidateTaskQueries(qc);
      options?.onSuccess?.(...args);
    },
  });
}

export function useCreateTask(
  options?: MutationOptions<CreateTaskResult, CreateTaskPayload>,
) {
  const qc = useQueryClient();
  return useMutation<CreateTaskResult, Error, CreateTaskPayload>({
    ...options,
    mutationFn: createTask,
    onSuccess: (...args) => {
      invalidateTaskQueries(qc);
      options?.onSuccess?.(...args);
    },
  });
}

export function useUpdateTaskStatus(
  options?: MutationOptions<{ status?: string; id?: string }, { id: string; status: "pending" | "cancelled" }>,
) {
  const qc = useQueryClient();
  return useMutation<{ status?: string; id?: string }, Error, { id: string; status: "pending" | "cancelled" }>({
    ...options,
    mutationFn: ({ id, status }) => updateTaskStatus(id, status),
    onSuccess: (...args) => {
      invalidateTaskQueries(qc);
      options?.onSuccess?.(...args);
    },
  });
}

export function useCleanupSessions(
  options?: MutationOptions<Awaited<ReturnType<typeof cleanupSessions>>, void>,
) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: cleanupSessions,
    ...options,
    onSuccess: (...args) => {
      qc.invalidateQueries({ queryKey: sessionKeys.all });
      options?.onSuccess?.(...args);
    },
  });
}
