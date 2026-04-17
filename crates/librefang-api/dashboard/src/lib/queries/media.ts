import { queryOptions, useQuery } from "@tanstack/react-query";
import { listMediaProviders } from "../../api";
import { mediaKeys } from "./keys";

const STALE_MS = 60_000;
const REFRESH_MS = 60_000;

export const mediaQueries = {
  providers: () =>
    queryOptions({
      queryKey: mediaKeys.providers(),
      queryFn: listMediaProviders,
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS,
    }),
};

export function useMediaProviders() {
  return useQuery(mediaQueries.providers());
}
