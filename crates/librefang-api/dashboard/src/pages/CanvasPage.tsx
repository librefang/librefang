import { useCallback, useState, useEffect, useMemo, useRef } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate, useSearch } from "@tanstack/react-router";
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
import { listAgents, listWorkflows, createWorkflow, updateWorkflow, deleteWorkflow, runWorkflow, listWorkflowTemplates, instantiateTemplate, type AgentItem, type WorkflowItem, type WorkflowTemplate as ApiWorkflowTemplate } from "../api";
import { listAgents, listWorkflows, createWorkflow, updateWorkflow, deleteWorkflow, runWorkflow, saveWorkflowAsTemplate, type AgentItem, type WorkflowItem } from "../api";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { useUIStore } from "../lib/store";
import {
  Play, Save, Trash2, Plus, FolderOpen, Loader2,
  Maximize2, Minimize2, ArrowLeft, X, Group, ChevronDown, ChevronRight,
  Copy, ClipboardPaste, LayoutGrid,
  Download, Upload, HelpCircle, Scan, Check, LayoutTemplate, Search, Tag
  Download, Upload, HelpCircle, Scan, Check, BookCopy
} from "lucide-react";

// 节点类型配置 — n8n 风格配色
const NODE_TYPES = [
  // 触发器（视觉标记）
  { type: "start", labelKey: "canvas.node_types.start", color: "#10b981", bg: "#ecfdf5", icon: "S", descKey: "canvas.node_types.start_desc" },
  { type: "end", labelKey: "canvas.node_types.end", color: "#ef4444", bg: "#fef2f2", icon: "E", descKey: "canvas.node_types.end_desc" },
  { type: "schedule", labelKey: "canvas.node_types.schedule", color: "#f59e0b", bg: "#fffbeb", icon: "C", descKey: "canvas.node_types.schedule_desc" },
  { type: "webhook", labelKey: "canvas.node_types.webhook", color: "#6366f1", bg: "#eef2ff", icon: "W", descKey: "canvas.node_types.webhook_desc" },
  { type: "channel", labelKey: "canvas.node_types.channel", color: "#8b5cf6", bg: "#f5f3ff", icon: "M", descKey: "canvas.node_types.channel_desc" },
  // 逻辑控制
  { type: "condition", labelKey: "canvas.node_types.condition", color: "#f59e0b", bg: "#fffbeb", icon: "?", descKey: "canvas.node_types.condition_desc" },
  { type: "loop", labelKey: "canvas.node_types.loop", color: "#8b5cf6", bg: "#f5f3ff", icon: "L", descKey: "canvas.node_types.loop_desc" },
  { type: "parallel", labelKey: "canvas.node_types.parallel", color: "#f59e0b", bg: "#fffbeb", icon: "P", descKey: "canvas.node_types.parallel_desc" },
  { type: "collect", labelKey: "canvas.node_types.collect", color: "#10b981", bg: "#ecfdf5", icon: "C", descKey: "canvas.node_types.collect_desc" },
  { type: "wait", labelKey: "canvas.node_types.wait", color: "#6b7280", bg: "#f9fafb", icon: "T", descKey: "canvas.node_types.wait_desc" },
  // 动作
  { type: "respond", labelKey: "canvas.node_types.respond", color: "#10b981", bg: "#ecfdf5", icon: "R", descKey: "canvas.node_types.respond_desc" },
  { type: "agent", labelKey: "canvas.node_types.agent", color: "#3b82f6", bg: "#eff6ff", icon: "A", descKey: "canvas.node_types.agent_desc" },
];

// 需要绑定 agent 的节点类型
const AGENT_NODE_TYPES_SET = new Set(["agent", "channel", "respond", "condition", "loop", "parallel", "collect"]);

// 自定义节点组件 — n8n 风格
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
      className={`rounded-2xl bg-surface min-w-[140px] max-w-[200px] overflow-hidden relative transition-all duration-200 border border-border-subtle hover:scale-[1.02] hover:shadow-lg ${
        runState === "running" ? "animate-pulse" : ""
      }`}
      style={{ border: `2px ${missingAgent ? "dashed" : "solid"} ${borderColor}`, ...ringStyle }}
    >
      {/* Target Handle */}
      {!isStart && (
        <Handle type="target" position={Position.Top}
          className="!w-3 !h-3 !rounded-full !border-2 !border-surface"
          style={{ backgroundColor: config.color }} />
      )}

      {/* Header: icon circle + label */}
      <div className="flex items-center gap-2.5 px-3 py-2.5" style={{ backgroundColor: `${config.color}15` }}>
        <div
          className="w-8 h-8 rounded-xl flex items-center justify-center text-white text-sm font-bold shrink-0 transition-all"
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

      {/* Source Handle */}
      {!isEnd && (
        <Handle type="source" position={Position.Bottom}
          className="!w-3 !h-3 !rounded-full !border-2 !border-surface"
          style={{ backgroundColor: config.color }} />
      )}
    </div>
  );
}

