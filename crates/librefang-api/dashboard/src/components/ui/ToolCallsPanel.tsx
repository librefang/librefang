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

  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        aria-haspopup="dialog"
        className="shrink-0 w-full flex items-center gap-2 px-3 py-2 text-left border-t border-border-subtle bg-surface hover:bg-surface-hover transition-colors"
      >
        <Wrench className="w-3.5 h-3.5 text-brand shrink-0" aria-hidden="true" />
        <span className="text-[11px] font-bold uppercase tracking-wider text-text-dim shrink-0">
          {t("chat.tool_calls", { count: stats.total, defaultValue: "{{count}} tool calls" })}
        </span>
        {stats.running > 0 && (
          <span className="inline-flex items-center gap-1 text-[10px] font-medium text-brand shrink-0" title={t("chat.tool_running", { defaultValue: "Running…" })}>
            <Loader2 className="w-3 h-3 animate-spin" aria-hidden="true" />
            {stats.running}
          </span>
        )}
        {stats.errors > 0 && (
          <span className="inline-flex items-center gap-1 text-[10px] font-medium text-error shrink-0" title={t("chat.tool_error", { defaultValue: "Error" })}>
            <AlertCircle className="w-3 h-3" aria-hidden="true" />
            {stats.errors}
          </span>
        )}
        {lastTool && (
          <span className="text-[10px] text-text-dim/70 truncate min-w-0">
            {prettifyToolName(lastTool.name)}
          </span>
        )}
        <ChevronRight className="ml-auto shrink-0 w-3.5 h-3.5 text-text-dim/60" aria-hidden="true" />
      </button>

      <Modal
        isOpen={open}
        onClose={() => setOpen(false)}
        title={t("chat.tool_calls", { count: stats.total, defaultValue: "{{count}} tool calls" })}
        size="2xl"
      >
        <div className="px-4 py-3">
          {tools.map((tool, i) => (
            <ToolCallCard
              key={tool._call_id ?? `${tool.name}-${i}`}
              tool={tool}
            />
          ))}
        </div>
      </Modal>
    </>
  );
});
