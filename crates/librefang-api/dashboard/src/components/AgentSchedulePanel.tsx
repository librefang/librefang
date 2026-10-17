import { type FormEvent, useCallback, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  AlertCircle,
  Clock,
  Loader2,
  Pencil,
  Plus,
  Trash2,
  Zap,
} from "lucide-react";
import type {
  AgentDetail,
  CreateTriggerPayload,
  CronJobItem,
  CronScheduleSpec,
  TriggerItem,
  TriggerPatch,
} from "../api";
import { Badge } from "./ui/Badge";
import { Button } from "./ui/Button";
import { DrawerPanel } from "./ui/DrawerPanel";
import { EmptyState } from "./ui/EmptyState";
import { formatRelativeTime } from "../lib/datetime";
import { useUIStore } from "../lib/store";
import { useAgentTriggers } from "../lib/queries/schedules";
import { useCronJobs } from "../lib/queries/runtime";
import {
  useCreateCronJob,
  useCreateTrigger,
  useDeleteCronJob,
  useDeleteTrigger,
  useToggleCronJob,
  useUpdateCronJob,
  useUpdateTrigger,
} from "../lib/mutations/schedules";
import { formatTriggerPattern } from "../lib/triggerPattern";
import { truncateId } from "../lib/string";

const INPUT_CLASS =
  "w-full rounded-xl border border-border-subtle bg-main px-3 py-2 text-sm outline-none focus:border-brand";

// Trigger pattern presets must match the wire shape of `TriggerPattern`
// in `librefang_kernel::triggers`:
//   - unit variants (`Lifecycle`, `AgentTerminated`, `All`) deserialize from
//     the bare string form (e.g. `"lifecycle"`)
//   - struct variants (`AgentSpawned { name_pattern }`) require the object
//     form `{ "agent_spawned": { "name_pattern": "..." } }`
// Sending `"agent_spawned"` as a bare string lands on serde's
// AgentSpawned arm without the required `name_pattern` field and the
// backend rejects with "Invalid trigger pattern" (issue surfaced by the
// Codex P2 review on PR #5256). Default `name_pattern: "*"` matches all
// spawn events — users can edit the JSON via the `custom` preset if they
// need a narrower glob.
const TRIGGER_PATTERN_PRESETS = [
  { labelKey: "scheduler.trigger_preset_lifecycle", defaultLabel: "lifecycle (spawned + terminated)", value: '"lifecycle"' },
  {
    labelKey: "scheduler.trigger_preset_agent_spawned_any",
    defaultLabel: "agent_spawned (any)",
    value: '{"agent_spawned":{"name_pattern":"*"}}',
  },
  { labelKey: "scheduler.trigger_preset_agent_terminated", defaultLabel: "agent_terminated", value: '"agent_terminated"' },
  { labelKey: "scheduler.trigger_preset_all_events", defaultLabel: "all events", value: '"all"' },
  { labelKey: "scheduler.trigger_preset_custom_json", defaultLabel: "custom JSON…", value: "custom" },
] as const;

/**
 * Render a {@link CronJobItem}'s schedule field into a one-line summary.
 *
 * Backend serializes `CronSchedule` as
 * `{ kind: "cron" | "every" | "at", … }`. The list endpoint hands us a
 * weakly-typed object (the dashboard `CronJobItem` is `[key: string]:
 * unknown`), so we narrow defensively and fall back to JSON for unknown
 * shapes rather than rendering `[object Object]`.
 */
function formatCronSchedule(raw: unknown): string {
  if (!raw) return "—";
  if (typeof raw === "string") return raw;
  if (typeof raw !== "object") return String(raw);
  const obj = raw as Record<string, unknown>;
  const kind = obj.kind;
  if (kind === "cron" && typeof obj.expr === "string") {
    const tz = typeof obj.tz === "string" && obj.tz.length > 0 ? ` ${obj.tz}` : "";
    return `${obj.expr}${tz}`;
  }
  if (kind === "every" && typeof obj.every_secs === "number") {
    return `every ${obj.every_secs}s`;
  }
  if (kind === "at" && typeof obj.at === "string") {
    return `at ${obj.at}`;
  }
  try {
    return JSON.stringify(raw);
  } catch {
    return String(raw);
  }
}

