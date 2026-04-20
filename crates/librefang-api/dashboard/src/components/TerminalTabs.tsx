import {
  useState,
  useCallback,
  useRef,
  useEffect,
  type RefObject,
} from "react";
import { useTranslation } from "react-i18next";
import { Plus, X } from "lucide-react";
import { useUIStore } from "../lib/store";
import { useTerminalWindows } from "../lib/queries/terminal";
import {
  useCreateTerminalWindow,
  useRenameTerminalWindow,
  useDeleteTerminalWindow,
} from "../lib/mutations/terminal";
import { ApiError, type TerminalWindow } from "../lib/http/client";
import type { Terminal } from "@xterm/xterm";
import type { FitAddon } from "@xterm/addon-fit";

interface TerminalTabsProps {
  ws: WebSocket | null;
  tmuxAvailable: boolean;
  maxWindows: number;
  displayedActiveWindowId: string | null;
  onSwitchWindow: (windowId: string) => void;
  terminalRef: RefObject<Terminal | null>;
  fitAddonRef: RefObject<FitAddon | null>;
}

const WINDOW_NAME_RE = /^[A-Za-z0-9 ._-]{1,64}$/;

export function TerminalTabs({
  ws,
  tmuxAvailable,
  maxWindows,
  displayedActiveWindowId,
  onSwitchWindow,
  terminalRef,
  fitAddonRef,
}: TerminalTabsProps) {
  const { t } = useTranslation();
  const { data: windows = [] } = useTerminalWindows({ enabled: tmuxAvailable });
  const createMutation = useCreateTerminalWindow();
  const renameMutation = useRenameTerminalWindow();
  const deleteMutation = useDeleteTerminalWindow();
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editValue, setEditValue] = useState("");
  const editInputRef = useRef<HTMLInputElement>(null);
  const settleTimeoutsRef = useRef<ReturnType<typeof setTimeout>[]>([]);
  const windowsRef = useRef<TerminalWindow[]>([]);

  useEffect(() => {
    windowsRef.current = windows;
  }, [windows]);

  const addToast = useUIStore((s) => s.addToast);

  const handleTabClick = useCallback(
    (windowId: string) => {
      if (editingId === windowId) return;
      if (!ws || ws.readyState !== WebSocket.OPEN) return;
      ws.send(JSON.stringify({ type: "switch_window", window: windowId }));
      onSwitchWindow(windowId);

      for (const id of settleTimeoutsRef.current) clearTimeout(id);
      settleTimeoutsRef.current = [];

      const tid = setTimeout(() => {
        const term = terminalRef.current;
        const fit = fitAddonRef.current;
        if (!term || !fit || !ws || ws.readyState !== WebSocket.OPEN) return;
        fit.fit();
        ws.send(JSON.stringify({ type: "resize", cols: term.cols, rows: term.rows }));
      }, 100);
      settleTimeoutsRef.current = [tid];
    },
    [ws, onSwitchWindow, terminalRef, fitAddonRef, editingId]
  );

  useEffect(() => {
    return () => {
      for (const id of settleTimeoutsRef.current) clearTimeout(id);
    };
  }, []);

  useEffect(() => {
    if (displayedActiveWindowId !== null || windows.length === 0) return;
    const active = windows.find((w) => w.active);
    onSwitchWindow(active ? active.id : windows[0].id);
  }, [windows, displayedActiveWindowId, onSwitchWindow]);

  useEffect(() => {
    if (editingId) {
      const tid = setTimeout(() => {
        editInputRef.current?.focus();
        editInputRef.current?.select();
      }, 0);
      return () => clearTimeout(tid);
    }
  }, [editingId]);

  const handleCreate = useCallback(async () => {
    if (createMutation.isPending) return;
    try {
      await createMutation.mutateAsync({});
    } catch (err) {
      if (err instanceof ApiError && err.status === 429) {
        addToast(t("terminal.tabs.limit_reached"), "error");
      } else {
        addToast(t("terminal.tabs.create_failed"), "error");
      }
    }
  }, [createMutation, addToast, t]);

  const startRename = useCallback((w: TerminalWindow) => {
    setEditingId(w.id);
    setEditValue(w.name);
  }, []);

  const cancelRename = useCallback(() => {
    setEditingId(null);
    setEditValue("");
  }, []);

  const commitRename = useCallback(() => {
    if (!editingId) return;
    const name = editValue.trim();
    const current = windowsRef.current.find((w) => w.id === editingId);
    // Nothing changed or empty → just close the editor.
    if (!current || name === "" || name === current.name) {
      cancelRename();
      return;
    }
    if (!WINDOW_NAME_RE.test(name)) {
      addToast(t("terminal.tabs.name_invalid"), "error");
      return;
    }
    const idToRename = editingId;
    cancelRename();
    renameMutation.mutate(
      { windowId: idToRename, name },
      {
        onError: () => addToast(t("terminal.tabs.rename_failed"), "error"),
      },
    );
  }, [editingId, editValue, renameMutation, cancelRename, addToast, t]);

  const handleCloseTab = useCallback(
    async (windowId: string, e: React.MouseEvent | React.KeyboardEvent) => {
      e.stopPropagation();
      const currentWindows = windowsRef.current;
      if (currentWindows.length <= 1) return;
      try {
        await deleteMutation.mutateAsync(windowId);
        if (displayedActiveWindowId === windowId) {
          const remaining = currentWindows.filter((w) => w.id !== windowId);
          if (remaining.length > 0) {
            const next = remaining[0];
            if (ws && ws.readyState === WebSocket.OPEN) {
              ws.send(JSON.stringify({ type: "switch_window", window: next.id }));
            }
            onSwitchWindow(next.id);
          } else {
            onSwitchWindow("");
          }
        }
      } catch (err) {
        console.error("Failed to delete terminal window", err);
        addToast(t("terminal.tabs.delete_failed"), "error");
      }
    },
    [deleteMutation, displayedActiveWindowId, ws, onSwitchWindow, addToast, t]
  );

  if (!tmuxAvailable) return null;

  const atLimit = windows.length >= maxWindows;

  return (
    <div className="flex items-center gap-1 px-2 py-1 bg-gray-900/80 border-b border-gray-700/50 overflow-x-auto shrink-0">
      {windows.map((w) => {
        const isActive = w.id === displayedActiveWindowId;
        const isEditing = editingId === w.id;
        return (
          <div
            key={w.id}
            onClick={() => handleTabClick(w.id)}
            onDoubleClick={(e) => {
              e.stopPropagation();
              startRename(w);
            }}
            onAuxClick={(e) => {
              // Middle-click closes, matching VS Code / browser tab behavior.
              if (e.button === 1 && windows.length > 1) {
                e.preventDefault();
                void handleCloseTab(w.id, e);
              }
            }}
            title={isEditing ? undefined : t("terminal.tabs.rename_hint")}
            className={`group flex items-center gap-1 px-3 py-1 rounded-t text-sm whitespace-nowrap transition-colors cursor-pointer select-none ${
              isActive
                ? "bg-[#1a1a2e] text-white border-t border-x border-gray-600"
                : "text-gray-400 hover:text-gray-200 hover:bg-gray-800/50"
            }`}
          >
            {isEditing ? (
              <input
                ref={editInputRef}
                value={editValue}
                onChange={(e) => setEditValue(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    void commitRename();
                  } else if (e.key === "Escape") {
                    e.preventDefault();
                    cancelRename();
                  }
                  e.stopPropagation();
                }}
                onBlur={() => void commitRename()}
                onClick={(e) => e.stopPropagation()}
                onDoubleClick={(e) => e.stopPropagation()}
                maxLength={64}
                aria-label={t("terminal.tabs.name_label")}
                className="bg-gray-800 text-white text-sm px-1 py-0 rounded border border-blue-500 outline-none w-32"
              />
            ) : (
              <span>{w.name || t("terminal.tabs.unnamed")}</span>
            )}
            {!isEditing && windows.length > 1 && (
              <span
                role="button"
                tabIndex={0}
                aria-label={t("terminal.tabs.close")}
                onClick={(e) => handleCloseTab(w.id, e)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") handleCloseTab(w.id, e);
                }}
                className={`text-gray-500 hover:text-red-400 cursor-pointer transition-opacity ${
                  isActive ? "opacity-100" : "opacity-0 group-hover:opacity-100"
                }`}
              >
                <X className="h-3 w-3" />
              </span>
            )}
          </div>
        );
      })}
      <button
        onClick={() => void handleCreate()}
        disabled={atLimit || createMutation.isPending}
        aria-label={t("terminal.tabs.new")}
        className="p-1 text-gray-500 hover:text-gray-300 transition-colors disabled:opacity-40 disabled:cursor-not-allowed disabled:hover:text-gray-500"
        title={
          atLimit
            ? t("terminal.tabs.limit_reached")
            : t("terminal.tabs.new")
        }
      >
        <Plus className="h-4 w-4" />
      </button>
      <span className="ml-auto pr-1 text-xs text-gray-500 shrink-0 tabular-nums">
        {t("terminal.tabs.counter", {
          used: windows.length,
          total: maxWindows,
        })}
      </span>
    </div>
  );
}
