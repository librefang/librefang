import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQueries, type UseQueryResult } from "@tanstack/react-query";
import { useNavigate, useSearch } from "@tanstack/react-router";
import { Database, FileText, HeartPulse, KeyRound, Moon, Settings } from "lucide-react";
import { PageHeader } from "../../components/ui/PageHeader";
import { ConfirmDialog } from "../../components/ui/ConfirmDialog";
import { Button } from "../../components/ui/Button";
import { Card } from "../../components/ui/Card";
import {
  useMemoryConfig,
  useMemorySearchOrList,
  useMemoryStats,
  agentKvMemoryQueryOptions,
} from "../../lib/queries/memory";
import { useAgents } from "../../lib/queries/agents";
import { useAutoDreamStatus } from "../../lib/queries/autoDream";
import { useDeleteMemory } from "../../lib/mutations/memory";
import { useUIStore } from "../../lib/store";
import { useCreateShortcut } from "../../lib/useCreateShortcut";
import type { AgentKvPair, MemoryItem } from "../../api";

import { AgentRail } from "./AgentRail";
import { ScopeSummary } from "./ScopeSummary";
import { RecordsTab } from "./tabs/RecordsTab";
import { KvTab } from "./tabs/KvTab";
import { AutoDreamTab } from "./tabs/AutoDreamTab";
import { HealthTab } from "./tabs/HealthTab";
import { AddMemoryDialog, EditMemoryDialog, MemoryConfigDialog } from "./dialogs";
import { MEMORY_TABS, type MemoryTab } from "./constants";

