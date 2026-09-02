import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { AnalyticsPage, escapeCsvField, rangeFileLabel } from "./AnalyticsPage";
import { resolveUsageRange } from "../lib/usageRange";
import {
  useUsageSummary,
  useUsageByAgent,
  useUsageByModel,
  useUsageDaily,
  useModelPerformance,
  useBudgetStatus,
  useProviderBudgets,
} from "../lib/queries/analytics";
import { useUpdateBudget, useUpdateProviderBudget } from "../lib/mutations/analytics";

vi.mock("../lib/queries/analytics", () => ({
  useUsageSummary: vi.fn(),
  useUsageByAgent: vi.fn(),
  useUsageByModel: vi.fn(),
  useUsageDaily: vi.fn(),
  useModelPerformance: vi.fn(),
  useBudgetStatus: vi.fn(),
  useProviderBudgets: vi.fn(),
}));

vi.mock("../lib/mutations/analytics", () => ({
  useUpdateBudget: vi.fn(),
  useUpdateProviderBudget: vi.fn(),
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
const useProviderBudgetsMock = useProviderBudgets as unknown as ReturnType<typeof vi.fn>;
const useUpdateBudgetMock = useUpdateBudget as unknown as ReturnType<typeof vi.fn>;
const useUpdateProviderBudgetMock = useUpdateProviderBudget as unknown as ReturnType<typeof vi.fn>;

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
  useProviderBudgetsMock.mockReturnValue(makeQuery(undefined, { isLoading: true }));
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
  useProviderBudgetsMock.mockReturnValue(
    makeQuery({ providers: [], alert_threshold: 0.8 }),
  );
}

function setMutationDefault(mutate = vi.fn()): ReturnType<typeof vi.fn> {
  useUpdateBudgetMock.mockReturnValue({
    mutate,
    isPending: false,
    isSuccess: false,
  });
  useUpdateProviderBudgetMock.mockReturnValue({
    mutate: vi.fn(),
    isPending: false,
    isSuccess: false,
  });
  return mutate;
}

/**
 * Read a KPI tile's value by its label.
 *
 * The same number legitimately shows up in a tile and in a table below it
 * (a worst-P95 tile over a P95 column, for instance), so a bare
 * `getByText(value)` is ambiguous and its failure message says nothing about
 * which surface was wrong. Tile markup is
 * `<Card><div><span>{label}</span><div>icon</div></div><p>{value}</p></Card>`.
 */
function kpiValue(label: string): string {
  const labelEl = screen.getByText(label);
  const card = labelEl.parentElement?.parentElement;
  if (!card) throw new Error(`no KPI card around label ${label}`);
  const value = card.querySelector("p");
  if (!value) throw new Error(`no KPI value under label ${label}`);
  return value.textContent ?? "";
}

/**
 * Read a range-summary cell's value by its label.
 *
 * Cell markup is `<div><p>{label}</p><p>{value}</p></div>`.
 */
function summaryValue(label: string): string {
  const labelEl = screen.getByText(label);
  const paragraphs = labelEl.parentElement?.querySelectorAll("p");
  if (!paragraphs || paragraphs.length < 2) {
    throw new Error(`no summary value beside label ${label}`);
  }
  return paragraphs[1].textContent ?? "";
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

  it("keeps hands out of the cost-by-agent chart but lists them in the detail table", () => {
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
        { agent_id: "h1", name: "a-hand", total_cost_usd: 999, is_hand: true },
        { agent_id: "a1", name: "alpha", total_cost_usd: 0.5, cost: 0.5 },
        { agent_id: "a2", name: "beta", total_cost_usd: 1.5, cost: 1.5 },
      ]),
    );
    useUsageByModelMock.mockReturnValue(makeQuery([]));
    useUsageDailyMock.mockReturnValue(makeQuery({ days: [], today_cost_usd: 0 }));
    useModelPerformanceMock.mockReturnValue(makeQuery([]));
    useBudgetStatusMock.mockReturnValue(makeQuery({}));
    useProviderBudgetsMock.mockReturnValue(
      makeQuery({ providers: [], alert_threshold: 0.8 }),
    );

    renderPage();

    // #8062 item 7: the hand appears exactly once — in the per-agent table,
    // carrying the `hand` tag — and not among the chart's category labels.
    // Before #8062 it was filtered out everywhere, which hid real spend.
    const handCells = screen.getAllByText("a-hand");
    expect(handCells).toHaveLength(1);
    expect(handCells[0].closest("tr")).not.toBeNull();
    expect(
      within(handCells[0].closest("tr")!).getByText("analytics.hand_tag"),
    ).toBeInTheDocument();

    // The two non-hand agents are in the table too, highest spend first.
    const table = screen.getByText("analytics.agent_detail_table").closest("div")
      ?.parentElement;
    expect(table).not.toBeNull();

    // Now that there's data, the CSV export button surface appears.
    expect(screen.getByTitle("analytics.export_csv")).toBeInTheDocument();
  });

  it("orders the per-agent detail table by spend, hands included", () => {
    setLoadedEmptyState();
    useUsageByAgentMock.mockReturnValue(
      makeQuery([
        { agent_id: "a1", name: "cheap", total_cost_usd: 0.5 },
        { agent_id: "h1", name: "pricey-hand", total_cost_usd: 9, is_hand: true },
        { agent_id: "a2", name: "mid", total_cost_usd: 1.5 },
      ]),
    );
    renderPage();

    const names = screen
      .getAllByRole("row")
      // Only the per-agent table's rows carry the hand tag column layout; match
      // on the three known names instead of guessing at table identity.
      .map((row) => row.textContent ?? "")
      .filter((text) => /cheap|pricey-hand|mid/.test(text));
    expect(names).toHaveLength(3);
    expect(names[0]).toContain("pricey-hand");
    expect(names[1]).toContain("mid");
    expect(names[2]).toContain("cheap");
  });

  it("renders the tool-call and cost-per-call KPI tiles from the summary", () => {
    setLoadedEmptyState();
    useUsageSummaryMock.mockReturnValue(
      makeQuery({
        call_count: 4,
        total_tool_calls: 17,
        total_input_tokens: 100,
        total_output_tokens: 100,
        total_cost_usd: 1,
      }),
    );
    renderPage();

    // #8062 item 1 — tool calls were the one summary field never surfaced.
    expect(kpiValue("analytics.total_tool_calls")).toBe("17");
    // #8062 item 2 — derived client-side as total_cost_usd / call_count.
    expect(kpiValue("analytics.cost_per_call_kpi")).toBe("$0.2500");
  });

  it("renders $0.0000 rather than NaN for cost per call with no calls", () => {
    setLoadedEmptyState();
    useUsageSummaryMock.mockReturnValue(
      makeQuery({ call_count: 0, total_cost_usd: 0 }),
    );
    renderPage();
    // Division by a zero call count must not reach the DOM as "$NaN".
    expect(kpiValue("analytics.cost_per_call_kpi")).toBe("$0.0000");
  });

  it("renders p95 latency and the input/output token split in the model table", () => {
    setLoadedEmptyState();
    useModelPerformanceMock.mockReturnValue(
      makeQuery([
        {
          model: "gpt-ladder",
          call_count: 20,
          total_cost_usd: 10,
          cost_per_call: 0.5,
          avg_latency_ms: 105,
          min_latency_ms: 10,
          max_latency_ms: 200,
          p95_latency_ms: 190,
          total_input_tokens: 2000,
          total_output_tokens: 1000,
        },
      ]),
    );
    renderPage();

    // #8062 item 8 — P95 is its own column, distinct from avg and min/max.
    expect(screen.getByText("analytics.p95_latency")).toBeInTheDocument();
    // ...and the worst-P95 SLO tile above the table reads the same field.
    expect(kpiValue("analytics.worst_p95_latency")).toBe("190ms");

    // The model name is also the "fastest model" tile's value, so pick the
    // table cell rather than whichever node happened to match first.
    const row = screen
      .getAllByText("gpt-ladder")
      .find((el) => el.tagName === "TD")
      ?.closest("tr");
    expect(row).not.toBeUndefined();
    const cells = within(row!)
      .getAllByRole("cell")
      .map((c) => c.textContent);
    // model, calls, total cost, cost/call, avg, P95, min/max, input, output, total.
    expect(cells).toEqual([
      "gpt-ladder",
      "20",
      "$10.0000",
      "$0.5000",
      "105ms",
      "190ms",
      "10/200ms",
      // #8062 item 6 — prompt/completion split alongside the combined total,
      // which is what makes two models comparable on price.
      "2,000",
      "1,000",
      "3,000",
    ]);
  });

  it("renders the range summary totalled from the daily rows", () => {
    setLoadedEmptyState();
    useUsageDailyMock.mockReturnValue(
      makeQuery({
        days: [
          { date: "2026-03-01", cost_usd: 1.5, tokens: 100, calls: 3 },
          { date: "2026-03-02", cost_usd: 2.5, tokens: 200, calls: 4 },
        ],
        today_cost_usd: 0,
      }),
    );
    renderPage();

    // #8062 item 9 — sums of the plotted days, not of the whole table.
    expect(summaryValue("analytics.range.summary_days")).toBe("2");
    expect(summaryValue("analytics.range.summary_calls")).toBe("7");
    expect(summaryValue("analytics.range.summary_tokens")).toBe("300");
    expect(summaryValue("analytics.range.summary_cost")).toContain("4");
    // The span caption states how much of the selected window had data at all,
    // so a mostly-idle month does not read as a mostly-missing one.
    expect(screen.getByText("analytics.range.summary_span")).toBeInTheDocument();
  });

  it("plots and totals every day of a year-wide window, with no residual 30-day cap", () => {
    setLoadedEmptyState();
    // 366 rows, the widest window `/api/usage/daily` will answer with.
    // `.slice(-30)` used to sit between this payload and the chart: a no-op at
    // the old fixed 7-day window, but a silent 92% truncation now that the
    // picker can ask for a year. The reader would have seen a caption claiming
    // 366 days above a chart drawing 30.
    //
    // recharts renders nothing in jsdom (its ResponsiveContainer is stubbed and
    // the chart surface never mounts), so chart geometry is not assertable
    // here. What is assertable is the range summary, which sums
    // `dailyChartData` — the very array handed to `<AreaChart data=…>` — so a
    // cap reintroduced anywhere upstream of the chart shows up here as a day
    // count that is not 366. Verified by putting `.slice(-30)` back: this test
    // fails with `expected '30' to be '366'`.
    const days = Array.from({ length: 366 }, (_, i) => ({
      date: new Date(Date.UTC(2025, 2, 1) + i * 86_400_000)
        .toISOString()
        .slice(0, 10),
      cost_usd: 0.5,
      tokens: 10,
      calls: 1,
    }));
    useUsageDailyMock.mockReturnValue(makeQuery({ days, today_cost_usd: 0 }));
    renderPage();

    expect(summaryValue("analytics.range.summary_days")).toBe("366");
    expect(summaryValue("analytics.range.summary_calls")).toBe("366");
    // 366 × $0.50. A 30-row truncation would report $15.
    expect(summaryValue("analytics.range.summary_cost")).toContain("183");
  });

  it("captions the stored window from first_event_date and retention_days", () => {
    setLoadedEmptyState();
    useUsageDailyMock.mockReturnValue(
      makeQuery({
        days: [],
        today_cost_usd: 0,
        // #8062 item 4 — the field is MIN(timestamp), so a full RFC 3339
        // instant. Only the date portion is rendered.
        first_event_date: "2026-02-03T04:05:06+00:00",
        retention_days: 90,
      }),
    );
    renderPage();

    expect(screen.getByText("analytics.range.data_since")).toBeInTheDocument();
    // #8062 item 10 — retention horizon beside it, so a truncated history is
    // distinguishable from a young deployment.
    expect(screen.getByText("analytics.range.retention")).toBeInTheDocument();
  });

  it("says retention is disabled when the horizon is zero", () => {
    setLoadedEmptyState();
    useUsageDailyMock.mockReturnValue(
      makeQuery({ days: [], today_cost_usd: 0, retention_days: 0 }),
    );
    renderPage();
    expect(
      screen.getByText("analytics.range.retention_disabled"),
    ).toBeInTheDocument();
  });

  // ---------------------------------------------------------------------------
  // Date-range picker (#8062 item 1)
  // ---------------------------------------------------------------------------
  //
  // These assert the range reaches EVERY usage hook, which is the specific
  // complaint in the issue: the page had a 7-day view and no way to change it,
  // and a picker wired only to the trend chart would be the same bug with a
  // control on top of it.

  // The page re-renders several times per interaction, so assert on the LAST
  // call — an earlier one still carries the pre-click window. Indexed rather
  // than `.at(-1)`: the tsconfig `lib` target predates `Array.prototype.at`.
  const lastCall = (mock: ReturnType<typeof vi.fn>): unknown[] => {
    const { calls } = mock.mock;
    expect(calls.length).toBeGreaterThan(0);
    return calls[calls.length - 1];
  };
  const lastRangeArg = (mock: ReturnType<typeof vi.fn>) => lastCall(mock)[0];
  const lastDaysArg = (mock: ReturnType<typeof vi.fn>) => lastCall(mock)[1];

  // Expectations come from the real `resolveUsageRange` rather than hardcoded
  // dates, so these assert the wiring (range reaches every hook) without
  // freezing the clock — the preset arithmetic itself is pinned against an
  // explicit `now` in `lib/usageRange.test.ts`.
  it("passes the default 7-day window to every usage query", () => {
    setLoadedEmptyState();
    renderPage();

    const expected = resolveUsageRange("7d");
    expect(Object.keys(expected)).toEqual(["start_date", "end_date"]);
    expect(lastRangeArg(useUsageSummaryMock)).toEqual(expected);
    expect(lastRangeArg(useUsageByAgentMock)).toEqual(expected);
    expect(lastRangeArg(useUsageByModelMock)).toEqual(expected);
    expect(lastRangeArg(useModelPerformanceMock)).toEqual(expected);
    expect(lastRangeArg(useUsageDailyMock)).toEqual(expected);
    // A bounded window must not also carry `days` — that combination is a 400.
    expect(lastDaysArg(useUsageDailyMock)).toBeUndefined();
  });

  it("re-queries every metric with the new window when a preset is clicked", () => {
    setLoadedEmptyState();
    renderPage();

    fireEvent.click(screen.getByText("analytics.range.last_month"));

    const lastMonth = resolveUsageRange("last_month");
    expect(lastRangeArg(useUsageSummaryMock)).toEqual(lastMonth);
    expect(lastRangeArg(useUsageByAgentMock)).toEqual(lastMonth);
    expect(lastRangeArg(useUsageByModelMock)).toEqual(lastMonth);
    expect(lastRangeArg(useModelPerformanceMock)).toEqual(lastMonth);
    expect(lastRangeArg(useUsageDailyMock)).toEqual(lastMonth);
    // The new window really is different from the default the page opened on.
    expect(lastMonth).not.toEqual(resolveUsageRange("7d"));
  });

  it("sends the unbounded window plus the endpoint's max days for the All preset", () => {
    setLoadedEmptyState();
    renderPage();

    fireEvent.click(screen.getByText("analytics.range.all"));

    expect(lastRangeArg(useUsageSummaryMock)).toEqual({});
    // `all` is genuinely unbounded for the rollups, but the daily breakdown
    // needs a number or the server falls back to its own 7-day default.
    expect(lastRangeArg(useUsageDailyMock)).toEqual({});
    expect(lastDaysArg(useUsageDailyMock)).toBe(366);
  });

  it("marks the selected preset as pressed for assistive tech", () => {
    setLoadedEmptyState();
    renderPage();

    expect(screen.getByText("analytics.range.7d")).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    fireEvent.click(screen.getByText("analytics.range.today"));
    expect(screen.getByText("analytics.range.today")).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(screen.getByText("analytics.range.7d")).toHaveAttribute(
      "aria-pressed",
      "false",
    );
  });

  it("applies a hand-typed custom range to every query", () => {
    setLoadedEmptyState();
    renderPage();

    fireEvent.change(screen.getByLabelText("analytics.range.from"), {
      target: { value: "2026-01-01" },
    });
    fireEvent.change(screen.getByLabelText("analytics.range.to"), {
      target: { value: "2026-01-31" },
    });

    const january = { start_date: "2026-01-01", end_date: "2026-01-31" };
    expect(lastRangeArg(useUsageSummaryMock)).toEqual(january);
    expect(lastRangeArg(useUsageByModelMock)).toEqual(january);
    // Editing an input deselects the preset rather than leaving a stale
    // highlight that disagrees with the dates shown next to it.
    expect(screen.getByText("analytics.range.7d")).toHaveAttribute(
      "aria-pressed",
      "false",
    );
  });

  it("does not query a half-typed date, and warns on an inverted range", () => {
    setLoadedEmptyState();
    renderPage();

    fireEvent.change(screen.getByLabelText("analytics.range.from"), {
      target: { value: "2026-0" },
    });
    // A malformed bound is dropped rather than forwarded — the endpoints answer
    // 400 for one, which would turn the whole page into an error state while
    // the operator is still typing. The valid `to` bound survives.
    expect(lastRangeArg(useUsageSummaryMock)).toEqual({
      end_date: resolveUsageRange("7d").end_date,
    });

    fireEvent.change(screen.getByLabelText("analytics.range.from"), {
      target: { value: "2026-03-31" },
    });
    fireEvent.change(screen.getByLabelText("analytics.range.to"), {
      target: { value: "2026-03-01" },
    });
    expect(screen.getByText("analytics.range.inverted")).toBeInTheDocument();
    expect(lastRangeArg(useUsageSummaryMock)).toEqual({});
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

  it("does not submit an empty payload for invalid global budget values", () => {
    setLoadedEmptyState();
    const mutate = vi.fn();
    setMutationDefault(mutate);
    renderPage();

    const inputs = screen.getAllByPlaceholderText("-");
    // hourly = "abc" (NaN) is filtered, daily = "-3" (negative) is filtered.
    fireEvent.change(inputs[0], { target: { value: "abc" } });
    fireEvent.change(inputs[1], { target: { value: "-3" } });

    fireEvent.click(screen.getByText("common.save"));

    expect(mutate).not.toHaveBeenCalled();
  });

  it("rejects a partial global budget save when any entered field is invalid", () => {
    setLoadedEmptyState();
    const mutate = vi.fn();
    setMutationDefault(mutate);
    renderPage();

    const inputs = screen.getAllByPlaceholderText("-");
    fireEvent.change(inputs[0], { target: { value: "5" } });
    fireEvent.change(inputs[3], { target: { value: "1.5" } });
    fireEvent.click(screen.getByText("common.save"));
    expect(mutate).not.toHaveBeenCalled();

    fireEvent.change(inputs[3], { target: { value: "1000" } });
    fireEvent.change(inputs[4], { target: { value: "1.1" } });
    fireEvent.click(screen.getByText("common.save"));
    expect(mutate).not.toHaveBeenCalled();

    fireEvent.change(inputs[3], { target: { value: "9007199254740993" } });
    fireEvent.change(inputs[4], { target: { value: "0.8" } });
    fireEvent.click(screen.getByText("common.save"));
    expect(mutate).not.toHaveBeenCalled();
  });

  it("rejects invalid provider caps instead of converting them to unlimited", () => {
    setLoadedEmptyState();
    const providerMutate = vi.fn();
    useUpdateProviderBudgetMock.mockReturnValue({
      mutate: providerMutate,
      isPending: false,
      isSuccess: false,
    });
    useProviderBudgetsMock.mockReturnValue(makeQuery({
      alert_threshold: 0.8,
      providers: [{
        provider: "openai",
        unconfigured: false,
        cap_hourly_usd: 10,
        cap_daily_usd: 20,
        cap_monthly_usd: 30,
        cap_tokens_per_hour: 1000,
        spend_hourly_usd: 1,
        spend_daily_usd: 2,
        spend_monthly_usd: 3,
        tokens_this_hour: 100,
        is_exhausted: false,
        exhaustion_reason: null,
        exhaustion_remaining_ms: null,
      }],
    }));
    renderPage();

    const row = screen.getByText("openai").closest("tr");
    expect(row).not.toBeNull();
    fireEvent.click(within(row!).getByText("analytics.provider_budgets.edit"));
    fireEvent.change(within(row!).getAllByRole("spinbutton")[0], { target: { value: "" } });
    fireEvent.click(within(row!).getByText("common.save"));

    expect(providerMutate).not.toHaveBeenCalled();
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
      providerBudgets: vi.fn().mockResolvedValue(undefined),
    };
    useUsageSummaryMock.mockReturnValue(makeQuery({ call_count: 0, total_input_tokens: 0, total_output_tokens: 0, total_cost_usd: 0 }, { refetch: refetches.usage }));
    useUsageByAgentMock.mockReturnValue(makeQuery([], { refetch: refetches.agent }));
    useUsageByModelMock.mockReturnValue(makeQuery([], { refetch: refetches.model }));
    useUsageDailyMock.mockReturnValue(makeQuery({ days: [], today_cost_usd: 0 }, { refetch: refetches.daily }));
    useModelPerformanceMock.mockReturnValue(makeQuery([], { refetch: refetches.perf }));
    useBudgetStatusMock.mockReturnValue(makeQuery({}, { refetch: refetches.budget }));
    useProviderBudgetsMock.mockReturnValue(
      makeQuery({ providers: [], alert_threshold: 0.8 }, { refetch: refetches.providerBudgets }),
    );

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
    expect(refetches.providerBudgets).toHaveBeenCalledTimes(1);
  });
});

