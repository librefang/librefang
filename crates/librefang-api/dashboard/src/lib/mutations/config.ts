import {
  useMutation,
  useQueryClient,
  type UseMutationOptions,
} from "@tanstack/react-query";
import { setConfigValue, reloadConfig } from "../http/client";
import { configKeys, overviewKeys } from "../queries/keys";

type SetConfigResult = { status: string; restart_required?: boolean };
type SetConfigVars = { path: string; value: unknown };

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
