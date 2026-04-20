import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown, ChevronRight, Loader2, CheckCircle, XCircle } from "lucide-react";
import type { AgentTool } from "../../api";
import { prettifyToolName } from "../../lib/string";

function formatToolContent(value: unknown): string {
  if (value == null) return "";
  if (typeof value === "string") {
    const trimmed = value.trim();
    if ((trimmed.startsWith("{") || trimmed.startsWith("[")) && trimmed.length > 1) {
      try {
        return JSON.stringify(JSON.parse(trimmed), null, 2);
      } catch { /* fall through */ }
    }
    return trimmed;
  }
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

export function ToolCallCard({ tool }: { tool: AgentTool }) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(tool.expanded ?? false);
  const isRunning = tool.running ?? false;
  const isError = tool.is_error ?? false;
  const hasResult = tool.result !== undefined && tool.result !== null;

  const inputText = formatToolContent(tool.input);
  const resultText = formatToolContent(tool.result);

  return (
    <div className="rounded-lg border border-border-subtle/50 bg-main/50 overflow-hidden my-1.5">
      {/* Header — always visible */}
      <button
        onClick={() => setExpanded(!expanded)}
        className="flex items-center gap-2 w-full px-3 py-2 text-left hover:bg-surface-hover/50 transition-colors"
      >
        {expanded
          ? <ChevronDown className="w-3 h-3 text-text-dim/60 shrink-0" />
          : <ChevronRight className="w-3 h-3 text-text-dim/60 shrink-0" />
        }
        <span className="text-[10px] font-bold text-brand uppercase tracking-wider truncate">
          {prettifyToolName(tool.name)}
        </span>
        <span className="ml-auto shrink-0">
          {isRunning ? (
            <Loader2 className="w-3 h-3 text-brand animate-spin" />
          ) : isError ? (
            <XCircle className="w-3 h-3 text-error" />
          ) : hasResult ? (
            <CheckCircle className="w-3 h-3 text-success" />
          ) : null}
        </span>
      </button>

      {/* Expanded content */}
      {expanded && (
        <div className="px-3 pb-2.5 space-y-2 border-t border-border-subtle/30">
          {/* Input */}
          {inputText && (
            <div className="mt-2">
              <span className="text-[9px] font-semibold text-text-dim/50 uppercase tracking-wider">{t("chat.tool_input", { defaultValue: "Input" })}</span>
              <pre className="mt-1 text-[11px] leading-relaxed text-text-dim whitespace-pre-wrap break-words overflow-y-auto max-h-40 rounded-md bg-main px-2.5 py-2 border border-border-subtle/30 font-mono">
                {inputText}
              </pre>
            </div>
          )}
          {/* Result */}
          {isRunning && !hasResult && (
            <div className="flex items-center gap-2 mt-2 text-[11px] text-text-dim/60">
              <Loader2 className="w-3 h-3 animate-spin" />
              <span>{t("chat.tool_running", { defaultValue: "Running…" })}</span>
            </div>
          )}
          {hasResult && (
            <div>
              <span className={`text-[9px] font-semibold uppercase tracking-wider ${isError ? "text-error/60" : "text-text-dim/50"}`}>
                {isError ? t("chat.tool_error", { defaultValue: "Error" }) : t("chat.tool_result", { defaultValue: "Result" })}
              </span>
              <pre className={`mt-1 text-[11px] leading-relaxed whitespace-pre-wrap break-words overflow-y-auto max-h-48 rounded-md px-2.5 py-2 border font-mono ${
                isError
                  ? "bg-error/5 border-error/20 text-error"
                  : "bg-main border-border-subtle/30 text-text-dim"
              }`}>
                {resultText}
              </pre>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
