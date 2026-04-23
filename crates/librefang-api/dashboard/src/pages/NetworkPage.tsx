import { useCallback } from "react";
import { formatDateTime } from "../lib/datetime";
import { useTranslation } from "react-i18next";
import { useNetworkStatus, usePeers } from "../lib/queries/network";
import { PageHeader } from "../components/ui/PageHeader";
import { Card } from "../components/ui/Card";
import { Badge } from "../components/ui/Badge";
import { EmptyState } from "../components/ui/EmptyState";
import { CardSkeleton } from "../components/ui/Skeleton";
import { Network, Globe, Server, Wifi, WifiOff, Hash, Clock } from "lucide-react";

export function NetworkPage() {
  const { t } = useTranslation();

  const statusQuery = useNetworkStatus();
  const peersQuery = usePeers();

  const status = statusQuery.data;
  const peers = peersQuery.data ?? [];
  const isLoading = statusQuery.isPending || peersQuery.isPending;

  const handleRefresh = useCallback(() => {
    void Promise.all([statusQuery.refetch(), peersQuery.refetch()]);
  }, [statusQuery, peersQuery]);

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("network.section")}
        title={t("network.title")}
        subtitle={t("network.subtitle")}
        isFetching={statusQuery.isFetching || peersQuery.isFetching}
        onRefresh={handleRefresh}
        icon={<Network className="h-4 w-4" />}
        helpText={t("network.help")}
      />

      {isLoading ? (
        <div className="grid gap-4 md:grid-cols-3">
          <CardSkeleton />
          <CardSkeleton />
          <CardSkeleton />
        </div>
      ) : (
        <>
          {/* Network status cards */}
          <div className="grid gap-2 sm:gap-4 md:grid-cols-3 stagger-children">
            <Card hover padding="md">
              <div className="flex items-center justify-between">
                <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">
                  {t("network.status_label")}
                </span>
                <div className={`w-8 h-8 rounded-lg flex items-center justify-center ${status?.online ? "bg-success/10" : "bg-error/10"}`}>
                  {status?.online ? (
                    <Wifi className="w-4 h-4 text-success" />
                  ) : (
                    <WifiOff className="w-4 h-4 text-error" />
                  )}
                </div>
              </div>
              <div className="mt-2 flex items-center gap-2">
                <Badge variant={status?.online ? "success" : "error"} dot>
                  {status?.online ? t("common.online") : t("common.offline")}
                </Badge>
              </div>
            </Card>

            <Card hover padding="md">
              <div className="flex items-center justify-between">
                <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">
                  {t("network.node_id")}
                </span>
                <div className="w-8 h-8 rounded-lg bg-primary/10 flex items-center justify-center">
                  <Hash className="w-4 h-4 text-primary" />
                </div>
              </div>
              <p className="text-sm font-mono font-bold mt-2 truncate" title={status?.node_id}>
                {status?.node_id || "-"}
              </p>
            </Card>

            <Card hover padding="md">
              <div className="flex items-center justify-between">
                <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">
                  {t("network.protocol")}
                </span>
                <div className="w-8 h-8 rounded-lg bg-info/10 flex items-center justify-center">
                  <Globe className="w-4 h-4 text-info" />
                </div>
              </div>
              <p className="text-lg font-black mt-2">{status?.protocol_version || "-"}</p>
              {status?.listen_addr ? (
                <p className="text-[10px] text-text-dim font-mono mt-1">{status.listen_addr}</p>
              ) : null}
            </Card>
          </div>

          {/* Peers list */}
          <div>
            <div className="flex items-center justify-between mb-4">
              <h2 className="text-lg font-black tracking-tight">{t("network.peers")}</h2>
              <Badge variant="brand">{peers.length} {t("network.connected")}</Badge>
            </div>

            {peers.length === 0 ? (
              <EmptyState
                icon={<Server className="h-8 w-8" />}
                title={t("network.no_peers")}
                description={t("network.no_peers_desc")}
              />
            ) : (
              <div className="grid gap-2 sm:gap-3 md:grid-cols-2 stagger-children">
                {peers.map((peer) => (
                  <Card key={peer.id} hover padding="md">
                    <div className="flex items-start justify-between">
                      <div className="flex items-center gap-3">
                        <div className="w-10 h-10 rounded-xl bg-linear-to-br from-brand/20 to-accent/20 flex items-center justify-center">
                          <Server className="w-5 h-5 text-primary" />
                        </div>
                        <div className="min-w-0">
                          <p className="text-sm font-bold truncate">{peer.name || peer.id}</p>
                          {peer.addr ? (
                            <p className="text-[10px] text-text-dim font-mono">{peer.addr}</p>
                          ) : null}
                        </div>
                      </div>
                      <Badge
                        variant={peer.status === "connected" ? "success" : "warning"}
                        dot
                      >
                        {peer.status || t("common.unknown")}
                      </Badge>
                    </div>
                    {(peer.version || peer.last_seen) ? (
                      <div className="flex items-center gap-3 mt-3 text-[10px] text-text-dim">
                        {peer.version ? (
                          <span className="flex items-center gap-1">
                            <Globe className="w-3 h-3" /> v{peer.version}
                          </span>
                        ) : null}
                        {peer.last_seen ? (
                          <span className="flex items-center gap-1">
                            <Clock className="w-3 h-3" /> {formatDateTime(peer.last_seen)}
                          </span>
                        ) : null}
                      </div>
                    ) : null}
                  </Card>
                ))}
              </div>
            )}
          </div>
        </>
      )}
    </div>
  );
}
