// Effective-permissions snapshot query — backs the permission simulator.
//
// Pages MUST consume this hook rather than calling `api.*` or `fetch`
// directly. The endpoint is admin-only on the daemon side; the query
// surfaces 404 / 403 through the standard react-query `error` channel
// so the page can render a "user not found" or "forbidden" empty state
// without inline fetch handling.

import { queryOptions, useQuery } from "@tanstack/react-query";
import { getEffectivePermissions } from "../http/client";
import { authzKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

const STALE_MS = 30_000;

export const authzQueries = {
  effective: (name: string) =>
    queryOptions({
      queryKey: authzKeys.effective(name),
      queryFn: () => getEffectivePermissions(name),
      enabled: !!name,
      staleTime: STALE_MS,
      // Don't retry 404s (unknown user) or 403s (caller not admin) — they
      // are deterministic and a refetch storm just hides the message.
      retry: false,
    }),
};

export function useEffectivePermissions(
  name: string,
  options: QueryOverrides = {},
) {
  return useQuery(withOverrides(authzQueries.effective(name), options));
}
