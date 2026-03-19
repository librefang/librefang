import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "@tanstack/react-router";
import { listAgents } from "../api";
import { PageHeader } from "../components/ui/PageHeader";
import { CardSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { Card } from "../components/ui/Card";
import { Input } from "../components/ui/Input";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { Avatar } from "../components/ui/Avatar";
import { Search, Users, Settings, MessageCircle } from "lucide-react";

const REFRESH_MS = 30000;

function getStatusVariant(status?: string) {
  const value = (status ?? "").toLowerCase();
  if (value === "running") return "success";
  if (value === "idle") return "warning";
  if (value === "error") return "error";
  return "default";
}

export function AgentsPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
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

      <Input
        value={search}
        onChange={(e) => setSearch(e.target.value)}
        placeholder={t("common.search")}
        leftIcon={<Search className="h-4 w-4" />}
      />

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
            <Card key={agent.id} hover padding="lg">
              <div className="flex items-start justify-between gap-4 mb-6">
                <div className="flex items-center gap-4 min-w-0">
                  <Avatar fallback={agent.name} size="lg" />
                  <div className="min-w-0">
                    <h2 className="text-lg font-black tracking-tight truncate group-hover:text-brand transition-colors">{agent.name}</h2>
                    <p className="text-[10px] font-black uppercase tracking-widest text-text-dim/60 truncate">{agent.id}</p>
                  </div>
                </div>
                <Badge variant={getStatusVariant(agent.state)}>
                  {agent.state ? t(`common.${agent.state.toLowerCase()}`, { defaultValue: agent.state }) : t("common.idle")}
                </Badge>
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
                <Button variant="secondary" size="sm" className="flex-1" onClick={() => navigate({ to: "/settings" })}>
                  <Settings className="h-3.5 w-3.5 mr-1" />
                  {t("common.config")}
                </Button>
                <Button variant="primary" size="sm" className="flex-1" onClick={() => navigate({ to: "/chat", search: { agentId: agent.id } })}>
                  <MessageCircle className="h-3.5 w-3.5 mr-1" />
                  {t("common.interact")}
                </Button>
              </div>
            </Card>
          ))}
        </div>
      )}
    </div>
  );
}
