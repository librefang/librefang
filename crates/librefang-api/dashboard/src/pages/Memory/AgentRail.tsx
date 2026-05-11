import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Database, Moon, Search, X } from "lucide-react";
import type { AgentItem, AutoDreamStatus } from "../../api";

interface Props {
  agents: AgentItem[];
  autoDream: AutoDreamStatus | undefined;
  // Per-agent memory counts (record count from useMemorySearchOrList; KV
  // count from per-agent useQueries). Used to render the small mini-metrics
  // line under each agent name.
  recordsByAgentId: Map<string, number>;
  kvCountByAgentId: Map<string, number>;
  // Aggregate counts for the "All agents" entry.
  totalRecords: number;
  totalKv: number;
  // Currently selected agent id; undefined ≡ "all agents".
  selectedAgentId: string | undefined;
  onSelect: (agentId: string | undefined) => void;
}

export function AgentRail({
  agents,
  autoDream,
  recordsByAgentId,
  kvCountByAgentId,
  totalRecords,
  totalKv,
  selectedAgentId,
  onSelect,
}: Props) {
  const { t } = useTranslation();
  const [filter, setFilter] = useState("");

  const dreamByAgentId = useMemo(() => {
    const m = new Map<string, boolean>();
    autoDream?.agents.forEach((a) => m.set(a.agent_id, a.auto_dream_enabled));
    return m;
  }, [autoDream]);

  const visibleAgents = useMemo(() => {
    if (!filter.trim()) return agents;
    const q = filter.trim().toLowerCase();
    return agents.filter(
      (a) =>
        a.name.toLowerCase().includes(q) ||
        a.id.toLowerCase().includes(q),
    );
  }, [agents, filter]);

  return (
    <aside className="flex flex-col gap-2 w-full lg:w-64 shrink-0">
      <div className="flex items-center justify-between">
        <p className="text-[10px] font-black uppercase tracking-widest text-text-dim">
          {t("memory.rail_title", { defaultValue: "Agent scope" })}
        </p>
        <span className="text-[10px] text-text-dim/60">{agents.length}</span>
      </div>

      <div className="relative">
        <Search className="w-3.5 h-3.5 absolute left-2.5 top-1/2 -translate-y-1/2 text-text-dim" />
        <input
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder={t("memory.rail_filter_placeholder", { defaultValue: "Filter agents" })}
          className="w-full rounded-lg border border-border-subtle bg-main pl-8 pr-7 py-1.5 text-xs outline-none focus:border-brand"
        />
        {filter && (
          <button
            onClick={() => setFilter("")}
            className="absolute right-2 top-1/2 -translate-y-1/2 text-text-dim hover:text-text-main"
            aria-label={t("common.clear_search", { defaultValue: "Clear" })}
          >
            <X className="w-3 h-3" />
          </button>
        )}
      </div>

      <div className="flex flex-col gap-1 max-h-[60vh] lg:max-h-none overflow-y-auto scrollbar-thin">
        {/* "All agents" aggregate scope. Always first, always present. */}
        <AgentRailRow
          icon={<Database className="w-3.5 h-3.5" />}
          name={t("memory.rail_all_agents", { defaultValue: "All agents" })}
          subtitle={`${totalRecords} ${t("memory.rail_records_short", { defaultValue: "mem" })} · ${totalKv} ${t("memory.rail_kv_short", { defaultValue: "KV" })}`}
          selected={selectedAgentId === undefined}
          onClick={() => onSelect(undefined)}
        />

        {visibleAgents.length === 0 && filter.trim() ? (
          <p className="px-2 py-3 text-[11px] text-text-dim italic">
            {t("memory.rail_no_match", { defaultValue: "No matching agents." })}
          </p>
        ) : (
          visibleAgents.map((agent) => {
            const records = recordsByAgentId.get(agent.id) ?? 0;
            const kv = kvCountByAgentId.get(agent.id) ?? 0;
            const dreamEnrolled = dreamByAgentId.get(agent.id) ?? false;
            return (
              <AgentRailRow
                key={agent.id}
                icon={
                  <Moon
                    className={`w-3.5 h-3.5 ${
                      dreamEnrolled ? "text-purple-400" : "text-text-dim/40"
                    }`}
                  />
                }
                name={agent.name}
                idHint={agent.id.slice(0, 6)}
                subtitle={`${records} ${t("memory.rail_records_short", { defaultValue: "mem" })} · ${kv} ${t("memory.rail_kv_short", { defaultValue: "KV" })}`}
                selected={selectedAgentId === agent.id}
                onClick={() => onSelect(agent.id)}
              />
            );
          })
        )}
      </div>
    </aside>
  );
}

interface RowProps {
  icon: React.ReactNode;
  name: string;
  idHint?: string;
  subtitle: string;
  selected: boolean;
  onClick: () => void;
}

function AgentRailRow({ icon, name, idHint, subtitle, selected, onClick }: RowProps) {
  return (
    <button
      onClick={onClick}
      aria-pressed={selected}
      className={`w-full text-left rounded-lg px-2 py-2 transition-colors flex items-start gap-2 ${
        selected
          ? "bg-brand/10 border border-brand/40"
          : "border border-transparent hover:bg-main/50"
      }`}
    >
      <span className={`mt-0.5 ${selected ? "text-brand" : ""}`}>{icon}</span>
      <span className="min-w-0 flex-1">
        <span className="flex items-baseline gap-1.5">
          <span
            className={`text-xs font-bold truncate ${selected ? "text-brand" : ""}`}
            title={name}
          >
            {name}
          </span>
          {idHint && (
            <span className="text-[9px] font-mono text-text-dim/60 shrink-0">{idHint}</span>
          )}
        </span>
        <span className="block text-[10px] text-text-dim mt-0.5">{subtitle}</span>
      </span>
    </button>
  );
}
