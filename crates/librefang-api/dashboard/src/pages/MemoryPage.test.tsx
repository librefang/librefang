// Tests for MemoryPage (refs #3853 — pages/ test gap).
//
// Mocks at the queries/mutations hook layer per the dashboard data-layer rule:
// pages MUST go through `lib/queries` / `lib/mutations`, never `fetch()`. We
// mock those hooks here and assert the page renders / wires mutations
// correctly — same convention as UserBudgetPage.test.tsx.

import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryPage } from "./MemoryPage";
import {
  useMemoryStats,
  useMemoryConfig,
  useMemoryHealth,
  useMemorySearchOrList,
} from "../lib/queries/memory";
import { useAgents } from "../lib/queries/agents";
import {
  useAddMemory,
  useUpdateMemory,
  useDeleteMemory,
  useCleanupMemories,
  useUpdateMemoryConfig,
} from "../lib/mutations/memory";

vi.mock("../lib/queries/memory", () => ({
  useMemoryStats: vi.fn(),
  useMemoryConfig: vi.fn(),
  useMemoryHealth: vi.fn(),
  useMemorySearchOrList: vi.fn(),
  // MemoryPage now batches per-agent KV via useQueries(agents.map(...));
  // this stub is only consulted when the test renders agents in the section.
  agentKvMemoryQueryOptions: vi.fn(() => ({
    queryKey: ["memory", "agent-kv", "stub"],
    queryFn: async () => [],
  })),
}));

vi.mock("../lib/queries/agents", () => ({
  useAgents: vi.fn(),
}));

vi.mock("../lib/mutations/memory", () => ({
  useAddMemory: vi.fn(),
  useUpdateMemory: vi.fn(),
  useDeleteMemory: vi.fn(),
  useCleanupMemories: vi.fn(),
  useUpdateMemoryConfig: vi.fn(),
}));

vi.mock("../lib/useCreateShortcut", () => ({
  useCreateShortcut: () => {},
}));

const addToastMock = vi.fn();
vi.mock("../lib/store", () => ({
  useUIStore: (selector: (state: { addToast: typeof addToastMock }) => unknown) =>
    selector({ addToast: addToastMock }),
}));

