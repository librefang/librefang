import { formatDate } from "../lib/datetime";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "@tanstack/react-router";
import {
  type DryRunResult,
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
  Calendar, FileText, Activity, Bot, ArrowRight, Loader2, Clock, ChevronRight,
  ChevronDown, FlaskConical, AlertCircle, CheckCircle2, SkipForward,
} from "lucide-react";
import {
  useWorkflows,
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

const categoryIconMap: Record<string, React.ComponentType<{ className?: string }>> = {
  creation: FileText, language: Bot, thinking: Activity, business: Calendar,
};

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
  const [expandedStepIdx, setExpandedStepIdx] = useState<number | null>(null);
  const [dryRunResult, setDryRunResult] = useState<DryRunResult | null>(null);
  const [manualSelection, setManualSelection] = useState(false);

  const workflowsQuery = useWorkflows();
  const runsQuery = useWorkflowRuns(selectedWorkflowId);
  const runDetailQuery = useWorkflowRunDetail(selectedRunId ?? "");
  const runMutation = useRunWorkflow();
  const dryRunMutation = useDryRunWorkflow();
  const deleteMutation = useDeleteWorkflow();
  const instantiateMutation = useInstantiateTemplate();
  const createScheduleMutation = useCreateSchedule();

  const workflows = useMemo(() =>
    [...(workflowsQuery.data ?? [])]
      .sort((a, b) => (b.created_at ?? "").localeCompare(a.created_at ?? ""))
      .filter(wf => !searchQuery || wf.name?.toLowerCase().includes(searchQuery.toLowerCase()) || wf.description?.toLowerCase().includes(searchQuery.toLowerCase())),
    [workflowsQuery.data, searchQuery]
  );

  useEffect(() => {
    if (!workflowsQuery.isSuccess) return;
    if (workflows.length === 0) {
      if (selectedWorkflowId) {
        setSelectedWorkflowId("");
        setSelectedRunId(null);
        setRunInput("");
        setDryRunResult(null);
      }
      return;
    }
    if (!selectedWorkflowId) {
      setSelectedWorkflowId(workflows[0]?.id ?? "");
      return;
    }
    if (!workflows.some((workflow) => workflow.id === selectedWorkflowId)) {
      setSelectedRunId(null);
      setRunInput("");
      setDryRunResult(null);
      setSelectedWorkflowId(workflows[0]?.id ?? "");
    }
  }, [workflows, selectedWorkflowId, manualSelection, workflowsQuery.isSuccess]);

  const handleRun = async () => {
    if (!selectedWorkflowId) return;
    setDryRunResult(null);
    dryRunMutation.reset();
    try {
      await runMutation.mutateAsync({ workflowId: selectedWorkflowId, input: runInput });
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
      const result = await dryRunMutation.mutateAsync({ workflowId: selectedWorkflowId, input: runInput });
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
      if (selectedWorkflowId === id) {
        setManualSelection(false);
      }
    } catch (err) {
      addToast(
        err instanceof Error ? err.message : t("workflows.delete_failed", { defaultValue: "Delete failed" }),
        "error",
      );
    }
  };

  const handleNewWorkflow = () => {
    sessionStorage.removeItem("canvasNodes");
    sessionStorage.removeItem("workflowTemplate");
    navigate({ to: "/canvas", search: { t: Date.now(), wf: undefined } });
  };
  useCreateShortcut(handleNewWorkflow);

  const handleUseTemplate = async (tmpl: WorkflowTemplate) => {
    const steps: any[] = (tmpl as any).steps ?? [];
    const nameToIdx = new Map(steps.map((s: any, i: number) => [s.name, i]));
    const nodes = steps.map((s: any, idx: number) => ({
      id: `node-${idx}`,
      type: "custom",
      position: { x: 50, y: idx * 160 },
      data: { label: s.name, prompt: s.prompt_template || "", nodeType: "agent" },
    }));
    const edges: any[] = [];
    steps.forEach((s: any, idx: number) => {
      (s.depends_on ?? []).forEach((dep: string) => {
        const src = nameToIdx.get(dep);
        if (src !== undefined) edges.push({ id: `e-${src}-${idx}`, source: `node-${src}`, target: `node-${idx}` });
      });
    });
    if (edges.length === 0 && nodes.length > 1) {
      nodes.slice(0, -1).forEach((_: any, i: number) =>
        edges.push({ id: `e-${i}`, source: `node-${i}`, target: `node-${i + 1}` })
      );
    }

    const hasRequiredParams = (tmpl.parameters ?? []).some(p => p.required);
    if (hasRequiredParams) {
      // Template has required params — open canvas pre-populated with nodes so
      // the user can see the workflow structure and fill in parameter values.
      sessionStorage.removeItem("canvasNodes");
      sessionStorage.setItem("workflowTemplate", JSON.stringify({
        nodes, edges, name: tmpl.name, description: tmpl.description ?? "",
      }));
      navigate({ to: "/canvas", search: { t: Date.now(), wf: undefined } });
      return;
    }
    try {
      const resp = await instantiateMutation.mutateAsync({ id: tmpl.id, params: {} });
      const workflowId = (resp as any).workflow_id || (resp as any).id;
      if (workflowId) {
        openWorkflow(workflowId);
      } else {
        // Instantiation succeeded but no ID returned — fall back to pre-populated canvas
        sessionStorage.removeItem("canvasNodes");
        sessionStorage.setItem("workflowTemplate", JSON.stringify({
          nodes, edges, name: tmpl.name, description: tmpl.description ?? "",
        }));
        navigate({ to: "/canvas", search: { t: Date.now(), wf: undefined } });
      }
    } catch {
      sessionStorage.removeItem("canvasNodes");
      sessionStorage.setItem("workflowTemplate", JSON.stringify({
        nodes, edges, name: tmpl.name, description: tmpl.description ?? "",
      }));
      navigate({ to: "/canvas", search: { t: Date.now(), wf: undefined } });
    }
  };

  const openWorkflow = (wfId: string) => {
    sessionStorage.removeItem("canvasNodes");
    sessionStorage.removeItem("workflowTemplate");
    navigate({ to: "/canvas", search: { t: undefined, wf: wfId } });
  };

  const templatesQuery = useWorkflowTemplates();
  const apiTemplates = templatesQuery.data ?? [];
  const lang = i18n.language?.split("-")[0] ?? "en";
  const tmplName = (tmpl: WorkflowTemplate) => tmpl.i18n?.[lang]?.name || tmpl.name;
  const tmplDesc = (tmpl: WorkflowTemplate) => tmpl.i18n?.[lang]?.description || tmpl.description;

  const hasWorkflows = workflows.length > 0;

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("workflows.automation_hub")}
        title={t("workflows.title")}
        subtitle={t("workflows.subtitle")}
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
      <div className="flex items-center gap-1 border-b border-border-subtle">
        <button
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
        apiTemplates.length > 0 ? (
          <div className="grid gap-3 sm:grid-cols-2 md:grid-cols-3 xl:grid-cols-4">
            {apiTemplates.map(tmpl => {
              const Icon = categoryIconMap[tmpl.category || ""] || Layers;
              const stepCount = tmpl.steps?.length ?? 0;
              return (
                <button key={tmpl.id} onClick={() => handleUseTemplate(tmpl)}
                  className="group text-left p-5 rounded-2xl border border-border-subtle bg-surface hover:border-brand/30 hover:shadow-lg hover:-translate-y-0.5 transition-all duration-300">
                  <div className="flex items-start gap-3">
                    <div className="w-10 h-10 rounded-xl bg-brand/10 flex items-center justify-center shrink-0 group-hover:bg-brand/20 transition-colors">
                      <Icon className="w-5 h-5 text-brand" />
                    </div>
                    <div className="min-w-0 flex-1">
                      <p className="text-sm font-bold truncate group-hover:text-brand transition-colors">{tmplName(tmpl)}</p>
                      <p className="text-[10px] text-text-dim mt-0.5 line-clamp-2">{tmplDesc(tmpl)}</p>
                      <div className="flex items-center gap-2 mt-2 text-[9px] font-semibold text-text-dim/50">
                        {stepCount > 0 && <span>{stepCount} {t("workflows.nodes_unit")}</span>}
                        {tmpl.tags && tmpl.tags.length > 0 && <span>{tmpl.tags[0]}</span>}
                        <ArrowRight className="w-3 h-3 text-brand/50 group-hover:translate-x-0.5 transition-transform" />
                      </div>
                    </div>
                  </div>
                </button>
              );
            })}
          </div>
        ) : (
          <div className="py-12 text-center text-text-dim text-sm">{t("common.no_data")}</div>
        )
      )}

      {/* Workflows Tab */}
      {activeTab === "workflows" && (
        <>
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
          <div className="space-y-2">
            <h2 className="text-xs font-bold uppercase tracking-widest text-text-dim/50 mb-1">
              {t("workflows.all_workflows")} ({workflows.length})
            </h2>
            {workflows.map(wf => (
              <div key={wf.id}
                onClick={() => {
                  setSelectedWorkflowId(wf.id);
                  setManualSelection(true);
                }}
                onDoubleClick={() => openWorkflow(wf.id)}
                className={`group flex items-center gap-4 p-4 rounded-2xl border cursor-pointer transition-colors ${
                  selectedWorkflowId === wf.id
                    ? "border-brand bg-brand/5 shadow-sm"
                    : "border-border-subtle bg-surface hover:border-brand/30 hover:shadow-sm"
                }`}>
                {/* Icon */}
                <div className={`w-10 h-10 rounded-xl flex items-center justify-center shrink-0 ${
                  selectedWorkflowId === wf.id ? "bg-brand text-white" : "bg-main text-brand"
                }`}>
                  <Layers className="w-5 h-5" />
                </div>
                {/* Info */}
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <h3 className="text-sm font-bold truncate">{wf.name}</h3>
                    <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-main text-text-dim font-semibold shrink-0">
                      {t("workflows.steps_count", { count: Array.isArray(wf.steps) ? wf.steps.length : (wf.steps || 0) })}
                    </span>
                  </div>
                  <p className="text-[10px] text-text-dim mt-0.5 truncate">{wf.description || t("common.no_data")}</p>
                  <div className="flex items-center gap-3 mt-1.5 text-[9px] text-text-dim/50">
                    <span className="flex items-center gap-1"><Clock className="w-3 h-3" />{formatDate(wf.created_at)}</span>
                    <span className="flex items-center gap-1"><Play className="w-3 h-3" />{(wf as any).run_count ?? 0} {t("workflows.runs_label", { defaultValue: "runs" })}</span>
                    {(wf as any).schedule && (
                      <span className={`flex items-center gap-1 px-1.5 py-0.5 rounded-full ${(wf as any).schedule.enabled ? "bg-success/10 text-success" : "bg-main text-text-dim"}`}>
                        <Calendar className="w-3 h-3" />
                        {(wf as any).schedule.cron}
                      </span>
                    )}
                  </div>
                </div>
                {/* Actions */}
                <div className="flex items-center gap-1 shrink-0" onClick={e => e.stopPropagation()}>
                  <button onClick={() => { setScheduleWorkflowId(wf.id); }}
                    className={`p-2 rounded-lg transition-colors ${(wf as any).schedule ? "text-success hover:text-success hover:bg-success/10" : "text-text-dim/40 hover:text-brand hover:bg-brand/10"}`}
                    title={t("nav.scheduler")}>
                    <Calendar className="w-3.5 h-3.5" />
                  </button>
                  <button onClick={() => openWorkflow(wf.id)}
                    className="p-2 rounded-lg text-text-dim/40 hover:text-brand hover:bg-brand/10 transition-colors"
                    title={t("canvas.ctx_edit")}>
                    <ChevronRight className="w-4 h-4" />
                  </button>
                  {confirmDeleteId === wf.id ? (
                    <div className="flex items-center gap-1">
                      <button onClick={() => handleDelete(wf.id)} className="px-2 py-1 rounded-lg bg-error text-white text-[10px] font-bold">{t("common.confirm")}</button>
                      <button onClick={() => setConfirmDeleteId(null)} className="px-2 py-1 rounded-lg bg-main text-text-dim text-[10px] font-bold">{t("common.cancel")}</button>
                    </div>
                  ) : (
                    <button onClick={() => handleDelete(wf.id)}
                      className="p-2 rounded-lg text-text-dim/30 hover:text-error hover:bg-error/10 transition-colors"
                      aria-label={t("common.delete")}>
                      <Trash2 className="w-3.5 h-3.5" />
                    </button>
                  )}
                </div>
              </div>
            ))}
          </div>

          {/* Right Panel: shown when a workflow is selected */}
          {selectedWorkflowId && (
            <div className="space-y-4">
              <Card padding="lg" className="sticky top-4 space-y-3">
                <h3 className="text-xs font-bold uppercase tracking-widest text-text-dim/50">{t("workflows.run_workflow")}</h3>
                <textarea value={runInput} onChange={e => setRunInput(e.target.value)}
                  placeholder={t("canvas.run_input_placeholder")} rows={4}
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
                      {dryRunResult.steps.map((step, i) => (
                        <div key={i} className="rounded-lg border border-border-subtle bg-main overflow-hidden">
                          <button
                            className="w-full flex items-center gap-2 px-3 py-2 text-left hover:bg-surface transition-colors"
                            onClick={() => setExpandedStepIdx(expandedStepIdx === i ? null : i)}>
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
                              <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-main border border-border-subtle text-text-dim/50 shrink-0">skip</span>
                            )}
                            <ChevronDown className={`w-3 h-3 text-text-dim/30 shrink-0 transition-transform ${expandedStepIdx === i ? "rotate-180" : ""}`} />
                          </button>
                          {expandedStepIdx === i && (
                            <div className="px-3 pb-3 space-y-1.5 border-t border-border-subtle">
                              {!step.agent_found && (
                                <p className="text-[10px] text-warning mt-2">Agent not found</p>
                              )}
                              {step.skip_reason && (
                                <p className="text-[10px] text-text-dim mt-2">{step.skip_reason}</p>
                              )}
                              <p className="text-[9px] font-bold text-text-dim/50 mt-2">Resolved prompt:</p>
                              <pre className="text-[10px] text-text whitespace-pre-wrap max-h-28 overflow-y-auto bg-surface rounded-lg p-2">
                                {step.resolved_prompt || "(empty)"}
                              </pre>
                            </div>
                          )}
                        </div>
                      ))}
                    </div>
                  </div>
                )}

                {/* Run Result */}
                {runMutation.data && (
                  <div className="p-3 rounded-xl bg-success/5 border border-success/20 space-y-2">
                    <p className="text-[10px] font-bold text-success">{t("canvas.run_result")}</p>
                    <pre className="text-xs text-text whitespace-pre-wrap max-h-32 overflow-y-auto">
                      {(runMutation.data as any).output || (runMutation.data as any).message || JSON.stringify(runMutation.data)}
                    </pre>
                    {/* Step-level I/O */}
                    {((runMutation.data as any).step_results as any[])?.length > 0 && (
                      <div className="space-y-1.5 border-t border-success/20 pt-2">
                        <p className="text-[9px] font-bold text-text-dim/50">Step details</p>
                        {((runMutation.data as any).step_results as any[]).map((s: any, i: number) => (
                          <div key={i} className="rounded-lg border border-border-subtle bg-main overflow-hidden">
                            <button
                              className="w-full flex items-center gap-2 px-3 py-2 text-left hover:bg-surface transition-colors"
                              onClick={() => setExpandedStepIdx(expandedStepIdx === i + 1000 ? null : i + 1000)}>
                              <CheckCircle2 className="w-3 h-3 text-success shrink-0" />
                              <span className="text-[10px] font-bold truncate flex-1">{s.step_name}</span>
                              <span className="text-[9px] text-text-dim/50 shrink-0">{s.duration_ms}ms</span>
                              <ChevronDown className={`w-3 h-3 text-text-dim/30 shrink-0 transition-transform ${expandedStepIdx === i + 1000 ? "rotate-180" : ""}`} />
                            </button>
                            {expandedStepIdx === i + 1000 && (
                              <div className="px-3 pb-3 space-y-2 border-t border-border-subtle">
                                <div>
                                  <p className="text-[9px] font-bold text-text-dim/50 mt-2">Prompt sent:</p>
                                  <pre className="text-[10px] text-text whitespace-pre-wrap max-h-24 overflow-y-auto bg-surface rounded-lg p-2 mt-1">
                                    {s.prompt || "(empty)"}
                                  </pre>
                                </div>
                                <div>
                                  <p className="text-[9px] font-bold text-text-dim/50">Output:</p>
                                  <pre className="text-[10px] text-text whitespace-pre-wrap max-h-24 overflow-y-auto bg-surface rounded-lg p-2 mt-1">
                                    {s.output || "(empty)"}
                                  </pre>
                                </div>
                                <p className="text-[9px] text-text-dim/40">
                                  {s.agent_name} · {s.input_tokens} in / {s.output_tokens} out tokens
                                </p>
                              </div>
                            )}
                          </div>
                        ))}
                      </div>
                    )}
                  </div>
                )}
                {runMutation.error && (
                  <div className="p-3 rounded-xl bg-error/5 border border-error/20">
                    <div className="flex items-center gap-1.5 mb-1">
                      <AlertCircle className="w-3.5 h-3.5 text-error shrink-0" />
                      <p className="text-[10px] font-bold text-error">Run failed</p>
                    </div>
                    <p className="text-xs text-error/80">
                      {(runMutation.error as any)?.message || String(runMutation.error)}
                    </p>
                  </div>
                )}
                {dryRunMutation.error && (
                  <div className="p-3 rounded-xl bg-error/5 border border-error/20">
                    <p className="text-xs text-error">
                      {(dryRunMutation.error as any)?.message || String(dryRunMutation.error)}
                    </p>
                  </div>
                )}
              </Card>

              {/* Run History */}
              {runsQuery.data && runsQuery.data.length > 0 && (
                <Card padding="lg" className="space-y-3">
                  <h3 className="text-xs font-bold uppercase tracking-widest text-text-dim/50">Run History</h3>
                  <div className="space-y-1.5">
                    {runsQuery.data.slice(0, 10).map((run) => {
                      const runId = (run as any).id as string | undefined;
                      const state = (run as any).state as string | undefined;
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
                              setExpandedStepIdx(null);
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
                              {runDetailQuery.data.step_results.map((step, si) => (
                                <div key={si} className="rounded-lg border border-border-subtle bg-main overflow-hidden">
                                  <button
                                    className="w-full flex items-center gap-2 px-3 py-2 text-left hover:bg-surface transition-colors"
                                    onClick={() => setExpandedStepIdx(expandedStepIdx === si + 2000 ? null : si + 2000)}>
                                    <CheckCircle2 className="w-3 h-3 text-success shrink-0" />
                                    <span className="text-[10px] font-bold truncate flex-1">{step.step_name}</span>
                                    <span className="text-[9px] text-text-dim/50 shrink-0">{step.duration_ms}ms</span>
                                    <ChevronDown className={`w-3 h-3 text-text-dim/30 shrink-0 transition-transform ${expandedStepIdx === si + 2000 ? "rotate-180" : ""}`} />
                                  </button>
                                  {expandedStepIdx === si + 2000 && (
                                    <div className="px-3 pb-3 space-y-2 border-t border-border-subtle">
                                      <div>
                                        <p className="text-[9px] font-bold text-text-dim/50 mt-2">Prompt sent:</p>
                                        <pre className="text-[10px] text-text whitespace-pre-wrap max-h-24 overflow-y-auto bg-surface rounded-lg p-2 mt-1">
                                          {step.prompt || "(empty)"}
                                        </pre>
                                      </div>
                                      <div>
                                        <p className="text-[9px] font-bold text-text-dim/50">Output:</p>
                                        <pre className="text-[10px] text-text whitespace-pre-wrap max-h-24 overflow-y-auto bg-surface rounded-lg p-2 mt-1">
                                          {step.output || "(empty)"}
                                        </pre>
                                      </div>
                                      <p className="text-[9px] text-text-dim/40">
                                        {step.agent_name} · {step.input_tokens} in / {step.output_tokens} out tokens
                                      </p>
                                    </div>
                                  )}
                                </div>
                              ))}
                            </div>
                          )}
                          {isSelected && runDetailQuery.isLoading && (
                            <div className="ml-5 mt-1 p-2 text-[10px] text-text-dim/50 flex items-center gap-1.5">
                              <Loader2 className="w-3 h-3 animate-spin" /> Loading details…
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
        </>
      )}
      {/* Schedule Modal */}
      {scheduleWorkflowId && (
        <ScheduleModal
          title={t("nav.scheduler")}
          subtitle={workflows.find(w => w.id === scheduleWorkflowId)?.name}
          initialCron={(workflows.find(w => w.id === scheduleWorkflowId) as any)?.schedule?.cron || "0 9 * * *"}
          initialTz={(workflows.find(w => w.id === scheduleWorkflowId) as any)?.schedule?.tz}
          onSave={async (cron, tz) => {
            const wf = workflows.find(w => w.id === scheduleWorkflowId);
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
