import {
  useMutation,
  useQueryClient,
  type UseMutationOptions,
} from "@tanstack/react-query";
import { setConfigValue, reloadConfig } from "../http/client";
import { configKeys, overviewKeys } from "../queries/keys";

type SetConfigResult = { status: string; restart_required?: boolean };
type SetConfigVars = { path: string; value: unknown };
type BatchSetConfigResult = Array<{
  path: string;
  value: unknown;
  data?: SetConfigResult;
  error?: Error;
}>;
type BatchSetConfigVars = SetConfigVars[];

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
      qc.invalidateQueries({ queryKey: configKeys.full() });
      qc.invalidateQueries({ queryKey: configKeys.schema() });
      qc.invalidateQueries({ queryKey: configKeys.rawToml() });
      options?.onSuccess?.(data, variables, context, meta);
    },
  });
}

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
      qc.invalidateQueries({ queryKey: configKeys.full() });
      qc.invalidateQueries({ queryKey: configKeys.schema() });
      qc.invalidateQueries({ queryKey: configKeys.rawToml() });
      options?.onSuccess?.(data, variables, context, meta);
    },
  });
}

type ReloadConfigResult = {
  status: string;
  restart_required?: boolean;
  restart_reasons?: string[];
};

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
