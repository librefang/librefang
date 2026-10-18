import { act, render, screen, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ChatPage } from "./ChatPage";
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
const router = vi.hoisted(() => ({
  navigate: vi.fn(),
  search: { agentId: "agent-a", sessionId: "s2" } as {
    agentId?: string;
    sessionId?: string;
  },
}));
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => router.navigate,
  useSearch: () => router.search,
}));

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
// no longer returns in full), which is the input behind the prune tests. The
// `dataUpdatedAt` stands in for React Query's fetch timestamp, which the
// session prune compares a tab's open time against (#8596).
const sessionList = vi.hoisted(() => ({
  data: [] as SessionListItem[],
  dataUpdatedAt: 0,
}));
// What the agents query would answer: `truncated` models a server page whose
// tail was cut off, which a prune must not read as "these agents are gone".
const agentList = vi.hoisted(() => ({
  data: [] as AgentItem[],
  truncated: false,
}));
// The credential running the page. `principal: null` is the synthetic root
// shape, so `whoamiPersonName` draws no person and rendering is unchanged
// from before; the role is what the agent prune gates on (#8596).
const whoami = vi.hoisted(() => ({
  data: undefined as { name: string; role: string; principal?: string | null } | undefined,
}));

vi.mock("../lib/queries/authz", async () => {
  const actual =
    await vi.importActual<typeof import("../lib/queries/authz")>("../lib/queries/authz");
  return { ...actual, useWhoami: () => ({ data: whoami.data }) };
});

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
  useAgents: () => ({
    data: agentList.data,
    truncated: agentList.truncated,
    refetch: vi.fn(),
    isFetching: false,
  }),
  useAgentSessions: () => ({
    data: sessionList.data,
    dataUpdatedAt: sessionList.dataUpdatedAt,
    refetch: vi.fn(),
  }),
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
  const view = render(
    <QueryClientProvider client={queryClient}>
      <ChatPage />
    </QueryClientProvider>,
  );
  // `rerender` shares this client: the #8596 session tests re-render the same
  // tree after moving the mocked search params, and a second client would
  // reset every query underneath the page.
  return { ...view, queryClient };
}

const tabLabels = (tablist: HTMLElement) =>
  within(tablist).getAllByRole("tab").map((tab) => tab.textContent);

