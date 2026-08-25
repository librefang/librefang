import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  GoalsPage,
  buildGoalRows,
  goalStatusBadgeVariant,
  progressForGoalStatus,
  runIndependentBatch,
} from "./GoalsPage";
import { useGoals, useGoalTemplates, useGoalRun } from "../lib/queries/goals";
import {
  useCreateGoal,
  useUpdateGoal,
  useDeleteGoal,
  useStartGoalRun,
  useStopGoalRun,
} from "../lib/mutations/goals";
import type { GoalItem, GoalTemplate } from "../api";

vi.mock("../lib/queries/goals", () => ({
  useGoals: vi.fn(),
  useGoalTemplates: vi.fn(),
  useGoalRun: vi.fn(),
}));

vi.mock("../lib/mutations/goals", () => ({
  useCreateGoal: vi.fn(),
  useUpdateGoal: vi.fn(),
  useDeleteGoal: vi.fn(),
  useStartGoalRun: vi.fn(),
  useStopGoalRun: vi.fn(),
}));

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>(
    "react-i18next",
  );
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, opts?: Record<string, unknown>) =>
        opts ? `${key}:${JSON.stringify(opts)}` : key,
    }),
  };
});

const useGoalsMock = useGoals as unknown as ReturnType<typeof vi.fn>;
const useGoalTemplatesMock = useGoalTemplates as unknown as ReturnType<typeof vi.fn>;
const useGoalRunMock = useGoalRun as unknown as ReturnType<typeof vi.fn>;
const useCreateGoalMock = useCreateGoal as unknown as ReturnType<typeof vi.fn>;
const useUpdateGoalMock = useUpdateGoal as unknown as ReturnType<typeof vi.fn>;
const useDeleteGoalMock = useDeleteGoal as unknown as ReturnType<typeof vi.fn>;
const useStartGoalRunMock = useStartGoalRun as unknown as ReturnType<typeof vi.fn>;
const useStopGoalRunMock = useStopGoalRun as unknown as ReturnType<typeof vi.fn>;

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

function setMutations(opts: {
  create?: ReturnType<typeof vi.fn>;
  update?: ReturnType<typeof vi.fn>;
  del?: ReturnType<typeof vi.fn>;
  createPending?: boolean;
} = {}): {
  create: ReturnType<typeof vi.fn>;
  update: ReturnType<typeof vi.fn>;
  del: ReturnType<typeof vi.fn>;
} {
  const create = opts.create ?? vi.fn().mockResolvedValue({ id: "new" });
  const update = opts.update ?? vi.fn().mockResolvedValue({ id: "u" });
  const del = opts.del ?? vi.fn().mockResolvedValue(undefined);
  useCreateGoalMock.mockReturnValue({
    mutateAsync: create,
    isPending: opts.createPending ?? false,
  });
  useUpdateGoalMock.mockReturnValue({ mutateAsync: update, isPending: false });
  useDeleteGoalMock.mockReturnValue({ mutateAsync: del, isPending: false });
  useStartGoalRunMock.mockReturnValue({ mutateAsync: vi.fn(), isPending: false });
  useStopGoalRunMock.mockReturnValue({ mutateAsync: vi.fn(), isPending: false });
  return { create, update, del };
}

function renderPage(): void {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  render(
    <QueryClientProvider client={qc}>
      <GoalsPage />
    </QueryClientProvider>,
  );
}

const SAMPLE_TEMPLATE: GoalTemplate = {
  id: "tpl-rocket",
  name: "Launch",
  icon: "rocket",
  description: "Bootstrap an agent",
  goals: [
    { title: "Define mission", description: "", status: "pending" },
    { title: "Pick a model", description: "", status: "pending" },
  ],
};

const PARENT_GOAL: GoalItem = {
  id: "g-parent",
  title: "Parent goal",
  description: "the root",
  status: "in_progress",
  progress: 50,
};

const CHILD_GOAL: GoalItem = {
  id: "g-child",
  title: "Child goal",
  parent_id: "g-parent",
  status: "pending",
  progress: 0,
};

const COMPLETED_GOAL: GoalItem = {
  id: "g-done",
  title: "Finished goal",
  status: "completed",
  progress: 100,
};

