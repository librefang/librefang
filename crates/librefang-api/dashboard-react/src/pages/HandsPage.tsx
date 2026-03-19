import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { activateHand, deactivateHand, listActiveHands, listHands } from "../api";
import { PageHeader } from "../components/ui/PageHeader";
import { CardSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { useUIStore } from "../lib/store";
import { Hand } from "lucide-react";

const REFRESH_MS = 15000;

export function HandsPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const addToast = useUIStore((s) => s.addToast);
  const [pendingActivateId, setPendingActivateId] = useState<string | null>(null);

  const handsQuery = useQuery({ queryKey: ["hands", "list"], queryFn: listHands, refetchInterval: REFRESH_MS });
  const activeQuery = useQuery({ queryKey: ["hands", "active"], queryFn: listActiveHands, refetchInterval: REFRESH_MS });

  const activateMutation = useMutation({ mutationFn: (id: string) => activateHand(id) });
  const deactivateMutation = useMutation({ mutationFn: (id: string) => deactivateHand(id) });

  const hands = handsQuery.data ?? [];
  const instances = activeQuery.data ?? [];
  const activeHandIds = useMemo(() => new Set(instances.map(i => i.hand_id).filter(Boolean)), [instances]);

  async function handleActivate(id: string) {
    setPendingActivateId(id);
    try {
      await activateMutation.mutateAsync(id);
      await queryClient.invalidateQueries({ queryKey: ["hands"] });
      addToast(t("common.success"), "success");
    } catch (e: any) {
      addToast(e.message || t("hands.activate_failed"), "error");
    } finally {
      setPendingActivateId(null);
    }
  }

  async function handleDeactivate(id: string) {
    setPendingActivateId(id);
    try {
      await deactivateMutation.mutateAsync(id);
      await queryClient.invalidateQueries({ queryKey: ["hands"] });
      addToast(t("common.success"), "success");
    } catch (e: any) {
      addToast(e.message || t("common.error"), "error");
    } finally {
      setPendingActivateId(null);
    }
  }

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("hands.orchestration")}
        title={t("hands.title")}
        subtitle={t("hands.subtitle")}
        isFetching={handsQuery.isLoading}
        onRefresh={() => void handsQuery.refetch()}
        icon={<Hand className="h-4 w-4" />}
      />

      {handsQuery.isLoading ? (
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
          {[1, 2, 3, 4].map((i) => <CardSkeleton key={i} />)}
        </div>
      ) : hands.length === 0 ? (
        <EmptyState
          title={t("common.no_data")}
          icon={<Hand className="h-6 w-6" />}
        />
      ) : (
        <>
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
            {[
              { label: t("hands.available"), value: hands.length },
              { label: t("hands.instances"), value: instances.length },
              { label: t("hands.ready_hands"), value: hands.filter(h => h.requirements_met).length },
              { label: t("common.running"), value: activeHandIds.size },
            ].map((stat, i) => (
              <article key={i} className="rounded-2xl border border-border-subtle bg-surface p-5 shadow-sm hover:border-brand/30 transition-all">
                <span className="text-[10px] font-black uppercase tracking-widest text-text-dim">{stat.label}</span>
                <strong className="mt-2 block text-3xl font-black tracking-tight">{stat.value}</strong>
              </article>
            ))}
          </div>

          <div className="grid gap-6 xl:grid-cols-2">
            <article className="rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm">
              <h2 className="text-lg font-black tracking-tight mb-1">{t("hands.available")}</h2>
              <p className="mb-6 text-xs text-text-dim">{t("hands.ready_description")}</p>
              <div className="space-y-3">
                {hands.map(h => (
                  <div key={h.id} className="p-4 rounded-xl bg-main/40 border border-border-subtle hover:border-brand/30 transition-all">
                    <div className="flex justify-between items-start">
                      <div><p className="text-sm font-black">{h.name || h.id}</p><p className="text-xs text-text-dim italic">{h.description || "-"}</p></div>
                      <button
                        className="rounded-lg bg-brand px-4 py-1.5 text-xs font-bold text-white shadow-lg hover:opacity-90 disabled:opacity-50 transition-all"
                        onClick={() => handleActivate(h.id)}
                        disabled={pendingActivateId === h.id}
                      >
                        {pendingActivateId === h.id ? t("common.loading") : t("hands.activate")}
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            </article>
          </div>
        </>
      )}
    </div>
  );
}
