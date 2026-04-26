// Per-user policy queries (M3 / #3205 stub).
//
// The endpoint `/api/users/{name}/policy` is owned by RBAC M3. The hook
// ships now so the matrix-editor page only needs the placeholder swap when
// M3 lands.

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
