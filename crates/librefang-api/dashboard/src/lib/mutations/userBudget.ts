// Per-user budget mutations (RBAC M5).
//
// Both writes invalidate the matching `userBudgetKeys.detail(name)` so any
// open detail panel re-fetches against the now-persisted config.toml. We
// also kick `userKeys.detail(name)` because `UserConfig.budget` is part of
// the `UserItem` payload (the M6 dashboard surfaces it on the user row),
// and `authzKeys.effective(name)` because the permission simulator reads
// `EffectivePermissions.budget` from the same `UserConfig` row (#3228
// follow-up).

import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  updateUserBudget,
  deleteUserBudget,
  type UserBudgetPayload,
} from "../http/client";
import { authzKeys, userBudgetKeys, userKeys } from "../queries/keys";

export function useUpdateUserBudget() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (vars: { name: string; payload: UserBudgetPayload }) =>
      updateUserBudget(vars.name, vars.payload),
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: userBudgetKeys.detail(variables.name) });
      qc.invalidateQueries({ queryKey: userKeys.detail(variables.name) });
      qc.invalidateQueries({ queryKey: userKeys.lists() });
      qc.invalidateQueries({
        queryKey: authzKeys.effective(variables.name),
      });
    },
  });
}

export function useDeleteUserBudget() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (name: string) => deleteUserBudget(name),
    onSuccess: (_data, name) => {
      qc.invalidateQueries({ queryKey: userBudgetKeys.detail(name) });
      qc.invalidateQueries({ queryKey: userKeys.detail(name) });
      qc.invalidateQueries({ queryKey: userKeys.lists() });
      qc.invalidateQueries({ queryKey: authzKeys.effective(name) });
    },
  });
}
