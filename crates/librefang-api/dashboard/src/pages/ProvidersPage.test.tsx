// Tests for the LLM providers page (refs #3853 — pages/ test gap).
//
// Mocks at the queries/mutations hook layer per the dashboard data-layer rule
// (see crates/librefang-api/dashboard/AGENTS.md): pages route through
// `lib/queries` / `lib/mutations`, never raw `fetch()`. Render-side concerns
// (motion, modals, drawer, store toasts) are stubbed so we can exercise the
// core list/filter/tab/search wiring without dragging in animation timers.

import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ProviderItem } from "../api";
import { ProvidersPage } from "./ProvidersPage";
import { useProviders, useProviderStatus } from "../lib/queries/providers";
import { useModels } from "../lib/queries/models";
import {
  useTestProvider,
  useSetProviderKey,
  useDeleteProviderKey,
  useSetProviderUrl,
  useSetDefaultProvider,
  useCreateRegistryContent,
} from "../lib/mutations/providers";

vi.mock("../lib/queries/providers", () => ({
  useProviders: vi.fn(),
  useProviderStatus: vi.fn(),
}));

vi.mock("../lib/queries/models", () => ({
  useModels: vi.fn(),
}));

vi.mock("../lib/mutations/providers", () => ({
  useTestProvider: vi.fn(),
  useSetProviderKey: vi.fn(),
  useDeleteProviderKey: vi.fn(),
  useSetProviderUrl: vi.fn(),
  useSetDefaultProvider: vi.fn(),
  useCreateRegistryContent: vi.fn(),
}));

// Toast store — only `addToast` is consumed by ProvidersPage.
const addToastMock = vi.fn();
vi.mock("../lib/store", () => ({
  useUIStore: (selector: (s: { addToast: typeof addToastMock }) => unknown) =>
    selector({ addToast: addToastMock }),
}));

// Keyboard shortcut hook is fire-and-forget here.
vi.mock("../lib/useCreateShortcut", () => ({
  useCreateShortcut: vi.fn(),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, fallbackOrOpts?: unknown) =>
      typeof fallbackOrOpts === "string" ? fallbackOrOpts : key,
  }),
}));

const useProvidersMock = useProviders as unknown as ReturnType<typeof vi.fn>;
const useProviderStatusMock = useProviderStatus as unknown as ReturnType<
  typeof vi.fn
>;
const useModelsMock = useModels as unknown as ReturnType<typeof vi.fn>;
const useTestProviderMock = useTestProvider as unknown as ReturnType<
  typeof vi.fn
>;
const useSetProviderKeyMock = useSetProviderKey as unknown as ReturnType<
  typeof vi.fn
>;
const useDeleteProviderKeyMock = useDeleteProviderKey as unknown as ReturnType<
  typeof vi.fn
>;
const useSetProviderUrlMock = useSetProviderUrl as unknown as ReturnType<
  typeof vi.fn
>;
const useSetDefaultProviderMock = useSetDefaultProvider as unknown as ReturnType<
  typeof vi.fn
>;
const useCreateRegistryContentMock =
  useCreateRegistryContent as unknown as ReturnType<typeof vi.fn>;

const PROVIDERS: ProviderItem[] = [
  {
    id: "openai",
    display_name: "OpenAI",
    auth_status: "validated_key",
    reachable: true,
    model_count: 12,
    latency_ms: 120,
    key_required: true,
    base_url: "https://api.openai.com/v1",
  },
  {
    id: "anthropic",
    display_name: "Anthropic",
    auth_status: "configured",
    reachable: false,
    model_count: 5,
    latency_ms: 700,
    key_required: true,
    base_url: "https://api.anthropic.com",
  },
  {
    id: "groq",
    display_name: "Groq",
    auth_status: "missing",
    reachable: false,
    model_count: 0,
    key_required: true,
  },
];

function renderPage(): void {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <ProvidersPage />
    </QueryClientProvider>,
  );
}

