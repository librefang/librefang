import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { OverviewPage } from "./OverviewPage";
import { useDashboardSnapshot, useVersionInfo } from "../lib/queries/overview";
import { useQuickInit } from "../lib/mutations/overview";

vi.mock("../lib/queries/overview", () => ({
  useDashboardSnapshot: vi.fn(),
  useVersionInfo: vi.fn(),
}));

vi.mock("../lib/mutations/overview", () => ({
  useQuickInit: vi.fn(),
}));

// `react-i18next` exposes more than just `useTranslation` — `i18n.ts` calls
// `i18n.use(initReactI18next)` at module load time. Spread the real export
// so unmocked entries (initReactI18next, Trans, …) keep working.
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

function setQuickInitDefault(): void {
  useQuickInitMock.mockReturnValue({
    mutateAsync: vi.fn().mockResolvedValue(undefined),
    isPending: false,
  });
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
  });

  it("renders the welcome heading and skeletons while loading", () => {
    useDashboardSnapshotMock.mockReturnValue({
      data: undefined,
      isLoading: true,
      isFetching: true,
      dataUpdatedAt: 0,
      refetch: vi.fn(),
    });
    useVersionInfoMock.mockReturnValue({ data: undefined, isLoading: true });

    renderPage();

    // Welcome heading is rendered in every state and uses an i18n key, so
    // the `t: (k) => k` mock makes it a stable anchor.
    expect(
      screen.getByRole("heading", { level: 1, name: "overview.welcome" }),
    ).toBeInTheDocument();
    // Loaded-state stats values must NOT be present yet.
    expect(screen.queryByText("42")).toBeNull();
  });

  it("renders snapshot data and version when loaded", () => {
    useDashboardSnapshotMock.mockReturnValue({
      data: {
        status: {
          active_agent_count: 42,
          agent_count: 100,
          uptime_seconds: 3600,
          session_count: 7,
          config_exists: true,
        },
        providers: [
          { id: "openai", auth_status: "ok" },
          { id: "anthropic", auth_status: "ok" },
        ],
        channels: [{ id: "telegram", configured: true }],
        agents: [],
        skillCount: 12,
        workflowCount: 3,
        health: { status: "ok", checks: [] },
      },
      isLoading: false,
      isFetching: false,
      dataUpdatedAt: 0,
      refetch: vi.fn(),
    });
    useVersionInfoMock.mockReturnValue({
      data: { version: "2026.4.27", commit: "abc1234" },
      isLoading: false,
    });

    renderPage();

    expect(
      screen.getByRole("heading", { level: 1, name: "overview.welcome" }),
    ).toBeInTheDocument();
    // Active agent count from snapshot
    expect(screen.getByText("42")).toBeInTheDocument();
    // Version pulled from /api/version
    expect(screen.getByText("2026.4.27")).toBeInTheDocument();
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
      dataUpdatedAt: 0,
      refetch: vi.fn(),
    });
    useVersionInfoMock.mockReturnValue({ data: undefined, isLoading: false });

    renderPage();

    // Setup banner heading uses the `overview.setup_title` i18n key.
    expect(
      screen.getByRole("heading", { name: "overview.setup_title" }),
    ).toBeInTheDocument();
  });
});
