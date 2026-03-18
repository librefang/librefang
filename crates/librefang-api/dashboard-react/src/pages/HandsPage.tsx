import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  activateHand, deactivateHand, getHandStats, listActiveHands, listHands, pauseHand, resumeHand,
  type ApiActionResponse, type HandStatsResponse
} from "../api";

const REFRESH_MS = 15000;

export function HandsPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [feedback, setFeedback] = useState<{type: "ok" | "error", text: string} | null>(null);
  const [pendingActivateId, setPendingActivateId] = useState<string | null>(null);
  const [pendingInstanceId, setPendingInstanceId] = useState<string | null>(null);
  const [statsByInstance, setStatsByInstance] = useState<Record<string, HandStatsResponse>>({});

  const handsQuery = useQuery({ queryKey: ["hands", "list"], queryFn: listHands, refetchInterval: REFRESH_MS });
  const activeQuery = useQuery({ queryKey: ["hands", "active"], queryFn: listActiveHands, refetchInterval: REFRESH_MS });

  const activateMutation = useMutation({ mutationFn: (id: string) => activateHand(id) });
  const pauseMutation = useMutation({ mutationFn: (id: string) => pauseHand(id) });
  const resumeMutation = useMutation({ mutationFn: (id: string) => resumeHand(id) });
  const deactivateMutation = useMutation({ mutationFn: (id: string) => deactivateHand(id) });

  const hands = handsQuery.data ?? [];
  const instances = activeQuery.data ?? [];
  const activeHandIds = useMemo(() => new Set(instances.map(i => i.hand_id).filter(Boolean)), [instances]);

  async function handleActivate(id: string) {
    setPendingActivateId(id);
    try { await activateMutation.mutateAsync(id); await queryClient.invalidateQueries({ queryKey: ["hands"] }); }
    catch (e: any) { setFeedback({ type: "error", text: t("hands.activate_failed") }); }
    finally { setPendingActivateId(null); }
  }

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <header className="flex flex-col justify-between gap-4 sm:flex-row sm:items-end">
        <div>
          <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
            <svg className="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M18 11V6a2 2 0 0 0-2-2v0a2 2 0 0 0-2 2v0" /><path d="M14 10V4a2 2 0 0 0-2-2v0a2 2 0 0 0-2 2v0" /><path d="M10 10.5V6a2 2 0 0 0-2-2v0a2 2 0 0 0-2 2v0" />
            </svg>
            {t("hands.orchestration")}
          </div>
          <h1 className="mt-2 text-3xl font-extrabold tracking-tight md:text-4xl">{t("hands.title")}</h1>
          <p className="mt-1 text-text-dim font-medium">{t("hands.subtitle")}</p>
        </div>
        <button className="flex h-9 items-center gap-2 rounded-xl border border-border-subtle bg-surface px-4 text-sm font-bold text-text-dim hover:text-brand transition-all shadow-sm" onClick={() => void handsQuery.refetch()}>
          <svg className={`h-3.5 w-3.5 ${handsQuery.isFetching ? "animate-spin" : ""}`} fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2"><path d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" /></svg>
          {t("common.refresh")}
        </button>
      </header>

      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        {[
          { label: t("hands.available"), value: hands.length },
          { label: t("hands.instances"), value: instances.length },
          { label: t("hands.ready_hands"), value: hands.filter(h => h.requirements_met).length },
          { label: t("common.running"), value: activeHandIds.size },
        ].map((stat, i) => (
          <article key={i} className="rounded-2xl border border-border-subtle bg-surface p-5 shadow-sm">
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
              <div key={h.id} className="p-4 rounded-xl bg-main/40 border border-border-subtle">
                <div className="flex justify-between items-start">
                  <div><p className="text-sm font-black">{h.name || h.id}</p><p className="text-xs text-text-dim italic">{h.description || "-"}</p></div>
                  <button className="rounded-lg bg-brand px-4 py-1.5 text-xs font-bold text-white shadow-lg" onClick={() => handleActivate(h.id)}>{t("hands.activate")}</button>
                </div>
              </div>
            ))}
          </div>
        </article>
      </div>
    </div>
  );
}
