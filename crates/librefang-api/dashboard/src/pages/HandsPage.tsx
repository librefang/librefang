import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { formatDateTime } from "../lib/datetime";
import { formatCost } from "../lib/format";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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
  setHandSecret,
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
} from "lucide-react";
import { PageHeader } from "../components/ui/PageHeader";
import { ListSkeleton } from "../components/ui/Skeleton";
import { truncateId } from "../lib/string";

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

  // Only show entries that have actual values (not "-" or empty)
  const entries = Object.entries(metrics).filter(([, m]) => m.value != null && String(m.value) !== "-" && String(m.value) !== "").slice(0, 3);
  if (entries.length === 0) return null;

  return (
    <div className="flex flex-wrap gap-x-3 gap-y-1 mt-1">
      {entries.map(([label, m]) => (
        <span key={label} className="text-[9px] text-text-dim/70 font-mono">
          <span className="text-text-dim/40">{label}:</span>{" "}
          <span className="text-brand/80">{String(m.value)}</span>
        </span>
      ))}
    </div>
  );
}

/* ── Chat panel for an active hand instance ──────────────── */

interface ChatMsg {
  id: string;
  role: "user" | "assistant";
  content: string;
  timestamp: Date;
  isLoading?: boolean;
  error?: string;
  tokens?: { input?: number; output?: number };
  cost_usd?: number;
}

