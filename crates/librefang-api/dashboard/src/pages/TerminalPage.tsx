import "@xterm/xterm/css/xterm.css";

import { useEffect, useRef, useState, useCallback, useMemo } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import { useTranslation } from "react-i18next";
import { useNavigate } from "@tanstack/react-router";
import { useQueryClient } from "@tanstack/react-query";
import { terminalKeys } from "../lib/queries/keys";
import {
  Terminal as TerminalIcon,
  Maximize2,
  Minimize2,
  AlertCircle,
  X,
} from "lucide-react";
import { useUIStore } from "../lib/store";
import { buildAuthenticatedWebSocket } from "../api";
import { Button } from "../components/ui/Button";
import { TerminalTabs } from "../components/TerminalTabs";
import { useTerminalHealth } from "../lib/queries/terminal";

interface ServerMessage {
  type: "started" | "output" | "exit" | "error" | "active_window";
  shell?: string;
  pid?: number;
  data?: string;
  binary?: boolean;
  code?: number;
  signal?: string;
  content?: string;
  isRoot?: boolean;
  window_id?: string;
}

const RECONNECT_DELAY_MS = 2000;
const MAX_RECONNECT_ATTEMPTS = 10;
// #4675: a "fast exit" is the shell process dying within 3 s of `started`.
// That window is set well above any realistic legitimate `exit 0` from the
// user (e.g. typing `exit` immediately after connecting still takes a few
// hundred ms of human latency) but well below tmux's failure mode where it
// errors out and exits within ~10 ms.
const FAST_EXIT_WINDOW_MS = 3000;
// #4675: stop the reconnect loop after this many consecutive fast-failed
// connections. One fast-fail could be a transient race; two in a row means
// the daemon is in a state where retrying won't recover it.
const MAX_CONSECUTIVE_FAST_FAILS = 2;

// Must match the server-side MAX_COLS / MAX_ROWS constants in routes/terminal.rs.
const TERM_MIN_COLS = 1;
const TERM_MAX_COLS = 1000;
const TERM_MIN_ROWS = 1;
const TERM_MAX_ROWS = 500;

function clampTermSize(cols: number, rows: number): { cols: number; rows: number } | null {
  const c = Math.max(TERM_MIN_COLS, Math.min(TERM_MAX_COLS, Math.floor(cols)));
  const r = Math.max(TERM_MIN_ROWS, Math.min(TERM_MAX_ROWS, Math.floor(rows)));
  if (!Number.isFinite(c) || !Number.isFinite(r)) return null;
  return { cols: c, rows: r };
}

function getTmuxInstallCommand(os: string): string {
  switch (os) {
    case "macos":
      return "brew install tmux";
    default:
      return "sudo apt-get update && sudo apt-get install -y tmux || sudo dnf install -y tmux || sudo yum install -y tmux || sudo pacman -S --noconfirm tmux || sudo apk add tmux";
  }
}

// GitHub Dark-inspired terminal theme.
const TERMINAL_THEME = {
  background: "#0d1117",
  foreground: "#e6edf3",
  cursor: "#58a6ff",
  cursorAccent: "#0d1117",
  selectionBackground: "rgba(88,166,255,0.25)",
  black: "#21262d",
  red: "#ff7b72",
  green: "#3fb950",
  yellow: "#d29922",
  blue: "#58a6ff",
  magenta: "#bc8cff",
  cyan: "#39c5cf",
  white: "#b1bac4",
  brightBlack: "#6e7681",
  brightRed: "#ffa198",
  brightGreen: "#56d364",
  brightYellow: "#e3b341",
  brightBlue: "#79c0ff",
  brightMagenta: "#d2a8ff",
  brightCyan: "#56d4dd",
  brightWhite: "#f0f6fc",
} as const;

