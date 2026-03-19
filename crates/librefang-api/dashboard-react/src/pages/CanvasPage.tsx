import { useCallback, useState, useEffect, useMemo } from "react";
import { useTranslation } from "react-i18next";
import {
  ReactFlow,
  Background,
  Controls,
  MiniMap,
  addEdge,
  useNodesState,
  useEdgesState,
  type Node,
  type Edge,
  type Connection,
  MarkerType,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { listAgents, listWorkflows, createWorkflow, updateWorkflow, deleteWorkflow, runWorkflow, type AgentItem, type WorkflowItem } from "../api";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { useUIStore } from "../lib/store";
import {
  Play, Save, Trash2, Plus, FolderOpen, X, Settings, Zap, Clock, Webhook, MessageSquare,
  GitBranch, Repeat, Layers, Hourglass, Send, Bot, ChevronRight, Loader2, Search
} from "lucide-react";

// 节点类型配置
const NODE_TYPES = [
  { type: "start", label: "Start", color: "#22c55e", icon: "S", description: "Workflow entry point" },
  { type: "end", label: "End", color: "#ef4444", icon: "E", description: "Workflow termination" },
  { type: "schedule", label: "Schedule", color: "#f59e0b", icon: "C", description: "Time-based trigger" },
  { type: "webhook", label: "Webhook", color: "#3b82f6", icon: "W", description: "HTTP webhook trigger" },
  { type: "channel", label: "Channel", color: "#8b5cf6", icon: "M", description: "Send to channel" },
  { type: "condition", label: "Condition", color: "#22c55e", icon: "?", description: "Branch logic" },
  { type: "loop", label: "Loop", color: "#8b5cf6", icon: "L", description: "Repeat actions" },
  { type: "parallel", label: "Parallel", color: "#f59e0b", icon: "P", description: "Parallel execution" },
  { type: "wait", label: "Wait", color: "#6b7280", icon: "T", description: "Delay/wait" },
  { type: "respond", label: "Respond", color: "#22c55e", icon: "R", description: "Send response" },
  { type: "agent", label: "Agent", color: "#3b82f6", icon: "A", description: "Run agent task" },
];

// 自定义节点组件
function CustomNode({ data, type: nodeTypeKey }: { data: any; type: string }) {
  const config = NODE_TYPES.find(n => n.type === nodeTypeKey) || NODE_TYPES[10];
  return (
    <div className="rounded-lg border-2 border-border-subtle bg-surface shadow-lg min-w-[160px] overflow-hidden">
      <div className="flex items-center gap-2 px-3 py-2" style={{ backgroundColor: config.color }}>
        <span className="text-sm font-bold text-white">{config.icon}</span>
        <span className="text-sm font-bold text-white truncate">{data.label || config.label}</span>
      </div>
      <div className="px-3 py-2 bg-surface">
        <p className="text-[10px] font-medium text-text-dim leading-tight">{data.description || config.description}</p>
        {data.agentName && (
          <p className="text-[10px] font-bold text-brand mt-1">Agent: {data.agentName}</p>
        )}
      </div>
    </div>
  );
}

// 工作流列表侧边栏
function WorkflowList({
  workflows,
  selectedId,
  onSelect,
  onDelete,
  onRun,
  isRunning,
  t
}: {
  workflows: WorkflowItem[];
  selectedId: string | null;
  onSelect: (w: WorkflowItem) => void;
  onDelete: (id: string) => void;
  onRun: (id: string) => void;
  isRunning: string | null;
  t: (key: string) => string;
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
            <div
              key={w.id}
              onClick={() => onSelect(w)}
              className={`p-3 rounded-xl border cursor-pointer transition-all ${
                selectedId === w.id
                  ? "border-brand bg-brand/5"
                  : "border-border-subtle hover:border-brand/50 bg-surface"
              }`}
            >
              <div className="flex items-center justify-between">
                <span className="text-sm font-bold truncate">{w.name}</span>
                <div className="flex gap-1">
                  <button
                    onClick={(e) => { e.stopPropagation(); onRun(w.id); }}
                    disabled={isRunning === w.id}
                    className="p-1.5 rounded-lg hover:bg-success/10 text-success disabled:opacity-50"
                  >
                    {isRunning === w.id ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Play className="w-3.5 h-3.5" />}
                  </button>
                  <button
                    onClick={(e) => { e.stopPropagation(); onDelete(w.id); }}
                    className="p-1.5 rounded-lg hover:bg-error/10 text-error"
                  >
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

export function CanvasPage() {
  const { t } = useTranslation();
  const { theme } = useUIStore();
  const [nodes, setNodes, onNodesChange] = useNodesState([]);
  const [edges, setEdges, onEdgesChange] = useEdgesState([]);
  const [agents, setAgents] = useState<AgentItem[]>([]);
  const [workflows, setWorkflows] = useState<WorkflowItem[]>([]);
  const [selectedWorkflow, setSelectedWorkflow] = useState<WorkflowItem | null>(null);
  const [workflowName, setWorkflowName] = useState("");
  const [workflowDescription, setWorkflowDescription] = useState("");
  const [showWorkflowPanel, setShowWorkflowPanel] = useState(true);
  const [runningWorkflowId, setRunningWorkflowId] = useState<string | null>(null);
  const [selectedAgent, setSelectedAgent] = useState<string>("");

  const nodeTypes = useMemo(() => ({ custom: CustomNode }), []);

  // 加载数据
  useEffect(() => {
    Promise.all([
      listAgents(),
      listWorkflows()
    ]).then(([agentsData, workflowsData]) => {
      setAgents(agentsData);
      setWorkflows(workflowsData);
    }).catch(() => {});
  }, []);

  // 添加节点
  const addNode = useCallback((type: string) => {
    const config = NODE_TYPES.find(n => n.type === type) || NODE_TYPES[10];
    const newNode: Node = {
      id: `${type}-${Date.now()}`,
      type: "custom",
      position: { x: 200 + Math.random() * 200, y: 100 + Math.random() * 200 },
      data: {
        label: config.label,
        description: config.description,
        agentName: type === "agent" ? agents.find(a => a.id === selectedAgent)?.name : undefined
      }
    };
    setNodes((nds) => [...nds, newNode]);
  }, [agents, selectedAgent, setNodes]);

  // 连线
  const onConnect = useCallback((params: Connection) => {
    setEdges((eds) => addEdge({
      ...params,
      markerEnd: { type: MarkerType.ArrowClosed, color: theme === "dark" ? "#fff" : "#000" },
    }, eds));
  }, [setEdges, theme]);

  // 保存工作流
  const handleSave = useCallback(async () => {
    if (!workflowName.trim()) return;

    const steps = nodes.map((n, idx) => ({
      name: n.data.label || `Step ${idx + 1}`,
      agent_id: n.data.agentId,
      agent_name: n.data.agentName,
      prompt: n.data.description || "",
      timeout_secs: 60,
    }));

    try {
      if (selectedWorkflow?.id) {
        await updateWorkflow(selectedWorkflow.id, {
          name: workflowName,
          description: workflowDescription,
          steps
        });
      } else {
        await createWorkflow({
          name: workflowName,
          description: workflowDescription,
          steps
        });
      }
      const workflowsData = await listWorkflows();
      setWorkflows(workflowsData);
    } catch (e) {
      console.error(e);
    }
  }, [workflowName, workflowDescription, selectedWorkflow, nodes]);

  // 运行工作流
  const handleRun = useCallback(async (id?: string) => {
    const workflowId = id || selectedWorkflow?.id;
    if (!workflowId) return;

    setRunningWorkflowId(workflowId);
    try {
      await runWorkflow(workflowId, "");
    } catch (e) {
      console.error(e);
    } finally {
      setRunningWorkflowId(null);
    }
  }, [selectedWorkflow]);

  // 删除工作流
  const handleDelete = useCallback(async (id: string) => {
    if (!window.confirm(t("workflows.delete_confirm"))) return;
    try {
      await deleteWorkflow(id);
      setWorkflows(prev => prev.filter(w => w.id !== id));
      if (selectedWorkflow?.id === id) {
        setSelectedWorkflow(null);
        setNodes([]);
        setEdges([]);
        setWorkflowName("");
        setWorkflowDescription("");
      }
    } catch (e) {
      console.error(e);
    }
  }, [selectedWorkflow, t, setNodes, setEdges]);

  // 选择工作流
  const handleSelectWorkflow = useCallback((w: WorkflowItem) => {
    setSelectedWorkflow(w);
    setWorkflowName(w.name);
    setWorkflowDescription(w.description || "");

    // 解析步骤为节点
    const newNodes: Node[] = (w.steps || []).map((step: any, idx: number) => ({
      id: `node-${idx}`,
      type: "custom",
      position: { x: 200, y: 100 + idx * 120 },
      data: {
        label: step.name,
        description: step.prompt,
        agentId: step.agent_id,
        agentName: step.agent_name
      }
    }));
    setNodes(newNodes);

    // 创建边
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
    setNodes([]);
    setEdges([]);
    setWorkflowName("");
    setWorkflowDescription("");
  }, [setNodes, setEdges]);

  return (
    <div className="flex h-[calc(100vh-140px)] flex-col">
      <header className="flex justify-between items-end pb-4">
        <div className="flex items-center gap-4">
          <div>
            <h1 className="text-2xl font-extrabold">{t("canvas.title")}</h1>
            <p className="text-text-dim font-medium text-sm">{t("canvas.subtitle")}</p>
          </div>
          <Button variant="secondary" size="sm" onClick={handleNewWorkflow}>
            <Plus className="w-4 h-4 mr-1" />
            {t("workflows.new_workflow")}
          </Button>
        </div>
        <div className="flex gap-2">
          <Button variant="secondary" onClick={() => setShowWorkflowPanel(!showWorkflowPanel)}>
            <FolderOpen className="w-4 h-4 mr-1" />
            {t("workflows.all_workflows")}
          </Button>
          <Button variant="secondary" onClick={() => { setNodes([]); setEdges([]); }}>
            {t("common.clear")}
          </Button>
          <Button variant="primary" onClick={handleSave} disabled={!workflowName.trim()}>
            <Save className="w-4 h-4 mr-1" />
            {t("common.save")}
          </Button>
          <Button
            variant="success"
            onClick={() => handleRun()}
            disabled={!selectedWorkflow && nodes.length === 0}
          >
            <Play className="w-4 h-4 mr-1" />
            {t("workflows.run_workflow")}
          </Button>
        </div>
      </header>

      <div className="flex flex-1 overflow-hidden rounded-2xl border border-border-subtle bg-surface">
        {/* 左侧工作流列表 */}
        {showWorkflowPanel && (
          <WorkflowList
            workflows={workflows}
            selectedId={selectedWorkflow?.id || null}
            onSelect={handleSelectWorkflow}
            onDelete={handleDelete}
            onRun={handleRun}
            isRunning={runningWorkflowId}
            t={t}
          />
        )}

        {/* 节点库 */}
        <Card padding="md" className="w-56 border-r border-border-subtle bg-main/30 overflow-y-auto rounded-none">
          <h3 className="text-[10px] font-black uppercase text-text-dim/60 mb-4">{t("canvas.node_library")}</h3>
          <div className="space-y-4">
            <div>
              <p className="text-[10px] font-bold text-brand uppercase mb-2">{t("canvas.triggers")}</p>
              <div className="grid gap-2">
                {NODE_TYPES.slice(0, 5).map(n => (
                  <button
                    key={n.type}
                    onClick={() => addNode(n.type)}
                    className="flex items-center gap-2 p-2 rounded-lg border border-border-subtle bg-surface hover:border-brand transition-all text-left"
                  >
                    <div className="h-7 w-7 rounded flex items-center justify-center text-white text-xs font-black" style={{ backgroundColor: n.color }}>
                      {n.icon}
                    </div>
                    <span className="text-xs font-bold truncate">{n.label}</span>
                  </button>
                ))}
              </div>
            </div>
            <div>
              <p className="text-[10px] font-bold text-warning uppercase mb-2">{t("canvas.logic")}</p>
              <div className="grid gap-2">
                {NODE_TYPES.slice(5, 9).map(n => (
                  <button
                    key={n.type}
                    onClick={() => addNode(n.type)}
                    className="flex items-center gap-2 p-2 rounded-lg border border-border-subtle bg-surface hover:border-brand transition-all text-left"
                  >
                    <div className="h-7 w-7 rounded flex items-center justify-center text-white text-xs font-black" style={{ backgroundColor: n.color }}>
                      {n.icon}
                    </div>
                    <span className="text-xs font-bold truncate">{n.label}</span>
                  </button>
                ))}
              </div>
            </div>
            <div>
              <p className="text-[10px] font-bold text-accent uppercase mb-2">{t("canvas.actions")}</p>
              <div className="grid gap-2">
                {NODE_TYPES.slice(9).map(n => (
                  <button
                    key={n.type}
                    onClick={() => addNode(n.type)}
                    className="flex items-center gap-2 p-2 rounded-lg border border-border-subtle bg-surface hover:border-brand transition-all text-left"
                  >
                    <div className="h-7 w-7 rounded flex items-center justify-center text-white text-xs font-black" style={{ backgroundColor: n.color }}>
                      {n.icon}
                    </div>
                    <span className="text-xs font-bold truncate">{n.label}</span>
                  </button>
                ))}
              </div>
            </div>
          </div>
        </Card>

        {/* 画布 */}
        <main className="flex-1 relative">
          {/* 工作流信息栏 */}
          <div className="absolute top-3 left-3 right-3 z-10 flex gap-3">
            <input
              type="text"
              value={workflowName}
              onChange={(e) => setWorkflowName(e.target.value)}
              placeholder={t("workflows.workflow_name")}
              className="flex-1 max-w-xs rounded-lg border border-border-subtle bg-surface px-3 py-2 text-sm font-bold focus:border-brand focus:ring-1 focus:ring-brand/20 outline-none"
            />
            <input
              type="text"
              value={workflowDescription}
              onChange={(e) => setWorkflowDescription(e.target.value)}
              placeholder={t("workflows.description")}
              className="flex-1 max-w-xs rounded-lg border border-border-subtle bg-surface px-3 py-2 text-sm focus:border-brand focus:ring-1 focus:ring-brand/20 outline-none"
            />
          </div>

          <ReactFlow
            nodes={nodes}
            edges={edges}
            onNodesChange={onNodesChange}
            onEdgesChange={onEdgesChange}
            onConnect={onConnect}
            nodeTypes={nodeTypes}
            colorMode={theme}
            fitView
            className="bg-main/20"
          >
            <Background color={theme === "dark" ? "#333" : "#ccc"} gap={20} />
            <Controls className="!bg-surface !border-border-subtle" />
            <MiniMap className="!bg-surface !border-border-subtle" nodeColor={(n) => NODE_TYPES.find(t => t.type === n.type)?.color || "#3b82f6"} />
          </ReactFlow>
        </main>
      </div>
    </div>
  );
}
