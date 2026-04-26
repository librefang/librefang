// Per-user budget queries (RBAC M5).

import { queryOptions, useQuery } from "@tanstack/react-query";
import { getUserBudget } from "../http/client";
import { userBudgetKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

const STALE_MS = 30_000;

export const userBudgetQueries = {
  detail: (name: string) =>
    queryOptions({
      queryKey: userBudgetKeys.detail(name),
      queryFn: () => getUserBudget(name),
      enabled: !!name,
      staleTime: STALE_MS,
    }),
};

export function useUserBudget(name: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(userBudgetQueries.detail(name), options));
}
