import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { listChannels } from "../api";

const REFRESH_MS = 30000;

export function ChannelsPage() {
  const { t } = useTranslation();
  const channelsQuery = useQuery({ queryKey: ["channels", "list"], queryFn: listChannels, refetchInterval: REFRESH_MS });

  const channels = channelsQuery.data ?? [];
  const activeCount = channels.filter(c => c.configured).length;

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <header className="flex flex-col justify-between gap-4 sm:flex-row sm:items-end">
        <div>
          <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
            <svg className="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><circle cx="18" cy="5" r="3" /><circle cx="6" cy="12" r="3" /><circle cx="18" cy="19" r="3" /></svg>
            {t("common.infrastructure")}
          </div>
          <h1 className="mt-2 text-3xl font-extrabold tracking-tight">{t("channels.title")}</h1>
          <p className="mt-1 text-text-dim font-medium">{t("channels.subtitle")}</p>
        </div>
        <div className="flex items-center gap-3">
          <div className="rounded-full border border-border-subtle bg-surface px-3 py-1.5 text-[10px] font-bold uppercase text-text-dim">
            {t("channels.configured_count", { count: activeCount })}
          </div>
          <button className="flex h-9 items-center gap-2 rounded-xl border border-border-subtle bg-surface px-4 text-sm font-bold text-text-dim hover:text-brand shadow-sm" onClick={() => void channelsQuery.refetch()}>
            {t("common.refresh")}
          </button>
        </div>
      </header>

      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
        {channels.map(c => (
          <article key={c.name} className="group rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm hover:border-brand/30 transition-all">
            <div className="flex items-center justify-between mb-4">
              <h2 className="text-lg font-black truncate">{c.display_name || c.name}</h2>
              <span className={`px-2 py-0.5 rounded-lg border text-[9px] font-black uppercase ${c.configured ? 'border-success/20 bg-success/10 text-success' : 'border-border-subtle bg-main text-text-dim'}`}>
                {c.configured ? t("common.online") : t("common.setup")}
              </span>
            </div>
            <p className="text-xs text-text-dim line-clamp-2 italic mb-6">{c.description || "-"}</p>
            <button className="w-full rounded-xl border border-border-subtle bg-surface py-2 text-xs font-black text-text-dim hover:text-brand transition-all">{t("channels.setup_adapter")}</button>
          </article>
        ))}
        {channels.length === 0 && !channelsQuery.isLoading && (
          <div className="col-span-full py-24 text-center border border-dashed border-border-subtle rounded-3xl bg-surface/30">
            <p className="text-sm text-text-dim font-black">{t("channels.no_channels")}</p>
          </div>
        )}
      </div>
    </div>
  );
}
