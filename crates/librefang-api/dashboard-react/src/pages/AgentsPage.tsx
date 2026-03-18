import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { FormEvent, useEffect, useRef, useState } from "react";
import type { AgentItem, AgentSessionImage, AgentSessionMessage, AgentTool } from "../api";
import { listAgents, loadAgentSession, sendAgentMessage } from "../api";
import { asText, formatMeta, normalizeRole } from "../lib/chat";

const AGENT_REFRESH_MS = 15000;

interface ChatTool {
  name: string;
  input: string;
  result: string;
  isError: boolean;
}

interface ChatImage {
  fileId: string;
  filename: string;
}

interface ChatMessage {
  id: string;
  role: "user" | "assistant" | "system";
  content: string;
  tools: ChatTool[];
  images: ChatImage[];
  meta?: string;
  pending?: boolean;
}

function toolToChatTool(tool: AgentTool): ChatTool {
  return {
    name: tool.name ?? "tool",
    input: asText(tool.input),
    result: tool.result ?? "",
    isError: Boolean(tool.is_error)
  };
}

function imageToChatImage(image: AgentSessionImage): ChatImage {
  return {
    fileId: image.file_id,
    filename: image.filename ?? "image"
  };
}

function fromSessionMessage(message: AgentSessionMessage, index: number): ChatMessage {
  return {
    id: `session-${index}`,
    role: normalizeRole(message.role),
    content: asText(message.content),
    tools: (message.tools ?? []).map(toolToChatTool),
    images: (message.images ?? []).map(imageToChatImage)
  };
}

function formatAgentModel(agent: AgentItem): string {
  const provider = agent.model_provider ?? "provider?";
  const model = agent.model_name ?? "model?";
  return `${provider} · ${model}`;
}

function formatAgentState(agent: AgentItem): string {
  if (agent.ready) return "Ready";
  if (agent.state) return agent.state;
  return "Unknown";
}

