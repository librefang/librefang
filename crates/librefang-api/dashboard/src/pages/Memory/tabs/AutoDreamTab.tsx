import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { Moon } from "lucide-react";
import { Card } from "../../../components/ui/Card";
import { EmptyState } from "../../../components/ui/EmptyState";
import { Badge } from "../../../components/ui/Badge";
import { useAutoDreamStatus } from "../../../lib/queries/autoDream";
import {
  useTriggerAutoDream,
  useAbortAutoDream,
  useSetAutoDreamEnabled,
} from "../../../lib/mutations/autoDream";
import type { AgentItem, AutoDreamAgentStatus } from "../../../api";
import { useUIStore } from "../../../lib/store";
import { AutoDreamAgentRow } from "../components/AutoDreamAgentRow";

interface Props {
  agents: AgentItem[];
  scopedAgentId: string | undefined;
}

export function AutoDreamTab({ agents, scopedAgentId }: Props) {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);
  const dreamStatusQuery = useAutoDreamStatus();
  const dreamTrigger = useTriggerAutoDream();
  const dreamAbort = useAbortAutoDream();
  const dreamSetEnabled = useSetAutoDreamEnabled();

  const dreamStatus = dreamStatusQuery.data;
  const dreamByAgentId = useMemo(() => {
    const m = new Map<string, AutoDreamAgentStatus>();
    dreamStatus?.agents.forEach((a) => m.set(a.agent_id, a));
    return m;
  }, [dreamStatus]);

  // All three actions surface feedback through the global toast queue so the
  // page is consistent with every other action (RecordsTab cleanup, the
  // dialogs, etc.). Local error/msg state would be inconsistent and would
  // also be lost when the user navigates between tabs.
  //
  // `outcome.reason` is backend-supplied free text and CAN be null/undefined
  // in edge cases; without a guard the toast would render the literal string
  // "undefined" to the user.
  const fallbackReason = () => t("common.unknown", { defaultValue: "Unknown" });
  const onTrigger = async (agentId: string) => {
    try {
      const outcome = await dreamTrigger.mutateAsync(agentId);
      addToast(
        outcome.fired
          ? t("settings.auto_dream_fired", "Consolidation fired")
          : (outcome.reason ?? fallbackReason()),
        outcome.fired ? "success" : "info",
      );
    } catch (e) {
      addToast(e instanceof Error ? e.message : String(e), "error");
    }
  };

  const onAbort = async (agentId: string) => {
    try {
      const outcome = await dreamAbort.mutateAsync(agentId);
      addToast(
        outcome.aborted
          ? t("settings.auto_dream_aborted", "Abort signalled")
          : (outcome.reason ?? fallbackReason()),
        outcome.aborted ? "success" : "info",
      );
    } catch (e) {
      addToast(e instanceof Error ? e.message : String(e), "error");
    }
  };

  const onToggle = async (agentId: string, enabled: boolean) => {
    try {
      await dreamSetEnabled.mutateAsync({ agentId, enabled });
      addToast(
        enabled
          ? t("settings.auto_dream_enrolled_ok", "Agent enrolled")
          : t("settings.auto_dream_unenrolled_ok", "Agent unenrolled"),
        "success",
      );
    } catch (e) {
      addToast(e instanceof Error ? e.message : String(e), "error");
    }
  };

  const visibleAgents = scopedAgentId
    ? agents.filter((a) => a.id === scopedAgentId)
    : agents;

  if (agents.length === 0) {
    return (
      <EmptyState
        title={t("memory.kv_no_agents", { defaultValue: "No agents available" })}
        icon={<Moon className="h-6 w-6" />}
      />
    );
  }

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-2 flex-wrap">
        <p className="text-xs text-text-dim">
          {t("memory.auto_dream_desc_inline", {
            defaultValue:
              "Periodic memory consolidation per agent. Manifest override > global default; configure global in config.toml under [auto_dream].",
          })}
        </p>
        {dreamStatus && (
          <Badge variant={dreamStatus.enabled ? "success" : "default"}>
            <Moon className="w-3 h-3 mr-1 inline" />
            {dreamStatus.enabled
              ? t("memory.auto_dream_on_badge", { defaultValue: "Auto-Dream on" })
              : t("memory.auto_dream_off_badge", { defaultValue: "Auto-Dream off" })}
          </Badge>
        )}
      </div>

      {visibleAgents.length === 0 ? (
        <EmptyState
          title={t("memory.dream_agent_missing", {
            defaultValue: "Selected agent has no dream status yet",
          })}
          icon={<Moon className="h-6 w-6" />}
        />
      ) : (
        <div className="flex flex-col gap-2">
          {visibleAgents.map((agent) => {
            const dream = dreamByAgentId.get(agent.id);
            if (!dream) {
              return (
                <Card key={agent.id} padding="md">
                  <div className="flex items-center gap-2 flex-wrap mb-1">
                    <h4 className="text-xs font-bold">{agent.name}</h4>
                    <span className="text-[10px] font-mono text-text-dim">
                      {agent.id.slice(0, 8)}
                    </span>
                  </div>
                  <p className="text-[11px] text-text-dim italic">
                    {t("memory.dream_no_status", {
                      defaultValue:
                        "No dream status yet — the scheduler hasn't registered this agent.",
                    })}
                  </p>
                </Card>
              );
            }
            return (
              <Card key={agent.id} padding="md">
                <div className="flex items-center gap-2 flex-wrap mb-2">
                  <h4 className="text-xs font-bold">{agent.name}</h4>
                  <span className="text-[10px] font-mono text-text-dim">
                    {agent.id.slice(0, 8)}
                  </span>
                </div>
                <AutoDreamAgentRow
                  agent={dream}
                  disabled={!dreamStatus?.enabled}
                  hideAgentName
                  onTrigger={onTrigger}
                  onAbort={onAbort}
                  onToggle={onToggle}
                  triggerPending={
                    dreamTrigger.isPending && dreamTrigger.variables === dream.agent_id
                  }
                  abortPending={
                    dreamAbort.isPending && dreamAbort.variables === dream.agent_id
                  }
                  togglePending={
                    dreamSetEnabled.isPending &&
                    dreamSetEnabled.variables?.agentId === dream.agent_id
                  }
                />
              </Card>
            );
          })}
        </div>
      )}

      {dreamStatusQuery.isError && (
        <p className="text-xs text-red-500">
          {t("settings.auto_dream_load_err", "Failed to load auto-dream status")}
        </p>
      )}
    </div>
  );
}
