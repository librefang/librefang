import { formatCost } from "../lib/format";
import { memo, useEffect, useMemo, useRef, useState, useCallback } from "react";
import rehypeKatex from "rehype-katex";
import remarkMath from "remark-math";
import { useTranslation } from "react-i18next";
import { useNavigate, useSearch } from "@tanstack/react-router";
import { buildAuthenticatedWebSocketUrl, sendAgentMessage, loadAgentSession } from "../api";
import { useQueryClient } from "@tanstack/react-query";
import type { ApprovalItem, SessionListItem, ModelItem, AgentTool, AgentItem } from "../api";
import { clearAgentHistory } from "../lib/http/client";
import { useFullConfig } from "../lib/queries/config";
import { useMediaProviders } from "../lib/queries/media";
import { useModels } from "../lib/queries/models";
import { usePendingApprovals } from "../lib/queries/approvals";
import { useAgents, useAgentSessions } from "../lib/queries/agents";
import { useActiveHandsWhen } from "../lib/queries/hands";
import { approvalKeys } from "../lib/queries/keys";
import { groupedPicker } from "../lib/chatPicker";
import { normalizeToolOutput } from "../lib/chat";
import { useTtsManager } from "../lib/tts";
import { MessageCircle, Send, Square, Bot, User, RefreshCw, AlertCircle, Wifi, Sparkles, X, ArrowRight, ArrowLeft, Zap, ShieldAlert, CheckCircle, XCircle, Clock, Plus, Trash2, ChevronDown, Loader2, Copy, Volume2, Pause, Download, Brain, Eye, EyeOff, Mic, MicOff, Globe } from "lucide-react";
import { Badge } from "../components/ui/Badge";
import { MarkdownContent } from "../components/ui/MarkdownContent";
import { useUIStore } from "../lib/store";
import { copyToClipboard } from "../lib/clipboard";
import { ToolCallCard } from "../components/ui/ToolCallCard";
import { filterVisible } from "../lib/hiddenModels";
import { useVoiceInput } from "../lib/useVoiceInput";
import { Typewriter_v2 } from "../components/Typewriter_v2";
import {
  useCreateAgentSession,
  useDeleteAgentSession,
  usePatchAgentConfig,
  useResolveApproval,
  useStopAgent,
} from "../lib/mutations/agents";
import "katex/dist/katex.min.css";

const isAuthUnavailable = (status?: string) =>
  !!status && status !== "configured" && status !== "validated_key" && status !== "configured_cli" && status !== "not_required" && status !== "auto_detected";

/**
 * Format a chat message's timestamp for the message footer.
 *
 * Shows time only when the message is from today, otherwise prepends a
 * short locale date so resumed sessions don't misleadingly show every
 * message as if it arrived at "today, 3:45 PM" (#2934).
 */
function formatMessageTimestamp(ts: Date): string {
  const now = new Date();
  const sameDay =
    ts.getFullYear() === now.getFullYear() &&
    ts.getMonth() === now.getMonth() &&
    ts.getDate() === now.getDate();
  const time = ts.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  if (sameDay) return time;
  return `${ts.toLocaleDateString([], { month: "short", day: "numeric" })} ${time}`;
}

interface ChatToolCall extends AgentTool {
  _call_id?: string;
}

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
  tools?: ChatToolCall[];
  /** Accumulated reasoning trace streamed via `thinking_delta` events. */
  thinking?: string;
  /** Whether the thinking block is collapsed in the UI. */
  thinkingCollapsed?: boolean;
}

// Slash commands — desc is an i18n key under "chat.cmd_*"
// noArgs: clicking fills + sends immediately; argsHint: shown as placeholder after completion
const SLASH_COMMANDS = [
  { cmd: "/help",    descKey: "cmd_help",    noArgs: true },
  { cmd: "/clear",   descKey: "cmd_clear",   noArgs: true },
  { cmd: "/agents",  descKey: "cmd_agents",  noArgs: true },
  { cmd: "/info",    descKey: "cmd_info",    noArgs: true },
  { cmd: "/new",     descKey: "cmd_new",     noArgs: true, backend: true },
  { cmd: "/compact", descKey: "cmd_compact", noArgs: true, backend: true },
  { cmd: "/reset",   descKey: "cmd_reset",   noArgs: true, backend: true },
  { cmd: "/reboot",  descKey: "cmd_reboot",  noArgs: true, backend: true },
  { cmd: "/stop",    descKey: "cmd_stop",    noArgs: true, backend: true },
  { cmd: "/model",   descKey: "cmd_model",   argsHint: "<provider/model>", backend: true },
  { cmd: "/usage",   descKey: "cmd_usage",   noArgs: true, backend: true },
  { cmd: "/context", descKey: "cmd_context", noArgs: true, backend: true },
  { cmd: "/verbose", descKey: "cmd_verbose", argsHint: "[level]", backend: true },
  { cmd: "/budget",  descKey: "cmd_budget",  noArgs: true, backend: true },
  { cmd: "/peers",   descKey: "cmd_peers",   noArgs: true, backend: true },
  { cmd: "/a2a",     descKey: "cmd_a2a",     noArgs: true, backend: true },
  { cmd: "/queue",   descKey: "cmd_queue",   noArgs: true, backend: true },
];

// Commands that require backend processing via WebSocket command protocol
const BACKEND_COMMANDS = SLASH_COMMANDS.filter(c => c.backend).map(c => c.cmd.slice(1));

const REMARK_PLUGINS = [remarkMath];
const REHYPE_PLUGINS = [rehypeKatex];

let _nextMessageId = 0;
function makeMessageId(prefix: string): string {
  _nextMessageId += 1;
  return `${prefix}-${Date.now()}-${_nextMessageId}`;
}


// WebSocket hook with auto-reconnect
function useWebSocket(agentId: string | null, sessionId: string | null = null) {
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

    // Issue #2959: per-connection session_id override. When the active
    // session is set (usually from the `?sessionId=` URL), append it to the
    // WS URL so every send on this socket targets that session regardless of
    // the agent's registry-canonical session — enabling multi-tab isolation.
    const base = `/api/agents/${encodeURIComponent(agentId)}/ws`;
    const wsPath = sessionId
      ? `${base}?session_id=${encodeURIComponent(sessionId)}`
      : base;
    const url = buildAuthenticatedWebSocketUrl(wsPath);

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
      const ws = wsRef.current;
      if (ws) {
        ws.onclose = null; // prevent reconnect on intentional close
        if (ws.readyState === WebSocket.CONNECTING) {
          // Closing a CONNECTING socket triggers a noisy browser warning;
          // defer the close until it actually opens.
          ws.onopen = () => ws.close();
        } else if (ws.readyState === WebSocket.OPEN) {
          ws.close();
        }
        wsRef.current = null;
      }
    };
  }, [agentId, sessionId]);

  return { ws: wsRef, wsConnected, onDropRef };
}

// Per-agent session cache — survives agent switches within the same page lifecycle
const sessionCache = new Map<string, ChatMessage[]>();

