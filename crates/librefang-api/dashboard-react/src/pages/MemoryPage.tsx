import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { listMemories } from "../api";
import { PageHeader } from "../components/ui/PageHeader";
import { ListSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { Database } from "lucide-react";

const REFRESH_MS = 30000;

export function MemoryPage() {
  const { t } = useTranslation();
  const memoryQuery = useQuery({ queryKey: ["memory", "list"], queryFn: () => listMemories(), refetchInterval: REFRESH_MS });

  const memories = memoryQuery.data?.memories ?? [];
  const totalCount = memoryQuery.data?.total ?? 0;

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("memory.cognitive_layer")}
        title={t("memory.title")}
        subtitle={t("memory.subtitle")}
        isFetching={memoryQuery.isFetching}
        onRefresh={() => void memoryQuery.refetch()}
        icon={<Database className="h-4 w-4" />}
        actions={
          <div className="rounded-full border border-border-subtle bg-surface px-3 py-1.5 text-[10px] font-bold uppercase text-text-dim">
            {t("memory.objects_stored", { count: totalCount })}
          </div>
        }
      />

      {memoryQuery.isLoading ? (
        <ListSkeleton rows={5} />
      ) : memories.length === 0 ? (
        <EmptyState
          title={t("common.no_data")}
          icon={<Database className="h-6 w-6" />}
        />
      ) : (
        <div className="grid gap-4">
          {memories.map((m) => (
            <article key={m.id} className="group rounded-2xl border border-border-subtle bg-surface p-5 shadow-sm transition-all hover:border-brand/30">
              <div className="flex items-center gap-2 mb-1">
                <h2 className="text-sm font-black truncate">{m.id}</h2>
                <span className="rounded-lg bg-brand/10 border border-brand/10 px-2 py-0.5 text-[9px] font-black text-brand uppercase">{m.level || "Vector"}</span>
              </div>
              <p className="text-xs text-text-dim line-clamp-2 leading-relaxed">{m.content || t("common.no_data")}</p>
            </article>
          ))}
        </div>
      )}
    </div>
  );
}
