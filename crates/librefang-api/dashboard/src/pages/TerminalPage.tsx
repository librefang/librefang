import "@xterm/xterm/css/xterm.css";

import { useEffect, useRef, useState, useCallback } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { useTranslation } from "react-i18next";
import { useNavigate } from "@tanstack/react-router";
import { useQueryClient } from "@tanstack/react-query";
import {
  Terminal as TerminalIcon,
  Maximize2,
  Minimize2,
} from "lucide-react";
import { useUIStore } from "../lib/store";
import { buildAuthenticatedWebSocketUrl, authHeader } from "../api";
import { PageHeader } from "../components/ui/PageHeader";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { EmptyState } from "../components/ui/EmptyState";
import { TerminalTabs } from "../components/TerminalTabs";

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

interface TerminalHealth {
  ok: boolean;
  tmux: boolean;
  max_windows: number;
  os: string;
}

const RECONNECT_DELAY_MS = 2000;
const MAX_RECONNECT_ATTEMPTS = 10;

function getTmuxInstallCommand(os: string): string {
  switch (os) {
    case "macos":
      return "brew install tmux";
    default:
      return "sudo apt-get update && sudo apt-get install -y tmux || sudo dnf install -y tmux || sudo yum install -y tmux || sudo pacman -S --noconfirm tmux || sudo apk add tmux";
  }
}

