import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ChatPage } from "./ChatPage";
import type { ReactNode } from "react";
import type { AgentItem } from "../api";

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

// The router hooks only need to say "we are on /chat with agent-a"; the page
// never renders a route tree in these tests.
const router = vi.hoisted(() => ({
  navigate: vi.fn(),
  search: { agentId: "agent-a" } as { agentId?: string; sessionId?: string },
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

vi.mock("../lib/queries/agents", () => ({
  agentQueries: {
    session: () => ({
      queryKey: ["agent-session", "agent-a", null],
      queryFn: () =>
        Promise.resolve({
          messages: [
            { role: "User", content: "hello there", timestamp: "2026-09-12T17:18:00Z" },
            { role: "Assistant", content: "hi from agent", timestamp: "2026-09-12T17:19:00Z" },
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
  useAgentSessions: () => ({ data: [], refetch: vi.fn() }),
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

// Rendering the real Markdown/asset/typewriter stack drags KaTeX and image
// auth into jsdom for a test about the export button. The transcript text is
// what matters here, so the leaf renderers stay trivial.
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

const uiState = {
  addSkillOutput: vi.fn(),
  addToast: vi.fn(),
  deepThinking: false,
  showThinkingProcess: false,
  setDeepThinking: vi.fn(),
  setShowThinkingProcess: vi.fn(),
  hiddenModelKeys: [] as string[],
  chatScale: 1,
  setChatScale: vi.fn(),
};
vi.mock("../lib/store", () => ({
  useUIStore: (selector: (state: Record<string, unknown>) => unknown) => selector(uiState),
  MIN_CHAT_SCALE: 0.75,
  MAX_CHAT_SCALE: 1.25,
  DEFAULT_CHAT_SCALE: 0.9,
  CHAT_SCALE_STEP: 0.05,
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

// Minimal WebSocket double: `useChatMessages` opens a socket on mount and the
// component only needs it to not explode. No frames are driven in this test.
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

describe("chat export from the selection bar", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.stubGlobal("WebSocket", MockWebSocket);
  });

  // Regression: the print path used `window.open("", "_blank",
  // "noopener,noreferrer")`, and by spec `window.open` returns null when
  // `noopener` is requested (browsers sever the link before returning). The
  // user got an unreachable blank tab and a "Allow pop-ups" toast even with
  // pop-ups allowed, and nothing ever printed. The open spy below models that
  // spec behaviour, so this test only passes if the page reaches and fills the
  // window it opened.
  it("fills and prints the selected transcript in a reachable window", async () => {
    type FakeWindow = Window & { print: ReturnType<typeof vi.fn> };
    const opened: FakeWindow[] = [];
    const openSpy = vi.spyOn(window, "open").mockImplementation(((url: string, target?: string, features?: string) => {
      expect(url).toBe("");
      expect(target).toBe("_blank");
      if (features?.includes("noopener") || features?.includes("noreferrer")) return null;
      const doc = document.implementation.createHTMLDocument("");
      const win = { document: doc, opener: window, print: vi.fn() } as unknown as FakeWindow;
      opened.push(win);
      return win;
    }) as typeof window.open);

    try {
      const user = userEvent.setup();
      renderPage();

      await screen.findByText("hello there");
      await user.click(screen.getByTitle("Pick messages to export or print"));
      await user.click(screen.getByRole("button", { name: "Print" }));

      expect(opened).toHaveLength(1);
      const win = opened[0];
      // Same-origin window, opener cut by hand — not by the features string.
      expect(win.opener).toBeNull();
      expect(win.print).toHaveBeenCalledTimes(1);
      const pre = win.document.querySelector("pre");
      expect(pre?.textContent).toContain("hello there");
      expect(pre?.textContent).toContain("hi from agent");
      expect(win.document.title).toMatch(/^Agent-A-\d{4}-\d{2}-\d{2}-\d{2}-\d{2}Z\.md$/);
    } finally {
      openSpy.mockRestore();
    }
  });
});