export function MemoryPage() {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);
  const navigate = useNavigate({ from: "/memory" });
  const search = useSearch({ from: "/memory" }) as {
    agent?: string;
    tab?: MemoryTab;
  };

  const selectedAgentId = search.agent;
  const activeTab: MemoryTab = MEMORY_TABS.includes(search.tab as MemoryTab)
    ? (search.tab as MemoryTab)
    : "records";

  // Dialog state. Add / Edit / Config / Delete are all driven from the
  // page entry so that buttons surfacing them can live in any tab without
  // threading state through props.
  const [showAddDialog, setShowAddDialog] = useState(false);
  const [showConfigDialog, setShowConfigDialog] = useState(false);
  const [editingMemory, setEditingMemory] = useState<MemoryItem | null>(null);
  const [deleteConfirm, setDeleteConfirm] = useState<{ id: string } | null>(null);
  useCreateShortcut(() => setShowAddDialog(true));

  // Page-level queries that feed both the rail (mini-metrics per agent) and
  // the scope summary. Each tab also subscribes to what it needs; React Query
  // dedupes the network hit.
  const agentsQuery = useAgents();
  const agents = useMemo(() => agentsQuery.data ?? [], [agentsQuery.data]);
  const autoDreamQuery = useAutoDreamStatus();
  const memoryConfigQuery = useMemoryConfig();

  // Source of truth for "is proactive memory available right now":
  //   1. /api/memory response carries `proactive_enabled` (preferred —
  //      reflects runtime store presence, not just config intent).
  //   2. Fall back to /api/memory/config while the list query is in flight
  //      so the UI doesn't flicker the disabled notice during load.
  // While both are still loading we default to `true` to avoid flashing
  // the disabled notice on first paint.
  const memoryQuery = useMemorySearchOrList("");
  const proactiveEnabled =
    memoryQuery.data?.proactive_enabled ??
    memoryConfigQuery.data?.proactive_memory?.enabled ??
    true;

  // Pre-compute per-agent record counts from the unfiltered memory list so
  // the rail can show "N mem" next to every agent without re-fetching.
  const recordsByAgentId = useMemo(() => {
    const m = new Map<string, number>();
    (memoryQuery.data?.memories ?? []).forEach((mem) => {
      if (mem.agent_id) m.set(mem.agent_id, (m.get(mem.agent_id) ?? 0) + 1);
    });
    return m;
  }, [memoryQuery.data]);
  const totalRecords = memoryQuery.data?.memories?.length ?? 0;

  // Per-agent KV: one `useQueries` observer set, owned at the page level so
  // the rail + scope summary + KV tab all share the same observers. Without
  // this, the KV tab would mount its own parallel `useQueries` and pay double
  // re-render cost on every cache update (the network fetch is deduped by
  // React Query, but observer overhead is per call site).
  const kvQueries = useQueries({
    queries: agents.map((agent) => agentKvMemoryQueryOptions(agent.id)),
  });
  // Zip results into a Map keyed by agent id so consumers don't have to
  // re-derive the agent→index alignment with O(n²) `findIndex` lookups.
  const kvQueryByAgentId = useMemo(() => {
    const m = new Map<string, UseQueryResult<AgentKvPair[]>>();
    agents.forEach((agent, idx) => {
      m.set(agent.id, kvQueries[idx]);
    });
    return m;
  }, [agents, kvQueries]);
  const kvCountByAgentId = useMemo(() => {
    const m = new Map<string, number>();
    agents.forEach((agent, idx) => {
      const data = kvQueries[idx]?.data;
      if (data) m.set(agent.id, data.length);
    });
    return m;
  }, [agents, kvQueries]);
  const totalKv = useMemo(
    () => Array.from(kvCountByAgentId.values()).reduce((a, b) => a + b, 0),
    [kvCountByAgentId],
  );

  // Scope summary needs stats narrowed to the selected agent. The endpoint
  // accepts an agent id and otherwise returns the workspace aggregate; either
  // way the response shape is MemoryStatsResponse so the UI doesn't branch.
  const scopedStatsQuery = useMemoryStats(selectedAgentId);
  const scopedAgent = selectedAgentId
    ? agents.find((a) => a.id === selectedAgentId)
    : undefined;
  const dreamForAgent = selectedAgentId
    ? autoDreamQuery.data?.agents.find((a) => a.agent_id === selectedAgentId)
    : undefined;
  const scopedKvCount = selectedAgentId
    ? (kvCountByAgentId.get(selectedAgentId) ?? 0)
    : totalKv;

  const deleteMutation = useDeleteMemory();

  const setSearchParam = (next: { agent?: string; tab?: MemoryTab }) => {
    navigate({
      search: (prev) => {
        const out: { agent?: string; tab?: MemoryTab } = { ...prev };
        if ("agent" in next) {
          if (next.agent === undefined) delete out.agent;
          else out.agent = next.agent;
        }
        if ("tab" in next) {
          if (next.tab === undefined) delete out.tab;
          else out.tab = next.tab;
        }
        return out;
      },
    });
  };

  // Stale-URL guard: if the URL points at an agent that no longer exists
  // (deleted out from under us, or a shared link from a different
  // workspace), silently fall back to the aggregate scope. Without this the
  // page would render "All agents" labels but with the deleted agent's
  // zero-count stats response, which is worse than just showing aggregate.
  useEffect(() => {
    if (!agentsQuery.isSuccess) return;
    if (!selectedAgentId) return;
    if (agents.some((a) => a.id === selectedAgentId)) return;
    setSearchParam({ agent: undefined });
    // setSearchParam closes over `navigate` only; it's stable across renders.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agentsQuery.isSuccess, selectedAgentId, agents]);

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("memory.cognitive_layer")}
        title={t("memory.title")}
        subtitle={t("memory.subtitle")}
        isFetching={memoryQuery.isFetching || agentsQuery.isFetching}
        onRefresh={() => {
          void memoryQuery.refetch();
          void agentsQuery.refetch();
          void autoDreamQuery.refetch();
        }}
        icon={<Database className="h-4 w-4" />}
        helpText={t("memory.help")}
        actions={
          <div className="flex items-center gap-2">
            <Button variant="secondary" size="sm" onClick={() => setShowConfigDialog(true)}>
              <Settings className="w-4 h-4" />
              <span className="hidden sm:inline ml-1">{t("common.settings", { defaultValue: "Settings" })}</span>
            </Button>
          </div>
        }
      />

      <div className="flex flex-col lg:flex-row gap-6">
        <AgentRail
          agents={agents}
          autoDream={autoDreamQuery.data}
          recordsByAgentId={recordsByAgentId}
          kvCountByAgentId={kvCountByAgentId}
          totalRecords={totalRecords}
          totalKv={totalKv}
          selectedAgentId={selectedAgentId}
          onSelect={(id) => setSearchParam({ agent: id })}
        />

        <div className="flex-1 min-w-0 flex flex-col gap-4">
          <ScopeSummary
            scopedAgent={scopedAgent}
            agentStats={scopedStatsQuery.data}
            autoDream={autoDreamQuery.data}
            dreamForAgent={dreamForAgent}
            kvCount={scopedKvCount}
          />

          <TabBar
            activeTab={activeTab}
            counts={{
              records:
                selectedAgentId
                  ? (recordsByAgentId.get(selectedAgentId) ?? 0)
                  : totalRecords,
              kv: scopedKvCount,
              // Dreams badge counts ENROLLED agents only — total agents would
              // include never-enrolled ones, which inflates the number past
              // what an operator means by "agents currently dreaming".
              dreams:
                selectedAgentId
                  ? (dreamForAgent?.auto_dream_enabled ? 1 : 0)
                  : (autoDreamQuery.data?.agents.filter((a) => a.auto_dream_enabled).length ?? 0),
              health: 0,
            }}
            onSelect={(tab) => setSearchParam({ tab })}
          />

          <div>
            {activeTab === "records" && (
              <RecordsTab
                scopedAgentId={selectedAgentId}
                proactiveEnabled={proactiveEnabled}
                onAdd={() => setShowAddDialog(true)}
                onEdit={(m) => setEditingMemory(m)}
                onDelete={(id) => setDeleteConfirm({ id })}
              />
            )}
            {activeTab === "kv" && (
              <KvTab
                agents={agents}
                scopedAgentId={selectedAgentId}
                kvQueryByAgentId={kvQueryByAgentId}
              />
            )}
            {activeTab === "dreams" && (
              <AutoDreamTab agents={agents} scopedAgentId={selectedAgentId} />
            )}
            {activeTab === "health" && (
              <HealthTab onOpenConfig={() => setShowConfigDialog(true)} />
            )}
          </div>
        </div>
      </div>

      {showAddDialog && <AddMemoryDialog onClose={() => setShowAddDialog(false)} />}
      {editingMemory && (
        <EditMemoryDialog memory={editingMemory} onClose={() => setEditingMemory(null)} />
      )}
      {showConfigDialog && <MemoryConfigDialog onClose={() => setShowConfigDialog(false)} />}

      <ConfirmDialog
        isOpen={deleteConfirm !== null}
        title={t("memory.delete_confirm_title", { defaultValue: "Delete Memory" })}
        message={t("memory.delete_confirm_message", {
          defaultValue: "This memory will be permanently deleted.",
        })}
        tone="destructive"
        confirmLabel={t("common.delete", { defaultValue: "Delete" })}
        onConfirm={() => {
          if (deleteConfirm) {
            deleteMutation.mutate(deleteConfirm.id, {
              onSuccess: () =>
                addToast(
                  t("memory.delete_success", { defaultValue: "Memory deleted" }),
                  "success",
                ),
              onError: (err) =>
                addToast(err instanceof Error ? err.message : t("common.error"), "error"),
            });
          }
          setDeleteConfirm(null);
        }}
        onClose={() => setDeleteConfirm(null)}
      />
    </div>
  );
}

