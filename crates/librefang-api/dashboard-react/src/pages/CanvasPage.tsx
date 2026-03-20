import { useCallback, useState, useEffect, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate, useSearch } from "@tanstack/react-router";
import {
  ReactFlow,
  Background,
  Controls,
  MiniMap,
  addEdge,
  useNodesState,
  useEdgesState,
  getNodesBounds,
  type Node,
  type Edge,
  type Connection,
  MarkerType,
  Handle,
  Position,
  type OnSelectionChangeParams,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { listAgents, listWorkflows, createWorkflow, updateWorkflow, deleteWorkflow, runWorkflow, type AgentItem, type WorkflowItem } from "../api";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { useUIStore } from "../lib/store";
import {
  Play, Save, Trash2, Plus, FolderOpen, Loader2,
  Maximize2, Minimize2, ArrowLeft, X, Group, ChevronDown, ChevronRight
} from "lucide-react";

// 节点类型配置
const NODE_TYPES = [
  // 触发器（视觉标记，不参与执行）
  { type: "start", labelKey: "canvas.node_types.start", color: "#22c55e", icon: "S", descKey: "canvas.node_types.start_desc" },
  { type: "end", labelKey: "canvas.node_types.end", color: "#ef4444", icon: "E", descKey: "canvas.node_types.end_desc" },
  { type: "schedule", labelKey: "canvas.node_types.schedule", color: "#f59e0b", icon: "⏱", descKey: "canvas.node_types.schedule_desc" },
  { type: "webhook", labelKey: "canvas.node_types.webhook", color: "#3b82f6", icon: "↗", descKey: "canvas.node_types.webhook_desc" },
  { type: "channel", labelKey: "canvas.node_types.channel", color: "#8b5cf6", icon: "📢", descKey: "canvas.node_types.channel_desc" },
  // 逻辑控制（绑 agent 后参与执行）
  { type: "condition", labelKey: "canvas.node_types.condition", color: "#f59e0b", icon: "?", descKey: "canvas.node_types.condition_desc" },
  { type: "loop", labelKey: "canvas.node_types.loop", color: "#8b5cf6", icon: "↻", descKey: "canvas.node_types.loop_desc" },
  { type: "parallel", labelKey: "canvas.node_types.parallel", color: "#f59e0b", icon: "⫸", descKey: "canvas.node_types.parallel_desc" },
  { type: "collect", labelKey: "canvas.node_types.collect", color: "#22c55e", icon: "⫷", descKey: "canvas.node_types.collect_desc" },
  { type: "wait", labelKey: "canvas.node_types.wait", color: "#6b7280", icon: "⏸", descKey: "canvas.node_types.wait_desc" },
  // 动作（核心执行节点）
  { type: "respond", labelKey: "canvas.node_types.respond", color: "#22c55e", icon: "↩", descKey: "canvas.node_types.respond_desc" },
  { type: "agent", labelKey: "canvas.node_types.agent", color: "#3b82f6", icon: "A", descKey: "canvas.node_types.agent_desc" },
];

// 自定义节点组件
function CustomNode({ data, type: nodeTypeKey, t }: { data: any; type: string; t: (key: string) => string }) {
  const config = NODE_TYPES.find(n => n.type === (data.nodeType || nodeTypeKey)) || NODE_TYPES[10];
  const isStart = data.nodeType === "start";
  const isEnd = data.nodeType === "end";
  // runState: "running" | "done" | undefined
  const runState = data._runState as string | undefined;
  const borderClass = runState === "running"
    ? "border-warning shadow-warning/30 shadow-lg animate-pulse"
    : runState === "done"
    ? "border-success shadow-success/20 shadow-md"
    : "border-border-subtle";
  return (
    <div className={`rounded-lg border-2 bg-surface min-w-[80px] overflow-hidden relative transition-all duration-300 ${borderClass}`}>
      {!isStart && <Handle type="target" position={Position.Top} className="!w-2 !h-2 !bg-border-subtle !border-surface" />}
      <div className="flex items-center gap-1 px-2 py-1" style={{ backgroundColor: config.color }}>
        {runState === "running" && <Loader2 className="w-3 h-3 text-white animate-spin shrink-0" />}
        {runState === "done" && <span className="text-xs text-white shrink-0">✓</span>}
        {!runState && <span className="text-xs font-bold text-white">{config.icon}</span>}
        <span className="text-xs font-bold text-white truncate">{data.label || t(config.labelKey)}</span>
      </div>
      <div className="px-2 py-1 bg-surface">
        <p className="text-[8px] font-medium text-text-dim leading-tight">{data.description || t(config.descKey)}</p>
        {data.agentName && (
          <p className="text-[8px] font-bold text-brand mt-0.5">{data.agentName}</p>
        )}
      </div>
      {!isEnd && <Handle type="source" position={Position.Bottom} className="!w-2 !h-2 !bg-border-subtle !border-surface" />}
    </div>
  );
}

// 分组节点组件
function GroupNodeComponent({ data, id }: { data: any; id: string }) {
  const expanded = data._expanded !== false; // 默认展开
  return (
    <div
      className={`rounded-xl border-2 border-dashed transition-all ${
        expanded ? "border-brand/40 bg-brand/5" : "border-brand bg-surface shadow-lg w-[160px]"
      }`}
      style={expanded ? { pointerEvents: "none" } : undefined}
    >
      <Handle type="target" position={Position.Top} className="!w-2 !h-2 !bg-brand !border-surface" />
      <div
        className="flex items-center gap-1.5 px-2 py-1.5 bg-brand/10 rounded-t-lg cursor-pointer relative z-10"
        style={{ pointerEvents: "auto" }}
        onClick={(e) => { e.stopPropagation(); data._onToggle?.(id); }}
      >
        {expanded
          ? <ChevronDown className="w-3 h-3 text-brand shrink-0" />
          : <ChevronRight className="w-3 h-3 text-brand shrink-0" />}
        <Group className="w-3 h-3 text-brand shrink-0" />
        <span className="text-xs font-bold text-brand truncate">{data.label || "Group"}</span>
        {!expanded && data._childCount > 0 && (
          <span className="text-[9px] text-brand/60 ml-auto">{data._childCount} nodes</span>
        )}
      </div>
      {!expanded && (
        <div className="px-2 py-1">
          <p className="text-[8px] text-text-dim italic">Click to expand</p>
        </div>
      )}
      <Handle type="source" position={Position.Bottom} className="!w-2 !h-2 !bg-brand !border-surface" />
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
              <div className="flex items-center justify-between">
                <span className="text-sm font-bold truncate">{w.name}</span>
                <div className="flex gap-1">
                  <button onClick={(e) => { e.stopPropagation(); onRun(w.id); }} disabled={isRunning === w.id}
                    className="p-1.5 rounded-lg hover:bg-success/10 text-success disabled:opacity-50">
                    {isRunning === w.id ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Play className="w-3.5 h-3.5" />}
                  </button>
                  <button onClick={(e) => { e.stopPropagation(); onDelete(w.id); }}
                    className="p-1.5 rounded-lg hover:bg-error/10 text-error">
                    <Trash2 className="w-3.5 h-3.5" />
                  </button>
                </div>
              </div>
              <p className="text-[10px] text-text-dim mt-1 truncate">{w.description || "-"}</p>
            </div>
          ))
        )}
      </div>
    </Card>
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
    <div className="absolute top-3 right-3 z-20 w-80 max-h-[calc(100%-24px)] rounded-xl border border-border-subtle bg-surface shadow-2xl overflow-hidden flex flex-col">
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
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { t: routeTimestamp } = useSearch({ from: "/canvas" });
  const { theme } = useUIStore();
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

  const nodeTypes = useMemo(() => ({
    custom: (props: any) => <CustomNode {...props} t={t} />,
    groupNode: (props: any) => <GroupNodeComponent {...props} data={{ ...props.data, _onToggle: toggleGroup }} />,
  }), [t, toggleGroup]);

  // 需要 agent 的节点类型（后端所有 step 都需要 agent）
  const AGENT_NODE_TYPES = new Set(["agent", "channel", "respond", "condition", "loop", "parallel", "collect"]);

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
    const newNode: Node = {
      id: `${type}-${Date.now()}`,
      type: "custom",
      position: { x: 50 + Math.random() * 50, y: 30 + Math.random() * 30 },
      data: {
        label: t(config.labelKey),
        description: t(config.descKey),
        nodeType: type,
        ...(defaultMode ? { stepMode: defaultMode } : {}),
      }
    };
    setNodes((nds) => [...nds, newNode]);
  }, [setNodes, t]);

  // 连线
  const onConnect = useCallback((params: Connection) => {
    setEdges((eds) => addEdge({
      ...params,
      markerEnd: { type: MarkerType.ArrowClosed, color: theme === "dark" ? "#fff" : "#000" },
    }, eds));
  }, [setEdges, theme]);

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

  // 跟踪选中的节点
  const onSelectionChange = useCallback(({ nodes: selected }: OnSelectionChangeParams) => {
    setSelectedNodeIds(new Set(selected.map(n => n.id)));
  }, []);

  // 创建分组：不改变子节点位置，只在底层加一个背景框 + 标记归属
  const createGroup = useCallback(() => {
    if (selectedNodeIds.size < 2) return;

    const selected = nodes.filter(n => selectedNodeIds.has(n.id) && n.type !== "groupNode");
    if (selected.length < 2) return;

    const bounds = getNodesBounds(selected);
    const padding = 30;
    const groupId = `group-${Date.now()}`;
    const childIds = selected.map(n => n.id);

    // group 节点放在最底层（z-index 通过数组顺序控制）
    const groupNode: Node = {
      id: groupId,
      type: "groupNode",
      position: { x: bounds.x - padding, y: bounds.y - padding - 30 },
      style: { width: bounds.width + padding * 2, height: bounds.height + padding * 2 + 30, zIndex: -1 },
      zIndex: -1,
      data: {
        label: t("canvas.new_group"),
        _expanded: true,
        _childIds: childIds,
        _childCount: childIds.length,
      },
    };

    // 标记子节点归属（不改 parentId 和 position）
    const updatedNodes = nodes.map(n => {
      if (childIds.includes(n.id)) {
        return { ...n, data: { ...(n.data as any), _groupId: groupId } };
      }
      return n;
    });

    setNodes([groupNode, ...updatedNodes]);
    setSelectedNodeIds(new Set());
  }, [selectedNodeIds, nodes, setNodes, t]);

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

  const showError = useCallback((msg: string) => {
    setErrorMsg(msg);
    setTimeout(() => setErrorMsg(null), 5000);
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
    } catch (e: any) {
      showError(e?.message || String(e));
    }
  }, [workflowName, workflowDescription, selectedWorkflow, nodes, buildSteps, t, showError]);

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
      // 3秒后清除 done 状态
      setTimeout(() => {
        setNodes(nds => nds.map(n => ({ ...n, data: { ...(n.data as any), _runState: undefined } })));
      }, 3000);
    } catch (e: any) {
      if (stepTimer) clearInterval(stepTimer);
      // 错误：清除所有状态
      setNodes(nds => nds.map(n => ({ ...n, data: { ...(n.data as any), _runState: undefined } })));
      showError(e?.message || String(e));
    } finally {
      setRunningWorkflowId(null);
    }
  }, [selectedWorkflow, nodes, edges, workflowName, workflowDescription, buildSteps, runInput, t, showError]);

  // 删除工作流
  const handleDelete = useCallback(async (id: string) => {
    if (!window.confirm(t("workflows.delete_confirm"))) return;
    try {
      await deleteWorkflow(id);
      setWorkflows(prev => prev.filter(w => w.id !== id));
      if (selectedWorkflow?.id === id) {
        setSelectedWorkflow(null);
        setNodes([]); setEdges([]);
        setWorkflowName(""); setWorkflowDescription("");
      }
    } catch (e) { console.error(e); }
  }, [selectedWorkflow, t, setNodes, setEdges]);

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

  // 有效 agent 步骤数量
  const agentStepCount = useMemo(() => buildSteps(nodes).length, [nodes, buildSteps]);

  return (
    <div className={`flex flex-col transition-all duration-300 ${isFullscreen ? "fixed inset-0 z-50 bg-main" : "h-[calc(100vh-140px)]"}`}>
      <header className="flex justify-between items-end pb-4">
        <div className="flex items-center gap-4">
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
        <div className="flex items-center gap-2">
          {selectedNodeIds.size >= 2 && (
            <Button variant="secondary" size="sm" onClick={createGroup}>
              <Group className="w-3.5 h-3.5 mr-1" />
              {t("canvas.create_group")}
            </Button>
          )}
          {agentStepCount > 0 && (
            <span className="text-[10px] font-bold text-success mr-1">
              {agentStepCount} {t("canvas.agent_steps")}
            </span>
          )}
          <Button variant="secondary" onClick={() => setIsFullscreen(!isFullscreen)}>
            {isFullscreen ? <Minimize2 className="w-4 h-4" /> : <Maximize2 className="w-4 h-4" />}
          </Button>
          <Button variant="secondary" onClick={() => setShowWorkflowPanel(!showWorkflowPanel)}>
            <FolderOpen className="w-4 h-4 mr-1" />
            {t("workflows.open_workflows")}
          </Button>
          <Button variant="secondary" onClick={() => { setNodes([]); setEdges([]); setEditingNode(null); }}>
            {t("common.clear")}
          </Button>
          <Button variant="primary" onClick={handleSave} disabled={!workflowName.trim() || agentStepCount === 0}>
            <Save className="w-4 h-4 mr-1" />
            {t("common.save")}
          </Button>
          <Button variant="primary" onClick={() => handleRunClick()}
            disabled={(!selectedWorkflow && agentStepCount === 0) || !!runningWorkflowId}>
            {runningWorkflowId ? <Loader2 className="w-4 h-4 mr-1 animate-spin" /> : <Play className="w-4 h-4 mr-1" />}
            {t("workflows.run_workflow")}
          </Button>
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
            onSelect={handleSelectWorkflow} onDelete={handleDelete} onRun={handleRunClick}
            isRunning={runningWorkflowId} t={t} />
        )}

        {/* 节点库 */}
        <Card padding="md" className="w-48 border-r border-border-subtle bg-main/30 overflow-y-auto rounded-none">
          <h3 className="text-[10px] font-black uppercase text-text-dim/60 mb-4">{t("canvas.node_library")}</h3>
          <div className="space-y-4">
            <div>
              <p className="text-[10px] font-bold text-brand uppercase mb-2">{t("canvas.triggers")}</p>
              <div className="grid gap-2">
                {NODE_TYPES.slice(0, 5).map(n => (
                  <button key={n.type} onClick={() => addNode(n.type)}
                    className="flex items-center gap-2 p-2 rounded-lg border border-border-subtle bg-surface hover:border-brand transition-all text-left">
                    <div className="h-7 w-7 rounded flex items-center justify-center text-white text-xs font-black" style={{ backgroundColor: n.color }}>{n.icon}</div>
                    <span className="text-xs font-bold truncate">{t(n.labelKey)}</span>
                  </button>
                ))}
              </div>
            </div>
            <div>
              <p className="text-[10px] font-bold text-warning uppercase mb-2">{t("canvas.logic")}</p>
              <div className="grid gap-2">
                {NODE_TYPES.slice(5, 10).map(n => (
                  <button key={n.type} onClick={() => addNode(n.type)}
                    className="flex items-center gap-2 p-2 rounded-lg border border-border-subtle bg-surface hover:border-brand transition-all text-left">
                    <div className="h-7 w-7 rounded flex items-center justify-center text-white text-xs font-black" style={{ backgroundColor: n.color }}>{n.icon}</div>
                    <span className="text-xs font-bold truncate">{t(n.labelKey)}</span>
                  </button>
                ))}
              </div>
            </div>
            <div>
              <p className="text-[10px] font-bold text-accent uppercase mb-2">{t("canvas.actions")}</p>
              <div className="grid gap-2">
                {NODE_TYPES.slice(10).map(n => (
                  <button key={n.type} onClick={() => addNode(n.type)}
                    className="flex items-center gap-2 p-2 rounded-lg border border-border-subtle bg-surface hover:border-brand transition-all text-left">
                    <div className="h-7 w-7 rounded flex items-center justify-center text-white text-xs font-black" style={{ backgroundColor: n.color }}>{n.icon}</div>
                    <span className="text-xs font-bold truncate">{t(n.labelKey)}</span>
                  </button>
                ))}
              </div>
            </div>
          </div>
        </Card>

        {/* 画布 */}
        <main className="flex-1 relative">
          <div className="absolute top-3 left-3 right-3 z-10 flex gap-3">
            <input type="text" value={workflowName} onChange={(e) => setWorkflowName(e.target.value)}
              placeholder={t("workflows.workflow_name")}
              className="flex-1 max-w-xs rounded-lg border border-border-subtle bg-surface px-3 py-2 text-sm font-bold focus:border-brand focus:ring-1 focus:ring-brand/20 outline-none" />
            <input type="text" value={workflowDescription} onChange={(e) => setWorkflowDescription(e.target.value)}
              placeholder={t("workflows.description")}
              className="flex-1 max-w-xs rounded-lg border border-border-subtle bg-surface px-3 py-2 text-sm focus:border-brand focus:ring-1 focus:ring-brand/20 outline-none" />
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
            onNodesChange={onNodesChange} onEdgesChange={onEdgesChange}
            onConnect={onConnect} onNodeClick={onNodeClick} onNodesDelete={onNodesDelete}
            onSelectionChange={onSelectionChange}
            nodeTypes={nodeTypes} colorMode={theme}
            defaultViewport={{ x: 50, y: 80, zoom: 1 }}
            minZoom={0.1} maxZoom={2} className="bg-main/20"
          >
            <Background color={theme === "dark" ? "#333" : "#ccc"} gap={20} />
            <Controls className="!bg-surface !border-border-subtle" />
            <MiniMap className="!bg-surface !border-border-subtle"
              nodeColor={(n) => NODE_TYPES.find(t => t.type === n.type)?.color || "#3b82f6"} />
          </ReactFlow>

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
    </div>
  );
}
