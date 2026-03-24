import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "@tanstack/react-router";
import { router } from "../router";
import {
  activateHand,
  deactivateHand,
  listActiveHands,
  listHands,
  pauseHand,
  resumeHand,
  getHandStats,
  getHandSettings,
  type HandDefinitionItem,
  type HandInstanceItem,
  type HandStatsResponse,
  type HandSettingsResponse,
} from "../api";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { useUIStore } from "../lib/store";
import { Input } from "../components/ui/Input";
import {
  Hand,
  Search,
  Power,
  PowerOff,
  Loader2,
  X,
  Pause,
  Play,
  BarChart3,
  Settings,
  CheckCircle2,
  XCircle,
  Wrench,
  Activity,
  MessageCircle,
  ChevronRight,
} from "lucide-react";
import { PageHeader } from "../components/ui/PageHeader";
import { ListSkeleton } from "../components/ui/Skeleton";

const REFRESH_MS = 15000;

/* ── Inject slideInRight keyframes once at module level ──── */
if (typeof document !== "undefined" && !document.getElementById("hands-keyframes")) {
  const style = document.createElement("style");
  style.id = "hands-keyframes";
  style.textContent = `
    @keyframes slideInRight {
      from { transform: translateX(100%); opacity: 0; }
      to   { transform: translateX(0);    opacity: 1; }
    }
  `;
  document.head.appendChild(style);
}


/* ── Inline metrics for active hand cards ─────────────────── */

function HandMetricsInline({ metrics }: { metrics?: Record<string, { value?: unknown; format?: string }> }) {
  if (!metrics || Object.keys(metrics).length === 0) return null;

  const entries = Object.entries(metrics).slice(0, 3);

  return (
    <div className="flex flex-wrap gap-x-3 gap-y-1 mt-1.5">
      {entries.map(([label, m]) => (
        <span key={label} className="text-[9px] text-text-dim/70 font-mono">
          <span className="text-text-dim/40">{label}:</span>{" "}
          <span className="text-brand/80">{String(m.value ?? "-")}</span>
        </span>
      ))}
    </div>
  );
}

/* ── Detail side panel ───────────────────────────────────── */