/** Read `message` out of a cron job's loosely-typed `action` field. */
function readActionMessage(action: unknown): string {
  if (action && typeof action === "object") {
    const obj = action as Record<string, unknown>;
    if (typeof obj.message === "string") return obj.message;
    if (typeof obj.text === "string") return obj.text;
  }
  return "";
}

/** True when the cron job's `schedule.kind` is `"cron"`.
 *
 * The simplified edit form only exposes a cron-expression input; `every`
 * and `at` schedules round-trip through the same field would either need
 * a structured editor (out of scope) or be silently rewritten to a cron
 * expression on save. We hard-disable the pencil for those rows so the
 * user can't accidentally convert their schedule kind — they can still
 * toggle / delete via the always-visible buttons.
 */
function isCronKindCron(schedule: unknown): boolean {
  return (
    !!schedule &&
    typeof schedule === "object" &&
    (schedule as Record<string, unknown>).kind === "cron"
  );
}

interface AgentSchedulePanelProps {
  agent: AgentDetail;
}

/**
 * The Schedule tab's runtime-registry half (issue #4924).
 *
 * Sections:
 *  - **Cron jobs**: list + create/edit/delete/toggle. POST/PUT/DELETE
 *    against `/api/cron/jobs`.
 *  - **Event triggers**: list + create/edit/delete/toggle. POST/PATCH/
 *    DELETE against `/api/triggers`.
 *
 * Both are records in the runtime registry, not manifest fields, which is
 * why they live here rather than in the manifest form below them on the
 * same tab — the form owns the manifest's `[schedule]` (mode, cron,
 * conditions) and every other field an `agent.toml` admits.
 *
 * All API access goes through the existing hooks layer
 * (`useCronJobs`, `useAgentTriggers`, `useCreate*` / `useUpdate*` /
 * `useDelete*`), preserving the dashboard data-layer contract. Mutation
 * hooks own their own invalidation; this component only attaches
 * per-call `onSuccess` callbacks for toast feedback and modal dismissal.
 */
