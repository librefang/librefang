// User RBAC queries (Phase 4 / RBAC M6).
//
// Pages MUST consume these hooks rather than calling `api.*` or `fetch`
// directly. Filtering happens client-side because the daemon endpoint
// returns the full list (the `users` array in config.toml is small by
// definition); having a single query keyed on `{}` keeps the cache hot for
// the simulator + identity-linking modal.

import { queryOptions, useQuery } from "@tanstack/react-query";
import {
  currentUserAvatarPath,
  fetchAuthenticatedImage,
  listUsers,
  getUser,
  type UserItem,
} from "../http/client";
import { userKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";
import { AVATAR_STALE_MS } from "./avatar";
import { useObjectUrl } from "../useObjectUrl";

const STALE_MS = 30_000;

export const userQueries = {
  list: () =>
    queryOptions({
      queryKey: userKeys.list({}),
      queryFn: listUsers,
      staleTime: STALE_MS,
    }),
  detail: (name: string) =>
    queryOptions({
      queryKey: userKeys.detail(name),
      queryFn: () => getUser(name),
      enabled: !!name,
      staleTime: STALE_MS,
    }),
  // The signed-in user's avatar as a Blob (#8339), for the reason the agent one
  // is a Blob: `GET /api/users/me/avatar` is authenticated, so an `<img src>`
  // pointed at it carries no bearer token and gets a 401.
  //
  // The request path is literal and `name` is only the cache key — see
  // `currentUserAvatarPath` for why a name never becomes a path segment, and
  // `userKeys.avatar` for why the key is still keyed by one.
  //
  // `enabled` is the caller's "the daemon says there is a picture", read out of
  // `whoami.has_avatar`. The chat already fetches `whoami` for the caller's name
  // and emoji, so the answer costs nothing extra, and gating on it is what
  // keeps a caller who has never uploaded one from paying a 404 per mount.
  // `retry: false` for the same reason it is not worth three attempts to learn
  // there is nothing.
  avatar: (name: string, enabled: boolean) =>
    queryOptions({
      queryKey: userKeys.avatar(name),
      queryFn: () => fetchAuthenticatedImage(currentUserAvatarPath()),
      enabled: !!name && enabled,
      staleTime: AVATAR_STALE_MS,
      retry: false,
    }),
};

/**
 * The signed-in user's avatar as an object URL, ready for an `<img src>`
 * (#8339).
 *
 * `name` is the signed-in user's name, and it is used for the cache key and
 * nothing else; the bytes come from the literal `me` path. `hasAvatar` is
 * `whoami.has_avatar`, and it gates the request. `useObjectUrl` owns the object
 * URL's lifetime and is shared with the agent avatar, where the effect was
 * first written.
 *
 * Returns `undefined` while there is nothing to show — no name yet, no picture
 * according to the daemon, or the request in flight — which is what `Avatar`'s
 * `src` wants: it falls back to the emoji and then to the initials on its own.
 */
export function useUserAvatarUrl(name: string, hasAvatar: boolean): string | undefined {
  const { data: blob } = useQuery(userQueries.avatar(name, hasAvatar));
  return useObjectUrl(blob);
}

function filterUsers(
  users: UserItem[],
  filters: { role?: string; search?: string },
): UserItem[] {
  let out = users;
  if (filters.role && filters.role !== "all") {
    const role = filters.role.toLowerCase();
    out = out.filter(u => u.role.toLowerCase() === role);
  }
  if (filters.search && filters.search.trim()) {
    const q = filters.search.trim().toLowerCase();
    out = out.filter(u => {
      if (u.name.toLowerCase().includes(q)) return true;
      // Match on any platform_id binding so admins can search by Telegram
      // ID etc.
      const bindings = u.channel_bindings;
      if (!bindings || typeof bindings !== "object") return false;
      return Object.values(bindings).some(v =>
        typeof v === "string" && v.toLowerCase().includes(q),
      );
    });
  }
  return out;
}

export function useUsers(
  filters: { role?: string; search?: string } = {},
  options: QueryOverrides = {},
) {
  return useQuery({
    ...withOverrides(userQueries.list(), options),
    select: (users) => filterUsers(users, filters),
  });
}

export function useUser(name: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(userQueries.detail(name), options));
}
