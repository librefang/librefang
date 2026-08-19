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

// `GET /api/credential-pools` — per-provider redacted snapshot of the
// multi-key rotation pool (key hints, priority, request counts, cooldown).
// Its 15-second freshness window is deliberately shorter than the provider
// list's 60 seconds. The modest 30-second foreground poll keeps cooldown
// state moving without hammering the kernel.
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
