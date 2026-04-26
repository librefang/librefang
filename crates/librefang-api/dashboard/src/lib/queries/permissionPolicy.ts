// Per-user policy queries (RBAC M3 / #3205, wired to the live daemon).
//
// `GET /api/users/{name}/policy` returns the per-user `tool_policy` /
// `tool_categories` / `memory_access` / `channel_tool_rules` slice. The
// matrix editor in `UserPolicyPage` reads this to populate the form.

import { queryOptions, useQuery } from "@tanstack/react-query";
import { getUserPolicy } from "../http/client";
import { permissionPolicyKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

const STALE_MS = 60_000;

export const permissionPolicyQueries = {
  detail: (name: string) =>
    queryOptions({
      queryKey: permissionPolicyKeys.detail(name),
      queryFn: () => getUserPolicy(name),
      enabled: !!name,
      staleTime: STALE_MS,
    }),
};

export function usePermissionPolicy(name: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(permissionPolicyQueries.detail(name), options));
}
