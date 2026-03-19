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
import { listAgents, type AgentItem } from "../api";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { useUIStore } from "../lib/store";

const STORAGE_KEY = "librefang-canvas-draft";

export function CanvasPage() {
  const { t } = useTranslation();
  const { theme } = useUIStore();
  const [nodes, setNodes, onNodesChange] = useNodesState([]);
  const [edges, setEdges, onEdgesChange] = useEdgesState([]);
  const [agents, setAgents] = useState<AgentItem[]>([]);
  const [workflowName, setWorkflowName] = useState(t("common.no_data"));

  const nodeTypesConfig = useMemo(() => [
    { type: "start", label: t("canvas.nodes.start"), color: "var(--success-color)", icon: "S", description: t("canvas.nodes.start_desc") },
    { type: "end", label: t("canvas.nodes.end"), color: "var(--error-color)", icon: "E", description: t("canvas.nodes.end_desc") },
    { type: "schedule", label: t("canvas.nodes.schedule"), color: "var(--warning-color)", icon: "C", description: t("canvas.nodes.schedule_desc") },
    { type: "webhook", label: t("canvas.nodes.webhook"), color: "var(--brand-color)", icon: "W", description: t("canvas.nodes.webhook_desc") },
    { type: "channel", label: t("canvas.nodes.channel"), color: "var(--accent-color)", icon: "M", description: t("canvas.nodes.channel_desc") },
    { type: "condition", label: t("canvas.nodes.condition"), color: "var(--success-color)", icon: "?", description: t("canvas.nodes.condition_desc") },
    { type: "loop", label: t("canvas.nodes.loop"), color: "var(--accent-color)", icon: "L", description: t("canvas.nodes.loop_desc") },
    { type: "parallel", label: t("canvas.nodes.parallel"), color: "var(--warning-color)", icon: "P", description: t("canvas.nodes.parallel_desc") },
    { type: "wait", label: t("canvas.nodes.wait"), color: "var(--text-muted)", icon: "T", description: t("canvas.nodes.wait_desc") },
    { type: "respond", label: t("canvas.nodes.respond"), color: "var(--success-color)", icon: "R", description: t("canvas.nodes.respond_desc") },
    { type: "agent", label: t("canvas.nodes.agent"), color: "var(--brand-color)", icon: "A", description: t("canvas.nodes.agent_desc") },
  ], [t]);

  const CustomNode = useCallback(({ data, type: nodeTypeKey }: any) => {
    const config = nodeTypesConfig.find(n => n.type === nodeTypeKey) || nodeTypesConfig[10];
    return (
      <div className="rounded-lg border-2 border-border-subtle bg-surface shadow-lg min-w-[150px] overflow-hidden">
        <div className="flex items-center gap-2 px-3 py-2" style={{ backgroundColor: config.color }}>
          <span className="text-sm font-bold text-white">{config.icon}</span>
          <span className="text-sm font-bold text-white truncate">{config.label}</span>
        </div>
        <div className="px-3 py-2 bg-surface">
          <p className="text-[10px] font-medium text-text-dim leading-tight">{config.description}</p>
        </div>
      </div>
    );
  }, [nodeTypesConfig]);

  useEffect(() => {
    listAgents().then(setAgents).catch(() => setAgents([]));
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved) {
      try {
        const data = JSON.parse(saved);
        if (data.nodes) setNodes(data.nodes);
        if (data.edges) setEdges(data.edges);
      } catch {}
    }
  }, []);

  const addNode = useCallback((type: string) => {
    const config = nodeTypesConfig.find(n => n.type === type) || nodeTypesConfig[10];
    const newNode: Node = {
      id: `${type}-${Date.now()}`,
      type: "custom",
      position: { x: 100, y: 100 },
      data: { label: config.label, description: config.description }
    };
    setNodes((nds) => [...nds, newNode]);
  }, [nodeTypesConfig, setNodes]);

  return (
    <div className="flex h-[calc(100vh-140px)] flex-col">
      <header className="flex justify-between items-end pb-6">
        <div>
          <h1 className="text-3xl font-extrabold">{t("canvas.title")}</h1>
          <p className="text-text-dim font-medium">{t("canvas.subtitle")}</p>
        </div>
        <div className="flex gap-2">
          <Button variant="secondary" onClick={() => window.confirm(t("canvas.clear_confirm")) && setNodes([])}>{t("common.clear")}</Button>
          <Button variant="primary">{t("canvas.save_workflow")}</Button>
        </div>
      </header>

      <div className="flex flex-1 overflow-hidden rounded-2xl border border-border-subtle bg-surface">
        <Card padding="md" className="w-64 border-r border-border-subtle bg-main/30 overflow-y-auto rounded-none">
          <h3 className="text-[10px] font-black uppercase text-text-dim/60 mb-4">{t("canvas.node_library")}</h3>
          <div className="space-y-6">
            <div>
              <p className="text-[10px] font-bold text-brand uppercase mb-2">{t("canvas.triggers")}</p>
              <div className="grid gap-2">
                {nodeTypesConfig.slice(0, 5).map(n => (
                  <button key={n.type} onClick={() => addNode(n.type)} className="flex items-center gap-3 p-2.5 rounded-xl border border-border-subtle bg-surface hover:border-brand transition-all text-left">
                    <div className="h-8 w-8 rounded-lg flex items-center justify-center text-white text-xs font-black" style={{ backgroundColor: n.color }}>{n.icon}</div>
                    <span className="text-xs font-bold">{n.label}</span>
                  </button>
                ))}
              </div>
            </div>
          </div>
        </Card>
        <main className="flex-1">
          <ReactFlow nodes={nodes} edges={edges} onNodesChange={onNodesChange} onEdgesChange={onEdgesChange} onConnect={(p) => setEdges(e => addEdge(p, e))} nodeTypes={{ custom: CustomNode }} colorMode={theme}>
            <Background color={theme === "dark" ? "#333" : "#ccc"} />
            <Controls className="!bg-surface !border-border-subtle" />
          </ReactFlow>
        </main>
      </div>
    </div>
  );
}
