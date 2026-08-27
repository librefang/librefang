import { renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import * as http from "../http/client";
import { budgetKeys, usageKeys, userBudgetKeys } from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";
import { useGenerateImage, useGenerateMusic } from "./media";

vi.mock("../http/client", () => ({
  generateImage: vi.fn().mockResolvedValue({ images: [] }),
  synthesizeSpeech: vi.fn().mockResolvedValue({}),
  submitVideo: vi.fn().mockResolvedValue({}),
  generateMusic: vi.fn().mockRejectedValue(new Error("provider failed")),
}));

describe("media mutations", () => {
  it("invalidates every spend projection and forwards onSettled", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const onSettled = vi.fn();
    const { result } = renderHook(() => useGenerateImage({ onSettled }), { wrapper });

    await result.current.mutateAsync({ prompt: "a fox" });

    for (const queryKey of [budgetKeys.all, userBudgetKeys.all, usageKeys.all]) {
      expect(invalidateSpy).toHaveBeenCalledWith({ queryKey });
    }
    expect(onSettled).toHaveBeenCalledOnce();
    expect(http.generateImage).toHaveBeenCalled();
  });

  it("uses the same invalidation policy after provider failure", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useGenerateMusic(), { wrapper });

    await expect(result.current.mutateAsync({ prompt: "song" })).rejects.toThrow(
      "provider failed",
    );

    for (const queryKey of [budgetKeys.all, userBudgetKeys.all, usageKeys.all]) {
      expect(invalidateSpy).toHaveBeenCalledWith({ queryKey });
    }
  });
});