interface TabBarProps {
  activeTab: MemoryTab;
  counts: Record<MemoryTab, number>;
  onSelect: (tab: MemoryTab) => void;
}

function TabBar({ activeTab, counts, onSelect }: TabBarProps) {
  const { t } = useTranslation();
  const tabs: { key: MemoryTab; label: string; icon: React.ReactNode }[] = [
    {
      key: "records",
      label: t("memory.tab_records", { defaultValue: "Records" }),
      icon: <FileText className="w-3.5 h-3.5" />,
    },
    {
      key: "kv",
      label: t("memory.tab_kv", { defaultValue: "KV" }),
      icon: <KeyRound className="w-3.5 h-3.5" />,
    },
    {
      key: "dreams",
      label: t("memory.tab_dreams", { defaultValue: "Auto-Dream" }),
      icon: <Moon className="w-3.5 h-3.5" />,
    },
    {
      key: "health",
      label: t("memory.tab_health", { defaultValue: "Health" }),
      icon: <HeartPulse className="w-3.5 h-3.5" />,
    },
  ];

  return (
    <Card padding="sm">
      <div
        role="tablist"
        aria-label={t("memory.tab_aria_label", { defaultValue: "Memory views" })}
        className="flex gap-1 overflow-x-auto scrollbar-thin"
      >
        {tabs.map((tab) => {
          const active = tab.key === activeTab;
          const count = counts[tab.key];
          return (
            <button
              key={tab.key}
              role="tab"
              aria-selected={active}
              onClick={() => onSelect(tab.key)}
              className={`flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-bold transition-colors shrink-0 ${
                active
                  ? "bg-brand/10 text-brand"
                  : "text-text-dim hover:text-text-main hover:bg-main/50"
              }`}
            >
              {tab.icon}
              {tab.label}
              {tab.key !== "health" && count > 0 && (
                <span
                  className={`text-[10px] font-mono tabular-nums px-1.5 py-0.5 rounded ${
                    active ? "bg-brand/20" : "bg-main/50 text-text-dim"
                  }`}
                >
                  {count}
                </span>
              )}
            </button>
          );
        })}
      </div>
    </Card>
  );
}
