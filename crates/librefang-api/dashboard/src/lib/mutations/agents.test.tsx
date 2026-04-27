import { describe, it, expect, vi } from "vitest";
import * as http from "../http/client";
import { renderHook } from "@testing-library/react";
import {
  useSwitchAgentSession,
  useDeleteAgentSession,
  usePatchAgent,
  usePatchAgentConfig,
  usePatchHandAgentRuntimeConfig,
  useClearHandAgentRuntimeConfig,
  useSpawnAgent,
  useSpawnUarAgent,
  useCloneAgent,
  useSuspendAgent,
  useDeleteAgent,
  useResumeAgent,
  useCreateAgentSession,
  useResolveApproval,
} from "./agents";
import { agentKeys, handKeys, sessionKeys, overviewKeys, approvalKeys } from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";

vi.mock("../http/client", () => ({
  switchAgentSession: vi.fn().mockResolvedValue({}),
  deleteSession: vi.fn().mockResolvedValue({}),
  patchAgent: vi.fn().mockResolvedValue({}),
  patchAgentConfig: vi.fn().mockResolvedValue({}),
  patchHandAgentRuntimeConfig: vi.fn().mockResolvedValue({}),
  clearHandAgentRuntimeConfig: vi.fn().mockResolvedValue(undefined),
  spawnAgent: vi.fn().mockResolvedValue({}),
  spawnUarAgent: vi.fn().mockResolvedValue({}),
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

describe("usePatchAgentConfig (non-hand)", () => {
  it("calls patchAgentConfig (→ /api/agents/{id}/config) and invalidates agent lists + detail", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    vi.mocked(http.patchAgentConfig).mockClear();
    vi.mocked(http.patchHandAgentRuntimeConfig).mockClear();

    const { result } = renderHook(() => usePatchAgentConfig(), {
      wrapper,
    });

    await result.current.mutateAsync({
      agentId: "agent-1",
      config: { max_tokens: 4096 },
    });

    // Hits the standalone /config route, never the hand override endpoint.
    expect(http.patchAgentConfig).toHaveBeenCalledTimes(1);
    expect(http.patchAgentConfig).toHaveBeenCalledWith("agent-1", {
      max_tokens: 4096,
    });
    expect(http.patchHandAgentRuntimeConfig).not.toHaveBeenCalled();

    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.lists(),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.detail("agent-1"),
    });
    // Non-hand mutations MUST NOT dirty hand-detail caches — asserting this
    // guards against regressions that widen invalidation unnecessarily.
    expect(invalidateSpy).not.toHaveBeenCalledWith({
      queryKey: handKeys.details(),
    });
  });
});

describe("usePatchHandAgentRuntimeConfig (hand)", () => {
  it("calls patchHandAgentRuntimeConfig (→ /api/agents/{id}/hand-runtime-config) and invalidates agent lists + detail + handKeys.details()", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    vi.mocked(http.patchAgentConfig).mockClear();
    vi.mocked(http.patchHandAgentRuntimeConfig).mockClear();

    const { result } = renderHook(() => usePatchHandAgentRuntimeConfig(), {
      wrapper,
    });

    await result.current.mutateAsync({
      agentId: "hand-agent-1",
      // Tri-state payload: api_key_env set, base_url cleared via empty string.
      config: { model: "gpt-4o", api_key_env: "OPENAI_KEY", base_url: "" },
    });

    // Hits the hand-runtime-config route, never the standalone /config path.
    expect(http.patchHandAgentRuntimeConfig).toHaveBeenCalledTimes(1);
    expect(http.patchHandAgentRuntimeConfig).toHaveBeenCalledWith(
      "hand-agent-1",
      { model: "gpt-4o", api_key_env: "OPENAI_KEY", base_url: "" },
    );
    expect(http.patchAgentConfig).not.toHaveBeenCalled();

    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.lists(),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.detail("hand-agent-1"),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: handKeys.details(),
    });
  });

  it("forwards whitespace-only hand override fields to the hand runtime API helper", async () => {
    const { wrapper } = createQueryClientWrapper();
    vi.mocked(http.patchHandAgentRuntimeConfig).mockClear();

    const { result } = renderHook(() => usePatchHandAgentRuntimeConfig(), {
      wrapper,
    });

    await result.current.mutateAsync({
      agentId: "hand-agent-1",
      config: {
        model: "gpt-4o",
        api_key_env: "   ",
        base_url: "   ",
      },
    });

    expect(http.patchHandAgentRuntimeConfig).toHaveBeenCalledWith(
      "hand-agent-1",
      { model: "gpt-4o", api_key_env: "   ", base_url: "   " },
    );
  });
});

describe("useClearHandAgentRuntimeConfig", () => {
  it("calls clearHandAgentRuntimeConfig (→ DELETE /api/agents/{id}/hand-runtime-config) and invalidates agent lists + detail + handKeys.details()", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    vi.mocked(http.clearHandAgentRuntimeConfig).mockClear();

    const { result } = renderHook(() => useClearHandAgentRuntimeConfig(), {
      wrapper,
    });

    await result.current.mutateAsync("hand-agent-1");

    expect(http.clearHandAgentRuntimeConfig).toHaveBeenCalledTimes(1);
    expect(http.clearHandAgentRuntimeConfig).toHaveBeenCalledWith("hand-agent-1");

    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.lists(),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.detail("hand-agent-1"),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: handKeys.details(),
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

describe("useSpawnUarAgent", () => {
  it("invalidates agentKeys.lists and overviewKeys.snapshot", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useSpawnUarAgent(), { wrapper });

    await result.current.mutateAsync({
      content: "# Agent: test\n\n## Metadata\nversion: 1.0.0\n## Identity\nrole: assistant",
      format: "markdown",
    });

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
