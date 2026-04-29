import { useCallback, useState, useEffect, useMemo, useRef } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate, useSearch } from "@tanstack/react-router";
import { AnimatePresence, motion } from "motion/react";
import { fadeInScale, APPLE_EASE } from "../lib/motion";
import {
  ReactFlow,
  Background,
  BackgroundVariant,
  Controls,
  MiniMap,
  addEdge,
  useNodesState,
  useEdgesState,
  type Node,
  type Edge,
  type Connection,
  MarkerType,
  Handle,
  Position,
  type OnSelectionChangeParams,
  useReactFlow,
  ReactFlowProvider,
  SelectionMode,
  ConnectionLineType,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { type AgentItem, type WorkflowItem, type WorkflowTemplate as ApiWorkflowTemplate, type DryRunResult, type WorkflowStepResult } from "../api";
import { Card } from "../components/ui/Card";
import { ScheduleModal } from "../components/ui/ScheduleModal";
import { DrawerPanel } from "../components/ui/DrawerPanel";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { InlineEmpty } from "../components/ui/InlineEmpty";
import { useUIStore } from "../lib/store";
import {
  Play, Save, Trash2, Plus, FolderOpen, Loader2,
  Maximize2, Minimize2, ArrowLeft, X, Group, ChevronDown, ChevronRight,
  Copy, ClipboardPaste, LayoutGrid,
  Download, Upload, HelpCircle, Scan, Check, LayoutTemplate, Search, Tag, BookCopy, Calendar,
  FlaskConical, AlertCircle, CheckCircle2, SkipForward, ChevronUp,
} from "lucide-react";
import { truncateId } from "../lib/string";
import {
  useCreateWorkflow,
  useDeleteWorkflow,
  useDryRunWorkflow,
  useInstantiateTemplate,
  useRunWorkflow,
  useSaveWorkflowAsTemplate,
  useUpdateWorkflow as useUpdateWorkflowMutation,
} from "../lib/mutations/workflows";
import { useCreateSchedule } from "../lib/mutations/schedules";
import { useWorkflows, useWorkflowTemplates, workflowQueries } from "../lib/queries/workflows";
import { useAgents } from "../lib/queries/agents";
import { useQueryClient } from "@tanstack/react-query";

type CanvasDraft = {
  nodes: Node[];
  edges: Edge[];
  workflowName: string;
  workflowDescription: string;
};

const CANVAS_DRAFT_KEY = "canvasDraft";

function readCanvasDraft(): CanvasDraft | null {
  if (typeof window === "undefined") return null;
  const rawDraft = sessionStorage.getItem(CANVAS_DRAFT_KEY);
  if (!rawDraft) return null;
  try {
    const parsed = JSON.parse(rawDraft) as Partial<CanvasDraft>;
    return {
      nodes: Array.isArray(parsed.nodes) ? parsed.nodes : [],
      edges: Array.isArray(parsed.edges) ? parsed.edges : [],
      workflowName: typeof parsed.workflowName === "string" ? parsed.workflowName : "",
      workflowDescription: typeof parsed.workflowDescription === "string" ? parsed.workflowDescription : "",
    };
  } catch {
    sessionStorage.removeItem(CANVAS_DRAFT_KEY);
    return null;
  }
}

function clearCanvasDraft() {
  if (typeof window === "undefined") return;
  sessionStorage.removeItem(CANVAS_DRAFT_KEY);
}

// Node type configuration — n8n-style color scheme
const NODE_TYPES = [
  // Triggers (visual markers)
  { type: "start", labelKey: "canvas.node_types.start", color: "#10b981", bg: "#ecfdf5", icon: "S", descKey: "canvas.node_types.start_desc" },
  { type: "end", labelKey: "canvas.node_types.end", color: "#ef4444", bg: "#fef2f2", icon: "E", descKey: "canvas.node_types.end_desc" },
  { type: "schedule", labelKey: "canvas.node_types.schedule", color: "#f59e0b", bg: "#fffbeb", icon: "C", descKey: "canvas.node_types.schedule_desc" },
  { type: "webhook", labelKey: "canvas.node_types.webhook", color: "#6366f1", bg: "#eef2ff", icon: "W", descKey: "canvas.node_types.webhook_desc" },
  { type: "channel", labelKey: "canvas.node_types.channel", color: "#8b5cf6", bg: "#f5f3ff", icon: "M", descKey: "canvas.node_types.channel_desc" },
  // Logic control
  { type: "condition", labelKey: "canvas.node_types.condition", color: "#f59e0b", bg: "#fffbeb", icon: "?", descKey: "canvas.node_types.condition_desc" },
  { type: "loop", labelKey: "canvas.node_types.loop", color: "#8b5cf6", bg: "#f5f3ff", icon: "L", descKey: "canvas.node_types.loop_desc" },
  { type: "parallel", labelKey: "canvas.node_types.parallel", color: "#f59e0b", bg: "#fffbeb", icon: "P", descKey: "canvas.node_types.parallel_desc" },
  { type: "collect", labelKey: "canvas.node_types.collect", color: "#10b981", bg: "#ecfdf5", icon: "C", descKey: "canvas.node_types.collect_desc" },
  { type: "wait", labelKey: "canvas.node_types.wait", color: "#6b7280", bg: "#f9fafb", icon: "T", descKey: "canvas.node_types.wait_desc" },
  // Actions
  { type: "respond", labelKey: "canvas.node_types.respond", color: "#10b981", bg: "#ecfdf5", icon: "R", descKey: "canvas.node_types.respond_desc" },
  { type: "agent", labelKey: "canvas.node_types.agent", color: "#3b82f6", bg: "#eff6ff", icon: "A", descKey: "canvas.node_types.agent_desc" },
];

// Node types that require an agent binding
const AGENT_NODE_TYPES_SET = new Set(["agent", "channel", "respond", "condition", "loop", "parallel", "collect"]);

// Custom node component — n8n style
function CustomNode({ data, type: nodeTypeKey, t }: { data: any; type: string; t: (key: string) => string }) {
  const config = NODE_TYPES.find(n => n.type === (data.nodeType || nodeTypeKey)) || NODE_TYPES[11];
  const isStart = data.nodeType === "start";
  const isEnd = data.nodeType === "end";
  const runState = data._runState as string | undefined;
  const needsAgent = AGENT_NODE_TYPES_SET.has(data.nodeType);
  const missingAgent = needsAgent && !data.agentId;

  const borderColor = runState === "done" ? "#10b981"
    : runState === "running" ? config.color
      : missingAgent ? "#f59e0b"
        : "transparent";
  const ringStyle = runState === "running"
    ? { boxShadow: `0 0 0 3px ${config.color}40, 0 8px 24px ${config.color}30` }
    : runState === "done"
      ? { boxShadow: `0 0 0 3px #10b98140, 0 4px 12px #10b98120` }
      : missingAgent
        ? { boxShadow: "0 0 0 2px #f59e0b30" }
        : { boxShadow: "0 2px 8px rgba(0,0,0,0.08), 0 1px 2px rgba(0,0,0,0.06)" };

  return (
    <div
      className={`rounded-2xl bg-surface min-w-[140px] max-w-[200px] overflow-hidden relative transition-all duration-200 border border-border-subtle hover:scale-[1.02] hover:shadow-lg ${runState === "running" ? "animate-pulse" : ""
        }`}
      style={{ border: `2px ${missingAgent ? "dashed" : "solid"} ${borderColor}`, ...ringStyle }}
    >
      {/* Target Handle */}
      {!isStart && (
        <Handle type="target" position={Position.Top}
          className="w-3! h-3! rounded-full! border-2! border-surface!"
          style={{ backgroundColor: config.color }} />
      )}

      {/* Header: icon circle + label */}
      <div className="flex items-center gap-2.5 px-3 py-2.5" style={{ backgroundColor: `${config.color}15` }}>
        <div
          className="w-8 h-8 rounded-xl flex items-center justify-center text-white text-sm font-bold shrink-0 transition-colors"
          style={{ backgroundColor: config.color }}
        >
          {runState === "running" ? <Loader2 className="w-4 h-4 animate-spin" /> :
            runState === "done" ? "✓" : config.icon}
        </div>
        <div className="min-w-0 flex-1">
          <p className="text-xs font-bold text-text truncate leading-tight">{data.label || t(config.labelKey)}</p>
          <p className="text-[9px] text-text-dim truncate leading-tight mt-0.5">{data.description || t(config.descKey)}</p>
        </div>
      </div>

      {/* Agent badge / missing warning */}
      {data.agentName ? (
        <div className="px-3 py-1.5 border-t border-border-subtle/50 flex items-center gap-1.5">
          <div className="w-1.5 h-1.5 rounded-full bg-success shrink-0" />
          <span className="text-[9px] font-semibold text-text-dim truncate">{data.agentName}</span>
        </div>
      ) : missingAgent ? (
        <div className="px-3 py-1 border-t border-warning/30 flex items-center gap-1.5">
          <span className="text-[9px] font-semibold text-warning">{t("canvas.click_to_assign")}</span>
        </div>
      ) : null}

      {/* Depends-on badge */}
      {data.dependsOn && data.dependsOn.length > 0 && (
        <div className="px-3 py-1 border-t border-border-subtle/50 flex items-center gap-1.5">
          <span className="text-[9px] text-text-dim/60">⬆ {data.dependsOn.length} dep{data.dependsOn.length > 1 ? "s" : ""}</span>
        </div>
      )}

      {/* Source Handle */}
      {!isEnd && (
        <Handle type="source" position={Position.Bottom}
          className="w-3! h-3! rounded-full! border-2! border-surface!"
          style={{ backgroundColor: config.color }} />
      )}
    </div>
  );
}

// Group node component
function GroupNodeComponent({ data, id }: { data: any; id: string }) {
  const { t } = useTranslation();
  const expanded = data._expanded !== false;
  return (
    <div
      className={`rounded-2xl border-2 border-dashed transition-colors ${expanded ? "border-primary/30 bg-primary/5" : "border-primary bg-surface shadow-lg"
        }`}
      style={expanded
        ? { width: "100%", height: "100%", pointerEvents: "none" }
        : { width: 180 }}
    >
      <Handle type="target" position={Position.Top} className="w-3! h-3! rounded-full! bg-primary! border-2! border-surface!" />
      <div
        className="flex items-center gap-2 px-3 py-2 bg-primary/10 rounded-t-xl cursor-pointer relative z-10"
        style={{ pointerEvents: "auto" }}
      >
        <div className="flex items-center gap-2 flex-1 min-w-0" onClick={(e) => { e.stopPropagation(); data._onToggle?.(id); }}>
          {expanded
            ? <ChevronDown className="w-3.5 h-3.5 text-primary shrink-0" />
            : <ChevronRight className="w-3.5 h-3.5 text-primary shrink-0" />}
          <Group className="w-3.5 h-3.5 text-primary shrink-0" />
          <span className="text-xs font-bold text-primary truncate">{data.label || t("canvas.group")}</span>
          {!expanded && data._childCount > 0 && (
            <span className="text-[9px] text-primary/50">{data._childCount}</span>
          )}
        </div>
        {/* Ungroup (keep child nodes) */}
        <button onClick={(e) => { e.stopPropagation(); data._onUngroup?.(id); }}
          title={t("canvas.ungroup")}
          className="p-0.5 rounded hover:bg-primary/20 text-primary/50 hover:text-primary shrink-0">
          <X className="w-3 h-3" />
        </button>
        {/* Delete group + child nodes */}
        <button onClick={(e) => { e.stopPropagation(); data._onDeleteGroup?.(id); }}
          title={t("canvas.delete_group")}
          className="p-0.5 rounded hover:bg-error/20 text-text-dim/30 hover:text-error shrink-0">
          <Trash2 className="w-3 h-3" />
        </button>
      </div>
      {!expanded && (
        <div className="px-3 py-2">
          <p className="text-[9px] text-text-dim">Click to expand</p>
        </div>
      )}
      <Handle type="source" position={Position.Bottom} className="w-3! h-3! rounded-full! bg-primary! border-2! border-surface!" />
    </div>
  );
}

// Workflow list sidebar
function WorkflowList({
  workflows, selectedId, onSelect, onDelete, onRun, isRunning, t
}: {
  workflows: WorkflowItem[]; selectedId: string | null;
  onSelect: (w: WorkflowItem) => void; onDelete: (id: string) => void;
  onRun: (id: string) => void; isRunning: string | null; t: (key: string) => string;
}) {
  const [confirmId, setConfirmId] = useState<string | null>(null);
  return (
    <Card padding="md" className="w-72 border-r border-border-subtle bg-main/30 overflow-y-auto rounded-none">
      <div className="flex items-center justify-between mb-4">
        <h3 className="text-[10px] font-black uppercase text-text-dim/60">{t("workflows.all_workflows")}</h3>
        <Badge variant="brand">{workflows.length}</Badge>
      </div>
      <div className="space-y-2">
        {workflows.length === 0 ? (
          <p className="text-xs text-text-dim italic text-center py-4">{t("common.no_data")}</p>
        ) : (
          workflows.map(w => (
            <div key={w.id} onClick={() => onSelect(w)}
              className={`p-3 rounded-xl border cursor-pointer transition-colors ${selectedId === w.id ? "border-primary bg-primary/5" : "border-border-subtle hover:border-primary/50 bg-surface"
                }`}>
              {confirmId === w.id ? (
                <div className="flex items-center justify-between gap-2">
                  <span className="text-xs text-text-dim truncate">{t("workflows.delete_confirm")}</span>
                  <div className="flex gap-1 shrink-0">
                    <button onClick={(e) => { e.stopPropagation(); onDelete(w.id); setConfirmId(null); }}
                      className="px-2 py-1 rounded-lg bg-error text-white text-[10px] font-bold">{t("common.confirm")}</button>
                    <button onClick={(e) => { e.stopPropagation(); setConfirmId(null); }}
                      className="px-2 py-1 rounded-lg bg-surface text-text-dim text-[10px] font-bold">{t("common.cancel")}</button>
                  </div>
                </div>
              ) : (
                <>
                  <div className="flex items-center justify-between">
                    <span className="text-sm font-bold truncate">{w.name}</span>
                    <div className="flex gap-1">
                      <button onClick={(e) => { e.stopPropagation(); onRun(w.id); }} disabled={isRunning === w.id}
                        className="p-1.5 rounded-lg hover:bg-success/10 text-success disabled:opacity-50">
                        {isRunning === w.id ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Play className="w-3.5 h-3.5" />}
                      </button>
                      <button onClick={(e) => { e.stopPropagation(); setConfirmId(w.id); }}
                        className="p-1.5 rounded-lg hover:bg-error/10 text-error">
                        <Trash2 className="w-3.5 h-3.5" />
                      </button>
                    </div>
                  </div>
                  <p className="text-[10px] text-text-dim mt-1 truncate">{w.description || "-"}</p>
                </>
              )}
            </div>
          ))
        )}
      </div>
    </Card>
  );
}

// Template browser
function TemplateBrowser({
  onInstantiate, onClose, t
}: {
  onInstantiate: (workflowId: string) => void;
  onClose: () => void;
  t: (key: string) => string;
}) {
  const [searchQuery, setSearchQuery] = useState("");
  const [selectedTemplate, setSelectedTemplate] = useState<ApiWorkflowTemplate | null>(null);
  const [paramValues, setParamValues] = useState<Record<string, unknown>>({});
  const [error, setError] = useState<string | null>(null);
  const instantiateTemplateMutation = useInstantiateTemplate();
  const templatesQuery = useWorkflowTemplates(searchQuery || undefined);
  const templates = templatesQuery.data ?? [];
  const loading = templatesQuery.isLoading || templatesQuery.isFetching;

  const handleSelect = (tmpl: ApiWorkflowTemplate) => {
    setSelectedTemplate(tmpl);
    setError(null);
    // Pre-fill defaults
    const defaults: Record<string, unknown> = {};
    for (const p of tmpl.parameters ?? []) {
      if (p.default !== undefined) defaults[p.name] = p.default;
    }
    setParamValues(defaults);
  };

  const handleInstantiate = async () => {
    if (!selectedTemplate) return;
    setError(null);
    try {
      const resp = await instantiateTemplateMutation.mutateAsync({ id: selectedTemplate.id, params: paramValues });
      const workflowId = (resp as any).workflow_id || (resp as any).id || "";
      onInstantiate(workflowId);
    } catch (e: any) {
      setError(e?.message || t("canvas.template_instantiate_error"));
    }
  };

  return (
    <DrawerPanel isOpen onClose={onClose} size="2xl" hideCloseButton>
        {/* Header — matches the existing inline icon + custom X. */}
        <div className="flex items-center justify-between px-5 py-3 border-b border-border-subtle sticky top-0 bg-surface z-10">
          <div className="flex items-center gap-2">
            <LayoutTemplate className="w-4 h-4 text-primary" />
            <h3 className="text-sm font-bold">{t("canvas.browse_templates")}</h3>
          </div>
          <button onClick={onClose} className="p-1 rounded hover:bg-main"><X className="w-4 h-4" /></button>
        </div>

        {selectedTemplate ? (
          /* Template detail + params form */
          <div className="flex-1 overflow-y-auto p-5 space-y-4">
            <button onClick={() => setSelectedTemplate(null)} className="text-xs text-primary hover:underline flex items-center gap-1">
              <ArrowLeft className="w-3 h-3" /> {t("common.back")}
            </button>
            <div>
              <h4 className="text-base font-bold">{selectedTemplate.name}</h4>
              {selectedTemplate.description && <p className="text-xs text-text-dim mt-1">{selectedTemplate.description}</p>}
              <div className="flex gap-1.5 mt-2">
                {selectedTemplate.category && <Badge variant="brand">{selectedTemplate.category}</Badge>}
                {selectedTemplate.tags?.map(tag => (
                  <Badge key={tag} variant="default">{tag}</Badge>
                ))}
              </div>
            </div>

            {(selectedTemplate.parameters ?? []).length > 0 && (
              <div className="space-y-3">
                <h5 className="text-[10px] font-black uppercase tracking-wider text-text-dim/50">{t("canvas.template_params")}</h5>
                {selectedTemplate.parameters!.map(p => (
                  <div key={p.name}>
                    <label className="text-[10px] font-bold text-text-dim uppercase">
                      {p.name}
                      {p.required && <span className="text-error ml-0.5">*</span>}
                    </label>
                    {p.description && <p className="text-[9px] text-text-dim/60 mt-0.5">{p.description}</p>}
                    <input
                      type={p.param_type === "number" ? "number" : "text"}
                      value={String(paramValues[p.name] ?? "")}
                      onChange={e => setParamValues(prev => ({ ...prev, [p.name]: p.param_type === "number" ? Number(e.target.value) : e.target.value }))}
                      className="mt-1 w-full rounded-lg border border-border-subtle bg-main px-2 py-1.5 text-xs outline-none focus:border-primary"
                      placeholder={p.description || p.name}
                    />
                  </div>
                ))}
              </div>
            )}

            {error && (
              <div className="px-3 py-2 rounded-lg bg-error/10 border border-error/30 text-error text-xs">{error}</div>
            )}

            <Button variant="primary" className="w-full" onClick={handleInstantiate} disabled={instantiateTemplateMutation.isPending}>
              {instantiateTemplateMutation.isPending ? <Loader2 className="w-4 h-4 mr-1 animate-spin" /> : <Play className="w-4 h-4 mr-1" />}
              {t("canvas.use_template")}
            </Button>
          </div>
        ) : (
          /* Template list — Modal handles outer scroll */
          <div>
            {/* Search */}
            <div className="px-5 pt-4 pb-2">
              <div className="relative">
                <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-text-dim/40" />
                <input
                  type="text" value={searchQuery}
                  onChange={e => setSearchQuery(e.target.value)}
                  placeholder={t("canvas.template_search")}
                  className="w-full rounded-xl border border-border-subtle bg-main pl-8 pr-3 py-2 text-xs outline-none focus:border-primary"
                />
              </div>
            </div>

            {loading ? (
              <div className="flex items-center justify-center py-12">
                <Loader2 className="w-5 h-5 animate-spin text-primary" />
              </div>
            ) : templates.length === 0 ? (
              <InlineEmpty
                icon={<LayoutTemplate className="w-5 h-5" />}
                message={t("canvas.no_templates")}
              />
            ) : (
              <div className="px-5 pb-4 grid gap-2">
                {templates.map(tmpl => (
                  <button
                    key={tmpl.id}
                    onClick={() => handleSelect(tmpl)}
                    className="p-3 rounded-xl border border-border-subtle bg-surface hover:border-primary/50 hover:shadow-sm transition-colors text-left"
                  >
                    <div className="flex items-center justify-between">
                      <span className="text-sm font-bold truncate">{tmpl.name}</span>
                      <div className="flex gap-1 shrink-0">
                        {tmpl.category && (
                          <span className="text-[9px] font-bold px-1.5 py-0.5 rounded-full bg-primary/10 text-primary">{tmpl.category}</span>
                        )}
                      </div>
                    </div>
                    {tmpl.description && <p className="text-[10px] text-text-dim mt-1 line-clamp-2">{tmpl.description}</p>}
                    {tmpl.tags && tmpl.tags.length > 0 && (
                      <div className="flex gap-1 mt-2 flex-wrap">
                        {tmpl.tags.map(tag => (
                          <span key={tag} className="text-[9px] px-1.5 py-0.5 rounded-full bg-main text-text-dim flex items-center gap-0.5">
                            <Tag className="w-2.5 h-2.5" /> {tag}
                          </span>
                        ))}
                      </div>
                    )}
                  </button>
                ))}
              </div>
            )}
          </div>
        )}
    </DrawerPanel>
  );
}

// Node configuration panel
const inputClass = "mt-1 w-full rounded-lg border border-border-subtle bg-main px-2 py-1.5 text-xs outline-none focus:border-primary";
const labelClass = "text-[10px] font-bold text-text-dim uppercase";

function NodeConfigPanel({
  node, agents, onUpdate, onClose, onDelete, t
}: {
  node: Node; agents: AgentItem[]; onUpdate: (id: string, data: any) => void;
  onClose: () => void; onDelete: (id: string) => void; t: (key: string) => string;
}) {
  const d = node.data as any;
  const [label, setLabel] = useState(d.label || "");
  const [description, setDescription] = useState(d.description || "");
  const [agentId, setAgentId] = useState(d.agentId || "");
  const [prompt, setPrompt] = useState(d.prompt || d.description || "");
  const [mode, setMode] = useState<string>(d.stepMode || "sequential");
  const [errorMode, setErrorMode] = useState<string>(d.errorMode || "fail");
  const [timeoutSecs, setTimeoutSecs] = useState<number>(d.timeoutSecs || 120);
  const [outputVar, setOutputVar] = useState(d.outputVar || "");
  // Conditional fields
  const [condition, setCondition] = useState(d.condition || "");
  // Loop fields
  const [maxIterations, setMaxIterations] = useState<number>(d.maxIterations || 5);
  const [until, setUntil] = useState(d.until || "");
  // Retry fields
  const [maxRetries, setMaxRetries] = useState<number>(d.maxRetries || 3);
  const [dependsOn, setDependsOn] = useState<string[]>(d.dependsOn || []);

  const handleSave = () => {
    const agent = agents.find(a => a.id === agentId);
    onUpdate(node.id, {
      ...d,
      label, description,
      agentId: agentId || undefined,
      agentName: agent?.name || undefined,
      prompt,
      stepMode: mode,
      errorMode,
      timeoutSecs,
      outputVar: outputVar || undefined,
      condition: mode === "conditional" ? condition : undefined,
      maxIterations: mode === "loop" ? maxIterations : undefined,
      until: mode === "loop" ? until : undefined,
      maxRetries: errorMode === "retry" ? maxRetries : undefined,
      dependsOn: dependsOn.length > 0 ? dependsOn : undefined,
    });
    onClose();
  };

  const hasAgent = !!agentId;

  return (
    <div className="absolute top-3 right-3 z-20 w-[calc(100%-24px)] sm:w-80 max-h-[calc(100%-24px)] rounded-xl border border-border-subtle bg-surface shadow-2xl overflow-hidden flex flex-col">
      <div className="flex items-center justify-between px-3 py-2 bg-main/50 border-b border-border-subtle shrink-0">
        <span className="text-xs font-bold">{t("canvas.node_config")}</span>
        <div className="flex items-center gap-1">
          <button onClick={() => { onDelete(node.id); onClose(); }}
            className="p-1 rounded hover:bg-error/10 text-text-dim/40 hover:text-error"><Trash2 className="w-3.5 h-3.5" /></button>
          <button onClick={onClose} className="p-1 rounded hover:bg-main"><X className="w-3.5 h-3.5" /></button>
        </div>
      </div>
      <div className="p-3 space-y-2.5 overflow-y-auto flex-1">
        {/* Basic info */}
        <div>
          <label className={labelClass}>{t("canvas.node_label")}</label>
          <input type="text" value={label} onChange={e => setLabel(e.target.value)} className={inputClass} />
        </div>
        <div>
          <label className={labelClass}>{t("canvas.node_desc")}</label>
          <input type="text" value={description} onChange={e => setDescription(e.target.value)} className={inputClass} />
        </div>

        {/* Agent binding */}
        <div>
          <label className={labelClass}>{t("canvas.assign_agent")}</label>
          <select value={agentId} onChange={e => setAgentId(e.target.value)} className={inputClass}>
            <option value="">{t("canvas.no_agent")}</option>
            {agents.map(a => (
              <option key={a.id} value={a.id}>{a.name}{a.state === "Running" ? "" : ` (${a.state})`}</option>
            ))}
          </select>
        </div>

        {/* Prompt */}
        {hasAgent && (
          <div>
            <label className={labelClass}>
              Prompt <span className="text-text-dim/50 normal-case font-normal">{"({{input}} = prev output)"}</span>
            </label>
            <textarea value={prompt} onChange={e => setPrompt(e.target.value)} rows={3}
              className={`${inputClass} resize-none`} />
          </div>
        )}

        {/* Execution mode */}
        {hasAgent && (
          <div>
            <label className={labelClass}>{t("canvas.step_mode")}</label>
            <select value={mode} onChange={e => setMode(e.target.value)} className={inputClass}>
              <option value="sequential">{t("canvas.mode_sequential")}</option>
              <option value="fan_out">{t("canvas.mode_fan_out")}</option>
              <option value="collect">{t("canvas.mode_collect")}</option>
              <option value="conditional">{t("canvas.mode_conditional")}</option>
              <option value="loop">{t("canvas.mode_loop")}</option>
            </select>
          </div>
        )}

        {/* Conditional-specific fields */}
        {hasAgent && mode === "conditional" && (
          <div>
            <label className={labelClass}>{t("canvas.condition_text")}</label>
            <input type="text" value={condition} onChange={e => setCondition(e.target.value)}
              placeholder={t("canvas.condition_placeholder")} className={inputClass} />
          </div>
        )}

        {/* Loop-specific fields */}
        {hasAgent && mode === "loop" && (
          <>
            <div>
              <label className={labelClass}>{t("canvas.loop_until")}</label>
              <input type="text" value={until} onChange={e => setUntil(e.target.value)}
                placeholder={t("canvas.loop_until_placeholder")} className={inputClass} />
            </div>
            <div>
              <label className={labelClass}>{t("canvas.loop_max")}</label>
              <input type="number" value={maxIterations} onChange={e => setMaxIterations(Number(e.target.value))}
                min={1} max={100} className={inputClass} />
            </div>
          </>
        )}

        {/* Error handling */}
        {hasAgent && (
          <div>
            <label className={labelClass}>{t("canvas.error_mode")}</label>
            <select value={errorMode} onChange={e => setErrorMode(e.target.value)} className={inputClass}>
              <option value="fail">{t("canvas.error_fail")}</option>
              <option value="skip">{t("canvas.error_skip")}</option>
              <option value="retry">{t("canvas.error_retry")}</option>
            </select>
          </div>
        )}
        {hasAgent && errorMode === "retry" && (
          <div>
            <label className={labelClass}>{t("canvas.max_retries")}</label>
            <input type="number" value={maxRetries} onChange={e => setMaxRetries(Number(e.target.value))}
              min={1} max={10} className={inputClass} />
          </div>
        )}

        {/* Advanced options */}
        {hasAgent && (
          <>
            <div>
              <label className={labelClass}>{t("canvas.timeout")}</label>
              <input type="number" value={timeoutSecs} onChange={e => setTimeoutSecs(Number(e.target.value))}
                min={10} max={3600} className={inputClass} />
            </div>
            <div>
              <label className={labelClass}>
                {t("canvas.output_var")} <span className="text-text-dim/50 normal-case font-normal">{t("canvas.output_var_hint")}</span>
              </label>
              <input type="text" value={outputVar} onChange={e => setOutputVar(e.target.value)}
                placeholder="e.g. research_result" className={inputClass} />
            </div>
            {/* Depends On — multi-select other step nodes */}
            {(() => {
              // Collect sibling nodes that have an agent (i.e. are steps), excluding self
              const siblingSteps = (node as any)._siblingNodes as Array<{ id: string; label: string }> | undefined;
              if (!siblingSteps || siblingSteps.length === 0) return null;
              return (
                <div>
                  <label className={labelClass}>
                    {t("canvas.depends_on")} <span className="text-text-dim/50 normal-case font-normal">{t("canvas.depends_on_hint")}</span>
                  </label>
                  <div className="mt-1 space-y-1 max-h-28 overflow-y-auto rounded-lg border border-border-subtle bg-main p-1.5">
                    {siblingSteps.map(s => (
                      <label key={s.id} className="flex items-center gap-2 px-1.5 py-1 rounded hover:bg-primary/5 cursor-pointer">
                        <input
                          type="checkbox"
                          checked={dependsOn.includes(s.label)}
                          onChange={e => {
                            if (e.target.checked) setDependsOn([...dependsOn, s.label]);
                            else setDependsOn(dependsOn.filter(n => n !== s.label));
                          }}
                          className="rounded border-border-subtle"
                        />
                        <span className="text-xs text-text truncate">{s.label}</span>
                      </label>
                    ))}
                  </div>
                </div>
              );
            })()}
          </>
        )}

        <Button variant="primary" size="sm" className="w-full" onClick={handleSave}>
          {t("common.save")}
        </Button>
      </div>
    </div>
  );
}

export function CanvasPage() {
  return (
    <ReactFlowProvider>
      <CanvasPageInner />
    </ReactFlowProvider>
  );
}

function CanvasPageInner() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { t: routeTimestamp, wf: routeWorkflowId } = useSearch({ from: "/canvas" });
  const theme = useUIStore((s) => s.theme);
  const { fitView } = useReactFlow();
  const [nodes, setNodes, onNodesChange] = useNodesState<Node>([]);
  const [edges, setEdges, onEdgesChange] = useEdgesState<Edge>([]);
  const queryClient = useQueryClient();
  const agentsQuery = useAgents();
  const agents = agentsQuery.data ?? [];
  const workflowsQuery = useWorkflows();
  const workflows = workflowsQuery.data ?? [];
  const [selectedWorkflow, setSelectedWorkflow] = useState<WorkflowItem | null>(null);
  const [workflowName, setWorkflowName] = useState("");
  const [workflowDescription, setWorkflowDescription] = useState("");
  const [showWorkflowPanel, setShowWorkflowPanel] = useState(false);
  const [isFullscreen, setIsFullscreen] = useState(true);
  const [runningWorkflowId, setRunningWorkflowId] = useState<string | null>(null);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [editingNode, setEditingNode] = useState<Node | null>(null);
  const [runResult, setRunResult] = useState<{ output: string; status: string; run_id: string; step_results?: WorkflowStepResult[] } | null>(null);
  const [showRunInput, setShowRunInput] = useState<false | "run" | "dry">(false);
  const [runInput, setRunInput] = useState("");
  const [dryRunResult, setDryRunResult] = useState<DryRunResult | null>(null);
  const [isDryRunning, setIsDryRunning] = useState(false);
  const [expandedRunStep, setExpandedRunStep] = useState<number | null>(null);
  const [expandedDryStep, setExpandedDryStep] = useState<number | null>(null);

  const [selectedNodeIds, setSelectedNodeIds] = useState<Set<string>>(new Set());
  const [spacePressed, setSpacePressed] = useState(false);
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number; nodeId?: string } | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  const [showHelp, setShowHelp] = useState(false);
  const [showTemplateBrowser, setShowTemplateBrowser] = useState(false);
  const [showScheduleModal, setShowScheduleModal] = useState(false);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [zoomLevel, setZoomLevel] = useState(100);

  const createWorkflowMutation = useCreateWorkflow();
  const updateWorkflowMutation = useUpdateWorkflowMutation();
  const deleteWorkflowMutation = useDeleteWorkflow();
  const runWorkflowMutation = useRunWorkflow();
  const createScheduleMutation = useCreateSchedule();
  const dryRunWorkflowMutation = useDryRunWorkflow();
  const saveWorkflowAsTemplateMutation = useSaveWorkflowAsTemplate();

  // Undo/redo history
  const historyRef = useRef<{ nodes: Node[]; edges: Edge[] }[]>([]);
  const historyIndexRef = useRef(-1);
  const clipboardRef = useRef<{ nodes: Node[]; edges: Edge[] } | null>(null);

  const pushHistory = useCallback(() => {
    const snapshot = { nodes: JSON.parse(JSON.stringify(nodes)), edges: JSON.parse(JSON.stringify(edges)) };
    historyRef.current = historyRef.current.slice(0, historyIndexRef.current + 1);
    historyRef.current.push(snapshot);
    if (historyRef.current.length > 50) historyRef.current.shift();
    historyIndexRef.current = historyRef.current.length - 1;
  }, [nodes, edges]);

  const undo = useCallback(() => {
    if (historyIndexRef.current <= 0) return;
    // Save current state to end of history (if not already saved)
    if (historyIndexRef.current === historyRef.current.length - 1) pushHistory();
    historyIndexRef.current--;
    const s = historyRef.current[historyIndexRef.current];
    if (s) { setNodes(s.nodes); setEdges(s.edges); }
  }, [pushHistory, setNodes, setEdges]);

  const redo = useCallback(() => {
    if (historyIndexRef.current >= historyRef.current.length - 1) return;
    historyIndexRef.current++;
    const s = historyRef.current[historyIndexRef.current];
    if (s) { setNodes(s.nodes); setEdges(s.edges); }
  }, [setNodes, setEdges]);

  // Record snapshot before key operations
  const onNodesChangeWithHistory = useCallback((changes: any) => {
    // Record on drag end / delete
    const hasEnd = changes.some((c: any) => c.type === "position" && c.dragging === false);
    const hasRemove = changes.some((c: any) => c.type === "remove");
    if (hasEnd || hasRemove) pushHistory();
    onNodesChange(changes);
  }, [onNodesChange, pushHistory]);

  const onEdgesChangeWithHistory = useCallback((changes: any) => {
    const hasRemove = changes.some((c: any) => c.type === "remove");
    if (hasRemove) pushHistory();
    onEdgesChange(changes);
  }, [onEdgesChange, pushHistory]);

  // Copy selected nodes
  const copySelected = useCallback(() => {
    const selNodes = nodes.filter(n => selectedNodeIds.has(n.id));
    if (selNodes.length === 0) return;
    const selIds = new Set(selNodes.map(n => n.id));
    const selEdges = edges.filter(e => selIds.has(e.source) && selIds.has(e.target));
    clipboardRef.current = { nodes: JSON.parse(JSON.stringify(selNodes)), edges: JSON.parse(JSON.stringify(selEdges)) };
  }, [nodes, edges, selectedNodeIds]);

  // Paste
  const paste = useCallback(() => {
    if (!clipboardRef.current) return;
    pushHistory();
    const offset = 40;
    const idMap = new Map<string, string>();
    const newNodes = clipboardRef.current.nodes.map(n => {
      const newId = `${(n.data as any)?.nodeType || "node"}-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`;
      idMap.set(n.id, newId);
      return { ...n, id: newId, position: { x: n.position.x + offset, y: n.position.y + offset }, selected: true };
    });
    const newEdges = clipboardRef.current.edges.map(e => ({
      ...e,
      id: `e-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`,
      source: idMap.get(e.source) || e.source,
      target: idMap.get(e.target) || e.target,
    }));
    // Deselect old nodes
    setNodes(nds => [...nds.map(n => ({ ...n, selected: false })), ...newNodes]);
    setEdges(eds => [...eds, ...newEdges]);
  }, [pushHistory, setNodes, setEdges]);

  // Duplicate selected nodes (Cmd+D)
  const duplicate = useCallback(() => {
    copySelected();
    paste();
  }, [copySelected, paste]);

  const showError = useCallback((msg: string) => {
    setErrorMsg(msg);
    setTimeout(() => setErrorMsg(null), 5000);
  }, []);

  const toErrorMessage = useCallback((error: unknown, fallback?: string) => {
    if (error instanceof Error && error.message) return error.message;
    if (typeof error === "string" && error) return error;
    return fallback ?? String(error);
  }, []);

  const clearDraft = useCallback(() => {
    clearCanvasDraft();
  }, []);

  const applyCanvasState = useCallback((draft: CanvasDraft) => {
    setNodes(draft.nodes);
    setEdges(draft.edges.map((edge) => ({
      ...edge,
      markerEnd: edge.markerEnd ?? { type: MarkerType.ArrowClosed },
    })));
    setWorkflowName(draft.workflowName);
    setWorkflowDescription(draft.workflowDescription);
    setSelectedWorkflow(null);
  }, [setNodes, setEdges]);

  const persistDraft = useCallback((draft: CanvasDraft) => {
    if (
      draft.nodes.length === 0
      && draft.edges.length === 0
      && !draft.workflowName.trim()
      && !draft.workflowDescription.trim()
    ) {
      clearDraft();
      return;
    }
    if (typeof window === "undefined") return;
    sessionStorage.setItem(CANVAS_DRAFT_KEY, JSON.stringify(draft));
  }, [clearDraft]);

  // Recalculate group bounds to contain all child nodes (declared early, needed by autoLayout)
  const NODE_W = 200;
  const NODE_H = 80;
  const GROUP_PAD = 30;
  const GROUP_HEADER = 36;
  const recalcGroupBounds = useCallback((nds: Node[], groupId: string): Node[] => {
    const groupNode = nds.find(n => n.id === groupId);
    if (!groupNode || (groupNode.data as any)._expanded === false) return nds;
    const childIds = new Set<string>((groupNode.data as any)?._childIds || []);
    const children = nds.filter(n => childIds.has(n.id) && !n.hidden);
    if (children.length === 0) return nds;
    let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
    for (const c of children) {
      const w = c.measured?.width ?? c.width ?? NODE_W;
      const h = c.measured?.height ?? c.height ?? NODE_H;
      minX = Math.min(minX, c.position.x);
      minY = Math.min(minY, c.position.y);
      maxX = Math.max(maxX, c.position.x + w);
      maxY = Math.max(maxY, c.position.y + h);
    }
    const gx = minX - GROUP_PAD;
    const gy = minY - GROUP_PAD - GROUP_HEADER;
    const gw = maxX - minX + GROUP_PAD * 2;
    const gh = maxY - minY + GROUP_PAD * 2 + GROUP_HEADER;
    return nds.map(n => n.id === groupId ? {
      ...n, position: { x: gx, y: gy },
      style: { ...n.style, width: gw, height: gh },
      data: { ...(n.data as any), _origWidth: gw, _origHeight: gh },
    } : n);
  }, []);

  // Select all
  const selectAll = useCallback(() => {
    setNodes(nds => nds.map(n => ({ ...n, selected: true })));
  }, [setNodes]);

  // Auto layout (simple vertical arrangement)
  const autoLayout = useCallback(() => {
    pushHistory();
    const agentNodes = nodes.filter(n => n.type === "custom" && !n.hidden);
    const groupNodes = nodes.filter(n => n.type === "groupNode");
    const x = 100;
    let y = 80;
    const gap = 100;
    const positioned = new Map<string, { x: number; y: number }>();
    agentNodes.forEach(n => {
      positioned.set(n.id, { x, y });
      y += (n.measured?.height || 80) + gap;
    });
    setNodes(nds => nds.map(n => {
      const pos = positioned.get(n.id);
      return pos ? { ...n, position: pos } : n;
    }));
    // Recalculate group bounds
    groupNodes.forEach(g => {
      setNodes(nds => recalcGroupBounds(nds, g.id));
    });
  }, [nodes, pushHistory, setNodes, recalcGroupBounds]);

  // Toast notification
  const showToast = useCallback((msg: string) => {
    setToast(msg);
    setTimeout(() => setToast(null), 2000);
  }, []);

  // Export workflow JSON
  const exportWorkflow = useCallback(() => {
    const data = { nodes, edges, name: workflowName, description: workflowDescription };
    const blob = new Blob([JSON.stringify(data, null, 2)], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `${workflowName || "workflow"}.json`;
    a.click();
    URL.revokeObjectURL(url);
    showToast(t("canvas.exported"));
  }, [nodes, edges, workflowName, workflowDescription, showToast, t]);

  // Import workflow JSON
  const importWorkflow = useCallback(() => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = ".json";
    input.onchange = (e) => {
      const file = (e.target as HTMLInputElement).files?.[0];
      if (!file) return;
      const reader = new FileReader();
      reader.onload = () => {
        try {
          const data = JSON.parse(reader.result as string);
          if (data.nodes) { pushHistory(); setNodes(data.nodes); }
          if (data.edges) setEdges(data.edges);
          if (data.name) setWorkflowName(data.name);
          if (data.description) setWorkflowDescription(data.description);
          showToast(t("canvas.imported"));
        } catch { showError(t("canvas.import_error")); }
      };
      reader.readAsText(file);
    };
    input.click();
  }, [pushHistory, setNodes, setEdges, showToast, showError, t]);

  // Connection validation: prevent source->source or target->target
  const isValidConnection = useCallback((connection: Edge | Connection) => {
    return connection.source !== connection.target;
  }, []);

  // Shortcut key refs
  const createGroupRef = useRef<() => void>(() => { });
  const ungroupRef = useRef<(id: string) => void>(() => { });

  // Stable refs for group callbacks — prevents nodeTypes from changing on every render
  const toggleGroupRef = useRef<(id: string) => void>(() => { });
  const ungroupNodesRef = useRef<(id: string) => void>(() => { });
  const deleteGroupAndChildrenRef = useRef<(id: string) => void>(() => { });
  const tRef = useRef(t);

  useEffect(() => {
    const isInput = () => {
      const tag = document.activeElement?.tagName;
      return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT";
    };
    const down = (e: KeyboardEvent) => {
      if (isInput()) return; // Only handle when not in an input field
      const mod = e.metaKey || e.ctrlKey;
      // Space: pan mode
      if (e.code === "Space" && !e.repeat) { e.preventDefault(); setSpacePressed(true); }
      // Cmd+Z: undo
      if (e.code === "KeyZ" && mod && !e.shiftKey) { e.preventDefault(); undo(); }
      // Cmd+Shift+Z: redo
      if (e.code === "KeyZ" && mod && e.shiftKey) { e.preventDefault(); redo(); }
      // Cmd+C: copy
      if (e.code === "KeyC" && mod) { e.preventDefault(); copySelected(); }
      // Cmd+V: paste
      if (e.code === "KeyV" && mod) { e.preventDefault(); paste(); }
      // Cmd+D: duplicate
      if (e.code === "KeyD" && mod) { e.preventDefault(); duplicate(); }
      // Cmd+A: select all
      if (e.code === "KeyA" && mod) { e.preventDefault(); selectAll(); }
      // Cmd+B: create group
      if (e.code === "KeyB" && mod && !e.shiftKey) { e.preventDefault(); createGroupRef.current(); }
      // Shift+Cmd+B: ungroup
      if (e.code === "KeyB" && mod && e.shiftKey) {
        e.preventDefault();
        const groupNode = nodes.find(n => selectedNodeIds.has(n.id) && n.type === "groupNode");
        if (groupNode) ungroupRef.current(groupNode.id);
      }
      // Cmd+1: fit viewport
      if (e.code === "Digit1" && mod) { e.preventDefault(); fitView({ padding: 0.2, duration: 300 }); }
      // Cmd+E: export
      if (e.code === "KeyE" && mod) { e.preventDefault(); exportWorkflow(); }
      // Cmd+I: import
      if (e.code === "KeyI" && mod) { e.preventDefault(); importWorkflow(); }
      // ?: shortcut help
      if (e.code === "Slash" && e.shiftKey && !mod) { e.preventDefault(); setShowHelp(h => !h); }
    };
    const up = (e: KeyboardEvent) => { if (e.code === "Space") setSpacePressed(false); };
    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);
    return () => { window.removeEventListener("keydown", down); window.removeEventListener("keyup", up); };
  }, [nodes, selectedNodeIds, undo, redo, copySelected, paste, duplicate, selectAll, fitView, exportWorkflow, importWorkflow]);

  // Collapse/expand group
  const toggleGroup = useCallback((groupId: string) => {
    setNodes(nds => {
      const groupNode = nds.find(n => n.id === groupId);
      if (!groupNode) return nds;
      const gd = groupNode.data as any;
      const isExpanded = gd._expanded !== false;
      const willCollapse = isExpanded;
      const childIds = new Set<string>(gd._childIds || []);

      // Record current dimensions on collapse, restore on expand
      const origStyle = willCollapse
        ? { _origWidth: groupNode.style?.width, _origHeight: groupNode.style?.height }
        : {};

      return nds.map(n => {
        if (n.id === groupId) {
          return {
            ...n,
            style: willCollapse
              ? { ...n.style, width: 160, height: undefined, zIndex: 0 }
              : { ...n.style, width: gd._origWidth || 300, height: gd._origHeight || 200, zIndex: -1 },
            data: { ...gd, ...origStyle, _expanded: !isExpanded },
          };
        }
        if (childIds.has(n.id)) {
          return { ...n, hidden: willCollapse };
        }
        return n;
      });
    });

    // Handle edges
    setEdges(eds => {
      const groupNode = nodes.find(n => n.id === groupId);
      const gd = groupNode?.data as any;
      const isExpanded = gd?._expanded !== false;
      const willCollapse = isExpanded;
      const childIds = new Set<string>(gd?._childIds || []);

      return eds.map(e => {
        const srcChild = childIds.has(e.source);
        const tgtChild = childIds.has(e.target);

        // Internal edges: hide on collapse
        if (srcChild && tgtChild) {
          return { ...e, hidden: willCollapse };
        }
        if (willCollapse) {
          // External edges: redirect to group node, save original endpoints
          if (srcChild) return { ...e, data: { ...e.data, _origSource: e.source }, source: groupId };
          if (tgtChild) return { ...e, data: { ...e.data, _origTarget: e.target }, target: groupId };
        } else {
          // Expand: restore original endpoints
          const ed = e.data as any;
          if (ed?._origSource) return { ...e, source: ed._origSource, data: { ...e.data, _origSource: undefined }, hidden: false };
          if (ed?._origTarget) return { ...e, target: ed._origTarget, data: { ...e.data, _origTarget: undefined }, hidden: false };
          // Restore internal edge visibility
          if (srcChild && tgtChild) return { ...e, hidden: false };
        }
        return e;
      });
    });
  }, [nodes, setNodes, setEdges]);

  // Ungroup: remove group node, keep child nodes and clear _groupId
  const ungroupNodes = useCallback((groupId: string) => {
    setNodes(nds => {
      const group = nds.find(n => n.id === groupId);
      const childIds = new Set<string>((group?.data as any)?._childIds || []);
      return nds
        .filter(n => n.id !== groupId)
        .map(n => childIds.has(n.id)
          ? { ...n, data: { ...(n.data as any), _groupId: undefined } }
          : n
        );
    });
    // Restore redirected edges
    setEdges(eds => eds.map(e => {
      const ed = e.data as any;
      if (ed?._origSource) return { ...e, source: ed._origSource, data: { ...e.data, _origSource: undefined }, hidden: false };
      if (ed?._origTarget) return { ...e, target: ed._origTarget, data: { ...e.data, _origTarget: undefined }, hidden: false };
      return { ...e, hidden: false };
    }));
  }, [setNodes, setEdges]);

  // Delete group + all child nodes
  const deleteGroupAndChildren = useCallback((groupId: string) => {
    setNodes(nds => {
      const group = nds.find(n => n.id === groupId);
      const childIds = new Set<string>((group?.data as any)?._childIds || []);
      childIds.add(groupId);
      return nds.filter(n => !childIds.has(n.id));
    });
    // Delete edges involving child nodes
    setEdges(eds => {
      const group = nodes.find(n => n.id === groupId);
      const childIds = new Set<string>((group?.data as any)?._childIds || []);
      childIds.add(groupId);
      return eds.filter(e => !childIds.has(e.source) && !childIds.has(e.target));
    });
  }, [nodes, setNodes, setEdges]);

  // IMPORTANT: nodeTypes must be referentially stable to prevent ReactFlow from
  // unmounting/remounting all nodes on every render, which breaks click handlers.
  // We use refs for all callbacks and the translation function so the deps are empty.
  const nodeTypes = useMemo(() => ({
    custom: (props: any) => <CustomNode {...props} t={tRef.current} />,
    groupNode: (props: any) => <GroupNodeComponent {...props} data={{
      ...props.data,
      _onToggle: (id: string) => toggleGroupRef.current(id),
      _onUngroup: (id: string) => ungroupNodesRef.current(id),
      _onDeleteGroup: (id: string) => deleteGroupAndChildrenRef.current(id),
    }} />,
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }), []);

  // Node types that require an agent (all backend steps need an agent)
  const AGENT_NODE_TYPES = AGENT_NODE_TYPES_SET;

  // Load template data (agents list passed in for auto-assignment)
  const loadTemplate = useCallback((availableAgents: AgentItem[]) => {
    const templateData = sessionStorage.getItem("workflowTemplate");
    if (templateData) {
      try {
        const { nodes: templateNodes, edges: templateEdges, name, description, workflowId } = JSON.parse(templateData);
        // Find an available agent as default assignment
        const defaultAgent = availableAgents.find(a => a.state === "Running") || availableAgents[0];
        // Determine output language instruction based on UI language
        const lang = t("_lang", { defaultValue: "en" });
        const langSuffix = lang === "zh" ? "\n\nIMPORTANT: You MUST respond entirely in Chinese (中文)." : "";
        const newNodes = templateNodes.map((n: any, idx: number) => {
          const nodeType = n.data?.nodeType;
          const needsAgent = AGENT_NODE_TYPES.has(nodeType);
          const rawPrompt = n.data?.prompt || (n.data?.description ? t(n.data.description) : "");
          return {
            id: n.id || `${n.type || 'custom'}-${Date.now()}-${idx}`,
            type: "custom",
            position: n.position || { x: 50, y: idx * 80 },
            data: {
              label: n.data?.label ? t(n.data.label) : t("canvas.node_types.start"),
              description: n.data?.description ? t(n.data.description) : t("canvas.node_types.start_desc"),
              nodeType,
              labelKey: n.data?.label,
              descKey: n.data?.description,
              // Keep existing agent binding (look up name by ID), or auto-assign default agent
              ...(n.data?.agentId ? {
                agentId: n.data.agentId,
                agentName: n.data.agentName || availableAgents.find(a => a.id === n.data.agentId)?.name || n.data.agentId,
                prompt: n.data.prompt || rawPrompt,
              } : needsAgent && defaultAgent ? {
                agentId: defaultAgent.id,
                agentName: defaultAgent.name,
                prompt: rawPrompt + langSuffix,
              } : {}),
            }
          };
        });
        setNodes(newNodes);
        if (Array.isArray(templateEdges) && templateEdges.length > 0) {
          setEdges(templateEdges.map((e: any) => ({
            ...e,
            markerEnd: { type: MarkerType.ArrowClosed },
          })));
        } else {
          setEdges([]);
        }
        if (name) setWorkflowName(name.startsWith("workflows.") ? t(name) : name);
        if (description) setWorkflowDescription(description.startsWith("workflows.") ? t(description) : description);
        // If editing an existing workflow, restore selectedWorkflow so save uses update logic
        if (workflowId) setSelectedWorkflow({ id: workflowId, name: name || "", description: description || "" } as WorkflowItem);
        sessionStorage.removeItem("workflowTemplate");
        return "loaded" as const;
      } catch (e: unknown) {
        sessionStorage.removeItem("workflowTemplate");
        showError(toErrorMessage(e, t("canvas.template_load_error", { defaultValue: "Failed to load template" })));
        return "failed" as const;
      }
    }
    return "missing" as const;
  }, [t, setNodes, setEdges, showError, toErrorMessage]);

  const loadWorkflowIntoCanvas = useCallback(async (workflowId: string, fallback?: WorkflowItem | null) => {
    const detail = await queryClient.fetchQuery(workflowQueries.detail(workflowId));
    let wfNodes: Node[];
    let wfEdges: Edge[];
    const layout = detail.layout as { nodes?: Node[]; edges?: Edge[] } | undefined;
    if (layout?.nodes) {
      wfNodes = layout.nodes;
      wfEdges = layout.edges || [];
    } else {
      const steps = Array.isArray(detail.steps) ? detail.steps : [];
      wfNodes = steps.map((s: any, idx: number) => ({
        id: `node-${idx}`,
        type: "custom",
        position: { x: 50, y: idx * 80 },
        data: { label: s.name, prompt: s.prompt_template || "", nodeType: "agent", agentId: s.agent?.id, agentName: s.agent?.name },
      }));
      const hasDag = steps.some((step: any) => Array.isArray(step.depends_on) && step.depends_on.length > 0);
      if (hasDag) {
        const nameToId: Record<string, string> = {};
        steps.forEach((step: any, idx: number) => {
          nameToId[step.name] = `node-${idx}`;
        });
        wfEdges = [];
        steps.forEach((step: any, idx: number) => {
          (step.depends_on || []).forEach((dep: string, depIdx: number) => {
            const sourceId = nameToId[dep];
            if (sourceId) {
              wfEdges.push({
                id: `dep-${idx}-${depIdx}`,
                source: sourceId,
                target: `node-${idx}`,
                style: { strokeDasharray: "6 3" },
                label: "depends",
                labelStyle: { fontSize: 9, fill: "#6b7280" },
              });
            }
          });
        });
      } else {
        wfEdges = wfNodes.slice(0, -1).map((_, i) => ({ id: `e-${i}`, source: `node-${i}`, target: `node-${i + 1}` }));
      }
    }

    setNodes(wfNodes);
    setEdges(wfEdges.map((edge) => ({ ...edge, markerEnd: { type: MarkerType.ArrowClosed } })));
    setWorkflowName(detail.name || fallback?.name || "");
    setWorkflowDescription(detail.description || fallback?.description || "");
    setSelectedWorkflow({
      id: workflowId,
      name: detail.name || fallback?.name || "",
      description: detail.description || fallback?.description || "",
    } as WorkflowItem);
    setErrorMsg(null);
  }, [setNodes, setEdges, queryClient]);

  // Snapshot the polling-driven agents/workflows lists into refs so the
  // load-route effect below doesn't depend on their reference identity.
  // useAgents() / useWorkflows() refetch every 30s, which produces fresh
  // array references on every poll cycle even when no row actually
  // changed.  Including them in the effect deps re-runs the loader and
  // calls setNodes/setEdges/setWorkflowName mid-edit — clobbering the
  // user's unsaved canvas every 30 seconds (#3958 follow-up).
  const agentsRef = useRef(agents);
  const workflowsRef = useRef(workflows);
  useEffect(() => {
    agentsRef.current = agents;
  }, [agents]);
  useEffect(() => {
    workflowsRef.current = workflows;
  }, [workflows]);

  // Track which (timestamp, workflowId) tuple has already been loaded so
  // even unrelated dep changes can't re-trigger the load.
  const loadedRouteKeyRef = useRef<string | null>(null);

  // Load template or workflow from URL once agent/workflow data is available
  useEffect(() => {
    if (agentsQuery.isLoading || workflowsQuery.isLoading) return;
    const routeKey = `${routeTimestamp ?? ""}|${routeWorkflowId ?? ""}`;
    if (loadedRouteKeyRef.current === routeKey) {
      return;
    }
    loadedRouteKeyRef.current = routeKey;
    const run = async () => {
        const draft = readCanvasDraft();
        // 1. Try loading from sessionStorage template
        const templateState = loadTemplate(agentsRef.current);
        if (templateState !== "missing") return;
        // 2. Try loading from URL ?wf= parameter
        if (routeWorkflowId) {
          try {
            await loadWorkflowIntoCanvas(
              routeWorkflowId,
              workflowsRef.current.find((item) => item.id === routeWorkflowId) ?? null,
            );
            return;
          } catch (e: unknown) {
            showError(toErrorMessage(e, t("canvas.workflow_load_error", { defaultValue: "Failed to load workflow" })));
            setNodes([]);
            setEdges([]);
            setWorkflowName("");
            setWorkflowDescription("");
            setSelectedWorkflow(null);
            return;
          }
        }
        // 3. Blank canvas can restore the unsaved draft.
        if (draft) {
          applyCanvasState(draft);
        }
    };
    run().catch((e: unknown) => { showError(toErrorMessage(e, t("canvas.load_error", { defaultValue: "Failed to load data" }))); });
  }, [routeTimestamp, routeWorkflowId, agentsQuery.isLoading, workflowsQuery.isLoading, applyCanvasState, loadTemplate, loadWorkflowIntoCanvas, showError, t, toErrorMessage, setNodes, setEdges]);

  // Persist only unsaved blank-canvas drafts. Saved workflows should reload from backend.
  useEffect(() => {
    if (routeWorkflowId || selectedWorkflow?.id) {
      clearDraft();
      return;
    }
    persistDraft({
      nodes,
      edges,
      workflowName,
      workflowDescription,
    });
  }, [nodes, edges, workflowName, workflowDescription, selectedWorkflow, routeWorkflowId, persistDraft, clearDraft]);

  // nodeType -> default stepMode mapping
  const NODE_MODE_MAP: Record<string, string> = {
    condition: "conditional",
    loop: "loop",
    parallel: "fan_out",
    collect: "collect",
  };

  // Add node
  const addNode = useCallback((type: string) => {
    const config = NODE_TYPES.find(n => n.type === type) || NODE_TYPES[10];
    const defaultMode = NODE_MODE_MAP[type];
    // Use functional update to read latest nodes, avoiding stale closures
    setNodes(nds => {
      const existing = nds.filter(n => n.type === "custom" && !n.hidden);
      let maxY = 0;
      for (const n of existing) {
        const bottom = n.position.y + (n.measured?.height || 80);
        if (bottom > maxY) maxY = bottom;
      }
      const newNode: Node = {
        id: `${type}-${Date.now()}-${Math.random().toString(36).slice(2, 5)}`,
        type: "custom",
        position: { x: 100, y: existing.length === 0 ? 80 : maxY + 40 },
        data: {
          label: t(config.labelKey),
          description: t(config.descKey),
          nodeType: type,
          ...(defaultMode ? { stepMode: defaultMode } : {}),
        }
      };
      return [...nds, newNode];
    });
  }, [setNodes, t]);

  // Edge connections
  const edgeColor = theme === "dark" ? "#6b7280" : "#94a3b8";
  const edgeColorActive = theme === "dark" ? "#818cf8" : "#6366f1";

  const defaultEdgeOptions = useMemo(() => ({
    type: "smoothstep" as const,
    animated: false,
    style: { stroke: edgeColor, strokeWidth: 2 },
    markerEnd: { type: MarkerType.ArrowClosed, color: edgeColor, width: 16, height: 16 },
  }), [edgeColor]);

  const onConnect = useCallback((params: Connection) => {
    setEdges((eds) => addEdge({
      ...params,
      type: "smoothstep",
      style: { stroke: edgeColorActive, strokeWidth: 2 },
      markerEnd: { type: MarkerType.ArrowClosed, color: edgeColorActive, width: 16, height: 16 },
    }, eds));
  }, [setEdges, edgeColorActive]);

  // Node click -> open config panel
  const onNodeClick = useCallback((_: any, node: Node) => {
    setEditingNode(node);
  }, []);

  // Clean up editing panel when nodes are deleted
  const onNodesDelete = useCallback((deleted: Node[]) => {
    if (editingNode && deleted.some(n => n.id === editingNode.id)) {
      setEditingNode(null);
    }
  }, [editingNode]);

  // Group drag moves child nodes along
  const groupDragStart = useRef<{ id: string; x: number; y: number } | null>(null);

  const onNodeDragStart = useCallback((_: any, node: Node) => {
    if (node.type === "groupNode") {
      groupDragStart.current = { id: node.id, x: node.position.x, y: node.position.y };
    }
  }, []);

  const onNodeDrag = useCallback((_: any, node: Node) => {
    if (node.type === "groupNode" && groupDragStart.current?.id === node.id) {
      // Dragging group -> move child nodes along
      const dx = node.position.x - groupDragStart.current.x;
      const dy = node.position.y - groupDragStart.current.y;
      if (dx === 0 && dy === 0) return;
      const childIds = new Set<string>((node.data as any)?._childIds || []);
      groupDragStart.current = { id: node.id, x: node.position.x, y: node.position.y };
      setNodes(nds => nds.map(n =>
        childIds.has(n.id) && !n.hidden
          ? { ...n, position: { x: n.position.x + dx, y: n.position.y + dy } }
          : n
      ));
    } else {
      // Dragging child node -> expand parent group bounds
      const groupId = (node.data as any)?._groupId;
      if (groupId) {
        setNodes(nds => recalcGroupBounds(nds, groupId));
      }
    }
  }, [setNodes, recalcGroupBounds]);

  // Track selected nodes
  const onSelectionChange = useCallback(({ nodes: selected }: OnSelectionChangeParams) => {
    setSelectedNodeIds(new Set(selected.map(n => n.id)));
  }, []);

  // Create group: keep child node positions, just add a background frame underneath + mark ownership
  const createGroup = useCallback(() => {
    if (selectedNodeIds.size < 2) return;

    const selected = nodes.filter(n => selectedNodeIds.has(n.id) && n.type !== "groupNode");
    if (selected.length < 2) return;

    // Manually calculate bounding box (considering node dimensions, getNodesBounds may not be accurate)
    const NODE_W = 200; // Max node width
    const NODE_H = 80;  // Estimated node height
    let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
    for (const n of selected) {
      const w = (n.measured?.width ?? n.width ?? NODE_W);
      const h = (n.measured?.height ?? n.height ?? NODE_H);
      minX = Math.min(minX, n.position.x);
      minY = Math.min(minY, n.position.y);
      maxX = Math.max(maxX, n.position.x + w);
      maxY = Math.max(maxY, n.position.y + h);
    }
    const padding = 30;
    const headerH = 36;
    const groupId = `group-${Date.now()}`;
    const childIds = selected.map(n => n.id);
    const gw = maxX - minX + padding * 2;
    const gh = maxY - minY + padding * 2 + headerH;

    const groupNode: Node = {
      id: groupId,
      type: "groupNode",
      position: { x: minX - padding, y: minY - padding - headerH },
      style: { width: gw, height: gh, zIndex: -1 },
      zIndex: -1,
      data: {
        label: t("canvas.new_group"),
        _expanded: true,
        _childIds: childIds,
        _childCount: childIds.length,
        _origWidth: gw,
        _origHeight: gh,
      },
    };

    // Use functional update: insert group before existing nodes, update child node data to mark ownership
    // Don't replace the entire array, preserve ReactFlow internal node state (measured, etc.)
    setNodes(nds => [
      groupNode,
      ...nds.map(n => childIds.includes(n.id)
        ? { ...n, data: { ...(n.data as any), _groupId: groupId } }
        : n
      ),
    ]);
    setSelectedNodeIds(new Set());
  }, [selectedNodeIds, nodes, setNodes, t]);

  // Sync shortcut key refs
  createGroupRef.current = createGroup;
  ungroupRef.current = ungroupNodes;

  // Sync stable refs for group callbacks used by nodeTypes
  toggleGroupRef.current = toggleGroup;
  ungroupNodesRef.current = ungroupNodes;
  deleteGroupAndChildrenRef.current = deleteGroupAndChildren;
  tRef.current = t;

  // Update node data
  const handleNodeUpdate = useCallback((id: string, newData: any) => {
    setNodes(nds => nds.map(n => n.id === id ? { ...n, data: newData } : n));
  }, [setNodes]);

  // Build backend steps from nodes: only nodes bound to a real agent are steps
  const buildSteps = useCallback((nodeList: Node[]) => {
    return nodeList
      .filter(n => {
        const d = n.data as any;
        return d.agentId || d.agentName;
      })
      .map((n, idx) => {
        const d = n.data as any;
        const step: any = {
          name: d.label || `Step ${idx + 1}`,
          agent_id: d.agentId,
          agent_name: d.agentName,
          prompt: d.prompt || d.description || "",
          timeout_secs: d.timeoutSecs || 120,
        };
        // Execution mode
        const mode = d.stepMode || "sequential";
        if (mode === "conditional") {
          step.mode = { conditional: { condition: d.condition || "" } };
        } else if (mode === "loop") {
          step.mode = { loop: { max_iterations: d.maxIterations || 5, until: d.until || "" } };
        } else {
          step.mode = mode;
        }
        // Error mode
        const errMode = d.errorMode || "fail";
        if (errMode === "retry") {
          step.error_mode = { retry: { max_retries: d.maxRetries || 3 } };
        } else {
          step.error_mode = errMode;
        }
        // Output variable
        if (d.outputVar) step.output_var = d.outputVar;
        // DAG dependencies
        if (d.dependsOn && d.dependsOn.length > 0) step.depends_on = d.dependsOn;
        return step;
      });
  }, []);

  const ensureSavedWorkflow = useCallback(async (options?: { requireName?: boolean }) => {
    const requireName = options?.requireName ?? false;
    const trimmedName = workflowName.trim();
    if (requireName && !trimmedName) {
      showError(t("canvas.name_required"));
      return null;
    }

    const steps = buildSteps(nodes);
    if (steps.length === 0) {
      showError(t("canvas.no_agent_steps"));
      return null;
    }

    const resolvedName = trimmedName || t("workflows.untitled_workflow");
    const layout = { nodes, edges };

    if (selectedWorkflow?.id) {
      const workflowId = selectedWorkflow.id;
      await updateWorkflowMutation.mutateAsync({
        workflowId,
        payload: { name: resolvedName, description: workflowDescription, steps, layout },
      });
      const updatedWorkflow = {
        id: workflowId,
        name: resolvedName,
        description: workflowDescription,
      } as WorkflowItem;
      setSelectedWorkflow(updatedWorkflow);
      setWorkflowName(resolvedName);
      navigate({ to: "/canvas", search: { t: undefined, wf: workflowId }, replace: true });
      clearDraft();
      return { id: workflowId, workflow: updatedWorkflow, created: false };
    }

    const created = await createWorkflowMutation.mutateAsync({
      name: resolvedName,
      description: workflowDescription,
      steps,
      layout,
    });
    const createdId = typeof created?.id === "string" ? created.id : null;
    if (!createdId) {
      throw new Error(t("canvas.save_error", { defaultValue: "Failed to create workflow" }));
    }
    const createdWorkflow = {
      id: createdId,
      name: resolvedName,
      description: workflowDescription,
      steps: typeof created.steps === "number" || Array.isArray(created.steps) ? created.steps : steps.length,
      created_at: created.created_at,
    } as WorkflowItem;
    setSelectedWorkflow(createdWorkflow);
    setWorkflowName(resolvedName);
    navigate({ to: "/canvas", search: { t: undefined, wf: createdId }, replace: true });
    clearDraft();
    return { id: createdId, workflow: createdWorkflow, created: true };
  }, [
    workflowName,
    buildSteps,
    nodes,
    edges,
    selectedWorkflow,
    workflowDescription,
    updateWorkflowMutation,
    navigate,
    clearDraft,
    t,
    createWorkflowMutation,
    showError,
  ]);

  // Save workflow
  const handleSave = useCallback(async () => {
    try {
      const saved = await ensureSavedWorkflow({ requireName: true });
      if (!saved) return;
      setErrorMsg(null);
      showToast(t("canvas.saved"));
    } catch (e: unknown) {
      showError(toErrorMessage(e));
    }
  }, [ensureSavedWorkflow, t, showError, showToast, toErrorMessage]);

  // Save as template
  const handleSaveAsTemplate = useCallback(async () => {
    if (!selectedWorkflow?.id) {
      showError(t("canvas.save_first_to_template"));
      return;
    }
    try {
      await saveWorkflowAsTemplateMutation.mutateAsync(selectedWorkflow.id);
      showToast(t("canvas.saved_as_template"));
    } catch (e: unknown) {
      showError(toErrorMessage(e));
    }
  }, [selectedWorkflow, t, showError, showToast, toErrorMessage, saveWorkflowAsTemplateMutation]);



  // Click run -> show input dialog
  const handleRunClick = useCallback((id?: string) => {
    if (id === "dry") {
      // Dry Run button clicked — open input dialog in dry-run mode
      setRunInput("");
      setShowRunInput("dry");
    } else if (id) {
      // Run saved workflow directly from sidebar
      setRunInput("");
      setShowRunInput("run");
    } else if (selectedWorkflow?.id || nodes.length > 0) {
      setRunInput("");
      setShowRunInput("run");
    }
  }, [selectedWorkflow, nodes]);

  // Confirm run
  const handleRunConfirm = useCallback(async (id?: string) => {
    setShowRunInput(false);
    let workflowId = id || selectedWorkflow?.id;

    // No saved workflow -> save first
    if (!workflowId && nodes.length > 0) {
      try {
        const saved = await ensureSavedWorkflow();
        if (!saved) return;
        workflowId = saved.id;
      } catch (e: unknown) {
        showError(toErrorMessage(e));
        return;
      }
    }

    if (!workflowId) return;

    setRunningWorkflowId(workflowId);
    setErrorMsg(null);
    setRunResult(null);
    setDryRunResult(null);
    setExpandedRunStep(null);
    // Edge animation during run
    setEdges(eds => eds.map(e => ({ ...e, animated: true })));
    setNodes(nds => nds.map(n => ({
      ...n,
      data: {
        ...(n.data as any),
        _runState: (n.data as any).agentId ? "running" : undefined,
      }
    })));

    try {
      const resp = await runWorkflowMutation.mutateAsync({ workflowId, input: runInput });
      setRunResult({
        output: (resp as any).output || (resp as any).message || JSON.stringify(resp),
        status: (resp as any).status || "completed",
        run_id: (resp as any).run_id || "",
        step_results: (resp as any).step_results ?? [],
      });
      setNodes(nds => nds.map(n => ({ ...n, data: { ...(n.data as any), _runState: undefined } })));
      setEdges(eds => eds.map(e => ({ ...e, animated: false })));
    } catch (e: any) {
      // Error: clear all state and edge animation
      setNodes(nds => nds.map(n => ({ ...n, data: { ...(n.data as any), _runState: undefined } })));
      setEdges(eds => eds.map(e => ({ ...e, animated: false })));
      const detail = (e as any)?.message || String(e);
      showError(detail);
    } finally {
      setRunningWorkflowId(null);
    }
  }, [selectedWorkflow, nodes.length, ensureSavedWorkflow, toErrorMessage, showError, runInput, runWorkflowMutation]);

  // Dry-run: resolve agents and expand prompts without calling any LLMs
  const handleDryRun = useCallback(async (id?: string) => {
    setShowRunInput(false);
    let workflowId = id || selectedWorkflow?.id;
    if (!workflowId) {
      showError(t("canvas.no_agent_steps"));
      return;
    }
    setIsDryRunning(true);
    setDryRunResult(null);
    setRunResult(null);
    setExpandedDryStep(null);
    try {
      const result = await dryRunWorkflowMutation.mutateAsync({ workflowId, input: runInput });
      setDryRunResult(result);
    } catch (e: any) {
      showError((e as any)?.message || String(e));
    } finally {
      setIsDryRunning(false);
    }
  }, [selectedWorkflow, runInput, showError, t, dryRunWorkflowMutation]);

  // Delete workflow
  const handleDeleteConfirmed = useCallback(async (id: string) => {
    try {
      await deleteWorkflowMutation.mutateAsync(id);
      if (selectedWorkflow?.id === id) {
        setSelectedWorkflow(null);
        setNodes([]); setEdges([]);
        setWorkflowName(""); setWorkflowDescription("");
        setEditingNode(null);
        clearDraft();
        navigate({ to: "/canvas", search: { t: undefined, wf: undefined }, replace: true });
      }
    } catch (e: unknown) {
      showError(toErrorMessage(e));
    }
  }, [selectedWorkflow, deleteWorkflowMutation, clearDraft, navigate, showError, toErrorMessage]);

  // Select saved workflow
  const handleSelectWorkflow = useCallback(async (w: WorkflowItem) => {
    setEditingNode(null);
    try {
      await loadWorkflowIntoCanvas(w.id, w);
      navigate({ to: "/canvas", search: { t: undefined, wf: w.id }, replace: true });
    } catch (e: unknown) {
      showError(toErrorMessage(e, t("canvas.workflow_load_error", { defaultValue: "Failed to load workflow" })));
    }
  }, [loadWorkflowIntoCanvas, navigate, showError, t, toErrorMessage]);

  // Create new workflow
  const handleNewWorkflow = useCallback(() => {
    setSelectedWorkflow(null);
    setNodes([]); setEdges([]);
    setWorkflowName(""); setWorkflowDescription("");
    setEditingNode(null);
    clearDraft();
    navigate({ to: "/canvas", search: { t: undefined, wf: undefined }, replace: true });
  }, [clearDraft, navigate, setNodes, setEdges]);

  // Template instantiation callback: close browser, refresh workflow list, select new workflow
  const handleTemplateInstantiate = useCallback(async (workflowId: string) => {
    setShowTemplateBrowser(false);
    try {
      // Fetch the fresh list synchronously to this scope rather than
      // invalidate-then-read-closure.  #3958 swapped pre-PR's
      // `await listWorkflows()` for `invalidateQueries()` plus a
      // closure read of `workflows` — but that closure value is the
      // pre-invalidation snapshot, which for a just-instantiated
      // template doesn't contain the new id yet, so `created` is
      // undefined and the canvas header falls back to "" name and
      // description until the next 30s poll.  fetchQuery awaits the
      // network round-trip and returns the up-to-date list directly.
      const fresh = await queryClient.fetchQuery(workflowQueries.list());
      const created = fresh.find((w) => w.id === workflowId);
      await loadWorkflowIntoCanvas(workflowId, created ?? null);
      navigate({ to: "/canvas", search: { t: undefined, wf: workflowId }, replace: true });
    } catch (e: unknown) {
      showError(toErrorMessage(e, t("canvas.template_instantiate_error")));
    }
  }, [queryClient, loadWorkflowIntoCanvas, navigate, showError, t, toErrorMessage]);

  // Valid agent step count
  const agentStepCount = useMemo(() => buildSteps(nodes).length, [nodes, buildSteps]);

  return (
    <div className={`flex flex-col transition-all duration-300 ${isFullscreen ? "fixed inset-0 z-100 bg-main" : "h-[calc(100vh-140px)]"}`}>
      <header className="flex flex-wrap justify-between items-center gap-2 pb-2 sm:pb-4">
        <div className="flex items-center gap-2 sm:gap-4">
          {isFullscreen && (
            <Button variant="ghost" size="sm" onClick={() => navigate({ to: "/workflows" })}>
              <ArrowLeft className="w-4 h-4 mr-1" />
              {t("common.back")}
            </Button>
          )}
          {!isFullscreen && (
            <>
              <div>
                <h1 className="text-2xl font-extrabold">{t("canvas.title")}</h1>
                <p className="text-text-dim font-medium text-sm">{t("canvas.subtitle")}</p>
              </div>
              <Button variant="secondary" size="sm" onClick={handleNewWorkflow}>
                <Plus className="w-4 h-4 mr-1" />
                {t("workflows.new_workflow")}
              </Button>
            </>
          )}
        </div>
        <div className="flex items-center gap-1 flex-wrap">
          {/* Status info */}
          {selectedNodeIds.size >= 2 && (
            <Button variant="secondary" size="sm" onClick={createGroup}>
              <Group className="w-3.5 h-3.5 mr-1" />
              <span className="hidden sm:inline">{t("canvas.create_group")}</span>
            </Button>
          )}
          {agentStepCount > 0 && (
            <span className="text-[10px] font-bold text-success px-2 hidden sm:inline">
              {agentStepCount} {t("canvas.agent_steps")}
            </span>
          )}

          {/* View tools */}
          <div className="flex items-center gap-0.5 px-0.5 sm:px-1">
            <Button variant="secondary" onClick={() => setIsFullscreen(!isFullscreen)} title={isFullscreen ? t("canvas.exit_fullscreen") : t("canvas.fullscreen")}>
              {isFullscreen ? <Minimize2 className="w-4 h-4" /> : <Maximize2 className="w-4 h-4" />}
            </Button>
            <Button variant="secondary" onClick={() => fitView({ padding: 0.2, duration: 300 })} title={t("canvas.fit_view")}>
              <Scan className="w-4 h-4" />
            </Button>
          </div>

          <div className="w-px h-5 bg-border-subtle hidden sm:block" />

          {/* File operations */}
          <div className="flex items-center gap-0.5 px-0.5 sm:px-1">
            <Button variant="secondary" onClick={() => setShowWorkflowPanel(!showWorkflowPanel)} title={t("workflows.open_workflows")}>
              <FolderOpen className="w-4 h-4" />
            </Button>
            <Button variant="secondary" onClick={() => setShowTemplateBrowser(true)} title={t("canvas.browse_templates")}>
              <LayoutTemplate className="w-4 h-4" />
            </Button>
            <Button variant="secondary" onClick={exportWorkflow} title={t("canvas.export")}>
              <Download className="w-4 h-4" />
            </Button>
            <Button variant="secondary" onClick={importWorkflow} title={t("canvas.import")} className="hidden sm:flex">
              <Upload className="w-4 h-4" />
            </Button>
          </div>

          <div className="w-px h-5 bg-border-subtle hidden sm:block" />

          {/* Canvas operations */}
          <div className="flex items-center gap-0.5 px-0.5 sm:px-1">
            <Button variant="secondary" onClick={handleNewWorkflow} title={t("common.clear")}>
              <Trash2 className="w-4 h-4" />
            </Button>
            <Button variant="secondary" onClick={() => setShowHelp(true)} title={t("canvas.shortcuts")} className="hidden sm:flex">
              <HelpCircle className="w-4 h-4" />
            </Button>
          </div>

          <div className="w-px h-5 bg-border-subtle hidden sm:block" />

          {/* Primary actions */}
          <div className="flex items-center gap-1 sm:gap-1.5 pl-0.5 sm:pl-1">
            <Button variant="primary" onClick={handleSave} disabled={!workflowName.trim() || nodes.length === 0}>
              <Save className="w-4 h-4" />
              <span className="hidden sm:inline ml-1">{t("common.save")}</span>
            </Button>
            <Button variant="ghost" onClick={handleSaveAsTemplate} disabled={!selectedWorkflow?.id}
              title={t("canvas.save_as_template")}>
              <BookCopy className="w-4 h-4 mr-1" />
              <span className="hidden sm:inline">{t("canvas.save_as_template")}</span>
            </Button>
            <Button variant="ghost" onClick={() => setShowScheduleModal(true)} disabled={!selectedWorkflow?.id}
              title={t("nav.scheduler")}>
              <Calendar className="w-4 h-4 mr-1" />
              <span className="hidden sm:inline">{t("nav.scheduler")}</span>
            </Button>
            <Button variant="secondary" onClick={() => handleRunClick("dry")}
              disabled={(!selectedWorkflow && agentStepCount === 0) || !!runningWorkflowId || isDryRunning}
              title={t("canvas.dry_run_hint")}>
              {isDryRunning ? <Loader2 className="w-4 h-4 animate-spin" /> : <FlaskConical className="w-4 h-4" />}
              <span className="hidden sm:inline ml-1">Dry Run</span>
            </Button>
            <Button variant="primary" onClick={() => handleRunClick()}
              disabled={(!selectedWorkflow && agentStepCount === 0) || !!runningWorkflowId || isDryRunning}>
              {runningWorkflowId ? <Loader2 className="w-4 h-4 animate-spin" /> : <Play className="w-4 h-4" />}
              <span className="hidden sm:inline ml-1">{t("workflows.run_workflow")}</span>
            </Button>
          </div>
        </div>
      </header>

      {errorMsg && (
        <div className="mx-1 mb-2 px-4 py-2 rounded-lg bg-error/10 border border-error/30 text-error text-sm font-medium flex items-center justify-between">
          <span>{errorMsg}</span>
          <button onClick={() => setErrorMsg(null)} className="ml-2 text-error/60 hover:text-error">&times;</button>
        </div>
      )}

      <div className="flex flex-1 overflow-hidden rounded-2xl border border-border-subtle bg-surface">
        {showWorkflowPanel && (
          <WorkflowList workflows={workflows} selectedId={selectedWorkflow?.id || null}
            onSelect={handleSelectWorkflow} onDelete={handleDeleteConfirmed} onRun={handleRunClick}
            isRunning={runningWorkflowId} t={t} />
        )}

        {/* Node library (collapsible) */}
        <div className={`border-r border-border-subtle bg-surface overflow-y-auto transition-all duration-200 hidden sm:block ${sidebarCollapsed ? "w-10 px-1 py-2" : "w-52 p-3 space-y-4"
          }`}>
          <button onClick={() => setSidebarCollapsed(!sidebarCollapsed)}
            className="w-full flex items-center justify-center p-1.5 rounded-lg hover:bg-main transition-colors mb-1">
            {sidebarCollapsed
              ? <ChevronRight className="w-3.5 h-3.5 text-text-dim" />
              : <ChevronDown className="w-3.5 h-3.5 text-text-dim" />}
          </button>
          {!sidebarCollapsed && (
            <>
              <h3 className="text-[10px] font-black uppercase tracking-wider text-text-dim/50">{t("canvas.node_library")}</h3>
              {[
                { label: t("canvas.triggers"), items: NODE_TYPES.slice(0, 5) },
                { label: t("canvas.logic"), items: NODE_TYPES.slice(5, 10) },
                { label: t("canvas.actions"), items: NODE_TYPES.slice(10) },
              ].map(group => (
                <div key={group.label}>
                  <p className="text-[9px] font-bold uppercase tracking-widest text-text-dim/40 mb-2">{group.label}</p>
                  <div className="grid gap-1.5">
                    {group.items.map(n => (
                      <button key={n.type} onClick={() => addNode(n.type)}
                        className="flex items-center gap-2.5 px-2.5 py-2 rounded-xl bg-surface hover:bg-main border border-transparent hover:border-border-subtle hover:shadow-sm transition-colors text-left group">
                        <div className="w-7 h-7 rounded-lg flex items-center justify-center text-sm shrink-0 transition-transform group-hover:scale-110"
                          style={{ backgroundColor: `${n.color}15`, color: n.color }}>
                          {n.icon}
                        </div>
                        <span className="text-[11px] font-semibold text-text truncate">{t(n.labelKey)}</span>
                      </button>
                    ))}
                  </div>
                </div>
              ))}
            </>
          )}
        </div>

        {/* Canvas */}
        <main className="flex-1 relative">
          <div className="absolute top-3 left-3 right-3 z-10 flex gap-2">
            <input type="text" value={workflowName} onChange={(e) => setWorkflowName(e.target.value)}
              placeholder={t("workflows.workflow_name")}
              className="flex-1 max-w-xs rounded-xl border border-border-subtle bg-surface px-3 py-2 text-sm font-bold focus:border-primary focus:ring-2 focus:ring-primary/20 outline-none shadow-sm" />
            <input type="text" value={workflowDescription} onChange={(e) => setWorkflowDescription(e.target.value)}
              placeholder={t("workflows.description")}
              className="flex-1 max-w-sm rounded-xl border border-border-subtle bg-surface px-3 py-2 text-sm text-text-dim focus:border-primary focus:ring-2 focus:ring-primary/20 outline-none shadow-sm" />
          </div>

          {/* Node configuration panel */}
          {editingNode && !showRunInput && (
            <NodeConfigPanel node={{
              ...editingNode,
              _siblingNodes: nodes
                .filter(n => n.id !== editingNode.id && AGENT_NODE_TYPES_SET.has((n.data as any).nodeType))
                .map(n => ({ id: n.id, label: (n.data as any).label || n.id })),
            } as any} agents={agents}
              onUpdate={handleNodeUpdate} onClose={() => setEditingNode(null)}
              onDelete={(id) => { setNodes(nds => nds.filter(n => n.id !== id)); setEditingNode(null); }}
              t={t} />
          )}

          {/* Run / Dry-run input dialog */}
          {showRunInput && (
            <div className="absolute top-3 right-3 z-20 w-80 rounded-xl border border-border-subtle bg-surface shadow-2xl overflow-hidden">
              <div className={`flex items-center justify-between px-3 py-2 border-b border-border-subtle ${showRunInput === "dry" ? "bg-primary/10" : "bg-success/10"}`}>
                <div className="flex items-center gap-2">
                  {showRunInput === "dry"
                    ? <FlaskConical className="w-3.5 h-3.5 text-primary" />
                    : <Play className="w-3.5 h-3.5 text-success" />}
                  <span className={`text-xs font-bold ${showRunInput === "dry" ? "text-primary" : "text-success"}`}>
                    {showRunInput === "dry" ? "Dry Run" : t("canvas.run_input_title")}
                  </span>
                </div>
                <button onClick={() => setShowRunInput(false)} className="p-1 rounded hover:bg-main"><X className="w-3.5 h-3.5" /></button>
              </div>
              <div className="p-3 space-y-3">
                <p className="text-[10px] text-text-dim">
                  {showRunInput === "dry"
                    ? t("canvas.dry_run_desc")
                    : t("canvas.run_input_hint")}
                </p>
                <textarea value={runInput} onChange={e => setRunInput(e.target.value)}
                  placeholder={t("canvas.run_input_placeholder")}
                  rows={4} autoFocus
                  className="w-full rounded-lg border border-border-subtle bg-main px-3 py-2 text-xs outline-none focus:border-primary resize-none"
                  onKeyDown={e => { if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) showRunInput === "dry" ? handleDryRun() : handleRunConfirm(); }}
                />
                <div className="flex gap-2">
                  {showRunInput === "dry" ? (
                    <Button variant="primary" size="sm" className="flex-1" onClick={() => handleDryRun()}
                      disabled={isDryRunning}>
                      {isDryRunning ? <Loader2 className="w-3.5 h-3.5 animate-spin mr-1" /> : <FlaskConical className="w-3.5 h-3.5 mr-1" />}
                      Validate
                    </Button>
                  ) : (
                    <>
                      <Button variant="primary" size="sm" className="flex-1" onClick={() => handleRunConfirm()}
                        disabled={!!runningWorkflowId}>
                        <Play className="w-3.5 h-3.5 mr-1" />
                        {t("canvas.run_now")}
                      </Button>
                      <Button variant="ghost" size="sm" onClick={() => handleDryRun()}
                        disabled={isDryRunning || !!runningWorkflowId}
                        title={t("canvas.dry_run")}>
                        <FlaskConical className="w-3.5 h-3.5" />
                      </Button>
                    </>
                  )}
                  <Button variant="secondary" size="sm" onClick={() => setShowRunInput(false)}>
                    {t("common.cancel")}
                  </Button>
                </div>
                <p className="text-[9px] text-text-dim/50 text-center">Ctrl+Enter {t("canvas.to_run")}</p>
              </div>
            </div>
          )}

          <ReactFlow
            nodes={nodes} edges={edges}
            onNodesChange={onNodesChangeWithHistory} onEdgesChange={onEdgesChangeWithHistory}
            onConnect={(p) => { pushHistory(); onConnect(p); }}
            onNodeClick={(_, n) => { setContextMenu(null); onNodeClick(_, n); }}
            onNodesDelete={onNodesDelete}
            onSelectionChange={onSelectionChange}
            onNodeDragStart={onNodeDragStart} onNodeDrag={onNodeDrag}
            onMoveEnd={(_, vp) => setZoomLevel(Math.round(vp.zoom * 100))}
            onPaneClick={() => { setContextMenu(null); setEditingNode(null); }}
            onPaneContextMenu={(e) => {
              e.preventDefault();
              setContextMenu({ x: e.clientX, y: e.clientY });
            }}
            onNodeContextMenu={(e, node) => {
              e.preventDefault();
              setContextMenu({ x: e.clientX, y: e.clientY, nodeId: node.id });
            }}
            nodeTypes={nodeTypes} colorMode={theme}
            defaultEdgeOptions={defaultEdgeOptions}
            defaultViewport={{ x: 50, y: 80, zoom: 1 }}
            minZoom={0.1} maxZoom={2}
            snapToGrid snapGrid={[12, 12]}
            // Interaction: default drag = box select, space + drag = pan
            panOnDrag={spacePressed}
            selectionOnDrag={!spacePressed}
            selectionMode={SelectionMode.Partial}
            zoomOnScroll
            className={`bg-transparent! ${spacePressed ? "cursor-grab!" : ""}`}
            connectionLineStyle={{ stroke: edgeColorActive, strokeWidth: 2 }}
            connectionLineType={ConnectionLineType.SmoothStep}
            isValidConnection={isValidConnection}
          >
            <Background variant={BackgroundVariant.Dots} color={theme === "dark" ? "#444" : "#cbd5e1"} gap={24} size={1.5} />
            <Controls className="bg-surface! border-border-subtle! rounded-xl! shadow-lg!" />
            <div className="react-flow__panel bottom-2! left-14!">
              <span className="text-[10px] font-mono text-text-dim/50 bg-surface/80 px-1.5 py-0.5 rounded">{zoomLevel}%</span>
            </div>
            <MiniMap className="bg-surface/80! border-border-subtle! rounded-xl! shadow-lg!"
              nodeColor={(n) => {
                const cfg = NODE_TYPES.find(t => t.type === (n.data as any)?.nodeType);
                return cfg?.color || "#3b82f6";
              }}
              maskColor={theme === "dark" ? "rgba(0,0,0,0.3)" : "rgba(0,0,0,0.08)"} />
          </ReactFlow>

          {/* Empty canvas guide */}
          {nodes.length === 0 && (
            <div className="absolute inset-0 flex items-center justify-center pointer-events-none z-10">
              <div className="text-center pointer-events-auto">
                <div className="w-12 h-12 rounded-2xl bg-primary/10 flex items-center justify-center mx-auto mb-3">
                  <Plus className="w-6 h-6 text-primary" />
                </div>
                <p className="text-sm font-bold text-text-dim">{t("canvas.empty_title")}</p>
                <p className="text-xs text-text-dim/60 mt-1">{t("canvas.empty_hint")}</p>
              </div>
            </div>
          )}

          {/* Right-click context menu */}
          {contextMenu && (
            <div className="fixed z-50 rounded-xl border border-border-subtle bg-surface shadow-2xl py-1 min-w-[160px]"
              style={{ left: contextMenu.x, top: contextMenu.y }}>
              {contextMenu.nodeId ? (
                <>
                  <button className="w-full px-3 py-1.5 text-xs text-left hover:bg-main flex items-center gap-2"
                    onClick={() => { setEditingNode(nodes.find(n => n.id === contextMenu.nodeId) || null); setContextMenu(null); }}>
                    {t("canvas.ctx_edit")}
                  </button>
                  <button className="w-full px-3 py-1.5 text-xs text-left hover:bg-main flex items-center gap-2"
                    onClick={() => { copySelected(); setContextMenu(null); }}>
                    <Copy className="w-3 h-3" /> {t("canvas.ctx_copy")}
                  </button>
                  <button className="w-full px-3 py-1.5 text-xs text-left hover:bg-main flex items-center gap-2"
                    onClick={() => { duplicate(); setContextMenu(null); }}>
                    {t("canvas.ctx_duplicate")}
                  </button>
                  <div className="h-px bg-border-subtle my-1" />
                  <button className="w-full px-3 py-1.5 text-xs text-left hover:bg-error/10 text-error flex items-center gap-2"
                    onClick={() => { setNodes(nds => nds.filter(n => n.id !== contextMenu.nodeId)); setContextMenu(null); }}>
                    <Trash2 className="w-3 h-3" /> {t("common.delete")}
                  </button>
                </>
              ) : (
                <>
                  <button className="w-full px-3 py-1.5 text-xs text-left hover:bg-main flex items-center gap-2"
                    onClick={() => { addNode("agent"); setContextMenu(null); }}>
                    <Plus className="w-3 h-3" /> {t("canvas.ctx_add_agent")}
                  </button>
                  {clipboardRef.current && (
                    <button className="w-full px-3 py-1.5 text-xs text-left hover:bg-main flex items-center gap-2"
                      onClick={() => { paste(); setContextMenu(null); }}>
                      <ClipboardPaste className="w-3 h-3" /> {t("canvas.ctx_paste")}
                    </button>
                  )}
                  <button className="w-full px-3 py-1.5 text-xs text-left hover:bg-main flex items-center gap-2"
                    onClick={() => { selectAll(); setContextMenu(null); }}>
                    {t("canvas.ctx_select_all")}
                  </button>
                  <div className="h-px bg-border-subtle my-1" />
                  <button className="w-full px-3 py-1.5 text-xs text-left hover:bg-main flex items-center gap-2"
                    onClick={() => { autoLayout(); setContextMenu(null); }}>
                    <LayoutGrid className="w-3 h-3" /> {t("canvas.ctx_auto_layout")}
                  </button>
                </>
              )}
            </div>
          )}

          {/* Dry-run result panel */}
          {dryRunResult && !runResult && (
            <div className="absolute bottom-3 left-3 right-3 z-20 max-h-64 rounded-xl border border-border-subtle bg-surface shadow-2xl overflow-hidden flex flex-col">
              <div className="flex items-center justify-between px-3 py-2 bg-primary/10 border-b border-border-subtle shrink-0">
                <div className="flex items-center gap-2">
                  <FlaskConical className="w-3.5 h-3.5 text-primary" />
                  <span className="text-xs font-bold text-primary">Dry Run</span>
                  {dryRunResult.valid
                    ? <Badge variant="success">valid</Badge>
                    : <Badge variant="error">issues found</Badge>}
                </div>
                <button onClick={() => setDryRunResult(null)} className="p-1 rounded hover:bg-main"><X className="w-3.5 h-3.5" /></button>
              </div>
              <div className="overflow-y-auto flex-1 p-2 space-y-1.5">
                {dryRunResult.steps.map((step, i) => (
                  <div key={i} className="rounded-lg border border-border-subtle bg-main overflow-hidden">
                    <button
                      className="w-full flex items-center gap-2 px-3 py-2 text-left hover:bg-surface transition-colors"
                      onClick={() => setExpandedDryStep(expandedDryStep === i ? null : i)}>
                      {step.skipped
                        ? <SkipForward className="w-3 h-3 text-text-dim/40 shrink-0" />
                        : step.agent_found
                          ? <CheckCircle2 className="w-3 h-3 text-success shrink-0" />
                          : <AlertCircle className="w-3 h-3 text-warning shrink-0" />}
                      <span className="text-[10px] font-bold truncate flex-1">{step.step_name}</span>
                      {step.agent_name && <span className="text-[9px] text-text-dim/50 shrink-0">{step.agent_name}</span>}
                      {step.skipped && <span className="text-[9px] px-1 rounded bg-main border border-border-subtle text-text-dim/40 shrink-0">skip</span>}
                      {expandedDryStep === i
                        ? <ChevronUp className="w-3 h-3 text-text-dim/30 shrink-0" />
                        : <ChevronDown className="w-3 h-3 text-text-dim/30 shrink-0" />}
                    </button>
                    {expandedDryStep === i && (
                      <div className="px-3 pb-3 space-y-1.5 border-t border-border-subtle">
                        {!step.agent_found && <p className="text-[10px] text-warning mt-2">Agent not found</p>}
                        {step.skip_reason && <p className="text-[10px] text-text-dim mt-2">{step.skip_reason}</p>}
                        <p className="text-[9px] font-bold text-text-dim/50 mt-2">Resolved prompt:</p>
                        <pre className="text-[10px] text-text whitespace-pre-wrap max-h-20 overflow-y-auto bg-surface rounded-lg p-2">
                          {step.resolved_prompt || "(empty)"}
                        </pre>
                      </div>
                    )}
                  </div>
                ))}
              </div>
            </div>
          )}

          {/* Run result panel */}
          {runResult && (
            <div className="absolute bottom-3 left-3 right-3 z-20 max-h-64 rounded-xl border border-border-subtle bg-surface shadow-2xl overflow-hidden flex flex-col">
              <div className="flex items-center justify-between px-3 py-2 bg-success/10 border-b border-border-subtle shrink-0">
                <div className="flex items-center gap-2">
                  <span className="text-xs font-bold text-success">{t("canvas.run_result")}</span>
                  <Badge variant="success">{runResult.status}</Badge>
                  {runResult.run_id && <span className="text-[9px] text-text-dim font-mono">{truncateId(runResult.run_id)}</span>}
                </div>
                <button onClick={() => setRunResult(null)} className="p-1 rounded hover:bg-main"><X className="w-3.5 h-3.5" /></button>
              </div>
              <div className="overflow-y-auto flex-1">
                <pre className="px-3 py-2 text-xs text-text whitespace-pre-wrap">{runResult.output}</pre>
                {/* Step-level I/O */}
                {runResult.step_results && runResult.step_results.length > 0 && (
                  <div className="px-3 pb-3 space-y-1.5 border-t border-border-subtle">
                    <p className="text-[9px] font-bold text-text-dim/50 pt-2">Step details</p>
                    {runResult.step_results.map((s, i) => (
                      <div key={i} className="rounded-lg border border-border-subtle bg-main overflow-hidden">
                        <button
                          className="w-full flex items-center gap-2 px-3 py-2 text-left hover:bg-surface transition-colors"
                          onClick={() => setExpandedRunStep(expandedRunStep === i ? null : i)}>
                          <CheckCircle2 className="w-3 h-3 text-success shrink-0" />
                          <span className="text-[10px] font-bold truncate flex-1">{s.step_name}</span>
                          <span className="text-[9px] text-text-dim/50 shrink-0">{s.duration_ms}ms</span>
                          {expandedRunStep === i
                            ? <ChevronUp className="w-3 h-3 text-text-dim/30 shrink-0" />
                            : <ChevronDown className="w-3 h-3 text-text-dim/30 shrink-0" />}
                        </button>
                        {expandedRunStep === i && (
                          <div className="px-3 pb-3 space-y-2 border-t border-border-subtle">
                            <div>
                              <p className="text-[9px] font-bold text-text-dim/50 mt-2">Prompt sent:</p>
                              <pre className="text-[10px] text-text whitespace-pre-wrap max-h-20 overflow-y-auto bg-surface rounded-lg p-2 mt-1">
                                {s.prompt || "(empty)"}
                              </pre>
                            </div>
                            <div>
                              <p className="text-[9px] font-bold text-text-dim/50">Output:</p>
                              <pre className="text-[10px] text-text whitespace-pre-wrap max-h-20 overflow-y-auto bg-surface rounded-lg p-2 mt-1">
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
            </div>
          )}
        </main>
      </div>

      {/* Template browser */}
      {showTemplateBrowser && (
        <TemplateBrowser
          onInstantiate={handleTemplateInstantiate}
          onClose={() => setShowTemplateBrowser(false)}
          t={t}
        />
      )}

      {/* Toast */}
      {toast && (
        <div className="fixed bottom-6 left-1/2 -translate-x-1/2 z-50 px-4 py-2 rounded-xl bg-text text-surface text-xs font-bold shadow-lg transition-colors">
          <Check className="w-3.5 h-3.5 inline mr-1.5" />{toast}
        </div>
      )}

      {/* Shortcut help panel */}
      <AnimatePresence>
        {showHelp && (
          <motion.div
            className="fixed inset-0 z-50 flex items-end sm:items-center justify-center bg-black/30 backdrop-blur-sm p-0 sm:p-4"
            onClick={() => setShowHelp(false)}
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.18, ease: APPLE_EASE }}
          >
          <motion.div
            role="dialog"
            aria-modal="true"
            aria-labelledby="canvas-shortcuts-dialog-title"
            className="bg-surface rounded-t-2xl sm:rounded-2xl shadow-2xl border border-border-subtle w-full sm:w-140 sm:max-w-[90vw] max-h-[85vh] sm:max-h-[80vh] overflow-y-auto"
            onClick={e => e.stopPropagation()}
            variants={fadeInScale}
            initial="initial"
            animate="animate"
            exit="exit"
          >
            <div className="flex items-center justify-between px-5 py-3 border-b border-border-subtle">
              <h3 id="canvas-shortcuts-dialog-title" className="text-sm font-bold">{t("canvas.shortcuts_title")}</h3>
              <button onClick={() => setShowHelp(false)} aria-label={t("common.close", { defaultValue: "Close dialog" })} className="p-1 rounded hover:bg-main"><X className="w-4 h-4" /></button>
            </div>
            <div className="p-5 space-y-1 text-xs">
              {[
                ["Cmd/Ctrl+Z", t("canvas.sc_undo")],
                ["Cmd/Ctrl+Shift+Z", t("canvas.sc_redo")],
                ["Cmd/Ctrl+C", t("canvas.sc_copy")],
                ["Cmd/Ctrl+V", t("canvas.sc_paste")],
                ["Cmd/Ctrl+D", t("canvas.sc_duplicate")],
                ["Cmd/Ctrl+A", t("canvas.sc_select_all")],
                ["Cmd/Ctrl+B", t("canvas.sc_group")],
                ["Shift+Cmd/Ctrl+B", t("canvas.sc_ungroup")],
                ["Cmd/Ctrl+1", t("canvas.sc_fit_view")],
                ["Cmd/Ctrl+E", t("canvas.sc_export")],
                ["Cmd/Ctrl+I", t("canvas.sc_import")],
                ["Delete", t("canvas.sc_delete")],
                ["Space + Drag", t("canvas.sc_pan")],
                ["Drag", t("canvas.sc_select")],
                ["Right Click", t("canvas.sc_context")],
                ["?", t("canvas.sc_help")],
              ].map(([key, desc]) => (
                <div key={key} className="flex items-center justify-between py-1.5 border-b border-border-subtle/30">
                  <span className="text-text-dim">{desc}</span>
                  <kbd className="px-2 py-0.5 rounded-md bg-main text-text font-mono text-[10px]">{key}</kbd>
                </div>
              ))}
            </div>
          </motion.div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* Schedule Modal */}
      {showScheduleModal && (
        <ScheduleModal
          isOpen={true}
          title={t("nav.scheduler")}
          subtitle={workflowName}
          initialCron="0 9 * * *"
          onSave={async (cron, tz) => {
            if (!selectedWorkflow?.id) return;
            try {
              await createScheduleMutation.mutateAsync({ name: `${workflowName || "workflow"} schedule`, cron, tz, workflow_id: selectedWorkflow.id, enabled: true });
              setShowScheduleModal(false);
              showToast(t("canvas.scheduled", { defaultValue: "Schedule created" }));
            } catch (e: any) { showError(e?.message || String(e)); }
          }}
          onClose={() => setShowScheduleModal(false)}
        />
      )}
    </div>
  );
}