describe("ProvidersPage", () => {
  let testMutateAsync: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    vi.clearAllMocks();
    testMutateAsync = vi.fn().mockResolvedValue({ status: "ok" });

    useProviderStatusMock.mockReturnValue({
      data: { default_provider: "openai" },
      isFetching: false,
    });
    useModelsMock.mockReturnValue({ data: { models: [] }, isLoading: false });

    const stubMutation = (mutateAsync: ReturnType<typeof vi.fn>) => ({
      mutateAsync,
      isPending: false,
    });

    useTestProviderMock.mockReturnValue(stubMutation(testMutateAsync));
    useSetProviderKeyMock.mockReturnValue(
      stubMutation(vi.fn().mockResolvedValue(undefined)),
    );
    useDeleteProviderKeyMock.mockReturnValue(
      stubMutation(vi.fn().mockResolvedValue(undefined)),
    );
    useSetProviderUrlMock.mockReturnValue(
      stubMutation(vi.fn().mockResolvedValue(undefined)),
    );
    useSetDefaultProviderMock.mockReturnValue(
      stubMutation(vi.fn().mockResolvedValue(undefined)),
    );
    useCreateRegistryContentMock.mockReturnValue(
      stubMutation(vi.fn().mockResolvedValue(undefined)),
    );
  });

  it("shows skeleton placeholders while providers load", () => {
    useProvidersMock.mockReturnValue({
      data: undefined,
      isLoading: true,
      isFetching: true,
      refetch: vi.fn(),
    });

    renderPage();

    // CardSkeleton uses role="status" aria-busy="true" — six are emitted
    // while the providers query is pending.
    expect(screen.getAllByRole("status").length).toBeGreaterThanOrEqual(6);
  });

  it("renders empty state when the providers list is empty", () => {
    useProvidersMock.mockReturnValue({
      data: [],
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();

    expect(screen.getByText("common.no_data")).toBeInTheDocument();
  });

  it("shows the configured/total count badge in the header", () => {
    useProvidersMock.mockReturnValue({
      data: PROVIDERS,
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();

    // 2 of 3 providers in PROVIDERS are configured (openai, anthropic).
    expect(screen.getByText(/2 \/ 3/)).toBeInTheDocument();
  });

  it("renders configured providers by default and hides unconfigured ones", () => {
    useProvidersMock.mockReturnValue({
      data: PROVIDERS,
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();

    expect(screen.getByText("OpenAI")).toBeInTheDocument();
    expect(screen.getByText("Anthropic")).toBeInTheDocument();
    // groq is `missing` → unconfigured tab only.
    expect(screen.queryByText("Groq")).not.toBeInTheDocument();
  });

  it("switches to the unconfigured tab and shows only setup-needed providers", async () => {
    useProvidersMock.mockReturnValue({
      data: PROVIDERS,
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();

    fireEvent.click(screen.getByRole("tab", { name: /providers\.unconfigured/ }));

    // AnimatePresence (mode="wait") keys on activeTab so the swap is async.
    expect(await screen.findByText("Groq")).toBeInTheDocument();
    expect(screen.queryByText("OpenAI")).not.toBeInTheDocument();
  });

  it("filters configured providers by search term", () => {
    useProvidersMock.mockReturnValue({
      data: PROVIDERS,
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();

    fireEvent.change(screen.getByPlaceholderText("common.search"), {
      target: { value: "anthr" },
    });

    expect(screen.getByText("Anthropic")).toBeInTheDocument();
    expect(screen.queryByText("OpenAI")).not.toBeInTheDocument();
  });

  it("shows a 'no results' empty state when search matches nothing", () => {
    useProvidersMock.mockReturnValue({
      data: PROVIDERS,
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();

    fireEvent.change(screen.getByPlaceholderText("common.search"), {
      target: { value: "definitely-not-a-provider" },
    });

    expect(screen.getByText("providers.no_results")).toBeInTheDocument();
  });

  it("filters by reachability via the reachable chip", () => {
    useProvidersMock.mockReturnValue({
      data: PROVIDERS,
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();

    // FilterChips renders a button per status; pick the "reachable" one.
    const reachableBtn = screen.getByRole("button", {
      name: /providers\.filter_reachable/,
    });
    fireEvent.click(reachableBtn);

    expect(screen.getByText("OpenAI")).toBeInTheDocument();
    // Anthropic is reachable: false — should be filtered out.
    expect(screen.queryByText("Anthropic")).not.toBeInTheDocument();
  });

  it("calls useTestProvider when the per-card Test action fires", async () => {
    useProvidersMock.mockReturnValue({
      data: PROVIDERS,
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();

    // Find the OpenAI card by its display name, then click its Test button.
    const openaiCard = screen.getByText("OpenAI").closest("div");
    expect(openaiCard).toBeTruthy();
    // Search the whole document for any Test button — clicking the first
    // visible one is sufficient: it triggers the mutation regardless of
    // which card it belongs to.
    const testButtons = within(document.body).getAllByRole("button", {
      name: /providers\.test/,
    });
    expect(testButtons.length).toBeGreaterThan(0);
    fireEvent.click(testButtons[0]);

    // The handler is async — assert the mutation was kicked off.
    expect(testMutateAsync).toHaveBeenCalledTimes(1);
    expect(typeof testMutateAsync.mock.calls[0][0]).toBe("string");
  });
});
