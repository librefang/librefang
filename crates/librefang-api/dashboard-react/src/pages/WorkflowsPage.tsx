import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  createWorkflow,
  deleteWorkflow,
  listWorkflowRuns,
  listWorkflows,
  runWorkflow,
} from "../api";
import { WorkflowEditor } from "../components/WorkflowEditor";
import { workflowTemplates, type WorkflowTemplate } from "../data/workflowTemplates";
import { Layers, RefreshCw, Trash2, FilePlus, Sparkles } from "lucide-react";

const REFRESH_MS = 30000;

export function WorkflowsPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [isEditing, setIsEditing] = useState(false);
  const [showTemplates, setShowTemplates] = useState(false);
  const [selectedWorkflowId, setSelectedWorkflowId] = useState<string>("");
  const [runInput, setRunInput] = useState("");
  const [initialData, setInitialData] = useState<{ nodes: any[]; edges: any[] } | undefined>();

  const workflowsQuery = useQuery({ queryKey: ["workflows", "list"], queryFn: listWorkflows, refetchInterval: REFRESH_MS });
  const runsQuery = useQuery({ queryKey: ["workflows", "runs", selectedWorkflowId], queryFn: () => listWorkflowRuns(selectedWorkflowId), enabled: Boolean(selectedWorkflowId) });

  const runMutation = useMutation({ mutationFn: ({ workflowId, input }: any) => runWorkflow(workflowId, input) });
  const deleteMutation = useMutation({ mutationFn: deleteWorkflow });
  const createMutation = useMutation({ mutationFn: createWorkflow });

  const workflows = useMemo(() => [...(workflowsQuery.data ?? [])].sort((a, b) => (b.created_at ?? "").localeCompare(a.created_at ?? "")), [workflowsQuery.data]);

  const handleRun = async () => {
    if (!selectedWorkflowId) return;
    try {
      await runMutation.mutateAsync({ workflowId: selectedWorkflowId, input: runInput });
      await runsQuery.refetch();
    } catch { /* ignore */ }
  };

  const handleDelete = async (id: string) => {
    if (!window.confirm(t("common.confirm"))) return;
    try { await deleteMutation.mutateAsync(id); await queryClient.invalidateQueries({ queryKey: ["workflows"] }); } catch { /* ignore */ }
  };

  const handleUseTemplate = async (template: WorkflowTemplate) => {
    try {
      await createMutation.mutateAsync({
        name: t(template.name),
        description: t(template.description),
        steps: template.steps?.map((s: { name: string; prompt: string }) => ({ name: s.name, prompt: s.prompt })) ?? []
      });
      await queryClient.invalidateQueries({ queryKey: ["workflows"] });
      setShowTemplates(false);
    } catch { /* ignore */ }
  };

  const handleNewWorkflow = (template?: WorkflowTemplate) => {
    setInitialData(template ? { nodes: template.nodes, edges: template.edges } : undefined);
    setShowTemplates(false);
    setIsEditing(true);
  };

  if (isEditing) {
    return <WorkflowEditor initialNodes={initialData?.nodes} initialEdges={initialData?.edges} onClose={() => { setIsEditing(false); setInitialData(undefined); }} onSave={() => { setIsEditing(false); setInitialData(undefined); }} />;
  }
  const [runResult, setRunResult] = useState<string>("");

  const workflowsQuery = useQuery({ queryKey: ["workflows", "list"], queryFn: listWorkflows, refetchInterval: REFRESH_MS });
  const runsQuery = useQuery({ queryKey: ["workflows", "runs", selectedWorkflowId], queryFn: () => listWorkflowRuns(selectedWorkflowId), enabled: Boolean(selectedWorkflowId) });

  const runMutation = useMutation({ mutationFn: ({ workflowId, input }: any) => runWorkflow(workflowId, input) });
  const deleteMutation = useMutation({ mutationFn: deleteWorkflow });

  const workflows = useMemo(() => [...(workflowsQuery.data ?? [])].sort((a, b) => (b.created_at ?? "").localeCompare(a.created_at ?? "")), [workflowsQuery.data]);

  const handleRun = async () => {
    if (!selectedWorkflowId) return;
    try {
      const result = await runMutation.mutateAsync({ workflowId: selectedWorkflowId, input: runInput });
      setRunResult(typeof result.message === "string" ? result.message : JSON.stringify(result));
      await runsQuery.refetch();
    } catch (err: any) { setRunResult(`Error: ${err.message}`); }
  };

  const handleDelete = async (id: string) => {
    if (!window.confirm(t("common.confirm"))) return;
    try { await deleteMutation.mutateAsync(id); await queryClient.invalidateQueries({ queryKey: ["workflows"] }); } catch {}
  };

  if (isEditing) {
    return <WorkflowEditor onClose={() => setIsEditing(false)} onSave={() => setIsEditing(false)} />;
  }

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <header className="flex flex-col justify-between gap-4 sm:flex-row sm:items-end">
        <div>
          <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
            <Layers className="h-4 w-4" />
            {t("workflows.automation_hub")}
          </div>
          <h1 className="mt-2 text-3xl font-extrabold tracking-tight md:text-4xl">{t("workflows.title")}</h1>
          <p className="mt-1 text-text-dim font-medium">{t("workflows.subtitle")}</p>
        </div>
        <div className="flex gap-3">
          <button onClick={() => setIsEditing(true)} className="px-6 py-2 rounded-xl bg-brand text-white text-sm font-black shadow-lg shadow-brand/20 hover:opacity-90 transition-all">
            {t("common.symbols.expand")} {t("overview.new_workflow")}
          </button>
          <button className="flex h-9 items-center gap-2 rounded-xl border border-border-subtle bg-surface px-4 text-sm font-bold text-text-dim hover:text-brand transition-all shadow-sm" onClick={() => void workflowsQuery.refetch()}>
            <RefreshCw className={`h-3.5 w-3.5 ${workflowsQuery.isFetching ? "animate-spin" : ""}`} />
            {t("common.refresh")}
          </button>
        </div>
      </header>

      <div className="grid gap-6 lg:grid-cols-[1fr_320px]">
        <div className="rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm">
          <h2 className="text-lg font-black tracking-tight mb-6">{t("workflows.all_workflows")}</h2>
          <div className="grid gap-3 sm:grid-cols-2">
            {workflows.map(wf => (
              <article key={wf.id} onClick={() => setSelectedWorkflowId(wf.id)} className={`group cursor-pointer rounded-xl border p-4 transition-all ${selectedWorkflowId === wf.id ? 'border-brand bg-brand/5 shadow-sm' : 'border-border-subtle bg-main/40 hover:border-brand/30'}`}>
                <div className="flex justify-between items-start">
                  <div className="min-w-0 flex-1">
                    <h3 className="text-sm font-black truncate group-hover:text-brand transition-colors">{wf.name}</h3>
                    <p className="text-[10px] text-text-dim mt-1 line-clamp-1 italic">{wf.description || t("common.no_data")}</p>
                    <div className="mt-3 flex items-center gap-3 text-[9px] font-bold text-text-dim/60 uppercase">
                      <span>{t("workflows.steps_count", { count: wf.steps || 0 })}</span>
                      <div className="h-1 w-1 rounded-full bg-border-subtle" />
                      <span>{t("common.created")}: {new Date(wf.created_at || "").toLocaleDateString()}</span>
                    </div>
                  </div>
                  <button onClick={(e) => { e.stopPropagation(); handleDelete(wf.id); }} className="opacity-0 group-hover:opacity-100 p-1.5 rounded-lg text-text-dim hover:text-error transition-all">
                    <Trash2 className="h-4 w-4" />
                  </button>
                </div>
              </article>
            ))}
          </div>
        </div>

        <aside className="flex flex-col gap-6">
          <div className="rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm">
            <h3 className="text-xs font-black uppercase tracking-widest text-text-dim mb-4">{t("workflows.run_workflow")}</h3>
            <select value={selectedWorkflowId} onChange={(e) => setSelectedWorkflowId(e.target.value)} className="w-full rounded-xl border border-border-subtle bg-main px-4 py-2 text-sm mb-3 outline-none focus:border-brand">
              <option value="">{t("workflows.select_workflow")}</option>
              {workflows.map(wf => <option key={wf.id} value={wf.id}>{wf.name}</option>)}
            </select>
            <textarea value={runInput} onChange={e => setRunInput(e.target.value)} placeholder={t("chat.transmit_command")} rows={3} className="w-full rounded-xl border border-border-subtle bg-main px-4 py-2 text-sm mb-4 outline-none focus:border-brand resize-none" />
            <button disabled={!selectedWorkflowId || runMutation.isPending} onClick={handleRun} className="w-full py-2.5 rounded-xl bg-success text-white text-xs font-black shadow-lg shadow-success/20 hover:opacity-90 transition-all disabled:opacity-30">{t("scheduler.run_now")}</button>
          </div>

          <div className="rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm">
            <h3 className="text-xs font-black uppercase tracking-widest text-text-dim mb-4">{t("workflows.recent_runs")}</h3>
            <p className="text-[10px] text-text-dim italic text-center py-8">{selectedWorkflowId ? t("workflows.no_runs") : t("workflows.select_workflow_hint")}</p>
          </div>
        </aside>
      </div>
    </div>
  );
}
