import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { AnalyticsPage } from "./AnalyticsPage";
import {
  useUsageSummary,
  useUsageByAgent,
  useUsageByModel,
  useUsageDaily,
  useModelPerformance,
  useBudgetStatus,
} from "../lib/queries/analytics";
import { useUpdateBudget } from "../lib/mutations/analytics";

vi.mock("../lib/queries/analytics", () => ({
  useUsageSummary: vi.fn(),
  useUsageByAgent: vi.fn(),
  useUsageByModel: vi.fn(),
  useUsageDaily: vi.fn(),
  useModelPerformance: vi.fn(),
  useBudgetStatus: vi.fn(),
}));

vi.mock("../lib/mutations/analytics", () => ({
  useUpdateBudget: vi.fn(),
}));

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>(
    "react-i18next",
  );
  return {
    ...actual,
    useTranslation: () => ({ t: (key: string) => key }),
  };
});

// recharts' ResponsiveContainer measures parent width via ResizeObserver and
// renders nothing in jsdom; stub the chart wrappers so chart-bearing branches
// still mount. We only assert on surrounding labels/inputs, not chart geometry.
vi.mock("recharts", async () => {
  const actual = await vi.importActual<typeof import("recharts")>("recharts");
  return {
    ...actual,
    ResponsiveContainer: ({ children }: { children: React.ReactNode }) => (
      <div data-testid="responsive-container" style={{ width: 600, height: 300 }}>
        {children}
      </div>
    ),
  };
});

const useUsageSummaryMock = useUsageSummary as unknown as ReturnType<typeof vi.fn>;
const useUsageByAgentMock = useUsageByAgent as unknown as ReturnType<typeof vi.fn>;
const useUsageByModelMock = useUsageByModel as unknown as ReturnType<typeof vi.fn>;
const useUsageDailyMock = useUsageDaily as unknown as ReturnType<typeof vi.fn>;
const useModelPerformanceMock = useModelPerformance as unknown as ReturnType<typeof vi.fn>;
const useBudgetStatusMock = useBudgetStatus as unknown as ReturnType<typeof vi.fn>;
const useUpdateBudgetMock = useUpdateBudget as unknown as ReturnType<typeof vi.fn>;

interface QueryShape<T> {
  data: T;
  isLoading: boolean;
  isFetching: boolean;
  isError: boolean;
  refetch: ReturnType<typeof vi.fn>;
}

function makeQuery<T>(data: T, overrides: Partial<QueryShape<T>> = {}): QueryShape<T> {
  return {
    data,
    isLoading: false,
    isFetching: false,
    isError: false,
    refetch: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  };
}

function setLoadingState(): void {
  useUsageSummaryMock.mockReturnValue(makeQuery(undefined, { isLoading: true, isFetching: true }));
  useUsageByAgentMock.mockReturnValue(makeQuery(undefined, { isLoading: true }));
  useUsageByModelMock.mockReturnValue(makeQuery(undefined, { isLoading: true }));
  useUsageDailyMock.mockReturnValue(makeQuery(undefined, { isLoading: true }));
  useModelPerformanceMock.mockReturnValue(makeQuery(undefined, { isLoading: true }));
  useBudgetStatusMock.mockReturnValue(makeQuery(undefined));
}

function setLoadedEmptyState(): void {
  useUsageSummaryMock.mockReturnValue(
    makeQuery({
      call_count: 0,
      total_input_tokens: 0,
      total_output_tokens: 0,
      total_cost_usd: 0,
    }),
  );
  useUsageByAgentMock.mockReturnValue(makeQuery([]));
  useUsageByModelMock.mockReturnValue(makeQuery([]));
  useUsageDailyMock.mockReturnValue(makeQuery({ days: [], today_cost_usd: 0 }));
  useModelPerformanceMock.mockReturnValue(makeQuery([]));
  useBudgetStatusMock.mockReturnValue(makeQuery({}));
}

function setMutationDefault(mutate = vi.fn()): ReturnType<typeof vi.fn> {
  useUpdateBudgetMock.mockReturnValue({
    mutate,
    isPending: false,
    isSuccess: false,
  });
  return mutate;
}

function renderPage(): void {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <AnalyticsPage />
    </QueryClientProvider>,
  );
}

