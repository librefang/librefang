import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { useSearch } from "@tanstack/react-router";
import { listAgents, sendAgentMessage, loadAgentSession } from "../api";
import { Button } from "../components/ui/Button";
import { Card } from "../components/ui/Card";
import { MessageCircle, Send, Bot, User, RefreshCw, AlertCircle, Wifi, Sparkles, X, ArrowRight } from "lucide-react";

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

// Slash 命令
const SLASH_COMMANDS = [
  { cmd: "/help", desc: "Show available commands" },
  { cmd: "/clear", desc: "Clear chat history" },
  { cmd: "/agents", desc: "List available agents" },
  { cmd: "/info", desc: "Show current agent info" },
];

// 流式打字机效果
function Typewriter({ text, speed = 15 }: { text: string; speed?: number }) {
  const [displayed, setDisplayed] = useState("");

  useEffect(() => {
    if (!text) { setDisplayed(""); return; }
    if (text.length <= displayed.length) { setDisplayed(text); return; }

    const interval = setInterval(() => {
      setDisplayed(prev => {
        if (prev.length >= text.length) {
          clearInterval(interval);
          return text;
        }
        return text.slice(0, prev.length + 2);
      });
    }, speed);

    return () => clearInterval(interval);
  }, [text, speed]);

  return <span>{displayed}</span>;
}

// 聊天消息管理 - 包含历史加载和发送
function useChatMessages(agentId: string | null, agents: any[] = []) {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [isLoading, setIsLoading] = useState(false);

  // 加载历史记录
  useEffect(() => {
    if (!agentId) return;
    loadAgentSession(agentId)
      .then(session => {
        if (session.messages?.length) {
          const historical: ChatMessage[] = session.messages.map((msg, idx) => ({
            id: `hist-${idx}`,
            role: msg.role === "user" ? "user" : "assistant",
            content: typeof msg.content === "string" ? msg.content : JSON.stringify(msg.content),
            timestamp: new Date(),
          }));
          setMessages(historical);
        }
      })
      .catch(() => {});
  }, [agentId]);

  // Slash 命令提示
  const [showSlash, setShowSlash] = useState(false);
  const [slashFilter, setSlashFilter] = useState("");

  // 发送消息并实现流式输出
  const sendMessage = useCallback(async (content: string) => {
    if (!content.trim()) return;
    const trimmed = content.trim();

    // Slash 命令处理
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

    try {
      const response = await sendAgentMessage(agentId, trimmed);
      const fullContent = response.response || "";
      let currentLength = 0;

      const streamInterval = setInterval(() => {
        if (currentLength < fullContent.length) {
          currentLength += Math.min(3, fullContent.length - currentLength);
          setMessages(prev => prev.map(m =>
            m.id === botMsg.id ? { ...m, content: fullContent.slice(0, currentLength) } : m
          ));
        } else {
          clearInterval(streamInterval);
          setMessages(prev => prev.map(m =>
            m.id === botMsg.id
              ? {
                  ...m, isStreaming: false,
                  tokens: { output: response.output_tokens, input: response.input_tokens },
                  cost_usd: response.cost_usd,
                  memories_saved: response.memories_saved,
                  memories_used: response.memories_used,
                }
              : m
          ));
          setIsLoading(false);
        }
      }, 20);
    } catch (err) {
      const errorMsg = err instanceof Error ? err.message : "Unknown error";
      setMessages(prev => prev.map(m =>
        m.id === botMsg.id ? { ...m, isStreaming: false, error: errorMsg } : m
      ));
      setIsLoading(false);
    }
  }, [agentId, agents]);

  const clearHistory = useCallback(() => setMessages([]), []);

  return { messages, isLoading, sendMessage, clearHistory };
}

