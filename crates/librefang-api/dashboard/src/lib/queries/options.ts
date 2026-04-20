export type QueryOverrides = {
  enabled?: boolean;
  staleTime?: number;
  refetchInterval?: number | false;
};

export function withOverrides<T>(base: T, overrides: QueryOverrides): T {
  const out = { ...base } as Record<string, unknown>;
  if (overrides.enabled !== undefined) out.enabled = overrides.enabled;
  if (overrides.staleTime !== undefined) out.staleTime = overrides.staleTime;
  if (overrides.refetchInterval !== undefined) out.refetchInterval = overrides.refetchInterval;
  return out as T;
}
