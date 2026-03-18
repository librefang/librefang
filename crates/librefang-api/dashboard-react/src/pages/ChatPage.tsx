import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { FormEvent, useEffect, useMemo, useRef, useState } from "react";
import {
  getSessionDetails,
  listAgents,
  listSessions,
  sendAgentMessage,
  type AgentItem,
  type AgentMessageResponse,
  type SessionListItem
} from "../api";
import { asText, formatMeta, normalizeRole } from "../lib/chat";

const REFRESH_MS = 30000;

interface ChatMessage {
  role: "user" | "assistant" | "system";
  content: string;
}

function dateText(value?: string): string {
  if (!value) return "-";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleTimeString();
}

export function ChatPage() {
  const queryClient = useQueryClient();
  const messagesEndRef = useRef<HTMLDivElement>(null);

  const [selectedAgentId, setSelectedAgentId] = useState<string>("");
  const [inputText, setInputText] = useState("");
  const [sending, setSending] = useState(false);
  const [messages, setMessages] = useState<ChatMessage[]>([]);

  const agentsQuery = useQuery({
    queryKey: ["agents", "list", "chat-helper"],
    queryFn: listAgents,
    refetchInterval: REFRESH_MS
  });

  const sessionsQuery = useQuery({
    queryKey: ["sessions", "list", "chat-helper"],
    queryFn: listSessions,
    refetchInterval: REFRESH_MS
  });

  const currentSessionQuery = useQuery({
    queryKey: ["sessions", "current", selectedAgentId],
    queryFn: async () => {
      if (!selectedAgentId) return null;
      const sessions = await listSessions();
      return sessions.find((s) => s.agent_id === selectedAgentId) ?? null;
    },
    enabled: Boolean(selectedAgentId)
  });

  const sessionDetailQuery = useQuery({
    queryKey: ["sessions", "detail", currentSessionQuery.data?.session_id],
    queryFn: () => getSessionDetails(currentSessionQuery.data?.session_id ?? ""),
    enabled: Boolean(currentSessionQuery.data?.session_id)
  });

  const sendMutation = useMutation({
    mutationFn: async ({ agentId, message }: { agentId: string; message: string }) =>
      sendAgentMessage(agentId, message)
  });

  const agents = agentsQuery.data ?? [];
  const sessions = sessionsQuery.data ?? [];

  // Auto-select first agent
  useEffect(() => {
    if (!selectedAgentId && agents.length > 0) {
      setSelectedAgentId(agents[0].id);
    }
  }, [agents, selectedAgentId]);

  // Load session messages when session changes
  useEffect(() => {
    const session = currentSessionQuery.data;
    if (session?.messages && Array.isArray(session.messages)) {
      const loaded: ChatMessage[] = session.messages.map((m) => ({
        role: normalizeRole(m.role),
        content: asText(m.content)
      }));
      setMessages(loaded);
    } else {
      setMessages([]);
    }
  }, [sessionDetailQuery.data]);

  // Scroll to bottom on new messages
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages]);

  const selectedAgent = useMemo(
    () => agents.find((a) => a.id === selectedAgentId),
    [agents, selectedAgentId]
  );

  async function handleSend(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const text = inputText.trim();
    if (!text || !selectedAgentId || sending) return;

    setSending(true);
    setInputText("");

    // Add user message immediately
    setMessages((prev) => [...prev, { role: "user", content: text }]);

    try {
      const result = await sendMutation.mutateAsync({
        agentId: selectedAgentId,
        message: text
      });

      // Add assistant response
      if (result.response) {
        setMessages((prev) => [...prev, { role: "assistant", content: result.response ?? "" }]);
      }

      // Refresh session data
      await queryClient.invalidateQueries({ queryKey: ["sessions"] });
    } catch (error) {
      setMessages((prev) => [
        ...prev,
        { role: "assistant", content: `Error: ${error instanceof Error ? error.message : "Unknown error"}` }
      ]);
    } finally {
      setSending(false);
    }
  }

  return (
    <section className="flex h-[calc(100vh-140px)] flex-col">
      <header className="flex flex-col justify-between gap-3 pb-4 sm:flex-row sm:items-start">
        <div>
          <h1 className="m-0 text-2xl font-semibold">Chat</h1>
          <p className="text-sm text-slate-400">Talk to your agents in real-time.</p>
        </div>
        <div className="flex items-center gap-2">
          <select
            value={selectedAgentId}
            onChange={(e) => setSelectedAgentId(e.target.value)}
            className="rounded-lg border border-slate-700 bg-slate-800 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
          >
            <option value="">Select agent</option>
            {agents.map((agent) => (
              <option key={agent.id} value={agent.id}>
                {agent.name}
              </option>
            ))}
          </select>
          <button
            className="rounded-lg border border-slate-600 bg-slate-800 px-3 py-2 text-sm font-medium text-slate-100 transition hover:border-sky-500 hover:bg-slate-700 disabled:cursor-not-allowed disabled:opacity-60"
            onClick={() => void queryClient.invalidateQueries({ queryKey: ["sessions"] })}
            disabled={sessionsQuery.isFetching}
          >
            Refresh
          </button>
        </div>
      </header>

      <div className="flex flex-1 flex-col overflow-hidden rounded-xl border border-slate-800 bg-slate-900/70">
        {/* Messages */}
        <div className="flex-1 overflow-y-auto p-4">
          {messages.length === 0 ? (
            <div className="flex h-full items-center justify-center">
              <p className="text-sm text-slate-400">
                {selectedAgentId ? "Send a message to start the conversation." : "Select an agent to start chatting."}
              </p>
            </div>
          ) : (
            <div className="flex flex-col gap-3">
              {messages.map((msg, index) => (
                <div
                  key={index}
                  className={`flex ${
                    msg.role === "user" ? "justify-end" : "justify-start"
                  }`}
                >
                  <div
                    className={`max-w-[80%] rounded-lg p-3 ${
                      msg.role === "user"
                        ? "border border-emerald-700 bg-emerald-700/15 text-emerald-100"
                        : msg.role === "system"
                          ? "border border-amber-700 bg-amber-700/15 text-amber-100"
                          : "border border-sky-700 bg-sky-700/15 text-sky-100"
                    }`}
                  >
                    <div className="whitespace-pre-wrap text-sm">{msg.content}</div>
                    {msg.role === "assistant" && index === messages.length - 1 && sending && (
                      <span className="mt-2 block text-xs text-slate-400">Thinking...</span>
                    )}
                  </div>
                </div>
              ))}
              <div ref={messagesEndRef} />
            </div>
          )}
        </div>

        {/* Input */}
        <form onSubmit={handleSend} className="border-t border-slate-800 p-4">
          <div className="flex gap-2">
            <input
              type="text"
              value={inputText}
              onChange={(e) => setInputText(e.target.value)}
              placeholder={selectedAgentId ? "Type a message..." : "Select an agent first"}
              className="flex-1 rounded-lg border border-slate-700 bg-slate-950/80 px-4 py-3 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
              disabled={!selectedAgentId || sending}
            />
            <button
              type="submit"
              className="rounded-lg border border-sky-500 bg-sky-600 px-4 py-2 text-sm font-medium text-white transition hover:bg-sky-500 disabled:cursor-not-allowed disabled:opacity-60"
              disabled={!selectedAgentId || !inputText.trim() || sending}
            >
              {sending ? "..." : "Send"}
            </button>
          </div>
        </form>
      </div>
    </section>
  );
}