// 分组节点组件
function GroupNodeComponent({ data, id }: { data: any; id: string }) {
  const expanded = data._expanded !== false;
  return (
    <div
      className={`rounded-2xl border-2 border-dashed transition-all ${
        expanded ? "border-brand/30 bg-brand/5" : "border-brand bg-surface shadow-lg"
      }`}
      style={expanded
        ? { width: "100%", height: "100%", pointerEvents: "none" }
        : { width: 180 }}
    >
      <Handle type="target" position={Position.Top} className="!w-3 !h-3 !rounded-full !bg-brand !border-2 !border-surface" />
      <div
        className="flex items-center gap-2 px-3 py-2 bg-brand/10 rounded-t-xl cursor-pointer relative z-10"
        style={{ pointerEvents: "auto" }}
      >
        <div className="flex items-center gap-2 flex-1 min-w-0" onClick={(e) => { e.stopPropagation(); data._onToggle?.(id); }}>
          {expanded
            ? <ChevronDown className="w-3.5 h-3.5 text-brand shrink-0" />
            : <ChevronRight className="w-3.5 h-3.5 text-brand shrink-0" />}
          <Group className="w-3.5 h-3.5 text-brand shrink-0" />
          <span className="text-xs font-bold text-brand truncate">{data.label || "Group"}</span>
          {!expanded && data._childCount > 0 && (
            <span className="text-[9px] text-brand/50">{data._childCount}</span>
          )}
        </div>
        {/* 解散分组（保留子节点） */}
        <button onClick={(e) => { e.stopPropagation(); data._onUngroup?.(id); }}
          title="Ungroup"
          className="p-0.5 rounded hover:bg-brand/20 text-brand/50 hover:text-brand shrink-0">
          <X className="w-3 h-3" />
        </button>
        {/* 删除分组+子节点 */}
        <button onClick={(e) => { e.stopPropagation(); data._onDeleteGroup?.(id); }}
          title="Delete group and children"
          className="p-0.5 rounded hover:bg-error/20 text-text-dim/30 hover:text-error shrink-0">
          <Trash2 className="w-3 h-3" />
        </button>
      </div>
      {!expanded && (
        <div className="px-3 py-2">
          <p className="text-[9px] text-text-dim">Click to expand</p>
        </div>
      )}
      <Handle type="source" position={Position.Bottom} className="!w-3 !h-3 !rounded-full !bg-brand !border-2 !border-surface" />
    </div>
  );
}

