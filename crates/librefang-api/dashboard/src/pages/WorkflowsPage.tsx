import { formatDate } from "../lib/datetime";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "@tanstack/react-router";
import {
  type ApiActionResponse,
  type DryRunResult,
  type ScheduleItem,
  type TemplateParameter,
  type WorkflowItem,
  type WorkflowRunItem,
  type WorkflowRunInput,
  type WorkflowStep,
  type WorkflowStepResult,
  type WorkflowTemplate,
} from "../api";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Input } from "../components/ui/Input";
import { PageHeader } from "../components/ui/PageHeader";
import { useCreateShortcut } from "../lib/useCreateShortcut";
import { ListSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { ScheduleModal } from "../components/ui/ScheduleModal";
import {
  Layers, Trash2, FilePlus, Play, Search,
  Calendar, FileText, Activity, Bot, Loader2, Clock, ChevronRight,
  ChevronDown, FlaskConical, AlertCircle, CheckCircle2, SkipForward,
  GitBranch, Eye, SearchX,
} from "lucide-react";
import {
  useWorkflows,
  useWorkflowDetail,
  useWorkflowRuns,
  useWorkflowRunDetail,
  useWorkflowTemplates,
} from "../lib/queries/workflows";
import {
  useRunWorkflow,
  useDryRunWorkflow,
  useDeleteWorkflow,
  useInstantiateTemplate,
} from "../lib/mutations/workflows";
import { useCreateSchedule } from "../lib/mutations/schedules";
import { useUIStore } from "../lib/store";
import { extractImageRefs } from "../lib/workflowOutputImages";
import { WorkflowStepImageGallery } from "../components/WorkflowStepImageGallery";

const categoryIconMap: Record<string, React.ComponentType<{ className?: string }>> = {
  creation: FileText, language: Bot, thinking: Activity, business: Calendar,
};

// Per-category accent — keeps templates visually grouped at a glance.
// Uses Tailwind palette so it inherits dark/light theme via opacity.
const categoryAccent: Record<string, { text: string; bg: string; border: string; bar: string }> = {
  devtools:     { text: "text-violet-500",  bg: "bg-violet-500/10",  border: "border-violet-500/40",  bar: "from-violet-500/15" },
  productivity: { text: "text-sky-500",     bg: "bg-sky-500/10",     border: "border-sky-500/40",     bar: "from-sky-500/15" },
  sre:          { text: "text-rose-500",    bg: "bg-rose-500/10",    border: "border-rose-500/40",    bar: "from-rose-500/15" },
  sales:        { text: "text-emerald-500", bg: "bg-emerald-500/10", border: "border-emerald-500/40", bar: "from-emerald-500/15" },
  research:     { text: "text-amber-500",   bg: "bg-amber-500/10",   border: "border-amber-500/40",   bar: "from-amber-500/15" },
  admin:        { text: "text-slate-500",   bg: "bg-slate-500/10",   border: "border-slate-500/40",   bar: "from-slate-500/15" },
  creation:     { text: "text-sky-500",     bg: "bg-sky-500/10",     border: "border-sky-500/40",     bar: "from-sky-500/15" },
  language:     { text: "text-violet-500",  bg: "bg-violet-500/10",  border: "border-violet-500/40",  bar: "from-violet-500/15" },
  thinking:     { text: "text-emerald-500", bg: "bg-emerald-500/10", border: "border-emerald-500/40", bar: "from-emerald-500/15" },
  business:     { text: "text-amber-500",   bg: "bg-amber-500/10",   border: "border-amber-500/40",   bar: "from-amber-500/15" },
};
const fallbackAccent = { text: "text-text-dim", bg: "bg-main", border: "border-border-subtle", bar: "from-brand/10" };
const accentFor = (cat?: string) => (cat && categoryAccent[cat]) || fallbackAccent;

// Visual mapping for the last-run state pill on workflow rows.
// Mirrors WorkflowRunState from crates/librefang-kernel/src/workflow.rs —
// "paused" is treated as in-flight (warning hue), terminal failures are
// hard error.
const lastRunBadge = (state?: string): { bg: string; text: string; dot: string } => {
  switch (state) {
    case "completed": return { bg: "bg-success/10",  text: "text-success",  dot: "bg-success" };
    case "failed":    return { bg: "bg-error/10",    text: "text-error",    dot: "bg-error" };
    case "running":   return { bg: "bg-brand/10",    text: "text-brand",    dot: "bg-brand" };
    case "paused":    return { bg: "bg-warning/10",  text: "text-warning",  dot: "bg-warning" };
    case "pending":   return { bg: "bg-main",        text: "text-text-dim", dot: "bg-text-dim/40" };
    default:          return { bg: "bg-main",        text: "text-text-dim", dot: "bg-text-dim/40" };
  }
};

type CanvasTemplate = {
  nodes: Array<{
    id: string;
    type: string;
    position: { x: number; y: number };
    data: { label: string; prompt: string; nodeType: string };
  }>;
  edges: Array<{ id: string; source: string; target: string }>;
  name: string;
  description: string;
};

type StepResultLike = {
  id?: string;
  step_id?: string;
  step_name?: string;
  name?: string;
  agent_name?: string;
  duration_ms?: number;
  input_tokens?: number;
  output_tokens?: number;
};

type TemplateInstantiationResponse = ApiActionResponse & {
  workflow_id?: unknown;
  id?: unknown;
};

type WorkflowRunResponse = ApiActionResponse & {
  output?: unknown;
  message?: unknown;
  step_results?: unknown;
};

type WorkflowListItem = WorkflowItem & {
  run_count?: unknown;
  schedule?: Pick<ScheduleItem, "cron" | "tz" | "enabled"> | null;
};

const isWorkflowStepArray = (steps: WorkflowTemplate["steps"]): steps is WorkflowStep[] =>
  Array.isArray(steps);

const getTemplateSteps = (tmpl: WorkflowTemplate): WorkflowStep[] =>
  isWorkflowStepArray(tmpl.steps) ? tmpl.steps : [];

const getWorkflowSchedule = (wf: WorkflowItem): WorkflowListItem["schedule"] =>
  (wf as WorkflowListItem).schedule;

const getWorkflowRunCount = (wf: WorkflowItem): number => {
  const value = (wf as WorkflowListItem).run_count;
  return typeof value === "number" ? value : 0;
};

const getWorkflowIdFromResponse = (resp: TemplateInstantiationResponse): string | undefined => {
  const workflowId = resp.workflow_id;
  if (typeof workflowId === "string" && workflowId) return workflowId;
  return typeof resp.id === "string" && resp.id ? resp.id : undefined;
};

const getRunMutationData = (data: ApiActionResponse | undefined): WorkflowRunResponse | undefined =>
  data as WorkflowRunResponse | undefined;

const getRunOutputText = (data: ApiActionResponse | undefined): string => {
  const response = getRunMutationData(data);
  if (typeof response?.output === "string" && response.output) return response.output;
  if (typeof response?.message === "string" && response.message) return response.message;
  return JSON.stringify(data);
};

const getRunStepResults = (data: ApiActionResponse | undefined): WorkflowStepResult[] => {
  const stepResults = getRunMutationData(data)?.step_results;
  return Array.isArray(stepResults) ? (stepResults as WorkflowStepResult[]) : [];
};

const getErrorMessage = (error: unknown): string =>
  error instanceof Error ? error.message : String(error);

const isRunState = (state: WorkflowRunItem["state"]): state is string => typeof state === "string";

// Image detection is `O(n²)` worst-case on malformed JSON-ish output (a
// run of unmatched `{` triggers a fresh scan from each one). Memoize so
// unrelated re-renders of the parent (poll ticks, prop bag churn) don't
// re-pay the cost on long step outputs.
function StepResultContent({ step }: { step: WorkflowStepResult }) {
  const { t } = useTranslation();
  const imageRefs = useMemo(() => extractImageRefs(step.output), [step.output]);
  return (
    <div className="px-3 pb-3 space-y-2 border-t border-border-subtle">
      <div>
        <p className="text-[9px] font-bold text-text-dim/50 mt-2">{t("workflows.prompt_sent", { defaultValue: "Prompt sent:" })}</p>
        <pre className="text-[10px] text-text whitespace-pre-wrap max-h-24 overflow-y-auto bg-surface rounded-lg p-2 mt-1">
          {step.prompt || "(empty)"}
        </pre>
      </div>
      {imageRefs.length > 0 && (
        <WorkflowStepImageGallery
          refs={imageRefs}
          label={t("workflows.generated_images", { defaultValue: "Generated images:" })}
        />
      )}
      <div>
        <p className="text-[9px] font-bold text-text-dim/50">{t("workflows.output", { defaultValue: "Output:" })}</p>
        <pre className="text-[10px] text-text whitespace-pre-wrap max-h-24 overflow-y-auto bg-surface rounded-lg p-2 mt-1">
          {step.output || "(empty)"}
        </pre>
      </div>
      <p className="text-[9px] text-text-dim/40">
        {step.agent_name} · {step.input_tokens} in / {step.output_tokens} out tokens
      </p>
    </div>
  );
}

function StepAccordion<T>({
  steps,
  getKey,
  renderHeader,
  renderContent,
}: {
  steps: T[];
  getKey: (step: T, index: number) => string | number;
  renderHeader: (step: T, index: number, isExpanded: boolean, toggle: () => void) => React.ReactNode;
  renderContent: (step: T, index: number) => React.ReactNode;
}) {
  const [expandedIdx, setExpandedIdx] = useState<number | null>(null);

  return (
    <>
      {steps.map((step, i) => (
        <div key={getKey(step, i)} className="rounded-lg border border-border-subtle bg-main overflow-hidden">
          {renderHeader(step, i, expandedIdx === i, () => setExpandedIdx(expandedIdx === i ? null : i))}
          {expandedIdx === i && renderContent(step, i)}
        </div>
      ))}
    </>
  );
}

// ---------------------------------------------------------------------------
// Workflow parameter detection — mirrors the kernel's `to_template()` logic
// that scans `prompt_template` fields for `{{var}}` placeholders.
// ---------------------------------------------------------------------------

/** Reserved variable names that are set by the engine at runtime. */
const RESERVED_VARS = new Set(["input"]);

/**
 * Extract typed parameters from a workflow's steps by scanning prompt templates
 * for `{{var}}` placeholders.  Returns one `TemplateParameter` per unique
 * variable, excluding reserved names and step output variables (step names).
 */
function extractWorkflowParams(steps: WorkflowStep[]): TemplateParameter[] {
  const re = /\{\{(\w+)\}\}/g;
  const stepNames = new Set(steps.map((s) => s.name));
  const seen = new Set<string>();
  const params: TemplateParameter[] = [];

  for (const step of steps) {
    let match: RegExpExecArray | null;
    while ((match = re.exec(step.prompt_template)) !== null) {
      const name = match[1];
      if (seen.has(name) || RESERVED_VARS.has(name) || stepNames.has(name)) continue;
      seen.add(name);
      params.push({
        name,
        description: `Parameter '${name}' used in step '${step.name}'`,
        param_type: "string",
        required: true,
      });
    }
  }
  return params;
}

/**
 * Inline form fields for detected workflow parameters.  Each parameter gets
 * its own labeled input.  Values are stored in the parent via `onChange`.
 */
function WorkflowParamFields({
  params,
  values,
  onChange,
}: {
  params: TemplateParameter[];
  values: Record<string, string>;
  onChange: (values: Record<string, string>) => void;
}) {
  const { t } = useTranslation();

  const handleChange = useCallback(
    (name: string, value: string) => {
      onChange({ ...values, [name]: value });
    },
    [values, onChange],
  );

  if (params.length === 0) return null;

  return (
    <div className="space-y-2.5">
      <p className="text-[9px] font-bold uppercase tracking-widest text-text-dim/50">
        {t("workflows.parameters", { defaultValue: "Parameters" })}
      </p>
      {params.map((p) => (
        <label key={p.name} className="block space-y-1">
          <span className="text-[11px] font-semibold text-text">
            {p.name}
            {p.required && <span className="text-rose-500 ml-0.5">*</span>}
          </span>
          <input
            type="text"
            value={values[p.name] ?? (p.default != null ? String(p.default) : "")}
            onChange={(e) => handleChange(p.name, e.target.value)}
            placeholder={p.description ?? p.name}
            className="w-full rounded-lg border border-border-subtle bg-main px-3 py-1.5 text-sm outline-none focus:border-brand transition-colors"
            aria-required={p.required}
          />
          {p.description && (
            <p className="text-[10px] text-text-dim/50 leading-snug">{p.description}</p>
          )}
        </label>
      ))}
    </div>
  );
}

export function WorkflowsPage() {
  const { t, i18n } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);
  const navigate = useNavigate();
  const [selectedWorkflowId, setSelectedWorkflowId] = useState<string>("");
  const [runInput, setRunInput] = useState("");
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const [activeTab, setActiveTab] = useState<"workflows" | "templates">("workflows");
  const [scheduleWorkflowId, setScheduleWorkflowId] = useState<string | null>(null);
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null);
  const [dryRunResult, setDryRunResult] = useState<DryRunResult | null>(null);
  const [paramValues, setParamValues] = useState<Record<string, string>>({});

  const workflowsQuery = useWorkflows();
  const workflowDetailQuery = useWorkflowDetail(selectedWorkflowId);
  const runsQuery = useWorkflowRuns(selectedWorkflowId);
  const runDetailQuery = useWorkflowRunDetail(selectedRunId ?? "");
  const runMutation = useRunWorkflow();
  const dryRunMutation = useDryRunWorkflow();
  const deleteMutation = useDeleteWorkflow();
  const instantiateMutation = useInstantiateTemplate();
  const createScheduleMutation = useCreateSchedule();

  const workflows = useMemo(() =>
    [...(workflowsQuery.data ?? [])]
      .filter(wf => !searchQuery || wf.name?.toLowerCase().includes(searchQuery.toLowerCase()) || wf.description?.toLowerCase().includes(searchQuery.toLowerCase()))
      .sort((a, b) => (b.created_at ?? "").localeCompare(a.created_at ?? "")),
    [workflowsQuery.data, searchQuery]
  );
  const allWorkflows = workflowsQuery.data ?? [];
  const scheduledWf = useMemo(
    () => allWorkflows.find(w => w.id === scheduleWorkflowId),
    [allWorkflows, scheduleWorkflowId]
  );

  // Detect parameters from the selected workflow's prompt templates.
  // Uses the detail query (which includes full step objects with
  // prompt_template strings) rather than the list query (which may
  // only carry a step count).
  const detectedParams = useMemo(() => {
    const detail = workflowDetailQuery.data;
    if (!detail || !Array.isArray(detail.steps)) return [];
    return extractWorkflowParams(detail.steps as WorkflowStep[]);
  }, [workflowDetailQuery.data]);

  // Seed parameter values once per workflow when its detected params
  // first become available. The `useWorkflowDetail` query is async, so
  // on first render `selectedWorkflowId` is set but `detectedParams`
  // is still empty — a naive `prev !== cur` ref bailed out on initial
  // render and never seeded again, which silently lost every
  // `p.default` on the first selection. Track which workflow we've
  // seeded for instead, and seed only after detail data has actually
  // arrived. When the destination workflow truly has zero parameters,
  // clear `paramValues` outright — otherwise stale values from the
  // previously selected (parameterised) workflow would linger and any
  // future code that reads `paramValues` directly without re-deriving
  // from `detectedParams` would silently see them.
  const seededForRef = useRef<string | null>(null);
  useEffect(() => {
    if (!selectedWorkflowId) {
      seededForRef.current = null;
      return;
    }
    if (seededForRef.current === selectedWorkflowId) return;
    // Wait for detail data to actually arrive before deciding whether
    // this workflow has parameters. `useWorkflowDetail.isFetching`
    // distinguishes "still loading" from "loaded with zero params".
    if (workflowDetailQuery.isFetching || workflowDetailQuery.isLoading) return;
    if (detectedParams.length === 0) {
      // Loaded, no params — clear any stale values from a previously
      // selected workflow.
      setParamValues({});
      seededForRef.current = selectedWorkflowId;
      return;
    }
    const defaults: Record<string, string> = {};
    for (const p of detectedParams) {
      if (p.default != null) defaults[p.name] = String(p.default);
    }
    setParamValues(defaults);
    seededForRef.current = selectedWorkflowId;
  }, [
    selectedWorkflowId,
    detectedParams,
    workflowDetailQuery.isFetching,
    workflowDetailQuery.isLoading,
  ]);

  // First-time visitors with no workflows configured land on the
  // marketplace tab — instantiating a template is the obvious next
  // step. Fires once per mount; if the user manually flips back to
  // "My Workflows", we don't override on the next refetch.
  const autoSwitchedRef = useRef(false);
  useEffect(() => {
    if (autoSwitchedRef.current) return;
    if (!workflowsQuery.isSuccess) return;
    autoSwitchedRef.current = true;
    if ((workflowsQuery.data ?? []).length === 0) setActiveTab("templates");
  }, [workflowsQuery.isSuccess, workflowsQuery.data]);

  useEffect(() => {
    if (!workflowsQuery.isSuccess) return;
    if (workflows.length === 0) {
      if (selectedWorkflowId) {
        setSelectedWorkflowId("");
        setSelectedRunId(null);
        setRunInput("");
        setParamValues({});
        setDryRunResult(null);
      }
      return;
    }
    if (!selectedWorkflowId) {
      setSelectedWorkflowId(workflows[0]?.id ?? "");
      return;
    }
    if (!allWorkflows.some((workflow) => workflow.id === selectedWorkflowId)) {
      setSelectedRunId(null);
      setRunInput("");
      setParamValues({});
      setDryRunResult(null);
      setSelectedWorkflowId(workflows[0]?.id ?? "");
    }
  }, [allWorkflows, workflows, selectedWorkflowId, workflowsQuery.isSuccess]);

  // Build the effective input for a run.
  //
  // When the workflow declares `{{var}}` placeholders, the per-parameter
  // form values are sent as a JSON object so the kernel's
  // `seed_input_vars_from_json` (#4982) binds each `{{name}}` to the
  // value the user typed — a step prompt like "Brainstorm: {{challenge}}"
  // resolves the challenge text instead of leaving the literal
  // placeholder. The free-text textarea (additional context) rides along
  // under the `input` key. When there are no detected params, the plain
  // free-text string is sent unchanged so `{{input}}` keeps its original
  // whole-blob meaning (backward-compatible).
  const buildRunInput = useCallback((): WorkflowRunInput => {
    const hasParams = detectedParams.length > 0 &&
      Object.values(paramValues).some((v) => v.trim() !== "");
    if (!hasParams) return runInput;
    const obj: Record<string, string> = {};
    for (const p of detectedParams) {
      const val = paramValues[p.name]?.trim() ?? "";
      if (val) obj[p.name] = val;
    }
    if (runInput.trim()) obj.input = runInput.trim();
    return obj;
  }, [detectedParams, paramValues, runInput]);

  const handleRun = async () => {
    if (!selectedWorkflowId) return;
    setDryRunResult(null);
    dryRunMutation.reset();
    try {
      await runMutation.mutateAsync({ workflowId: selectedWorkflowId, input: buildRunInput() });
      addToast(t("workflows.run_started", { defaultValue: "Run started" }), "success");
    } catch (err) {
      addToast(
        err instanceof Error ? err.message : t("workflows.run_failed", { defaultValue: "Run failed" }),
        "error",
      );
    }
  };

  const handleDryRun = async () => {
    if (!selectedWorkflowId) return;
    setDryRunResult(null);
    runMutation.reset();
    try {
      const result = await dryRunMutation.mutateAsync({ workflowId: selectedWorkflowId, input: buildRunInput() });
      setDryRunResult(result);
    } catch {
      // Error already surfaced via dryRunMutation.error panel at line 465.
    }
  };


  const handleDelete = async (id: string) => {
    if (confirmDeleteId !== id) { setConfirmDeleteId(id); return; }
    setConfirmDeleteId(null);
    try {
      await deleteMutation.mutateAsync(id);
    } catch (err) {
      addToast(
        err instanceof Error ? err.message : t("workflows.delete_failed", { defaultValue: "Delete failed" }),
        "error",
      );
    }
  };

  const navigateToCanvas = (wfId?: string, template?: CanvasTemplate) => {
    sessionStorage.removeItem("canvasNodes");
    sessionStorage.removeItem("workflowTemplate");
    if (template) {
      sessionStorage.setItem("workflowTemplate", JSON.stringify(template));
    }
    navigate({ to: "/canvas", search: { t: wfId ? undefined : Date.now(), wf: wfId } });
  };

  const handleNewWorkflow = () => {
    navigateToCanvas();
  };
  useCreateShortcut(handleNewWorkflow);

  const buildCanvasTemplateFor = (tmpl: WorkflowTemplate): CanvasTemplate => {
    const steps = getTemplateSteps(tmpl);
    const nameToIdx = new Map(steps.map((s, i) => [s.name, i]));
    const nodes = steps.map((s, idx) => ({
      id: `node-${idx}`,
      type: "custom",
      position: { x: 50 + idx * 260, y: 100 },
      data: { label: s.name, prompt: s.prompt_template || "", nodeType: "agent" },
    }));
    const edges: CanvasTemplate["edges"] = [];
    steps.forEach((s, idx) => {
      (s.depends_on ?? []).forEach((dep: string) => {
        const src = nameToIdx.get(dep);
        if (src !== undefined) edges.push({ id: `e-${src}-${idx}`, source: `node-${src}`, target: `node-${idx}` });
      });
    });
    if (edges.length === 0 && nodes.length > 1) {
      nodes.slice(0, -1).forEach((_, i) =>
        edges.push({ id: `e-${i}`, source: `node-${i}`, target: `node-${i + 1}` })
      );
    }
    return { nodes, edges, name: tmpl.name, description: tmpl.description ?? "" };
  };

  // Preview opens the template in canvas without persisting anything —
  // the user can iterate on layout / prompts before deciding to save.
  const handlePreviewTemplate = (tmpl: WorkflowTemplate) => {
    navigateToCanvas(undefined, buildCanvasTemplateFor(tmpl));
  };

  const handleUseTemplate = async (tmpl: WorkflowTemplate) => {
    const canvasTpl = buildCanvasTemplateFor(tmpl);
    const hasRequiredParams = (tmpl.parameters ?? []).some(p => p.required);
    if (hasRequiredParams) {
      // Template has required params — open canvas pre-populated with nodes so
      // the user can see the workflow structure and fill in parameter values.
      navigateToCanvas(undefined, canvasTpl);
      return;
    }
    try {
      const resp = await instantiateMutation.mutateAsync({ id: tmpl.id, params: {} });
      const workflowId = getWorkflowIdFromResponse(resp as TemplateInstantiationResponse);
      if (workflowId) {
        openWorkflow(workflowId);
      } else {
        // Instantiation succeeded but no ID returned — fall back to pre-populated canvas
        navigateToCanvas(undefined, canvasTpl);
      }
    } catch {
      navigateToCanvas(undefined, canvasTpl);
    }
  };

  const openWorkflow = (wfId: string) => {
    navigateToCanvas(wfId);
  };

  const templatesQuery = useWorkflowTemplates();
  const apiTemplates = templatesQuery.data ?? [];
  const lang = i18n.language?.split("-")[0] ?? "en";
  const tmplName = (tmpl: WorkflowTemplate) => tmpl.i18n?.[lang]?.name || tmpl.name;
  const tmplDesc = (tmpl: WorkflowTemplate) => tmpl.i18n?.[lang]?.description || tmpl.description;

  const hasWorkflows = workflows.length > 0;
  const [tplCategory, setTplCategory] = useState<string>("all");
  const tplCategories = useMemo(() => {
    const map = new Map<string, number>();
    apiTemplates.forEach((t) => {
      const c = t.category || "uncategorized";
      map.set(c, (map.get(c) ?? 0) + 1);
    });
    return Array.from(map.entries()).sort((a, b) => b[1] - a[1]);
  }, [apiTemplates]);
  const filteredTemplates = useMemo(() => {
    const q = searchQuery.trim().toLowerCase();
    return apiTemplates.filter((t) => {
      if (tplCategory !== "all" && (t.category || "uncategorized") !== tplCategory) return false;
      if (!q) return true;
      const hay = `${tmplName(t)} ${tmplDesc(t) ?? ""} ${(t.tags ?? []).join(" ")}`.toLowerCase();
      return hay.includes(q);
    });
  // tmplName / tmplDesc are stable per language change; depend on language token
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [apiTemplates, tplCategory, searchQuery, lang]);

  const getStepResultKey = (step: StepResultLike, index: number) =>
    step.id ?? step.step_id ?? step.step_name ?? step.name ?? ([step.agent_name, step.duration_ms, step.input_tokens, step.output_tokens].filter(Boolean).join(":") || index);

  const stepResultHeader = (step: WorkflowStepResult, _idx: number, isExpanded: boolean, toggle: () => void) => (
    <button
      className="w-full flex items-center gap-2 px-3 py-2 text-left hover:bg-surface transition-colors"
      onClick={toggle}>
      <CheckCircle2 className="w-3 h-3 text-success shrink-0" />
      <span className="text-[10px] font-bold truncate flex-1">{step.step_name}</span>
      <span className="text-[9px] text-text-dim/50 shrink-0">{step.duration_ms}ms</span>
      <ChevronDown className={`w-3 h-3 text-text-dim/30 shrink-0 transition-transform ${isExpanded ? "rotate-180" : ""}`} />
    </button>
  );

  const stepResultContent = (step: WorkflowStepResult, _idx: number) => (
    <StepResultContent step={step} />
  );

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("workflows.automation_hub")}
        title={t("workflows.title")}
        subtitle={
          <span className="font-mono text-text-dim/70">
            {t("workflows.flows_count", { defaultValue: "{{n}} flows", n: allWorkflows.length })}
            <span className="px-1.5 text-text-dim/40">·</span>
            {t("workflows.templates_count_meta", { defaultValue: "{{n}} templates", n: apiTemplates.length })}
            <span className="px-1.5 text-text-dim/40">·</span>
            <span className="text-text-dim/50">/api/workflows</span>
          </span>
        }
        isFetching={workflowsQuery.isFetching}
        onRefresh={() => void workflowsQuery.refetch()}
        icon={<Layers className="h-4 w-4" />}
        helpText={t("workflows.help")}
        actions={hasWorkflows ?
          <Button variant="primary" onClick={handleNewWorkflow} title={t("workflows.create_blank") + " (n)"}>
            <FilePlus className="h-4 w-4" />
            <span>{t("workflows.create_blank")}</span>
            <kbd className="hidden sm:inline-flex h-5 min-w-[20px] items-center justify-center rounded border border-white/30 bg-white/10 px-1 text-[9px] font-mono font-semibold">n</kbd>
          </Button> : undefined
        }
      />

      {/* Tabs */}
      <div role="tablist" aria-label={t("nav.workflows", { defaultValue: "Workflows" })} className="flex items-center gap-1 border-b border-border-subtle">
        <button
          id="workflows-tab-workflows"
          role="tab"
          aria-selected={activeTab === "workflows"}
          aria-controls="workflows-panel-workflows"
          tabIndex={activeTab === "workflows" ? 0 : -1}
          onClick={() => setActiveTab("workflows")}
          className={`px-4 py-2.5 text-sm font-bold transition-colors border-b-2 -mb-px ${
            activeTab === "workflows"
              ? "border-brand text-brand"
              : "border-transparent text-text-dim hover:text-brand/70"
          }`}
        >
          {t("workflows.my_workflows")}
          {workflows.length > 0 && <span className="ml-1.5 text-[10px] font-semibold px-1.5 py-0.5 rounded-full bg-brand/10 text-brand">{workflows.length}</span>}
        </button>
        <button
          id="workflows-tab-templates"
          role="tab"
          aria-selected={activeTab === "templates"}
          aria-controls="workflows-panel-templates"
          tabIndex={activeTab === "templates" ? 0 : -1}
          onClick={() => setActiveTab("templates")}
          className={`px-4 py-2.5 text-sm font-bold transition-colors border-b-2 -mb-px ${
            activeTab === "templates"
              ? "border-brand text-brand"
              : "border-transparent text-text-dim hover:text-brand/70"
          }`}
        >
          {t("workflows.template_library")}
          {apiTemplates.length > 0 && <span className="ml-1.5 text-[10px] font-semibold px-1.5 py-0.5 rounded-full bg-brand/10 text-brand">{apiTemplates.length}</span>}
        </button>
      </div>

      {/* Templates Tab */}
      {activeTab === "templates" && (
        <div id="workflows-panel-templates" role="tabpanel" aria-labelledby="workflows-tab-templates" className="space-y-4">
          {/* Search + category filter row */}
          {apiTemplates.length > 0 && (
            <div className="flex items-center gap-2 flex-wrap">
              <Input
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                placeholder={t("workflows.search_templates_placeholder", { defaultValue: "Search templates…" })}
                leftIcon={<Search className="h-4 w-4" />}
                className="sm:w-72"
              />
              <div className="flex items-center gap-1 flex-wrap">
                {([["all", apiTemplates.length], ...tplCategories] as Array<[string, number]>).map(([id, count]) => {
                  const active = tplCategory === id;
                  const a = id === "all" ? fallbackAccent : accentFor(id);
                  return (
                    <button
                      key={id}
                      onClick={() => setTplCategory(id)}
                      className={`inline-flex items-center gap-1.5 px-3 py-1 rounded-full text-[11px] font-semibold border capitalize transition-colors ${
                        active
                          ? `${a.text} ${a.bg} ${a.border}`
                          : "text-text-dim border-border-subtle hover:text-text bg-surface"
                      }`}
                    >
                      {id === "all" ? t("common.all", { defaultValue: "All" }) : id}
                      <span className="font-mono text-[10px] text-text-dim/60">{count}</span>
                    </button>
                  );
                })}
              </div>
            </div>
          )}

          {filteredTemplates.length > 0 ? (
            <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
              {filteredTemplates.map((tmpl) => {
                const Icon = categoryIconMap[tmpl.category || ""] || Layers;
                const a = accentFor(tmpl.category);
                const stepCount = tmpl.steps?.length ?? 0;
                const requiredParams = (tmpl.parameters ?? []).filter((p) => p.required);
                const optionalParams = (tmpl.parameters ?? []).filter((p) => !p.required);
                return (
                  <div
                    key={tmpl.id}
                    className="group flex flex-col rounded-2xl border border-border-subtle bg-surface overflow-hidden hover:border-brand/30 hover:shadow-md transition-colors"
                  >
                    <div className={`px-4 pt-3 pb-2.5 border-b border-border-subtle bg-gradient-to-br ${a.bar} to-transparent`}>
                      <div className="flex items-center gap-2">
                        <span className={`inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] font-bold uppercase tracking-wider ${a.bg} ${a.text}`}>
                          <Icon className="w-3 h-3" />
                          {tmpl.category || "uncategorized"}
                        </span>
                        <span className="ml-auto font-mono text-[10px] text-text-dim/60">
                          {stepCount} {t("workflows.steps_unit", { defaultValue: "steps" })}
                        </span>
                      </div>
                      <p className="mt-1.5 text-sm font-bold truncate">{tmplName(tmpl)}</p>
                      <p className="mt-0.5 font-mono text-[10px] text-text-dim/60 truncate">{tmpl.id}</p>
                    </div>
                    <div className="px-4 py-3 flex-1">
                      <p className="text-[11px] leading-snug text-text-dim line-clamp-3">{tmplDesc(tmpl)}</p>
                      {(tmpl.parameters?.length ?? 0) > 0 && (
                        <div className="mt-2.5">
                          <p className="text-[9px] font-bold uppercase tracking-wider text-text-dim/50 mb-1">
                            {t("workflows.parameters", { defaultValue: "Parameters" })} · {tmpl.parameters?.length}
                          </p>
                          <div className="flex flex-wrap gap-1">
                            {[...requiredParams, ...optionalParams].slice(0, 6).map((p) => (
                              <span
                                key={p.name}
                                className="font-mono text-[10px] px-1.5 py-0.5 rounded border border-border-subtle bg-main text-text-dim"
                                title={p.description ?? p.name}
                              >
                                {p.name}{p.required ? <span className="text-rose-500 ml-0.5">*</span> : null}
                              </span>
                            ))}
                            {(tmpl.parameters?.length ?? 0) > 6 && (
                              <span className="font-mono text-[10px] px-1.5 py-0.5 text-text-dim/60">
                                +{(tmpl.parameters?.length ?? 0) - 6}
                              </span>
                            )}
                          </div>
                        </div>
                      )}
                      {tmpl.tags && tmpl.tags.length > 0 && (
                        <div className="mt-2.5 flex flex-wrap gap-1">
                          {tmpl.tags.slice(0, 5).map((tag) => (
                            <span key={tag} className="text-[10px] px-1.5 py-0.5 rounded-full border border-border-subtle text-text-dim/70">
                              {tag}
                            </span>
                          ))}
                        </div>
                      )}
                    </div>
                    <div className="px-4 py-2.5 border-t border-border-subtle flex items-center gap-2">
                      <Button
                        variant="primary"
                        className="flex-1 justify-center"
                        onClick={() => handleUseTemplate(tmpl)}
                        disabled={instantiateMutation.isPending}
                      >
                        {instantiateMutation.isPending
                          ? <Loader2 className="w-3.5 h-3.5 animate-spin" />
                          : <FilePlus className="w-3.5 h-3.5" />}
                        <span className="text-[11px]">{t("workflows.use_template", { defaultValue: "Use template" })}</span>
                      </Button>
                      <Button
                        variant="secondary"
                        onClick={() => handlePreviewTemplate(tmpl)}
                        title={t("workflows.preview_template", { defaultValue: "Preview in canvas" })}
                      >
                        <Eye className="w-3.5 h-3.5" />
                      </Button>
                    </div>
                  </div>
                );
              })}
            </div>
          ) : apiTemplates.length === 0 ? (
            <div className="py-12 text-center text-text-dim text-sm">{t("common.no_data")}</div>
          ) : (
            <div className="py-12 text-center text-text-dim">
              <SearchX className="w-7 h-7 mx-auto mb-2 text-text-dim/50" />
              <p className="text-sm">{t("workflows.no_templates_match", { defaultValue: "No templates match." })}</p>
            </div>
          )}
        </div>
      )}

      {/* Workflows Tab */}
      {activeTab === "workflows" && (
        <div id="workflows-panel-workflows" role="tabpanel" aria-labelledby="workflows-tab-workflows">
          {/* Search Bar */}
          {hasWorkflows && (
            <Input value={searchQuery} onChange={e => setSearchQuery(e.target.value)}
              placeholder={t("workflows.search_placeholder")}
              leftIcon={<Search className="h-4 w-4" />}
              data-shortcut-search />
          )}

          {/* Loading Skeleton */}
          {workflowsQuery.isLoading && (
            <ListSkeleton rows={3} />
          )}

      {/* Main Content Area */}
      {hasWorkflows ? (
        <div className="grid gap-6 lg:grid-cols-[1fr_300px] xl:grid-cols-[1fr_340px]">
          {/* Workflow List */}
          <div className="space-y-1.5">
            <h2 className="text-[10px] font-bold uppercase tracking-widest text-text-dim/50 mb-1.5 flex items-center gap-2">
              <span>{t("workflows.all_workflows")}</span>
              <span className="font-mono text-text-dim/40">{workflows.length}</span>
            </h2>
            {workflows.map(wf => {
              const schedule = getWorkflowSchedule(wf);
              const runCount = getWorkflowRunCount(wf);
              const stepCount = Array.isArray(wf.steps) ? wf.steps.length : (wf.steps || 0);
              const isSelected = selectedWorkflowId === wf.id;
              const confirming = confirmDeleteId === wf.id;
              const lastRun = wf.last_run ?? null;
              const successRate = typeof wf.success_rate === "number" ? wf.success_rate : null;
              const lastRunStyle = lastRunBadge(lastRun?.state);
              return (
                <div
                  key={wf.id}
                  onClick={() => setSelectedWorkflowId(wf.id)}
                  onDoubleClick={() => openWorkflow(wf.id)}
                  className={`group grid items-center gap-3 px-3.5 py-2.5 rounded-xl border cursor-pointer transition-colors
                    grid-cols-[1fr_auto] sm:grid-cols-[1fr_110px_120px_90px_auto]
                    ${isSelected
                      ? "border-brand bg-brand/5"
                      : "border-border-subtle bg-surface hover:border-brand/30 hover:bg-main/40"}`}
                >
                  {/* Name + description */}
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <GitBranch className="w-3.5 h-3.5 text-brand shrink-0" />
                      <span className="font-mono text-[13px] font-bold truncate">{wf.name}</span>
                      <span className="font-mono text-[10px] text-text-dim/60 shrink-0">{stepCount} steps</span>
                    </div>
                    <p className="text-[11px] text-text-dim mt-0.5 truncate">{wf.description || t("common.no_data")}</p>
                  </div>

                  {/* Last-run state pill — desktop */}
                  <div className="hidden sm:flex items-center min-w-0">
                    {lastRun ? (
                      <span className={`inline-flex items-center gap-1 px-1.5 py-0.5 rounded-full text-[10px] font-mono ${lastRunStyle.bg} ${lastRunStyle.text}`}>
                        <span className={`w-1.5 h-1.5 rounded-full ${lastRunStyle.dot} ${lastRun.state === "running" ? "animate-pulse" : ""}`} />
                        {lastRun.state}
                      </span>
                    ) : (
                      <span className="font-mono text-[10px] text-text-dim/40">no runs</span>
                    )}
                  </div>

                  {/* Schedule pill — desktop */}
                  <div className="hidden sm:flex items-center min-w-0">
                    {schedule ? (
                      <span className={`inline-flex items-center gap-1 px-1.5 py-0.5 rounded-full text-[10px] font-mono truncate ${schedule.enabled ? "bg-success/10 text-success" : "bg-main text-text-dim"}`}>
                        <Calendar className="w-3 h-3 shrink-0" />
                        <span className="truncate">{schedule.cron}</span>
                      </span>
                    ) : (
                      <span className="font-mono text-[10px] text-text-dim/40">no schedule</span>
                    )}
                  </div>

                  {/* Success rate + last-run time / runs — desktop */}
                  <div className="hidden sm:block font-mono text-[10px] text-text-dim/70 leading-tight">
                    <div className="flex items-center gap-1">
                      {successRate !== null
                        ? <><span className="text-text">{Math.round(successRate * 100)}%</span><span className="text-text-dim/50">ok</span></>
                        : <><Play className="w-2.5 h-2.5" />{runCount} {t("workflows.runs_label", { defaultValue: "runs" })}</>}
                    </div>
                    <div className="flex items-center gap-1 text-text-dim/50 mt-0.5">
                      <Clock className="w-2.5 h-2.5" />
                      {lastRun ? formatDate(lastRun.started_at) : formatDate(wf.created_at)}
                    </div>
                  </div>

                  {/* Actions */}
                  <div className="flex items-center gap-0.5 shrink-0" onClick={e => e.stopPropagation()}>
                    <button onClick={() => setScheduleWorkflowId(wf.id)}
                      className={`p-1.5 rounded-lg transition-colors ${schedule ? "text-success hover:bg-success/10" : "text-text-dim/40 hover:text-brand hover:bg-brand/10"}`}
                      title={t("nav.scheduler")}>
                      <Calendar className="w-3.5 h-3.5" />
                    </button>
                    {confirming ? (
                      <div className="flex items-center gap-1">
                        <button onClick={() => handleDelete(wf.id)} className="px-2 py-1 rounded-lg bg-error text-white text-[10px] font-bold">{t("common.confirm")}</button>
                        <button onClick={() => setConfirmDeleteId(null)} className="px-2 py-1 rounded-lg bg-main text-text-dim text-[10px] font-bold">{t("common.cancel")}</button>
                      </div>
                    ) : (
                      <button onClick={() => handleDelete(wf.id)}
                        className="p-1.5 rounded-lg text-text-dim/30 hover:text-error hover:bg-error/10 transition-colors"
                        aria-label={t("common.delete")}>
                        <Trash2 className="w-3.5 h-3.5" />
                      </button>
                    )}
                    <button onClick={() => openWorkflow(wf.id)}
                      className="p-1.5 rounded-lg text-text-dim/40 hover:text-brand hover:bg-brand/10 transition-colors"
                      title={t("canvas.ctx_edit")}>
                      <ChevronRight className="w-4 h-4" />
                    </button>
                  </div>
                </div>
              );
            })}
            {workflows.length === 0 && searchQuery && (
              <div className="py-10 text-center text-text-dim">
                <SearchX className="w-7 h-7 mx-auto mb-2 text-text-dim/50" />
                <p className="text-sm">
                  {t("workflows.no_match", { defaultValue: "No workflows match." })}
                </p>
              </div>
            )}
          </div>

          {/* Right Panel: shown when a workflow is selected */}
          {selectedWorkflowId && (
            <div className="space-y-4">
              <Card padding="lg" className="sticky top-4 space-y-3">
                <h3 className="text-xs font-bold uppercase tracking-widest text-text-dim/50">{t("workflows.run_workflow")}</h3>
                {detectedParams.length > 0 && (
                  <WorkflowParamFields
                    params={detectedParams}
                    values={paramValues}
                    onChange={setParamValues}
                  />
                )}
                <textarea value={runInput} onChange={e => setRunInput(e.target.value)}
                  placeholder={
                    detectedParams.length > 0
                      ? t("workflows.additional_context_placeholder", { defaultValue: "Additional context (optional)..." })
                      : t("canvas.run_input_placeholder")
                  }
                  rows={detectedParams.length > 0 ? 2 : 4}
                  className="w-full rounded-xl border border-border-subtle bg-main px-4 py-2.5 text-sm outline-none focus:border-brand resize-none" />
                <div className="flex gap-2">
                  <Button variant="primary" className="flex-1" disabled={runMutation.isPending || dryRunMutation.isPending} onClick={handleRun}>
                    {runMutation.isPending ? <Loader2 className="w-4 h-4 animate-spin mr-2" /> : <Play className="w-4 h-4 mr-2" />}
                    {t("canvas.run_now")}
                  </Button>
                  <Button variant="secondary" disabled={runMutation.isPending || dryRunMutation.isPending} onClick={handleDryRun}
                    title={t("workflows.dry_run_hint")}>
                    {dryRunMutation.isPending ? <Loader2 className="w-4 h-4 animate-spin" /> : <FlaskConical className="w-4 h-4" />}
                    <span className="hidden sm:inline ml-1.5">{t("workflows.dry_run")}</span>
                  </Button>
                </div>

                {/* Dry-run result */}
                {dryRunResult && (
                  <div className={`p-3 rounded-xl border ${dryRunResult.valid ? "bg-success/5 border-success/20" : "bg-warning/5 border-warning/20"}`}>
                    <div className="flex items-center gap-2 mb-2">
                      {dryRunResult.valid
                        ? <CheckCircle2 className="w-3.5 h-3.5 text-success" />
                        : <AlertCircle className="w-3.5 h-3.5 text-warning" />}
                      <p className={`text-[10px] font-bold ${dryRunResult.valid ? "text-success" : "text-warning"}`}>
                        {dryRunResult.valid ? t("workflows.dry_run_valid") : t("workflows.dry_run_warning")}
                      </p>
                    </div>
                    <div className="space-y-2">
                      <StepAccordion
                        steps={dryRunResult.steps}
                        getKey={getStepResultKey}
                        renderHeader={(step, _i, isExpanded, toggle) => (
                          <button
                            className="w-full flex items-center gap-2 px-3 py-2 text-left hover:bg-surface transition-colors"
                            onClick={toggle}>
                            {step.skipped
                              ? <SkipForward className="w-3 h-3 text-text-dim/40 shrink-0" />
                              : step.agent_found
                                ? <CheckCircle2 className="w-3 h-3 text-success shrink-0" />
                                : <AlertCircle className="w-3 h-3 text-warning shrink-0" />}
                            <span className="text-[10px] font-bold truncate flex-1">{step.step_name}</span>
                            {step.agent_name && (
                              <span className="text-[9px] text-text-dim/50 shrink-0">{step.agent_name}</span>
                            )}
                            {step.skipped && (
                              <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-main border border-border-subtle text-text-dim/50 shrink-0">{t("workflows.skip", { defaultValue: "skip" })}</span>
                            )}
                            <ChevronDown className={`w-3 h-3 text-text-dim/30 shrink-0 transition-transform ${isExpanded ? "rotate-180" : ""}`} />
                          </button>
                        )}
                        renderContent={(step) => (
                          <div className="px-3 pb-3 space-y-1.5 border-t border-border-subtle">
                            {!step.agent_found && (
                              <p className="text-[10px] text-warning mt-2">{t("workflows.agent_not_found", { defaultValue: "Agent not found" })}</p>
                            )}
                            {step.skip_reason && (
                              <p className="text-[10px] text-text-dim mt-2">{step.skip_reason}</p>
                            )}
                            <p className="text-[9px] font-bold text-text-dim/50 mt-2">{t("workflows.resolved_prompt", { defaultValue: "Resolved prompt:" })}</p>
                            <pre className="text-[10px] text-text whitespace-pre-wrap max-h-28 overflow-y-auto bg-surface rounded-lg p-2">
                              {step.resolved_prompt || "(empty)"}
                            </pre>
                          </div>
                        )}
                      />
                    </div>
                  </div>
                )}

                {/* Run Result */}
                {runMutation.data && (
                  <div className="p-3 rounded-xl bg-success/5 border border-success/20 space-y-2">
                    <p className="text-[10px] font-bold text-success">{t("canvas.run_result")}</p>
                    <pre className="text-xs text-text whitespace-pre-wrap max-h-32 overflow-y-auto">
                      {getRunOutputText(runMutation.data)}
                    </pre>
                    {/* Step-level I/O */}
                    {getRunStepResults(runMutation.data).length > 0 && (
                      <div className="space-y-1.5 border-t border-success/20 pt-2">
                        <p className="text-[9px] font-bold text-text-dim/50">{t("workflows.step_details", { defaultValue: "Step details" })}</p>
                        <StepAccordion
                          steps={getRunStepResults(runMutation.data)}
                          getKey={getStepResultKey}
                          renderHeader={stepResultHeader}
                          renderContent={stepResultContent}
                        />
                      </div>
                    )}
                  </div>
                )}
                {runMutation.error && (
                  <div className="p-3 rounded-xl bg-error/5 border border-error/20">
                    <div className="flex items-center gap-1.5 mb-1">
                      <AlertCircle className="w-3.5 h-3.5 text-error shrink-0" />
                      <p className="text-[10px] font-bold text-error">{t("workflows.run_failed", { defaultValue: "Run failed" })}</p>
                    </div>
                    <p className="text-xs text-error/80">
                      {getErrorMessage(runMutation.error)}
                    </p>
                  </div>
                )}
                {dryRunMutation.error && (
                  <div className="p-3 rounded-xl bg-error/5 border border-error/20">
                    <p className="text-xs text-error">
                      {getErrorMessage(dryRunMutation.error)}
                    </p>
                  </div>
                )}
              </Card>

              {/* Run History */}
              {runsQuery.data && runsQuery.data.length > 0 && (
                <Card padding="lg" className="space-y-3">
                  <h3 className="text-xs font-bold uppercase tracking-widest text-text-dim/50">{t("workflows.run_history", { defaultValue: "Run History" })}</h3>
                  <div className="space-y-1.5">
                    {runsQuery.data.slice(0, 10).map((run) => {
                      const runId = run.id;
                      const state = isRunState(run.state) ? run.state : undefined;
                      const isSelected = selectedRunId === runId;
                      return (
                        <div key={runId}>
                          <button
                            className={`w-full flex items-center gap-3 p-2.5 rounded-xl border text-left transition-colors ${
                              isSelected
                                ? "border-brand bg-brand/5"
                                : "border-border-subtle bg-main hover:bg-surface"
                            }`}
                            onClick={() => {
                              setSelectedRunId(isSelected ? null : (runId ?? null));
                            }}>
                            <div className={`w-2 h-2 rounded-full shrink-0 ${
                              state === "completed" ? "bg-success" :
                              state === "failed" ? "bg-error" :
                              state === "running" ? "bg-brand animate-pulse" : "bg-text-dim/30"
                            }`} />
                            <div className="flex-1 min-w-0">
                              <p className="text-[10px] font-bold truncate">{run.workflow_name}</p>
                              <p className="text-[9px] text-text-dim/50">{formatDate(run.started_at)}</p>
                            </div>
                            <span className={`text-[9px] font-semibold px-1.5 py-0.5 rounded-full shrink-0 ${
                              state === "completed" ? "bg-success/10 text-success" :
                              state === "failed" ? "bg-error/10 text-error" :
                              "bg-main text-text-dim"
                            }`}>{state}</span>
                          </button>
                          {/* Inline run detail */}
                          {isSelected && runDetailQuery.data && (
                            <div className="ml-5 mt-1 space-y-1.5">
                              {runDetailQuery.data.error && (
                                <div className="flex items-start gap-1.5 p-2 rounded-lg bg-error/5 border border-error/20">
                                  <AlertCircle className="w-3 h-3 text-error shrink-0 mt-0.5" />
                                  <p className="text-[10px] text-error">{runDetailQuery.data.error}</p>
                                </div>
                              )}
                              <StepAccordion
                                steps={runDetailQuery.data.step_results}
                                getKey={getStepResultKey}
                                renderHeader={stepResultHeader}
                                renderContent={stepResultContent}
                              />
                            </div>
                          )}
                          {isSelected && runDetailQuery.isLoading && (
                            <div className="ml-5 mt-1 p-2 text-[10px] text-text-dim/50 flex items-center gap-1.5">
                              <Loader2 className="w-3 h-3 animate-spin" /> {t("workflows.loading_details", { defaultValue: "Loading details…" })}
                            </div>
                          )}
                        </div>
                      );
                    })}
                  </div>
                </Card>
              )}
            </div>
          )}
        </div>
      ) : (
        /* Empty State */
        !workflowsQuery.isLoading && (
          <EmptyState
            icon={<Layers className="w-7 h-7" />}
            title={t("workflows.empty_title")}
            description={t("workflows.empty_desc")}
            action={
              <div className="flex items-center justify-center gap-3">
                <Button variant="primary" onClick={() => handleNewWorkflow()}>
                  <FilePlus className="w-4 h-4" />
                  {t("workflows.create_blank")}
                </Button>
                {apiTemplates.length > 0 && (
                  <Button variant="secondary" onClick={() => setActiveTab("templates")}>
                    <Layers className="w-4 h-4" />
                    {t("workflows.template_library")}
                  </Button>
                )}
              </div>
            }
          />
        )
      )}
        </div>
      )}
      {/* Schedule Modal */}
      {scheduleWorkflowId && (
          <ScheduleModal
            isOpen={true}
            title={t("nav.scheduler")}
            subtitle={scheduledWf?.name}
            initialCron={getWorkflowSchedule(scheduledWf ?? { id: "", name: "" })?.cron || "0 9 * * *"}
            initialTz={getWorkflowSchedule(scheduledWf ?? { id: "", name: "" })?.tz ?? undefined}
            onSave={async (cron, tz) => {
            const wf = scheduledWf;
            try {
              await createScheduleMutation.mutateAsync({
                name: `${wf?.name || "workflow"} schedule`,
                cron,
                tz,
                workflow_id: scheduleWorkflowId,
                enabled: true,
              });
              addToast(t("scheduler.save_success", { defaultValue: "Schedule saved" }), "success");
              setScheduleWorkflowId(null);
            } catch (err) {
              addToast(
                err instanceof Error
                  ? err.message
                  : t("workflows.schedule_failed", { defaultValue: "Schedule creation failed" }),
                "error",
              );
            }
          }}
          onClose={() => setScheduleWorkflowId(null)}
        />
      )}
    </div>
  );
}
