import { describe, it, expect, vi } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import * as http from "../http/client";
import {
  useActivatePromptVersion,
  useStartExperiment,
  usePauseExperiment,
  useCompleteExperiment,
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
    startExperiment: vi.fn().mockResolvedValue({ id: "exp-1", status: "running" }),
    pauseExperiment: vi.fn().mockResolvedValue({ id: "exp-1", status: "paused" }),
    completeExperiment: vi.fn().mockResolvedValue({ id: "exp-1", status: "completed" }),
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

  it("patches a returned version without refetching the same list", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    queryClient.setQueryData(agentKeys.promptVersions("agent-1"), [
      { id: "v-1", is_active: false },
      { id: "v-2", is_active: true },
    ]);
    vi.mocked(http.activatePromptVersion).mockResolvedValueOnce({
      id: "v-1",
      agent_id: "agent-1",
      version: 1,
      content_hash: "hash",
      system_prompt: "prompt",
      tools: [],
      variables: [],
      created_at: "2026-01-01T00:00:00Z",
      created_by: "user",
      is_active: true,
    });
    const { result } = renderHook(() => useActivatePromptVersion(), { wrapper });

    await result.current.mutateAsync({ versionId: "v-1", agentId: "agent-1" });

    expect(invalidateSpy).not.toHaveBeenCalledWith({
      queryKey: agentKeys.promptVersions("agent-1"),
    });
    expect(queryClient.getQueryData(agentKeys.promptVersions("agent-1"))).toEqual([
      expect.objectContaining({ id: "v-1", is_active: true }),
      { id: "v-2", is_active: false },
    ]);
  });
});

describe("useStartExperiment", () => {
  it("patches experiments and invalidates only experiment metrics", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const setSpy = vi.spyOn(queryClient, "setQueryData");
    const { result } = renderHook(() => useStartExperiment(), { wrapper });

    await result.current.mutateAsync({ experimentId: "exp-1", agentId: "agent-1" });

    await waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalledTimes(1);
    });
    expect(invalidateSpy).not.toHaveBeenCalledWith({
      queryKey: agentKeys.experiments("agent-1"),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.experimentMetrics("exp-1"),
    });
    expect(setSpy).toHaveBeenCalledWith(
      agentKeys.experiments("agent-1"),
      expect.any(Function),
    );
    const updater = setSpy.mock.calls[0]?.[1] as (
      previous: Array<{ id: string; status: string }>,
    ) => unknown;
    expect(updater([{ id: "exp-1", status: "draft" }])).toEqual([
      { id: "exp-1", status: "running" },
    ]);
  });
});

describe("usePauseExperiment", () => {
  it("patches experiments and invalidates only experiment metrics", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => usePauseExperiment(), { wrapper });

    result.current.mutate({ experimentId: "exp-1", agentId: "agent-1" });

    await waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalledTimes(1);
    });
    expect(invalidateSpy).not.toHaveBeenCalledWith({
      queryKey: agentKeys.experiments("agent-1"),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.experimentMetrics("exp-1"),
    });
  });
});

describe("useCompleteExperiment", () => {
  it("patches experiments and invalidates only experiment metrics", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useCompleteExperiment(), { wrapper });

    await result.current.mutateAsync({ experimentId: "exp-1", agentId: "agent-1" });

    expect(invalidateSpy).not.toHaveBeenCalledWith({
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
