import {
  useState,
  useCallback,
  useRef,
  useEffect,
  useMemo,
  type RefObject,
} from "react";
import { useTranslation } from "react-i18next";
import { Plus, X, HelpCircle, ChevronLeft, ChevronRight } from "lucide-react";
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

// Must match the server-side MAX_COLS / MAX_ROWS constants in routes/terminal.rs.
const TERM_MIN_COLS = 1;
const TERM_MAX_COLS = 1000;
const TERM_MIN_ROWS = 1;
const TERM_MAX_ROWS = 500;
const SETTLE_TIMEOUT_MS = 100;

function clampTermSize(cols: number, rows: number): { cols: number; rows: number } | null {
  const c = Math.max(TERM_MIN_COLS, Math.min(TERM_MAX_COLS, Math.floor(cols)));
  const r = Math.max(TERM_MIN_ROWS, Math.min(TERM_MAX_ROWS, Math.floor(rows)));
  if (!Number.isFinite(c) || !Number.isFinite(r)) return null;
  return { cols: c, rows: r };
}

interface TerminalTabsProps {
  ws: WebSocket | null;
  tmuxAvailable: boolean;
  maxWindows: number;
  displayedActiveWindowId: string | null;
  onSwitchWindow: (windowId: string | null) => void;
  terminalRef: RefObject<Terminal | null>;
  fitAddonRef: RefObject<FitAddon | null>;
}

// Match backend validate_window_name: any Unicode except control chars and '|', 1–64 chars.
const WINDOW_NAME_RE = /^[^|\x00-\x1f\x7f]{1,64}$/u;

const ORDER_KEY = "terminal.tabOrder";

