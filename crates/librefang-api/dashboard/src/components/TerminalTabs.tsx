import {
  useState,
  useCallback,
  useRef,
  useEffect,
  useMemo,
  type RefObject,
} from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { Plus, X, AlertCircle } from "lucide-react";
import { authHeader } from "../api";
import { useUIStore } from "../lib/store";
import type { Terminal } from "@xterm/xterm";
import type { FitAddon } from "@xterm/addon-fit";
import { Modal } from "./ui/Modal";
import { Button } from "./ui/Button";

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
    refetchInterval: 10000,
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
  const [isCreateOpen, setIsCreateOpen] = useState(false);
  const [newName, setNewName] = useState("");
  const [creating, setCreating] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const settleTimeoutsRef = useRef<ReturnType<typeof setTimeout>[]>([]);
  const windowsRef = useRef<WindowInfo[]>([]);

  useEffect(() => {
    windowsRef.current = windows;
  }, [windows]);

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
    if (activeWindowId !== null || windows.length === 0) return;
    const active = windows.find((w) => w.active);
    onSwitchWindow(active ? active.id : windows[0].id);
  }, [windows, activeWindowId, onSwitchWindow]);

  // Auto-focus the input when the modal opens.
  useEffect(() => {
    if (!isCreateOpen) return;
    const tid = setTimeout(() => {
      inputRef.current?.focus();
      inputRef.current?.select();
    }, 50);
    return () => clearTimeout(tid);
  }, [isCreateOpen]);

  const suggestedName = useMemo(() => {
    const base = shellName || "sh";
    const used = new Set(windows.map((w) => w.name));
    for (let i = windows.length + 1; i <= maxWindows + 1; i++) {
      const candidate = `${base}-${i}`;
      if (!used.has(candidate)) return candidate;
    }
    return `${base}-${Date.now().toString().slice(-4)}`;
  }, [shellName, windows, maxWindows]);

  const addToast = useUIStore((s) => s.addToast);

  const openCreateModal = useCallback(() => {
    setNewName(suggestedName);
    setCreateError(null);
    setIsCreateOpen(true);
  }, [suggestedName]);

  const closeCreateModal = useCallback(() => {
    if (creating) return;
    setIsCreateOpen(false);
    setNewName("");
    setCreateError(null);
  }, [creating]);

  const trimmedName = newName.trim();
  const nameValid = trimmedName === "" || WINDOW_NAME_RE.test(trimmedName);

  const handleCreate = useCallback(async () => {
    const name = newName.trim();
    if (name !== "" && !WINDOW_NAME_RE.test(name)) {
      setCreateError(t("terminal.tabs.name_invalid"));
      return;
    }

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
      setIsCreateOpen(false);
      addToast(t("terminal.tabs.create_success"), "success");
    } catch {
      setCreateError(t("terminal.tabs.create_failed"));
    } finally {
      setCreating(false);
    }
  }, [newName, queryClient, t, addToast]);

  const handleCloseTab = useCallback(
    async (windowId: string, e: React.MouseEvent) => {
      e.stopPropagation();
      const currentWindows = windowsRef.current;
      if (currentWindows.length <= 1) return;
      try {
        const res = await fetch(
          `/api/terminal/windows/${encodeURIComponent(windowId)}`,
          { method: "DELETE", headers: authHeader() }
        );
        if (res.ok) {
          const isActive = activeWindowId === windowId;
          if (isActive) {
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
    [queryClient, activeWindowId, ws, onSwitchWindow, addToast, t]
  );

  if (!tmuxAvailable) return null;

  const atLimit = windows.length >= maxWindows;

  return (
    <>
      <div className="flex items-center gap-1 px-2 py-1 bg-gray-900/80 border-b border-gray-700/50 overflow-x-auto shrink-0">
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
        <button
          onClick={openCreateModal}
          disabled={atLimit}
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

      <Modal
        isOpen={isCreateOpen}
        onClose={closeCreateModal}
        title={t("terminal.tabs.create_title")}
        size="sm"
      >
        <div className="p-5 space-y-3">
          <label className="block text-sm">
            <span className="block text-text-dim mb-1.5">
              {t("terminal.tabs.name_label")}
            </span>
            <input
              ref={inputRef}
              value={newName}
              onChange={(e) => {
                setNewName(e.target.value);
                setCreateError(null);
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !creating && nameValid) {
                  e.preventDefault();
                  void handleCreate();
                }
              }}
              placeholder={t("terminal.tabs.name_placeholder")}
              maxLength={64}
              aria-invalid={!nameValid}
              className={`w-full px-3 py-2 text-sm bg-gray-800 text-white border rounded-lg focus:outline-none transition-colors ${
                !nameValid || createError
                  ? "border-red-500/70 focus:border-red-500"
                  : "border-gray-600 focus:border-blue-500"
              }`}
            />
          </label>

          <p className="text-xs text-text-dim leading-relaxed">
            {t("terminal.tabs.name_hint")}
          </p>

          {!nameValid && (
            <div className="flex items-start gap-1.5 text-xs text-red-400">
              <AlertCircle className="h-3.5 w-3.5 shrink-0 mt-0.5" />
              <span>{t("terminal.tabs.name_invalid")}</span>
            </div>
          )}

          {createError && nameValid && (
            <div className="flex items-start gap-1.5 text-xs text-red-400">
              <AlertCircle className="h-3.5 w-3.5 shrink-0 mt-0.5" />
              <span>{createError}</span>
            </div>
          )}

          <div className="flex items-center justify-between pt-2">
            <span className="text-xs text-text-dim tabular-nums">
              {t("terminal.tabs.counter", {
                used: windows.length,
                total: maxWindows,
              })}
            </span>
            <div className="flex items-center gap-2">
              <Button
                variant="secondary"
                onClick={closeCreateModal}
                disabled={creating}
              >
                {t("common.cancel")}
              </Button>
              <Button
                onClick={() => void handleCreate()}
                disabled={creating || !nameValid || atLimit}
              >
                {creating
                  ? t("terminal.tabs.creating")
                  : t("terminal.tabs.create")}
              </Button>
            </div>
          </div>
        </div>
      </Modal>
    </>
  );
}
