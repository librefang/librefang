// User RBAC queries (Phase 4 / RBAC M6).
//
// Pages MUST consume these hooks rather than calling `api.*` or `fetch`
// directly. Filtering happens client-side because the daemon endpoint
// returns the full list (the `users` array in config.toml is small by
// definition); having a single query keyed on `{}` keeps the cache hot for
// the simulator + identity-linking modal.

import { queryOptions, useQuery } from "@tanstack/react-query";
import { listUsers, getUser, type UserItem } from "../http/client";
import { userKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

const STALE_MS = 30_000;

export const userQueries = {
  list: (filters: { role?: string; search?: string } = {}) =>
    queryOptions({
      queryKey: userKeys.list(filters),
      queryFn: async () => {
        const all = await listUsers();
        return filterUsers(all, filters);
      },
      staleTime: STALE_MS,
    }),
  detail: (name: string) =>
    queryOptions({
      queryKey: userKeys.detail(name),
      queryFn: () => getUser(name),
      enabled: !!name,
      staleTime: STALE_MS,
    }),
};

function filterUsers(
  users: UserItem[],
  filters: { role?: string; search?: string },
): UserItem[] {
  let out = users;
  if (filters.role && filters.role !== "all") {
    out = out.filter(u => u.role === filters.role);
  }
  if (filters.search && filters.search.trim()) {
    const q = filters.search.trim().toLowerCase();
    out = out.filter(u => {
      if (u.name.toLowerCase().includes(q)) return true;
      // Match on any platform_id binding so admins can search by Telegram
      // ID etc.
      return Object.values(u.channel_bindings).some(v =>
        v.toLowerCase().includes(q),
      );
    });
  }
  return out;
}

export function useUsers(
  filters: { role?: string; search?: string } = {},
  options: QueryOverrides = {},
) {
  return useQuery(withOverrides(userQueries.list(filters), options));
}

export function useUser(name: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(userQueries.detail(name), options));
}
