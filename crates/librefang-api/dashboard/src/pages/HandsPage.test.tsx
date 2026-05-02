import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { HandsPage } from "./HandsPage";
import {
  useHands,
  useActiveHands,
  useHandStatsBatch,
  useHandDetail,
  useHandSettings,
  useHandStats,
  useHandManifestToml,
} from "../lib/queries/hands";
import {
  useActivateHand,
  useDeactivateHand,
  usePauseHand,
  useResumeHand,
  useUninstallHand,
} from "../lib/mutations/hands";
import { useCronJobs } from "../lib/queries/runtime";

vi.mock("../lib/queries/hands", () => ({
  useHands: vi.fn(),
  useActiveHands: vi.fn(),
  useHandStatsBatch: vi.fn(),
  useHandDetail: vi.fn(),
  useHandSettings: vi.fn(),
  useHandStats: vi.fn(),
  useHandManifestToml: vi.fn(),
}));

vi.mock("../lib/mutations/hands", () => ({
  useActivateHand: vi.fn(),
  useDeactivateHand: vi.fn(),
  usePauseHand: vi.fn(),
  useResumeHand: vi.fn(),
  useUninstallHand: vi.fn(),
  useSetHandSecret: vi.fn(() => ({ mutateAsync: vi.fn(), isPending: false })),
  useUpdateHandSettings: vi.fn(() => ({ mutate: vi.fn(), isPending: false })),
}));

vi.mock("../lib/mutations/schedules", () => ({
  useCreateSchedule: vi.fn(() => ({ mutateAsync: vi.fn(), isPending: false })),
  useUpdateSchedule: vi.fn(() => ({ mutateAsync: vi.fn(), isPending: false })),
  useDeleteSchedule: vi.fn(() => ({ mutateAsync: vi.fn(), isPending: false })),
}));

vi.mock("../lib/queries/runtime", () => ({
  useCronJobs: vi.fn(),
}));

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>(
    "react-i18next",
  );
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, opts?: Record<string, unknown>) =>
        (opts?.defaultValue as string | undefined) ?? key,
    }),
  };
});

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => vi.fn(),
}));

vi.mock("../router", () => ({
  router: { preloadRoute: vi.fn().mockResolvedValue(undefined) },
}));

vi.mock("../lib/store", () => ({
  useUIStore: (selector: (state: { addToast: (m: string, t?: string) => void }) => unknown) =>
    selector({ addToast: vi.fn() }),
}));

const useHandsMock = useHands as unknown as ReturnType<typeof vi.fn>;
const useActiveHandsMock = useActiveHands as unknown as ReturnType<typeof vi.fn>;
const useHandStatsBatchMock = useHandStatsBatch as unknown as ReturnType<typeof vi.fn>;
const useHandDetailMock = useHandDetail as unknown as ReturnType<typeof vi.fn>;
const useHandSettingsMock = useHandSettings as unknown as ReturnType<typeof vi.fn>;
const useHandStatsMock = useHandStats as unknown as ReturnType<typeof vi.fn>;
const useHandManifestTomlMock = useHandManifestToml as unknown as ReturnType<typeof vi.fn>;
const useActivateHandMock = useActivateHand as unknown as ReturnType<typeof vi.fn>;
const useDeactivateHandMock = useDeactivateHand as unknown as ReturnType<typeof vi.fn>;
const usePauseHandMock = usePauseHand as unknown as ReturnType<typeof vi.fn>;
const useResumeHandMock = useResumeHand as unknown as ReturnType<typeof vi.fn>;
const useUninstallHandMock = useUninstallHand as unknown as ReturnType<typeof vi.fn>;
const useCronJobsMock = useCronJobs as unknown as ReturnType<typeof vi.fn>;

function setMutationDefaults(): void {
  const mut = { mutateAsync: vi.fn().mockResolvedValue(undefined), isPending: false };
  useActivateHandMock.mockReturnValue(mut);
  useDeactivateHandMock.mockReturnValue(mut);
  usePauseHandMock.mockReturnValue(mut);
  useResumeHandMock.mockReturnValue(mut);
  useUninstallHandMock.mockReturnValue(mut);
}

