import { renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { modelKeys } from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";
import { useDeleteModelOverrides, useRemoveCustomModel } from "./models";

vi.mock("../http/client", () => ({
  addCustomModel: vi.fn(),
  removeCustomModel: vi.fn().mockResolvedValue(undefined),
  updateModelOverrides: vi.fn(),
  deleteModelOverrides: vi.fn().mockResolvedValue(undefined),
}));

describe("model delete mutations", () => {
  it.each([
    ["custom model", useRemoveCustomModel],
    ["model overrides", useDeleteModelOverrides],
  ])("clears override details when deleting a %s", async (_label, useDelete) => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const removeSpy = vi.spyOn(queryClient, "removeQueries");
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useDelete(), { wrapper });

    await result.current.mutateAsync("provider/model");

    expect(removeSpy).toHaveBeenCalledWith({
      queryKey: modelKeys.overrides("provider/model"),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: modelKeys.lists() });
    expect(invalidateSpy).not.toHaveBeenCalledWith({
      queryKey: modelKeys.overrides("provider/model"),
    });
  });
});
