import { useState, useCallback, useEffect, useMemo } from "react";
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
  ArrowUpNarrowWide,
  Hourglass,
} from "lucide-react";
import { PageHeader } from "../components/ui/PageHeader";
import { Card } from "../components/ui/Card";
import { Badge } from "../components/ui/Badge";
import { Button } from "../components/ui/Button";
import { Modal } from "../components/ui/Modal";
import { useTaskQueue } from "../lib/queries/runtime";
import { useAgents } from "../lib/queries/agents";
import {
  useCreateTask,
  useUpdateTaskStatus,
  useDeleteTask,
  useRetryTask,
} from "../lib/mutations/runtime";
import type { AgentItem, TaskQueueItem } from "../api";
import { toastErr } from "../lib/errors";
import { useUIStore } from "../lib/store";

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

// Operator-facing priority levels. The engine takes any integer and orders the
// claim queue `priority DESC, created_at ASC`; these are the named rungs the
// board offers. 0 is the neutral default every historical task carries.
const PRIORITY_LEVELS: Array<{ value: number; labelKey: string }> = [
  { value: 2,  labelKey: "tasks.priority_urgent" },
  { value: 1,  labelKey: "tasks.priority_high" },
  { value: 0,  labelKey: "tasks.priority_normal" },
  { value: -1, labelKey: "tasks.priority_low" },
];

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

// `assigned_to` is stored as whichever spelling the poster used: the backend's
// `task_claim` matches the canonical UUID *or* the display name (issue #2841),
// and tasks predating the agent-registry-backed picker hold names. Resolve to a
// name for display so a task posted by id does not render as a raw UUID, and
// fall back to the stored string so a name-stored (or orphaned) task stays
// readable instead of blanking out.
function assigneeLabel(raw: string, agentsById: Map<string, AgentItem>): string {
  const known = agentsById.get(raw);
  if (known) return known.name;
  return UUID_RE.test(raw) ? raw.slice(0, 8) : raw;
}

// True when `task` is assigned to `agentId`, in either stored spelling.
function taskMatchesAgent(
  task: TaskQueueItem,
  agentId: string,
  agentsById: Map<string, AgentItem>,
): boolean {
  const raw = task.assigned_to ?? "";
  if (!raw) return false;
  if (raw === agentId) return true;
  return agentsById.get(agentId)?.name === raw;
}

// A deadline badge that rounds is worse than no badge: an operator who set 90s
// and reads "2m" will mistrust the field. Keep one decimal when the value does
// not divide evenly, so the rendered figure is always the one that was set.
function formatTimeout(secs: number | null | undefined): string | null {
  if (secs === null || secs === undefined) return null;
  if (secs === 0) return "∞";
  if (secs < 60) return `${secs}s`;
  if (secs < 3600) {
    const mins = secs / 60;
    return Number.isInteger(mins) ? `${mins}m` : `${mins.toFixed(1)}m`;
  }
  const hrs = secs / 3600;
  return Number.isInteger(hrs) ? `${hrs}h` : `${hrs.toFixed(1)}h`;
}

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
  agentsById: Map<string, AgentItem>;
}

