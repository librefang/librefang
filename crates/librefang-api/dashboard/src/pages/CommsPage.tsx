import { formatTime } from "../lib/datetime";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { type CommsEventItem } from "../api";
import { useChannels, useCommsTopology, useCommsEvents } from "../lib/queries/channels";
import { useDashboardSnapshot } from "../lib/queries/overview";
import { PageHeader } from "../components/ui/PageHeader";
import { CardSkeleton, ListSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { Card } from "../components/ui/Card";
import { Badge } from "../components/ui/Badge";
import { Input } from "../components/ui/Input";
import {
  Radio, Activity, Zap, Clock, CheckCircle2, XCircle, MessageSquare, Send,
  Mail, Phone, Link2, Wifi, Globe, ChevronRight, Search, X,
  ArrowsUpFromLine, Users
} from "lucide-react";

// Channel icons
const channelIcons: Record<string, React.ReactNode> = {
  slack: <MessageSquare className="w-5 h-5" />,
  discord: <MessageSquare className="w-5 h-5" />,
  telegram: <Send className="w-5 h-5" />,
  whatsapp: <Phone className="w-5 h-5" />,
  email: <Mail className="w-5 h-5" />,
  sms: <MessageSquare className="w-5 h-5" />,
  webhook: <Link2 className="w-5 h-5" />,
  http: <Globe className="w-5 h-5" />,
  websocket: <Radio className="w-5 h-5" />,
  mqtt: <Wifi className="w-5 h-5" />,
  slack_events: <Activity className="w-5 h-5" />,
  teams: <MessageSquare className="w-5 h-5" />,
};

function getChannelIcon(name: string): React.ReactNode {
  const key = name.toLowerCase().split("-")[0];
  return channelIcons[key] || <Radio className="w-5 h-5" />;
}

// Event Timeline Item
function EventItem({ event }: { event: CommsEventItem }) {
  const getEventIcon = () => {
    if (event.kind?.includes("send")) return <Send className="w-3 h-3" />;
    if (event.kind?.includes("receive")) return <ArrowsUpFromLine className="w-3 h-3" />;
    if (event.kind?.includes("error")) return <XCircle className="w-3 h-3 text-error" />;
    return <Activity className="w-3 h-3" />;
  };

  return (
    <div className="flex items-start gap-3 p-3 hover:bg-surface-hover rounded-lg transition-colors">
      <div className={`w-8 h-8 rounded-lg flex items-center justify-center shrink-0 ${event.kind?.includes("error") ? "bg-error/10" : "bg-primary/10"}`}>
        {getEventIcon()}
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="text-sm font-bold truncate">{event.source_name || event.source_id || "-"}</span>
          <Badge variant={event.kind?.includes("error") ? "error" : "default"} className="shrink-0">
            {event.kind || "event"}
          </Badge>
        </div>
        <p className="text-xs text-text-dim truncate mt-0.5">{event.detail || "-"}</p>
      </div>
      <span className="text-[10px] text-text-dim shrink-0">
        {formatTime(event.timestamp)}
      </span>
    </div>
  );
}

// Topology Node
function TopologyNode({ node, onClick }: { node: { id: string; name?: string; state?: string; model?: string }; onClick?: () => void }) {
  const getNodeColor = () => {
    if (node.state === "Running") return "from-brand to-brand/60";
    if (node.state === "Idle") return "from-success to-success/60";
    if (node.state === "Stopped") return "from-warning to-warning/60";
    return "from-text-dim to-text-dim/60";
  };

  return (
    <div
      onClick={onClick}
      className="flex flex-col items-center gap-2 p-4 rounded-xl bg-surface border border-border-subtle hover:border-primary transition-colors cursor-pointer"
    >
      <div className={`w-12 h-12 rounded-xl bg-linear-to-br ${getNodeColor()} flex items-center justify-center shadow-lg`}>
        <Users className="w-6 h-6 text-white" />
      </div>
      <div className="text-center">
        <p className="text-xs font-bold truncate max-w-[80px]">{node.name || node.id}</p>
        <p className="text-[10px] text-text-dim uppercase">{node.model || "agent"}</p>
      </div>
      <div className={`w-2 h-2 rounded-full ${node.state === "Running" ? "bg-success" : node.state === "Idle" ? "bg-warning" : "bg-text-dim/30"}`} />
    </div>
  );
}

export function CommsPage() {
  const { t } = useTranslation();
  const [activeTab, setActiveTab] = useState<"channels" | "topology" | "events">("channels");
  const [search, setSearch] = useState("");

  const channelsQuery = useChannels();

  const snapshotQuery = useDashboardSnapshot();

  const topologyQuery = useCommsTopology();

  const eventsQuery = useCommsEvents(50, {
    enabled: activeTab === "events",
    refetchInterval: 5_000,
  });

  const channels = channelsQuery.data ?? [];
  const snapshot = snapshotQuery.data ?? null;
  const topology = topologyQuery.data ?? null;
  const events = eventsQuery.data ?? [];
  const isLoading = channelsQuery.isLoading || snapshotQuery.isLoading;

  const configuredCount = useMemo(() => channels.filter(c => c.configured).length, [channels]);

  const filteredChannels = useMemo(
    () => channels
      .filter(c => !search || (c.display_name || c.name).toLowerCase().includes(search.toLowerCase()))
      .sort((a, b) => {
        // Configured first
        if (a.configured && !b.configured) return -1;
        if (!a.configured && b.configured) return 1;
        return (a.display_name || a.name).localeCompare(b.display_name || b.name);
      }),
    [channels, search],
  );

  const filteredEvents = useMemo(
    () => events.filter(e =>
      !search || (e.source_name || e.source_id || "").toLowerCase().includes(search.toLowerCase()) ||
      (e.detail || "").toLowerCase().includes(search.toLowerCase())
    ),
    [events, search],
  );

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("comms.bus")}
        title={t("nav.comms")}
        subtitle={t("comms.subtitle")}
        isFetching={isLoading}
        onRefresh={() => {
          void channelsQuery.refetch();
          void snapshotQuery.refetch();
          void topologyQuery.refetch();
          void eventsQuery.refetch();
        }}
        icon={<Radio className="h-4 w-4" />}
        helpText={t("comms.help")}
        actions={
          <div className="flex items-center gap-3">
            <div className="flex items-center gap-2 px-3 py-1.5 rounded-full bg-success/10 border border-success/20">
              <span className="w-2 h-2 rounded-full bg-success animate-pulse" />
              <span className="text-[10px] font-bold text-success uppercase">{t("common.online")}</span>
            </div>
            <div className="text-xs text-text-dim">
              {configuredCount} / {channels.length} {t("channels.configured")}
            </div>
          </div>
        }
      />

      {/* Tabs */}
      <div className="flex gap-1 p-1 bg-main/30 rounded-xl w-fit overflow-x-auto">
        <button
          onClick={() => setActiveTab("channels")}
          className={`flex items-center gap-1.5 sm:gap-2 px-3 sm:px-4 py-2 rounded-lg text-xs sm:text-sm font-bold transition-colors whitespace-nowrap ${
            activeTab === "channels" ? "bg-surface text-primary shadow-sm" : "text-text-dim hover:text-text-main"
          }`}
        >
          <Radio className="w-4 h-4" />
          {t("comms.active_channels")}
        </button>
        <button
          onClick={() => setActiveTab("topology")}
          className={`flex items-center gap-1.5 sm:gap-2 px-3 sm:px-4 py-2 rounded-lg text-xs sm:text-sm font-bold transition-colors whitespace-nowrap ${
            activeTab === "topology" ? "bg-surface text-primary shadow-sm" : "text-text-dim hover:text-text-main"
          }`}
        >
          <Zap className="w-4 h-4" />
          {t("comms.topology")}
        </button>
        <button
          onClick={() => setActiveTab("events")}
          className={`flex items-center gap-1.5 sm:gap-2 px-3 sm:px-4 py-2 rounded-lg text-xs sm:text-sm font-bold transition-colors whitespace-nowrap ${
            activeTab === "events" ? "bg-surface text-primary shadow-sm" : "text-text-dim hover:text-text-main"
          }`}
        >
          <Activity className="w-4 h-4" />
          {t("comms.events")}
        </button>
      </div>

      {/* Search */}
      <Input
        value={search}
        onChange={(e) => setSearch(e.target.value)}
        placeholder={t("common.search")}
        leftIcon={<Search className="w-4 h-4" />}
        rightIcon={search && (
          <button onClick={() => setSearch("")} className="hover:text-text-main" aria-label={t("common.clear_search", { defaultValue: "Clear search" })}>
            <X className="w-3 h-3" />
          </button>
        )}
        className="max-w-md"
      />

      {/* Content */}
      {activeTab === "channels" && (
        <>
          {/* Stats Grid */}
          <div className="grid grid-cols-2 gap-2 sm:gap-4 md:grid-cols-4 stagger-children">
            {[
              { icon: Radio, label: t("comms.total_channels"), value: channels.length, color: "text-primary", bg: "bg-primary/10" },
              { icon: CheckCircle2, label: t("comms.connected"), value: configuredCount, color: "text-success", bg: "bg-success/10" },
              { icon: Activity, label: t("comms.events_today"), value: events.length, color: "text-warning", bg: "bg-warning/10" },
              { icon: Clock, label: t("comms.uptime"), value: "99.9%", color: "text-accent", bg: "bg-accent/10" },
            ].map((kpi, i) => (
              <Card key={i} hover padding="md">
                <div className="flex items-center justify-between">
                  <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{kpi.label}</span>
                  <div className={`w-8 h-8 rounded-lg ${kpi.bg} flex items-center justify-center`}><kpi.icon className={`w-4 h-4 ${kpi.color}`} /></div>
                </div>
                <p className={`text-3xl font-black tracking-tight mt-2 ${kpi.color}`}>{kpi.value}</p>
              </Card>
            ))}
          </div>

          {/* Health Checks */}
          <Card padding="lg">
            <h2 className="text-lg font-black tracking-tight mb-1">{t("overview.system_status")}</h2>
            <p className="mb-6 text-xs text-text-dim font-medium">{t("comms.health_description")}</p>

            <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
              {snapshot?.health.checks?.map((check, i) => (
                <div key={i} className="flex items-center justify-between p-4 rounded-xl bg-main/40 border border-border-subtle/50">
                  <div className="flex items-center gap-3">
                    <div className={`w-2 h-2 rounded-full ${check.status === 'ok' ? 'bg-success' : 'bg-error'}`} />
                    <span className="text-sm font-bold">{check.name}</span>
                  </div>
                  <Badge variant={check.status === 'ok' ? "success" : "error"}>
                    {check.status === 'ok' ? t("common.ok") : t("common.error")}
                  </Badge>
                </div>
              ))}
              {(!snapshot?.health?.checks || snapshot.health.checks.length === 0) && (
                <div className="flex items-center gap-2 py-4 col-span-full justify-center">
                  <div className="w-2 h-2 rounded-full bg-success" />
                  <p className="text-xs text-text-dim">{snapshot?.health?.status === "ok" ? t("common.daemon_online") : t("common.no_data")}</p>
                </div>
              )}
            </div>
          </Card>

          {/* Channels Grid */}
          {isLoading ? (
            <div className="grid gap-4 sm:grid-cols-2 md:grid-cols-3 xl:grid-cols-4 2xl:grid-cols-5 3xl:grid-cols-6">
              {[1, 2, 3, 4, 5, 6].map(i => <CardSkeleton key={i} />)}
            </div>
          ) : filteredChannels.length === 0 ? (
            <EmptyState title={t("common.no_data")} icon={<Radio className="h-6 w-6" />} />
          ) : (
            <div className="grid gap-4 sm:grid-cols-2 md:grid-cols-3 xl:grid-cols-4 2xl:grid-cols-5 3xl:grid-cols-6">
              {filteredChannels.map((c) => (
                <Card key={c.name} hover padding="md">
                  <div className="flex items-center justify-between mb-3">
                    <div className="flex items-center gap-3">
                      <div className={`w-10 h-10 rounded-xl flex items-center justify-center ${c.configured ? "bg-success/10 border border-success/20" : "bg-primary/10 border border-primary/20"}`}>
                        {getChannelIcon(c.name)}
                      </div>
                      <div>
                        <h3 className="text-sm font-bold">{c.display_name || c.name}</h3>
                        <p className="text-[10px] text-text-dim">{c.category || c.name}</p>
                      </div>
                    </div>
                    <Badge variant={c.configured ? "success" : "warning"}>
                      {c.configured ? t("common.online") : t("common.setup")}
                    </Badge>
                  </div>
                  {c.description && (
                    <p className="text-xs text-text-dim line-clamp-2">{c.description}</p>
                  )}
                </Card>
              ))}
            </div>
          )}
        </>
      )}

      {/* Topology Tab */}
      {activeTab === "topology" && (
        <Card padding="lg">
          <h2 className="text-lg font-black tracking-tight mb-1">{t("comms.topology")}</h2>
          <p className="mb-6 text-xs text-text-dim font-medium">{t("comms.topology_description")}</p>

          {topologyQuery.isLoading ? (
            <div className="py-12 text-center">
              <div className="w-8 h-8 border-2 border-primary border-t-transparent rounded-full animate-spin mx-auto" />
            </div>
          ) : topology ? (
            <div className="space-y-6">
              {/* Nodes */}
              <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-6 gap-3 sm:gap-4">
                {topology.nodes?.map((node, i) => (
                  <TopologyNode key={node.id || i} node={node} />
                ))}
                {(!topology.nodes || topology.nodes.length === 0) && (
                  <p className="col-span-full text-center text-text-dim py-8">{t("common.no_data")}</p>
                )}
              </div>

              {/* Connections */}
              {topology.edges && topology.edges.length > 0 && (
                <div className="border-t border-border-subtle pt-6">
                  <h3 className="text-sm font-bold mb-4">{t("comms.connections")}</h3>
                  <div className="space-y-2">
                    {topology.edges.map((conn, i) => (
                      <div key={i} className="flex items-center gap-3 p-3 rounded-lg bg-main/30">
                        <span className="text-xs font-bold text-primary">{conn.from || "-"}</span>
                        <ChevronRight className="w-4 h-4 text-text-dim" />
                        <span className="text-xs font-bold text-success">{conn.to || "-"}</span>
                        <Badge variant="default" className="ml-auto">
                          {conn.kind || "link"}
                        </Badge>
                      </div>
                    ))}
                  </div>
                </div>
              )}
            </div>
          ) : (
            <div className="py-12 text-center text-text-dim">
              <Zap className="w-12 h-12 mx-auto mb-4 opacity-30" />
              <p>{t("comms.topology_description")}</p>
            </div>
          )}
        </Card>
      )}

      {/* Events Tab */}
      {activeTab === "events" && (
        <Card padding="lg">
          <div className="flex items-center justify-between mb-4">
            <div>
              <h2 className="text-lg font-black tracking-tight">{t("comms.events")}</h2>
              <p className="text-xs text-text-dim">{t("comms.events_desc")}</p>
            </div>
            <div className="flex items-center gap-2">
              <span className={`w-2 h-2 rounded-full ${eventsQuery.isFetching ? "bg-warning animate-pulse" : "bg-success"}`} />
              <span className="text-xs text-text-dim">{eventsQuery.isFetching ? t("common.loading") : t("common.online")}</span>
            </div>
          </div>

          {eventsQuery.isLoading ? (
            <ListSkeleton rows={5} />
          ) : filteredEvents.length === 0 ? (
            <EmptyState title={t("common.no_data")} icon={<Activity className="h-6 w-6" />} />
          ) : (
            <div className="space-y-1 max-h-[60vh] overflow-y-auto">
              {filteredEvents.map((event, i) => (
                <EventItem key={event.id || i} event={event} />
              ))}
            </div>
          )}
        </Card>
      )}
    </div>
  );
}
