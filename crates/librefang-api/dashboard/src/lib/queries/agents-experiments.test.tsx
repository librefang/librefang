import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import * as http from "../http/client";
import { usePromptVersions, useExperiments, useExperimentMetrics } from "./agents";
import { agentKeys } from "./keys";
import { createQueryClientWrapper } from "../test/query-client";

vi.mock("../http/client", () => ({
  listPromptVersions: vi.fn(),
  listExperiments: vi.fn(),
  getExperimentMetrics: vi.fn(),
  ApiError: class ApiError extends Error {},
}));

describe("usePromptVersions", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("should be disabled when agentId is empty string", () => {
    const { result } = renderHook(() => usePromptVersions(""), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(http.listPromptVersions).not.toHaveBeenCalled();
  });

  it("should be enabled and fetch when agentId is valid", async () => {
    const mockData = [
      {
        id: "v1",
        agent_id: "agent-1",
        version: 1,
        content_hash: "hash-1",
        system_prompt: "system",
        tools: [],
        variables: [],
        created_at: "2024-01-01T00:00:00Z",
        created_by: "tester",
        is_active: true,
      },
    ];
    vi.mocked(http.listPromptVersions).mockResolvedValue(mockData);

    const { result } = renderHook(() => usePromptVersions("agent-1"), {
      wrapper: createQueryClientWrapper().wrapper,
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
    const { queryClient, wrapper } = createQueryClientWrapper();

    renderHook(() => usePromptVersions("test-agent"), { wrapper });

    await waitFor(() => {
      expect(queryClient.getQueryCache().find({ queryKey: agentKeys.promptVersions("test-agent") })).toBeDefined();
    });

    const cache = queryClient.getQueryCache().find({ queryKey: agentKeys.promptVersions("test-agent") });
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
      wrapper: createQueryClientWrapper().wrapper,
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(http.listExperiments).not.toHaveBeenCalled();
  });

  it("should be enabled and fetch when agentId is valid", async () => {
    const mockData = [
      {
        id: "exp-1",
        agent_id: "agent-1",
        name: "Test Experiment",
        status: "running" as const,
        traffic_split: [100],
        success_criteria: {
          require_user_helpful: true,
          require_no_tool_errors: true,
          require_non_empty: true,
        },
        created_at: "2024-01-01T00:00:00Z",
        variants: [{ name: "A", prompt_version_id: "v1" }],
      },
    ];
    vi.mocked(http.listExperiments).mockResolvedValue(mockData);

    const { result } = renderHook(() => useExperiments("agent-1"), {
      wrapper: createQueryClientWrapper().wrapper,
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
    const { queryClient, wrapper } = createQueryClientWrapper();

    renderHook(() => useExperiments("test-agent"), { wrapper });

    await waitFor(() => {
      expect(queryClient.getQueryCache().find({ queryKey: agentKeys.experiments("test-agent") })).toBeDefined();
    });

    const cache = queryClient.getQueryCache().find({ queryKey: agentKeys.experiments("test-agent") });
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
      wrapper: createQueryClientWrapper().wrapper,
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(http.getExperimentMetrics).not.toHaveBeenCalled();
  });

  it("should be enabled and fetch when experimentId is valid", async () => {
    const mockData = [
      {
        variant_id: "v1",
        variant_name: "Variant A",
        total_requests: 10,
        successful_requests: 9,
        failed_requests: 1,
        success_rate: 0.95,
        avg_latency_ms: 100,
        avg_cost_usd: 0.001,
        total_cost_usd: 0.01,
      },
    ];
    vi.mocked(http.getExperimentMetrics).mockResolvedValue(mockData);

    const { result } = renderHook(() => useExperimentMetrics("exp-1"), {
      wrapper: createQueryClientWrapper().wrapper,
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
    const mockData = [{ variant_id: "v1", variant_name: "Variant A", total_requests: 2, successful_requests: 1, failed_requests: 1, success_rate: 0.5, avg_latency_ms: 50, avg_cost_usd: 0.01, total_cost_usd: 0.02 }];
    vi.mocked(http.getExperimentMetrics).mockResolvedValue(mockData);

    const { queryClient, wrapper } = createQueryClientWrapper();

    renderHook(() => useExperimentMetrics("test-exp"), { wrapper });

    await waitFor(() => {
      expect(queryClient.getQueryCache().find({ queryKey: agentKeys.experimentMetrics("test-exp") })).toBeDefined();
    });

    expect(
      queryClient.getQueryCache().find({ queryKey: agentKeys.experimentMetrics("test-exp") })?.queryKey,
    ).toEqual(agentKeys.experimentMetrics("test-exp"));
  });
});