function HandDetailPanel({
  hand,
  instance,
  isActive,
  onClose,
  onActivate,
  onDeactivate,
  onPause,
  onResume,
  onChat,
  isPending,
}: {
  hand: HandDefinitionItem;
  instance: HandInstanceItem | undefined;
  isActive: boolean;
  onClose: () => void;
  onActivate: (id: string) => void;
  onDeactivate: (id: string) => void;
  onPause: (id: string) => void;
  onResume: (id: string) => void;
  onChat: (instanceId: string, handName: string) => void;
  isPending: boolean;
}) {
  const { t } = useTranslation();
  const isPaused = instance?.status === "paused";

  const settingsQuery = useQuery({
    queryKey: ["hands", "settings", hand.id],
    queryFn: () => getHandSettings(hand.id),
    enabled: !!hand.id,
  });

  const statsQuery = useQuery({
    queryKey: ["hands", "stats", instance?.instance_id],
    queryFn: () => getHandStats(instance!.instance_id),
    refetchInterval: REFRESH_MS,
    enabled: !!instance?.instance_id,
  });

  const settings: HandSettingsResponse = settingsQuery.data ?? {};
  const stats: HandStatsResponse = statsQuery.data ?? {};

  return (
    <div
      className="fixed inset-0 z-50 flex justify-end bg-black/30 backdrop-blur-sm"
      onClick={onClose}
    >
      <div
        className="bg-surface w-full sm:w-[480px] h-full shadow-2xl border-l border-border-subtle flex flex-col animate-slide-in-right"
        onClick={(e) => e.stopPropagation()}
        style={{ animation: "slideInRight 0.25s ease-out" }}
      >
        {/* Header */}
        <div className="px-6 py-5 border-b border-border-subtle shrink-0">
          <div className="flex items-start justify-between">
            <div className="flex items-center gap-3 min-w-0">
              <div
                className={`w-12 h-12 rounded-2xl flex items-center justify-center shrink-0 ${
                  isActive
                    ? isPaused
                      ? "bg-warning/15 text-warning"
                      : "bg-success/15 text-success"
                    : "bg-brand/10 text-brand"
                }`}
              >
                <Hand className="w-6 h-6" />
              </div>
              <div className="min-w-0">
                <h2 className="text-lg font-bold truncate">{hand.name || hand.id}</h2>
                <div className="flex items-center gap-1.5 mt-1 flex-wrap">
                  {isActive ? (
                    isPaused ? (
                      <Badge variant="warning" dot>{t("hands.paused")}</Badge>
                    ) : (
                      <Badge variant="success" dot>{t("hands.active_label")}</Badge>
                    )
                  ) : hand.requirements_met ? (
                    <Badge variant="default">{t("hands.ready")}</Badge>
                  ) : (
                    <Badge variant="warning">{t("hands.missing_req")}</Badge>
                  )}
                  {hand.category && (
                    <Badge variant="info">
                      {t(`hands.cat_${hand.category}`, { defaultValue: hand.category })}
                    </Badge>
                  )}
                </div>
              </div>
            </div>
            <button
              onClick={onClose}
              className="p-2 -mr-2 rounded-xl text-text-dim hover:text-text hover:bg-main transition-colors"
            >
              <X className="w-5 h-5" />
            </button>
          </div>

          {/* Action buttons */}
          <div className="flex items-center gap-2 mt-4">
            {isActive ? (
              <>
                <Button
                  variant="primary"
                  size="sm"
                  onClick={() =>
                    instance && onChat(instance.instance_id, hand.name || hand.id)
                  }
                  disabled={isPaused}
                  className="flex-1"
                >
                  <MessageCircle className="w-3.5 h-3.5 mr-1.5" />
                  {t("chat.title")}
                </Button>
                {isPaused ? (
                  <Button
                    variant="success"
                    size="sm"
                    onClick={() => instance && onResume(instance.instance_id)}
                    disabled={isPending}
                    title={t("hands.resume")}
                  >
                    {isPending ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Play className="w-3.5 h-3.5" />}
                  </Button>
                ) : (
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={() => instance && onPause(instance.instance_id)}
                    disabled={isPending}
                    title={t("hands.pause")}
                  >
                    {isPending ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Pause className="w-3.5 h-3.5" />}
                  </Button>
                )}
                <Button
                  variant="danger"
                  size="sm"
                  onClick={() => instance && onDeactivate(instance.instance_id)}
                  disabled={isPending}
                  title={t("hands.deactivate")}
                >
                  {isPending ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <PowerOff className="w-3.5 h-3.5" />}
                </Button>
              </>
            ) : (
              <Button
                variant="primary"
                size="sm"
                onClick={() => onActivate(hand.id)}
                disabled={isPending || !hand.requirements_met}
                className="flex-1"
              >
                {isPending ? (
                  <Loader2 className="w-3.5 h-3.5 animate-spin mr-1.5" />
                ) : (
                  <Power className="w-3.5 h-3.5 mr-1.5" />
                )}
                {t("hands.activate")}
              </Button>
            )}
          </div>
        </div>

        {/* Scrollable content */}
        <div className="flex-1 overflow-y-auto scrollbar-thin">
          <div className="px-6 py-5 space-y-6">
            {/* Description */}
            {hand.description && (
              <div>
                <p className="text-sm text-text-dim leading-relaxed">
                  {hand.description}
                </p>
              </div>
            )}

            {/* Instance info */}
            {instance && (
              <div className="p-3 rounded-xl bg-main/50 border border-border-subtle space-y-1.5">
                <p className="text-[10px] text-text-dim/60 font-mono">
                  {t("hands.instance")}: {instance.instance_id?.slice(0, 16)}
                </p>
                {instance.agent_name && (
                  <p className="text-[10px] text-text-dim/60">
                    {t("hands.agent")}: {instance.agent_name}
                  </p>
                )}
                {instance.activated_at && (
                  <p className="text-[10px] text-text-dim/60">
                    {t("hands.activated_at")}:{" "}
                    {new Date(instance.activated_at).toLocaleString()}
                  </p>
                )}
              </div>
            )}

            {/* Metrics */}
            {isActive && (
              <div>
                <div className="flex items-center gap-2 mb-3">
                  <BarChart3 className="w-4 h-4 text-brand/60" />
                  <span className="text-xs font-bold">{t("hands.metrics")}</span>
                </div>
                {statsQuery.isLoading ? (
                  <div className="flex items-center gap-2 text-text-dim/50 text-[10px]">
                    <Loader2 className="w-3 h-3 animate-spin" />
                    {t("common.loading")}
                  </div>
                ) : stats.metrics && Object.keys(stats.metrics).length > 0 ? (
                  <div className="grid grid-cols-2 gap-2">
                    {Object.entries(stats.metrics).map(([label, m]) => (
                      <div
                        key={label}
                        className="p-3 rounded-xl bg-main border border-border-subtle"
                      >
                        <p className="text-[10px] text-text-dim/60 truncate">{label}</p>
                        <p className="text-base font-bold text-brand mt-0.5">
                          {String(m.value ?? "-")}
                        </p>
                        {m.format && (
                          <p className="text-[8px] text-text-dim/40">{m.format}</p>
                        )}
                      </div>
                    ))}
                  </div>
                ) : (
                  <p className="text-[10px] text-text-dim/50">
                    {t("hands.metrics_no_data")}
                  </p>
                )}
              </div>
            )}

            {/* Settings */}
            <div>
              <div className="flex items-center gap-2 mb-3">
                <Settings className="w-4 h-4 text-text-dim/60" />
                <span className="text-xs font-bold">{t("hands.settings")}</span>
                {settings.settings && settings.settings.length > 0 && (
                  <span className="text-[10px] text-text-dim/40">{settings.settings.length}</span>
                )}
              </div>
              {settingsQuery.isLoading ? (
                <div className="flex items-center gap-2 text-text-dim/50 text-[10px]">
                  <Loader2 className="w-3 h-3 animate-spin" />
                  {t("common.loading")}
                </div>
              ) : settings.settings && settings.settings.length > 0 ? (
                <div className="rounded-xl border border-border-subtle overflow-hidden divide-y divide-border-subtle/50">
                  {settings.settings.map((s) => {
                    const currentVal = settings.current_values?.[s.key ?? ""];
                    const displayVal = currentVal !== undefined ? String(currentVal) : (s.default !== undefined ? String(s.default) : undefined);
                    const isDefault = currentVal === undefined;
                    return (
                      <div key={s.key} className="px-3.5 py-2.5 bg-main/30 hover:bg-main/60 transition-colors">
                        <div className="flex items-start justify-between gap-3">
                          <div className="min-w-0 flex-1">
                            <span className="text-[11px] font-semibold block">
                              {s.label || s.key}
                            </span>
                            {s.description && (
                              <p className="text-[10px] text-text-dim/45 mt-0.5 leading-snug">
                                {s.description}
                              </p>
                            )}
                          </div>
                          {displayVal !== undefined && (
                            <span className={`text-[10px] font-mono shrink-0 px-2 py-0.5 rounded-md mt-0.5 ${
                              isDefault
                                ? "text-text-dim/50 bg-main"
                                : "text-brand bg-brand/8"
                            }`}>
                              {displayVal || <span className="text-text-dim/30 italic">-</span>}
                            </span>
                          )}
                        </div>
                      </div>
                    );
                  })}
                </div>
              ) : (
                <p className="text-[10px] text-text-dim/50">
                  {t("hands.settings_empty")}
                </p>
              )}
            </div>

            {/* Requirements */}
            {hand.requirements && hand.requirements.length > 0 && (
              <div>
                <div className="flex items-center gap-2 mb-3">
                  <CheckCircle2 className="w-4 h-4 text-text-dim/60" />
                  <span className="text-xs font-bold">
                    {t("hands.requirements")}
                  </span>
                </div>
                <div className="space-y-1.5">
                  {hand.requirements.map((r) => (
                    <div
                      key={r.key}
                      className="flex items-center gap-2 text-[11px]"
                    >
                      {r.satisfied ? (
                        <CheckCircle2 className="w-3.5 h-3.5 text-success shrink-0" />
                      ) : (
                        <XCircle className="w-3.5 h-3.5 text-error shrink-0" />
                      )}
                      <span className={r.satisfied ? "text-text-dim" : "text-error"}>
                        {r.label || r.key}
                      </span>
                      {r.optional && (
                        <span className="text-[9px] text-text-dim/40">(optional)</span>
                      )}
                    </div>
                  ))}
                </div>
              </div>
            )}

            {/* Tools */}
            {hand.tools && hand.tools.length > 0 && (
              <div>
                <div className="flex items-center gap-2 mb-3">
                  <Wrench className="w-4 h-4 text-text-dim/60" />
                  <span className="text-xs font-bold">{t("hands.tools")}</span>
                  <span className="text-[10px] text-text-dim/40">{hand.tools.length}</span>
                </div>
                <div className="grid grid-cols-2 gap-1.5">
                  {hand.tools.map((tool) => (
                    <div
                      key={tool}
                      className="flex items-center gap-2 px-2.5 py-2 rounded-lg bg-main/50 border border-border-subtle/50"
                    >
                      <div className="w-1.5 h-1.5 rounded-full bg-brand/40 shrink-0" />
                      <span className="text-[10px] font-mono text-text-dim truncate">{tool}</span>
                    </div>
                  ))}
                </div>
              </div>
            )}
          </div>
        </div>
      </div>

    </div>
  );
}

