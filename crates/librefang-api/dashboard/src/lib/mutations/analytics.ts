import { useMutation, useQueryClient } from "@tanstack/react-query";
import { updateBudget, updateProviderBudget } from "../http/client";
import type { ProviderBudgetPayload } from "../http/client";
import { budgetKeys } from "../queries/keys";

function useBudgetMutation<TVariables, TResult>(
  mutationFn: (variables: TVariables) => Promise<TResult>,
) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn,
    onSuccess: () => qc.invalidateQueries({ queryKey: budgetKeys.all }),
  });
}

export function useUpdateBudget() {
  return useBudgetMutation(updateBudget);
}

// PUT /api/budget/providers/{provider_id} (#5650). Invalidate the whole
// `budgetKeys.all` tree so the global budget status (which echoes the
// `alert_threshold` rendered against each provider row) and the per-
// provider snapshot both refetch — same pattern as `useUpdateBudget`.
export function useUpdateProviderBudget() {
  return useBudgetMutation(
    (vars: { providerId: string; payload: ProviderBudgetPayload }) =>
      updateProviderBudget(vars.providerId, vars.payload),
  );
}
