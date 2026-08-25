import { renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { updateMemoryConfig } from "../http/client";
import { memoryKeys } from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";
import { useUpdateMemoryConfig } from "./memory";

vi.mock("../http/client", () => ({
  addMemoryFromText: vi.fn(),
  updateMemory: vi.fn(),
  deleteMemory: vi.fn(),
  cleanupMemories: vi.fn(),
  updateMemoryConfig: vi.fn().mockResolvedValue({ decay_rate: 0.2 }),
}));

describe("useUpdateMemoryConfig", () => {
  it("keeps the canonical PATCH response without refetching it", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const setSpy = vi.spyOn(queryClient, "setQueryData");
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useUpdateMemoryConfig(), { wrapper });

    await result.current.mutateAsync({ decay_rate: 0.2 });

    expect(updateMemoryConfig).toHaveBeenCalled();
    expect(setSpy).toHaveBeenCalledWith(memoryKeys.config(), { decay_rate: 0.2 });
    expect(invalidateSpy).not.toHaveBeenCalledWith({ queryKey: memoryKeys.config() });
  });
});