function TaskCard({ task, isDragTarget, onDragStart, agentsById }: TaskCardProps) {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);
  const deleteMutation = useDeleteTask();
  const retryMutation = useRetryTask();
  const updateMutation = useUpdateTaskStatus();

  const id = task.id ?? "";
  const actions = allowedActions(task.status);

  function handleAction(action: "requeue" | "cancel" | "retry" | "delete") {
    if (!id) return;
    const onError = (err: unknown) => addToast(
      toastErr(err, t("common.error")),
      "error",
    );
    if (action === "delete") { deleteMutation.mutate(id, { onError }); return; }
    if (action === "retry")  { retryMutation.mutate(id, { onError }); return; }
    if (action === "cancel") { updateMutation.mutate({ id, status: "cancelled" }, { onError }); return; }
    if (action === "requeue"){ updateMutation.mutate({ id, status: "pending" }, { onError }); return; }
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
          <span
            title={task.assigned_to}
            className="flex items-center gap-1 text-[10px] font-mono text-brand bg-brand/8 px-1.5 py-0.5 rounded-md shrink-0"
          >
            <User className="w-2.5 h-2.5" />
            {assigneeLabel(task.assigned_to, agentsById)}
          </span>
        ) : (
          <span className="text-[10px] text-text-dim/50 italic shrink-0">{t("tasks.unassigned")}</span>
        )}
        {/* Only a non-neutral priority is worth pixels — every historical task
            carries 0, so badging it would be noise on the whole board. */}
        {typeof task.priority === "number" && task.priority !== 0 && (
          <span
            title={t("tasks.priority_tooltip")}
            className={`flex items-center gap-1 text-[10px] font-bold px-1.5 py-0.5 rounded-md shrink-0
              ${task.priority > 0 ? "text-warning bg-warning/10" : "text-text-dim/60 bg-main/50"}`}
          >
            <ArrowUpNarrowWide className="w-2.5 h-2.5" />
            {t(
              PRIORITY_LEVELS.find((p) => p.value === task.priority)?.labelKey
                ?? "tasks.priority_custom",
              { value: task.priority },
            )}
          </span>
        )}
        {formatTimeout(task.timeout_secs) && (
          <span
            title={t("tasks.timeout_tooltip")}
            className="flex items-center gap-1 text-[10px] font-mono text-text-dim/60 bg-main/50 px-1.5 py-0.5 rounded-md shrink-0"
          >
            <Hourglass className="w-2.5 h-2.5" />
            {formatTimeout(task.timeout_secs)}
          </span>
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
  agentsById: Map<string, AgentItem>;
}

function KanbanColumn({
  columnKey,
  label,
  badgeVariant,
  tasks,
  dragTaskId,
  onDragStart,
  onDropRequeue,
  agentsById,
}: KanbanColumnProps) {
  const { t } = useTranslation();
  const [isDragOver, setIsDragOver] = useState(false);

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
    if (!acceptsDrop) return;
    const taskId = e.dataTransfer.getData("text/plain") || dragTaskId;
    if (taskId) onDropRequeue(taskId);
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
          tasks.filter((task) => Boolean(task.id)).map((task) => (
            <TaskCard
              key={task.id}
              task={task}
              isDragTarget={dragTaskId === task.id && !acceptsDrop}
              onDragStart={onDragStart}
              agentsById={agentsById}
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
  agents: AgentItem[];
}

function NewTaskModal({ isOpen, onClose, agents }: NewTaskModalProps) {
  const { t } = useTranslation();
  const createMutation = useCreateTask({
    onSuccess: () => { onClose(); },
  });

  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [assignee, setAssignee] = useState("");
  const [priority, setPriority] = useState(0);
  // Kept as a string so the field can be genuinely empty ("inherit the global
  // TTL"), which `0` cannot express — `0` means "never reclaim".
  const [timeout, setTimeout] = useState("");

  const timeoutSecs = timeout.trim() === "" ? undefined : Number(timeout);
  const timeoutInvalid =
    timeoutSecs !== undefined && (!Number.isInteger(timeoutSecs) || timeoutSecs < 0);

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!title.trim() || !description.trim() || timeoutInvalid) return;
    createMutation.mutate({
      title: title.trim(),
      description: description.trim(),
      ...(assignee ? { assigned_to: assignee } : {}),
      ...(priority !== 0 ? { priority } : {}),
      ...(timeoutSecs !== undefined ? { timeout_secs: timeoutSecs } : {}),
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
      setPriority(0);
      setTimeout("");
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
          {/* The agent reads this verbatim, so it is where the operator puts
              the actual brief — resizable and roomy rather than a 3-line slot. */}
          <textarea
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            placeholder={t("tasks.field_description_placeholder")}
            required
            rows={8}
            className={`${INPUT_CLASS} resize-y min-h-[8rem] font-mono text-[13px] leading-relaxed`}
          />
          <p className="mt-1 text-[10px] text-text-dim/50">{t("tasks.field_description_hint")}</p>
        </div>

        <div>
          <label className="block text-xs font-semibold text-text-dim mb-1.5">
            {t("tasks.field_assignee")}
          </label>
          {/* Sourced from the agent registry, not from the assignees of tasks
              that happen to exist: a brand-new agent is selectable before its
              first task, and a deleted one stops being offered. The value is
              the agent id, which survives a rename — the label resolves back
              to the name on the board. */}
          <select
            value={assignee}
            onChange={(e) => setAssignee(e.target.value)}
            disabled={agents.length === 0}
            className={`${INPUT_CLASS} disabled:opacity-50`}
          >
            <option value="">{t("tasks.assignee_none")}</option>
            {agents.map((a) => (
              <option key={a.id} value={a.id}>{a.name}</option>
            ))}
          </select>
          {agents.length === 0 && (
            <p className="mt-1 text-[10px] text-text-dim/60">{t("tasks.no_agents_hint")}</p>
          )}
        </div>

        <div className="grid grid-cols-2 gap-3">
          <div>
            <label className="block text-xs font-semibold text-text-dim mb-1.5">
              {t("tasks.field_priority")}
            </label>
            <select
              value={priority}
              onChange={(e) => setPriority(Number(e.target.value))}
              className={INPUT_CLASS}
            >
              {PRIORITY_LEVELS.map((p) => (
                <option key={p.value} value={p.value}>{t(p.labelKey)}</option>
              ))}
            </select>
            <p className="mt-1 text-[10px] text-text-dim/50">{t("tasks.field_priority_hint")}</p>
          </div>

          <div>
            <label className="block text-xs font-semibold text-text-dim mb-1.5">
              {t("tasks.field_timeout")}
            </label>
            <input
              type="number"
              min={0}
              step={1}
              value={timeout}
              onChange={(e) => setTimeout(e.target.value)}
              placeholder={t("tasks.field_timeout_placeholder")}
              className={`${INPUT_CLASS} ${timeoutInvalid ? "border-error" : ""}`}
            />
            <p className="mt-1 text-[10px] text-text-dim/50">{t("tasks.field_timeout_hint")}</p>
          </div>
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
            disabled={
              !title.trim() || !description.trim() || timeoutInvalid || createMutation.isPending
            }
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
  const addToast = useUIStore((s) => s.addToast);
  const [agentFilter, setAgentFilter] = useState("");
  const [showNewTask, setShowNewTask] = useState(false);
  const [dragTaskId, setDragTaskId] = useState<string | null>(null);
  const updateMutation = useUpdateTaskStatus();

  // Fetch all tasks (no status filter — we split client-side)
  const taskListQuery = useTaskQueue();

  const allTasks: TaskQueueItem[] = taskListQuery.data?.tasks ?? [];
  const validTasks = allTasks.filter(
    (task): task is TaskQueueItem & { id: string } => typeof task.id === "string" && task.id.length > 0,
  );

  // The agent registry is the source of truth for who can hold a task. The
  // previous list was derived from the `assigned_to` of tasks that already
  // existed, which meant a new agent was unreachable until someone had already
  // assigned it something, a deleted agent lingered forever, and an empty
  // board offered no picker at all.
  const agentsQuery = useAgents();
  const agents = useMemo(() => agentsQuery.data ?? [], [agentsQuery.data]);
  const agentsById = useMemo(
    () => new Map(agents.map((a) => [a.id, a])),
    [agents],
  );

  // Apply agent filter
  const filteredTasks = agentFilter
    ? allTasks.filter((t) => taskMatchesAgent(t, agentFilter, agentsById))
    : allTasks;

  // Group by status
  function getColumnTasks(statuses: string[]): TaskQueueItem[] {
    return filteredTasks.filter((t) => statuses.includes(t.status ?? ""));
  }

  const handleDragStart = useCallback((id: string) => {
    setDragTaskId(id);
  }, []);

  const handleDropRequeue = useCallback((taskId: string) => {
    setDragTaskId(null);
    updateMutation.mutate(
      { id: taskId, status: "pending" },
      {
        onError: (err) => addToast(
          toastErr(err, t("common.error")),
          "error",
        ),
      },
    );
  }, [addToast, t, updateMutation]);

  function handleRefresh() {
    taskListQuery.refetch();
  }

  const isLoading = taskListQuery.isLoading;
  const isError = taskListQuery.isError;

  // Summary stats
  const countStatus = (status: string) => filteredTasks.filter((task) => task.status === status).length;
  const failedCount = countStatus("failed");
  const summaryStats = [
    { key: "status_total",       value: filteredTasks.length,          color: "text-text" },
    { key: "status_pending",     value: countStatus("pending"),        color: "text-warning" },
    { key: "status_in_progress", value: countStatus("in_progress"),    color: "text-brand" },
    { key: "status_completed",   value: countStatus("completed"),      color: "text-success" },
    { key: "status_failed",      value: failedCount, color: failedCount > 0 ? "text-error" : "text-text-dim" },
  ];

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
              {agents.map((a) => (
                <option key={a.id} value={a.id}>{a.name}</option>
              ))}
            </select>

            <Button
              variant="secondary"
              size="sm"
              leftIcon={<RefreshCw className="w-3 h-3" />}
              onClick={handleRefresh}
              isLoading={taskListQuery.isFetching}
              aria-label={t("common.refresh")}
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
                agentsById={agentsById}
              />
            );
          })}
        </div>
      )}

      {/* ── No tasks at all ── */}
      {!isLoading && !isError && validTasks.length === 0 && (
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
        agents={agents}
      />
    </div>
  );
}
