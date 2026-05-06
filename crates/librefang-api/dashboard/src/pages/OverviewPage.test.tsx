import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { OverviewPage } from "./OverviewPage";
import { useDashboardSnapshot, useVersionInfo } from "../lib/queries/overview";
import { useQuickInit } from "../lib/mutations/overview";
import { useApprovalCount } from "../lib/queries/approvals";
import { useMcpServers } from "../lib/queries/mcp";
import { usePeers } from "../lib/queries/network";
import { useSchedules } from "../lib/queries/schedules";
import { useSessions } from "../lib/queries/sessions";
import {
  useBudgetStatus,
  useUsageDaily,
  useUsageByAgent,
  useUsageByModel,
  useModelPerformance,
} from "../lib/queries/analytics";

vi.mock("../lib/queries/overview", () => ({
  useDashboardSnapshot: vi.fn(),
  useVersionInfo: vi.fn(),
}));

vi.mock("../lib/mutations/overview", () => ({
  useQuickInit: vi.fn(),
}));

vi.mock("../lib/queries/approvals", () => ({
  useApprovalCount: vi.fn(),
}));

vi.mock("../lib/queries/mcp", () => ({
  useMcpServers: vi.fn(),
}));

vi.mock("../lib/queries/network", () => ({
  usePeers: vi.fn(),
}));

vi.mock("../lib/queries/schedules", () => ({
  useSchedules: vi.fn(),
}));

vi.mock("../lib/queries/sessions", () => ({
  useSessions: vi.fn(),
}));

vi.mock("../lib/queries/analytics", () => ({
  useBudgetStatus: vi.fn(),
  useUsageDaily: vi.fn(),
  useUsageByAgent: vi.fn(),
  useUsageByModel: vi.fn(),
  useModelPerformance: vi.fn(),
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

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => vi.fn(),
}));

const useDashboardSnapshotMock = useDashboardSnapshot as unknown as ReturnType<
  typeof vi.fn
>;
const useVersionInfoMock = useVersionInfo as unknown as ReturnType<typeof vi.fn>;
const useQuickInitMock = useQuickInit as unknown as ReturnType<typeof vi.fn>;
const useApprovalCountMock = useApprovalCount as unknown as ReturnType<typeof vi.fn>;
const useMcpServersMock = useMcpServers as unknown as ReturnType<typeof vi.fn>;
const usePeersMock = usePeers as unknown as ReturnType<typeof vi.fn>;
const useSchedulesMock = useSchedules as unknown as ReturnType<typeof vi.fn>;
const useSessionsMock = useSessions as unknown as ReturnType<typeof vi.fn>;
const useBudgetStatusMock = useBudgetStatus as unknown as ReturnType<typeof vi.fn>;
const useUsageDailyMock = useUsageDaily as unknown as ReturnType<typeof vi.fn>;
const useUsageByAgentMock = useUsageByAgent as unknown as ReturnType<typeof vi.fn>;
const useUsageByModelMock = useUsageByModel as unknown as ReturnType<typeof vi.fn>;
const useModelPerformanceMock = useModelPerformance as unknown as ReturnType<typeof vi.fn>;

function setQuickInitDefault(): void {
  useQuickInitMock.mockReturnValue({
    mutateAsync: vi.fn().mockResolvedValue(undefined),
    isPending: false,
  });
}

function setSidecarHooksDefault(): void {
  useApprovalCountMock.mockReturnValue({ data: 0 });
  useMcpServersMock.mockReturnValue({
    data: { configured: [], connected: [], total_configured: 0, total_connected: 0 },
  });
  usePeersMock.mockReturnValue({ data: [] });
  useSchedulesMock.mockReturnValue({ data: [] });
  useSessionsMock.mockReturnValue({ data: [] });
  useBudgetStatusMock.mockReturnValue({ data: undefined });
  useUsageDailyMock.mockReturnValue({ data: { days: [], today_cost_usd: 0 } });
  useUsageByAgentMock.mockReturnValue({ data: [] });
  useUsageByModelMock.mockReturnValue({ data: [] });
  useModelPerformanceMock.mockReturnValue({ data: [] });
}