/* ── Active hand card (horizontal strip) ─────────────────── */

function ActiveHandCard({
  hand,
  instance,
  onChat,
  onDeactivate,
  onDetail,
  isPending,
  metrics,
}: {
  hand: HandDefinitionItem;
  instance: HandInstanceItem;
  onChat: (instanceId: string, handName: string) => void;
  onDeactivate: (id: string) => void;
  onDetail: (hand: HandDefinitionItem) => void;
  isPending: boolean;
  metrics?: Record<string, { value?: unknown; format?: string }>;
}) {
  const { t } = useTranslation();
  const isPaused = instance.status === "paused";

  return (
    <div
      className={`group relative flex items-center gap-3 px-4 py-3 rounded-2xl border cursor-pointer transition-all shrink-0 min-w-[240px] max-w-[320px] ${
        isPaused
          ? "border-warning/30 bg-warning/5 hover:border-warning/50"
          : "border-success/30 bg-success/5 hover:border-success/50"
      }`}
      onClick={() => onDetail(hand)}
    >
      <div
        className={`w-10 h-10 rounded-xl flex items-center justify-center shrink-0 ${
          isPaused
            ? "bg-warning/20 text-warning"
            : "bg-success/20 text-success"
        }`}
      >
        <Hand className="w-5 h-5" />
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <h4 className="text-sm font-bold truncate">{hand.name || hand.id}</h4>
          {isPaused && (
            <span className="w-1.5 h-1.5 rounded-full bg-warning shrink-0" />
          )}
          {!isPaused && (
            <span className="w-1.5 h-1.5 rounded-full bg-success animate-pulse shrink-0" />
          )}
        </div>
        {instance.instance_id && (
          <HandMetricsInline metrics={metrics} />
        )}
      </div>
      <div
        className="flex items-center gap-1 opacity-0 group-hover:opacity-100 transition-opacity shrink-0"
        onClick={(e) => e.stopPropagation()}
      >
        {!isPaused && (
          <button
            onClick={() => onChat(instance.instance_id, hand.name || hand.id)}
            className="p-1.5 rounded-lg text-brand hover:bg-brand/10 transition-colors"
            title={t("chat.title")}
          >
            <MessageCircle className="w-4 h-4" />
          </button>
        )}
        <button
          onClick={() => onDeactivate(instance.instance_id)}
          disabled={isPending}
          className="p-1.5 rounded-lg text-text-dim hover:text-error hover:bg-error/10 transition-colors disabled:opacity-40"
          title={t("hands.deactivate")}
        >
          {isPending ? <Loader2 className="w-4 h-4 animate-spin" /> : <PowerOff className="w-4 h-4" />}
        </button>
      </div>
    </div>
  );
}

