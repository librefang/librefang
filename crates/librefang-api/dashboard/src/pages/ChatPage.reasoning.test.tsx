import { act, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import type { AgentItem } from "../api";
import { useChatMessages, useOptimisticPinnedReasoning } from "./ChatPage";

const sendAgentMessage = vi.fn().mockResolvedValue({ response: "http answer" });

// `t` and the object wrapping it must be referentially stable: the history-load
// effect lists `t` in its dependency array, so a fresh lambda per render re-runs
// the effect on every render and spins the hook forever.
const translation = { t: (key: string) => key };
vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>("react-i18next");
  return { ...actual, useTranslation: () => translation };
});

vi.mock("../lib/queries/commands", () => ({
  useChatCommands: () => ({ data: [], isPending: false }),
}));

vi.mock("../lib/mutations/agents", () => ({
  useCreateAgentSession: () => ({ mutateAsync: vi.fn(), isPending: false }),
  useDeleteAgentSession: () => ({ mutateAsync: vi.fn(), isPending: false }),
  usePatchAgentRuntimeConfig: () => ({ mutateAsync: vi.fn(), isPending: false }),
  useResolveApproval: () => ({ mutateAsync: vi.fn(), isPending: false }),
  useSendAgentMessage: () => ({ mutateAsync: sendAgentMessage, isPending: false }),
  useStopAgent: () => ({ mutateAsync: vi.fn(), isPending: false }),
  useUploadAgentFile: () => ({ mutateAsync: vi.fn(), isPending: false }),
}));

vi.mock("../lib/queries/agents", () => ({
  agentQueries: {
    session: (agentId: string, sessionId: string | null) => ({
      queryKey: ["agent-session", agentId, sessionId],
      queryFn: () => Promise.resolve({ messages: [] }),
    }),
    sessionContext: () => ({ queryKey: ["ctx"], queryFn: () => Promise.resolve({}) }),
  },
  useAgents: () => ({ data: [] }),
  useAgentSessions: () => ({ data: [] }),
}));

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return {
    ...actual,
    buildAuthenticatedWebSocket: (path: string) => ({
      url: `ws://test${path}`,
      protocols: [] as string[],
    }),
  };
});

// The store double is stateful on the one field the send path consumes: the
// real store both arms the one-shot and clears it after the turn, and a static
// mock could not tell those two moments apart.
const store = vi.hoisted(() => {
  const state = { nextReasoningEffort: null as string | null };
  return {
    state,
    setNextReasoningEffort: vi.fn((value: string | null) => {
      state.nextReasoningEffort = value;
    }),
  };
});

vi.mock("../lib/store", () => ({
  useUIStore: (selector: (s: Record<string, unknown>) => unknown) =>
    selector({
      addSkillOutput: vi.fn(),
      addToast: vi.fn(),
      deepThinking: false,
      showThinkingProcess: false,
      nextReasoningEffort: store.state.nextReasoningEffort,
      setNextReasoningEffort: store.setNextReasoningEffort,
    }),
}));

type Listener = (event: unknown) => void;

class MockWebSocket {
  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSED = 3;
  static instances: MockWebSocket[] = [];

  readyState = MockWebSocket.CONNECTING;
  sent: string[] = [];
  onopen: (() => void) | null = null;
  onclose: ((event: { code: number }) => void) | null = null;
  onerror: (() => void) | null = null;
  private listeners = new Map<string, Set<Listener>>();

  constructor(public url: string) {
    MockWebSocket.instances.push(this);
  }

  addEventListener(type: string, fn: Listener) {
    const set = this.listeners.get(type) ?? new Set<Listener>();
    set.add(fn);
    this.listeners.set(type, set);
  }

  removeEventListener(type: string, fn: Listener) {
    this.listeners.get(type)?.delete(fn);
  }

  send(data: string) {
    this.sent.push(data);
  }

  close() {
    this.readyState = MockWebSocket.CLOSED;
    this.onclose?.({ code: 1000 });
  }

  emitOpen() {
    this.readyState = MockWebSocket.OPEN;
    this.onopen?.();
  }
}

function wrapper({ children }: { children: ReactNode }) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0, staleTime: 0 } },
  });
  return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>;
}

const AGENTS = [{ id: "agent-a", name: "Agent A" }] as AgentItem[];

async function connected() {
  const view = renderHook(({ agent }: { agent: string }) => useChatMessages(agent, AGENTS), {
    wrapper,
    initialProps: { agent: AGENTS[0].id },
  });
  const socket = MockWebSocket.instances[MockWebSocket.instances.length - 1];
  if (!socket) throw new Error("hook did not open a socket");
  await act(async () => {
    socket.emitOpen();
  });
  await waitFor(() => expect(view.result.current.wsConnected).toBe(true));
  return { ...view, socket };
}

/** The most recent turn frame the daemon received, parsed. */
function turnFrame(socket: MockWebSocket): Record<string, unknown> {
  const frames = socket.sent
    .map((raw) => JSON.parse(raw) as Record<string, unknown>)
    .filter((parsed) => parsed.type === "message");
  const frame = frames[frames.length - 1];
  if (!frame) throw new Error("no turn frame was written");
  return frame;
}