function renderPage(): void {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <OverviewPage />
    </QueryClientProvider>,
  );
}

describe("OverviewPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setQuickInitDefault();
    setSidecarHooksDefault();
  });

  it("shows the loading-runtime hero state while snapshot is loading", () => {
    useDashboardSnapshotMock.mockReturnValue({
      data: undefined,
      isLoading: true,
      isFetching: true,
      isError: false,
      dataUpdatedAt: 0,
      refetch: vi.fn(),
    });
    useVersionInfoMock.mockReturnValue({ data: undefined, isLoading: true });

    renderPage();

    // Hero h1 falls back to the loading-runtime copy until snapshot resolves.
    expect(
      screen.getByRole("heading", { level: 1 }),
    ).toHaveTextContent("overview.loading_runtime");
  });

  it("renders running-agent count and KPI labels once snapshot is loaded", () => {
    useDashboardSnapshotMock.mockReturnValue({
      data: {
        status: {
          // 2 of 3 mocked agents are in the "running" state — the backend's
          // pre-computed count must match (it's authoritative for the hero).
          active_agent_count: 2,
          agent_count: 3,
          uptime_seconds: 3600,
          session_count: 7,
          config_exists: true,
          hostname: "node-test",
          version: "2026.4.27",
        },
        providers: [
          { id: "openai", auth_status: "ok" },
          { id: "anthropic", auth_status: "ok" },
        ],
        channels: [{ id: "telegram", configured: true }],
        agents: [
          { id: "a1", name: "alpha", state: "running", model_name: "claude-sonnet-4-5" },
          { id: "a2", name: "beta",  state: "running", model_name: "gpt-4.1" },
          { id: "a3", name: "gamma", state: "idle",    model_name: "gpt-4.1-mini" },
        ],
        skillCount: 12,
        workflowCount: 3,
        health: { status: "ok", checks: [] },
      },
      isLoading: false,
      isFetching: false,
      isError: false,
      dataUpdatedAt: 0,
      refetch: vi.fn(),
    });
    useVersionInfoMock.mockReturnValue({
      data: { version: "2026.4.27", commit: "abc1234" },
      isLoading: false,
    });

    renderPage();

    // Hero counts running agents from the snapshot.agents array.
    expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent("2");
    // KPI tile labels render using i18n keys.
    expect(screen.getByText("overview.kpi.active_agents")).toBeInTheDocument();
    expect(screen.getByText("analytics.avg_latency")).toBeInTheDocument();
    // Recent-agents fallback table (and tokens-by-agent breakdown)
    // surface the agent name from snapshot.agents.
    expect(screen.getAllByText("alpha").length).toBeGreaterThan(0);
  });

  it("keeps overview usage labels honest for seven-day backend data", () => {
    useDashboardSnapshotMock.mockReturnValue({
      data: {
        status: {
          active_agent_count: 1,
          agent_count: 1,
          uptime_seconds: 60,
          session_count: 1,
          config_exists: true,
          hostname: "node-test",
          version: "2026.4.27",
        },
        providers: [],
        channels: [],
        agents: [
          { id: "a1", name: "alpha", state: "running", model_name: "gpt-4.1" },
        ],
        skillCount: 0,
        workflowCount: 0,
        health: { status: "ok", checks: [] },
      },
      isLoading: false,
      isFetching: false,
      isError: false,
      dataUpdatedAt: 0,
      refetch: vi.fn(),
    });
    useVersionInfoMock.mockReturnValue({ data: undefined, isLoading: false });
    useUsageDailyMock.mockReturnValue({
      data: {
        days: [
          { date: "2026-05-01", tokens: 0, cost_usd: 0 },
          { date: "2026-05-02", tokens: 0, cost_usd: 0 },
          { date: "2026-05-03", tokens: 0, cost_usd: 0 },
          { date: "2026-05-04", tokens: 0, cost_usd: 0 },
          { date: "2026-05-05", tokens: 0, cost_usd: 0 },
          { date: "2026-05-06", tokens: 0, cost_usd: 0 },
          { date: "2026-05-07", tokens: 0, cost_usd: 0 },
        ],
        today_cost_usd: 0,
      },
    });
    useModelPerformanceMock.mockReturnValue({
      data: [{ model: "gpt-4.1", call_count: 2, avg_latency_ms: 125 }],
    });
    useUsageByAgentMock.mockReturnValue({
      data: [{ agent_id: "a1", name: "alpha", total_tokens: 1234 }],
    });
    useUsageByModelMock.mockReturnValue({
      data: [{ model: "gpt-4.1", total_cost_usd: 1.23 }],
    });

    renderPage();

    expect(screen.getAllByText("overview.range.7d").length).toBeGreaterThan(0);
    expect(screen.queryByText("30d")).not.toBeInTheDocument();
    expect(screen.queryByText("90d")).not.toBeInTheDocument();
    expect(screen.getByText(/overview\.kpi\.tokens_over_7d$/)).toBeInTheDocument();
    expect(screen.getByText("overview.kpi.no_prior")).toBeInTheDocument();
    expect(screen.getByText("overview.cost.provider_all_time")).toBeInTheDocument();
    expect(screen.getByText("analytics.avg_latency")).toBeInTheDocument();
  });

  it("renders the setup banner when config does not exist", () => {
    useDashboardSnapshotMock.mockReturnValue({
      data: {
        status: {
          active_agent_count: 0,
          agent_count: 0,
          uptime_seconds: 0,
          session_count: 0,
          config_exists: false,
        },
        providers: [],
        channels: [],
        agents: [],
        skillCount: 0,
        workflowCount: 0,
        health: { status: "ok", checks: [] },
      },
      isLoading: false,
      isFetching: false,
      isError: false,
      dataUpdatedAt: 0,
      refetch: vi.fn(),
    });
    useVersionInfoMock.mockReturnValue({ data: undefined, isLoading: false });

    renderPage();

    expect(
      screen.getByRole("heading", { name: "overview.setup_title" }),
    ).toBeInTheDocument();
  });

  // Bug #3849: health-status pill flips silently on every snapshot poll.
  // Wrap text node in aria-live=polite so SR users hear ok ↔ degraded.
  it("wraps the health-status pill text in an aria-live=polite region", () => {
    useDashboardSnapshotMock.mockReturnValue({
      data: {
        status: {
          active_agent_count: 1,
          agent_count: 1,
          uptime_seconds: 60,
          session_count: 1,
          config_exists: true,
          hostname: "node-test",
          version: "2026.4.27",
        },
        providers: [{ id: "openai", auth_status: "ok" }],
        channels: [],
        agents: [
          { id: "a1", name: "alpha", state: "running", model_name: "gpt-4.1" },
        ],
        skillCount: 0,
        workflowCount: 0,
        health: { status: "degraded", checks: [] },
      },
      isLoading: false,
      isFetching: false,
      isError: false,
      dataUpdatedAt: 0,
      refetch: vi.fn(),
    });
    useVersionInfoMock.mockReturnValue({ data: undefined, isLoading: false });

    renderPage();

    // Both pills (hero + Health column) should render their text inside an
    // aria-live=polite region so background snapshot polling announces flips.
    const heroDegraded = screen.getByText("degraded");
    const live1 = heroDegraded.closest('[aria-live="polite"]');
    expect(live1).not.toBeNull();
    expect(live1?.getAttribute("aria-atomic")).toBe("true");

    const columnDegraded = screen.getByText("DEGRADED");
    const live2 = columnDegraded.closest('[aria-live="polite"]');
    expect(live2).not.toBeNull();
    expect(live2?.getAttribute("aria-atomic")).toBe("true");
  });
});
