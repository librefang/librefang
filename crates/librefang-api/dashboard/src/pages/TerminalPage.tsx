import "@xterm/xterm/css/xterm.css";

import { useEffect, useRef, useState, useCallback } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { useTranslation } from "react-i18next";
import { Terminal as TerminalIcon } from "lucide-react";
import { buildAuthenticatedWebSocketUrl } from "../api";
import { PageHeader } from "../components/ui/PageHeader";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";

interface ServerMessage {
  type: "started" | "output" | "exit" | "error";
  shell?: string;
  pid?: number;
  data?: string;
  binary?: boolean;
  code?: number;
  signal?: string;
  content?: string;
}

const RECONNECT_DELAY_MS = 2000;

export function TerminalPage() {
  const { t } = useTranslation();
  const containerRef = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const wsRef = useRef<WebSocket | null>(null);
  const reconnectTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const intentionalDisconnectRef = useRef(false);
  const connectRef = useRef<() => void>(() => {});

  const [isConnected, setIsConnected] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const connect = useCallback(() => {
    if (wsRef.current) {
      wsRef.current.close();
    }

    setError(null);
    const url = buildAuthenticatedWebSocketUrl("/api/terminal/ws");
    const ws = new WebSocket(url);
    wsRef.current = ws;

    ws.onopen = () => {
      setIsConnected(true);
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
      }
    };

    ws.onerror = () => {
      setError("WebSocket connection error");
    };

    ws.onclose = () => {
      setIsConnected(false);
      if (intentionalDisconnectRef.current) {
        intentionalDisconnectRef.current = false;
        return;
      }
      reconnectTimeoutRef.current = setTimeout(() => {
        if (
          wsRef.current === null ||
          wsRef.current.readyState === WebSocket.CLOSED
        ) {
          connect();
        }
      }, RECONNECT_DELAY_MS);
    };
  }, [t]);

  connectRef.current = connect;

  const disconnect = useCallback(() => {
    if (reconnectTimeoutRef.current) {
      clearTimeout(reconnectTimeoutRef.current);
      reconnectTimeoutRef.current = null;
    }

    if (wsRef.current) {
      intentionalDisconnectRef.current = true;
      wsRef.current.send(JSON.stringify({ type: "close" }));
      wsRef.current.close();
      wsRef.current = null;
    }
    setIsConnected(false);
  }, []);

  useEffect(() => {
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
        wsRef.current.send(JSON.stringify({ type: "close" }));
        wsRef.current.close();
        wsRef.current = null;
      }
      setIsConnected(false);
      term.dispose();
    };
  }, []);

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
            <Button onClick={connect} disabled={isConnected}>
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
          <div className="h-full min-h-[400px] flex flex-col">
            <div
              ref={containerRef}
              className="flex-1 bg-[#1a1a2e] rounded-b-lg p-2 overflow-hidden h-full min-[1001px]:h-[70%]"
            />
          </div>
        </Card>
      </div>
    </div>
  );
}
