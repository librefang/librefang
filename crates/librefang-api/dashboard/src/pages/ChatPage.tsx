import { useQuery, useQueryClient } from "@tanstack/react-query";
import { formatCost } from "../lib/format";
import { memo, useEffect, useMemo, useRef, useState, useCallback } from "react";
import rehypeKatex from "rehype-katex";
import remarkMath from "remark-math";
import { useTranslation } from "react-i18next";
import { useNavigate, useSearch } from "@tanstack/react-router";
import { buildAuthenticatedWebSocketUrl, listAgents, sendAgentMessage, loadAgentSession, listPendingApprovals, resolveApproval } from "../api";
import type { ApprovalItem } from "../api";
import { normalizeToolOutput } from "../lib/chat";
import { MessageCircle, Send, Bot, User, RefreshCw, AlertCircle, Wifi, Sparkles, X, ArrowRight, Zap, ShieldAlert, CheckCircle, XCircle } from "lucide-react";
import { Badge } from "../components/ui/Badge";
import { MarkdownContent } from "../components/ui/MarkdownContent";
import { useUIStore } from "../lib/store";
import { Typewriter_v2 } from "../components/Typewriter_v2";
import "katex/dist/katex.min.css";

const isAuthUnavailable = (status?: string) =>
  !!status && status !== "configured" && status !== "configured_cli" && status !== "not_required";

interface ChatMessage {
  id: string;
  role: "user" | "assistant" | "system";
  content: string;
  timestamp: Date;
  isStreaming?: boolean;
  error?: string;
  tokens?: { input?: number; output?: number };
  cost_usd?: number;
  memories_saved?: string[];
  memories_used?: string[];
}

// Slash commands
const SLASH_COMMANDS = [
  { cmd: "/help", desc: "Show available commands" },
  { cmd: "/clear", desc: "Clear chat history" },
  { cmd: "/agents", desc: "List available agents" },
  { cmd: "/info", desc: "Show current agent info" },
];


// WebSocket hook with auto-reconnect
function useWebSocket(agentId: string | null) {
  const wsRef = useRef<WebSocket | null>(null);
  const [wsConnected, setWsConnected] = useState(false);
  const reconnectTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const retriesRef = useRef(0);
  // Callback fired when WS closes while a response is pending
  const onDropRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    if (!agentId) {
      setWsConnected(false);
      return;
    }

    const url = buildAuthenticatedWebSocketUrl(`/api/agents/${encodeURIComponent(agentId)}/ws`);

    function connect() {
      try {
        const ws = new WebSocket(url);

        ws.onopen = () => {
          setWsConnected(true);
          retriesRef.current = 0;
        };

        ws.onclose = () => {
          setWsConnected(false);
          // Notify pending response handler
          if (onDropRef.current) {
            onDropRef.current();
            onDropRef.current = null;
          }
          // Auto-reconnect with exponential backoff (max 15s)
          const delay = Math.min(1000 * 2 ** retriesRef.current, 15000);
          retriesRef.current++;
          reconnectTimer.current = setTimeout(connect, delay);
        };

        ws.onerror = () => {
          // onclose will fire after onerror, reconnect handled there
        };

        wsRef.current = ws;
      } catch {
        setWsConnected(false);
      }
    }

    connect();

    return () => {
      if (reconnectTimer.current) clearTimeout(reconnectTimer.current);
      retriesRef.current = 0;
      onDropRef.current = null;
      if (wsRef.current) {
        wsRef.current.onclose = null; // prevent reconnect on intentional close
        wsRef.current.close();
        wsRef.current = null;
      }
    };
  }, [agentId]);

  return { ws: wsRef, wsConnected, onDropRef };
}

// Per-agent session cache — survives agent switches within the same page lifecycle
const sessionCache = new Map<string, ChatMessage[]>();

