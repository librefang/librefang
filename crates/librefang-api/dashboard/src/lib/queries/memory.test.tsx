import { describe, it, expect } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider, useQuery } from "@tanstack/react-query";
import { ReactNode } from "react";
import { useMemoryHealth } from "./memory";
import { healthDetailQueryOptions } from "./runtime";
import { runtimeKeys } from "./keys";

function createWrapper(queryClient: QueryClient) {
  return function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>;
  };
}

describe("useMemoryHealth", () => {
  it("should return true when data.memory.embedding_available is true", async () => {
    const qc = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    qc.setQueryData(runtimeKeys.healthDetail(), {
      memory: { embedding_available: true },
    });

    const { result } = renderHook(() => useMemoryHealth(), {
      wrapper: createWrapper(qc),
    });

    await waitFor(() => expect(result.current.data).toBe(true));
  });

  it("should return false when data.memory.embedding_available is false", async () => {
    const qc = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    qc.setQueryData(runtimeKeys.healthDetail(), {
      memory: { embedding_available: false },
    });

    const { result } = renderHook(() => useMemoryHealth(), {
      wrapper: createWrapper(qc),
    });

    await waitFor(() => expect(result.current.data).toBe(false));
  });

  it("should return false when data.memory is undefined (default fallback)", async () => {
    const qc = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    qc.setQueryData(runtimeKeys.healthDetail(), {
      status: "ok",
    });

    const { result } = renderHook(() => useMemoryHealth(), {
      wrapper: createWrapper(qc),
    });

    await waitFor(() => expect(result.current.data).toBe(false));
  });

  it("should respect enabled option (not fetch when enabled: false)", async () => {
    const qc = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });

    const { result } = renderHook(
      () => useMemoryHealth({ enabled: false }),
      { wrapper: createWrapper(qc) },
    );

    expect(result.current.data).toBeUndefined();
    expect(result.current.status).toBe("pending");
  });

  it("should share the same queryKey as healthDetailQueryOptions (cache sharing)", async () => {
    const qc = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });

    qc.setQueryData(runtimeKeys.healthDetail(), {
      memory: { embedding_available: true },
    });

    const { result: healthResult } = renderHook(
      () => useQuery(healthDetailQueryOptions()),
      { wrapper: createWrapper(qc) },
    );

    const { result: memoryResult } = renderHook(
      () => useMemoryHealth(),
      { wrapper: createWrapper(qc) },
    );

    await waitFor(() => expect(healthResult.current.data).toBeDefined());
    await waitFor(() => expect(memoryResult.current.data).toBe(true));

    expect(healthDetailQueryOptions().queryKey).toEqual(runtimeKeys.healthDetail());
    expect(qc.getQueryData(runtimeKeys.healthDetail())).toEqual({
      memory: { embedding_available: true },
    });
  });
});
