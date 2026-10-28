import { act, fireEvent, render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useState, type ReactNode } from "react";
import { ChatPage } from "./ChatPage";
import type { AgentItem, ModelItem, SessionListItem } from "../api";

// The composer's level list is only useful if it describes the model the
// session actually runs on. Two page-level contracts live here: the session's
// `model_override` outranks the agent's model, and a failed pin is reported
// only on the session that failed.

const translation = {
  t: (key: string, opts?: Record<string, unknown>) =>
    typeof opts?.defaultValue === "string" ? opts.defaultValue : key,
};
vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>("react-i18next");
  return { ...actual, useTranslation: () => translation };
});

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

const modelsState = vi.hoisted(() => ({ models: [] as ModelItem[] }));
vi.mock("../lib/queries/models", () => ({
  useModels: () => ({
    data: { models: modelsState.models, total: modelsState.models.length, available: modelsState.models.length },
    isLoading: false,
    isFetching: false,
    error: null,
    refetch: vi.fn(),
  }),
}));

const sessionDetailState = vi.hoisted(() => ({
  data: undefined as { model_override?: string | null; reasoning_effort?: string | null } | undefined,
}));
vi.mock("../lib/queries/sessions", () => ({
  useSessionStream: () => ({ events: [], isAttached: false, lastError: null }),
  useSessionDetails: () => ({ data: sessionDetailState.data, isPending: false }),
}));

const reasoningMutation = vi.hoisted(() => ({
  isError: false,
  isPending: false,
  variables: undefined as { sessionId?: string } | undefined,
  mutate: vi.fn(),
  // The page resets the mutation when the session changes; the stub has to
  // clear its own error state or the reset would be invisible to the test.
  reset: vi.fn(() => {
    reasoningMutation.isError = false;
    reasoningMutation.variables = undefined;
  }),
}));
vi.mock("../lib/mutations/sessions", async () => {
  const actual = await vi.importActual<typeof import("../lib/mutations/sessions")>(
    "../lib/mutations/sessions",
  );
  return { ...actual, useSetSessionReasoning: () => reasoningMutation };
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

const AGENTS = [
  {
    id: "agent-a",
    name: "Agent A",
    model_name: "agent-model",
    model_provider: "anthropic",
    supports_thinking: true,
  },
] as AgentItem[];
const SESSIONS: SessionListItem[] = [
  { session_id: "s1", agent_id: "agent-a", label: "One", created_at: "2026-09-12T17:00:00Z" },
  { session_id: "s2", agent_id: "agent-a", label: "Two", created_at: "2026-09-12T17:05:00Z" },
];

vi.mock("../lib/queries/agents", () => ({
  agentQueries: {
    session: () => ({
      queryKey: ["agent-session", "agent-a"],
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
  useAgentAvatarUrl: () => undefined,
}));

vi.mock("../lib/queries/config", () => ({
  useFullConfig: () => ({ data: {}, isSuccess: true }),
}));

vi.mock("../lib/queries/media", () => ({
  useMediaProviders: () => ({ data: [] }),
}));

vi.mock("../lib/queries/approvals", () => ({
  usePendingApprovals: () => ({ data: [] }),
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

function renderPage(children: ReactNode = <ChatPage />) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0, staleTime: 0 } },
  });
  return render(<QueryClientProvider client={queryClient}>{children}</QueryClientProvider>);
}

async function openLevelMenu() {
  await screen.findByText("hello there");
  fireEvent.click(screen.getByTestId("reasoning-effort-button"));
}

function offeredLevels(): string[] {
  return screen
    .getAllByRole("menuitemradio")
    .map((row) => row.textContent ?? "")
    .filter((label) => label !== "chat.reasoning_default");
}

describe("chat reasoning levels follow the session's model", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.stubGlobal("WebSocket", MockWebSocket);
    modelsState.models = [
      { provider: "anthropic", id: "agent-model", supported_efforts: ["low", "max"] },
      { provider: "openai", id: "gpt-5", supported_efforts: ["minimal", "high"] },
    ];
    sessionDetailState.data = undefined;
    reasoningMutation.isError = false;
    reasoningMutation.variables = undefined;
    reasoningMutation.mutate = vi.fn();
    router.search = { agentId: "agent-a", sessionId: "s2" };
  });

  it("uses the session model_override's levels, split as provider/model", async () => {
    // `model_override` is stored the way the backend accepts it.
    sessionDetailState.data = { model_override: "openai/gpt-5" };
    renderPage();
    await openLevelMenu();

    // Offered levels are gpt-5's, not the agent model's (low/max).
    expect(offeredLevels()).toEqual(["None", "Minimal", "High"]);
    expect(screen.queryByRole("menuitemradio", { name: "Low" })).toBeNull();
    expect(screen.queryByRole("menuitemradio", { name: "Max" })).toBeNull();
  });

  it("falls back to the agent's model when the session has no override", async () => {
    sessionDetailState.data = { model_override: null };
    renderPage();
    await openLevelMenu();

    expect(offeredLevels()).toEqual(["None", "Low", "Max"]);
    expect(screen.queryByRole("menuitemradio", { name: "Minimal" })).toBeNull();
  });

  // A bare override carries no provider. The catalog can hold the same id
  // under two providers with different level sets, so the agent's provider
  // decides which entry the level list comes from. Here the wrong choice
  // (first id match) is the openai row, which the order makes tempting.
  it("keeps the agent's provider for a bare model_override", async () => {
    modelsState.models = [
      { provider: "openai", id: "shared-model", supported_efforts: ["minimal", "high"] },
      { provider: "anthropic", id: "shared-model", supported_efforts: ["low", "max"] },
    ];
    sessionDetailState.data = { model_override: "shared-model" };
    renderPage();
    await openLevelMenu();

    expect(offeredLevels()).toEqual(["None", "Low", "Max"]);
  });
});