// Chat message management - includes history loading and sending (with WS streaming)
// sessionVersion: bump to force reload after session switch
function useChatMessages(agentId: string | null, agents: AgentItem[] = [], sessionVersion = 0, onModelSwitch?: () => void, onClearError?: (message: string) => void, sessionId: string | null = null) {
  const { t } = useTranslation();
  const stopAgentMutation = useStopAgent();
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  // Per-agent loading state. A single shared `isLoading` would freeze the
  // ChatInput on every agent while one of them is streaming (#2322). Keyed
  // by agentId so switching away from a busy agent unblocks the new one,
  // and coming back still reflects the in-flight status of the original.
  const [loadingAgents, setLoadingAgents] = useState<Record<string, boolean>>({});
  const isLoading = agentId ? loadingAgents[agentId] === true : false;
  const setAgentLoading = useCallback((id: string, on: boolean) => {
    setLoadingAgents(prev => {
      if ((prev[id] ?? false) === on) return prev;
      const next = { ...prev };
      if (on) next[id] = true; else delete next[id];
      return next;
    });
  }, []);
  // Tracks the bot message id of the most recent send per agent. Message
  // handlers for an older turn must NOT clear `isLoading` when a newer turn
  // is already in flight (user can now type + send while the previous turn's
  // `response` event is still pending — see `ChatInput` inputDisabled split).
  const latestTurnRef = useRef<Record<string, string>>({});
  // Per-agent in-flight turn lifecycle state, hoisted out of sendMessage's
  // closure so stopMessage can reach `cleanup`, `fallbackTimer`, and
  // `responded`. Without this, stopMessage POSTs /stop but the WS fallback
  // watchdog stays armed and 180s later re-sends the stopped message over
  // HTTP (#2787 review).
  const activeTurnsRef = useRef<Record<string, {
    cleanup?: () => void;
    fallbackTimer?: ReturnType<typeof setTimeout> | null;
    responded: boolean;
  }>>({});
  const finishTurnIfCurrent = useCallback((agent: string, botId: string) => {
    if (latestTurnRef.current[agent] === botId) {
      setAgentLoading(agent, false);
      delete latestTurnRef.current[agent];
    }
  }, [setAgentLoading]);
  // Garbage-collect loading flags for agents that no longer exist so the
  // map doesn't accumulate dead entries over a long session.
  useEffect(() => {
    const alive = new Set(agents.map(a => a.id));
    setLoadingAgents(prev => {
      const next: Record<string, boolean> = {};
      let changed = false;
      for (const [id, on] of Object.entries(prev)) {
        if (alive.has(id)) next[id] = on; else changed = true;
      }
      return changed ? next : prev;
    });

    const latestTurns = latestTurnRef.current;
    for (const id of Object.keys(latestTurns)) {
      if (!alive.has(id)) delete latestTurns[id];
    }
  }, [agents]);
  const { ws, wsConnected, onDropRef } = useWebSocket(agentId, sessionId);
  const addSkillOutput = useUIStore((s) => s.addSkillOutput);
  const deepThinking = useUIStore((s) => s.deepThinking);
  const showThinkingProcess = useUIStore((s) => s.showThinkingProcess);

  // Track the currently-viewed agent in a ref so async handlers registered
  // during a previous render can tell whether their target is still on screen.
  const currentAgentRef = useRef<string | null>(agentId);
  useEffect(() => { currentAgentRef.current = agentId; }, [agentId]);

  // Route a message update to either live React state (when the target agent
  // is on screen) or straight to the session cache (when the user has
  // switched away). Without this, updates against a swapped-out agent fall
  // into `setMessages(prev => prev.map(...))` whose `prev` is the OTHER
  // agent's array — the update becomes a silent no-op and the in-flight
  // response is lost once the user switches back.
  const updateAgentMessages = useCallback((
    id: string,
    updater: (msgs: ChatMessage[]) => ChatMessage[],
  ) => {
    if (id === currentAgentRef.current) {
      setMessages(updater);
    } else {
      const current = sessionCache.get(id) ?? [];
      sessionCache.set(id, updater(current));
    }
  }, []);

  // Save current messages to cache when switching away. The cleanup must
  // read the LATEST messages at unmount/agent-swap time, so we keep a
  // ref that tracks messages and only fire the save effect on agentId
  // changes (previously had no deps, so cleanup+re-run every render).
  const prevAgentRef = useRef<string | null>(null);
  const messagesRef = useRef<ChatMessage[]>(messages);
  messagesRef.current = messages;
  useEffect(() => {
    if (!agentId) {
      prevAgentRef.current = null;
      return;
    }

    prevAgentRef.current = agentId;
    const ownedAgentId = agentId;

    return () => {
      sessionCache.set(ownedAgentId, messagesRef.current);
    };
  }, [agentId]);

  // Load history — use cache if available, otherwise fetch
  // sessionVersion changes force a fresh load (skip cache)
  useEffect(() => {
    if (!agentId) { setMessages([]); return; }

    if (sessionVersion === 0) {
      const cached = sessionCache.get(agentId);
      if (cached) {
        setMessages(cached);
        return;
      }
    } else {
      sessionCache.delete(agentId);
    }

    setMessages([]);
    const loadId = agentId;
    setAgentLoading(loadId, true);
    loadAgentSession(loadId)
      .then(session => {
        if (session.messages?.length) {
          const historical: ChatMessage[] = session.messages.flatMap((msg, idx) => {
            let content: string;
            if (typeof msg.content === "string") {
              content = msg.content;
            } else if (Array.isArray(msg.content)) {
              // Extract only text blocks — skip tool_use/tool_result
              content = (msg.content as Array<Record<string, unknown>>)
                .filter((b) => b.type === "text" && typeof b.text === "string")
                .map((b) => b.text as string)
                .join("\n");
            } else {
              content = msg.content == null ? "" : String(msg.content);
            }

            const hasTools = msg.tools && msg.tools.length > 0;
            if (!content.trim() && !hasTools) return [];

            return [{
              id: `hist-${idx}`,
              role: msg.role === "User"
                ? "user"
                : msg.role === "System"
                  ? "system"
                  : "assistant",
              content,
              // Use the real server-side timestamp when available so
              // resumed sessions render the original send time instead of
              // the page-load time. Fall back to `now` only for messages
              // persisted before the backend started stamping (#2934).
              timestamp: msg.timestamp ? new Date(msg.timestamp) : new Date(),
              tools: msg.tools,
            }];
          });
          // Refresh the cache unconditionally — the data is still correct
          // for loadId. Only touch live React state when the user is still
          // viewing loadId; otherwise a slow A load resolving after the
          // user has swapped to B would overwrite B's displayed messages.
          sessionCache.set(loadId, historical);
          if (loadId === currentAgentRef.current) {
            setMessages(historical);
          }
        }
      })
      .catch(() => { /* Session load failure — user will see empty chat */ })
      .finally(() => setAgentLoading(loadId, false));
  }, [agentId, sessionVersion]);

  const clearHistory = useCallback(async () => {
    if (!agentId) {
      setMessages([]);
      return;
    }
    try {
      await clearAgentHistory(agentId);
      sessionCache.delete(agentId);
      if (prevAgentRef.current === agentId) {
        messagesRef.current = [];
      }
      setMessages([]);
    } catch (error) {
      onClearError?.(error instanceof Error ? error.message : t("common.error"));
    }
  }, [agentId, onClearError, t]);

  // Send message - WS first, HTTP fallback
  const sendMessage = useCallback(async (content: string) => {
    if (!content.trim()) return;
    const trimmed = content.trim();

    // Slash command handling
    if (trimmed.startsWith("/")) {
      const sysMsg = (text: string) => {
        setMessages(prev => [...prev,
          { id: makeMessageId("user"), role: "user" as const, content: trimmed, timestamp: new Date() },
          { id: makeMessageId("sys"), role: "system" as const, content: text, timestamp: new Date() }
        ]);
      };
      if (trimmed === "/help") {
        sysMsg(SLASH_COMMANDS.map(c =>
          `- \`${c.cmd}${c.argsHint ? " " + c.argsHint : ""}\` — ${t(`chat.${c.descKey}`)}`
        ).join("\n"));
        return;
      }
      if (trimmed === "/clear") {
        void clearHistory();
        return;
      }
      if (trimmed === "/agents") {
        const names = agents.map(a => `- **${a.name}** (${a.state || "unknown"})`).join("\n");
        sysMsg(names || t("chat.no_agents_available"));
        return;
      }
      if (trimmed === "/info") {
        const a = agents.find(a => a.id === agentId);
        sysMsg(a ? `**${a.name}**\n${t("chat.info_model")}: ${a.model_name || "-"}\n${t("chat.info_provider")}: ${a.model_provider || "-"}\n${t("chat.info_state")}: ${a.state}` : t("chat.no_agent_selected"));
        return;
      }

      // Backend commands: send as {"type": "command"} via WS, bypassing LLM
      const parts = trimmed.slice(1).split(/\s+/, 2);
      const cmd = parts[0];
      const cmdArgs = trimmed.slice(1 + cmd.length).trim();
      if (BACKEND_COMMANDS.includes(cmd)) {
        setMessages(prev => [...prev,
          { id: makeMessageId("user"), role: "user" as const, content: trimmed, timestamp: new Date() },
        ]);
        if (ws.current && ws.current.readyState === WebSocket.OPEN) {
          const handleCmdResponse = (event: MessageEvent) => {
            try {
              const data = JSON.parse(event.data as string);
              if (data.type === "command_result" || data.type === "error") {
                ws.current?.removeEventListener("message", handleCmdResponse);
                const responseText = data.message || data.content || "";
                // /new and /reset clear the backend session, so clear frontend too
                if (data.type === "command_result" && (cmd === "new" || cmd === "reset")) {
                  setMessages([
                    { id: makeMessageId("sys"), role: "system" as const, content: responseText, timestamp: new Date() },
                  ]);
                } else {
                  setMessages(prev => [...prev,
                    { id: makeMessageId("sys"), role: "system" as const, content: responseText, timestamp: new Date() },
                  ]);
                }
                // Refresh agent data so model/provider badge reflects the change
                if (data.type === "command_result" && cmd === "model") {
                  onModelSwitch?.();
                }
              }
            } catch { /* ignore non-JSON */ }
          };
          ws.current.addEventListener("message", handleCmdResponse);
          ws.current.send(JSON.stringify({ type: "command", command: cmd, args: cmdArgs }));
        } else {
          sysMsg(t("chat.ws_not_connected"));
        }
        return;
      }
    }

    if (!agentId) return;
    // Snapshot the agent at send time. The user may switch agents before
    // the response finishes; all completion/cleanup paths must still target
    // the original sender so we don't flip loading state / HTTP routes on
    // the agent the user is now looking at.
    const sendAgentId = agentId;

    const userMsg: ChatMessage = {
      id: makeMessageId("user"),
      role: "user",
      content: trimmed,
      timestamp: new Date(),
    };

    const botMsg: ChatMessage = {
      id: makeMessageId("bot"),
      role: "assistant",
      content: "",
      timestamp: new Date(),
      isStreaming: true,
    };

    setMessages(prev => [...prev, userMsg, botMsg]);
    setAgentLoading(sendAgentId, true);
    latestTurnRef.current[sendAgentId] = botMsg.id;

    // Helper: send via HTTP (used as primary fallback and WS drop recovery)
    const sendViaHttp = async () => {
      try {
        const response = await sendAgentMessage(sendAgentId, trimmed, {
          thinking: deepThinking,
          show_thinking: showThinkingProcess,
          session_id: sessionId,
        });
        const fullContent = response.response || "";
        updateAgentMessages(sendAgentId, prev => prev.map(m =>
          m.id === botMsg.id
            ? {
                ...m, content: fullContent, isStreaming: false,
                tokens: { output: response.output_tokens, input: response.input_tokens },
                cost_usd: response.cost_usd,
                memories_saved: response.memories_saved,
                memories_used: response.memories_used,
                thinking: response.thinking ?? m.thinking,
                thinkingCollapsed: m.thinkingCollapsed ?? true,
              }
            : m
        ));
        if (response.memories_saved?.length) {
          const agentName = agents.find(a => a.id === sendAgentId)?.name;
          response.memories_saved.forEach((mem: string) => {
            addSkillOutput({ skillName: "memory", agentId: sendAgentId, agentName, content: mem });
          });
        }
      } catch (err) {
        const errorMsg = err instanceof Error ? err.message : "Unknown error";
        updateAgentMessages(sendAgentId, prev => prev.map(m =>
          m.id === botMsg.id ? { ...m, isStreaming: false, error: errorMsg } : m
        ));
      } finally {
        finishTurnIfCurrent(sendAgentId, botMsg.id);
      }
    };

    // Try WebSocket streaming first
    if (wsConnected && ws.current && ws.current.readyState === WebSocket.OPEN) {
      try {
        // Hoist per-turn lifecycle state onto the ref so stopMessage can tear
        // down the WS listener + watchdog. Any prior entry for this agent
        // should already have been cleaned up by its own terminal path, but
        // be defensive and clear it.
        const prevTurn = activeTurnsRef.current[sendAgentId];
        if (prevTurn) {
          prevTurn.responded = true;
          if (prevTurn.fallbackTimer) clearTimeout(prevTurn.fallbackTimer);
          prevTurn.cleanup?.();
        }
        const turn: {
          cleanup?: () => void;
          fallbackTimer?: ReturnType<typeof setTimeout> | null;
          responded: boolean;
        } = { responded: false, fallbackTimer: null };
        activeTurnsRef.current[sendAgentId] = turn;

        const resetFallbackTimer = () => {
          if (turn.fallbackTimer) clearTimeout(turn.fallbackTimer);
          turn.fallbackTimer = setTimeout(() => {
            if (!turn.responded) {
              cleanup();
              sendViaHttp();
            }
          }, 180000);
        };

        const cleanup = () => {
          turn.responded = true;
          if (turn.fallbackTimer) { clearTimeout(turn.fallbackTimer); turn.fallbackTimer = null; }
          onDropRef.current = null;
          ws.current?.removeEventListener("message", handleMessage);
          if (activeTurnsRef.current[sendAgentId] === turn) {
            delete activeTurnsRef.current[sendAgentId];
          }
        };
        turn.cleanup = cleanup;

        // Set up message handler for this response
        const handleMessage = (event: MessageEvent) => {
          // Reset inactivity timeout on every event
          resetFallbackTimer();
          try {
            const data = JSON.parse(event.data as string);
            if (data.type === "text_delta") {
              const chunk = data.content || "";
              updateAgentMessages(sendAgentId, prev => prev.map(m =>
                m.id === botMsg.id ? { ...m, content: m.content + chunk, error: undefined } : m
              ));
            } else if (data.type === "thinking_delta") {
              const chunk = data.content || "";
              updateAgentMessages(sendAgentId, prev => prev.map(m =>
                m.id === botMsg.id
                  ? {
                      ...m,
                      thinking: (m.thinking ?? "") + chunk,
                      thinkingCollapsed: m.thinkingCollapsed ?? false,
                    }
                  : m
              ));
            } else if (data.type === "typing") {
              if (data.state === "stop") {
                updateAgentMessages(sendAgentId, prev => prev.map(m =>
                  m.id === botMsg.id ? { ...m, isStreaming: false } : m
                ));
              }
            } else if (data.type === "tool_start") {
              // Agent started a tool call — add a running tool entry
              const toolName = typeof data.tool === "string" ? data.tool : "unknown";
              const toolId = data.id || makeMessageId("tool");
              updateAgentMessages(sendAgentId, prev => prev.map(m =>
                m.id === botMsg.id
                  ? { ...m, tools: [...(m.tools || []), { name: toolName, running: true, expanded: false, is_error: false, input: undefined, result: undefined, _call_id: toolId }] }
                  : m
              ));
            } else if (data.type === "tool_end") {
              // LLM finished specifying the tool call — attach input but keep running
              // (tool_end means the LLM output is complete, NOT that the tool finished executing;
              // the tool stays "running" until tool_result arrives)
              const toolId = data.id;
              let parsedInput: unknown;
              try { parsedInput = typeof data.input === "string" ? JSON.parse(data.input) : data.input; } catch { parsedInput = data.input; }
              updateAgentMessages(sendAgentId, prev => prev.map(m => {
                if (m.id !== botMsg.id) return m;
                const tools = (m.tools || []).map(t =>
                  t._call_id === toolId ? { ...t, input: parsedInput } : t
                );
                return { ...m, tools };
              }));
            } else if (data.type === "tool_result") {
              // Attach result to the most recent tool matching by name
              const toolName = typeof data.tool === "string" ? data.tool : "";
              const isError = Boolean(data.is_error);
              const result = typeof data.result === "string" ? data.result : data.result != null ? JSON.stringify(data.result) : "";
              updateAgentMessages(sendAgentId, prev => prev.map(m => {
                if (m.id !== botMsg.id) return m;
                const tools = [...(m.tools || [])];
                // Find last tool with this name that has no result yet
                for (let i = tools.length - 1; i >= 0; i--) {
                  if (tools[i].name === toolName && tools[i].result === undefined) {
                    tools[i] = { ...tools[i], result, is_error: isError, running: false };
                    break;
                  }
                }
                return { ...m, tools };
              }));
              // Also keep the skill output panel behavior
              const entry = normalizeToolOutput(data);
              if (entry) {
                addSkillOutput({ skillName: entry.tool, agentId: sendAgentId, content: entry.content });
              }
            } else if (data.type === "silent_complete") {
              updateAgentMessages(sendAgentId, prev => prev.filter(m => m.id !== botMsg.id));
              finishTurnIfCurrent(sendAgentId, botMsg.id);
              cleanup();
            } else if (data.type === "error") {
              const error = data.content || "WebSocket error";
              updateAgentMessages(sendAgentId, prev => prev.map(m =>
                m.id === botMsg.id ? { ...m, isStreaming: false, error } : m
              ));
              // Release the loading flag so the send button re-enables and the
              // "streaming" badge drops — the user just saw an error and
              // should be able to retry immediately (#2745). Keep the WS
              // listener + recovery fallback timer alive in case the agent
              // does send a late `response` event; those handlers operate on
              // the same botMsg.id and will overwrite the error state if
              // recovery actually happens.
              finishTurnIfCurrent(sendAgentId, botMsg.id);
              // Don't cleanup the listener immediately — the agent may recover
              // and send a final response. Shorten the inactivity window to
              // 30s so the WS doesn't stay half-open forever if the failure
              // is terminal.
              if (turn.fallbackTimer) clearTimeout(turn.fallbackTimer);
              turn.fallbackTimer = setTimeout(() => {
                if (!turn.responded) { cleanup(); sendViaHttp(); }
              }, 30_000);
            } else if (data.type === "response") {
              updateAgentMessages(sendAgentId, prev => prev.map(m =>
                m.id === botMsg.id
                  ? {
                      ...m, content: data.content || m.content, isStreaming: false,
                      tokens: { output: data.output_tokens, input: data.input_tokens },
                      cost_usd: data.cost_usd,
                      memories_saved: data.memories_saved,
                      memories_used: data.memories_used,
                      thinking: typeof data.thinking === "string" ? data.thinking : m.thinking,
                      thinkingCollapsed: m.thinkingCollapsed ?? true,
                    }
                  : m
              ));
              finishTurnIfCurrent(sendAgentId, botMsg.id);
              cleanup();
            }
          } catch {
            // Non-JSON text chunk
            updateAgentMessages(sendAgentId, prev => prev.map(m =>
              m.id === botMsg.id ? { ...m, content: m.content + event.data } : m
            ));
          }
        };

        // Register fallback: if WS drops mid-stream, retry via HTTP
        onDropRef.current = () => {
          if (!turn.responded) {
            ws.current?.removeEventListener("message", handleMessage);
            if (activeTurnsRef.current[sendAgentId] === turn) {
              delete activeTurnsRef.current[sendAgentId];
            }
            sendViaHttp();
          }
        };

        ws.current.addEventListener("message", handleMessage);
        ws.current.send(JSON.stringify({
          type: "message",
          content: trimmed,
          thinking: deepThinking,
          show_thinking: showThinkingProcess,
        }));

        // Start inactivity timeout — resets on every received event
        resetFallbackTimer();

        return;
      } catch {
        // Fall through to HTTP
      }
    }

    // HTTP fallback — direct, no fake streaming
    await sendViaHttp();
  }, [agentId, agents, wsConnected, ws, deepThinking, showThinkingProcess, finishTurnIfCurrent, clearHistory]);

  // Abort an in-flight agent run. Hits the backend stop endpoint (which aborts
  // the tokio task on the kernel side) and optimistically finalizes any
  // streaming messages for this agent so the input re-enables immediately —
  // we don't wait for the WS to emit a terminal event.
  const stopMessage = useCallback(async () => {
    if (!agentId) return;
    const targetId = agentId;
    updateAgentMessages(targetId, prev => prev.map(m =>
      m.isStreaming ? { ...m, isStreaming: false } : m,
    ));
    const pendingBotId = latestTurnRef.current[targetId];
    if (pendingBotId) finishTurnIfCurrent(targetId, pendingBotId);
    // Tear down the in-flight WS turn: mark responded, clear the 180s/30s
    // watchdog, and detach the message listener. Without this the watchdog
    // would fire after the backend aborts (WS goes silent) and re-send the
    // stopped message over HTTP (#2787 review).
    const turn = activeTurnsRef.current[targetId];
    if (turn) {
      turn.responded = true;
      if (turn.fallbackTimer) {
        clearTimeout(turn.fallbackTimer);
        turn.fallbackTimer = null;
      }
      turn.cleanup?.();
      // cleanup() deletes the entry itself, but guard against custom
      // cleanup implementations.
      delete activeTurnsRef.current[targetId];
    }
    try {
      await stopAgentMutation.mutateAsync(targetId);
    } catch {
      // Backend stop failure is non-fatal for the UI — the run may have
      // completed between user click and request. UI is already unblocked.
    }
  }, [agentId, updateAgentMessages, finishTurnIfCurrent, stopAgentMutation]);

  return { messages, isLoading, sendMessage, stopMessage, clearHistory, wsConnected };
}

