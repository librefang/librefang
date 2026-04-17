import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";

// ── Mock API layer ──
const { mockListApprovals, mockFetchApprovalCount } = vi.hoisted(() => ({
  mockListApprovals: vi.fn(),
  mockFetchApprovalCount: vi.fn(),
}));
const { mockListAvailableIntegrations, mockListPluginRegistries } = vi.hoisted(() => ({
  mockListAvailableIntegrations: vi.fn(),
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
    listAvailableIntegrations: mockListAvailableIntegrations,
    listPluginRegistries: mockListPluginRegistries,
  };
});

// ── Import hooks after mocks are set up ──
import { useApprovals, useApprovalCount } from "./approvals";
import { useAvailableIntegrations } from "./mcp";
import { usePluginRegistries } from "./plugins";
import { approvalKeys, mcpKeys, pluginKeys } from "./keys";

function createWrapper() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

// ── useApprovals ──

describe("useApprovals", () => {
  it("should not fetch when enabled is false", async () => {
    const { result } = renderHook(() => useApprovals({ enabled: false }), {
      wrapper: createWrapper(),
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(mockListApprovals).not.toHaveBeenCalled();
  });

  it("should fetch by default when enabled is undefined", async () => {
    const { result } = renderHook(() => useApprovals(), {
      wrapper: createWrapper(),
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
      wrapper: createWrapper(),
    });

    await waitFor(() => expect(result.current.data).toEqual(mockData));
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(mockListApprovals).toHaveBeenCalledTimes(1);
  });

  it("should use approvalKeys.lists() as queryKey", async () => {
    mockListApprovals.mockResolvedValue([]);

    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={qc}>{children}</QueryClientProvider>
    );

    renderHook(() => useApprovals({ enabled: true }), { wrapper });

    await waitFor(() => {
      expect(qc.getQueryCache().find(approvalKeys.lists())).toBeDefined();
    });
    expect(
      qc.getQueryCache().find(approvalKeys.lists())?.queryKey,
    ).toEqual(approvalKeys.lists());
  });
});

// ── useAvailableIntegrations ──

describe("useAvailableIntegrations", () => {
  it("should not fetch when enabled is false", async () => {
    const { result } = renderHook(
      () => useAvailableIntegrations({ enabled: false }),
      { wrapper: createWrapper() },
    );

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(mockListAvailableIntegrations).not.toHaveBeenCalled();
  });

  it("should fetch by default when enabled is undefined", async () => {
    const { result } = renderHook(() => useAvailableIntegrations(), {
      wrapper: createWrapper(),
    });

    // mcpQueries.integrations() sets enabled: opts.enabled which is undefined
    // useQuery treats undefined enabled as true, so it WILL fetch
    await vi.waitFor(() => {
      expect(mockListAvailableIntegrations).toHaveBeenCalled();
    });
  });

  it("should fetch when enabled is true", async () => {
    const mockData = { integrations: [{ id: "slack", name: "Slack" }] };
    mockListAvailableIntegrations.mockResolvedValue(mockData);

    const { result } = renderHook(
      () => useAvailableIntegrations({ enabled: true }),
      { wrapper: createWrapper() },
    );

    await waitFor(() => expect(result.current.data).toEqual(mockData));
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(mockListAvailableIntegrations).toHaveBeenCalledTimes(1);
  });

  it("should use mcpKeys.integrations() as queryKey", async () => {
    mockListAvailableIntegrations.mockResolvedValue({ integrations: [] });

    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={qc}>{children}</QueryClientProvider>
    );

    renderHook(() => useAvailableIntegrations({ enabled: true }), { wrapper });

    await waitFor(() => {
      expect(qc.getQueryCache().find(mcpKeys.integrations())).toBeDefined();
    });
    expect(
      qc.getQueryCache().find(mcpKeys.integrations())?.queryKey,
    ).toEqual(mcpKeys.integrations());
  });
});

// ── usePluginRegistries ──

describe("usePluginRegistries", () => {
  it("should not fetch when enabled is false", async () => {
    const { result } = renderHook(() => usePluginRegistries(false), {
      wrapper: createWrapper(),
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(mockListPluginRegistries).not.toHaveBeenCalled();
  });

  it("should fetch by default when enabled is undefined", async () => {
    const { result } = renderHook(() => usePluginRegistries(undefined), {
      wrapper: createWrapper(),
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
      wrapper: createWrapper(),
    });

    await waitFor(() => expect(result.current.data).toEqual(mockData));
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(mockListPluginRegistries).toHaveBeenCalledTimes(1);
  });

  it("should use pluginKeys.registries() as queryKey", async () => {
    mockListPluginRegistries.mockResolvedValue({ registries: [] });

    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={qc}>{children}</QueryClientProvider>
    );

    renderHook(() => usePluginRegistries(true), { wrapper });

    await waitFor(() => {
      expect(qc.getQueryCache().find(pluginKeys.registries())).toBeDefined();
    });
    expect(
      qc.getQueryCache().find(pluginKeys.registries())?.queryKey,
    ).toEqual(pluginKeys.registries());
  });
});

// ── useApprovalCount ──

function createWrapperWithClient() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const wrapper = function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
  };
  return { wrapper, qc };
}

describe("useApprovalCount", () => {
  it("should fetch by default (always enabled)", async () => {
    mockFetchApprovalCount.mockResolvedValue({ count: 5 });

    const { result } = renderHook(() => useApprovalCount(), {
      wrapper: createWrapper(),
    });

    await waitFor(() => expect(result.current.data).toEqual({ count: 5 }));
    expect(mockFetchApprovalCount).toHaveBeenCalledTimes(1);
  });

  it("should use default refetchInterval when not provided", async () => {
    mockFetchApprovalCount.mockResolvedValue({ count: 0 });

    const { wrapper, qc } = createWrapperWithClient();
    renderHook(() => useApprovalCount(), { wrapper });

    await vi.waitFor(() => {
      const query = qc.getQueryCache().find({ queryKey: approvalKeys.count() });
      expect(query).toBeDefined();
    });

    const query = qc.getQueryCache().find({ queryKey: approvalKeys.count() })!;
    expect(query.options.refetchInterval).toBe(15_000);
  });

  it("should override refetchInterval when provided", async () => {
    mockFetchApprovalCount.mockResolvedValue({ count: 0 });

    const { wrapper, qc } = createWrapperWithClient();
    renderHook(() => useApprovalCount({ refetchInterval: 5_000 }), { wrapper });

    await vi.waitFor(() => {
      const query = qc.getQueryCache().find({ queryKey: approvalKeys.count() });
      expect(query).toBeDefined();
    });

    const query = qc.getQueryCache().find({ queryKey: approvalKeys.count() })!;
    expect(query.options.refetchInterval).toBe(5_000);
  });

  it("should use approvalKeys.count() as queryKey", async () => {
    mockFetchApprovalCount.mockResolvedValue({ count: 0 });

    const { wrapper, qc } = createWrapperWithClient();
    renderHook(() => useApprovalCount(), { wrapper });

    await vi.waitFor(() => {
      const query = qc.getQueryCache().find({ queryKey: approvalKeys.count() });
      expect(query).toBeDefined();
    });

    const query = qc.getQueryCache().find({ queryKey: approvalKeys.count() })!;
    expect(query.queryKey).toEqual(approvalKeys.count());
  });
});
