// Tests for the redesigned MemoryPage (agent rail + tabs).
//
// Mocks at the queries/mutations hook layer per the dashboard data-layer
// rule: pages MUST go through `lib/queries` / `lib/mutations`, never
// `fetch()`. The router hooks are stubbed so the page can be rendered
// inside a plain `QueryClientProvider` (no MemoryRouter or RouterProvider
// needed for these unit tests).

import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryPage } from "./index";
import {
  useMemoryStats,
  useMemoryConfig,
  useMemoryHealth,
  useMemorySearchOrList,
} from "../../lib/queries/memory";
import { useAgents } from "../../lib/queries/agents";
import { useAutoDreamStatus } from "../../lib/queries/autoDream";
import {
  useAddMemory,
  useUpdateMemory,
  useDeleteMemory,
  useCleanupMemories,
  useUpdateMemoryConfig,
} from "../../lib/mutations/memory";
import {
  useTriggerAutoDream,
  useAbortAutoDream,
  useSetAutoDreamEnabled,
} from "../../lib/mutations/autoDream";

vi.mock("../../lib/queries/memory", () => ({
  useMemoryStats: vi.fn(),
  useMemoryConfig: vi.fn(),
  useMemoryHealth: vi.fn(),
  useMemorySearchOrList: vi.fn(),
  agentKvMemoryQueryOptions: vi.fn((agentId: string) => ({
    queryKey: ["memory", "agent-kv", agentId],
    queryFn: async () => [],
  })),
}));

vi.mock("../../lib/queries/agents", () => ({
  useAgents: vi.fn(),
}));

vi.mock("../../lib/queries/autoDream", () => ({
  useAutoDreamStatus: vi.fn(),
}));

vi.mock("../../lib/queries/models", () => ({
  useModels: vi.fn(() => ({ data: { models: [], total: 0, available: 0 }, isSuccess: true, isLoading: false })),
}));

vi.mock("../../lib/mutations/memory", () => ({
  useAddMemory: vi.fn(),
  useUpdateMemory: vi.fn(),
  useDeleteMemory: vi.fn(),
  useCleanupMemories: vi.fn(),
  useUpdateMemoryConfig: vi.fn(),
}));

vi.mock("../../lib/mutations/autoDream", () => ({
  useTriggerAutoDream: vi.fn(),
  useAbortAutoDream: vi.fn(),
  useSetAutoDreamEnabled: vi.fn(),
}));

vi.mock("../../lib/useCreateShortcut", () => ({
  useCreateShortcut: () => {},
}));

const addToastMock = vi.fn();
vi.mock("../../lib/store", () => ({
  useUIStore: (selector: (state: { addToast: typeof addToastMock }) => unknown) =>
    selector({ addToast: addToastMock }),
}));

// Mock the router hooks so the page can be rendered without a RouterProvider.
// Tests that need to assert navigation can read `navigateMock.mock.calls`.
const navigateMock = vi.fn();
let searchState: { agent?: string; tab?: string } = {};
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => navigateMock,
  useSearch: () => searchState,
}));

vi.mock("../../components/ui/DrawerPanel", () => ({
  DrawerPanel: ({
    isOpen,
    children,
    title,
  }: {
    isOpen: boolean;
    title?: React.ReactNode;
    children: React.ReactNode;
  }) =>
    isOpen ? (
      <div data-testid="drawer">
        <div>{title}</div>
        {children}
      </div>
    ) : null,
}));

vi.mock("../../components/ui/MarkdownContent", () => ({
  MarkdownContent: ({ children }: { children: React.ReactNode }) => (
    <div data-testid="markdown">{children}</div>
  ),
}));

vi.mock("motion/react", () => ({
  AnimatePresence: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  motion: new Proxy(
    {},
    {
      get: (_target: unknown, prop: string) =>
        ({ children, ...rest }: { children?: React.ReactNode } & Record<string, unknown>) =>
          React.createElement(prop, rest, children),
    },
  ),
}));

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>("react-i18next");
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, fallbackOrOpts?: unknown, maybeOpts?: unknown) => {
        const interp = (str: string, opts: unknown) => {
          if (opts && typeof opts === "object") {
            return Object.entries(opts as Record<string, unknown>).reduce<string>(
              (acc, [k, v]) => acc.replace(`{{${k}}}`, String(v)),
              str,
            );
          }
          return str;
        };
        if (typeof fallbackOrOpts === "string") {
          return interp(fallbackOrOpts, maybeOpts);
        }
        if (
          fallbackOrOpts &&
          typeof fallbackOrOpts === "object" &&
          "defaultValue" in (fallbackOrOpts as Record<string, unknown>)
        ) {
          const dv = (fallbackOrOpts as { defaultValue?: string }).defaultValue;
          if (typeof dv === "string") return interp(dv, fallbackOrOpts);
        }
        return key;
      },
      i18n: { language: "en" },
    }),
  };
});

