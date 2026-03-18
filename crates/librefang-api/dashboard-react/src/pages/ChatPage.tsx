import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { listAgents, listSessions, type AgentItem, type SessionItem } from "../api";

export function ChatPage() {
  const queryClient = useQueryClient();
  const [selectedAgentId, setSelectedAgentId] = useState<string>("");
  const [message, setMessage] = useState("");
  const scrollRef = useRef<HTMLDivElement>(null);

  const agentsQuery = useQuery({
    queryKey: ["agents", "list", "chat"],
    queryFn: listAgents
  });

  const sessionsQuery = useQuery({
    queryKey: ["sessions", "list", "chat"],
    queryFn: listSessions
  });

  const agents = agentsQuery.data ?? [];
  const sessions = sessionsQuery.data ?? [];
  
  // Find the active session for the selected agent
  const activeSession = selectedAgentId 
    ? sessions.find(s => s.agent_id === selectedAgentId && s.active) || sessions.find(s => s.agent_id === selectedAgentId)
    : sessions.find(s => s.active);

  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [selectedAgentId, activeSession]);

  const inputClass = "flex-1 rounded-xl border border-border-subtle bg-surface px-4 py-3 text-sm focus:border-brand focus:ring-2 focus:ring-brand/20 transition-all outline-none";

  const selectedAgent = agents.find(a => a.id === selectedAgentId);

  return (
    <div className="flex h-[calc(100vh-140px)] flex-col transition-colors duration-300">
      <header className="pb-6">
        <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
          <svg className="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
          </svg>
          Neural Terminal
        </div>
        <h1 className="mt-2 text-3xl font-extrabold tracking-tight">Agent Communications</h1>
      </header>

      <div className="flex flex-1 overflow-hidden rounded-2xl border border-border-subtle bg-surface shadow-xl relative ring-1 ring-black/5 dark:ring-white/5">
        {/* Left Sidebar: Agent List */}
        <aside className="w-64 flex-shrink-0 border-r border-border-subtle bg-main/30 backdrop-blur-md flex flex-col">
          <div className="p-4 border-b border-border-subtle">
            <h3 className="text-[10px] font-black uppercase tracking-[0.2em] text-text-dim/60">Active Agents</h3>
          </div>
          <div className="flex-1 overflow-y-auto p-2 space-y-1 scrollbar-thin">
            {agentsQuery.isLoading && (
              <div className="p-4 text-center">
                <div className="h-4 w-4 mx-auto animate-spin rounded-full border-2 border-brand border-t-transparent" />
              </div>
            )}
            {agents.map((agent) => {
              const isActive = selectedAgentId === agent.id;
              const hasSession = sessions.some(s => s.agent_id === agent.id);
              
              return (
                <button
                  key={agent.id}
                  onClick={() => setSelectedAgentId(agent.id)}
                  className={`w-full flex items-center gap-3 p-3 rounded-xl transition-all text-left group ${
                    isActive 
                      ? "bg-brand text-white shadow-lg shadow-brand/20" 
                      : "hover:bg-surface-hover text-slate-700 dark:text-slate-300"
                  }`}
                >
                  <div className={`h-8 w-8 rounded-lg flex items-center justify-center font-black text-xs shrink-0 ${
                    isActive ? "bg-white/20" : "bg-brand/10 text-brand group-hover:bg-brand group-hover:text-white"
                  }`}>
                    {agent.name.charAt(0)}
                  </div>
                  <div className="min-w-0 flex-1">
                    <p className={`text-xs font-black truncate ${isActive ? "text-white" : ""}`}>{agent.name}</p>
                    <div className="flex items-center gap-1.5 mt-0.5">
                      <div className={`h-1 w-1 rounded-full ${hasSession ? 'bg-success' : 'bg-text-dim/40'}`} />
                      <p className={`text-[9px] font-bold uppercase tracking-tight truncate ${isActive ? "text-white/70" : "text-text-dim"}`}>
                        {agent.model_name || "Neural Core"}
                      </p>
                    </div>
                  </div>
                </button>
              );
            })}
            {agents.length === 0 && !agentsQuery.isLoading && (
              <p className="p-4 text-[10px] text-text-dim font-bold text-center uppercase tracking-widest italic">No agents found</p>
            )}
          </div>
        </aside>

        {/* Right Content Area: Chat Messages */}
        <main className="flex-1 flex flex-col overflow-hidden bg-main/10 relative">
          {/* Main Chat Area */}
          <div ref={scrollRef} className="flex-1 overflow-y-auto p-6 space-y-6 scrollbar-thin">
            {!selectedAgentId ? (
              <div className="h-full flex flex-col items-center justify-center text-center p-8">
                <div className="h-16 w-16 rounded-full bg-brand/5 flex items-center justify-center text-brand mb-4">
                  <svg className="h-8 w-8" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="1.5"><path d="M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z" /></svg>
                </div>
                <h3 className="text-lg font-black tracking-tight">Select a Neural Core</h3>
                <p className="text-sm text-text-dim mt-1 max-w-xs font-medium">Choose an agent from the list to establish a communication bridge.</p>
              </div>
            ) : (
              <div className="flex flex-col gap-6">
                <div className="flex justify-center">
                  <span className="px-3 py-1 rounded-full bg-surface border border-border-subtle text-[9px] font-black text-text-dim uppercase tracking-[0.2em] shadow-sm">
                    {activeSession ? `Secure Link: ${activeSession.session_id.slice(0, 8)}` : `Standby: ${selectedAgent?.name}`}
                  </span>
                </div>

                {/* Agent Message (Simulated) */}
                <div className="flex justify-start">
                  <div className="max-w-[85%] flex gap-3 items-end">
                    <div className="h-6 w-6 rounded bg-brand/10 flex items-center justify-center text-brand text-[10px] font-black shrink-0">A</div>
                    <div className="rounded-2xl rounded-bl-sm bg-surface-hover border border-border-subtle p-4 shadow-sm">
                      <p className="text-[10px] font-black text-brand uppercase tracking-widest mb-1">{selectedAgent?.name}</p>
                      <p className="text-sm leading-relaxed font-medium">Neural bridge for <span className="font-black underline decoration-brand/30">{selectedAgent?.name}</span> established. All streams are encrypted. How can I assist with your objectives today?</p>
                    </div>
                  </div>
                </div>

                {/* Placeholder for history */}
                <div className="flex justify-center py-8">
                  <div className="h-[1px] w-12 bg-border-subtle" />
                  <p className="mx-4 text-[9px] font-black text-text-dim/30 uppercase tracking-[0.3em]">End of recent history</p>
                  <div className="h-[1px] w-12 bg-border-subtle" />
                </div>
              </div>
            )}
          </div>

          {/* Input Area */}
          <div className={`p-4 border-t border-border-subtle bg-surface transition-opacity duration-300 ${!selectedAgentId ? 'opacity-30 pointer-events-none' : ''}`}>
            <form className="flex gap-3" onSubmit={(e) => e.preventDefault()}>
              <input
                type="text"
                value={message}
                onChange={(e) => setMessage(e.target.value)}
                placeholder={selectedAgentId ? `Transmit command to ${selectedAgent?.name}...` : "Establish link first..."}
                className={inputClass}
              />
              <button
                type="submit"
                disabled={!message.trim() || !selectedAgentId}
                className="px-6 rounded-xl bg-brand text-white font-black text-sm shadow-lg shadow-brand/20 hover:opacity-90 transition-all disabled:opacity-50 disabled:shadow-none flex items-center justify-center gap-2"
              >
                Send
                <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2.5"><path d="M5 12h14M12 5l7 7-7 7" /></svg>
              </button>
            </form>
            <div className="mt-3 flex justify-between items-center px-1">
              <div className="flex gap-4">
                <p className="text-[9px] font-black text-text-dim/60 uppercase tracking-widest">Latency: 42ms</p>
                <p className="text-[9px] font-black text-text-dim/60 uppercase tracking-widest">Tokens: 0</p>
              </div>
              <div className="flex items-center gap-1.5">
                <div className="h-1 w-1 rounded-full bg-success animate-pulse" />
                <p className="text-[9px] font-black text-text-dim/60 uppercase tracking-widest">Bridge Active</p>
              </div>
            </div>
          </div>
        </main>
      </div>
    </div>
  );
}
