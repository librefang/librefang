// Tests for SchedulerPage (refs #3853 — pages/ test gap).
//
// Mocks at the queries/mutations hook layer per the dashboard data-layer
// rule: pages MUST go through `lib/queries` / `lib/mutations`, never
// `fetch()`. We assert the page mounts, surfaces empty/loading branches,
// and wires user interactions to the right mutations.

import { describe, it, expect, vi, beforeEach } from "vitest";
import { act, render, screen, fireEvent, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { SchedulerPage } from "./SchedulerPage";
import { PushDrawer } from "../components/ui/PushDrawer";
import { useDrawerStore } from "../lib/drawerStore";
import { useAgents } from "../lib/queries/agents";
import { useWorkflows } from "../lib/queries/workflows";
import { useSchedules, useTriggers } from "../lib/queries/schedules";
import {
  useCreateSchedule,
  useCreateTrigger,
  useDeleteSchedule,
  useRunSchedule,
  useUpdateSchedule,
  useSetScheduleDeliveryTargets,
  useUpdateTrigger,
  useDeleteTrigger,
} from "../lib/mutations/schedules";

vi.mock("../lib/queries/agents", () => ({
  useAgents: vi.fn(),
}));

vi.mock("../lib/queries/workflows", () => ({
  useWorkflows: vi.fn(),
}));

vi.mock("../lib/queries/schedules", () => ({
  useSchedules: vi.fn(),
  useTriggers: vi.fn(),
}));

vi.mock("../lib/mutations/schedules", () => ({
  useCreateSchedule: vi.fn(),
  useCreateTrigger: vi.fn(),
  useDeleteSchedule: vi.fn(),
  useRunSchedule: vi.fn(),
  useUpdateSchedule: vi.fn(),
  useSetScheduleDeliveryTargets: vi.fn(),
  useUpdateTrigger: vi.fn(),
  useDeleteTrigger: vi.fn(),
}));

const addToast = vi.fn();
vi.mock("../lib/store", () => ({
  useUIStore: (
    selector: (state: {
      addToast: (m: string, t?: string) => void;
    }) => unknown,
  ) => selector({ addToast }),
}));

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>(
    "react-i18next",
  );
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, fallbackOrOpts?: unknown) => {
        if (typeof fallbackOrOpts === "string") return fallbackOrOpts;
        if (
          fallbackOrOpts &&
          typeof fallbackOrOpts === "object" &&
          "defaultValue" in (fallbackOrOpts as Record<string, unknown>)
        ) {
          return String(
            (fallbackOrOpts as Record<string, unknown>).defaultValue,
          );
        }
        return key;
      },
    }),
  };
});

const useAgentsMock = useAgents as unknown as ReturnType<typeof vi.fn>;
const useWorkflowsMock = useWorkflows as unknown as ReturnType<typeof vi.fn>;
const useSchedulesMock = useSchedules as unknown as ReturnType<typeof vi.fn>;
const useTriggersMock = useTriggers as unknown as ReturnType<typeof vi.fn>;
const useCreateScheduleMock = useCreateSchedule as unknown as ReturnType<typeof vi.fn>;
const useCreateTriggerMock = useCreateTrigger as unknown as ReturnType<typeof vi.fn>;
const useDeleteScheduleMock = useDeleteSchedule as unknown as ReturnType<typeof vi.fn>;
const useRunScheduleMock = useRunSchedule as unknown as ReturnType<typeof vi.fn>;
const useUpdateScheduleMock = useUpdateSchedule as unknown as ReturnType<typeof vi.fn>;
const useSetScheduleDeliveryTargetsMock = useSetScheduleDeliveryTargets as unknown as ReturnType<typeof vi.fn>;
const useUpdateTriggerMock = useUpdateTrigger as unknown as ReturnType<typeof vi.fn>;
const useDeleteTriggerMock = useDeleteTrigger as unknown as ReturnType<typeof vi.fn>;

interface QueryShape<T> {
  data: T;
  isLoading: boolean;
  isFetching: boolean;
  isError: boolean;
  refetch: ReturnType<typeof vi.fn>;
}

function makeQuery<T>(
  data: T,
  overrides: Partial<QueryShape<T>> = {},
): QueryShape<T> {
  return {
    data,
    isLoading: false,
    isFetching: false,
    isError: false,
    refetch: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  };
}

function makeMutation(extra: Record<string, unknown> = {}) {
  return {
    mutate: vi.fn(),
    mutateAsync: vi.fn().mockResolvedValue(undefined),
    isPending: false,
    error: null,
    variables: undefined,
    ...extra,
  };
}

function renderPage(opts: { withPushDrawer?: boolean } = {}) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  // PushDrawer is App.tsx's global slot host. Tests that exercise the
  // create-form body or the cron-picker MUST include it, otherwise the
  // DrawerPanel.body is pushed into the store but never mounted to the
  // DOM (and the test can't reach any input inside it).
  return render(
    <QueryClientProvider client={queryClient}>
      <SchedulerPage />
      {opts.withPushDrawer && <PushDrawer />}
    </QueryClientProvider>,
  );
}

