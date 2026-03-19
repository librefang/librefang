import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { listAgents } from "../api";
import { PageHeader } from "../components/ui/PageHeader";
import { CardSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { Search, Users } from "lucide-react";

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
      <PageHeader
        badge={t("common.kernel_runtime")}
        title={t("agents.title")}
        subtitle={t("agents.subtitle")}
        isFetching={agentsQuery.isFetching}
        onRefresh={() => void agentsQuery.refetch()}
        icon={<Users className="h-4 w-4" />}
      />

      <div className="relative">
        <div className="absolute inset-y-0 left-0 pl-4 flex items-center pointer-events-none text-text-dim">
          <Search className="h-4 w-4" />
        </div>
        <input
          type="text"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder={t("common.search")}
          className="w-full pl-11 pr-4 py-3 rounded-2xl border border-border-subtle bg-surface shadow-sm focus:ring-2 focus:ring-brand/20 focus:border-brand outline-none transition-all font-medium"
        />
      </div>

      {agentsQuery.isLoading ? (
        <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
          {[1, 2, 3, 4, 5, 6].map((i) => <CardSkeleton key={i} />)}
        </div>
      ) : filteredAgents.length === 0 ? (
        search ? (
          <EmptyState
            title={t("agents.no_matching")}
            icon={<Search className="h-6 w-6" />}
          />
        ) : (
          <EmptyState
            title={t("common.no_data")}
            icon={<Users className="h-6 w-6" />}
          />
        )
      ) : (
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
      )}
    </div>
  );
}
