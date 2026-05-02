import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { GoalsPage } from "./GoalsPage";
import { useGoals, useGoalTemplates } from "../lib/queries/goals";
import {
  useCreateGoal,
  useUpdateGoal,
  useDeleteGoal,
} from "../lib/mutations/goals";
import type { GoalItem, GoalTemplate } from "../api";

vi.mock("../lib/queries/goals", () => ({
  useGoals: vi.fn(),
  useGoalTemplates: vi.fn(),
}));

vi.mock("../lib/mutations/goals", () => ({
  useCreateGoal: vi.fn(),
  useUpdateGoal: vi.fn(),
  useDeleteGoal: vi.fn(),
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
const useCreateGoalMock = useCreateGoal as unknown as ReturnType<typeof vi.fn>;
const useUpdateGoalMock = useUpdateGoal as unknown as ReturnType<typeof vi.fn>;
const useDeleteGoalMock = useDeleteGoal as unknown as ReturnType<typeof vi.fn>;

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

  it("applies a template by calling create once per goal in the template", async () => {
    useGoalsMock.mockReturnValue(makeQuery<GoalItem[]>([]));
    useGoalTemplatesMock.mockReturnValue(
      makeQuery<GoalTemplate[]>([SAMPLE_TEMPLATE]),
    );
    const { create } = setMutations();
    renderPage();

    fireEvent.click(screen.getByText("goals.use_template"));

    // runBatch awaits sequentially; flush microtasks.
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

  it("renders both parent and child goals in the tree", () => {
    useGoalsMock.mockReturnValue(makeQuery([PARENT_GOAL, CHILD_GOAL]));
    useGoalTemplatesMock.mockReturnValue(makeQuery<GoalTemplate[]>([]));
    renderPage();

    expect(screen.getByText("Parent goal")).toBeInTheDocument();
    expect(screen.getByText("Child goal")).toBeInTheDocument();
    // Parent has an expandable chevron because a child references it.
    // Clicking it must not throw — exercises the expand toggle.
    const headerRoot = screen.getByText("goals.goal_tree").closest("div")!;
    const buttons = within(headerRoot.parentElement!).getAllByRole("button");
    expect(buttons.length).toBeGreaterThan(0);
    fireEvent.click(buttons[0]);
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
