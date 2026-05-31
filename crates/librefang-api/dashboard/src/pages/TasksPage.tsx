import { useState, useCallback, useEffect } from "react";
import { useTranslation } from "react-i18next";
import {
  Kanban,
  Plus,
  RefreshCw,
  RotateCcw,
  XCircle,
  Trash2,
  Clock,
  User,
  AlertTriangle,
  CheckCircle2,
  Loader2,
} from "lucide-react";
import { PageHeader } from "../components/ui/PageHeader";
import { Card } from "../components/ui/Card";
import { Badge } from "../components/ui/Badge";
import { Button } from "../components/ui/Button";
import { Modal } from "../components/ui/Modal";
import { useTaskQueue, useTaskQueueStatus } from "../lib/queries/runtime";
import {
  useCreateTask,
  useUpdateTaskStatus,
  useDeleteTask,
  useRetryTask,
} from "../lib/mutations/runtime";
import type { TaskQueueItem } from "../api";

// Operator-visible status columns (order: left to right).
// Cancelled is shown last and is collapsed when empty.
const COLUMNS: Array<{
  key: string;
  labelKey: string;
  variant: "warning" | "brand" | "success" | "error" | "default";
  // Which statuses live in this column
  statuses: string[];
}> = [
  { key: "pending",     labelKey: "tasks.col_pending",     variant: "warning", statuses: ["pending"] },
  { key: "in_progress", labelKey: "tasks.col_in_progress", variant: "brand",   statuses: ["in_progress"] },
  { key: "completed",   labelKey: "tasks.col_completed",   variant: "success", statuses: ["completed"] },
  { key: "failed",      labelKey: "tasks.col_failed",      variant: "error",   statuses: ["failed"] },
  { key: "cancelled",   labelKey: "tasks.col_cancelled",   variant: "default", statuses: ["cancelled"] },
];

