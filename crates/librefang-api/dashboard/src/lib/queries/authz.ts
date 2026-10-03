// Effective-permissions snapshot query — backs the permission simulator.
//
// Pages MUST consume this hook rather than calling `api.*` or `fetch`
// directly. The endpoint is admin-only on the daemon side; the query
// surfaces 404 / 403 through the standard react-query `error` channel
// so the page can render a "user not found" or "forbidden" empty state
// without inline fetch handling.

import { queryOptions, useQuery } from "@tanstack/react-query";
import { ApiError, getEffectivePermissions, getWhoami } from "../http/client";
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
      retry: (failureCount, error) => {
        if (error instanceof ApiError && (error.status === 403 || error.status === 404)) {
          return false;
        }
        return failureCount < 3;
      },
    }),
  // The calling credential's own identity (#8339) — the name and emoji the chat
  // draws on the user's side of a message.
  //
  // `staleTime: 0` and `gcTime: 0`, deliberately, where the queries above use
  // long windows. What this answers is a property of the *credential*, and the
  // credential changes while the page is loaded: `App.tsx` renders the login
  // dialog in place of the router once `authNeeded` is set (see its
  // `setOnUnauthorized`), and again on a re-login. Either way every page that
  // reads this unmounts, and with nothing cached there is nothing for the next
  // mount to inherit — so the previous user's name cannot end up on the current
  // user's messages.
  //
  // Retrying is wrong in no-auth mode, where the route answers 401 and the
  // caller is legitimate.
  whoami: () =>
    queryOptions({
      queryKey: authzKeys.whoami(),
      queryFn: () => getWhoami(),
      staleTime: 0,
      gcTime: 0,
      retry: false,
    }),
};

export function useEffectivePermissions(
  name: string,
  options: QueryOverrides = {},
) {
  return useQuery(withOverrides(authzQueries.effective(name), {
    ...options,
    enabled: Boolean(name) && options.enabled !== false,
  }));
}

export function useWhoami(options: QueryOverrides = {}) {
  return useQuery(withOverrides(authzQueries.whoami(), options));
}
