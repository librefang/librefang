import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "@tanstack/react-router";
import {
  deleteWorkflow,
  listWorkflowRuns,
  listWorkflows,
  runWorkflow,
  getWorkflow,
  listWorkflowTemplates,
  instantiateTemplate,
  type WorkflowTemplate,
} from "../api";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Input } from "../components/ui/Input";
import { PageHeader } from "../components/ui/PageHeader";
import { ListSkeleton } from "../components/ui/Skeleton";
import {
  Layers, Trash2, FilePlus, Play, Search,
  Calendar, FileText, Activity, Bot, ArrowRight, Loader2, Clock, ChevronRight
} from "lucide-react";

const iconMap: Record<string, React.ComponentType<{ className?: string }>> = { Calendar, FileText, Activity, Bot };
const categoryIconMap: Record<string, React.ComponentType<{ className?: string }>> = {
  creation: FileText, language: Bot, thinking: Activity, business: Calendar,
};
const REFRESH_MS = 30000;

export function WorkflowsPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [selectedWorkflowId, setSelectedWorkflowId] = useState<string>("");
  const [runInput, setRunInput] = useState("");
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState("");

  const workflowsQuery = useQuery({ queryKey: ["workflows", "list"], queryFn: listWorkflows, refetchInterval: REFRESH_MS });
  const runsQuery = useQuery({ queryKey: ["workflows", "runs", selectedWorkflowId], queryFn: () => listWorkflowRuns(selectedWorkflowId), enabled: Boolean(selectedWorkflowId) });
  const runMutation = useMutation({ mutationFn: ({ workflowId, input }: any) => runWorkflow(workflowId, input) });
  const deleteMutation = useMutation({ mutationFn: deleteWorkflow });

  const workflows = useMemo(() =>
    [...(workflowsQuery.data ?? [])]
      .sort((a, b) => (b.created_at ?? "").localeCompare(a.created_at ?? ""))
      .filter(wf => !searchQuery || wf.name?.toLowerCase().includes(searchQuery.toLowerCase()) || wf.description?.toLowerCase().includes(searchQuery.toLowerCase())),
    [workflowsQuery.data, searchQuery]
  );

  const handleRun = async () => {
    if (!selectedWorkflowId) return;
    try {
      await runMutation.mutateAsync({ workflowId: selectedWorkflowId, input: runInput });
      await runsQuery.refetch();
    } catch { /* ignore */ }
  };

  const handleDelete = async (id: string) => {
    if (confirmDeleteId !== id) { setConfirmDeleteId(id); return; }
    setConfirmDeleteId(null);
    try { await deleteMutation.mutateAsync(id); await queryClient.invalidateQueries({ queryKey: ["workflows"] }); } catch { /* ignore */ }
  };

  const handleNewWorkflow = () => {
    sessionStorage.removeItem("canvasNodes");
    sessionStorage.removeItem("workflowTemplate");
    navigate({ to: "/canvas", search: { t: Date.now() } });
  };

  const handleUseTemplate = async (tmpl: WorkflowTemplate) => {
    const hasRequiredParams = (tmpl.parameters ?? []).some(p => p.required);
    if (hasRequiredParams) {
      // Template needs params — open canvas with TemplateBrowser
      sessionStorage.removeItem("canvasNodes");
      sessionStorage.removeItem("workflowTemplate");
      navigate({ to: "/canvas", search: { t: Date.now(), openTemplates: "1" } });
      return;
    }
    try {
      const resp = await instantiateTemplate(tmpl.id, {});
      const workflowId = (resp as any).workflow_id || (resp as any).id;
      if (workflowId) {
        await queryClient.invalidateQueries({ queryKey: ["workflows"] });
        openWorkflow(workflowId);
      }
    } catch {
      sessionStorage.removeItem("canvasNodes");
      sessionStorage.removeItem("workflowTemplate");
      navigate({ to: "/canvas", search: { t: Date.now() } });
    }
  };

  const openWorkflow = async (wfId: string) => {
    try {
      const detail = await getWorkflow(wfId);
      let nodes, edges;
      if (detail.layout?.nodes) {
        nodes = detail.layout.nodes;
        edges = detail.layout.edges || [];
      } else {
        const steps = Array.isArray(detail.steps) ? detail.steps : [];
        nodes = steps.map((s: any, idx: number) => {
          const p = s.prompt_template || s.prompt || "";
          return { id: `node-${idx}`, type: "custom", position: { x: 50, y: idx * 80 },
            data: { label: s.name, description: p.length > 40 ? p.slice(0, 40) + "..." : p, prompt: p, nodeType: "agent", agentId: s.agent?.agent_id, agentName: s.agent?.name } };
        });
        edges = nodes.slice(0, -1).map((_: any, i: number) => ({ id: `e-${i}`, source: `node-${i}`, target: `node-${i + 1}` }));
      }
      sessionStorage.removeItem("canvasNodes");
      sessionStorage.setItem("workflowTemplate", JSON.stringify({ nodes, edges, name: detail.name, description: detail.description, workflowId: wfId }));
      navigate({ to: "/canvas", search: { t: Date.now() } });
    } catch (e) { console.error("Failed to load workflow:", e); }
  };

  const templatesQuery = useQuery({ queryKey: ["workflow-templates"], queryFn: () => listWorkflowTemplates() });
  const apiTemplates = templatesQuery.data ?? [];

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
        actions={
          <Button variant="primary" onClick={handleNewWorkflow}>
            <FilePlus className="h-4 w-4" />
            {t("workflows.create_blank")}
          </Button>
        }
      />

      {/* Template Recommendations — loaded from API */}
      {apiTemplates.length > 0 && (
      <div>
        <h2 className="text-xs font-bold uppercase tracking-widest text-text-dim/50 mb-3">{t("workflows.templates")}</h2>
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
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
                    <p className="text-sm font-bold truncate group-hover:text-brand transition-colors">{tmpl.name}</p>
                    <p className="text-[10px] text-text-dim mt-0.5 line-clamp-2">{tmpl.description}</p>
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
      </div>
      )}

      {/* Search Bar */}
      {hasWorkflows && (
        <Input value={searchQuery} onChange={e => setSearchQuery(e.target.value)}
          placeholder={t("workflows.search_placeholder")}
          leftIcon={<Search className="h-4 w-4" />} />
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
                onClick={() => setSelectedWorkflowId(wf.id)}
                onDoubleClick={() => openWorkflow(wf.id)}
                className={`group flex items-center gap-4 p-4 rounded-2xl border cursor-pointer transition-all ${
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
                  <div className="flex items-center gap-2 mt-1.5 text-[9px] text-text-dim/50">
                    <Clock className="w-3 h-3" />
                    <span>{new Date(wf.created_at || "").toLocaleDateString()}</span>
                  </div>
                </div>
                {/* Actions */}
                <div className="flex items-center gap-1 shrink-0" onClick={e => e.stopPropagation()}>
                  <button onClick={() => openWorkflow(wf.id)}
                    className="p-2 rounded-lg text-text-dim/40 hover:text-brand hover:bg-brand/10 transition-all"
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
                      className="p-2 rounded-lg text-text-dim/30 hover:text-error hover:bg-error/10 transition-all">
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
              <Card padding="lg" className="sticky top-4">
                <h3 className="text-xs font-bold uppercase tracking-widest text-text-dim/50 mb-4">{t("workflows.run_workflow")}</h3>
                <textarea value={runInput} onChange={e => setRunInput(e.target.value)}
                  placeholder={t("canvas.run_input_placeholder")} rows={4}
                  className="w-full rounded-xl border border-border-subtle bg-main px-4 py-2.5 text-sm mb-3 outline-none focus:border-brand resize-none" />
                <Button variant="primary" className="w-full" disabled={runMutation.isPending} onClick={handleRun}>
                  {runMutation.isPending ? <Loader2 className="w-4 h-4 animate-spin mr-2" /> : <Play className="w-4 h-4 mr-2" />}
                  {t("canvas.run_now")}
                </Button>

                {/* Run Result */}
                {runMutation.data && (
                  <div className="mt-4 p-3 rounded-xl bg-success/5 border border-success/20">
                    <p className="text-[10px] font-bold text-success mb-1">{t("canvas.run_result")}</p>
                    <pre className="text-xs text-text whitespace-pre-wrap max-h-40 overflow-y-auto">
                      {(runMutation.data as any).output || (runMutation.data as any).message || JSON.stringify(runMutation.data)}
                    </pre>
                  </div>
                )}
                {runMutation.error && (
                  <div className="mt-4 p-3 rounded-xl bg-error/5 border border-error/20">
                    <p className="text-xs text-error">{(runMutation.error as any)?.message || String(runMutation.error)}</p>
                  </div>
                )}
              </Card>
            </div>
          )}
        </div>
      ) : (
        /* Empty State */
        !workflowsQuery.isLoading && (
          <div className="text-center py-16">
            <div className="w-16 h-16 rounded-2xl bg-brand/10 flex items-center justify-center mx-auto mb-4">
              <Layers className="w-8 h-8 text-brand" />
            </div>
            <h3 className="text-lg font-bold">{t("workflows.empty_title")}</h3>
            <p className="text-sm text-text-dim mt-1 mb-6">{t("workflows.empty_desc")}</p>
            <Button variant="primary" onClick={() => handleNewWorkflow()}>
              <FilePlus className="w-4 h-4" />
              {t("workflows.create_blank")}
            </Button>
          </div>
        )
      )}
    </div>
  );
}
