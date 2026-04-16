import "@xterm/xterm/css/xterm.css";

import { useEffect, useRef, useState, useCallback } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { useTranslation } from "react-i18next";
import { useNavigate } from "@tanstack/react-router";
import { useQueryClient } from "@tanstack/react-query";
import { Terminal as TerminalIcon } from "lucide-react";
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
  const [shellName, setShellName] = useState<string>("sh");
  const [serverOs, setServerOs] = useState<string>("linux");
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
          setShellName(msg.shell ? msg.shell.split("/").pop() || "sh" : "sh");
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

    return () => {
      window.removeEventListener("resize", handleResize);
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

  return (
    <div className="flex flex-col h-full">
      <PageHeader
        badge={t("terminal.badge")}
        title={t("nav.terminal")}
        subtitle={
          error
            ? t("terminal.subtitle_error", { error })
            : isConnected
              ? t("terminal.subtitle_connected")
              : t("terminal.subtitle_disconnected")
        }
        icon={<TerminalIcon className="h-4 w-4" />}
        actions={
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
          </>
        }
      />
      <div className="flex-1 p-4">
        <Card className="h-full">
          {isRoot && (
            <div className="bg-red-500/20 border border-red-500/50 text-red-300 px-4 py-2 rounded-lg text-sm mb-2">
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
            shellName={shellName}
          />
          <div className="h-full flex flex-col overflow-hidden">
            <div
              ref={containerRef}
              className="flex-1 bg-[#1a1a2e] rounded-b-lg p-2 overflow-hidden"
            />
          </div>
        </Card>
      </div>
    </div>
  );
}
