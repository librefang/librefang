import { queryOptions, useQuery } from "@tanstack/react-query";
import { listProviders } from "../http/client";
import { providerKeys } from "./keys";

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
