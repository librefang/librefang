import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { listChannels } from "../api";
import { PageHeader } from "../components/ui/PageHeader";
import { CardSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { Network } from "lucide-react";

const REFRESH_MS = 30000;

export function ChannelsPage() {
  const { t } = useTranslation();
  const channelsQuery = useQuery({ queryKey: ["channels", "list"], queryFn: listChannels, refetchInterval: REFRESH_MS });

  const channels = channelsQuery.data ?? [];
  const activeCount = channels.filter(c => c.configured).length;

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("common.infrastructure")}
        title={t("channels.title")}
        subtitle={t("channels.subtitle")}
        isFetching={channelsQuery.isFetching}
        onRefresh={() => void channelsQuery.refetch()}
        icon={<Network className="h-4 w-4" />}
        actions={
          <div className="rounded-full border border-border-subtle bg-surface px-3 py-1.5 text-[10px] font-bold uppercase text-text-dim">
            {t("channels.configured_count", { count: activeCount })}
          </div>
        }
      />

      {channelsQuery.isLoading ? (
        <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
          {[1, 2, 3].map((i) => <CardSkeleton key={i} />)}
        </div>
      ) : channels.length === 0 ? (
        <EmptyState
          title={t("channels.no_channels")}
          icon={<Network className="h-6 w-6" />}
        />
      ) : (
        <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
          {channels.map(c => (
            <article key={c.name} className="group rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm hover:border-brand/30 transition-all">
              <div className="flex items-center justify-between mb-4">
                <h2 className="text-lg font-black truncate">{c.display_name || c.name}</h2>
                <span className={`px-2 py-0.5 rounded-lg border text-[9px] font-black uppercase ${c.configured ? 'border-success/20 bg-success/10 text-success' : 'border-border-subtle bg-main text-text-dim'}`}>
                  {c.configured ? t("common.online") : t("common.setup")}
                </span>
              </div>
              <p className="text-xs text-text-dim line-clamp-2 italic mb-6">{c.description || "-"}</p>
              <button className="w-full rounded-xl border border-border-subtle bg-surface py-2 text-xs font-black text-text-dim hover:text-brand transition-all">{t("channels.setup_adapter")}</button>
            </article>
          ))}
        </div>
      )}
    </div>
  );
}