// 消息气泡组件
function MessageBubble({ message }: { message: ChatMessage }) {
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
    <div className={`flex ${isUser ? "justify-start" : "justify-end"} animate-fade-in-up`}>
      <div className={`flex flex-col max-w-[75%] ${isUser ? "items-start" : "items-end"}`}>
        {/* 头像 + 名字 */}
        <div className={`flex items-center gap-2 mb-1.5 ${isUser ? "self-start" : "self-end"}`}>
          <div className={`h-7 w-7 rounded-lg flex items-center justify-center ${
            isUser ? "bg-gradient-to-br from-brand to-accent text-white shadow-md" : "bg-surface border border-border-subtle"
          }`}>
            {isUser ? <User className="h-3.5 w-3.5" /> : <Bot className="h-3.5 w-3.5 text-brand" />}
          </div>
          <span className={`text-[11px] font-bold uppercase tracking-wider ${isUser ? "text-brand" : "text-text-dim"}`}>
            {isUser ? "You" : "Bot"}
          </span>
        </div>

        {/* 消息内容 */}
        <div className={`relative px-4 py-3 rounded-2xl text-sm leading-relaxed shadow-sm transition-all ${
          isUser
            ? "bg-gradient-to-br from-brand to-brand/90 text-white rounded-tl-md"
            : message.error
              ? "bg-error/10 border border-error/20 text-error rounded-tr-md"
              : "bg-surface border border-border-subtle rounded-tr-md"
        }`}>
          {message.isStreaming ? (
            <div className="flex items-center gap-1">
              <span className="w-1.5 h-1.5 bg-brand/60 rounded-full animate-bounce" style={{ animationDelay: "0ms" }} />
              <span className="w-1.5 h-1.5 bg-brand/60 rounded-full animate-bounce" style={{ animationDelay: "150ms" }} />
              <span className="w-1.5 h-1.5 bg-brand/60 rounded-full animate-bounce" style={{ animationDelay: "300ms" }} />
            </div>
          ) : message.error ? (
            <div className="flex items-start gap-2">
              <AlertCircle className="h-4 w-4 shrink-0 mt-0.5" />
              <span>{message.error}</span>
            </div>
          ) : (
            <Typewriter text={message.content} speed={10} />
          )}
        </div>

        {/* 元信息 */}
        <div className="flex items-center gap-2 mt-1.5 text-[10px] text-text-dim/50">
          <span>{message.timestamp.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}</span>
          {message.tokens?.output && !message.isStreaming && (
            <span className="px-1.5 py-0.5 rounded bg-brand/10 text-brand/70 font-mono text-[9px]">
              {message.tokens.output} tok
            </span>
          )}
          {message.cost_usd !== undefined && message.cost_usd > 0 && (
            <span className="px-1.5 py-0.5 rounded bg-success/10 text-success/70 font-mono text-[9px]">
              ${message.cost_usd.toFixed(4)}
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
}

// 输入框 - 带快捷键提示
function ChatInput({ onSend, disabled, placeholder }: { onSend: (msg: string) => void; disabled: boolean; placeholder: string }) {
  const [message, setMessage] = useState("");
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (message.trim() && !disabled) {
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

  return (
    <form onSubmit={handleSubmit} className="space-y-2">
      {/* Slash 命令补全 */}
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
      <div className="flex gap-3 items-end">
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
            disabled={disabled}
            rows={1}
            className="w-full min-h-[52px] max-h-[150px] rounded-xl border border-border-subtle bg-surface px-4 py-3 text-sm focus:border-brand focus:ring-2 focus:ring-brand/10 outline-none resize-none placeholder:text-text-dim/50"
          />
        </div>
        <button
          type="submit"
          disabled={!message.trim() || disabled}
          className="group relative px-5 py-3 rounded-xl bg-gradient-to-r from-brand to-brand/90 text-white font-bold text-sm shadow-lg shadow-brand/20 hover:shadow-brand/40 hover:-translate-y-0.5 transition-all disabled:opacity-40 disabled:cursor-not-allowed"
        >
          <Send className="h-4 w-4" />
          <span className="absolute -top-8 right-0 bg-surface border border-border-subtle rounded-lg px-2 py-1 text-[10px] text-text-dim opacity-0 group-hover:opacity-100 transition-opacity whitespace-nowrap">
            Enter 发送 · ⌘+↵ 换行
          </span>
        </button>
      </div>
    </form>
  );
}

// 连接状态栏
function ConnectionBar({ agentName, isLoading, messageCount, onClear }: { agentName: string; isLoading: boolean; messageCount: number; onClear: () => void }) {
  const { t } = useTranslation();
  return (
    <div className="px-4 py-2.5 border-b border-border-subtle/50 bg-gradient-to-r from-surface/80 to-transparent flex items-center justify-between backdrop-blur-sm">
      <div className="flex items-center gap-3">
        <div className="relative">
          <Wifi className="h-3.5 w-3.5 text-success" />
          <span className="absolute inset-0 rounded-full bg-success/30 animate-ping" />
        </div>
        <span className="text-xs font-semibold text-success uppercase tracking-wide">{t("chat.secure_link")}</span>
        <span className="text-text-dim/30">•</span>
        <span className="text-xs font-medium text-text-dim">{agentName}</span>
        {isLoading && (
          <span className="ml-2 px-2 py-0.5 rounded-full bg-brand/10 text-brand text-[10px] font-medium animate-pulse">
            生成中...
          </span>
        )}
      </div>
      {messageCount > 0 && (
        <button onClick={onClear} className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium text-text-dim/60 hover:text-error hover:bg-error/5 transition-colors">
          <X className="h-3 w-3" />
          清空对话
        </button>
      )}
    </div>
  );
}

export function ChatPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const search = useSearch({ from: "/chat" });
  const initialAgentId = search?.agentId || "";
  const [selectedAgentId, setSelectedAgentId] = useState(initialAgentId);
  const messagesEndRef = useRef<HTMLDivElement>(null);

  const agentsQuery = useQuery({ queryKey: ["agents", "list", "chat"], queryFn: listAgents, staleTime: 30000 });
  const agents = agentsQuery.data ?? [];
  const { messages, isLoading, sendMessage, clearHistory } = useChatMessages(selectedAgentId || null, agents);
  const selectedAgent = agents.find(a => a.id === selectedAgentId);

  useEffect(() => {
    if (!selectedAgentId && agents.length > 0) setSelectedAgentId(agents[0].id);
  }, [agents, selectedAgentId]);

  // 滚动到最新消息
  useEffect(() => {
    if (messages.length > 0) {
      setTimeout(() => {
        messagesEndRef.current?.scrollIntoView({ behavior: "smooth", block: "end" });
      }, 100);
    }
  }, [messages]);

  return (
    <div className="flex h-[calc(100vh-140px)] flex-col">
      {/* 头部 */}
      <header className="pb-4">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-3">
            <div className="relative">
              <Sparkles className="h-5 w-5 text-brand" />
              <span className="absolute inset-0 bg-brand/30 animate-ping" />
            </div>
            <span className="text-brand font-bold uppercase tracking-widest text-[10px]">{t("chat.neural_terminal")}</span>
          </div>
          <button
            onClick={() => queryClient.invalidateQueries({ queryKey: ["agents", "list"] })}
            className="p-2.5 rounded-xl hover:bg-surface-hover text-text-dim hover:text-brand transition-all"
          >
            <RefreshCw className={`h-4 w-4 ${agentsQuery.isFetching ? "animate-spin" : ""}`} />
          </button>
        </div>
        <h1 className="mt-2 text-3xl font-extrabold tracking-tight">{t("chat.title")}</h1>
      </header>

      {/* 主内容区 */}
      <div className="flex flex-1 overflow-hidden rounded-2xl border border-border-subtle bg-surface shadow-xl ring-1 ring-black/5 dark:ring-white/5">
        {/* 左侧 Agent 列表 */}
        <aside className="w-64 flex-shrink-0 border-r border-border-subtle bg-main/30 backdrop-blur-md flex flex-col">
          <div className="p-4 border-b border-border-subtle">
            <h3 className="text-[10px] font-black uppercase tracking-[0.2em] text-text-dim/60">{t("nav.agents")}</h3>
          </div>
          <div className="flex-1 overflow-y-auto p-3 space-y-2">
            {agents.length === 0 ? (
              <div className="p-4 text-center text-text-dim text-sm">{t("common.no_data")}</div>
            ) : (
              agents.map(agent => (
                <button
                  key={agent.id}
                  onClick={() => setSelectedAgentId(agent.id)}
                  className={`w-full flex items-center gap-3 p-3 rounded-xl transition-all text-left group ${
                    selectedAgentId === agent.id
                      ? "bg-brand text-white shadow-lg shadow-brand/20"
                      : "hover:bg-surface-hover"
                  }`}
                >
                  <div className={`relative h-10 w-10 rounded-xl flex items-center justify-center font-black text-lg ${
                    selectedAgentId === agent.id ? "bg-white/20" : "bg-gradient-to-br from-brand/20 to-accent/20 text-brand group-hover:from-brand group-hover:to-brand"
                  }`}>
                    {agent.name.charAt(0).toUpperCase()}
                    {agent.state === "running" && (
                      <span className="absolute -bottom-0.5 -right-0.5 w-2.5 h-2.5 rounded-full bg-success border-2 border-white dark:border-surface animate-pulse" />
                    )}
                  </div>
                  <div className="min-w-0 flex-1">
                    <p className="text-sm font-bold truncate">{agent.name}</p>
                    <p className={`text-[10px] truncate ${selectedAgentId === agent.id ? "text-white/70" : "text-text-dim"}`}>
                      {agent.model_name || t("common.unknown")}
                    </p>
                  </div>
                  <ArrowRight className={`h-4 w-4 transition-transform ${selectedAgentId === agent.id ? "rotate-90" : "opacity-0 group-hover:opacity-100"}`} />
                </button>
              ))
            )}
          </div>
        </aside>

        {/* 右侧聊天区域 */}
        <main className="flex-1 flex flex-col overflow-hidden bg-main/10 relative">
          {/* 背景装饰 */}
          <div className="absolute inset-0 pointer-events-none opacity-30">
            <div className="absolute top-0 left-0 w-64 h-64 bg-brand/5 rounded-full blur-3xl" />
            <div className="absolute bottom-0 right-0 w-48 h-48 bg-accent/5 rounded-full blur-3xl" />
          </div>

          {selectedAgentId && (
            <ConnectionBar
              agentName={selectedAgent?.name || ""}
              isLoading={isLoading}
              messageCount={messages.length}
              onClear={clearHistory}
            />
          )}

          {/* 消息区域 */}
          <div className="flex-1 overflow-y-auto p-6 space-y-6 scrollbar-thin">
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
                <div ref={messagesEndRef} />
              </div>
            )}
          </div>

          {/* 输入区域 */}
          <div className={`p-4 border-t border-border-subtle bg-surface/90 backdrop-blur-md transition-all ${!selectedAgentId ? "opacity-30 pointer-events-none" : ""}`}>
            <ChatInput
              onSend={sendMessage}
              disabled={isLoading}
              placeholder={selectedAgentId ? t("chat.input_placeholder_with_agent", { name: selectedAgent?.name }) : t("chat.transmit_command")}
            />
          </div>
        </main>
      </div>
    </div>
  );
}