import { act, render, screen, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
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
  useAgentSessions: () => ({ data: SESSIONS, refetch: vi.fn() }),
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
});
