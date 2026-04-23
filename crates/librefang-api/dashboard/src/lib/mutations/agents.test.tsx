import { describe, it, expect, vi } from "vitest";
import { renderHook } from "@testing-library/react";
import {
  useSwitchAgentSession,
  useDeleteAgentSession,
  usePatchAgent,
  usePatchAgentConfig,
  useSpawnAgent,
  useCloneAgent,
  useSuspendAgent,
  useDeleteAgent,
  useResumeAgent,
  useCreateAgentSession,
  useResolveApproval,
} from "./agents";
import { agentKeys, sessionKeys, overviewKeys, approvalKeys } from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";

vi.mock("../http/client", () => ({
  switchAgentSession: vi.fn().mockResolvedValue({}),
  deleteSession: vi.fn().mockResolvedValue({}),
  patchAgent: vi.fn().mockResolvedValue({}),
  patchAgentConfig: vi.fn().mockResolvedValue({}),
  spawnAgent: vi.fn().mockResolvedValue({}),
  cloneAgent: vi.fn().mockResolvedValue({}),
  suspendAgent: vi.fn().mockResolvedValue({}),
  resumeAgent: vi.fn().mockResolvedValue({}),
  deleteAgent: vi.fn().mockResolvedValue({}),
  createAgentSession: vi.fn().mockResolvedValue({}),
  resolveApproval: vi.fn().mockResolvedValue({}),
}));

describe("useSwitchAgentSession", () => {
  it("invalidates agent detail, agent sessions, and session lists", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useSwitchAgentSession(), {
      wrapper,
    });

    await result.current.mutateAsync({
      agentId: "agent-1",
      sessionId: "session-1",
    });

    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.detail("agent-1"),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.sessions("agent-1"),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: sessionKeys.lists(),
    });
  });
});

describe("useDeleteAgentSession", () => {
  it("with agentId invalidates agent sessions, agent detail, and session lists", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useDeleteAgentSession(), {
      wrapper,
    });

    await result.current.mutateAsync({
      sessionId: "session-1",
      agentId: "agent-1",
    });

    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.sessions("agent-1"),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.detail("agent-1"),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: sessionKeys.lists(),
    });
  });

  it("without agentId invalidates agentKeys.all and session lists", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useDeleteAgentSession(), {
      wrapper,
    });

    await result.current.mutateAsync({
      sessionId: "session-1",
    });

    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.all,
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: sessionKeys.lists(),
    });
  });
});

describe("usePatchAgent", () => {
  it("invalidates agent lists and agent detail on rename", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => usePatchAgent(), { wrapper });

    await result.current.mutateAsync({
      agentId: "agent-1",
      body: { name: "renamed-agent" },
    });

    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.lists(),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.detail("agent-1"),
    });
  });
});

describe("usePatchAgentConfig", () => {
  it("invalidates agent lists and agent detail", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => usePatchAgentConfig(), {
      wrapper,
    });

    await result.current.mutateAsync({
      agentId: "agent-1",
      config: { max_tokens: 4096 },
    });

    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.lists(),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.detail("agent-1"),
    });
  });
});

describe.each([
  { name: "useSpawnAgent", hook: useSpawnAgent, arg: "agent-1" },
  { name: "useCloneAgent", hook: useCloneAgent, arg: "agent-1" },
  { name: "useSuspendAgent", hook: useSuspendAgent, arg: "agent-1" },
  { name: "useDeleteAgent", hook: useDeleteAgent, arg: "agent-1" },
  { name: "useResumeAgent", hook: useResumeAgent, arg: "agent-1" },
])("$name invalidates agentKeys.lists and overviewKeys.snapshot", ({ hook, arg }) => {
  it("invalidates both keys", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => hook(), { wrapper });

    await result.current.mutateAsync(arg);

    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: agentKeys.lists() });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: overviewKeys.snapshot() });
  });
});

describe("useCreateAgentSession", () => {
  it("invalidates agentKeys.sessions, agentKeys.detail, and sessionKeys.lists", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useCreateAgentSession(), {
      wrapper,
    });

    await result.current.mutateAsync({ agentId: "agent-1", label: "test" });

    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.sessions("agent-1"),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.detail("agent-1"),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: sessionKeys.lists(),
    });
  });
});

describe("useResolveApproval", () => {
  it("invalidates approvalKeys.all", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useResolveApproval(), {
      wrapper,
    });

    await result.current.mutateAsync({ id: "approval-1", approved: true });

    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: approvalKeys.all });
  });
});