// 工作流列表侧边栏
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
              className={`p-3 rounded-xl border cursor-pointer transition-all ${
                selectedId === w.id ? "border-brand bg-brand/5" : "border-border-subtle hover:border-brand/50 bg-surface"
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

// 模板浏览器
function TemplateBrowser({
  onInstantiate, onClose, t
}: {
  onInstantiate: (workflowId: string) => void;
  onClose: () => void;
  t: (key: string) => string;
}) {
  const [templates, setTemplates] = useState<ApiWorkflowTemplate[]>([]);
  const [loading, setLoading] = useState(true);
  const [searchQuery, setSearchQuery] = useState("");
  const [selectedTemplate, setSelectedTemplate] = useState<ApiWorkflowTemplate | null>(null);
  const [paramValues, setParamValues] = useState<Record<string, unknown>>({});
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setLoading(true);
    listWorkflowTemplates(searchQuery || undefined)
      .then(setTemplates)
      .catch(() => setTemplates([]))
      .finally(() => setLoading(false));
  }, [searchQuery]);

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
    setSubmitting(true);
    setError(null);
    try {
      const resp = await instantiateTemplate(selectedTemplate.id, paramValues);
      const workflowId = (resp as any).workflow_id || (resp as any).id || "";
      onInstantiate(workflowId);
    } catch (e: any) {
      setError(e?.message || t("canvas.template_instantiate_error"));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 backdrop-blur-xl backdrop-saturate-150" onClick={onClose}>
      <div className="bg-surface rounded-2xl shadow-2xl border border-border-subtle w-[640px] max-w-[90vw] max-h-[80vh] flex flex-col animate-fade-in-scale" onClick={e => e.stopPropagation()}>
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-3 border-b border-border-subtle shrink-0">
          <div className="flex items-center gap-2">
            <LayoutTemplate className="w-4 h-4 text-brand" />
            <h3 className="text-sm font-bold">{t("canvas.browse_templates")}</h3>
          </div>
          <button onClick={onClose} className="p-1 rounded hover:bg-main"><X className="w-4 h-4" /></button>
        </div>

        {selectedTemplate ? (
          /* Template detail + params form */
          <div className="flex-1 overflow-y-auto p-5 space-y-4">
            <button onClick={() => setSelectedTemplate(null)} className="text-xs text-brand hover:underline flex items-center gap-1">
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
                      className="mt-1 w-full rounded-lg border border-border-subtle bg-main px-2 py-1.5 text-xs outline-none focus:border-brand"
                      placeholder={p.description || p.name}
                    />
                  </div>
                ))}
              </div>
            )}

            {error && (
              <div className="px-3 py-2 rounded-lg bg-error/10 border border-error/30 text-error text-xs">{error}</div>
            )}

            <Button variant="primary" className="w-full" onClick={handleInstantiate} disabled={submitting}>
              {submitting ? <Loader2 className="w-4 h-4 mr-1 animate-spin" /> : <Play className="w-4 h-4 mr-1" />}
              {t("canvas.use_template")}
            </Button>
          </div>
        ) : (
          /* Template list */
          <div className="flex-1 overflow-y-auto">
            {/* Search */}
            <div className="px-5 pt-4 pb-2">
              <div className="relative">
                <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-text-dim/40" />
                <input
                  type="text" value={searchQuery}
                  onChange={e => setSearchQuery(e.target.value)}
                  placeholder={t("canvas.template_search")}
                  className="w-full rounded-xl border border-border-subtle bg-main pl-8 pr-3 py-2 text-xs outline-none focus:border-brand"
                />
              </div>
            </div>

            {loading ? (
              <div className="flex items-center justify-center py-12">
                <Loader2 className="w-5 h-5 animate-spin text-brand" />
              </div>
            ) : templates.length === 0 ? (
              <div className="text-center py-12">
                <p className="text-xs text-text-dim">{t("canvas.no_templates")}</p>
              </div>
            ) : (
              <div className="px-5 pb-4 grid gap-2">
                {templates.map(tmpl => (
                  <button
                    key={tmpl.id}
                    onClick={() => handleSelect(tmpl)}
                    className="p-3 rounded-xl border border-border-subtle bg-surface hover:border-brand/50 hover:shadow-sm transition-all text-left"
                  >
                    <div className="flex items-center justify-between">
                      <span className="text-sm font-bold truncate">{tmpl.name}</span>
                      <div className="flex gap-1 shrink-0">
                        {tmpl.category && (
                          <span className="text-[9px] font-bold px-1.5 py-0.5 rounded-full bg-brand/10 text-brand">{tmpl.category}</span>
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
      </div>
    </div>
  );
}

// 节点配置面板
const inputClass = "mt-1 w-full rounded-lg border border-border-subtle bg-main px-2 py-1.5 text-xs outline-none focus:border-brand";
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
        {/* 基础信息 */}
        <div>
          <label className={labelClass}>{t("canvas.node_label")}</label>
          <input type="text" value={label} onChange={e => setLabel(e.target.value)} className={inputClass} />
        </div>
        <div>
          <label className={labelClass}>{t("canvas.node_desc")}</label>
          <input type="text" value={description} onChange={e => setDescription(e.target.value)} className={inputClass} />
        </div>

        {/* Agent 绑定 */}
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

        {/* 执行模式 */}
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

        {/* Conditional 专属字段 */}
        {hasAgent && mode === "conditional" && (
          <div>
            <label className={labelClass}>{t("canvas.condition_text")}</label>
            <input type="text" value={condition} onChange={e => setCondition(e.target.value)}
              placeholder={t("canvas.condition_placeholder")} className={inputClass} />
          </div>
        )}

        {/* Loop 专属字段 */}
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

        {/* 错误处理 */}
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

        {/* 高级选项 */}
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
  const { t: routeTimestamp } = useSearch({ from: "/canvas" });
  const { theme } = useUIStore();
  const { fitView } = useReactFlow();
  const [nodes, setNodes, onNodesChange] = useNodesState<Node>([]);
  const [edges, setEdges, onEdgesChange] = useEdgesState<Edge>([]);
  const [agents, setAgents] = useState<AgentItem[]>([]);
  const [workflows, setWorkflows] = useState<WorkflowItem[]>([]);
  const [selectedWorkflow, setSelectedWorkflow] = useState<WorkflowItem | null>(null);
  const [workflowName, setWorkflowName] = useState("");
  const [workflowDescription, setWorkflowDescription] = useState("");
  const [showWorkflowPanel, setShowWorkflowPanel] = useState(false);
  const [isFullscreen, setIsFullscreen] = useState(true);
  const [runningWorkflowId, setRunningWorkflowId] = useState<string | null>(null);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [editingNode, setEditingNode] = useState<Node | null>(null);
  const [runResult, setRunResult] = useState<{ output: string; status: string; run_id: string } | null>(null);
  const [showRunInput, setShowRunInput] = useState(false);
  const [runInput, setRunInput] = useState("");

  const [selectedNodeIds, setSelectedNodeIds] = useState<Set<string>>(new Set());
  const [spacePressed, setSpacePressed] = useState(false);
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number; nodeId?: string } | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  const [showHelp, setShowHelp] = useState(false);
  const [showTemplateBrowser, setShowTemplateBrowser] = useState(false);

  // 撤销/重做历史
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
    // 先保存当前状态到历史末尾（如果还没保存）
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

  // 记录关键操作前的快照
  const onNodesChangeWithHistory = useCallback((changes: any) => {
    // 拖拽结束/删除时记录
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

  // 复制选中节点
  const copySelected = useCallback(() => {
    const selNodes = nodes.filter(n => selectedNodeIds.has(n.id));
    if (selNodes.length === 0) return;
    const selIds = new Set(selNodes.map(n => n.id));
    const selEdges = edges.filter(e => selIds.has(e.source) && selIds.has(e.target));
    clipboardRef.current = { nodes: JSON.parse(JSON.stringify(selNodes)), edges: JSON.parse(JSON.stringify(selEdges)) };
  }, [nodes, edges, selectedNodeIds]);

  // 粘贴
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
    // 取消选择旧节点
    setNodes(nds => [...nds.map(n => ({ ...n, selected: false })), ...newNodes]);
    setEdges(eds => [...eds, ...newEdges]);
  }, [pushHistory, setNodes, setEdges]);

  // 复制选中节点（Cmd+D）
  const duplicate = useCallback(() => {
    copySelected();
    paste();
  }, [copySelected, paste]);

  const showError = useCallback((msg: string) => {
    setErrorMsg(msg);
    setTimeout(() => setErrorMsg(null), 5000);
  }, []);

  // 重新计算分组边框以包含所有子节点（提前声明，autoLayout 需要）
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

  // 全选
  const selectAll = useCallback(() => {
    setNodes(nds => nds.map(n => ({ ...n, selected: true })));
  }, [setNodes]);

  // 自动布局（简单的纵向排列）
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
    // 重新计算分组边框
    groupNodes.forEach(g => {
      setNodes(nds => recalcGroupBounds(nds, g.id));
    });
  }, [nodes, pushHistory, setNodes, recalcGroupBounds]);

  // Toast 提示
  const showToast = useCallback((msg: string) => {
    setToast(msg);
    setTimeout(() => setToast(null), 2000);
  }, []);

  // 导出工作流 JSON
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

  // 导入工作流 JSON
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

  // 连线验证：阻止 source→source 或 target→target
  const isValidConnection = useCallback((connection: Edge | Connection) => {
    return connection.source !== connection.target;
  }, []);

  // 快捷键 refs
  const createGroupRef = useRef<() => void>(() => {});
  const ungroupRef = useRef<(id: string) => void>(() => {});

  // Stable refs for group callbacks — prevents nodeTypes from changing on every render
  const toggleGroupRef = useRef<(id: string) => void>(() => {});
  const ungroupNodesRef = useRef<(id: string) => void>(() => {});
  const deleteGroupAndChildrenRef = useRef<(id: string) => void>(() => {});
  const tRef = useRef(t);

  useEffect(() => {
    const isInput = () => {
      const tag = document.activeElement?.tagName;
      return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT";
    };
    const down = (e: KeyboardEvent) => {
      if (isInput()) return; // 不在输入框中才处理
      const mod = e.metaKey || e.ctrlKey;
      // 空格：平移模式
      if (e.code === "Space" && !e.repeat) { e.preventDefault(); setSpacePressed(true); }
      // Cmd+Z：撤销
      if (e.code === "KeyZ" && mod && !e.shiftKey) { e.preventDefault(); undo(); }
      // Cmd+Shift+Z：重做
      if (e.code === "KeyZ" && mod && e.shiftKey) { e.preventDefault(); redo(); }
      // Cmd+C：复制
      if (e.code === "KeyC" && mod) { e.preventDefault(); copySelected(); }
      // Cmd+V：粘贴
      if (e.code === "KeyV" && mod) { e.preventDefault(); paste(); }
      // Cmd+D：复制
      if (e.code === "KeyD" && mod) { e.preventDefault(); duplicate(); }
      // Cmd+A：全选
      if (e.code === "KeyA" && mod) { e.preventDefault(); selectAll(); }
      // Cmd+B：创建分组
      if (e.code === "KeyB" && mod && !e.shiftKey) { e.preventDefault(); createGroupRef.current(); }
      // Shift+Cmd+B：解散分组
      if (e.code === "KeyB" && mod && e.shiftKey) {
        e.preventDefault();
        const groupNode = nodes.find(n => selectedNodeIds.has(n.id) && n.type === "groupNode");
        if (groupNode) ungroupRef.current(groupNode.id);
      }
      // Cmd+1：适配视口
      if (e.code === "Digit1" && mod) { e.preventDefault(); fitView({ padding: 0.2, duration: 300 }); }
      // Cmd+E：导出
      if (e.code === "KeyE" && mod) { e.preventDefault(); exportWorkflow(); }
      // Cmd+I：导入
      if (e.code === "KeyI" && mod) { e.preventDefault(); importWorkflow(); }
      // ?：快捷键帮助
      if (e.code === "Slash" && e.shiftKey && !mod) { e.preventDefault(); setShowHelp(h => !h); }
    };
    const up = (e: KeyboardEvent) => { if (e.code === "Space") setSpacePressed(false); };
    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);
    return () => { window.removeEventListener("keydown", down); window.removeEventListener("keyup", up); };
  }, [nodes, selectedNodeIds, undo, redo, copySelected, paste, duplicate, selectAll, fitView, exportWorkflow, importWorkflow]);

  // 折叠/展开分组
  const toggleGroup = useCallback((groupId: string) => {
    setNodes(nds => {
      const groupNode = nds.find(n => n.id === groupId);
      if (!groupNode) return nds;
      const gd = groupNode.data as any;
      const isExpanded = gd._expanded !== false;
      const willCollapse = isExpanded;
      const childIds = new Set<string>(gd._childIds || []);

      // 折叠时记录当前尺寸，展开时恢复
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

    // 处理边
    setEdges(eds => {
      const groupNode = nodes.find(n => n.id === groupId);
      const gd = groupNode?.data as any;
      const isExpanded = gd?._expanded !== false;
      const willCollapse = isExpanded;
      const childIds = new Set<string>(gd?._childIds || []);

      return eds.map(e => {
        const srcChild = childIds.has(e.source);
        const tgtChild = childIds.has(e.target);

        // 内部边：折叠时隐藏
        if (srcChild && tgtChild) {
          return { ...e, hidden: willCollapse };
        }
        if (willCollapse) {
          // 外部边：重定向到 group 节点，保存原始端点
          if (srcChild) return { ...e, data: { ...e.data, _origSource: e.source }, source: groupId };
          if (tgtChild) return { ...e, data: { ...e.data, _origTarget: e.target }, target: groupId };
        } else {
          // 展开：恢复原始端点
          const ed = e.data as any;
          if (ed?._origSource) return { ...e, source: ed._origSource, data: { ...e.data, _origSource: undefined }, hidden: false };
          if (ed?._origTarget) return { ...e, target: ed._origTarget, data: { ...e.data, _origTarget: undefined }, hidden: false };
          // 恢复内部边可见
          if (srcChild && tgtChild) return { ...e, hidden: false };
        }
        return e;
      });
    });
  }, [nodes, setNodes, setEdges]);

  // 解散分组：删掉 group 节点，子节点保留并清除 _groupId
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
    // 恢复被重定向的边
    setEdges(eds => eds.map(e => {
      const ed = e.data as any;
      if (ed?._origSource) return { ...e, source: ed._origSource, data: { ...e.data, _origSource: undefined }, hidden: false };
      if (ed?._origTarget) return { ...e, target: ed._origTarget, data: { ...e.data, _origTarget: undefined }, hidden: false };
      return { ...e, hidden: false };
    }));
  }, [setNodes, setEdges]);

  // 删除分组 + 所有子节点
  const deleteGroupAndChildren = useCallback((groupId: string) => {
    setNodes(nds => {
      const group = nds.find(n => n.id === groupId);
      const childIds = new Set<string>((group?.data as any)?._childIds || []);
      childIds.add(groupId);
      return nds.filter(n => !childIds.has(n.id));
    });
    // 删除涉及子节点的边
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

  // 需要 agent 的节点类型（后端所有 step 都需要 agent）
  const AGENT_NODE_TYPES = AGENT_NODE_TYPES_SET;

  // 加载模板数据（传入 agents 列表以便自动分配）
  const loadTemplate = useCallback((availableAgents: AgentItem[]) => {
    const templateData = sessionStorage.getItem("workflowTemplate");
    if (templateData) {
      try {
        const { nodes: templateNodes, edges: templateEdges, name, description, workflowId } = JSON.parse(templateData);
        // 找一个可用的 agent 作为默认分配
        const defaultAgent = availableAgents.find(a => a.state === "Running") || availableAgents[0];
        // 根据界面语言决定输出语言指令
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
              // 保留已有 agent 绑定（按 ID 查名字），或自动分配默认 agent
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
        // 如果是编辑已有工作流，恢复 selectedWorkflow 以便保存时走更新逻辑
        if (workflowId) setSelectedWorkflow({ id: workflowId, name: name || "", description: description || "" } as WorkflowItem);
        sessionStorage.removeItem("workflowTemplate");
        return true;
      } catch { /* ignore */ }
    }
    return false;
  }, [t, setNodes, setEdges]);

  // 加载智能体、工作流，然后加载模板
  useEffect(() => {
    Promise.all([listAgents(), listWorkflows()])
      .then(([a, w]) => {
        setAgents(a);
        setWorkflows(w);
        // agents 就绪后再加载模板
        if (!loadTemplate(a)) {
          const savedNodes = sessionStorage.getItem("canvasNodes");
          if (savedNodes) {
            try { setNodes(JSON.parse(savedNodes)); } catch { /* ignore */ }
          }
        }
      })
      .catch(() => {});
  }, [routeTimestamp, loadTemplate]);

  // 保存节点到 sessionStorage
  useEffect(() => {
    if (nodes.length > 0) sessionStorage.setItem("canvasNodes", JSON.stringify(nodes));
  }, [nodes]);

  // nodeType → 默认 stepMode 映射
  const NODE_MODE_MAP: Record<string, string> = {
    condition: "conditional",
    loop: "loop",
    parallel: "fan_out",
    collect: "collect",
  };

  // 添加节点
  const addNode = useCallback((type: string) => {
    const config = NODE_TYPES.find(n => n.type === type) || NODE_TYPES[10];
    const defaultMode = NODE_MODE_MAP[type];
    // 用函数式更新读最新 nodes，避免闭包过期
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

  // 连线
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

  // 节点点击 → 打开配置面板
  const onNodeClick = useCallback((_: any, node: Node) => {
    setEditingNode(node);
  }, []);

  // 节点被删除时清理编辑面板
  const onNodesDelete = useCallback((deleted: Node[]) => {
    if (editingNode && deleted.some(n => n.id === editingNode.id)) {
      setEditingNode(null);
    }
  }, [editingNode]);

  // group 拖拽时带动子节点
  const groupDragStart = useRef<{ id: string; x: number; y: number } | null>(null);

  const onNodeDragStart = useCallback((_: any, node: Node) => {
    if (node.type === "groupNode") {
      groupDragStart.current = { id: node.id, x: node.position.x, y: node.position.y };
    }
  }, []);

  const onNodeDrag = useCallback((_: any, node: Node) => {
    if (node.type === "groupNode" && groupDragStart.current?.id === node.id) {
      // 拖拽分组 → 带动子节点
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
      // 拖拽子节点 → 扩展所属分组边框
      const groupId = (node.data as any)?._groupId;
      if (groupId) {
        setNodes(nds => recalcGroupBounds(nds, groupId));
      }
    }
  }, [setNodes, recalcGroupBounds]);

  // 跟踪选中的节点
  const onSelectionChange = useCallback(({ nodes: selected }: OnSelectionChangeParams) => {
    setSelectedNodeIds(new Set(selected.map(n => n.id)));
  }, []);

  // 创建分组：不改变子节点位置，只在底层加一个背景框 + 标记归属
  const createGroup = useCallback(() => {
    if (selectedNodeIds.size < 2) return;

    const selected = nodes.filter(n => selectedNodeIds.has(n.id) && n.type !== "groupNode");
    if (selected.length < 2) return;

    // 手动计算包围盒（考虑节点自身宽高，getNodesBounds 不一定准）
    const NODE_W = 200; // 节点最大宽度
    const NODE_H = 80;  // 节点估算高度
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

    // 用函数式更新：在现有节点前面插入 group，修改子节点 data 标记归属
    // 不替换整个数组，保留 ReactFlow 内部节点状态（measured 等）
    setNodes(nds => [
      groupNode,
      ...nds.map(n => childIds.includes(n.id)
        ? { ...n, data: { ...(n.data as any), _groupId: groupId } }
        : n
      ),
    ]);
    setSelectedNodeIds(new Set());
  }, [selectedNodeIds, nodes, setNodes, t]);

  // 同步快捷键 refs
  createGroupRef.current = createGroup;
  ungroupRef.current = ungroupNodes;

  // Sync stable refs for group callbacks used by nodeTypes
  toggleGroupRef.current = toggleGroup;
  ungroupNodesRef.current = ungroupNodes;
  deleteGroupAndChildrenRef.current = deleteGroupAndChildren;
  tRef.current = t;

  // 更新节点数据
  const handleNodeUpdate = useCallback((id: string, newData: any) => {
    setNodes(nds => nds.map(n => n.id === id ? { ...n, data: newData } : n));
  }, [setNodes]);

  // 从节点构建后端 steps：只有绑定了真实 agent 的节点才是 step
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
        // 执行模式
        const mode = d.stepMode || "sequential";
        if (mode === "conditional") {
          step.mode = { conditional: { condition: d.condition || "" } };
        } else if (mode === "loop") {
          step.mode = { loop: { max_iterations: d.maxIterations || 5, until: d.until || "" } };
        } else {
          step.mode = mode;
        }
        // 错误模式
        const errMode = d.errorMode || "fail";
        if (errMode === "retry") {
          step.error_mode = { retry: { max_retries: d.maxRetries || 3 } };
        } else {
          step.error_mode = errMode;
        }
        // 输出变量
        if (d.outputVar) step.output_var = d.outputVar;
        return step;
      });
  }, []);

  // 保存工作流
  const handleSave = useCallback(async () => {
    if (!workflowName.trim()) {
      showError(t("canvas.name_required"));
      return;
    }
    const steps = buildSteps(nodes);
    if (steps.length === 0) {
      showError(t("canvas.no_agent_steps"));
      return;
    }
    const layout = { nodes, edges };
    try {
      if (selectedWorkflow?.id) {
        await updateWorkflow(selectedWorkflow.id, { name: workflowName, description: workflowDescription, steps, layout });
      } else {
        await createWorkflow({ name: workflowName, description: workflowDescription, steps, layout });
      }
      const workflowsData = await listWorkflows();
      setWorkflows(workflowsData);
      if (!selectedWorkflow?.id) {
        const newest = [...workflowsData].sort((a, b) => (b.created_at ?? "").localeCompare(a.created_at ?? ""))[0];
        if (newest) setSelectedWorkflow(newest);
      }
      setErrorMsg(null);
      showToast(t("canvas.saved"));
    } catch (e: any) {
      showError(e?.message || String(e));
    }
  }, [workflowName, workflowDescription, selectedWorkflow, nodes, edges, buildSteps, t, showError, showToast]);

  // 保存为模板
  const handleSaveAsTemplate = useCallback(async () => {
    if (!selectedWorkflow?.id) {
      showError(t("canvas.save_first_to_template"));
      return;
    }
    try {
      await saveWorkflowAsTemplate(selectedWorkflow.id);
      showToast(t("canvas.saved_as_template"));
    } catch (e: any) {
      showError(e?.message || String(e));
    }
  }, [selectedWorkflow, t, showError, showToast]);

  // 点击运行 → 弹出输入框
  const handleRunClick = useCallback((id?: string) => {
    if (id) {
      // 从侧边栏直接运行已保存的工作流
      setRunInput("");
      setShowRunInput(true);
    } else if (selectedWorkflow?.id || nodes.length > 0) {
      setRunInput("");
      setShowRunInput(true);
    }
  }, [selectedWorkflow, nodes]);

  // 确认运行
  const handleRunConfirm = useCallback(async (id?: string) => {
    setShowRunInput(false);
    let workflowId = id || selectedWorkflow?.id;

    // 没有已保存的工作流 → 先保存
    if (!workflowId && nodes.length > 0) {
      const steps = buildSteps(nodes);
      if (steps.length === 0) {
        showError(t("canvas.no_agent_steps"));
        return;
      }
      const name = workflowName.trim() || t("workflows.untitled_workflow");
      const layout = { nodes, edges };
      try {
        await createWorkflow({ name, description: workflowDescription, steps, layout });
        const updatedList = await listWorkflows();
        setWorkflows(updatedList);
        const newest = [...updatedList].sort((a, b) => (b.created_at ?? "").localeCompare(a.created_at ?? ""))[0];
        if (newest) {
          workflowId = newest.id;
          setSelectedWorkflow(newest);
          setWorkflowName(name);
        }
      } catch (e: any) {
        showError(e?.message || String(e));
        return;
      }
    }

    if (!workflowId) return;

    setRunningWorkflowId(workflowId);
    setErrorMsg(null);
    setRunResult(null);
    // 运行时边动画
    setEdges(eds => eds.map(e => ({ ...e, animated: true })));

    // 逐步点亮节点动画
    const agentNodeIds = nodes.filter(n => (n.data as any).agentId).map(n => n.id);
    const allNodeIds = nodes.map(n => n.id);
    let stepTimer: ReturnType<typeof setInterval> | null = null;
    let currentStep = 0;

    const updateRunState = (runningId: string | null, doneIds: Set<string>) => {
      setNodes(nds => nds.map(n => ({
        ...n,
        data: {
          ...(n.data as any),
          _runState: doneIds.has(n.id) ? "done" : n.id === runningId ? "running" : undefined,
        }
      })));
    };

    // 逐步推进动画，最后一个节点保持 running 直到 API 返回
    const doneSet = new Set<string>();
    if (agentNodeIds.length > 0) {
      updateRunState(agentNodeIds[0], doneSet);
      if (agentNodeIds.length > 1) {
        stepTimer = setInterval(() => {
          if (currentStep < agentNodeIds.length - 1) {
            // 标记当前为 done，推进到下一个
            doneSet.add(agentNodeIds[currentStep]);
            currentStep++;
            updateRunState(agentNodeIds[currentStep], doneSet);
          }
          // 到最后一个节点就停止 timer，保持 running 等 API 返回
          if (currentStep >= agentNodeIds.length - 1) {
            if (stepTimer) clearInterval(stepTimer);
            stepTimer = null;
          }
        }, 20000);
      }
    }

    try {
      const resp = await runWorkflow(workflowId, runInput);
      // 完成：所有节点标记 done
      if (stepTimer) clearInterval(stepTimer);
      setNodes(nds => nds.map(n => ({
        ...n,
        data: { ...(n.data as any), _runState: allNodeIds.includes(n.id) ? "done" : undefined }
      })));
      setRunResult({
        output: (resp as any).output || (resp as any).message || JSON.stringify(resp),
        status: (resp as any).status || "completed",
        run_id: (resp as any).run_id || "",
      });
      // 3秒后清除 done 状态和边动画
      setTimeout(() => {
        setNodes(nds => nds.map(n => ({ ...n, data: { ...(n.data as any), _runState: undefined } })));
        setEdges(eds => eds.map(e => ({ ...e, animated: false })));
      }, 3000);
    } catch (e: any) {
      if (stepTimer) clearInterval(stepTimer);
      // 错误：清除所有状态和边动画
      setNodes(nds => nds.map(n => ({ ...n, data: { ...(n.data as any), _runState: undefined } })));
      setEdges(eds => eds.map(e => ({ ...e, animated: false })));
      showError(e?.message || String(e));
    } finally {
      setRunningWorkflowId(null);
    }
  }, [selectedWorkflow, nodes, edges, workflowName, workflowDescription, buildSteps, runInput, t, showError]);

  // 删除工作流
  const handleDeleteConfirmed = useCallback(async (id: string) => {
    try {
      await deleteWorkflow(id);
      setWorkflows(prev => prev.filter(w => w.id !== id));
      if (selectedWorkflow?.id === id) {
        setSelectedWorkflow(null);
        setNodes([]); setEdges([]);
        setWorkflowName(""); setWorkflowDescription("");
      }
    } catch (e) { console.error(e); }
  }, [selectedWorkflow, setNodes, setEdges]);

  // 选择已保存的工作流
  const handleSelectWorkflow = useCallback((w: WorkflowItem) => {
    setSelectedWorkflow(w);
    setWorkflowName(w.name);
    setWorkflowDescription(w.description || "");
    setEditingNode(null);

    const stepsArray = Array.isArray(w.steps) ? w.steps : [];
    const newNodes: Node[] = stepsArray.map((step: any, idx: number) => {
      const fullPrompt = step.prompt_template || step.prompt || "";
      return {
      id: `node-${idx}`,
      type: "custom",
      position: { x: 50, y: idx * 80 },
      data: {
        label: step.name,
        description: fullPrompt.length > 40 ? fullPrompt.slice(0, 40) + "..." : fullPrompt,
        prompt: fullPrompt,
        agentId: step.agent_id || step.agent?.agent_id,
        agentName: step.agent_name || step.agent?.name,
        nodeType: "agent",
      }
    };
    });
    setNodes(newNodes);

    const newEdges: Edge[] = [];
    for (let i = 0; i < newNodes.length - 1; i++) {
      newEdges.push({
        id: `edge-${i}`,
        source: newNodes[i].id,
        target: newNodes[i + 1].id,
        markerEnd: { type: MarkerType.ArrowClosed }
      });
    }
    setEdges(newEdges);
  }, [setNodes, setEdges]);

  // 新建工作流
  const handleNewWorkflow = useCallback(() => {
    setSelectedWorkflow(null);
    setNodes([]); setEdges([]);
    setWorkflowName(""); setWorkflowDescription("");
    setEditingNode(null);
  }, [setNodes, setEdges]);

  // 模板实例化回调：关闭浏览器，刷新工作流列表，选中新工作流
  const handleTemplateInstantiate = useCallback(async (workflowId: string) => {
    setShowTemplateBrowser(false);
    try {
      const updatedList = await listWorkflows();
      setWorkflows(updatedList);
      const created = updatedList.find(w => w.id === workflowId);
      if (created) {
        handleSelectWorkflow(created);
      }
    } catch { /* ignore */ }
  }, [handleSelectWorkflow]);

  // 有效 agent 步骤数量
  const agentStepCount = useMemo(() => buildSteps(nodes).length, [nodes, buildSteps]);

  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [zoomLevel, setZoomLevel] = useState(100);

  return (
    <div className={`flex flex-col transition-all duration-300 ${isFullscreen ? "fixed inset-0 z-[100] bg-main" : "h-[calc(100vh-140px)]"}`}>
      <header className="flex flex-wrap justify-between items-end gap-2 pb-2 sm:pb-4">
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
          {/* 状态信息 */}
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

          {/* 视图工具 */}
          <div className="flex items-center gap-0.5 px-0.5 sm:px-1">
            <Button variant="secondary" onClick={() => setIsFullscreen(!isFullscreen)} title={isFullscreen ? "Exit fullscreen" : "Fullscreen"}>
              {isFullscreen ? <Minimize2 className="w-4 h-4" /> : <Maximize2 className="w-4 h-4" />}
            </Button>
            <Button variant="secondary" onClick={() => fitView({ padding: 0.2, duration: 300 })} title="Fit View (Cmd+1)">
              <Scan className="w-4 h-4" />
            </Button>
          </div>

          <div className="w-px h-5 bg-border-subtle hidden sm:block" />

          {/* 文件操作 */}
          <div className="flex items-center gap-0.5 px-0.5 sm:px-1">
            <Button variant="secondary" onClick={() => setShowWorkflowPanel(!showWorkflowPanel)} title={t("workflows.open_workflows")}>
              <FolderOpen className="w-4 h-4" />
            </Button>
            <Button variant="secondary" onClick={() => setShowTemplateBrowser(true)} title={t("canvas.browse_templates")}>
              <LayoutTemplate className="w-4 h-4" />
            </Button>
            <Button variant="secondary" onClick={exportWorkflow} title="Export (Cmd+E)">
              <Download className="w-4 h-4" />
            </Button>
            <Button variant="secondary" onClick={importWorkflow} title="Import (Cmd+I)" className="hidden sm:flex">
              <Upload className="w-4 h-4" />
            </Button>
          </div>

          <div className="w-px h-5 bg-border-subtle hidden sm:block" />

          {/* 画布操作 */}
          <div className="flex items-center gap-0.5 px-0.5 sm:px-1">
            <Button variant="secondary" onClick={() => { setNodes([]); setEdges([]); setEditingNode(null); }} title={t("common.clear")}>
              <Trash2 className="w-4 h-4" />
            </Button>
            <Button variant="secondary" onClick={() => setShowHelp(true)} title="Shortcuts (?)" className="hidden sm:flex">
              <HelpCircle className="w-4 h-4" />
            </Button>
          </div>

          <div className="w-px h-5 bg-border-subtle hidden sm:block" />

          {/* 主操作 */}
          <div className="flex items-center gap-1 sm:gap-1.5 pl-0.5 sm:pl-1">
            <Button variant="primary" onClick={handleSave} disabled={!workflowName.trim() || agentStepCount === 0}>
              <Save className="w-4 h-4" />
              <span className="hidden sm:inline ml-1">{t("common.save")}</span>
            </Button>
            <Button variant="ghost" onClick={handleSaveAsTemplate} disabled={!selectedWorkflow?.id}
              title={t("canvas.save_as_template")}>
              <BookCopy className="w-4 h-4 mr-1" />
              {t("canvas.save_as_template")}
            </Button>
            <Button variant="primary" onClick={() => handleRunClick()}
              disabled={(!selectedWorkflow && agentStepCount === 0) || !!runningWorkflowId}>
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

        {/* 节点库（可折叠） */}
        <div className={`border-r border-border-subtle bg-surface/50 backdrop-blur overflow-y-auto transition-all duration-200 hidden sm:block ${
          sidebarCollapsed ? "w-10 px-1 py-2" : "w-52 p-3 space-y-4"
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
                        className="flex items-center gap-2.5 px-2.5 py-2 rounded-xl bg-surface hover:bg-main border border-transparent hover:border-border-subtle hover:shadow-sm transition-all text-left group">
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

        {/* 画布 */}
        <main className="flex-1 relative">
          <div className="absolute top-3 left-3 right-3 z-10 flex gap-2">
            <input type="text" value={workflowName} onChange={(e) => setWorkflowName(e.target.value)}
              placeholder={t("workflows.workflow_name")}
              className="flex-1 max-w-xs rounded-xl border border-border-subtle bg-surface/90 backdrop-blur px-3 py-2 text-sm font-bold focus:border-brand focus:ring-2 focus:ring-brand/20 outline-none shadow-sm" />
            <input type="text" value={workflowDescription} onChange={(e) => setWorkflowDescription(e.target.value)}
              placeholder={t("workflows.description")}
              className="flex-1 max-w-sm rounded-xl border border-border-subtle bg-surface/90 backdrop-blur px-3 py-2 text-sm text-text-dim focus:border-brand focus:ring-2 focus:ring-brand/20 outline-none shadow-sm" />
          </div>

          {/* 节点配置面板 */}
          {editingNode && !showRunInput && (
            <NodeConfigPanel node={editingNode} agents={agents}
              onUpdate={handleNodeUpdate} onClose={() => setEditingNode(null)}
              onDelete={(id) => { setNodes(nds => nds.filter(n => n.id !== id)); setEditingNode(null); }}
              t={t} />
          )}

          {/* 运行输入弹窗 */}
          {showRunInput && (
            <div className="absolute top-3 right-3 z-20 w-80 rounded-xl border border-border-subtle bg-surface shadow-2xl overflow-hidden">
              <div className="flex items-center justify-between px-3 py-2 bg-success/10 border-b border-border-subtle">
                <span className="text-xs font-bold text-success">{t("canvas.run_input_title")}</span>
                <button onClick={() => setShowRunInput(false)} className="p-1 rounded hover:bg-main"><X className="w-3.5 h-3.5" /></button>
              </div>
              <div className="p-3 space-y-3">
                <p className="text-[10px] text-text-dim">{t("canvas.run_input_hint")}</p>
                <textarea value={runInput} onChange={e => setRunInput(e.target.value)}
                  placeholder={t("canvas.run_input_placeholder")}
                  rows={4} autoFocus
                  className="w-full rounded-lg border border-border-subtle bg-main px-3 py-2 text-xs outline-none focus:border-brand resize-none"
                  onKeyDown={e => { if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) handleRunConfirm(); }}
                />
                <div className="flex gap-2">
                  <Button variant="primary" size="sm" className="flex-1" onClick={() => handleRunConfirm()}
                    disabled={!!runningWorkflowId}>
                    <Play className="w-3.5 h-3.5 mr-1" />
                    {t("canvas.run_now")}
                  </Button>
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
            // 交互：默认拖拽=框选，空格+拖拽=平移
            panOnDrag={spacePressed}
            selectionOnDrag={!spacePressed}
            panOnScroll
            selectionMode={SelectionMode.Partial}
            zoomOnScroll
            className={`!bg-transparent ${spacePressed ? "!cursor-grab" : ""}`}
            connectionLineStyle={{ stroke: edgeColorActive, strokeWidth: 2 }}
            connectionLineType={ConnectionLineType.SmoothStep}
            isValidConnection={isValidConnection}
          >
            <Background variant={BackgroundVariant.Dots} color={theme === "dark" ? "#444" : "#cbd5e1"} gap={24} size={1.5} />
            <Controls className="!bg-surface !border-border-subtle !rounded-xl !shadow-lg" />
            <div className="react-flow__panel !bottom-2 !left-14">
              <span className="text-[10px] font-mono text-text-dim/50 bg-surface/80 px-1.5 py-0.5 rounded">{zoomLevel}%</span>
            </div>
            <MiniMap className="!bg-surface/80 !border-border-subtle !rounded-xl !shadow-lg"
              nodeColor={(n) => {
                const cfg = NODE_TYPES.find(t => t.type === (n.data as any)?.nodeType);
                return cfg?.color || "#3b82f6";
              }}
              maskColor={theme === "dark" ? "rgba(0,0,0,0.3)" : "rgba(0,0,0,0.08)"} />
          </ReactFlow>

          {/* 空画布引导 */}
          {nodes.length === 0 && (
            <div className="absolute inset-0 flex items-center justify-center pointer-events-none z-10">
              <div className="text-center pointer-events-auto">
                <div className="w-12 h-12 rounded-2xl bg-brand/10 flex items-center justify-center mx-auto mb-3">
                  <Plus className="w-6 h-6 text-brand" />
                </div>
                <p className="text-sm font-bold text-text-dim">{t("canvas.empty_title")}</p>
                <p className="text-xs text-text-dim/60 mt-1">{t("canvas.empty_hint")}</p>
              </div>
            </div>
          )}

          {/* 右键上下文菜单 */}
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

          {/* 运行结果面板 */}
          {runResult && (
            <div className="absolute bottom-3 left-3 right-3 z-20 max-h-48 rounded-xl border border-border-subtle bg-surface shadow-2xl overflow-hidden flex flex-col">
              <div className="flex items-center justify-between px-3 py-2 bg-success/10 border-b border-border-subtle shrink-0">
                <div className="flex items-center gap-2">
                  <span className="text-xs font-bold text-success">{t("canvas.run_result")}</span>
                  <Badge variant="success">{runResult.status}</Badge>
                  {runResult.run_id && <span className="text-[9px] text-text-dim font-mono">{runResult.run_id.slice(0, 8)}</span>}
                </div>
                <button onClick={() => setRunResult(null)} className="p-1 rounded hover:bg-main"><X className="w-3.5 h-3.5" /></button>
              </div>
              <pre className="px-3 py-2 text-xs text-text whitespace-pre-wrap overflow-y-auto flex-1">{runResult.output}</pre>
            </div>
          )}
        </main>
      </div>

      {/* 模板浏览器 */}
      {showTemplateBrowser && (
        <TemplateBrowser
          onInstantiate={handleTemplateInstantiate}
          onClose={() => setShowTemplateBrowser(false)}
          t={t}
        />
      )}

      {/* Toast */}
      {toast && (
        <div className="fixed bottom-6 left-1/2 -translate-x-1/2 z-50 px-4 py-2 rounded-xl bg-text text-surface text-xs font-bold shadow-lg transition-all">
          <Check className="w-3.5 h-3.5 inline mr-1.5" />{toast}
        </div>
      )}

      {/* 快捷键帮助面板 */}
      {showHelp && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 backdrop-blur-xl backdrop-saturate-150" onClick={() => setShowHelp(false)}>
          <div className="bg-surface rounded-2xl shadow-2xl border border-border-subtle w-[420px] max-w-[90vw] max-h-[80vh] overflow-y-auto animate-fade-in-scale" onClick={e => e.stopPropagation()}>
            <div className="flex items-center justify-between px-5 py-3 border-b border-border-subtle">
              <h3 className="text-sm font-bold">{t("canvas.shortcuts_title")}</h3>
              <button onClick={() => setShowHelp(false)} className="p-1 rounded hover:bg-main"><X className="w-4 h-4" /></button>
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
          </div>
        </div>
      )}
    </div>
  );
}