const AGENTS = [
  { id: "agent-1", name: "alpha" },
  { id: "agent-2", name: "beta" },
];

const SCHEDULE: import("../api").ScheduleItem = {
  id: "sched-1",
  name: "morning report",
  cron: "0 9 * * *",
  tz: "UTC",
  enabled: true,
  agent_id: "agent-1",
  created_at: "2026-01-01T00:00:00Z",
  delivery_targets: [],
};

const TRIGGER: import("../api").TriggerItem = {
  id: "trig-1",
  agent_id: "agent-1",
  pattern: "lifecycle",
  prompt_template: "lifecycle prompt",
  enabled: true,
  fire_count: 3,
  max_fires: 0,
  target_agent_id: null,
  cooldown_secs: null,
  session_mode: null,
};

describe("SchedulerPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // The push-drawer slot is a process-singleton — reset between tests
    // so a leftover drawer from a prior test can't satisfy a getByText
    // assertion in the next one.
    useDrawerStore.setState({ isOpen: false, content: null });
    useAgentsMock.mockReturnValue(makeQuery(AGENTS));
    useWorkflowsMock.mockReturnValue(makeQuery([]));
    useSchedulesMock.mockReturnValue(makeQuery([SCHEDULE]));
    useTriggersMock.mockReturnValue(makeQuery([TRIGGER]));
    useCreateScheduleMock.mockReturnValue(makeMutation());
    useCreateTriggerMock.mockReturnValue(makeMutation());
    useDeleteScheduleMock.mockReturnValue(makeMutation());
    useRunScheduleMock.mockReturnValue(makeMutation());
    useUpdateScheduleMock.mockReturnValue(makeMutation());
    useSetScheduleDeliveryTargetsMock.mockReturnValue(makeMutation());
    useUpdateTriggerMock.mockReturnValue(makeMutation());
    useDeleteTriggerMock.mockReturnValue(makeMutation());
  });

  it("renders skeleton placeholders while schedule and trigger queries are loading", () => {
    useSchedulesMock.mockReturnValue(makeQuery(undefined, { isLoading: true }));
    useTriggersMock.mockReturnValue(makeQuery(undefined, { isLoading: true }));

    renderPage();

    // Header still mounts.
    expect(screen.getByText("scheduler.title")).toBeInTheDocument();
    // Empty-state copy must NOT be present while loading.
    expect(screen.queryByText("scheduler.no_schedules")).not.toBeInTheDocument();
  });

  it("renders both empty states when schedules and triggers are []", () => {
    useSchedulesMock.mockReturnValue(makeQuery([]));
    useTriggersMock.mockReturnValue(makeQuery([]));

    renderPage();

    expect(screen.getByText("scheduler.no_schedules")).toBeInTheDocument();
    expect(screen.getByText("common.no_data")).toBeInTheDocument();
    // Stat badges show zero counts.
    expect(screen.getByText(/0 scheduler\.schedules/)).toBeInTheDocument();
    expect(screen.getByText(/0 scheduler\.triggers_label/)).toBeInTheDocument();
  });

  it("renders schedule and trigger rows with agent name and pattern label", () => {
    renderPage();

    expect(screen.getByText("morning report")).toBeInTheDocument();
    // cron expression rendered in monospace span.
    expect(screen.getByText("0 9 * * *")).toBeInTheDocument();
    // agent name resolves via agentMap.
    expect(screen.getByText("alpha")).toBeInTheDocument();
    // trigger fire count summary.
    expect(screen.getByText(/Fired:\s*3/)).toBeInTheDocument();
  });

  it("toggles a schedule's enabled flag via useUpdateSchedule", () => {
    const mutate = vi.fn();
    useUpdateScheduleMock.mockReturnValue(makeMutation({ mutate }));

    renderPage();

    // The active-state pill is the only button labeled common.active in the
    // schedule row — clicking flips enabled to false.
    const activeBtns = screen.getAllByText("common.active");
    fireEvent.click(activeBtns[0]);

    expect(mutate).toHaveBeenCalledWith({
      id: "sched-1",
      data: { enabled: false },
    });
  });

  it("requires confirm-then-click before calling useDeleteSchedule", () => {
    const mutateAsync = vi.fn().mockResolvedValue(undefined);
    useDeleteScheduleMock.mockReturnValue(makeMutation({ mutateAsync }));

    renderPage();

    // Before any click there should be NO confirm buttons in the page.
    expect(screen.queryByText("common.confirm")).not.toBeInTheDocument();

    // Locate the trash button by its lucide-trash2 svg ancestor. There are
    // two trash buttons (one per schedule row, one per trigger row); the
    // schedule's is first in DOM order.
    const trashIcons = document.querySelectorAll("svg.lucide-trash-2");
    expect(trashIcons.length).toBeGreaterThanOrEqual(2);
    const scheduleTrashBtn = trashIcons[0].closest("button") as HTMLButtonElement;
    fireEvent.click(scheduleTrashBtn);
    // First click only flips confirm state — mutation not called yet.
    expect(mutateAsync).not.toHaveBeenCalled();

    // After first click, a Confirm button appears.
    const confirmBtn = screen.getByText("common.confirm");
    fireEvent.click(confirmBtn);
    expect(mutateAsync).toHaveBeenCalledWith("sched-1");
  });

  it("invokes useRunSchedule when the play button is clicked on an enabled schedule", () => {
    const mutate = vi.fn();
    useRunScheduleMock.mockReturnValue(makeMutation({ mutate }));

    renderPage();

    // The Play button is the first action button in the schedule row.
    // Disambiguate by finding the schedule's row container.
    const scheduleCard = screen.getByText("morning report").closest("div")!
      .parentElement!;
    const buttons = within(scheduleCard).getAllByRole("button");
    // Order in row: [active toggle, run, trash]. Run is index 1.
    fireEvent.click(buttons[1]);

    expect(mutate).toHaveBeenCalledWith("sched-1");
  });

  it("toggles a trigger via useUpdateTrigger including its agentId", () => {
    const mutate = vi.fn();
    useUpdateTriggerMock.mockReturnValue(makeMutation({ mutate }));

    renderPage();

    // There are 2 common.active pills (schedule + trigger). The trigger
    // pill is the second.
    const activeBtns = screen.getAllByText("common.active");
    expect(activeBtns.length).toBe(2);
    fireEvent.click(activeBtns[1]);

    expect(mutate).toHaveBeenCalledWith({
      id: "trig-1",
      data: { enabled: false },
      agentId: "agent-1",
    });
  });

  it("renders disabled-row styling and OFF pill for a disabled schedule", () => {
    useSchedulesMock.mockReturnValue(
      makeQuery([{ ...SCHEDULE, enabled: false }]),
    );

    renderPage();

    // Disabled schedule renders the OFF pill (defaultValue) instead of
    // common.active.
    expect(screen.getByText("OFF")).toBeInTheDocument();
    expect(screen.queryByText("common.active")).toBeInTheDocument();
    // common.active still appears for the trigger row, but not for the
    // schedule row — sanity check by counting.
    expect(screen.getAllByText("common.active").length).toBe(1);
  });

  // Regression test for #5247 — clicking the cron-expression chip
  // inside the Create-schedule form opens the ScheduleModal picker
  // without tearing the surrounding create-form drawer down.
  //
  // Root cause: the picker was a DrawerPanel nested inside the
  // create-form's DrawerPanel body. Both push to the single global
  // push-slot. As soon as the inner DrawerPanel claimed the slot,
  // PushDrawer stopped rendering the outer body — which unmounted
  // the inner one — and the slot collapsed back to the empty state.
  // The visible symptom matched the issue text exactly: "while trying
  // to click on Cron Expression the window closed automatically and
  // no option to pick expression".
  //
  // Fix: ScheduleModal now uses Modal (fixed overlay), not DrawerPanel.
  // The picker no longer competes for the push-slot, and it survives
  // independently of whichever DrawerPanel hosted the button that
  // opened it.
  it("opens the cron-picker without closing the create-schedule form (#5247)", () => {
    renderPage({ withPushDrawer: true });

    // 1. Open the create-job drawer. The header button is the only
    //    one labelled scheduler.create_job at this point.
    fireEvent.click(screen.getByText("scheduler.create_job"));

    // 2. The create form mounted inside the drawer body, including
    //    the cron-expression chip button. The cron text appears in
    //    both the existing schedule row AND the picker chip inside
    //    the drawer — find the chip specifically (it's the only one
    //    whose closest button is also the picker chip).
    const cronTextNodes = screen.getAllByText("0 9 * * *");
    const cronChip = cronTextNodes
      .map((n) => n.closest("button"))
      .find((b) => b !== null) as HTMLButtonElement | undefined;
    expect(cronChip).toBeDefined();

    // 3. Click the chip — this used to collapse the entire create
    //    drawer (#5247). With the fix, the create form remains
    //    visible AND the picker appears.
    act(() => {
      fireEvent.click(cronChip!);
    });

    // 4. The picker's signature element — the timezone field label —
    //    must be in the DOM. (The picker emits its own
    //    scheduler.timezone label with the "Timezone" defaultValue.)
    //    Modal's AnimatePresence may double-mount during the test's
    //    fake animation lifecycle in jsdom, so accept ≥1.
    expect(screen.getAllByText("Timezone").length).toBeGreaterThanOrEqual(1);

    // 5. Crucially, the create drawer's body must STILL be visible.
    //    Before the fix, the inner DrawerPanel claimed the global
    //    push-slot, PushDrawer stopped rendering the outer body, and
    //    the chip we clicked vanished from the DOM.
    //
    //    The Mode tab labels live ONLY in the create drawer body —
    //    if the drawer collapsed, the create-form's copy of this
    //    label disappears and we drop from 2 occurrences (page-level
    //    section header + create-form Mode tab) to 1.
    expect(screen.getAllByText("scheduler.event_triggers").length).toBeGreaterThanOrEqual(2);
    // Two occurrences of the cron text now expected: the schedule
    // list row AND the create-form chip we just clicked.
    expect(screen.getAllByText("0 9 * * *").length).toBeGreaterThanOrEqual(2);
  });
});
