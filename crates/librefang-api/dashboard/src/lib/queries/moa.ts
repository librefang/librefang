import { queryOptions, useQuery } from "@tanstack/react-query";
import { getMoaPresets } from "../http/client";
import { moaKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

export const moaQueries = {
  presets: () =>
    queryOptions({
      queryKey: moaKeys.presets(),
      queryFn: getMoaPresets,
      staleTime: 30_000,
    }),
};

export function useMoaPresets(options: QueryOverrides = {}) {
  return useQuery(withOverrides(moaQueries.presets(), options));
}
