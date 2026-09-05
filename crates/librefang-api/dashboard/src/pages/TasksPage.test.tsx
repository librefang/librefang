import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { TasksPage } from "./TasksPage";
import { useTaskQueue, useTaskQueueStatus } from "../lib/queries/runtime";
import { useAgents } from "../lib/queries/agents";
import {
  useCreateTask,
  useUpdateTaskStatus,
  useDeleteTask,
  useRetryTask,
} from "../lib/mutations/runtime";

// ── Module mocks ────────────────────────────────────────────────────────────

vi.mock("../lib/queries/runtime", () => ({
  useTaskQueue: vi.fn(),
  useTaskQueueStatus: vi.fn(),
}));

vi.mock("../lib/queries/agents", () => ({
  useAgents: vi.fn(),
}));

vi.mock("../lib/mutations/runtime", () => ({
  useCreateTask: vi.fn(),
  useUpdateTaskStatus: vi.fn(),
  useDeleteTask: vi.fn(),
  useRetryTask: vi.fn(),
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

// ── Helpers ─────────────────────────────────────────────────────────────────

const m = <T,>(fn: T) => fn as unknown as ReturnType<typeof vi.fn>;

const useTaskQueueMock       = m(useTaskQueue);
const useTaskQueueStatusMock = m(useTaskQueueStatus);
const useAgentsMock          = m(useAgents);
const useCreateTaskMock      = m(useCreateTask);
const useUpdateTaskStatusMock = m(useUpdateTaskStatus);
const useDeleteTaskMock      = m(useDeleteTask);
const useRetryTaskMock       = m(useRetryTask);

function makeQuery<T>(data: T, overrides: Record<string, unknown> = {}) {
  return {
    data,
    isLoading: false,
    isFetching: false,
    isError: false,
    isSuccess: data !== undefined,
    refetch: vi.fn().mockResolvedValue({ data, isSuccess: true, isError: false }),
    ...overrides,
  };
}

function makeMutation(overrides: Record<string, unknown> = {}) {
  return {
    mutate: vi.fn(),
    mutateAsync: vi.fn().mockResolvedValue(undefined),
    isPending: false,
    isSuccess: false,
    isError: false,
    data: undefined,
    error: null,
    ...overrides,
  };
}

const SAMPLE_TASKS = [
  {
    id: "task-pending-1",
    status: "pending",
    title: "Pending task title",
    description: "Pending task description",
    assigned_to: "agent-alpha",
    created_by: "operator",
    created_at: new Date(Date.now() - 60_000).toISOString(),
  },
  {
    id: "task-inprogress-1",
    status: "in_progress",
    title: "Running task",
    description: "This task is running",
    assigned_to: "agent-beta",
    created_by: "operator",
    claimed_at: new Date(Date.now() - 30_000).toISOString(),
    created_at: new Date(Date.now() - 120_000).toISOString(),
  },
  {
    id: "task-completed-1",
    status: "completed",
    title: "Done task",
    description: "This one finished",
    assigned_to: "agent-alpha",
    created_by: "operator",
    result: "Success output text",
    completed_at: new Date().toISOString(),
    created_at: new Date(Date.now() - 300_000).toISOString(),
  },
  {
    id: "task-failed-1",
    status: "failed",
    title: "Failed task",
    description: "This one failed",
    assigned_to: "agent-beta",
    created_by: "operator",
    result: "Error: something went wrong",
    created_at: new Date(Date.now() - 600_000).toISOString(),
  },
];

const SAMPLE_STATUS = { total: 4, pending: 1, in_progress: 1, completed: 1, failed: 1 };

// The agent registry, which is what the pickers are built from. `SAMPLE_TASKS`
// stores assignees by *name* — the pre-existing spelling — while the picker
// posts ids, so these fixtures together cover both stored forms.
const SAMPLE_AGENTS = [
  { id: "11111111-1111-4111-8111-111111111111", name: "agent-alpha" },
  { id: "22222222-2222-4222-8222-222222222222", name: "agent-beta" },
  { id: "33333333-3333-4333-8333-333333333333", name: "agent-gamma" },
];
const ALPHA_ID = SAMPLE_AGENTS[0].id;

function setQueryDefaults() {
  useTaskQueueMock.mockReturnValue(makeQuery({ tasks: SAMPLE_TASKS, total: SAMPLE_TASKS.length }));
  useTaskQueueStatusMock.mockReturnValue(makeQuery(SAMPLE_STATUS));
  useAgentsMock.mockReturnValue(makeQuery(SAMPLE_AGENTS));
}

function setMutationDefaults() {
  useCreateTaskMock.mockReturnValue(makeMutation());
  useUpdateTaskStatusMock.mockReturnValue(makeMutation());
  useDeleteTaskMock.mockReturnValue(makeMutation());
  useRetryTaskMock.mockReturnValue(makeMutation());
}

function renderPage() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={qc}>
      <TasksPage />
    </QueryClientProvider>,
  );
}

// ── Tests ────────────────────────────────────────────────────────────────────

describe("TasksPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setQueryDefaults();
    setMutationDefaults();
  });

  describe("column rendering", () => {
    it("renders all four status columns", () => {
      renderPage();
      expect(screen.getByText("tasks.col_pending")).toBeInTheDocument();
      expect(screen.getByText("tasks.col_in_progress")).toBeInTheDocument();
      expect(screen.getByText("tasks.col_completed")).toBeInTheDocument();
      expect(screen.getByText("tasks.col_failed")).toBeInTheDocument();
    });

    it("places task cards in the correct column based on status", () => {
      renderPage();
      expect(screen.getByText("Pending task title")).toBeInTheDocument();
      expect(screen.getByText("Running task")).toBeInTheDocument();
      expect(screen.getByText("Done task")).toBeInTheDocument();
      expect(screen.getByText("Failed task")).toBeInTheDocument();
    });

    it("shows task description in the card", () => {
      renderPage();
      expect(screen.getByText("Pending task description")).toBeInTheDocument();
    });

    it("shows assignee badge when assigned_to is set", () => {
      renderPage();
      // Multiple tasks are assigned to agent-alpha and agent-beta
      expect(screen.getAllByText("agent-alpha").length).toBeGreaterThan(0);
    });

    it("shows result preview for completed and failed tasks", () => {
      renderPage();
      expect(screen.getByText("Success output text")).toBeInTheDocument();
      expect(screen.getByText("Error: something went wrong")).toBeInTheDocument();
    });

    it("hides the Cancelled column when there are no cancelled tasks", () => {
      renderPage();
      expect(screen.queryByText("tasks.col_cancelled")).not.toBeInTheDocument();
    });

    it("shows the Cancelled column when there are cancelled tasks", () => {
      useTaskQueueMock.mockReturnValue(
        makeQuery({
          tasks: [
            ...SAMPLE_TASKS,
            { id: "task-cancelled-1", status: "cancelled", title: "Cancelled one", description: "Was cancelled", created_at: new Date().toISOString() },
          ],
          total: SAMPLE_TASKS.length + 1,
        }),
      );
      renderPage();
      expect(screen.getByText("tasks.col_cancelled")).toBeInTheDocument();
      expect(screen.getByText("Cancelled one")).toBeInTheDocument();
    });
  });

  describe("drag to re-queue", () => {
    function withCancelledTask() {
      useTaskQueueMock.mockReturnValue(
        makeQuery({
          tasks: [
            ...SAMPLE_TASKS,
            { id: "task-cancelled-1", status: "cancelled", title: "Cancelled one", description: "Was cancelled", created_at: new Date().toISOString() },
          ],
          total: SAMPLE_TASKS.length + 1,
        }),
      );
    }

    function fakeDataTransfer() {
      const store: Record<string, string> = {};
      return {
        setData: vi.fn((k: string, v: string) => { store[k] = v; }),
        getData: vi.fn((k: string) => store[k] ?? ""),
        effectAllowed: "",
        dropEffect: "",
      } as unknown as DataTransfer;
    }

    it("populates dataTransfer on dragStart (Firefox requires it)", () => {
      withCancelledTask();
      renderPage();
      const card = screen.getByText("Cancelled one").closest("[draggable='true']");
      expect(card).not.toBeNull();
      const dt = fakeDataTransfer();
      fireEvent.dragStart(card as Element, { dataTransfer: dt });
      // Without setData here the drag is a silent no-op in Firefox.
      expect(dt.setData).toHaveBeenCalledWith("text/plain", "task-cancelled-1");
    });

    it("re-queues a cancelled task when dropped on the Pending column", () => {
      withCancelledTask();
      const mutate = vi.fn();
      useUpdateTaskStatusMock.mockReturnValue(makeMutation({ mutate }));
      renderPage();
      const card = screen.getByText("Cancelled one").closest("[draggable='true']");
      const pendingColumn = screen.getByText("tasks.col_pending").closest("[data-column='pending']");
      expect(pendingColumn).not.toBeNull();
      const dt = fakeDataTransfer();
      fireEvent.dragStart(card as Element, { dataTransfer: dt });
      fireEvent.dragOver(pendingColumn as Element, { dataTransfer: dt });
      fireEvent.drop(pendingColumn as Element, { dataTransfer: dt });
      expect(mutate).toHaveBeenCalledWith(
        expect.objectContaining({ id: "task-cancelled-1", status: "pending" }),
        expect.objectContaining({ onError: expect.any(Function) }),
      );
    });
  });

  describe("status summary bar", () => {
    it("renders status counters", () => {
      renderPage();
      expect(screen.getByText("tasks.status_total")).toBeInTheDocument();
      expect(screen.getByText("tasks.status_pending")).toBeInTheDocument();
      expect(screen.getByText("tasks.status_failed")).toBeInTheDocument();
    });

    it("derives counters from the same task list as the columns", () => {
      useTaskQueueStatusMock.mockReturnValue(
        makeQuery({ total: 999, pending: 999, in_progress: 999, completed: 999, failed: 999 }),
      );
      renderPage();
      const totalLabel = screen.getByText("tasks.status_total");
      expect(totalLabel.previousElementSibling).toHaveTextContent("4");
    });

    it("gives the icon-only refresh control an accessible name", () => {
      renderPage();
      expect(screen.getAllByRole("button", { name: "common.refresh" }).length).toBeGreaterThan(0);
    });
  });

  describe("New Task modal", () => {
    it("opens the modal when the New Task button is clicked", () => {
      renderPage();
      fireEvent.click(screen.getByRole("button", { name: /tasks.new_task/i }));
      expect(screen.getByText("tasks.modal_title")).toBeInTheDocument();
    });

    it("calls createTask mutation when the form is submitted with valid data", async () => {
      const mutate = vi.fn();
      useCreateTaskMock.mockReturnValue(makeMutation({ mutate }));

      renderPage();
      fireEvent.click(screen.getByRole("button", { name: /tasks.new_task/i }));

      // Fill in the form
      fireEvent.change(screen.getByPlaceholderText("tasks.field_title_placeholder"), {
        target: { value: "My new task" },
      });
      fireEvent.change(screen.getByPlaceholderText("tasks.field_description_placeholder"), {
        target: { value: "Task description here" },
      });

      fireEvent.click(screen.getByRole("button", { name: "tasks.submit" }));

      await waitFor(() => {
        expect(mutate).toHaveBeenCalledWith(
          expect.objectContaining({
            title: "My new task",
            description: "Task description here",
          }),
        );
      });
    });

    it("disables the submit button when title or description is empty", () => {
      renderPage();
      fireEvent.click(screen.getByRole("button", { name: /tasks.new_task/i }));

      const submitBtn = screen.getByRole("button", { name: "tasks.submit" });
      expect(submitBtn).toBeDisabled();

      // Fill only title
      fireEvent.change(screen.getByPlaceholderText("tasks.field_title_placeholder"), {
        target: { value: "Only title" },
      });
      expect(submitBtn).toBeDisabled();
    });
  });

  describe("per-card action buttons", () => {
    it("shows Cancel button for pending tasks and calls updateTaskStatus with 'cancelled'", async () => {
      const mutate = vi.fn();
      useUpdateTaskStatusMock.mockReturnValue(makeMutation({ mutate }));

      renderPage();

      // Find the cancel button in the pending card
      const cancelBtns = screen.getAllByRole("button", { name: /tasks.action_cancel/i });
      fireEvent.click(cancelBtns[0]);

      await waitFor(() => {
        expect(mutate).toHaveBeenCalledWith(
          expect.objectContaining({ id: "task-pending-1", status: "cancelled" }),
          expect.objectContaining({ onError: expect.any(Function) }),
        );
      });
    });

    it("shows Retry button for failed tasks and calls retryTask mutation", async () => {
      const mutate = vi.fn();
      useRetryTaskMock.mockReturnValue(makeMutation({ mutate }));

      renderPage();

      const retryBtn = screen.getByRole("button", { name: /tasks.action_retry/i });
      fireEvent.click(retryBtn);

      await waitFor(() => {
        expect(mutate).toHaveBeenCalledWith(
          "task-failed-1",
          expect.objectContaining({ onError: expect.any(Function) }),
        );
      });
    });

    it("does not render malformed task records without an id", () => {
      useTaskQueueMock.mockReturnValue(
        makeQuery({ tasks: [{ status: "pending", title: "Missing id" }], total: 1 }),
      );
      renderPage();
      expect(screen.queryByText("Missing id")).not.toBeInTheDocument();
    });

    it("shows Delete button for completed tasks and calls deleteTask mutation", async () => {
      const mutate = vi.fn();
      useDeleteTaskMock.mockReturnValue(makeMutation({ mutate }));

      renderPage();

      // The completed task card has a Delete button — fire the first one visible
      const deleteBtns = screen.getAllByRole("button", { name: /tasks.action_delete/i });
      fireEvent.click(deleteBtns[0]);

      await waitFor(() => {
        expect(mutate).toHaveBeenCalled();
      });
    });

    it("does NOT show Re-queue button for pending tasks (only for cancelled/failed → pending)", () => {
      renderPage();
      // Pending tasks only get Cancel/Delete, not Requeue
      const pendingCard = screen.getByText("Pending task title").closest("div");
      expect(pendingCard).not.toBeNull();
      // Make sure no Requeue inside the pending card area
      expect(screen.queryAllByRole("button", { name: /tasks.action_requeue/i })).toHaveLength(0);
    });
  });

  describe("error and loading states", () => {
    it("shows error card when task list fails", () => {
      useTaskQueueMock.mockReturnValue(makeQuery(undefined, { isError: true, isLoading: false }));
      renderPage();
      expect(screen.getByText("tasks.load_error")).toBeInTheDocument();
    });

    it("shows loading spinner while tasks are loading", () => {
      useTaskQueueMock.mockReturnValue(
        makeQuery(undefined, { isLoading: true, isFetching: true }),
      );
      renderPage();
      // Spinner is rendered while loading (no column labels visible)
      expect(screen.queryByText("tasks.col_pending")).not.toBeInTheDocument();
    });
  });

  describe("agent filter", () => {
    it("filters tasks by assigned agent", () => {
      renderPage();
      const filterSelect = screen.getByDisplayValue("tasks.all_agents");
      // The option value is the agent id; the sample tasks store the *name*.
      // Matching both spellings is what keeps pre-existing tasks filterable
      // after the picker moved to ids.
      fireEvent.change(filterSelect, { target: { value: ALPHA_ID } });

      // After filtering, only agent-alpha tasks visible (pending + completed)
      expect(screen.getByText("Pending task title")).toBeInTheDocument();
      expect(screen.getByText("Done task")).toBeInTheDocument();
      // agent-beta tasks not visible
      expect(screen.queryByText("Running task")).not.toBeInTheDocument();
    });
  });

  // ── The reported bug: the picker was built from the assignees of tasks that
  //    already existed, so it could not offer an agent that had never been
  //    assigned one, and kept offering agents that no longer exist. ──────────
  describe("agent picker is sourced from the agent registry", () => {
    it("offers an agent that has no tasks yet", () => {
      renderPage();
      // agent-gamma appears in no task in SAMPLE_TASKS. Under the old
      // derived-from-tasks list it was unreachable; it must be selectable now.
      const filterSelect = screen.getByDisplayValue("tasks.all_agents");
      const offered = Array.from(filterSelect.querySelectorAll("option")).map(
        (o) => o.textContent,
      );
      expect(offered).toContain("agent-gamma");
    });

    it("stops offering an agent that has been deleted, even while its tasks remain", () => {
      // agent-beta is gone from the registry but still owns two tasks.
      useAgentsMock.mockReturnValue(
        makeQuery(SAMPLE_AGENTS.filter((a) => a.name !== "agent-beta")),
      );
      renderPage();
      const filterSelect = screen.getByDisplayValue("tasks.all_agents");
      const offered = Array.from(filterSelect.querySelectorAll("option")).map(
        (o) => o.textContent,
      );
      expect(offered).not.toContain("agent-beta");
      // Its tasks stay visible and readable — they are not hidden by the
      // agent's absence, only unfilterable.
      expect(screen.getByText("Running task")).toBeInTheDocument();
      expect(screen.getAllByText("agent-beta").length).toBeGreaterThan(0);
    });

    it("offers every registered agent even when the board is empty", () => {
      useTaskQueueMock.mockReturnValue(makeQuery({ tasks: [], total: 0 }));
      useTaskQueueStatusMock.mockReturnValue(
        makeQuery({ total: 0, pending: 0, in_progress: 0, completed: 0, failed: 0 }),
      );
      renderPage();
      fireEvent.click(screen.getAllByText("tasks.new_task")[0]);
      const assigneeSelect = screen.getByDisplayValue("tasks.assignee_none");
      const offered = Array.from(assigneeSelect.querySelectorAll("option")).map(
        (o) => o.textContent,
      );
      // Previously an empty board meant an empty list, which downgraded the
      // field to a free-text box that could only produce a rejected post.
      expect(offered).toEqual(
        expect.arrayContaining(["agent-alpha", "agent-beta", "agent-gamma"]),
      );
    });
  });

  describe("task limits", () => {
    it("posts the agent id, priority and timeout the operator chose", async () => {
      const mutate = vi.fn();
      useCreateTaskMock.mockReturnValue(makeMutation({ mutate }));
      renderPage();
      fireEvent.click(screen.getAllByText("tasks.new_task")[0]);

      fireEvent.change(screen.getByPlaceholderText("tasks.field_title_placeholder"), {
        target: { value: "Urgent probe" },
      });
      fireEvent.change(screen.getByPlaceholderText("tasks.field_description_placeholder"), {
        target: { value: "Check the thing" },
      });
      fireEvent.change(screen.getByDisplayValue("tasks.assignee_none"), {
        target: { value: ALPHA_ID },
      });
      fireEvent.change(screen.getByDisplayValue("tasks.priority_normal"), {
        target: { value: "2" },
      });
      fireEvent.change(screen.getByPlaceholderText("tasks.field_timeout_placeholder"), {
        target: { value: "90" },
      });
      fireEvent.click(screen.getByText("tasks.submit"));

      await waitFor(() => expect(mutate).toHaveBeenCalled());
      expect(mutate).toHaveBeenCalledWith(
        expect.objectContaining({
          title: "Urgent probe",
          description: "Check the thing",
          assigned_to: ALPHA_ID,
          priority: 2,
          timeout_secs: 90,
        }),
      );
    });

    it("omits priority and timeout when the operator left the defaults", async () => {
      const mutate = vi.fn();
      useCreateTaskMock.mockReturnValue(makeMutation({ mutate }));
      renderPage();
      fireEvent.click(screen.getAllByText("tasks.new_task")[0]);
      fireEvent.change(screen.getByPlaceholderText("tasks.field_title_placeholder"), {
        target: { value: "Plain" },
      });
      fireEvent.change(screen.getByPlaceholderText("tasks.field_description_placeholder"), {
        target: { value: "No limits" },
      });
      fireEvent.click(screen.getByText("tasks.submit"));

      await waitFor(() => expect(mutate).toHaveBeenCalled());
      const payload = mutate.mock.calls[0][0];
      // Absent, not zero: `priority: 0` is the neutral default and
      // `timeout_secs: 0` would mean "never reclaim", a very different order.
      expect(payload).not.toHaveProperty("priority");
      expect(payload).not.toHaveProperty("timeout_secs");
    });

    it("blocks submission on a negative timeout instead of posting it", () => {
      const mutate = vi.fn();
      useCreateTaskMock.mockReturnValue(makeMutation({ mutate }));
      renderPage();
      fireEvent.click(screen.getAllByText("tasks.new_task")[0]);
      fireEvent.change(screen.getByPlaceholderText("tasks.field_title_placeholder"), {
        target: { value: "Bad" },
      });
      fireEvent.change(screen.getByPlaceholderText("tasks.field_description_placeholder"), {
        target: { value: "Bad timeout" },
      });
      fireEvent.change(screen.getByPlaceholderText("tasks.field_timeout_placeholder"), {
        target: { value: "-5" },
      });
      fireEvent.click(screen.getByText("tasks.submit"));
      expect(mutate).not.toHaveBeenCalled();
    });

    it("badges a non-neutral priority and a per-task timeout on the card", () => {
      useTaskQueueMock.mockReturnValue(
        makeQuery({
          tasks: [
            {
              id: "t-urgent",
              status: "pending",
              title: "Urgent one",
              description: "d",
              assigned_to: ALPHA_ID,
              priority: 2,
              timeout_secs: 90,   // 1.5 min — must render exactly, not rounded
              created_at: new Date().toISOString(),
            },
            {
              id: "t-plain",
              status: "pending",
              title: "Plain one",
              description: "d",
              priority: 0,
              timeout_secs: null,
              created_at: new Date().toISOString(),
            },
          ],
          total: 2,
        }),
      );
      renderPage();
      expect(screen.getByText("tasks.priority_urgent")).toBeInTheDocument();
      expect(screen.getByText("1.5m")).toBeInTheDocument();
      // The neutral priority is not badged — every historical task carries 0,
      // so badging it would paint the whole board.
      expect(screen.queryByText("tasks.priority_normal")).not.toBeInTheDocument();
    });

    it("renders an id-assigned task under the agent name, not a raw UUID", () => {
      useTaskQueueMock.mockReturnValue(
        makeQuery({
          tasks: [
            {
              id: "t-byid",
              status: "pending",
              title: "Posted by id",
              description: "d",
              assigned_to: ALPHA_ID,
              created_at: new Date().toISOString(),
            },
          ],
          total: 1,
        }),
      );
      renderPage();
      // Scoped to the card's badge — "agent-alpha" also appears as a filter
      // option, so a bare text query would not prove the card resolved it.
      const badge = document.querySelector(`span[title="${ALPHA_ID}"]`);
      expect(badge).not.toBeNull();
      expect(badge!.textContent).toBe("agent-alpha");
      expect(screen.queryByText(ALPHA_ID)).not.toBeInTheDocument();
    });
  });
});