// Chat message management - includes history loading and sending (with WS streaming)
function useChatMessages(agentId: string | null, agents: any[] = []) {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const { ws, wsConnected, onDropRef } = useWebSocket(agentId);
  const addSkillOutput = useUIStore((s) => s.addSkillOutput);

  // Save current messages to cache when switching away
  const prevAgentRef = useRef<string | null>(null);
  useEffect(() => {
    return () => {
      if (prevAgentRef.current) {
        sessionCache.set(prevAgentRef.current, messages);
      }
    };
  });
  useEffect(() => { prevAgentRef.current = agentId; }, [agentId]);

  // Load history — use cache if available, otherwise fetch
  useEffect(() => {
    if (!agentId) { setMessages([]); return; }

    const cached = sessionCache.get(agentId);
    if (cached) {
      setMessages(cached);
      return;
    }

    setMessages([]);
    setIsLoading(true);
    loadAgentSession(agentId)
      .then(session => {
        if (session.messages?.length) {
          const historical: ChatMessage[] = session.messages.flatMap((msg, idx) => {
            const content = typeof msg.content === "string"
              ? msg.content
              : msg.content == null
                ? ""
                : JSON.stringify(msg.content);

            if (!content.trim()) return [];

            return [{
              id: `hist-${idx}`,
              role: msg.role === "User"
                ? "user"
                : msg.role === "System"
                  ? "system"
                  : "assistant",
              content,
              timestamp: new Date(),
            }];
          });
          setMessages(historical);
          sessionCache.set(agentId, historical);
        }
      })
      .catch(() => {})
      .finally(() => setIsLoading(false));
  }, [agentId]);

  // Send message - WS first, HTTP fallback
  const sendMessage = useCallback(async (content: string) => {
    if (!content.trim()) return;
    const trimmed = content.trim();

    // Slash command handling
    if (trimmed.startsWith("/")) {
      const sysMsg = (text: string) => {
        setMessages(prev => [...prev,
          { id: `user-${Date.now()}`, role: "user" as const, content: trimmed, timestamp: new Date() },
          { id: `sys-${Date.now()}`, role: "system" as const, content: text, timestamp: new Date() }
        ]);
      };
      if (trimmed === "/help") {
        sysMsg(SLASH_COMMANDS.map(c => `**${c.cmd}** — ${c.desc}`).join("\n"));
        return;
      }
      if (trimmed === "/clear") { setMessages([]); return; }
      if (trimmed === "/agents") {
        const names = agents.map(a => `- **${a.name}** (${a.state || "unknown"})`).join("\n");
        sysMsg(names || "No agents available.");
        return;
      }
      if (trimmed === "/info") {
        const a = agents.find(a => a.id === agentId);
        sysMsg(a ? `**${a.name}**\nModel: ${a.model_name || "-"}\nProvider: ${a.model_provider || "-"}\nState: ${a.state}` : "No agent selected.");
        return;
      }
    }

    if (!agentId) return;

    const userMsg: ChatMessage = {
      id: `user-${Date.now()}`,
      role: "user",
      content: trimmed,
      timestamp: new Date(),
    };

    const botMsg: ChatMessage = {
      id: `bot-${Date.now()}`,
      role: "assistant",
      content: "",
      timestamp: new Date(),
      isStreaming: true,
    };

    setMessages(prev => [...prev, userMsg, botMsg]);
    setIsLoading(true);

    // Helper: send via HTTP (used as primary fallback and WS drop recovery)
    const sendViaHttp = async () => {
      try {
        const response = await sendAgentMessage(agentId, trimmed);
        const fullContent = response.response || "";
        setMessages(prev => prev.map(m =>
          m.id === botMsg.id
            ? {
                ...m, content: fullContent, isStreaming: false,
                tokens: { output: response.output_tokens, input: response.input_tokens },
                cost_usd: response.cost_usd,
                memories_saved: response.memories_saved,
                memories_used: response.memories_used,
              }
            : m
        ));
        if (response.memories_saved?.length) {
          const agentName = agents.find(a => a.id === agentId)?.name;
          response.memories_saved.forEach((mem: string) => {
            addSkillOutput({ skillName: "memory", agentId: agentId || undefined, agentName, content: mem });
          });
        }
      } catch (err) {
        const errorMsg = err instanceof Error ? err.message : "Unknown error";
        setMessages(prev => prev.map(m =>
          m.id === botMsg.id ? { ...m, isStreaming: false, error: errorMsg } : m
        ));
      } finally {
        setIsLoading(false);
      }
    };

    // Try WebSocket streaming first
    if (wsConnected && ws.current && ws.current.readyState === WebSocket.OPEN) {
      try {
        let responded = false;
        let fallbackTimer: ReturnType<typeof setTimeout> | null = null;

        const resetFallbackTimer = () => {
          if (fallbackTimer) clearTimeout(fallbackTimer);
          fallbackTimer = setTimeout(() => {
            if (!responded) {
              cleanup();
              sendViaHttp();
            }
          }, 180000);
        };

        const cleanup = () => {
          responded = true;
          if (fallbackTimer) { clearTimeout(fallbackTimer); fallbackTimer = null; }
          onDropRef.current = null;
          ws.current?.removeEventListener("message", handleMessage);
        };

        // Set up message handler for this response
        const handleMessage = (event: MessageEvent) => {
          // Reset inactivity timeout on every event
          resetFallbackTimer();
          try {
            const data = JSON.parse(event.data as string);
            if (data.type === "text_delta") {
              const chunk = data.content || "";
              setMessages(prev => prev.map(m =>
                m.id === botMsg.id ? { ...m, content: m.content + chunk } : m
              ));
            } else if (data.type === "typing") {
              if (data.state === "stop") {
                setMessages(prev => prev.map(m =>
                  m.id === botMsg.id ? { ...m, isStreaming: false } : m
                ));
              }
            } else if (data.type === "tool_result") {
              const entry = normalizeToolOutput(data);
              if (entry) {
                addSkillOutput({ skillName: entry.tool, agentId: agentId || undefined, content: entry.content });
              }
            } else if (data.type === "silent_complete") {
              setMessages(prev => prev.filter(m => m.id !== botMsg.id));
              setIsLoading(false);
              cleanup();
            } else if (data.type === "error") {
              const error = data.content || "WebSocket error";
              setMessages(prev => prev.map(m =>
                m.id === botMsg.id ? { ...m, isStreaming: false, error } : m
              ));
              setIsLoading(false);
              cleanup();
            } else if (data.type === "response") {
              setMessages(prev => prev.map(m =>
                m.id === botMsg.id
                  ? {
                      ...m, content: data.content || m.content, isStreaming: false,
                      tokens: { output: data.output_tokens, input: data.input_tokens },
                      cost_usd: data.cost_usd,
                      memories_saved: data.memories_saved,
                      memories_used: data.memories_used,
                    }
                  : m
              ));
              setIsLoading(false);
              cleanup();
            }
          } catch {
            // Non-JSON text chunk
            setMessages(prev => prev.map(m =>
              m.id === botMsg.id ? { ...m, content: m.content + event.data } : m
            ));
          }
        };

        // Register fallback: if WS drops mid-stream, retry via HTTP
        onDropRef.current = () => {
          if (!responded) {
            ws.current?.removeEventListener("message", handleMessage);
            sendViaHttp();
          }
        };

        ws.current.addEventListener("message", handleMessage);
        ws.current.send(JSON.stringify({ type: "message", content: trimmed }));

        // Start inactivity timeout — resets on every received event
        resetFallbackTimer();

        return;
      } catch {
        // Fall through to HTTP
      }
    }

    // HTTP fallback — direct, no fake streaming
    await sendViaHttp();
  }, [agentId, agents, wsConnected, ws]);

  const clearHistory = useCallback(() => setMessages([]), []);

  return { messages, isLoading, sendMessage, clearHistory, wsConnected };
}