function HandChatPanel({
  instanceId,
  handName,
  onClose,
}: {
  instanceId: string;
  handName: string;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [messages, setMessages] = useState<ChatMsg[]>([]);
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const endRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    getHandSession(instanceId)
      .then((data) => {
        if (data.messages?.length) {
          const hist: ChatMsg[] = data.messages.map((m: HandSessionMessage, i: number) => ({
            id: `hist-${i}`,
            role: m.role === "user" ? "user" as const : "assistant" as const,
            content: m.content || "",
            timestamp: m.timestamp ? new Date(m.timestamp) : new Date(),
          }));
          setMessages(hist);
        }
      })
      .catch(() => {});
  }, [instanceId]);

  useEffect(() => {
    if (messages.length > 0) {
      setTimeout(() => endRef.current?.scrollIntoView({ behavior: "smooth" }), 60);
    }
  }, [messages]);

  useEffect(() => {
    setTimeout(() => inputRef.current?.focus(), 100);
  }, []);

  const handleSend = useCallback(async () => {
    const text = input.trim();
    if (!text || sending) return;

    const userMsg: ChatMsg = {
      id: `u-${Date.now()}`,
      role: "user",
      content: text,
      timestamp: new Date(),
    };
    const botMsg: ChatMsg = {
      id: `b-${Date.now()}`,
      role: "assistant",
      content: "",
      timestamp: new Date(),
      isLoading: true,
    };

    setMessages((prev) => [...prev, userMsg, botMsg]);
    setInput("");
    setSending(true);

    try {
      const res = await sendHandMessage(instanceId, text);
      setMessages((prev) =>
        prev.map((m) =>
          m.id === botMsg.id
            ? {
                ...m,
                content: res.response || "",
                isLoading: false,
                tokens: { input: res.input_tokens, output: res.output_tokens },
                cost_usd: res.cost_usd,
              }
            : m
        )
      );
    } catch (err) {
      const errMsg = err instanceof Error ? err.message : "Error";
      setMessages((prev) =>
        prev.map((m) =>
          m.id === botMsg.id ? { ...m, isLoading: false, error: errMsg } : m
        )
      );
    } finally {
      setSending(false);
      setTimeout(() => inputRef.current?.focus(), 50);
    }
  }, [input, sending, instanceId]);

  return (
    <div
      className="fixed inset-0 z-50 flex items-end sm:items-center justify-center bg-black/40 backdrop-blur-xl backdrop-saturate-150"
      onClick={onClose}
    >
      <div
        className="bg-surface rounded-t-2xl sm:rounded-2xl shadow-2xl border border-border-subtle w-full sm:w-[640px] sm:max-w-[92vw] h-[85vh] sm:h-[80vh] flex flex-col animate-fade-in-scale"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="px-5 py-3.5 border-b border-border-subtle flex items-center justify-between shrink-0">
          <div className="flex items-center gap-2.5">
            <div className="w-8 h-8 rounded-lg bg-brand/15 text-brand flex items-center justify-center">
              <MessageCircle className="w-4 h-4" />
            </div>
            <div>
              <h3 className="text-sm font-bold">{handName}</h3>
              <p className="text-[9px] text-text-dim/60 font-mono">
                {truncateId(instanceId, 12)}
              </p>
            </div>
          </div>
          <button
            onClick={onClose}
            className="p-1.5 rounded-lg text-text-dim hover:text-text hover:bg-main transition-colors"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* Messages */}
        <div className="flex-1 overflow-y-auto p-4 space-y-3 scrollbar-thin">
          {messages.length === 0 && !sending && (
            <div className="h-full flex flex-col items-center justify-center text-center">
              <div className="w-14 h-14 rounded-xl bg-brand/10 flex items-center justify-center mb-3">
                <Bot className="w-7 h-7 text-brand/60" />
              </div>
              <p className="text-sm font-bold">{handName}</p>
              <p className="text-xs text-text-dim mt-1">{t("chat.welcome_system")}</p>
            </div>
          )}
          {messages.map((msg) => (
            <div
              key={msg.id}
              className={`flex ${msg.role === "user" ? "justify-end" : "justify-start"}`}
            >
              <div className={`max-w-[85%] ${msg.role === "user" ? "items-end" : "items-start"}`}>
                <div className={`flex items-center gap-1.5 mb-1 ${msg.role === "user" ? "justify-end" : ""}`}>
                  <div className={`h-5 w-5 rounded-md flex items-center justify-center ${
                    msg.role === "user"
                      ? "bg-brand text-white"
                      : "bg-surface border border-border-subtle"
                  }`}>
                    {msg.role === "user" ? (
                      <User className="h-2.5 w-2.5" />
                    ) : (
                      <Bot className="h-2.5 w-2.5 text-brand" />
                    )}
                  </div>
                  <span className="text-[9px] text-text-dim/50">
                    {msg.timestamp.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}
                  </span>
                </div>
                <div
                  className={`px-3 py-2 rounded-xl text-xs leading-relaxed ${
                    msg.role === "user"
                      ? "bg-brand text-white rounded-tr-sm"
                      : msg.error
                        ? "bg-error/10 border border-error/20 text-error rounded-tl-sm"
                        : "bg-surface border border-border-subtle rounded-tl-sm"
                  }`}
                >
                  {msg.isLoading ? (
                    <div className="flex items-center gap-1 py-1">
                      <span className="w-1.5 h-1.5 bg-brand/60 rounded-full animate-bounce" style={{ animationDelay: "0ms" }} />
                      <span className="w-1.5 h-1.5 bg-brand/60 rounded-full animate-bounce" style={{ animationDelay: "150ms" }} />
                      <span className="w-1.5 h-1.5 bg-brand/60 rounded-full animate-bounce" style={{ animationDelay: "300ms" }} />
                    </div>
                  ) : msg.error ? (
                    <div className="flex items-start gap-1.5">
                      <AlertCircle className="h-3.5 w-3.5 shrink-0 mt-0.5" />
                      <span>{msg.error}</span>
                    </div>
                  ) : msg.role === "user" ? (
                    <span>{msg.content}</span>
                  ) : (
                    <Markdown remarkPlugins={[remarkGfm]} components={mdComponents as Record<string, React.ComponentType>}>
                      {msg.content}
                    </Markdown>
                  )}
                </div>
                {msg.tokens?.output && !msg.isLoading && (
                  <div className="flex items-center gap-1.5 mt-1">
                    <span className="text-[8px] text-text-dim/40 font-mono">
                      {msg.tokens.output} tok
                    </span>
                    {msg.cost_usd !== undefined && msg.cost_usd > 0 && (
                      <span className="text-[8px] text-success/60 font-mono">
                        {formatCost(msg.cost_usd)}
                      </span>
                    )}
                  </div>
                )}
              </div>
            </div>
          ))}
          <div ref={endRef} />
        </div>

        {/* Input */}
        <div className="px-4 py-3 border-t border-border-subtle shrink-0">
          <form
            onSubmit={(e) => {
              e.preventDefault();
              handleSend();
            }}
            className="flex gap-2 items-end"
          >
            <textarea
              ref={inputRef}
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  handleSend();
                }
              }}
              placeholder={t("chat.input_placeholder_with_agent", { name: handName })}
              disabled={sending}
              rows={1}
              className="flex-1 min-h-[40px] max-h-[100px] rounded-xl border border-border-subtle bg-main px-3 py-2.5 text-sm focus:border-brand focus:ring-2 focus:ring-brand/10 outline-none resize-none placeholder:text-text-dim/40"
            />
            <button
              type="submit"
              disabled={!input.trim() || sending}
              className="px-3.5 py-2.5 rounded-xl bg-brand text-white font-bold text-sm shadow-lg shadow-brand/20 hover:shadow-brand/40 hover:-translate-y-0.5 transition-all disabled:opacity-40 disabled:cursor-not-allowed disabled:hover:translate-y-0"
            >
              <Send className="h-4 w-4" />
            </button>
          </form>
        </div>
      </div>
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
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 backdrop-blur-sm p-4"
      onClick={onClose}
    >
      <div
        className="bg-surface w-full max-w-lg h-[70vh] rounded-2xl shadow-2xl border border-border-subtle flex flex-col overflow-hidden"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Title bar */}
        <div className="flex items-center justify-between px-5 py-3 border-b border-border-subtle shrink-0">
          <h2 className="text-sm font-bold truncate">{hand.name || hand.id}</h2>
          <button onClick={onClose} className="p-1 rounded-lg text-text-dim/40 hover:text-text hover:bg-main transition-colors">
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* Scrollable body */}
        <div className="flex-1 overflow-y-auto scrollbar-thin">
          <div className="px-5 py-4 space-y-4">
            {/* Status + actions */}
            <div className="flex items-center gap-2">
              {isActive ? (
                isPaused
                  ? <Badge variant="warning" dot>{t("hands.paused")}</Badge>
                  : <Badge variant="success" dot>{t("hands.active_label")}</Badge>
              ) : hand.requirements_met ? (
                <Badge variant="default">{t("hands.ready")}</Badge>
              ) : (
                <Badge variant="warning">{t("hands.missing_req")}</Badge>
              )}
              {hand.category && (
                <Badge variant="info">{t(`hands.cat_${hand.category}`, { defaultValue: hand.category })}</Badge>
              )}
              <div className="flex-1" />
              {isActive && instance ? (
                <div className="flex items-center gap-1">
                  <button onClick={() => onChat(instance.instance_id, hand.name || hand.id)} disabled={isPaused}
                    className="px-2.5 py-1 rounded-lg text-[11px] font-medium text-brand hover:bg-brand/10 transition-colors disabled:opacity-30">
                    {t("chat.title")}
                  </button>
                  {isPaused ? (
                    <button onClick={() => onResume(instance.instance_id)} disabled={isPending}
                      className="px-2.5 py-1 rounded-lg text-[11px] font-medium text-success hover:bg-success/10 transition-colors disabled:opacity-30">
                      {isPending ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : t("hands.resume")}
                    </button>
                  ) : (
                    <button onClick={() => onPause(instance.instance_id)} disabled={isPending}
                      className="px-2.5 py-1 rounded-lg text-[11px] font-medium text-text-dim hover:bg-main transition-colors disabled:opacity-30">
                      {isPending ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : t("hands.pause")}
                    </button>
                  )}
                  <button onClick={() => onDeactivate(instance.instance_id)} disabled={isPending}
                    className="px-2.5 py-1 rounded-lg text-[11px] font-medium text-error hover:bg-error/10 transition-colors disabled:opacity-30">
                    {isPending ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : t("hands.deactivate")}
                  </button>
                </div>
              ) : (
                <button onClick={() => onActivate(hand.id)} disabled={isPending || !hand.requirements_met}
                  className="px-3 py-1 rounded-lg text-[11px] font-medium text-white bg-brand hover:bg-brand/90 transition-colors disabled:opacity-40 disabled:cursor-not-allowed">
                  {isPending ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : t("hands.activate")}
                </button>
              )}
            </div>

            {/* Description */}
            {hand.description && (
              <p className="text-[13px] text-text-dim leading-relaxed">{hand.description}</p>
            )}

            {/* Sections */}
            <DetailTabs key={hand.id} hand={hand} instance={instance} isActive={isActive} settings={settings} settingsQuery={settingsQuery} stats={stats} statsQuery={statsQuery} />
          </div>
        </div>
      </div>

    </div>
  );
}

