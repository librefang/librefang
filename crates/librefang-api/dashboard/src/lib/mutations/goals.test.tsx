import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import * as http from "../http/client";
import { useStartGoalRun } from "./goals";
import { goalKeys } from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";

// Mutations import from the typed http/client whitelist (see dashboard/CLAUDE.md),
// so the mock target is "../http/client", not "../../api".
vi.mock("../http/client", () => ({
  startGoalRun: vi.fn(),
  stopGoalRun: vi.fn(),
  createGoal: vi.fn(),
  updateGoal: vi.fn(),
  deleteGoal: vi.fn(),
}));

describe("useStartGoalRun", () => {
  beforeEach(() => {
    vi.mocked(http.startGoalRun).mockResolvedValue({ ok: true, run: null });
  });

  // A run with nothing to configure posts no body at all, exactly as it did
  // before `verify_max_retries` existed.
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