// Message bubble component — memoized to skip re-render during streaming of other messages
const MessageBubble = memo(function MessageBubble({ message }: { message: ChatMessage }) {
  const { t } = useTranslation();
  const isUser = message.role === "user";

  if (message.role === "system") {
    return (
      <div className="flex justify-center py-6">
        <div className="flex items-center gap-4">
          <div className="h-px w-16 bg-gradient-to-r from-transparent to-border-subtle" />
          <span className="text-[10px] font-medium text-text-dim/40 tracking-[0.2em] uppercase">{message.content}</span>
          <div className="h-px w-16 bg-gradient-to-l from-transparent to-border-subtle" />
        </div>
      </div>
    );
  }

  return (
    <div className={`flex ${isUser ? "justify-end" : "justify-start"}`}>
      <div className={`flex flex-col max-w-[90%] sm:max-w-[75%] ${isUser ? "items-end" : "items-start"}`}>
        {/* Avatar + name */}
        <div className={`flex items-center gap-2 mb-1.5 ${isUser ? "self-end flex-row-reverse" : "self-start"}`}>
          <div className={`h-7 w-7 rounded-lg flex items-center justify-center ${
            isUser ? "bg-gradient-to-br from-brand to-accent text-white shadow-md" : "bg-surface border border-border-subtle"
          }`}>
            {isUser ? <User className="h-3.5 w-3.5" /> : <Bot className="h-3.5 w-3.5 text-brand" />}
          </div>
          <span className={`text-[11px] font-bold uppercase tracking-wider ${isUser ? "text-brand" : "text-text-dim"}`}>
            {isUser ? t("chat.you") : t("chat.bot")}
          </span>
        </div>

        {/* Message content */}
        <div className={`relative px-4 py-3 rounded-2xl text-sm leading-relaxed shadow-sm transition-colors ${
          isUser
            ? "bg-gradient-to-br from-brand to-brand/90 text-white rounded-tr-md"
            : message.error
              ? "bg-error/10 border border-error/20 text-error rounded-tl-md"
              : "bg-surface border border-border-subtle rounded-tl-md"
        }`}>
          {message.isStreaming ? (
            message.content ? (
              <Typewriter_v2 text={message.content} speed={10} />
            ) : (
              <div className="flex items-center gap-1">
                <span className="w-1.5 h-1.5 bg-brand/60 rounded-full animate-bounce" style={{ animationDelay: "0ms" }} />
                <span className="w-1.5 h-1.5 bg-brand/60 rounded-full animate-bounce" style={{ animationDelay: "150ms" }} />
                <span className="w-1.5 h-1.5 bg-brand/60 rounded-full animate-bounce" style={{ animationDelay: "300ms" }} />
              </div>
            )
          ) : message.error ? (
            <div className="flex items-start gap-2">
              <AlertCircle className="h-4 w-4 shrink-0 mt-0.5" />
              <span>{message.error}</span>
            </div>
          ) : (
            <MarkdownContent
              remarkPlugins={[remarkMath]}
              rehypePlugins={[rehypeKatex]}
            >
              {message.content}
            </MarkdownContent>
          )}
        </div>

        {/* Meta info */}
        <div className="flex items-center gap-2 mt-1.5 text-[10px] text-text-dim/50">
          <span>{message.timestamp.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}</span>
          {message.tokens?.output && !message.isStreaming && (
            <span className="px-1.5 py-0.5 rounded bg-brand/10 text-brand/70 font-mono text-[9px]">
              {message.tokens.output} tok
            </span>
          )}
          {message.cost_usd !== undefined && message.cost_usd > 0 && (
            <span className="px-1.5 py-0.5 rounded bg-success/10 text-success/70 font-mono text-[9px]">
              {formatCost(message.cost_usd)}
            </span>
          )}
        </div>
        {message.memories_saved && message.memories_saved.length > 0 && (
          <div className="mt-1 flex flex-wrap gap-1">
            {message.memories_saved.map((m, i) => (
              <span key={i} className="text-[8px] px-1.5 py-0.5 rounded bg-warning/10 text-warning/70 truncate max-w-[200px]">
                {m}
              </span>
            ))}
          </div>
        )}
      </div>
    </div>
  );
});