const useMemoryStatsMock = useMemoryStats as unknown as ReturnType<typeof vi.fn>;
const useMemoryConfigMock = useMemoryConfig as unknown as ReturnType<typeof vi.fn>;
const useMemoryHealthMock = useMemoryHealth as unknown as ReturnType<typeof vi.fn>;
const useMemorySearchOrListMock = useMemorySearchOrList as unknown as ReturnType<typeof vi.fn>;
const useAgentsMock = useAgents as unknown as ReturnType<typeof vi.fn>;
const useAutoDreamStatusMock = useAutoDreamStatus as unknown as ReturnType<typeof vi.fn>;
const useAddMemoryMock = useAddMemory as unknown as ReturnType<typeof vi.fn>;
const useUpdateMemoryMock = useUpdateMemory as unknown as ReturnType<typeof vi.fn>;
const useDeleteMemoryMock = useDeleteMemory as unknown as ReturnType<typeof vi.fn>;
const useCleanupMemoriesMock = useCleanupMemories as unknown as ReturnType<typeof vi.fn>;
const useUpdateMemoryConfigMock = useUpdateMemoryConfig as unknown as ReturnType<typeof vi.fn>;
const useTriggerAutoDreamMock = useTriggerAutoDream as unknown as ReturnType<typeof vi.fn>;
const useAbortAutoDreamMock = useAbortAutoDream as unknown as ReturnType<typeof vi.fn>;
const useSetAutoDreamEnabledMock = useSetAutoDreamEnabled as unknown as ReturnType<typeof vi.fn>;

const STATS = { total: 7, user_count: 2, session_count: 3, agent_count: 2 };
const CONFIG = {
  embedding_provider: "openai",
  embedding_model: "text-embedding-3-small",
  embedding_api_key_env: "OPENAI_API_KEY",
  decay_rate: 0.05,
  proactive_memory: {
    enabled: true,
    auto_memorize: true,
    auto_retrieve: true,
    extraction_model: "gpt-4o-mini",
    max_retrieve: 10,
  },
};
const MEMORIES = [
  {
    id: "mem-aaaaaaaa",
    content: "remember to water the plants",
    level: "user",
    confidence: 0.9,
    created_at: "2025-01-01T00:00:00Z",
    accessed_at: "2025-01-02T00:00:00Z",
    access_count: 3,
    agent_id: "agent-1",
    category: "personal",
  },
  {
    id: "mem-bbbbbbbb",
    content: "session note",
    level: "session",
    confidence: 0.5,
    created_at: "2025-01-01T00:00:00Z",
  },
];

function renderPage(): void {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  render(
    <QueryClientProvider client={qc}>
      <MemoryPage />
    </QueryClientProvider>,
  );
}

