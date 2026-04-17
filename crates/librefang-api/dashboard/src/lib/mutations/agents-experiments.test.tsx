import { describe, it, expect, vi } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import {
  useActivatePromptVersion,
  useStartExperiment,
  usePauseExperiment,
  useDeletePromptVersion,
  useCreatePromptVersion,
  useCreateExperiment,
} from "./agents";
import { agentKeys } from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";

vi.mock("../http/client", async () => {
  const actual = await vi.importActual<typeof import("../http/client")>(
    "../http/client",
  );
  return {
    ...actual,
    activatePromptVersion: vi.fn().mockResolvedValue({}),
    startExperiment: vi.fn().mockResolvedValue({}),
    pauseExperiment: vi.fn().mockResolvedValue({}),
    deletePromptVersion: vi.fn().mockResolvedValue({}),
    createPromptVersion: vi.fn().mockResolvedValue({}),
    createExperiment: vi.fn().mockResolvedValue({}),
  };
});

describe("useActivatePromptVersion", () => {
  it("invalidates promptVersions and detail keys for the agent", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useActivatePromptVersion(), { wrapper });

    result.current.mutate({ versionId: "v-1", agentId: "agent-1" });

    await waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalledTimes(2);
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.promptVersions("agent-1"),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.detail("agent-1"),
    });
  });
});

describe("useStartExperiment", () => {
  it("invalidates experiments and experimentMetrics keys", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useStartExperiment(), { wrapper });

    result.current.mutate({ experimentId: "exp-1", agentId: "agent-1" });

    await waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalledTimes(2);
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.experiments("agent-1"),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.experimentMetrics("exp-1"),
    });
  });
});

describe("usePauseExperiment", () => {
  it("invalidates experiments and experimentMetrics keys", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => usePauseExperiment(), { wrapper });

    result.current.mutate({ experimentId: "exp-1", agentId: "agent-1" });

    await waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalledTimes(2);
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.experiments("agent-1"),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.experimentMetrics("exp-1"),
    });
  });
});

describe("useDeletePromptVersion", () => {
  it("invalidates promptVersions for the agent", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useDeletePromptVersion(), { wrapper });

    await result.current.mutateAsync({ versionId: "v-1", agentId: "agent-1" });

    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.promptVersions("agent-1"),
    });
  });
});

describe("useCreatePromptVersion", () => {
  it("invalidates promptVersions for the agent", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useCreatePromptVersion(), { wrapper });

    await result.current.mutateAsync({ agentId: "agent-1", version: { version: 1, content_hash: "abc", system_prompt: "sys", tools: [], variables: [], created_by: "user" } });

    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.promptVersions("agent-1"),
    });
  });
});

describe("useCreateExperiment", () => {
  it("invalidates experiments for the agent", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useCreateExperiment(), { wrapper });

    await result.current.mutateAsync({ agentId: "agent-1", experiment: { name: "exp-1", status: "draft", traffic_split: [50, 50], success_criteria: { require_user_helpful: true, require_no_tool_errors: true, require_non_empty: true }, variants: [] } });

    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.experiments("agent-1"),
    });
  });
});
