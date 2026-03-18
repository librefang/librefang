import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { listMemories } from "../api";

const REFRESH_MS = 30000;

export function MemoryPage() {
  const { t } = useTranslation();
  const memoryQuery = useQuery({ queryKey: ["memory", "list"], queryFn: listMemories, refetchInterval: REFRESH_MS });

  const memories = memoryQuery.data?.memories ?? [];
  const totalCount = memoryQuery.data?.total ?? 0;

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <header className="flex flex-col justify-between gap-4 sm:flex-row sm:items-end">
        <div>
          <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
            <svg className="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z" /></svg>
            {t("memory.cognitive_layer")}
          </div>
          <h1 className="mt-2 text-3xl font-extrabold tracking-tight md:text-4xl">{t("memory.title")}</h1>
          <p className="mt-1 text-text-dim font-medium">{t("memory.subtitle")}</p>
        </div>
        <div className="flex items-center gap-3">
          <div className="rounded-full border border-border-subtle bg-surface px-3 py-1.5 text-[10px] font-bold uppercase text-text-dim">
            {t("memory.objects_stored", { count: totalCount })}
          </div>
          <button className="flex h-9 items-center gap-2 rounded-xl border border-border-subtle bg-surface px-4 text-sm font-bold text-text-dim hover:text-brand shadow-sm" onClick={() => void memoryQuery.refetch()}>
            {t("common.refresh")}
          </button>
        </div>
      </header>

      <div className="grid gap-4">
        {memories.map((m) => (
          <article key={m.id} className="group rounded-2xl border border-border-subtle bg-surface p-5 shadow-sm transition-all hover:border-brand/30">
            <div className="flex items-center gap-2 mb-1">
              <h2 className="text-sm font-black truncate">{m.id}</h2>
              <span className="rounded-lg bg-brand/10 border border-brand/10 px-2 py-0.5 text-[9px] font-black text-brand uppercase">{m.level || "Vector"}</span>
            </div>
            <p className="text-xs text-text-dim line-clamp-2 leading-relaxed">{m.content || t("common.no_data")}</p>
          </article>
        ))}
      </div>
    </div>
  );
}
