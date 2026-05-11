import { useDeferredValue, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Database, Plus, Search, Trash2, X, Loader2 } from "lucide-react";
import { Card } from "../../../components/ui/Card";
import { CardSkeleton } from "../../../components/ui/Skeleton";
import { EmptyState } from "../../../components/ui/EmptyState";
import { Button } from "../../../components/ui/Button";
import { Input } from "../../../components/ui/Input";
import { useMemorySearchOrList } from "../../../lib/queries/memory";
import { useCleanupMemories } from "../../../lib/mutations/memory";
import { useUIStore } from "../../../lib/store";
import type { MemoryItem } from "../../../api";
import { MemoryRecordCard } from "../components/MemoryRecordCard";

interface Props {
  // When defined, the records list is filtered to just this agent. When
  // undefined, all memories across every agent are shown (the aggregate
  // scope from the "All agents" rail entry).
  scopedAgentId: string | undefined;
  proactiveEnabled: boolean;
  onAdd: () => void;
  onEdit: (memory: MemoryItem) => void;
  onDelete: (id: string) => void;
}

export function RecordsTab({
  scopedAgentId,
  proactiveEnabled,
  onAdd,
  onEdit,
  onDelete,
}: Props) {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);
  const [search, setSearch] = useState("");
  const [levelFilter, setLevelFilter] = useState<string>("all");
  const deferredSearch = useDeferredValue(search);
  const memoryQuery = useMemorySearchOrList(deferredSearch);
  const cleanupMutation = useCleanupMemories();

  const allMemories = memoryQuery.data?.memories ?? [];
  const totalCount = memoryQuery.data?.total ?? 0;

  const scopedMemories = useMemo(() => {
    if (!scopedAgentId) return allMemories;
    return allMemories.filter((m) => m.agent_id === scopedAgentId);
  }, [allMemories, scopedAgentId]);

  const filteredMemories = useMemo(() => {
    if (levelFilter === "all") return scopedMemories;
    return scopedMemories.filter((m) => m.level === levelFilter);
  }, [scopedMemories, levelFilter]);

  const levels = useMemo(
    () => Array.from(new Set(scopedMemories.map((m) => m.level).filter(Boolean))),
    [scopedMemories],
  );

  if (!proactiveEnabled) {
    return (
      <Card padding="md">
        <p className="text-xs text-text-dim">
          {t("memory.proactive_disabled_notice", {
            defaultValue:
              "Proactive memory is disabled in config — no records to show. KV memory and Auto-Dream still work.",
          })}
        </p>
      </Card>
    );
  }

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col sm:flex-row gap-3">
        <div className="flex-1">
          <Input
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder={t("common.search")}
            leftIcon={<Search className="w-4 h-4" />}
            rightIcon={
              search && (
                <button
                  onClick={() => setSearch("")}
                  className="hover:text-text-main"
                  aria-label={t("common.clear_search", { defaultValue: "Clear search" })}
                >
                  <X className="w-3 h-3" />
                </button>
              )
            }
          />
        </div>
        <div className="flex gap-1 p-1 bg-main/30 rounded-lg">
          <button
            onClick={() => setLevelFilter("all")}
            className={`px-3 py-1.5 rounded-md text-xs font-bold transition-colors ${
              levelFilter === "all" ? "bg-surface shadow-sm" : "text-text-dim hover:text-text-main"
            }`}
          >
            {t("memory.filter_all")}
          </button>
          {levels.map((level) => (
            <button
              key={level}
              onClick={() => setLevelFilter(level || "all")}
              className={`px-3 py-1.5 rounded-md text-xs font-bold transition-colors ${
                levelFilter === level ? "bg-surface shadow-sm" : "text-text-dim hover:text-text-main"
              }`}
            >
              {level}
            </button>
          ))}
        </div>
        <div className="flex gap-1">
          <Button
            variant="secondary"
            size="sm"
            onClick={() =>
              cleanupMutation.mutate(undefined, {
                onSuccess: () =>
                  addToast(
                    t("memory.cleanup_success", { defaultValue: "Cleanup complete" }),
                    "success",
                  ),
                onError: (err) =>
                  addToast(err instanceof Error ? err.message : t("common.error"), "error"),
              })
            }
            disabled={cleanupMutation.isPending}
          >
            {cleanupMutation.isPending ? (
              <Loader2 className="w-4 h-4 animate-spin" />
            ) : (
              <Trash2 className="w-4 h-4" />
            )}
            <span className="hidden md:inline ml-1">{t("memory.cleanup")}</span>
          </Button>
          <Button variant="primary" size="sm" onClick={onAdd}>
            <Plus className="w-4 h-4" />
            <span className="hidden md:inline ml-1">{t("memory.add")}</span>
          </Button>
        </div>
      </div>

      <div className="text-xs text-text-dim">
        {t("memory.showing", { count: filteredMemories.length, total: totalCount })}
        {scopedAgentId && (
          <span className="ml-2 text-text-dim/60">
            ({t("memory.records_filtered_to_agent", { defaultValue: "filtered to this agent" })})
          </span>
        )}
      </div>

      {memoryQuery.isLoading ? (
        <div className="grid gap-4">
          {[1, 2, 3, 4, 5].map((i) => (
            <CardSkeleton key={i} />
          ))}
        </div>
      ) : memoryQuery.isError ? (
        <EmptyState
          title={t("common.error")}
          description={t("common.error_loading_data", {
            defaultValue: "Failed to load memories",
          })}
          icon={<Database className="h-6 w-6" />}
        />
      ) : filteredMemories.length === 0 ? (
        <EmptyState
          title={
            search || levelFilter !== "all" || scopedAgentId
              ? t("common.no_data")
              : t("memory.no_memories")
          }
          icon={<Database className="h-6 w-6" />}
        />
      ) : (
        <div className="grid gap-4">
          {filteredMemories.map((m) => (
            <MemoryRecordCard key={m.id} memory={m} onEdit={onEdit} onDelete={onDelete} />
          ))}
        </div>
      )}
    </div>
  );
}
