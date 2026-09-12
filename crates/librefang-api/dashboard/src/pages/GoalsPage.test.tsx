import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  GoalsPage,
  GoalRunPhaseBadge,
  buildGoalRows,
  goalStatusBadgeVariant,
  progressForGoalStatus,
  runIndependentBatch,
} from "./GoalsPage";
import { useAgents } from "../lib/queries/agents";
import { useGoals, useGoalTemplates, useGoalRun } from "../lib/queries/goals";
import {
  useCreateGoal,
  useUpdateGoal,
  useDeleteGoal,
  useStartGoalRun,
  useStopGoalRun,
  usePauseGoalRun,
  useResumeGoalRun,
} from "../lib/mutations/goals";
import type { AgentItem, GoalItem, GoalTemplate } from "../api";

vi.mock("../lib/queries/agents", () => ({
  useAgents: vi.fn(),
}));

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
  usePauseGoalRun: vi.fn(),
  useResumeGoalRun: vi.fn(),
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

const useAgentsMock = useAgents as unknown as ReturnType<typeof vi.fn>;
const useGoalsMock = useGoals as unknown as ReturnType<typeof vi.fn>;
const useGoalTemplatesMock = useGoalTemplates as unknown as ReturnType<typeof vi.fn>;
const useGoalRunMock = useGoalRun as unknown as ReturnType<typeof vi.fn>;
const useCreateGoalMock = useCreateGoal as unknown as ReturnType<typeof vi.fn>;
const useUpdateGoalMock = useUpdateGoal as unknown as ReturnType<typeof vi.fn>;
const useDeleteGoalMock = useDeleteGoal as unknown as ReturnType<typeof vi.fn>;
const useStartGoalRunMock = useStartGoalRun as unknown as ReturnType<typeof vi.fn>;
const useStopGoalRunMock = useStopGoalRun as unknown as ReturnType<typeof vi.fn>;
const usePauseGoalRunMock = usePauseGoalRun as unknown as ReturnType<typeof vi.fn>;
const useResumeGoalRunMock = useResumeGoalRun as unknown as ReturnType<typeof vi.fn>;

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
  usePauseGoalRunMock.mockReturnValue({ mutateAsync: vi.fn(), isPending: false });
  useResumeGoalRunMock.mockReturnValue({ mutateAsync: vi.fn(), isPending: false });
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

