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
  // The calling credential's own identity (#8339) — the name and emoji the
  // chat draws on the user's side of a message, and the role the agent
  // editor's identity controls are gated on.
  //
  // `staleTime: 0` and `gcTime: 0`, deliberately: what this answers is a
  // property of the *credential*, and the credential changes while the page
  // is loaded (App.tsx swaps the login dialog in on a 401 or a re-login,
  // which unmounts every page that reads this). With nothing cached there is
  // nothing for the next mount to inherit, so the previous user's role cannot
  // gate the current user's editor.
  //
  // `retry: false`: a failure here is a failure of the credential, not of the
  // request — a second attempt would carry the same bearer to the same
  // answer. No-auth mode is not a failure in the first place: the middleware
  // admits the caller without an `AuthenticatedApiUser` and `routes/authz.rs`
  // answers 200 with the synthetic root Owner the rest of the surface uses.
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

/** The calling credential's own identity and role (#8339). */
export function useWhoami(options: QueryOverrides = {}) {
  return useQuery(withOverrides(authzQueries.whoami(), options));
}
