/** Per-call overrides for a shared query factory.
 *
 *  `TData` names what the query resolves to so the functional
 *  `refetchInterval` form can read `query.state.data` without a cast; it
 *  defaults to `unknown` for the callers that only pass literals.
 *
 *  The callback parameter is the structural slice of TanStack's `Query` that
 *  a polling predicate actually reads, not `Query` itself: `Query` is
 *  invariant in its query-key type, so naming it here would reject every hook
 *  whose factory uses a tuple key. */
export type QueryOverrides<TData = unknown> = {
  enabled?: boolean;
  staleTime?: number;
  refetchInterval?:
    | number
    | false
    | ((query: { state: { data: TData | undefined } }) => number | false | undefined);
};

export function withOverrides<T, TData = unknown>(
  base: T,
  overrides: QueryOverrides<TData>,
): T {
  const out = { ...base } as Record<string, unknown>;
  if (overrides.enabled !== undefined) out.enabled = overrides.enabled;
  if (overrides.staleTime !== undefined) out.staleTime = overrides.staleTime;
  if (overrides.refetchInterval !== undefined) out.refetchInterval = overrides.refetchInterval;
  return out as T;
}