function loadOrder(): string[] {
  try { return JSON.parse(localStorage.getItem(ORDER_KEY) ?? "[]"); } catch (e) { console.warn("Failed to parse tab order from localStorage:", e); return []; }
}
function saveOrder(ids: string[]) {
  localStorage.setItem(ORDER_KEY, JSON.stringify(ids));
}

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
  const [tabOrder, setTabOrder] = useState<string[]>(loadOrder);
  const [dragId, setDragId] = useState<string | null>(null);
  const [editValue, setEditValue] = useState("");
  const editInputRef = useRef<HTMLInputElement>(null);
  const settleTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const editingIdRef = useRef<string | null>(null);
  const windowsRef = useRef<TerminalWindow[]>([]);

  useEffect(() => {
    windowsRef.current = windows;
  }, [windows]);

  useEffect(() => {
    editingIdRef.current = editingId;
  }, [editingId]);

  // Keep tab order in sync with server windows list.
  // Skip when windows is empty (query still loading) to avoid wiping persisted order.
  useEffect(() => {
    if (windows.length === 0) return;
    setTabOrder(prev => {
      const existing = new Set(windows.map(w => w.id));
      const filtered = prev.filter(id => existing.has(id));
      const filteredSet = new Set(filtered);
      const newIds = windows.map(w => w.id).filter(id => !filteredSet.has(id));
      const next = [...filtered, ...newIds];
      saveOrder(next);
      return next;
    });
  }, [windows]);

  const addToast = useUIStore((s) => s.addToast);

  const handleTabClick = useCallback(
    (windowId: string) => {
      if (editingIdRef.current === windowId) return;
      if (!ws || ws.readyState !== WebSocket.OPEN) return;
      ws.send(JSON.stringify({ type: "switch_window", window: windowId }));
      onSwitchWindow(windowId);

      if (settleTimeoutRef.current !== null) {
        clearTimeout(settleTimeoutRef.current);
      }

      settleTimeoutRef.current = setTimeout(() => {
        const term = terminalRef.current;
        const fit = fitAddonRef.current;
        if (!term || !fit || !ws || ws.readyState !== WebSocket.OPEN) return;
        fit.fit();
        const size = clampTermSize(term.cols, term.rows);
        if (size) ws.send(JSON.stringify({ type: "resize", ...size }));
      }, SETTLE_TIMEOUT_MS);
    },
    [ws, onSwitchWindow, terminalRef, fitAddonRef]
  );

  useEffect(() => {
    return () => {
      if (settleTimeoutRef.current !== null) {
        clearTimeout(settleTimeoutRef.current);
      }
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
      }
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
            onSwitchWindow(null);
          }
        }
      } catch {
        addToast(t("terminal.tabs.delete_failed"), "error");
      }
    },
    [deleteMutation, displayedActiveWindowId, ws, onSwitchWindow, addToast, t]
  );

  // ── Tab overflow indicators ───────────────────────────────────────────────
  const tabScrollRef = useRef<HTMLDivElement>(null);
  const [showLeftArrow, setShowLeftArrow] = useState(false);
  const [showRightArrow, setShowRightArrow] = useState(false);

  const updateOverflowArrows = useCallback(() => {
    const el = tabScrollRef.current;
    if (!el) return;
    setShowLeftArrow(el.scrollLeft > 0);
    setShowRightArrow(el.scrollLeft + el.clientWidth < el.scrollWidth - 1);
  }, []);

  useEffect(() => {
    const el = tabScrollRef.current;
    if (!el) return;
    updateOverflowArrows();
    el.addEventListener("scroll", updateOverflowArrows, { passive: true });
    const ro = new ResizeObserver(updateOverflowArrows);
    ro.observe(el);
    return () => {
      el.removeEventListener("scroll", updateOverflowArrows);
      ro.disconnect();
    };
  }, [updateOverflowArrows]);

  const [showHelp, setShowHelp] = useState(false);
  const helpRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!showHelp) return;
    const onPointerDown = (e: PointerEvent) => {
      if (helpRef.current && !helpRef.current.contains(e.target as Node)) {
        setShowHelp(false);
      }
    };
    document.addEventListener("pointerdown", onPointerDown);
    return () => document.removeEventListener("pointerdown", onPointerDown);
  }, [showHelp]);

  const sortedWindows = useMemo(() => {
    const orderMap = new Map(tabOrder.map((id, i) => [id, i]));
    return [...windows].sort((a, b) => {
      const ai = orderMap.get(a.id) ?? Infinity;
      const bi = orderMap.get(b.id) ?? Infinity;
      return ai - bi;
    });
  }, [windows, tabOrder]);

  // Re-check arrows when the windows list changes (tabs added/removed).
  useEffect(() => {
    updateOverflowArrows();
  }, [sortedWindows, updateOverflowArrows]);

  if (!tmuxAvailable) return null;

  const atLimit = windows.length >= maxWindows;
  const nearLimit = windows.length >= Math.max(2, maxWindows - 2);

  return (
    <div className="flex items-end bg-[#161b22] border-b border-gray-700/60 shrink-0">
      <div className="relative flex-1 min-w-0">
        {showLeftArrow && (
          <div className="pointer-events-none absolute left-0 inset-y-0 flex items-center pl-0.5 z-10">
            <ChevronLeft className="h-3 w-3 text-gray-500" />
          </div>
        )}
        {showRightArrow && (
          <div className="pointer-events-none absolute right-0 inset-y-0 flex items-center pr-0.5 z-10">
            <ChevronRight className="h-3 w-3 text-gray-500" />
          </div>
        )}
      <div ref={tabScrollRef} role="tablist" className="flex items-end gap-0.5 px-2 pt-1.5 overflow-x-auto flex-1 scrollbar-thin">
      {sortedWindows.map((w) => {
        const isActive = w.id === displayedActiveWindowId;
        const isEditing = editingId === w.id;
        return (
          <div
            key={w.id}
            role="tab"
            tabIndex={0}
            aria-selected={isActive}
            onClick={() => handleTabClick(w.id)}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === " ") { e.preventDefault(); handleTabClick(w.id); }
            }}
            onDoubleClick={(e) => {
              e.stopPropagation();
              startRename(w);
            }}
            onAuxClick={(e) => {
              if (e.button === 1 && windows.length > 1) {
                e.preventDefault();
                void handleCloseTab(w.id, e);
              }
            }}
            draggable={!isEditing}
            onDragStart={(e) => { e.dataTransfer.effectAllowed = "move"; setDragId(w.id); }}
            onDragOver={(e) => { e.preventDefault(); e.dataTransfer.dropEffect = "move"; }}
            onDrop={(e) => {
              e.preventDefault();
              if (!dragId || dragId === w.id) return;
              setTabOrder(prev => {
                const next = [...prev];
                const fromIdx = next.indexOf(dragId);
                const toIdx = next.indexOf(w.id);
                if (fromIdx < 0 || toIdx < 0) return prev;
                next.splice(fromIdx, 1);
                next.splice(toIdx, 0, dragId);
                saveOrder(next);
                return next;
              });
              setDragId(null);
            }}
            onDragEnd={() => setDragId(null)}
            title={isEditing ? undefined : t("terminal.tabs.rename_hint")}
            className={`group flex items-center gap-1.5 px-3 py-1.5 text-xs whitespace-nowrap transition-colors cursor-pointer select-none rounded-t-md border-t border-x ${
              isActive
                ? "bg-[#0d1117] text-gray-200 border-gray-700/70 -mb-px pb-[7px]"
                : "text-gray-500 border-transparent hover:text-gray-300 hover:bg-gray-800/40 mb-0"
            } ${dragId === w.id ? "opacity-50" : ""}`}
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
                className="bg-gray-900 text-gray-200 text-xs px-1.5 py-0.5 rounded border border-blue-500/70 outline-none w-28"
              />
            ) : (
              <span className="max-w-[120px] truncate">
                {w.name || t("terminal.tabs.unnamed")}
              </span>
            )}
            {!isEditing && windows.length > 1 && (
              <span
                role="button"
                tabIndex={0}
                aria-label={t("terminal.tabs.close")}
                onClick={(e) => void handleCloseTab(w.id, e)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") void handleCloseTab(w.id, e);
                }}
                className={`text-gray-600 hover:text-red-400 cursor-pointer transition-colors rounded ${
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
        title={atLimit ? t("terminal.tabs.limit_reached") : t("terminal.tabs.new")}
        className="mb-0.5 flex items-center justify-center w-6 h-6 rounded text-gray-600 hover:text-gray-300 hover:bg-gray-800/50 transition-colors disabled:opacity-30 disabled:cursor-not-allowed disabled:hover:text-gray-600 disabled:hover:bg-transparent"
      >
        <Plus className="h-3.5 w-3.5" />
      </button>
      </div>
      </div>

      <div className="flex items-center gap-1 shrink-0 self-center pr-2 pb-0.5">
        {nearLimit && (
          <span className="text-[10px] text-gray-600 tabular-nums">
            {windows.length}/{maxWindows}
          </span>
        )}

        <div ref={helpRef} className="relative">
          <button
            onClick={() => setShowHelp((v) => !v)}
            aria-label={t("terminal.tabs.help")}
            className="flex items-center justify-center w-5 h-5 rounded text-gray-600 hover:text-gray-400 transition-colors"
          >
            <HelpCircle className="h-3.5 w-3.5" />
          </button>

          {showHelp && (
            <div className="absolute right-0 top-full mt-1 z-50 w-56 rounded-lg border border-gray-700/80 bg-[#1c2128] shadow-xl text-xs text-gray-300 p-3 space-y-2">
              <p className="font-semibold text-gray-200 mb-1">{t("terminal.tabs.help_title")}</p>
              {([
                ["terminal.tabs.help_switch",  "terminal.tabs.help_switch_key"],
                ["terminal.tabs.help_rename",  "terminal.tabs.help_rename_key"],
                ["terminal.tabs.help_close",   "terminal.tabs.help_close_key"],
                ["terminal.tabs.help_new",     "terminal.tabs.help_new_key"],
              ] as const).map(([desc, key]) => (
                <div key={key} className="flex items-start justify-between gap-2">
                  <span className="text-gray-400 leading-snug">{t(desc)}</span>
                  <kbd className="shrink-0 px-1.5 py-0.5 rounded bg-gray-700/60 text-[10px] text-gray-300 font-mono leading-tight whitespace-nowrap">
                    {t(key)}
                  </kbd>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