export function AgentSchedulePanel({ agent }: AgentSchedulePanelProps) {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);
  // Stable per-instance id prefix so multiple AgentSchedulePanel instances
  // on the same page (e.g. agent compare view) don't collide on input ids
  // when wiring htmlFor → input.id pairs for a11y.
  const formId = useId();

  const cronJobsQuery = useCronJobs(agent.id);
  const triggersQuery = useAgentTriggers(agent.id);

  const createCron = useCreateCronJob();
  const updateCron = useUpdateCronJob();
  const deleteCron = useDeleteCronJob();
  const toggleCron = useToggleCronJob();
  const createTrigger = useCreateTrigger();
  const updateTrigger = useUpdateTrigger();
  const deleteTrigger = useDeleteTrigger();

  // ----- cron create / edit ------------------------------------------------
  const [cronModal, setCronModal] = useState<
    | { mode: "create" }
    | { mode: "edit"; job: CronJobItem }
    | null
  >(null);
  const [cronName, setCronName] = useState("");
  const [cronExpr, setCronExpr] = useState("0 9 * * *");
  const [cronTz, setCronTz] = useState("");
  const [cronMessage, setCronMessage] = useState("");
  const [cronEnabled, setCronEnabled] = useState(true);

  const openCreateCron = () => {
    setCronName("");
    setCronExpr("0 9 * * *");
    setCronTz("");
    setCronMessage("");
    setCronEnabled(true);
    setCronModal({ mode: "create" });
  };

  const openEditCron = (job: CronJobItem) => {
    // Defense in depth — the pencil button is already disabled for
    // non-cron schedule kinds (`every` / `at`). If a future refactor
    // exposes another call site, refuse to open the modal here too so
    // the simplified form can't silently rewrite the schedule.
    if (!isCronKindCron(job.schedule)) {
      addToast(
        t("agents.detail.cron.edit_disabled_non_cron", {
          defaultValue:
            "This schedule uses 'every' or 'at' — edit it from agent.toml to avoid silent conversion to a cron expression.",
        }),
        "error",
      );
      return;
    }
    setCronName(typeof job.name === "string" ? job.name : "");
    const sched = job.schedule as Record<string, unknown>;
    setCronExpr(typeof sched.expr === "string" ? sched.expr : "");
    setCronTz(typeof sched.tz === "string" ? sched.tz : "");
    setCronMessage(readActionMessage(job.action));
    setCronEnabled(job.enabled !== false);
    setCronModal({ mode: "edit", job });
  };

  const handleCronSubmit = useCallback(
    async (e: FormEvent) => {
      e.preventDefault();
      if (!cronModal) return;
      if (!cronName.trim()) {
        addToast(
          t("agents.detail.cron.name_required", { defaultValue: "Name is required" }),
          "error",
        );
        return;
      }
      if (!cronExpr.trim()) {
        addToast(
          t("agents.detail.cron.expr_required", {
            defaultValue: "Cron expression is required",
          }),
          "error",
        );
        return;
      }
      const schedule: CronScheduleSpec = {
        kind: "cron",
        expr: cronExpr.trim(),
        tz: cronTz.trim() ? cronTz.trim() : null,
      };
      try {
        if (cronModal.mode === "create") {
          await createCron.mutateAsync({
            agent_id: agent.id,
            name: cronName.trim(),
            schedule,
            action: { kind: "agent_turn", message: cronMessage },
          });
          addToast(
            t("agents.detail.cron.created", { defaultValue: "Cron job created" }),
            "success",
          );
        } else {
          const id = typeof cronModal.job.id === "string" ? cronModal.job.id : "";
          if (!id) {
            addToast(
              t("agents.detail.cron.no_id", { defaultValue: "Cron job has no id" }),
              "error",
            );
            return;
          }
          await updateCron.mutateAsync({
            id,
            data: {
              name: cronName.trim(),
              schedule,
              action: { kind: "agent_turn", message: cronMessage },
              enabled: cronEnabled,
            },
          });
          addToast(
            t("agents.detail.cron.updated", { defaultValue: "Cron job updated" }),
            "success",
          );
        }
        setCronModal(null);
      } catch (err: unknown) {
        const msg = err instanceof Error ? err.message : String(err);
        addToast(msg || t("common.error", { defaultValue: "Error" }), "error");
      }
    },
    [
      cronModal,
      cronName,
      cronExpr,
      cronTz,
      cronMessage,
      cronEnabled,
      createCron,
      updateCron,
      agent.id,
      addToast,
      t,
    ],
  );

  // ----- trigger create / edit ---------------------------------------------
  const [triggerModal, setTriggerModal] = useState<
    | { mode: "create" }
    | { mode: "edit"; trigger: TriggerItem }
    | null
  >(null);
  const [trigPatternPreset, setTrigPatternPreset] = useState<string>('"lifecycle"');
  const [trigPatternCustom, setTrigPatternCustom] = useState("");
  const [trigPrompt, setTrigPrompt] = useState("");
  const [trigMaxFires, setTrigMaxFires] = useState(0);
  const [trigCooldown, setTrigCooldown] = useState("");
  const [trigSessionMode, setTrigSessionMode] = useState("");

  const openCreateTrigger = () => {
    setTrigPatternPreset('"lifecycle"');
    setTrigPatternCustom("");
    setTrigPrompt("");
    setTrigMaxFires(0);
    setTrigCooldown("");
    setTrigSessionMode("");
    setTriggerModal({ mode: "create" });
  };

  const openEditTrigger = (trigger: TriggerItem) => {
    // Map server-side pattern back into the preset selector. Anything
    // not in the preset list lands in "custom" with the JSON pre-filled.
    const stringified = JSON.stringify(trigger.pattern);
    const presetMatch = TRIGGER_PATTERN_PRESETS.find((p) => p.value === stringified);
    if (presetMatch) {
      setTrigPatternPreset(presetMatch.value);
      setTrigPatternCustom("");
    } else {
      setTrigPatternPreset("custom");
      setTrigPatternCustom(stringified);
    }
    setTrigPrompt(trigger.prompt_template ?? "");
    setTrigMaxFires(trigger.max_fires ?? 0);
    setTrigCooldown(trigger.cooldown_secs != null ? String(trigger.cooldown_secs) : "");
    setTrigSessionMode(trigger.session_mode ?? "");
    setTriggerModal({ mode: "edit", trigger });
  };

  const handleTriggerSubmit = useCallback(
    async (e: FormEvent) => {
      e.preventDefault();
      if (!triggerModal) return;
      const patternStr =
        trigPatternPreset === "custom" ? trigPatternCustom : trigPatternPreset;
      let pattern: unknown;
      try {
        pattern = JSON.parse(patternStr);
      } catch {
        addToast(
          t("agents.detail.trigger.invalid_pattern", {
            defaultValue: "Invalid trigger pattern JSON",
          }),
          "error",
        );
        return;
      }
      try {
        if (triggerModal.mode === "create") {
          const payload: CreateTriggerPayload = {
            agent_id: agent.id,
            pattern,
            prompt_template: trigPrompt,
            ...(trigMaxFires > 0 ? { max_fires: trigMaxFires } : {}),
            ...(trigCooldown ? { cooldown_secs: Number(trigCooldown) } : {}),
            ...(trigSessionMode ? { session_mode: trigSessionMode } : {}),
          };
          await createTrigger.mutateAsync(payload);
          addToast(
            t("agents.detail.trigger.created", { defaultValue: "Trigger created" }),
            "success",
          );
        } else {
          const patch: TriggerPatch = {
            pattern,
            prompt_template: trigPrompt,
            max_fires: trigMaxFires,
            cooldown_secs: trigCooldown === "" ? null : Number(trigCooldown),
            session_mode: trigSessionMode === "" ? null : trigSessionMode,
          };
          await updateTrigger.mutateAsync({
            id: triggerModal.trigger.id,
            data: patch,
          });
          addToast(
            t("agents.detail.trigger.updated", { defaultValue: "Trigger updated" }),
            "success",
          );
        }
        setTriggerModal(null);
      } catch (err: unknown) {
        const msg = err instanceof Error ? err.message : String(err);
        addToast(msg || t("common.error", { defaultValue: "Error" }), "error");
      }
    },
    [
      triggerModal,
      trigPatternPreset,
      trigPatternCustom,
      trigPrompt,
      trigMaxFires,
      trigCooldown,
      trigSessionMode,
      createTrigger,
      updateTrigger,
      agent.id,
      addToast,
      t,
    ],
  );

  // ----- delete confirmation state -----------------------------------------
  const [pendingDelete, setPendingDelete] = useState<
    | { kind: "cron"; id: string }
    | { kind: "trigger"; id: string }
    | null
  >(null);

  const confirmDeleteCron = useCallback(
    async (id: string) => {
      try {
        await deleteCron.mutateAsync({ id });
        addToast(
          t("agents.detail.cron.deleted", { defaultValue: "Cron job deleted" }),
          "success",
        );
        setPendingDelete(null);
      } catch (err: unknown) {
        const msg = err instanceof Error ? err.message : String(err);
        addToast(msg || t("common.error", { defaultValue: "Error" }), "error");
      }
    },
    [deleteCron, addToast, t],
  );

  const confirmDeleteTrigger = useCallback(
    async (id: string) => {
      try {
        await deleteTrigger.mutateAsync({ id });
        addToast(
          t("agents.detail.trigger.deleted", { defaultValue: "Trigger deleted" }),
          "success",
        );
        setPendingDelete(null);
      } catch (err: unknown) {
        const msg = err instanceof Error ? err.message : String(err);
        addToast(msg || t("common.error", { defaultValue: "Error" }), "error");
      }
    },
    [deleteTrigger, addToast, t],
  );

  const cronJobs = (cronJobsQuery.data ?? []) as CronJobItem[];
  const triggers = triggersQuery.data ?? [];

  return (
    <div className="flex flex-col gap-4">
      {/* ---- Cron jobs ------------------------------------------------------- */}
      <div className="flex items-center justify-between mt-2">
        <div className="text-[11px] uppercase font-semibold tracking-[0.08em] text-text-dim">
          {t("agents.detail.cron_jobs", { defaultValue: "Cron jobs" })}
        </div>
        <Button variant="ghost" size="sm" onClick={openCreateCron}>
          <Plus className="w-3.5 h-3.5 mr-1" />
          {t("agents.detail.add_cron", { defaultValue: "Add cron" })}
        </Button>
      </div>
      {cronJobs.length === 0 ? (
        <EmptyState
          icon={<Clock className="w-6 h-6" />}
          title={t("agents.detail.no_cron", { defaultValue: "No cron jobs" })}
        />
      ) : (
        <div className="flex flex-col gap-2">
          {cronJobs.map((c) => {
            const id = typeof c.id === "string" ? c.id : "";
            const enabled = c.enabled !== false;
            const isConfirming = pendingDelete?.kind === "cron" && pendingDelete.id === id;
            const editable = isCronKindCron(c.schedule);
            return (
              <div
                key={id || JSON.stringify(c)}
                className={`px-3.5 py-3 rounded-lg border bg-main/40 flex items-center gap-3 transition-colors ${enabled ? "border-border-subtle" : "border-border-subtle/50 opacity-60"}`}
              >
                <div className="w-8 h-8 rounded-md bg-accent/10 text-accent grid place-items-center shrink-0">
                  <Clock className="w-4 h-4" />
                </div>
                <div className="flex-1 min-w-0">
                  <div className="font-mono text-[13px] truncate text-text-main">
                    {typeof c.name === "string" && c.name.length > 0
                      ? c.name
                      : truncateId(id, 12)}
                  </div>
                  <div className="text-[11px] text-text-dim/80 mt-0.5 truncate font-mono">
                    {formatCronSchedule(c.schedule)}
                  </div>
                </div>
                {id ? (
                  <Badge
                    variant={enabled ? "brand" : "default"}
                    className="text-[9px] cursor-pointer"
                    onClick={() =>
                      toggleCron.mutate(
                        { id, enabled: !enabled },
                        {
                          onError: (err) =>
                            addToast(
                              err instanceof Error ? err.message : String(err),
                              "error",
                            ),
                        },
                      )
                    }
                  >
                    {enabled
                      ? t("common.active", { defaultValue: "ACTIVE" })
                      : t("common.disabled", { defaultValue: "OFF" })}
                  </Badge>
                ) : null}
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => openEditCron(c)}
                  disabled={!id || !editable}
                  title={
                    !editable
                      ? t("agents.detail.cron.edit_disabled_non_cron", {
                          defaultValue:
                            "This schedule uses 'every' or 'at' — edit it from agent.toml to avoid silent conversion to a cron expression.",
                        })
                      : undefined
                  }
                >
                  <Pencil className="w-3.5 h-3.5" />
                </Button>
                {isConfirming ? (
                  <div className="flex items-center gap-1">
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => confirmDeleteCron(id)}
                      disabled={deleteCron.isPending}
                    >
                      {t("common.confirm", { defaultValue: "Confirm" })}
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => setPendingDelete(null)}
                    >
                      {t("common.cancel", { defaultValue: "Cancel" })}
                    </Button>
                  </div>
                ) : (
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => setPendingDelete({ kind: "cron", id })}
                    disabled={!id}
                  >
                    <Trash2 className="w-3.5 h-3.5" />
                  </Button>
                )}
              </div>
            );
          })}
        </div>
      )}

      {/* ---- Triggers -------------------------------------------------------- */}
      <div className="flex items-center justify-between mt-2">
        <div className="text-[11px] uppercase font-semibold tracking-[0.08em] text-text-dim">
          {t("agents.detail.event_triggers", { defaultValue: "Event triggers" })}
        </div>
        <Button variant="ghost" size="sm" onClick={openCreateTrigger}>
          <Plus className="w-3.5 h-3.5 mr-1" />
          {t("agents.detail.add_trigger", { defaultValue: "Add trigger" })}
        </Button>
      </div>
      {triggers.length === 0 ? (
        <EmptyState
          icon={<Zap className="w-6 h-6" />}
          title={t("agents.detail.no_triggers", { defaultValue: "No event triggers" })}
        />
      ) : (
        <div className="flex flex-col gap-2">
          {triggers.map((tr) => {
            const enabled = tr.enabled !== false;
            const isConfirming =
              pendingDelete?.kind === "trigger" && pendingDelete.id === tr.id;
            return (
              <div
                key={tr.id}
                className={`px-3.5 py-3 rounded-lg border bg-main/40 flex items-center gap-3 transition-colors ${enabled ? "border-border-subtle" : "border-border-subtle/50 opacity-60"}`}
              >
                <div className="w-8 h-8 rounded-md bg-warning/10 text-warning grid place-items-center shrink-0">
                  <Zap className="w-4 h-4" />
                </div>
                <div className="flex-1 min-w-0">
                  <div className="font-mono text-[13px] truncate text-text-main">
                    {formatTriggerPattern(tr.pattern) || truncateId(tr.id, 12)}
                  </div>
                  {tr.prompt_template ? (
                    <div className="text-[11px] text-text-dim/80 mt-0.5 truncate">
                      {tr.prompt_template}
                    </div>
                  ) : tr.created_at ? (
                    <div className="text-[11px] text-text-dim/60 mt-0.5">
                      {formatRelativeTime(tr.created_at)}
                    </div>
                  ) : null}
                </div>
                <Badge
                  variant={enabled ? "brand" : "default"}
                  className="text-[9px] cursor-pointer"
                  onClick={() =>
                    updateTrigger.mutate(
                      { id: tr.id, data: { enabled: !enabled } },
                      {
                        onError: (err) =>
                          addToast(
                            err instanceof Error ? err.message : String(err),
                            "error",
                          ),
                      },
                    )
                  }
                >
                  {enabled
                    ? t("common.active", { defaultValue: "ACTIVE" })
                    : t("common.disabled", { defaultValue: "OFF" })}
                </Badge>
                <Button variant="ghost" size="sm" onClick={() => openEditTrigger(tr)}>
                  <Pencil className="w-3.5 h-3.5" />
                </Button>
                {isConfirming ? (
                  <div className="flex items-center gap-1">
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => confirmDeleteTrigger(tr.id)}
                      disabled={deleteTrigger.isPending}
                    >
                      {t("common.confirm", { defaultValue: "Confirm" })}
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => setPendingDelete(null)}
                    >
                      {t("common.cancel", { defaultValue: "Cancel" })}
                    </Button>
                  </div>
                ) : (
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => setPendingDelete({ kind: "trigger", id: tr.id })}
                  >
                    <Trash2 className="w-3.5 h-3.5" />
                  </Button>
                )}
              </div>
            );
          })}
        </div>
      )}

      {/* ---- Cron create / edit modal --------------------------------------- */}
      <DrawerPanel
        isOpen={cronModal !== null}
        onClose={() => {
          if (createCron.isPending || updateCron.isPending) return;
          setCronModal(null);
        }}
        title={
          cronModal?.mode === "edit"
            ? t("agents.detail.cron.edit", { defaultValue: "Edit cron job" })
            : t("agents.detail.cron.create", { defaultValue: "Create cron job" })
        }
        size="md"
      >
        <form onSubmit={handleCronSubmit} className="p-5 space-y-4">
          <div>
            <label
              htmlFor={`${formId}-cron-name`}
              className="text-[10px] font-bold text-text-dim uppercase"
            >
              {t("agents.detail.cron.name", { defaultValue: "Name" })}
            </label>
            <input
              id={`${formId}-cron-name`}
              value={cronName}
              onChange={(e) => setCronName(e.target.value)}
              placeholder="daily-summary"
              className={INPUT_CLASS}
              required
            />
          </div>
          <div>
            <label
              htmlFor={`${formId}-cron-expr`}
              className="text-[10px] font-bold text-text-dim uppercase"
            >
              {t("agents.detail.cron.expr", { defaultValue: "Cron expression" })}
            </label>
            <input
              id={`${formId}-cron-expr`}
              value={cronExpr}
              onChange={(e) => setCronExpr(e.target.value)}
              placeholder="0 9 * * *"
              className={`${INPUT_CLASS} font-mono`}
              required
            />
            <p className="text-[10px] text-text-dim/60 mt-1">
              {t("agents.detail.cron.expr_hint", {
                defaultValue: "5-field standard cron (min hr dom mon dow)",
              })}
            </p>
          </div>
          <div>
            <label
              htmlFor={`${formId}-cron-tz`}
              className="text-[10px] font-bold text-text-dim uppercase"
            >
              {t("agents.detail.cron.tz", { defaultValue: "Timezone (optional)" })}
            </label>
            <input
              id={`${formId}-cron-tz`}
              value={cronTz}
              onChange={(e) => setCronTz(e.target.value)}
              placeholder="UTC"
              className={INPUT_CLASS}
            />
          </div>
          <div>
            <label
              htmlFor={`${formId}-cron-message`}
              className="text-[10px] font-bold text-text-dim uppercase"
            >
              {t("agents.detail.cron.message", { defaultValue: "Message" })}
            </label>
            <textarea
              id={`${formId}-cron-message`}
              value={cronMessage}
              onChange={(e) => setCronMessage(e.target.value)}
              rows={3}
              placeholder={t("agents.detail.cron.message_placeholder", {
                defaultValue: "Message sent to the agent when the cron fires…",
              })}
              className={`${INPUT_CLASS} resize-none`}
            />
          </div>
          {cronModal?.mode === "edit" ? (
            <label className="flex items-center gap-2 text-[11px] text-text-dim">
              <input
                type="checkbox"
                checked={cronEnabled}
                onChange={(e) => setCronEnabled(e.target.checked)}
              />
              {t("agents.detail.cron.enabled", { defaultValue: "Enabled" })}
            </label>
          ) : null}
          {(createCron.error || updateCron.error) && (
            <div className="flex items-center gap-2 text-error text-xs">
              <AlertCircle className="w-4 h-4" />
              {(createCron.error || updateCron.error) instanceof Error
                ? (createCron.error || updateCron.error)?.toString()
                : String(createCron.error || updateCron.error)}
            </div>
          )}
          <div className="flex gap-2 pt-2">
            <Button
              type="submit"
              variant="primary"
              className="flex-1"
              disabled={createCron.isPending || updateCron.isPending}
            >
              {(createCron.isPending || updateCron.isPending) ? (
                <Loader2 className="w-4 h-4 animate-spin mr-1" />
              ) : (
                <Plus className="w-4 h-4 mr-1" />
              )}
              {cronModal?.mode === "edit"
                ? t("common.save", { defaultValue: "Save" })
                : t("agents.detail.cron.create", { defaultValue: "Create cron job" })}
            </Button>
            <Button
              type="button"
              variant="secondary"
              onClick={() => setCronModal(null)}
              disabled={createCron.isPending || updateCron.isPending}
            >
              {t("common.cancel", { defaultValue: "Cancel" })}
            </Button>
          </div>
        </form>
      </DrawerPanel>

      {/* ---- Trigger create / edit modal ------------------------------------ */}
      <DrawerPanel
        isOpen={triggerModal !== null}
        onClose={() => {
          if (createTrigger.isPending || updateTrigger.isPending) return;
          setTriggerModal(null);
        }}
        title={
          triggerModal?.mode === "edit"
            ? t("agents.detail.trigger.edit", { defaultValue: "Edit trigger" })
            : t("agents.detail.trigger.create", { defaultValue: "Create trigger" })
        }
        size="md"
      >
        <form onSubmit={handleTriggerSubmit} className="p-5 space-y-4">
          <div>
            <label
              htmlFor={`${formId}-trigger-pattern`}
              className="text-[10px] font-bold text-text-dim uppercase"
            >
              {t("agents.detail.trigger.pattern", { defaultValue: "Event pattern" })}
            </label>
            <select
              id={`${formId}-trigger-pattern`}
              value={trigPatternPreset}
              onChange={(e) => setTrigPatternPreset(e.target.value)}
              className={INPUT_CLASS}
            >
              {TRIGGER_PATTERN_PRESETS.map((p) => (
                <option key={p.value} value={p.value}>
                  {t(p.labelKey, { defaultValue: p.defaultLabel })}
                </option>
              ))}
            </select>
            {trigPatternPreset === "custom" && (
              <input
                aria-label={t("agents.detail.trigger.pattern_custom", {
                  defaultValue: "Custom event pattern (JSON or string)",
                })}
                value={trigPatternCustom}
                onChange={(e) => setTrigPatternCustom(e.target.value)}
                placeholder='e.g. "agent_spawned" or {"agent_spawned":{"name_pattern":"*"}}'
                className={`${INPUT_CLASS} mt-1 font-mono text-xs`}
              />
            )}
          </div>
          <div>
            <label
              htmlFor={`${formId}-trigger-prompt`}
              className="text-[10px] font-bold text-text-dim uppercase"
            >
              {t("agents.detail.trigger.prompt", {
                defaultValue: "Prompt template",
              })}
            </label>
            <textarea
              id={`${formId}-trigger-prompt`}
              value={trigPrompt}
              onChange={(e) => setTrigPrompt(e.target.value)}
              rows={3}
              placeholder={t("agents.detail.trigger.prompt_placeholder", {
                defaultValue: "Prompt sent to the agent when the event fires…",
              })}
              className={`${INPUT_CLASS} resize-none`}
            />
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div>
              <label
                htmlFor={`${formId}-trigger-max-fires`}
                className="text-[10px] font-bold text-text-dim uppercase"
              >
                {t("agents.detail.trigger.max_fires", {
                  defaultValue: "Max fires (0 = unlimited)",
                })}
              </label>
              <input
                id={`${formId}-trigger-max-fires`}
                type="number"
                min={0}
                value={trigMaxFires}
                onChange={(e) => setTrigMaxFires(Math.max(0, Number(e.target.value)))}
                className={INPUT_CLASS}
              />
            </div>
            <div>
              <label
                htmlFor={`${formId}-trigger-cooldown`}
                className="text-[10px] font-bold text-text-dim uppercase"
              >
                {t("agents.detail.trigger.cooldown", {
                  defaultValue: "Cooldown (seconds, blank = none)",
                })}
              </label>
              <input
                id={`${formId}-trigger-cooldown`}
                type="number"
                min={0}
                value={trigCooldown}
                onChange={(e) => setTrigCooldown(e.target.value)}
                placeholder="none"
                className={INPUT_CLASS}
              />
            </div>
          </div>
          <div>
            <label
              htmlFor={`${formId}-trigger-session-mode`}
              className="text-[10px] font-bold text-text-dim uppercase"
            >
              {t("agents.detail.trigger.session_mode", {
                defaultValue: "Session mode (blank = agent default)",
              })}
            </label>
            <select
              id={`${formId}-trigger-session-mode`}
              value={trigSessionMode}
              onChange={(e) => setTrigSessionMode(e.target.value)}
              className={INPUT_CLASS}
            >
              <option value="">{t("scheduler.agent_default", { defaultValue: "agent default" })}</option>
              <option value="persistent">persistent</option>
              <option value="new">new</option>
            </select>
          </div>
          {(createTrigger.error || updateTrigger.error) && (
            <div className="flex items-center gap-2 text-error text-xs">
              <AlertCircle className="w-4 h-4" />
              {(createTrigger.error || updateTrigger.error) instanceof Error
                ? (createTrigger.error || updateTrigger.error)?.toString()
                : String(createTrigger.error || updateTrigger.error)}
            </div>
          )}
          <div className="flex gap-2 pt-2">
            <Button
              type="submit"
              variant="primary"
              className="flex-1"
              disabled={createTrigger.isPending || updateTrigger.isPending}
            >
              {(createTrigger.isPending || updateTrigger.isPending) ? (
                <Loader2 className="w-4 h-4 animate-spin mr-1" />
              ) : (
                <Zap className="w-4 h-4 mr-1" />
              )}
              {triggerModal?.mode === "edit"
                ? t("common.save", { defaultValue: "Save" })
                : t("agents.detail.trigger.create", { defaultValue: "Create trigger" })}
            </Button>
            <Button
              type="button"
              variant="secondary"
              onClick={() => setTriggerModal(null)}
              disabled={createTrigger.isPending || updateTrigger.isPending}
            >
              {t("common.cancel", { defaultValue: "Cancel" })}
            </Button>
          </div>
        </form>
      </DrawerPanel>
    </div>
  );
}