/* ── Hand card (grid item) ───────────────────────────────── */

function HandCard({
  hand,
  instance,
  isActive,
  onActivate,
  onDeactivate,
  onDetail,
  onChat,
  isPending,
}: {
  hand: HandDefinitionItem;
  instance: HandInstanceItem | undefined;
  isActive: boolean;
  onActivate: (id: string) => void;
  onDeactivate: (id: string) => void;
  onDetail: (hand: HandDefinitionItem) => void;
  onChat: (instanceId: string, handName: string) => void;
  isPending: boolean;
}) {
  const { t } = useTranslation();
  const isPaused = instance?.status === "paused";

  return (
    <div
      className="group p-4 rounded-2xl border border-border-subtle hover:border-brand/30 bg-surface hover:bg-surface/80 transition-all cursor-pointer"
      onClick={() => onDetail(hand)}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onDetail(hand);
        }
      }}
    >
      {/* Top: icon + status */}
      <div className="flex items-start justify-between mb-3">
        <div
          className={`w-11 h-11 rounded-2xl flex items-center justify-center ${
            isActive
              ? isPaused
                ? "bg-warning/15 text-warning"
                : "bg-success/15 text-success"
              : "bg-brand/8 text-brand/60 group-hover:bg-brand/12 group-hover:text-brand"
          } transition-colors`}
        >
          <Hand className="w-5 h-5" />
        </div>
        <div
          className="flex items-center gap-1"
          onClick={(e) => e.stopPropagation()}
        >
          {isActive && instance && !isPaused && (
            <button
              onClick={() => onChat(instance.instance_id, hand.name || hand.id)}
              className="p-1.5 rounded-lg text-brand/60 hover:text-brand hover:bg-brand/10 opacity-0 group-hover:opacity-100 transition-all"
              title={t("chat.title")}
            >
              <MessageCircle className="w-4 h-4" />
            </button>
          )}
          {isActive && instance ? (
            <button
              onClick={() => onDeactivate(instance.instance_id)}
              disabled={isPending}
              className="p-1.5 rounded-lg text-text-dim/40 hover:text-error hover:bg-error/10 opacity-0 group-hover:opacity-100 transition-all disabled:opacity-40"
              title={t("hands.deactivate")}
            >
              {isPending ? <Loader2 className="w-4 h-4 animate-spin" /> : <PowerOff className="w-4 h-4" />}
            </button>
          ) : (
            <button
              onClick={() => onActivate(hand.id)}
              disabled={isPending || !hand.requirements_met}
              className="p-1.5 rounded-lg text-text-dim/40 hover:text-brand hover:bg-brand/10 opacity-0 group-hover:opacity-100 transition-all disabled:opacity-40 disabled:cursor-not-allowed"
              title={t("hands.activate")}
            >
              {isPending ? <Loader2 className="w-4 h-4 animate-spin" /> : <Power className="w-4 h-4" />}
            </button>
          )}
        </div>
      </div>

      {/* Name + category */}
      <h3 className="text-sm font-bold truncate mb-1">{hand.name || hand.id}</h3>
      <div className="flex items-center gap-1.5 mb-2 flex-wrap">
        {isActive && (
          isPaused ? (
            <Badge variant="warning" dot>{t("hands.paused")}</Badge>
          ) : (
            <Badge variant="success" dot>{t("hands.active_label")}</Badge>
          )
        )}
        {!isActive && !hand.requirements_met && (
          <Badge variant="warning">{t("hands.missing_req")}</Badge>
        )}
        {hand.category && (
          <span className="text-[10px] text-text-dim/50">
            {t(`hands.cat_${hand.category}`, { defaultValue: hand.category })}
          </span>
        )}
      </div>

      {/* Description */}
      <p className="text-[11px] text-text-dim leading-relaxed line-clamp-2 min-h-[2.5em]">
        {hand.description || "-"}
      </p>

      {/* Tools count + metrics */}
      <div className="flex items-center justify-between mt-3 pt-3 border-t border-border-subtle/50">
        <div className="flex items-center gap-3">
          {hand.tools && hand.tools.length > 0 && (
            <span className="text-[10px] text-text-dim/50 flex items-center gap-1">
              <Wrench className="w-3 h-3" />
              {hand.tools.length}
            </span>
          )}
          {hand.requirements && hand.requirements.length > 0 && (
            <span className="text-[10px] text-text-dim/50 flex items-center gap-1">
              <CheckCircle2 className="w-3 h-3" />
              {hand.requirements.filter((r) => r.satisfied).length}/{hand.requirements.length}
            </span>
          )}
        </div>
        <ChevronRight className="w-3.5 h-3.5 text-text-dim/30 group-hover:text-brand/50 transition-colors" />
      </div>
    </div>
  );
}

