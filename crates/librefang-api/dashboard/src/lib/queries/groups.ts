// User-group queries (#7745).
//
// Pages MUST consume these hooks rather than calling `api.*` or `fetch`
// directly. Filtering is client-side for the same reason the users surface
// does it: the daemon returns the whole `[[groups]]` array, which is small by
// construction (it is an operator-edited section of `config.toml`), so one
// query keyed on `{}` keeps the cache hot for the list page and for any
// membership picker that needs the same rows.

import { queryOptions, useQuery } from "@tanstack/react-query";
import { listGroups, getGroup, getUserGroups, type GroupItem } from "../http/client";
import { groupKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

const STALE_MS = 30_000;

export const groupQueries = {
  list: () =>
    queryOptions({
      queryKey: groupKeys.list({}),
      queryFn: listGroups,
      staleTime: STALE_MS,
    }),
  detail: (name: string) =>
    queryOptions({
      queryKey: groupKeys.detail(name),
      queryFn: () => getGroup(name),
      enabled: !!name,
      staleTime: STALE_MS,
    }),
  membership: (user: string) =>
    queryOptions({
      queryKey: groupKeys.membership(user),
      queryFn: () => getUserGroups(user),
      enabled: !!user,
      staleTime: STALE_MS,
    }),
};

function filterGroups(groups: GroupItem[], filters: { search?: string }): GroupItem[] {
  const q = filters.search?.trim().toLowerCase();
  if (!q) return groups;
  return groups.filter(g => {
    if (g.name.toLowerCase().includes(q)) return true;
    if (g.description.toLowerCase().includes(q)) return true;
    // Search by member so an operator can answer "which teams is alice on"
    // from the list page without opening every row.
    if (g.members.some(m => m.toLowerCase().includes(q))) return true;
    return g.roles.some(r => r.toLowerCase().includes(q));
  });
}

export function useGroups(
  filters: { search?: string } = {},
  options: QueryOverrides = {},
) {
  return useQuery({
    ...withOverrides(groupQueries.list(), options),
    select: (groups) => filterGroups(groups, filters),
  });
}

export function useGroup(name: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(groupQueries.detail(name), options));
}

// Reverse lookup: the groups a user belongs to plus the roles that confers.
// Backs the group column on the users page and, later, the ownership filters
// #7744 describes.
export function useUserGroups(user: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(groupQueries.membership(user), options));
}
