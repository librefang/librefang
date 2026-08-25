import { renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { deleteGoal } from "../http/client";
import { goalKeys } from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";
import { useDeleteGoal } from "./goals";

vi.mock("../http/client", () => ({
  createGoal: vi.fn(),
  updateGoal: vi.fn(),
  deleteGoal: vi.fn().mockResolvedValue(undefined),
  startGoalRun: vi.fn(),
  stopGoalRun: vi.fn(),
}));

describe("useDeleteGoal", () => {
  it("removes the list row immediately and clears its run cache", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const setSpy = vi.spyOn(queryClient, "setQueryData");
    const removeSpy = vi.spyOn(queryClient, "removeQueries");
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useDeleteGoal(), { wrapper });

    await result.current.mutateAsync("goal-1");

    expect(deleteGoal).toHaveBeenCalledWith("goal-1", expect.any(Object));
    expect(setSpy).toHaveBeenCalledWith(goalKeys.lists(), expect.any(Function));
    const updater = setSpy.mock.calls[0]?.[1] as (
      previous: Array<{ id: string }>,
    ) => unknown;
    expect(updater([{ id: "goal-1" }, { id: "goal-2" }])).toEqual([{ id: "goal-2" }]);
    expect(removeSpy).toHaveBeenCalledWith({ queryKey: goalKeys.run("goal-1") });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: goalKeys.lists() });
  });
});
