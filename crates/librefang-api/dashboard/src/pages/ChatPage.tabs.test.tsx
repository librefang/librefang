import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ChatPage } from "./ChatPage";
import { clearChatSessionCacheForAgent } from "../lib/chatSessionCache";
import { useUIStore } from "../lib/store";
import type { ReactNode } from "react";
import type { AgentItem, SessionListItem } from "../api";

// `t` must be referentially stable: `useChatMessages` lists it in an effect
// dependency array, so a fresh lambda per render re-runs the loader forever.
const translation = {
  t: (key: string, opts?: Record<string, unknown>) =>
    typeof opts?.defaultValue === "string" ? opts.defaultValue : key,
};
vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>("react-i18next");
  return { ...actual, useTranslation: () => translation };
});

// Pinned to s2 so the tab strip, the dropdown and the active highlight all
// agree on which conversation is being viewed.
//
// The search params are an external store rather than a frozen object: the
// a11y tests drive the strip with the keyboard and must see the active tab
// actually move, and the page only learns about a tab switch through
// `navigate()`. A static mock would leave `aria-selected` forever on s2 and
// the assertions would pass against a strip that ignores the arrow keys.
const router = vi.hoisted(() => {
  type Search = { agentId?: string; sessionId?: string };
  let snapshot: Search = { agentId: "agent-a", sessionId: "s2" };
  const listeners = new Set<() => void>();
  const notify = () => {
    for (const listener of listeners) listener();
  };
  return {
    navigate: vi.fn((options: { search?: Search }) => {
      if (options?.search) {
        snapshot = { ...options.search };
        notify();
      }
    }),
    subscribe: (listener: () => void) => {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    getSnapshot: () => snapshot,
    reset: () => {
      snapshot = { agentId: "agent-a", sessionId: "s2" };
      notify();
    },
  };
});
vi.mock("@tanstack/react-router", async () => {
  const React = await vi.importActual<typeof import("react")>("react");
  return {
    useNavigate: () => router.navigate,
    useSearch: () =>
      React.useSyncExternalStore(router.subscribe, router.getSnapshot, router.getSnapshot),
  };
});

vi.mock("../lib/queries/commands", () => ({
  useChatCommands: () => ({ data: [], isPending: false }),
}));

vi.mock("../lib/mutations/agents", () => ({
  useCreateAgentSession: () => ({ mutateAsync: vi.fn(), isPending: false }),
  useDeleteAgentSession: () => ({ mutateAsync: vi.fn(), isPending: false }),
  usePatchAgentRuntimeConfig: () => ({ mutateAsync: vi.fn(), isPending: false }),
  useResolveApproval: () => ({ mutateAsync: vi.fn(), isPending: false }),
  useSendAgentMessage: () => ({ mutateAsync: vi.fn(), isPending: false }),
  useStopAgent: () => ({ mutateAsync: vi.fn(), isPending: false }),
  useUploadAgentFile: () => ({ mutateAsync: vi.fn(), isPending: false }),
}));

const AGENTS = [{ id: "agent-a", name: "Agent A" }] as AgentItem[];
const SESSIONS: SessionListItem[] = [
  { session_id: "s1", agent_id: "agent-a", label: "One", created_at: "2026-09-12T17:00:00Z" },
  { session_id: "s2", agent_id: "agent-a", label: "Two", created_at: "2026-09-12T17:05:00Z" },
  { session_id: "s3", agent_id: "agent-a", label: "Three", created_at: "2026-09-12T17:10:00Z" },
];

// Mutable so individual tests can serve a stale session list (one the server
// no longer returns in full), which is the input behind the prune tests.
const sessionList = vi.hoisted(() => ({ data: [] as SessionListItem[] }));

vi.mock("../lib/queries/agents", () => ({
  agentQueries: {
    session: () => ({
      queryKey: ["agent-session", "agent-a", "s2"],
      queryFn: () =>
        Promise.resolve({
          messages: [
            { role: "User", content: "hello there", timestamp: "2026-09-12T17:18:00Z" },
          ],
        }),
    }),
    sessionContext: () => ({
      queryKey: ["ctx"],
      queryFn: () =>
        Promise.resolve({ used_tokens: 10, max_context_tokens: 100, pct: 10, pressure: "low" }),
    }),
  },
  useAgents: () => ({ data: AGENTS, refetch: vi.fn(), isFetching: false }),
  useAgentSessions: () => ({ data: sessionList.data, refetch: vi.fn() }),
  // `ChatPage` resolves the agent avatar through this hook since main grew the
  // avatar picker (#8371). This test neither asserts nor renders avatars, so a
  // stub that reports "no avatar" keeps the component mountable.
  useAgentAvatarUrl: () => undefined,
}));

vi.mock("../lib/queries/config", () => ({
  useFullConfig: () => ({ data: {}, isSuccess: true }),
}));

vi.mock("../lib/queries/media", () => ({
  useMediaProviders: () => ({ data: [] }),
}));

vi.mock("../lib/queries/models", () => ({
  useModels: () => ({ data: { models: [] }, isLoading: false, isFetching: false, error: null, refetch: vi.fn() }),
}));

vi.mock("../lib/queries/approvals", () => ({
  usePendingApprovals: () => ({ data: [] }),
}));

vi.mock("../lib/queries/sessions", () => ({
  useSessionStream: () => ({ events: [], isAttached: false, lastError: null }),
}));

vi.mock("../lib/queries/hands", () => ({
  useActiveHandsWhen: () => ({ data: [] }),
}));

vi.mock("../lib/tts", () => ({
  useTtsManager: () => ({
    speakingMessageId: null,
    status: "idle",
    error: null,
    toggle: vi.fn(),
    stop: vi.fn(),
    clearCache: vi.fn(),
  }),
}));

vi.mock("../lib/useVoiceInput", () => ({
  useVoiceInput: () => ({
    isRecording: false,
    isTranscribing: false,
    isSupported: false,
    startRecording: vi.fn(),
    stopRecording: vi.fn(),
    toggleRecording: vi.fn(),
  }),
}));

vi.mock("../lib/hooks/useMathPlugins", () => ({
  useMathPlugins: () => ({ remarkPlugins: [], rehypePlugins: [] }),
}));

vi.mock("../components/ui/MarkdownContent", () => ({
  MarkdownContent: ({ children }: { children?: ReactNode }) => <div>{children}</div>,
}));
vi.mock("../components/Typewriter_v2", () => ({
  Typewriter_v2: ({ text }: { text: string }) => <span>{text}</span>,
}));
vi.mock("../components/AuthenticatedImage", () => ({
  AuthenticatedImage: () => null,
}));
vi.mock("../components/ui/ToolCallsPanel", () => ({
  ToolCallsPanel: () => null,
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

class MockWebSocket {
  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSED = 3;
  readyState = MockWebSocket.CONNECTING;
  onopen: (() => void) | null = null;
  onclose: ((event: { code: number }) => void) | null = null;
  onerror: (() => void) | null = null;
  send() {}
  close() {
    this.readyState = MockWebSocket.CLOSED;
  }
  addEventListener() {}
  removeEventListener() {}
}

function renderPage() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0, staleTime: 0 } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <ChatPage />
    </QueryClientProvider>,
  );
}

const tabLabels = (tablist: HTMLElement) =>
  within(tablist).getAllByRole("tab").map((tab) => tab.textContent);

describe("chat session tab strip order", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.stubGlobal("WebSocket", MockWebSocket);
    sessionList.data = SESSIONS;
    router.reset();
    clearChatSessionCacheForAgent("agent-a");
    // The real store: this test is about the order the component renders from
    // `openChatTabs`, which is exactly the array the page subscribes to.
    useUIStore.setState({
      openChatTabs: { "agent-a": ["s1", "s2", "s3"] },
      chatTabRecency: {},
    });
  });

  it("keeps the rendered order when an existing tab is revisited", async () => {
    renderPage();
    await screen.findByText("hello there");

    const tablist = screen.getByRole("tablist", { name: "Open conversations" });
    expect(tabLabels(tablist)).toEqual(["One", "Two", "Three"]);

    // What the page does on every active-session change: visit a session that
    // already has a tab. The strip maps straight over `openChatTabs` in order,
    // so the DOM must not move — the previous LRU reorder turned this into
    // ["Two", "Three", "One"] and the tabs rotated under the cursor.
    act(() => {
      useUIStore.getState().openChatTab("agent-a", "s1");
    });

    expect(tabLabels(tablist)).toEqual(["One", "Two", "Three"]);
  });

  it("keeps the active session in the set handed to pruneChatTabs", async () => {
    // The cached list is stale: it was fetched before the session in the URL
    // existed (created in another window, or reached from the Sessions page).
    // Pruning with it alone would drop the tab of the session on screen; the
    // open effect would have to append it back at the end, moving the active
    // tab under the cursor.
    sessionList.data = SESSIONS.filter((session) => session.session_id !== "s2");
    useUIStore.setState({
      openChatTabs: { "agent-a": ["s1", "s2", "s3", "deleted"] },
      chatTabRecency: {},
    });
    const realPrune = useUIStore.getState().pruneChatTabs;
    const pruneSpy = vi.fn(realPrune);
    useUIStore.setState({ pruneChatTabs: pruneSpy });

    renderPage();
    await screen.findByText("hello there");
    await vi.waitFor(() => expect(pruneSpy).toHaveBeenCalled());

    // The active session is in the valid set even though the list omits it…
    const validSets = pruneSpy.mock.calls.map(([, valid]) => valid as Set<string>);
    expect(validSets[validSets.length - 1].has("s2")).toBe(true);
    // …while a session the list genuinely no longer has is still pruned, and
    // the strip keeps its order. The active tab falls back to its id as the
    // label because the stale list cannot name it.
    const tablist = screen.getByRole("tablist", { name: "Open conversations" });
    await vi.waitFor(() => expect(tabLabels(tablist)).toEqual(["One", "s2", "Three"]));

    useUIStore.setState({ pruneChatTabs: realPrune });
  });

  it("re-adds the active tab after a cross-window rehydrate removes it", async () => {
    renderPage();
    await screen.findByText("hello there");
    const tablist = screen.getByRole("tablist", { name: "Open conversations" });
    expect(tabLabels(tablist)).toEqual(["One", "Two", "Three"]);

    // The other window closed (or evicted) this session's tab and wrote the
    // record; the `storage` listener rehydrates it here, replacing the tab
    // list under the session on screen. Nothing else would re-add it until
    // the next navigation.
    localStorage.setItem(
      "librefang-ui-storage",
      JSON.stringify({ version: 1, state: { openChatTabs: { "agent-a": ["s1", "s3"] } } }),
    );
    await act(async () => {
      await useUIStore.persist.rehydrate();
    });

    // Appended, not inserted at its old position: the strip preserves the
    // order tabs were last opened in.
    await vi.waitFor(() => expect(tabLabels(tablist)).toEqual(["One", "Three", "Two"]));
  });
});

