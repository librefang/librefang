import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { WorkflowsPage } from "./WorkflowsPage";
import {
  useWorkflows,
  useWorkflowDetail,
  useWorkflowRuns,
  useWorkflowRunDetail,
  useWorkflowTemplates,
  usePendingOperatorRuns,
  useWorkflowOperatorPause,
} from "../lib/queries/workflows";
import {
  useRunWorkflow,
  useDryRunWorkflow,
  useDeleteWorkflow,
  useInstantiateTemplate,
  useResolveOperatorStep,
} from "../lib/mutations/workflows";
import { useCreateSchedule } from "../lib/mutations/schedules";

vi.mock("../lib/queries/workflows", () => ({
  useWorkflows: vi.fn(),
  useWorkflowDetail: vi.fn(),
  useWorkflowRuns: vi.fn(),
  useWorkflowRunDetail: vi.fn(),
  useWorkflowTemplates: vi.fn(),
  // HITL operator-step hooks (#4977) — the banner + action bar mounted
  // from WorkflowsPage import them, so the mock must expose every
  // symbol or the page crashes at module load.
  usePendingOperatorRuns: vi.fn(),
  useWorkflowOperatorPause: vi.fn(),
}));

vi.mock("../lib/mutations/workflows", () => ({
  useRunWorkflow: vi.fn(),
  useDryRunWorkflow: vi.fn(),
  useDeleteWorkflow: vi.fn(),
  useInstantiateTemplate: vi.fn(),
  // Resolution mutation pulled in by the OperatorActionBar — same
  // reason as above.
  useResolveOperatorStep: vi.fn(),
}));

vi.mock("../lib/mutations/schedules", () => ({
  useCreateSchedule: vi.fn(),
}));

const navigateMock = vi.fn();
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => navigateMock,
}));

const addToastMock = vi.fn();
vi.mock("../lib/store", () => ({
  useUIStore: (selector: (state: { addToast: typeof addToastMock }) => unknown) =>
    selector({ addToast: addToastMock }),
}));

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>(
    "react-i18next",
  );
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, opts?: { defaultValue?: string }) =>
        opts?.defaultValue ?? key,
      i18n: { language: "en" },
    }),
  };
});

const useWorkflowsMock = useWorkflows as unknown as ReturnType<typeof vi.fn>;
const useWorkflowDetailMock = useWorkflowDetail as unknown as ReturnType<typeof vi.fn>;
const useWorkflowRunsMock = useWorkflowRuns as unknown as ReturnType<typeof vi.fn>;
const useWorkflowRunDetailMock = useWorkflowRunDetail as unknown as ReturnType<typeof vi.fn>;
const useWorkflowTemplatesMock = useWorkflowTemplates as unknown as ReturnType<typeof vi.fn>;
const usePendingOperatorRunsMock =
  usePendingOperatorRuns as unknown as ReturnType<typeof vi.fn>;
const useWorkflowOperatorPauseMock =
  useWorkflowOperatorPause as unknown as ReturnType<typeof vi.fn>;
const useRunWorkflowMock = useRunWorkflow as unknown as ReturnType<typeof vi.fn>;
const useDryRunWorkflowMock = useDryRunWorkflow as unknown as ReturnType<typeof vi.fn>;
const useDeleteWorkflowMock = useDeleteWorkflow as unknown as ReturnType<typeof vi.fn>;
const useInstantiateTemplateMock = useInstantiateTemplate as unknown as ReturnType<typeof vi.fn>;
const useResolveOperatorStepMock =
  useResolveOperatorStep as unknown as ReturnType<typeof vi.fn>;
const useCreateScheduleMock = useCreateSchedule as unknown as ReturnType<typeof vi.fn>;

interface QueryShape<T> {
  data: T;
  isLoading: boolean;
  isFetching: boolean;
  isSuccess: boolean;
  isError: boolean;
  refetch: ReturnType<typeof vi.fn>;
}

function makeQuery<T>(data: T, overrides: Partial<QueryShape<T>> = {}): QueryShape<T> {
  return {
    data,
    isLoading: false,
    isFetching: false,
    isSuccess: data !== undefined,
    isError: false,
    refetch: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  };
}

