import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { listAgents, type AgentItem } from "../api";

const REFRESH_MS = 30000;

function statusClass(status?: string): string {
  const value = (status ?? "").toLowerCase();
  if (value === "running") return "border-success/20 bg-success/10 text-success";
  if (value === "idle") return "border-warning/20 bg-warning/10 text-warning";
  if (value === "error") return "border-error/20 bg-error/10 text-error";
  return "border-border-subtle bg-surface-hover text-text-dim";
}

export function AgentsPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [search, setSearch] = useState("");

  const agentsQuery = useQuery({
    queryKey: ["agents", "list"],
    queryFn: listAgents,
    refetchInterval: REFRESH_MS
  });

  const agents = agentsQuery.data ?? [];
  const filteredAgents = agents.filter(a => 
    a.name.toLowerCase().includes(search.toLowerCase()) || 
    a.id.toLowerCase().includes(search.toLowerCase())
  );

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <header className="flex flex-col justify-between gap-4 sm:flex-row sm:items-end">
        <div>
          <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
            <svg className="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2" /><circle cx="9" cy="7" r="4" />
            </svg>
            {t("common.kernel_runtime")}
          </div>
          <h1 className="mt-2 text-3xl font-extrabold tracking-tight md:text-4xl">{t("agents.title")}</h1>
          <p className="mt-1 text-text-dim font-medium max-w-2xl">{t("agents.subtitle")}</p>
        </div>
        <div className="flex gap-2">
          <button
            className="flex h-9 items-center gap-2 rounded-xl border border-border-subtle bg-surface px-4 text-sm font-bold text-text-dim hover:text-brand transition-all shadow-sm"
            onClick={() => void agentsQuery.refetch()}
          >
            <svg className={`h-3.5 w-3.5 ${agentsQuery.isFetching ? "animate-spin" : ""}`} fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2">
              <path d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
            </svg>
            {t("common.refresh")}
          </button>
        </div>
      </header>

      <div className="relative">
        <div className="absolute inset-y-0 left-0 pl-4 flex items-center pointer-events-none text-text-dim">
          <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2.5"><path d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" /></svg>
        </div>
        <input
          type="text"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder={t("common.search")}
          className="w-full pl-11 pr-4 py-3 rounded-2xl border border-border-subtle bg-surface shadow-sm focus:ring-2 focus:ring-brand/20 focus:border-brand outline-none transition-all font-medium"
        />
      </div>

      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
        {filteredAgents.map((agent) => (
          <article key={agent.id} className="group rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm transition-all hover:border-brand/30 ring-1 ring-black/5 dark:ring-white/5">
            <div className="flex items-start justify-between gap-4 mb-6">
              <div className="flex items-center gap-4 min-w-0">
                <div className="h-12 w-12 rounded-2xl bg-brand/10 flex items-center justify-center text-brand text-xl font-black shrink-0 shadow-inner">
                  {agent.name.charAt(0)}
                </div>
                <div className="min-w-0">
                  <h2 className="text-lg font-black tracking-tight truncate group-hover:text-brand transition-colors">{agent.name}</h2>
                  <p className="text-[10px] font-black uppercase tracking-widest text-text-dim/60 truncate">{agent.id}</p>
                </div>
              </div>
              <span className={`shrink-0 rounded-lg border px-2 py-0.5 text-[10px] font-black uppercase tracking-widest ${statusClass(agent.state)}`}>
                {agent.state ? t(`common.${agent.state.toLowerCase()}`, { defaultValue: agent.state }) : t("common.idle")}
              </span>
            </div>

            <div className="space-y-3 mb-6">
              <div className="flex justify-between items-center text-xs">
                <span className="text-text-dim font-bold uppercase tracking-wider text-[10px]">{t("agents.model")}</span>
                <span className="font-black text-slate-700 dark:text-slate-300">{agent.model_name || t("common.unknown")}</span>
              </div>
              <div className="flex justify-between items-center text-xs">
                <span className="text-text-dim font-bold uppercase tracking-wider text-[10px]">{t("agents.provider")}</span>
                <span className="font-black text-brand">{agent.model_provider || t("common.local")}</span>
              </div>
              <div className="flex justify-between items-center text-xs">
                <span className="text-text-dim font-bold uppercase tracking-wider text-[10px]">{t("agents.last_active")}</span>
                <span className="font-mono text-[10px]">{agent.last_active ? new Date(agent.last_active).toLocaleTimeString() : t("common.never")}</span>
              </div>
            </div>

            <div className="pt-4 border-t border-border-subtle/30 flex gap-2">
              <button className="flex-1 rounded-xl border border-border-subtle bg-surface py-2 text-[10px] font-black uppercase tracking-widest text-text-dim hover:text-brand hover:border-brand/30 transition-all shadow-sm">
                {t("common.config")}
              </button>
              <button className="flex-1 rounded-xl bg-brand/5 border border-brand/10 py-2 text-[10px] font-black uppercase tracking-widest text-brand hover:bg-brand/10 transition-all shadow-sm">
                {t("common.interact")}
              </button>
            </div>
          </article>
        ))}
      </div>

      {filteredAgents.length === 0 && !agentsQuery.isLoading && (
        <div className="py-24 text-center border border-dashed border-border-subtle rounded-3xl bg-surface/30">
          <p className="text-sm text-text-dim font-black tracking-tight italic">{t("agents.no_matching")}</p>
        </div>
      )}
    </div>
  );
}
