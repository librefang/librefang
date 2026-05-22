import { afterEach, describe, it, expect, vi } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { useCompleteExperiment, useResetAgentSession } from "./agents";
import { useSetSessionLabel } from "./sessions";
import { useInstallSkill } from "./skills";
import {
  agentKeys,
  sessionKeys,
  skillKeys,
  fanghubKeys,
  clawhubKeys,
  clawhubCnKeys,
  skillhubKeys,
  overviewKeys,
} from "../queries/keys";
import {
  chatSessionCacheKey,
  clearChatSessionCacheForAgent,
  getCachedChatMessages,
  setCachedChatMessages,
} from "../chatSessionCache";
import { createQueryClientWrapper } from "../test/query-client";

vi.mock("../http/client", async () => {
  const actual = await vi.importActual<typeof import("../http/client")>(
    "../http/client",
  );
  return {
    ...actual,
    completeExperiment: vi.fn().mockResolvedValue({}),
    resetAgentSession: vi.fn().mockResolvedValue({}),
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

describe("useResetAgentSession", () => {
  afterEach(() => {
    clearChatSessionCacheForAgent("agent-1");
    clearChatSessionCacheForAgent("agent-2");
  });

  it("invalidates reset-stale query keys and clears cached chat messages", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const agentSessionKey = chatSessionCacheKey("agent-1", "sess-1");
    const otherAgentSessionKey = chatSessionCacheKey("agent-2", "sess-1");
    setCachedChatMessages(agentSessionKey, ["stale"]);
    setCachedChatMessages(chatSessionCacheKey("agent-1", null), ["stale-current"]);
    setCachedChatMessages(otherAgentSessionKey, ["fresh"]);

    const { result } = renderHook(() => useResetAgentSession(), {
      wrapper,
    });

    await result.current.mutateAsync("agent-1");

    await waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalledTimes(5);
    });
    expect(getCachedChatMessages(agentSessionKey)).toBeUndefined();
    expect(getCachedChatMessages(chatSessionCacheKey("agent-1", null))).toBeUndefined();
    expect(getCachedChatMessages(otherAgentSessionKey)).toEqual(["fresh"]);
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.detail("agent-1"),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.sessionSnapshots("agent-1"),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.sessions("agent-1"),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: sessionKeys.lists(),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: overviewKeys.snapshot(),
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
  // #4689 — skill install must invalidate every hub surface so the per-hub
  // browse buttons (FangHub / SkillHub / ClawHub / ClawHub-CN) flip to
  // "Installed" without waiting for the next refetchInterval.
  const ALL_SKILL_SURFACE_KEYS = [
    skillKeys.all,
    fanghubKeys.all,
    clawhubKeys.all,
    clawhubCnKeys.all,
    skillhubKeys.all,
  ] as const;

  it("invalidates every skill surface", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useInstallSkill(), {
      wrapper,
    });

    await result.current.mutateAsync({ name: "test-skill" });

    await waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalledTimes(ALL_SKILL_SURFACE_KEYS.length);
    });
    for (const key of ALL_SKILL_SURFACE_KEYS) {
      expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: key });
    }
  });

  it("invalidates every skill surface with hand parameter", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useInstallSkill(), {
      wrapper,
    });

    await result.current.mutateAsync({ name: "test-skill", hand: "test-hand" });

    await waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalledTimes(ALL_SKILL_SURFACE_KEYS.length);
    });
    for (const key of ALL_SKILL_SURFACE_KEYS) {
      expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: key });
    }
  });
});