// Issue #8597 — the strip carries `role="tablist"` but none of the tab
// semantics behind it: no panel linkage, no roving tabindex, no arrow keys,
// the new-session button inside the list, and a close target under 24 px.
describe("chat session tab strip semantics and keyboard model", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.stubGlobal("WebSocket", MockWebSocket);
    sessionList.data = SESSIONS;
    router.reset();
    // The session cache is module-level and survives tests. Keyboard tests
    // switch sessions faster than the mocked fetch resolves, so a test can
    // unmount with an empty array stored for s2 — which the hook then serves
    // as a cache hit, hiding the transcript for every later test.
    clearChatSessionCacheForAgent("agent-a");
    useUIStore.setState({
      openChatTabs: { "agent-a": ["s1", "s2", "s3"] },
      chatTabRecency: {},
    });
  });

  async function renderStrip() {
    renderPage();
    await screen.findByText("hello there");
    return screen.getByRole("tablist", { name: "Open conversations" });
  }

  const tabs = (tablist: HTMLElement) => within(tablist).getAllByRole("tab");

  it("links every tab to the message-area tabpanel", async () => {
    const tablist = await renderStrip();
    const all = tabs(tablist);
    expect(all).toHaveLength(3);

    const panel = screen.getByRole("tabpanel");
    expect(panel).toBe(screen.getByTestId("chat-message-area"));
    expect(panel.id).not.toBe("");
    for (const tab of all) {
      expect(tab).toHaveAttribute("aria-controls", panel.id);
      expect(tab.id).not.toBe("");
    }

    // The panel is labelled by the tab that is actually selected, so a screen
    // reader reading the message area names the conversation it belongs to.
    const selected = all.find((tab) => tab.getAttribute("aria-selected") === "true");
    expect(selected).toBeDefined();
    expect(panel).toHaveAttribute("aria-labelledby", selected!.id);
  });

  it("keeps invalid children out of the tablist: tabs only, no + or close", async () => {
    const tablist = await renderStrip();
    for (const child of Array.from(tablist.children)) {
      expect(child).toHaveAttribute("role", "tab");
    }
    const newSession = screen.getByRole("button", { name: "New session" });
    expect(newSession.closest('[role="tablist"]')).toBeNull();
  });

  it("roves the tabindex so only the selected tab is in the tab order", async () => {
    const tablist = await renderStrip();
    const all = tabs(tablist);
    const ordered = all.filter((tab) => tab.tabIndex === 0);
    expect(ordered).toHaveLength(1);
    expect(ordered[0]).toHaveAttribute("aria-selected", "true");
    for (const tab of all) {
      if (tab !== ordered[0]) expect(tab.tabIndex).toBe(-1);
    }
  });

  it("activates the neighbour with ArrowRight and ArrowLeft in one step", async () => {
    const tablist = await renderStrip();
    const all = tabs(tablist); // One, Two (active), Three
    all[1].focus();
    expect(document.activeElement).toBe(all[1]);

    fireEvent.keyDown(all[1], { key: "ArrowRight" });
    await vi.waitFor(() => {
      expect(tabs(tablist)[2]).toHaveAttribute("aria-selected", "true");
    });
    expect(document.activeElement).toBe(tabs(tablist)[2]);
    expect(tabs(tablist)[2].tabIndex).toBe(0);
    expect(tabs(tablist)[1].tabIndex).toBe(-1);
    expect(router.navigate).toHaveBeenCalledWith(
      expect.objectContaining({ search: expect.objectContaining({ sessionId: "s3" }) }),
    );

    fireEvent.keyDown(document.activeElement!, { key: "ArrowLeft" });
    await vi.waitFor(() => {
      expect(tabs(tablist)[1]).toHaveAttribute("aria-selected", "true");
    });
    expect(document.activeElement).toBe(tabs(tablist)[1]);
  });

  it("jumps to the edges with Home and End", async () => {
    const tablist = await renderStrip();
    const all = tabs(tablist);
    all[1].focus();

    fireEvent.keyDown(all[1], { key: "End" });
    await vi.waitFor(() => {
      expect(tabs(tablist)[2]).toHaveAttribute("aria-selected", "true");
    });
    expect(document.activeElement).toBe(tabs(tablist)[2]);

    fireEvent.keyDown(document.activeElement!, { key: "Home" });
    await vi.waitFor(() => {
      expect(tabs(tablist)[0]).toHaveAttribute("aria-selected", "true");
    });
    expect(document.activeElement).toBe(tabs(tablist)[0]);
  });

  it("gives the close button a 24x24 hit target and keeps it out of the tab order", async () => {
    const tablist = await renderStrip();
    const closes = within(tablist).getAllByRole("button", { name: "Close this tab" });
    expect(closes).toHaveLength(3);
    for (const close of closes) {
      expect(close.tabIndex).toBe(-1);
      // h-6/w-6 = 1.5rem = 24px, the WCAG 2.5.8 minimum. The icon stays 10 px.
      expect(close).toHaveClass("h-6", "w-6");
    }
  });

  it("keeps the message area a plain region when a lone conversation has no strip", async () => {
    useUIStore.setState({ openChatTabs: { "agent-a": ["s2"] }, chatTabRecency: {} });
    renderPage();
    await screen.findByText("hello there");
    // A tabpanel without a tablist would be a dangling reference; the panel
    // semantics only switch on with the strip.
    expect(screen.queryByRole("tablist")).toBeNull();
    expect(screen.queryByRole("tabpanel")).toBeNull();
    expect(screen.getByTestId("chat-message-area")).not.toHaveAttribute("role");
  });

  it("closes the selected tab with Delete and hands focus to the neighbour", async () => {
    const tablist = await renderStrip();
    const all = tabs(tablist);
    all[1].focus();

    fireEvent.keyDown(all[1], { key: "Delete" });
    await vi.waitFor(() => expect(tabLabels(tablist)).toEqual(["One", "Three"]));
    // Closing from the keyboard lands on the neighbour that took the tab's
    // place, so the tab key path never drops the user on <body>.
    expect(tabs(tablist)[1]).toHaveAttribute("aria-selected", "true");
    expect(document.activeElement).toBe(tabs(tablist)[1]);
  });
});