describe("chat reasoning effort travels with the turn", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    MockWebSocket.instances = [];
    store.state.nextReasoningEffort = null;
    vi.stubGlobal("WebSocket", MockWebSocket);
  });

  // The one-shot: the operator armed a level on the composer control and it
  // must ride exactly one user message — sent over the socket — and then fall
  // back to the session/agent default on its own.
  it("sends the armed level on the next WS frame, then resets to the session default", async () => {
    store.state.nextReasoningEffort = "xhigh";
    const { result, socket, rerender } = await connected();

    await act(async () => {
      result.current.sendMessage("first turn");
    });
    expect(turnFrame(socket).reasoning_effort).toBe("xhigh");
    // Consumed: the store is no longer holding it for a later message.
    expect(store.setNextReasoningEffort).toHaveBeenCalledWith(null);
    expect(store.state.nextReasoningEffort).toBeNull();

    // The re-render is what a real store subscription would do for us; the
    // mock store cannot push, so the test pulls.
    await act(async () => {
      rerender({ agent: "agent-a" });
    });
    await act(async () => {
      result.current.sendMessage("second turn");
    });
    const second = turnFrame(socket);
    // Absent, not `null`: an unset override is not a statement about the turn.
    expect("reasoning_effort" in second).toBe(false);
  });

  it("sends nothing extra when no level is armed", async () => {
    const { result, socket } = await connected();

    await act(async () => {
      result.current.sendMessage("plain turn");
    });

    expect("reasoning_effort" in turnFrame(socket)).toBe(false);
  });

  // The HTTP fallback is the same turn over a different transport — a socket
  // that never opened must not silently drop the operator's choice.
  it("carries the armed level on the HTTP fallback too", async () => {
    store.state.nextReasoningEffort = "high";
    const view = renderHook(({ agent }: { agent: string }) => useChatMessages(agent, AGENTS), {
      wrapper,
      initialProps: { agent: AGENTS[0].id },
    });

    await act(async () => {
      view.result.current.sendMessage("fallback turn");
    });
    await waitFor(() => expect(sendAgentMessage).toHaveBeenCalled());

    const call = sendAgentMessage.mock.calls[0][0] as {
      options?: { reasoning_effort?: string };
    };
    expect(call.options?.reasoning_effort).toBe("high");
    expect(store.state.nextReasoningEffort).toBeNull();
  });

  it("leaves the armed level alone when the turn never leaves the composer", async () => {
    store.state.nextReasoningEffort = "max";
    const { result } = await connected();

    // An empty submit is a no-op, not a consumed one-shot.
    await act(async () => {
      result.current.sendMessage("   ");
    });

    expect(store.setNextReasoningEffort).not.toHaveBeenCalled();
    expect(store.state.nextReasoningEffort).toBe("max");
  });
});

// The pinned level is server state, but the window between a PATCH and the
// server echo is long enough to see — and on a daemon without the field, the
// echo never comes at all. These pin the merge policy that keeps the control
// truthful across both.
describe("session pin merge (optimistic over server echo)", () => {
  const renderMerge = (sessionId: string | null, serverValue: string | null | undefined) =>
    renderHook(
      ({ sessionId, serverValue }: { sessionId: string | null; serverValue: string | null | undefined }) =>
        useOptimisticPinnedReasoning(sessionId, serverValue),
      { initialProps: { sessionId, serverValue } },
    );

  it("draws the server value when nothing was PATCHed in this window", () => {
    expect(renderMerge("s1", "high").result.current.pinned).toBe("high");
    expect(renderMerge("s1", null).result.current.pinned).toBeNull();
    // Field absent (older daemon) — nothing pinned, and no crash.
    expect(renderMerge("s1", undefined).result.current.pinned).toBeNull();
  });

  it("keeps a fresh pin visible while the server echo is still stale", async () => {
    const view = renderMerge("s1", null);

    await act(async () => {
      view.result.current.setOptimistic("xhigh");
    });

    // The PATCH response has not invalidated the session record yet.
    expect(view.result.current.pinned).toBe("xhigh");

    // The echo catches up — same level, now server-driven.
    await act(async () => {
      view.rerender({ sessionId: "s1", serverValue: "xhigh" });
    });
    expect(view.result.current.pinned).toBe("xhigh");

    // And when the server later reports a different level (another client
    // changed it), the override is gone and the server is the authority.
    await act(async () => {
      view.rerender({ sessionId: "s1", serverValue: "low" });
    });
    expect(view.result.current.pinned).toBe("low");
  });

  it("keeps the pin visible on a daemon that never echoes the field", async () => {
    const view = renderMerge("s1", undefined);

    await act(async () => {
      view.result.current.setOptimistic("max");
    });
    expect(view.result.current.pinned).toBe("max");

    // Refetch still returns a record without the field: the local record of
    // the accepted PATCH is the only truth available, so it stays.
    await act(async () => {
      view.rerender({ sessionId: "s1", serverValue: undefined });
    });
    expect(view.result.current.pinned).toBe("max");
  });

  it("shows an unpin immediately, even against a stale pinned server value", async () => {
    const view = renderMerge("s1", "high");

    await act(async () => {
      view.result.current.setOptimistic(null);
    });
    expect(view.result.current.pinned).toBeNull();
  });

  it("drops the override when the session changes", async () => {
    const view = renderMerge("s1", null);

    await act(async () => {
      view.result.current.setOptimistic("high");
    });
    expect(view.result.current.pinned).toBe("high");

    await act(async () => {
      view.rerender({ sessionId: "s2", serverValue: null });
    });
    expect(view.result.current.pinned).toBeNull();
  });

  it("falls back to the server when a failed PATCH clears the override", async () => {
    const view = renderMerge("s1", null);

    await act(async () => {
      view.result.current.setOptimistic("high");
    });
    await act(async () => {
      view.result.current.clearOptimistic();
    });

    expect(view.result.current.pinned).toBeNull();
  });
});