// Input box - with shortcut hints
function ChatInput({ onSend, disabled, placeholder, authMissing, providerName }: { onSend: (msg: string) => void; disabled: boolean; placeholder: string; authMissing?: boolean; providerName?: string }) {
  const { t } = useTranslation();
  const [message, setMessage] = useState("");
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (message.trim() && !effectiveDisabled) {
      onSend(message);
      setMessage("");
    }
  };

  useEffect(() => {
    if (textareaRef.current) {
      textareaRef.current.style.height = "auto";
      textareaRef.current.style.height = Math.min(textareaRef.current.scrollHeight, 150) + "px";
    }
  }, [message]);

  const showingSlash = message.startsWith("/") && !message.includes(" ");
  const filteredCmds = showingSlash ? SLASH_COMMANDS.filter(c => c.cmd.startsWith(message)) : [];

  const effectiveDisabled = disabled || !!authMissing;

  return (
    <form onSubmit={handleSubmit} className="space-y-2">
      {/* Auth missing warning */}
      {authMissing && (
        <div className="flex items-center gap-2 rounded-xl border border-warning/30 bg-warning/5 px-4 py-2.5 text-sm text-warning">
          <AlertCircle className="h-4 w-4 flex-shrink-0" />
          <span>{t("chat.auth_missing", { provider: providerName || "unknown" })}</span>
        </div>
      )}
      {/* Slash command autocomplete */}
      {showingSlash && filteredCmds.length > 0 && (
        <div className="rounded-xl border border-border-subtle bg-surface shadow-lg p-1 mb-1">
          {filteredCmds.map(c => (
            <button key={c.cmd} type="button"
              onClick={() => { setMessage(c.cmd); onSend(c.cmd); setMessage(""); }}
              className="w-full flex items-center gap-2 px-3 py-1.5 rounded-lg hover:bg-main text-left transition-colors">
              <span className="text-xs font-mono font-bold text-brand">{c.cmd}</span>
              <span className="text-[10px] text-text-dim">{c.desc}</span>
            </button>
          ))}
        </div>
      )}
      <div className="flex gap-2 sm:gap-3 items-end">
        <div className="flex-1">
          <textarea
            ref={textareaRef}
            value={message}
            onChange={(e) => setMessage(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey && !e.metaKey) {
                e.preventDefault();
                handleSubmit(e);
              }
            }}
            placeholder={placeholder}
            disabled={effectiveDisabled}
            rows={1}
            className="w-full min-h-[44px] sm:min-h-[52px] max-h-[150px] rounded-2xl border border-border-subtle bg-surface px-3 sm:px-5 py-2.5 sm:py-3.5 text-sm focus:border-brand focus:ring-2 focus:ring-brand/10 outline-none resize-none placeholder:text-text-dim/40 shadow-sm"
          />
        </div>
        <button
          type="submit"
          disabled={!message.trim() || effectiveDisabled}
          className="group relative px-3.5 sm:px-5 py-2.5 sm:py-3.5 rounded-2xl bg-gradient-to-r from-brand to-brand/90 text-white font-bold text-sm shadow-lg shadow-brand/20 hover:shadow-brand/40 hover:-translate-y-0.5 transition-all duration-300 disabled:opacity-40 disabled:cursor-not-allowed disabled:hover:translate-y-0"
        >
          <Send className="h-4 w-4" />
          <span className="absolute -top-8 right-0 bg-surface border border-border-subtle rounded-lg px-2 py-1 text-[10px] text-text-dim opacity-0 group-hover:opacity-100 transition-opacity whitespace-nowrap hidden sm:block">
            {t("chat.send_hint")}
          </span>
        </button>
      </div>
    </form>
  );
}