describe("chat reasoning pin failures", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.stubGlobal("WebSocket", MockWebSocket);
    modelsState.models = [
      { provider: "anthropic", id: "agent-model", supported_efforts: ["none", "low", "high"] },
    ];
    sessionDetailState.data = { reasoning_effort: null };
    reasoningMutation.isError = false;
    reasoningMutation.variables = undefined;
    reasoningMutation.mutate = vi.fn();
    router.search = { agentId: "agent-a", sessionId: "s2" };
  });

  it("resets a failed pin on session change so it does not return to that session", async () => {
    function SessionSwitch() {
      const [sessionId, setSessionId] = useState("s2");
      const [, bump] = useState(0);
      // The router double reads this object on every render, like the real
      // `useSearch()` does after a navigation.
      router.search = { agentId: "agent-a", sessionId };
      return (
        <>
          <button type="button" onClick={() => setSessionId("s1")}>
            switch-session
          </button>
          <button type="button" onClick={() => setSessionId("s2")}>
            return-session
          </button>
          {/* The failure arrives after mount, as it does in production; the
              bump is the mutation result changing under a live component. */}
          <button
            type="button"
            onClick={() => {
              reasoningMutation.isError = true;
              reasoningMutation.variables = { sessionId: "s2" };
              bump((n) => n + 1);
            }}
          >
            fail-pin
          </button>
          <ChatPage />
        </>
      );
    }

    renderPage(<SessionSwitch />);
    await openLevelMenu();

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "fail-pin" }));
    });
    expect(screen.getByText("chat.reasoning_pin_failed")).toBeInTheDocument();

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "switch-session" }));
    });
    expect(screen.queryByText("chat.reasoning_pin_failed")).toBeNull();

    // Back on the session the PATCH failed for: the reset was a real one, not
    // a render-time hide that the return would undo.
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "return-session" }));
    });
    expect(screen.queryByText("chat.reasoning_pin_failed")).toBeNull();
    expect(reasoningMutation.reset).toHaveBeenCalled();
    // The menu itself stays usable on both sessions.
    expect(screen.getByTestId("reasoning-effort-button")).toBeInTheDocument();
  });
});
