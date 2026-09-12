import { renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi, beforeEach } from "vitest";
import * as http from "../http/client";
import { deleteGoal } from "../http/client";
import { goalKeys } from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";
import { useDeleteGoal, useStartGoalRun } from "./goals";

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

describe("useStartGoalRun", () => {
  beforeEach(() => {
    vi.mocked(http.startGoalRun).mockResolvedValue({ ok: true, run: null });
  });

  it("sends no payload when neither bound is set", async () => {
    const { wrapper } = createQueryClientWrapper();
    const { result } = renderHook(() => useStartGoalRun(), { wrapper });

    result.current.mutate({ id: "g-1" });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(http.startGoalRun).toHaveBeenCalledWith("g-1", undefined);
  });

  it("forwards the rework budget on its own", async () => {
    const { wrapper } = createQueryClientWrapper();
    const { result } = renderHook(() => useStartGoalRun(), { wrapper });

    result.current.mutate({ id: "g-1", verifyMaxRetries: 2 });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(http.startGoalRun).toHaveBeenCalledWith("g-1", {
      max_iterations: undefined,
      verify_max_retries: 2,
    });
  });

  it("forwards both bounds together", async () => {
    const { wrapper } = createQueryClientWrapper();
    const { result } = renderHook(() => useStartGoalRun(), { wrapper });

    result.current.mutate({ id: "g-1", maxIterations: 10, verifyMaxRetries: 3 });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(http.startGoalRun).toHaveBeenCalledWith("g-1", {
      max_iterations: 10,
      verify_max_retries: 3,
    });
  });

  it("invalidates the goal's run and the goal list on success", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useStartGoalRun(), { wrapper });

    result.current.mutate({ id: "g-1" });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: goalKeys.run("g-1") });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: goalKeys.lists() });
  });
});