export function TerminalPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const containerRef = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const searchAddonRef = useRef<SearchAddon | null>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const wsRef = useRef<WebSocket | null>(null);
  const wsGenerationRef = useRef(0);
  const reconnectTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const toastDismissTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const intentionalDisconnectRef = useRef(false);
  const connectRef = useRef<() => void>(() => {});
  const attemptRef = useRef(0);
  const desiredWindowIdRef = useRef<string | null>(null);
  // #4675: fast-fail tracking. Two paths flip `connectionFastFailRef` true:
  // (1) the shell started and then exited with a non-zero code within
  //     FAST_EXIT_WINDOW_MS of `started` — host could spawn but the shell
  //     immediately died (tmux missing terminfo, broken rc files, etc.);
  // (2) the WS opened but `started` never arrived and the close fired
  //     within FAST_EXIT_WINDOW_MS of `open` — host couldn't even reach
  //     the spawn-success point (shell binary missing, PTY allocation
  //     failure, daemon-side panic).
  // `wsOpenedAtRef` is the anchor for path (2); `startedAtRef` for (1).
  // `consecutiveFastFailRef` counts how many connections in a row hit
  // either path; the reconnect loop bails at MAX_CONSECUTIVE_FAST_FAILS.
  const startedAtRef = useRef<number | null>(null);
  const wsOpenedAtRef = useRef<number | null>(null);
  const connectionFastFailRef = useRef(false);
  const consecutiveFastFailRef = useRef(0);

  const [isConnected, setIsConnected] = useState(false);
  const [isConnecting, setIsConnecting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [isRoot, setIsRoot] = useState(false);
  const [activeWindowId, setActiveWindowId] = useState<string | null>(null);
  const [pendingWindowId, setPendingWindowId] = useState<string | null>(null);
  const [isFullscreen, setIsFullscreen] = useState(false);
  const [reconnectAttempt, setReconnectAttempt] = useState(0);
  const [searchVisible, setSearchVisible] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [fontSize, setFontSize] = useState<number>(() => {
    const stored = localStorage.getItem("terminal.fontSize");
    const parsed = stored ? parseInt(stored, 10) : NaN;
    return Number.isFinite(parsed) ? Math.max(10, Math.min(20, parsed)) : 13;
  });

  const terminalEnabled = useUIStore((s) => s.terminalEnabled);
  const addToast = useUIStore((s) => s.addToast);
  const removeToast = useUIStore((s) => s.removeToast);
  const {
    data: terminalHealth,
    isError: terminalHealthError,
  } = useTerminalHealth({ enabled: terminalEnabled === true });

  const serverOs = terminalHealth?.os ?? "linux";
  const tmuxAvailable = !terminalHealthError && (terminalHealth?.tmux ?? false);
  const maxWindows = terminalHealth?.max_windows ?? 16;

  const displayedActiveWindowId = useMemo(
    () => pendingWindowId ?? activeWindowId,
    [pendingWindowId, activeWindowId]
  );

  useEffect(() => {
    if (terminalEnabled === false) {
      void navigate({ to: "/overview" });
    }
  }, [terminalEnabled, navigate]);

  const sendCloseMessage = useCallback((ws: WebSocket | null) => {
    if (ws?.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "close" }));
    }
  }, []);

  const connect = useCallback(() => {
    if (terminalEnabled !== true) return;

    const gen = ++wsGenerationRef.current;

    if (wsRef.current) {
      wsRef.current.onclose = null;
      wsRef.current.onerror = null;
      wsRef.current.close();
    }

    setError(null);
    setIsConnecting(true);
    setIsRoot(false);
    // #4675: reset per-connection fast-fail trackers; cross-connection
    // counter (`consecutiveFastFailRef`) is left alone so the close handler
    // can compare this connection's outcome against the previous run.
    startedAtRef.current = null;
    wsOpenedAtRef.current = null;
    connectionFastFailRef.current = false;
    const { url: wsUrl, protocols: wsProtocols } =
      buildAuthenticatedWebSocket("/api/terminal/ws");
    const url = new URL(wsUrl);
    if (terminalRef.current) {
      const size = clampTermSize(terminalRef.current.cols, terminalRef.current.rows);
      if (size) {
        url.searchParams.set("cols", String(size.cols));
        url.searchParams.set("rows", String(size.rows));
      }
    }
    const ws = new WebSocket(
      url.toString(),
      wsProtocols.length > 0 ? wsProtocols : undefined,
    );
    wsRef.current = ws;

    ws.onopen = () => {
      if (gen !== wsGenerationRef.current) return;
      // #4675: anchor the no-started fast-fail window from the moment
      // the WS handshake completes. Read in the close handler below.
      wsOpenedAtRef.current = Date.now();
      const wasReconnect = attemptRef.current > 0;
      setIsConnecting(false);
      setIsConnected(true);
      attemptRef.current = 0;
      setReconnectAttempt(0);
      setError(null);
      if (terminalRef.current && fitAddonRef.current) {
        const size = clampTermSize(terminalRef.current.cols, terminalRef.current.rows);
        if (size) ws.send(JSON.stringify({ type: "resize", ...size }));
      }
      if (desiredWindowIdRef.current) {
        ws.send(JSON.stringify({ type: "switch_window", window: desiredWindowIdRef.current }));
      }
      if (wasReconnect) {
        addToast(t("terminal.reconnected"), "success");
        // Grab the id that addToast just inserted (it uses Date.now() as id).
        const toasts = useUIStore.getState().toasts;
        const latest = toasts[toasts.length - 1];
        if (latest) {
          if (toastDismissTimerRef.current) {
            clearTimeout(toastDismissTimerRef.current);
          }
          toastDismissTimerRef.current = setTimeout(() => {
            removeToast(latest.id);
            toastDismissTimerRef.current = null;
          }, 3000);
        }
      }
      const hintKey = "terminal.copyPasteHintShown";
      if (!localStorage.getItem(hintKey)) {
        localStorage.setItem(hintKey, "1");
        addToast(t("terminal.copy_paste_hint"), "info");
      }
    };

    ws.onmessage = (event) => {
      if (gen !== wsGenerationRef.current) return;
      let msg: ServerMessage;
      try {
        msg = JSON.parse(event.data);
      } catch {
        return;
      }

      switch (msg.type) {
        case "started":
          setIsRoot(msg.isRoot ?? false);
          // #4675: anchor the fast-exit window from the moment the server
          // tells us the shell is up. Output that arrives between this and
          // `exit` is the shell's own stderr, e.g. tmux's "open terminal
          // failed: terminal does not support clear" message.
          startedAtRef.current = Date.now();
          terminalRef.current?.write(
            t("terminal.started", { shell: msg.shell, pid: msg.pid }) + "\r\n"
          );
          break;
        case "output":
          if (msg.binary && msg.data) {
            try {
              const binary = atob(msg.data);
              const bytes = new Uint8Array(binary.length);
              for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
              terminalRef.current?.write(bytes);
            } catch {
              terminalRef.current?.write(msg.data);
            }
          } else if (typeof msg.data === "string") {
            terminalRef.current?.write(msg.data);
          }
          break;
        case "exit":
          terminalRef.current?.write(
            "\r\n" + t("terminal.exited", { code: msg.code }) + "\r\n"
          );
          // #4675: classify this connection as a fast-fail when the shell
          // exits with a non-zero code within the FAST_EXIT_WINDOW_MS slot
          // that opened on `started`. The close handler reads this flag to
          // decide whether to keep reconnecting.
          if (
            typeof msg.code === "number" &&
            msg.code !== 0 &&
            startedAtRef.current !== null &&
            Date.now() - startedAtRef.current < FAST_EXIT_WINDOW_MS
          ) {
            connectionFastFailRef.current = true;
          }
          break;
        case "error":
          setError(typeof msg.content === "string" && msg.content
            ? msg.content
            : t("terminal.error_unknown"));
          setPendingWindowId(null);
          break;
        case "active_window":
          if (msg.window_id) {
            desiredWindowIdRef.current = msg.window_id;
            setActiveWindowId(msg.window_id);
            setPendingWindowId(null);
            queryClient.invalidateQueries({ queryKey: terminalKeys.windows() });
          }
          break;
      }
    };

    ws.onerror = () => {
      if (gen !== wsGenerationRef.current) return;
      setIsConnecting(false);
      setError(t("terminal.websocket_error"));
    };

    ws.onclose = (event: CloseEvent) => {
      if (gen !== wsGenerationRef.current) return;
      setIsConnected(false);
      setIsConnecting(false);

      if (intentionalDisconnectRef.current) {
        intentionalDisconnectRef.current = false;
        return;
      }

      const isAppError = event.code >= 4000 && event.code <= 4999;
      const isNonTransient = event.code === 1008 || event.code === 1011 || isAppError;
      if (isNonTransient) {
        setError(event.reason || t("terminal.connection_closed_non_recoverable"));
        return;
      }

      // #4675: ALSO classify the connection as a fast-fail when it opened
      // but `started` never arrived and the close fired inside the same
      // window. That covers host-side spawn failures (shell binary
      // missing, PTY allocation failure, daemon panic during spawn) where
      // the daemon never reached the point of telling us the shell was
      // up. The other path (started + non-zero exit fast) sets the flag
      // in the `case "exit"` branch above.
      if (
        !connectionFastFailRef.current &&
        startedAtRef.current === null &&
        wsOpenedAtRef.current !== null &&
        Date.now() - wsOpenedAtRef.current < FAST_EXIT_WINDOW_MS
      ) {
        connectionFastFailRef.current = true;
      }

      // #4675: a connection that fast-failed (either path) is almost
      // always a host-side configuration problem (TERM/terminfo, missing
      // tmux binary, missing shell binary, broken shell startup) —
      // reconnecting won't fix it. Track consecutive occurrences and
      // bail out of the retry loop after a small ceiling so the user
      // gets a clear error instead of a forever-spinning page.
      if (connectionFastFailRef.current) {
        consecutiveFastFailRef.current += 1;
        if (consecutiveFastFailRef.current >= MAX_CONSECUTIVE_FAST_FAILS) {
          setError(t("terminal.fast_exit_giveup"));
          return;
        }
      } else {
        consecutiveFastFailRef.current = 0;
      }

      if (attemptRef.current >= MAX_RECONNECT_ATTEMPTS) {
        setError(t("terminal.max_reconnect_exceeded"));
        return;
      }
      const delay = Math.min(RECONNECT_DELAY_MS * 2 ** attemptRef.current, 30_000) + Math.random() * 1000;
      attemptRef.current += 1;
      setReconnectAttempt(attemptRef.current);
      setIsConnecting(true);
      reconnectTimeoutRef.current = setTimeout(() => {
        if (wsRef.current === null || wsRef.current.readyState === WebSocket.CLOSED) {
          connect();
        }
      }, delay);
    };
  }, [t, terminalEnabled, queryClient, addToast]);

  connectRef.current = connect;

  // #4675: explicit user-initiated connect resets both ceilings
  // (auto-reconnect attempts and consecutive fast-fail count) so the user
  // can recover after fixing host config without reloading the page.
  const manualConnect = useCallback(() => {
    attemptRef.current = 0;
    consecutiveFastFailRef.current = 0;
    setReconnectAttempt(0);
    connect();
  }, [connect]);

  const disconnect = useCallback(() => {
    if (reconnectTimeoutRef.current) {
      clearTimeout(reconnectTimeoutRef.current);
      reconnectTimeoutRef.current = null;
    }
    if (wsRef.current) {
      intentionalDisconnectRef.current = true;
      sendCloseMessage(wsRef.current);
      wsRef.current.close();
      wsRef.current = null;
    }
    if (activeWindowId) {
      desiredWindowIdRef.current = activeWindowId;
    }
    setPendingWindowId(null);
    setIsConnected(false);
    setIsConnecting(false);
    setReconnectAttempt(0);
  }, [sendCloseMessage, activeWindowId]);

  useEffect(() => {
    if (terminalEnabled === true) return;
    desiredWindowIdRef.current = null;
    setPendingWindowId(null);
  }, [terminalEnabled]);

  const handleInstallTmux = useCallback(() => {
    const cmd = getTmuxInstallCommand(serverOs);
    if (wsRef.current?.readyState === WebSocket.OPEN) {
      wsRef.current.send(JSON.stringify({ type: "input", data: cmd + "\n" }));
    }
  }, [serverOs]);

  const handleSwitchWindow = useCallback((id: string | null) => {
    desiredWindowIdRef.current = id;
    setPendingWindowId(id);
  }, []);

  const toggleFullscreen = useCallback(() => {
    setIsFullscreen((v) => !v);
  }, []);

  // Focus search input when search bar becomes visible.
  useEffect(() => {
    if (!searchVisible) return;
    const raf = requestAnimationFrame(() => {
      searchInputRef.current?.focus();
    });
    return () => cancelAnimationFrame(raf);
  }, [searchVisible]);

  // Update terminal font size when fontSize state changes.
  useEffect(() => {
    const term = terminalRef.current;
    const fit = fitAddonRef.current;
    if (!term || !fit) return;
    term.options.fontSize = fontSize;
    try { fit.fit(); } catch { /* ignore */ }
    const ws = wsRef.current;
    if (ws?.readyState === WebSocket.OPEN) {
      const size = clampTermSize(term.cols, term.rows);
      if (size) ws.send(JSON.stringify({ type: "resize", ...size }));
    }
  }, [fontSize]);

  // Refit the terminal after fullscreen toggles.
  useEffect(() => {
    if (!terminalRef.current || !fitAddonRef.current) return;
    let raf2: number | null = null;
    const raf1 = requestAnimationFrame(() => {
      raf2 = requestAnimationFrame(() => {
        try {
          fitAddonRef.current?.fit();
        } catch { /* xterm not attached yet */ }
        const term = terminalRef.current;
        const ws = wsRef.current;
        if (term && ws?.readyState === WebSocket.OPEN) {
          const size = clampTermSize(term.cols, term.rows);
          if (size) ws.send(JSON.stringify({ type: "resize", ...size }));
        }
      });
    });
    return () => {
      cancelAnimationFrame(raf1);
      if (raf2 !== null) cancelAnimationFrame(raf2);
    };
  }, [isFullscreen]);

  // ESC exits fullscreen, but not when focus is inside the terminal.
  useEffect(() => {
    if (!isFullscreen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      const active = document.activeElement;
      if (active && containerRef.current?.contains(active)) return;
      setIsFullscreen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [isFullscreen]);

  useEffect(() => {
    if (terminalEnabled !== true) return;
    if (!containerRef.current) return;

    const term = new Terminal({
      theme: TERMINAL_THEME,
      fontSize: fontSize,
      fontFamily:
        "'Cascadia Code', 'JetBrains Mono', 'Fira Code', 'SF Mono', Consolas, 'Liberation Mono', monospace",
      lineHeight: 1.2,
      cursorBlink: true,
      cursorStyle: "block",
      // Show a dimmed underline cursor when the terminal loses focus (xterm v5.5+).
      // The option is augmented onto ITerminalOptions in vite-env.d.ts.
      cursorInactiveStyle: "underline",
      scrollback: 5000,
    });

    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);

    const searchAddon = new SearchAddon();
    term.loadAddon(searchAddon);
    searchAddonRef.current = searchAddon;

    term.open(containerRef.current);
    fitAddon.fit();

    terminalRef.current = term;
    fitAddonRef.current = fitAddon;

    term.attachCustomKeyEventHandler((e) => {
      if (e.ctrlKey && e.key === "f") {
        setSearchVisible(true);
        return false; // prevent xterm default
      }
      // Ctrl+L: clear the visible terminal buffer and forward \x0c to the shell
      // so the shell's own clear handler also runs (e.g. bash/zsh clear scrollback).
      if (e.type === "keydown" && e.ctrlKey && e.key === "l") {
        term.clear();
        if (wsRef.current?.readyState === WebSocket.OPEN) {
          wsRef.current.send(JSON.stringify({ type: "input", data: "\x0c" }));
        }
        return false; // prevent xterm from passing the keystroke a second time
      }
      return true;
    });

    term.onData((data) => {
      if (wsRef.current?.readyState === WebSocket.OPEN) {
        wsRef.current.send(JSON.stringify({ type: "input", data }));
      }
    });

    term.onResize(({ cols, rows }) => {
      if (wsRef.current?.readyState === WebSocket.OPEN) {
        const size = clampTermSize(cols, rows);
        if (size) wsRef.current.send(JSON.stringify({ type: "resize", ...size }));
      }
    });

    connectRef.current?.();

    const handleResize = () => fitAddon.fit();
    window.addEventListener("resize", handleResize);

    const ro = new ResizeObserver(() => {
      try { fitAddon.fit(); } catch { /* ignore */ }
    });
    ro.observe(containerRef.current);

    return () => {
      window.removeEventListener("resize", handleResize);
      ro.disconnect();
      if (reconnectTimeoutRef.current) {
        clearTimeout(reconnectTimeoutRef.current);
      }
      if (toastDismissTimerRef.current) {
        clearTimeout(toastDismissTimerRef.current);
      }
      if (wsRef.current) {
        intentionalDisconnectRef.current = true;
        sendCloseMessage(wsRef.current);
        wsRef.current.close();
        wsRef.current = null;
      }
      setIsConnected(false);
      setIsConnecting(false);
      term.dispose();
    };
  }, [sendCloseMessage, terminalEnabled]);

  // ── Derived UI state ─────────────────────────────────────────────────────────

  const statusDotClass = error
    ? "bg-red-400"
    : isConnecting
      ? "bg-amber-400 animate-pulse"
      : isConnected
        ? "bg-emerald-400"
        : "bg-gray-500";

  const statusLabel = isConnecting
    ? reconnectAttempt > 0
      ? t("terminal.subtitle_reconnecting", {
          attempt: reconnectAttempt,
          max: MAX_RECONNECT_ATTEMPTS,
        })
      : t("terminal.subtitle_connecting")
    : isConnected
      ? t("terminal.subtitle_connected")
      : t("terminal.subtitle_disconnected");

  // ── Actions ──────────────────────────────────────────────────────────────────

  const actions = (
    <div className="flex items-center gap-2">
      {!tmuxAvailable && isConnected && (
        <Button onClick={handleInstallTmux} variant="secondary" size="sm">
          {t("terminal.install_tmux")}
        </Button>
      )}
      <div className="flex items-center gap-0.5">
        <button
          onClick={() => {
            const n = Math.max(10, fontSize - 1);
            setFontSize(n);
            localStorage.setItem("terminal.fontSize", String(n));
          }}
          className="flex items-center justify-center w-6 h-6 rounded text-gray-500 hover:text-gray-300 hover:bg-gray-700/40 transition-colors text-xs font-mono"
          title={t("terminal.font_decrease")}
        >A-</button>
        <button
          onClick={() => {
            const n = Math.min(20, fontSize + 1);
            setFontSize(n);
            localStorage.setItem("terminal.fontSize", String(n));
          }}
          className="flex items-center justify-center w-7 h-6 rounded text-gray-500 hover:text-gray-300 hover:bg-gray-700/40 transition-colors text-xs font-mono"
          title={t("terminal.font_increase")}
        >A+</button>
      </div>
      {isConnected ? (
        <Button onClick={disconnect} variant="secondary" size="sm">
          {t("terminal.disconnect")}
        </Button>
      ) : (
        <Button
          onClick={manualConnect}
          isLoading={isConnecting}
          disabled={isConnecting}
          size="sm"
        >
          {t("terminal.connect")}
        </Button>
      )}
      <button
        onClick={toggleFullscreen}
        className="flex items-center justify-center w-8 h-8 rounded-xl border border-border-subtle bg-surface text-text-dim hover:text-brand hover:border-brand/30 transition-colors shadow-sm"
        aria-label={
          isFullscreen ? t("terminal.exit_fullscreen") : t("terminal.enter_fullscreen")
        }
        title={isFullscreen ? t("terminal.exit_fullscreen") : t("terminal.enter_fullscreen")}
      >
        {isFullscreen ? (
          <Minimize2 className="h-3.5 w-3.5" />
        ) : (
          <Maximize2 className="h-3.5 w-3.5" />
        )}
      </button>
    </div>
  );

  // ── Terminal body ─────────────────────────────────────────────────────────────

  const terminalBody = (
    <div className="flex flex-col flex-1 min-h-0 overflow-hidden">
      {isRoot && (
        <div className="shrink-0 flex items-center gap-2 bg-red-950/60 border-b border-red-800/50 px-4 py-2 text-xs text-red-400">
          <AlertCircle className="h-3.5 w-3.5 shrink-0" />
          <span>{t("terminal.root_warning")}</span>
        </div>
      )}
      {error && (
        <div className="shrink-0 flex items-center gap-2 bg-red-950/40 border-b border-red-800/40 px-4 py-2 text-xs text-red-400">
          <AlertCircle className="h-3.5 w-3.5 shrink-0" />
          <span className="flex-1 truncate min-w-0">{error}</span>
          <button
            onClick={() => setError(null)}
            className="shrink-0 ml-1 hover:text-red-300 transition-colors"
            aria-label={t("terminal.dismiss_error")}
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
      )}
      <TerminalTabs
        ws={wsRef.current}
        tmuxAvailable={tmuxAvailable}
        maxWindows={maxWindows}
        displayedActiveWindowId={displayedActiveWindowId}
        onSwitchWindow={handleSwitchWindow}
        terminalRef={terminalRef}
        fitAddonRef={fitAddonRef}
      />
      {searchVisible && (
        <div className="shrink-0 flex items-center gap-2 px-3 py-1.5 bg-[#1c2128] border-b border-gray-700/50">
          <input
            ref={searchInputRef}
            type="text"
            value={searchQuery}
            onChange={(e) => {
              setSearchQuery(e.target.value);
              searchAddonRef.current?.findNext(e.target.value, { incremental: true });
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.shiftKey
                  ? searchAddonRef.current?.findPrevious(searchQuery)
                  : searchAddonRef.current?.findNext(searchQuery);
              }
              if (e.key === "Escape") {
                setSearchVisible(false);
                terminalRef.current?.focus();
              }
              e.stopPropagation();
            }}
            placeholder={t("terminal.search_placeholder")}
            className="flex-1 min-w-0 bg-gray-900/60 text-gray-200 text-xs px-2 py-1 rounded border border-gray-700/60 outline-none focus:border-blue-500/60 placeholder:text-gray-600"
          />
          <button
            onClick={() => searchAddonRef.current?.findPrevious(searchQuery)}
            className="text-gray-500 hover:text-gray-300 transition-colors text-xs px-1.5 py-1 rounded hover:bg-gray-700/40"
            title={t("terminal.search_prev")}
          >↑</button>
          <button
            onClick={() => searchAddonRef.current?.findNext(searchQuery)}
            className="text-gray-500 hover:text-gray-300 transition-colors text-xs px-1.5 py-1 rounded hover:bg-gray-700/40"
            title={t("terminal.search_next")}
          >↓</button>
          <button
            onClick={() => { setSearchVisible(false); terminalRef.current?.focus(); }}
            className="text-gray-500 hover:text-gray-300 transition-colors"
            aria-label={t("terminal.search_close")}
          ><X className="h-3.5 w-3.5" /></button>
        </div>
      )}
      <div ref={containerRef} className="flex-1 min-h-0 overflow-hidden" />
    </div>
  );

  // ── Loading state ─────────────────────────────────────────────────────────────

  if (terminalEnabled === null) {
    return (
      <div className="flex flex-col flex-1 min-h-0">
        <header className="shrink-0 flex items-center gap-3 px-4 sm:px-6 py-3 bg-surface border-b border-border-subtle">
          <div className="p-1.5 rounded-lg bg-brand/10 text-brand shrink-0">
            <TerminalIcon className="h-4 w-4" />
          </div>
          <h1 className="text-sm font-extrabold tracking-tight">{t("nav.terminal")}</h1>
        </header>
        <div className="flex-1 min-h-0 flex items-center justify-center bg-[#0d1117] text-gray-500 text-sm">
          {t("common.loading")}
        </div>
      </div>
    );
  }

  // ── Headers ───────────────────────────────────────────────────────────────────

  const normalHeader = (
    <header className="shrink-0 flex items-center justify-between gap-3 px-4 sm:px-6 py-3 bg-surface border-b border-border-subtle">
      <div className="flex items-center gap-3 min-w-0">
        <div className="p-1.5 rounded-lg bg-brand/10 text-brand shrink-0">
          <TerminalIcon className="h-4 w-4" />
        </div>
        <div className="min-w-0">
          <h1 className="text-sm font-extrabold tracking-tight leading-tight">
            {t("nav.terminal")}
          </h1>
          <div className="flex items-center gap-1.5 mt-0.5">
            <span className={`w-1.5 h-1.5 rounded-full shrink-0 ${statusDotClass}`} />
            <p className="text-[11px] text-text-dim truncate">{statusLabel}</p>
          </div>
        </div>
      </div>
      <div className="flex items-center gap-2 shrink-0">{actions}</div>
    </header>
  );

  const fullscreenHeader = (
    <header className="shrink-0 flex items-center justify-between gap-3 px-4 py-2 bg-[#161b22] border-b border-gray-700/50">
      <div className="flex items-center gap-2 min-w-0">
        <TerminalIcon className="h-3.5 w-3.5 text-gray-400 shrink-0" />
        <span className="text-sm font-semibold text-gray-200 truncate">
          {t("nav.terminal")}
        </span>
        <span className={`w-1.5 h-1.5 rounded-full shrink-0 ${statusDotClass}`} />
        <span className="text-xs text-gray-400 truncate">{statusLabel}</span>
      </div>
      <div className="flex items-center gap-2 shrink-0">{actions}</div>
    </header>
  );

  // ── Main render ───────────────────────────────────────────────────────────────

  // Single tree in both modes so React doesn't unmount the xterm container
  // when toggling fullscreen. Only the header chrome and outer className swap.
  return (
    <div
      className={
        isFullscreen
          ? "fixed inset-0 z-50 flex flex-col bg-[#0d1117]"
          : "flex flex-col flex-1 min-h-0"
      }
    >
      {isFullscreen ? fullscreenHeader : normalHeader}
      <div className={isFullscreen ? "flex flex-col flex-1 min-h-0" : "flex flex-col flex-1 px-4 pb-4 pt-3 min-h-0"}>
        <div
          className={
            isFullscreen
              ? "flex flex-col flex-1 min-h-0 overflow-hidden"
              : "flex flex-col flex-1 min-h-0 overflow-hidden rounded-xl sm:rounded-2xl border border-gray-800 bg-[#0d1117] shadow-lg"
          }
        >
          {terminalBody}
        </div>
      </div>
    </div>
  );
}
