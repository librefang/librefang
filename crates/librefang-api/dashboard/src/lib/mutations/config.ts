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
import { buildMediaEndpointPayload } from "../mediaModelEndpoints";
import type { MediaModelEndpoint, MediaModelEndpointDraft } from "../../api";

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

export type SaveMediaModelEndpointVars = {
  endpoint: MediaModelEndpoint;
  draft: MediaModelEndpointDraft;
  /**
   * Provider name that selects this endpoint (`media.audio_provider`,
   * `tts.provider`, …). Sent only when it differs from what the config already
   * reports, so an untouched selector is not rewritten on every save.
   */
  provider: string;
};

export type SaveMediaModelEndpointResult = {
  /** One entry per `POST /api/config/set` this save issued, in the order sent. */
  writes: { path: string; value: unknown; data: SetConfigResult }[];
};

/**
 * Persist one custom media endpoint through the config API that already
 * serves it (refs #8038, #8011).
 *
 * Two writes at most, in this order: the endpoint table (`media.custom_stt`,
 * `tts.custom`, …) and then the provider selector, so a provider name is never
 * pointed at a table that has not been written yet. Both are sequential rather
 * than concurrent because `POST /api/config/set` serializes on one
 * `config.toml` read-modify-write lock — issuing them in parallel just queues
 * them behind each other with a less predictable order.
 */
export function useSaveMediaModelEndpoint(
  options?: Partial<
    UseMutationOptions<SaveMediaModelEndpointResult, Error, SaveMediaModelEndpointVars>
  >,
) {
  const qc = useQueryClient();
  return useMutation<SaveMediaModelEndpointResult, Error, SaveMediaModelEndpointVars>({
    ...options,
    mutationFn: async ({ endpoint, draft, provider }) => {
      const writes: SaveMediaModelEndpointResult["writes"] = [];
      const table = buildMediaEndpointPayload(endpoint, draft);
      writes.push({
        path: endpoint.config_path,
        value: table,
        data: await setConfigValue(endpoint.config_path, table),
      });
      const nextProvider = provider.trim();
      if (nextProvider !== endpoint.provider) {
        // `null` removes the key rather than writing an empty string, which is
        // what `Option<String>` provider selectors expect for "unset".
        const value = nextProvider === "" ? null : nextProvider;
        writes.push({
          path: endpoint.provider_path,
          value,
          data: await setConfigValue(endpoint.provider_path, value),
        });
      }
      return { writes };
    },
    onSuccess: (data, variables, context, meta) => {
      // `full` holds the endpoint tables and `rawToml` the file they were
      // written into, so the whole domain is dirty after either write.
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
