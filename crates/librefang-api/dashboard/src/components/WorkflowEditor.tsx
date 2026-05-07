import React, { useCallback, useState } from "react";
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
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { useUIStore, createClientId } from "../lib/store";
import { ChevronLeft } from "lucide-react";

interface CustomNodeProps {
  data: { label: string; description: string; nodeType?: string };
  type: string;
}

interface WorkflowEditorProps {
  initialNodes?: Node[];
  initialEdges?: Edge[];
  onSave?: (nodes: Node[], edges: Edge[]) => void;
  onClose?: () => void;
  title?: string;
}

const nodeTypesConfig = [
  { type: "start", color: "var(--success-color)", icon: "S" },
  { type: "end", color: "var(--error-color)", icon: "E" },
  { type: "agent", color: "var(--brand-color)", icon: "A" },
  { type: "condition", color: "var(--success-color)", icon: "?" },
  { type: "webhook", color: "var(--brand-color)", icon: "W" },
  { type: "schedule", color: "var(--warning-color)", icon: "C" },
  { type: "channel", color: "var(--accent-color)", icon: "M" },
];

const CustomNode = React.memo(function CustomNode({ data, type }: CustomNodeProps) {
  const config = nodeTypesConfig.find(c => c.type === (data.nodeType ?? type)) ?? nodeTypesConfig[2];
  return (
    <div className="rounded-lg border-2 border-border-subtle bg-surface shadow-lg min-w-[150px] overflow-hidden">
      <div className="flex items-center gap-2 px-3 py-2" style={{ backgroundColor: config.color }}>
        <span className="text-sm font-bold text-white">{config.icon}</span>
        <span className="text-sm font-bold text-white truncate">{data.label}</span>
      </div>
      <div className="px-3 py-2 bg-surface">
        <p className="text-[10px] font-medium text-text-dim leading-tight">{data.description}</p>
      </div>
    </div>
  );
});

const nodeTypes = { custom: CustomNode };

const triggerTypes = nodeTypesConfig.filter(n =>
  ["start", "schedule", "webhook", "channel"].includes(n.type)
);
const logicTypes = nodeTypesConfig.filter(n =>
  ["agent", "condition"].includes(n.type)
);

export function WorkflowEditor({ initialNodes = [], initialEdges = [], onSave, onClose, title }: WorkflowEditorProps) {
  const { t } = useTranslation();
  const theme = useUIStore((s) => s.theme);
  const [nodes, setNodes, onNodesChange] = useNodesState(initialNodes);
  const [edges, setEdges, onEdgesChange] = useEdgesState(initialEdges);
  const [showClearConfirm, setShowClearConfirm] = useState(false);

  const onConnect = useCallback((params: Connection) => setEdges((eds) => addEdge(params, eds)), [setEdges]);

  const addNode = useCallback((type: string) => {
    const newNode: Node = {
      id: `${type}-${createClientId()}`,
      type: "custom",
      position: { x: 0, y: 0 },
      data: { label: t(`canvas.nodes.${type}`), description: t(`canvas.nodes.${type}_desc`), nodeType: type }
    };
    setNodes((nds) => [...nds, { ...newNode, position: { x: 100 + nds.length * 40, y: 100 + nds.length * 40 } }]);
  }, [setNodes, t]);

  return (
    <div className="fixed inset-0 z-[100] flex flex-col bg-main animate-in fade-in duration-300">
      <header className="flex items-center justify-between border-b border-border-subtle bg-surface px-6 py-4 shadow-sm">
        <div className="flex items-center gap-4">
          <button onClick={onClose} className="p-2 rounded-xl hover:bg-surface-hover text-text-dim transition-colors">
            <ChevronLeft className="h-5 w-5" />
          </button>
          <div>
            <h2 className="text-lg font-black tracking-tight">{title || t("canvas.title")}</h2>
            <p className="text-[10px] font-bold text-text-dim uppercase tracking-widest">{t("canvas.subtitle")}</p>
          </div>
        </div>
        <div className="flex gap-3 items-center">
          {showClearConfirm ? (
            <>
              <span className="text-xs text-text-dim">{t("canvas.clear_confirm")}</span>
              <button onClick={() => { setNodes([]); setShowClearConfirm(false); }} className="px-3 py-1.5 rounded-xl bg-error text-white text-sm font-bold">
                {t("common.confirm")}
              </button>
              <button onClick={() => setShowClearConfirm(false)} className="px-3 py-1.5 rounded-xl border border-border-subtle text-sm font-bold text-text-dim">
                {t("common.cancel")}
              </button>
            </>
          ) : (
            <button onClick={() => setShowClearConfirm(true)} className="px-4 py-2 rounded-xl border border-border-subtle text-sm font-bold text-text-dim hover:text-brand transition-colors">
              {t("common.clear")}
            </button>
          )}
          <button onClick={() => onSave?.(nodes, edges)} className="px-8 py-2 rounded-xl bg-brand text-white text-sm font-black shadow-lg shadow-brand/20 hover:opacity-90 transition-opacity">
            {t("common.save")}
          </button>
        </div>
      </header>

      <div className="flex flex-1 overflow-hidden">
        <aside className="w-64 border-r border-border-subtle bg-surface p-4 overflow-y-auto">
          <h3 className="text-[10px] font-black uppercase text-text-dim/60 mb-6">{t("canvas.node_library")}</h3>
          <div className="space-y-6">
            <section>
              <p className="text-[10px] font-bold text-brand uppercase mb-3">{t("canvas.triggers")}</p>
              <div className="grid gap-2">
                {triggerTypes.map(n => (
                  <button key={n.type} onClick={() => addNode(n.type)} className="flex items-center gap-3 p-3 rounded-xl border border-border-subtle bg-main/50 hover:border-brand transition-colors text-left group">
                    <div className="h-8 w-8 rounded-lg flex items-center justify-center text-white text-xs font-black shadow-sm" style={{ backgroundColor: n.color }}>{n.icon}</div>
                    <span className="text-xs font-bold group-hover:text-brand">{t(`canvas.nodes.${n.type}`)}</span>
                  </button>
                ))}
              </div>
            </section>
            <section>
              <p className="text-[10px] font-bold text-brand uppercase mb-3">{t("canvas.logic_actions")}</p>
              <div className="grid gap-2">
                {logicTypes.map(n => (
                  <button key={n.type} onClick={() => addNode(n.type)} className="flex items-center gap-3 p-3 rounded-xl border border-border-subtle bg-main/50 hover:border-brand transition-colors text-left group">
                    <div className="h-8 w-8 rounded-lg flex items-center justify-center text-white text-xs font-black shadow-sm" style={{ backgroundColor: n.color }}>{n.icon}</div>
                    <span className="text-xs font-bold group-hover:text-brand">{t(`canvas.nodes.${n.type}`)}</span>
                  </button>
                ))}
              </div>
            </section>
          </div>
        </aside>
        <main className="flex-1 relative">
          <ReactFlow nodes={nodes} edges={edges} onNodesChange={onNodesChange} onEdgesChange={onEdgesChange} onConnect={onConnect} nodeTypes={nodeTypes} colorMode={theme}>
            <Background color={theme === "dark" ? "rgba(255,255,255,0.05)" : "rgba(0,0,0,0.05)"} />
            <Controls className="bg-surface! border-border-subtle! shadow-xl!" />
            <MiniMap nodeStrokeColor="var(--border-color)" maskColor={theme === "dark" ? "rgba(0,0,0,0.6)" : "rgba(255,255,255,0.6)"} className="bg-surface! border-border-subtle! rounded-xl!" />
          </ReactFlow>
        </main>
      </div>
    </div>
  );
}
