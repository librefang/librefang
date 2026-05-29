import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { TasksPage } from "./TasksPage";
import { useTaskQueue, useTaskQueueStatus } from "../lib/queries/runtime";
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

function setQueryDefaults() {
  useTaskQueueMock.mockReturnValue(makeQuery({ tasks: SAMPLE_TASKS, total: SAMPLE_TASKS.length }));
  useTaskQueueStatusMock.mockReturnValue(makeQuery(SAMPLE_STATUS));
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
        expect(mutate).toHaveBeenCalledWith("task-failed-1");
      });
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
      fireEvent.change(filterSelect, { target: { value: "agent-alpha" } });

      // After filtering, only agent-alpha tasks visible (pending + completed)
      expect(screen.getByText("Pending task title")).toBeInTheDocument();
      expect(screen.getByText("Done task")).toBeInTheDocument();
      // agent-beta tasks not visible
      expect(screen.queryByText("Running task")).not.toBeInTheDocument();
    });
  });
});