describe("chat session tab strip order", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.stubGlobal("WebSocket", MockWebSocket);
    sessionList.data = SESSIONS;
    sessionList.dataUpdatedAt = 0;
    agentList.data = AGENTS;
    agentList.truncated = false;
    whoami.data = { name: "root", role: "owner", principal: null };
    router.search.agentId = "agent-a";
    router.search.sessionId = "s2";
    localStorage.removeItem("librefang.chat.show_hand_agents");
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

// Which set the page hands to `pruneChatTabAgents` is the whole question of
// #8596: the agents list cannot always testify that an agent is gone. The
// server owner-scopes it for every role below Admin, and past
// `AGENT_LIST_LIMIT` it is a page whose tail was cut off. These drive the
// page, not the store action — the store action may prune whatever set it is
// handed; deciding *when* it is allowed to is the page's business.
describe("chat tab agent prune (#8596)", () => {
  const realPruneChatTabAgents = useUIStore.getState().pruneChatTabAgents;

  beforeEach(() => {
    vi.clearAllMocks();
    vi.stubGlobal("WebSocket", MockWebSocket);
    sessionList.data = SESSIONS;
    sessionList.dataUpdatedAt = 0;
    agentList.data = AGENTS;
    agentList.truncated = false;
    whoami.data = { name: "root", role: "owner", principal: null };
    router.search.agentId = "agent-a";
    router.search.sessionId = "s2";
    // The agent prune only claims the list is authoritative when hand agents
    // are included: with them hidden the query returns a subset by design.
    localStorage.setItem("librefang.chat.show_hand_agents", "1");
    useUIStore.setState({
      openChatTabs: { "agent-a": ["s2"], "agent-gone": ["s9"] },
      chatTabRecency: { "agent-a": ["s2"], "agent-gone": ["s9"] },
      pruneChatTabAgents: realPruneChatTabAgents,
    });
  });

  afterEach(() => {
    localStorage.removeItem("librefang.chat.show_hand_agents");
    useUIStore.setState({ pruneChatTabAgents: realPruneChatTabAgents });
  });

  it("prunes agents missing from a complete, unscoped list (admin/owner)", async () => {
    const pruneSpy = vi.fn(realPruneChatTabAgents);
    useUIStore.setState({ pruneChatTabAgents: pruneSpy });
    renderPage();
    await screen.findByText("hello there");

    await vi.waitFor(() => expect(pruneSpy).toHaveBeenCalled());
    expect(pruneSpy).toHaveBeenCalledWith(new Set(["agent-a"]));
    expect(useUIStore.getState().openChatTabs).toEqual({ "agent-a": ["s2"] });
  });

  it("keeps tabs when the list is a truncated page", async () => {
    agentList.truncated = true;
    const pruneSpy = vi.fn(realPruneChatTabAgents);
    useUIStore.setState({ pruneChatTabAgents: pruneSpy });
    renderPage();
    await screen.findByText("hello there");

    // The missing agent may sit in the page's cut-off tail; a tab is cheap,
    // a silently dropped one is not.
    expect(pruneSpy).not.toHaveBeenCalled();
    expect(useUIStore.getState().openChatTabs).toEqual({
      "agent-a": ["s2"],
      "agent-gone": ["s9"],
    });
  });

  it("keeps tabs for a non-admin role, whose list is owner-scoped", async () => {
    whoami.data = { name: "root", role: "user", principal: null };
    const pruneSpy = vi.fn(realPruneChatTabAgents);
    useUIStore.setState({ pruneChatTabAgents: pruneSpy });
    renderPage();
    await screen.findByText("hello there");

    expect(pruneSpy).not.toHaveBeenCalled();
    expect(useUIStore.getState().openChatTabs).toEqual({
      "agent-a": ["s2"],
      "agent-gone": ["s9"],
    });
  });

  it("keeps tabs until whoami says which role is calling", async () => {
    whoami.data = undefined;
    const pruneSpy = vi.fn(realPruneChatTabAgents);
    useUIStore.setState({ pruneChatTabAgents: pruneSpy });
    renderPage();
    await screen.findByText("hello there");

    // Role unknown: a late prune is a stale tab, a wrong one is silent loss.
    expect(pruneSpy).not.toHaveBeenCalled();
  });
});

// The session half of #8596: the prune drops tabs the cached session list
// omits, but a list fetched before the tab was opened never had a chance to
// include it (sessions are created from other windows, the CLI, or the
// Sessions page). A tab opened against such a snapshot survives until a fetch
// that already knew about the visit answers; from then on the list is
// authoritative again.
describe("chat tab session prune (#8596)", () => {
  const realPruneChatTabs = useUIStore.getState().pruneChatTabs;
  const realPruneChatTabAgents = useUIStore.getState().pruneChatTabAgents;

  beforeEach(() => {
    vi.clearAllMocks();
    vi.stubGlobal("WebSocket", MockWebSocket);
    agentList.data = AGENTS;
    agentList.truncated = false;
    whoami.data = { name: "root", role: "owner", principal: null };
    router.search.agentId = "agent-a";
    localStorage.removeItem("librefang.chat.show_hand_agents");
    // Start on s3, a session the cached snapshot does not contain.
    router.search.sessionId = "s3";
    sessionList.data = SESSIONS.filter((session) => session.session_id !== "s3");
    sessionList.dataUpdatedAt = 1_000;
    useUIStore.setState({
      openChatTabs: { "agent-a": ["s1", "s2", "s3"] },
      chatTabRecency: {},
      pruneChatTabs: realPruneChatTabs,
    });
  });

  afterEach(() => {
    useUIStore.setState({
      pruneChatTabs: realPruneChatTabs,
      pruneChatTabAgents: realPruneChatTabAgents,
    });
  });

  it("keeps a tab opened after the cached list was fetched, until a later list omits it", async () => {
    const pruneSpy = vi.fn(realPruneChatTabs);
    useUIStore.setState({ pruneChatTabs: pruneSpy });
    const lastValid = () => {
      const calls = pruneSpy.mock.calls;
      return calls.length > 0 ? calls[calls.length - 1][1] : undefined;
    };

    const { rerender, queryClient } = renderPage();
    await screen.findByText("hello there");

    // The visit lands after the snapshot: s3 must be kept, not treated as a
    // session the server no longer has.
    await vi.waitFor(() => expect(lastValid()?.has("s3")).toBe(true));
    expect(useUIStore.getState().openChatTabs["agent-a"]).toEqual(["s1", "s2", "s3"]);

    // Switching away is the moment the old code dropped s3; the snapshot
    // still predates the visit, so it must survive this prune too.
    router.search.sessionId = "s2";
    rerender(
      <QueryClientProvider client={queryClient}>
        <ChatPage />
      </QueryClientProvider>,
    );
    await vi.waitFor(() => {
      const valid = lastValid();
      expect(valid?.has("s2")).toBe(true);
      expect(valid?.has("s3")).toBe(true);
    });
    expect(useUIStore.getState().openChatTabs["agent-a"]).toEqual(["s1", "s2", "s3"]);

    // A fresh snapshot that answered after the visit and still omits s3 is
    // the server's answer: now the tab goes.
    sessionList.dataUpdatedAt = Date.now() + 1_000;
    rerender(
      <QueryClientProvider client={queryClient}>
        <ChatPage />
      </QueryClientProvider>,
    );
    await vi.waitFor(() =>
      expect(useUIStore.getState().openChatTabs["agent-a"]).toEqual(["s1", "s2"]),
    );
  });
});
