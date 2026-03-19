import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { listChannels } from "../api";
import { PageHeader } from "../components/ui/PageHeader";
import { CardSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
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
            <Card key={c.name} hover padding="lg">
              <div className="flex items-center justify-between mb-4">
                <h2 className="text-lg font-black truncate">{c.display_name || c.name}</h2>
                <Badge variant={c.configured ? "success" : "default"}>
                  {c.configured ? t("common.online") : t("common.setup")}
                </Badge>
              </div>
              <p className="text-xs text-text-dim line-clamp-2 italic mb-6">{c.description || "-"}</p>
              <Button variant="secondary" className="w-full">{t("channels.setup_adapter")}</Button>
            </Card>
          ))}
        </div>
      )}
    </div>
  );
}