describe("escapeCsvField", () => {
  it("neutralizes spreadsheet formulas", () => {
    expect(escapeCsvField("=HYPERLINK(\"https://example.test\")")).toBe("\"'=HYPERLINK(\"\"https://example.test\"\")\"");
    expect(escapeCsvField("+1+1")).toBe("'+1+1");
    expect(escapeCsvField("@SUM(A1:A2)")).toBe("'@SUM(A1:A2)");
    expect(escapeCsvField("\n=1+1")).toBe("\"'\n=1+1\"");
  });

  it("retains standard CSV quoting", () => {
    expect(escapeCsvField("plain")).toBe("plain");
    expect(escapeCsvField("two, fields")).toBe("\"two, fields\"");
  });
});

describe("rangeFileLabel", () => {
  // #8062 item 11 — the export is named after the window it covers, so three
  // exports of three different months are three distinct files rather than one
  // name overwritten twice. Mirrors the server-side naming in
  // `routes/budget.rs::usage_export`.
  it("names a bounded window by its endpoints", () => {
    expect(
      rangeFileLabel({ start_date: "2026-03-01", end_date: "2026-03-31" }),
    ).toBe("2026-03-01-to-2026-03-31");
  });

  it("collapses a single-day window to one date", () => {
    expect(
      rangeFileLabel({ start_date: "2026-03-15", end_date: "2026-03-15" }),
    ).toBe("2026-03-15");
  });

  it("labels one-sided and unbounded windows", () => {
    expect(rangeFileLabel({ start_date: "2026-03-01" })).toBe("from-2026-03-01");
    expect(rangeFileLabel({ end_date: "2026-03-31" })).toBe("through-2026-03-31");
    expect(rangeFileLabel({})).toBe("all");
  });
});