interface MutationShape {
  mutateAsync: ReturnType<typeof vi.fn>;
  reset: ReturnType<typeof vi.fn>;
  isPending: boolean;
  data: unknown;
  error: unknown;
}

function makeMutation(overrides: Partial<MutationShape> = {}): MutationShape {
  return {
    mutateAsync: vi.fn().mockResolvedValue(undefined),
    reset: vi.fn(),
    isPending: false,
    data: undefined,
    error: undefined,
    ...overrides,
  };
}

function setMutationDefaults(): {
  run: MutationShape;
  dryRun: MutationShape;
  del: MutationShape;
  inst: MutationShape;
  sched: MutationShape;
} {
  const run = makeMutation();
  const dryRun = makeMutation();
  const del = makeMutation();
  const inst = makeMutation();
  const sched = makeMutation();
  useRunWorkflowMock.mockReturnValue(run);
  useDryRunWorkflowMock.mockReturnValue(dryRun);
  useDeleteWorkflowMock.mockReturnValue(del);
  useInstantiateTemplateMock.mockReturnValue(inst);
  useCreateScheduleMock.mockReturnValue(sched);
  return { run, dryRun, del, inst, sched };
}

function renderPage(): void {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <WorkflowsPage />
    </QueryClientProvider>,
  );
}

const sampleWorkflow = {
  id: "wf-1",
  name: "alpha-flow",
  description: "Alpha description",
  steps: 3,
  created_at: "2026-01-01T00:00:00Z",
};

