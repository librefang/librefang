import React, { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronRight, Wrench, Loader2, AlertCircle } from "lucide-react";
import type { AgentTool } from "../../api";
import { ToolCallCard } from "./ToolCallCard";
import { Modal } from "./Modal";
import { prettifyToolName } from "../../lib/string";

type PanelTool = AgentTool & { _call_id?: string };

interface ToolCallsPanelProps {
  tools: ReadonlyArray<PanelTool>;
}

function stableToolKey(tool: PanelTool, i: number): string {
  if (tool._call_id) return tool._call_id;
  const raw = `${tool.name ?? ""}:${JSON.stringify(tool.input ?? "")}`;
  let h = 0;
  for (let j = 0; j < raw.length; j++) {
    h = ((h << 5) - h + raw.charCodeAt(j)) | 0;
  }
  return `${tool.name ?? "tool"}-${(h >>> 0).toString(36)}-${i}`;
}

export const ToolCallsPanel = React.memo(function ToolCallsPanel({ tools }: ToolCallsPanelProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);

  const stats = useMemo(() => {
    let running = 0;
    let errors = 0;
    for (const tool of tools) {
      if (tool.running) running += 1;
      if (tool.is_error) errors += 1;
    }
    return { total: tools.length, running, errors };
  }, [tools]);

  if (tools.length === 0) return null;

  const lastTool = tools[tools.length - 1];
  const titleText = t("chat.tool_calls", { count: stats.total, defaultValue: "{{count}} tool calls" });

  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        aria-haspopup="dialog"
        aria-expanded={open}
        className="inline-flex items-center gap-1.5 max-w-full px-2 py-1 rounded-md border border-border-subtle bg-surface text-[10px] font-medium text-text-dim hover:text-text hover:border-border transition-colors"
      >
        <Wrench className="w-3 h-3 text-brand shrink-0" aria-hidden="true" />
        <span className="shrink-0 font-bold uppercase tracking-wider">
          {titleText}
        </span>
        {stats.running > 0 && (
          <span className="inline-flex items-center gap-0.5 text-brand shrink-0" title={t("chat.tool_running", { defaultValue: "Running…" })}>
            <Loader2 className="w-3 h-3 animate-spin" aria-hidden="true" />
            {stats.running}
          </span>
        )}
        {stats.errors > 0 && (
          <span className="inline-flex items-center gap-0.5 text-error shrink-0" title={t("chat.tool_error", { defaultValue: "Error" })}>
            <AlertCircle className="w-3 h-3" aria-hidden="true" />
            {stats.errors}
          </span>
        )}
        {lastTool && (
          <span className="truncate text-text-dim/70 min-w-0">
            · {prettifyToolName(lastTool.name)}
          </span>
        )}
        <ChevronRight className="shrink-0 w-3 h-3 text-text-dim/60" aria-hidden="true" />
      </button>

      <Modal
        isOpen={open}
        onClose={() => setOpen(false)}
        title={titleText}
        size="2xl"
      >
        <div className="px-4 py-3">
          {tools.map((tool, idx) => (
            <ToolCallCard
              key={stableToolKey(tool, idx)}
              tool={tool}
            />
          ))}
        </div>
      </Modal>
    </>
  );
});
