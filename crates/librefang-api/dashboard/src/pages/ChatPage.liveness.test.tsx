import { act, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import type { AgentItem } from "../api";
import { useChatMessages } from "./ChatPage";

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
  useSendAgentMessage: () => ({
    mutateAsync: vi.fn().mockResolvedValue({ response: "http answer" }),
    isPending: false,
  }),
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

/**
 * A socket that has gone half-open: still `OPEN`, still accepting writes that go
 * nowhere, and — the defining trait — it never fires `onclose` on its own.
 *
 * That last part is the whole point. `close()` fires `onclose` because a browser
 * does when *you* close a socket; nothing in this double ever fires it
 * spontaneously, because a dead link never tells the browser it died.
 */
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
    // Fail fast instead of exhausting the heap: a reconnect storm shows up here
    // first, and an OOM'd worker reports no test results at all — it looks like
    // the suite never ran rather than like a bug.
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

  emitOpen() {
    this.readyState = MockWebSocket.OPEN;
    this.onopen?.();
  }

  /** Deliver a server frame to every `message` listener the hook has attached. */
  emitMessage(data: unknown) {
    const event = { data: JSON.stringify(data) };
    for (const fn of this.listeners.get("message") ?? []) fn(event);
  }

  /** Every frame this socket was asked to write, as parsed message types. */
  sentTypes(): string[] {
    return this.sent.map((frame) => {
      try {
        return String((JSON.parse(frame) as { type?: unknown }).type ?? "");
      } catch {
        return "";
      }
    });
  }
}

function wrapper({ children }: { children: ReactNode }) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0, staleTime: 0 } },
  });
  return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>;
}

const AGENTS = [{ id: "agent-a", name: "Agent A" }] as AgentItem[];

/** Longer than any probe deadline the hook could reasonably use. */
const PAST_ANY_PROBE_DEADLINE = 30_000;

function setVisibility(state: "hidden" | "visible") {
  Object.defineProperty(document, "visibilityState", { configurable: true, value: state });
  document.dispatchEvent(new Event("visibilitychange"));
}

/** Render the hook on `agent-a` and bring its socket up, as a real upgrade would. */
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

/** Leave the tab and come back, with the link having died silently in between. */
async function leaveAndReturn() {
  await act(async () => {
    setVisibility("hidden");
  });
  await act(async () => {
    setVisibility("visible");
  });
}

describe("chat socket liveness when the tab comes back", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    MockWebSocket.instances = [];
    vi.stubGlobal("WebSocket", MockWebSocket);
    setVisibility("visible");
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  // The control. If the harness could not open a socket or drive it to OPEN,
  // every assertion below would fail for that reason and read exactly like a
  // missing probe.
  it("opens a socket that reports itself connected", async () => {
    const { result, socket } = await connected();
    expect(result.current.wsConnected).toBe(true);
    expect(socket.readyState).toBe(MockWebSocket.OPEN);
  });

  // The defect. `wakeUp` returns early unless `gaveUpRef` is set, and that flag is
  // only ever set inside `ws.onclose` after the retries run out — an event a
  // half-open socket never fires. So the one case the listener exists for is the
  // one case it cannot act on, and `readyState` keeps saying OPEN.
  it("probes the socket when the tab becomes visible again", async () => {
    const { socket } = await connected();

    await leaveAndReturn();

    expect(socket.sentTypes()).toContain("ping");
  });

  // Sending the probe is only useful if an unanswered one ends the connection:
  // closing it is what hands the turn to the reconnect path that already exists.
  it("closes a socket that never answers the probe", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { socket } = await connected();

    await leaveAndReturn();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(PAST_ANY_PROBE_DEADLINE);
    });

    expect(socket.readyState).toBe(MockWebSocket.CLOSED);
  });

  // Closing is only the handover, not the outcome the user cares about. Assert the
  // rest of the chain actually runs: `onclose` feeds the existing backoff
  // reconnect, so the tab ends up on a working socket rather than merely a
  // truthfully-closed one.
  it("reconnects after discarding a socket that failed the probe", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    await connected();
    expect(MockWebSocket.instances).toHaveLength(1);

    await leaveAndReturn();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(PAST_ANY_PROBE_DEADLINE);
    });

    expect(MockWebSocket.instances.length).toBeGreaterThan(1);
  });

  // The discriminator against a fix that just drops sockets on every tab switch.
  // This must pass both before and after the change, so the two tests above
  // cannot be made green by closing the connection unconditionally.
  it("keeps a socket that answers the probe", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { socket } = await connected();

    await leaveAndReturn();
    await act(async () => {
      socket.emitMessage({ type: "pong" });
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(PAST_ANY_PROBE_DEADLINE);
    });

    expect(socket.readyState).toBe(MockWebSocket.OPEN);
  });

  // The guard reads `onDropRef`, which is the turn-in-flight marker rather than a
  // flag of its own. If a normally-completed turn ever left it set, the guard
  // would latch on and no socket would be probed again for the life of the page —
  // the fix would be inert and the symptom identical. This is the user's actual
  // sequence: ask, get answered, leave the window, come back.
  it("probes again after a turn has completed normally", async () => {
    const { result, socket } = await connected();
    await act(async () => {
      result.current.sendMessage("what is the status");
    });
    await waitFor(() => expect(result.current.isLoading).toBe(true));

    await act(async () => {
      socket.emitMessage({ type: "response", content: "all good" });
    });
    await waitFor(() => expect(result.current.isLoading).toBe(false));

    await leaveAndReturn();

    expect(socket.sentTypes()).toContain("ping");
  });

  // `ws.rs:1492` awaits the whole agent turn inside the main loop, so while a turn
  // runs the daemon is not reading the socket and cannot answer a probe. Probing
  // there would time out on a healthy connection and close a live chat, so the
  // turn's own 180 s watchdog owns that window instead.
  it("does not probe while a turn is in flight", async () => {
    const { result, socket } = await connected();
    await act(async () => {
      result.current.sendMessage("run the long job");
    });
    await waitFor(() => expect(result.current.isLoading).toBe(true));
    const beforeReturn = socket.sentTypes().filter((type) => type === "ping").length;

    await leaveAndReturn();

    expect(socket.sentTypes().filter((type) => type === "ping").length).toBe(beforeReturn);
  });
});
