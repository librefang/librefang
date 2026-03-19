import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { listSkills } from "../api";
import { PageHeader } from "../components/ui/PageHeader";
import { CardSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { Bell } from "lucide-react";

const REFRESH_MS = 30000;

export function SkillsPage() {
  const { t } = useTranslation();
  const skillsQuery = useQuery({ queryKey: ["skills", "list"], queryFn: listSkills, refetchInterval: REFRESH_MS });

  const skills = skillsQuery.data ?? [];

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("common.infrastructure")}
        title={t("skills.title")}
        subtitle={t("skills.subtitle")}
        isFetching={skillsQuery.isFetching}
        onRefresh={() => void skillsQuery.refetch()}
        icon={<Bell className="h-4 w-4" />}
      />

      {skillsQuery.isLoading ? (
        <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
          {[1, 2, 3, 4, 5, 6].map((i) => <CardSkeleton key={i} />)}
        </div>
      ) : skills.length === 0 ? (
        <EmptyState
          title={t("skills.no_skills")}
          icon={<Bell className="h-6 w-6" />}
        />
      ) : (
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
        </div>
      )}
    </div>
  );
}