vi.mock("../components/ui/DrawerPanel", () => ({
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

vi.mock("../components/ui/MarkdownContent", () => ({
  MarkdownContent: ({ children }: { children: React.ReactNode }) => (
    <div data-testid="markdown">{children}</div>
  ),
}));

vi.mock("motion/react", () => ({
  AnimatePresence: ({ children }: { children: React.ReactNode }) => (
    <>{children}</>
  ),
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
  const actual = await vi.importActual<typeof import("react-i18next")>(
    "react-i18next",
  );
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
    }),
  };
});

const useMemoryStatsMock = useMemoryStats as unknown as ReturnType<typeof vi.fn>;
const useMemoryConfigMock = useMemoryConfig as unknown as ReturnType<typeof vi.fn>;
const useMemoryHealthMock = useMemoryHealth as unknown as ReturnType<typeof vi.fn>;
const useMemorySearchOrListMock = useMemorySearchOrList as unknown as ReturnType<
  typeof vi.fn
>;
const useAgentsMock = useAgents as unknown as ReturnType<typeof vi.fn>;
const useAddMemoryMock = useAddMemory as unknown as ReturnType<typeof vi.fn>;
const useUpdateMemoryMock = useUpdateMemory as unknown as ReturnType<typeof vi.fn>;
const useDeleteMemoryMock = useDeleteMemory as unknown as ReturnType<typeof vi.fn>;
const useCleanupMemoriesMock = useCleanupMemories as unknown as ReturnType<
  typeof vi.fn
>;
const useUpdateMemoryConfigMock = useUpdateMemoryConfig as unknown as ReturnType<
  typeof vi.fn
>;

const STATS = {
  total: 7,
  user_count: 2,
  session_count: 3,
  agent_count: 2,
};

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
    agent_id: "agent-1234567890",
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

describe("MemoryPage", () => {
  let addMutate: ReturnType<typeof vi.fn>;
  let updateMutate: ReturnType<typeof vi.fn>;
  let deleteMutate: ReturnType<typeof vi.fn>;
  let cleanupMutate: ReturnType<typeof vi.fn>;
  let configMutateAsync: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    vi.clearAllMocks();
    addMutate = vi.fn();
    updateMutate = vi.fn();
    deleteMutate = vi.fn();
    cleanupMutate = vi.fn();
    configMutateAsync = vi.fn().mockResolvedValue(undefined);

    useAddMemoryMock.mockReturnValue({ mutate: addMutate, isPending: false });
    useUpdateMemoryMock.mockReturnValue({ mutate: updateMutate, isPending: false });
    useDeleteMemoryMock.mockReturnValue({ mutate: deleteMutate, isPending: false });
    useCleanupMemoriesMock.mockReturnValue({
      mutate: cleanupMutate,
      isPending: false,
    });
    useUpdateMemoryConfigMock.mockReturnValue({
      mutateAsync: configMutateAsync,
      isPending: false,
    });

    useMemoryHealthMock.mockReturnValue({ data: true });
    useMemoryConfigMock.mockReturnValue({
      data: CONFIG,
      isLoading: false,
      isError: false,
    });
    useMemoryStatsMock.mockReturnValue({
      data: STATS,
      isLoading: false,
      isError: false,
    });
    useMemorySearchOrListMock.mockReturnValue({
      data: { memories: MEMORIES, total: 2, proactive_enabled: true },
      isLoading: false,
      isError: false,
      isFetching: false,
      refetch: vi.fn(),
    });
    useAgentsMock.mockReturnValue({ data: [] });
  });

  it("renders KPI stats from useMemoryStats", () => {
    renderPage();
    // total=7 should be rendered as KPI value
    expect(screen.getByText("7")).toBeInTheDocument();
    // session_count=3, agent_count=2, user_count=2
    expect(screen.getByText("3")).toBeInTheDocument();
  });

  it("renders memory list items from useMemorySearchOrList", () => {
    renderPage();
    expect(screen.getByText("mem-aaaaaaaa")).toBeInTheDocument();
    expect(screen.getByText("mem-bbbbbbbb")).toBeInTheDocument();
    expect(screen.getByText("remember to water the plants")).toBeInTheDocument();
  });

  it("shows loading skeletons while memory list is loading", () => {
    useMemorySearchOrListMock.mockReturnValue({
      data: undefined,
      isLoading: true,
      isError: false,
      isFetching: true,
      refetch: vi.fn(),
    });
    const { container } = render(
      <QueryClientProvider
        client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
      >
        <MemoryPage />
      </QueryClientProvider>,
    );
    // Skeletons render as elements with `animate-pulse` class — at least one.
    expect(container.querySelectorAll(".animate-pulse").length).toBeGreaterThan(0);
  });

  it("renders proactive-disabled notice when proactive_enabled is false", () => {
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
        "Proactive memory is disabled in config — showing per-agent KV memories instead.",
      ),
    ).toBeInTheDocument();
  });

  it("calls useDeleteMemory.mutate with the memory id when trash is clicked and confirmed", async () => {
    renderPage();
    const firstCardId = screen.getByText("mem-aaaaaaaa");
    const card = firstCardId.closest("div.flex")?.parentElement?.parentElement;
    expect(card).toBeTruthy();
    const buttons = within(card as HTMLElement).getAllByRole("button");
    fireEvent.click(buttons[buttons.length - 1]);
    const confirmBtn = await screen.findByRole("button", { name: "Delete" });
    fireEvent.click(confirmBtn);
    await waitFor(() => {
      expect(deleteMutate).toHaveBeenCalledTimes(1);
    });
    expect(deleteMutate.mock.calls[0][0]).toBe("mem-aaaaaaaa");
  });

  it("calls useCleanupMemories.mutate when Cleanup is clicked", async () => {
    renderPage();
    fireEvent.click(screen.getByRole("button", { name: /memory\.cleanup/i }));
    await waitFor(() => {
      expect(cleanupMutate).toHaveBeenCalledTimes(1);
    });
  });

  it("opens the Add Memory drawer and calls useAddMemory.mutate with the entered content", async () => {
    renderPage();
    fireEvent.click(screen.getByRole("button", { name: /memory\.add/i }));
    // Drawer's textarea now visible.
    const textarea = await screen.findByPlaceholderText("memory.content_placeholder");
    fireEvent.change(textarea, { target: { value: "new memory" } });
    // The Save buttons in drawer use common.save key.
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

  it("filters memory list by level when a level chip is clicked", () => {
    renderPage();
    // Initially both memories visible.
    expect(screen.getByText("mem-aaaaaaaa")).toBeInTheDocument();
    expect(screen.getByText("mem-bbbbbbbb")).toBeInTheDocument();
    // Click the "user" filter chip (only memories with level=user remain).
    fireEvent.click(screen.getByRole("button", { name: "user" }));
    expect(screen.getByText("mem-aaaaaaaa")).toBeInTheDocument();
    expect(screen.queryByText("mem-bbbbbbbb")).not.toBeInTheDocument();
  });

  it("renders the embedding-config summary card with provider and model from useMemoryConfig", () => {
    renderPage();
    expect(screen.getByText("openai / text-embedding-3-small")).toBeInTheDocument();
    expect(screen.getByText("gpt-4o-mini")).toBeInTheDocument();
  });

  it("renders empty-state for per-agent KV when there are no agents", () => {
    renderPage();
    expect(screen.getByText("No agents available")).toBeInTheDocument();
  });
});