// Message bubble component — memoized to skip re-render during streaming of other messages
interface MessageBubbleProps {
  message: ChatMessage;
  usageFooter: string;
  onCopy?: (messageId: string, content: string) => void;
  copied?: boolean;
  onSpeak?: (messageId: string, content: string) => void;
  isSpeaking?: boolean;
  ttsStatus?: "idle" | "loading" | "playing" | "paused";
  ttsAvailable?: boolean;
}

const MessageBubble = memo(function MessageBubble({ message, usageFooter, onCopy, copied, onSpeak, isSpeaking, ttsStatus, ttsAvailable }: MessageBubbleProps) {
  const { t } = useTranslation();
  const isUser = message.role === "user";
  const [thinkingExpanded, setThinkingExpanded] = useState(() => !(message.thinkingCollapsed ?? false));

  if (message.role === "system") {
    const isMultiLine = message.content.includes("\n");
    if (isMultiLine) {
      return (
        <div className="flex justify-start py-2">
          <div className="max-w-[min(90%,56ch)] text-xs text-text-dim/70 [&_code]:text-primary [&_code]:font-mono [&_ul]:space-y-1 [&_ul>li]:list-none [&_ul>li]:flex [&_ul>li]:gap-2">
            <MarkdownContent>{message.content}</MarkdownContent>
          </div>
        </div>
      );
    }
    return (
      <div className="flex justify-center py-6">
        <div className="flex items-center gap-4">
          <div className="h-px w-16 bg-linear-to-r from-transparent to-border-subtle" />
          <span className="text-[10px] font-medium text-text-dim/40 tracking-[0.2em] uppercase">{message.content}</span>
          <div className="h-px w-16 bg-linear-to-l from-transparent to-border-subtle" />
        </div>
      </div>
    );
  }

  // Strip <tool_call>...</tool_call> XML blocks and orphaned closing tags from LLM output
  const displayContent = useMemo(() => {
    if (isUser) return message.content;
    return message.content
      .replace(/<tool_call>[\s\S]*?<\/tool_call>/g, "")
      .replace(/<\/?tool_calls?>/g, "")
      .trim();
  }, [message.content, isUser]);

  return (
    <div className={`flex animate-message-in ${isUser ? "justify-end" : "justify-start"}`}>
      <div className={`flex flex-col min-w-0 w-fit max-w-[90%] sm:max-w-[min(75%,70ch)] ${isUser ? "items-end" : "items-start"}`}>
        {/* Avatar + name */}
        <div className={`flex items-center gap-2 mb-1.5 ${isUser ? "self-end flex-row-reverse" : "self-start"}`}>
          <div className={`h-7 w-7 rounded-lg flex items-center justify-center ${
            isUser ? "bg-primary text-white shadow-sm" : "bg-surface border border-border-subtle"
          }`}>
            {isUser ? <User className="h-3.5 w-3.5" /> : <Bot className="h-3.5 w-3.5 text-primary" />}
          </div>
          <span className={`text-[11px] font-bold uppercase tracking-wider ${isUser ? "text-primary" : "text-text-dim"}`}>
            {isUser ? t("chat.you") : t("chat.bot")}
          </span>
        </div>

        {/* Thinking trace — collapsible, above tools */}
        {!isUser && message.thinking && message.thinking.trim().length > 0 && (
          <div className="w-full mb-1.5">
            <button
              type="button"
              onClick={() => setThinkingExpanded((v) => !v)}
              className="inline-flex items-center gap-1.5 px-2 py-1 rounded-md border border-border-subtle bg-surface text-[10px] font-medium text-text-dim hover:text-text hover:border-border transition-colors"
            >
              <Brain className="h-3 w-3" />
              <span>{t("chat.thinking_label")}</span>
              <ChevronDown
                className={`h-3 w-3 transition-transform ${thinkingExpanded ? "rotate-180" : ""}`}
              />
            </button>
            {thinkingExpanded && (
              <div className="mt-1 px-3 py-2 rounded-lg border border-border-subtle bg-surface/50 text-[12px] leading-relaxed text-text-dim break-words prose-sm">
                <MarkdownContent>{message.thinking ?? ""}</MarkdownContent>
              </div>
            )}
          </div>
        )}

        {/* Tool calls — rendered above text for assistant messages */}
        {!isUser && message.tools && message.tools.length > 0 && (
          <div className="w-full mb-1">
            {message.tools.map((tool, i) => (
              <ToolCallCard key={tool._call_id ?? `${tool.name}-${i}`} tool={tool} />
            ))}
          </div>
        )}

        {/* Message content */}
        {(displayContent || isUser || message.isStreaming || message.error) && (
        <div className={`relative px-3.5 py-2.5 rounded-2xl text-sm leading-relaxed shadow-sm min-w-0 [overflow-wrap:anywhere] ${
          isUser
            ? "bg-primary text-white rounded-tr-md"
            : message.error
              ? "bg-error/10 border border-error/20 text-error rounded-tl-md"
              : "bg-surface border border-border-subtle rounded-tl-md"
        }`}>
          {message.isStreaming ? (
            displayContent ? (
              <Typewriter_v2 text={displayContent} speed={10} />
            ) : (
              <div className="flex items-center gap-1 py-0.5">
                <span className="w-1.5 h-1.5 bg-primary/60 rounded-full animate-bounce" style={{ animationDelay: "0ms" }} />
                <span className="w-1.5 h-1.5 bg-primary/60 rounded-full animate-bounce" style={{ animationDelay: "150ms" }} />
                <span className="w-1.5 h-1.5 bg-primary/60 rounded-full animate-bounce" style={{ animationDelay: "300ms" }} />
              </div>
            )
          ) : message.error ? (
            <div className="flex items-start gap-2">
              <AlertCircle className="h-4 w-4 shrink-0 mt-0.5" />
              <span>{message.error}</span>
            </div>
          ) : isUser ? (
            // `break-words` only splits on spaces, so a bare URL/token
            // long enough to overflow its parent used to push the whole
            // 75%-wide bubble out of frame. `overflow-wrap: anywhere` is
            // the standard safe-wrap value that only kicks in when the
            // word would otherwise overflow, so normal text is untouched.
            <p className="whitespace-pre-line [overflow-wrap:anywhere]">{displayContent}</p>
          ) : (
            <MarkdownContent
              remarkPlugins={REMARK_PLUGINS}
              rehypePlugins={REHYPE_PLUGINS}
            >
              {displayContent}
            </MarkdownContent>
          )}
        </div>
        )}

        {/* Meta info + action buttons */}
        <div className={`flex items-center justify-between w-full mt-1.5 ${isUser ? "flex-row-reverse" : ""}`}>
          <div className="flex items-center gap-2 text-[10px] text-text-dim/50">
            <span>{formatMessageTimestamp(message.timestamp)}</span>
            {!message.isStreaming && usageFooter !== "off" && (() => {
              const showTokens = usageFooter === "full" || usageFooter === "tokens";
              const showCost = (usageFooter === "full" || usageFooter === "cost") && message.cost_usd !== undefined && message.cost_usd > 0;
              const hasInput = message.tokens?.input !== undefined && message.tokens.input > 0;
              const hasOutput = message.tokens?.output !== undefined && message.tokens.output > 0;
              if (!showTokens && !showCost) return null;
              if (showTokens && !hasInput && !hasOutput && !showCost) return null;
              const parts: string[] = [];
              if (showTokens && (hasInput || hasOutput)) parts.push(`${message.tokens?.input ?? 0} in, ${message.tokens?.output ?? 0} out`);
              if (showCost) parts.push(formatCost(message.cost_usd!));
              if (parts.length === 0) return null;
              return (
                <span className="px-1.5 py-0.5 rounded bg-primary/10 text-primary/70 font-mono text-[9px]">
                  {parts.join(" | ")}
                </span>
              );
            })()}
          </div>
          <div className="flex items-center gap-1">
            {!message.isStreaming && !message.error && message.role === "assistant" && ttsAvailable && onSpeak && (
              <button
                onClick={() => onSpeak(message.id, message.content)}
                className="h-6 w-6 rounded-md flex items-center justify-center text-text-dim/60 hover:text-primary hover:bg-surface-hover transition-colors"
                title={
                  ttsStatus === "loading" ? t("chat.tts_generating") :
                  isSpeaking && ttsStatus === "playing" ? t("chat.pause") :
                  isSpeaking && ttsStatus === "paused" ? t("chat.resume") :
                  t("chat.speak")
                }
                disabled={ttsStatus === "loading"}
              >
                {ttsStatus === "loading" && isSpeaking ? (
                  <Loader2 size={12} className="animate-spin" />
                ) : isSpeaking && ttsStatus === "playing" ? (
                  <Pause size={12} />
                ) : (
                  <Volume2 size={12} />
                )}
              </button>
            )}
            {!message.error && onCopy && (
              <button
                onClick={() => onCopy(message.id, message.content)}
                className={`h-6 w-6 rounded-md flex items-center justify-center transition-colors ${
                  copied
                    ? "text-success"
                    : "text-text-dim/60 hover:text-primary hover:bg-surface-hover"
                }`}
                title={copied ? t("chat.copied") : t("chat.copy")}
              >
                {copied ? <CheckCircle size={12} /> : <Copy size={12} />}
              </button>
            )}
          </div>
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
function ChatInput({ onSend, onStop, isStreaming, disabled, inputDisabled, placeholder, authMissing, authStatus, providerName, supportsThinking, sttAvailable }: { onSend: (msg: string) => void; onStop?: () => void; isStreaming?: boolean; disabled: boolean; inputDisabled?: boolean; placeholder: string; authMissing?: boolean; authStatus?: string; providerName?: string; supportsThinking?: boolean; sttAvailable?: boolean }) {
  const { t } = useTranslation();
  const [message, setMessage] = useState("");
  const [activeIndex, setActiveIndex] = useState(-1);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const deepThinking = useUIStore((s) => s.deepThinking);
  const showThinkingProcess = useUIStore((s) => s.showThinkingProcess);
  const setDeepThinking = useUIStore((s) => s.setDeepThinking);
  const setShowThinkingProcess = useUIStore((s) => s.setShowThinkingProcess);

  const voiceInput = useVoiceInput(useCallback((text: string) => {
    setMessage((prev) => (prev ? prev + " " + text : text));
  }, []));

  // ── Slash command completion ──────────────────────────────────────────────
  const isSlashPrefix = message.startsWith("/") && !message.includes(" ");
  const isModelArg = /^\/model\s/i.test(message);

  const filteredCmds = useMemo(
    () => isSlashPrefix ? SLASH_COMMANDS.filter(c => c.cmd.startsWith(message.toLowerCase())) : [],
    [isSlashPrefix, message],
  );

  const modelQuery = useModels({}, { enabled: isModelArg });

  const modelArg = isModelArg ? message.slice(message.indexOf(" ") + 1).toLowerCase() : "";
  const filteredModels = useMemo(() => {
    if (!isModelArg || !modelQuery.data?.models) return [];
    const all = modelQuery.data.models;
    const q = modelArg.trim();
    const matched = q
      ? all.filter(m =>
          m.id.toLowerCase().includes(q) ||
          m.provider.toLowerCase().includes(q) ||
          (m.display_name || "").toLowerCase().includes(q),
        )
      : all;
    return matched.slice(0, 12);
  }, [isModelArg, modelQuery.data, modelArg]);

  const hasDropdown = (isSlashPrefix && filteredCmds.length > 0) || (isModelArg && filteredModels.length > 0);
  const dropdownLen = isSlashPrefix ? filteredCmds.length : filteredModels.length;

  // Reset selection when list changes
  useEffect(() => { setActiveIndex(-1); }, [message]);

  const selectCmd = useCallback((c: typeof SLASH_COMMANDS[number]) => {
    if (c.noArgs) {
      onSend(c.cmd);
      setMessage("");
    } else {
      setMessage(c.cmd + " ");
      setTimeout(() => textareaRef.current?.focus(), 0);
    }
  }, [onSend]);

  const selectModel = useCallback((m: ModelItem) => {
    onSend(`/model ${m.provider}/${m.id}`);
    setMessage("");
  }, [onSend]);

  const handleKeyDown = useCallback((e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (!hasDropdown) return;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActiveIndex(i => Math.min(i + 1, dropdownLen - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActiveIndex(i => Math.max(i - 1, 0));
    } else if (e.key === "Escape") {
      e.preventDefault();
      setMessage("");
    } else if ((e.key === "Enter" || e.key === "Tab") && activeIndex >= 0) {
      e.preventDefault();
      if (isSlashPrefix) selectCmd(filteredCmds[activeIndex]);
      else if (isModelArg) selectModel(filteredModels[activeIndex]);
    }
  }, [hasDropdown, dropdownLen, activeIndex, isSlashPrefix, isModelArg, filteredCmds, filteredModels, selectCmd, selectModel]);

  // ─────────────────────────────────────────────────────────────────────────

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

  const effectiveDisabled = disabled || !!authMissing;
  // Textarea only locked while the agent is actively streaming text. Once the
  // model emits `typing:stop` the user can start composing the next message
  // even while background post-processing (memory save) is still running —
  // the send button stays gated on `effectiveDisabled` until the `response`
  // event arrives with final tokens/cost.
  const textareaDisabled = (inputDisabled ?? disabled) || !!authMissing;

  return (
    <form onSubmit={handleSubmit} className="space-y-2">
      {/* Auth missing warning */}
      {authMissing && (
        <div className="flex items-center gap-2 rounded-xl border border-warning/30 bg-warning/5 px-4 py-2.5 text-sm text-warning">
          <AlertCircle className="h-4 w-4 shrink-0" />
          <span>{authStatus === "local_offline"
            ? t("chat.provider_offline", { provider: providerName || "unknown" })
            : t("chat.auth_missing", { provider: providerName || "unknown" })}</span>
        </div>
      )}
      {/* Slash command autocomplete */}
      {isSlashPrefix && filteredCmds.length > 0 && (
        <div className="rounded-xl border border-border-subtle bg-surface shadow-lg p-1 mb-1">
          {filteredCmds.map((c, i) => (
            <button key={c.cmd} type="button"
              onClick={() => selectCmd(c)}
              className={`w-full flex items-center gap-2 px-3 py-1.5 rounded-lg text-left transition-colors ${i === activeIndex ? "bg-main" : "hover:bg-main"}`}>
              <span className="text-xs font-mono font-bold text-primary">{c.cmd}</span>
              {c.argsHint && <span className="text-[10px] font-mono text-text-dim/60">{c.argsHint}</span>}
              <span className="text-[10px] text-text-dim ml-auto">{t(`chat.${c.descKey}`)}</span>
            </button>
          ))}
        </div>
      )}
      {/* /model second-level model completion */}
      {isModelArg && filteredModels.length > 0 && (
        <div className="rounded-xl border border-border-subtle bg-surface shadow-lg p-1 mb-1 max-h-48 overflow-y-auto">
          {filteredModels.map((m, i) => (
            <button key={`${m.provider}/${m.id}`} type="button"
              onClick={() => selectModel(m)}
              className={`w-full flex items-center gap-2 px-3 py-1.5 rounded-lg text-left transition-colors ${i === activeIndex ? "bg-main" : "hover:bg-main"}`}>
              <span className="text-xs font-mono font-bold text-primary">{m.provider}</span>
              <span className="text-xs font-mono text-text">/</span>
              <span className="text-xs font-mono text-text">{m.id}</span>
              {m.display_name && <span className="text-[10px] text-text-dim ml-auto truncate max-w-[120px]">{m.display_name}</span>}
            </button>
          ))}
        </div>
      )}
      {/* Thinking mode toggles — only shown when the model supports thinking */}
      {supportsThinking && (
      <div className="flex items-center gap-2 flex-wrap">
        <button
          type="button"
          onClick={() => setDeepThinking(!deepThinking)}
          title={t("chat.deep_thinking_hint")}
          className={`inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full border text-[11px] font-medium transition-colors ${
            deepThinking
              ? "border-primary/40 bg-primary/10 text-primary"
              : "border-border-subtle bg-surface text-text-dim hover:text-text hover:border-border"
          }`}
        >
          <Brain className="h-3 w-3" />
          <span>{t("chat.deep_thinking")}</span>
        </button>
        <button
          type="button"
          onClick={() => setShowThinkingProcess(!showThinkingProcess)}
          title={t("chat.show_thinking_hint")}
          className={`inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full border text-[11px] font-medium transition-colors ${
            showThinkingProcess
              ? "border-primary/40 bg-primary/10 text-primary"
              : "border-border-subtle bg-surface text-text-dim hover:text-text hover:border-border"
          }`}
        >
          {showThinkingProcess ? <Eye className="h-3 w-3" /> : <EyeOff className="h-3 w-3" />}
          <span>{t("chat.show_thinking")}</span>
        </button>
      </div>
      )}
      <div className="flex gap-2 sm:gap-3 items-end">
        <div className="flex-1">
          <textarea
            ref={textareaRef}
            value={message}
            onChange={(e) => setMessage(e.target.value)}
            onKeyDown={(e) => {
              // Dropdown navigation takes priority
              if (hasDropdown) {
                handleKeyDown(e);
                if (e.defaultPrevented) return;
              }
              if (e.key === "Enter" && !e.shiftKey && !e.metaKey) {
                e.preventDefault();
                handleSubmit(e);
              }
            }}
            placeholder={voiceInput.isRecording ? t("chat.voice_recording") : voiceInput.isTranscribing ? t("chat.voice_transcribing") : placeholder}
            disabled={textareaDisabled}
            rows={1}
            className="w-full min-h-[44px] sm:min-h-[52px] max-h-[150px] rounded-2xl border border-border-subtle bg-surface px-3 sm:px-5 py-2.5 sm:py-3.5 text-sm focus:border-primary focus:ring-2 focus:ring-primary/10 outline-none resize-none placeholder:text-text-dim/40 shadow-sm"
          />
        </div>
        {voiceInput.isSupported && (
          <button
            type="button"
            onClick={sttAvailable ? voiceInput.toggleRecording : undefined}
            disabled={!sttAvailable || textareaDisabled || voiceInput.isTranscribing}
            title={!sttAvailable ? t("chat.voice_not_configured") : voiceInput.isRecording ? t("chat.voice_stop") : t("chat.voice_input")}
            className={`group relative px-3 sm:px-3.5 py-2.5 sm:py-3.5 rounded-2xl font-bold text-sm transition-all duration-300 disabled:opacity-40 disabled:cursor-not-allowed ${
              voiceInput.isRecording
                ? "bg-error/10 text-error border border-error/30 animate-pulse"
                : voiceInput.isTranscribing
                  ? "bg-warning/10 text-warning border border-warning/30"
                  : "bg-surface text-text-dim border border-border-subtle hover:text-text hover:border-border hover:-translate-y-0.5"
            }`}
          >
            {voiceInput.isRecording ? <MicOff className="h-4 w-4" /> : voiceInput.isTranscribing ? <Loader2 className="h-4 w-4 animate-spin" /> : <Mic className="h-4 w-4" />}
          </button>
        )}
        {isStreaming && onStop ? (
          <button
            type="button"
            onClick={onStop}
            title={t("chat.stop_hint")}
            className="group relative px-3.5 sm:px-5 py-2.5 sm:py-3.5 rounded-2xl bg-linear-to-r from-error to-error/90 text-white font-bold text-sm shadow-lg shadow-error/20 hover:shadow-error/40 hover:-translate-y-0.5 transition-all duration-300"
          >
            <Square className="h-4 w-4 fill-current" />
            <span className="absolute -top-8 right-0 bg-surface border border-border-subtle rounded-lg px-2 py-1 text-[10px] text-text-dim opacity-0 group-hover:opacity-100 transition-opacity whitespace-nowrap hidden sm:block">
              {t("chat.stop_hint")}
            </span>
          </button>
        ) : (
          <button
            type="submit"
            disabled={!message.trim() || effectiveDisabled}
            className="group relative px-3.5 sm:px-5 py-2.5 sm:py-3.5 rounded-2xl bg-linear-to-r from-brand to-brand/90 text-white font-bold text-sm shadow-lg shadow-brand/20 hover:shadow-brand/40 hover:-translate-y-0.5 transition-all duration-300 disabled:opacity-40 disabled:cursor-not-allowed disabled:hover:translate-y-0"
          >
            <Send className="h-4 w-4" />
            <span className="absolute -top-8 right-0 bg-surface border border-border-subtle rounded-lg px-2 py-1 text-[10px] text-text-dim opacity-0 group-hover:opacity-100 transition-opacity whitespace-nowrap hidden sm:block">
              {t("chat.send_hint")}
            </span>
          </button>
        )}
      </div>
    </form>
  );
}

// Connection status bar with session dropdown
function ConnectionBar({ agentName, isLoading, messageCount, onClear, onExport, wsConnected, modelName, modelProvider, sessions, activeSessionId, onSwitchSession, onNewSession, onDeleteSession, agentId, onModelChange, webSearchAugmentation, onWebSearchChange, webSearchAvailable, onOpenConfig }: {
  agentName: string; isLoading: boolean; messageCount: number; onClear: () => void; onExport: () => void; wsConnected?: boolean; modelName?: string; modelProvider?: string;
  sessions?: SessionListItem[]; activeSessionId?: string;
  onSwitchSession?: (sessionId: string) => void; onNewSession?: () => void; onDeleteSession?: (sessionId: string) => void;
  agentId: string; onModelChange: () => void;
  webSearchAugmentation?: "off" | "auto" | "always"; onWebSearchChange?: (mode: "off" | "auto" | "always") => void;
  webSearchAvailable?: boolean;
  onOpenConfig: () => void;
}) {
  const { t } = useTranslation();
  const [sessionOpen, setSessionOpen] = useState(false);
  const dropdownRef = useRef<HTMLDivElement>(null);

  // Model popover state
  const [modelOpen, setModelOpen] = useState(false);
  const modelRef = useRef<HTMLDivElement>(null);
  const [modelSearch, setModelSearch] = useState("");
  const [patchError, setPatchError] = useState<string | null>(null);
  const [patchPending, setPatchPending] = useState(false);
  const [optimisticModel, setOptimisticModel] = useState<string | null>(null);
  const [selectedProvider, setSelectedProvider] = useState<string>("");

  const hiddenModelKeys = useUIStore((s) => s.hiddenModelKeys);
  const hiddenSet = useMemo(() => new Set(hiddenModelKeys), [hiddenModelKeys]);
  const patchAgentConfigMutation = usePatchAgentConfig();
  const modelsQuery = useModels(
    { available: true },
    {
      enabled: modelOpen,
      // Model picker opens on demand. Keep query idle until popover visible.
      staleTime: 0,
    },
  );

  // Clear optimistic model once the real modelName catches up
  useEffect(() => {
    if (optimisticModel && modelName === optimisticModel) {
      setOptimisticModel(null);
    }
  }, [modelName, optimisticModel]);

  // Close session dropdown on outside click
  useEffect(() => {
    if (!sessionOpen) return;
    const handler = (e: MouseEvent) => {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) {
        setSessionOpen(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [sessionOpen]);

  // Close model popover on outside click
  useEffect(() => {
    if (!modelOpen) return;
    const handler = (e: MouseEvent) => {
      if (modelRef.current && !modelRef.current.contains(e.target as Node)) {
        setModelOpen(false);
        setSelectedProvider("");
        setModelSearch("");
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [modelOpen]);

  const models = modelsQuery.data?.models ?? [];
  const modelLoading = modelsQuery.isLoading || modelsQuery.isFetching;
  const modelFetchError = modelsQuery.error ? t("chat.unable_to_load_models") : null;
  const visibleModels = useMemo(() => filterVisible(models, hiddenSet), [models, hiddenSet]);

  // Unique providers derived from loaded models, sorted alphabetically
  const providers = useMemo(() => {
    const map = new Map<string, number>();
    for (const m of visibleModels) {
      map.set(m.provider, (map.get(m.provider) ?? 0) + 1);
    }
    return Array.from(map.entries())
      .map(([id, count]) => ({ id, count }))
      .sort((a, b) => a.id.localeCompare(b.id));
  }, [visibleModels]);

  // Models filtered by selected provider, then by search
  const filteredModels = useMemo(() => {
    let list = visibleModels;
    if (selectedProvider) {
      list = list.filter(m => m.provider === selectedProvider);
    }
    if (modelSearch) {
      const q = modelSearch.toLowerCase();
      list = list.filter(m => (m.id || "").toLowerCase().includes(q) || (m.display_name || "").toLowerCase().includes(q));
    }
    return list;
  }, [visibleModels, selectedProvider, modelSearch]);

  // Filter providers by search when in provider view
  const filteredProviders = useMemo(() => {
    if (!modelSearch) return providers;
    const q = modelSearch.toLowerCase();
    return providers.filter(p => p.id.toLowerCase().includes(q));
  }, [providers, modelSearch]);

  async function handleSelectModel(model: ModelItem) {
    const prev = optimisticModel ?? modelName ?? null;
    setOptimisticModel(model.id);
    setPatchPending(true);
    setPatchError(null);
    try {
      await patchAgentConfigMutation.mutateAsync({
        agentId,
        config: { model: model.id, provider: model.provider },
      });
      setModelOpen(false);
      setSelectedProvider("");
      setModelSearch("");
      onModelChange(); // invalidates queries; useEffect clears optimisticModel when modelName catches up
    } catch {
      setOptimisticModel(prev);
      setPatchError(t("chat.model_update_failed"));
    } finally {
      setPatchPending(false);
    }
  }

  return (
    <div className="px-2 sm:px-4 py-2 sm:py-2.5 border-b border-border-subtle/50 bg-linear-to-r from-surface to-transparent flex items-center justify-between">
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
          <span className="ml-2 px-2 py-0.5 rounded-full bg-primary/10 text-primary text-[10px] font-medium animate-pulse">
            {wsConnected ? t("chat.ws_streaming") : t("chat.generating")}
          </span>
        )}
      </div>
      <div className="flex items-center gap-2">
        {/* Model switcher */}
        <div className="relative hidden sm:block" ref={modelRef}>
          <button
            onClick={() => { setModelOpen(v => { if (v) { setSelectedProvider(""); setModelSearch(""); } return !v; }); }}
            className="flex items-center gap-1 px-2 py-1 rounded-lg text-[10px] font-mono text-text-dim/50 hover:text-text hover:bg-surface-hover transition-colors truncate max-w-[200px]"
            title={t("chat.switch_model")}
          >
            <span className="truncate">{optimisticModel ?? modelName ?? t("chat.no_model")}</span>
            <ChevronDown className={`h-2.5 w-2.5 shrink-0 transition-transform ${modelOpen ? "rotate-180" : ""}`} />
          </button>
          {modelOpen && (
            <div className="absolute right-0 top-full mt-1 w-80 bg-surface border border-border-subtle rounded-xl shadow-xl z-50 overflow-hidden">
              {/* Header */}
              <div className="p-2 border-b border-border-subtle/50 flex items-center gap-2">
                {selectedProvider && (
                  <button
                    onClick={() => { setSelectedProvider(""); setModelSearch(""); }}
                    className="p-0.5 rounded hover:bg-surface-hover transition-colors"
                  >
                    <ArrowLeft className="h-3.5 w-3.5 text-text-dim" />
                  </button>
                )}
                <span className="text-[10px] font-semibold text-text-dim/50 uppercase tracking-wider px-1">
                  {selectedProvider || t("chat.select_provider", { defaultValue: "Select Provider" })}
                </span>
              </div>
              {/* Search */}
              <div className="p-2 border-b border-border-subtle/50">
                <input
                  autoFocus
                  type="text"
                  value={modelSearch}
                  onChange={e => setModelSearch(e.target.value)}
                  placeholder={selectedProvider ? t("chat.search_models") : t("chat.search_providers", { defaultValue: "Search providers..." })}
                  className="w-full px-2.5 py-1.5 text-xs rounded-lg bg-main border border-border-subtle focus:outline-none focus:border-primary"
                />
                {patchError && (
                  <p className="text-error text-[10px] mt-1.5 px-1">{patchError}</p>
                )}
              </div>
              <div className={`max-h-64 overflow-y-auto scrollbar-thin p-1.5 space-y-0.5 ${patchPending ? "pointer-events-none opacity-60" : ""}`}>
                {modelLoading && (
                  <div className="flex items-center gap-2 px-2.5 py-2 text-xs text-text-dim">
                    <Loader2 className="h-3 w-3 animate-spin" />
                    {t("chat.loading_models")}
                  </div>
                )}
                {modelFetchError && (
                  <div className="px-2.5 py-2 space-y-1.5">
                    <p className="text-xs text-error">{modelFetchError}</p>
                    <button
                      onClick={() => { void modelsQuery.refetch(); }}
                      className="text-[10px] text-primary hover:underline"
                    >
                      {t("chat.retry")}
                    </button>
                  </div>
                )}

                {/* Provider list view */}
                {!modelLoading && !modelFetchError && !selectedProvider && (() => {
                  // Use the agent's known provider (from props) to highlight the current provider.
                  // Falls back to scanning the model list only if the prop is unavailable.
                  const agentProvider = modelProvider || models.find(m => m.id === (optimisticModel ?? modelName))?.provider;
                  return (
                  <>
                    {filteredProviders.length === 0 && (
                      <p className="px-2.5 py-2 text-xs text-text-dim">{t("chat.no_models_found")}</p>
                    )}
                    {filteredProviders.map(p => {
                      const isCurrent = p.id === agentProvider;
                      return (
                        <div
                          key={p.id}
                          onClick={() => { setSelectedProvider(p.id); setModelSearch(""); }}
                          className={`flex items-center justify-between px-2.5 py-2 rounded-lg cursor-pointer transition-colors ${isCurrent ? "bg-primary/10 text-primary" : "hover:bg-surface-hover text-text-dim"}`}
                        >
                          <div className="flex items-center gap-2">
                            {isCurrent && <span className="w-1.5 h-1.5 rounded-full bg-success shrink-0" />}
                            <span className="text-xs font-medium">{p.id}</span>
                          </div>
                          <div className="flex items-center gap-1.5">
                            <span className="text-[10px] text-text-dim/40">{p.count} {p.count === 1 ? "model" : "models"}</span>
                            <ArrowRight className="h-3 w-3 text-text-dim/30" />
                          </div>
                        </div>
                      );
                    })}
                  </>
                  );
                })()}

                {/* Model list view (filtered by selected provider) */}
                {!modelLoading && !modelFetchError && selectedProvider && (
                  <>
                    {filteredModels.length === 0 && (
                      <p className="px-2.5 py-2 text-xs text-text-dim">{t("chat.no_models_found")}</p>
                    )}
                    {filteredModels.map(model => {
                      const isActive = model.id === (optimisticModel ?? modelName) && model.provider === selectedProvider;
                      return (
                        <div
                          key={`${model.provider}/${model.id}`}
                          onClick={() => { if (!isActive) handleSelectModel(model); }}
                          className={`flex items-center gap-2 px-2.5 py-2 rounded-lg cursor-pointer transition-colors ${isActive ? "bg-primary/10 text-primary" : "hover:bg-surface-hover text-text-dim"}`}
                        >
                          {isActive && patchPending
                            ? <Loader2 className="h-3 w-3 animate-spin shrink-0" />
                            : isActive && <span className="w-1.5 h-1.5 rounded-full bg-success shrink-0" />
                          }
                          <span className="text-xs font-medium truncate">{model.display_name || model.id}</span>
                        </div>
                      );
                    })}
                  </>
                )}
              </div>
            </div>
          )}
        </div>
        {/* Web Search toggle (off → auto → always → off) with config check */}
        {onWebSearchChange && (() => {
          const mode = webSearchAugmentation || "auto";
          const isActive = mode !== "off";
          const noKey = !webSearchAvailable;
          return (
            <div className="hidden sm:flex items-center gap-1.5">
              <button
                onClick={() => {
                    if (noKey && mode === "off") {
                      // No search key configured — navigate to Config page Web section
                      onOpenConfig();
                      return;
                    }
                  const cycle: Record<string, "off" | "auto" | "always"> = { off: "auto", auto: "always", always: "off" };
                  onWebSearchChange(cycle[mode] || "auto");
                }}
                title={noKey ? t("chat.web_search_no_key", { defaultValue: "No search API key configured. Click to open settings." }) : undefined}
                className={`flex items-center gap-1 px-2 py-1 rounded-lg text-[10px] font-mono transition-colors ${
                  noKey && !isActive
                    ? "text-warning/50 hover:text-warning hover:bg-warning/10"
                    : mode === "always"
                      ? "text-primary bg-primary/10 hover:bg-primary/20"
                      : mode === "auto"
                        ? "text-text-dim/50 hover:text-text hover:bg-surface-hover"
                        : "text-text-dim/30 hover:text-text-dim/60 hover:bg-surface-hover"
                }`}
              >
                <Globe className="h-3 w-3" />
                <span>{noKey && !isActive
                  ? t("chat.web_search_setup", { defaultValue: "Search" })
                  : mode === "always" ? t("common.always", { defaultValue: "Always" }) : mode === "auto" ? t("common.auto", { defaultValue: "Auto" }) : t("common.off", { defaultValue: "Off" })
                }</span>
                {noKey && !isActive && <AlertCircle className="h-2.5 w-2.5 text-warning" />}
              </button>
              {isActive && noKey && (
                <button
                  onClick={onOpenConfig}
                  className="text-[9px] text-warning hover:text-warning/80 underline hidden xl:inline"
                >
                  {t("chat.web_search_configure", { defaultValue: "Configure API key" })}
                </button>
              )}
            </div>
          );
        })()}
        {/* Session dropdown */}
        {sessions && sessions.length > 0 && (
          <div className="relative" ref={dropdownRef}>
            <button
              onClick={() => setSessionOpen(v => !v)}
              className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg text-xs font-medium text-text-dim/70 hover:text-text hover:bg-surface-hover transition-colors"
            >
              <Clock className="h-3 w-3" />
              <span className="hidden sm:inline truncate max-w-[100px]">
                {(() => {
                  const active = sessions.find(s => s.session_id === activeSessionId);
                  return active?.label || activeSessionId?.slice(0, 8) || t("chat.session");
                })()}
              </span>
              <ChevronDown className={`h-3 w-3 transition-transform ${sessionOpen ? "rotate-180" : ""}`} />
            </button>
            {sessionOpen && (
              <div className="absolute right-0 top-full mt-1 w-72 bg-surface border border-border-subtle rounded-xl shadow-xl z-50 overflow-hidden">
                <div className="p-2 border-b border-border-subtle/50">
                  <span className="text-[10px] font-semibold text-text-dim/50 uppercase tracking-wider px-2">{t("chat.sessions_title", { defaultValue: "Sessions" })}</span>
                </div>
                <div className="max-h-64 overflow-y-auto scrollbar-thin p-1.5 space-y-0.5">
                  {sessions.map(session => {
                    const isActive = session.session_id === activeSessionId;
                    return (
                      <div
                        key={session.session_id}
                        className={`group flex items-center gap-2 px-2.5 py-2 rounded-lg cursor-pointer transition-colors ${isActive ? "bg-primary/10 text-primary" : "hover:bg-surface-hover text-text-dim"}`}
                        onClick={() => {
                          if (!isActive) {
                            onSwitchSession?.(session.session_id);
                            setSessionOpen(false);
                          }
                        }}
                      >
                        <div className="flex-1 min-w-0">
                          <div className="flex items-center gap-1.5">
                            {isActive && <span className="w-1.5 h-1.5 rounded-full bg-success shrink-0" />}
                            <span className="text-xs font-medium truncate">
                              {session.label || session.session_id?.slice(0, 12)}
                            </span>
                          </div>
                          <div className="flex items-center gap-2 mt-0.5">
                            <span className="text-[10px] text-text-dim/50">{session.message_count ?? 0} msgs</span>
                            {session.created_at && (
                              <span className="text-[10px] text-text-dim/40">{new Date(session.created_at).toLocaleDateString()}</span>
                            )}
                          </div>
                        </div>
                        {!isActive && onDeleteSession && (
                          <button
                            onClick={(e) => { e.stopPropagation(); onDeleteSession(session.session_id); }}
                            className="opacity-0 group-hover:opacity-100 p-1 rounded hover:bg-error/10 hover:text-error transition-all"
                            title={t("chat.delete_session", { defaultValue: "Delete session" })}
                          >
                            <Trash2 className="h-3 w-3" />
                          </button>
                        )}
                      </div>
                    );
                  })}
                </div>
                {onNewSession && (
                  <div className="p-1.5 border-t border-border-subtle/50">
                    <button
                      onClick={() => { onNewSession(); setSessionOpen(false); }}
                      className="w-full flex items-center gap-2 px-2.5 py-2 rounded-lg text-xs font-medium text-primary hover:bg-primary/5 transition-colors"
                    >
                      <Plus className="h-3.5 w-3.5" />
                      {t("chat.new_session", { defaultValue: "New session" })}
                    </button>
                  </div>
                )}
              </div>
            )}
          </div>
        )}
        {messageCount > 0 && (
          <>
            <button
              onClick={onExport}
              title={t("chat.export_markdown", { defaultValue: "Export as Markdown" })}
              className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium text-text-dim/60 hover:text-primary hover:bg-primary/5 transition-colors"
            >
              <Download className="h-3 w-3" />
              <span className="hidden sm:inline">{t("chat.export", { defaultValue: "Export" })}</span>
            </button>
            <button onClick={onClear} className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium text-text-dim/60 hover:text-error hover:bg-error/5 transition-colors">
              <X className="h-3 w-3" />
              {t("chat.clear_chat")}
            </button>
          </>
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
  const approvalsQuery = usePendingApprovals(agentId ?? undefined, {
    enabled: Boolean(agentId), // Poll approvals only when chat has concrete agent selected.
  });

  const remove = useCallback((id: string) => {
    queryClient.setQueryData<ApprovalItem[]>(
      approvalKeys.pending(agentId),
      (prev) => prev?.filter((a: ApprovalItem) => a.id !== id) ?? [],
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
  medium: { bg: "bg-primary/10", text: "text-primary", border: "border-primary/30" },
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
  const resolveApprovalMutation = useResolveApproval();

  const handleResolve = async (approved: boolean) => {
    setResolving(approved ? "approve" : "deny");
    try {
      await resolveApprovalMutation.mutateAsync({ id: approval.id, approved });
      onResolved(approval.id);
    } catch {
      // Approval may have already been resolved or timed out
      onResolved(approval.id);
    } finally {
      setResolving(null);
    }
  };

  const rs = riskStyle(approval.risk_level);

  const riskLabel = approval.risk_level
    ? t(`chat.approval_risk_${approval.risk_level}`, { defaultValue: approval.risk_level })
    : null;

  return (
    <div className={`mx-auto w-full max-w-lg rounded-2xl border ${rs.border} ${rs.bg} p-4 shadow-lg animate-fade-in-up`}>
      {/* Header */}
      <div className="flex items-center gap-2 mb-3">
        <ShieldAlert className={`h-5 w-5 ${rs.text}`} />
        <span className={`text-xs font-black uppercase tracking-widest ${rs.text}`}>
          {t("chat.approval_required")}
        </span>
        {riskLabel && (
          <span className={`ml-auto text-[10px] font-bold uppercase px-2 py-0.5 rounded-full ${rs.bg} ${rs.text} border ${rs.border}`}>
            {riskLabel}
          </span>
        )}
      </div>

      {/* Tool info */}
      <div className="space-y-2 mb-4">
        <div className="flex items-center gap-2">
          <span className="text-[10px] font-bold uppercase text-text-dim tracking-wider">{t("chat.approval_tool")}</span>
          <code className="text-xs font-mono font-bold px-1.5 py-0.5 rounded bg-main">{approval.tool_name || "unknown"}</code>
        </div>
        {(approval.description || approval.action_summary || approval.action) && (
          <p className="text-xs text-text-dim leading-relaxed bg-main/50 rounded-lg px-3 py-2 font-mono whitespace-pre-wrap break-all">
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
  const navigate = useNavigate();
  const search = useSearch({ from: "/chat" });
  const initialAgentId = search?.agentId || "";
  const [selectedAgentId, setSelectedAgentId] = useState(initialAgentId);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const [copiedMessageId, setCopiedMessageId] = useState<string | null>(null);
  const addToast = useUIStore((s) => s.addToast);
  const createSessionMutation = useCreateAgentSession();
  // NOTE: switch_agent_session is no longer called from ChatPage — see issue
  // #2959. Sessions are URL-driven per tab; other callers (CLI, cron) still
  // use the endpoint for registry-canonical switching.
  const deleteSessionMutation = useDeleteAgentSession();
  const patchAgentConfigMutation = usePatchAgentConfig();

  // Sync agent selection to URL search params
  const selectAgent = useCallback((id: string) => {
    setSelectedAgentId(id);
    navigate({ to: "/chat", search: { agentId: id }, replace: true });
  }, [navigate]);

  // Check TTS provider availability
  const mediaProvidersQuery = useMediaProviders();
  const ttsAvailable = useMemo(
    () => (mediaProvidersQuery.data ?? []).some(p => p.configured && p.capabilities.includes("text_to_speech")),
    [mediaProvidersQuery.data],
  );

  const handleCopy = useCallback(async (messageId: string, content: string) => {
    if (await copyToClipboard(content)) {
      setCopiedMessageId(messageId);
      setTimeout(() => setCopiedMessageId(null), 1500);
    } else {
      addToast(t("common.copy_failed"), "error");
    }
  }, [addToast, t]);

  const configQuery = useFullConfig();
  const usageFooter = (configQuery.data as Record<string, unknown>)?.usage_footer as string | undefined ?? "full";
  const mediaConfigRaw = (configQuery.data as Record<string, unknown>)?.media as Record<string, unknown> | undefined;
  const sttAvailable = mediaConfigRaw?.stt_available === true;
  const ttsConfigRaw = (configQuery.data as Record<string, unknown>)?.tts as Record<string, unknown> | undefined;
  const ttsProvider = ttsConfigRaw?.provider as string | undefined;
  const ttsSpeechConfig = useMemo(() => {
    if (!ttsConfigRaw || !ttsProvider) return undefined;
    const subKey = ttsProvider === "google_tts" ? "google" : ttsProvider;
    const sub = ttsConfigRaw[subKey] as Record<string, unknown> | undefined;
    if (!sub) return { provider: ttsProvider };
    switch (ttsProvider) {
      case "google_tts":
        return {
          provider: ttsProvider,
          voice: sub.voice as string | undefined,
          language: sub.language_code as string | undefined,
          speed: sub.speaking_rate as number | undefined,
        };
      case "openai":
        return {
          provider: ttsProvider,
          voice: sub.voice as string | undefined,
          speed: sub.speed as number | undefined,
        };
      case "elevenlabs":
        return {
          provider: ttsProvider,
          voice: sub.voice_id as string | undefined,
        };
      default:
        return { provider: ttsProvider, voice: sub.voice as string | undefined };
    }
  }, [ttsConfigRaw, ttsProvider]);
  const tts = useTtsManager(ttsSpeechConfig);

  // Stop TTS when agent changes
  useEffect(() => {
    tts.stop();
  }, [selectedAgentId, tts.stop]);

  const [showHandAgents, setShowHandAgents] = useState<boolean>(() => {
    if (typeof window === "undefined") return false;
    return localStorage.getItem("librefang.chat.show_hand_agents") === "1";
  });
  useEffect(() => {
    if (typeof window === "undefined") return;
    localStorage.setItem(
      "librefang.chat.show_hand_agents",
      showHandAgents ? "1" : "0",
    );
  }, [showHandAgents]);

  const agentsQuery = useAgents({ includeHands: showHandAgents });
  // Check if web search is available (any search API key configured)
  const webSearchAvailable = ((configQuery.data as Record<string, unknown>)?.web as Record<string, unknown> | undefined)?.search_available === true;
  const handsQuery = useActiveHandsWhen(showHandAgents);

  const sortedAgents = useMemo(
    () =>
      [...(agentsQuery.data ?? [])].sort((a, b) => {
        // Auth missing → sort to bottom
        const aNoAuth = isAuthUnavailable(a.auth_status) ? 1 : 0;
        const bNoAuth = isAuthUnavailable(b.auth_status) ? 1 : 0;
        if (aNoAuth !== bNoAuth) return aNoAuth - bNoAuth;
        const aSusp = (a.state || "").toLowerCase() === "suspended" ? 1 : 0;
        const bSusp = (b.state || "").toLowerCase() === "suspended" ? 1 : 0;
        if (aSusp !== bSusp) return aSusp - bSusp;
        return a.name.localeCompare(b.name);
      }),
    [agentsQuery.data],
  );

  const picker = useMemo(
    () =>
      groupedPicker(sortedAgents, handsQuery.data, showHandAgents),
    [sortedAgents, handsQuery.data, showHandAgents],
  );

  // Flat view used by downstream consumers (default-selection logic,
  // selectedAgent lookup, export). Standalone first, then groups in order.
  const agents = useMemo(
    () => [
      ...picker.standalone,
      ...picker.handGroups.flatMap((g) => g.agents),
    ],
    [picker],
  );
  // Session state — bump version to force message reload after switch
  const [sessionVersion, setSessionVersion] = useState(0);
  // URL-driven session selection (issue #2959). When a `sessionId` query
  // param is present, it wins over the server's canonical active session so
  // two browser tabs on the same agent can hold independent sessions.
  const urlSessionId = search?.sessionId || null;
  const { messages, isLoading, sendMessage, stopMessage, clearHistory, wsConnected } = useChatMessages(
    selectedAgentId || null,
    agents,
    sessionVersion,
    () => void agentsQuery.refetch(),
    (message) => addToast(message, "error"),
    urlSessionId,
  );
  // Track LLM text streaming (cleared on `typing:stop`) independently of
  // `isLoading`, which stays true through post-processing until the final
  // `response` event. Textarea unblocks as soon as streaming ends so the user
  // can compose the next message immediately.
  const isStreaming = messages.some(m => m.role === "assistant" && m.isStreaming);

  // Export current conversation as a markdown file. Keeps the local
  // timestamp, role, content, and (when present) tool call summaries
  // so operators can archive or share transcripts.
  const handleExport = useCallback(() => {
    if (messages.length === 0) return;
    const agentName = agents.find(a => a.id === selectedAgentId)?.name ?? selectedAgentId;
    const lines: string[] = [
      `# Conversation with ${agentName}`,
      "",
      `_Exported: ${new Date().toISOString()}_`,
      `_${messages.length} messages_`,
      "",
      "---",
      "",
    ];
    for (const m of messages) {
      const ts = m.timestamp instanceof Date ? m.timestamp.toISOString() : new Date(m.timestamp as string).toISOString();
      const role = m.role === "assistant" ? agentName : m.role;
      lines.push(`### ${role} · ${ts}`);
      lines.push("");
      if (m.content) {
        lines.push(m.content);
        lines.push("");
      }
      if (m.tools && m.tools.length > 0) {
        lines.push(`_Tools: ${m.tools.map(t => t.name).join(", ")}_`);
        lines.push("");
      }
    }
    const blob = new Blob([lines.join("\n")], { type: "text/markdown;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    const date = new Date().toISOString().slice(0, 10);
    a.download = `chat-${agentName.replace(/[^a-zA-Z0-9-_]/g, "_")}-${date}.md`;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }, [messages, agents, selectedAgentId]);
  const { pendingApprovals, removeApproval } = useApprovalPoller(selectedAgentId || null);
  const selectedAgent = agents.find(a => a.id === selectedAgentId);

  // Per-agent session list. `activeSessionId` is derived from the URL first
  // (multi-tab safety, issue #2959); if absent, fall back to the server's
  // canonical active session so initial navigation still highlights correctly.
  const sessionsQuery = useAgentSessions(selectedAgentId);
  const serverActiveSessionId = useMemo(() => {
    const active = sessionsQuery.data?.find((s: SessionListItem) => s.active);
    return active?.session_id;
  }, [sessionsQuery.data]);
  const activeSessionId = urlSessionId ?? serverActiveSessionId;

  // Sidebar clicks update the URL — no switch_agent_session POST. Each tab's
  // URL carries its own sessionId, and the send path forwards it per-request.
  const handleSwitchSession = useCallback(async (sessionId: string) => {
    if (!selectedAgentId) return;
    navigate({
      to: "/chat",
      search: { agentId: selectedAgentId, sessionId },
      replace: false,
    });
    setSessionVersion(v => v + 1);
  }, [selectedAgentId, navigate]);

  const handleNewSession = useCallback(async () => {
    if (!selectedAgentId) return;
    const result = await createSessionMutation.mutateAsync({ agentId: selectedAgentId });
    navigate({
      to: "/chat",
      search: { agentId: selectedAgentId, sessionId: result.session_id },
      replace: false,
    });
    setSessionVersion(v => v + 1);
  }, [selectedAgentId, createSessionMutation, navigate]);

  const handleDeleteSession = useCallback(async (sessionId: string) => {
    await deleteSessionMutation.mutateAsync({ sessionId, agentId: selectedAgentId });
    // If the deleted session is the one pinned in the URL, drop the param
    // so the next render falls back to server-active (if any).
    if (urlSessionId && urlSessionId === sessionId) {
      navigate({
        to: "/chat",
        search: { agentId: selectedAgentId },
        replace: true,
      });
    }
  }, [deleteSessionMutation, selectedAgentId, urlSessionId, navigate]);

  // If the current selection is no longer visible (e.g. hand agents toggled
  // off while a hand-spawned agent was selected), clear it so the auto-select
  // effect below picks a new one instead of leaving the chat pane in a broken
  // state with selectedAgent === undefined.
  useEffect(() => {
    if (!selectedAgentId) return;
    if (agentsQuery.data === undefined) return;
    if (!agents.some(a => a.id === selectedAgentId)) {
      setSelectedAgentId("");
    }
  }, [agents, selectedAgentId, agentsQuery.data]);

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

  const renderAgentButton = (
    agent: AgentItem,
    role?: string,
    isCoordinator?: boolean,
  ) => (
    <button
      key={agent.id}
      onClick={() => selectAgent(agent.id)}
      className={`w-full flex items-center gap-3 p-3 rounded-xl transition-colors text-left group ${
        selectedAgentId === agent.id
          ? "bg-primary text-white shadow-lg shadow-brand/20"
          : "hover:bg-surface-hover"
      }`}
    >
      <div className={`relative h-10 w-10 rounded-xl flex items-center justify-center font-black text-lg ${
        selectedAgentId === agent.id ? "bg-white/20"
        : (agent.state || "").toLowerCase() === "running" ? "bg-linear-to-br from-brand/20 to-accent/20 text-primary"
        : "bg-main text-text-dim/40"
      }`}>
        {t(`agents.builtin.${agent.name}.name`, { defaultValue: agent.name }).charAt(0).toUpperCase()}
        {(agent.state || "").toLowerCase() === "running" ? (
          <span className="absolute -bottom-0.5 -right-0.5 w-2.5 h-2.5 rounded-full bg-success border-2 border-white dark:border-surface animate-pulse" />
        ) : (
          <span className="absolute -bottom-0.5 -right-0.5 w-2.5 h-2.5 rounded-full bg-text-dim/30 border-2 border-white dark:border-surface" />
        )}
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5">
          <p className={`text-sm font-bold truncate ${(agent.state || "").toLowerCase() !== "running" ? "opacity-50" : ""}`}>
            {role ?? t(`agents.builtin.${agent.name}.name`, { defaultValue: agent.name })}
          </p>
          {(agent.auth_status === "configured" || agent.auth_status === "validated_key") && <span className={`shrink-0 px-1 py-0.5 rounded text-[8px] font-bold uppercase leading-none ${selectedAgentId === agent.id ? "bg-white/20" : "bg-primary/10 text-primary"}`}>KEY</span>}
          {agent.auth_status === "configured_cli" && <span className={`shrink-0 px-1 py-0.5 rounded text-[8px] font-bold uppercase leading-none ${selectedAgentId === agent.id ? "bg-white/20" : "bg-accent/10 text-accent"}`}>CLI</span>}
          {agent.auth_status === "auto_detected" && <span className={`shrink-0 px-1 py-0.5 rounded text-[8px] font-bold uppercase leading-none ${selectedAgentId === agent.id ? "bg-white/20" : "bg-warning/10 text-warning"}`}>AUTO</span>}
          {isAuthUnavailable(agent.auth_status) && <AlertCircle className="h-3 w-3 text-warning shrink-0" />}
        </div>
        {isCoordinator ? (
          <p className={`text-[10px] truncate ${selectedAgentId === agent.id ? "text-white/70" : "text-text-dim"}`}>
            {t("chat.hand_coordinator", { defaultValue: "coordinator" })}
          </p>
        ) : (
          <p className={`text-[10px] truncate ${selectedAgentId === agent.id ? "text-white/70" : "text-text-dim"}`}>
            {agent.model_provider || t("common.unknown")}
          </p>
        )}
      </div>
      <ArrowRight className={`h-4 w-4 shrink-0 transition-transform ${selectedAgentId === agent.id ? "rotate-90" : "opacity-0 group-hover:opacity-100"}`} />
    </button>
  );

  return (
    <div className="flex h-[calc(100vh-100px)] sm:h-[calc(100vh-140px)] flex-col">
      {/* Header */}
      <header className="pb-2 sm:pb-4">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2 sm:gap-3">
            <div className="relative hidden sm:block">
              <Sparkles className="h-5 w-5 text-primary" />
              <span className="absolute inset-0 bg-primary/30 animate-pulse" />
            </div>
            <span className="text-primary font-bold uppercase tracking-widest text-[10px] hidden sm:inline">{t("chat.neural_terminal")}</span>
            <h1 className="text-xl sm:text-3xl font-extrabold tracking-tight">{t("chat.title")}</h1>
          </div>
          <button
            onClick={() => void agentsQuery.refetch()}
            className="p-2 sm:p-2.5 rounded-xl hover:bg-surface-hover text-text-dim hover:text-primary transition-colors"
          >
            <RefreshCw className={`h-4 w-4 ${agentsQuery.isFetching ? "animate-spin" : ""}`} />
          </button>
        </div>
      </header>

      {/* Main content area */}
      <div className="flex flex-1 overflow-hidden rounded-2xl border border-border-subtle bg-surface shadow-xl ring-1 ring-black/5 dark:ring-white/5">
        {/* Left sidebar - Agent list */}
        <aside className="hidden md:flex w-64 shrink-0 border-r border-border-subtle bg-main flex-col">
          <div className="p-4 border-b border-border-subtle space-y-2">
            <h3 className="text-[10px] font-black uppercase tracking-[0.2em] text-text-dim/60">{t("nav.agents")}</h3>
            <button
              onClick={() => setShowHandAgents((value) => !value)}
              aria-pressed={showHandAgents}
              className={`inline-flex items-center gap-1.5 rounded-full border px-3 py-1 text-[11px] font-bold transition-colors ${
                showHandAgents
                  ? "border-primary/30 bg-primary/10 text-primary"
                  : "border-border-subtle bg-surface text-text-dim hover:border-primary/20 hover:text-primary"
              }`}
            >
              <span>{t("agents.show_hand_agents", { defaultValue: "Show hand agents" })}</span>
            </button>
          </div>
          <div className="flex-1 overflow-y-auto p-3 space-y-2 scrollbar-thin">
            {picker.standalone.length === 0 && picker.handGroups.length === 0 ? (
              <div className="p-4 text-center text-text-dim text-sm">{t("common.no_data")}</div>
            ) : (
              <>
                {picker.standalone.length > 0 && (
                  <div className="space-y-2">
                    {picker.handGroups.length > 0 && (
                      <h4 className="px-1 pt-1 text-[10px] font-black uppercase tracking-[0.2em] text-text-dim/60">
                        {t("chat.group_standalone", { defaultValue: "Standalone" })}
                      </h4>
                    )}
                    {picker.standalone.map((agent) => renderAgentButton(agent))}
                  </div>
                )}
                {picker.handGroups.map((group) => (
                  <div key={group.hand_id} className="space-y-2 pt-3">
                    <h4 className="px-1 text-[10px] font-black uppercase tracking-[0.2em] text-text-dim/60 flex items-center gap-1.5">
                      {group.hand_icon && <span aria-hidden="true">{group.hand_icon}</span>}
                      <span>{group.hand_name}</span>
                    </h4>
                    {group.agents.map((agent) =>
                      renderAgentButton(agent, agent.role, agent.isCoordinator),
                    )}
                  </div>
                ))}
              </>
            )}
          </div>
        </aside>

        {/* Right side - Chat area */}
        <main className="flex-1 flex flex-col overflow-hidden bg-main/10 relative">
          {/* Background decoration */}
          <div className="absolute inset-0 pointer-events-none opacity-30">
            <div className="absolute top-0 left-0 w-64 h-64 bg-primary/5 rounded-full blur-3xl" />
            <div className="absolute bottom-0 right-0 w-48 h-48 bg-accent/5 rounded-full blur-3xl" />
          </div>

          {/* Mobile agent selector */}
          <div className="md:hidden px-3 py-2 border-b border-border-subtle bg-surface/80">
            <select
              value={selectedAgentId}
              onChange={(e) => selectAgent(e.target.value)}
              className="w-full rounded-lg border border-border-subtle bg-main px-3 py-2 text-sm font-bold outline-none focus:border-primary"
            >
              <option value="">{t("chat.select_agent")}</option>
              {picker.standalone.map((agent) => (
                <option key={agent.id} value={agent.id}>
                  {t(`agents.builtin.${agent.name}.name`, { defaultValue: agent.name })} ({agent.state || "unknown"})
                </option>
              ))}
              {picker.handGroups.map((group) => (
                <optgroup
                  key={group.hand_id}
                  label={`${group.hand_icon ?? ""} ${group.hand_name}`.trim()}
                >
                  {group.agents.map((agent) => (
                    <option key={agent.id} value={agent.id}>
                      {agent.role}
                      {agent.isCoordinator
                        ? ` (${t("chat.hand_coordinator", { defaultValue: "coordinator" })})`
                        : ""}
                    </option>
                  ))}
                </optgroup>
              ))}
            </select>
          </div>

          {selectedAgentId && (
            <ConnectionBar
              agentName={selectedAgent?.name || ""}
              isLoading={isLoading}
              messageCount={messages.length}
              onClear={() => { void clearHistory(); }}
              onExport={handleExport}
              wsConnected={wsConnected}
              modelName={selectedAgent?.model_name}
              modelProvider={selectedAgent?.model_provider}
              sessions={sessionsQuery.data}
              activeSessionId={activeSessionId}
              onSwitchSession={handleSwitchSession}
              onNewSession={handleNewSession}
              onDeleteSession={handleDeleteSession}
              agentId={selectedAgentId}
              onModelChange={() => void agentsQuery.refetch()}
              onOpenConfig={() => navigate({ to: "/config" })}
              webSearchAugmentation={selectedAgent?.web_search_augmentation}
              webSearchAvailable={webSearchAvailable}
              onWebSearchChange={async (mode) => {
                try {
                  await patchAgentConfigMutation.mutateAsync({
                    agentId: selectedAgentId,
                    config: { web_search_augmentation: mode },
                  });
                  await agentsQuery.refetch();
                } catch { /* Config update failure — non-critical */ }
              }}
            />
          )}

          {/* Message area */}
          <div className="flex-1 overflow-y-auto p-3 sm:p-6 scrollbar-thin">
            <div className="w-full space-y-4 sm:space-y-6">
            {!selectedAgentId ? (
              <div className="h-full flex flex-col items-center justify-center text-center relative">
                <div className="absolute inset-0 bg-linear-to-b from-transparent via-transparent to-main/50" />
                <div className="relative">
                  <div className="w-24 h-24 rounded-3xl bg-linear-to-br from-brand/20 to-accent/20 flex items-center justify-center mb-6 ring-4 ring-primary/10">
                    <MessageCircle className="h-12 w-12 text-primary" />
                  </div>
                  <div className="absolute inset-0 rounded-3xl bg-primary/10 animate-pulse" />
                </div>
                <h3 className="text-2xl font-black mb-2">{t("chat.select_agent")}</h3>
                <p className="text-sm text-text-dim max-w-xs">{t("chat.select_agent_desc")}</p>
              </div>
            ) : messages.length === 0 ? (
              <div className="h-full flex flex-col items-center justify-center text-center">
                <div className="w-20 h-20 rounded-2xl bg-linear-to-br from-brand/10 to-accent/10 flex items-center justify-center mb-4 ring-2 ring-primary/10">
                  <Bot className="h-10 w-10 text-primary" />
                </div>
                <h3 className="text-xl font-black">{selectedAgent?.name}</h3>
                <p className="text-sm text-text-dim mt-2">{t("chat.welcome_system")}</p>
              </div>
            ) : (
              <div className="space-y-6">
                {messages.map(msg => (
                  <MessageBubble
                    key={msg.id}
                    message={msg}
                    usageFooter={usageFooter}
                    onCopy={handleCopy}
                    copied={copiedMessageId === msg.id}
                    onSpeak={ttsAvailable ? tts.toggle : undefined}
                    isSpeaking={tts.speakingMessageId === msg.id}
                    ttsStatus={tts.speakingMessageId === msg.id ? tts.status : "idle"}
                    ttsAvailable={ttsAvailable}
                  />
                ))}
                {/* Inline approval cards for pending requests */}
                {pendingApprovals.map(approval => (
                  <ApprovalCard key={approval.id} approval={approval} onResolved={removeApproval} />
                ))}
                <div ref={messagesEndRef} />
              </div>
            )}
            </div>
          </div>

          {/* Input area */}
          <div className={`p-2 sm:p-4 border-t border-border-subtle bg-surface transition-opacity ${!selectedAgentId ? "opacity-30 pointer-events-none" : ""}`}>
            <ChatInput
              onSend={sendMessage}
              onStop={stopMessage}
              isStreaming={isStreaming}
              disabled={isLoading}
              inputDisabled={isStreaming}
              placeholder={isStreaming ? t("chat.generating") : selectedAgentId ? t("chat.input_placeholder_with_agent", { name: selectedAgent?.name }) : t("chat.transmit_command")}
              authMissing={isAuthUnavailable(selectedAgent?.auth_status)}
              authStatus={selectedAgent?.auth_status}
              providerName={selectedAgent?.model_provider}
              supportsThinking={selectedAgent?.supports_thinking}
              sttAvailable={sttAvailable}
            />
          </div>
        </main>
      </div>
    </div>
  );
}

