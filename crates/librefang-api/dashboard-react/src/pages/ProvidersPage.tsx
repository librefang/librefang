import { useMutation, useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { listProviders, testProvider } from "../api";

const REFRESH_MS = 30000;

export function ProvidersPage() {
  const { t } = useTranslation();
  const [feedback, setFeedback] = useState<Record<string, any>>({});
  const [pendingId, setPendingId] = useState<string | null>(null);

  const providersQuery = useQuery({ queryKey: ["providers", "list"], queryFn: listProviders, refetchInterval: REFRESH_MS });
  const testMutation = useMutation({ mutationFn: testProvider });

  const providers = providersQuery.data ?? [];
  const configuredCount = providers.filter(p => p.auth_status === "configured").length;

  async function handleTest(id: string) {
    setPendingId(id);
    try { await testMutation.mutateAsync(id); setFeedback(c => ({...c, [id]: {type: "ok", text: t("common.ok")}})); }
    catch (e: any) { setFeedback(c => ({...c, [id]: {type: "error", text: e.message}})); }
    finally { setPendingId(null); }
  }

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <header className="flex flex-col justify-between gap-4 sm:flex-row sm:items-end">
        <div>
          <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
            <svg className="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><rect x="2" y="2" width="20" height="8" rx="2" /><rect x="2" y="14" width="20" height="8" rx="2" /></svg>
            {t("common.infrastructure")}
          </div>
          <h1 className="mt-2 text-3xl font-extrabold tracking-tight">{t("providers.title")}</h1>
          <p className="mt-1 text-text-dim font-medium">{t("providers.subtitle")}</p>
        </div>
        <div className="flex items-center gap-3">
          <div className="hidden rounded-full border border-border-subtle bg-surface px-3 py-1.5 text-[10px] font-bold uppercase text-text-dim sm:block">{t("providers.configured_count", { configured: configuredCount, total: providers.length })}</div>
          <button className="flex h-9 items-center gap-2 rounded-xl border border-border-subtle bg-surface px-4 text-sm font-bold text-text-dim hover:text-brand transition-all shadow-sm" onClick={() => void providersQuery.refetch()}>{t("common.refresh")}</button>
        </div>
      </header>

      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
        {providers.map((p) => (
          <article key={p.id} className="group flex flex-col rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm ring-1 ring-black/5 dark:ring-white/5">
            <div className="mb-5 flex items-start justify-between gap-3">
              <div className="min-w-0"><h2 className="m-0 text-lg font-black truncate">{p.display_name || p.id}</h2><p className="text-[10px] font-black uppercase text-text-dim/60 mt-0.5">{p.id}</p></div>
              <span className={`rounded-lg border px-2 py-0.5 text-[10px] font-black uppercase ${p.auth_status === 'configured' ? 'border-success/20 bg-success/10 text-success' : 'border-border-subtle bg-main text-text-dim'}`}>{p.auth_status === 'configured' ? t("common.active") : t("common.setup")}</span>
            </div>
            <div className="grid grid-cols-2 gap-4 mb-6">
              <div className="p-3 rounded-xl bg-main/40"><p className="text-[10px] font-black text-text-dim/60 uppercase mb-1">{t("providers.models")}</p><p className="text-xl font-black">{p.model_count || 0}</p></div>
              <div className="p-3 rounded-xl bg-main/40"><p className="text-[10px] font-black text-text-dim/60 uppercase mb-1">{t("providers.latency")}</p><p className="text-xl font-black">{p.latency_ms ? `${p.latency_ms}ms` : "-"}</p></div>
            </div>
            <button className="w-full rounded-xl border border-border-subtle bg-surface py-2 text-xs font-black text-text-dim hover:text-brand transition-all" onClick={() => handleTest(p.id)} disabled={pendingId === p.id}>{pendingId === p.id ? t("providers.analyzing") : t("providers.test_connection")}</button>
          </article>
        ))}
      </div>
    </div>
  );
}
