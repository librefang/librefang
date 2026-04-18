import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";

// ── Mock API layer ──
const { mockListApprovals, mockFetchApprovalCount } = vi.hoisted(() => ({
  mockListApprovals: vi.fn(),
  mockFetchApprovalCount: vi.fn(),
}));
const { mockListPluginRegistries } = vi.hoisted(() => ({
  mockListPluginRegistries: vi.fn(),
}));

vi.mock("../../api", async () => {
  const actual = await vi.importActual("../../api");
  return { ...actual, listApprovals: mockListApprovals, fetchApprovalCount: mockFetchApprovalCount };
});

vi.mock("../http/client", async () => {
  const actual = await vi.importActual("../http/client");
  return {
    ...actual,
    listPluginRegistries: mockListPluginRegistries,
  };
});

// ── Import hooks after mocks are set up ──
import { useApprovals, useApprovalCount } from "./approvals";
import { usePluginRegistries } from "./plugins";
import { approvalKeys, pluginKeys } from "./keys";
import { createQueryClientWrapper } from "../test/query-client";

beforeEach(() => {
  vi.clearAllMocks();
});

// ── useApprovals ──

describe("useApprovals", () => {
  it("should not fetch when enabled is false", async () => {
    const { result } = renderHook(() => useApprovals({ enabled: false }), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(mockListApprovals).not.toHaveBeenCalled();
  });

  it("should fetch by default when enabled is undefined", async () => {
    renderHook(() => useApprovals(), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    // enabled defaults to undefined → query is enabled by default
    // but since we don't mock data, it will attempt to fetch
    // Actually, when enabled is undefined, useQuery treats it as true
    await vi.waitFor(() => {
      expect(mockListApprovals).toHaveBeenCalled();
    });
  });

  it("should fetch when enabled is true", async () => {
    const mockData = [{ id: "1", tool_name: "test" }];
    mockListApprovals.mockResolvedValue(mockData);

    const { result } = renderHook(() => useApprovals({ enabled: true }), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    await waitFor(() => expect(result.current.data).toEqual(mockData));
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(mockListApprovals).toHaveBeenCalledTimes(1);
  });

  it("should use approvalKeys.lists() as queryKey", async () => {
    const mockData: Array<{ id: string; tool_name: string }> = [];
    mockListApprovals.mockResolvedValue(mockData);

    const { queryClient, wrapper } = createQueryClientWrapper();

    renderHook(() => useApprovals({ enabled: true }), { wrapper });

    await waitFor(() => {
      expect(queryClient.getQueryData(approvalKeys.lists())).toEqual(mockData);
    });
  });
});

// ── usePluginRegistries ──

describe("usePluginRegistries", () => {
  it("should not fetch when enabled is false", async () => {
    const { result } = renderHook(() => usePluginRegistries(false), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(mockListPluginRegistries).not.toHaveBeenCalled();
  });

  it("should fetch by default when enabled is undefined", async () => {
    renderHook(() => usePluginRegistries(undefined), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    // enabled is undefined → useQuery treats it as true → WILL fetch
    await vi.waitFor(() => {
      expect(mockListPluginRegistries).toHaveBeenCalled();
    });
  });

  it("should fetch when enabled is true", async () => {
    const mockData = { registries: [{ id: "npm", url: "https://registry.npmjs.org" }] };
    mockListPluginRegistries.mockResolvedValue(mockData);

    const { result } = renderHook(() => usePluginRegistries(true), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    await waitFor(() => expect(result.current.data).toEqual(mockData));
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(mockListPluginRegistries).toHaveBeenCalledTimes(1);
  });

  it("should use pluginKeys.registries() as queryKey", async () => {
    const mockData = { registries: [] };
    mockListPluginRegistries.mockResolvedValue(mockData);

    const { queryClient, wrapper } = createQueryClientWrapper();

    renderHook(() => usePluginRegistries(true), { wrapper });

    await waitFor(() => {
      expect(queryClient.getQueryData(pluginKeys.registries())).toEqual(mockData);
    });
  });
});

// ── useApprovalCount ──

describe("useApprovalCount", () => {
  it("should fetch by default (always enabled)", async () => {
    mockFetchApprovalCount.mockResolvedValue({ count: 5 });

    const { result } = renderHook(() => useApprovalCount(), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    await waitFor(() => expect(result.current.data).toEqual({ count: 5 }));
    expect(mockFetchApprovalCount).toHaveBeenCalledTimes(1);
  });

  it("should use default refetchInterval when not provided", async () => {
    mockFetchApprovalCount.mockResolvedValue({ count: 0 });

    const { wrapper, queryClient } = createQueryClientWrapper();
    renderHook(() => useApprovalCount(), { wrapper });

    await vi.waitFor(() => {
      const query = queryClient.getQueryCache().find({ queryKey: approvalKeys.count() });
      expect(query).toBeDefined();
    });

    const query = queryClient.getQueryCache().find({ queryKey: approvalKeys.count() });
    expect(query).toBeDefined();
    expect((query?.options as { refetchInterval?: number }).refetchInterval).toBe(15_000);
  });

  it("should override refetchInterval when provided", async () => {
    mockFetchApprovalCount.mockResolvedValue({ count: 0 });

    const { wrapper, queryClient } = createQueryClientWrapper();
    renderHook(() => useApprovalCount({ refetchInterval: 5_000 }), { wrapper });

    await vi.waitFor(() => {
      const query = queryClient.getQueryCache().find({ queryKey: approvalKeys.count() });
      expect(query).toBeDefined();
    });

    const query = queryClient.getQueryCache().find({ queryKey: approvalKeys.count() });
    expect(query).toBeDefined();
    expect((query?.options as { refetchInterval?: number }).refetchInterval).toBe(5_000);
  });

  it("should use approvalKeys.count() as queryKey", async () => {
    const mockData = { count: 0 };
    mockFetchApprovalCount.mockResolvedValue(mockData);

    const { wrapper, queryClient } = createQueryClientWrapper();
    renderHook(() => useApprovalCount(), { wrapper });

    await vi.waitFor(() => {
      expect(queryClient.getQueryData(approvalKeys.count())).toEqual(mockData);
    });
  });
});