describe("WorkflowsPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setMutationDefaults();
    useWorkflowDetailMock.mockReturnValue(makeQuery(undefined));
    useWorkflowRunsMock.mockReturnValue(makeQuery([]));
    useWorkflowRunDetailMock.mockReturnValue(makeQuery(undefined));
    useWorkflowTemplatesMock.mockReturnValue(makeQuery([]));
    // #4977: default the operator banner to an empty worklist so the
    // existing assertions ("no banner rendered") still hold. Individual
    // tests can override to surface pending rows.
    usePendingOperatorRunsMock.mockReturnValue(makeQuery([]));
    useWorkflowOperatorPauseMock.mockReturnValue(makeQuery(undefined));
    useResolveOperatorStepMock.mockReturnValue(makeMutation());
  });

  it("renders loading skeleton while workflows query is loading", () => {
    useWorkflowsMock.mockReturnValue(
      makeQuery(undefined, { isLoading: true, isFetching: true, isSuccess: false }),
    );
    renderPage();
    // Header still mounts with the workflows title.
    expect(screen.getByText("workflows.title")).toBeInTheDocument();
    // No workflow rows can render yet.
    expect(screen.queryByText("alpha-flow")).not.toBeInTheDocument();
  });

  it("auto-switches to templates tab when there are no workflows", () => {
    useWorkflowsMock.mockReturnValue(makeQuery([]));
    useWorkflowTemplatesMock.mockReturnValue(
      makeQuery([
        {
          id: "tpl-1",
          name: "Sample Template",
          description: "demo",
          category: "creation",
          steps: [{ name: "s1", prompt_template: "hi" }],
        },
      ]),
    );
    renderPage();
    // Templates tab content surfaces the template card.
    expect(screen.getByText("Sample Template")).toBeInTheDocument();
    // Templates tab is selected.
    const templatesTab = screen.getByRole("tab", { name: /workflows.template_library/ });
    expect(templatesTab).toHaveAttribute("aria-selected", "true");
  });

  it("renders workflow rows from the query data", () => {
    useWorkflowsMock.mockReturnValue(makeQuery([sampleWorkflow]));
    renderPage();
    expect(screen.getByText("alpha-flow")).toBeInTheDocument();
    expect(screen.getByText("Alpha description")).toBeInTheDocument();
  });

  it("shows the empty state when the user flips back to the workflows tab with no flows", () => {
    useWorkflowsMock.mockReturnValue(makeQuery([]));
    useWorkflowTemplatesMock.mockReturnValue(makeQuery([]));
    renderPage();
    // Auto-switch landed us on Templates; click back to "My Workflows"
    // to surface the EmptyState that lives inside the workflows panel.
    fireEvent.click(screen.getByRole("tab", { name: /workflows.my_workflows/ }));
    expect(screen.getByText("workflows.empty_title")).toBeInTheDocument();
  });

  it("calls runMutation.mutateAsync with the selected workflow id and input on Run", async () => {
    useWorkflowsMock.mockReturnValue(makeQuery([sampleWorkflow]));
    const mutations = setMutationDefaults();
    renderPage();

    // The run textarea is the only textarea on the page.
    const textarea = screen.getByPlaceholderText("canvas.run_input_placeholder");
    fireEvent.change(textarea, { target: { value: "hello" } });

    fireEvent.click(screen.getByText("canvas.run_now"));

    expect(mutations.run.mutateAsync).toHaveBeenCalledTimes(1);
    expect(mutations.run.mutateAsync).toHaveBeenCalledWith({
      workflowId: "wf-1",
      input: "hello",
    });
  });

  it("requires a second click to confirm delete and only then calls the mutation", () => {
    useWorkflowsMock.mockReturnValue(makeQuery([sampleWorkflow]));
    const mutations = setMutationDefaults();
    renderPage();

    // First click on the trash icon arms the confirmation.
    const trashBtn = screen.getByLabelText("common.delete");
    fireEvent.click(trashBtn);
    expect(mutations.del.mutateAsync).not.toHaveBeenCalled();

    // Confirm now visible — clicking it issues the mutation.
    fireEvent.click(screen.getByText("common.confirm"));
    expect(mutations.del.mutateAsync).toHaveBeenCalledWith("wf-1");
  });

  it("filters workflow rows by the search query", () => {
    useWorkflowsMock.mockReturnValue(
      makeQuery([
        sampleWorkflow,
        { id: "wf-2", name: "beta-flow", description: "beta", created_at: "2026-01-02" },
      ]),
    );
    renderPage();

    const search = screen.getByPlaceholderText("workflows.search_placeholder");
    fireEvent.change(search, { target: { value: "beta" } });

    expect(screen.queryByText("alpha-flow")).not.toBeInTheDocument();
    expect(screen.getByText("beta-flow")).toBeInTheDocument();
  });

  it("instantiates a template without required params and navigates to canvas", async () => {
    useWorkflowsMock.mockReturnValue(makeQuery([]));
    useWorkflowTemplatesMock.mockReturnValue(
      makeQuery([
        {
          id: "tpl-1",
          name: "ParamlessTpl",
          steps: [{ name: "s1", prompt_template: "hi" }],
          parameters: [],
        },
      ]),
    );
    const mutations = setMutationDefaults();
    mutations.inst.mutateAsync.mockResolvedValue({ workflow_id: "wf-new" });

    renderPage();

    // The Use template button drives instantiation.
    fireEvent.click(screen.getByText("Use template"));

    expect(mutations.inst.mutateAsync).toHaveBeenCalledWith({ id: "tpl-1", params: {} });
  });

  it("opens the canvas without persisting when previewing a template", () => {
    useWorkflowsMock.mockReturnValue(makeQuery([]));
    useWorkflowTemplatesMock.mockReturnValue(
      makeQuery([
        {
          id: "tpl-2",
          name: "PreviewTpl",
          steps: [
            { name: "a", prompt_template: "p1" },
            { name: "b", prompt_template: "p2", depends_on: ["a"] },
          ],
        },
      ]),
    );
    const mutations = setMutationDefaults();
    renderPage();

    // The preview button uses the Eye icon — find it as the second button
    // inside the template card footer (the first is "Use template").
    const previewButtons = screen.getAllByTitle("Preview in canvas");
    fireEvent.click(previewButtons[0]);

    // Preview must NOT call instantiate — it only stores in sessionStorage
    // and navigates.
    expect(mutations.inst.mutateAsync).not.toHaveBeenCalled();
    expect(navigateMock).toHaveBeenCalled();
    // Verify the template was stashed in sessionStorage for the canvas.
    const stored = sessionStorage.getItem("workflowTemplate");
    expect(stored).toContain("PreviewTpl");
  });

  it("renders parameter form fields when workflow detail has template placeholders", () => {
    useWorkflowsMock.mockReturnValue(makeQuery([sampleWorkflow]));
    useWorkflowDetailMock.mockReturnValue(
      makeQuery({
        ...sampleWorkflow,
        steps: [
          { name: "step1", prompt_template: "Summarize {{topic}} for {{audience}}" },
        ],
      }),
    );
    renderPage();

    // Parameter fields should be rendered with labels.
    expect(screen.getByText("topic")).toBeInTheDocument();
    expect(screen.getByText("audience")).toBeInTheDocument();
    // The textarea should show the "additional context" placeholder
    // when parameters are present.
    expect(
      screen.getByPlaceholderText("Additional context (optional)..."),
    ).toBeInTheDocument();
  });

  it("does not render parameter fields when workflow has no template placeholders", () => {
    useWorkflowsMock.mockReturnValue(makeQuery([sampleWorkflow]));
    useWorkflowDetailMock.mockReturnValue(
      makeQuery({
        ...sampleWorkflow,
        steps: [
          { name: "step1", prompt_template: "Do the thing with {{input}}" },
        ],
      }),
    );
    renderPage();

    // {{input}} is a reserved variable — should not become a form field.
    expect(screen.queryByText("Parameters")).not.toBeInTheDocument();
    expect(
      screen.getByPlaceholderText("canvas.run_input_placeholder"),
    ).toBeInTheDocument();
  });

  it("excludes step output variable names from detected parameters", () => {
    useWorkflowsMock.mockReturnValue(makeQuery([sampleWorkflow]));
    useWorkflowDetailMock.mockReturnValue(
      makeQuery({
        ...sampleWorkflow,
        steps: [
          { name: "research", prompt_template: "Research {{topic}}" },
          { name: "summarize", prompt_template: "Summarize {{research}} for {{audience}}" },
        ],
      }),
    );
    renderPage();

    // "topic" and "audience" should be rendered as parameter fields.
    expect(screen.getByText("topic")).toBeInTheDocument();
    expect(screen.getByText("audience")).toBeInTheDocument();
    // "research" is a step name (output var) — should NOT appear as a
    // parameter field label.  The description hints mention step names but
    // never as a standalone label element.
    const paramSection = screen.getByText("Parameters").parentElement!;
    const labels = paramSection.querySelectorAll("label > span");
    const labelTexts = Array.from(labels).map((el) => el.textContent?.replace("*", "").trim());
    expect(labelTexts).toContain("topic");
    expect(labelTexts).toContain("audience");
    expect(labelTexts).not.toContain("research");
  });

  it("sends param values as a structured object so {{var}} binds at run time", async () => {
    useWorkflowsMock.mockReturnValue(makeQuery([sampleWorkflow]));
    useWorkflowDetailMock.mockReturnValue(
      makeQuery({
        ...sampleWorkflow,
        steps: [
          { name: "step1", prompt_template: "Tell me about {{topic}}" },
        ],
      }),
    );
    const mutations = setMutationDefaults();
    renderPage();

    // Fill in the parameter field.
    const topicInput = screen.getByPlaceholderText("Parameter 'topic' used in step 'step1'");
    fireEvent.change(topicInput, { target: { value: "quantum computing" } });

    fireEvent.click(screen.getByText("canvas.run_now"));

    expect(mutations.run.mutateAsync).toHaveBeenCalledTimes(1);
    const callArgs = mutations.run.mutateAsync.mock.calls[0][0];
    expect(callArgs.workflowId).toBe("wf-1");
    // Filled params are sent as an object keyed by parameter name so the
    // backend's seed_input_vars_from_json binds `{{topic}}` at run time
    // (not folded into a free-text `{{input}}` blob). No free-text was
    // typed, so there is no `input` key.
    expect(callArgs.input).toEqual({ topic: "quantum computing" });
  });

  it("filters templates by the active category pill", () => {
    useWorkflowsMock.mockReturnValue(makeQuery([]));
    useWorkflowTemplatesMock.mockReturnValue(
      makeQuery([
        { id: "t-a", name: "AlphaTpl", category: "creation", steps: [] },
        { id: "t-b", name: "BetaTpl", category: "thinking", steps: [] },
      ]),
    );
    renderPage();

    // Both render under the default "all" filter.
    expect(screen.getByText("AlphaTpl")).toBeInTheDocument();
    expect(screen.getByText("BetaTpl")).toBeInTheDocument();

    // Click the "thinking" category pill — both pill labels render lowercase.
    fireEvent.click(screen.getByRole("button", { name: /thinking/i }));

    expect(screen.queryByText("AlphaTpl")).not.toBeInTheDocument();
    expect(screen.getByText("BetaTpl")).toBeInTheDocument();
  });

  // ----- HITL operator-step banner (#4977) -----

  it("does not render the pending-operator banner when the worklist is empty", () => {
    useWorkflowsMock.mockReturnValue(makeQuery([sampleWorkflow]));
    usePendingOperatorRunsMock.mockReturnValue(makeQuery([]));
    renderPage();
    expect(
      screen.queryByText(/awaiting operator review/i),
    ).not.toBeInTheDocument();
  });

  it("renders the pending-operator banner with row counts when the worklist has entries", () => {
    useWorkflowsMock.mockReturnValue(makeQuery([sampleWorkflow]));
    usePendingOperatorRunsMock.mockReturnValue(
      makeQuery([
        {
          run_id: "11111111-1111-1111-1111-111111111111",
          workflow_id: "wf-1",
          workflow_name: "alpha-flow",
          step_name: "review_draft",
          operator_step_index: 1,
          artifact: "draft body",
          actions: ["approve", "reject"],
          started_at: "2026-05-01T12:00:00Z",
          paused_at: "2026-05-01T12:00:30Z",
        },
      ]),
    );
    renderPage();
    // The banner header reflects the row count.
    expect(
      screen.getByText(/1 workflow run awaiting operator review/i),
    ).toBeInTheDocument();
    // The row surfaces the step name + action count badge.
    expect(screen.getByText("review_draft")).toBeInTheDocument();
    expect(screen.getByText(/2 actions/i)).toBeInTheDocument();
  });

  // Regression for #4977 review: the Rust `WorkflowRunState::Paused {…}`
  // variant serialises to `{paused: {…}}` (externally-tagged struct
  // variant) — not the bare string `"paused"`. The OperatorActionBar
  // mount guard must accept the object form, or every real paused run
  // silently fails to surface the resolution UI.
  it("mounts the operator action bar when the run detail state is a tagged Paused object", () => {
    useWorkflowsMock.mockReturnValue(makeQuery([sampleWorkflow]));
    // Run-list row labelled distinctly from the workflow row so the
    // click target is unambiguous.
    useWorkflowRunsMock.mockReturnValue(
      makeQuery([
        {
          id: "run-paused-1",
          workflow_name: "paused-run-row",
          state: { paused: { resume_token_hash: "h", reason: "operator step", paused_at: "2026-05-01T12:00:30Z" } },
          steps_completed: 1,
          started_at: "2026-05-01T12:00:00Z",
        },
      ]),
    );
    useWorkflowRunDetailMock.mockReturnValue(
      makeQuery({
        id: "run-paused-1",
        workflow_id: "wf-1",
        workflow_name: "paused-run-row",
        input: "seed",
        // Real wire shape from `serde_json::to_value(&run.state)` for the
        // `WorkflowRunState::Paused` struct variant.
        state: { paused: { resume_token_hash: "h", reason: "operator step", paused_at: "2026-05-01T12:00:30Z" } },
        started_at: "2026-05-01T12:00:00Z",
        step_results: [],
      }),
    );
    // Stub the inspect query so the bar resolves to a renderable pause —
    // confirms the *mount* happened and the data flowed through.
    useWorkflowOperatorPauseMock.mockReturnValue(
      makeQuery({
        run_id: "run-paused-1",
        workflow_id: "wf-1",
        workflow_name: "paused-run-row",
        step_name: "review_draft",
        operator_step_index: 0,
        artifact: "the draft",
        actions: ["approve"],
        started_at: "2026-05-01T12:00:00Z",
        paused_at: "2026-05-01T12:00:30Z",
      }),
    );
    renderPage();
    // Select the paused run in the Run History list to reveal the
    // OperatorActionBar inside the inline run detail panel.
    fireEvent.click(screen.getByText("paused-run-row"));
    // The bar's hallmark copy renders only when the mount guard passed
    // AND the inspect query produced a pause.
    expect(screen.getByText(/Operator review required/i)).toBeInTheDocument();
    expect(screen.getByText("the draft")).toBeInTheDocument();
  });
});
