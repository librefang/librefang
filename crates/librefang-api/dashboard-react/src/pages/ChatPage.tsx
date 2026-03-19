import { useQuery } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { listAgents, listSessions } from "../api";
import { MessageCircle, Send } from "lucide-react";

export function ChatPage() {
  const { t } = useTranslation();
  const [selectedAgentId, setSelectedAgentId] = useState<string>("");
  const [message, setMessage] = useState("");
  const scrollRef = useRef<HTMLDivElement>(null);

  const agentsQuery = useQuery({ queryKey: ["agents", "list", "chat"], queryFn: listAgents });
  const sessionsQuery = useQuery({ queryKey: ["sessions", "list", "chat"], queryFn: listSessions });

  const agents = agentsQuery.data ?? [];
  const sessions = sessionsQuery.data ?? [];

  const activeSession = selectedAgentId
    ? sessions.find(s => s.agent_id === selectedAgentId) || null
    : sessions[0] || null;

  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [selectedAgentId, activeSession]);

  const selectedAgent = agents.find(a => a.id === selectedAgentId);

  return (
    <div className="flex h-[calc(100vh-140px)] flex-col transition-colors duration-300">
      <header className="pb-6">
        <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
          <MessageCircle className="h-4 w-4" />
          {t("chat.neural_terminal")}
        </div>
        <h1 className="mt-2 text-3xl font-extrabold tracking-tight">{t("chat.title")}</h1>
      </header>

      <div className="flex flex-1 overflow-hidden rounded-2xl border border-border-subtle bg-surface shadow-xl relative ring-1 ring-black/5 dark:ring-white/5">
        <aside className="w-64 flex-shrink-0 border-r border-border-subtle bg-main/30 backdrop-blur-md flex flex-col">
          <div className="p-4 border-b border-border-subtle">
            <h3 className="text-[10px] font-black uppercase tracking-[0.2em] text-text-dim/60">{t("nav.agents")}</h3>
          </div>
          <div className="flex-1 overflow-y-auto p-2 space-y-1 scrollbar-thin">
            {agents.map((agent) => (
              <button key={agent.id} onClick={() => setSelectedAgentId(agent.id)} className={`w-full flex items-center gap-3 p-3 rounded-xl transition-all text-left group ${selectedAgentId === agent.id ? "bg-brand text-white shadow-lg" : "hover:bg-surface-hover text-slate-700 dark:text-slate-300"}`}>
                <div className={`h-8 w-8 rounded-lg flex items-center justify-center font-black text-xs shrink-0 ${selectedAgentId === agent.id ? "bg-white/20" : "bg-brand/10 text-brand group-hover:bg-brand group-hover:text-white"}`}>{agent.name.charAt(0)}</div>
                <div className="min-w-0 flex-1">
                  <p className="text-xs font-black truncate">{agent.name}</p>
                  <p className={`text-[9px] font-bold uppercase tracking-tight truncate ${selectedAgentId === agent.id ? "text-white/70" : "text-text-dim"}`}>{agent.model_name || t("common.unknown")}</p>
                </div>
              </button>
            ))}
          </div>
        </aside>

        <main className="flex-1 flex flex-col overflow-hidden bg-main/10 relative">
          <div ref={scrollRef} className="flex-1 overflow-y-auto p-6 space-y-6 scrollbar-thin">
            {!selectedAgentId ? (
              <div className="h-full flex flex-col items-center justify-center text-center p-8">
                <div className="h-16 w-16 rounded-full bg-brand/5 flex items-center justify-center text-brand mb-4">
                  <MessageCircle className="h-8 w-8" />
                </div>
                <h3 className="text-lg font-black tracking-tight">{t("chat.select_agent")}</h3>
                <p className="text-sm text-text-dim mt-1 max-w-xs font-medium">{t("chat.select_agent_desc")}</p>
              </div>
            ) : (
              <div className="flex flex-col gap-6">
                <div className="flex justify-center"><span className="px-3 py-1 rounded-full bg-surface border border-border-subtle text-[9px] font-black text-text-dim uppercase tracking-[0.2em] shadow-sm">{activeSession ? `${t("chat.secure_link")}: ${activeSession.session_id.slice(0, 8)}` : `${t("chat.standby")}: ${selectedAgent?.name}`}</span></div>
                <div className="flex justify-start"><div className="max-w-[85%] flex gap-3 items-end"><div className="h-6 w-6 rounded bg-brand/10 flex items-center justify-center text-brand text-[10px] font-black shrink-0">A</div><div className="rounded-2xl rounded-bl-sm bg-surface-hover border border-border-subtle p-4 shadow-sm"><p className="text-[10px] font-black text-brand uppercase tracking-widest mb-1">{selectedAgent?.name}</p><p className="text-sm leading-relaxed font-medium">{t("chat.welcome_system")}</p></div></div></div>
                <div className="flex justify-center py-8"><div className="h-[1px] w-12 bg-border-subtle" /><p className="mx-4 text-[9px] font-black text-text-dim/30 uppercase tracking-[0.3em]">{t("chat.end_history")}</p><div className="h-[1px] w-12 bg-border-subtle" /></div>
              </div>
            )}
          </div>

          <div className={`p-4 border-t border-border-subtle bg-surface transition-opacity duration-300 ${!selectedAgentId ? 'opacity-30 pointer-events-none' : ''}`}>
            <form className="flex gap-3" onSubmit={(e) => e.preventDefault()}>
              <input type="text" value={message} onChange={(e) => setMessage(e.target.value)} placeholder={selectedAgentId ? t("chat.input_placeholder_with_agent", { name: selectedAgent?.name }) : t("chat.transmit_command")} className="flex-1 rounded-xl border border-border-subtle bg-surface px-4 py-3 text-sm focus:border-brand outline-none transition-all" />
              <button type="submit" disabled={!message.trim() || !selectedAgentId} className="px-6 rounded-xl bg-brand text-white font-black text-sm shadow-lg hover:opacity-90 transition-all flex items-center justify-center gap-2">{t("chat.send")}<Send className="h-4 w-4" /></button>
            </form>
          </div>
        </main>
      </div>
    </div>
  );
}