function relativeTime(iso?: string): string {
  if (!iso) return "-";
  const diff = Date.now() - new Date(iso).getTime();
  const mins = Math.floor(diff / 60_000);
  if (mins < 1) return "<1m";
  if (mins < 60) return `${mins}m`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h`;
  return `${Math.floor(hrs / 24)}d`;
}

// Operator-allowed transitions. Agent-driven ones (pending→in_progress,
// in_progress→completed/failed) are intentionally absent.
function allowedActions(status?: string): Array<"requeue" | "cancel" | "retry" | "delete"> {
  switch (status) {
    case "pending":     return ["cancel", "delete"];
    case "in_progress": return ["cancel", "delete"];
    case "failed":      return ["retry", "delete"];
    case "completed":   return ["delete"];
    case "cancelled":   return ["requeue", "delete"];
    default:            return ["delete"];
  }
}

// ────────────────────────────────────────────────────────────────────────────
// Task card
// ────────────────────────────────────────────────────────────────────────────

interface TaskCardProps {
  task: TaskQueueItem;
  isDragTarget?: boolean;
  onDragStart: (id: string) => void;
}

function TaskCard({ task, isDragTarget, onDragStart }: TaskCardProps) {
  const { t } = useTranslation();
  const deleteMutation = useDeleteTask();
  const retryMutation = useRetryTask();
  const updateMutation = useUpdateTaskStatus();

  const id = task.id ?? "";
  const actions = allowedActions(task.status);

  function handleAction(action: "requeue" | "cancel" | "retry" | "delete") {
    if (!id) return;
    if (action === "delete") { deleteMutation.mutate(id); return; }
    if (action === "retry")  { retryMutation.mutate(id); return; }
    if (action === "cancel") { updateMutation.mutate({ id, status: "cancelled" }); return; }
    if (action === "requeue"){ updateMutation.mutate({ id, status: "pending" }); return; }
  }

  const isBusy = deleteMutation.isPending || retryMutation.isPending || updateMutation.isPending;

  return (
    <div
      draggable={actions.includes("requeue")}
      onDragStart={e => {
        if (!id) return;
        // Firefox refuses to start a drag unless dataTransfer is populated in
        // dragstart; without this the entire re-queue-by-drag gesture is a
        // no-op there. The payload also carries the id so the drop does not
        // rely solely on React state.
        e.dataTransfer.setData("text/plain", id);
        e.dataTransfer.effectAllowed = "move";
        onDragStart(id);
      }}
      className={`rounded-xl border p-3 text-sm transition-shadow cursor-default select-none
        ${isDragTarget ? "border-brand/60 bg-brand/5" : "border-border-subtle bg-surface hover:shadow-sm"}
      `}
    >
      {/* Title row */}
      <div className="flex items-start justify-between gap-2 mb-1.5">
        <p className="font-semibold text-[13px] leading-snug flex-1 truncate">
          {task.title ?? id.slice(0, 12)}
        </p>
        {isBusy && <Loader2 className="w-3.5 h-3.5 animate-spin text-brand shrink-0 mt-0.5" />}
      </div>

      {/* Description */}
      {task.description && (
        <p className="text-xs text-text-dim leading-snug line-clamp-2 mb-2">
          {task.description}
        </p>
      )}

      {/* Result preview (completed / failed) */}
      {task.result && (
        <div className="mt-1.5 mb-2 rounded-md bg-main/50 px-2 py-1.5">
          <p className="text-[10px] font-bold uppercase text-text-dim/50 mb-0.5">
            {t("tasks.result_label")}
          </p>
          <p className="text-[11px] text-text-dim line-clamp-2">{task.result}</p>
        </div>
      )}

      {/* Meta row */}
      <div className="flex items-center gap-2 mt-1.5 flex-wrap">
        {task.assigned_to ? (
          <span className="flex items-center gap-1 text-[10px] font-mono text-brand bg-brand/8 px-1.5 py-0.5 rounded-md shrink-0">
            <User className="w-2.5 h-2.5" />
            {task.assigned_to}
          </span>
        ) : (
          <span className="text-[10px] text-text-dim/50 italic shrink-0">{t("tasks.unassigned")}</span>
        )}
        {task.created_by && (
          <span className="text-[10px] text-text-dim/50 shrink-0">
            {t("tasks.by")} {task.created_by}
          </span>
        )}
        <span className="ml-auto flex items-center gap-1 text-[10px] text-text-dim/50 shrink-0">
          <Clock className="w-2.5 h-2.5" />
          {relativeTime(task.created_at)}
        </span>
      </div>

      {/* Action buttons */}
      {actions.length > 0 && (
        <div className="flex items-center gap-1.5 mt-2.5 pt-2 border-t border-border-subtle/50 flex-wrap">
          {actions.includes("requeue") && (
            <button
              onClick={() => handleAction("requeue")}
              disabled={isBusy}
              className="flex items-center gap-1 text-[10px] font-bold text-brand hover:text-brand/80 disabled:opacity-50"
            >
              <RotateCcw className="w-2.5 h-2.5" />
              {t("tasks.action_requeue")}
            </button>
          )}
          {actions.includes("retry") && (
            <button
              onClick={() => handleAction("retry")}
              disabled={isBusy}
              className="flex items-center gap-1 text-[10px] font-bold text-brand hover:text-brand/80 disabled:opacity-50"
            >
              <RotateCcw className="w-2.5 h-2.5" />
              {t("tasks.action_retry")}
            </button>
          )}
          {actions.includes("cancel") && (
            <button
              onClick={() => handleAction("cancel")}
              disabled={isBusy}
              className="flex items-center gap-1 text-[10px] font-bold text-text-dim hover:text-error disabled:opacity-50"
            >
              <XCircle className="w-2.5 h-2.5" />
              {t("tasks.action_cancel")}
            </button>
          )}
          {actions.includes("delete") && (
            <button
              onClick={() => handleAction("delete")}
              disabled={isBusy}
              className="flex items-center gap-1 text-[10px] font-bold text-text-dim hover:text-error disabled:opacity-50 ml-auto"
            >
              <Trash2 className="w-2.5 h-2.5" />
              {t("tasks.action_delete")}
            </button>
          )}
        </div>
      )}
    </div>
  );
}

// ────────────────────────────────────────────────────────────────────────────
// Kanban column (with native HTML5 drag-and-drop support for re-queue only)
// ────────────────────────────────────────────────────────────────────────────

interface KanbanColumnProps {
  columnKey: string;
  label: string;
  badgeVariant: "warning" | "brand" | "success" | "error" | "default";
  tasks: TaskQueueItem[];
  dragTaskId: string | null;
  onDragStart: (id: string) => void;
  onDropRequeue: (taskId: string) => void;
}

function KanbanColumn({
  columnKey,
  label,
  badgeVariant,
  tasks,
  dragTaskId,
  onDragStart,
  onDropRequeue,
}: KanbanColumnProps) {
  const { t } = useTranslation();
  const [isDragOver, setIsDragOver] = useState(false);
  const updateMutation = useUpdateTaskStatus();

  // Only the Pending column accepts drops (re-queue operation)
  const acceptsDrop = columnKey === "pending";

  function handleDragOver(e: React.DragEvent) {
    if (!acceptsDrop || !dragTaskId) return;
    e.preventDefault();
    setIsDragOver(true);
  }

  function handleDragLeave() {
    setIsDragOver(false);
  }

  function handleDrop(e: React.DragEvent) {
    e.preventDefault();
    setIsDragOver(false);
    if (!acceptsDrop || !dragTaskId) return;
    onDropRequeue(dragTaskId);
    updateMutation.mutate({ id: dragTaskId, status: "pending" });
  }

  return (
    <div
      data-column={columnKey}
      className={`flex flex-col min-w-[240px] max-w-[320px] flex-1 rounded-2xl border transition-colors
        ${isDragOver && acceptsDrop
          ? "border-brand bg-brand/5"
          : "border-border-subtle bg-main/30"
        }
      `}
      onDragOver={handleDragOver}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
    >
      {/* Column header */}
      <div className="flex items-center gap-2 px-3 pt-3 pb-2">
        <span className="text-xs font-black uppercase tracking-widest flex-1">{label}</span>
        <Badge variant={badgeVariant}>{tasks.length}</Badge>
      </div>

      {/* Drop hint for pending column */}
      {acceptsDrop && isDragOver && dragTaskId && (
        <div className="mx-3 mb-2 rounded-lg border-2 border-dashed border-brand/40 bg-brand/5 px-3 py-2 text-center text-[10px] text-brand/70">
          {t("tasks.drag_hint")}
        </div>
      )}

      {/* Cards */}
      <div className="flex flex-col gap-2 px-3 pb-3 overflow-y-auto max-h-[calc(100vh-280px)]">
        {tasks.length === 0 ? (
          <p className="text-[11px] text-text-dim/40 italic text-center py-4">{t("tasks.empty")}</p>
        ) : (
          tasks.map((task) => (
            <TaskCard
              key={task.id ?? task.created_at}
              task={task}
              isDragTarget={dragTaskId === task.id && !acceptsDrop}
              onDragStart={onDragStart}
            />
          ))
        )}
      </div>
    </div>
  );
}

// ────────────────────────────────────────────────────────────────────────────
// New Task modal
// ────────────────────────────────────────────────────────────────────────────

interface NewTaskModalProps {
  isOpen: boolean;
  onClose: () => void;
  agents: string[];
}

function NewTaskModal({ isOpen, onClose, agents }: NewTaskModalProps) {
  const { t } = useTranslation();
  const createMutation = useCreateTask({
    onSuccess: () => { onClose(); },
  });

  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [assignee, setAssignee] = useState("");

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!title.trim() || !description.trim()) return;
    createMutation.mutate({
      title: title.trim(),
      description: description.trim(),
      ...(assignee.trim() ? { assigned_to: assignee.trim() } : {}),
    });
  }

  // Reset form when modal opens
  // Reset form when modal opens. The previous `if (isOpen && !prev.current)
  // setX(...)` block called setState during render, which React strict-mode
  // warns against and can misbehave under Suspense / concurrent rendering.
  useEffect(() => {
    if (isOpen) {
      setTitle("");
      setDescription("");
      setAssignee("");
    }
  }, [isOpen]);

  const INPUT_CLASS = "w-full rounded-xl border border-border-subtle bg-main px-3 py-2.5 text-sm focus:border-brand focus:ring-2 focus:ring-brand/10 outline-none transition-colors placeholder:text-text-dim/40";

  return (
    <Modal isOpen={isOpen} onClose={onClose} title={t("tasks.modal_title")} size="md">
      <form onSubmit={handleSubmit} className="px-6 pb-6 space-y-4">
        <div>
          <label className="block text-xs font-semibold text-text-dim mb-1.5">
            {t("tasks.field_title")} <span className="text-error">*</span>
          </label>
          <input
            type="text"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            placeholder={t("tasks.field_title_placeholder")}
            required
            autoFocus
            className={INPUT_CLASS}
          />
        </div>

        <div>
          <label className="block text-xs font-semibold text-text-dim mb-1.5">
            {t("tasks.field_description")} <span className="text-error">*</span>
          </label>
          <textarea
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            placeholder={t("tasks.field_description_placeholder")}
            required
            rows={3}
            className={`${INPUT_CLASS} resize-none`}
          />
        </div>

        <div>
          <label className="block text-xs font-semibold text-text-dim mb-1.5">
            {t("tasks.field_assignee")}
          </label>
          {agents.length > 0 ? (
            <select
              value={assignee}
              onChange={(e) => setAssignee(e.target.value)}
              className={INPUT_CLASS}
            >
              <option value="">{t("tasks.all_agents")}</option>
              {agents.map((a) => (
                <option key={a} value={a}>{a}</option>
              ))}
            </select>
          ) : (
            <input
              type="text"
              value={assignee}
              onChange={(e) => setAssignee(e.target.value)}
              placeholder={t("tasks.field_assignee_placeholder")}
              className={INPUT_CLASS}
            />
          )}
        </div>

        {createMutation.isError && (
          <p className="text-xs text-error">
            {createMutation.error instanceof Error
              ? createMutation.error.message
              : String(createMutation.error)}
          </p>
        )}

        <div className="flex gap-3 pt-1">
          <Button
            type="button"
            variant="secondary"
            size="md"
            className="flex-1"
            onClick={onClose}
            disabled={createMutation.isPending}
          >
            {t("tasks.cancel")}
          </Button>
          <Button
            type="submit"
            variant="primary"
            size="md"
            className="flex-1"
            isLoading={createMutation.isPending}
            disabled={!title.trim() || !description.trim() || createMutation.isPending}
          >
            {t("tasks.submit")}
          </Button>
        </div>
      </form>
    </Modal>
  );
}