/* ── Collapsible section helper ──────────────────────────── */

/* ── Detail tabs content ─────────────────────────────────── */

function RequirementsForm({ handId, requirements }: { handId: string; requirements: HandDefinitionItem["requirements"] }) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const addToast = useUIStore((s) => s.addToast);
  const [values, setValues] = useState<Record<string, string>>(() => {
    const init: Record<string, string> = {};
    for (const r of requirements ?? []) {
      if (r.key && r.current_value) init[r.key] = r.current_value;
    }
    return init;
  });
  const [saving, setSaving] = useState<string | null>(null);

  if (!requirements || requirements.length === 0) return null;

  const handleSave = async (key: string) => {
    const val = values[key]?.trim();
    if (!val) return;
    setSaving(key);
    try {
      await setHandSecret(handId, key, val);
      addToast(t("common.success"), "success");
      queryClient.invalidateQueries({ queryKey: ["hands"] });
    } catch (e: unknown) {
      addToast(e instanceof Error ? e.message : t("common.error"), "error");
    } finally {
      setSaving(null);
    }
  };

  return (
    <div className="space-y-2.5">
      {requirements.map((r) => (
        <div key={r.key}>
          <div className="flex items-center gap-2 mb-1">
            {r.satisfied ? <CheckCircle2 className="w-3.5 h-3.5 text-success shrink-0" /> : <XCircle className="w-3.5 h-3.5 text-error shrink-0" />}
            <span className={`text-[11px] font-medium ${r.satisfied ? "text-text-dim" : "text-text"}`}>{r.label || r.key}</span>
            {r.optional && <span className="text-[9px] text-text-dim/40">(optional)</span>}
          </div>
          {r.key && (
            <div className="flex gap-1.5 ml-5">
              <input
                type="text"
                autoComplete="off"
                placeholder={r.satisfied ? "••••••••" : r.key}
                value={values[r.key!] ?? ""}
                onChange={(e) => { setValues(prev => ({ ...prev, [r.key!]: e.target.value })); }}
                onKeyDown={(e) => { if (e.key === "Enter") { e.preventDefault(); handleSave(r.key!); } }}
                className={`flex-1 px-2.5 py-1.5 rounded-lg border text-[11px] font-mono outline-none focus:border-brand placeholder:text-text-dim/30 ${
                  r.satisfied ? "border-success/20 bg-success/5" : "border-border-subtle bg-main"
                }`}
              />
              <button
                type="button"
                onClick={(e) => { e.preventDefault(); e.stopPropagation(); handleSave(r.key!); }}
                disabled={!values[r.key!]?.trim() || saving === r.key}
                className="px-2.5 py-1.5 rounded-lg text-[11px] font-medium text-white bg-brand hover:bg-brand/90 transition-colors disabled:opacity-40"
              >
                {saving === r.key ? <Loader2 className="w-3 h-3 animate-spin" /> : t("common.save")}
              </button>
            </div>
          )}
        </div>
      ))}
    </div>
  );
}