describe("GoalsPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setMutations();
    // GoalRunControl calls useGoalRun for every rendered goal; default to an
    // idle (no active run) query so the control renders its start button.
    useGoalRunMock.mockReturnValue(makeQuery({ running: false }));
  });

  it("renders the loading skeleton while goals are fetching", () => {
    useGoalsMock.mockReturnValue(makeQuery(undefined, { isLoading: true }));
    useGoalTemplatesMock.mockReturnValue(makeQuery([]));
    renderPage();

    // Header still renders even during the loading branch.
    expect(screen.getByText("goals.title")).toBeInTheDocument();
    // KPI/total label is not rendered while skeleton is shown.
    expect(screen.queryByText("goals.total")).not.toBeInTheDocument();
  });

  it("renders the template picker empty-state when there are no goals", () => {
    useGoalsMock.mockReturnValue(makeQuery<GoalItem[]>([]));
    useGoalTemplatesMock.mockReturnValue(
      makeQuery<GoalTemplate[]>([SAMPLE_TEMPLATE]),
    );
    renderPage();

    expect(screen.getByText("goals.pick_template")).toBeInTheDocument();
    expect(screen.getByText("Launch")).toBeInTheDocument();
    expect(screen.getByText("Define mission")).toBeInTheDocument();
    expect(screen.getByText("goals.use_template")).toBeInTheDocument();
  });

  // #6654: a failed load and an empty daemon are different things.
  // The server now answers a goals storage failure with a 500 rather than an empty page, but the page had no error branch — the query yielded no goals and the template picker rendered over data that had failed to load, telling the operator to start from scratch.
  it("renders the error state, not the template picker, when the goals query fails", () => {
    useGoalsMock.mockReturnValue(
      makeQuery<GoalItem[] | undefined>(undefined, { isError: true }),
    );
    useGoalTemplatesMock.mockReturnValue(
      makeQuery<GoalTemplate[]>([SAMPLE_TEMPLATE]),
    );
    renderPage();

    expect(screen.getByRole("alert")).toBeInTheDocument();
    expect(screen.getByText("goals.loadError")).toBeInTheDocument();
    expect(screen.queryByText("goals.pick_template")).not.toBeInTheDocument();
    expect(screen.queryByText("goals.use_template")).not.toBeInTheDocument();
  });

  it("retries the goals query from the error state", () => {
    const query = makeQuery<GoalItem[] | undefined>(undefined, {
      isError: true,
    });
    useGoalsMock.mockReturnValue(query);
    useGoalTemplatesMock.mockReturnValue(makeQuery<GoalTemplate[]>([]));
    renderPage();

    fireEvent.click(within(screen.getByRole("alert")).getByRole("button"));
    expect(query.refetch).toHaveBeenCalled();
  });

  it("applies a template by calling create once per goal in the template", async () => {
    useGoalsMock.mockReturnValue(makeQuery<GoalItem[]>([]));
    useGoalTemplatesMock.mockReturnValue(
      makeQuery<GoalTemplate[]>([SAMPLE_TEMPLATE]),
    );
    const { create } = setMutations();
    renderPage();

    fireEvent.click(screen.getByText("goals.use_template"));

    // Flush the allSettled batch.
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();

    expect(create).toHaveBeenCalledTimes(SAMPLE_TEMPLATE.goals.length);
    expect(create.mock.calls[0][0]).toMatchObject({ title: "Define mission" });
    expect(create.mock.calls[1][0]).toMatchObject({ title: "Pick a model" });
  });

  it("renders KPI totals derived from goals.status", () => {
    useGoalsMock.mockReturnValue(
      makeQuery([PARENT_GOAL, CHILD_GOAL, COMPLETED_GOAL]),
    );
    useGoalTemplatesMock.mockReturnValue(makeQuery<GoalTemplate[]>([]));
    renderPage();

    // 1 completed of 3 goals = 33%.
    expect(screen.getByText("33%")).toBeInTheDocument();
    // Goal tree heading appears once goals exist.
    expect(screen.getByText("goals.goal_tree")).toBeInTheDocument();
  });

  it("submits the create form via useCreateGoal with the typed title", async () => {
    useGoalsMock.mockReturnValue(makeQuery([PARENT_GOAL]));
    useGoalTemplatesMock.mockReturnValue(makeQuery<GoalTemplate[]>([]));
    const { create } = setMutations();
    renderPage();

    const titleInput = screen.getByPlaceholderText(
      "goals.goal_title_placeholder",
    ) as HTMLInputElement;
    fireEvent.change(titleInput, { target: { value: "  Ship release  " } });

    // Submit button label is goals.create_goal; pick the actual <button>.
    const submitBtn = screen
      .getAllByText("goals.create_goal")
      .map((el) => el.closest("button"))
      .find((b): b is HTMLButtonElement => !!b && b.type === "submit");
    expect(submitBtn).toBeTruthy();
    fireEvent.click(submitBtn!);

    await Promise.resolve();

    expect(create).toHaveBeenCalledTimes(1);
    expect(create.mock.calls[0][0]).toMatchObject({
      title: "  Ship release  ",
      status: "pending",
    });
  });

  it("omits blank parent_id / agent_id from the create payload (#6562)", async () => {
    useGoalsMock.mockReturnValue(makeQuery([PARENT_GOAL]));
    useGoalTemplatesMock.mockReturnValue(makeQuery<GoalTemplate[]>([]));
    const { create } = setMutations();
    renderPage();

    const titleInput = screen.getByPlaceholderText(
      "goals.goal_title_placeholder",
    ) as HTMLInputElement;
    fireEvent.change(titleInput, { target: { value: "No parent" } });

    const submitBtn = screen
      .getAllByText("goals.create_goal")
      .map((el) => el.closest("button"))
      .find((b): b is HTMLButtonElement => !!b && b.type === "submit");
    fireEvent.click(submitBtn!);

    await Promise.resolve();

    expect(create).toHaveBeenCalledTimes(1);
    const payload = create.mock.calls[0][0] as Record<string, unknown>;
    // `parent_id: ""` used to reach the backend and fail its parent-existence check with "Parent goal '' not found"; `agent_id: ""` persisted an unparsable assignment that broke the goal runner's start route.
    expect(payload).not.toHaveProperty("parent_id");
    expect(payload).not.toHaveProperty("agent_id");
  });

  it("does not submit the create form when the title is whitespace-only", () => {
    useGoalsMock.mockReturnValue(makeQuery([PARENT_GOAL]));
    useGoalTemplatesMock.mockReturnValue(makeQuery<GoalTemplate[]>([]));
    const { create } = setMutations();
    renderPage();

    const submitBtn = screen
      .getAllByText("goals.create_goal")
      .map((el) => el.closest("button"))
      .find((b): b is HTMLButtonElement => !!b && b.type === "submit");
    expect(submitBtn).toBeDisabled();
    expect(create).not.toHaveBeenCalled();
  });

  it("cycles status pending -> in_progress -> completed via the status icon button", async () => {
    const pendingGoal: GoalItem = {
      id: "g-p",
      title: "Pending",
      status: "pending",
      progress: 0,
    };
    useGoalsMock.mockReturnValue(makeQuery([pendingGoal]));
    useGoalTemplatesMock.mockReturnValue(makeQuery<GoalTemplate[]>([]));
    const { update } = setMutations();
    renderPage();

    // Status toggle button has title=goals.toggle_reset.
    const toggle = screen.getByTitle("goals.toggle_reset");
    fireEvent.click(toggle);
    await Promise.resolve();

    expect(update).toHaveBeenCalledTimes(1);
    expect(update.mock.calls[0][0]).toEqual({
      id: "g-p",
      data: { status: "in_progress", progress: 50 },
    });
  });

  it("requires a confirm click before useDeleteGoal fires", async () => {
    useGoalsMock.mockReturnValue(makeQuery([PARENT_GOAL]));
    useGoalTemplatesMock.mockReturnValue(makeQuery<GoalTemplate[]>([]));
    const { del } = setMutations();
    renderPage();

    // First click only puts the row into delete-confirm state.
    fireEvent.click(screen.getByTitle("common.delete"));
    expect(del).not.toHaveBeenCalled();
    expect(screen.getByText("goals.delete_confirm")).toBeInTheDocument();

    // Now click the confirm button.
    fireEvent.click(screen.getByText("common.confirm"));
    await Promise.resolve();

    expect(del).toHaveBeenCalledWith("g-parent");
  });

  it("cancelling the delete confirmation prevents useDeleteGoal from firing", () => {
    useGoalsMock.mockReturnValue(makeQuery([PARENT_GOAL]));
    useGoalTemplatesMock.mockReturnValue(makeQuery<GoalTemplate[]>([]));
    const { del } = setMutations();
    renderPage();

    fireEvent.click(screen.getByTitle("common.delete"));
    fireEvent.click(screen.getByText("common.cancel"));

    expect(del).not.toHaveBeenCalled();
    expect(screen.queryByText("goals.delete_confirm")).not.toBeInTheDocument();
  });

  it("hides collapsed descendants and reveals them when expanded", () => {
    useGoalsMock.mockReturnValue(makeQuery([PARENT_GOAL, CHILD_GOAL]));
    useGoalTemplatesMock.mockReturnValue(makeQuery<GoalTemplate[]>([]));
    renderPage();

    expect(screen.getByText("Parent goal")).toBeInTheDocument();
    expect(screen.queryByText("Child goal")).not.toBeInTheDocument();

    const parentRow = screen.getByText("Parent goal").closest("div.rounded-xl");
    expect(parentRow).toBeTruthy();
    fireEvent.click(within(parentRow as HTMLElement).getAllByRole("button")[0]);
    expect(screen.getByText("Child goal")).toBeInTheDocument();
  });

  it("entering edit mode and saving calls useUpdateGoal with the edited draft", async () => {
    useGoalsMock.mockReturnValue(makeQuery([PARENT_GOAL]));
    useGoalTemplatesMock.mockReturnValue(makeQuery<GoalTemplate[]>([]));
    const { update } = setMutations();
    renderPage();

    fireEvent.click(screen.getByTitle("common.edit"));

    // The edit form pre-fills the title from goal.title.
    const titleInput = screen.getByDisplayValue("Parent goal") as HTMLInputElement;
    fireEvent.change(titleInput, { target: { value: "Renamed parent" } });

    fireEvent.click(screen.getByText("common.save"));
    await Promise.resolve();

    expect(update).toHaveBeenCalledTimes(1);
    expect(update.mock.calls[0][0]).toMatchObject({
      id: "g-parent",
      data: expect.objectContaining({ title: "Renamed parent" }),
    });
  });
});