// ────────────────────────────────────────────────────────────────────────────
// TasksPage (main export)
// ────────────────────────────────────────────────────────────────────────────

export function TasksPage() {
  const { t } = useTranslation();
  const [agentFilter, setAgentFilter] = useState("");
  const [showNewTask, setShowNewTask] = useState(false);
  const [dragTaskId, setDragTaskId] = useState<string | null>(null);

  // Fetch all tasks (no status filter — we split client-side)
  const taskListQuery = useTaskQueue();
  const taskStatusQuery = useTaskQueueStatus();

  const allTasks: TaskQueueItem[] = taskListQuery.data?.tasks ?? [];
  const taskStatus = taskStatusQuery.data;

  // Derive unique agent names for the filter dropdown
  const agentNames = Array.from(
    new Set(
      allTasks
        .map((t) => t.assigned_to)
        .filter((a): a is string => typeof a === "string" && a.length > 0),
    ),
  ).sort();

  // Apply agent filter
  const filteredTasks = agentFilter
    ? allTasks.filter((t) => t.assigned_to === agentFilter)
    : allTasks;

  // Group by status
  function getColumnTasks(statuses: string[]): TaskQueueItem[] {
    return filteredTasks.filter((t) => statuses.includes(t.status ?? ""));
  }

  const handleDragStart = useCallback((id: string) => {
    setDragTaskId(id);
  }, []);

  const handleDropRequeue = useCallback((_taskId: string) => {
    setDragTaskId(null);
    // The actual mutation is fired inside KanbanColumn.handleDrop
    // to keep column-level responsibility clear.
  }, []);

  function handleRefresh() {
    taskListQuery.refetch();
    taskStatusQuery.refetch();
  }

  const isLoading = taskListQuery.isLoading;
  const isError = taskListQuery.isError;

  // Summary stats
  const summaryStats = taskStatus
    ? [
        { key: "status_total",       value: taskStatus.total ?? 0,       color: "text-text" },
        { key: "status_pending",     value: taskStatus.pending ?? 0,     color: "text-warning" },
        { key: "status_in_progress", value: taskStatus.in_progress ?? 0, color: "text-brand" },
        { key: "status_completed",   value: taskStatus.completed ?? 0,   color: "text-success" },
        { key: "status_failed",      value: taskStatus.failed ?? 0,      color: (taskStatus.failed ?? 0) > 0 ? "text-error" : "text-text-dim" },
      ]
    : [];

  // Columns — hide Cancelled when it has no tasks and no filter
  const visibleColumns = COLUMNS.filter((col) => {
    if (col.key !== "cancelled") return true;
    return getColumnTasks(col.statuses).length > 0;
  });

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("tasks.badge")}
        title={t("tasks.title")}
        subtitle={t("tasks.subtitle")}
        isFetching={taskListQuery.isFetching}
        onRefresh={handleRefresh}
        icon={<Kanban className="h-4 w-4" />}
        helpText={t("tasks.help")}
      />

      {/* ── Status summary bar ── */}
      {summaryStats.length > 0 && (
        <div className="flex items-center gap-4 flex-wrap">
          {summaryStats.map((s) => (
            <div key={s.key} className="flex items-center gap-1.5">
              <span className={`text-lg font-black ${s.color}`}>{s.value}</span>
              <span className="text-[10px] text-text-dim/60 uppercase tracking-wider">{t(`tasks.${s.key}`)}</span>
            </div>
          ))}

          {/* Agent filter */}
          <div className="ml-auto flex items-center gap-2">
            <label className="text-[11px] text-text-dim/60 uppercase tracking-wider">{t("tasks.filter_agent")}</label>
            <select
              value={agentFilter}
              onChange={(e) => setAgentFilter(e.target.value)}
              className="rounded-lg border border-border-subtle bg-main px-2 py-1 text-xs focus:border-brand focus:ring-1 focus:ring-brand/10 outline-none"
            >
              <option value="">{t("tasks.all_agents")}</option>
              {agentNames.map((a) => (
                <option key={a} value={a}>{a}</option>
              ))}
            </select>

            <Button
              variant="secondary"
              size="sm"
              leftIcon={<RefreshCw className="w-3 h-3" />}
              onClick={handleRefresh}
              isLoading={taskListQuery.isFetching}
            >
              {null}
            </Button>

            <Button
              variant="primary"
              size="sm"
              leftIcon={<Plus className="w-3 h-3" />}
              onClick={() => setShowNewTask(true)}
            >
              {t("tasks.new_task")}
            </Button>
          </div>
        </div>
      )}

      {/* ── Error state ── */}
      {isError && (
        <Card padding="lg">
          <div className="flex items-center gap-3 text-error">
            <AlertTriangle className="w-5 h-5 shrink-0" />
            <div className="flex-1">
              <p className="text-sm font-bold">{t("tasks.load_error")}</p>
            </div>
            <Button variant="secondary" size="sm" onClick={handleRefresh}>
              {t("tasks.retry_load")}
            </Button>
          </div>
        </Card>
      )}

      {/* ── Loading state ── */}
      {isLoading && (
        <div className="flex items-center justify-center py-16">
          <Loader2 className="w-6 h-6 animate-spin text-brand" />
        </div>
      )}

      {/* ── Kanban board ── */}
      {!isLoading && !isError && (
        <div
          className="flex gap-3 overflow-x-auto pb-4"
          onDragEnd={() => setDragTaskId(null)}
        >
          {visibleColumns.map((col) => {
            const colTasks = getColumnTasks(col.statuses);
            return (
              <KanbanColumn
                key={col.key}
                columnKey={col.key}
                label={t(col.labelKey)}
                badgeVariant={col.variant}
                tasks={colTasks}
                dragTaskId={dragTaskId}
                onDragStart={handleDragStart}
                onDropRequeue={handleDropRequeue}
              />
            );
          })}
        </div>
      )}

      {/* ── No tasks at all ── */}
      {!isLoading && !isError && allTasks.length === 0 && (
        <div className="flex flex-col items-center justify-center py-16 gap-4">
          <CheckCircle2 className="w-10 h-10 text-text-dim/30" />
          <p className="text-sm text-text-dim">{t("tasks.empty")}</p>
          <Button
            variant="primary"
            size="sm"
            leftIcon={<Plus className="w-3 h-3" />}
            onClick={() => setShowNewTask(true)}
          >
            {t("tasks.new_task")}
          </Button>
        </div>
      )}

      {/* ── New Task Modal ── */}
      <NewTaskModal
        isOpen={showNewTask}
        onClose={() => setShowNewTask(false)}
        agents={agentNames}
      />
    </div>
  );
}
