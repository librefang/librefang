import { useState, useCallback, useRef, useEffect, type RefObject } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { Plus, X, AlertCircle } from "lucide-react";
import { authHeader } from "../api";
import { useUIStore } from "../lib/store";
import type { Terminal } from "@xterm/xterm";
import type { FitAddon } from "@xterm/addon-fit";

interface WindowInfo {
  id: string;
  index: number;
  name: string;
  active: boolean;
}

interface TerminalTabsProps {
  ws: WebSocket | null;
  tmuxAvailable: boolean;
  maxWindows: number;
  activeWindowId: string | null;
  onSwitchWindow: (windowId: string) => void;
  terminalRef: RefObject<Terminal | null>;
  fitAddonRef: RefObject<FitAddon | null>;
  shellName: string;
}

const WINDOW_NAME_RE = /^[A-Za-z0-9 ._-]{1,64}$/;

function useTmuxWindows(tmuxAvailable: boolean) {
  return useQuery<WindowInfo[]>({
    queryKey: ["terminal-windows"],
    queryFn: async () => {
      const res = await fetch("/api/terminal/windows", { headers: authHeader() });
      if (!res.ok) throw new Error("Failed to fetch windows");
      return res.json();
    },
    refetchInterval: 2000,
    enabled: tmuxAvailable,
  });
}

export function TerminalTabs({
  ws,
  tmuxAvailable,
  maxWindows,
  activeWindowId,
  onSwitchWindow,
  terminalRef,
  fitAddonRef,
  shellName,
}: TerminalTabsProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const { data: windows = [] } = useTmuxWindows(tmuxAvailable);
  const [showNameInput, setShowNameInput] = useState(false);
  const [newName, setNewName] = useState("");
  const [creating, setCreating] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const settleTimeoutsRef = useRef<ReturnType<typeof setTimeout>[]>([]);

  const handleTabClick = useCallback(
    (windowId: string) => {
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
    [ws, onSwitchWindow, terminalRef, fitAddonRef]
  );

  useEffect(() => {
    return () => {
      for (const id of settleTimeoutsRef.current) clearTimeout(id);
    };
  }, []);

  useEffect(() => {
    if (showNameInput) {
      inputRef.current?.focus();
      inputRef.current?.select();
    }
  }, [showNameInput]);

  useEffect(() => {
    if (activeWindowId !== null || windows.length === 0) return;
    const active = windows.find((w) => w.active);
    onSwitchWindow(active ? active.id : windows[0].id);
  }, [windows, activeWindowId, onSwitchWindow]);

  const handleCreate = useCallback(async () => {
    const name = newName.trim();
    if (name && !WINDOW_NAME_RE.test(name)) return;

    setCreating(true);
    setCreateError(null);
    try {
      const res = await fetch("/api/terminal/windows", {
        method: "POST",
        headers: { ...authHeader(), "Content-Type": "application/json" },
        body: JSON.stringify(name ? { name } : {}),
      });
      if (res.status === 429) {
        setCreateError(t("terminal.tabs.limit_reached"));
        return;
      }
      if (!res.ok) {
        setCreateError(t("terminal.tabs.create_failed"));
        return;
      }
      queryClient.invalidateQueries({ queryKey: ["terminal-windows"] });
      setNewName("");
      setShowNameInput(false);
    } finally {
      setCreating(false);
    }
  }, [newName, queryClient, t]);

  const addToast = useUIStore((s) => s.addToast);

  const handleCloseTab = useCallback(
    async (windowId: string, e: React.MouseEvent) => {
      e.stopPropagation();
      if (windows.length <= 1) return;
      try {
        const res = await fetch(
          `/api/terminal/windows/${encodeURIComponent(windowId)}`,
          { method: "DELETE", headers: authHeader() }
        );
        if (res.ok) {
          const isActive = activeWindowId === windowId;
          if (isActive) {
            const remaining = windows.filter((w) => w.id !== windowId);
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
          queryClient.invalidateQueries({ queryKey: ["terminal-windows"] });
        } else {
          console.error("Failed to delete terminal window", res.status);
          addToast(t("terminal.tabs.delete_failed"), "error");
        }
      } catch (err) {
        console.error("Failed to delete terminal window", err);
        addToast(t("terminal.tabs.delete_failed"), "error");
      }
    },
    [queryClient, windows, activeWindowId, ws, onSwitchWindow, addToast, t]
  );

  if (!tmuxAvailable) return null;

  const atLimit = windows.length >= maxWindows;

  return (
    <div className="flex items-center gap-1 px-2 py-1 bg-gray-900/80 border-b border-gray-700/50 overflow-x-auto">
      {windows.map((w) => (
        <button
          key={w.id}
          onClick={() => handleTabClick(w.id)}
          className={`px-3 py-1 rounded-t text-sm whitespace-nowrap transition-colors ${
            w.id === activeWindowId
              ? "bg-[#1a1a2e] text-white border-t border-x border-gray-600"
              : "text-gray-400 hover:text-gray-200 hover:bg-gray-800/50"
          }`}
        >
          <span className="flex items-center gap-1">
            {w.name || t("terminal.tabs.unnamed")}
            {windows.length > 1 && (
              <span
                role="button"
                tabIndex={0}
                onClick={(e) => handleCloseTab(w.id, e)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") handleCloseTab(w.id, e as unknown as React.MouseEvent);
                }}
                className="text-gray-500 hover:text-red-400 cursor-pointer"
              >
                <X className="h-3 w-3" />
              </span>
            )}
          </span>
        </button>
      ))}
      {showNameInput ? (
        <div className="flex items-center gap-1 ml-1">
          <input
            ref={inputRef}
            value={newName}
            onChange={(e) => {
              setNewName(e.target.value);
              setCreateError(null);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") handleCreate();
              if (e.key === "Escape") {
                setShowNameInput(false);
                setNewName("");
                setCreateError(null);
              }
            }}
            placeholder={t("terminal.tabs.new")}
            maxLength={64}
            className="px-2 py-0.5 text-sm bg-gray-800 text-white border border-gray-600 rounded w-32 focus:outline-none focus:border-blue-500"
            autoFocus
          />
          <button
            onClick={handleCreate}
            disabled={creating}
            className="text-xs text-green-400 hover:text-green-300"
          >
            OK
          </button>
          <button
            onClick={() => {
              setShowNameInput(false);
              setNewName("");
              setCreateError(null);
            }}
            className="text-xs text-gray-500 hover:text-gray-300"
          >
            <X className="h-3 w-3" />
          </button>
          {createError && (
            <span className="flex items-center gap-1 text-xs text-red-400">
              <AlertCircle className="h-3 w-3" />
              {createError}
            </span>
          )}
        </div>
      ) : (
        !atLimit && (
          <button
            onClick={() => {
              setNewName(`${shellName}_${windows.length + 1}`);
              setShowNameInput(true);
            }}
            className="p-1 text-gray-500 hover:text-gray-300 transition-colors"
            title={t("terminal.tabs.new")}
          >
            <Plus className="h-4 w-4" />
          </button>
        )
      )}
    </div>
  );
}