// Connection status bar
function ConnectionBar({ agentName, isLoading, messageCount, onClear, wsConnected, modelName }: { agentName: string; isLoading: boolean; messageCount: number; onClear: () => void; wsConnected?: boolean; modelName?: string }) {
  const { t } = useTranslation();
  return (
    <div className="px-2 sm:px-4 py-2 sm:py-2.5 border-b border-border-subtle/50 bg-gradient-to-r from-surface to-transparent flex items-center justify-between">
      <div className="flex items-center gap-2 sm:gap-3 min-w-0 flex-1">
        <div className="relative">
          <Wifi className="h-3.5 w-3.5 text-success" />
          <span className="absolute inset-0 rounded-full bg-success/30 animate-pulse" />
        </div>
        <span className="text-xs font-semibold text-success uppercase tracking-wide hidden sm:inline">{t("chat.secure_link")}</span>
        {wsConnected && (
          <Badge variant="brand" dot>
            <Zap className="h-2.5 w-2.5 mr-0.5" />
            {t("chat.ws_connected")}
          </Badge>
        )}
        <span className="text-text-dim/30 hidden sm:inline">&bull;</span>
        <span className="text-xs font-medium text-text-dim truncate">{agentName}</span>
        {isLoading && (
          <span className="ml-2 px-2 py-0.5 rounded-full bg-brand/10 text-brand text-[10px] font-medium animate-pulse">
            {wsConnected ? t("chat.ws_streaming") : t("chat.generating")}
          </span>
        )}
      </div>
      <div className="flex items-center gap-2">
        {modelName && (
          <span className="hidden sm:inline text-[10px] text-text-dim/50 font-mono truncate max-w-[200px]">{modelName}</span>
        )}
        {messageCount > 0 && (
          <button onClick={onClear} className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium text-text-dim/60 hover:text-error hover:bg-error/5 transition-colors">
            <X className="h-3 w-3" />
            {t("chat.clear_chat")}
          </button>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Approval polling — uses React Query for caching, background pause, dedup
// ---------------------------------------------------------------------------
function useApprovalPoller(agentId: string | null) {
  const queryClient = useQueryClient();
  const approvalsQuery = useQuery({
    queryKey: ["approvals", "pending", agentId],
    queryFn: () => listPendingApprovals(agentId!),
    enabled: !!agentId,
    refetchInterval: 5000,
  });

  const remove = useCallback((id: string) => {
    queryClient.setQueryData<ApprovalItem[]>(
      ["approvals", "pending", agentId],
      (prev) => prev?.filter((a) => a.id !== id) ?? [],
    );
  }, [agentId, queryClient]);

  return { pendingApprovals: approvalsQuery.data ?? [], removeApproval: remove };
}

// ---------------------------------------------------------------------------
// Risk level styling helpers
// ---------------------------------------------------------------------------
const RISK_COLORS: Record<string, { bg: string; text: string; border: string }> = {
  critical: { bg: "bg-error/10", text: "text-error", border: "border-error/30" },
  high: { bg: "bg-warning/10", text: "text-warning", border: "border-warning/30" },
  medium: { bg: "bg-brand/10", text: "text-brand", border: "border-brand/30" },
  low: { bg: "bg-success/10", text: "text-success", border: "border-success/30" },
};

function riskStyle(level?: string) {
  return RISK_COLORS[(level || "low").toLowerCase()] ?? RISK_COLORS.low;
}

// ---------------------------------------------------------------------------
// Approval card displayed inline in the chat area
// ---------------------------------------------------------------------------
function ApprovalCard({ approval, onResolved }: { approval: ApprovalItem; onResolved: (id: string) => void }) {
  const { t } = useTranslation();
  const [resolving, setResolving] = useState<"approve" | "deny" | null>(null);

  const handleResolve = async (approved: boolean) => {
    setResolving(approved ? "approve" : "deny");
    try {
      await resolveApproval(approval.id, approved);
      onResolved(approval.id);
    } catch {
      // Approval may have already been resolved or timed out
      onResolved(approval.id);
    } finally {
      setResolving(null);
    }
  };

  const rs = riskStyle(approval.risk_level);

  return (
    <div className={`mx-auto w-full max-w-lg rounded-2xl border ${rs.border} ${rs.bg} p-4 shadow-lg animate-fade-in-up`}>
      {/* Header */}
      <div className="flex items-center gap-2 mb-3">
        <ShieldAlert className={`h-5 w-5 ${rs.text}`} />
        <span className={`text-xs font-black uppercase tracking-widest ${rs.text}`}>
          {t("chat.approval_required")}
        </span>
        {approval.risk_level && (
          <span className={`ml-auto text-[10px] font-bold uppercase px-2 py-0.5 rounded-full ${rs.bg} ${rs.text} border ${rs.border}`}>
            {approval.risk_level}
          </span>
        )}
      </div>

      {/* Tool info */}
      <div className="space-y-1 mb-4">
        <div className="flex items-center gap-2">
          <span className="text-[10px] font-bold uppercase text-text-dim tracking-wider">{t("chat.approval_tool")}</span>
          <code className="text-xs font-mono font-bold px-1.5 py-0.5 rounded bg-main">{approval.tool_name || "unknown"}</code>
        </div>
        {(approval.description || approval.action_summary || approval.action) && (
          <p className="text-sm text-text-dim leading-relaxed">
            {approval.description || approval.action_summary || approval.action}
          </p>
        )}
      </div>

      {/* Action buttons */}
      <div className="flex gap-3">
        <button
          onClick={() => handleResolve(true)}
          disabled={resolving !== null}
          className="flex-1 flex items-center justify-center gap-1.5 px-4 py-2.5 rounded-xl bg-success text-white font-bold text-sm shadow-lg shadow-success/20 hover:shadow-success/40 hover:-translate-y-0.5 transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed"
        >
          {resolving === "approve" ? (
            <RefreshCw className="h-4 w-4 animate-spin" />
          ) : (
            <CheckCircle className="h-4 w-4" />
          )}
          {t("approvals.approve")}
        </button>
        <button
          onClick={() => handleResolve(false)}
          disabled={resolving !== null}
          className="flex-1 flex items-center justify-center gap-1.5 px-4 py-2.5 rounded-xl bg-error text-white font-bold text-sm shadow-lg shadow-error/20 hover:shadow-error/40 hover:-translate-y-0.5 transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed"
        >
          {resolving === "deny" ? (
            <RefreshCw className="h-4 w-4 animate-spin" />
          ) : (
            <XCircle className="h-4 w-4" />
          )}
          {t("approvals.reject")}
        </button>
      </div>
    </div>
  );
}

export function ChatPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const search = useSearch({ from: "/chat" });
  const initialAgentId = search?.agentId || "";
  const [selectedAgentId, setSelectedAgentId] = useState(initialAgentId);
  const messagesEndRef = useRef<HTMLDivElement>(null);

  // Sync agent selection to URL search params
  const selectAgent = useCallback((id: string) => {
    setSelectedAgentId(id);
    navigate({ to: "/chat", search: { agentId: id }, replace: true });
  }, [navigate]);

  const agentsQuery = useQuery({ queryKey: ["agents", "list", "chat"], queryFn: listAgents, staleTime: 30000 });
  const agents = useMemo(() => [...(agentsQuery.data ?? [])].filter(a => !a.name.endsWith("-hand")).sort((a, b) => {
    // Auth missing → sort to bottom
    const aNoAuth = isAuthUnavailable(a.auth_status) ? 1 : 0;
    const bNoAuth = isAuthUnavailable(b.auth_status) ? 1 : 0;
    if (aNoAuth !== bNoAuth) return aNoAuth - bNoAuth;
    const aSusp = (a.state || "").toLowerCase() === "suspended" ? 1 : 0;
    const bSusp = (b.state || "").toLowerCase() === "suspended" ? 1 : 0;
    if (aSusp !== bSusp) return aSusp - bSusp;
    return a.name.localeCompare(b.name);
  }), [agentsQuery.data]);
  const { messages, isLoading, sendMessage, clearHistory, wsConnected } = useChatMessages(selectedAgentId || null, agents);
  const { pendingApprovals, removeApproval } = useApprovalPoller(selectedAgentId || null);
  const selectedAgent = agents.find(a => a.id === selectedAgentId);

  useEffect(() => {
    // Auto-select first running agent
    if (!selectedAgentId && agents.length > 0) {
      const firstRunning = agents.find(a => (a.state || "").toLowerCase() === "running");
      selectAgent((firstRunning || agents[0]).id);
    }
  }, [agents, selectedAgentId, selectAgent]);

  // Scroll to latest message — instant on agent switch, smooth on new messages
  const prevMsgCountRef = useRef(0);
  useEffect(() => {
    if (messages.length > 0) {
      const behavior = prevMsgCountRef.current === 0 ? "instant" as const : "smooth" as const;
      setTimeout(() => {
        messagesEndRef.current?.scrollIntoView({ behavior, block: "end" });
      }, 30);
    }
    prevMsgCountRef.current = messages.length;
  }, [messages]);

  return (
    <div className="flex h-[calc(100vh-100px)] sm:h-[calc(100vh-140px)] flex-col">
      {/* Header */}
      <header className="pb-2 sm:pb-4">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2 sm:gap-3">
            <div className="relative hidden sm:block">
              <Sparkles className="h-5 w-5 text-brand" />
              <span className="absolute inset-0 bg-brand/30 animate-pulse" />
            </div>
            <span className="text-brand font-bold uppercase tracking-widest text-[10px] hidden sm:inline">{t("chat.neural_terminal")}</span>
            <h1 className="text-xl sm:text-3xl font-extrabold tracking-tight">{t("chat.title")}</h1>
          </div>
          <button
            onClick={() => queryClient.invalidateQueries({ queryKey: ["agents", "list"] })}
            className="p-2 sm:p-2.5 rounded-xl hover:bg-surface-hover text-text-dim hover:text-brand transition-colors"
          >
            <RefreshCw className={`h-4 w-4 ${agentsQuery.isFetching ? "animate-spin" : ""}`} />
          </button>
        </div>
      </header>

      {/* Main content area */}
      <div className="flex flex-1 overflow-hidden rounded-2xl border border-border-subtle bg-surface shadow-xl ring-1 ring-black/5 dark:ring-white/5">
        {/* Left sidebar - Agent list */}
        <aside className="hidden md:flex w-64 flex-shrink-0 border-r border-border-subtle bg-main flex-col">
          <div className="p-4 border-b border-border-subtle">
            <h3 className="text-[10px] font-black uppercase tracking-[0.2em] text-text-dim/60">{t("nav.agents")}</h3>
          </div>
          <div className="flex-1 overflow-y-auto p-3 space-y-2 scrollbar-thin">
            {agents.length === 0 ? (
              <div className="p-4 text-center text-text-dim text-sm">{t("common.no_data")}</div>
            ) : (
              agents.map(agent => (
                <button
                  key={agent.id}
                  onClick={() => selectAgent(agent.id)}
                  className={`w-full flex items-center gap-3 p-3 rounded-xl transition-colors text-left group ${
                    selectedAgentId === agent.id
                      ? "bg-brand text-white shadow-lg shadow-brand/20"
                      : "hover:bg-surface-hover"
                  }`}
                >
                  <div className={`relative h-10 w-10 rounded-xl flex items-center justify-center font-black text-lg ${
                    selectedAgentId === agent.id ? "bg-white/20"
                    : (agent.state || "").toLowerCase() === "running" ? "bg-gradient-to-br from-brand/20 to-accent/20 text-brand"
                    : "bg-main text-text-dim/40"
                  }`}>
                    {agent.name.charAt(0).toUpperCase()}
                    {(agent.state || "").toLowerCase() === "running" ? (
                      <span className="absolute -bottom-0.5 -right-0.5 w-2.5 h-2.5 rounded-full bg-success border-2 border-white dark:border-surface animate-pulse" />
                    ) : (
                      <span className="absolute -bottom-0.5 -right-0.5 w-2.5 h-2.5 rounded-full bg-text-dim/30 border-2 border-white dark:border-surface" />
                    )}
                  </div>
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-1.5">
                      <p className={`text-sm font-bold truncate ${(agent.state || "").toLowerCase() !== "running" ? "opacity-50" : ""}`}>{agent.name}</p>
                      {agent.auth_status === "configured" && <span className={`flex-shrink-0 px-1 py-0.5 rounded text-[8px] font-bold uppercase leading-none ${selectedAgentId === agent.id ? "bg-white/20" : "bg-brand/10 text-brand"}`}>KEY</span>}
                      {agent.auth_status === "configured_cli" && <span className={`flex-shrink-0 px-1 py-0.5 rounded text-[8px] font-bold uppercase leading-none ${selectedAgentId === agent.id ? "bg-white/20" : "bg-accent/10 text-accent"}`}>CLI</span>}
                      {isAuthUnavailable(agent.auth_status) && <AlertCircle className="h-3 w-3 text-warning flex-shrink-0" />}
                    </div>
                    <p className={`text-[10px] truncate ${selectedAgentId === agent.id ? "text-white/70" : "text-text-dim"}`}>
                      {agent.model_provider || t("common.unknown")}
                    </p>
                  </div>
                  <ArrowRight className={`h-4 w-4 flex-shrink-0 transition-transform ${selectedAgentId === agent.id ? "rotate-90" : "opacity-0 group-hover:opacity-100"}`} />
                </button>
              ))
            )}
          </div>
        </aside>

        {/* Right side - Chat area */}
        <main className="flex-1 flex flex-col overflow-hidden bg-main/10 relative">
          {/* Background decoration */}
          <div className="absolute inset-0 pointer-events-none opacity-30">
            <div className="absolute top-0 left-0 w-64 h-64 bg-brand/5 rounded-full blur-3xl" />
            <div className="absolute bottom-0 right-0 w-48 h-48 bg-accent/5 rounded-full blur-3xl" />
          </div>

          {/* Mobile agent selector */}
          <div className="md:hidden px-3 py-2 border-b border-border-subtle bg-surface/80">
            <select
              value={selectedAgentId}
              onChange={(e) => selectAgent(e.target.value)}
              className="w-full rounded-lg border border-border-subtle bg-main px-3 py-2 text-sm font-bold outline-none focus:border-brand"
            >
              <option value="">{t("chat.select_agent")}</option>
              {agents.map(agent => (
                <option key={agent.id} value={agent.id}>
                  {agent.name} ({agent.state || "unknown"})
                </option>
              ))}
            </select>
          </div>

          {selectedAgentId && (
            <ConnectionBar
              agentName={selectedAgent?.name || ""}
              isLoading={isLoading}
              messageCount={messages.length}
              onClear={clearHistory}
              wsConnected={wsConnected}
              modelName={selectedAgent?.model_name}
            />
          )}

          {/* Message area */}
          <div className="flex-1 overflow-y-auto p-3 sm:p-6 space-y-4 sm:space-y-6 scrollbar-thin">
            {!selectedAgentId ? (
              <div className="h-full flex flex-col items-center justify-center text-center relative">
                <div className="absolute inset-0 bg-gradient-to-b from-transparent via-transparent to-main/50" />
                <div className="relative">
                  <div className="w-24 h-24 rounded-3xl bg-gradient-to-br from-brand/20 to-accent/20 flex items-center justify-center mb-6 ring-4 ring-brand/10">
                    <MessageCircle className="h-12 w-12 text-brand" />
                  </div>
                  <div className="absolute inset-0 rounded-3xl bg-brand/10 animate-pulse" />
                </div>
                <h3 className="text-2xl font-black mb-2">{t("chat.select_agent")}</h3>
                <p className="text-sm text-text-dim max-w-xs">{t("chat.select_agent_desc")}</p>
              </div>
            ) : messages.length === 0 ? (
              <div className="h-full flex flex-col items-center justify-center text-center">
                <div className="w-20 h-20 rounded-2xl bg-gradient-to-br from-brand/10 to-accent/10 flex items-center justify-center mb-4 ring-2 ring-brand/10">
                  <Bot className="h-10 w-10 text-brand" />
                </div>
                <h3 className="text-xl font-black">{selectedAgent?.name}</h3>
                <p className="text-sm text-text-dim mt-2">{t("chat.welcome_system")}</p>
              </div>
            ) : (
              <div className="space-y-6">
                {messages.map(msg => <MessageBubble key={msg.id} message={msg} />)}
                {/* Inline approval cards for pending requests */}
                {pendingApprovals.map(approval => (
                  <ApprovalCard key={approval.id} approval={approval} onResolved={removeApproval} />
                ))}
                <div ref={messagesEndRef} />
              </div>
            )}
          </div>

          {/* Input area */}
          <div className={`p-2 sm:p-4 border-t border-border-subtle bg-surface transition-opacity ${!selectedAgentId ? "opacity-30 pointer-events-none" : ""}`}>
            <ChatInput
              onSend={sendMessage}
              disabled={isLoading}
              placeholder={selectedAgentId ? t("chat.input_placeholder_with_agent", { name: selectedAgent?.name }) : t("chat.transmit_command")}
              authMissing={isAuthUnavailable(selectedAgent?.auth_status)}
              providerName={selectedAgent?.model_provider}
            />
          </div>
        </main>
      </div>
    </div>
  );
}