function setSidecarDefaults(): void {
  useActiveHandsMock.mockReturnValue({ data: [], refetch: vi.fn(), isFetching: false });
  useHandStatsBatchMock.mockReturnValue({ data: {} });
  useHandDetailMock.mockReturnValue({ data: undefined });
  useHandSettingsMock.mockReturnValue({ data: {}, isLoading: false });
  useHandStatsMock.mockReturnValue({ data: {} });
  useHandManifestTomlMock.mockReturnValue({ data: undefined, error: null });
  useCronJobsMock.mockReturnValue({ data: [], isLoading: false, refetch: vi.fn() });
}

function renderPage() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  return render(
    <QueryClientProvider client={qc}>
      <HandsPage />
    </QueryClientProvider>,
  );
}

describe("HandsPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setMutationDefaults();
    setSidecarDefaults();
  });

  it("shows the grid skeleton while hands are loading", () => {
    useHandsMock.mockReturnValue({
      data: undefined,
      isLoading: true,
      isFetching: true,
      refetch: vi.fn(),
    });

    const { container } = renderPage();

    // Skeleton grid renders 6 placeholder cards using the bg-linear-to-r
    // shimmer class on each Skeleton motion div. Loading state must NOT
    // show the empty-state text.
    const shimmers = container.querySelectorAll('[class*="bg-linear-to-r"]');
    expect(shimmers.length).toBeGreaterThan(0);
    expect(screen.queryByText("common.no_data")).toBeNull();
  });

  it("renders the empty state when zero hands are installed", () => {
    useHandsMock.mockReturnValue({
      data: [],
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();

    expect(screen.getByText("common.no_data")).toBeInTheDocument();
  });

  it("shows total and active badges in the header", () => {
    useHandsMock.mockReturnValue({
      data: [
        { id: "h1", name: "Alpha", requirements_met: true },
        { id: "h2", name: "Beta", requirements_met: true },
        { id: "h3", name: "Gamma", requirements_met: false },
      ],
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });
    useActiveHandsMock.mockReturnValue({
      data: [{ instance_id: "i1", hand_id: "h1", status: "running" }],
      refetch: vi.fn(),
      isFetching: false,
    });

    renderPage();

    // active count 1, total 3 — both rendered as plain text in the header badges
    // Badge child text is split across nodes (count text node + label text
    // node) inside the same <span>. Use a function matcher against the full
    // node textContent.
    const activeBadge = screen.getByText((_, el) => {
      if (!el || el.tagName !== "SPAN") return false;
      return /^\s*1\s+hands\.active_label\s*$/.test(el.textContent ?? "");
    });
    expect(activeBadge).toBeInTheDocument();
    const totalBadge = screen.getByText((_, el) => {
      if (!el || el.tagName !== "SPAN") return false;
      return /^\s*3\s+hands\.total_label\s*$/.test(el.textContent ?? "");
    });
    expect(totalBadge).toBeInTheDocument();
  });

  it("renders all installed hands as cards", () => {
    useHandsMock.mockReturnValue({
      data: [
        { id: "h1", name: "Alpha", description: "first", requirements_met: true },
        { id: "h2", name: "Beta", description: "second", requirements_met: true },
      ],
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();

    expect(screen.getByText("Alpha")).toBeInTheDocument();
    expect(screen.getByText("Beta")).toBeInTheDocument();
  });

  it("filters hands by search query (substring match against name)", async () => {
    const user = userEvent.setup();
    useHandsMock.mockReturnValue({
      data: [
        { id: "h1", name: "Alpha", requirements_met: true },
        { id: "h2", name: "Beta", requirements_met: true },
      ],
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();

    expect(screen.getByText("Alpha")).toBeInTheDocument();
    expect(screen.getByText("Beta")).toBeInTheDocument();

    const input = screen.getByPlaceholderText("hands.search_placeholder");
    await user.type(input, "alph");

    expect(screen.getByText("Alpha")).toBeInTheDocument();
    expect(screen.queryByText("Beta")).toBeNull();
  });

  it("renders category filter buttons with counts and an 'all' default", () => {
    useHandsMock.mockReturnValue({
      data: [
        { id: "h1", name: "Alpha", category: "data", requirements_met: true },
        { id: "h2", name: "Beta", category: "data", requirements_met: true },
        { id: "h3", name: "Gamma", category: "io", requirements_met: true },
      ],
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();

    // Category buttons: All (3), data (2), io (1)
    expect(screen.getByText("providers.filter_all")).toBeInTheDocument();
    expect(screen.getByText("(3)")).toBeInTheDocument();
    expect(screen.getByText("(2)")).toBeInTheDocument();
    expect(screen.getByText("(1)")).toBeInTheDocument();
  });

  it("filters by category when a category button is clicked", async () => {
    const user = userEvent.setup();
    useHandsMock.mockReturnValue({
      data: [
        { id: "h1", name: "Alpha", category: "data", requirements_met: true },
        { id: "h2", name: "Beta", category: "io", requirements_met: true },
      ],
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();

    expect(screen.getByText("Alpha")).toBeInTheDocument();
    expect(screen.getByText("Beta")).toBeInTheDocument();

    // Click the "data" category button (button text contains "data" + count).
    await user.click(screen.getByRole("button", { name: /data/i }));

    expect(screen.getByText("Alpha")).toBeInTheDocument();
    expect(screen.queryByText("Beta")).toBeNull();
  });

  it("shows the no-matching empty state with a clear-filters action when search has no hits", async () => {
    const user = userEvent.setup();
    useHandsMock.mockReturnValue({
      data: [{ id: "h1", name: "Alpha", requirements_met: true }],
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();

    const input = screen.getByPlaceholderText("hands.search_placeholder");
    await user.type(input, "zzznomatch");

    expect(screen.getByText("agents.no_matching")).toBeInTheDocument();
    // The clear-filters button should be present (it's only rendered when
    // search/category is active and there are zero matches).
    expect(
      screen.getByRole("button", { name: "hands.clear_filters" }),
    ).toBeInTheDocument();
  });

  it("clears search when the clear-filters button is clicked", async () => {
    const user = userEvent.setup();
    useHandsMock.mockReturnValue({
      data: [{ id: "h1", name: "Alpha", requirements_met: true }],
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();

    const input = screen.getByPlaceholderText("hands.search_placeholder") as HTMLInputElement;
    await user.type(input, "zzz");
    expect(input.value).toBe("zzz");

    await user.click(screen.getByRole("button", { name: "hands.clear_filters" }));

    expect((screen.getByPlaceholderText("hands.search_placeholder") as HTMLInputElement).value).toBe("");
    expect(screen.getByText("Alpha")).toBeInTheDocument();
  });

  it("renders the running-now strip when active hands exist", () => {
    useHandsMock.mockReturnValue({
      data: [{ id: "h1", name: "Alpha", requirements_met: true }],
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });
    useActiveHandsMock.mockReturnValue({
      data: [{ instance_id: "i1", hand_id: "h1", status: "running", agent_id: "a1" }],
      refetch: vi.fn(),
      isFetching: false,
    });

    renderPage();

    expect(screen.getByText("hands.running_now")).toBeInTheDocument();
  });

  it("hides the running-now strip when no hands are active", () => {
    useHandsMock.mockReturnValue({
      data: [{ id: "h1", name: "Alpha", requirements_met: true }],
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });
    useActiveHandsMock.mockReturnValue({
      data: [],
      refetch: vi.fn(),
      isFetching: false,
    });

    renderPage();

    expect(screen.queryByText("hands.running_now")).toBeNull();
  });

  it("disables the activate button when requirements are not met", () => {
    useHandsMock.mockReturnValue({
      data: [{ id: "h1", name: "Alpha", requirements_met: false }],
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });

    renderPage();

    const activateBtn = screen.getByRole("button", { name: "hands.activate" });
    expect(activateBtn).toBeDisabled();
  });
});
