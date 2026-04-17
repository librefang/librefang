import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { useAgentDetail, useAgentSessions, useAgentTemplates } from "./agents";
import * as httpClient from "../http/client";
import { agentKeys } from "./keys";
import { createQueryClientWrapper } from "../test/query-client";

vi.mock("../http/client", () => ({
  getAgentDetail: vi.fn(),
  listAgentSessions: vi.fn(),
  listAgentTemplates: vi.fn(),
}));

beforeEach(() => {
  vi.clearAllMocks();
});

describe("useAgentDetail", () => {
  it("should be disabled when agentId is empty string", () => {
    const { result } = renderHook(() => useAgentDetail(""), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(httpClient.getAgentDetail).not.toHaveBeenCalled();
  });

  it("should be enabled when agentId is a valid string", async () => {
    const mockAgent = { id: "agent-1", name: "Test Agent" };
    vi.mocked(httpClient.getAgentDetail).mockResolvedValue(mockAgent);

    const { result } = renderHook(() => useAgentDetail("agent-1"), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(result.current.data).toEqual(mockAgent);
    expect(httpClient.getAgentDetail).toHaveBeenCalledWith("agent-1");
  });

  it("should use the correct queryKey", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();

    renderHook(() => useAgentDetail("test-id"), { wrapper });

    await waitFor(() => {
      expect(queryClient.getQueryCache().find({ queryKey: agentKeys.detail("test-id") })).toBeDefined();
    });

    const cache = queryClient.getQueryCache().find({ queryKey: agentKeys.detail("test-id") });
    expect(cache).toBeDefined();
    expect(cache?.queryKey).toEqual(agentKeys.detail("test-id"));
  });
});

describe("useAgentSessions", () => {
  it("should be disabled when agentId is empty string", () => {
    const { result } = renderHook(() => useAgentSessions(""), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(httpClient.listAgentSessions).not.toHaveBeenCalled();
  });

  it("should be enabled when agentId is a valid string", async () => {
    const mockSessions = [{ session_id: "session-1", agent_id: "agent-1" }];
    vi.mocked(httpClient.listAgentSessions).mockResolvedValue(mockSessions);

    const { result } = renderHook(() => useAgentSessions("agent-1"), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(result.current.data).toEqual(mockSessions);
    expect(httpClient.listAgentSessions).toHaveBeenCalledWith("agent-1");
  });
});

describe("useAgentTemplates", () => {
  it("should fetch by default when enabled is not provided", async () => {
    const mockTemplates = [{ name: "Test Template", description: "Test description" }];
    vi.mocked(httpClient.listAgentTemplates).mockResolvedValue(mockTemplates);

    const { result } = renderHook(() => useAgentTemplates(), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(result.current.data).toEqual(mockTemplates);
    expect(httpClient.listAgentTemplates).toHaveBeenCalled();
  });

  it("should not fetch when enabled is false", () => {
    const { result } = renderHook(() => useAgentTemplates({ enabled: false }), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(httpClient.listAgentTemplates).not.toHaveBeenCalled();
  });

  it("should fetch when enabled is true", async () => {
    const mockTemplates = [{ name: "Test Template", description: "Test description" }];
    vi.mocked(httpClient.listAgentTemplates).mockResolvedValue(mockTemplates);

    const { result } = renderHook(() => useAgentTemplates({ enabled: true }), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(result.current.data).toEqual(mockTemplates);
    expect(httpClient.listAgentTemplates).toHaveBeenCalled();
  });
});
