import { useQuery } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { listModels, type ModelItem } from "../api";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import {
  Cpu, Search, RefreshCw, Check, X, Eye, Wrench, Zap,
  ChevronDown, DollarSign, Database
} from "lucide-react";

const TIERS = ["all", "basic", "smart", "standard", "advanced", "enterprise"] as const;
const REFRESH_MS = 60000;

export function ModelsPage() {
  const { t } = useTranslation();
  const [search, setSearch] = useState("");
  const [tierFilter, setTierFilter] = useState<string>("all");
  const [providerFilter, setProviderFilter] = useState<string>("all");
  const [availableOnly, setAvailableOnly] = useState(false);

  const modelsQuery = useQuery({
    queryKey: ["models"],
    queryFn: () => listModels(),
    refetchInterval: REFRESH_MS
  });

  const allModels = modelsQuery.data?.models ?? [];
  const totalAvailable = modelsQuery.data?.available ?? 0;

  // 提取 provider 列表
  const providers = useMemo(() => {
    const set = new Set(allModels.map(m => m.provider));
    return ["all", ...Array.from(set).sort()];
  }, [allModels]);

  // 过滤
  const filtered = useMemo(() => {
    return allModels.filter(m => {
      if (search && !m.id.toLowerCase().includes(search.toLowerCase()) && !(m.display_name || "").toLowerCase().includes(search.toLowerCase())) return false;
      if (tierFilter !== "all" && m.tier !== tierFilter) return false;
      if (providerFilter !== "all" && m.provider !== providerFilter) return false;
      if (availableOnly && !m.available) return false;
      return true;
    });
  }, [allModels, search, tierFilter, providerFilter, availableOnly]);

  const tierColor = (tier?: string) => {
    switch (tier) {
      case "basic": return "text-text-dim bg-main";
      case "smart": return "text-blue-600 bg-blue-50 dark:text-blue-400 dark:bg-blue-900/20";
      case "standard": return "text-green-600 bg-green-50 dark:text-green-400 dark:bg-green-900/20";
      case "advanced": return "text-purple-600 bg-purple-50 dark:text-purple-400 dark:bg-purple-900/20";
      case "enterprise": return "text-amber-600 bg-amber-50 dark:text-amber-400 dark:bg-amber-900/20";
      default: return "text-text-dim bg-main";
    }
  };

  const formatCost = (cost?: number) => {
    if (cost === undefined || cost === null) return "-";
    if (cost === 0) return t("models.free");
    return `$${cost.toFixed(2)}`;
  };

  const formatContext = (tokens?: number) => {
    if (!tokens) return "-";
    if (tokens >= 1000000) return `${(tokens / 1000000).toFixed(1)}M`;
    if (tokens >= 1000) return `${Math.round(tokens / 1000)}K`;
    return String(tokens);
  };

  return (
    <div className="flex flex-col gap-6">
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
          <Badge variant="brand">{totalAvailable} / {allModels.length} {t("models.available")}</Badge>
          <Button variant="secondary" onClick={() => modelsQuery.refetch()}>
            <RefreshCw className={`h-3.5 w-3.5 ${modelsQuery.isFetching ? "animate-spin" : ""}`} />
          </Button>
        </div>
      </header>

      {/* Filters */}
      <div className="flex flex-wrap gap-3 items-center">
        <div className="relative flex-1 max-w-sm">
          <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-text-dim/40" />
          <input type="text" value={search} onChange={e => setSearch(e.target.value)}
            placeholder={t("models.search_placeholder")}
            className="w-full pl-10 pr-4 py-2 rounded-xl border border-border-subtle bg-surface text-sm outline-none focus:border-brand" />
        </div>

        <select value={providerFilter} onChange={e => setProviderFilter(e.target.value)}
          className="rounded-xl border border-border-subtle bg-surface px-3 py-2 text-xs outline-none focus:border-brand">
          {providers.map(p => <option key={p} value={p}>{p === "all" ? t("models.all_providers") : p}</option>)}
        </select>

        <div className="flex gap-1 rounded-xl border border-border-subtle bg-surface p-0.5">
          {TIERS.map(tier => (
            <button key={tier} onClick={() => setTierFilter(tier)}
              className={`px-2.5 py-1 rounded-lg text-[10px] font-bold transition-colors ${
                tierFilter === tier ? "bg-brand text-white" : "text-text-dim hover:text-text"
              }`}>
              {tier === "all" ? t("models.all_tiers") : tier}
            </button>
          ))}
        </div>

        <button onClick={() => setAvailableOnly(!availableOnly)}
          className={`flex items-center gap-1.5 px-3 py-2 rounded-xl border text-xs font-bold transition-colors ${
            availableOnly ? "border-success bg-success/10 text-success" : "border-border-subtle text-text-dim"
          }`}>
          <Check className="w-3 h-3" />
          {t("models.available_only")}
        </button>
      </div>

      {/* Results count */}
      <p className="text-xs text-text-dim">{filtered.length} {t("models.results")}</p>

      {/* Model List */}
      {modelsQuery.isLoading ? (
        <div className="space-y-2">
          {[1, 2, 3, 4].map(i => (
            <div key={i} className="p-3 rounded-xl border border-border-subtle animate-pulse flex gap-3">
              <div className="w-8 h-8 rounded-lg bg-main" /><div className="flex-1 space-y-1.5"><div className="h-3 w-48 bg-main rounded" /><div className="h-2.5 w-32 bg-main rounded" /></div>
            </div>
          ))}
        </div>
      ) : (
        <div className="space-y-1">
          {/* Table header */}
          <div className="grid grid-cols-[1fr_100px_80px_80px_80px_60px_60px_60px] gap-2 px-3 py-2 text-[9px] font-bold text-text-dim/50 uppercase">
            <span>{t("models.col_model")}</span>
            <span>{t("models.col_provider")}</span>
            <span>{t("models.col_tier")}</span>
            <span>{t("models.col_context")}</span>
            <span>{t("models.col_input")}</span>
            <span className="text-center"><Wrench className="w-3 h-3 inline" /></span>
            <span className="text-center"><Eye className="w-3 h-3 inline" /></span>
            <span className="text-center"><Zap className="w-3 h-3 inline" /></span>
          </div>

          {filtered.map(m => (
            <div key={m.id}
              className={`grid grid-cols-[1fr_100px_80px_80px_80px_60px_60px_60px] gap-2 px-3 py-2.5 rounded-xl border border-transparent hover:border-border-subtle hover:bg-surface transition-all items-center ${
                !m.available ? "opacity-50" : ""
              }`}>
              <div className="min-w-0">
                <p className="text-xs font-bold truncate">{m.display_name || m.id}</p>
                {m.display_name && <p className="text-[9px] text-text-dim/50 font-mono truncate">{m.id}</p>}
              </div>
              <span className="text-[10px] font-semibold text-text-dim truncate">{m.provider}</span>
              <span className={`text-[9px] font-bold px-1.5 py-0.5 rounded-md inline-block w-fit ${tierColor(m.tier)}`}>{m.tier || "-"}</span>
              <span className="text-[10px] font-mono text-text-dim">{formatContext(m.context_window)}</span>
              <span className="text-[10px] font-mono text-text-dim">{formatCost(m.input_cost_per_m)}</span>
              <span className="text-center">{m.supports_tools ? <Check className="w-3.5 h-3.5 text-success inline" /> : <X className="w-3.5 h-3.5 text-text-dim/20 inline" />}</span>
              <span className="text-center">{m.supports_vision ? <Check className="w-3.5 h-3.5 text-success inline" /> : <X className="w-3.5 h-3.5 text-text-dim/20 inline" />}</span>
              <span className="text-center">{m.supports_streaming ? <Check className="w-3.5 h-3.5 text-success inline" /> : <X className="w-3.5 h-3.5 text-text-dim/20 inline" />}</span>
            </div>
          ))}

          {filtered.length === 0 && (
            <div className="text-center py-12 text-sm text-text-dim">{t("models.no_results")}</div>
          )}
        </div>
      )}
    </div>
  );
}