describe("AnalyticsPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setMutationDefault();
  });

  it("renders skeleton placeholders while usage queries are loading", () => {
    setLoadingState();
    renderPage();

    // Header still mounts; KPI cards are replaced with skeletons, so the
    // total_calls KPI label must NOT be in the document yet.
    expect(screen.getByText("analytics.title")).toBeInTheDocument();
    expect(screen.queryByText("analytics.total_calls")).not.toBeInTheDocument();
  });

  it("renders KPI tiles and empty-state copy when there is no usage data", () => {
    setLoadedEmptyState();
    renderPage();

    // KPI labels render even when totals are zero.
    expect(screen.getByText("analytics.total_calls")).toBeInTheDocument();
    expect(screen.getByText("analytics.total_tokens_label")).toBeInTheDocument();
    // EmptyState in usage_by_agent / usage_by_model cards uses common.no_data.
    const noData = screen.getAllByText("common.no_data");
    expect(noData.length).toBeGreaterThanOrEqual(2);
    expect(screen.getByText("analytics.no_agent_data")).toBeInTheDocument();
    expect(screen.getByText("analytics.no_model_data")).toBeInTheDocument();
  });

  it("hides the CSV export button when there are no agent or model rows", () => {
    setLoadedEmptyState();
    renderPage();
    // The export button only renders when there is something to export.
    expect(screen.queryByTitle("analytics.export_csv")).not.toBeInTheDocument();
  });

  it("renders agent rows sorted by total_cost_usd descending and excludes hands", () => {
    useUsageSummaryMock.mockReturnValue(
      makeQuery({
        call_count: 42,
        total_input_tokens: 1000,
        total_output_tokens: 500,
        total_cost_usd: 1.23,
      }),
    );
    useUsageByAgentMock.mockReturnValue(
      makeQuery([
        // Hand entries must be filtered out — they're tracked separately
        // and double-count if mixed in.
        { agent_id: "h1", name: "ignored-hand", total_cost_usd: 999, is_hand: true },
        { agent_id: "a1", name: "alpha", total_cost_usd: 0.5, cost: 0.5 },
        { agent_id: "a2", name: "beta", total_cost_usd: 1.5, cost: 1.5 },
      ]),
    );
    useUsageByModelMock.mockReturnValue(makeQuery([]));
    useUsageDailyMock.mockReturnValue(makeQuery({ days: [], today_cost_usd: 0 }));
    useModelPerformanceMock.mockReturnValue(makeQuery([]));
    useBudgetStatusMock.mockReturnValue(makeQuery({}));

    renderPage();

    expect(screen.queryByText("ignored-hand")).not.toBeInTheDocument();
    // Now that there's data, the CSV export button surface appears.
    expect(screen.getByTitle("analytics.export_csv")).toBeInTheDocument();
  });

  it("invokes useUpdateBudget with parsed numeric payload when Save is clicked", () => {
    setLoadedEmptyState();
    const mutate = vi.fn();
    setMutationDefault(mutate);
    renderPage();

    // Type into hourly + alert; leave others blank — payload should only
    // include the keys the user actually edited.
    const inputs = screen.getAllByPlaceholderText("-");
    // Order matches the field array: [hourly, daily, monthly, tokens, alert].
    fireEvent.change(inputs[0], { target: { value: "5" } });

    fireEvent.click(screen.getByText("common.save"));

    expect(mutate).toHaveBeenCalledTimes(1);
    const payload = mutate.mock.calls[0][0];
    expect(payload).toEqual({ max_hourly_usd: 5 });
  });

  it("ignores non-numeric and negative values in the budget save payload", () => {
    setLoadedEmptyState();
    const mutate = vi.fn();
    setMutationDefault(mutate);
    renderPage();

    const inputs = screen.getAllByPlaceholderText("-");
    // hourly = "abc" (NaN) is filtered, daily = "-3" (negative) is filtered.
    fireEvent.change(inputs[0], { target: { value: "abc" } });
    fireEvent.change(inputs[1], { target: { value: "-3" } });

    fireEvent.click(screen.getByText("common.save"));

    expect(mutate).toHaveBeenCalledTimes(1);
    expect(mutate.mock.calls[0][0]).toEqual({});
  });

  it("disables the Save button while the budget mutation is pending", () => {
    setLoadedEmptyState();
    useUpdateBudgetMock.mockReturnValue({
      mutate: vi.fn(),
      isPending: true,
      isSuccess: false,
    });
    renderPage();

    const save = screen.getByText("common.save").closest("button");
    expect(save).toBeDisabled();
  });

  it("shows the budget-saved confirmation after a successful mutation", () => {
    setLoadedEmptyState();
    useUpdateBudgetMock.mockReturnValue({
      mutate: vi.fn(),
      isPending: false,
      isSuccess: true,
    });
    renderPage();

    expect(screen.getByText("analytics.budget_saved")).toBeInTheDocument();
  });

  it("refetches every analytics query when the header refresh action fires", () => {
    setLoadedEmptyState();
    const refetches = {
      usage: vi.fn().mockResolvedValue(undefined),
      agent: vi.fn().mockResolvedValue(undefined),
      model: vi.fn().mockResolvedValue(undefined),
      daily: vi.fn().mockResolvedValue(undefined),
      perf: vi.fn().mockResolvedValue(undefined),
      budget: vi.fn().mockResolvedValue(undefined),
    };
    useUsageSummaryMock.mockReturnValue(makeQuery({ call_count: 0, total_input_tokens: 0, total_output_tokens: 0, total_cost_usd: 0 }, { refetch: refetches.usage }));
    useUsageByAgentMock.mockReturnValue(makeQuery([], { refetch: refetches.agent }));
    useUsageByModelMock.mockReturnValue(makeQuery([], { refetch: refetches.model }));
    useUsageDailyMock.mockReturnValue(makeQuery({ days: [], today_cost_usd: 0 }, { refetch: refetches.daily }));
    useModelPerformanceMock.mockReturnValue(makeQuery([], { refetch: refetches.perf }));
    useBudgetStatusMock.mockReturnValue(makeQuery({}, { refetch: refetches.budget }));

    renderPage();

    // PageHeader's refresh button has aria-label/title "common.refresh"; it
    // also renders a generic <button>. Find by accessible text or icon-only
    // button — fall back to scanning all buttons for the click.
    const refreshBtn = screen.getByLabelText("common.refresh");
    fireEvent.click(refreshBtn);

    expect(refetches.usage).toHaveBeenCalledTimes(1);
    expect(refetches.agent).toHaveBeenCalledTimes(1);
    expect(refetches.model).toHaveBeenCalledTimes(1);
    expect(refetches.daily).toHaveBeenCalledTimes(1);
    expect(refetches.perf).toHaveBeenCalledTimes(1);
    expect(refetches.budget).toHaveBeenCalledTimes(1);
  });
});
