import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { listSkills } from "../api";

const REFRESH_MS = 30000;

export function SkillsPage() {
  const { t } = useTranslation();
  const skillsQuery = useQuery({ queryKey: ["skills", "list"], queryFn: listSkills, refetchInterval: REFRESH_MS });

  const skills = skillsQuery.data ?? [];

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <header className="flex flex-col justify-between gap-4 sm:flex-row sm:items-end">
        <div>
          <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
            <svg className="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M18 8A6 6 0 0 0 6 8c0 7-3 9-3 9h18s-3-2-3-9" /></svg>
            {t("common.infrastructure")}
          </div>
          <h1 className="mt-2 text-3xl font-extrabold tracking-tight">{t("skills.title")}</h1>
          <p className="mt-1 text-text-dim font-medium">{t("skills.subtitle")}</p>
        </div>
        <button className="flex h-9 items-center gap-2 rounded-xl border border-border-subtle bg-surface px-4 text-sm font-bold text-text-dim hover:text-brand shadow-sm" onClick={() => void skillsQuery.refetch()}>
          {t("common.refresh")}
        </button>
      </header>

      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
        {skills.map(s => (
          <article key={s.name} className="group rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm hover:border-brand/30 transition-all">
            <div className="flex items-center justify-between mb-4">
              <h2 className="text-lg font-black truncate">{s.name}</h2>
              <span className="px-2 py-0.5 rounded-lg bg-brand/10 border border-brand/20 text-[9px] font-black text-brand uppercase">{s.version || "1.0.0"}</span>
            </div>
            <p className="text-xs text-text-dim line-clamp-2 italic mb-6">{s.description || "-"}</p>
            <div className="flex justify-between items-center text-[10px] font-bold text-text-dim uppercase mb-6">
              <span>{t("skills.author")}: {s.author || t("common.unknown")}</span>
              <span>{t("skills.tools")}: {s.tools_count || 0}</span>
            </div>
            <button className="w-full rounded-xl border border-border-subtle bg-surface py-2 text-xs font-black text-text-dim hover:text-error transition-all">{t("skills.uninstall")}</button>
          </article>
        ))}
        {skills.length === 0 && !skillsQuery.isLoading && (
          <div className="col-span-full py-24 text-center border border-dashed border-border-subtle rounded-3xl bg-surface/30">
            <p className="text-sm text-text-dim font-black">{t("skills.no_skills")}</p>
          </div>
        )}
      </div>
    </div>
  );
}
