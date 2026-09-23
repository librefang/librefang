import { queryOptions, useQuery } from "@tanstack/react-query";
import { listVaultKeys } from "../http/client";
import { vaultKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

// Operator-facing and rarely changing; nothing polls it in the background.
const STALE_MS = 30_000;

export const vaultQueries = {
  list: () =>
    queryOptions({
      queryKey: vaultKeys.list(),
      queryFn: listVaultKeys,
      staleTime: STALE_MS,
    }),
};

export function useVaultKeys(options: QueryOverrides = {}) {
  return useQuery(withOverrides(vaultQueries.list(), options));
}
