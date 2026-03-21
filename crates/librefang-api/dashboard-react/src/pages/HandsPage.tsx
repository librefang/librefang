import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { activateHand, deactivateHand, listActiveHands, listHands } from "../api";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { useUIStore } from "../lib/store";
import { Input } from "../components/ui/Input";
import { Hand, RefreshCw, Search, Power, PowerOff, Loader2, Check } from "lucide-react";

const REFRESH_MS = 15000;

export function HandsPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const addToast = useUIStore((s) => s.addToast);
  const [pendingId, setPendingId] = useState<string | null>(null);
  const [search, setSearch] = useState("");

  const handsQuery = useQuery({ queryKey: ["hands", "list"], queryFn: listHands, refetchInterval: REFRESH_MS });
  const activeQuery = useQuery({ queryKey: ["hands", "active"], queryFn: listActiveHands, refetchInterval: REFRESH_MS });

  const activateMutation = useMutation({ mutationFn: (id: string) => activateHand(id) });
  const deactivateMutation = useMutation({ mutationFn: (id: string) => deactivateHand(id) });

  const hands = handsQuery.data ?? [];
  const instances = activeQuery.data ?? [];
  const activeHandIds = useMemo(() => new Set(instances.map(i => i.hand_id).filter(Boolean)), [instances]);

  const filtered = hands.filter(h => {
    if (!search) return true;
    const q = search.toLowerCase();
    return (h.name || "").toLowerCase().includes(q) || (h.id || "").toLowerCase().includes(q) || (h.description || "").toLowerCase().includes(q);
  }).sort((a, b) => {
    // Active first
    const aActive = activeHandIds.has(a.id) ? 0 : 1;
    const bActive = activeHandIds.has(b.id) ? 0 : 1;
    return aActive - bActive || (a.name || a.id).localeCompare(b.name || b.id);
  });

  async function handleActivate(id: string) {
    setPendingId(id);
    try {
      await activateMutation.mutateAsync(id);
      await queryClient.invalidateQueries({ queryKey: ["hands"] });
      addToast(t("common.success"), "success");
    } catch (e: any) {
      addToast(e.message || t("common.error"), "error");
    } finally { setPendingId(null); }
  }

  async function handleDeactivate(id: string) {
    setPendingId(id);
    try {
      await deactivateMutation.mutateAsync(id);
      await queryClient.invalidateQueries({ queryKey: ["hands"] });
      addToast(t("common.success"), "success");
    } catch (e: any) {
      addToast(e.message || t("common.error"), "error");
    } finally { setPendingId(null); }
  }

  const activeCount = activeHandIds.size;

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      {/* Header */}
      <header className="flex flex-col justify-between gap-4 sm:flex-row sm:items-end">
        <div>
          <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
            <Hand className="h-4 w-4" />
            {t("hands.orchestration")}
          </div>
          <h1 className="mt-2 text-3xl font-extrabold tracking-tight">{t("hands.title")}</h1>
          <p className="mt-1 text-text-dim font-medium text-sm">{t("hands.subtitle")}</p>
        </div>
        <div className="flex items-center gap-3">
          <Badge variant="success">{activeCount} {t("hands.active_label")}</Badge>
          <Badge variant="default">{hands.length} {t("hands.total_label")}</Badge>
          <Button variant="secondary" onClick={() => { handsQuery.refetch(); activeQuery.refetch(); }}>
            <RefreshCw className={`h-3.5 w-3.5 ${handsQuery.isFetching ? "animate-spin" : ""}`} />
          </Button>
        </div>
      </header>

      {/* Search */}
      {hands.length > 0 && (
        <Input value={search} onChange={(e) => setSearch(e.target.value)}
          placeholder={t("hands.search_placeholder")}
          leftIcon={<Search className="h-4 w-4" />} />
      )}

      {/* Hands List */}
      {handsQuery.isLoading ? (
        <div className="grid gap-3 md:grid-cols-2">
          {[1, 2, 3, 4].map(i => <div key={i} className="h-28 rounded-2xl bg-main animate-pulse" />)}
        </div>
      ) : hands.length === 0 ? (
        <div className="text-center py-16">
          <Hand className="w-10 h-10 text-text-dim/20 mx-auto mb-3" />
          <p className="text-sm text-text-dim">{t("common.no_data")}</p>
        </div>
      ) : (
        <div className="grid gap-3 md:grid-cols-2">
          {filtered.map(h => {
            const isActive = activeHandIds.has(h.id);
            const instance = instances.find(i => i.hand_id === h.id);
            const isPending = pendingId === h.id;
            return (
              <div key={h.id}
                className={`p-4 rounded-2xl border transition-all ${
                  isActive ? "border-success/30 bg-success/5" : "border-border-subtle hover:border-brand/30"
                }`}>
                <div className="flex items-start gap-3">
                  <div className={`w-10 h-10 rounded-xl flex items-center justify-center shrink-0 ${
                    isActive ? "bg-success/20 text-success" : "bg-main text-text-dim/40"
                  }`}>
                    <Hand className="w-5 h-5" />
                  </div>
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <h3 className="text-sm font-bold truncate">{h.name || h.id}</h3>
                      {isActive ? (
                        <Badge variant="success"><Check className="w-3 h-3 mr-0.5" />{t("hands.active_label")}</Badge>
                      ) : h.requirements_met ? (
                        <Badge variant="default">{t("hands.ready")}</Badge>
                      ) : (
                        <Badge variant="warning">{t("hands.missing_req")}</Badge>
                      )}
                    </div>
                    <p className="text-[10px] text-text-dim mt-0.5 line-clamp-2">{h.description || "-"}</p>
                    {instance && (
                      <p className="text-[9px] text-text-dim/50 font-mono mt-1">{t("hands.instance")}: {instance.instance_id?.slice(0, 8)}</p>
                    )}
                  </div>
                  <div className="shrink-0">
                    {isActive ? (
                      <Button variant="secondary" size="sm" onClick={() => instance && handleDeactivate(instance.instance_id)} disabled={isPending}>
                        {isPending ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <PowerOff className="w-3.5 h-3.5" />}
                      </Button>
                    ) : (
                      <Button variant="primary" size="sm" onClick={() => handleActivate(h.id)} disabled={isPending || !h.requirements_met}>
                        {isPending ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Power className="w-3.5 h-3.5" />}
                      </Button>
                    )}
                  </div>
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
