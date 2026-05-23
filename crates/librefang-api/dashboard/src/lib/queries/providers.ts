import { queryOptions, useQuery } from "@tanstack/react-query";
import { listCredentialPools, listProviders } from "../http/client";
import { credentialPoolKeys, providerKeys } from "./keys";

export { useSystemStatus as useProviderStatus } from "./runtime";

export const providersQueryOptions = () =>
  queryOptions({
    queryKey: providerKeys.lists(),
    queryFn: listProviders,
    staleTime: 60_000,
  });

export function useProviders() {
  return useQuery(providersQueryOptions());
}

// ── Credential pools (#4965) ────────────────────────────────────────────────

/// `GET /api/credential-pools` — per-provider redacted snapshot of the
/// multi-key rotation pool (key hints, priority, request counts, cooldown).
/// `staleTime` matches `useProviders` so dashboard refreshes don't hammer
/// the kernel, but `refetchInterval` is short so cooldown countdowns
/// visibly tick when a key is exhausted.
export const credentialPoolsQueryOptions = () =>
  queryOptions({
    queryKey: credentialPoolKeys.lists(),
    queryFn: listCredentialPools,
    staleTime: 15_000,
    refetchInterval: 30_000,
  });

export function useCredentialPools() {
  return useQuery(credentialPoolsQueryOptions());
}