function DetailTabs({ hand, instance, isActive, settings, settingsQuery, stats, statsQuery }: {
  hand: HandDefinitionItem; instance: HandInstanceItem | undefined; isActive: boolean;
  settings: HandSettingsResponse; settingsQuery: any; stats: HandStatsResponse; statsQuery: any;
}) {
  const { t } = useTranslation();
  const hasMetrics = isActive && !statsQuery.isLoading && stats.metrics &&
    Object.entries(stats.metrics).some(([, m]) => m.value != null && String(m.value) !== "-" && String(m.value) !== "");

  type Tab = "settings" | "requirements" | "tools" | "metrics";
  const tabs: { id: Tab; label: string; count?: number; show: boolean }[] = [
    { id: "settings", label: t("hands.settings"), count: settings.settings?.length, show: true },
    { id: "requirements", label: t("hands.requirements"), count: hand.requirements?.length, show: !!(hand.requirements && hand.requirements.length > 0) },
    { id: "tools", label: t("hands.tools"), count: hand.tools?.length, show: !!(hand.tools && hand.tools.length > 0) },
    { id: "metrics", label: t("hands.metrics"), show: !!hasMetrics },
  ];
  const visibleTabs = tabs.filter(t => t.show);
  const [activeTab, setActiveTab] = useState<Tab>(visibleTabs[0]?.id ?? "settings");

  return (
    <div>
      {/* Tab bar */}
      <div className="flex border-b border-border-subtle">
        {visibleTabs.map(tab => (
          <button key={tab.id} onClick={() => setActiveTab(tab.id)}
            className={`px-3 py-2 text-[11px] font-semibold transition-colors relative ${
              activeTab === tab.id
                ? "text-brand"
                : "text-text-dim/50 hover:text-text-dim"
            }`}>
            {tab.label}
            {tab.count !== undefined && <span className="ml-1 text-[9px] opacity-50">{tab.count}</span>}
            {activeTab === tab.id && <span className="absolute bottom-0 left-1 right-1 h-0.5 bg-brand rounded-full" />}
          </button>
        ))}
      </div>

      {/* Tab content */}
      <div className="py-3">
        {activeTab === "metrics" && hasMetrics && (
          <div className="grid grid-cols-2 gap-2">
            {Object.entries(stats.metrics!).filter(([, m]) => m.value != null && String(m.value) !== "-" && String(m.value) !== "").map(([label, m]) => (
              <div key={label} className="p-2.5 rounded-lg bg-main border border-border-subtle">
                <p className="text-[10px] text-text-dim/60 truncate">{label}</p>
                <p className="text-sm font-bold text-brand">{String(m.value)}</p>
              </div>
            ))}
          </div>
        )}

        {activeTab === "settings" && (
          settingsQuery.isLoading ? (
            <div className="flex items-center gap-2 text-text-dim/50 text-[10px]">
              <Loader2 className="w-3 h-3 animate-spin" /> {t("common.loading")}
            </div>
          ) : settings.settings && settings.settings.length > 0 ? (
            <div className="space-y-0.5">
              {settings.settings.map((s) => {
                const currentVal = settings.current_values?.[s.key ?? ""];
                const displayVal = currentVal !== undefined ? String(currentVal) : (s.default !== undefined ? String(s.default) : undefined);
                const isDefault = currentVal === undefined;
                return (
                  <div key={s.key} className="flex items-center justify-between gap-2 py-1.5">
                    <span className="text-[11px] font-medium truncate">{s.label || s.key}</span>
                    {displayVal !== undefined && (
                      <span className={`text-[10px] font-mono shrink-0 px-1.5 py-0.5 rounded ${isDefault ? "text-text-dim/50 bg-main" : "text-brand bg-brand/8"}`}>
                        {displayVal || "-"}
                      </span>
                    )}
                  </div>
                );
              })}
            </div>
          ) : (
            <p className="text-[10px] text-text-dim/50">{t("hands.settings_empty")}</p>
          )
        )}

        {activeTab === "requirements" && hand.requirements && (
          <RequirementsForm handId={hand.id} requirements={hand.requirements} />
        )}

        {activeTab === "tools" && hand.tools && (
          <div className="flex flex-wrap gap-1.5">
            {hand.tools.map((tool) => (
              <span key={tool} className="text-[10px] font-mono text-text-dim px-2 py-1 rounded-md bg-main/50 border border-border-subtle/50">{tool}</span>
            ))}
          </div>
        )}
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
      className={`group relative flex items-center gap-3 px-4 py-3 rounded-2xl border cursor-pointer transition-colors shrink-0 min-w-[240px] max-w-[320px] ${
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
      className={`group relative flex items-center gap-3 p-3 rounded-xl border cursor-pointer transition-colors ${
        isActive
          ? isPaused
            ? "border-warning/20 bg-warning/5 hover:border-warning/40"
            : "border-success/20 bg-success/5 hover:border-success/40"
          : "border-border-subtle hover:border-brand/30 bg-surface"
      }`}
      onClick={() => onDetail(hand)}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); onDetail(hand); } }}
    >
      <div className={`w-9 h-9 rounded-xl flex items-center justify-center shrink-0 ${
        isActive ? isPaused ? "bg-warning/15 text-warning" : "bg-success/15 text-success" : "bg-brand/8 text-brand/60"
      }`}>
        <Hand className="w-4 h-4" />
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <h3 className="text-sm font-bold truncate">{hand.name || hand.id}</h3>
          {isActive ? (
            isPaused ? <Badge variant="warning" dot>{t("hands.paused")}</Badge> : <Badge variant="success" dot>{t("hands.active_label")}</Badge>
          ) : !hand.requirements_met ? (
            <Badge variant="warning">{t("hands.missing_req")}</Badge>
          ) : null}
        </div>
        <div className="flex items-center gap-2 mt-0.5">
          {hand.category && <span className="text-[10px] text-text-dim/50">{t(`hands.cat_${hand.category}`, { defaultValue: hand.category })}</span>}
          {hand.tools && hand.tools.length > 0 && (
            <span className="text-[10px] text-text-dim/40 flex items-center gap-0.5"><Wrench className="w-2.5 h-2.5" />{hand.tools.length}</span>
          )}
        </div>
      </div>
      <div className="flex items-center gap-1 shrink-0 opacity-0 group-hover:opacity-100 transition-opacity" onClick={(e) => e.stopPropagation()}>
        {isActive && instance && !isPaused && (
          <button onClick={() => onChat(instance.instance_id, hand.name || hand.id)}
            className="p-1.5 rounded-lg text-brand/60 hover:text-brand hover:bg-brand/10 transition-colors" title={t("chat.title")}>
            <MessageCircle className="w-3.5 h-3.5" />
          </button>
        )}
        {isActive && instance ? (
          <button onClick={() => onDeactivate(instance.instance_id)} disabled={isPending}
            className="p-1.5 rounded-lg text-text-dim/40 hover:text-error hover:bg-error/10 transition-colors disabled:opacity-40" title={t("hands.deactivate")}>
            {isPending ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <PowerOff className="w-3.5 h-3.5" />}
          </button>
        ) : (
          <button onClick={() => onActivate(hand.id)} disabled={isPending || !hand.requirements_met}
            className="p-1.5 rounded-lg text-text-dim/40 hover:text-brand hover:bg-brand/10 transition-colors disabled:opacity-40 disabled:cursor-not-allowed" title={t("hands.activate")}>
            {isPending ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Power className="w-3.5 h-3.5" />}
          </button>
        )}
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
  const [statusFilter, setStatusFilter] = useState<"all" | "active" | "inactive">("all");
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
        // Status filter
        if (statusFilter === "active" && !activeHandIds.has(h.id)) return false;
        if (statusFilter === "inactive" && activeHandIds.has(h.id)) return false;
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
      .sort((a, b) => {
        const aActive = activeHandIds.has(a.id) ? 0 : 1;
        const bActive = activeHandIds.has(b.id) ? 0 : 1;
        if (aActive !== bActive) return aActive - bActive;
        return (a.name || a.id).localeCompare(b.name || b.id);
      });
  }, [hands, search, selectedCategory, statusFilter, activeHandIds]);

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

  // Always read the latest hand data from the query cache so the modal
  // reflects changes (e.g. requirement satisfaction) after saving secrets.
  const detailHandLatest = detailHand
    ? hands.find((h) => h.id === detailHand.id) ?? detailHand
    : null;
  const detailInstance = detailHandLatest
    ? instances.find((i) => i.hand_id === detailHandLatest.id)
    : undefined;
  const detailIsActive = detailHandLatest ? activeHandIds.has(detailHandLatest.id) : false;

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

      {/* Filters */}
      {hands.length > 0 && (
        <div className="space-y-3">
          {/* Row 1: Status tabs + Search */}
          <div className="flex items-center gap-3">
            <div className="flex items-center gap-1 shrink-0">
              {(["all", "active", "inactive"] as const).map((s) => {
                const label = s === "all" ? t("providers.filter_all") : s === "active" ? t("hands.active_label") : t("hands.inactive_label");
                const count = s === "all" ? hands.length : s === "active" ? activeCount : hands.length - activeCount;
                return (
                  <button key={s} onClick={() => setStatusFilter(s)}
                    className={`px-3 py-1.5 rounded-lg text-[11px] font-bold whitespace-nowrap transition-colors ${
                      statusFilter === s
                        ? "bg-brand text-white shadow-sm"
                        : "text-text-dim hover:text-text hover:bg-main"
                    }`}>
                    {label} <span className="opacity-60">({count})</span>
                  </button>
                );
              })}
            </div>
            <div className="flex-1">
              <Input
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                placeholder={t("hands.search_placeholder")}
                leftIcon={<Search className="h-4 w-4" />}
              />
            </div>
          </div>
          {/* Row 2: Category pills */}
          <div className="flex items-center gap-1.5 overflow-x-auto scrollbar-thin">
            <button
              onClick={() => setSelectedCategory("all")}
              className={`px-2.5 py-1 rounded-lg text-[10px] font-bold whitespace-nowrap transition-colors ${
                selectedCategory === "all"
                  ? "bg-brand/10 text-brand border border-brand/30"
                  : "text-text-dim/60 hover:text-text-dim hover:bg-main border border-transparent"
              }`}
            >
              {t("providers.filter_all")}
            </button>
            {categories.map((cat) => (
              <button
                key={cat}
                onClick={() => setSelectedCategory(selectedCategory === cat ? "all" : cat)}
                className={`px-2.5 py-1 rounded-lg text-[10px] font-bold whitespace-nowrap transition-colors ${
                  selectedCategory === cat
                    ? "bg-brand/10 text-brand border border-brand/30"
                    : "text-text-dim/60 hover:text-text-dim hover:bg-main border border-transparent"
                }`}
              >
                {t(`hands.cat_${cat}`, { defaultValue: cat })}
              </button>
            ))}
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
        <div className="grid gap-2 grid-cols-1 sm:grid-cols-2 stagger-children">
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
      {detailHandLatest && (
        <HandDetailPanel
          key={detailHandLatest.id}
          hand={detailHandLatest}
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
          isPending={pendingId === detailHandLatest.id}
        />
      )}

    </div>
  );
}
