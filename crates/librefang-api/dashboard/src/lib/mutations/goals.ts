import { useMutation, useQueryClient } from "@tanstack/react-query";
import { createGoal, updateGoal, deleteGoal } from "../http/client";
import type { GoalItem } from "../../api";
import { goalKeys } from "../queries/keys";

export function useCreateGoal() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: createGoal,
    onSuccess: () => qc.invalidateQueries({ queryKey: goalKeys.lists() }),
  });
}

export function useUpdateGoal() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, data }: { id: string; data: Parameters<typeof updateGoal>[1] }) =>
      updateGoal(id, data),
    // Issue #3832: handler now returns the mutated GoalItem, so we can patch the
    // cached list immediately for an instant UI update, then invalidate as a
    // belt-and-suspenders guard against drift.
    onSuccess: (updated: GoalItem) => {
      qc.setQueryData<GoalItem[]>(goalKeys.lists(), (prev) =>
        prev ? prev.map((g) => (g.id === updated.id ? updated : g)) : prev,
      );
      qc.invalidateQueries({ queryKey: goalKeys.lists() });
    },
  });
}

export function useDeleteGoal() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: deleteGoal,
    onSuccess: () => qc.invalidateQueries({ queryKey: goalKeys.lists() }),
  });
}
