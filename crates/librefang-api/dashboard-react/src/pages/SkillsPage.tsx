import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { listSkills } from "../api";
import { PageHeader } from "../components/ui/PageHeader";
import { CardSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
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
            <Card key={s.name} hover padding="lg">
              <div className="flex items-center justify-between mb-4">
                <h2 className="text-lg font-black truncate">{s.name}</h2>
                <Badge variant="brand">{s.version || "1.0.0"}</Badge>
              </div>
              <p className="text-xs text-text-dim line-clamp-2 italic mb-6">{s.description || "-"}</p>
              <div className="flex justify-between items-center text-[10px] font-bold text-text-dim uppercase mb-6">
                <span>{t("skills.author")}: {s.author || t("common.unknown")}</span>
                <span>{t("skills.tools")}: {s.tools_count || 0}</span>
              </div>
              <Button variant="ghost" className="w-full">{t("skills.uninstall")}</Button>
            </Card>
          ))}
        </div>
      )}
    </div>
  );
}
