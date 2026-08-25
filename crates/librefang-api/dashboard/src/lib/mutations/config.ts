import {
  useMutation,
  useQueryClient,
  type UseMutationOptions,
} from "@tanstack/react-query";
import {
  setConfigValue,
  reloadConfig,
  type ReloadConfigResult,
} from "../http/client";
import { configKeys, overviewKeys } from "../queries/keys";

export type SetConfigResult = {
  status: string;
  restart_required?: boolean;
  reload_error?: string;
};
type SetConfigVars = { path: string; value: unknown };
export type BatchSetConfigItem = {
  path: string;
  value: unknown;
  data?: SetConfigResult;
  error?: Error;
};
export type BatchSetConfigResult = BatchSetConfigItem[];
type BatchSetConfigVars = SetConfigVars[];

export function hasBatchConfigErrors(results: BatchSetConfigResult): boolean {
  return results.some((result) => result.error !== undefined);
}

export function useSetConfigValue(
  options?: Partial<
    UseMutationOptions<SetConfigResult, Error, SetConfigVars>
  >,
) {
  const qc = useQueryClient();
  return useMutation<SetConfigResult, Error, SetConfigVars>({
    ...options,
    mutationFn: ({ path, value }) => setConfigValue(path, value),
    onSuccess: (data, variables, context, meta) => {
      qc.invalidateQueries({ queryKey: configKeys.all });
      options?.onSuccess?.(data, variables, context, meta);
    },
  });
}

/**
 * Save every entry independently and always resolve with one result per input.
 *
 * Partial success is intentional: ConfigPage clears successfully persisted
 * drafts while retaining failed fields for retry. Callers that only need an
 * aggregate outcome should use `hasBatchConfigErrors` on the resolved value.
 */
export function useBatchSetConfigValues(
  options?: Partial<
    UseMutationOptions<BatchSetConfigResult, Error, BatchSetConfigVars>
  >,
) {
  const qc = useQueryClient();
  return useMutation<BatchSetConfigResult, Error, BatchSetConfigVars>({
    ...options,
    mutationFn: async (entries) => Promise.all(entries.map(async ({ path, value }) => {
      try {
        const data = await setConfigValue(path, value);
        return { path, value, data };
      } catch (error) {
        return {
          path,
          value,
          error: error instanceof Error ? error : new Error(String(error)),
        };
      }
    })),
    onSuccess: (data, variables, context, meta) => {
      qc.invalidateQueries({ queryKey: configKeys.all });
      options?.onSuccess?.(data, variables, context, meta);
    },
  });
}

export function useReloadConfig(
  options?: Partial<
    UseMutationOptions<ReloadConfigResult, Error, void>
  >,
) {
  const qc = useQueryClient();
  return useMutation<ReloadConfigResult, Error, void>({
    ...options,
    mutationFn: reloadConfig,
    onSuccess: (data, variables, context, meta) => {
      qc.invalidateQueries({ queryKey: configKeys.all });
      qc.invalidateQueries({ queryKey: overviewKeys.snapshot() });
      options?.onSuccess?.(data, variables, context, meta);
    },
  });
}