describe("MemoryPage (redesigned)", () => {
  let addMutate: ReturnType<typeof vi.fn>;
  let updateMutate: ReturnType<typeof vi.fn>;
  let deleteMutate: ReturnType<typeof vi.fn>;
  let cleanupMutate: ReturnType<typeof vi.fn>;
  let configMutateAsync: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    vi.clearAllMocks();
    searchState = {};
    addMutate = vi.fn();
    updateMutate = vi.fn();
    deleteMutate = vi.fn();
    cleanupMutate = vi.fn();
    configMutateAsync = vi.fn().mockResolvedValue(undefined);

    useMemoryStatsMock.mockReturnValue({
      data: STATS,
      isLoading: false,
      isError: false,
    });
    useMemoryConfigMock.mockReturnValue({
      data: CONFIG,
      isLoading: false,
      isError: false,
    });
    useMemoryHealthMock.mockReturnValue({ data: true, isLoading: false });
    useMemorySearchOrListMock.mockReturnValue({
      data: { memories: MEMORIES, total: MEMORIES.length, proactive_enabled: true },
      isLoading: false,
      isError: false,
      isFetching: false,
      refetch: vi.fn(),
    });
    useAgentsMock.mockReturnValue({ data: [], isFetching: false, refetch: vi.fn() });
    useAutoDreamStatusMock.mockReturnValue({
      data: { enabled: false, agents: [] },
      isLoading: false,
      isError: false,
      refetch: vi.fn(),
    });
    useAddMemoryMock.mockReturnValue({ mutate: addMutate, isPending: false });
    useUpdateMemoryMock.mockReturnValue({ mutate: updateMutate, isPending: false });
    useDeleteMemoryMock.mockReturnValue({ mutate: deleteMutate, isPending: false });
    useCleanupMemoriesMock.mockReturnValue({ mutate: cleanupMutate, isPending: false });
    useUpdateMemoryConfigMock.mockReturnValue({
      mutateAsync: configMutateAsync,
      isPending: false,
    });
    useTriggerAutoDreamMock.mockReturnValue({ mutateAsync: vi.fn(), isPending: false });
    useAbortAutoDreamMock.mockReturnValue({ mutateAsync: vi.fn(), isPending: false });
    useSetAutoDreamEnabledMock.mockReturnValue({ mutateAsync: vi.fn(), isPending: false });
  });

  it("renders scope summary with totals from useMemoryStats", () => {
    renderPage();
    // STATS.total = 7 — rendered as the headline number in ScopeSummary
    // (uniquely "7"; user/session/agent counts of 2/3/2 share digits with
    // each other so assert on the unique total).
    expect(screen.getByText("7")).toBeInTheDocument();
    // session_count=3 is also unique among the breakdown chips.
    expect(screen.getByText("3")).toBeInTheDocument();
  });

  it("renders memory record cards by default (records tab)", () => {
    renderPage();
    expect(screen.getByText("mem-aaaaaaaa")).toBeInTheDocument();
    expect(screen.getByText("mem-bbbbbbbb")).toBeInTheDocument();
    expect(screen.getByText("remember to water the plants")).toBeInTheDocument();
  });

  it("switches to KV tab when the KV tab button is clicked", () => {
    renderPage();
    // The TabBar button's accessible name is exactly "KV"; the agent rail's
    // "All agents" row also contains "KV" in its subtitle ("0 mem · 0 KV"),
    // so match exactly.
    fireEvent.click(screen.getByRole("button", { name: "KV" }));
    expect(navigateMock).toHaveBeenCalled();
    const call = navigateMock.mock.calls[0][0];
    expect(typeof call.search).toBe("function");
    const next = call.search({});
    expect(next.tab).toBe("kv");
  });

  it("opens the Add Memory drawer and calls useAddMemory.mutate", async () => {
    renderPage();
    fireEvent.click(screen.getByRole("button", { name: /memory\.add/i }));
    const textarea = await screen.findByPlaceholderText("memory.content_placeholder");
    fireEvent.change(textarea, { target: { value: "new memory" } });
    const saveButtons = screen.getAllByRole("button", { name: /common\.save/ });
    fireEvent.click(saveButtons[saveButtons.length - 1]);
    await waitFor(() => {
      expect(addMutate).toHaveBeenCalledTimes(1);
    });
    expect(addMutate.mock.calls[0][0]).toEqual({
      content: "new memory",
      level: "session",
      agentId: undefined,
    });
  });

  it("opens the Settings (memory config) drawer when header settings button is clicked", () => {
    renderPage();
    fireEvent.click(screen.getByRole("button", { name: /Settings/i }));
    // i18n mock returns the defaultValue, so the drawer title renders as
    // "Memory Configuration".
    expect(screen.getByText("Memory Configuration")).toBeInTheDocument();
  });

  it("shows the proactive-disabled notice on the Records tab when proactive memory is off", () => {
    useMemorySearchOrListMock.mockReturnValue({
      data: { memories: [], total: 0, proactive_enabled: false },
      isLoading: false,
      isError: false,
      isFetching: false,
      refetch: vi.fn(),
    });
    useMemoryConfigMock.mockReturnValue({
      data: { ...CONFIG, proactive_memory: { ...CONFIG.proactive_memory, enabled: false } },
      isLoading: false,
      isError: false,
    });
    renderPage();
    expect(
      screen.getByText(
        /Proactive memory is disabled in config/i,
      ),
    ).toBeInTheDocument();
  });

  it("respects the tab search param — health tab renders config readout", () => {
    searchState = { tab: "health" };
    renderPage();
    // Health tab surfaces the embedding provider readout. Default values
    // from the i18n mock — not the key strings.
    expect(screen.getByText("Embedding backbone")).toBeInTheDocument();
  });
});