/* ── Main page ────────────────────────────────────────────── */

export function HandsPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const addToast = useUIStore((s) => s.addToast);
  const [pendingId, setPendingId] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [selectedCategory, setSelectedCategory] = useState<string>("all");
  const [detailHand, setDetailHand] = useState<HandDefinitionItem | null>(null);
  const navigate = useNavigate();

  // Preload ChatPage chunk so navigate is instant
  useEffect(() => {
    router.preloadRoute({ to: "/chat", search: { agentId: undefined } }).catch(() => {});
  }, []);

  const handsQuery = useQuery({
    queryKey: ["hands", "list"],
    queryFn: listHands,
    refetchInterval: REFRESH_MS,
  });
  const activeQuery = useQuery({
    queryKey: ["hands", "active"],
    queryFn: listActiveHands,
    refetchInterval: REFRESH_MS,
  });

  const activateMutation = useMutation({
    mutationFn: (id: string) => activateHand(id),
  });
  const deactivateMutation = useMutation({
    mutationFn: (id: string) => deactivateHand(id),
  });
  const pauseMutation = useMutation({
    mutationFn: (id: string) => pauseHand(id),
  });
  const resumeMutation = useMutation({
    mutationFn: (id: string) => resumeHand(id),
  });

  const hands = handsQuery.data ?? [];
  const instances = activeQuery.data ?? [];

  // Batch-fetch stats for all active instances (avoids N+1 queries)
  const activeInstanceIds = useMemo(() => instances.map(i => i.instance_id).filter(Boolean), [instances]);
  const allStatsQuery = useQuery({
    queryKey: ["hands", "stats", "batch", activeInstanceIds],
    queryFn: async () => {
      const results: Record<string, HandStatsResponse> = {};
      await Promise.all(activeInstanceIds.map(async id => {
        try { results[id] = await getHandStats(id); } catch { /* skip */ }
      }));
      return results;
    },
    refetchInterval: REFRESH_MS,
    enabled: activeInstanceIds.length > 0,
  });
  const allStats = allStatsQuery.data ?? {};

  const activeHandIds = useMemo(
    () => new Set(instances.map((i) => i.hand_id).filter(Boolean)),
    [instances],
  );

  // Extract unique categories
  const categories = useMemo(() => {
    const cats = new Set<string>();
    for (const h of hands) {
      if (h.category) cats.add(h.category);
    }
    return Array.from(cats).sort();
  }, [hands]);

  // Active hands with their definitions
  const activeHands = useMemo(
    () =>
      instances
        .map((inst) => ({
          instance: inst,
          hand: hands.find((h) => h.id === inst.hand_id),
        }))
        .filter((x) => x.hand != null) as Array<{
        instance: HandInstanceItem;
        hand: HandDefinitionItem;
      }>,
    [instances, hands],
  );

  // Filtered hands for the grid — exclude hands already shown in active strip
  const filtered = useMemo(() => {
    return hands
      .filter((h) => {
        // Exclude hands already displayed in the active strip
        if (activeHandIds.has(h.id)) return false;
        // Category filter
        if (selectedCategory !== "all" && h.category !== selectedCategory) return false;
        // Search filter
        if (search) {
          const q = search.toLowerCase();
          return (
            (h.name || "").toLowerCase().includes(q) ||
            (h.id || "").toLowerCase().includes(q) ||
            (h.description || "").toLowerCase().includes(q)
          );
        }
        return true;
      })
      .sort((a, b) => (a.name || a.id).localeCompare(b.name || b.id));
  }, [hands, search, selectedCategory, activeHandIds]);

  async function handleActivate(id: string) {
    setPendingId(id);
    try {
      await activateMutation.mutateAsync(id);
      await queryClient.invalidateQueries({ queryKey: ["hands"] });
      addToast(t("common.success"), "success");
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : t("common.error");
      addToast(msg, "error");
    } finally {
      setPendingId(null);
    }
  }

  async function handleDeactivate(id: string) {
    setPendingId(id);
    try {
      await deactivateMutation.mutateAsync(id);
      await queryClient.invalidateQueries({ queryKey: ["hands"] });
      addToast(t("common.success"), "success");
      setDetailHand(null);
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : t("common.error");
      addToast(msg, "error");
    } finally {
      setPendingId(null);
    }
  }

  async function handlePause(id: string) {
    setPendingId(id);
    try {
      await pauseMutation.mutateAsync(id);
      await queryClient.invalidateQueries({ queryKey: ["hands"] });
      addToast(t("common.success"), "success");
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : t("common.error");
      addToast(msg, "error");
    } finally {
      setPendingId(null);
    }
  }

  async function handleResume(id: string) {
    setPendingId(id);
    try {
      await resumeMutation.mutateAsync(id);
      await queryClient.invalidateQueries({ queryKey: ["hands"] });
      addToast(t("common.success"), "success");
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : t("common.error");
      addToast(msg, "error");
    } finally {
      setPendingId(null);
    }
  }

  const activeCount = activeHandIds.size;

  const detailInstance = detailHand
    ? instances.find((i) => i.hand_id === detailHand.id)
    : undefined;
  const detailIsActive = detailHand ? activeHandIds.has(detailHand.id) : false;

  return (
    <div className="flex flex-col gap-5 transition-colors duration-300">
      <PageHeader
        badge={t("hands.orchestration")}
        title={t("hands.title")}
        subtitle={t("hands.subtitle")}
        isFetching={handsQuery.isFetching}
        onRefresh={() => {
          handsQuery.refetch();
          activeQuery.refetch();
        }}
        icon={<Hand className="h-4 w-4" />}
        helpText={t("hands.help")}
        actions={
          <div className="flex items-center gap-3">
            <Badge variant="success" dot>
              {activeCount} {t("hands.active_label")}
            </Badge>
            <Badge variant="default">
              {hands.length} {t("hands.total_label")}
            </Badge>
          </div>
        }
      />

      {/* Active hands strip */}
      {activeHands.length > 0 && (
        <div>
          <div className="flex items-center gap-2 mb-2.5">
            <Activity className="w-4 h-4 text-success" />
            <span className="text-xs font-bold text-text-dim">
              {t("hands.instances")}
            </span>
          </div>
          <div className="flex gap-3 overflow-x-auto pb-2 scrollbar-thin -mx-1 px-1">
            {activeHands.map(({ hand, instance }) => (
              <ActiveHandCard
                key={instance.instance_id}
                hand={hand}
                instance={instance}
                onChat={(instanceId) => {
                  const inst = instances.find(i => i.instance_id === instanceId);
                  navigate({ to: "/chat", search: { agentId: inst?.agent_id || instanceId } });
                }}
                onDeactivate={handleDeactivate}
                onDetail={setDetailHand}
                isPending={pendingId === hand.id || pendingId === instance.instance_id}
                metrics={allStats[instance.instance_id]?.metrics}
              />
            ))}
          </div>
        </div>
      )}

      {/* Category filter + Search */}
      {hands.length > 0 && (
        <div className="flex flex-col sm:flex-row gap-3">
          {/* Category pills */}
          <div className="flex items-center gap-1.5 overflow-x-auto scrollbar-thin pb-1 shrink-0">
            <button
              onClick={() => setSelectedCategory("all")}
              className={`px-3 py-1.5 rounded-xl text-[11px] font-bold whitespace-nowrap transition-all ${
                selectedCategory === "all"
                  ? "bg-brand text-white shadow-sm shadow-brand/20"
                  : "bg-main text-text-dim hover:text-text hover:bg-main/80 border border-border-subtle"
              }`}
            >
              {t("providers.filter_all")}
            </button>
            {categories.map((cat) => (
              <button
                key={cat}
                onClick={() =>
                  setSelectedCategory(selectedCategory === cat ? "all" : cat)
                }
                className={`px-3 py-1.5 rounded-xl text-[11px] font-bold whitespace-nowrap transition-all ${
                  selectedCategory === cat
                    ? "bg-brand text-white shadow-sm shadow-brand/20"
                    : "bg-main text-text-dim hover:text-text hover:bg-main/80 border border-border-subtle"
                }`}
              >
                {t(`hands.cat_${cat}`, { defaultValue: cat })}
              </button>
            ))}
          </div>

          {/* Search */}
          <div className="flex-1">
            <Input
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder={t("hands.search_placeholder")}
              leftIcon={<Search className="h-4 w-4" />}
            />
          </div>
        </div>
      )}

      {/* Hands grid */}
      {handsQuery.isLoading ? (
        <ListSkeleton rows={6} />
      ) : hands.length === 0 ? (
        <div className="text-center py-20">
          <div className="w-16 h-16 rounded-3xl bg-brand/8 flex items-center justify-center mx-auto mb-4">
            <Hand className="w-8 h-8 text-brand/30" />
          </div>
          <p className="text-sm font-bold text-text-dim/60">{t("common.no_data")}</p>
          <p className="text-xs text-text-dim/40 mt-1">{t("hands.subtitle")}</p>
        </div>
      ) : filtered.length === 0 ? (
        <div className="text-center py-16">
          <Search className="w-8 h-8 text-text-dim/20 mx-auto mb-3" />
          <p className="text-sm text-text-dim/50">
            {t("agents.no_matching")}
          </p>
        </div>
      ) : (
        <div className="grid gap-3 grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 stagger-children">
          {filtered.map((h) => {
            const isActive = activeHandIds.has(h.id);
            const instance = instances.find((i) => i.hand_id === h.id);
            return (
              <HandCard
                key={h.id}
                hand={h}
                instance={instance}
                isActive={isActive}
                onActivate={handleActivate}
                onDeactivate={(id) => handleDeactivate(id)}
                onDetail={setDetailHand}
                onChat={(instanceId) => {
                  const inst = instances.find(i => i.instance_id === instanceId);
                  navigate({ to: "/chat", search: { agentId: inst?.agent_id || instanceId } });
                }}
                isPending={pendingId === h.id || (instance ? pendingId === instance.instance_id : false)}
              />
            );
          })}
        </div>
      )}

      {/* Detail side panel */}
      {detailHand && (
        <HandDetailPanel
          hand={detailHand}
          instance={detailInstance}
          isActive={detailIsActive}
          onClose={() => setDetailHand(null)}
          onActivate={handleActivate}
          onDeactivate={handleDeactivate}
          onPause={handlePause}
          onResume={handleResume}
          onChat={(instanceId) => {
            const inst = instances.find(i => i.instance_id === instanceId);
            navigate({ to: "/chat", search: { agentId: inst?.agent_id || instanceId } });
          }}
          isPending={pendingId === detailHand.id}
        />
      )}

    </div>
  );
}
