import { useTranslation } from "react-i18next";
import { Clock, Database, Moon, Sparkles, Zap } from "lucide-react";
import { Badge } from "../../components/ui/Badge";
import type {
  AgentItem,
  AutoDreamAgentStatus,
  AutoDreamStatus,
  MemoryStatsResponse,
} from "../../api";
import { formatRelativeMs } from "./formatters";

interface Props {
  scopedAgent: AgentItem | undefined;
  agentStats: MemoryStatsResponse | undefined;
  autoDream: AutoDreamStatus | undefined;
  dreamForAgent: AutoDreamAgentStatus | undefined;
  // Number of KV pairs in the current scope. For "all agents" the caller
  // passes the aggregate; for a single agent, just that agent's KV count.
  kvCount: number;
}

export function ScopeSummary({
  scopedAgent,
  agentStats,
  autoDream,
  dreamForAgent,
  kvCount,
}: Props) {
  const { t, i18n } = useTranslation();
  const now = Date.now();
  const tNever = () => t("common.never", { defaultValue: "never" });

  const totalMemories = agentStats?.total ?? 0;

  // Composition chips: user / session / agent level counts. Shown for both
  // the aggregate scope and per-agent scope so the user always sees the same
  // breakdown shape.
  const composition = [
    {
      icon: Sparkles,
      label: t("memory.user", { defaultValue: "User" }),
      value: agentStats?.user_count ?? 0,
      color: "text-success",
    },
    {
      icon: Clock,
      label: t("memory.session", { defaultValue: "Session" }),
      value: agentStats?.session_count ?? 0,
      color: "text-accent",
    },
    {
      icon: Zap,
      label: t("memory.agent", { defaultValue: "Agent" }),
      value: agentStats?.agent_count ?? 0,
      color: "text-warning",
    },
  ];

  return (
    <div className="rounded-2xl border border-border-subtle bg-surface px-4 py-3">
      <div className="flex flex-wrap items-center gap-x-5 gap-y-2">
        <div className="flex items-center gap-2">
          <Database className="w-4 h-4 text-brand" />
          <div className="flex flex-col">
            <p className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">
              {scopedAgent
                ? t("memory.scope_agent_label", { defaultValue: "Agent" })
                : t("memory.scope_all_label", { defaultValue: "Scope" })}
            </p>
            <p className="text-sm font-bold">
              {scopedAgent
                ? scopedAgent.name
                : t("memory.rail_all_agents", { defaultValue: "All agents" })}
            </p>
          </div>
        </div>

        <Divider />

        <div className="flex items-baseline gap-1.5">
          <span className="text-2xl font-black text-brand tabular-nums">{totalMemories}</span>
          <span className="text-[10px] uppercase tracking-widest text-text-dim/60">
            {t("memory.total_memories")}
          </span>
        </div>

        <div className="flex items-baseline gap-1.5">
          <span className="text-2xl font-black text-text-main tabular-nums">{kvCount}</span>
          <span className="text-[10px] uppercase tracking-widest text-text-dim/60">
            {t("memory.kv_label_short", { defaultValue: "KV pairs" })}
          </span>
        </div>

        <Divider />

        <div className="flex items-center gap-3 flex-wrap">
          {composition.map((c) => (
            <div key={c.label} className="flex items-center gap-1.5">
              <c.icon className={`w-3.5 h-3.5 ${c.color}`} />
              <span className="text-xs">
                <span className="font-bold tabular-nums">{c.value}</span>
                <span className="text-text-dim ml-1">{c.label}</span>
              </span>
            </div>
          ))}
        </div>

        <Divider />

        {/* Auto-Dream status. Global enabled/disabled is shown regardless of
            scope; per-agent enroll + last/next is shown only when scoped to a
            single agent. */}
        <div className="flex items-center gap-2 flex-wrap">
          <Moon
            className={`w-3.5 h-3.5 ${
              autoDream?.enabled ? "text-purple-400" : "text-text-dim/40"
            }`}
          />
          <Badge variant={autoDream?.enabled ? "success" : "default"}>
            {autoDream?.enabled
              ? t("memory.auto_dream_on_badge", { defaultValue: "Auto-Dream on" })
              : t("memory.auto_dream_off_badge", { defaultValue: "Auto-Dream off" })}
          </Badge>
          {scopedAgent && dreamForAgent && (
            <>
              <Badge
                variant={dreamForAgent.auto_dream_enabled ? "info" : "default"}
              >
                {dreamForAgent.auto_dream_enabled
                  ? t("settings.auto_dream_enrolled", "Enrolled")
                  : t("settings.auto_dream_not_enrolled", "Off")}
              </Badge>
              {dreamForAgent.auto_dream_enabled && (
                <span className="text-[11px] text-text-dim">
                  {t("settings.auto_dream_last", "Last")}:{" "}
                  {formatRelativeMs(
                    dreamForAgent.last_consolidated_at_ms,
                    now,
                    i18n.language,
                    tNever,
                  )}
                  {" · "}
                  {t("settings.auto_dream_next", "Next")}:{" "}
                  {formatRelativeMs(
                    dreamForAgent.next_eligible_at_ms,
                    now,
                    i18n.language,
                    tNever,
                  )}
                </span>
              )}
            </>
          )}
        </div>
      </div>
    </div>
  );
}

function Divider() {
  return <span className="hidden md:inline-block w-px h-6 bg-border-subtle/50" />;
}