export function AgentsPage() {
  const queryClient = useQueryClient();
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [inputText, setInputText] = useState("");
  const messagesRef = useRef<HTMLDivElement | null>(null);
  const selectedAgentIdRef = useRef<string | null>(null);

  const agentsQuery = useQuery({
    queryKey: ["agents", "list"],
    queryFn: listAgents,
    refetchInterval: AGENT_REFRESH_MS
  });

  const sessionQuery = useQuery({
    queryKey: ["agents", "session", selectedAgentId],
    queryFn: () => loadAgentSession(selectedAgentId ?? ""),
    enabled: Boolean(selectedAgentId)
  });

  const sendMutation = useMutation({
    mutationFn: ({ agentId, message }: { agentId: string; message: string }) =>
      sendAgentMessage(agentId, message)
  });

  useEffect(() => {
    selectedAgentIdRef.current = selectedAgentId;
  }, [selectedAgentId]);

  useEffect(() => {
    const nextAgents = agentsQuery.data ?? [];
    setSelectedAgentId((current) => {
      if (current && nextAgents.some((agent) => agent.id === current)) {
        return current;
      }
      return nextAgents.length > 0 ? nextAgents[0].id : null;
    });
  }, [agentsQuery.data]);

  useEffect(() => {
    if (!selectedAgentId) {
      setMessages([]);
      return;
    }
    const nextMessages = (sessionQuery.data?.messages ?? []).map(fromSessionMessage);
    setMessages(nextMessages);
  }, [selectedAgentId, sessionQuery.data?.messages]);

  useEffect(() => {
    const container = messagesRef.current;
    if (!container) return;
    container.scrollTop = container.scrollHeight;
  }, [messages, selectedAgentId, sessionQuery.isLoading]);

  async function handleSend(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (sendMutation.isPending || !selectedAgentId) return;

    const text = inputText.trim();
    if (!text) return;

    const agentId = selectedAgentId;
    setInputText("");

    const now = Date.now();
    setMessages((current) => [
      ...current,
      {
        id: `user-${now}`,
        role: "user",
        content: text,
        tools: [],
        images: []
      },
      {
        id: `pending-${now}`,
        role: "assistant",
        content: "Thinking...",
        tools: [],
        images: [],
        pending: true
      }
    ]);

    try {
      const response = await sendMutation.mutateAsync({ agentId, message: text });
      if (selectedAgentIdRef.current !== agentId) return;

      if (response.silent) {
        setMessages((current) => current.filter((message) => !message.pending));
      } else {
        setMessages((current) => [
          ...current.filter((message) => !message.pending),
          {
            id: `assistant-${Date.now()}`,
            role: "assistant",
            content:
              typeof response.response === "string" && response.response.trim().length > 0
                ? response.response
                : "[empty response]",
            tools: [],
            images: [],
            meta: formatMeta(response)
          }
        ]);
      }

      const refreshed = await queryClient.fetchQuery({
        queryKey: ["agents", "session", agentId],
        queryFn: () => loadAgentSession(agentId)
      });
      if (selectedAgentIdRef.current !== agentId) return;
      setMessages((refreshed.messages ?? []).map(fromSessionMessage));
    } catch (error) {
      if (selectedAgentIdRef.current !== agentId) return;
      setMessages((current) => [
        ...current.filter((message) => !message.pending),
        {
          id: `error-${Date.now()}`,
          role: "system",
          content: error instanceof Error ? `Error: ${error.message}` : "Failed to send message.",
          tools: [],
          images: []
        }
      ]);
    }
  }

  const agents = agentsQuery.data ?? [];
  const selectedAgent = selectedAgentId
    ? agents.find((agent) => agent.id === selectedAgentId) ?? null
    : null;
  const agentsError = agentsQuery.error instanceof Error ? agentsQuery.error.message : "";
  const sessionError = sessionQuery.error instanceof Error ? sessionQuery.error.message : "";

  return (
    <section className="flex flex-col gap-4">
      <header>
        <h1 className="m-0 text-2xl font-semibold">Agents</h1>
        <p className="text-sm text-slate-400">TanStack Query powered chat workspace.</p>
      </header>

      {agentsError ? (
        <div className="rounded-xl border border-rose-700 bg-rose-700/15 p-4 text-rose-200">{agentsError}</div>
      ) : null}

      <div className="grid gap-3 xl:grid-cols-[320px_1fr]">
        <aside className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          <div className="mb-3 flex items-center justify-between gap-2">
            <h2 className="m-0 text-base font-semibold">Available Agents</h2>
            <button
              className="rounded-lg border border-sky-500 bg-sky-600 px-3 py-2 text-xs font-medium text-white transition hover:bg-sky-500 disabled:cursor-not-allowed disabled:opacity-60"
              type="button"
              onClick={() => void agentsQuery.refetch()}
              disabled={agentsQuery.isFetching}
            >
              Refresh
            </button>
          </div>

          {agentsQuery.isLoading && agents.length === 0 ? (
            <p className="text-sm text-slate-400">Loading agents...</p>
          ) : null}
          {!agentsQuery.isLoading && agents.length === 0 ? (
            <p className="text-sm text-slate-400">No agents found. Create one from CLI/API first.</p>
          ) : null}

          <div className="flex max-h-[65vh] flex-col gap-2 overflow-y-auto pr-1">
            {agents.map((agent) => (
              <button
                key={agent.id}
                className={`flex w-full flex-col items-start gap-1 rounded-lg border p-3 text-left transition ${
                  agent.id === selectedAgentId
                    ? "border-sky-500 bg-sky-500/15"
                    : "border-slate-700 bg-slate-900/60 hover:border-slate-500 hover:bg-slate-800/60"
                }`}
                type="button"
                onClick={() => setSelectedAgentId(agent.id)}
                disabled={sendMutation.isPending}
              >
                <span className="font-medium">{agent.name}</span>
                <span className="text-xs text-slate-400">{formatAgentModel(agent)}</span>
                <span
                  className={`rounded-full border px-2 py-1 text-[11px] ${
                    agent.ready
                      ? "border-emerald-700 bg-emerald-700/20 text-emerald-300"
                      : "border-amber-700 bg-amber-700/20 text-amber-300"
                  }`}
                >
                  {formatAgentState(agent)}
                </span>
              </button>
            ))}
          </div>
        </aside>

        <section className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          {selectedAgent ? (
            <div className="flex h-full min-h-[70vh] flex-col gap-3">
              <div className="flex items-start justify-between gap-3 border-b border-slate-800 pb-3">
                <div>
                  <h2 className="m-0 text-lg font-semibold">{selectedAgent.name}</h2>
                  <p className="text-sm text-slate-400">
                    {formatAgentModel(selectedAgent)} · mode: {selectedAgent.mode ?? "unknown"}
                  </p>
                </div>
                <span
                  className={`rounded-full border px-2 py-1 text-xs ${
                    selectedAgent.ready
                      ? "border-emerald-700 bg-emerald-700/20 text-emerald-300"
                      : "border-amber-700 bg-amber-700/20 text-amber-300"
                  }`}
                >
                  {selectedAgent.ready ? "Ready" : "Needs Attention"}
                </span>
              </div>

              {sessionError ? (
                <div className="rounded-lg border border-rose-700 bg-rose-700/15 p-3 text-sm text-rose-200">
                  {sessionError}
                </div>
              ) : null}

              <div
                ref={messagesRef}
                className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto rounded-lg border border-slate-800 bg-slate-950/70 p-3"
              >
                {sessionQuery.isLoading ? <div className="text-sm text-slate-400">Loading session...</div> : null}
                {!sessionQuery.isLoading && messages.length === 0 ? (
                  <div className="text-sm text-slate-400">No messages yet. Start the conversation below.</div>
                ) : null}

                {messages.map((message) => (
                  <article
                    key={message.id}
                    className={`max-w-[90%] rounded-lg border p-3 ${
                      message.role === "user"
                        ? "self-end border-sky-500/70 bg-sky-500/20"
                        : message.role === "system"
                          ? "self-center border-amber-700 bg-amber-700/20"
                          : "self-start border-slate-700 bg-slate-900"
                    }`}
                  >
                    <div className="mb-1 text-[11px] uppercase tracking-wide text-slate-400">{message.role}</div>
                    <div className="whitespace-pre-wrap text-sm">{message.content}</div>

                    {message.images.length > 0 ? (
                      <div className="mt-3 grid gap-2 sm:grid-cols-2">
                        {message.images.map((image) => (
                          <img
                            key={`${message.id}-${image.fileId}`}
                            className="max-h-72 w-full rounded-md border border-slate-700 object-contain"
                            src={`/api/uploads/${encodeURIComponent(image.fileId)}`}
                            alt={image.filename}
                            loading="lazy"
                          />
                        ))}
                      </div>
                    ) : null}

                    {message.tools.length > 0 ? (
                      <div className="mt-3 flex flex-col gap-2">
                        {message.tools.map((tool, index) => (
                          <div key={`${message.id}-tool-${index}`} className="rounded-md border border-slate-700 bg-slate-950/70 p-2">
                            <div className="mb-1 text-xs font-semibold text-slate-300">{tool.name}</div>
                            {tool.input ? (
                              <pre className="max-h-52 overflow-auto whitespace-pre-wrap rounded bg-slate-900 p-2 text-xs text-slate-300">
                                <code>{tool.input}</code>
                              </pre>
                            ) : null}
                            {tool.result ? (
                              <pre
                                className={`mt-2 max-h-52 overflow-auto whitespace-pre-wrap rounded p-2 text-xs ${
                                  tool.isError ? "bg-rose-900/30 text-rose-200" : "bg-slate-900 text-slate-300"
                                }`}
                              >
                                <code>{tool.result}</code>
                              </pre>
                            ) : null}
                          </div>
                        ))}
                      </div>
                    ) : null}

                    {message.meta ? <div className="mt-2 text-[11px] text-slate-400">{message.meta}</div> : null}
                  </article>
                ))}
              </div>

              <form className="flex flex-col gap-2 border-t border-slate-800 pt-3" onSubmit={handleSend}>
                <textarea
                  value={inputText}
                  onChange={(event) => setInputText(event.target.value)}
                  placeholder="Type a message to the selected agent..."
                  rows={3}
                  disabled={sendMutation.isPending}
                  className="w-full resize-y rounded-lg border border-slate-700 bg-slate-950/80 p-3 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
                />
                <div className="flex justify-end">
                  <button
                    className="rounded-lg border border-sky-500 bg-sky-600 px-4 py-2 text-sm font-medium text-white transition hover:bg-sky-500 disabled:cursor-not-allowed disabled:opacity-60"
                    type="submit"
                    disabled={sendMutation.isPending || inputText.trim().length === 0}
                  >
                    {sendMutation.isPending ? "Sending..." : "Send"}
                  </button>
                </div>
              </form>
            </div>
          ) : (
            <div className="text-sm text-slate-400">Select an agent to open chat.</div>
          )}
        </section>
      </div>
    </section>
  );
}