const AGENTS: AgentItem[] = [
  { id: "a-worker", name: "worker" },
  { id: "a-reviewer", name: "reviewer" },
];

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
    useAgentsMock.mockReturnValue(makeQuery<AgentItem[]>(AGENTS));
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
    // Same rule for the loop-engineering ids: the backend rejects a non-UUID
    // verify_agent_id outright, and `""` is not a UUID.
    expect(payload).not.toHaveProperty("verify_agent_id");
    expect(payload).not.toHaveProperty("evaluator_model");
  });

  // Loop engineering is opt-in, so the controls that configure it stay out of
  // the way until it is switched on — and a goal that never switches it on
  // must say so explicitly rather than omitting the field.
  it("reveals the verifier and evaluator controls only once loop engineering is ticked", () => {
    useGoalsMock.mockReturnValue(makeQuery([PARENT_GOAL]));
    useGoalTemplatesMock.mockReturnValue(makeQuery<GoalTemplate[]>([]));
    renderPage();

    expect(
      screen.queryByPlaceholderText("goals.evaluator_model_placeholder"),
    ).not.toBeInTheDocument();

    fireEvent.click(screen.getByLabelText("goals.loop_engineering"));

    expect(
      screen.getByPlaceholderText("goals.evaluator_model_placeholder"),
    ).toBeInTheDocument();
    expect(screen.getByText("goals.no_verifier_selected")).toBeInTheDocument();
  });

  it("sends the loop-engineering configuration on create", async () => {
    useGoalsMock.mockReturnValue(makeQuery([PARENT_GOAL]));
    useGoalTemplatesMock.mockReturnValue(makeQuery<GoalTemplate[]>([]));
    const { create } = setMutations();
    renderPage();

    fireEvent.change(
      screen.getByPlaceholderText("goals.goal_title_placeholder"),
      { target: { value: "Verified goal" } },
    );
    fireEvent.click(screen.getByLabelText("goals.loop_engineering"));
    // The verifier is picked from the agent list, not typed: a hand-typed id
    // is how a goal ends up storing something the run route has to reject.
    fireEvent.change(screen.getByLabelText("goals.verifier_agent"), {
      target: { value: "a-reviewer" },
    });
    fireEvent.change(
      screen.getByPlaceholderText("goals.evaluator_model_placeholder"),
      { target: { value: "haiku" } },
    );

    const submitBtn = screen
      .getAllByText("goals.create_goal")
      .map((el) => el.closest("button"))
      .find((b): b is HTMLButtonElement => !!b && b.type === "submit");
    fireEvent.click(submitBtn!);

    await Promise.resolve();

    expect(create).toHaveBeenCalledTimes(1);
    expect(create.mock.calls[0][0]).toMatchObject({
      title: "Verified goal",
      loop_engineering: true,
      verify_agent_id: "a-reviewer",
      evaluator_model: "haiku",
    });
  });

  it("marks a loop-engineered goal in the tree and leaves a plain one unmarked", () => {
    useGoalTemplatesMock.mockReturnValue(makeQuery<GoalTemplate[]>([]));

    useGoalsMock.mockReturnValue(makeQuery([PARENT_GOAL]));
    const plain = render(
      <QueryClientProvider
        client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
      >
        <GoalsPage />
      </QueryClientProvider>,
    );
    // The badge carries the hint as its title, which the checkbox label does
    // not — so this identifies the tree marker and nothing else.
    expect(
      plain.queryByTitle("goals.loop_engineering_hint"),
    ).not.toBeInTheDocument();
    plain.unmount();

    useGoalsMock.mockReturnValue(
      makeQuery([{ ...PARENT_GOAL, loop_engineering: true }]),
    );
    renderPage();
    expect(screen.getByTitle("goals.loop_engineering_hint")).toBeInTheDocument();
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

  // #8108: GoalRunInfo used to be gated on `status !== "completed"`, but
  // `GoalRunPhase::Finished` only occurs once the goal is already completed —
  // so the finished badge could never render.
  it("shows the finished run badge on a completed goal (#8108)", () => {
    const completedWithRun: GoalItem = { ...COMPLETED_GOAL, agent_id: "a1" };
    useGoalsMock.mockReturnValue(makeQuery([completedWithRun]));
    useGoalTemplatesMock.mockReturnValue(makeQuery<GoalTemplate[]>([]));
    useGoalRunMock.mockReturnValue(
      makeQuery({
        running: false,
        run: {
          goal_id: completedWithRun.id,
          agent_id: "a1",
          phase: "finished",
          iteration: 3,
          max_iterations: 10,
          last_progress: 100,
          started_at: "",
          updated_at: "",
        },
      }),
    );
    renderPage();

    expect(
      screen.getByText(
        'goals.run_phase_finished:{"defaultValue":"finished"}',
      ),
    ).toBeInTheDocument();
    expect(screen.getByText("3/10")).toBeInTheDocument();
  });

  const RUNNING_RUN = {
    goal_id: "g-r",
    agent_id: "a1",
    phase: "running",
    iteration: 2,
    max_iterations: 10,
    last_progress: 20,
    started_at: "",
    updated_at: "",
  } as const;

  it("fires usePauseGoalRun from the pause button on a running goal", async () => {
    const goalWithAgent: GoalItem = { ...PARENT_GOAL, agent_id: "a1" };
    useGoalsMock.mockReturnValue(makeQuery([goalWithAgent]));
    useGoalTemplatesMock.mockReturnValue(makeQuery<GoalTemplate[]>([]));
    useGoalRunMock.mockReturnValue(makeQuery({ running: true, run: RUNNING_RUN }));
    const pause = vi.fn().mockResolvedValue({});
    usePauseGoalRunMock.mockReturnValue({ mutateAsync: pause, isPending: false });
    renderPage();

    fireEvent.click(screen.getByTitle("goals.run_pause"));
    await Promise.resolve();

    expect(pause).toHaveBeenCalledWith("g-parent");
  });

  it("fires useResumeGoalRun from the resume button on a paused run", async () => {
    const goalWithAgent: GoalItem = { ...PARENT_GOAL, agent_id: "a1" };
    useGoalsMock.mockReturnValue(makeQuery([goalWithAgent]));
    useGoalTemplatesMock.mockReturnValue(makeQuery<GoalTemplate[]>([]));
    useGoalRunMock.mockReturnValue(
      makeQuery({ running: false, run: { ...RUNNING_RUN, phase: "paused" } }),
    );
    const resume = vi.fn().mockResolvedValue(undefined);
    useResumeGoalRunMock.mockReturnValue({ mutateAsync: resume, isPending: false });
    renderPage();

    fireEvent.click(screen.getByTitle("goals.run_resume"));
    await Promise.resolve();

    expect(resume).toHaveBeenCalledWith("g-parent");
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

// #8067 review: the badge must reuse the already-translated `goals.run_phase_*`
// keys, and an unknown phase must render honestly rather than as a confident "Stopped".
describe("GoalRunPhaseBadge", () => {
  const API_PHASES = [
    "running",
    "paused",
    "finished",
    "max_iterations_reached",
    "rate_limited",
    "stopped",
  ] as const;

  it("renders each API-emittable phase from the existing translated goals.run_phase_* keys", () => {
    render(
      <div>
        {API_PHASES.map((phase) => (
          <GoalRunPhaseBadge key={phase} phase={phase} />
        ))}
      </div>,
    );

    for (const phase of API_PHASES) {
      expect(
        screen.getByText(
          `goals.run_phase_${phase}:{"defaultValue":"${phase.replace(/_/g, " ")}"}`,
        ),
      ).toBeInTheDocument();
    }
  });

  // `Badge` draws its own dot whenever `dot` is passed, so a known phase that
  // also carried an icon showed a coloured dot AND a lucide glyph before its
  // label. The two are mutually exclusive now: icon for a known phase, dot for
  // the unknown one, which has nothing else to lead with.
  it("leads a known phase with one glyph, not a dot and an icon", () => {
    const { container } = render(<GoalRunPhaseBadge phase="running" />);
    const badge = container.querySelector("span.inline-flex");
    expect(badge).not.toBeNull();

    // `Badge`'s dot is the only `aria-hidden` span it renders.
    expect(badge!.querySelectorAll("span[aria-hidden='true']")).toHaveLength(0);
    expect(badge!.querySelectorAll("svg")).toHaveLength(1);
    // No `mr-*` on the icon: `Badge`'s own `gap-1.5` already spaces every child,
    // and a second margin made the icon→label gap disagree with the dot→label one.
    expect(badge!.querySelector("svg")!.getAttribute("class")).not.toMatch(/\bmr-/);
  });

  // "paused" reached the switch's `default` arm until this PR, so it rendered with the same neutral styling as a phase the dashboard had never heard of.
  // Asserting the variant is what separates the two: without it the badge passes whether or not `paused` has an arm of its own.
  it("gives paused its own warning variant and icon rather than the unknown-phase fallback", () => {
    const { container } = render(<GoalRunPhaseBadge phase="paused" />);
    const badge = container.querySelector("span.inline-flex")!;
    expect(badge.className).toContain("bg-warning/10");
    expect(badge.className).not.toContain("bg-main");
    // A known phase leads with its icon and drops `Badge`'s dot.
    expect(badge.querySelectorAll("svg")).toHaveLength(1);
    expect(badge.querySelectorAll("span[aria-hidden='true']")).toHaveLength(0);
  });

  it("renders an unknown phase under the neutral variant with its own key, not a confident Stopped", () => {
    // This used to render "paused", which was unknown to the switch until this PR gave it its own arm.
    // The case being guarded is a phase the daemon emits before the dashboard has learned it, so the example has to be one the switch still does not know.
    const { container } = render(<GoalRunPhaseBadge phase="quiescing" />);

    // The label is asked of i18n by the phase's own key with the raw phase as
    // the fallback, so a locale that gains `run_phase_paused` starts using it
    // with no code change. The previous shape gated translation on a hardcoded
    // `labelKey` per phase, so an unknown phase could never pick one up.
    // (`t` is mocked here as `key:{options}`; in production this renders the
    // translation when the key exists and "paused" when it does not.)
    expect(
      screen.getByText('goals.run_phase_quiescing:{"defaultValue":"quiescing"}'),
    ).toBeInTheDocument();
    expect(screen.queryByText(/goals\.run_phase_stopped/)).not.toBeInTheDocument();

    // Assert the variant itself, not just the text: the classes `Badge` applies
    // for `default` (Badge.tsx:13). Without this the test passed under any
    // variant, including the `error` styling of a phase it does not know.
    const badge = container.querySelector("span.inline-flex")!;
    expect(badge.className).toContain("bg-main");
    expect(badge.className).toContain("text-text-dim");
    // The unknown branch is the one that keeps the dot, having no icon.
    expect(badge.querySelectorAll("span[aria-hidden='true']")).toHaveLength(1);
    expect(badge.querySelectorAll("svg")).toHaveLength(0);
  });
});

// The duplicate-badge regression the standalone suite above cannot reach: it
// renders `GoalRunPhaseBadge` directly, while the bug was that the *page*
// rendered the phase twice in one row — once in `GoalRunControl`'s action
// cluster and once in `GoalRunInfo` below it. Only `renderPage()` sees both.
describe("GoalsPage run phase is rendered once per row", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setMutations();
    useGoalTemplatesMock.mockReturnValue(makeQuery<GoalTemplate[]>([]));
  });

  // `in_progress`, deliberately: `GoalRunControl` is gated on
  // `status !== "completed"`, so a completed goal renders only one of the two
  // sites and could never have caught this.
  it("shows a running goal's phase exactly once, not once per render site", () => {
    const runningGoal: GoalItem = { ...PARENT_GOAL, agent_id: "a1" };
    useGoalsMock.mockReturnValue(makeQuery([runningGoal]));
    useGoalRunMock.mockReturnValue(
      makeQuery({
        running: true,
        run: {
          goal_id: runningGoal.id,
          agent_id: "a1",
          phase: "running",
          iteration: 3,
          max_iterations: 10,
          last_progress: 30,
          started_at: "",
          updated_at: "",
        },
      }),
    );
    renderPage();

    expect(
      screen.getAllByText('goals.run_phase_running:{"defaultValue":"running"}'),
    ).toHaveLength(1);
    // The iteration count has one home too — it used to appear in the badge and
    // again in the info row.
    expect(screen.getAllByText("3/10")).toHaveLength(1);
  });

  it("shows a stopped goal's phase exactly once", () => {
    const stoppedGoal: GoalItem = { ...PARENT_GOAL, agent_id: "a1" };
    useGoalsMock.mockReturnValue(makeQuery([stoppedGoal]));
    useGoalRunMock.mockReturnValue(
      makeQuery({
        running: false,
        run: {
          goal_id: stoppedGoal.id,
          agent_id: "a1",
          phase: "stopped",
          iteration: 4,
          max_iterations: 10,
          last_progress: 40,
          started_at: "",
          updated_at: "",
        },
      }),
    );
    renderPage();

    expect(
      screen.getAllByText('goals.run_phase_stopped:{"defaultValue":"stopped"}'),
    ).toHaveLength(1);
  });
});
