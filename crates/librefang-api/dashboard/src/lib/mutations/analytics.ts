import { useMutation, useQueryClient } from "@tanstack/react-query";
import { updateBudget } from "../http/client";
import { budgetKeys } from "../queries/keys";

export function useUpdateBudget() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: updateBudget,
    onSuccess: () => qc.invalidateQueries({ queryKey: budgetKeys.all }),
  });
}
