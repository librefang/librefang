/**
 * How long a cached avatar image stays fresh, for both the agent's and the
 * signed-in user's (#8339).
 *
 * It lives here rather than in either module because the number is not a
 * property of agents or of users — it is a property of the route that serves
 * the bytes, and both routes serve them the same way: an `ETag` with
 * `no-cache`, so a revalidation that finds nothing changed is a bodiless 304.
 *
 * The long window is what that buys. A re-upload is picked up by the mutations
 * invalidating the key, not by polling for it, so there is nothing to gain from
 * asking more often.
 */
export const AVATAR_STALE_MS = 300_000;
