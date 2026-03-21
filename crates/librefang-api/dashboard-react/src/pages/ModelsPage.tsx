import { useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { listModels, type ModelItem } from "../api";
import { Badge } from "../components/ui/Badge";
import { Button } from "../components/ui/Button";
import {
  Cpu, Search, RefreshCw, Check, X, Eye, Wrench, Zap, AlertCircle, Lock
} from "lucide-react";

// Dynamic tiers from data - fallback list
const DEFAULT_TIERS = ["all", "frontier", "smart", "balanced", "fast", "local"] as const;
const REFRESH_MS = 60000;
const PAGE_SIZE = 50;

export function ModelsPage() {
  const { t } = useTranslation();
  const [search, setSearch] = useState("");
  const [tierFilter, setTierFilter] = useState<string>("all");
  const [providerFilter, setProviderFilter] = useState<string>("all");
  const [availableOnly, setAvailableOnly] = useState(false);
  const [page, setPage] = useState(0);

  const modelsQuery = useQuery({
    queryKey: ["models"],
    queryFn: () => listModels(),
    refetchInterval: REFRESH_MS,
  });

  // Available models first, unavailable last
  const allModels = [...(modelsQuery.data?.models ?? [])].sort((a, b) => {
    if (a.available && !b.available) return -1;
    if (!a.available && b.available) return 1;
    return 0;
  });
  const totalAvailable = modelsQuery.data?.available ?? 0;

  const providers = ["all", ...Array.from(new Set(allModels.map(m => m.provider))).sort()];
  const tiers = ["all", ...Array.from(new Set(allModels.map(m => m.tier).filter(Boolean))).sort()];

  const filtered = allModels.filter(m => {
    const q = search.toLowerCase();
    if (search && !m.id.toLowerCase().includes(q) && !(m.display_name || "").toLowerCase().includes(q) && !m.provider.toLowerCase().includes(q)) return false;
    if (tierFilter !== "all" && m.tier !== tierFilter) return false;
    if (providerFilter !== "all" && m.provider !== providerFilter) return false;
    if (availableOnly && !m.available) return false;
    return true;
  });

  // Reset page when filters change
  useEffect(() => { setPage(0); }, [search, tierFilter, providerFilter, availableOnly]);

  const paged = filtered.slice(page * PAGE_SIZE, (page + 1) * PAGE_SIZE);
  const totalPages = Math.ceil(filtered.length / PAGE_SIZE);

  const tierColor = (tier?: string) => {
    switch (tier) {
      case "basic": return "bg-slate-100 text-slate-600 dark:bg-slate-800 dark:text-slate-400";
      case "fast": return "bg-cyan-50 text-cyan-600 dark:bg-cyan-900/30 dark:text-cyan-400";
      case "smart": return "bg-blue-50 text-blue-600 dark:bg-blue-900/30 dark:text-blue-400";
      case "balanced": return "bg-teal-50 text-teal-600 dark:bg-teal-900/30 dark:text-teal-400";
      case "standard": return "bg-green-50 text-green-600 dark:bg-green-900/30 dark:text-green-400";
      case "advanced": return "bg-purple-50 text-purple-600 dark:bg-purple-900/30 dark:text-purple-400";
      case "frontier": return "bg-rose-50 text-rose-600 dark:bg-rose-900/30 dark:text-rose-400";
      case "enterprise": return "bg-amber-50 text-amber-600 dark:bg-amber-900/30 dark:text-amber-400";
      case "local": return "bg-orange-50 text-orange-600 dark:bg-orange-900/30 dark:text-orange-400";
      default: return "bg-main text-text-dim";
    }
  };

  const formatCost = (cost?: number) => {
    if (cost === undefined || cost === null) return "-";
    if (cost === 0) return t("models.free");
    if (cost < 0.01) return `$${cost.toFixed(4)}`;
    return `$${cost.toFixed(2)}`;
  };

  const formatCtx = (tokens?: number) => {
    if (!tokens) return "-";
    if (tokens >= 1000000) return `${(tokens / 1000000).toFixed(1)}M`;
    if (tokens >= 1000) return `${Math.round(tokens / 1000)}K`;
    return String(tokens);
  };

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      {/* Header */}
      <header className="flex flex-col justify-between gap-4 sm:flex-row sm:items-end">
        <div>
          <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
            <Cpu className="h-4 w-4" />
            {t("models.section")}
          </div>
          <h1 className="mt-2 text-3xl font-extrabold tracking-tight">{t("models.title")}</h1>
          <p className="mt-1 text-text-dim font-medium text-sm">{t("models.subtitle")}</p>
        </div>
        <div className="flex items-center gap-3">
          {allModels.length > 0 && (
            <Badge variant="brand">{totalAvailable} / {allModels.length} {t("models.available")}</Badge>
          )}
          <Button variant="secondary" onClick={() => modelsQuery.refetch()}>
            <RefreshCw className={`h-3.5 w-3.5 ${modelsQuery.isFetching ? "animate-spin" : ""}`} />
          </Button>
        </div>
      </header>

      {/* Error state */}
      {modelsQuery.isError && (
        <div className="flex items-center gap-3 p-4 rounded-2xl bg-error/5 border border-error/20 text-error">
          <AlertCircle className="w-5 h-5 shrink-0" />
          <p className="text-sm">{t("models.load_error")}</p>
        </div>
      )}

      {/* Filters */}
      <div className="flex flex-wrap gap-3 items-center">
        <div className="relative flex-1 min-w-[200px] max-w-sm">
          <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-text-dim/40" />
          <input type="text" value={search} onChange={e => setSearch(e.target.value)}
            placeholder={t("models.search_placeholder")}
            className="w-full pl-10 pr-4 py-2.5 rounded-xl border border-border-subtle bg-surface text-sm outline-none focus:border-brand" />
        </div>

        <select value={providerFilter} onChange={e => setProviderFilter(e.target.value)}
          className="rounded-xl border border-border-subtle bg-surface px-3 py-2.5 text-xs outline-none focus:border-brand">
          {providers.map(p => <option key={p} value={p}>{p === "all" ? t("models.all_providers") : p}</option>)}
        </select>

        <div className="flex gap-0.5 rounded-xl border border-border-subtle bg-surface p-0.5 flex-wrap">
          {tiers.map(tier => (
            <button key={tier} onClick={() => setTierFilter(tier)}
              className={`px-2.5 py-1.5 rounded-lg text-[10px] font-bold transition-colors ${
                tierFilter === tier ? "bg-brand text-white shadow-sm" : "text-text-dim hover:text-text hover:bg-main"
              }`}>
              {t(`models.tier_${tier}`, { defaultValue: tier })}
            </button>
          ))}
        </div>

        <button onClick={() => setAvailableOnly(!availableOnly)}
          className={`flex items-center gap-1.5 px-3 py-2.5 rounded-xl border text-xs font-bold transition-colors ${
            availableOnly ? "border-success bg-success/10 text-success" : "border-border-subtle text-text-dim hover:border-brand/30"
          }`}>
          <Check className="w-3 h-3" />
          {t("models.available_only")}
        </button>
      </div>

      {/* Results count + pagination */}
      <div className="flex items-center justify-between">
        <p className="text-xs text-text-dim">{filtered.length} {t("models.results")}</p>
        {totalPages > 1 && (
          <div className="flex items-center gap-2">
            <button onClick={() => setPage(p => Math.max(0, p - 1))} disabled={page === 0}
              className="px-2 py-1 rounded-lg text-xs font-bold text-text-dim hover:bg-main disabled:opacity-30">&lt;</button>
            <span className="text-xs text-text-dim">{page + 1} / {totalPages}</span>
            <button onClick={() => setPage(p => Math.min(totalPages - 1, p + 1))} disabled={page >= totalPages - 1}
              className="px-2 py-1 rounded-lg text-xs font-bold text-text-dim hover:bg-main disabled:opacity-30">&gt;</button>
          </div>
        )}
      </div>

      {/* Model List */}
      {modelsQuery.isLoading ? (
        <div className="space-y-2">
          {[1, 2, 3, 4, 5].map(i => <div key={i} className="h-12 rounded-xl bg-main animate-pulse" />)}
        </div>
      ) : filtered.length === 0 ? (
        <div className="text-center py-16">
          <Cpu className="w-10 h-10 text-text-dim/20 mx-auto mb-3" />
          <p className="text-sm text-text-dim">{allModels.length === 0 ? t("models.no_models") : t("models.no_results")}</p>
        </div>
      ) : (
        <div className="rounded-2xl border border-border-subtle overflow-hidden">
          {/* Table header */}
          <div className="grid grid-cols-[1fr_100px_80px_80px_80px_50px_50px_50px] gap-3 px-5 py-3 bg-main text-[11px] font-bold text-text-dim/60 uppercase">
            <span>{t("models.col_model")}</span>
            <span>{t("models.col_provider")}</span>
            <span>{t("models.col_tier")}</span>
            <span>{t("models.col_context")}</span>
            <span>{t("models.col_input")}</span>
            <span className="text-center" title={t("models.col_tools")}><Wrench className="w-3.5 h-3.5 inline" /></span>
            <span className="text-center" title={t("models.col_vision")}><Eye className="w-3.5 h-3.5 inline" /></span>
            <span className="text-center" title={t("models.col_streaming")}><Zap className="w-3.5 h-3.5 inline" /></span>
          </div>

          {paged.map((m, i) => (
            <div key={`${m.provider}:${m.id}`}
              className={`grid grid-cols-[1fr_100px_80px_80px_80px_50px_50px_50px] gap-3 px-5 py-3 items-center border-t border-border-subtle/50 hover:bg-surface transition-colors ${
                !m.available ? "opacity-40" : ""
              } ${i % 2 === 0 ? "" : "bg-main/30"}`}>
              <div className="min-w-0">
                <div className="flex items-center gap-1.5">
                  <p className="text-sm font-bold truncate">{m.display_name || m.id}</p>
                  {m.available ? (
                    <span className="w-2 h-2 rounded-full bg-success shrink-0" />
                  ) : (
                    <span className="flex items-center gap-0.5 text-[9px] text-text-dim/60 shrink-0">
                      <Lock className="w-3 h-3" /> {t("models.no_key")}
                    </span>
                  )}
                </div>
                {m.display_name && m.display_name !== m.id && (
                  <p className="text-[10px] text-text-dim/40 font-mono truncate">{m.id}</p>
                )}
              </div>
              <span className="text-xs font-semibold text-text truncate">{m.provider}</span>
              <span className={`text-[10px] font-bold px-2 py-0.5 rounded-md w-fit ${tierColor(m.tier)}`}>{m.tier || "-"}</span>
              <span className="text-xs font-mono text-text">{formatCtx(m.context_window)}</span>
              <span className="text-xs font-mono text-text">{formatCost(m.input_cost_per_m)}</span>
              <span className="text-center">{m.supports_tools ? <Check className="w-4 h-4 text-success inline" /> : <X className="w-4 h-4 text-text-dim/15 inline" />}</span>
              <span className="text-center">{m.supports_vision ? <Check className="w-4 h-4 text-success inline" /> : <X className="w-4 h-4 text-text-dim/15 inline" />}</span>
              <span className="text-center">{m.supports_streaming ? <Check className="w-4 h-4 text-success inline" /> : <X className="w-4 h-4 text-text-dim/15 inline" />}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
