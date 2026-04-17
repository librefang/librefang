import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";
import * as http from "../http/client";
import { usePromptVersions, useExperiments, useExperimentMetrics } from "./agents";
import { agentKeys } from "./keys";

vi.mock("../http/client", () => ({
  listPromptVersions: vi.fn(),
  listExperiments: vi.fn(),
  getExperimentMetrics: vi.fn(),
  ApiError: class ApiError extends Error {},
}));

function createWrapper() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
  };
}

describe("usePromptVersions", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("should be disabled when agentId is empty string", () => {
    const { result } = renderHook(() => usePromptVersions(""), {
      wrapper: createWrapper(),
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(http.listPromptVersions).not.toHaveBeenCalled();
  });

  it("should be enabled and fetch when agentId is valid", async () => {
    const mockData = [
      { id: "v1", agent_id: "agent-1", version: 1, is_active: true, created_at: "2024-01-01T00:00:00Z" },
    ];
    vi.mocked(http.listPromptVersions).mockResolvedValue(mockData);

    const { result } = renderHook(() => usePromptVersions("agent-1"), {
      wrapper: createWrapper(),
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.fetchStatus).toBe("fetching");

    await waitFor(() => {
      expect(result.current.isSuccess).toBe(true);
    });

    expect(result.current.data).toEqual(mockData);
    expect(http.listPromptVersions).toHaveBeenCalledWith("agent-1");
  });

  it("should use the correct queryKey", async () => {
    const qc = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={qc}>{children}</QueryClientProvider>
    );

    renderHook(() => usePromptVersions("test-agent"), { wrapper });

    await waitFor(() => {
      expect(qc.getQueryCache().find(agentKeys.promptVersions("test-agent"))).toBeDefined();
    });

    const cache = qc.getQueryCache().find(agentKeys.promptVersions("test-agent"));
    expect(cache).toBeDefined();
    expect(cache?.queryKey).toEqual(agentKeys.promptVersions("test-agent"));
  });
});

describe("useExperiments", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("should be disabled when agentId is empty string", () => {
    const { result } = renderHook(() => useExperiments(""), {
      wrapper: createWrapper(),
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(http.listExperiments).not.toHaveBeenCalled();
  });

  it("should be enabled and fetch when agentId is valid", async () => {
    const mockData = [
      { id: "exp-1", agent_id: "agent-1", name: "Test Experiment", status: "running", created_at: "2024-01-01T00:00:00Z" },
    ];
    vi.mocked(http.listExperiments).mockResolvedValue(mockData);

    const { result } = renderHook(() => useExperiments("agent-1"), {
      wrapper: createWrapper(),
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.fetchStatus).toBe("fetching");

    await waitFor(() => {
      expect(result.current.isSuccess).toBe(true);
    });

    expect(result.current.data).toEqual(mockData);
    expect(http.listExperiments).toHaveBeenCalledWith("agent-1");
  });

  it("should use the correct queryKey", async () => {
    const qc = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={qc}>{children}</QueryClientProvider>
    );

    renderHook(() => useExperiments("test-agent"), { wrapper });

    await waitFor(() => {
      expect(qc.getQueryCache().find(agentKeys.experiments("test-agent"))).toBeDefined();
    });

    const cache = qc.getQueryCache().find(agentKeys.experiments("test-agent"));
    expect(cache).toBeDefined();
    expect(cache?.queryKey).toEqual(agentKeys.experiments("test-agent"));
  });
});

describe("useExperimentMetrics", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("should be disabled when experimentId is empty string", () => {
    const { result } = renderHook(() => useExperimentMetrics(""), {
      wrapper: createWrapper(),
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(http.getExperimentMetrics).not.toHaveBeenCalled();
  });

  it("should be enabled and fetch when experimentId is valid", async () => {
    const mockData = [
      { variant_id: "v1", success_rate: 0.95, avg_tokens: 100, total_cost_usd: 0.01 },
    ];
    vi.mocked(http.getExperimentMetrics).mockResolvedValue(mockData);

    const { result } = renderHook(() => useExperimentMetrics("exp-1"), {
      wrapper: createWrapper(),
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.fetchStatus).toBe("fetching");

    await waitFor(() => {
      expect(result.current.isSuccess).toBe(true);
    });

    expect(result.current.data).toEqual(mockData);
    expect(http.getExperimentMetrics).toHaveBeenCalledWith("exp-1");
  });

  it("should use the correct queryKey", async () => {
    const mockData = [{ variant_id: "v1", success_rate: 0.5 }];
    vi.mocked(http.getExperimentMetrics).mockResolvedValue(mockData);

    const qc = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={qc}>{children}</QueryClientProvider>
    );

    renderHook(() => useExperimentMetrics("test-exp"), { wrapper });

    await waitFor(() => {
      expect(qc.getQueryCache().find(agentKeys.experimentMetrics("test-exp"))).toBeDefined();
    });

    expect(
      qc.getQueryCache().find(agentKeys.experimentMetrics("test-exp"))?.queryKey,
    ).toEqual(agentKeys.experimentMetrics("test-exp"));
  });
});
