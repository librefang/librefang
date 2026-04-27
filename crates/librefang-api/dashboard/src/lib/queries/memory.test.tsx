import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { useQuery } from "@tanstack/react-query";
import { useMemoryHealth, useAgentKvMemory } from "./memory";
import { healthDetailQueryOptions } from "./runtime";
import { runtimeKeys, memoryKeys } from "./keys";
import { createQueryClientWrapper } from "../test/query-client";
import * as httpClient from "../http/client";

vi.mock("../http/client", async () => {
  const actual = await vi.importActual<typeof import("../http/client")>("../http/client");
  return {
    ...actual,
    getAgentKvMemory: vi.fn(),
  };
});

beforeEach(() => {
  vi.clearAllMocks();
});

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

describe("useAgentKvMemory", () => {
  it("should be disabled when agentId is empty string", () => {
    const { result } = renderHook(() => useAgentKvMemory(""), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.fetchStatus).toBe("idle");
    expect(httpClient.getAgentKvMemory).not.toHaveBeenCalled();
  });

  it("should fetch and unwrap kv_pairs when agentId is provided", async () => {
    vi.mocked(httpClient.getAgentKvMemory).mockResolvedValue({
      kv_pairs: [
        { key: "name", value: "alice" },
        { key: "tz", value: "UTC" },
      ],
    });

    const { result } = renderHook(() => useAgentKvMemory("agent-1"), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(httpClient.getAgentKvMemory).toHaveBeenCalledWith("agent-1");
    expect(result.current.data).toEqual([
      { key: "name", value: "alice" },
      { key: "tz", value: "UTC" },
    ]);
  });

  it("should normalize missing kv_pairs to an empty array", async () => {
    vi.mocked(httpClient.getAgentKvMemory).mockResolvedValue({});

    const { result } = renderHook(() => useAgentKvMemory("agent-1"), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data).toEqual([]);
  });

  it("should cache under memoryKeys.agentKv(agentId)", async () => {
    vi.mocked(httpClient.getAgentKvMemory).mockResolvedValue({
      kv_pairs: [{ key: "k", value: "v" }],
    });
    const { queryClient, wrapper } = createQueryClientWrapper();

    renderHook(() => useAgentKvMemory("agent-xyz"), { wrapper });

    await waitFor(() => {
      expect(queryClient.getQueryData(memoryKeys.agentKv("agent-xyz"))).toEqual([
        { key: "k", value: "v" },
      ]);
    });
  });
});
