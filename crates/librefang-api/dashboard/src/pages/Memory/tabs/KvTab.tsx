import { useTranslation } from "react-i18next";
import type { UseQueryResult } from "@tanstack/react-query";
import { Database } from "lucide-react";
import { Card } from "../../../components/ui/Card";
import { EmptyState } from "../../../components/ui/EmptyState";
import type { AgentItem, AgentKvPair } from "../../../api";
import { AgentKvRows } from "../components/AgentKvRows";

interface Props {
  agents: AgentItem[];
  // When defined, only that agent's card is rendered. Otherwise every agent
  // gets a card (the aggregate view).
  scopedAgentId: string | undefined;
  // Pre-computed Map of agent.id → KV query result. Owned by the page entry
  // so this tab can stay presentational (no second `useQueries` observer set
  // duplicating subscriptions on every cache update). See `pages/Memory/index.tsx`.
  kvQueryByAgentId: Map<string, UseQueryResult<AgentKvPair[]>>;
}

export function KvTab({ agents, scopedAgentId, kvQueryByAgentId }: Props) {
  const { t } = useTranslation();

  const visibleAgents = scopedAgentId
    ? agents.filter((a) => a.id === scopedAgentId)
    : agents;

  if (agents.length === 0) {
    return (
      <EmptyState
        title={t("memory.kv_no_agents", { defaultValue: "No agents available" })}
        icon={<Database className="h-6 w-6" />}
      />
    );
  }

  if (visibleAgents.length === 0) {
    return (
      <EmptyState
        title={t("memory.kv_agent_missing", {
          defaultValue: "Selected agent has no KV data yet",
        })}
        icon={<Database className="h-6 w-6" />}
      />
    );
  }

  return (
    <div className="grid gap-4">
      {visibleAgents.map((agent) => {
        const kvQuery = kvQueryByAgentId.get(agent.id);
        return (
          <Card key={agent.id} padding="md">
            <div className="flex items-center gap-2 mb-3 flex-wrap">
              <h4 className="text-xs font-bold">{agent.name}</h4>
              <span className="text-[10px] font-mono text-text-dim">
                {agent.id.slice(0, 8)}
              </span>
            </div>
            <div className="overflow-x-auto">
              <table className="w-full text-left">
                <thead>
                  <tr className="text-[10px] font-bold uppercase tracking-widest text-text-dim/60">
                    <th className="px-3 py-2">
                      {t("memory.kv_key", { defaultValue: "Key" })}
                    </th>
                    <th className="px-3 py-2">
                      {t("memory.kv_value", { defaultValue: "Value" })}
                    </th>
                    <th className="px-3 py-2">
                      {t("memory.kv_source", { defaultValue: "Source" })}
                    </th>
                    <th className="px-3 py-2">
                      {t("memory.created", { defaultValue: "Created" })}
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {kvQuery ? (
                    <AgentKvRows kvQuery={kvQuery} />
                  ) : (
                    <tr>
                      <td colSpan={4} className="px-3 py-2 text-xs text-text-dim/60 italic">
                        {t("memory.kv_empty", { defaultValue: "No KV entries" })}
                      </td>
                    </tr>
                  )}
                </tbody>
              </table>
            </div>
          </Card>
        );
      })}
    </div>
  );
}
