import { describe, it, expect, vi } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { useCompleteExperiment } from "./agents";
import { useSetSessionLabel } from "./sessions";
import { useInstallSkill } from "./skills";
import { agentKeys, sessionKeys, skillKeys, fanghubKeys } from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";

vi.mock("../http/client", async () => {
  const actual = await vi.importActual<typeof import("../http/client")>(
    "../http/client",
  );
  return {
    ...actual,
    completeExperiment: vi.fn().mockResolvedValue({}),
    setSessionLabel: vi.fn().mockResolvedValue({}),
    installSkill: vi.fn().mockResolvedValue({}),
  };
});

describe("useCompleteExperiment", () => {
  it("invalidates experiments and experimentMetrics keys", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useCompleteExperiment(), {
      wrapper,
    });

    const variables = { experimentId: "exp-1", agentId: "agent-1" };
    await result.current.mutateAsync(variables);

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

describe("useSetSessionLabel", () => {
  it("with agentId invalidates session lists, detail and agent sessions", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useSetSessionLabel(), {
      wrapper,
    });

    await result.current.mutateAsync({
      sessionId: "sess-1",
      label: "test label",
      agentId: "agent-1",
    });

    await waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalledTimes(3);
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: sessionKeys.lists(),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: sessionKeys.detail("sess-1"),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.sessions("agent-1"),
    });
  });

  it("without agentId invalidates session lists and detail", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useSetSessionLabel(), {
      wrapper,
    });

    await result.current.mutateAsync({ sessionId: "sess-1", label: "test label" });

    await waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalledTimes(2);
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: sessionKeys.lists(),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: sessionKeys.detail("sess-1"),
    });
  });
});

describe("useInstallSkill", () => {
  it("invalidates skillKeys.all and fanghubKeys.all", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useInstallSkill(), {
      wrapper,
    });

    await result.current.mutateAsync({ name: "test-skill" });

    await waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalledTimes(2);
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: skillKeys.all,
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: fanghubKeys.all,
    });
  });

  it("invalidates skillKeys.all and fanghubKeys.all with hand parameter", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useInstallSkill(), {
      wrapper,
    });

    await result.current.mutateAsync({ name: "test-skill", hand: "test-hand" });

    await waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalledTimes(2);
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: skillKeys.all,
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: fanghubKeys.all,
    });
  });
});