export function TerminalPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const containerRef = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const wsRef = useRef<WebSocket | null>(null);
  const reconnectTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const intentionalDisconnectRef = useRef(false);
  const connectRef = useRef<() => void>(() => {});
  const attemptRef = useRef(0);

  const [isConnected, setIsConnected] = useState(false);
  const [isConnecting, setIsConnecting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [isRoot, setIsRoot] = useState(false);
  const [tmuxAvailable, setTmuxAvailable] = useState(false);
  const [maxWindows, setMaxWindows] = useState(16);
  const [activeWindowId, setActiveWindowId] = useState<string | null>(null);
  const [serverOs, setServerOs] = useState<string>("linux");
  const [isFullscreen, setIsFullscreen] = useState(false);
  const terminalEnabled = useUIStore((s) => s.terminalEnabled);

  useEffect(() => {
    if (terminalEnabled === false) {
      void navigate({ to: "/overview" });
    }
  }, [terminalEnabled, navigate]);

  // Fetch terminal health for tmux feature flag.
  useEffect(() => {
    if (terminalEnabled !== true) return;
    fetch("/api/terminal/health", { headers: authHeader() })
      .then((r) => r.json())
      .then((data: TerminalHealth) => {
        setTmuxAvailable(data.tmux ?? false);
        setMaxWindows(data.max_windows ?? 16);
        if (data.os) setServerOs(data.os);
      })
      .catch(() => {
        setTmuxAvailable(false);
      });
  }, [terminalEnabled]);

  const sendCloseMessage = useCallback((ws: WebSocket | null) => {
    if (ws?.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "close" }));
    }
  }, []);

  const connect = useCallback(() => {
    if (terminalEnabled !== true) {
      return;
    }

    if (wsRef.current) {
      wsRef.current.close();
    }

    setError(null);
    setIsConnecting(true);
    setIsRoot(false);
    const url = new URL(buildAuthenticatedWebSocketUrl("/api/terminal/ws"));
    if (terminalRef.current) {
      url.searchParams.set("cols", String(terminalRef.current.cols));
      url.searchParams.set("rows", String(terminalRef.current.rows));
    }
    const ws = new WebSocket(url.toString());
    wsRef.current = ws;

    ws.onopen = () => {
      setIsConnecting(false);
      setIsConnected(true);
      attemptRef.current = 0;
      setError(null);
      if (terminalRef.current && fitAddonRef.current) {
        const { cols, rows } = terminalRef.current;
        ws.send(JSON.stringify({ type: "resize", cols, rows }));
      }
    };

    ws.onmessage = (event) => {
      let msg: ServerMessage;
      try {
        msg = JSON.parse(event.data);
      } catch {
        return;
      }

      switch (msg.type) {
        case "started":
          setIsRoot(msg.isRoot ?? false);
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
          break;
        case "error":
          setError(typeof msg.content === "string" && msg.content
            ? msg.content
            : t("terminal.error_unknown"));
          break;
        case "active_window":
          if (msg.window_id) {
            setActiveWindowId(msg.window_id);
            queryClient.invalidateQueries({ queryKey: ["terminal-windows"] });
          }
          break;
      }
    };

    ws.onerror = () => {
      setIsConnecting(false);
      setError(t("terminal.websocket_error"));
    };

    ws.onclose = (event: CloseEvent) => {
      setIsConnected(false);
      setIsConnecting(false);

      if (intentionalDisconnectRef.current) {
        intentionalDisconnectRef.current = false;
        return;
      }

      // Non-transient close codes: stop reconnecting
      const isAppError = event.code >= 4000 && event.code <= 4999;
      const isNonTransient = event.code === 1008 || event.code === 1011 || isAppError;
      if (isNonTransient) {
        setError(event.reason || t("terminal.connection_closed_non_recoverable"));
        return;
      }

      if (attemptRef.current >= MAX_RECONNECT_ATTEMPTS) {
        setError(t("terminal.max_reconnect_exceeded"));
        return;
      }
      const delay = Math.min(RECONNECT_DELAY_MS * 2 ** attemptRef.current, 30_000) + Math.random() * 1000;
      attemptRef.current += 1;
      reconnectTimeoutRef.current = setTimeout(() => {
        if (
          wsRef.current === null ||
          wsRef.current.readyState === WebSocket.CLOSED
        ) {
          connect();
        }
      }, delay);
    };
  }, [t, terminalEnabled, queryClient]);

  connectRef.current = connect;

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
    setIsConnected(false);
    setIsConnecting(false);
  }, [sendCloseMessage]);

  const handleInstallTmux = useCallback(() => {
    const cmd = getTmuxInstallCommand(serverOs);
    if (wsRef.current?.readyState === WebSocket.OPEN) {
      wsRef.current.send(JSON.stringify({ type: "input", data: cmd }));
    }
  }, [serverOs]);

  const handleSwitchWindow = useCallback((id: string) => {
    setActiveWindowId(id);
  }, []);

  const toggleFullscreen = useCallback(() => {
    setIsFullscreen((v) => !v);
  }, []);

  // Refit the terminal after fullscreen toggles, and notify the remote pty of
  // the new size. We rAF twice so layout has actually settled before we measure.
  useEffect(() => {
    if (!terminalRef.current || !fitAddonRef.current) return;
    const raf1 = requestAnimationFrame(() => {
      const raf2 = requestAnimationFrame(() => {
        try {
          fitAddonRef.current?.fit();
        } catch {
          /* xterm not attached yet */
        }
        const term = terminalRef.current;
        const ws = wsRef.current;
        if (term && ws?.readyState === WebSocket.OPEN) {
          ws.send(JSON.stringify({ type: "resize", cols: term.cols, rows: term.rows }));
        }
      });
      return () => cancelAnimationFrame(raf2);
    });
    return () => cancelAnimationFrame(raf1);
  }, [isFullscreen]);

  // ESC exits fullscreen — but not when focus is inside the terminal, since
  // vim/less/tmux all use Escape as a meaningful key.
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
    if (terminalEnabled !== true) {
      return;
    }

    if (!containerRef.current) return;

    const term = new Terminal({
      theme: {
        background: "#1a1a2e",
        foreground: "#eee",
        cursor: "#f00",
      },
      fontSize: 14,
      fontFamily: "monospace",
    });

    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);

    term.open(containerRef.current);
    fitAddon.fit();

    terminalRef.current = term;
    fitAddonRef.current = fitAddon;

    term.onData((data) => {
      if (wsRef.current?.readyState === WebSocket.OPEN) {
        wsRef.current.send(JSON.stringify({ type: "input", data }));
      }
    });

    term.onResize(({ cols, rows }) => {
      if (wsRef.current?.readyState === WebSocket.OPEN) {
        wsRef.current.send(JSON.stringify({ type: "resize", cols, rows }));
      }
    });

    connectRef.current?.();

    const handleResize = () => fitAddon.fit();
    window.addEventListener("resize", handleResize);

    // Also refit when the container itself changes size (sidebar toggle,
    // parent layout changes, etc) so we don't get a mismatched pty size.
    const ro = new ResizeObserver(() => {
      try {
        fitAddon.fit();
      } catch {
        /* ignore */
      }
    });
    ro.observe(containerRef.current);

    return () => {
      window.removeEventListener("resize", handleResize);
      ro.disconnect();
      if (reconnectTimeoutRef.current) {
        clearTimeout(reconnectTimeoutRef.current);
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

  if (terminalEnabled === null) {
    return (
      <div className="flex flex-col h-full">
        <PageHeader
          badge={t("terminal.badge")}
          title={t("nav.terminal")}
          subtitle={t("common.loading")}
          icon={<TerminalIcon className="h-4 w-4" />}
        />
        <div className="flex-1 p-4">
          <Card className="h-full flex items-center justify-center">
            <EmptyState title={t("common.loading")} icon={<TerminalIcon className="h-6 w-6" />} />
          </Card>
        </div>
      </div>
    );
  }

  const statusLabel = error
    ? t("terminal.subtitle_error", { error })
    : isConnected
      ? t("terminal.subtitle_connected")
      : t("terminal.subtitle_disconnected");

  const actions = (
    <>
      {!tmuxAvailable && isConnected && (
        <Button onClick={handleInstallTmux} variant="secondary">
          {t("terminal.install_tmux")}
        </Button>
      )}
      <Button onClick={connect} disabled={isConnected || isConnecting}>
        {isConnected
          ? t("terminal.subtitle_connected")
          : t("terminal.connect")}
      </Button>
      {isConnected && (
        <Button onClick={disconnect} variant="secondary">
          {t("terminal.disconnect")}
        </Button>
      )}
      <Button
        onClick={toggleFullscreen}
        variant="secondary"
        aria-label={
          isFullscreen
            ? t("terminal.exit_fullscreen")
            : t("terminal.enter_fullscreen")
        }
        title={
          isFullscreen
            ? t("terminal.exit_fullscreen")
            : t("terminal.enter_fullscreen")
        }
      >
        {isFullscreen ? (
          <Minimize2 className="h-4 w-4" />
        ) : (
          <Maximize2 className="h-4 w-4" />
        )}
      </Button>
    </>
  );

  // The terminal body. Rendered identically in normal and fullscreen modes so
  // xterm never unmounts — otherwise the pty session would be torn down every
  // time we toggle fullscreen.
  const terminalBody = (
    <div className="h-full flex flex-col overflow-hidden">
      {isRoot && (
        <div className="bg-red-500/20 border border-red-500/50 text-red-300 px-4 py-2 text-sm shrink-0">
          {t("terminal.root_warning")}
        </div>
      )}
      <TerminalTabs
        ws={wsRef.current}
        tmuxAvailable={tmuxAvailable}
        maxWindows={maxWindows}
        activeWindowId={activeWindowId}
        onSwitchWindow={handleSwitchWindow}
        terminalRef={terminalRef}
        fitAddonRef={fitAddonRef}
      />
      <div
        ref={containerRef}
        className="flex-1 bg-[#1a1a2e] p-2 overflow-hidden min-h-0"
      />
    </div>
  );

  // Single tree in both modes so React doesn't unmount the xterm container
  // when toggling fullscreen. Only the header chrome and outer className swap.
  return (
    <div
      className={
        isFullscreen
          ? "fixed inset-0 z-50 flex flex-col bg-[#1a1a2e]"
          : "flex flex-col h-full"
      }
    >
      {isFullscreen ? (
        <div className="flex items-center justify-between gap-3 px-4 py-2 bg-surface border-b border-border-subtle shrink-0">
          <div className="flex items-center gap-2 min-w-0">
            <TerminalIcon className="h-4 w-4 text-text-dim shrink-0" />
            <span className="font-semibold truncate">{t("nav.terminal")}</span>
            <span className="text-xs text-text-dim truncate">· {statusLabel}</span>
          </div>
          <div className="flex items-center gap-2 shrink-0">{actions}</div>
        </div>
      ) : (
        <PageHeader
          badge={t("terminal.badge")}
          title={t("nav.terminal")}
          subtitle={statusLabel}
          icon={<TerminalIcon className="h-4 w-4" />}
          actions={actions}
        />
      )}
      <div className={isFullscreen ? "flex-1 min-h-0" : "flex-1 p-4 min-h-0"}>
        <div
          className={
            isFullscreen
              ? "h-full flex flex-col overflow-hidden"
              : "h-full flex flex-col overflow-hidden rounded-xl sm:rounded-2xl border border-border-subtle bg-surface shadow-sm"
          }
        >
          {terminalBody}
        </div>
      </div>
    </div>
  );
}
