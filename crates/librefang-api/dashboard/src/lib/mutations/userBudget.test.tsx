import { renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import * as http from "../http/client";
import { authzKeys, userBudgetKeys, userKeys } from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";
import { useDeleteUserBudget, useUpdateUserBudget } from "./userBudget";

vi.mock("../http/client", () => ({
  updateUserBudget: vi.fn(),
  deleteUserBudget: vi.fn(),
}));

function expectBudgetDependentsInvalidated(
  spy: ReturnType<typeof vi.spyOn>,
  name: string,
) {
  expect(spy).toHaveBeenCalledWith({ queryKey: userBudgetKeys.detail(name) });
  expect(spy).toHaveBeenCalledWith({ queryKey: userKeys.detail(name) });
  expect(spy).toHaveBeenCalledWith({ queryKey: userKeys.lists() });
  expect(spy).toHaveBeenCalledWith({ queryKey: authzKeys.effective(name) });
}

describe("user budget mutations", () => {
  beforeEach(() => {
    vi.mocked(http.updateUserBudget).mockReset().mockResolvedValue({
      max_hourly_usd: 5,
      max_daily_usd: 20,
      max_monthly_usd: 50,
      alert_threshold: 0.8,
    });
    vi.mocked(http.deleteUserBudget).mockReset().mockResolvedValue({ status: "ok" });
  });

  it("reconciles every dependent cache after update", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useUpdateUserBudget(), { wrapper });

    await result.current.mutateAsync({
      name: "alice",
      payload: {
        max_hourly_usd: 5,
        max_daily_usd: 20,
        max_monthly_usd: 50,
        alert_threshold: 0.8,
      },
    });

    expectBudgetDependentsInvalidated(invalidateSpy, "alice");
  });

  it("reconciles every dependent cache after delete", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useDeleteUserBudget(), { wrapper });

    await result.current.mutateAsync("alice");

    expectBudgetDependentsInvalidated(invalidateSpy, "alice");
  });
});
