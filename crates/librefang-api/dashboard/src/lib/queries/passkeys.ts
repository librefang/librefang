import { queryOptions, useQuery } from "@tanstack/react-query";
import { listPasskeys } from "../../api";
import { passkeyKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

const STALE_PASSKEYS = 30_000;

export const passkeyQueries = {
  list: () =>
    queryOptions({
      queryKey: passkeyKeys.list(),
      queryFn: listPasskeys,
      staleTime: STALE_PASSKEYS,
    }),
};

export function usePasskeys(options: QueryOverrides = {}) {
  return useQuery(withOverrides(passkeyQueries.list(), options));
}
