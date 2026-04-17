import { describe, it, expect, vi } from "vitest";
import { renderHook } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";
import {
  useSwitchAgentSession,
  useDeleteAgentSession,
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

vi.mock("../http/client", () => ({
  switchAgentSession: vi.fn().mockResolvedValue({}),
  deleteSession: vi.fn().mockResolvedValue({}),
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
    const qc = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const spy = vi.spyOn(qc, "invalidateQueries");
    const Wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={qc}>{children}</QueryClientProvider>
    );

    const { result } = renderHook(() => useSwitchAgentSession(), {
      wrapper: Wrapper,
    });

    await result.current.mutateAsync({
      agentId: "agent-1",
      sessionId: "session-1",
    });

    expect(spy).toHaveBeenCalledWith({
      queryKey: agentKeys.detail("agent-1"),
    });
    expect(spy).toHaveBeenCalledWith({
      queryKey: agentKeys.sessions("agent-1"),
    });
    expect(spy).toHaveBeenCalledWith({
      queryKey: sessionKeys.lists(),
    });
  });
});

describe("useDeleteAgentSession", () => {
  it("with agentId invalidates agent sessions, agent detail, and session lists", async () => {
    const qc = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const spy = vi.spyOn(qc, "invalidateQueries");
    const Wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={qc}>{children}</QueryClientProvider>
    );

    const { result } = renderHook(() => useDeleteAgentSession(), {
      wrapper: Wrapper,
    });

    await result.current.mutateAsync({
      sessionId: "session-1",
      agentId: "agent-1",
    });

    expect(spy).toHaveBeenCalledWith({
      queryKey: agentKeys.sessions("agent-1"),
    });
    expect(spy).toHaveBeenCalledWith({
      queryKey: agentKeys.detail("agent-1"),
    });
    expect(spy).toHaveBeenCalledWith({
      queryKey: sessionKeys.lists(),
    });
  });

  it("without agentId invalidates agentKeys.all and session lists", async () => {
    const qc = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const spy = vi.spyOn(qc, "invalidateQueries");
    const Wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={qc}>{children}</QueryClientProvider>
    );

    const { result } = renderHook(() => useDeleteAgentSession(), {
      wrapper: Wrapper,
    });

    await result.current.mutateAsync({
      sessionId: "session-1",
    });

    expect(spy).toHaveBeenCalledWith({
      queryKey: agentKeys.all,
    });
    expect(spy).toHaveBeenCalledWith({
      queryKey: sessionKeys.lists(),
    });
  });
});

describe("usePatchAgentConfig", () => {
  it("invalidates agent lists and agent detail", async () => {
    const qc = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const spy = vi.spyOn(qc, "invalidateQueries");
    const Wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={qc}>{children}</QueryClientProvider>
    );

    const { result } = renderHook(() => usePatchAgentConfig(), {
      wrapper: Wrapper,
    });

    await result.current.mutateAsync({
      agentId: "agent-1",
      config: { max_tokens: 4096 },
    });

    expect(spy).toHaveBeenCalledWith({
      queryKey: agentKeys.lists(),
    });
    expect(spy).toHaveBeenCalledWith({
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
])("$name invalidates agentKeys.all and overviewKeys.snapshot", ({ hook, arg }) => {
  it("invalidates both keys", async () => {
    const qc = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const spy = vi.spyOn(qc, "invalidateQueries");
    const Wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={qc}>{children}</QueryClientProvider>
    );

    const { result } = renderHook(() => hook(), { wrapper: Wrapper });

    await result.current.mutateAsync(arg);

    expect(spy).toHaveBeenCalledWith({ queryKey: agentKeys.all });
    expect(spy).toHaveBeenCalledWith({ queryKey: overviewKeys.snapshot() });
  });
});

describe("useCreateAgentSession", () => {
  it("invalidates agentKeys.all", async () => {
    const qc = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const spy = vi.spyOn(qc, "invalidateQueries");
    const Wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={qc}>{children}</QueryClientProvider>
    );

    const { result } = renderHook(() => useCreateAgentSession(), {
      wrapper: Wrapper,
    });

    await result.current.mutateAsync({ agentId: "agent-1", label: "test" });

    expect(spy).toHaveBeenCalledWith({ queryKey: agentKeys.all });
  });
});

describe("useResolveApproval", () => {
  it("invalidates approvalKeys.all", async () => {
    const qc = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const spy = vi.spyOn(qc, "invalidateQueries");
    const Wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={qc}>{children}</QueryClientProvider>
    );

    const { result } = renderHook(() => useResolveApproval(), {
      wrapper: Wrapper,
    });

    await result.current.mutateAsync({ id: "approval-1", approved: true });

    expect(spy).toHaveBeenCalledWith({ queryKey: approvalKeys.all });
  });
});
