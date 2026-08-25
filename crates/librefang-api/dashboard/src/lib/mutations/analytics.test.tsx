import { renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import * as http from "../http/client";
import { budgetKeys } from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";
import { useUpdateBudget, useUpdateProviderBudget } from "./analytics";

vi.mock("../http/client", () => ({
  updateBudget: vi.fn().mockResolvedValue({}),
  updateProviderBudget: vi.fn().mockResolvedValue({}),
}));

describe("budget mutations", () => {
  it("updates the global budget and invalidates the shared budget tree", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useUpdateBudget(), { wrapper });

    await result.current.mutateAsync({ max_hourly_usd: 10 });

    expect(http.updateBudget).toHaveBeenCalledWith(
      { max_hourly_usd: 10 },
      expect.any(Object),
    );
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: budgetKeys.all });
  });

  it("updates a provider budget and uses the same invalidation policy", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useUpdateProviderBudget(), { wrapper });
    const payload = { max_cost_per_day_usd: 25 };

    await result.current.mutateAsync({ providerId: "openai", payload });

    expect(http.updateProviderBudget).toHaveBeenCalledWith("openai", payload);
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: budgetKeys.all });
  });
});
