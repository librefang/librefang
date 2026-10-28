import { act, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import type { AgentItem } from "../api";
import { thinkingFrameField, useChatMessages } from "./ChatPage";

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

vi.mock("../lib/store", () => ({
  useUIStore: (selector: (s: Record<string, unknown>) => unknown) =>
    selector({
      addSkillOutput: vi.fn(),
      addToast: vi.fn(),
      deepThinking: false,
      showThinkingProcess: false,
    }),
}));

type Listener = (event: unknown) => void;

// `ChatPage` drives the socket through `addEventListener("message", …)` for turn
// frames and through the `onopen` / `onclose` properties for lifecycle, so the
// double is faithful to both. `close()` fires `onclose` synchronously, which
// models the real teardown accurately enough: the production cleanup nulls
// `ws.onclose` *before* calling `close()`, so on that path there is nothing left
// to fire — which is the whole point of these tests.
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
    // Fail fast instead of exhausting the heap: a render loop or a reconnect
    // storm shows up here first, and an OOM'd worker reports no test results at
    // all — it looks like the suite never ran rather than like a bug.
    if (MockWebSocket.instances.length > 50) {
      throw new Error("runaway WebSocket construction — the hook is reconnecting in a loop");
    }
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

  /** Drive the socket to OPEN, as a real server accepting the upgrade would. */
  emitOpen() {
    this.readyState = MockWebSocket.OPEN;
    this.onopen?.();
  }

  /** Number of turn-frame listeners still attached — a leak detector. */
  messageListenerCount() {
    return this.listeners.get("message")?.size ?? 0;
  }
}

function wrapper({ children }: { children: ReactNode }) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0, staleTime: 0 } },
  });
  return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>;
}

const AGENTS = [
  { id: "agent-a", name: "Agent A" },
  { id: "agent-b", name: "Agent B" },
] as AgentItem[];

/**
 * Render the hook on `agent-a`, bring its socket up, and send one message over it.
 * Returns the live socket so a test can assert on what the daemon received.
 */
async function sendOverSocket(agents: AgentItem[] = AGENTS) {
  const view = renderHook(
    ({ agent }: { agent: string }) => useChatMessages(agent, agents),
    { wrapper, initialProps: { agent: agents[0].id } },
  );

  const socket: MockWebSocket | undefined =
    MockWebSocket.instances[MockWebSocket.instances.length - 1];
  if (!socket) throw new Error("hook did not open a socket");
  await act(async () => {
    socket.emitOpen();
  });
  await waitFor(() => expect(view.result.current.wsConnected).toBe(true));

  await act(async () => {
    view.result.current.sendMessage("run the long job");
  });
  await waitFor(() => expect(view.result.current.isLoading).toBe(true));

  // The frame really did reach the daemon — this is what makes an HTTP retry a
  // duplicate execution rather than a recovery.
  expect(socket.sent.some((frame) => frame.includes("run the long job"))).toBe(true);

  return { ...view, socket };
}

describe("chat turn survives an agent switch", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    MockWebSocket.instances = [];
    vi.stubGlobal("WebSocket", MockWebSocket);
  });

  // The per-agent loading map is deliberate (#2322): switching away from a busy
  // agent must unblock the new one, and coming back must still show the original
  // as busy. The defect is that nothing ever finishes the turn — the WS effect
  // cleanup drops `onDropRef` without invoking it and nulls `ws.onclose`, so the
  // flag is raised by `sendMessage` and never lowered by anything.
  it("releases the agent's input after switching away mid-turn and back", async () => {
    const { result, rerender } = await sendOverSocket();

    await act(async () => {
      rerender({ agent: "agent-b" });
    });
    await act(async () => {
      rerender({ agent: "agent-a" });
    });

    expect(result.current.isLoading).toBe(false);
  });

  // The 180s watchdog armed for the turn is not cleared by the socket teardown,
  // so it eventually re-POSTs a message the daemon already accepted and is still
  // executing — two runs writing into one session's history.
  it("does not re-send the delivered message over HTTP after the socket is torn down", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const { rerender } = await sendOverSocket();
      sendAgentMessage.mockClear();

      await act(async () => {
        rerender({ agent: "agent-b" });
      });
      await act(async () => {
        vi.advanceTimersByTime(180_001);
      });

      expect(sendAgentMessage).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });

  // Navigating away from the chat unmounts the hook, which runs the same socket
  // teardown by a different route. The loading flag dies with the component there,
  // so the damage that outlives it is the armed watchdog: three minutes later it
  // re-POSTs a message the daemon already accepted, from a page nobody is looking at.
  it("does not re-send the delivered message after the chat is unmounted", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const { unmount } = await sendOverSocket();
      sendAgentMessage.mockClear();

      await act(async () => {
        unmount();
      });
      await act(async () => {
        vi.advanceTimersByTime(180_001);
      });

      expect(sendAgentMessage).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });

  // The turn's frame listener is attached to the socket being discarded. Leaving
  // it attached is how a previous turn's terminal frame reaches a dead socket.
  it("detaches the turn's frame listener from the socket it tears down", async () => {
    const { rerender, socket } = await sendOverSocket();
    expect(socket.messageListenerCount()).toBe(1);

    await act(async () => {
      rerender({ agent: "agent-b" });
    });

    expect(socket.messageListenerCount()).toBe(0);
  });
});

// Refs #8616: the chat page sent `thinking: deepThinking` on every frame, so
// an untouched toggle sent the legacy `false` and the kernel cleared the
// thinking config — erasing a per-session reasoning pin. Until the user moves
// the toggle the key is omitted and the kernel inherits.
describe("deep-thinking frame field", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    MockWebSocket.instances = [];
    vi.stubGlobal("WebSocket", MockWebSocket);
  });

  it("omits the legacy thinking key while the toggle is untouched", async () => {
    const { socket } = await sendOverSocket();
    const frames = socket.sent.map((frame) => JSON.parse(frame) as Record<string, unknown>);
    const turn = frames.find((frame) => frame.content === "run the long job");
    expect(turn).toBeDefined();
    expect(turn).not.toHaveProperty("thinking");
  });

  it("sends the boolean only once the user has toggled", () => {
    expect(thinkingFrameField(false, false)).toEqual({});
    expect(thinkingFrameField(false, true)).toEqual({ thinking: false });
    expect(thinkingFrameField(true, true)).toEqual({ thinking: true });
  });
});