describe("GoalsPage helpers", () => {
  it("builds only visible tree rows with child depth", () => {
    expect(buildGoalRows([PARENT_GOAL, CHILD_GOAL], {})).toEqual([
      { goal: PARENT_GOAL, depth: 0, hasChildren: true },
    ]);
    expect(
      buildGoalRows([PARENT_GOAL, CHILD_GOAL], { "g-parent": true }),
    ).toEqual([
      { goal: PARENT_GOAL, depth: 0, hasChildren: true },
      { goal: CHILD_GOAL, depth: 1, hasChildren: false },
    ]);
  });

  it("starts independent batch actions before waiting for settlement", async () => {
    let releaseFirst: () => void = () => undefined;
    const first = new Promise<void>((resolve) => {
      releaseFirst = resolve;
    });
    const failure = new Error("second failed");
    const action = vi.fn((item: number) =>
      item === 1 ? first : Promise.reject(failure),
    );

    const pending = runIndependentBatch([1, 2], action);
    expect(action).toHaveBeenCalledTimes(2);
    releaseFirst();

    await expect(pending).resolves.toEqual({
      total: 2,
      succeeded: 1,
      failed: 1,
      errors: [failure],
    });
  });

  it("derives progress and badge variants without nested status branches", () => {
    expect(progressForGoalStatus("completed", 12)).toBe(100);
    expect(progressForGoalStatus("in_progress", 12)).toBe(50);
    expect(progressForGoalStatus("in_progress", 80)).toBe(80);
    expect(progressForGoalStatus("pending", 80)).toBe(0);
    expect(goalStatusBadgeVariant("completed")).toBe("success");
    expect(goalStatusBadgeVariant("in_progress")).toBe("warning");
    expect(goalStatusBadgeVariant("pending")).toBe("default");
  });
});
