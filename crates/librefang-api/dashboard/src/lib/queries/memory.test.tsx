import { describe, it, expect } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { useQuery } from "@tanstack/react-query";
import { useMemoryHealth } from "./memory";
import { healthDetailQueryOptions } from "./runtime";
import { runtimeKeys } from "./keys";
import { createQueryClientWrapper } from "../test/query-client";

describe("useMemoryHealth", () => {
  it("should return true when data.memory.embedding_available is true", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    queryClient.setQueryData(runtimeKeys.healthDetail(), {
      memory: { embedding_available: true },
    });

    const { result } = renderHook(() => useMemoryHealth(), {
      wrapper,
    });

    await waitFor(() => expect(result.current.data).toBe(true));
  });

  it("should return false when data.memory.embedding_available is false", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    queryClient.setQueryData(runtimeKeys.healthDetail(), {
      memory: { embedding_available: false },
    });

    const { result } = renderHook(() => useMemoryHealth(), {
      wrapper,
    });

    await waitFor(() => expect(result.current.data).toBe(false));
  });

  it("should return false when data.memory is undefined (default fallback)", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    queryClient.setQueryData(runtimeKeys.healthDetail(), {
      status: "ok",
    });

    const { result } = renderHook(() => useMemoryHealth(), {
      wrapper,
    });

    await waitFor(() => expect(result.current.data).toBe(false));
  });

  it("should respect enabled option (not fetch when enabled: false)", async () => {
    const { wrapper } = createQueryClientWrapper();

    const { result } = renderHook(
      () => useMemoryHealth({ enabled: false }),
      { wrapper },
    );

    expect(result.current.data).toBeUndefined();
    expect(result.current.status).toBe("pending");
  });

  it("should share the same queryKey as healthDetailQueryOptions (cache sharing)", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const sharedQueryState = {
      memory: { embedding_available: true },
    };

    queryClient.setQueryData(runtimeKeys.healthDetail(), sharedQueryState);

    const { result: healthResult } = renderHook(
      () => useQuery(healthDetailQueryOptions()),
      { wrapper },
    );

    const { result: memoryResult } = renderHook(
      () => useMemoryHealth(),
      { wrapper },
    );

    await waitFor(() => expect(healthResult.current.data).toBeDefined());
    await waitFor(() => expect(memoryResult.current.data).toBe(true));

    expect(healthResult.current.data).toBe(sharedQueryState);
    expect(queryClient.getQueryData(runtimeKeys.healthDetail())).toBe(sharedQueryState);
  });
});
